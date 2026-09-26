//! `vak intent` and `vak commit`.
//!
//! `intent explain` is the load-bearing one. Every later phase of this work —
//! capability slicing, approval posture, route demand — changes behaviour on
//! the strength of a reading, and a reading nobody can inspect is a reading
//! nobody can debug. This prints the whole decision: each signal with the
//! weight it carried, the axes it produced, and precisely what the engagement
//! narrows relative to doing nothing.
//!
//! It costs nothing and dispatches nothing, so it is safe to run against any
//! prompt before committing to it.

use vak_commit::{CommitmentLedger, Verdict};
use vak_core::Core;
use vak_intent::{Act, Declared, Evidence, Horizon, Limits, Stakes, Surface as IntentSurface};

use crate::cli::{CommitAction, IntentAction};

pub(crate) fn run_intent(cwd: std::path::PathBuf, action: IntentAction) -> i32 {
    let core = match Core::new(cwd) {
        Ok(core) => core.with_surface(vak_core::Surface::Cli),
        Err(error) => {
            eprintln!("error: {error}");
            return 2;
        }
    };
    dispatch_intent(&core, action)
}

fn dispatch_intent(core: &Core, action: IntentAction) -> i32 {
    match action {
        IntentAction::Show => show_policy(core),
        IntentAction::Explain {
            prompt,
            surface,
            act,
            horizon,
            stakes,
            evidence,
            json,
        } => explain(core, &prompt, surface, act, horizon, stakes, evidence, json),
    }
}

fn show_policy(core: &Core) -> i32 {
    let config = core.config();
    println!("intent kernel");
    println!("  enabled              {}", config.intent.enabled);
    println!(
        "  accept confidence    {:.2}",
        config.intent.accept_confidence
    );
    println!(
        "  provisional          {:.2}",
        config.intent.provisional_confidence
    );
    println!(
        "  capability slicing   {}",
        config.intent.slice_capabilities
    );
    println!("  approval posture     {}", config.intent.posture);
    println!("  escalation           {}", config.intent.escalate);
    if config.intent.escalate == "cloud" {
        println!(
            "  classification cap   ${:.4} per dispatch",
            config.intent.max_classify_usd
        );
        println!(
            "  classification model {}",
            config
                .intent
                .classify_model
                .as_deref()
                .unwrap_or("(cheapest leg on the frozen ladder)")
        );
    }
    println!("  autonomy             {}", config.intent.autonomy);
    // Did we read it right? The misread ledger's per-cell accuracy, so the
    // loop closes on a person rather than on nothing.
    let ledger = vak_core::misread::MisreadLedger::new(&core.sessions_home());
    let cells = ledger.accuracy();
    let observed: u64 = cells
        .iter()
        .filter(|cell| cell.resolver_version == vak_intent::RESOLVER_VERSION)
        .map(|cell| cell.observations())
        .sum();
    println!(
        "  readings observed    {observed} (resolver v{})",
        vak_intent::RESOLVER_VERSION
    );
    let weak = vak_core::misread::weak_cells(&ledger, 5);
    if weak.is_empty() {
        println!(
            "  weak cells           none (nothing contradicted, fewer than 5 observations, \
             or accuracy ≥ 0.75)"
        );
    } else {
        for cell in weak {
            let wanted = cell
                .wanted
                .iter()
                .take(3)
                .map(|(name, count)| format!("{name}×{count}"))
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "  weak cell            {}/{} accuracy {:.2} over {} — model asked for: {}",
                cell.act,
                cell.stakes,
                cell.accuracy(),
                cell.observations(),
                if wanted.is_empty() {
                    "-".to_string()
                } else {
                    wanted
                }
            );
        }
    }
    println!();
    println!("commitments");
    println!("  enabled              {}", config.commitment.enabled);
    println!("  stall limit          {}", config.commitment.stall_limit);
    match config.commitment.lifetime_budget_usd {
        Some(budget) => println!("  lifetime budget      ${budget:.2}"),
        None => println!("  lifetime budget      (none)"),
    }
    match config.commitment.default_ttl_days {
        Some(days) => println!("  default TTL          {days} day(s)"),
        None => println!("  default TTL          (none)"),
    }
    0
}

