//! Reconciling *configured* capability with *reachable* capability.
//!
//! vak decides what a turn may do across a lot of independent layers:
//! permission mode (itself capped bot → chat → workspace), allow/ask/deny
//! rules, channel overlays, the frozen session contract, MCP server and
//! tool globs, the sandbox, approval mode, and finally whichever
//! `Approver` the hosting surface installed. Each layer is individually
//! correct and individually tested. Nothing composed them.
//!
//! The gap that opened is not a bug in any one layer — it is that the
//! system prompt advertised the *configured* set while dispatch enforced
//! the *composed* set, and the two were never compared. A Telegram turn
//! would be told "Configured MCP servers: search", spend three tool calls
//! discovering that every one of them is refused by an approver that was
//! never going to say yes, and then tell the user it had no way to search
//! the web. Everything worked exactly as designed and the outcome was a
//! lie.
//!
//! This module is the comparison. It runs the real [`PermissionEngine`]
//! against a probe of each advertised capability, folds in the approval
//! mode and whether this surface's approver can answer anything at all,
//! and returns one [`Standing`] per capability. Every surface reads it:
//! the prompt advertises only what is reachable and names what is not, the
//! tool registry drops what cannot be called, `doctor` reports the
//! mismatch, and the audit log records it.
//!
//! It grants nothing. A `Blocked` standing removes a capability from the
//! turn; it never adds one, and it never resolves a gate (AGENTS.md
//! invariant 15 — unattended surfaces fail closed, and they still do).
//! The change is that failing closed is now *stated up front* instead of
//! discovered one denied tool call at a time.

use std::path::Path;

use serde_json::json;

use vak_permission::{Decision, Mode, PermissionEngine};

/// What a capability's standing actually is once every layer has spoken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Callable. No gate, or a gate this turn's approval mode resolves.
    Open,
    /// Callable, but each use raises a gate somebody can answer.
    Gated,
    /// Configured, and not callable on this turn. Either a rule denies it
    /// outright, or it gates on an approval nobody here can answer.
    Blocked,
}

impl Reach {
    pub fn is_blocked(self) -> bool {
        self == Reach::Blocked
    }
}

/// One capability's standing, with enough detail to explain itself to a
/// model, an operator, and an audit log without any of them re-deriving it.
#[derive(Debug, Clone)]
pub struct Standing {
    /// The tool the model would actually call (`mcp`, `webfetch`, `browse`).
    pub tool: String,
    /// How the capability is named to a reader: `mcp server \`search\``.
    pub label: String,
    pub reach: Reach,
    /// The deciding layer's own words.
    pub reason: String,
    /// What an operator would change to make it reachable. Empty when it
    /// already is.
    pub remedy: String,
}

/// Everything the composed policy needs in order to answer "can this turn
/// actually use that?". Gathered by the caller because every field is
/// per-turn state: the engine and mode come from the channel-capped
/// resolution, the approver from the hosting surface.
pub struct Probe<'a> {
    pub engine: &'a PermissionEngine,
    pub mode: Mode,
    pub approval_mode: vak_agent::ApprovalMode,
    pub sandboxed: bool,
    pub cwd: &'a Path,
    /// `Approver::answerable()` for this turn's approver. `false` for an
    /// unattended surface; `None` when no approver is installed at all,
    /// which is also unanswerable.
    pub approver_answerable: bool,
    /// Configured MCP server names, after channel filtering.
    pub mcp_servers: &'a [String],
    /// Registered network tools, after channel filtering.
    pub network_tools: &'a [String],
    /// Discovered skills, after channel filtering.
    pub skills: &'a [String],
}

/// Compute one standing per advertised capability.
pub fn standings(probe: &Probe<'_>) -> Vec<Standing> {
    let mut out = Vec::new();
    for server in probe.mcp_servers {
        // No tool name yet: `arg_candidates` yields nothing for a call
        // without one, so patterned rules cannot match and the answer is
        // the blanket floor. `resolve` degrades to `Gated` when a
        // patterned rule exists, precisely because this probe cannot see it.
        let args = json!({ "action": "call", "server": server });
        let (reach, reason) = resolve(probe, "mcp", &args);
        // The engine describes a call with no tool name as
        // `mcp call <server>/<missing tool>` — right for a real call that
        // forgot the argument, wrong here, where the absence is the probe's
        // own doing. The standing is about the server, so drop the
        // placeholder rather than reporting a malformed call to a reader.
        let reason = reason.replace("/<missing tool>", "");
        out.push(Standing {
            tool: "mcp".into(),
            label: format!("mcp server `{server}`"),
            remedy: remedy_for(probe, "mcp", reach),
            reach,
            reason,
        });
    }
    for tool in probe.network_tools {
        let (reach, reason) = resolve(probe, tool, &json!({}));
        out.push(Standing {
            tool: tool.clone(),
            label: format!("`{tool}`"),
            remedy: remedy_for(probe, tool, reach),
            reach,
            reason,
        });
    }
    for skill in probe.skills {
        let args = json!({ "name": skill });
        let (reach, reason) = resolve(probe, "skill", &args);
        out.push(Standing {
            tool: "skill".into(),
            label: format!("skill `{skill}`"),
            remedy: remedy_for(probe, "skill", reach),
            reach,
            reason,
        });
    }
    out
}

