//! Deterministic subject namespace algebra and role-based ACL evaluator.
//!
//! NATS subject format:
//! `vak.<plane>.<workspace_id>.<entity>.<target_or_session>.<verb_or_type>`

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SubjectError {
    #[error("empty subject string")]
    Empty,
    #[error("subject must start with 'vak.': {0}")]
    InvalidPrefix(String),
    #[error("invalid subject structure: {0}")]
    InvalidStructure(String),
}

/// Typed subject definitions across the ephemeral event and durable work planes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subject {
    /// Ephemeral streaming assistant & thinking tokens
    EventsTokens {
        workspace_id: String,
        session_id: String,
    },
    /// Ephemeral streaming ANSI stdout/stderr chunks
    EventsTerminal {
        workspace_id: String,
        session_id: String,
    },
    /// Ephemeral streaming process telemetry (500ms probe)
    EventsTelemetry {
        workspace_id: String,
        session_id: String,
    },
    /// Durable JetStream work queue for worker task claiming
    WorkTask { workspace_id: String, role: String },
    /// Durable point-to-point agent steering & direct inbox
    AgentInbox {
        workspace_id: String,
        agent_id: String,
    },
    /// Durable verified work receipts and execution evidence
    Receipts {
        workspace_id: String,
        session_id: String,
    },
    /// Durable elevated permission approval requests & verdicts
    Approvals {
        workspace_id: String,
        session_id: String,
    },
    /// Durable Dead-Letter Queue (DLQ) for poison tasks
    DeadLetter {
        workspace_id: String,
        agent_id: String,
    },
    /// Generic or custom subject string
    Custom(String),
}

impl Subject {
    /// Render subject to canonical NATS wire string.
    pub fn to_subject_string(&self) -> String {
        match self {
            Self::EventsTokens {
                workspace_id,
                session_id,
            } => format!("vak.events.{workspace_id}.{session_id}.tokens"),
            Self::EventsTerminal {
                workspace_id,
                session_id,
            } => format!("vak.events.{workspace_id}.{session_id}.terminal"),
            Self::EventsTelemetry {
                workspace_id,
                session_id,
            } => format!("vak.events.{workspace_id}.{session_id}.telemetry"),
            Self::WorkTask { workspace_id, role } => {
                format!("vak.work.{workspace_id}.{role}.task")
            }
            Self::AgentInbox {
                workspace_id,
                agent_id,
            } => format!("vak.agent.{workspace_id}.{agent_id}.inbox"),
            Self::Receipts {
                workspace_id,
                session_id,
            } => format!("vak.receipts.{workspace_id}.{session_id}.completed"),
            Self::Approvals {
                workspace_id,
                session_id,
            } => format!("vak.approvals.{workspace_id}.{session_id}.request"),
            Self::DeadLetter {
                workspace_id,
                agent_id,
            } => format!("vak.dlq.{workspace_id}.{agent_id}.failed"),
            Self::Custom(s) => s.clone(),
        }
    }

    /// Parse a wire subject string into a typed Subject.
    pub fn parse(s: &str) -> Result<Self, SubjectError> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.is_empty() {
            return Err(SubjectError::Empty);
        }
        if parts[0] != "vak" {
            return Err(SubjectError::InvalidPrefix(s.to_string()));
        }

        if parts.len() < 4 {
            return Ok(Self::Custom(s.to_string()));
        }

        match (parts[1], parts.len()) {
            ("events", 5) if parts[4] == "tokens" => Ok(Self::EventsTokens {
                workspace_id: parts[2].to_string(),
                session_id: parts[3].to_string(),
            }),
            ("events", 5) if parts[4] == "terminal" => Ok(Self::EventsTerminal {
                workspace_id: parts[2].to_string(),
                session_id: parts[3].to_string(),
            }),
            ("events", 5) if parts[4] == "telemetry" => Ok(Self::EventsTelemetry {
                workspace_id: parts[2].to_string(),
                session_id: parts[3].to_string(),
            }),
            ("work", 5) if parts[4] == "task" => Ok(Self::WorkTask {
                workspace_id: parts[2].to_string(),
                role: parts[3].to_string(),
            }),
            ("agent", 5) if parts[4] == "inbox" => Ok(Self::AgentInbox {
                workspace_id: parts[2].to_string(),
                agent_id: parts[3].to_string(),
            }),
            ("receipts", 5) if parts[4] == "completed" => Ok(Self::Receipts {
                workspace_id: parts[2].to_string(),
                session_id: parts[3].to_string(),
            }),
            ("approvals", 5) if parts[4] == "request" => Ok(Self::Approvals {
                workspace_id: parts[2].to_string(),
                session_id: parts[3].to_string(),
            }),
            ("dlq", 5) if parts[4] == "failed" => Ok(Self::DeadLetter {
                workspace_id: parts[2].to_string(),
                agent_id: parts[3].to_string(),
            }),
            _ => Ok(Self::Custom(s.to_string())),
        }
    }
}

