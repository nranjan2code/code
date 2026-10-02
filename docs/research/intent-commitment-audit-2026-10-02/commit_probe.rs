#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use vak_commit::{Advancement, CommitmentLedger, Economics, Event, EventKind};
use vak_intent::{Authority, Declared, HistoryFacts, Reading, Request, ResolverConfig};

fn resolve(text: &str, history: HistoryFacts) -> vak_intent::Intent {
    vak_intent::resolve(
        &Request {
            text,
            turn_id: text,
            history,
            ..Request::default()
        },
        &Declared::default(),
        &Authority::default(),
        &ResolverConfig::default(),
    )
    .intent()
}

#[test]
fn print_domain_and_language_matrix() {
    for text in [
        "Write a poem every morning",
        "Write a poem tomorrow",
        "Remind me to call Mum tomorrow",
        "हर सुबह एक कविता लिखो",
        "Escribe un poema cada mañana",
        "每天早上写一首诗",
        "Research renewable energy every week and cite sources",
        "Draft a letter, then translate it into French, then summarize the budget",
        "Book a flight to Delhi",
        "Transfer 500 dollars to Alice",
        "Cancel my subscription",
    ] {
        let i = resolve(text, HistoryFacts::default());
        let spec = vak_intent::OutcomeSpec::from_intent(text, &i);
        println!(
            "{}",
            serde_json::json!({"text":text,"act":i.reading.act,"horizon":i.reading.horizon,"stakes":i.reading.stakes,"confidence":i.reading.confidence,"strands":i.strands.len(),"requirements":spec.requirements,"open_commitment":i.engagement.posture.open_commitment})
        );
    }
}

#[test]
fn unrelated_requests_can_share_a_commitment_thread() {
    let first = resolve("Write a poem every morning", HistoryFacts::default());
    let s = &first.strands[0];
    let second = resolve(
        "Write a resignation letter",
        HistoryFacts {
            open_threads: vec![vak_intent::ThreadFact {
                thread_id: s.thread_id.clone(),
                act: s.reading.act,
                domains: s.reading.domains.clone(),
                keywords: vak_intent::strand::keywords(&s.text),
            }],
            ..HistoryFacts::default()
        },
    );
    println!("lineage={:?}", second.strands[0].lineage);
    assert_eq!(second.strands[0].thread_id, s.thread_id);
}

#[test]
fn duplicate_and_unknown_episode_settlements_change_spend() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let id = ledger
        .open_commitment(vak_commit::spec_from_reading(
            "audit",
            Reading::general(),
            vec![],
            dir.path().into(),
            Economics::default(),
        ))
        .unwrap();
    ledger
        .append(&Event::new(
            &id,
            EventKind::EpisodeStarted {
                episode_id: "episode".into(),
                session_id: "session".into(),
            },
        ))
        .unwrap();
    let event = Event::new(
        &id,
        EventKind::EpisodeEnded {
            episode_id: "episode".into(),
            advancement: Advancement::Learned {
                fact: "same receipt".into(),
            },
            spend_usd: 2.0,
        },
    );
    ledger.append(&event).unwrap();
    ledger.append(&event).unwrap();
    assert_eq!(ledger.get(&id).unwrap().unwrap().spend_usd, 4.0);
    ledger
        .append(&Event::new(
            &id,
            EventKind::EpisodeEnded {
                episode_id: "unknown".into(),
                advancement: Advancement::Learned {
                    fact: "unknown".into(),
                },
                spend_usd: -3.0,
            },
        ))
        .unwrap();
    assert_eq!(ledger.get(&id).unwrap().unwrap().spend_usd, 1.0);
    println!("duplicate counted twice; unknown negative settlement reduced spend to $1");
}

#[test]
fn multi_result_contract_can_be_met_by_one_unrelated_sentence() {
    let text = "Draft a letter, then translate it into French, then summarize the budget";
    let i = resolve(text, HistoryFacts::default());
    assert!(i.strands.len() >= 3);
    let spec = vak_intent::OutcomeSpec::from_intent(text, &i);
    assert_eq!(spec.requirements.len(), 1);
    let evaluated = vak_intent::evaluate_requirements_with_state(
        &spec,
        Some("The weather is pleasant."),
        vak_intent::EvidenceState::None,
    );
    assert_eq!(evaluated[0].status, vak_intent::RequirementStatus::Met);
    assert_eq!(
        vak_intent::evaluate_completion(vak_intent::OutcomeStatus::Produced, &evaluated, &spec),
        vak_intent::CompletionVerdict::Complete
    );
    println!(
        "{} strands collapse to one met deliverable",
        i.strands.len()
    );
}