#[allow(clippy::too_many_arguments)]
fn explain(
    core: &Core,
    prompt: &str,
    surface: Option<String>,
    act: Option<String>,
    horizon: Option<String>,
    stakes: Option<String>,
    evidence: Option<String>,
    json: bool,
) -> i32 {
    let mut declared = Declared::default();
    // An unparseable override is an error rather than a silent fallback: the
    // whole point of this command is to answer "what would happen", and
    // quietly ignoring half the question makes the answer wrong.
    macro_rules! parse_override {
        ($value:expr, $parser:path, $name:literal, $target:expr) => {
            if let Some(raw) = $value {
                match $parser(&raw) {
                    Some(parsed) => $target = Some(parsed),
                    None => {
                        eprintln!("error: unknown {} '{}'", $name, raw);
                        return 2;
                    }
                }
            }
        };
    }
    parse_override!(act, Act::parse, "act", declared.act);
    parse_override!(horizon, Horizon::parse, "horizon", declared.horizon);
    parse_override!(stakes, Stakes::parse, "stakes", declared.stakes);
    parse_override!(evidence, Evidence::parse, "evidence", declared.evidence);

    let core_surface = match surface.as_deref() {
        None => core.surface().clone(),
        Some(raw) => match IntentSurface::parse(raw) {
            Some(IntentSurface::Cli) => vak_core::Surface::Cli,
            Some(IntentSurface::Desktop) => vak_core::Surface::Desktop,
            Some(IntentSurface::Server) => vak_core::Surface::Server,
            Some(IntentSurface::Chat) => vak_core::Surface::Chat {
                channel: "chat".into(),
            },
            Some(IntentSurface::Cron | IntentSurface::Heartbeat) => vak_core::Surface::Background,
            Some(IntentSurface::Worker) => vak_core::Surface::Worker,
            None => {
                eprintln!("error: unknown surface '{raw}'");
                return 2;
            }
        },
    };

    let authority = core.turn_authority_for(&core_surface);
    let resolution = vak_core::intent::resolve_turn(
        prompt,
        "",
        &core_surface,
        &[],
        vak_core::intent::workspace_facts(core.cwd()),
        vak_intent::HistoryFacts::default(),
        &declared,
        &authority,
        &vak_core::intent::resolver_config(core.config()),
    );
    let escalation = match &resolution {
        vak_intent::Resolution::Escalate { reason, .. } => Some(reason.clone()),
        vak_intent::Resolution::Settled(_) => None,
    };
    let intent = resolution.intent();

    if json {
        let value = serde_json::json!({
            "reading": intent.reading,
            "strands": intent.strands,
            "engagement": intent.engagement,
            "provenance": intent.provenance,
            "narrows": intent.engagement.limits.diff_from(&Limits::unrestricted()),
            "escalation_recommended": escalation,
            "model_visible": intent.model_visible(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
        );
        return 0;
    }

    if intent.strands.len() > 1 {
        println!("parts ({})", intent.strands.len());
        for (index, strand) in intent.strands.iter().enumerate() {
            let relation = match &strand.relation {
                vak_intent::StrandRelation::Independent => String::new(),
                vak_intent::StrandRelation::Sequential { after } => {
                    format!(", after {after}")
                }
                vak_intent::StrandRelation::Dependent { on } => format!(", uses {on}"),
            };
            let lineage = match &strand.lineage {
                vak_intent::Lineage::New => String::new(),
                vak_intent::Lineage::Continues { thread_id } => {
                    format!(", continues {thread_id}")
                }
                vak_intent::Lineage::Corrects { thread_id } => {
                    format!(", corrects {thread_id}")
                }
                vak_intent::Lineage::Replaces { thread_id } => {
                    format!(", replaces {thread_id}")
                }
            };
            println!(
                "  {}. [{}] {}/{}/{}/{} {:.0}%{relation}{lineage}  “{}”",
                index + 1,
                strand.strand_id,
                strand.reading.act.as_str(),
                strand.reading.horizon.as_str(),
                strand.reading.stakes.as_str(),
                strand.reading.evidence.as_str(),
                strand.reading.confidence * 100.0,
                strand.text
            );
        }
        println!();
    }

    let reading = &intent.reading;
    println!("reading (composite)");
    println!("  act          {}", reading.act.as_str());
    println!("  horizon      {}", reading.horizon.as_str());
    println!("  stakes       {}", reading.stakes.as_str());
    println!("  evidence     {}", reading.evidence.as_str());
    println!("  clarity      {}", reading.clarity.as_str());
    println!("  attendance   {}", reading.attendance.as_str());
    if !reading.domains.is_empty() {
        println!(
            "  domains      {}",
            reading
                .domains
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!(
        "  confidence   {:.2} overall (act {:.2}, horizon {:.2}, stakes {:.2}, evidence {:.2})",
        reading.confidence,
        reading.axis_confidence.act,
        reading.axis_confidence.horizon,
        reading.axis_confidence.stakes,
        reading.axis_confidence.evidence
    );

    println!();
    println!(
        "resolved by  {} ({})",
        intent.provenance.tier.as_str(),
        if intent.provenance.reproducible {
            "reproducible from this ledger"
        } else {
            "NOT reproducible — a model decided it"
        }
    );
    if let Some(model) = &intent.provenance.model {
        println!("  model        {model}");
    }
    if let Some(note) = &intent.provenance.escalation_note {
        println!("  note         {note}");
    }
    if let Some(reason) = &escalation {
        println!("  escalation   recommended: {reason}");
    }

    println!();
    println!("signals");
    if intent.provenance.signals.is_empty() {
        println!("  (none — nothing in this request matched the lexicon)");
    }
    for signal in &intent.provenance.signals {
        println!(
            "  {:<11} {:>5.2}  {}",
            signal.kind.as_str(),
            signal.weight,
            signal.detail
        );
    }

    println!();
    println!("engagement (relative to doing nothing)");
    let narrows = intent.engagement.limits.diff_from(&Limits::unrestricted());
    if narrows.is_empty() {
        println!("  nothing narrowed — this is the general engagement");
    }
    for line in &narrows {
        println!("  - {line}");
    }

    let posture = &intent.engagement.posture;
    println!();
    println!("posture");
    println!(
        "  work mode    {}",
        if posture.managed { "managed" } else { "direct" }
    );
    println!("  commitment   {}", posture.open_commitment);
    println!("  checkpoint   {}", posture.checkpoint_before_effect);
    println!("  human loop   {}", posture.hil.as_str());
    println!("  clarify      {}", posture.clarify.as_str());
    println!("  stop when    {}", posture.stop.as_str());
    println!("  context      {}", posture.context.as_str());
    println!(
        "  deliver      {} / {} / {}",
        posture.delivery.shape.as_str(),
        posture.delivery.cadence.as_str(),
        posture.delivery.urgency.as_str()
    );
    println!(
        "  demand       reasoning={} evidence={} structured={}",
        posture.demand.reasoning_required,
        posture.demand.evidence_required,
        posture.demand.structured_output
    );

    if let Some(note) = intent.model_visible() {
        println!();
        println!("the model will additionally be told");
        for line in note.lines() {
            println!("  {line}");
        }
    }
    0
}

// ------------------------------------------------------------- commit ---

pub(crate) fn run_commit(cwd: std::path::PathBuf, action: CommitAction) -> i32 {
    let core = match Core::new(cwd) {
        Ok(core) => core,
        Err(error) => {
            eprintln!("error: {error}");
            return 2;
        }
    };
    dispatch_commit(&core, action)
}

fn dispatch_commit(core: &Core, action: CommitAction) -> i32 {
    let ledger = CommitmentLedger::new(&core.sessions_home());
    match action {
        CommitAction::List { all, json } => list(&ledger, all, json),
        CommitAction::Show { id, json } => show(&ledger, &id, json),
        CommitAction::Close { id, verdict, note } => close(&ledger, &id, &verdict, &note),
        CommitAction::Supersede { id, by, reason } => supersede(&ledger, &id, &by, &reason),
        CommitAction::Attest {
            id,
            criterion,
            note,
        } => attest(&ledger, &id, &criterion, &note),
    }
}

fn list(ledger: &CommitmentLedger, all: bool, json: bool) -> i32 {
    let commitments = if all { ledger.all() } else { ledger.open() };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&commitments).unwrap_or_else(|_| "[]".into())
        );
        return 0;
    }
    if commitments.is_empty() {
        println!("no commitments");
        return 0;
    }
    // Ordered by the same scheduler the runtime uses, so this listing answers
    // "what happens next" rather than merely "what exists".
    let ranked = vak_commit::rank(&commitments, &vak_commit::SchedulerContext::default());
    for priority in ranked {
        let Some(commitment) = commitments
            .iter()
            .find(|c| c.commitment_id == priority.commitment_id)
        else {
            continue;
        };
        println!(
            "{}  {}",
            &commitment.commitment_id[..8.min(commitment.commitment_id.len())],
            commitment.summary()
        );
        println!("          {}", priority.explain());
    }
    0
}

fn show(ledger: &CommitmentLedger, id: &str, json: bool) -> i32 {
    let Some(commitment) = resolve_id(ledger, id) else {
        eprintln!("error: no commitment matching '{id}'");
        return 2;
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&commitment).unwrap_or_else(|_| "{}".into())
        );
        return 0;
    }
    println!("{}", commitment.spec.objective);
    println!("  id           {}", commitment.commitment_id);
    println!("  phase        {}", commitment.phase.as_str());
    println!("  opened       {}", commitment.opened_at.to_rfc3339());
    println!(
        "  requires     {} evidence to close fulfilled",
        commitment.spec.min_satisfaction.as_str()
    );
    println!(
        "  achieved     {} evidence",
        commitment.achieved_strength().as_str()
    );
    println!("  spend        ${:.4}", commitment.spend_usd);
    if let Some(suspension) = &commitment.suspension {
        println!("  waiting      {}", suspension.describe());
    }
    if let Some(blocker) = &commitment.blocker {
        println!("  blocked      {blocker}");
    }
    if commitment.is_stalled() {
        println!(
            "  stalled      {} consecutive episodes made no progress",
            commitment.consecutive_stalls
        );
    }
    if !commitment.drift.is_empty() {
        println!("  drift        {}", commitment.drift.join("; "));
    }
    if !commitment.criteria.is_empty() {
        println!("  criteria");
        for criterion in &commitment.criteria {
            let mark = if criterion.passed() { "PASS" } else { "open" };
            println!(
                "    [{mark}] {} ({})",
                criterion.statement,
                criterion
                    .strength
                    .map(|s| s.as_str())
                    .unwrap_or("not evaluated")
            );
        }
    }
    if !commitment.episodes.is_empty() {
        println!("  episodes");
        for episode in &commitment.episodes {
            println!(
                "    {} {} ${:.4}",
                episode.session_id,
                episode
                    .advancement
                    .as_ref()
                    .map(|a| a.as_str())
                    .unwrap_or("running"),
                episode.spend_usd
            );
        }
    }
    if let Some(closure) = &commitment.closure {
        println!(
            "  closed       {} ({} evidence) — {}",
            closure.verdict.as_str(),
            closure.strength.as_str(),
            closure.note
        );
    }
    0
}

