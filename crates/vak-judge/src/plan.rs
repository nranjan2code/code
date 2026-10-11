//! The runtime's check of a declared plan against what the turn produced
//! (docs/design/86-model-plans-runtime-checks.md §2.2). The loop gathers the
//! receipts; this says which deliverables have none, and whether the answer
//! stands. It states what was observed and attributes nothing it did not
//! see.

use vak_control::{AnswerVerdict, Cause};
use vak_intent::{Deliverable, DeliverableForm};

/// What the turn produced, as the loop observed it.
pub struct Produced<'a> {
    /// Every card recorded this turn, in emit order, with the deliverable
    /// its `answers` field named.
    pub cards: &'a [Option<String>],
    pub final_text: &'a str,
    /// A file delivered for review, or a declared artifact.
    pub file_delivered: bool,
    /// A file changed, a command ran, or an integration acted.
    pub changed: bool,
    /// A command ran and nothing changed after it.
    pub checked: bool,
}

/// What a plan still lacks.
pub struct Gap<'a> {
    /// Planned cards that no card's `answers` names, in plan order.
    pub cards_unnamed: Vec<&'a Deliverable>,
    /// Cards shown whose `answers` names no planned card.
    pub cards_unattributed: usize,
    /// Deliverables of every other form with no receipt, in plan order.
    pub others: Vec<&'a Deliverable>,
}

impl Gap<'_> {
    /// How many planned cards have no card at all. A card that names no
    /// planned deliverable still counts as one of them, but the runtime
    /// never says which: it did not see that.
    pub fn cards_short(&self) -> usize {
        self.cards_unnamed
            .len()
            .saturating_sub(self.cards_unattributed)
    }

    pub fn is_empty(&self) -> bool {
        self.cards_short() == 0 && self.others.is_empty()
    }
}

/// Whether the plan's receipts let the answer stand.
pub fn verdict(gap: &Gap) -> AnswerVerdict {
    if gap.is_empty() {
        AnswerVerdict::Accepted
    } else {
        AnswerVerdict::Held(Cause::Plan)
    }
}

/// Compares the plan with the receipts. A card covers the planned card its
/// `answers` names; the rest are compared by count alone.
pub fn gap<'a>(plan: &'a [Deliverable], produced: &Produced) -> Gap<'a> {
    let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    let mut covered = vec![false; plan.len()];
    let mut cards_unattributed = 0usize;
    for answers in produced.cards {
        let named = answers.as_deref().and_then(|name| {
            plan.iter().enumerate().position(|(at, deliverable)| {
                !covered[at]
                    && deliverable.form == DeliverableForm::Card
                    && same(&deliverable.name, name)
            })
        });
        match named {
            Some(at) => covered[at] = true,
            None => cards_unattributed += 1,
        }
    }
    let uncovered: Vec<&Deliverable> = plan
        .iter()
        .zip(&covered)
        .filter(|(_, covered)| !**covered)
        .map(|(deliverable, _)| deliverable)
        .collect();
    let (cards_unnamed, rest): (Vec<_>, Vec<_>) = uncovered
        .into_iter()
        .partition(|deliverable| deliverable.form == DeliverableForm::Card);
    Gap {
        cards_unnamed,
        cards_unattributed,
        others: rest
            .into_iter()
            .filter(|deliverable| match deliverable.form {
                DeliverableForm::Card => false,
                DeliverableForm::Text => produced.final_text.trim().is_empty(),
                DeliverableForm::File => !produced.file_delivered,
                DeliverableForm::Change => !produced.changed,
                DeliverableForm::Check => !produced.checked,
            })
            .collect(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn plan(entries: &[(&str, DeliverableForm)]) -> Vec<Deliverable> {
        entries
            .iter()
            .map(|(name, form)| Deliverable {
                name: (*name).into(),
                form: *form,
            })
            .collect()
    }

    fn produced<'a>(cards: &'a [Option<String>], text: &'a str) -> Produced<'a> {
        Produced {
            cards,
            final_text: text,
            file_delivered: false,
            changed: false,
            checked: false,
        }
    }

    #[test]
    fn a_card_covers_the_planned_card_it_names_and_the_rest_are_counted() {
        let declared = plan(&[
            ("routine", DeliverableForm::Card),
            ("comparison", DeliverableForm::Card),
        ]);
        let cards = [Some("Comparison".to_string())];
        let left = gap(&declared, &produced(&cards, ""));
        assert_eq!(left.cards_short(), 1);
        assert_eq!(left.cards_unnamed.len(), 1);
        assert_eq!(left.cards_unnamed[0].name, "routine");

        let cards = [None, None];
        assert!(gap(&declared, &produced(&cards, "")).is_empty());
    }

    /// Found live (run_01a1293b-45c3-7f21-8fbf-bff3e2f9d3c6): one card that
    /// named nothing was attributed to the first planned card by emit
    /// order, and the model was told the other was the one missing. The
    /// runtime did not see which it was, so it names both and the count.
    #[test]
    fn an_unattributed_card_is_counted_and_never_assigned_a_name() {
        let declared = plan(&[
            ("comparison", DeliverableForm::Card),
            ("plan", DeliverableForm::Card),
        ]);
        let cards = [Some("first week".to_string())];
        let left = gap(&declared, &produced(&cards, ""));
        assert_eq!(left.cards_short(), 1);
        assert_eq!(left.cards_unnamed.len(), 2);
        assert_eq!(left.cards_unattributed, 1);
        assert_eq!(verdict(&left), AnswerVerdict::Held(Cause::Plan));
    }

    #[test]
    fn each_form_is_covered_by_its_own_receipt() {
        let declared = plan(&[
            ("note", DeliverableForm::Text),
            ("fix", DeliverableForm::Change),
            ("proof", DeliverableForm::Check),
            ("report", DeliverableForm::File),
        ]);
        let none: [Option<String>; 0] = [];
        let left = gap(&declared, &produced(&none, " "));
        assert_eq!(left.others.len(), 4);
        let all = Produced {
            cards: &none,
            final_text: "Done: the parser now accepts empty input.",
            file_delivered: true,
            changed: true,
            checked: true,
        };
        assert_eq!(verdict(&gap(&declared, &all)), AnswerVerdict::Accepted);
    }
}
