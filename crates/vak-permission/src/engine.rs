use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::Mode;
use crate::reach::{Declared, Reach, Subject};
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
    /// The reaches an isolated run may use at all, imposed by its host.
    /// Checked before every rule and mode.
    allowed_reach: Option<Vec<Reach>>,
    /// What each tool declared it reaches and what a rule about it matches
    /// (`Tool::permission`), supplied by the host from the tools it offers.
    /// A tool not here is `Declared::default()`: an unknown.
    declared: HashMap<String, Declared>,
}

impl PermissionEngine {
    pub fn new(rules: Vec<Rule>) -> Self {
        PermissionEngine {
            rules,
            write_scope: None,
            allowed_reach: None,
            declared: HashMap::new(),
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
            allowed_reach: None,
            declared: HashMap::new(),
        })
    }

    /// What the offered tools declared (`Tool::permission`). Added to what
    /// was declared before, a later declaration of a name replacing an
    /// earlier one.
    pub fn with_tools(mut self, declared: impl IntoIterator<Item = (String, Declared)>) -> Self {
        self.declared.extend(declared);
        self
    }

    /// What `tool` declared, or an unknown's nothing.
    pub fn declared(&self, tool: &str) -> Declared {
        self.declared.get(tool).copied().unwrap_or_default()
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

    /// Narrow an isolated run to tools of these reaches. No configured allow
    /// rule or broad permission mode can re-enable a tool outside them.
    pub fn restrict_reach(mut self, reaches: &[Reach]) -> Self {
        self.allowed_reach = Some(reaches.to_vec());
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
        self.evaluate_as(tool, self.declared(tool), args, mode, cwd)
    }

    /// `evaluate` with the declaration of the tool the call names, as the
    /// caller holds it: a run's own tools may differ from those the engine
    /// was told of when it was built.
    pub fn evaluate_as(
        &self,
        tool: &str,
        declared: Declared,
        args: &Value,
        mode: Mode,
        cwd: &std::path::Path,
    ) -> Decision {
        if self
            .allowed_reach
            .as_ref()
            .is_some_and(|allowed| !allowed.contains(&declared.reach))
        {
            return Decision::Deny {
                reason: format!("'{tool}' is outside this run's tool scope"),
            };
        }
        if declared.reach == Reach::WritePath
            && let Some(scope) = &self.write_scope
        {
            let Some(path) = path_argument(declared.subject, args) else {
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
        if self.rules.iter().any(|rule| {
            rule.decision == RuleDecision::Deny && rule.matches(tool, declared.subject, args)
        }) {
            return Decision::Deny {
                reason: format!("denied by rule: {}", describe(tool, declared.subject, args)),
            };
        }
        if self.rules.iter().any(|rule| {
            rule.decision == RuleDecision::Ask && rule.matches(tool, declared.subject, args)
        }) {
            return Decision::Ask {
                reason: format!(
                    "rule requires approval: {}",
                    describe(tool, declared.subject, args)
                ),
                source: AskSource::Rule,
            };
        }
        // A display or coordination tool reaches nothing, so every mode lets
        // it through after the operator's deny and ask rules: gating a
        // question on an approval would make the question impossible to ask.
        if crate::rules::allow_covers(&self.rules, tool, declared.subject, args)
            || matches!(declared.reach, Reach::Display | Reach::Coordination)
        {
            return Decision::Allow;
        }
        let lists_catalogue = declared.reach == Reach::Integration
            && args.get("action").and_then(Value::as_str) == Some("list");

        match mode {
            Mode::FullAccess => Decision::Allow,
            Mode::ReadOnly => match declared.reach {
                Reach::ReadPath => scoped_read_decision(declared.subject, args, cwd),
                Reach::ReadStore => Decision::Allow,
                _ if lists_catalogue => Decision::Allow,
                Reach::Network => Decision::Ask {
                    reason: format!("'{tool}' reaches outside the workspace"),
                    source: AskSource::ModeDefault,
                },
                _ => Decision::Deny {
                    reason: format!("read-only mode denies '{tool}'; switch modes or add a rule"),
                },
            },
            Mode::WorkspaceWrite => match declared.reach {
                Reach::ReadPath => scoped_read_decision(declared.subject, args, cwd),
                Reach::ReadStore | Reach::Learning => Decision::Allow,
                _ if lists_catalogue => Decision::Allow,
                Reach::WritePath => match path_argument(declared.subject, args) {
                    Some(path) if path_in_workspace(Path::new(path), cwd) => Decision::Allow,
                    Some(path) => Decision::Ask {
                        reason: format!(
                            "'{path}' is outside the workspace, which is {}",
                            cwd.display()
                        ),
                        source: AskSource::Scope,
                    },
                    None => Decision::Ask {
                        reason: "missing path argument".into(),
                        source: AskSource::ModeDefault,
                    },
                },
                Reach::Command => Decision::Ask {
                    reason: format!(
                        "shell command needs approval: {}",
                        describe(tool, declared.subject, args)
                    ),
                    source: AskSource::ModeDefault,
                },
                _ => Decision::Ask {
                    reason: format!(
                        "'{}' needs approval: {}",
                        tool,
                        describe(tool, declared.subject, args)
                    ),
                    source: AskSource::ModeDefault,
                },
            },
        }
    }
}

/// The path a path-scoped tool's call names, by its declared argument.
fn path_argument(subject: Subject, args: &Value) -> Option<&str> {
    match subject {
        Subject::Path { argument, .. } => args.get(argument).and_then(Value::as_str),
        _ => None,
    }
}

fn scoped_read_decision(subject: Subject, args: &Value, cwd: &Path) -> Decision {
    let Subject::Path { argument, required } = subject else {
        return Decision::Allow;
    };
    let path = args.get(argument).and_then(Value::as_str);
    if required && path.is_none() {
        return Decision::Deny {
            reason: format!("a workspace-scoped `{argument}` is required"),
        };
    }
    match path {
        Some(path) if !path_in_workspace(Path::new(path), cwd) => Decision::Deny {
            reason: format!(
                "'{path}' is outside the workspace, which is {}; a file in it is named by its path relative to the workspace. Reaching outside it needs full-access or an explicit scoped rule",
                cwd.display()
            ),
        },
        _ => Decision::Allow,
    }
}

/// The call as a person reads it in an approval: the tool and what its
/// rule subject names.
fn describe(tool: &str, subject: Subject, args: &Value) -> String {
    match subject {
        Subject::Command(_) => crate::rules::arg_candidates(subject, args)
            .first()
            .map(|command| {
                let preview: String = command.chars().take(80).collect();
                format!("{tool} `{preview}`")
            })
            .unwrap_or_else(|| tool.to_string()),
        Subject::IntegrationCall => match args.get("action").and_then(Value::as_str) {
            Some("list") => format!("{tool} list"),
            _ => crate::rules::arg_candidates(subject, args)
                .first()
                .map(|call| format!("{tool} call {call}"))
                .unwrap_or_else(|| tool.to_string()),
        },
        _ => crate::rules::arg_candidates(subject, args)
            .first()
            .map(|value| format!("{tool} {value}"))
            .unwrap_or_else(|| tool.to_string()),
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::test_support as support;

    #[test]
    fn mail_calendar_is_a_read_capability_but_explicit_rules_still_win() {
        let empty = PermissionEngine::default().with_tools(support::built_in());
        assert_eq!(
            empty.evaluate(
                "mail_calendar",
                &serde_json::json!({}),
                Mode::ReadOnly,
                Path::new("/workspace")
            ),
            Decision::Allow
        );
        let denied = support::engine(&["-mail_calendar".into()]).expect("valid rule");
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
        let engine = PermissionEngine::default().with_tools(support::built_in());
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
        let denied = support::engine(&["-mail_calendar_send".into()]).expect("valid rule");
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
    fn mail_calendar_provider_reconciliation_requires_an_explicit_read_approval() {
        let engine = PermissionEngine::default().with_tools(support::built_in());
        for action in [
            "mail_calendar_mail_reconcile",
            "mail_calendar_event_reconcile",
        ] {
            for mode in [Mode::ReadOnly, Mode::WorkspaceWrite] {
                assert!(
                    matches!(
                        engine.evaluate(
                            action,
                            &serde_json::json!({"candidate_id":"candidate-v7"}),
                            mode,
                            Path::new("/workspace")
                        ),
                        Decision::Ask { .. }
                    ),
                    "{action} in {mode:?}"
                );
            }
            let denied = support::engine(&[format!("-{action}")]).expect("valid rule");
            assert!(matches!(
                denied.evaluate(
                    action,
                    &serde_json::json!({"candidate_id":"candidate-v7"}),
                    Mode::FullAccess,
                    Path::new("/workspace")
                ),
                Decision::Deny { .. }
            ));
        }
    }

    #[test]
    fn mail_calendar_event_create_requires_approval_and_explicit_deny_wins() {
        let engine = PermissionEngine::default().with_tools(support::built_in());
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
            assert!(matches!(
                engine.evaluate(
                    "mail_calendar_event_cancel",
                    &serde_json::json!({"candidate_id":"candidate-v7"}),
                    mode,
                    Path::new("/workspace")
                ),
                Decision::Ask { .. }
            ));
        }
        let denied = support::engine(&["-mail_calendar_event_create".into()]).expect("valid rule");
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
            support::engine(&["-mail_calendar_event_update".into()]).expect("valid rule");
        assert!(matches!(
            denied_update.evaluate(
                "mail_calendar_event_update",
                &serde_json::json!({}),
                Mode::FullAccess,
                Path::new("/workspace")
            ),
            Decision::Deny { .. }
        ));
        let denied_cancel =
            support::engine(&["-mail_calendar_event_cancel".into()]).expect("valid rule");
        assert!(matches!(
            denied_cancel.evaluate(
                "mail_calendar_event_cancel",
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
        assert_eq!(
            describe("task", Subject::Value("label"), &task_args),
            "task build project"
        );
    }

    #[test]
    fn coordination_tools_are_allowed_in_every_mode_but_a_deny_rule_still_wins() {
        let cwd = std::path::Path::new(".");
        let args = serde_json::json!({"question": "Which year?"});
        let open = PermissionEngine::new(Vec::new()).with_tools(support::built_in());
        for mode in [Mode::ReadOnly, Mode::WorkspaceWrite, Mode::FullAccess] {
            for tool in ["ask_parent", "workers"] {
                assert!(
                    matches!(open.evaluate(tool, &args, mode, cwd), Decision::Allow),
                    "{tool} in {mode:?}"
                );
            }
        }
        let denied = PermissionEngine::new(vec![Rule::parse("-ask_parent").unwrap()])
            .with_tools(support::built_in());
        assert!(matches!(
            denied.evaluate("ask_parent", &args, Mode::FullAccess, cwd),
            Decision::Deny { .. }
        ));
    }
}