fn resolve(probe: &Probe<'_>, tool: &str, args: &serde_json::Value) -> (Reach, String) {
    match probe.engine.evaluate(tool, args, probe.mode, probe.cwd) {
        Decision::Allow => (Reach::Open, String::new()),
        Decision::Deny { reason } => (Reach::Blocked, reason),
        Decision::Ask { reason, source } => {
            if vak_agent::auto_approve(
                probe.approval_mode,
                source,
                tool,
                args,
                probe.mode,
                probe.sandboxed,
                probe.cwd,
            ) {
                return (Reach::Open, String::new());
            }
            if probe.approver_answerable {
                return (Reach::Gated, reason);
            }
            // Uncertainty degrades to `Gated`, never to `Blocked`. A
            // patterned rule this probe could not evaluate might allow the
            // real call, and hiding a capability that would have worked is
            // worse than advertising one that gates.
            if probe.engine.has_patterned_rule(tool) {
                return (Reach::Gated, reason);
            }
            (
                Reach::Blocked,
                format!("{reason}, and this surface has no approver to answer it"),
            )
        }
    }
}

fn remedy_for(probe: &Probe<'_>, tool: &str, reach: Reach) -> String {
    if !reach.is_blocked() {
        return String::new();
    }
    let mut options = vec![format!(
        "allow `{tool}` outright with `allow = [\"{tool}\"]` in .vak/config.toml"
    )];
    if !probe.approver_answerable {
        options.push(
            "give the surface an approver that can answer — for a chat gateway, \
             `[gateway] approvals = \"forward\"` with `approver = \"<surface>:<chat>\"`"
                .to_string(),
        );
    }
    options.join("; or ")
}

/// Tools with no reachable use left on this turn, so the registry can drop
/// them. A tool stays if *any* of its standings is reachable: one blocked
/// MCP server must not take the broker down with it.
pub fn fully_blocked_tools(standings: &[Standing]) -> Vec<String> {
    let mut blocked: Vec<String> = Vec::new();
    for standing in standings {
        if blocked.contains(&standing.tool) {
            continue;
        }
        let all_blocked = standings
            .iter()
            .filter(|other| other.tool == standing.tool)
            .all(|other| other.reach.is_blocked());
        if all_blocked {
            blocked.push(standing.tool.clone());
        }
    }
    blocked
}

/// MCP servers that cannot be called, so the prompt's MCP catalogue can
/// leave them out rather than advertising them as usable.
pub fn blocked_mcp_servers(standings: &[Standing]) -> Vec<String> {
    standings
        .iter()
        .filter(|standing| standing.tool == "mcp" && standing.reach.is_blocked())
        .filter_map(|standing| {
            standing
                .label
                .strip_prefix("mcp server `")
                .and_then(|rest| rest.strip_suffix('`'))
                .map(str::to_string)
        })
        .collect()
}

/// Skills that cannot be loaded, so the prompt's skills section and
/// admitted capabilities can exclude them rather than advertising them as usable.
pub fn blocked_skills(standings: &[Standing]) -> Vec<String> {
    standings
        .iter()
        .filter(|standing| standing.tool == "skill" && standing.reach.is_blocked())
        .filter_map(|standing| {
            standing
                .label
                .strip_prefix("skill `")
                .and_then(|rest| rest.strip_suffix('`'))
                .map(str::to_string)
        })
        .collect()
}

/// The model-visible section. Stating this is the point: a model that
/// knows a capability is configured but unreachable can say so, and say
/// what would fix it, instead of spending the turn discovering it one
/// denied call at a time and then reporting that it has no such tool.
pub fn prompt_section(standings: &[Standing]) -> String {
    let blocked: Vec<&Standing> = standings
        .iter()
        .filter(|standing| standing.reach.is_blocked())
        .collect();
    if blocked.is_empty() {
        return String::new();
    }
    let mut section = String::from(
        "\nConfigured but NOT usable on this turn. These are not in your tool \
         schemas and calling them will fail. If the request needs one, say so \
         plainly, name the capability, and give the operator the fix — do not \
         substitute a different tool and do not answer as though you had the \
         data:\n",
    );
    for standing in blocked {
        section.push_str(&format!("- {}: {}.", standing.label, standing.reason));
        if !standing.remedy.is_empty() {
            section.push_str(&format!(" Fix: {}.", standing.remedy));
        }
        section.push('\n');
    }
    section
}

/// One audit line per blocked capability, for the security log.
pub fn audit_details(standings: &[Standing]) -> Vec<String> {
    standings
        .iter()
        .filter(|standing| standing.reach.is_blocked())
        .map(|standing| format!("capability={} reason={}", standing.label, standing.reason))
        .collect()
}
