//! Runs→flows adoption (docs/design/10-flows.md): convert proven work —
//! planner plan ledgers, flow-run snapshots, or a session's settled shell
//! commands — into a governed, hand-editable flow file. Provider/model are
//! NEVER baked in from caller hints; attribution stays in the receipts.

use crate::types::FlowState;

#[derive(Debug, Clone)]
pub struct AdoptedFlow {
    pub name: String,
    pub toml: String,
    pub warnings: Vec<String>,
}

fn header_comment(source: &str, note: &str) -> String {
    format!(
        "# adopted by `flow adopt` from {source} at {}\n# {note}\n",
        chrono::Utc::now().to_rfc3339(),
    )
}

/// Adopt a frozen plan/flow-run snapshot: its definition TOML is already
/// validated and proven — reuse it byte-for-byte under a new name.
pub fn from_flow_state(
    state_json: &str,
    new_name: &str,
    source_label: &str,
) -> Result<AdoptedFlow, String> {
    let state: FlowState =
        serde_json::from_str(state_json).map_err(|e| format!("not a flow-run ledger: {e}"))?;
    if state.nodes.is_empty() {
        return Err("run has no node results to adopt".into());
    }
    let mut warnings = Vec::new();
    // Rename inside the TOML so the adopted copy does not collide with the
    // source flow's own identity.
    let renamed = state.definition_toml.replacen(
        &format!("name = \"{}\"", state.flow_name),
        &format!("name = \"{new_name}\""),
        1,
    );
    if renamed == state.definition_toml
        && !state
            .definition_toml
            .contains(&format!("name = \"{new_name}\""))
    {
        warnings.push("could not rewrite [flow] name; edit the header manually".into());
    }
    let toml = format!(
        "{}{}",
        header_comment(source_label, "frozen snapshot reused verbatim"),
        renamed
    );
    Ok(AdoptedFlow {
        name: new_name.to_string(),
        toml,
        warnings,
    })
}

/// Adopt a session's settled shell work: every DISTINCT command that ran
/// green, in first-run order, chained linearly with a merge node that
/// reports partial failure. This is the honest extractable core of a
/// coding session — prompts and model turns stay behind.
pub fn from_green_commands(
    name: &str,
    description: &str,
    commands: &[String],
) -> Result<AdoptedFlow, String> {
    if commands.is_empty() {
        return Err("no green bash commands found to adopt".into());
    }
    let mut toml = String::new();
    toml.push_str(&header_comment(
        "session commands (green only)",
        description,
    ));
    toml.push_str(&format!("[flow]\nname = \"{name}\"\n"));
    if !description.trim().is_empty() {
        // Single-line safe description.
        let d: String = description
            .chars()
            .map(|c| if c == '\n' { ' ' } else { c })
            .collect();
        toml.push_str(&format!("description = \"{d}\"\n"));
    }
    toml.push('\n');

    let mut prev: Option<String> = None;
    for (i, cmd) in commands.iter().enumerate() {
        let id = format!("step_{}", i + 1);
        toml.push_str("[[nodes]]\n");
        toml.push_str(&format!("id = \"{id}\"\n"));
        toml.push_str("type = \"bash\"\n");
        if let Some(p) = &prev {
            toml.push_str(&format!("deps = [\"{p}\"]\n"));
        }
        // Escape for basic TOML string: backslashes and quotes.
        let escaped = cmd.replace('\\', "\\\\").replace('"', "\\\"");
        toml.push_str(&format!("command = \"{escaped}\"\n\n"));
        prev = Some(id);
    }

    // Merge tail always runs so a broken chain still reports partial output.
    let Some(last) = prev else {
        return Err("no commands to chain".into());
    };
    toml.push_str("[[nodes]]\n");
    toml.push_str("id = \"summary\"\ntype = \"merge\"\n");
    toml.push_str(&format!("deps = [\"{last}\"]\nrequired = false\n"));

    Ok(AdoptedFlow {
        name: name.to_string(),
        toml,
        warnings: Vec::new(),
    })
}

/// Deterministic run-vs-run comparison over two ledger JSON strings:
/// per-node status deltas and output differences. Pure; no model calls.
pub fn diff_flow_states(a_json: &str, b_json: &str) -> Result<String, String> {
    let a: FlowState = serde_json::from_str(a_json).map_err(|e| format!("ledger A: {e}"))?;
    let b: FlowState = serde_json::from_str(b_json).map_err(|e| format!("ledger B: {e}"))?;

    let mut out = String::new();
    out.push_str(&format!(
        "comparing {} vs {} (same definition: {})\n",
        a.run_id,
        b.run_id,
        a.definition_toml == b.definition_toml
    ));

    let mut keys: Vec<&String> = a.nodes.keys().chain(b.nodes.keys()).collect();
    keys.sort();
    keys.dedup();

    let mut diffs = 0usize;
    for k in keys {
        let sa = a.nodes.get(k);
        let sb = b.nodes.get(k);
        match (sa, sb) {
            (Some(x), Some(y)) => {
                if x.status != y.status {
                    out.push_str(&format!(
                        "  ~ {k}: status {:?} → {:?}\n",
                        x.status, y.status
                    ));
                    diffs += 1;
                } else if x.output != y.output {
                    out.push_str(&format!(
                        "  ~ {k}: output differs ({} vs {} bytes)\n",
                        x.output.len(),
                        y.output.len()
                    ));
                    diffs += 1;
                }
            }
            (Some(x), None) => {
                out.push_str(&format!("  - {k}: only in A ({:?})\n", x.status));
                diffs += 1;
            }
            (None, Some(y)) => {
                out.push_str(&format!("  + {k}: only in B ({:?})\n", y.status));
                diffs += 1;
            }
            (None, None) => unreachable!(),
        }
    }
    if diffs == 0 {
        out.push_str("  identical (no node status/output differences)\n");
    } else {
        out.insert_str(0, &format!("{diffs} difference(s):\n"));
    }
    Ok(out)
}

