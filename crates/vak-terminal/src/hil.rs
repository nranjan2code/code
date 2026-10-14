//! Human-In-The-Loop (HIL) state machine. The operator approves or denies
//! the call as it was asked; there is no way to approve a changed call.
//!
//! A real `HilApprovalState` is populated from an `AgentEvent::ApprovalRequested`
//! that arrives over the SSE event stream, not from a hardcoded demo fixture.
//! When the operator approves or denies, the outcome is dispatched to the
//! server via `POST /sessions/{id}/approvals/{req_id}` — there is no local
//! state change that never reaches the server.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HilOutcome {
    ApproveOnce,
    AlwaysForSession,
    Deny,
    Cancel,
}

/// Represents whether a "remember this decision" checkbox is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RememberMode {
    No,
    Yes,
}

/// A real approval request that arrived over the SSE event stream.
#[derive(Debug, Clone)]
pub struct HilApprovalState {
    pub request_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub args_json: String,
    pub risk_reason: String,
    pub workspace_path: String,
    pub cost_estimate_usd: Option<f64>,
    /// What the approval gates, shown as asked: a call's `command` argument
    /// when it has one, else its arguments.
    pub original_command: String,
    pub remember: RememberMode,
}

impl HilApprovalState {
    /// Build from a real `ApprovalRequested` agent event.
    pub fn from_approval_event(
        session_id: impl Into<String>,
        request_id: impl Into<String>,
        tool_name: impl Into<String>,
        args_json: impl Into<String>,
        reason: impl Into<String>,
        workspace_path: impl Into<String>,
        cost_estimate_usd: Option<f64>,
    ) -> Self {
        let tool_name = tool_name.into();
        let args_json = args_json.into();
        let cmd = Self::command_from_args(&args_json);
        Self {
            request_id: request_id.into(),
            session_id: session_id.into(),
            tool_name,
            args_json,
            risk_reason: reason.into(),
            workspace_path: workspace_path.into(),
            cost_estimate_usd,
            original_command: cmd,
            remember: RememberMode::No,
        }
    }

    /// A human-readable form of the arguments: the `command` argument when
    /// the call has one, else the arguments as compact JSON. Read from the
    /// arguments, never from the tool's name.
    fn command_from_args(args_json: &str) -> String {
        match serde_json::from_str::<Value>(args_json) {
            Ok(val) => match val.get("command").and_then(Value::as_str) {
                Some(code) => code.to_string(),
                None => serde_json::to_string(&val).unwrap_or_else(|_| args_json.to_string()),
            },
            Err(_) => args_json.to_string(),
        }
    }

    pub fn toggle_remember(&mut self) {
        self.remember = match self.remember {
            RememberMode::No => RememberMode::Yes,
            RememberMode::Yes => RememberMode::No,
        };
    }
}

/// Helper to extract an `ApprovalRequested` event from an `AgentEvent` JSON
/// value (as received over the SSE event stream).  Returns `None` if the
/// event is not an approval request or is malformed.
pub fn extract_approval_request(event_json: &serde_json::Value) -> Option<RealApprovalRequest> {
    // The server emits AgentEvent::ApprovalRequested as:
    // {"ApprovalRequested": {"id": "...", "tool": "...", "args_json": "...", "reason": "..."}}
    let approval = event_json.get("ApprovalRequested")?;
    Some(RealApprovalRequest {
        request_id: approval
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from)?,
        tool: approval
            .get("tool")
            .and_then(|v| v.as_str())
            .map(String::from)?,
        args_json: approval
            .get("args_json")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_default(),
        reason: approval
            .get("reason")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_default(),
    })
}

/// A real approval request extracted from the agent event stream.
#[derive(Debug, Clone)]
pub struct RealApprovalRequest {
    pub request_id: String,
    pub tool: String,
    pub args_json: String,
    pub reason: String,
}
