//! M6 exit tests that need no server: a catalog built a step at a time
//! equals one rebuilt from the records, search keeps to its audience,
//! lineage reaches a cause from anything, and a query at a million nodes
//! answers well under 50 ms.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use vak_llm::Role;
use vak_llm::types::{ContentBlock, Message};
use vak_session::SessionLog;
use vak_session::trace::{Cause, TraceKey, local};
use vak_session::types::{
    CallEffect, CallEffectRecord, ConversationContext, Entry, EntryPayload, FrozenContract,
    MessageRecord, SessionHeader,
};

/// Tests that write records share the binary's one data home, so they run
/// one at a time and each reads its own catalog file.
static RECORDS: Mutex<()> = Mutex::new(());

/// The data home and a bound folder for this test, holding the records
/// lock for the test's length.
fn home() -> (
    std::sync::MutexGuard<'static, ()>,
    tempfile::TempDir,
    PathBuf,
    PathBuf,
) {
    let guard = RECORDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    vak_config::paths::isolate_home_for_tests();
    let data = vak_config::paths::data_home();
    let cwd = tempfile::tempdir().unwrap();
    let folder = cwd.path().to_path_buf();
    vak_config::spaces::bind(&folder).unwrap();
    (guard, cwd, data, folder)
}

fn header(
    id: &str,
    cwd: &Path,
    agent: &str,
    audience: &str,
    run: Option<&TraceKey>,
) -> SessionHeader {
    SessionHeader {
        space: Some(local::space(cwd)),
        run: run.map(|trace| trace.run),
        cause: run.map(|trace| trace.cause.clone()),
        agent: Some(
            serde_json::from_value(serde_json::json!({
                "id": agent, "revision": 1, "name": agent,
                "personality": "", "behaviour": ""
            }))
            .unwrap(),
        ),
        session_id: id.to_string(),
        created_at: chrono::Utc::now(),
        cwd: cwd.to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: Some(ConversationContext {
            conversation_id: format!("cnv-{agent}"),
            audience_id: audience.to_string(),
            origin: None,
        }),
        contract: FrozenContract {
            app_version: "test".into(),
            provider: "scripted".into(),
            model: "m".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: String::new(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

fn message(role: Role, content: Vec<ContentBlock>) -> MessageRecord {
    MessageRecord {
        message: Message { role, content },
        meta: None,
    }
}

fn agent_home(data: &Path, agent: &str) -> PathBuf {
    if agent == "vak" {
        data.to_path_buf()
    } else {
        data.join("agents").join(agent)
    }
}

/// One turn in a new session: the user asks, the model writes a file with
/// a tool, and answers. Returns the session's plain id and its turn id.
fn turn(
    data: &Path,
    cwd: &Path,
    agent: &str,
    audience: &str,
    run: &TraceKey,
    words: &str,
) -> (String, String) {
    let id = uuid::Uuid::now_v7().to_string();
    let path = vak_session::SessionPath::new_session_file(&agent_home(data, agent), cwd, &id);
    let mut log = SessionLog::create(path, header(&id, cwd, agent, audience, Some(run))).unwrap();
    let turn = uuid::Uuid::now_v7().to_string();
    log.begin_turn(&turn).unwrap();
    log.append_message(message(Role::User, vec![ContentBlock::text(words)]))
        .unwrap();
    log.append_message(message(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: "toolu_1".into(),
            name: "write".into(),
            input: serde_json::json!({ "path": "report.md" }),
        }],
    ))
    .unwrap();
    log.append_message(message(
        Role::User,
        vec![ContentBlock::ToolResult {
            tool_use_id: "toolu_1".into(),
            content: "wrote report.md".into(),
            is_error: false,
        }],
    ))
    .unwrap();
    let parent = log.tail_id().cloned();
    log.append(Entry::new(
        parent,
        EntryPayload::CallEffect(CallEffectRecord {
            tool_use_id: "toolu_1".into(),
            effect: CallEffect::FileWrite {
                path: "report.md".into(),
                digest: None,
                bytes: Some(10),
            },
        }),
    ))
    .unwrap();
    log.append_message(message(
        Role::Assistant,
        vec![ContentBlock::text("The report is written.")],
    ))
    .unwrap();
    (id, turn)
}

fn runs(data: &Path) -> vak_session::runs::Runs {
    vak_session::runs::Runs::at(
        vak_config::scope::SharedScope::new(data).runs(),
        vak_config::paths::local_tenant_home(),
    )
}

fn effects(data: &Path) -> vak_session::effects::Effects {
    vak_session::effects::Effects::at(
        vak_config::scope::SharedScope::new(data).effects(),
        vak_config::paths::local_tenant_home(),
    )
}

/// A run of `agent` that wrote a session and turn, and sent one message.
fn run_with_turn(
    data: &Path,
    cwd: &Path,
    agent: &str,
    audience: &str,
    words: &str,
) -> (TraceKey, String, String, String) {
    let trace = TraceKey::root(
        local::tenant(),
        local::space(cwd),
        local::agent(agent),
        Cause::User {
            request_id: "req-1".into(),
        },
    )
    .acting(local::local_owner(), None);
    let (session, turn) = turn(data, cwd, agent, audience, &trace, words);
    let trace = TraceKey {
        session: vak_session::ids::SessionId::parse(&format!("ses_{session}")).ok(),
        turn: vak_session::ids::TurnId::parse(&format!("trn_{turn}")).ok(),
        ..trace
    };
    let runs = runs(data);
    runs.open_in(&trace, None, None, 1, Some(&session), None)
        .unwrap();
    let effect = effects(data)
        .prepare(vak_session::effects::Prepare::new(
            vak_session::effects::EffectKind::delivery("log:main"),
            "log:main",
            Some(trace.clone()),
            b"{}".to_vec(),
        ))
        .unwrap();
    runs.settle(trace.run, vak_session::runs::RunOutcome::Completed, None)
        .unwrap();
    (trace, session, turn, effect.id.to_string())
}

/// A catalog in its own file, over the data home.
fn catalog(dir: &tempfile::TempDir, data: &Path, name: &str) -> Catalog {
    Catalog::open(&dir.path().join(name), data).unwrap()
}

#[test]
fn catalog_rebuild_equals_incremental() {
    let (_lock, cwd, data, folder) = home();
    let incremental = catalog(&cwd, &data, "incremental.db");
    run_with_turn(
        &data,
        &folder,
        "vak",
        "local",
        "quarterly lighthouse figures",
    );
    incremental.catch_up().unwrap();
    run_with_turn(&data, &folder, "scout", "local", "harbour tide tables");
    incremental.catch_up().unwrap();
    // Nothing new: a second catch-up takes nothing and changes nothing.
    let before = incremental.dump().unwrap();
    assert_eq!(incremental.catch_up().unwrap().rows, 0);
    assert_eq!(incremental.dump().unwrap(), before);
    assert!(!incremental.stale().unwrap());

    let rebuilt = catalog(&cwd, &data, "rebuilt.db");
    rebuilt.rebuild().unwrap();
    assert_eq!(rebuilt.dump().unwrap(), incremental.dump().unwrap());
    // Rebuilding in place gives the same rows again.
    incremental.rebuild().unwrap();
    assert_eq!(incremental.dump().unwrap(), before);
    let counts = incremental.counts().unwrap();
    for kind in ["session", "turn", "call", "file", "run", "effect"] {
        assert!(
            counts.get(kind).copied().unwrap_or(0) > 0,
            "{kind}: {counts:?}"
        );
    }
}

#[test]
fn a_new_row_makes_the_catalog_stale_until_caught_up() {
    let (_lock, cwd, data, folder) = home();
    let catalog = catalog(&cwd, &data, "catalog.db");
    run_with_turn(&data, &folder, "vak", "local", "first");
    catalog.catch_up().unwrap();
    assert!(!catalog.stale().unwrap());
    run_with_turn(&data, &folder, "vak", "local", "second");
    assert!(catalog.stale().unwrap());
    catalog.catch_up().unwrap();
    assert!(!catalog.stale().unwrap());
}

#[test]
fn search_respects_audience() {
    let (_lock, cwd, data, folder) = home();
    let (_, mine, _, _) = run_with_turn(&data, &folder, "vak", "local", "zephyr budget review");
    let (_, theirs, _, _) = run_with_turn(
        &data,
        &folder,
        "scout",
        "telegram:42",
        "zephyr budget for the scout",
    );
    let (_, trashed, _, _) = run_with_turn(&data, &folder, "vak", "local", "zephyr trashed");
    let catalog = catalog(&cwd, &data, "catalog.db");
    catalog.catch_up().unwrap();
    let sessions = |audience: &Audience| -> HashSet<String> {
        catalog
            .search("zephyr", audience, 50)
            .unwrap()
            .into_iter()
            .filter_map(|hit| hit.node.session)
            .collect()
    };
    // The owner reads everything that is not in the trash.
    let owner = Audience {
        exclude_sessions: [trashed.clone()].into_iter().collect(),
        ..Default::default()
    };
    let seen = sessions(&owner);
    assert!(seen.contains(&format!("ses_{mine}")) && seen.contains(&format!("ses_{theirs}")));
    assert!(!seen.contains(&format!("ses_{trashed}")));
    // A chat with the scout sees only the scout's things in that chat.
    let chat = Audience {
        agents: Some(vec![local::agent("scout").to_string()]),
        audience: Some("telegram:42".into()),
        ..Default::default()
    };
    assert_eq!(
        sessions(&chat),
        [format!("ses_{theirs}")].into_iter().collect()
    );
    // The built-in Agent's local audience never sees the scout's chat.
    let local_vak = Audience {
        agents: Some(vec![local::agent("vak").to_string()]),
        audience: Some("local".into()),
        ..Default::default()
    };
    assert!(!sessions(&local_vak).contains(&format!("ses_{theirs}")));
    // Raw tool output is not indexed; its digest is, and no thinking is.
    let hits = catalog.search("report", &owner, 50).unwrap();
    assert!(
        hits.iter()
            .any(|hit| hit.node.kind == "call" || hit.node.kind == "turn")
    );
}

#[test]
fn lineage_from_any_artifact_to_cause() {
    let (_lock, cwd, data, folder) = home();
    let (trace, session, turn, effect) =
        run_with_turn(&data, &folder, "scout", "local", "lineage words");
    let catalog = catalog(&cwd, &data, "catalog.db");
    catalog.catch_up().unwrap();
    let space = local::space(&folder).to_string();
    let file = format!("file:{space}:report.md");
    let call = format!("call:ses_{session}:toolu_1");
    for start in [
        file.as_str(),
        call.as_str(),
        &format!("trn_{turn}"),
        &format!("ses_{session}"),
        session.as_str(),
        effect.as_str(),
        &trace.run.to_string(),
    ] {
        let lineage = catalog.lineage(start).unwrap().expect(start);
        assert_eq!(
            lineage.run.as_ref().map(|run| run.id.clone()),
            Some(trace.run.to_string()),
            "{start}: {lineage:?}"
        );
        assert_eq!(lineage.cause.as_deref(), Some("user"), "{start}");
        assert_eq!(
            lineage.agent,
            Some(local::agent("scout").to_string()),
            "{start}"
        );
        assert_eq!(lineage.space.as_deref(), Some(space.as_str()), "{start}");
        assert_eq!(
            lineage.actor,
            Some(local::local_owner().to_string()),
            "{start}"
        );
    }
    let from_file = catalog.lineage(&file).unwrap().unwrap();
    assert_eq!(
        from_file.session.map(|node| node.id),
        Some(format!("ses_{session}"))
    );
    assert_eq!(
        from_file.turn.map(|node| node.id),
        Some(format!("trn_{turn}"))
    );
    // Where the session's bytes are is one lookup, never a directory walk.
    let dir = catalog.session_dir(&session).unwrap().unwrap();
    assert_eq!(vak_config::scope::ledger_session_id(&dir), Some(session));
}

#[test]
fn catalog_query_p95_under_50ms_at_1m_nodes() {
    let dir = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(&dir.path().join("catalog.db"), dir.path()).unwrap();
    const RUNS: usize = 100_000;
    // Ten nodes per run: the run, its session, a turn, four calls, two
    // files and an effect, with the edges that join them.
    catalog
        .seed(|tx| {
            let mut node = tx.prepare(
                "INSERT INTO nodes (id, kind, agent, session, run, cause, created_at)
                 VALUES (?1, ?2, 'agt_a', ?3, ?4, 'user', '2026-10-06T00:00:00Z')",
            )?;
            let mut edge = tx.prepare("INSERT INTO edges (src, kind, dst) VALUES (?1, ?2, ?3)")?;
            for r in 0..RUNS {
                let run = format!("run_{r}");
                let session = format!("ses_{r}");
                let turn = format!("trn_{r}");
                node.execute(params_of(&run, "run", &session, &run))?;
                node.execute(params_of(&session, "session", &session, &run))?;
                node.execute(params_of(&turn, "turn", &session, &run))?;
                edge.execute([&session, "caused_by", &run])?;
                edge.execute([&turn, "produced_by", &run])?;
                edge.execute([&turn, "part_of", &session])?;
                for c in 0..4 {
                    let call = format!("call:{session}:{c}");
                    node.execute(params_of(&call, "call", &session, &run))?;
                    edge.execute([&call, "produced_by", &turn])?;
                    if c < 2 {
                        let file = format!("file:{r}:{c}");
                        node.execute(params_of(&file, "file", &session, &run))?;
                        edge.execute([&file, "produced_by", &call])?;
                    }
                }
                let effect = format!("eff_{r}");
                node.execute(params_of(&effect, "effect", &session, &run))?;
                edge.execute([&effect, "produced_by", &run])?;
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(
        catalog.counts().unwrap().values().sum::<u64>(),
        10 * RUNS as u64
    );
    let mut times = Vec::new();
    for i in 0..200 {
        let r = (i * 7919) % RUNS;
        let started = std::time::Instant::now();
        let lineage = catalog.lineage(&format!("file:{r}:1")).unwrap().unwrap();
        assert_eq!(lineage.run.unwrap().id, format!("run_{r}"));
        let opened = catalog.open_node(&format!("ses_{r}")).unwrap().unwrap();
        assert_eq!(opened.kind, "session");
        times.push(started.elapsed());
    }
    times.sort();
    let p95 = times[times.len() * 95 / 100];
    assert!(p95 < std::time::Duration::from_millis(50), "p95 {p95:?}");
}

fn params_of<'a>(id: &'a str, kind: &'a str, session: &'a str, run: &'a str) -> [&'a str; 4] {
    [id, kind, session, run]
}

#[test]
fn documents_and_commitments_are_searchable_and_follow_new_versions() {
    let (_lock, cwd, data, folder) = home();
    let scope = vak_config::scope::AgentScope::new(&data);
    let notes = scope.memory_notes(&folder);
    vak_session::documents::create(&notes, "The owner prefers marmalade on toast.").unwrap();
    let trigger = vak_config::scope::SharedScope::new(&data)
        .triggers()
        .join(vak_session::ids::TriggerId::new().to_string());
    let id = trigger.file_name().unwrap().to_str().unwrap().to_string();
    vak_session::documents::create(
        &trigger,
        &serde_json::json!({ "id": id, "name": "Weekly kumquat digest", "agent": "vak" })
            .to_string(),
    )
    .unwrap();
    vak_session::chain::RecordChain::at(scope.commitments())
        .append(&serde_json::json!({
            "event_id": "e1", "commitment_id": "c1", "ts": "2026-10-06T00:00:00Z",
            "kind": "opened", "spec": { "objective": "Ship the persimmon report" }
        }))
        .unwrap();
    let catalog = catalog(&cwd, &data, "catalog.db");
    catalog.catch_up().unwrap();
    let kinds = |word: &str| -> Vec<String> {
        catalog
            .search(word, &Audience::default(), 10)
            .unwrap()
            .into_iter()
            .map(|hit| hit.node.kind)
            .collect()
    };
    assert_eq!(kinds("marmalade"), ["memory"]);
    assert_eq!(kinds("kumquat"), ["trigger"]);
    assert_eq!(kinds("persimmon"), ["commitment"]);
    // A memory note is private to its Agent: a conversation audience never
    // reaches it.
    let chat = Audience {
        audience: Some("telegram:42".into()),
        ..Default::default()
    };
    assert!(catalog.search("marmalade", &chat, 10).unwrap().is_empty());
    // A new version replaces what was indexed.
    vak_session::documents::update(&notes, |_| Ok(Some(("Now: apricot jam.".to_string(), ()))))
        .unwrap();
    assert!(catalog.stale().unwrap());
    catalog.catch_up().unwrap();
    assert!(kinds("marmalade").is_empty());
    assert_eq!(kinds("apricot"), ["memory"]);
}