fn close(ledger: &CommitmentLedger, id: &str, verdict: &str, note: &str) -> i32 {
    let Some(commitment) = resolve_id(ledger, id) else {
        eprintln!("error: no commitment matching '{id}'");
        return 2;
    };
    let verdict = match verdict {
        "fulfilled" => Verdict::Fulfilled,
        "partial" => Verdict::Partial,
        "failed" => Verdict::Failed,
        "abandoned" => Verdict::Abandoned,
        "expired" => Verdict::Expired,
        "unknown" => Verdict::Unknown,
        other => {
            eprintln!(
                "error: unknown verdict '{other}' \
                 (fulfilled, partial, failed, abandoned, expired, unknown)"
            );
            return 2;
        }
    };
    let strength = commitment.achieved_strength();
    match ledger.append(&vak_commit::Event::new(
        &commitment.commitment_id,
        vak_commit::EventKind::Closed {
            verdict,
            strength,
            evidence: Vec::new(),
            note: note.to_string(),
        },
    )) {
        Ok(()) => {
            println!(
                "closed {} as {}",
                &commitment.commitment_id[..8.min(commitment.commitment_id.len())],
                verdict.as_str()
            );
            0
        }
        Err(error) => {
            // The refusal explains itself: this is where the closure invariant
            // becomes visible to a person rather than staying an internal rule.
            eprintln!("error: {error}");
            2
        }
    }
}

