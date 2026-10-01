//! The trace key every durable record carries
//! (docs/design/73-data-architecture-and-lifecycle.md §4).

use crate::ids::{
    AgentId, ConversationId, PrincipalId, RunId, SessionId, SpaceId, TenantId, TriggerId, TurnId,
};
use serde::{Deserialize, Serialize};

/// A span id: W3C span-id, 16 lowercase hex characters, never zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpanId(u64);

impl SpanId {
    fn fresh() -> Self {
        let n = (uuid::Uuid::now_v7().as_u128() & u128::from(u64::MAX)) as u64;
        Self(if n == 0 { 1 } else { n })
    }

    pub fn parse(s: &str) -> Option<Self> {
        if s.len() != 16 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return None;
        }
        u64::from_str_radix(s, 16)
            .ok()
            .filter(|n| *n != 0)
            .map(Self)
    }
}

impl std::fmt::Display for SpanId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl Serialize for SpanId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SpanId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).ok_or_else(|| serde::de::Error::custom("invalid span id"))
    }
}

/// Why a run exists. Additive-only (invariant 29).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cause {
    User {
        request_id: String,
    },
    Channel {
        endpoint: String,
        request_id: String,
    },
    Schedule {
        schedule: String,
        slot: String,
    },
    Delegation {
        parent_run: RunId,
        tool_use_id: String,
    },
    Revision {
        candidate: String,
    },
    /// Started by a registered trigger (a webhook, a watcher, a gateway).
    Trigger {
        trigger: TriggerId,
        request_id: String,
    },
    Heartbeat,
    System {
        job: String,
    },
}

/// Identity of a unit of work, propagated by value and never re-derived from
/// ambient state. `run` is the W3C trace-id and `span` the span-id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceKey {
    pub tenant: TenantId,
    pub space: SpaceId,
    pub agent: AgentId,
    /// The principal that acted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PrincipalId>,
    /// The principal the actor acted for, when different.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<PrincipalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<ConversationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnId>,
    pub run: RunId,
    pub span: SpanId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span: Option<SpanId>,
    pub cause: Cause,
}

impl TraceKey {
    /// The root of a new run: a fresh run id and span, no parent.
    pub fn root(tenant: TenantId, space: SpaceId, agent: AgentId, cause: Cause) -> Self {
        Self {
            tenant,
            space,
            agent,
            actor: None,
            on_behalf_of: None,
            conversation: None,
            session: None,
            turn: None,
            run: RunId::new(),
            span: SpanId::fresh(),
            parent_span: None,
            cause,
        }
    }

    /// The only span constructor: same run, a fresh span whose parent is this
    /// one. Callers narrow the returned key's optional scope fields.
    pub fn child(&self) -> Self {
        let mut c = self.clone();
        c.parent_span = Some(self.span);
        c.span = SpanId::fresh();
        c
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn root() -> TraceKey {
        TraceKey::root(
            TenantId::new(),
            SpaceId::new(),
            AgentId::new(),
            Cause::Heartbeat,
        )
    }

    #[test]
    fn child_keeps_run_and_links_parent() {
        let r = root();
        let c = r.child();
        assert_eq!(c.run, r.run);
        assert_ne!(c.span, r.span);
        assert_eq!(c.parent_span, Some(r.span));
        assert_eq!(c.tenant, r.tenant);
        let g = c.child();
        assert_eq!(g.parent_span, Some(c.span));
        assert_eq!(r.parent_span, None);
    }

    #[test]
    fn span_is_sixteen_hex_and_round_trips() {
        let s = root().span.to_string();
        assert_eq!(s.len(), 16);
        assert_eq!(SpanId::parse(&s).unwrap().to_string(), s);
        assert!(SpanId::parse("0000000000000000").is_none());
        assert!(SpanId::parse("XYZ").is_none());
    }

    #[test]
    fn serde_round_trip_with_actor_and_causes() {
        let mut k = root();
        k.actor = Some(PrincipalId::new());
        k.on_behalf_of = Some(PrincipalId::new());
        for cause in [
            Cause::User {
                request_id: "r".into(),
            },
            Cause::Channel {
                endpoint: "e".into(),
                request_id: "r".into(),
            },
            Cause::Schedule {
                schedule: "s".into(),
                slot: "t".into(),
            },
            Cause::Delegation {
                parent_run: RunId::new(),
                tool_use_id: "tu".into(),
            },
            Cause::Revision {
                candidate: "c".into(),
            },
            Cause::Trigger {
                trigger: TriggerId::new(),
                request_id: "r".into(),
            },
            Cause::Heartbeat,
            Cause::System { job: "j".into() },
        ] {
            k.cause = cause;
            let j = serde_json::to_string(&k).unwrap();
            assert_eq!(serde_json::from_str::<TraceKey>(&j).unwrap(), k);
        }
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let k = root();
        let mut v = serde_json::to_value(&k).unwrap();
        v["added_later"] = serde_json::json!(1);
        assert_eq!(serde_json::from_value::<TraceKey>(v).unwrap(), k);
    }
}
