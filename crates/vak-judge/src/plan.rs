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
    /// How many distinct files were delivered for review or declared as
    /// artifacts. Each planned file needs one: two drafts of one workbook
    /// once stood for a planned report and a planned workbook.
    pub files_delivered: usize,
    /// A file changed, a command ran, or an integration acted.
    pub changed: bool,
    /// A command ran and nothing changed after it.
    pub checked: bool,
    /// The turn can show no card any more (the tool that shows them was
    /// withdrawn): a planned card is then given as text, and checked so.
    pub cards_withdrawn: bool,
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
    /// How many planned cards no card names. A card that names no planned
    /// deliverable covers none of them: two such cards answering another
    /// request once stood for a plan of two
    /// (docs/audits/cards-in-answers-2026-10-11.md, F4).
    pub fn cards_short(&self) -> usize {
        self.cards_unnamed.len()
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

/// Compares the plan with the receipts. A card covers the planned card or
/// text its `answers` names, and only that one; a card that names a planned
/// change, check or file is part of it and covers nothing.
pub fn gap<'a>(plan: &'a [Deliverable], produced: &Produced) -> Gap<'a> {
    let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    let mut covered = vec![false; plan.len()];
    let mut cards_unattributed = 0usize;
    for answers in produced.cards {
        let Some(name) = answers.as_deref() else {
            cards_unattributed += 1;
            continue;
        };
        let covers = plan.iter().enumerate().position(|(at, deliverable)| {
            !covered[at]
                && matches!(
                    deliverable.form,
                    DeliverableForm::Card | DeliverableForm::Text
                )
                && same(&deliverable.name, name)
        });
        match covers {
            Some(at) => covered[at] = true,
            // A card that shows a planned change, check or file is part of
            // that entry, and covers none of it: the entry still needs its
            // own receipt.
            None if plan.iter().any(|deliverable| same(&deliverable.name, name)) => {}
            None => cards_unattributed += 1,
        }
    }
    let uncovered: Vec<&Deliverable> = plan
        .iter()
        .zip(&covered)
        .filter(|(_, covered)| !**covered)
        .map(|(deliverable, _)| deliverable)
        .collect();
    let (cards_unnamed, rest): (Vec<_>, Vec<_>) = uncovered.into_iter().partition(|deliverable| {
        deliverable.form == DeliverableForm::Card && !produced.cards_withdrawn
    });
    let mut files_left = produced.files_delivered;
    Gap {
        cards_unnamed,
        cards_unattributed,
        others: rest
            .into_iter()
            .filter(|deliverable| match deliverable.form {
                DeliverableForm::Card | DeliverableForm::Text => {
                    produced.final_text.trim().is_empty()
                }
                DeliverableForm::File => match files_left.checked_sub(1) {
                    Some(left) => {
                        files_left = left;
                        false
                    }
                    None => true,
                },
                DeliverableForm::Change => !produced.changed,
                DeliverableForm::Check => !produced.checked,
            })
            .collect(),
    }
}

/// Whether an answer to `plan` makes claims that could rest on material it
/// retrieved: no plan, a text, a file, or a card the runtime does not fill
/// from calls (`filled_by_calls` names the card kinds it does: a diff, a
/// terminal, a test report). A plan of changes, checks and such cards rests
/// on the runtime's own receipts instead (found live 2026-10-19: a coding
/// turn that read the file it fixed was marked a partial result).
pub fn makes_claims(plan: &[Deliverable], filled_by_calls: impl Fn(&str) -> bool) -> bool {
    plan.is_empty()
        || plan.iter().any(|deliverable| match deliverable.form {
            DeliverableForm::Text | DeliverableForm::File => true,
            DeliverableForm::Card => !deliverable.kind.as_deref().is_some_and(&filled_by_calls),
            DeliverableForm::Change | DeliverableForm::Check => false,
        })
}

