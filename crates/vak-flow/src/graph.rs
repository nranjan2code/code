//! Run-graph snapshots (docs/design/10-flows.md): typed projection over a
//! flow-run ledger — zero rendering opinions, safe to serve as JSON or
//! drive any UI strip.

use crate::types::{FlowState, NodeStatus};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    pub id: String,
    /// Topological layer index (1-based) derived from the frozen
    /// definition; 0 when the definition no longer parses.
    pub layer: usize,
    pub status: &'static str,
    pub output_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunGraph {
    pub run_id: String,
    pub flow_name: String,
    pub layers_total: usize,
    pub nodes: Vec<GraphNode>,
    pub completed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub pending_or_running: usize,
    pub generated_at: String,
}

fn status_str(s: NodeStatus) -> &'static str {
    match s {
        NodeStatus::Pending => "pending",
        NodeStatus::Running => "running",
        NodeStatus::Completed => "completed",
        NodeStatus::Failed => "failed",
        NodeStatus::Skipped => "skipped",
    }
}

/// Layer assignment mirrors `parse::layers`: repeated passes peel off
/// ready nodes until exhaustion (cycle-safe).
fn compute_layers(
    nodes: &[crate::types::NodeDef],
) -> (std::collections::HashMap<String, usize>, usize) {
    let mut map = std::collections::HashMap::new();
    let mut remaining: Vec<&crate::types::NodeDef> = nodes.iter().collect();
    let mut layer = 0usize;
    while !remaining.is_empty() {
        layer += 1;
        // Ready = every dep already placed in an EARLIER pass.
        let placed_now: Vec<String> = Vec::new();
        let _ = placed_now;
        let mut progressed = false;
        let newly: Vec<String> = remaining
            .iter()
            .filter(|n| {
                n.deps
                    .iter()
                    .all(|d| map.contains_key(d) || !remaining.iter().any(|r| &r.id == d))
            })
            .map(|n| n.id.clone())
            .collect();
        if !newly.is_empty() {
            for id in &newly {
                map.insert(id.clone(), layer);
            }
            progressed = true;
            remaining.retain(|n| !newly.contains(&n.id));
        }
        if !progressed {
            // Cycle or dangling deps: pin the rest to the next layer so the
            // snapshot still renders instead of diverging.
            for n in remaining {
                map.insert(n.id.clone(), layer);
            }
            break;
        }
    }
    (map, layer)
}

pub fn graph_snapshot(state: &FlowState) -> RunGraph {
    let (layer_map, layers_total) = match crate::parse_flow(&state.definition_toml) {
        Ok(flow) => {
            let (m, l) = compute_layers(&flow.nodes);
            (m, l)
        }
        Err(_) => (std::collections::HashMap::new(), 0),
    };

    let mut nodes: Vec<GraphNode> = Vec::new();
    let (mut completed, mut failed, mut skipped, mut pending) = (0usize, 0, 0, 0);
    // Deterministic order: by (layer, id).
    let mut entries: Vec<(&String, &crate::types::NodeResult)> = state.nodes.iter().collect();
    entries.sort_by(|a, b| {
        let la = layer_map.get(a.0).copied().unwrap_or(0);
        let lb = layer_map.get(b.0).copied().unwrap_or(0);
        la.cmp(&lb).then(a.0.cmp(b.0))
    });
    for (id, res) in entries {
        let status = status_str(res.status);
        match res.status {
            NodeStatus::Completed => completed += 1,
            NodeStatus::Failed => failed += 1,
            NodeStatus::Skipped => skipped += 1,
            _ => pending += 1,
        }
        nodes.push(GraphNode {
            id: id.clone(),
            layer: layer_map.get(id).copied().unwrap_or(0),
            status,
            output_bytes: res.output.len(),
        });
    }

    RunGraph {
        run_id: state.run_id.clone(),
        flow_name: state.flow_name.clone(),
        layers_total,
        nodes,
        completed,
        failed,
        skipped,
        pending_or_running: pending,
        generated_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::{FlowState, NodeResult};

    const DEF: &str = "[flow]\nname=\"g\"\ndescription=\"\"\n\n[[nodes]]\nid=\"a\"\ntype=\"bash\"\ncommand=\"echo a\"\n\n[[nodes]]\nid=\"b\"\ntype=\"bash\"\ncommand=\"echo b\"\ndeps=[\"a\"]\n";

    fn state_with(nodes: &[(&str, NodeStatus)]) -> FlowState {
        let mut st = FlowState {
            run_id: "r".into(),
            flow_name: "g".into(),
            definition_toml: DEF.into(),
            started_at: chrono::Utc::now(),
            nodes: Default::default(),
        };
        for (id, status) in nodes {
            st.nodes.insert(
                (*id).into(),
                NodeResult {
                    status: *status,
                    output: "x".into(),
                },
            );
        }
        st
    }

    #[test]
    fn snapshot_maps_layers_statuses_and_counts() {
        let st = state_with(&[("a", NodeStatus::Completed), ("b", NodeStatus::Running)]);
        let g = graph_snapshot(&st);
        assert_eq!(g.layers_total, 2);
        assert_eq!(g.completed, 1);
        assert_eq!(g.pending_or_running, 1);
        assert_eq!(g.nodes[0].id, "a");
        assert_eq!(g.nodes[0].layer, 1);
        assert_eq!(g.nodes[1].layer, 2);
        assert_eq!(g.nodes[1].status, "running");
    }

    #[test]
    fn unparseable_definition_still_snapshots_with_layer_zero() {
        let mut st = state_with(&[("a", NodeStatus::Failed)]);
        st.definition_toml = "not [ valid toml".into();
        let g = graph_snapshot(&st);
        assert_eq!(g.layers_total, 0);
        assert_eq!(g.nodes[0].layer, 0);
        assert_eq!(g.failed, 1);
    }

    #[test]
    fn deterministic_ordering_across_calls() {
        let st = state_with(&[("b", NodeStatus::Completed), ("a", NodeStatus::Skipped)]);
        let g1 = graph_snapshot(&st);
        let g2 = graph_snapshot(&st);
        assert_eq!(
            serde_json::to_string(&g1.nodes).unwrap(),
            serde_json::to_string(&g2.nodes).unwrap()
        );
    }
}
