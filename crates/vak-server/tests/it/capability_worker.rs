//! A skill's program runs in the broker worker under the sandbox its
//! declaration gives it (docs/design/93-self-evolving-capabilities.md, E1).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vak_tools::capability::{CapabilityTool, DECLARATION, Program};
use vak_tools::{Tool, ToolContext};

fn tool(skill: &Path, program: &str, extra: Value) -> CapabilityTool {
    std::fs::write(skill.join("run.py"), program).unwrap();
    let mut declaration = json!({
        "entry": "run.py",
        "arguments": {"type": "object", "properties": {
            "amount": {"type": "number", "description": "The bill"},
            "people": {"type": "integer", "description": "How many share it"}
        }}
    });
    for (key, value) in extra.as_object().unwrap() {
        declaration[key] = value.clone();
    }
    std::fs::write(skill.join(DECLARATION), declaration.to_string()).unwrap();
    let tests = skill.join(vak_tools::capability::TESTS_DIR);
    if !tests.exists() {
        std::fs::create_dir(&tests).unwrap();
        std::fs::write(
            tests.join("runs.json"),
            r#"{"arguments": {}, "expect": {}}"#,
        )
        .unwrap();
    }
    let program = Program::load(skill).unwrap().unwrap();
    CapabilityTool::new(
        "split-bill".into(),
        "Split a bill".into(),
        program,
        PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")),
    )
}

fn context(workspace: &Path) -> ToolContext {
    ToolContext::new(workspace.to_path_buf()).with_agent_id("capability-test")
}

#[tokio::test]
async fn a_program_answers_in_json_through_the_worker() {
    let workspace = tempfile::tempdir().unwrap();
    let skill = tempfile::tempdir().unwrap();
    let tool = tool(
        skill.path(),
        "import json, sys\nargs = json.load(sys.stdin)\nprint(json.dumps({'each': args['amount'] / args['people']}))\n",
        json!({}),
    );
    let output = tool
        .execute(
            &json!({"amount": 90, "people": 3, "timeout_ms": 20000}),
            &context(workspace.path()),
        )
        .await;
    assert!(!output.is_error(), "{}", output.content);
    assert_eq!(
        serde_json::from_str::<Value>(&output.content).unwrap(),
        json!({"each": 30.0})
    );
}

#[tokio::test]
async fn a_program_that_prints_no_json_fails_with_what_it_printed() {
    let workspace = tempfile::tempdir().unwrap();
    let skill = tempfile::tempdir().unwrap();
    let tool = tool(skill.path(), "print('thirty each')\n", json!({}));
    let output = tool
        .execute(&json!({"timeout_ms": 20000}), &context(workspace.path()))
        .await;
    assert!(output.is_error());
    assert!(
        output.content.contains("one JSON value"),
        "{}",
        output.content
    );
    assert!(output.content.contains("thirty each"), "{}", output.content);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn a_program_that_declares_no_hosts_reaches_no_network() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let workspace = tempfile::tempdir().unwrap();
    let skill = tempfile::tempdir().unwrap();
    let tool = tool(
        skill.path(),
        &format!(
            "import json, socket\ntry:\n    socket.create_connection(('127.0.0.1', {port}), timeout=3)\n    print(json.dumps({{'connected': True}}))\nexcept OSError as error:\n    print(json.dumps({{'connected': False}}))\n"
        ),
        json!({}),
    );
    let output = tool
        .execute(&json!({"timeout_ms": 20000}), &context(workspace.path()))
        .await;
    assert!(!output.is_error(), "{}", output.content);
    assert_eq!(
        serde_json::from_str::<Value>(&output.content).unwrap(),
        json!({"connected": false})
    );
    drop(listener);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn a_program_writes_its_drafts_and_never_the_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let skill = tempfile::tempdir().unwrap();
    let tool = tool(
        skill.path(),
        "import json, os\nopen(os.path.join(os.environ['VAK_DRAFTS'], 'split.csv'), 'w').write('a,30\\n')\ntry:\n    open('escaped.txt', 'w').write('no')\n    wrote = True\nexcept OSError:\n    wrote = False\nprint(json.dumps({'workspace_written': wrote}))\n",
        json!({"writes": "drafts"}),
    );
    let output = tool
        .execute(&json!({"timeout_ms": 20000}), &context(workspace.path()))
        .await;
    assert!(!output.is_error(), "{}", output.content);
    let result: Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(result["result"], json!({"workspace_written": false}));
    let drafts = result["drafts"].as_array().unwrap();
    assert_eq!(drafts.len(), 1, "{result}");
    assert!(
        drafts[0].as_str().unwrap().ends_with("/split.csv"),
        "{result}"
    );
    assert!(!workspace.path().join("escaped.txt").exists());
}

