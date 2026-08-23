use serde_json::Value;

use crate::Mode;
use crate::rules::{Rule, RuleDecision};

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}

#[derive(Debug, Clone, Default)]
pub struct PermissionEngine {
    rules: Vec<Rule>,
}

const READ_TOOLS: [&str; 5] = ["read", "glob", "grep", "ls", "search"];
const PATH_SCOPED_READ_TOOLS: [&str; 4] = ["read", "glob", "grep", "ls"];
const WRITE_TOOLS: [&str; 2] = ["write", "edit"];

impl PermissionEngine {
    pub fn new(rules: Vec<Rule>) -> Self {
        PermissionEngine { rules }
    }

    pub fn from_rule_strings(specs: &[String]) -> Result<Self, crate::rules::RuleError> {
        let mut rules = Vec::with_capacity(specs.len());
        for s in specs {
            rules.push(Rule::parse(s)?);
        }
        Ok(PermissionEngine { rules })
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Severity aggregation over matching rules — Deny > Ask > Allow no
    /// matter what order the rules were registered in. A deny rule can
    /// never be shadowed by an allow rule purely because of vec ordering.
    /// With no matching rule, the mode's default applies.
    pub fn evaluate(
        &self,
        tool: &str,
        args: &Value,
        mode: Mode,
        cwd: &std::path::Path,
    ) -> Decision {
        let mut best: Option<(u8, Decision)> = None;
        for rule in &self.rules {
            if !rule.matches(tool, args) {
                continue;
            }
            let severity = match rule.decision {
                RuleDecision::Deny => 2u8,
                RuleDecision::Ask => 1,
                RuleDecision::Allow => 0,
            };
            let better = match &best {
                Some((s, _)) => severity > *s,
                None => true,
            };
            if better {
                let decision = match rule.decision {
                    RuleDecision::Allow => Decision::Allow,
                    RuleDecision::Ask => Decision::Ask {
                        reason: format!("rule requires approval: {}", describe(tool, args)),
                    },
                    RuleDecision::Deny => Decision::Deny {
                        reason: format!("denied by rule: {}", describe(tool, args)),
                    },
                };
                best = Some((severity, decision));
            }
        }
        if let Some((_, decision)) = best {
            return decision;
        }

        match mode {
            Mode::FullAccess => Decision::Allow,
            Mode::ReadOnly => {
                if READ_TOOLS.contains(&tool) {
                    scoped_read_decision(tool, args, cwd)
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
                if WRITE_TOOLS.contains(&tool) {
                    return match args.get("path").and_then(|p| p.as_str()) {
                        Some(path) => {
                            if path_in_workspace(std::path::Path::new(path), cwd) {
                                Decision::Allow
                            } else {
                                Decision::Ask {
                                    reason: format!("'{path}' is outside the workspace"),
                                }
                            }
                        }
                        None => Decision::Ask {
                            reason: "missing path argument".into(),
                        },
                    };
                }
                if tool == "bash" {
                    return Decision::Ask {
                        reason: format!("shell command needs approval: {}", describe(tool, args)),
                    };
                }
                Decision::Ask {
                    reason: format!("'{}' needs approval: {}", tool, describe(tool, args)),
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

fn path_in_workspace(path: &std::path::Path, cwd: &std::path::Path) -> bool {
    let Ok(cwd_abs) = cwd.canonicalize() else {
        return false;
    };
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd_abs.join(path)
    };
    match candidate.canonicalize() {
        Ok(p) => p.starts_with(&cwd_abs),
        Err(_) => {
            let mut acc = cwd_abs.clone();
            for comp in candidate
                .strip_prefix(&cwd_abs)
                .unwrap_or(candidate.components().as_path())
                .components()
            {
                match comp {
                    std::path::Component::ParentDir => {
                        if !acc.pop() {
                            return false;
                        }
                    }
                    std::path::Component::Normal(c) => acc.push(c),
                    std::path::Component::CurDir => {}
                    _ => return false,
                }
            }
            acc.starts_with(&cwd_abs)
        }
    }
}
