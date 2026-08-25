//! `vakcoder digest [--days N]` (docs/design/29-personal-os.md P3): the
//! weekly usage report rendered as aligned plain text from
//! `vak_core::digest`. Pure rendering; all math lives in the library.

use std::path::PathBuf;

use vak_core::{Core, digest::DigestReport};

const MIN_DAYS: u32 = 1;
const MAX_DAYS: u32 = 90;
const TOP_ROWS: usize = 5;

pub fn clamp_days(days: u32) -> u32 {
    days.clamp(MIN_DAYS, MAX_DAYS)
}

pub fn run_digest(cwd: PathBuf, days: u32) -> i32 {
    let core = match Core::new(cwd) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let days = clamp_days(days);
    let report = vak_core::digest::digest(&core.sessions_home(), days);
    print_digest(&report);
    0
}

fn usd(v: f64) -> String {
    format!("${v:.2}")
}

fn print_digest(report: &DigestReport) {
    let days = report.days;
    if report.dispatches == 0
        && report.memory_notes_appended == 0
        && report.skill_proposals_opened == 0
    {
        println!("digest — no usage recorded in the last {days} day(s)");
        return;
    }

    println!(
        "digest — last {days} day(s), since {}",
        report
            .since
            .map(|s| s.to_rfc3339())
            .unwrap_or_else(|| "—".into())
    );

    let unpriced_suffix = if report.unpriced_rows > 0 {
        format!(" · {} unpriced dispatch(es) excluded", report.unpriced_rows)
    } else {
        String::new()
    };
    println!(
        "  {:<14} {} total{}",
        "spend",
        usd(report.total_usd),
        unpriced_suffix
    );
    println!(
        "  {:<14} {} dispatch(es) across {} session(s)",
        "dispatches",
        report.dispatches,
        report.distinct_sessions.len()
    );
    println!(
        "  {:<14} in {} / out {} / cache-read {}",
        "tokens", report.input_tokens, report.output_tokens, report.cache_read_tokens
    );

    let mut models: Vec<(String, vak_core::digest::ModelRollup)> = report
        .by_model
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    models.sort_by(|a, b| {
        b.1.usd
            .total_cmp(&a.1.usd)
            .then(b.1.rows.cmp(&a.1.rows))
            .then(a.0.cmp(&b.0))
    });
    let model_parts: Vec<String> = models
        .iter()
        .take(TOP_ROWS)
        .map(|(name, r)| {
            format!(
                "{name} {} ({} row(s), in {}/out {})",
                usd(r.usd),
                r.rows,
                r.input_tokens,
                r.output_tokens
            )
        })
        .collect();
    print_row("top models", &model_parts);

    let mut providers: Vec<(String, vak_core::digest::ProviderRollup)> = report
        .by_provider
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    providers.sort_by(|a, b| {
        b.1.usd
            .total_cmp(&a.1.usd)
            .then(b.1.rows.cmp(&a.1.rows))
            .then(a.0.cmp(&b.0))
    });
    let provider_parts: Vec<String> = providers
        .iter()
        .take(TOP_ROWS)
        .map(|(name, r)| format!("{name} {} ({} row(s))", usd(r.usd), r.rows))
        .collect();
    print_row("providers", &provider_parts);

    for day in &report.per_day {
        let unpriced = if day.unpriced_rows > 0 {
            format!(" · {} unpriced", day.unpriced_rows)
        } else {
            String::new()
        };
        println!(
            "  {:<14} {} {}{}",
            "per day",
            day.day,
            usd(day.usd),
            unpriced
        );
    }

    println!(
        "  {:<14} {} note(s) appended · {} proposal(s) opened",
        "learning", report.memory_notes_appended, report.skill_proposals_opened
    );
}

fn print_row(label: &str, parts: &[String]) {
    if !parts.is_empty() {
        println!("  {:<14} {}", label, parts.join(" · "));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn days_clamp_to_one_through_ninety() {
        assert_eq!(clamp_days(0), 1);
        assert_eq!(clamp_days(1), 1);
        assert_eq!(clamp_days(7), 7);
        assert_eq!(clamp_days(90), 90);
        assert_eq!(clamp_days(91), 90);
        assert_eq!(clamp_days(u32::MAX), 90);
    }

    #[test]
    fn empty_and_populated_reports_render_without_panicking() {
        let empty = DigestReport {
            days: 7,
            ..Default::default()
        };
        print_digest(&empty);

        let mut populated = DigestReport {
            days: 3,
            total_usd: 4.2,
            unpriced_rows: 1,
            input_tokens: 100,
            output_tokens: 50,
            dispatches: 2,
            ..Default::default()
        };
        populated.by_model.insert(
            "claude-x".into(),
            vak_core::digest::ModelRollup {
                rows: 2,
                usd: 4.2,
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 10,
            },
        );
        populated.by_provider.insert(
            "anthropic".into(),
            vak_core::digest::ProviderRollup { rows: 2, usd: 4.2 },
        );
        populated.distinct_sessions = vec!["s1".into()];
        populated.per_day.push(vak_core::digest::DayRollup {
            day: "2026-08-24".into(),
            usd: 4.2,
            unpriced_rows: 1,
        });
        print_digest(&populated);
    }
}
