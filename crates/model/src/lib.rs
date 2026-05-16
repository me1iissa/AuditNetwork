//! Wire-level deserialisation types for Claude Code JSONL transcripts.
//!
//! The shape is best-effort: the corpus is large, evolving, and we have
//! observed several event kinds (`user`, `assistant`, `ai-title`,
//! `queue-operation`, `attachment`). We accept any line as `Transcript`,
//! preserve unknown fields under `extra`, and fall through to `Kind::Other`
//! for unrecognised top-level `type` values.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One JSONL line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    #[serde(rename = "type")]
    pub kind: Kind,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(
        default,
        rename = "parentUuid",
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_uuid: Option<String>,
    #[serde(default, rename = "sessionId", skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,

    #[serde(
        default,
        rename = "isSidechain",
        skip_serializing_if = "Option::is_none"
    )]
    pub is_sidechain: Option<bool>,
    #[serde(default, rename = "agentId", skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, rename = "promptId", skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    #[serde(default, rename = "requestId", skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, rename = "gitBranch", skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    #[serde(default, rename = "userType", skip_serializing_if = "Option::is_none")]
    pub user_type: Option<String>,
    #[serde(
        default,
        rename = "permissionMode",
        skip_serializing_if = "Option::is_none"
    )]
    pub permission_mode: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<MessageBody>,

    /// Top-level fields for `ai-title` / `queue-operation` etc.
    #[serde(default, rename = "aiTitle", skip_serializing_if = "Option::is_none")]
    pub ai_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Value>,

    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// Top-level `type` discriminator. Forward-compatible via `Other`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    User,
    Assistant,
    System,
    AiTitle,
    QueueOperation,
    Attachment,
    ToolResult,
    Compaction,
    #[serde(other)]
    Other,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::User => "user",
            Kind::Assistant => "assistant",
            Kind::System => "system",
            Kind::AiTitle => "ai-title",
            Kind::QueueOperation => "queue-operation",
            Kind::Attachment => "attachment",
            Kind::ToolResult => "tool-result",
            Kind::Compaction => "compaction",
            Kind::Other => "other",
        }
    }
}

/// `message` body (Claude API shape) when present on `user`/`assistant` rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `content` is either a plain string (older user messages) or an array of blocks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// Anthropic usage block.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: Option<i64>,
    #[serde(default)]
    pub output_tokens: Option<i64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// Content-block discriminated union — handle each variant explicitly.
/// We do NOT pre-parse blocks; the parser walks the raw `content` Value
/// so it stays tolerant of unfamiliar block types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        #[serde(default)]
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        content: Option<Value>,
        #[serde(default)]
        is_error: Option<bool>,
    },
    /// Forward-compat catch-all. Stored verbatim.
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ai_title() {
        let line = r#"{"type":"ai-title","aiTitle":"hello","sessionId":"s1"}"#;
        let t: Transcript = serde_json::from_str(line).unwrap();
        assert_eq!(t.kind, Kind::AiTitle);
        assert_eq!(t.ai_title.as_deref(), Some("hello"));
        assert_eq!(t.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn parse_user_with_string_content() {
        let line = r#"{"parentUuid":null,"isSidechain":false,"type":"user","uuid":"u1","timestamp":"2026-05-16T00:00:00Z","sessionId":"s1","message":{"role":"user","content":"hi"}}"#;
        let t: Transcript = serde_json::from_str(line).unwrap();
        assert_eq!(t.kind, Kind::User);
        assert_eq!(t.is_sidechain, Some(false));
        let body = t.message.unwrap();
        assert_eq!(body.role.as_deref(), Some("user"));
        assert!(body.content.unwrap().is_string());
    }

    #[test]
    fn parse_assistant_tool_use() {
        let line = r#"{"type":"assistant","uuid":"u2","timestamp":"2026-05-16T00:00:00Z","sessionId":"s1","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":5}}}"#;
        let t: Transcript = serde_json::from_str(line).unwrap();
        assert_eq!(t.kind, Kind::Assistant);
        let body = t.message.unwrap();
        assert_eq!(body.usage.unwrap().input_tokens, Some(10));
        let blocks: Vec<ContentBlock> = serde_json::from_value(body.content.unwrap()).unwrap();
        match &blocks[0] {
            ContentBlock::ToolUse { name, id, input } => {
                assert_eq!(name, "Bash");
                assert_eq!(id, "toolu_1");
                assert_eq!(input["command"].as_str(), Some("ls"));
            }
            _ => panic!("expected tool_use"),
        }
    }

    #[test]
    fn forward_compat_unknown_type() {
        let line = r#"{"type":"future-unknown-thing","uuid":"u3"}"#;
        let t: Transcript = serde_json::from_str(line).unwrap();
        assert_eq!(t.kind, Kind::Other);
        assert_eq!(t.uuid.as_deref(), Some("u3"));
    }
}
