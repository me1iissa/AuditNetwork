//! Project a tool_use input into denormalised facts: artifacts touched,
//! edge access_kind, and per-tool extras (file_ops, bash_commands,
//! web_fetches). The mapping is intentionally explicit per tool name
//! rather than driven by data, because each tool's input shape is a
//! contract worth pinning down.

use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactKind {
    File,
    Url,
    Command,
    GlobPattern,
    McpResource,
    Agent,
}

impl ArtifactKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ArtifactKind::File => "file",
            ArtifactKind::Url => "url",
            ArtifactKind::Command => "command",
            ArtifactKind::GlobPattern => "glob_pattern",
            ArtifactKind::McpResource => "mcp_resource",
            ArtifactKind::Agent => "agent",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ArtifactRef {
    pub kind: ArtifactKind,
    pub canonical_key: String,
    pub display: String,
    pub access_kind: &'static str, // 'read'|'write'|'edit'|'fetch'|'exec'|'list'|'grep'
}

#[derive(Debug, Clone, Default)]
pub struct ToolProjection {
    pub artifacts: Vec<ArtifactRef>,
    pub file_op: Option<FileOp>,
    pub bash: Option<BashFacts>,
    pub web: Option<WebFacts>,
}

#[derive(Debug, Clone)]
pub struct FileOp {
    pub file_path: String,
    pub op: &'static str, // 'read'|'write'|'edit'|'delete'
}

#[derive(Debug, Clone)]
pub struct BashFacts {
    pub command: String,
    pub argv0: String,
    pub command_hash: String,
}

#[derive(Debug, Clone)]
pub struct WebFacts {
    pub url: String,
    pub host: String,
    pub path: String,
    pub url_hash: String,
}

pub fn project(tool_name: &str, input: &Value) -> ToolProjection {
    match tool_name {
        "Read" | "NotebookRead" => project_file(input, "read"),
        "Write" => project_file(input, "write"),
        "Edit" | "MultiEdit" | "NotebookEdit" => project_file(input, "edit"),
        "Glob" => project_glob(input),
        "Grep" => project_grep(input),
        "Bash" | "BashOutput" => project_bash(input),
        "WebFetch" | "WebSearch" => project_web(input),
        "Agent" | "Task" => project_agent(input),
        _ => project_mcp_or_unknown(tool_name, input),
    }
}

fn project_file(input: &Value, op: &'static str) -> ToolProjection {
    let path = input.get("file_path").or_else(|| input.get("notebook_path"))
        .and_then(|v| v.as_str()).unwrap_or("").to_string();
    if path.is_empty() {
        return ToolProjection::default();
    }
    let access_kind: &'static str = match op {
        "read" => "read",
        "write" => "write",
        "edit" => "edit",
        "delete" => "write",
        _ => "read",
    };
    ToolProjection {
        artifacts: vec![ArtifactRef {
            kind: ArtifactKind::File,
            canonical_key: path.clone(),
            display: path.clone(),
            access_kind,
        }],
        file_op: Some(FileOp { file_path: path, op }),
        bash: None,
        web: None,
    }
}

fn project_glob(input: &Value) -> ToolProjection {
    let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if pattern.is_empty() {
        return ToolProjection::default();
    }
    ToolProjection {
        artifacts: vec![ArtifactRef {
            kind: ArtifactKind::GlobPattern,
            canonical_key: pattern.clone(),
            display: pattern,
            access_kind: "list",
        }],
        ..Default::default()
    }
}

fn project_grep(input: &Value) -> ToolProjection {
    let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = input.get("path").and_then(|v| v.as_str()).map(str::to_owned);
    let mut artifacts = Vec::new();
    if !pattern.is_empty() {
        artifacts.push(ArtifactRef {
            kind: ArtifactKind::GlobPattern,
            canonical_key: format!("grep:{}", pattern),
            display: pattern.clone(),
            access_kind: "grep",
        });
    }
    if let Some(p) = path {
        if !p.is_empty() {
            artifacts.push(ArtifactRef {
                kind: ArtifactKind::File,
                canonical_key: p.clone(),
                display: p,
                access_kind: "grep",
            });
        }
    }
    ToolProjection { artifacts, ..Default::default() }
}