/// Evidence questions are separate from output receipts. A recorded answer
/// satisfies delivery only; source relevance and support remain to be judged.
/// `retrieved` says the turn retrieved material its claims may rest on
/// ([`makes_claims`]).
pub fn evidence_requirements(
    plan: &[Deliverable],
    retrieved: bool,
) -> Vec<vak_intent::OutcomeRequirement> {
    let mut requirements: Vec<_> = plan
        .iter()
        .enumerate()
        .flat_map(|(part, deliverable)| {
            deliverable
                .questions
                .iter()
                .enumerate()
                .map(move |(question, text)| vak_intent::OutcomeRequirement {
                    id: format!("plan-evidence-{}-{}", part + 1, question + 1),
                    kind: vak_intent::RequirementKind::Evidence,
                    description: text.clone(),
                    origin: vak_intent::RequirementOrigin::Declared,
                    importance: vak_intent::RequirementImportance::Must,
                    target: Some(deliverable.name.clone()),
                    form: None,
                })
        })
        .collect();
    // Even when no plan declared the questions, transport receipts cannot
    // establish that an answer relying on retrieved material is supported.
    if requirements.is_empty() && retrieved {
        requirements.push(vak_intent::OutcomeRequirement {
            id: "retrieved-source-support".into(),
            kind: vak_intent::RequirementKind::Evidence,
            description:
                "Establish relevance, claim support and source freshness for the retrieved material"
                    .into(),
            origin: vak_intent::RequirementOrigin::Inferred,
            importance: vak_intent::RequirementImportance::Must,
            target: Some("primary".into()),
            form: None,
        });
    }
    requirements
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn plan(entries: &[(&str, DeliverableForm)]) -> Vec<Deliverable> {
        entries
            .iter()
            .map(|(name, form)| Deliverable {
                questions: Vec::new(),
                kind: None,
                name: (*name).into(),
                form: *form,
            })
            .collect()
    }

    fn produced<'a>(cards: &'a [Option<String>], text: &'a str) -> Produced<'a> {
        Produced {
            cards,
            final_text: text,
            files_delivered: 0,
            changed: false,
            checked: false,
            cards_withdrawn: false,
        }
    }

    #[test]
    fn delivery_receipts_do_not_establish_evidence_questions() {
        assert!(evidence_requirements(&[], false).is_empty());
        assert_eq!(evidence_requirements(&[], true).len(), 1);
        let mut declared = plan(&[("findings", DeliverableForm::Card)]);
        declared[0].questions = vec!["Which effects were independently measured?".into()];
        let cards = [Some("findings".into())];
        assert!(gap(&declared, &produced(&cards, "")).is_empty());
        let mut spec = vak_intent::OutcomeSpec::from_reading(
            "investigate",
            &vak_intent::Reading::general(),
            1,
        );
        spec.requirements
            .extend(evidence_requirements(&declared, true));
        let facts = crate::outcome::TurnFacts {
            cards_recorded: 1,
            evidence_card: true,
            evidence_state: vak_intent::EvidenceState::Fresh,
            ..crate::outcome::TurnFacts::reply(None)
        };
        let evaluations = crate::outcome::evaluate_requirements(&spec, &facts);
        assert_eq!(
            evaluations[1].status,
            vak_intent::RequirementStatus::Unknown
        );
        assert_eq!(
            crate::outcome::evaluate_completion(
                vak_intent::OutcomeStatus::Produced,
                &evaluations,
                &spec
            ),
            vak_intent::CompletionVerdict::Unknown
        );
        assert_eq!(spec.requirements[1].target.as_deref(), Some("findings"));
    }

    #[test]
    fn a_planned_card_is_checked_as_text_once_no_card_can_be_shown() {
        let declared = plan(&[("comparison", DeliverableForm::Card)]);
        let mut seen = produced(&[], "Obsidian is local; Notion is shared.");
        assert_eq!(gap(&declared, &seen).cards_short(), 1);
        seen.cards_withdrawn = true;
        assert!(gap(&declared, &seen).is_empty());
        seen.final_text = " ";
        let left = gap(&declared, &seen);
        assert_eq!((left.cards_short(), left.others.len()), (0, 1));
    }

    #[test]
    fn a_card_covers_only_the_planned_card_it_names() {
        let declared = plan(&[
            ("routine", DeliverableForm::Card),
            ("comparison", DeliverableForm::Card),
        ]);
        let cards = [Some("Comparison".to_string())];
        let left = gap(&declared, &produced(&cards, ""));
        assert_eq!(left.cards_short(), 1);
        assert_eq!(left.cards_unnamed.len(), 1);
        assert_eq!(left.cards_unnamed[0].name, "routine");

        // Two cards that name nothing cover neither planned card: two such
        // cards answering another request once stood for a plan of two.
        let cards = [None, None];
        assert_eq!(gap(&declared, &produced(&cards, "")).cards_short(), 2);
    }

    /// Found live (run_01a1293b-45c3-7f21-8fbf-bff3e2f9d3c6): one card that
    /// named nothing was attributed to the first planned card by emit
    /// order. A card is never assigned a name the runtime did not see, and
    /// one that names no planned card covers none of them.
    #[test]
    fn an_unattributed_card_is_counted_and_never_assigned_a_name() {
        let declared = plan(&[
            ("comparison", DeliverableForm::Card),
            ("plan", DeliverableForm::Card),
        ]);
        let cards = [Some("first week".to_string())];
        let left = gap(&declared, &produced(&cards, ""));
        assert_eq!(left.cards_short(), 2);
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
            files_delivered: 1,
            changed: true,
            checked: true,
            cards_withdrawn: false,
        };
        assert_eq!(verdict(&gap(&declared, &all)), AnswerVerdict::Accepted);
    }

    #[test]
    fn a_card_delivers_the_text_deliverable_it_names() {
        let plan = plan(&[
            ("route", DeliverableForm::Text),
            ("link", DeliverableForm::Text),
        ]);
        let cards = [Some("route".to_string())];
        let gap = gap(&plan, &produced(&cards, ""));
        assert_eq!(gap.cards_unattributed, 0);
        let left: Vec<&str> = gap.others.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(left, ["link"], "the card covers only the text it names");
    }

    /// Found live 2026-10-19: a diff card naming the planned change was
    /// refused three times and never shown. It is part of the change, and
    /// the change still needs a change.
    #[test]
    fn a_card_that_shows_a_planned_change_is_attributed_and_covers_nothing() {
        let declared = plan(&[
            ("parser fix", DeliverableForm::Change),
            ("tests pass", DeliverableForm::Check),
        ]);
        let cards = [
            Some("parser fix".to_string()),
            Some("Tests pass".to_string()),
        ];
        let left = gap(&declared, &produced(&cards, ""));
        assert_eq!(left.cards_unattributed, 0);
        assert_eq!(left.others.len(), 2, "a card is no change and no check");
    }

    #[test]
    fn a_plan_of_changes_checks_and_call_filled_cards_makes_no_claims() {
        let filled = |kind: &str| matches!(kind, "diff" | "terminal" | "test_report");
        let mut coding = plan(&[
            ("fix", DeliverableForm::Change),
            ("tests pass", DeliverableForm::Check),
            ("the diff", DeliverableForm::Card),
        ]);
        coding[2].kind = Some("diff".into());
        assert!(!makes_claims(&coding, filled));
        // A card of no named kind, a text, a file or no plan at all may
        // rest on what was retrieved.
        coding[2].kind = None;
        assert!(makes_claims(&coding, filled));
        for form in [DeliverableForm::Text, DeliverableForm::File] {
            assert!(makes_claims(&plan(&[("part", form)]), filled));
        }
        assert!(makes_claims(&[], filled));
    }

    #[test]
    fn each_planned_file_needs_a_file_of_its_own() {
        let declared = plan(&[
            ("report document", DeliverableForm::File),
            ("workbook", DeliverableForm::File),
        ]);
        let none: [Option<String>; 0] = [];
        let one = Produced {
            files_delivered: 1,
            ..produced(&none, "The workbook is ready.")
        };
        let left = gap(&declared, &one);
        assert_eq!(left.others.len(), 1);
        assert_eq!(left.others[0].name, "workbook");
        let two = Produced {
            files_delivered: 2,
            ..produced(&none, "Both are ready.")
        };
        assert_eq!(verdict(&gap(&declared, &two)), AnswerVerdict::Accepted);
    }
}
