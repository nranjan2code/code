//! Content-free timing inspection through the trash-aware session reader.
use vak_session::EntryPayload;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let cwd = args.next().ok_or("workspace required")?;
    let id = args.next().ok_or("session required")?;
    let core = vak_core::Core::new(cwd.into())?;
    let log = core.open_session_read_only(&id).await?;
    let mut index = vak_session::TurnIndex::from_log(&log);
    for (number, turn) in index.turns.iter().enumerate() {
        println!(
            "turn {} closed={} card={} chars={} costs={:?}",
            number + 1,
            turn.closed,
            turn.card.is_some(),
            vak_context::assemble::messages_chars(&turn.full_record()),
            turn.card
                .as_ref()
                .map(|card| (card.tokens_full, card.tokens_card))
        );
    }
    if let Some(query) = args.next() {
        let profile = vak_context::capacity::CapacityProfile::from_metadata_only(
            128_000,
            0,
            "diagnostic".into(),
            std::time::SystemTime::now(),
        );
        index.ensure_cards(&|text| profile.estimate_tokens(text.len() as u64));
        for (number, turn) in index.turns.iter_mut().enumerate() {
            let full_cost =
                profile.estimate_tokens(vak_context::assemble::messages_chars(&turn.full_record()));
            if let Some(card) = &mut turn.card {
                card.tokens_full = full_cost;
                card.tokens_card = profile.estimate_tokens(card.line(number + 1).len() as u64);
            }
        }
        let plan = vak_context::planner::plan(vak_context::planner::PlanInput {
            profile: &profile,
            index: &index,
            directive: &query,
            reading: None,
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        println!("plan {plan:?}");
        println!(
            "projected_chars={}",
            vak_context::assemble::messages_chars(&log.derive_with_plan(&plan))
        );
        return Ok(());
    }
    for entry in log.chain_to_root() {
        match &entry.payload {
            EntryPayload::Receipt(receipt) => {
                let mut receipt = receipt.clone();
                for attempt in &mut receipt.attempts {
                    attempt.error = attempt
                        .error
                        .as_ref()
                        .map(|_| "provider error (see failure domain)".into());
                }
                println!("{} receipt {}", entry.ts, serde_json::to_string(&receipt)?);
            }
            EntryPayload::Message(record) => {
                println!("{} message {:?}", entry.ts, record.message.role)
            }
            EntryPayload::Activity(record) => println!(
                "{} activity {:?} {:?}",
                entry.ts, record.kind, record.status
            ),
            _ => {}
        }
    }
    Ok(())
}
