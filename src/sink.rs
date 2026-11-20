//! Client side of the wire protocol: best-effort, never-blocking delivery of
//! events to the daemon. Used by the proxy and the hook entry point.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::event::Kind;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// FNV-1a over UTF-16 code units, matching the JavaScript adapters.
pub fn hash8(s: &str) -> u8 {
    let mut h: u32 = 2_166_136_261;
    for u in s.encode_utf16() {
        h = (h ^ u as u32).wrapping_mul(16_777_619);
    }
    (h & 0xff) as u8
}

/// One wire line. Sizes and a hash only, never text.
pub fn event_line(agent: &str, kind: Kind, text: Option<&str>) -> String {
    let (wire, error) = match kind {
        Kind::Text => ("text", false),
        Kind::Reasoning => ("reasoning", false),
        Kind::ToolArgs => ("tool_args", false),
        Kind::ToolStart => ("tool_start", false),
        Kind::ToolEnd => ("tool_end", false),
        Kind::ToolError => ("tool_end", true),
        Kind::Error => ("error", false),
        Kind::Approval => ("approval", false),
        Kind::Prompt => ("prompt", false),
        Kind::Done => ("done", false),
    };
    let mut v = serde_json::json!({ "v": 1, "t": now_ms(), "agent": agent, "kind": wire });
    if let Some(t) = text {
        v["chars"] = t.encode_utf16().count().into();
        v["h"] = hash8(t).into();
    }
    if error {
        v["error"] = true.into();
    }
    v.to_string()
}

pub struct Sink {
    path: PathBuf,
    state: Mutex<(Option<UnixStream>, Option<Instant>)>,
}

impl Sink {
    pub fn new(path: PathBuf) -> Self {
        Sink {
            path,
            state: Mutex::new((None, None)),
        }
    }

    /// Send one line; silently drops it if the daemon is not running.
    pub fn send(&self, line: &str) {
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        if st.0.is_none() {
            if st.1.is_some_and(|retry| Instant::now() < retry) {
                return;
            }
            match UnixStream::connect(&self.path) {
                Ok(s) => {
                    let _ = s.set_write_timeout(Some(Duration::from_millis(50)));
                    st.0 = Some(s);
                }
                Err(_) => {
                    st.1 = Some(Instant::now() + Duration::from_secs(2));
                    return;
                }
            }
        }
        let ok = st.0.as_mut().is_some_and(|s| writeln!(s, "{line}").is_ok());
        if !ok {
            st.0 = None;
            st.1 = Some(Instant::now() + Duration::from_secs(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_js_adapter() {
        // Values computed with the JS hash8 in adapters/dsh/tokenpop.mjs.
        assert_eq!(hash8(""), (2_166_136_261u32 & 0xff) as u8);
        let line = event_line("a", Kind::Text, Some("héllo"));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["chars"], 5);
        assert_eq!(v["kind"], "text");
        let e: serde_json::Value =
            serde_json::from_str(&event_line("a", Kind::ToolError, None)).unwrap();
        assert_eq!(
            (e["kind"].as_str(), e["error"].as_bool()),
            (Some("tool_end"), Some(true))
        );
    }
}