fn supersede(ledger: &CommitmentLedger, id: &str, by: &str, reason: &str) -> i32 {
    let (Some(old), Some(new)) = (resolve_id(ledger, id), resolve_id(ledger, by)) else {
        eprintln!("error: both the superseded and the replacing commitment must exist");
        return 2;
    };
    match ledger.append(&vak_commit::Event::new(
        &old.commitment_id,
        vak_commit::EventKind::Superseded {
            by: new.commitment_id.clone(),
            reason: reason.to_string(),
        },
    )) {
        Ok(()) => {
            println!("superseded by {}", new.commitment_id);
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            2
        }
    }
}

fn attest(ledger: &CommitmentLedger, id: &str, criterion: &str, note: &str) -> i32 {
    let Some(commitment) = resolve_id(ledger, id) else {
        eprintln!("error: no commitment matching '{id}'");
        return 2;
    };
    if !commitment
        .criteria
        .iter()
        .any(|c| c.criterion_id == criterion)
    {
        eprintln!("error: commitment has no criterion '{criterion}'");
        return 2;
    }
    let by = std::env::var("USER").unwrap_or_else(|_| "operator".into());
    let evaluation = vak_commit::Evaluation::attested(criterion, &by, note);
    match ledger.append(&vak_commit::Event::new(
        &commitment.commitment_id,
        vak_commit::EventKind::CriterionEvaluated {
            criterion_id: evaluation.criterion_id.clone(),
            result: evaluation.result.clone(),
            strength: evaluation.strength,
        },
    )) {
        Ok(()) => {
            println!("attested '{criterion}' as {by}");
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            2
        }
    }
}

