//! Content-free timing inspection through the trash-aware session reader.
// A test's output is for the person running it.
#![allow(clippy::disallowed_macros)]
use vak_session::EntryPayload;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let cwd = args.next().ok_or("workspace required")?;
    let id = args.next().ok_or("session required")?;
    let core = vak_core::Core::new(cwd.into())?;
    let log = core.open_session_read_only(&id).await?;
    let index = vak_session::TurnIndex::from_log(&log);
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
    if args.next().is_some() {
        let profile = vak_context::capacity::CapacityProfile::from_metadata_only(
            None,
            0,
            "diagnostic".into(),
            std::time::SystemTime::now(),
        );
        let plan = vak_context::plan_for_session(&log, &profile, 0, 0);
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