fn project_bash(input: &Value) -> ToolProjection {
    let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if command.is_empty() {
        return ToolProjection::default();
    }
    let argv0 = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches('"')
        .to_string();
    let mut h = Sha256::new();
    h.update(command.as_bytes());
    let command_hash = hex::encode(h.finalize());

    ToolProjection {
        artifacts: vec![ArtifactRef {
            kind: ArtifactKind::Command,
            canonical_key: command_hash.clone(),
            display: command.clone(),
            access_kind: "exec",
        }],
        bash: Some(BashFacts { command, argv0, command_hash }),
        ..Default::default()
    }
}

fn project_web(input: &Value) -> ToolProjection {
    let raw_url = input.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let raw_query = input.get("query").and_then(|v| v.as_str()).map(str::to_owned);

    if !raw_url.is_empty() {
        let parsed = Url::parse(&raw_url).ok();
        let host = parsed.as_ref().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
        let path = parsed.as_ref().map(|u| u.path().to_string()).unwrap_or_default();
        let mut h = Sha256::new();
        h.update(raw_url.as_bytes());
        let url_hash = hex::encode(h.finalize());
        ToolProjection {
            artifacts: vec![ArtifactRef {
                kind: ArtifactKind::Url,
                canonical_key: raw_url.clone(),
                display: raw_url.clone(),
                access_kind: "fetch",
            }],
            web: Some(WebFacts { url: raw_url, host, path, url_hash }),
            ..Default::default()
        }
    } else if let Some(q) = raw_query {
        ToolProjection {
            artifacts: vec![ArtifactRef {
                kind: ArtifactKind::GlobPattern,
                canonical_key: format!("websearch:{q}"),
                display: q,
                access_kind: "grep",
            }],
            ..Default::default()
        }
    } else {
        ToolProjection::default()
    }
}

fn project_agent(input: &Value) -> ToolProjection {
    let subagent_type = input.get("subagent_type").and_then(|v| v.as_str()).unwrap_or("general-purpose").to_string();
    ToolProjection {
        artifacts: vec![ArtifactRef {
            kind: ArtifactKind::Agent,
            canonical_key: subagent_type.clone(),
            display: format!("agent:{subagent_type}"),
            access_kind: "exec",
        }],
        ..Default::default()
    }
}

fn project_mcp_or_unknown(tool_name: &str, _input: &Value) -> ToolProjection {
    if let Some(rest) = tool_name.strip_prefix("mcp__") {
        ToolProjection {
            artifacts: vec![ArtifactRef {
                kind: ArtifactKind::McpResource,
                canonical_key: rest.to_string(),
                display: tool_name.to_string(),
                access_kind: "exec",
            }],
            ..Default::default()
        }
    } else {
        ToolProjection::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_projects_file() {
        let p = project("Read", &json!({"file_path":"/tmp/x"}));
        assert_eq!(p.artifacts.len(), 1);
        assert_eq!(p.artifacts[0].kind.as_str(), "file");
        assert_eq!(p.artifacts[0].access_kind, "read");
        assert_eq!(p.file_op.as_ref().unwrap().op, "read");
    }

    #[test]
    fn bash_hashes_command() {
        let p = project("Bash", &json!({"command":"git status -s"}));
        let b = p.bash.unwrap();
        assert_eq!(b.argv0, "git");
        assert_eq!(b.command_hash.len(), 64);
    }

    #[test]
    fn webfetch_parses_host_path() {
        let p = project("WebFetch", &json!({"url":"https://example.com/path/x?y=1"}));
        let w = p.web.unwrap();
        assert_eq!(w.host, "example.com");
        assert_eq!(w.path, "/path/x");
    }

    #[test]
    fn mcp_tool_becomes_mcp_resource() {
        let p = project("mcp__github__list_issues", &json!({}));
        assert_eq!(p.artifacts[0].kind.as_str(), "mcp_resource");
    }
}
