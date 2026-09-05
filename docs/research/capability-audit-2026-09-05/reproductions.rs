#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::{collections::BTreeSet, sync::Arc};
use vak_core::capability::*;
use vak_session::types::CapabilityKind;

fn cap(kind: CapabilityKind, name: &str) -> Capability {
    Capability {
        id: CapabilityId::new(kind, name),
        origin: Origin::Workspace,
        summary: String::new(),
        serves: Serves::Undeclared,
        digest: None,
        source: None,
        resolution: Resolution::Static,
        configuration: serde_json::Value::Null,
    }
}
fn project(
    set: &CapabilitySet,
    policy: &vak_config::ChannelPolicy,
    standings: &[vak_core::reach::Standing],
) -> TurnCapabilities {
    TurnCapabilities::build(&TurnProbe {
        capabilities: set,
        session_contract: None,
        channel_policy: policy,
        reach_standings: standings,
        required_domains: &BTreeSet::new(),
        mcp_inventory: None,
        orientation_floor: &[],
        builtin_names: vec![],
    })
}
#[test]
fn specific_mcp_allow_retains_server() {
    let set = CapabilitySet::new(1, vec![cap(CapabilityKind::McpServer, "search")]);
    let policy = vak_config::ChannelPolicy {
        mcp_allow: Some(vec!["search/query".into()]),
        ..Default::default()
    };
    assert_eq!(project(&set, &policy, &[]).mcp_server_names, vec!["search"]);
}
#[test]
fn blocked_mcp_instance_is_removed() {
    let set = CapabilitySet::new(1, vec![cap(CapabilityKind::McpServer, "search")]);
    let standing = vak_core::reach::Standing {
        tool: "mcp".into(),
        label: "mcp server `search`".into(),
        reach: vak_core::reach::Reach::Blocked,
        reason: "denied".into(),
        remedy: String::new(),
    };
    assert!(
        project(&set, &Default::default(), &[standing])
            .mcp_server_names
            .is_empty()
    );
}
#[test]
fn accepted_hook_event_reaches_turn() {
    let mut hook = cap(CapabilityKind::Hook, "pre-tool-use/true");
    hook.configuration = serde_json::json!({"event":"pre-tool-use","command":"true"});
    let set = CapabilitySet::new(1, vec![hook]);
    assert_eq!(project(&set, &Default::default(), &[]).hooks.len(), 1);
}
struct Provider;
#[async_trait::async_trait]
impl CapabilityProvider for Provider {
    fn declare(&self) -> Vec<Declaration> {
        vec![Declaration {
            id: CapabilityId::new(CapabilityKind::McpServer, "search"),
            origin: Origin::Workspace,
            summary: String::new(),
            serves: Serves::Undeclared,
            digest: None,
            source: None,
            configuration: serde_json::Value::Null,
            needs_probe: true,
        }]
    }
    async fn probe(&self, _: &CapabilityId) -> Result<ProbeReport, ProbeFailure> {
        Ok(ProbeReport {
            configuration: serde_json::json!({"tools":[{"name":"query"}]}),
            announces_changes: true,
        })
    }
}
#[tokio::test]
async fn unchanged_reconcile_preserves_catalog() {
    let (registry, _) = CapabilityRegistry::new(Arc::new(Provider));
    registry.reconcile().await;
    let before = registry.current().await;
    registry.reconcile().await;
    let after = registry.current().await;
    assert_eq!(before.descriptors(), after.descriptors());
}
#[test]
fn disabled_project_hook_shadows_shared_hook() {
    let root = tempfile::tempdir().unwrap();
    vak_config::paths::set_home_override(root.path());
    let shared = root.path().join("vak-home/.vak");
    let project = root.path().join("project/.vak");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let hook = "[[hooks]]\nevent = \"stop\"\ncommand = \"true\"\nenabled = ";
    std::fs::write(shared.join("config.toml"), format!("{hook}true\n")).unwrap();
    std::fs::write(project.join("config.toml"), format!("{hook}false\n")).unwrap();
    let core = vak_core::Core::new_with_trust(project.parent().unwrap().into(), true).unwrap();
    assert!(!core.effective_hooks().iter().any(|h| h.enabled));
}
#[test]
fn unmanaged_plugin_directory_is_not_enabled_command_source() {
    let root = tempfile::tempdir().unwrap();
    let commands = root.path().join(".vak/plugins/not-installed/commands");
    std::fs::create_dir_all(&commands).unwrap();
    std::fs::write(
        commands.join("ghost.md"),
        "---\ndescription: Ghost\n---\nGhost template",
    )
    .unwrap();
    let found = vak_core::custom_commands::discover_with_plugins(
        root.path(),
        &root.path().join("shared"),
        &[],
    );
    assert!(found.is_empty());
}
#[test]
fn project_command_wins_over_enabled_plugin_command() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join(".vak/commands");
    let package = root.path().join("package");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(package.join("commands")).unwrap();
    std::fs::write(
        project.join("review.md"),
        "---\ndescription: Project\n---\nProject template",
    )
    .unwrap();
    std::fs::write(
        package.join("commands/review.md"),
        "---\ndescription: Plugin\n---\nPlugin template",
    )
    .unwrap();
    let found = vak_core::custom_commands::discover_with_plugins(
        root.path(),
        &root.path().join("shared"),
        &[(package, "plugin:project:p".into())],
    );
    assert_eq!(found[0].source, "project");
}
struct Capture(std::sync::Mutex<Vec<vak_llm::ChatRequest>>);
#[async_trait::async_trait]
impl vak_llm::Provider for Capture {
    fn name(&self) -> &str {
        "audit"
    }
    async fn stream(
        &self,
        request: vak_llm::ChatRequest,
        _: tokio_util::sync::CancellationToken,
    ) -> Result<vak_llm::EventStream, vak_llm::LlmError> {
        self.0.lock().unwrap().push(request);
        let (mut sink, rx) = vak_llm::stream::channel(8);
        let message = vak_llm::AssistantMessage {
            content: vec![vak_llm::ContentBlock::text("Done.")],
            stop_reason: vak_llm::StopReason::EndTurn,
            usage: Default::default(),
            model: "test".into(),
        };
        sink.push(vak_llm::stream::StreamEvent::Start {
            partial: message.clone(),
        });
        sink.close_message(message).await;
        Ok(rx)
    }
}
async fn captured_turn_after_skill_change(add: bool) -> vak_llm::ChatRequest {
    let root = tempfile::tempdir().unwrap();
    vak_config::paths::set_home_override(root.path());
    let project = root.path().join("project");
    let path = project.join(".vak/skills/audit-skill/SKILL.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        project.join(".vak/config.toml"),
        "[intent]\nenabled = false\n[commitment]\nenabled = false\n",
    )
    .unwrap();
    let body = "---\nname: audit-skill\ndescription: Audit skill\n---\nInstructions.";
    if !add {
        std::fs::write(&path, body).unwrap();
    }
    let core = vak_core::Core::new_with_trust(project, true).unwrap();
    let capture = Arc::new(Capture(Default::default()));
    core.set_provider_instance(capture.clone());
    let session = core
        .start_session_with_route("audit".into(), "test".into())
        .await
        .unwrap();
    if add {
        std::fs::write(&path, body).unwrap();
    } else {
        std::fs::remove_file(&path).unwrap();
    }
    core.reconcile_capabilities().await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    core.run_turn(
        session,
        "hello",
        tokio_util::sync::CancellationToken::new(),
        tx,
    )
    .await
    .unwrap();
    drain.await.unwrap();
    capture.0.lock().unwrap().last().unwrap().clone()
}
#[tokio::test]
async fn existing_session_receives_new_skill_schema() {
    let request = captured_turn_after_skill_change(true).await;
    assert!(request.tools.iter().any(|t| t.name == "skill"));
}
#[tokio::test]
async fn removed_skill_disappears_from_actual_model_prompt() {
    let request = captured_turn_after_skill_change(false).await;
    assert!(!request.system.unwrap_or_default().contains("audit-skill"));
}