/// Role-based subject ACL evaluator enforcing least-privilege topic isolation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AclPolicy {
    pub allow_publish: Vec<String>,
    pub deny_publish: Vec<String>,
    pub allow_subscribe: Vec<String>,
    pub deny_subscribe: Vec<String>,
}

impl AclPolicy {
    /// Construct a least-privilege policy for a specific worker agent.
    pub fn for_worker(workspace_id: &str, session_id: &str, agent_id: &str, role: &str) -> Self {
        Self {
            allow_publish: vec![
                format!("vak.events.{workspace_id}.{session_id}.>"),
                format!("vak.receipts.{workspace_id}.{session_id}.*"),
                format!("vak.approvals.{workspace_id}.{session_id}.*"),
                format!("vak.dlq.{workspace_id}.{agent_id}.*"),
            ],
            deny_publish: vec![
                format!("vak.work.{workspace_id}.>"), // Workers cannot forge work items
            ],
            allow_subscribe: vec![
                format!("vak.agent.{workspace_id}.{agent_id}.inbox"),
                format!("vak.work.{workspace_id}.{role}.task"),
            ],
            deny_subscribe: vec![
                format!("vak.approvals.{workspace_id}.>"), // Workers cannot snoop on approval channels
            ],
        }
    }

    /// Check whether publishing to `subject` is permitted.
    pub fn can_publish(&self, subject: &str) -> bool {
        // Deny rules take strict precedence
        for deny in &self.deny_publish {
            if matches_pattern(deny, subject) {
                return false;
            }
        }
        for allow in &self.allow_publish {
            if matches_pattern(allow, subject) {
                return true;
            }
        }
        false
    }

    /// Check whether subscribing to `subject` is permitted.
    pub fn can_subscribe(&self, subject: &str) -> bool {
        // Deny rules take strict precedence
        for deny in &self.deny_subscribe {
            if matches_pattern(deny, subject) {
                return false;
            }
        }
        for allow in &self.allow_subscribe {
            if matches_pattern(allow, subject) {
                return true;
            }
        }
        false
    }
}

/// Matches a NATS subject pattern containing `*` (single token) or `>` (multi-token tail).
pub fn matches_pattern(pattern: &str, subject: &str) -> bool {
    let p_tokens: Vec<&str> = pattern.split('.').collect();
    let s_tokens: Vec<&str> = subject.split('.').collect();

    let mut i = 0;
    while i < p_tokens.len() {
        if p_tokens[i] == ">" {
            // '>' matches everything from here to the end
            return true;
        }
        if i >= s_tokens.len() {
            return false;
        }
        if p_tokens[i] != "*" && p_tokens[i] != s_tokens[i] {
            return false;
        }
        i += 1;
    }

    i == s_tokens.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_pattern_matching() {
        assert!(matches_pattern(
            "vak.events.ws1.>",
            "vak.events.ws1.sess1.tokens"
        ));
        assert!(matches_pattern(
            "vak.events.ws1.sess1.*",
            "vak.events.ws1.sess1.tokens"
        ));
        assert!(!matches_pattern(
            "vak.events.ws1.sess1.*",
            "vak.events.ws1.sess1.tokens.extra"
        ));
        assert!(!matches_pattern(
            "vak.events.ws2.>",
            "vak.events.ws1.sess1.tokens"
        ));
    }

    #[test]
    fn acl_worker_enforcement() {
        let policy = AclPolicy::for_worker("ws_alpha", "sess_001", "agent_coder", "coder");

        // Allowed publish
        assert!(policy.can_publish("vak.events.ws_alpha.sess_001.tokens"));
        assert!(policy.can_publish("vak.events.ws_alpha.sess_001.terminal"));
        assert!(policy.can_publish("vak.receipts.ws_alpha.sess_001.completed"));

        // Denied publish: cannot publish to work queue (denied) or other session
        assert!(!policy.can_publish("vak.work.ws_alpha.coder.task"));
        assert!(!policy.can_publish("vak.events.ws_alpha.sess_002.tokens"));

        // Allowed subscribe
        assert!(policy.can_subscribe("vak.agent.ws_alpha.agent_coder.inbox"));
        assert!(policy.can_subscribe("vak.work.ws_alpha.coder.task"));

        // Denied subscribe: cannot snoop other agent inboxes
        assert!(!policy.can_subscribe("vak.agent.ws_alpha.agent_other.inbox"));
    }
}
