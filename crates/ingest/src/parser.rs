//! Translate a JSONL line into canonical records the denormaliser writes
//! to SQLite. We never fail on schema drift: unknown line kinds become a
//! `ParsedLine::Meta` row with the raw JSON preserved.

use model::{ContentBlock, Transcript, Usage};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct ParsedLine {
    pub raw: String,
    pub raw_sha256: String,
    pub transcript: Transcript,
    pub ts_ms: i64,
    pub blocks: Vec<ContentBlock>,
}

impl ParsedLine {
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim_end_matches('\n');
        if line.is_empty() {
            return None;
        }
        let transcript: Transcript = serde_json::from_str(line).ok()?;
        let ts_ms = transcript
            .timestamp
            .as_deref()
            .and_then(parse_ts_to_ms)
            .unwrap_or(0);
        let blocks = transcript
            .message
            .as_ref()
            .and_then(|m| m.content.clone())
            .map(|c| parse_blocks(&c))
            .unwrap_or_default();
        let raw_sha256 = sha256_hex(line.as_bytes());
        Some(Self {
            raw: line.to_string(),
            raw_sha256,
            transcript,
            ts_ms,
            blocks,
        })
    }

    pub fn kind_str(&self) -> &'static str {
        self.transcript.kind.as_str()
    }

    pub fn session_id(&self) -> Option<&str> {
        self.transcript.session_id.as_deref()
    }

    pub fn usage(&self) -> Option<&Usage> {
        self.transcript
            .message
            .as_ref()
            .and_then(|m| m.usage.as_ref())
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// ISO-8601 → unix-ms.
pub fn parse_ts_to_ms(s: &str) -> Option<i64> {
    use time::format_description::well_known::Iso8601;
    use time::OffsetDateTime;
    let dt = OffsetDateTime::parse(s, &Iso8601::DEFAULT).ok()?;
    let nanos = dt.unix_timestamp_nanos();
    Some((nanos / 1_000_000) as i64)
}

/// `content` may be a JSON string (older user messages), or an array of
/// blocks. String content is mapped to a single Text block.
pub fn parse_blocks(v: &Value) -> Vec<ContentBlock> {
    match v {
        Value::String(s) => vec![ContentBlock::Text { text: s.clone() }],
        Value::Array(arr) => arr
            .iter()
            .filter_map(|b| serde_json::from_value::<ContentBlock>(b.clone()).ok())
            .collect(),
        _ => Vec::new(),
    }
}

/// Materialise the (`tool_use_id`, name, input) triples present in a line.
pub fn tool_uses(line: &ParsedLine) -> Vec<(String, String, Value)> {
    line.blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, input } => {
                Some((id.clone(), name.clone(), input.clone()))
            }
            _ => None,
        })
        .collect()
}

pub fn tool_results(line: &ParsedLine) -> Vec<(String, Option<bool>, Option<Value>)> {
    line.blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => Some((tool_use_id.clone(), *is_error, content.clone())),
            _ => None,
        })
        .collect()
}

pub fn text_and_thinking_chars(line: &ParsedLine) -> (i64, i64) {
    let mut text = 0i64;
    let mut think = 0i64;
    for b in &line.blocks {
        match b {
            ContentBlock::Text { text: t } => text += t.chars().count() as i64,
            ContentBlock::Thinking { thinking, .. } => think += thinking.chars().count() as i64,
            _ => {}
        }
    }
    (text, think)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ai_title_line() {
        let line = r#"{"type":"ai-title","aiTitle":"hello","sessionId":"s1"}"#;
        let p = ParsedLine::parse(line).unwrap();
        assert_eq!(p.kind_str(), "ai-title");
        assert_eq!(p.transcript.ai_title.as_deref(), Some("hello"));
    }

    #[test]
    fn parses_iso_timestamp() {
        let ms = parse_ts_to_ms("2026-05-16T00:53:57.670Z").unwrap();
        assert!(ms > 0);
    }

    #[test]
    fn parses_tool_use_and_result() {
        let assistant = r#"{"type":"assistant","uuid":"u1","timestamp":"2026-05-16T00:00:00Z","sessionId":"s1","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#;
        let p = ParsedLine::parse(assistant).unwrap();
        let uses = tool_uses(&p);
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].1, "Bash");

        let user = r#"{"type":"user","uuid":"u2","timestamp":"2026-05-16T00:00:01Z","sessionId":"s1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok","is_error":false}]}}"#;
        let p2 = ParsedLine::parse(user).unwrap();
        let res = tool_results(&p2);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].0, "t1");
        assert_eq!(res[0].1, Some(false));
    }

    #[test]
    fn forward_compat_unknown_top_level_type() {
        let line = r#"{"type":"future-unknown","uuid":"u9","timestamp":"2026-05-16T00:00:00Z","sessionId":"s1"}"#;
        let p = ParsedLine::parse(line).unwrap();
        assert_eq!(p.kind_str(), "other");
    }
}
