//! `vak effects` (plan M4.5): the effect records read directly, the way
//! `runs` reads its chain, and the owner's reconcile. Sending an effect
//! again needs the server's channel adapters, so it is the server's
//! `POST /effects/{id}/resend`, not a CLI verb. A record that cannot be
//! read is an error, never skipped.

use std::path::PathBuf;

use vak_core::Core;
use vak_session::effects::{EffectKind, EffectRecord, EffectStatus, Reconciled};
use vak_session::ids::{EffectId, RunId};

use crate::cli::{EffectsAction, ReconcileArg};

pub(crate) fn run_effects(cwd: PathBuf, action: Option<EffectsAction>) -> i32 {
    let core = match Core::new(cwd) {
        Ok(core) => core,
        Err(error) => {
            eprintln!("error: {error}");
            return 2;
        }
    };
    let effects = core.effects();
    let result = match action.unwrap_or(EffectsAction::List {
        status: None,
        run: None,
        limit: 50,
    }) {
        EffectsAction::List { status, run, limit } => list(&effects, status, run, limit),
        EffectsAction::Show { id } => parse(&id).and_then(|id| match effects.get(id) {
            Ok(Some(effect)) => serde_json::to_string_pretty(&effect)
                .map(|json| println!("{json}"))
                .map_err(|error| error.to_string()),
            Ok(None) => Err(format!("no effect {id}")),
            Err(error) => Err(error.to_string()),
        }),
        EffectsAction::Reconcile { id, outcome } => parse(&id).and_then(|id| {
            let outcome = match outcome {
                ReconcileArg::Sent => Reconciled::Sent,
                ReconcileArg::NotSent => Reconciled::NotSent,
            };
            effects
                .reconcile(id, outcome, None, vak_session::trace::local::local_owner())
                .map(|effect| println!("{}", row(&effect)))
                .map_err(|error| error.to_string())
        }),
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

fn parse(id: &str) -> Result<EffectId, String> {
    EffectId::parse(id).map_err(|_| format!("not an effect id: {id}"))
}

fn list(
    effects: &vak_session::effects::Effects,
    status: Option<String>,
    run: Option<String>,
    limit: usize,
) -> Result<(), String> {
    let status: Option<EffectStatus> = status
        .map(|text| {
            serde_json::from_value(serde_json::Value::String(text.to_ascii_lowercase()))
                .map_err(|_| format!("unknown status {text}"))
        })
        .transpose()?;
    let run = run
        .map(|text| RunId::parse(&text).map_err(|_| format!("not a run id: {text}")))
        .transpose()?;
    let all = effects.list().map_err(|error| error.to_string())?;
    let rows: Vec<_> = all
        .iter()
        .filter(|effect| status.is_none_or(|status| effect.status == status))
        .filter(|effect| run.is_none_or(|run| effect.run == Some(run)))
        .take(limit.max(1))
        .collect();
    if rows.is_empty() {
        println!("no effects");
    }
    for effect in rows {
        println!("{}", row(effect));
    }
    Ok(())
}

fn row(effect: &EffectRecord) -> String {
    let status = serde_json::to_value(effect.status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    let what = match &effect.kind {
        EffectKind::Delivery { surface, .. } => format!("delivery:{surface}"),
    };
    format!(
        "{}  {:<10} {:<18} {}  {}{}",
        effect.id,
        status,
        what,
        effect.prepared_at.format("%Y-%m-%d %H:%M:%S"),
        effect
            .run
            .map_or_else(|| "-".to_string(), |run| run.to_string()),
        effect
            .reason
            .as_deref()
            .map(|reason| format!("  ({reason})"))
            .unwrap_or_default()
    )
}
