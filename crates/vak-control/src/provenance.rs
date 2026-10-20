//! Source labels for approval of mixed-context mutations and outbound calls.
//! Owner requests are private; unknown authorship in stored or editable input
//! is conservatively both private and untrusted. Nothing inspects wording to
//! grant authority (docs/design/08-permissions.md "Provenance").

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// What a tool result holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Provenance {
    /// Something the owner keeps in Vak: a workspace file, Vak's own
    /// records, a connected account's data.
    #[serde(default, skip_serializing_if = "is_false")]
    pub private: bool,
    /// Words someone other than the owner wrote: a web page, an integration
    /// server's answer, a message that reached a connected account.
    #[serde(default, skip_serializing_if = "is_false")]
    pub untrusted: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Provenance {
    pub const NONE: Self = Self {
        private: false,
        untrusted: false,
    };
    pub const PRIVATE: Self = Self {
        private: true,
        untrusted: false,
    };
    pub const UNTRUSTED: Self = Self {
        private: false,
        untrusted: true,
    };
    pub const BOTH: Self = Self {
        private: true,
        untrusted: true,
    };

    /// Everything either holds. Labels only ever join: nothing a request
    /// carried stops counting because a later step carries less.
    #[must_use]
    pub fn join(self, other: Self) -> Self {
        Self {
            private: self.private || other.private,
            untrusted: self.untrusted || other.untrusted,
        }
    }

    pub fn is_none(&self) -> bool {
        !self.private && !self.untrusted
    }

    /// Both together: mutation and outbound calls now go to a person.
    pub fn gates_exits(&self) -> bool {
        self.private && self.untrusted
    }
}

/// What a turn's requests carry, and from where, so the person asked can
/// see why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Held {
    labels: Provenance,
    private_from: BTreeSet<String>,
    untrusted_from: BTreeSet<String>,
}

impl Held {
    /// What a run starts with: a delegated worker starts with what its
    /// parent held when it delegated, because its task was written from it.
    pub fn starting_with(labels: Provenance) -> Self {
        let mut held = Self::default();
        held.note(DELEGATED_FROM, labels);
        held
    }

    /// Adds what one source holds: a tool's result by the tool's name, or
    /// what the request carried from earlier turns.
    pub fn note(&mut self, source: &str, labels: Provenance) {
        if labels.private {
            self.private_from.insert(source.to_string());
        }
        if labels.untrusted {
            self.untrusted_from.insert(source.to_string());
        }
        self.labels = self.labels.join(labels);
    }

    pub fn labels(&self) -> Provenance {
        self.labels
    }

    /// The reason a gated call is put to the person, naming where each kind
    /// of content came from.
    pub fn gate_reason(&self) -> String {
        let list =
            |sources: &BTreeSet<String>| sources.iter().cloned().collect::<Vec<_>>().join(", ");
        format!(
            "this turn holds private data (from {}) and words written by others (from {}), \
             and this call can change stored data or send data off this machine",
            list(&self.private_from),
            list(&self.untrusted_from),
        )
    }
}

/// The source name for what a request carried from earlier turns.
pub const EARLIER_TURNS: &str = "earlier turns";

/// The source name for inherited admission or delegated-parent context.
pub const DELEGATED_FROM: &str = "inherited run context";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_only_join() {
        assert_eq!(
            Provenance::PRIVATE.join(Provenance::UNTRUSTED),
            Provenance::BOTH
        );
        assert_eq!(Provenance::BOTH.join(Provenance::NONE), Provenance::BOTH);
        assert!(!Provenance::PRIVATE.gates_exits());
        assert!(!Provenance::UNTRUSTED.gates_exits());
        assert!(Provenance::BOTH.gates_exits());
    }

    #[test]
    fn the_reason_names_both_sources() {
        let mut held = Held::default();
        held.note("read", Provenance::PRIVATE);
        held.note("webfetch", Provenance::UNTRUSTED);
        assert!(held.labels().gates_exits());
        let reason = held.gate_reason();
        assert!(reason.contains("from read"));
        assert!(reason.contains("from webfetch"));
    }

    #[test]
    fn a_worker_starts_with_what_its_parent_held() {
        let held = Held::starting_with(Provenance::BOTH);
        assert!(held.labels().gates_exits());
        assert!(held.gate_reason().contains(DELEGATED_FROM));
        assert!(Held::starting_with(Provenance::NONE).labels().is_none());
    }

    #[test]
    fn an_empty_label_is_not_written() {
        assert_eq!(
            serde_json::to_string(&Provenance::NONE).ok().as_deref(),
            Some("{}")
        );
        assert_eq!(
            serde_json::from_str::<Provenance>("{}").ok(),
            Some(Provenance::NONE)
        );
    }
}
