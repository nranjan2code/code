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

    pub fn targets(&self, tool: &str) -> bool {
        self.tool.eq_ignore_ascii_case(tool)
    }

    /// Existential match: true when ANY candidate string derived from the
    /// arguments matches. Correct for `Deny` and `Ask`, where seeing one
    /// dangerous effect is enough to gate the whole invocation.
    ///
    /// This is deliberately NOT the test for `Allow`; see
    /// [`crate::engine::PermissionEngine::evaluate`], which requires an
    /// allow rule to cover every effect the invocation can have.
    pub fn matches(&self, tool: &str, args: &Value) -> bool {
        if !self.targets(tool) {
            return false;
        }
        match &self.arg_glob {
            None => true,
            Some(gs) => arg_candidates(tool, args).iter().any(|c| gs.is_match(c)),
        }
    }

    fn matches_unit(&self, unit: &[String]) -> bool {
        match &self.arg_glob {
            None => true,
            Some(gs) => unit.iter().any(|c| gs.is_match(c)),
        }
    }
}

/// Extracts every string a restrictive (deny/ask) rule may match against.
/// Bash contributes the raw command plus each top-level segment; file tools
/// contribute their path; MCP calls contribute `server/tool`.
///
/// Restrictive matching is best-effort by design: the raw command is always
/// included so a `-Bash(*rm *)` rule still fires on a command whose structure
/// defeats segmentation.
pub fn arg_candidates(tool: &str, args: &Value) -> Vec<String> {
    match tool {
        "bash" => {
            let Some(cmd) = args.get("command").and_then(|c| c.as_str()) else {
                return Vec::new();
            };
            let mut out = vec![cmd.to_string()];
            if let Some(segments) = split_top_level(cmd) {
                for segment in segments {
                    out.extend(segment_candidates(&segment));
                }
            }
            out
        }
        "write" | "edit" | "read" | "doc_read" | "office_apply" | "glob" | "grep" => args
            .get("path")
            .and_then(|p| p.as_str())
            .map(|p| vec![p.to_string()])
            .unwrap_or_default(),
        "skill" => args
            .get("name")
            .and_then(|name| name.as_str())
            .map(|name| vec![name.to_string()])
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

/// The units an `Allow` rule set must cover in full before the invocation may
/// be allowed. Each unit is one independently-executable effect, represented
/// by the candidate strings that could match it.
///
/// `None` means the invocation's effects cannot be enumerated — an unbalanced
/// quote, command/process substitution, or a redirection to a real path. No
/// allow rule may then claim coverage, and the invocation falls through to
/// the mode default. Restrictive rules are unaffected: they match through
/// [`arg_candidates`], which always includes the raw command.
pub fn allow_coverage_units(tool: &str, args: &Value) -> Option<Vec<Vec<String>>> {
    if tool != "bash" {
        return Some(vec![arg_candidates(tool, args)]);
    }
    let cmd = args.get("command").and_then(|c| c.as_str())?;
    if is_opaque_command(cmd) {
        return None;
    }
    let segments = split_top_level(cmd)?;
    let mut units = Vec::with_capacity(segments.len());
    for segment in segments {
        if redirects_to_path(&segment) {
            return None;
        }
        let candidates = segment_candidates(&segment);
        if candidates.is_empty() {
            return None;
        }
        units.push(candidates);
    }
    if units.is_empty() { None } else { Some(units) }
}

/// True when the command's structure can hide an effect from segmentation:
/// command substitution or process substitution. Newlines are handled by
/// [`split_top_level`], which treats them as ordinary separators.
fn is_opaque_command(cmd: &str) -> bool {
    let mut quote = Quote::None;
    let mut chars = cmd.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if quote.consume(c, &mut chars) {
            continue;
        }
        // Shell substitutions remain active inside double quotes. They can
        // run arbitrary commands even though the outer command looks like a
        // single harmless printf/echo invocation.
        if quote == Quote::Double {
            match c {
                '`' => return true,
                '$' if cmd[i + 1..].starts_with('(') => return true,
                _ => {}
            }
            continue;
        }
        match c {
            '`' => return true,
            '$' if cmd[i + 1..].starts_with('(') => return true,
            '<' | '>' if cmd[i + 1..].starts_with('(') => return true,
            _ => {}
        }
    }
    false
}

/// True when the segment writes to or reads from a path through redirection.
/// File-descriptor duplication (`2>&1`, `&>`) and the two null devices carry
/// no filesystem effect and are exempt, because they appear in almost every
/// real command and gating them would make allow rules useless.
fn redirects_to_path(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    let mut quote = Quote::None;
    let mut chars = segment.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if quote.consume(c, &mut chars) {
            continue;
        }
        if quote != Quote::None || (c != '>' && c != '<') {
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && (bytes[j] == b'>' || bytes[j] == b'<') {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == b'&' {
            continue;
        }
        let target = segment[j..].trim_start();
        let target = target
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(['"', '\'']);
        if !matches!(target, "/dev/null" | "/dev/stdout" | "/dev/stderr") {
            return true;
        }
    }
    false
}

/// Splits a command on unquoted `;`, `&&`, `||`, `|`, `&`, and newlines.
/// Returns `None` when a quote is left unterminated, which makes the
/// command's structure unknowable.
///
/// Quote awareness is what makes universal allow-coverage usable: without it
/// `git commit -m "fix; ship"` splits into a bogus `ship"` segment that no
/// reasonable rule covers.
fn split_top_level(cmd: &str) -> Option<Vec<String>> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote = Quote::None;
    let mut chars = cmd.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if quote.consume(c, &mut chars) {
            current.push(c);
            continue;
        }
        // `2>&1` and `&>file` are redirections, not control operators: the
        // `&` binds to an adjacent angle bracket rather than separating two
        // commands. Splitting there would strand a bare `1` as its own
        // segment that no reasonable rule covers.
        let redirection_amp = c == '&'
            && (matches!(
                current.trim_end().chars().next_back(),
                Some('>') | Some('<')
            ) || matches!(chars.peek(), Some((_, '>'))));
        if quote == Quote::None && !redirection_amp && matches!(c, ';' | '|' | '&' | '\n' | '\r') {
            segments.push(std::mem::take(&mut current));
            continue;
        }
        current.push(c);
    }
    if quote != Quote::None {
        return None;
    }
    segments.push(current);
    let segments: Vec<String> = segments
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Some(segments)
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Quote {
    None,
    Single,
    Double,
}

impl Quote {
    /// Advances the quoting state for `c`. Returns true when the character was
    /// consumed as quoting syntax or as the escaped body of a backslash pair,
    /// meaning the caller must not interpret it structurally.
    fn consume(
        &mut self,
        c: char,
        chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    ) -> bool {
        match (*self, c) {
            (Quote::None, '\\') | (Quote::Double, '\\') => {
                chars.next();
                true
            }
            (Quote::None, '\'') => {
                *self = Quote::Single;
                true
            }
            (Quote::None, '"') => {
                *self = Quote::Double;
                true
            }
            (Quote::Single, '\'') | (Quote::Double, '"') => {
                *self = Quote::None;
                true
            }
            _ => false,
        }
    }
}

/// Candidate strings for one segment: the segment itself, its command name,
/// and `name *` so both `Bash(git)` and `Bash(git *)` express the same intent.
fn segment_candidates(segment: &str) -> Vec<String> {
    let stripped = strip_leading_env(segment);
    if stripped.is_empty() {
        return Vec::new();
    }
    let first_word_end = stripped.find(char::is_whitespace).unwrap_or(stripped.len());
    let name = &stripped[..first_word_end];
    let mut out = vec![stripped.to_string()];
    if name != stripped {
        out.push(name.to_string());
    }
    out.push(format!("{name} *"));
    out
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

/// True when `rules` collectively allow every effect `args` can have.
///
/// A blanket allow rule (no pattern) covers the whole invocation. Otherwise
/// every unit from [`allow_coverage_units`] must be matched by some allow
/// rule; one covered segment never authorizes its neighbours.
pub(crate) fn allow_covers(rules: &[Rule], tool: &str, args: &Value) -> bool {
    let allows: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.decision == RuleDecision::Allow && r.targets(tool))
        .collect();
    if allows.is_empty() {
        return false;
    }
    if allows.iter().any(|r| r.arg_glob.is_none()) {
        return true;
    }
    let Some(units) = allow_coverage_units(tool, args) else {
        return false;
    };
    !units.is_empty()
        && units
            .iter()
            .all(|unit| !unit.is_empty() && allows.iter().any(|r| r.matches_unit(unit)))
}

#[cfg(test)]
mod opaque_substitution_tests {
    use super::*;

    #[test]
    fn quoted_command_substitutions_are_not_covered_by_outer_allow() {
        let args = serde_json::json!({
            "command": "printf '%s' \"$(touch hidden-effect)\""
        });
        let rule = Rule::parse("+Bash(printf *)").expect("valid allow rule");
        assert!(!allow_covers(&[rule], "bash", &args));
    }

    #[test]
    fn quoted_backtick_substitutions_are_not_covered_by_outer_allow() {
        let args = serde_json::json!({
            "command": "printf '%s' \"`touch hidden-effect`\""
        });
        let rule = Rule::parse("+Bash(printf *)").expect("valid allow rule");
        assert!(!allow_covers(&[rule], "bash", &args));
    }
}
