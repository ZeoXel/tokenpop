//! `tokenpop proxy`: a local pass-through for model API traffic.
//!
//! Point an agent's base URL at it (e.g. `ANTHROPIC_BASE_URL=http://127.0.0.1:7317`).
//! Requests are forwarded unchanged to `--upstream`; streamed responses are
//! relayed chunk by chunk (each flushed immediately) while a tap parses the SSE
//! and sends sizes and hashes to the daemon. Nothing is logged or stored.
//!
//! The server side is a deliberately small HTTP/1.1 implementation: one request
//! per connection (`Connection: close`), so streaming is never held up by a
//! buffering layer.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use crate::event::Kind;
use crate::hook::claude_agent;
use crate::sink::{event_line, Sink};
use crate::sse::{Signal, SseParser};

#[derive(Clone)]
pub struct Options {
    pub upstream: String,
    pub port: u16,
    /// Agent id for events (overridden per request by a Claude Code session header).
    pub agent: String,
    /// Also emit tool_start / done cues from the stream (for agents without hooks).
    pub cues: bool,
}

pub fn run(opts: Options, sink: Arc<Sink>) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", opts.port))
        .map_err(|e| format!("bind 127.0.0.1:{}: {e}", opts.port))?;
    let client = reqwest::blocking::Client::builder()
        .timeout(None)
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    eprintln!(
        "tokenpop proxy: http://127.0.0.1:{} -> {}",
        opts.port, opts.upstream
    );
    for conn in listener.incoming().flatten() {
        let (client, opts, sink) = (client.clone(), opts.clone(), sink.clone());
        thread::spawn(move || {
            if let Err(e) = handle(conn, &client, &opts, &sink) {
                if std::env::var_os("TOKENPOP_DEBUG").is_some() {
                    eprintln!("tokenpop proxy: {e}");
                }
            }
        });
    }
    Ok(())
}

struct Request {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn read_request(r: &mut BufReader<TcpStream>) -> io::Result<Request> {
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = match (parts.next(), parts.next()) {
        (Some(m), Some(t)) => (m.to_string(), t.to_string()),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bad request line",
            ))
        }
    };
    let mut headers = Vec::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            break;
        }
        let l = line.trim_end_matches(['\r', '\n']);
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let get = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    let mut body = Vec::new();
    if get("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            line.clear();
            r.read_line(&mut line)?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"))?;
            if size == 0 {
                line.clear();
                r.read_line(&mut line)?;
                break;
            }
            let start = body.len();
            body.resize(start + size, 0);
            r.read_exact(&mut body[start..])?;
            line.clear();
            r.read_line(&mut line)?;
        }
    } else if let Some(len) = get("content-length").and_then(|v| v.parse::<usize>().ok()) {
        body.resize(len, 0);
        r.read_exact(&mut body)?;
    }
    Ok(Request {
        method,
        target,
        headers,
        body,
    })
}

const HOP_BY_HOP: [&str; 7] = [
    "host",
    "connection",
    "content-length",
    "transfer-encoding",
    "accept-encoding",
    "keep-alive",
    "proxy-connection",
];

fn handle(
    conn: TcpStream,
    client: &reqwest::blocking::Client,
    opts: &Options,
    sink: &Sink,
) -> io::Result<()> {
    let mut out = conn.try_clone()?;
    let req = read_request(&mut BufReader::new(conn))?;
    let url = format!("{}{}", opts.upstream.trim_end_matches('/'), req.target);
    let method = reqwest::Method::from_bytes(req.method.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad method"))?;

    let session = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("x-claude-code-session-id"))
        .map(|(_, v)| v.clone());
    let agent = match session {
        Some(s) => claude_agent(Some(&s)),
        None => opts.agent.clone(),
    };

    let mut rb = client.request(method, &url);
    for (k, v) in &req.headers {
        if !HOP_BY_HOP.iter().any(|h| k.eq_ignore_ascii_case(h)) {
            rb = rb.header(k.as_str(), v.as_str());
        }
    }
    // Ask for an uncompressed body so the stream can be parsed as it passes.
    rb = rb.header("accept-encoding", "identity").body(req.body);

    let mut resp = match rb.send() {
        Ok(r) => r,
        Err(e) => {
            let msg = format!("tokenpop proxy: upstream error: {e}");
            write!(
                out,
                "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{msg}",
                msg.len()
            )?;
            return Ok(());
        }
    };

    let status = resp.status();
    let is_sse = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/event-stream"));
    let length = if is_sse { None } else { resp.content_length() };

    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    for (k, v) in resp.headers() {
        let k = k.as_str();
        if matches!(
            k,
            "connection" | "transfer-encoding" | "content-length" | "keep-alive"
        ) {
            continue;
        }
        if let Ok(v) = v.to_str() {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    match length {
        Some(n) => head.push_str(&format!("Content-Length: {n}\r\n")),
        None => head.push_str("Transfer-Encoding: chunked\r\n"),
    }
    head.push_str("Connection: close\r\n\r\n");
    out.write_all(head.as_bytes())?;

    let mut parser = SseParser::new();
    let mut signals = Vec::new();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if length.is_some() {
            out.write_all(&buf[..n])?;
        } else {
            write!(out, "{n:x}\r\n")?;
            out.write_all(&buf[..n])?;
            out.write_all(b"\r\n")?;
        }
        out.flush()?;
        if is_sse {
            parser.feed(&buf[..n], &mut signals);
            for s in signals.drain(..) {
                match s {
                    Signal::Delta(kind, text) => sink.send(&event_line(&agent, kind, Some(&text))),
                    Signal::ToolUse if opts.cues => {
                        sink.send(&event_line(&agent, Kind::ToolStart, None))
                    }
                    Signal::Done if opts.cues => sink.send(&event_line(&agent, Kind::Done, None)),
                    _ => {}
                }
            }
        }
    }
    if length.is_none() {
        out.write_all(b"0\r\n\r\n")?;
    }
    out.flush()
}
