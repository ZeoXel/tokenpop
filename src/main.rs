use std::io::Write;
use std::process::ExitCode;

use tokenpop::pack::Pack;
use tokenpop::render;

const USAGE: &str = "\
tokenpop: hear your agents think

USAGE:
  tokenpop daemon [--pack NAME] [--record FILE] [--no-audio]
                                           play events from the local socket
  tokenpop proxy --upstream URL [--port N] [--agent NAME] [--cues]
                                           tap a model API's streamed responses
  tokenpop hook claude|codex [JSON]        forward an agent hook event
  tokenpop render [OPTIONS] REC[@SECS]...  render recordings to a WAV file
  tokenpop mute | unmute                   silence / resume a running daemon
  tokenpop pack NAME                       switch the running daemon's pack
  tokenpop send JSON                       send one raw event line (testing)

RENDER OPTIONS:
  -o, --out FILE    output path (default: tokenpop.wav)
  --pack NAME       blip (default) | mech | chip
  --speed X         playback speed of the recordings (default 1.0)
  --gain DB         extra output gain for listening demos (default 0)
  REC@SECS          place a recording at SECS on the timeline (default 0)

ENV:
  TOKENPOP_SOCK     socket path override
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let res = match args.first().map(String::as_str) {
        Some("render") => cmd_render(&args[1..]),
        #[cfg(unix)]
        Some("daemon") => {
            let sock = tokenpop::daemon::default_socket();
            let record =
                tokenpop::daemon::open_recorder(flag(&args, "--record").map(std::path::Path::new));
            let pack = flag(&args, "--pack").map_or(Ok(Pack::Blip), parse_pack);
            match (pack, record) {
                (Ok(_), Ok(r)) if args.iter().any(|a| a == "--no-audio") => {
                    tokenpop::daemon::run_silent(&sock, r)
                }
                (Ok(p), Ok(r)) => tokenpop::daemon::run(&sock, p, r),
                (Err(e), _) | (_, Err(e)) => Err(e),
            }
        }
        #[cfg(all(unix, feature = "proxy"))]
        Some("proxy") => cmd_proxy(&args),
        #[cfg(unix)]
        Some("hook") => {
            cmd_hook(&args);
            Ok(())
        }
        #[cfg(unix)]
        Some("mute") => send_line(r#"{"cmd":"mute"}"#),
        #[cfg(unix)]
        Some("unmute") => send_line(r#"{"cmd":"unmute"}"#),
        #[cfg(unix)]
        Some("pack") => match args.get(1) {
            Some(n) => {
                parse_pack(n).and_then(|_| send_line(&format!(r#"{{"cmd":"pack","name":"{n}"}}"#)))
            }
            None => Err("usage: tokenpop pack NAME".into()),
        },
        #[cfg(unix)]
        Some("send") => match args.get(1) {
            Some(j) => send_line(j),
            None => Err("usage: tokenpop send JSON".into()),
        },
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tokenpop: {e}");
            ExitCode::FAILURE
        }
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn parse_pack(name: &str) -> Result<Pack, String> {
    Pack::from_name(name).ok_or_else(|| {
        let all: Vec<_> = Pack::ALL.iter().map(|p| p.name()).collect();
        format!("unknown pack `{name}` (available: {})", all.join(", "))
    })
}

#[cfg(unix)]
fn send_line(line: &str) -> Result<(), String> {
    let path = tokenpop::daemon::default_socket();
    let mut s = std::os::unix::net::UnixStream::connect(&path)
        .map_err(|e| format!("daemon not reachable at {}: {e}", path.display()))?;
    writeln!(s, "{line}").map_err(|e| e.to_string())
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let mut out = "tokenpop.wav".to_string();
    let mut pack = Pack::Blip;
    let mut speed = 1.0f64;
    let mut gain_db = 0.0f32;
    let mut recs = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "-o" | "--out" => out = val()?,
            "--pack" => pack = parse_pack(&val()?)?,
            "--speed" => speed = val()?.parse().map_err(|_| "bad --speed")?,
            "--gain" => gain_db = val()?.parse().map_err(|_| "bad --gain")?,
            _ => recs.push(a.clone()),
        }
    }
    if recs.is_empty() {
        return Err("render needs at least one recording (.jsonl)".into());
    }
    let mut tracks = Vec::new();
    for (i, r) in recs.iter().enumerate() {
        let (path, offset) = match r.rsplit_once('@') {
            Some((p, s)) => (p, s.parse().map_err(|_| format!("bad offset in `{r}`"))?),
            None => (r.as_str(), 0.0),
        };
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        tracks.push(render::load_track(&text, i, offset, speed));
    }
    let sr = 48_000;
    let mut samples = render::render(&tracks, sr, pack, 1.5);
    if gain_db != 0.0 {
        let g = 10f32.powf(gain_db / 20.0);
        samples
            .iter_mut()
            .for_each(|x| *x = (*x * g).clamp(-1.0, 1.0));
    }
    let mut f =
        std::io::BufWriter::new(std::fs::File::create(&out).map_err(|e| format!("{out}: {e}"))?);
    render::write_wav(&mut f, &samples, sr).map_err(|e| e.to_string())?;
    f.flush().map_err(|e| e.to_string())?;
    let n: usize = tracks.iter().map(|t| t.events.len()).sum();
    eprintln!(
        "tokenpop: {n} events -> {out} ({:.1}s, pack {})",
        samples.len() as f64 / 2.0 / sr as f64,
        pack.name()
    );
    Ok(())
}

#[cfg(all(unix, feature = "proxy"))]
fn cmd_proxy(args: &[String]) -> Result<(), String> {
    let upstream = flag(args, "--upstream")
        .ok_or("proxy needs --upstream URL (e.g. https://api.anthropic.com)")?;
    let port = flag(args, "--port").map_or(Ok(7317), |p| p.parse().map_err(|_| "bad --port"))?;
    let opts = tokenpop::proxy::Options {
        upstream: upstream.to_string(),
        port,
        agent: flag(args, "--agent").unwrap_or("proxy").to_string(),
        cues: args.iter().any(|a| a == "--cues"),
    };
    let sink = std::sync::Arc::new(tokenpop::sink::Sink::new(tokenpop::daemon::default_socket()));
    tokenpop::proxy::run(opts, sink)
}

/// Never fails and never prints: a hook must not change agent behavior.
#[cfg(unix)]
fn cmd_hook(args: &[String]) {
    use std::io::Read;
    let Some(source) = args.get(1) else { return };
    let json = match args.get(2) {
        Some(j) => j.clone(),
        None => {
            let mut s = String::new();
            let _ = std::io::stdin().take(1 << 20).read_to_string(&mut s);
            s
        }
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else {
        return;
    };
    if let Some((agent, kind)) = tokenpop::hook::map(source, &v) {
        let sink = tokenpop::sink::Sink::new(tokenpop::daemon::default_socket());
        sink.send(&tokenpop::sink::event_line(&agent, kind, None));
    }
}
