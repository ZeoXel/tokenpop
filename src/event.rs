//! Wire protocol: one JSON object per line.
//!
//! `{"v":1,"t":1732080000123,"agent":"dsh:0","kind":"text","chars":4,"h":183}`
//!
//! Adapters send sizes and an 8-bit hash only, never the text.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Streamed answer text.
    Text,
    /// Streamed thinking / reasoning text.
    Reasoning,
    /// Streamed tool-call arguments (the model "typing" a command or a diff).
    ToolArgs,
    ToolStart,
    ToolEnd,
    ToolError,
    Error,
    Approval,
    Prompt,
    Done,
}

impl Kind {
    /// Streaming kinds drive the pop scheduler; the rest are one-shot cues.
    pub fn is_stream(self) -> bool {
        matches!(self, Kind::Text | Kind::Reasoning | Kind::ToolArgs)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Milliseconds (only differences matter).
    pub t_ms: f64,
    pub agent: String,
    pub kind: Kind,
    pub chars: u32,
    pub h: u8,
}

#[derive(Deserialize)]
struct Wire {
    #[serde(default)]
    t: Option<f64>,
    #[serde(default)]
    agent: Option<String>,
    kind: String,
    #[serde(default)]
    chars: u32,
    #[serde(default)]
    h: u8,
    #[serde(default)]
    error: bool,
}

/// Parse one JSONL line. Unknown kinds return `None` so newer adapters
/// never crash an older daemon.
pub fn parse_line(line: &str) -> Option<Event> {
    let w: Wire = serde_json::from_str(line.trim()).ok()?;
    let kind = match w.kind.as_str() {
        "text" => Kind::Text,
        "reasoning" => Kind::Reasoning,
        "tool_args" => Kind::ToolArgs,
        "tool_start" => Kind::ToolStart,
        "tool_end" if w.error => Kind::ToolError,
        "tool_end" => Kind::ToolEnd,
        "error" => Kind::Error,
        "approval" => Kind::Approval,
        "prompt" => Kind::Prompt,
        "done" => Kind::Done,
        _ => return None,
    };
    Some(Event {
        t_ms: w.t.unwrap_or(0.0),
        agent: w.agent.unwrap_or_else(|| "default".into()),
        kind,
        chars: w.chars,
        h: w.h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_and_skips_unknown() {
        let e = parse_line(r#"{"v":1,"t":5,"agent":"a","kind":"text","chars":4,"h":9}"#).unwrap();
        assert_eq!(e.kind, Kind::Text);
        assert_eq!((e.chars, e.h), (4, 9));
        assert_eq!(
            parse_line(r#"{"kind":"tool_end","error":true}"#)
                .unwrap()
                .kind,
            Kind::ToolError
        );
        assert!(parse_line(r#"{"kind":"future_thing"}"#).is_none());
        assert!(parse_line("not json").is_none());
    }
}
