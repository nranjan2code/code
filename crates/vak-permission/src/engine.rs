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
}

const READ_TOOLS: [&str; 7] = [
    "read",
    "glob",
    "grep",
    "ls",
    "search",
    "session_search",
    "skill",
];
const PATH_SCOPED_READ_TOOLS: [&str; 4] = ["read", "glob", "grep", "ls"];
const WRITE_TOOLS: [&str; 2] = ["write", "edit"];
/// Learning-loop journaling into vak's own per-workspace store
/// (docs/design/26-learning.md): sanctioned under workspace-write, still
/// denied by read-only's default arm below.
const LEARNING_TOOLS: [&str; 2] = ["remember", "propose_skill"];

impl PermissionEngine {
    pub fn new(rules: Vec<Rule>) -> Self {
        PermissionEngine {
            rules,
            write_scope: None,
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
        })
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
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
        if crate::rules::allow_covers(&self.rules, tool, args) {
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
        "write" | "edit" | "read" => args
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
