//! The runtime's check of a declared plan against what the turn produced
//! (docs/design/86-model-plans-runtime-checks.md §2.2). Pure: the loop
//! gathers the receipts, this decides which deliverables have none.

use vak_intent::DeliverableForm;
use vak_tools::Deliverable;

/// What the turn produced, as the loop observed it.
pub(crate) struct Produced<'a> {
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

/// The declared deliverables with no receipt, in plan order.
///
/// A card covers the card deliverable its `answers` names; a card that names
/// none, or names something the plan does not, covers the next uncovered
/// card deliverable in plan order.
pub(crate) fn missing<'a>(plan: &'a [Deliverable], produced: &Produced) -> Vec<&'a Deliverable> {
    let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    let mut covered = vec![false; plan.len()];
    let mut unnamed = 0usize;
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
            None => unnamed += 1,
        }
    }
    for (at, deliverable) in plan.iter().enumerate() {
        if covered[at] || deliverable.form != DeliverableForm::Card || unnamed == 0 {
            continue;
        }
        covered[at] = true;
        unnamed -= 1;
    }
    plan.iter()
        .enumerate()
        .filter(|(at, deliverable)| {
            !covered[*at]
                && match deliverable.form {
                    DeliverableForm::Card => true,
                    DeliverableForm::Text => produced.final_text.trim().is_empty(),
                    DeliverableForm::File => !produced.file_delivered,
                    DeliverableForm::Change => !produced.changed,
                    DeliverableForm::Check => !produced.checked,
                }
        })
        .map(|(_, deliverable)| deliverable)
        .collect()
}

/// The control message for deliverables still missing, in the model's own
/// words. It names what is missing and asks for nothing else.
pub(crate) fn missing_message(missing: &[&Deliverable]) -> String {
    let listed = missing
        .iter()
        .map(|deliverable| format!("\"{}\" ({})", deliverable.name, deliverable.form.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "[plan-check]: Your plan lists {listed}, and this turn has not produced it yet. \
         Produce it now, or say plainly why it cannot be done."
    )
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
    fn a_named_card_covers_its_deliverable_and_an_unnamed_one_the_next() {
        let declared = plan(&[
            ("routine", DeliverableForm::Card),
            ("comparison", DeliverableForm::Card),
        ]);
        let cards = [Some("Comparison".to_string())];
        let left = missing(&declared, &produced(&cards, ""));
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].name, "routine");

        let cards = [None, None];
        assert!(missing(&declared, &produced(&cards, "")).is_empty());
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
        assert_eq!(missing(&declared, &produced(&none, " ")).len(), 4);
        let all = Produced {
            cards: &none,
            final_text: "Done: the parser now accepts empty input.",
            file_delivered: true,
            changed: true,
            checked: true,
        };
        assert!(missing(&declared, &all).is_empty());
    }

    #[test]
    fn the_message_names_only_what_is_missing() {
        let declared = plan(&[("weekly routine", DeliverableForm::Card)]);
        let none: [Option<String>; 0] = [];
        let left = missing(&declared, &produced(&none, ""));
        let message = missing_message(&left);
        assert!(message.contains("\"weekly routine\" (card)"), "{message}");
    }
}
