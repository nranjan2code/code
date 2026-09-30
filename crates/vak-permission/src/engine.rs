use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::Mode;
use crate::rules::{Rule, RuleDecision};

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Allow,
    Ask { reason: String, source: AskSource },
    Deny { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskSource {
    Rule,
    Scope,
    ModeDefault,
    CircuitBreaker,
}

#[derive(Debug, Clone, Default)]
pub struct PermissionEngine {
    rules: Vec<Rule>,
    write_scope: Option<Vec<PathBuf>>,
    /// Host-imposed execution vocabulary. Checked before user allow rules.
    allowed_tools: Option<Vec<String>>,
    /// Tools that only display something to the user (`Tool::presents_cards`).
    /// Supplied by the host from the tools' own declarations, never inferred
    /// from a name here.
    presenting: Vec<String>,
}

/// Tools that read and change nothing. `commitments` reads the Agent's own
/// portfolio, like `session_search` reads its own history: a model that must
/// ask a person before it may look at its own obligations cannot answer
/// "what are you working on?" on an unattended surface at all.
const READ_TOOLS: [&str; 10] = [
    "read",
    "doc_read",
    "glob",
    "grep",
    "ls",
    "search",
    "session_search",
    "skill",
    "commitments",
    "mail_calendar",
];
const PATH_SCOPED_READ_TOOLS: [&str; 5] = ["read", "doc_read", "glob", "grep", "ls"];
const WRITE_TOOLS: [&str; 3] = ["write", "edit", "office_apply"];
/// Learning-loop journaling into vak's own per-workspace store
/// (docs/design/26-learning.md): sanctioned under workspace-write, still
/// denied by read-only's default arm below.
const LEARNING_TOOLS: [&str; 2] = ["remember", "propose_skill"];
/// Tools whose reach exceeds the workspace (docs/design/29-personal-os.md
/// P4). Outside FullAccess they gate on approval rather than following the
/// surrounding mode's arm: read-only would otherwise deny them outright,
/// and the intent is "a human may still say yes", not "never".
///
/// This lives HERE, in the mode arms, rather than being injected as a
/// synthetic `?webfetch` rule by the engine's caller. An injected rule is
/// indistinguishable from one an operator typed, and `auto_approve`
/// deliberately refuses to resolve rule-sourced asks on the model's behalf
/// — so the injection silently made `approval_mode = "auto-approve"` a
/// no-op for exactly these two tools while working for every other one.
/// A mode default must be sourced as a mode default.
const NETWORK_TOOLS: [&str; 5] = [
    "webfetch",
    "browse",
    "mail_calendar_send",
    "mail_calendar_event_create",
    "mail_calendar_event_update",
];

impl PermissionEngine {
    pub fn new(rules: Vec<Rule>) -> Self {
        PermissionEngine {
            rules,
            write_scope: None,
            allowed_tools: None,
            presenting: Vec::new(),
        }
    }

    pub fn from_rule_strings(specs: &[String]) -> Result<Self, crate::rules::RuleError> {
        let mut rules = Vec::with_capacity(specs.len());
        for s in specs {
            rules.push(Rule::parse(s)?);
        }
        Ok(PermissionEngine {
            rules,
            write_scope: None,
            allowed_tools: None,
            presenting: Vec::new(),
        })
    }

    /// Declare the tools that present cards. They show the user something
    /// Vak already holds and reach nothing outside the conversation, so every
    /// mode lets them through (read-only included) with no approval. An
    /// operator's explicit Deny or Ask rule still wins.
    pub fn with_presenting_tools(mut self, names: impl IntoIterator<Item = String>) -> Self {
        self.presenting = names.into_iter().collect();
        self
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// True when some rule targets `tool` with an argument pattern.
    ///
    /// Reachability preflight (`vak_core::reach`) probes a capability
    /// before its arguments exist, so a patterned rule cannot be evaluated
    /// yet. Its existence means the probe's answer is provisional, and the
    /// preflight degrades to "gated" rather than reporting a capability as
    /// unreachable on evidence it does not have. Hiding a capability that
    /// would in fact have worked is the one outcome worse than advertising
    /// one that gates.
    pub fn has_patterned_rule(&self, tool: &str) -> bool {
        self.rules
            .iter()
            .any(|rule| rule.targets(tool) && rule.arg_glob.is_some())
    }

    /// Restrict direct file mutations to paths explicitly supplied by the
    /// hosting surface. This is an execution contract, not a model prompt;
    /// it is evaluated before rules and permission mode.
    pub fn restrict_write_paths(mut self, cwd: &Path, paths: &[PathBuf]) -> Self {
        self.write_scope = Some(
            paths
                .iter()
                .map(|path| normalize_scope_path(path, cwd))
                .collect(),
        );
        self
    }

    /// Narrow an isolated run to these tool names. No configured allow rule
    /// or broad permission mode can re-enable a tool outside this set.
    pub fn restrict_tools(mut self, names: &[&str]) -> Self {
        self.allowed_tools = Some(names.iter().map(|name| name.to_ascii_lowercase()).collect());
        self
    }

    /// Restrictive rules first — Deny beats Ask no matter what order they
    /// were registered in — then allow-coverage, then the mode default.
    ///
    /// `Allow` is universally quantified: the allow rules must cover EVERY
    /// effect the invocation can have. One matching segment of a compound
    /// shell command never authorizes its neighbours, so `+Bash(git *)`
    /// does not allow `git status; rm -rf /`.
    pub fn evaluate(
        &self,
        tool: &str,
        args: &Value,
        mode: Mode,
        cwd: &std::path::Path,
    ) -> Decision {
        if self
            .allowed_tools
            .as_ref()
            .is_some_and(|allowed| !allowed.iter().any(|name| name.eq_ignore_ascii_case(tool)))
        {
            return Decision::Deny {
                reason: format!("'{tool}' is outside this run's tool scope"),
            };
        }
        if WRITE_TOOLS.contains(&tool)
            && let Some(scope) = &self.write_scope
        {
            let Some(path) = args.get("path").and_then(|value| value.as_str()) else {
                return Decision::Deny {
                    reason: "write scope requires a path argument".into(),
                };
            };
            let candidate = normalize_scope_path(Path::new(path), cwd);
            if !scope.contains(&candidate) {
                return Decision::Deny {
                    reason: format!("'{path}' is outside this run's declared write scope"),
                };
            }
        }
        if self
            .rules
            .iter()
            .any(|rule| rule.decision == RuleDecision::Deny && rule.matches(tool, args))
        {
            return Decision::Deny {
                reason: format!("denied by rule: {}", describe(tool, args)),
            };
        }
        if self
            .rules
            .iter()
            .any(|rule| rule.decision == RuleDecision::Ask && rule.matches(tool, args))
        {
            return Decision::Ask {
                reason: format!("rule requires approval: {}", describe(tool, args)),
                source: AskSource::Rule,
            };
        }
        if crate::rules::allow_covers(&self.rules, tool, args)
            || self.presenting.iter().any(|name| name == tool)
        {
            return Decision::Allow;
        }

        match mode {
            Mode::FullAccess => Decision::Allow,
            Mode::ReadOnly => {
                if READ_TOOLS.contains(&tool) {
                    scoped_read_decision(tool, args, cwd)
                } else if tool == "mcp"
                    && args.get("action").and_then(|a| a.as_str()) == Some("list")
                {
                    Decision::Allow
                } else if NETWORK_TOOLS.contains(&tool) {
                    Decision::Ask {
                        reason: format!("'{tool}' reaches outside the workspace"),
                        source: AskSource::ModeDefault,
                    }
                } else {
                    Decision::Deny {
                        reason: format!(
                            "read-only mode denies '{tool}'; switch modes or add a rule"
                        ),
                    }
                }
            }
            Mode::WorkspaceWrite => {
                if READ_TOOLS.contains(&tool) {
                    return scoped_read_decision(tool, args, cwd);
                }
                if tool == "mcp" && args.get("action").and_then(|a| a.as_str()) == Some("list") {
                    return Decision::Allow;
                }
                if WRITE_TOOLS.contains(&tool) {
                    return match args.get("path").and_then(|p| p.as_str()) {
                        Some(path) => {
                            if path_in_workspace(std::path::Path::new(path), cwd) {
                                Decision::Allow
                            } else {
                                Decision::Ask {
                                    reason: format!("'{path}' is outside the workspace"),
                                    source: AskSource::Scope,
                                }
                            }
                        }
                        None => Decision::Ask {
                            reason: "missing path argument".into(),
                            source: AskSource::ModeDefault,
                        },
                    };
                }
                if LEARNING_TOOLS.contains(&tool) {
                    return Decision::Allow;
                }
                if tool == "bash" {
                    return Decision::Ask {
                        reason: format!("shell command needs approval: {}", describe(tool, args)),
                        source: AskSource::ModeDefault,
                    };
                }
                Decision::Ask {
                    reason: format!("'{}' needs approval: {}", tool, describe(tool, args)),
                    source: AskSource::ModeDefault,
                }
            }
        }
    }
}

fn scoped_read_decision(tool: &str, args: &Value, cwd: &std::path::Path) -> Decision {
    if !PATH_SCOPED_READ_TOOLS.contains(&tool) {
        return Decision::Allow;
    }
    let path = args.get("path").and_then(|p| p.as_str());
    if tool == "read" && path.is_none() {
        return Decision::Deny {
            reason: "read requires a workspace-scoped path".into(),
        };
    }
    match path {
        Some(path) if !path_in_workspace(std::path::Path::new(path), cwd) => Decision::Deny {
            reason: format!(
                "'{path}' is outside the workspace; use full-access or an explicit scoped rule"
            ),
        },
        _ => Decision::Allow,
    }
}

fn describe(tool: &str, args: &Value) -> String {
    match tool {
        "bash" => args
            .get("command")
            .and_then(|c| c.as_str())
            .map(|c| {
                let preview: String = c.chars().take(80).collect();
                format!("bash `{preview}`")
            })
            .unwrap_or_else(|| "bash".into()),
        "write" | "edit" | "read" | "doc_read" | "office_apply" => args
            .get("path")
            .and_then(|p| p.as_str())
            .map(|p| format!("{tool} {p}"))
            .unwrap_or_else(|| tool.to_string()),
        "mcp" => match (
            args.get("action").and_then(|a| a.as_str()),
            args.get("server").and_then(|s| s.as_str()),
            args.get("tool").and_then(|t| t.as_str()),
        ) {
            (Some("call"), Some(server), Some(mcp_tool)) => {
                format!("mcp call {server}/{mcp_tool}")
            }
            (Some("call"), Some(server), None) => format!("mcp call {server}/<missing tool>"),
            (Some("list"), _, _) => "mcp list".to_string(),
            _ => "mcp".to_string(),
        },
        "task" => {
            let label = args
                .get("label")
                .and_then(|l| l.as_str())
                .map(String::from)
                .unwrap_or_else(|| {
                    let prompt = args.get("prompt").and_then(|p| p.as_str()).unwrap_or("");
                    prompt.chars().take(60).collect()
                });
            format!("task '{label}'")
        }
        other => other.to_string(),
    }
}

pub fn path_in_workspace(path: &std::path::Path, cwd: &std::path::Path) -> bool {
    let Ok(cwd_abs) = cwd.canonicalize() else {
        return false;
    };
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd_abs.join(path)
    };
    match resolve_through_existing(&candidate) {
        Some(resolved) => resolved.starts_with(&cwd_abs),
        None => false,
    }
}

/// Canonicalizes the longest existing prefix of `path`, then re-applies the
/// components that do not exist yet.
///
/// `Path::canonicalize` fails outright when the leaf has not been created —
/// the normal case for `write`. Resolving purely lexically instead (what this
/// used to do) cannot see a symlinked ancestor, so a workspace containing
/// `link -> /etc` accepted a write to `link/passwd` as in-workspace.
///
/// Returns `None` when the path cannot be resolved at all; callers treat that
/// as "outside", so this fails closed.
fn resolve_through_existing(path: &Path) -> Option<PathBuf> {
    let mut existing = path.to_path_buf();
    let mut pending: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(base) = existing.canonicalize() {
            let mut out = base;
            for part in pending.iter().rev() {
                if part == ".." {
                    if !out.pop() {
                        return None;
                    }
                } else if part != "." {
                    out.push(part);
                }
            }
            return Some(out);
        }
        pending.push(existing.file_name()?.to_os_string());
        if !existing.pop() {
            return None;
        }
    }
}

