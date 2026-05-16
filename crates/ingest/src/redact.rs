//! Streaming redaction. Two layers: a regex pack for well-known secret
//! shapes, plus a Shannon-entropy scan for long high-entropy tokens that
//! look like opaque secrets. Redaction is destructive — the original byte
//! never enters the SQLite store. The matching rule is recorded so audits
//! can know what kind of secret was removed.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub rule: &'static str,
    pub field_path: String,
}

struct RegexRule {
    name: &'static str,
    pat: Regex,
}

static RULES: Lazy<Vec<RegexRule>> = Lazy::new(|| {
    vec![
        RegexRule {
            name: "aws_access_key",
            pat: Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
        },
        RegexRule {
            name: "github_token",
            pat: Regex::new(r"gh[pousr]_[A-Za-z0-9]{36,}").unwrap(),
        },
        RegexRule {
            name: "bearer_token",
            pat: Regex::new(r"(?i)bearer\s+[A-Za-z0-9._\-+/=]{20,}").unwrap(),
        },
        RegexRule {
            name: "jwt",
            pat: Regex::new(r"eyJ[A-Za-z0-9_\-]{10,}\.eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}")
                .unwrap(),
        },
        RegexRule {
            name: "anthropic_key",
            pat: Regex::new(r"sk-ant-[A-Za-z0-9\-_]{20,}").unwrap(),
        },
        RegexRule {
            name: "openai_key",
            pat: Regex::new(r"sk-[A-Za-z0-9]{32,}").unwrap(),
        },
        RegexRule {
            name: "google_api_key",
            pat: Regex::new(r"AIza[0-9A-Za-z\-_]{35}").unwrap(),
        },
    ]
});

/// Redact a string in place. Returns the list of rule hits, with
/// `field_path` left empty for the caller to fill in (it knows context).
pub fn redact_string(s: &mut String) -> Vec<Hit> {
    let mut hits = Vec::new();
    for rule in RULES.iter() {
        if rule.pat.is_match(s) {
            let replaced = rule.pat.replace_all(s, format!("«REDACTED:{}»", rule.name));
            *s = replaced.into_owned();
            hits.push(Hit {
                rule: rule.name,
                field_path: String::new(),
            });
        }
    }
    // Entropy scan after regex so we don't double-redact our own marker.
    if let Some(rule) = scan_entropy(s) {
        let stripped = strip_high_entropy(s);
        if stripped != *s {
            *s = stripped;
            hits.push(Hit {
                rule,
                field_path: String::new(),
            });
        }
    }
    hits
}

/// Walk a serde_json::Value tree, redacting strings. Returns hits with
/// dotted field paths so the redactions table reflects what was touched.
pub fn redact_value(v: &mut Value, base_path: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    redact_value_inner(v, base_path, &mut hits);
    hits
}

fn redact_value_inner(v: &mut Value, path: &str, hits: &mut Vec<Hit>) {
    match v {
        Value::String(s) => {
            let mut owned = std::mem::take(s);
            let mut local = redact_string(&mut owned);
            for h in local.iter_mut() {
                h.field_path = path.to_string();
            }
            hits.extend(local);
            *s = owned;
        }
        Value::Array(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                let child = format!("{path}[{i}]");
                redact_value_inner(item, &child, hits);
            }
        }
        Value::Object(map) => {
            for (k, item) in map.iter_mut() {
                let child = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                redact_value_inner(item, &child, hits);
            }
        }
        _ => {}
    }
}

/// Coarse high-entropy detection. We don't try to be clever — just
/// strings ≥ MIN_LEN with Shannon entropy ≥ MIN_BITS that aren't
/// hex-only path-like content.
const MIN_LEN: usize = 32;
const MIN_BITS: f64 = 4.5;

fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    let mut total = 0u32;
    for &b in s.as_bytes() {
        counts[b as usize] += 1;
        total += 1;
    }
    let mut h = 0.0;
    for c in counts.iter() {
        if *c == 0 {
            continue;
        }
        let p = *c as f64 / total as f64;
        h -= p * p.log2();
    }
    h
}

fn is_delim(b: u8) -> bool {
    // ASCII-only set; byte positions of delimiters are always char boundaries.
    b.is_ascii_whitespace()
        || matches!(
            b,
            b'"' | b'\''
                | b','
                | b';'
                | b'('
                | b')'
                | b'{'
                | b'}'
                | b'['
                | b']'
                | b'<'
                | b'>'
                | b'`'
        )
}

fn token_is_secret_shaped(tok: &str) -> bool {
    // Cheap filters to avoid false positives on natural text.
    if tok.len() < MIN_LEN {
        return false;
    }
    if !tok.is_ascii() {
        return false;
    } // Chinese, accented Latin, etc. → not a secret token.
    if looks_like_path(tok) {
        return false;
    }
    if tok.contains('.') && tok.split('.').count() > 3 {
        return false;
    } // probably a filename or version.
      // Require a mix of letters + digits to look opaque.
    let has_alpha = tok.bytes().any(|b| b.is_ascii_alphabetic());
    let has_digit = tok.bytes().any(|b| b.is_ascii_digit());
    if !(has_alpha && has_digit) {
        return false;
    }
    shannon_entropy(tok) >= MIN_BITS
}

fn looks_like_path(s: &str) -> bool {
    s.contains('/') || s.contains('\\') || s.starts_with("http")
}

fn scan_entropy(s: &str) -> Option<&'static str> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if is_delim(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && !is_delim(bytes[i]) {
            i += 1;
        }
        if token_is_secret_shaped(&s[start..i]) {
            return Some("high_entropy");
        }
    }
    None
}

fn strip_high_entropy(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if is_delim(bytes[i]) {
            // Safe: ASCII byte is a one-byte UTF-8 codepoint.
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && !is_delim(bytes[i]) {
            i += 1;
        }
        // start and i are both at delimiter byte positions (or 0/len) which
        // are guaranteed char boundaries because delimiters are ASCII.
        let tok = &s[start..i];
        if token_is_secret_shaped(tok) {
            out.push_str("«REDACTED:high_entropy»");
        } else {
            out.push_str(tok);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_aws_key() {
        let mut s = String::from("export AWS_KEY=AKIAIOSFODNN7EXAMPLE rest");
        let hits = redact_string(&mut s);
        assert!(hits.iter().any(|h| h.rule == "aws_access_key"));
        assert!(s.contains("«REDACTED:aws_access_key»"));
        assert!(!s.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn redacts_github_token() {
        let mut s = String::from("token: ghp_abcdefghijklmnopqrstuvwxyz0123456789AA");
        let hits = redact_string(&mut s);
        assert!(hits.iter().any(|h| h.rule == "github_token"));
        assert!(!s.contains("ghp_abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn redacts_value_tree() {
        let mut v: Value = serde_json::json!({
            "command": "curl -H 'Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123' https://api",
            "nested": { "key": "ghp_abcdefghijklmnopqrstuvwxyz0123456789AA" },
        });
        let hits = redact_value(&mut v, "");
        assert!(hits.iter().any(|h| h.field_path == "command"));
        assert!(hits.iter().any(|h| h.field_path == "nested.key"));
    }

    #[test]
    fn leaves_normal_text_alone() {
        let mut s = String::from("this is just a normal sentence with words");
        let hits = redact_string(&mut s);
        assert!(hits.is_empty());
        assert_eq!(s, "this is just a normal sentence with words");
    }
}
