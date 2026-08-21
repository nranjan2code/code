use globset::{Glob, GlobSet};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleDecision {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, thiserror::Error)]
pub enum RuleError {
    #[error("invalid rule '{0}': expected Tool(pattern) or Tool")]
    Invalid(String),
    #[error("invalid glob in rule '{rule}': {source}")]
    Glob {
        rule: String,
        source: globset::Error,
    },
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub tool: String,
    pub arg_glob: Option<GlobSet>,
    pub decision: RuleDecision,
}

impl Rule {
    /// Syntax: `Tool` or `Tool(pattern)` with optional `+`/`-`/`?` prefix for
    /// allow/ask/deny. Bare rules default to allow. Examples:
    ///   `Bash(git *)`, `-Bash(rm *)`, `?Edit(src/**)`, `read`
    pub fn parse(input: &str) -> Result<Rule, RuleError> {
        let input = input.trim();
        let (decision, rest) = match input.chars().next() {
            Some('+') => (RuleDecision::Allow, &input[1..]),
            Some('-') => (RuleDecision::Deny, &input[1..]),
            Some('?') => (RuleDecision::Ask, &input[1..]),
            _ => (RuleDecision::Allow, input),
        };

        let open = rest.find('(');
        let close = rest.rfind(')');
        let (tool, pattern) = match (open, close) {
            (Some(o), Some(c)) if c > o => (rest[..o].trim(), Some(rest[o + 1..c].trim())),
            (None, None) => (rest.trim(), None),
            _ => return Err(RuleError::Invalid(input.to_string())),
        };
        if tool.is_empty()
            || !tool
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(RuleError::Invalid(input.to_string()));
        }

        let arg_glob = match pattern {
            None => None,
            Some(p) if p == "*" || p.is_empty() => None,
            Some(p) => {
                let glob = Glob::new(p).map_err(|source| RuleError::Glob {
                    rule: input.to_string(),
                    source,
                })?;
                let gs = globset::GlobSetBuilder::new()
                    .add(glob)
                    .build()
                    .map_err(|source| RuleError::Glob {
                        rule: input.to_string(),
                        source,
                    })?;
                Some(gs)
            }
        };

        Ok(Rule {
            tool: tool.to_string(),
            arg_glob,
            decision,
        })
    }

    pub fn matches(&self, tool: &str, args: &Value) -> bool {
        if !self.tool.eq_ignore_ascii_case(tool) {
            return false;
        }
        match &self.arg_glob {
            None => true,
            Some(gs) => arg_candidates(tool, args).iter().any(|c| gs.is_match(c)),
        }
    }
}

/// Extracts the strings a rule pattern can match against, per tool family.
/// Bash matches its command and each subcommand split on `&&`, `;`, `|`.
/// File tools match their path argument. MCP calls match `server/tool`.
///
/// Commands containing newlines or shell substitution (`$(...)`, backticks,
/// process substitution) deliberately produce NO candidates: pattern-based
/// allow rules cannot see inside them, so matching them would be a lie.
/// Blanket rules (no pattern) still apply; such commands otherwise fall
/// through to the mode default / approver, which can see the full command
/// via describe().
pub fn arg_candidates(tool: &str, args: &Value) -> Vec<String> {
    match tool {
        "bash" => {
            let mut out = Vec::new();
            if let Some(cmd) = args.get("command").and_then(|c| c.as_str()) {
                if is_opaque_command(cmd) {
                    return Vec::new();
                }
                out.push(cmd.to_string());
                for seg in split_shell(cmd) {
                    out.push(seg);
                }
            }
            out
        }
        "write" | "edit" | "read" | "glob" | "grep" => args
            .get("path")
            .and_then(|p| p.as_str())
            .map(|p| vec![p.to_string()])
            .unwrap_or_default(),
        "mcp" => match (
            args.get("action").and_then(|a| a.as_str()),
            args.get("server").and_then(|s| s.as_str()),
            args.get("tool").and_then(|t| t.as_str()),
        ) {
            (Some("call"), Some(server), Some(mcp_tool)) => {
                vec![format!("{server}/{mcp_tool}")]
            }
            _ => Vec::new(),
        },
        "task" => args
            .get("label")
            .and_then(|l| l.as_str())
            .map(|l| vec![l.to_string()])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// True when the command's structure can hide a second command from naive
/// splitting: line continuations/newlines, command substitution, process
/// substitution.
fn is_opaque_command(cmd: &str) -> bool {
    cmd.contains('\n')
        || cmd.contains('\r')
        || cmd.contains("$(")
        || cmd.contains('`')
        || cmd.contains("<(")
        || cmd.contains(">(")
}

fn split_shell(cmd: &str) -> Vec<String> {
    cmd.split(['&', ';', '|'])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .flat_map(|seg| {
            let stripped = strip_leading_env(seg);
            let first_word_end = stripped.find(char::is_whitespace).unwrap_or(stripped.len());
            let name = &stripped[..first_word_end];
            let mut v = vec![stripped.to_string()];
            if name != stripped.trim() {
                v.push(name.to_string());
                v.push(format!("{name} *"));
            } else {
                v.push(format!("{name} *"));
            }
            v
        })
        .collect()
}

fn strip_leading_env(s: &str) -> &str {
    let mut cur = s.trim();
    while let Some(sp) = cur.find(char::is_whitespace) {
        let first = &cur[..sp];
        match first.find('=') {
            Some(eq) if eq > 0 && valid_name(&first[..eq]) => {
                cur = cur[sp + 1..].trim_start();
            }
            _ => break,
        }
    }
    cur
}

fn valid_name(w: &str) -> bool {
    !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
