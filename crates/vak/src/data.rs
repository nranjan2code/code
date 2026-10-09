//! `vak data` (plan M7a-c): what the data home holds and what its
//! retention would do, read directly like `vak runs`. Observe-only: every
//! verb here reads, and the plan it prints is not carried out.

use std::path::PathBuf;

use vak_core::Core;

fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(|name| name.replace('_', " ")))
        .unwrap_or_default()
}

pub(crate) fn run_data(cwd: PathBuf, action: Option<crate::cli::DataAction>) -> i32 {
    let core = match Core::new(cwd) {
        Ok(core) => core,
        Err(error) => {
            eprintln!("error: {error}");
            return 2;
        }
    };
    match action.unwrap_or(crate::cli::DataAction::Status) {
        crate::cli::DataAction::Status => {
            let status = core.data_status();
            println!(
                "retention     {}",
                if status.mode == "commit" {
                    "removing what is due (for the kinds listed as carried out)"
                } else {
                    "observing only: the plan is shown and nothing is removed"
                }
            );
            println!("label         {}", status.label.name);
            println!(
                "held          {} in {} files",
                size(status.bytes),
                status.files
            );
            let quota = &status.quota;
            let kept = size(quota.kept_bytes);
            println!(
                "limit         {}",
                match (quota.state, quota.limit_bytes) {
                    (vak_core::lifecycle::QuotaState::Hard, Some(limit)) => format!(
                        "full: {kept} kept of {}; new work is refused until space is freed or the limit is raised",
                        size(limit)
                    ),
                    (vak_core::lifecycle::QuotaState::Soft, Some(limit)) =>
                        format!("nearly full: {kept} kept of {}", size(limit)),
                    (_, Some(limit)) => format!("{kept} kept of {}", size(limit)),
                    (_, None) => "none set ([lifecycle] quota_gb)".to_string(),
                }
            );
            println!(
                "due now       {} items, {}",
                status.due,
                size(status.reclaimable_bytes)
            );
            println!(
                "kept back     {} items (in use, or kept by a person)",
                status.guarded
            );
            let list = |classes: &[vak_lifecycle::DataClass]| {
                classes.iter().map(name).collect::<Vec<_>>().join(", ")
            };
            println!("watched       {}", list(&status.observed));
            println!("carried out   {}", list(&status.committed));
            println!("not watched   {}", list(&status.unobserved));
            0
        }
        crate::cli::DataAction::Usage => {
            let usage = core.data_usage();
            println!(
                "{:<9} {:<12} {:<22} {:>8} {:>10}",
                "root", "class", "owner", "files", "size"
            );
            for row in &usage.rows {
                println!(
                    "{:<9} {:<12} {:<22} {:>8} {:>10}",
                    row.root,
                    row.class,
                    row.owner,
                    row.files,
                    size(row.bytes)
                );
            }
            println!(
                "{:<9} {:<12} {:<22} {:>8} {:>10}",
                "total",
                "",
                "",
                usage.files,
                size(usage.bytes)
            );
            0
        }
        crate::cli::DataAction::Gc { dry_run } => {
            let allowed = core.config().lifecycle.commit;
            let tick = core.lifecycle_tick(allowed && !dry_run);
            for row in &tick.committed {
                println!(
                    "removed {:<16} {:>10}  {}",
                    name(&row.class),
                    size(row.bytes),
                    row.item
                );
            }
            for row in &tick.failed {
                println!(
                    "failed  {:<16} {}  ({})",
                    name(&row.class),
                    row.item,
                    row.error_kind.as_deref().unwrap_or("unknown")
                );
            }
            if tick.mode == "commit" {
                println!(
                    "removed {} items, {}; {} due items are of kinds not carried out yet",
                    tick.committed.len(),
                    size(tick.reclaimed_bytes),
                    tick.left
                );
            } else {
                println!(
                    "{} items are due, {}; nothing was removed{}",
                    tick.plan.actions.len(),
                    size(tick.plan.reclaimable_bytes),
                    if allowed || dry_run {
                        ""
                    } else {
                        " (retention is observing; set [lifecycle] mode = \"commit\" to act)"
                    }
                );
            }
            i32::from(!tick.failed.is_empty())
        }
        crate::cli::DataAction::Erase {
            session,
            scope,
            guest,
        } => {
            let sure = |question: &str| {
                println!("{question} Type erase to go on:");
                let mut typed = String::new();
                std::io::stdin().read_line(&mut typed).is_ok() && typed.trim() == "erase"
            };
            let report = |erased: Result<
                vak_core::erasure::Receipt,
                vak_core::erasure::ErasureError,
            >| match erased {
                Ok(receipt) => {
                    println!(
                        "Erased. Receipt {} ({} keys destroyed, {} stored items deleted).",
                        receipt.id, receipt.keys_destroyed, receipt.objects_deleted
                    );
                    for line in &receipt.not_reached {
                        println!("  not reached: {line}");
                    }
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            };
            if scope == "install" {
                if session != "everything" {
                    eprintln!(
                        "error: to erase the whole install, run: vak data erase everything --scope install"
                    );
                    return 2;
                }
                let preview = match core.install_erasure_preview() {
                    Ok(preview) => preview,
                    Err(error) => {
                        eprintln!("error: {error}");
                        return 1;
                    }
                };
                if preview.held > 0 {
                    println!("{} on hold. Release every hold first.", preview.held);
                    return 1;
                }
                println!(
                    "This erases everything Vakyartha has stored on this machine, for good: {} conversations, {} files in the Library, every Agent, automation, connected account and saved key. Your own folders are not touched.",
                    preview.conversations, preview.artifacts
                );
                println!("Stop Vakyartha first if it is running.");
                println!("Type {} to go on:", vak_core::erasure::INSTALL_CONFIRMATION);
                let mut typed = String::new();
                if std::io::stdin().read_line(&mut typed).is_err()
                    || typed.trim() != vak_core::erasure::INSTALL_CONFIRMATION
                {
                    println!("Not erased.");
                    return 1;
                }
                return match core.erase_install(
                    Some(&preview.digest),
                    vak_core::erasure::Cause::Person,
                    None,
                ) {
                    Ok(receipt) => {
                        println!(
                            "Erased. Receipt {} ({} keys destroyed).",
                            receipt.id, receipt.keys_destroyed
                        );
                        println!(
                            "The receipt is kept at {}",
                            vak_config::paths::data_home()
                                .join("erased")
                                .join(format!("{}.json", receipt.id))
                                .display()
                        );
                        for line in &receipt.not_reached {
                            println!("  not reached: {line}");
                        }
                        0
                    }
                    Err(error) => {
                        eprintln!("error: {error}");
                        1
                    }
                };
            }
            if scope == "agent" || scope == "project" {
                let project = scope == "project";
                let looked = if project {
                    core.project_erasure_preview(&session)
                } else {
                    core.agent_erasure_preview(&session)
                };
                let preview = match looked {
                    Ok(preview) => preview,
                    Err(error) => {
                        eprintln!("error: {error}");
                        return 1;
                    }
                };
                if project {
                    println!(
                        "This erases everything Vakyartha keeps for the project {session}, for good. Its folder is not touched:"
                    );
                } else {
                    println!("This erases everything the Agent {session} holds, for good:");
                }
                println!("  conversations      {}", preview.conversations);
                println!("  memory and notes   {}", preview.documents);
                println!("  files              {}", preview.artifacts);
                println!("  automations        {}", preview.automations);
                println!("  workspace files    {}", preview.workspace_files);
                if preview.held {
                    println!("Something of it is on hold; release the hold first.");
                    return 1;
                }
                println!("Type its id to erase it:");
                let mut typed = String::new();
                if std::io::stdin().read_line(&mut typed).is_err() || typed.trim() != session {
                    println!("Not erased.");
                    return 1;
                }
                let cause = vak_core::erasure::Cause::Person;
                return report(if project {
                    core.erase_project(&session, Some(&preview.digest), cause, None)
                } else {
                    core.erase_agent(&session, Some(&preview.digest), cause, None)
                });
            }
            if scope == "account" {
                if !sure(&format!(
                    "This erases what account {session} returned, from every conversation that read it. The conversations stay."
                )) {
                    println!("Not erased.");
                    return 1;
                }
                return report(core.erase_account(
                    &session,
                    vak_core::erasure::Cause::Person,
                    None,
                ));
            }
            if scope == "guest" {
                let Some(guest) = guest else {
                    let guests = core.conversation_guests(&session);
                    if guests.is_empty() {
                        println!("No guest wrote in this conversation.");
                    } else {
                        println!("Name one with --guest:");
                        for guest in guests {
                            println!("  {guest}");
                        }
                    }
                    return 2;
                };
                let preview = match core.guest_erasure_preview(&session, &guest) {
                    Ok(preview) => preview,
                    Err(error) => {
                        eprintln!("error: {error}");
                        return 1;
                    }
                };
                if preview.held {
                    println!("It is on hold and cannot be erased until the hold is released.");
                    return 1;
                }
                if !sure(&format!(
                    "This erases what {guest} wrote in this conversation and in comments on {} of its files. The conversation stays.",
                    preview.artifacts.len()
                )) {
                    println!("Not erased.");
                    return 1;
                }
                return report(core.erase_guest(
                    &session,
                    &guest,
                    Some(&preview.digest),
                    vak_core::erasure::Cause::Person,
                    None,
                ));
            }
            if scope != "conversation" {
                eprintln!(
                    "error: --scope is conversation, guest, account, agent, project or install"
                );
                return 2;
            }
            let preview = match core.erasure_preview(&session) {
                Ok(preview) => preview,
                Err(error) => {
                    eprintln!("error: {error}");
                    return 1;
                }
            };
            let title = core.erasure_confirmation(&session);
            println!("This erases the conversation \"{title}\" for good. It cannot be undone.");
            println!(
                "  conversations   {} (it and the workers it started)",
                preview.conversations.len()
            );
            println!("  unkept drafts   {}", preview.artifacts.len());
            println!("  memory notes    {}", preview.memory_notes);
            println!(
                "  already sent    {} (messages and changes outside Vakyartha stay sent)",
                preview.sent_outside
            );
            if preview.held {
                println!("It is on hold and cannot be erased until the hold is released.");
                return 1;
            }
            if preview.trashed_at.is_none() {
                println!("It is not in the trash. Move it to the trash first.");
                return 1;
            }
            println!("Type the conversation's title to erase it:");
            let mut typed = String::new();
            if std::io::stdin().read_line(&mut typed).is_err() || typed.trim() != title {
                println!("Not erased: the title did not match.");
                return 1;
            }
            match core.erase_conversation(
                &session,
                Some(&preview.digest),
                vak_core::erasure::Cause::Person,
                Some(vak_session::trace::local::local_owner()),
            ) {
                Ok(receipt) => {
                    println!(
                        "Erased. Receipt {} ({} keys destroyed, {} search rows removed).",
                        receipt.id, receipt.keys_destroyed, receipt.search_rows_removed
                    );
                    println!("Not reached by this erasure:");
                    for line in &receipt.not_reached {
                        println!("  - {line}");
                    }
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
        crate::cli::DataAction::Hold { session, release } => {
            match core.hold_conversation(&session, !release) {
                Ok(()) => {
                    println!(
                        "{}",
                        if release {
                            "The hold is released."
                        } else {
                            "On hold: nothing erases this conversation until the hold is released."
                        }
                    );
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
        crate::cli::DataAction::Verify { json } => {
            let report = core.data_integrity();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).unwrap_or_default()
                );
                return i32::from(!report.sound());
            }
            println!(
                "{} conversations and {} record chains checked: {} records in {} segments",
                report.conversations, report.chains, report.records, report.segments
            );
            for broken in &report.broken {
                println!("  DAMAGED  {} (segment {})", broken.at, broken.segment);
            }
            if report.torn_tails > 0 {
                println!(
                    "  {} unfinished record(s) at the end of a segment, left by a stop mid-write; the next write trims them",
                    report.torn_tails
                );
            }
            println!(
                "keys      {} in use, {} destroyed, {} on hold",
                report.keys, report.keys_destroyed, report.keys_held
            );
            println!(
                "receipts  {} ({} with a signature that does not verify)",
                report.receipts, report.receipts_unverified
            );
            println!(
                "search    {}",
                match report.search {
                    "current" => "up to date",
                    "behind" =>
                        "behind the records; `vak data rebuild-catalog` brings it up to date",
                    _ => "could not be read",
                }
            );
            if report.fenced {
                println!(
                    "This data home was restored; start Vakyartha again before writing to it."
                );
            }
            println!(
                "{}",
                if report.sound() {
                    "Nothing is damaged."
                } else {
                    "Something is damaged: see above."
                }
            );
            i32::from(!report.sound())
        }
        crate::cli::DataAction::Rules { set, reset } => {
            let current = vak_core::lifecycle::retention_label();
            let defaults = vak_lifecycle::Label::default_tenant();
            let name = |class: vak_lifecycle::DataClass| {
                serde_json::to_value(class)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_default()
            };
            if set.is_empty() && !reset {
                println!("How long each kind of data is kept ({}):", current.name);
                for rule in &current.rules {
                    let Some(secs) = rule.delete_after_secs else {
                        continue;
                    };
                    let default = defaults
                        .rule(rule.class)
                        .and_then(|rule| rule.delete_after_secs);
                    println!(
                        "  {:<18} {:>5} days{}",
                        name(rule.class),
                        secs / 86_400,
                        if default == Some(secs) {
                            ""
                        } else {
                            "  (changed)"
                        }
                    );
                }
                return 0;
            }
            // What stands now, less the defaults, plus what is asked.
            let mut keep_days = std::collections::BTreeMap::new();
            if !reset {
                for rule in &current.rules {
                    let default = defaults
                        .rule(rule.class)
                        .and_then(|rule| rule.delete_after_secs);
                    if let Some(secs) = rule.delete_after_secs.filter(|secs| default != Some(*secs))
                    {
                        keep_days.insert(rule.class, secs / 86_400);
                    }
                }
            }
            for pair in &set {
                let parsed = pair.split_once('=').and_then(|(kind, days)| {
                    let class: vak_lifecycle::DataClass =
                        serde_json::from_value(serde_json::Value::String(kind.trim().into()))
                            .ok()?;
                    Some((class, days.trim().parse::<i64>().ok()?))
                });
                let Some((class, days)) = parsed else {
                    eprintln!("error: '{pair}' is not kind=days");
                    return 2;
                };
                keep_days.insert(class, days);
            }
            let preview = match core.retention_preview(&keep_days) {
                Ok(preview) => preview,
                Err(error) => {
                    eprintln!("error: {error}");
                    return 2;
                }
            };
            let mut digest = None;
            if !preview.shortened.is_empty() {
                println!("These keep times get shorter:");
                for class in &preview.shortened {
                    println!("  {}", name(*class));
                }
                if preview.newly_due.is_empty() {
                    println!("Nothing is past the new times yet.");
                }
                for impact in &preview.newly_due {
                    println!(
                        "  the next pass would remove {} of {} ({} bytes) that it keeps now",
                        impact.items,
                        name(impact.class),
                        impact.bytes
                    );
                }
                println!("Type shorten to go on:");
                let mut typed = String::new();
                if std::io::stdin().read_line(&mut typed).is_err() || typed.trim() != "shorten" {
                    println!("Not changed.");
                    return 1;
                }
                digest = Some(preview.digest.clone());
            }
            match core.set_retention(&keep_days, digest.as_deref()) {
                Ok(label) => {
                    println!("Saved. The rules are now: {}.", label.name);
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
        crate::cli::DataAction::RebuildCatalog => {
            match core.catalog().and_then(|catalog| Ok(catalog.rebuild()?)) {
                Ok(rebuilt) => {
                    println!("search and lineage rebuilt from {} records", rebuilt.rows);
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
        crate::cli::DataAction::Cat { session } => {
            if vak_core::trash::is_trashed(&core.shared_scope(), &session) {
                eprintln!("error: that conversation is in the trash or was erased");
                return 1;
            }
            // No server may have run on this data home: read the records
            // in before looking the conversation up.
            let ledger = core.catalog().ok().and_then(|catalog| {
                let _ = catalog.catch_up();
                catalog.session_dir(&session).ok().flatten()
            });
            let Some(ledger) = ledger else {
                eprintln!("error: no conversation {session}");
                return 1;
            };
            match vak_session::SessionLog::open_read_only(ledger) {
                Ok(log) => {
                    print!(
                        "{}",
                        vak_core::transcript_md::render_markdown(&log.derive_messages())
                    );
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
        crate::cli::DataAction::Grep { text, limit } => {
            let audience = vak_catalog::Audience {
                exclude_sessions: vak_core::trash::search_exclusions(&core.shared_scope(), None),
                held: true,
                ..Default::default()
            };
            let found = core.catalog().and_then(|catalog| {
                let _ = catalog.catch_up();
                Ok(catalog.search(&text, &audience, &vak_catalog::Scope::default(), limit)?)
            });
            match found {
                Ok(hits) => {
                    if hits.is_empty() {
                        println!("nothing found");
                    }
                    for hit in hits {
                        println!(
                            "{}  {}  {}",
                            hit.node.kind,
                            hit.node.session.as_deref().unwrap_or(&hit.node.id),
                            hit.snippet.replace('\n', " ")
                        );
                    }
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
        crate::cli::DataAction::Keys { rotate, retire } => {
            if retire {
                match core.retire_keys(None) {
                    Ok(row) => println!(
                        "Retired {} earlier key{}. A backup or key file made before the last rotation no longer opens here.",
                        row.retired,
                        if row.retired == 1 { "" } else { "s" }
                    ),
                    Err(error) => {
                        eprintln!("error: {error}");
                        return 1;
                    }
                }
            }
            if rotate {
                match core.rotate_keys(None) {
                    Ok(rotation) => println!(
                        "Rotated. Key {} is in use; {} stored keys were protected again under it.",
                        rotation.version + 1,
                        rotation.rewrapped
                    ),
                    Err(error) => {
                        eprintln!("error: {error}");
                        return 1;
                    }
                }
            }
            match core.key_status() {
                Ok(status) => {
                    println!(
                        "kept in: {}",
                        if status.kept_in == "keychain" {
                            "this computer's keychain"
                        } else {
                            "an encrypted file in the data home (no keychain was reachable)"
                        }
                    );
                    println!(
                        "key in use: {} (oldest still protecting something: {})",
                        status.version + 1,
                        status.oldest_in_use + 1
                    );
                    println!(
                        "{} keys, {} destroyed, {} on hold",
                        status.keys, status.destroyed, status.held
                    );
                    let rotations: Vec<_> = status
                        .rotations
                        .iter()
                        .filter(|row| row.retired == 0)
                        .collect();
                    match rotations.last() {
                        Some(last) => println!(
                            "rotated {} times, last {}",
                            rotations.len(),
                            last.at.format("%Y-%m-%d %H:%M")
                        ),
                        None => println!("never rotated"),
                    }
                    if status.retired > 0 {
                        println!("{} earlier keys retired", status.retired);
                    }
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
        crate::cli::DataAction::Receipts => {
            let receipts = core.erasure_receipts();
            if receipts.is_empty() {
                println!("nothing has been erased");
            }
            for receipt in &receipts {
                println!(
                    "{}  {}  {:<8} {}  {}",
                    receipt.at.format("%Y-%m-%d %H:%M"),
                    receipt.id,
                    name(&receipt.cause),
                    receipt.subject,
                    if receipt.verifies() {
                        "signature ok"
                    } else {
                        "SIGNATURE DOES NOT VERIFY"
                    }
                );
            }
            0
        }
        crate::cli::DataAction::Plan { json } => {
            let plan = core.lifecycle_plan();
            if json {
                return match serde_json::to_string_pretty(&plan) {
                    Ok(text) => {
                        println!("{text}");
                        0
                    }
                    Err(error) => {
                        eprintln!("error: {error}");
                        1
                    }
                };
            }
            if plan.actions.is_empty() {
                println!("nothing is due");
            }
            for action in &plan.actions {
                println!(
                    "{:<7} {:<16} {:>10}  due {}  {}",
                    name(&action.does),
                    name(&action.class),
                    size(action.bytes),
                    action.due.format("%Y-%m-%d"),
                    action.item
                );
            }
            for kept in &plan.guarded {
                println!(
                    "{:<7} {:<16} {:>10}  {}  {}",
                    "keep",
                    name(&kept.class),
                    "",
                    name(&kept.guard),
                    kept.item
                );
            }
            if !plan.unobserved.is_empty() {
                let classes: Vec<String> = plan.unobserved.iter().map(name).collect();
                println!("not watched yet: {}", classes.join(", "));
            }
            println!("this is a dry run: nothing was removed");
            0
        }
    }
}