fn normalize_scope_path(path: &Path, cwd: &Path) -> PathBuf {
    let base = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    resolve_through_existing(&candidate).unwrap_or(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_calendar_is_a_read_capability_but_explicit_rules_still_win() {
        let empty = PermissionEngine::default();
        assert_eq!(
            empty.evaluate(
                "mail_calendar",
                &serde_json::json!({}),
                Mode::ReadOnly,
                Path::new("/workspace")
            ),
            Decision::Allow
        );
        let denied =
            PermissionEngine::from_rule_strings(&["-mail_calendar".into()]).expect("valid rule");
        assert!(matches!(
            denied.evaluate(
                "mail_calendar",
                &serde_json::json!({}),
                Mode::ReadOnly,
                Path::new("/workspace")
            ),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn mail_calendar_send_requires_approval_in_read_only_and_workspace_write() {
        let engine = PermissionEngine::default();
        for mode in [Mode::ReadOnly, Mode::WorkspaceWrite] {
            assert!(matches!(
                engine.evaluate(
                    "mail_calendar_send",
                    &serde_json::json!({"candidate_id": "candidate-v7"}),
                    mode,
                    Path::new("/workspace")
                ),
                Decision::Ask { .. }
            ));
        }
        let denied = PermissionEngine::from_rule_strings(&["-mail_calendar_send".into()])
            .expect("valid rule");
        assert!(matches!(
            denied.evaluate(
                "mail_calendar_send",
                &serde_json::json!({"candidate_id": "candidate-v7"}),
                Mode::FullAccess,
                Path::new("/workspace")
            ),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn mail_calendar_event_create_requires_approval_and_explicit_deny_wins() {
        let engine = PermissionEngine::default();
        for mode in [Mode::ReadOnly, Mode::WorkspaceWrite] {
            assert!(matches!(
                engine.evaluate(
                    "mail_calendar_event_create",
                    &serde_json::json!({"candidate_id":"candidate-v7"}),
                    mode,
                    Path::new("/workspace")
                ),
                Decision::Ask { .. }
            ));
            assert!(matches!(
                engine.evaluate(
                    "mail_calendar_event_update",
                    &serde_json::json!({"candidate_id":"candidate-v7"}),
                    mode,
                    Path::new("/workspace")
                ),
                Decision::Ask { .. }
            ));
        }
        let denied = PermissionEngine::from_rule_strings(&["-mail_calendar_event_create".into()])
            .expect("valid rule");
        assert!(matches!(
            denied.evaluate(
                "mail_calendar_event_create",
                &serde_json::json!({}),
                Mode::FullAccess,
                Path::new("/workspace")
            ),
            Decision::Deny { .. }
        ));
        let denied_update =
            PermissionEngine::from_rule_strings(&["-mail_calendar_event_update".into()])
                .expect("valid rule");
        assert!(matches!(
            denied_update.evaluate(
                "mail_calendar_event_update",
                &serde_json::json!({}),
                Mode::FullAccess,
                Path::new("/workspace")
            ),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn test_describe_formats_task() {
        let task_args = serde_json::json!({
            "label": "build project"
        });
        assert_eq!(describe("task", &task_args), "task 'build project'");
    }
}
