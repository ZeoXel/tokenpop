//! `tokenpop hook <source>`: entry point for agent hook systems.
//!
//! * `claude`: Claude Code hooks. The hook JSON arrives on stdin.
//! * `codex`: Codex hooks (stdin, same schema as Claude Code) or Codex
//!   `notify` (JSON as the last argument).
//! * `gemini`: Gemini CLI hooks (stdin).
//!
//! Always exits 0 and prints nothing, so it can never change agent behavior.

use serde_json::Value;

use crate::event::Kind;

/// Map one hook payload to (agent id, event kind).
pub fn map(source: &str, v: &Value) -> Option<(String, Kind)> {
    match source {
        "claude" | "codex" if v["hook_event_name"].is_string() => {
            // Only Claude Code also exposes its session to the proxy (a request
            // header), so only it gets per-session voices for now.
            let agent = if source == "claude" {
                claude_agent(v["session_id"].as_str())
            } else {
                "codex".into()
            };
            let kind = match v["hook_event_name"].as_str()? {
                "UserPromptSubmit" => Kind::Prompt,
                "PreToolUse" => Kind::ToolStart,
                "PostToolUse" => {
                    let r = &v["tool_response"];
                    if r["is_error"].as_bool() == Some(true)
                        || r["interrupted"].as_bool() == Some(true)
                    {
                        Kind::ToolError
                    } else {
                        Kind::ToolEnd
                    }
                }
                "PostToolUseFailure" => Kind::ToolError,
                "PermissionRequest" => Kind::Approval,
                "Notification" => {
                    let ty = v["notification_type"].as_str().unwrap_or("");
                    let msg = v["message"].as_str().unwrap_or("");
                    if ty == "permission_prompt" || msg.contains("permission") {
                        Kind::Approval
                    } else {
                        return None;
                    }
                }
                "Stop" => Kind::Done,
                _ => return None,
            };
            Some((agent, kind))
        }
        "codex" => match v["type"].as_str()? {
            "agent-turn-complete" => Some(("codex".into(), Kind::Done)),
            _ => None,
        },
        "gemini" => {
            let agent = "gemini".to_string();
            let kind = match v["hook_event_name"].as_str()? {
                "BeforeAgent" => Kind::Prompt,
                "BeforeTool" => Kind::ToolStart,
                "AfterTool" => {
                    let r = &v["tool_response"];
                    if r["error"].is_object()
                        || r["error"].is_string()
                        || r["isError"].as_bool() == Some(true)
                    {
                        Kind::ToolError
                    } else {
                        Kind::ToolEnd
                    }
                }
                "Notification" if v["notification_type"].as_str() == Some("ToolPermission") => {
                    Kind::Approval
                }
                "AfterAgent" => Kind::Done,
                _ => return None,
            };
            Some((agent, kind))
        }
        _ => None,
    }
}

/// Claude Code sessions get their own voice; the proxy derives the same id
/// from the session header so stream pops and hook cues share it.
pub fn claude_agent(session_id: Option<&str>) -> String {
    session_agent("claude", session_id)
}

pub fn session_agent(prefix: &str, session_id: Option<&str>) -> String {
    match session_id {
        Some(s) if !s.is_empty() => format!("{prefix}:{}", s.chars().take(8).collect::<String>()),
        _ => prefix.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_claude_hooks() {
        let m = |v: Value| map("claude", &v);
        assert_eq!(
            m(json!({"hook_event_name":"PreToolUse","session_id":"abcdef123456"})),
            Some(("claude:abcdef12".into(), Kind::ToolStart))
        );
        assert_eq!(m(json!({"hook_event_name":"Stop"})).unwrap().1, Kind::Done);
        assert_eq!(
            m(json!({"hook_event_name":"PostToolUse","tool_response":{"is_error":true}}))
                .unwrap()
                .1,
            Kind::ToolError
        );
        assert_eq!(
            m(json!({"hook_event_name":"Notification","notification_type":"permission_prompt"}))
                .unwrap()
                .1,
            Kind::Approval
        );
        assert!(
            m(json!({"hook_event_name":"Notification","notification_type":"idle_prompt"}))
                .is_none()
        );
        assert!(m(json!({"hook_event_name":"SessionStart"})).is_none());
    }

    #[test]
    fn maps_codex_hooks_and_gemini() {
        assert_eq!(
            map(
                "codex",
                &json!({"hook_event_name":"PreToolUse","session_id":"019abc"})
            ),
            Some(("codex".into(), Kind::ToolStart))
        );
        let g = |v: Value| map("gemini", &v).map(|x| x.1);
        assert_eq!(
            g(json!({"hook_event_name":"BeforeTool"})),
            Some(Kind::ToolStart)
        );
        assert_eq!(
            g(json!({"hook_event_name":"AfterTool","tool_response":{"error":"boom"}})),
            Some(Kind::ToolError)
        );
        assert_eq!(
            g(json!({"hook_event_name":"Notification","notification_type":"ToolPermission"})),
            Some(Kind::Approval)
        );
        assert_eq!(g(json!({"hook_event_name":"AfterAgent"})), Some(Kind::Done));
        assert_eq!(g(json!({"hook_event_name":"BeforeModel"})), None);
    }

    #[test]
    fn maps_codex_notify() {
        assert_eq!(
            map("codex", &json!({"type":"agent-turn-complete"}))
                .unwrap()
                .1,
            Kind::Done
        );
        assert!(map("codex", &json!({"type":"other"})).is_none());
    }
}
