//! Streaming-response parser for model APIs. Pure and incremental: feed it
//! bytes as they pass through the proxy and it yields what the model is doing.
//!
//! Understands:
//! * Anthropic Messages (`content_block_delta` with text / thinking / input_json)
//! * OpenAI Responses (`response.output_text.delta`, reasoning, function args)
//! * OpenAI-compatible Chat Completions chunks (incl. DeepSeek `reasoning_content`)

use serde_json::Value;

use crate::event::Kind;

#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    /// Streamed text of a given kind (`Text`, `Reasoning` or `ToolArgs`).
    Delta(Kind, String),
    /// The response ended by asking for a tool call.
    ToolUse,
    /// The response ended normally.
    Done,
}

#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    /// Gemini reports function calls inline; remember one was seen so the
    /// final `finishReason` can be classified as a tool call.
    saw_function_call: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed raw bytes; complete `data:` lines are parsed into `out`.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Signal>) {
        self.buf.extend_from_slice(bytes);
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if let Some(d) = line.strip_prefix("data:") {
                self.parse_data(d.trim_start(), out);
            }
        }
        // A runaway line without newlines must not grow forever.
        if self.buf.len() > 1 << 20 {
            self.buf.clear();
        }
    }
}

fn delta(out: &mut Vec<Signal>, kind: Kind, v: &Value) {
    if let Some(s) = v.as_str() {
        if !s.is_empty() {
            out.push(Signal::Delta(kind, s.to_string()));
        }
    }
}

