//! Typed identifiers: `<prefix>_<full UUIDv7>`, never truncated, never
//! derived from a path, a display name or the clock alone
//! (docs/design/73-data-architecture-and-lifecycle.md §3).

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// Why a string is not a valid id of a given type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    #[error("expected prefix `{expected}_`, found `{found}`")]
    WrongPrefix {
        expected: &'static str,
        found: String,
    },
    #[error("`{0}` is not a full UUIDv7 in canonical form")]
    BadUuid(String),
}

fn parse_body(prefix: &'static str, s: &str) -> Result<uuid::Uuid, IdError> {
    let body = s
        .strip_prefix(prefix)
        .and_then(|r| r.strip_prefix('_'))
        .ok_or_else(|| IdError::WrongPrefix {
            expected: prefix,
            found: s.chars().take(16).collect(),
        })?;
    let u = uuid::Uuid::parse_str(body).map_err(|_| IdError::BadUuid(body.to_string()))?;
    let canonical = body == u.hyphenated().to_string();
    if !canonical || u.get_version_num() != 7 {
        return Err(IdError::BadUuid(body.to_string()));
    }
    Ok(u)
}

macro_rules! typed_id {
    ($(#[$m:meta])* $name:ident, $prefix:literal) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(uuid::Uuid);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// A fresh id.
            pub fn new() -> Self {
                Self(uuid::Uuid::now_v7())
            }

            pub fn parse(s: &str) -> Result<Self, IdError> {
                parse_body($prefix, s).map(Self)
            }

            /// A stable id derived from `seed`, still a full UUIDv7. For the
            /// identities an install has before it has a record to persist
            /// one in (its local tenant, the built-in Agent, the system, a
            /// channel sender): the same seed is the same id on every run,
            /// and the seed is never recoverable from the id.
            pub fn derived(seed: &str) -> Self {
                use sha2::{Digest, Sha256};
                let mut h = Sha256::new();
                h.update($prefix.as_bytes());
                h.update([0u8]);
                h.update(seed.as_bytes());
                let digest = h.finalize();
                let mut bytes = [0u8; 10];
                bytes.copy_from_slice(&digest[..10]);
                Self(uuid::Builder::from_unix_timestamp_millis(0, &bytes).into_uuid())
            }

            /// Adopt an existing UUIDv7 (an owner record's id) as this id.
            pub fn from_uuid(u: uuid::Uuid) -> Option<Self> {
                (u.get_version_num() == 7).then_some(Self(u))
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}_{}", $prefix, self.0.hyphenated())
            }
        }

        impl FromStr for $name {
            type Err = IdError;
            fn from_str(s: &str) -> Result<Self, IdError> {
                Self::parse(s)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                Self::parse(&s).map_err(serde::de::Error::custom)
            }
        }
    };
}

typed_id!(
    /// The isolation, encryption and billing boundary.
    TenantId, "ten"
);
typed_id!(
    /// What a person calls a workspace; a path is only a binding to it.
    SpaceId, "spc"
);
typed_id!(AgentId, "agt");
typed_id!(
    /// A verified person or service identity.
    PrincipalId, "prn"
);
typed_id!(ConversationId, "cnv");
typed_id!(SessionId, "ses");
typed_id!(TurnId, "trn");
typed_id!(
    /// Any unit of work with a cause; also the W3C trace-id.
    RunId, "run"
);
typed_id!(ExecutionId, "exe");
typed_id!(ArtifactId, "art");
typed_id!(VersionId, "ver");
typed_id!(
    /// What started a run: a message, a schedule, a channel request.
    TriggerId, "trg"
);
typed_id!(
    /// One externally visible effect, so a retry is not a second one.
    EffectId, "eff"
);
typed_id!(ConnectionId, "con");
typed_id!(SourceId, "src");
typed_id!(DeliveryId, "dlv");

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_string_and_serde() {
        let id = RunId::new();
        let s = id.to_string();
        assert!(s.starts_with("run_"));
        assert_eq!(s.len(), 4 + 36);
        assert_eq!(RunId::parse(&s).unwrap(), id);
        let j = serde_json::to_string(&id).unwrap();
        assert_eq!(j, format!("\"{s}\""));
        assert_eq!(serde_json::from_str::<RunId>(&j).unwrap(), id);
    }

    #[test]
    fn prefixes_are_distinct_and_enforced() {
        let t = TriggerId::new().to_string();
        assert!(t.starts_with("trg_"));
        assert!(matches!(
            EffectId::parse(&t),
            Err(IdError::WrongPrefix { .. })
        ));
        assert!(PrincipalId::new().to_string().starts_with("prn_"));
        assert!(EffectId::new().to_string().starts_with("eff_"));
    }

    #[test]
    fn rejects_truncated_clock_derived_and_non_v7() {
        assert!(SessionId::parse("ses_1699999999").is_err());
        assert!(SessionId::parse("ses_").is_err());
        let v4 = format!("ses_{}", uuid::Uuid::new_v4());
        assert!(SessionId::parse(&v4).is_err());
        let simple = format!("ses_{}", uuid::Uuid::now_v7().simple());
        assert!(SessionId::parse(&simple).is_err());
        assert!(serde_json::from_str::<SessionId>("\"nope\"").is_err());
    }

    #[test]
    fn derived_ids_are_stable_valid_and_distinct() {
        let a = AgentId::derived("vak");
        assert_eq!(a, AgentId::derived("vak"));
        assert_ne!(a, AgentId::derived("other"));
        assert_eq!(AgentId::parse(&a.to_string()).unwrap(), a);
        assert_ne!(
            PrincipalId::derived("x").to_string()[4..],
            AgentId::derived("x").to_string()[4..]
        );
    }

    #[test]
    fn fresh_ids_differ() {
        assert_ne!(TurnId::new(), TurnId::new());
    }
}