#[tokio::test]
async fn a_program_changed_after_the_turn_froze_it_is_not_run() {
    let workspace = tempfile::tempdir().unwrap();
    let skill = tempfile::tempdir().unwrap();
    let tool = tool(skill.path(), "print('{}')\n", json!({}));
    std::fs::write(skill.path().join("run.py"), "print('[]')\n").unwrap();
    let output = tool
        .execute(&json!({"timeout_ms": 20000}), &context(workspace.path()))
        .await;
    assert!(output.is_error());
    assert!(output.content.contains("next turn"), "{}", output.content);
}

/// An echo server on loopback, and the port it listens on.
async fn echo() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = stream.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    port
}

/// A program that asks its proxy for `target` and reports what came back,
/// and whether it could also connect to `direct` without the proxy.
fn through_proxy(target: &str, direct: u16) -> String {
    format!(
        r#"import json, os, socket
proxy = os.environ.get('HTTPS_PROXY', '')
answer = {{'proxy': bool(proxy)}}
if proxy:
    host, port = proxy.rsplit('/', 1)[-1].split(':')
    s = socket.create_connection((host, int(port)), timeout=5)
    s.sendall(b'CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n')
    head = s.recv(1024).decode()
    answer['status'] = head.split(' ')[1] if ' ' in head else head
    if answer['status'] == '200':
        s.sendall(b'ping')
        answer['echo'] = s.recv(16).decode()
try:
    socket.create_connection(('127.0.0.1', {direct}), timeout=3)
    answer['direct'] = True
except OSError:
    answer['direct'] = False
print(json.dumps(answer))
"#
    )
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn a_declared_host_is_reached_through_the_proxy_and_nothing_else() {
    let allowed = echo().await;
    let other = echo().await;
    let workspace = tempfile::tempdir().unwrap();
    for (target, status, echoed) in [
        (format!("127.0.0.1:{allowed}"), "200", Some("ping")),
        (format!("127.0.0.1:{other}"), "403", None),
    ] {
        let skill = tempfile::tempdir().unwrap();
        let tool = tool(
            skill.path(),
            &through_proxy(&target, allowed),
            json!({"hosts": [format!("127.0.0.1:{allowed}")]}),
        );
        assert!(tool.leaves_machine());
        let output = tool
            .execute(&json!({"timeout_ms": 20000}), &context(workspace.path()))
            .await;
        assert!(!output.is_error(), "{}", output.content);
        let answer: Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(answer["proxy"], json!(true), "{answer}");
        assert_eq!(answer["status"], json!(status), "{target}: {answer}");
        assert_eq!(answer["echo"].as_str(), echoed, "{answer}");
        assert_eq!(
            answer["direct"],
            json!(false),
            "only through the proxy: {answer}"
        );
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn requirements_are_installed_once_into_their_own_environment() {
    let workspace = tempfile::tempdir().unwrap();
    let skill = tempfile::tempdir().unwrap();
    std::fs::write(skill.path().join("requirements.txt"), "").unwrap();
    let first = tool(
        skill.path(),
        "import json, sys\nprint(json.dumps({'prefix': sys.prefix}))\n",
        json!({"requirements": {"file": "requirements.txt", "hosts": ["127.0.0.1:9"]}}),
    );
    let env = Program::load(skill.path())
        .unwrap()
        .unwrap()
        .requirements
        .unwrap()
        .env;
    let _ = std::fs::remove_dir_all(&env);
    assert!(first.leaves_machine(), "the first call installs");
    let output = first
        .execute(&json!({"timeout_ms": 120000}), &context(workspace.path()))
        .await;
    assert!(!output.is_error(), "{}", output.content);
    let answer: Value = serde_json::from_str(&output.content).unwrap();
    let prefix = PathBuf::from(answer["prefix"].as_str().unwrap());
    assert_eq!(
        prefix.canonicalize().unwrap(),
        env.canonicalize().unwrap(),
        "{answer}"
    );
    let staging: Vec<_> = std::fs::read_dir(env.parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".staging"))
        .collect();
    assert!(staging.is_empty(), "no staging left behind");
    let again = tool(
        skill.path(),
        "import json, sys\nprint(json.dumps({'prefix': sys.prefix}))\n",
        json!({"requirements": {"file": "requirements.txt", "hosts": ["127.0.0.1:9"]}}),
    );
    assert!(
        !again.leaves_machine(),
        "installed: later calls stay on the machine"
    );
}

#[tokio::test]
async fn verification_runs_each_case_in_a_run_of_its_own_and_keeps_what_it_found() {
    vak_config::paths::isolate_home_for_tests();
    let workspace = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(workspace.path()).unwrap();
    let skill = tempfile::tempdir().unwrap();
    std::fs::write(
        skill.path().join("run.py"),
        "import json, sys\nargs = json.load(sys.stdin)\nprint(json.dumps({'each': args['amount'] / args['people']}))\n",
    )
    .unwrap();
    std::fs::write(
        skill.path().join(DECLARATION),
        json!({"entry": "run.py", "arguments": {"type": "object", "properties": {
            "amount": {"type": "number", "description": "The bill"},
            "people": {"type": "integer", "description": "How many share it"}
        }}})
        .to_string(),
    )
    .unwrap();
    let tests = skill.path().join(vak_tools::capability::TESTS_DIR);
    std::fs::create_dir(&tests).unwrap();
    std::fs::write(
        tests.join("even.json"),
        r#"{"arguments": {"amount": 90, "people": 3}, "expect": {"each": 30}}"#,
    )
    .unwrap();
    std::fs::write(
        tests.join("wrong.json"),
        r#"{"arguments": {"amount": 90, "people": 3}, "expect": {"each": 31}}"#,
    )
    .unwrap();
    std::fs::write(
        tests.join("nobody.json"),
        r#"{"arguments": {"amount": 90, "people": 0}, "fails": true}"#,
    )
    .unwrap();
    let core = vak_core::Core::new_with_trust(workspace.path().to_path_buf(), true).unwrap();
    core.set_tool_worker_exe(PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")));

    let verification = core
        .verify_skill_program("split", skill.path())
        .await
        .unwrap();
    let found: Vec<(&str, bool)> = verification
        .cases
        .iter()
        .map(|case| (case.name.as_str(), case.passed))
        .collect();
    assert_eq!(
        found,
        vec![("even", true), ("nobody", true), ("wrong", false)]
    );
    let wrong = &verification.cases[2];
    assert!(
        wrong.detail.as_deref().unwrap().contains("$.each is 30"),
        "{wrong:?}"
    );
    assert!(!verification.passed());

    let kept = core
        .skill_program_verification(&verification.digest)
        .unwrap();
    assert_eq!(kept, verification, "kept under the program's digest");
    let run = core.runs().get(verification.run).unwrap().unwrap();
    assert_eq!(
        run.status,
        vak_session::runs::RunStatus::Failed,
        "the run settled as its tests did"
    );

    let again = core
        .verify_skill_program("split", skill.path())
        .await
        .expect("the same program is verified again");
    assert_ne!(again.run, verification.run);
    assert_eq!(
        core.skill_program_verification(&verification.digest)
            .unwrap()
            .run,
        again.run,
        "the latest run is the one kept"
    );
}

/// A folder the Agent prepared: a program splitting a bill, and its tests.
fn prepared(workspace: &Path, folder: &str, divisor: &str, expect_each: u64) {
    let dir = workspace.join(folder);
    std::fs::create_dir_all(dir.join(vak_tools::capability::TESTS_DIR)).unwrap();
    std::fs::write(
        dir.join("run.py"),
        format!(
            "import json, sys\nargs = json.load(sys.stdin)\nprint(json.dumps({{'each': args['amount'] / {divisor}}}))\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join(DECLARATION),
        json!({"entry": "run.py", "arguments": {"type": "object", "properties": {
            "amount": {"type": "number", "description": "The bill"},
            "people": {"type": "integer", "description": "How many share it"}
        }}})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.join("tests/even.json"),
        json!({"arguments": {"amount": 90, "people": 3}, "expect": {"each": expect_each}})
            .to_string(),
    )
    .unwrap();
}

#[tokio::test]
async fn a_proposed_program_is_verified_versioned_and_admitted_by_its_tier() {
    vak_config::paths::isolate_home_for_tests();
    let workspace = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(workspace.path()).unwrap();
    let core = vak_core::Core::new_with_trust(workspace.path().to_path_buf(), true).unwrap();
    core.set_tool_worker_exe(PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")));
    let tool = vak_core::learning::ProposeSkillTool {
        sessions_home: core.scope().into_root(),
        cwd: workspace.path().to_path_buf(),
        session_id: "sess-propose".into(),
        core: Some(core.clone()),
    };
    let propose = |source: &str| {
        json!({"name": "split-bill-e3", "description": "Split a bill evenly",
               "instructions": "Call the tool with the amount and people.", "source": source})
    };
    let ctx = ToolContext::new(workspace.path().to_path_buf()).with_agent_id("vak");

    prepared(workspace.path(), "first", "args['people']", 30);
    let out = tool.execute(&propose("first"), &ctx).await;
    assert!(!out.is_error(), "{}", out.content);
    assert!(
        out.content.contains("yours from your next turn"),
        "{}",
        out.content
    );
    let installed = core.scope().skills().join("split-bill-e3");
    assert!(installed.join("SKILL.md").is_file() && installed.join("run.py").is_file());
    let skill = core
        .skills()
        .into_iter()
        .find(|skill| skill.name == "split-bill-e3")
        .expect("the Agent's own skill is discovered");
    assert_eq!(skill.provenance.as_deref(), Some("agent"));

    prepared(workspace.path(), "second", "max(args['people'], 1)", 30);
    let out = tool.execute(&propose("second"), &ctx).await;
    assert!(
        out.content.contains("yours from your next turn"),
        "{}",
        out.content
    );
    let artifact = core
        .artifacts()
        .list()
        .into_iter()
        .find(|artifact| artifact.kind == vak_core::artifacts::ArtifactKind::Skill)
        .expect("recorded in the Library");
    assert_eq!(artifact.versions.len(), 2);
    assert_eq!(artifact.versions[1].parent, Some(artifact.versions[0].id));
    assert!(
        artifact
            .versions
            .iter()
            .all(|version| version.verified.as_ref().is_some_and(|v| v.passed))
    );
    assert!(
        std::fs::read_to_string(installed.join("run.py"))
            .unwrap()
            .contains("max(")
    );

    prepared(workspace.path(), "wrong", "args['people']", 31);
    let out = tool.execute(&propose("wrong"), &ctx).await;
    assert!(out.content.contains("not admitted"), "{}", out.content);
    assert!(out.content.contains("$.each is 30"), "{}", out.content);
    assert!(
        !std::fs::read_to_string(installed.join("run.py"))
            .unwrap()
            .contains("31")
    );

    let mut held = ToolContext::new(workspace.path().to_path_buf()).with_agent_id("vak");
    held.provenance = vak_control::Provenance::BOTH;
    prepared(workspace.path(), "steered", "args['people']", 30);
    let out = tool.execute(&json!({"name": "steered-skill", "description": "Split", "instructions": "Use it.", "source": "steered"}), &held).await;
    assert!(
        out.content.contains("waits for the person's review"),
        "{}",
        out.content
    );
    assert!(!core.scope().skills().join("steered-skill").exists());
    let proposal = vak_core::learning::list_proposals(&core.scope(), workspace.path())
        .into_iter()
        .find(|proposal| proposal.name == "steered-skill")
        .expect("queued for review");
    let shared = tempfile::tempdir().unwrap();
    let library = core.artifacts();
    vak_core::learning::promote(
        &core.scope(),
        workspace.path(),
        &proposal.id,
        shared.path(),
        Some(&library),
    )
    .unwrap();
    assert!(
        shared.path().join("steered-skill/run.py").is_file(),
        "promoted with its program"
    );
    assert!(
        Program::load(&shared.path().join("steered-skill"))
            .unwrap()
            .is_some()
    );
}

/// Doc 93 E3b: with the owner's switch off, an ability that reaches only
/// the project's files still waits for review; the proposal and its inbox
/// entry say why and what it would reach, and approving it clears the
/// entry. A pack the skill carries draws its kind once the skill is live,
/// and a pack that names no form keeps the skill out.
#[tokio::test]
async fn the_owners_switch_and_a_skills_packs_are_honoured() {
    vak_config::paths::isolate_home_for_tests();
    let workspace = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(workspace.path()).unwrap();
    let project = workspace.path().join(".vak");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("config.toml"),
        "[owner]\nabilities_go_live = false\n",
    )
    .unwrap();
    let core = vak_core::Core::new_with_trust(workspace.path().to_path_buf(), true).unwrap();
    core.set_tool_worker_exe(PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")));
    assert!(!core.abilities_go_live(), "a narrower layer turns it off");
    let tool = vak_core::learning::ProposeSkillTool {
        sessions_home: core.scope().into_root(),
        cwd: workspace.path().to_path_buf(),
        session_id: "sess-switch".into(),
        core: Some(core.clone()),
    };
    let ctx = ToolContext::new(workspace.path().to_path_buf()).with_agent_id("vak");
    let propose = |name: &str, source: &str| {
        json!({"name": name, "description": "Split a bill evenly",
               "instructions": "Call the tool with the amount and people.", "source": source})
    };

    prepared(workspace.path(), "reviewed", "args['people']", 30);
    let out = tool
        .execute(&propose("reviewed-split", "reviewed"), &ctx)
        .await;
    assert!(
        out.content.contains("the owner reviews every new ability"),
        "{}",
        out.content
    );
    let proposal = vak_core::learning::list_proposals(&core.scope(), workspace.path())
        .into_iter()
        .find(|proposal| proposal.name == "reviewed-split")
        .expect("queued for review");
    let review = proposal.program.clone().expect("the review record");
    assert_eq!(review.why, "the owner reviews every new ability");
    assert!(
        review
            .reaches
            .iter()
            .any(|line| line == "Reads the files in this project")
    );
    assert!(
        review
            .reaches
            .iter()
            .any(|line| line == "Reaches nothing outside this computer")
    );
    let inbox = core.shared_scope().as_agent();
    let entry = vak_core::inbox::unread(&inbox)
        .into_iter()
        .find(|entry| entry.title == "reviewed-split is waiting for your review")
        .expect("the inbox says it is waiting");
    assert_eq!(entry.kind, vak_core::inbox::Kind::ProposalOpened);
    assert!(
        entry.body.contains("- Reads the files in this project"),
        "{}",
        entry.body
    );
    let shared = tempfile::tempdir().unwrap();
    vak_core::learning::promote(
        &core.scope(),
        workspace.path(),
        &proposal.id,
        shared.path(),
        Some(&core.artifacts()),
    )
    .unwrap();
    core.proposal_settled(&proposal.id);
    assert!(
        !vak_core::inbox::unread(&inbox)
            .iter()
            .any(|entry| entry.title == "reviewed-split is waiting for your review"),
        "approving it clears the entry"
    );

    std::fs::write(project.join("config.toml"), "").unwrap();
    core.refresh_persisted_preferences().unwrap();
    assert!(core.abilities_go_live(), "on unless a layer turns it off");
    let pack = |block: &str| {
        let mut spec: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../packages/answer-layouts/presentation/at-a-glance.json"),
            )
            .unwrap(),
        )
        .unwrap();
        spec["id"] = json!("split.receipt");
        spec["accepts"] = json!(["bill_split"]);
        spec["block"] = json!(block);
        spec.to_string()
    };
    prepared(workspace.path(), "badpack", "args['people']", 30);
    let dir = workspace.path().join("badpack/presentation");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("receipt.json"), pack("receipt")).unwrap();
    let out = tool
        .execute(&propose("packed-split", "badpack"), &ctx)
        .await;
    assert!(out.is_error(), "{}", out.content);
    assert!(
        out.content.contains("is not a card form"),
        "{}",
        out.content
    );
    assert!(!core.scope().skills().join("packed-split").exists());

    std::fs::write(dir.join("receipt.json"), pack("stats")).unwrap();
    let out = tool
        .execute(&propose("packed-split", "badpack"), &ctx)
        .await;
    assert!(
        out.content.contains("yours from your next turn"),
        "{}",
        out.content
    );
    assert_eq!(core.pack_kinds().block_of("bill_split"), Some("stats"));
}

/// Doc 93 E4: a skill proposed with `serves` gives its program's tool those
/// domains, so the tool surface defers it and the prompt's catalogue names
/// it; loading the skill shows its program's files and how to improve it,
/// and proposing it again under its name records a version whose parent is
/// the last one.
#[tokio::test]
async fn a_learned_ability_defers_by_what_it_serves_and_is_improved_from_its_files() {
    vak_config::paths::isolate_home_for_tests();
    let workspace = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(workspace.path()).unwrap();
    let core = vak_core::Core::new_with_trust(workspace.path().to_path_buf(), true).unwrap();
    core.set_tool_worker_exe(PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")));
    let tool = vak_core::learning::ProposeSkillTool {
        sessions_home: core.scope().into_root(),
        cwd: workspace.path().to_path_buf(),
        session_id: "sess-serves".into(),
        core: Some(core.clone()),
    };
    let ctx = ToolContext::new(workspace.path().to_path_buf()).with_agent_id("vak");
    let propose = |source: &str| {
        json!({"name": "bill-splitter", "description": "Split a bill evenly",
               "instructions": "Call the tool with the amount and people.",
               "source": source, "serves": ["documents", "Bill Splitting", "money"]})
    };
    prepared(workspace.path(), "v1", "args['people']", 30);
    let out = tool.execute(&propose("v1"), &ctx).await;
    assert!(
        out.content.contains("yours from your next turn"),
        "{}",
        out.content
    );

    let frozen: Vec<_> = vak_core::skills::frozen_from_capabilities(&core.capability_descriptors())
        .into_iter()
        .filter(|skill| skill.name == "bill-splitter")
        .collect();
    assert_eq!(frozen.len(), 1);
    let programs = vak_core::skills::programs(
        &frozen,
        PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")),
    );
    assert_eq!(programs[0].serves(), vec!["documents", "money"]);
    let surface = vak_core::capability::build_tool_surface(&programs, true, &Default::default());
    assert!(surface.core.is_empty());
    assert_eq!(surface.deferred[0].name, "bill-splitter");

    assert_eq!(frozen[0].provenance.as_deref(), Some("agent"));
    let loaded = frozen[0].load().unwrap();
    assert!(loaded.contains("You made this ability."), "{loaded}");
    assert!(
        loaded.contains("propose it again as `bill-splitter`"),
        "{loaded}"
    );
    assert!(loaded.contains("<file path=\"run.py\">"), "{loaded}");
    assert!(
        loaded.contains("<file path=\"tests/even.json\">"),
        "{loaded}"
    );

    prepared(workspace.path(), "v2", "max(args['people'], 1)", 30);
    let out = tool.execute(&propose("v2"), &ctx).await;
    assert!(
        out.content.contains("yours from your next turn"),
        "{}",
        out.content
    );
    let artifact = core
        .artifacts()
        .list()
        .into_iter()
        .find(|artifact| artifact.title.as_deref() == Some("bill-splitter"))
        .expect("recorded in the Library");
    assert_eq!(artifact.versions.len(), 2);
    assert_eq!(artifact.versions[1].parent, Some(artifact.versions[0].id));
}