/// Resolve a full id or an unambiguous prefix.
fn resolve_id(ledger: &CommitmentLedger, id: &str) -> Option<vak_commit::Commitment> {
    let all = ledger.all();
    if let Some(exact) = all.iter().find(|c| c.commitment_id == id) {
        return Some(exact.clone());
    }
    let mut matches = all
        .iter()
        .filter(|c| c.commitment_id.starts_with(id))
        .cloned();
    let first = matches.next()?;
    // An ambiguous prefix resolves to nothing rather than to whichever row
    // happened to sort first.
    if matches.next().is_some() {
        return None;
    }
    Some(first)
}

// -------------------------------------------------------------- grants ---

/// Delegate authority to one commitment.
///
/// An envelope is **pre-authorization within existing authority**, never a
/// grant of new authority: its permission ceiling can only lower the mode
/// already in force, and irreversible work still reaches a human whatever was
/// delegated. What it buys is silence on the ordinary case — a `delegated`
/// agent stops asking about the things you already said yes to, inside the
/// boundary you drew.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_grant(
    cwd: std::path::PathBuf,
    id: String,
    paths: Vec<String>,
    tools: Vec<String>,
    spend_usd: Option<f64>,
    hours: Option<i64>,
    permission: String,
    on_silence: String,
    after_hours: u32,
) -> i32 {
    let core = match Core::new(cwd) {
        Ok(core) => core,
        Err(error) => {
            eprintln!("error: {error}");
            return 2;
        }
    };
    let ledger = CommitmentLedger::new(&core.sessions_home());
    let Some(commitment) = resolve_id(&ledger, &id) else {
        eprintln!("error: no commitment matching '{id}'");
        return 2;
    };
    let Some(ceiling) = vak_intent::PermissionCeiling::parse(&permission) else {
        eprintln!(
            "error: unknown permission '{permission}' \
             (read-only, workspace-write, full-access)"
        );
        return 2;
    };
    let escalation = match on_silence.as_str() {
        "wait" => vak_intent::Escalation::WaitIndefinitely,
        "assume" => vak_intent::Escalation::AssumeConservative { after_hours },
        "abandon" => vak_intent::Escalation::AbandonAfter { after_hours },
        other => {
            eprintln!("error: unknown --on-silence '{other}' (wait, assume, abandon)");
            return 2;
        }
    };
    // A limit that is not a finite, non-negative amount is no limit: `NaN`
    // compares false against every spend and would silently remove it.
    if spend_usd.is_some_and(|cap| !cap.is_finite() || cap < 0.0) {
        eprintln!("error: --spend-usd must be a finite amount of zero or more");
        return 2;
    }
    // Assuming a default for an irreversible action because nobody replied is
    // the exact autonomy this system exists to prevent, so the refusal is
    // enforced rather than documented.
    if !escalation.permitted_for(commitment.spec.reading.stakes) {
        eprintln!(
            "error: --on-silence assume is not available for {} work; \
             a default nobody confirmed cannot stand in for consent here",
            commitment.spec.reading.stakes.as_str()
        );
        return 2;
    }

    let envelope = vak_intent::Envelope {
        envelope_id: uuid::Uuid::now_v7().to_string(),
        granted_by: std::env::var("USER").unwrap_or_else(|_| "operator".into()),
        granted_at: chrono::Utc::now(),
        expires_at: hours.map(|h| chrono::Utc::now() + chrono::Duration::hours(h)),
        spend_limit_usd: spend_usd,
        path_scope: paths,
        tool_scope: tools,
        permission_ceiling: ceiling,
        escalation,
        revoked_at: None,
    };
    let envelope_id = envelope.envelope_id.clone();
    match ledger.append(&vak_commit::Event::new(
        &commitment.commitment_id,
        vak_commit::EventKind::EnvelopeGranted {
            envelope: Box::new(envelope),
        },
    )) {
        Ok(()) => {
            println!("granted {envelope_id} on {}", commitment.spec.objective);
            println!(
                "  this narrows, it does not widen: the permission mode is capped at \
                 {permission} and irreversible steps still ask."
            );
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            2
        }
    }
}

/// Withdraw a grant.
///
/// Revocation takes effect on read rather than being remembered: a revoked
/// envelope narrows nothing further and grants nothing at all, so an in-flight
/// run loses the delegation at its next authority check (`AGENTS.md`
/// invariant 11).
pub(crate) fn run_revoke(cwd: std::path::PathBuf, id: String) -> i32 {
    let core = match Core::new(cwd) {
        Ok(core) => core,
        Err(error) => {
            eprintln!("error: {error}");
            return 2;
        }
    };
    let ledger = CommitmentLedger::new(&core.sessions_home());
    let Some(commitment) = resolve_id(&ledger, &id) else {
        eprintln!("error: no commitment matching '{id}'");
        return 2;
    };
    let Some(envelope) = commitment.envelope.as_ref() else {
        eprintln!("error: that commitment has no grant to revoke");
        return 2;
    };
    match ledger.append(&vak_commit::Event::new(
        &commitment.commitment_id,
        vak_commit::EventKind::EnvelopeRevoked {
            envelope_id: envelope.envelope_id.clone(),
            by: std::env::var("USER").unwrap_or_else(|_| "operator".into()),
        },
    )) {
        Ok(()) => {
            println!("revoked {}", envelope.envelope_id);
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            2
        }
    }
}
