//! Human-In-The-Loop (HIL) state machine with in-place command editing.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HilOutcome {
    ApproveOnce { modified_command: Option<String> },
    AlwaysForSession,
    Deny,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct HilApprovalState {
    pub request_id: String,
    pub tool_name: String,
    pub original_command: String,
    pub edited_command: String,
    pub is_editing: bool,
    pub edit_cursor: usize,
    pub risk_reason: String,
    pub workspace_path: String,
    pub cost_estimate_usd: f64,
}

impl HilApprovalState {
    pub fn new(
        request_id: impl Into<String>,
        tool_name: impl Into<String>,
        command: impl Into<String>,
        risk_reason: impl Into<String>,
        workspace_path: impl Into<String>,
        cost_estimate_usd: f64,
    ) -> Self {
        let cmd = command.into();
        Self {
            request_id: request_id.into(),
            tool_name: tool_name.into(),
            original_command: cmd.clone(),
            edited_command: cmd.clone(),
            is_editing: false,
            edit_cursor: cmd.len(),
            risk_reason: risk_reason.into(),
            workspace_path: workspace_path.into(),
            cost_estimate_usd,
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
}
