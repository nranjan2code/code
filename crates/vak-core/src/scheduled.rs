//! A scheduled run's result, delivered into the conversation its
//! automation belongs to (docs/design/94 §4).

use vak_session::types::{
    MessageMeta, MessageRecord, PresentationRecord, PresentationSource, ScheduledMeta,
    TurnCardRecord,
};
use vak_session::{SessionError, SessionLog};

/// What a run delivers.
pub struct Delivery<'a> {
    pub meta: ScheduledMeta,
    /// The automation's own words, as the owner set them.
    pub prompt: &'a str,
    /// When it ran, in the owner's words (`2026-10-15 08:00 Asia/Kolkata`).
    pub when: &'a str,
    /// The run's answer, cards in their text form, or why it failed.
    pub text: &'a str,
    /// What the run's turns held (invariant 43): the delivered turn holds
    /// it too, so a reply to it is labelled as the run was.
    pub provenance: vak_control::Provenance,
    /// The cards the run showed, drawn as cards in the delivered turn; the
    /// answer's text carries their text form for the model.
    pub cards: Vec<PresentationRecord>,
}

/// Appends the delivery to `log` as a turn of its own, at a turn boundary
/// the caller holds: the automation's words as the directive and the run's
/// answer as the Agent's, both marked `scheduled`, closed by a turn card so
/// the next request follows it as it follows any turn. Returns the turn id.
pub fn deliver(log: &mut SessionLog, delivery: &Delivery<'_>) -> Result<String, SessionError> {
    let turn_id = uuid::Uuid::now_v7().to_string();
    log.begin_turn(&turn_id)?;
    let meta = || {
        Some(MessageMeta {
            scheduled: Some(delivery.meta.clone()),
            ..MessageMeta::default()
        })
    };
    let directive = format!(
        "Scheduled automation '{}', run at {}:\n{}",
        delivery.meta.name, delivery.when, delivery.prompt
    );
    log.append_message(MessageRecord {
        message: vak_llm::Message::user_text(directive),
        meta: meta(),
    })?;
    log.append_message(MessageRecord {
        message: vak_llm::Message::assistant(vec![vak_llm::ContentBlock::text(delivery.text)]),
        meta: meta(),
    })?;
    for card in &delivery.cards {
        let mut card = card.clone();
        card.turn_id = turn_id.clone();
        card.source = PresentationSource::Scheduled {
            run_session_id: delivery.meta.run_session.clone().unwrap_or_default(),
        };
        log.append_presentation(card)?;
    }
    if delivery.provenance != vak_control::Provenance::NONE {
        log.append_call_effect(vak_session::types::CallEffectRecord {
            tool_use_id: format!("scheduled:{}", delivery.meta.run),
            effect: vak_session::types::CallEffect::Provenance(delivery.provenance),
        })?;
    }
    let profile = vak_context::capacity::CapacityProfile::from_metadata_only(
        None,
        0,
        "scheduled-delivery".into(),
        std::time::SystemTime::now(),
    );
    let index = log.current_turn_index();
    if let Some(turn) = index.turn_by_id(&turn_id) {
        let outcome = if delivery.meta.failed {
            "failed"
        } else {
            "completed"
        };
        let card = turn.build_card(outcome, delivery.text.to_string(), &|text: &str| {
            profile.estimate_tokens(text.len() as u64)
        });
        log.append_turn_card(TurnCardRecord {
            turn_id: turn_id.clone(),
            card,
        })?;
    }
    Ok(turn_id)
}