/// Recovery audit for `--resume`: classify the snapshot relationship and
/// return the typed action. Fail-closed on drift unless accepted.
pub fn recovery_audit(
    state_json: &str,
    live_definition: Option<&str>,
) -> Result<(String, &'static str), String> {
    let state: FlowState = serde_json::from_str(state_json).map_err(|e| e.to_string())?;
    match live_definition {
        None => Ok(("missing".into(), "rebuild")),
        Some(live) if live.trim() == state.definition_toml.trim() => {
            Ok(("frozen".into(), "resume"))
        }
        Some(_) => Ok(("drifted".into(), "repair")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const SAMPLE_STATE: &str = r#"{
      "run_id":"r1","flow_name":"old-name",
      "definition_toml":"[flow]\nname = \"old-name\"\n\n[[nodes]]\nid = \"a\"\ntype = \"bash\"\ncommand = \"echo hi\"\n",
      "started_at":"2026-08-24T00:00:00Z",
      "nodes":{"a":{"status":"completed","output":"hi"}}
    }"#;

    #[test]
    fn adopt_from_state_reuses_frozen_toml_and_renames() {
        let f = from_flow_state(SAMPLE_STATE, "new-name", "plan-1.json").unwrap();
        assert!(f.toml.contains("name = \"new-name\""));
        assert!(
            f.toml
                .contains("# adopted by `flow adopt` from plan-1.json")
        );
        assert!(f.toml.contains("command = \"echo hi\""));
        assert!(f.warnings.is_empty());
    }

    #[test]
    fn adopt_rejects_empty_run() {
        let empty = r#"{"run_id":"r","flow_name":"x","definition_toml":"","started_at":"2026-08-24T00:00:00Z","nodes":{}}"#;
        assert!(from_flow_state(empty, "n", "s").is_err());
    }

    #[test]
    fn green_commands_become_chained_bash_nodes_with_merge_tail() {
        let f = from_green_commands(
            "verify-loop",
            "rebuild + test",
            &["cargo build".into(), "cargo test -q".into()],
        )
        .unwrap();
        assert!(f.toml.contains("id = \"step_1\""));
        assert!(f.toml.contains("id = \"step_2\""));
        assert!(f.toml.contains("deps = [\"step_1\"]"));
        assert!(f.toml.contains("type = \"merge\""));
        assert!(f.toml.contains("command = \"cargo test -q\""));
    }

    #[test]
    fn escapes_quotes_and_backslashes_in_commands() {
        let f = from_green_commands("x", "", &[r#"echo "a\b""#.into()]).unwrap();
        assert!(f.toml.contains(r#"command = "echo \"a\\b\"""#));
    }

    #[test]
    fn empty_command_list_is_an_error() {
        assert!(from_green_commands("x", "", &[]).is_err());
    }

    const STATE_A: &str = r#"{"run_id":"A","flow_name":"f","definition_toml":"[flow]\nname=\"f\"","started_at":"2026-08-24T00:00:00Z","nodes":{"n1":{"status":"completed","output":"ok"}}}"#;
    const STATE_B_SAME: &str = r#"{"run_id":"B","flow_name":"f","definition_toml":"[flow]\nname=\"f\"","started_at":"2026-08-24T01:00:00Z","nodes":{"n1":{"status":"completed","output":"ok"}}}"#;
    const STATE_C_DIFF: &str = r#"{"run_id":"C","flow_name":"f","definition_toml":"[flow]\nname=\"f\"","started_at":"2026-08-24T02:00:00Z","nodes":{"n1":{"status":"failed","output":"boom"}}}"#;

    #[test]
    fn diff_reports_identical_and_differences() {
        let same = diff_flow_states(STATE_A, STATE_B_SAME).unwrap();
        assert!(same.contains("identical"));
        let d = diff_flow_states(STATE_A, STATE_C_DIFF).unwrap();
        assert!(d.contains("1 difference(s)"));
        assert!(d.contains("status Completed → Failed"));
    }

    #[test]
    fn recovery_audit_classifies_snapshot_relationship() {
        let live_same = "[flow]\nname=\"f\"";
        let (snap, action) = recovery_audit(STATE_A, Some(live_same)).unwrap();
        assert_eq!((snap.as_str(), action), ("frozen", "resume"));

        let (snap, action) = recovery_audit(STATE_A, Some("[flow]\nname=\"edited\"")).unwrap();
        assert_eq!((snap.as_str(), action), ("drifted", "repair"));

        let (snap, action) = recovery_audit(STATE_A, None).unwrap();
        assert_eq!((snap.as_str(), action), ("missing", "rebuild"));
    }
}
