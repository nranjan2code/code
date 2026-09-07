//! Universal CloudEvents 1.0 envelope with W3C Distributed Tracing and Merkle causal hash chaining.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// W3C Distributed Trace Context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceContext {
    /// W3C traceparent string: "00-<trace_id_32_hex>-<parent_id_16_hex>-<trace_flags_2_hex>".
    pub traceparent: String,
    /// Optional W3C tracestate vendor key-value pairs.
    pub tracestate: Option<String>,
}

impl TraceContext {
    /// Generate a new root W3C trace context.
    pub fn new_root() -> Self {
        let trace_id = format!("{:032x}", Uuid::now_v7().as_u128());
        let parent_id = format!("{:016x}", (Uuid::now_v7().as_u128() & 0xFFFFFFFFFFFFFFFF));
        let traceparent = format!("00-{trace_id}-{parent_id}-01");
        Self {
            traceparent,
            tracestate: None,
        }
    }

    /// Create a child trace context inheriting the parent trace_id.
    pub fn child_span(&self) -> Self {
        let parts: Vec<&str> = self.traceparent.split('-').collect();
        if parts.len() == 4 {
            let trace_id = parts[1];
            let new_span_id = format!("{:016x}", (Uuid::now_v7().as_u128() & 0xFFFFFFFFFFFFFFFF));
            let traceparent = format!("00-{trace_id}-{new_span_id}-01");
            Self {
                traceparent,
                tracestate: self.tracestate.clone(),
            }
        } else {
            Self::new_root()
        }
    }

    /// Extract the 32-character hex trace_id.
    pub fn trace_id(&self) -> &str {
        let parts: Vec<&str> = self.traceparent.split('-').collect();
        if parts.len() == 4 { parts[1] } else { "" }
    }

    /// Extract the 16-character hex span_id.
    pub fn span_id(&self) -> &str {
        let parts: Vec<&str> = self.traceparent.split('-').collect();
        if parts.len() == 4 { parts[2] } else { "" }
    }
}

/// Metadata describing payload encryption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptionMeta {
    /// Algorithm identifier (e.g. "AES-256-GCM").
    pub algorithm: String,
    /// Key identifier used for derivation.
    pub key_id: String,
    /// Initialization vector / nonce hex string (12 bytes / 24 hex characters).
    pub nonce_hex: String,
}

/// Causal lineage and Merkle chain coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CausalLineage {
    pub workspace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    pub origin_agent_id: String,
    pub seq_num: u64,
    pub prev_event_hash: String,
    pub payload_hash: String,
}

/// Universal CloudEvents-compatible message envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageEnvelope {
    pub specversion: String,
    pub id: String,
    pub source: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub time: DateTime<Utc>,
    pub datacontenttype: String,
    pub trace: TraceContext,
    pub lineage: CausalLineage,
    pub payload: Vec<u8>,
    pub encrypted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption: Option<EncryptionMeta>,
}

impl MessageEnvelope {
    /// Genesis hash used for the first event in a stream or session.
    pub const GENESIS_HASH: &'static str =
        "0000000000000000000000000000000000000000000000000000000000000000";

    /// Construct a new unencrypted message envelope.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: impl Into<String>,
        event_type: impl Into<String>,
        workspace_id: impl Into<String>,
        origin_agent_id: impl Into<String>,
        seq_num: u64,
        prev_event_hash: impl Into<String>,
        payload: Vec<u8>,
        trace: Option<TraceContext>,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let payload_hash = hex::encode(hasher.finalize());

        let lineage = CausalLineage {
            workspace_id: workspace_id.into(),
            session_id: None,
            parent_agent_id: None,
            origin_agent_id: origin_agent_id.into(),
            seq_num,
            prev_event_hash: prev_event_hash.into(),
            payload_hash,
        };

        Self {
            specversion: "1.0".to_string(),
            id: Uuid::now_v7().to_string(),
            source: source.into(),
            event_type: event_type.into(),
            time: Utc::now(),
            datacontenttype: "application/json".to_string(),
            trace: trace.unwrap_or_else(TraceContext::new_root),
            lineage,
            payload,
            encrypted: false,
            encryption: None,
        }
    }

    /// Attach session and parent context.
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.lineage.session_id = Some(session_id.into());
        self
    }

    pub fn with_parent_agent(mut self, parent_agent_id: impl Into<String>) -> Self {
        self.lineage.parent_agent_id = Some(parent_agent_id.into());
        self
    }

    /// Compute cryptographic hash of envelope headers and payload hash (Merkle leaf).
    pub fn compute_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.specversion.as_bytes());
        hasher.update(b"|");
        hasher.update(self.id.as_bytes());
        hasher.update(b"|");
        hasher.update(self.source.as_bytes());
        hasher.update(b"|");
        hasher.update(self.event_type.as_bytes());
        hasher.update(b"|");
        hasher.update(self.time.to_rfc3339().as_bytes());
        hasher.update(b"|");
        hasher.update(self.lineage.workspace_id.as_bytes());
        hasher.update(b"|");
        hasher.update(self.lineage.origin_agent_id.as_bytes());
        hasher.update(b"|");
        hasher.update(self.lineage.seq_num.to_be_bytes());
        hasher.update(b"|");
        hasher.update(self.lineage.prev_event_hash.as_bytes());
        hasher.update(b"|");
        hasher.update(self.lineage.payload_hash.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Verify causal link between previous and current envelope in a stream.
    pub fn verify_merkle_link(prev: &MessageEnvelope, current: &MessageEnvelope) -> bool {
        if current.lineage.seq_num != prev.lineage.seq_num + 1 {
            return false;
        }
        let expected_prev_hash = prev.compute_hash();
        subtle::ConstantTimeEq::ct_eq(
            current.lineage.prev_event_hash.as_bytes(),
            expected_prev_hash.as_bytes(),
        )
        .into()
    }

    /// Additional Authenticated Data (AAD) for AES-256-GCM encryption.
    pub fn aad_bytes(&self) -> Vec<u8> {
        format!(
            "{}:{}:{}:{}",
            self.id, self.event_type, self.lineage.workspace_id, self.lineage.seq_num
        )
        .into_bytes()
    }
}