impl SseParser {
    fn parse_data(&mut self, d: &str, out: &mut Vec<Signal>) {
        if d.is_empty() || d == "[DONE]" {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(d) else {
            return;
        };
        if let Some(cands) = v["candidates"].as_array() {
            self.gemini(cands, out);
            return;
        }
        parse_typed(&v, out);
    }

    /// Gemini `streamGenerateContent?alt=sse` chunks.
    fn gemini(&mut self, cands: &[Value], out: &mut Vec<Signal>) {
        for c in cands {
            for part in c["content"]["parts"].as_array().into_iter().flatten() {
                if part["functionCall"].is_object() {
                    self.saw_function_call = true;
                    out.push(Signal::Delta(
                        Kind::ToolArgs,
                        part["functionCall"]["args"].to_string(),
                    ));
                } else if part["thought"].as_bool() == Some(true) {
                    delta(out, Kind::Reasoning, &part["text"]);
                } else {
                    delta(out, Kind::Text, &part["text"]);
                }
            }
            if c["finishReason"]
                .as_str()
                .is_some_and(|r| !r.is_empty() && r != "FINISH_REASON_UNSPECIFIED")
            {
                out.push(if self.saw_function_call {
                    Signal::ToolUse
                } else {
                    Signal::Done
                });
            }
        }
    }
}

fn parse_typed(v: &Value, out: &mut Vec<Signal>) {
    match v["type"].as_str().unwrap_or("") {
        // Anthropic Messages
        "content_block_delta" => {
            let dl = &v["delta"];
            match dl["type"].as_str() {
                Some("text_delta") => delta(out, Kind::Text, &dl["text"]),
                Some("thinking_delta") => delta(out, Kind::Reasoning, &dl["thinking"]),
                Some("input_json_delta") => delta(out, Kind::ToolArgs, &dl["partial_json"]),
                _ => {}
            }
        }
        "message_delta" => match v["delta"]["stop_reason"].as_str() {
            Some("tool_use") => out.push(Signal::ToolUse),
            Some("end_turn" | "stop_sequence") => out.push(Signal::Done),
            _ => {}
        },
        // OpenAI Responses
        "response.output_text.delta" => delta(out, Kind::Text, &v["delta"]),
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            delta(out, Kind::Reasoning, &v["delta"])
        }
        "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
            delta(out, Kind::ToolArgs, &v["delta"])
        }
        "response.completed" => {
            let calls_tool = v["response"]["output"].as_array().is_some_and(|items| {
                items.iter().any(|i| {
                    matches!(
                        i["type"].as_str(),
                        Some("function_call" | "custom_tool_call" | "local_shell_call")
                    )
                })
            });
            out.push(if calls_tool {
                Signal::ToolUse
            } else {
                Signal::Done
            });
        }
        // OpenAI-compatible Chat Completions
        _ if v["object"].as_str() == Some("chat.completion.chunk") => {
            for c in v["choices"].as_array().into_iter().flatten() {
                let dl = &c["delta"];
                delta(out, Kind::Reasoning, &dl["reasoning_content"]);
                delta(out, Kind::Reasoning, &dl["reasoning"]);
                delta(out, Kind::Text, &dl["content"]);
                for tc in dl["tool_calls"].as_array().into_iter().flatten() {
                    delta(out, Kind::ToolArgs, &tc["function"]["arguments"]);
                }
                match c["finish_reason"].as_str() {
                    Some("tool_calls") => out.push(Signal::ToolUse),
                    Some("stop") => out.push(Signal::Done),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_stream() {
        let s = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"weighing\",\"thought\":true}],\"role\":\"model\"}}]}\r\n\r\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Sure\"}]}}]}\r\n\r\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"ls\",\"args\":{\"p\":1}}}]},\"finishReason\":\"STOP\"}]}\r\n\r\n",
        );
        assert_eq!(
            run(&[s]),
            vec![
                Signal::Delta(Kind::Reasoning, "weighing".into()),
                Signal::Delta(Kind::Text, "Sure".into()),
                Signal::Delta(Kind::ToolArgs, "{\"p\":1}".into()),
                Signal::ToolUse,
            ]
        );
    }

    fn run(chunks: &[&str]) -> Vec<Signal> {
        let mut p = SseParser::new();
        let mut out = Vec::new();
        for c in chunks {
            p.feed(c.as_bytes(), &mut out);
        }
        out
    }

    #[test]
    fn anthropic_stream_split_across_reads() {
        let s = concat!(
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Hmm\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"cmd\"}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
        );
        // Split at awkward places, including mid-line and mid-UTF-8-free JSON.
        let (a, b) = s.split_at(37);
        let (b, c) = b.split_at(101);
        assert_eq!(
            run(&[a, b, c]),
            vec![
                Signal::Delta(Kind::Reasoning, "Hmm".into()),
                Signal::Delta(Kind::Text, "Hello".into()),
                Signal::Delta(Kind::ToolArgs, "{\"cmd".into()),
                Signal::ToolUse,
            ]
        );
    }

    #[test]
    fn openai_responses_stream() {
        let s = concat!(
            "event: response.reasoning_summary_text.delta\n",
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"plan\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{}\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"function_call\"}]}}\n\n",
        );
        assert_eq!(
            run(&[s]),
            vec![
                Signal::Delta(Kind::Reasoning, "plan".into()),
                Signal::Delta(Kind::Text, "Hi".into()),
                Signal::Delta(Kind::ToolArgs, "{}".into()),
                Signal::ToolUse,
            ]
        );
    }

    #[test]
    fn chat_completions_with_deepseek_reasoning() {
        let s = concat!(
            "data: {\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{\"reasoning_content\":\"think\"}}]}\n\n",
            "data: {\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
            "data: {\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\"arguments\":\"{\\\"a\"}}]}}]}\n\n",
            "data: {\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        assert_eq!(
            run(&[s]),
            vec![
                Signal::Delta(Kind::Reasoning, "think".into()),
                Signal::Delta(Kind::Text, "ok".into()),
                Signal::Delta(Kind::ToolArgs, "{\"a".into()),
                Signal::Done,
            ]
        );
    }

    #[test]
    fn ignores_noise() {
        assert!(
            run(&[": ping\n\nevent: ping\ndata: {\"type\":\"ping\"}\n\ndata: not json\n"])
                .is_empty()
        );
    }
}
