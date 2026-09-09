//! Human-In-The-Loop (HIL) state machine with in-place command editing.
//!
//! A real `HilApprovalState` is populated from an `AgentEvent::ApprovalRequested`
//! that arrives over the SSE event stream, not from a hardcoded demo fixture.
//! When the operator approves or denies, the outcome is dispatched to the
//! server via `POST /sessions/{id}/approvals/{req_id}` — there is no local
//! state change that never reaches the server.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HilOutcome {
    ApproveOnce { modified_command: Option<String> },
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
    /// The command/script the approval gates (if applicable, e.g. for `bash`).
    pub original_command: String,
    /// Mutable in-place-edited copy of the command.
    pub edited_command: String,
    pub is_editing: bool,
    pub edit_cursor: usize,
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
        let cmd = Self::command_from_args(&tool_name, &args_json);
        let cmd_owned = cmd.clone();
        Self {
            request_id: request_id.into(),
            session_id: session_id.into(),
            tool_name,
            args_json,
            risk_reason: reason.into(),
            workspace_path: workspace_path.into(),
            cost_estimate_usd,
            original_command: cmd_owned.clone(),
            edited_command: cmd_owned,
            is_editing: false,
            edit_cursor: cmd.len(),
            remember: RememberMode::No,
        }
    }

    /// Extract a human-readable command string from tool arguments.
    /// For `bash`-type tools this is the script; for others it is a
    /// rendered summary of the args JSON.
    fn command_from_args(tool_name: &str, args_json: &str) -> String {
        if tool_name == "bash" || tool_name == "shell" {
            if let Ok(val) = serde_json::from_str::<Value>(args_json)
                && let Some(code) = val.get("command").and_then(|v| v.as_str())
            {
                return code.to_string();
            }
            args_json.to_string()
        } else {
            // For non-bash tools, render the args as a compact summary.
            if let Ok(val) = serde_json::from_str::<Value>(args_json) {
                serde_json::to_string(&val).unwrap_or_else(|_| args_json.to_string())
            } else {
                args_json.to_string()
            }
        }
    }

    pub fn start_editing(&mut self) {
        self.is_editing = true;
        self.edit_cursor = self.edited_command.len();
    }

    pub fn cancel_editing(&mut self) {
        self.is_editing = false;
        self.edited_command = self.original_command.clone();
    }

    pub fn toggle_remember(&mut self) {
        self.remember = match self.remember {
            RememberMode::No => RememberMode::Yes,
            RememberMode::Yes => RememberMode::No,
        };
    }

    pub fn insert_char(&mut self, c: char) {
        if self.is_editing {
            if self.edit_cursor >= self.edited_command.len() {
                self.edited_command.push(c);
            } else {
                self.edited_command.insert(self.edit_cursor, c);
            }
            self.edit_cursor += c.len_utf8();
        }
    }

    pub fn delete_backspace(&mut self) {
        if self.is_editing && self.edit_cursor > 0 && !self.edited_command.is_empty() {
            let mut prev = self.edit_cursor - 1;
            while prev > 0 && !self.edited_command.is_char_boundary(prev) {
                prev -= 1;
            }
            self.edited_command.remove(prev);
            self.edit_cursor = prev;
        }
    }

    pub fn finish_approval(&self) -> HilOutcome {
        let modified = if self.edited_command != self.original_command {
            Some(self.edited_command.clone())
        } else {
            None
        };
        HilOutcome::ApproveOnce {
            modified_command: modified,
        }
    }

    /// Whether the edited command differs from the original.
    pub fn has_modification(&self) -> bool {
        self.edited_command != self.original_command
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
