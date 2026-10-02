//! End-to-end guards for the capability lifecycle
//! (docs/design/41-capability-registry.md).
//!
//! Every test here pins a behaviour that was broken in a way no error
//! surfaced: the agent did not fail, it answered from memory and sounded
//! certain. That invisibility is why these are integration tests rather than
//! notes in a changelog.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::{BTreeSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_core::capability::registry::{CapabilityProvider, Declaration};
use vak_core::capability::{CapabilityId, CapabilityRegistry, Domain, Origin, Serves};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::types::CapabilityKind;

// ---------------------------------------------------------------- fakes ---

struct Fake {
    declarations: Mutex<Vec<Declaration>>,
}

impl Fake {
    fn new(declarations: Vec<Declaration>) -> Arc<Self> {
        Arc::new(Fake {
            declarations: Mutex::new(declarations),
        })
    }
}

#[async_trait::async_trait]
impl CapabilityProvider for Fake {
    fn declare(&self) -> Vec<Declaration> {
        self.declarations.lock().unwrap().clone()
    }
}

fn declaration(name: &str, kind: CapabilityKind) -> Declaration {
    Declaration {
        id: CapabilityId::new(kind, name),
        origin: Origin::Workspace,
        summary: String::new(),
        serves: Serves::Undeclared,
        digest: None,
        source: None,
        configuration: serde_json::Value::Null,
    }
}

struct Scripted {
    capacity_key: crate::support::CapacityKey,
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.0.clone()
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

fn reply(content: ContentBlock, stop: vak_llm::types::StopReason) -> AssistantMessage {
    AssistantMessage {
        content: vec![content],
        stop_reason: stop,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

fn spawns(marker: &Path) -> usize {
    std::fs::read_to_string(marker)
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

// ------------------------------------------------------------- the case ---

/// A configured MCP server is started by demand and nothing else
/// (AGENTS.md invariant 25). Admission, reconciliation and prompt assembly
/// all run without starting it; the model's first `mcp` call starts it once;
/// and what that call learned reaches the next turn's prompt with no restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_mcp_server_starts_on_demand_and_its_catalog_reaches_the_next_turn() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("workspace");
    std::fs::create_dir_all(&cwd).unwrap();
    let marker = dir.path().join("spawns");
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/fake_mcp_server.py")
        .canonicalize()
        .unwrap();

    let core = Core::new_with_trust(cwd, true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    let mut mcp = vak_config::McpConfig::default();
    mcp.servers.insert(
        "fake".into(),
        vak_config::McpServerConfig {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                format!(
                    "echo started >> '{}'; exec python3 '{}'",
                    marker.display(),
                    script.display()
                ),
            ],
            env: Default::default(),
            network: false,
            serves: Vec::new(),
        },
    );
    core.set_mcp_servers(mcp);

    core.admitted_capabilities().await;
    core.reconcile_capabilities().await;
    let before = core.system_prompt();
    assert!(before.contains("\n- fake\n"), "named, not yet listed");
    assert_eq!(
        spawns(&marker),
        0,
        "admission and prompt assembly start nothing"
    );

    core.set_provider_instance(Arc::new(Scripted {
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from(vec![
            reply(
                ContentBlock::ToolUse {
                    id: "m1".into(),
                    name: "mcp".into(),
                    input: serde_json::json!({"action": "list", "server": "fake"}),
                },
                vak_llm::types::StopReason::ToolUse,
            ),
            reply(
                ContentBlock::text("listed"),
                vak_llm::types::StopReason::EndTurn,
            ),
        ])),
    }));
    let session = core.start_session().await.unwrap();
    let (events, _rx) = tokio::sync::mpsc::channel(256);
    let (outcome, _) = core
        .run_turn_with(
            session,
            "what can the fake server do?",
            CancellationToken::new(),
            None,
            None,
            None,
            events,
        )
        .await
        .unwrap();
    assert!(matches!(outcome, vak_agent::TurnOutcome::Completed { .. }));
    assert_eq!(spawns(&marker), 1, "the model's call started it, once");

    core.reconcile_capabilities().await;
    assert!(
        core.system_prompt()
            .contains("- fake: echo, boom, secret_result"),
        "the observed catalog reaches the next prompt: {}",
        core.system_prompt()
    );
    assert_eq!(
        spawns(&marker),
        1,
        "learning the catalog started nothing new"
    );
}

// ------------------------------------------------------------ lifecycle ---

/// Add, edit and remove, on a registry that is never restarted.
#[tokio::test]
async fn capabilities_can_be_added_edited_and_removed_while_running() {
    let provider = Fake::new(vec![declaration("read", CapabilityKind::Tool)]);
    let (registry, _hints) = CapabilityRegistry::new(provider.clone());
    registry.reconcile().await;
    let first = registry.current().await.epoch;

    // --- added -----------------------------------------------------------
    provider
        .declarations
        .lock()
        .unwrap()
        .push(declaration("pdf", CapabilityKind::Skill));
    let delta = registry.reconcile().await.expect("adding is a change");
    assert_eq!(delta.added.len(), 1);
    assert!(delta.describe().contains("now available"));
    let second = registry.current().await.epoch;
    assert!(second > first, "a change publishes a new epoch");

    // --- edited ----------------------------------------------------------
    provider
        .declarations
        .lock()
        .unwrap()
        .iter_mut()
        .filter(|d| d.id.name == "pdf")
        .for_each(|d| d.digest = Some("rewritten".into()));
    let delta = registry.reconcile().await.expect("editing is a change");
    assert_eq!(delta.updated.len(), 1);
    assert!(registry.current().await.epoch > second);

    // --- removed ---------------------------------------------------------
    provider
        .declarations
        .lock()
        .unwrap()
        .retain(|d| d.id.name != "pdf");
    let delta = registry.reconcile().await.expect("removing is a change");
    assert_eq!(delta.removed.len(), 1);
    assert!(
        delta.describe().contains("no longer available"),
        "removal is announced, not silent"
    );
}

/// Reconciling an unchanged world must publish nothing.
///
/// A loop that ran every ten seconds for three weeks and republished each
/// time would churn every live session's prompt and defeat the point.
#[tokio::test]
async fn a_quiet_world_never_churns_the_epoch() {
    let provider = Fake::new(vec![declaration("read", CapabilityKind::Tool)]);
    let (registry, _hints) = CapabilityRegistry::new(provider);
    registry.reconcile().await;
    let epoch = registry.current().await.epoch;
    for _ in 0..25 {
        assert!(registry.reconcile().await.is_none());
    }
    assert_eq!(registry.current().await.epoch, epoch);
}

// ----------------------------------------------------------- revocation ---

/// Revocation is immediate and independent of any epoch. An operator
/// disabling a compromised plugin must not wait for a long turn to finish.
#[tokio::test]
async fn revocation_applies_immediately_and_restoration_republishes() {
    let provider = Fake::new(vec![declaration("bash", CapabilityKind::Tool)]);
    let (registry, _hints) = CapabilityRegistry::new(provider);
    registry.reconcile().await;
    let id = CapabilityId::new(CapabilityKind::Tool, "bash");

    registry.revoke(id.clone(), "operator disabled").await;
    assert_eq!(
        registry.revocation(&id).await.as_deref(),
        Some("operator disabled"),
        "visible at dispatch with no reconcile in between"
    );

    registry.reconcile().await;
    assert_eq!(registry.current().await.usable().count(), 0);

    registry.restore(&id).await;
    registry.reconcile().await;
    assert_eq!(registry.current().await.usable().count(), 1);
}

// -------------------------------------------------------------- domains ---

/// A domain this build has never heard of must round-trip and match itself,
/// so a plugin with its own vocabulary is not silently flattened.
#[test]
fn an_unknown_domain_survives_and_matches() {
    let parsed = Domain::parse("procurement");
    assert_eq!(parsed, Domain::Custom("procurement".into()));
    let serves = Serves::declared([parsed.clone()]);
    assert!(serves.serves_any(&BTreeSet::from([parsed])));
    assert!(!serves.serves_any(&BTreeSet::from([Domain::Web])));
}

/// A changed secret changes the server's declared digest, which is a change
/// to the world: admission sees pending work and the next turn binds the new
/// epoch, with no restart.
#[tokio::test]
async fn a_resolved_secret_is_a_pending_change_at_the_next_admission() {
    let mut decl = declaration("search_srv", CapabilityKind::McpServer);
    decl.digest = Some("unresolved_hash".into());
    let provider = Fake::new(vec![decl.clone()]);
    let (registry, _hints) = CapabilityRegistry::new(provider.clone());
    registry.reconcile().await;
    let first = registry.current().await.epoch;

    decl.digest = Some("resolved_hash_with_key".into());
    decl.configuration = serde_json::json!({"tools": [{"name": "search"}]});
    *provider.declarations.lock().unwrap() = vec![decl];
    assert!(registry.has_pending_changes().await);
    assert!(registry.reconcile().await.is_some());
    let current = registry.current().await;
    assert!(current.epoch > first);

    let inventory = current.mcp_inventory();
    let empty_policy = vak_config::ChannelPolicy::default();
    let builtins: BTreeSet<String> = ["read".to_string(), "glob".to_string()].into();
    let tc = vak_core::capability::TurnCapabilities::build(&vak_core::capability::TurnProbe {
        capabilities: &current,
        revoked_ids: BTreeSet::new(),
        channel_policy: &empty_policy,
        reach_standings: &[],
        mcp_inventory: &inventory,
        builtin_names: &builtins,
    });
    assert_eq!(
        tc.mcp_tool_index.get("search").map(String::as_str),
        Some("search_srv"),
        "a bare `search` call resolves to its server in turn 2"
    );
    assert!(tc.mcp_server_names.contains(&"search_srv".to_string()));
}

// -------------------------------------------------------- agent network ---

/// Inter-agent messaging is offered only while an operator has authorized
/// this workspace on the broker, and withdrawn when that authorization is.
#[tokio::test]
async fn agent_network_is_offered_only_while_the_workspace_is_authorized() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
    let offered = |core: &Core| core.tool_names().iter().any(|n| n == "agent_network");
    assert!(!offered(&core), "off by default");

    let broker = core.agent_network_broker();
    let capability = broker.register(
        core.cwd().canonicalize().unwrap().display().to_string(),
        vak_core::agent_network::WorkspaceNetworkPolicy {
            enabled: true,
            allowed_peers: ["peer-workspace".to_string()].into(),
            max_message_bytes: 1024,
        },
    );
    assert!(offered(&core));

    assert!(broker.revoke(&capability));
    assert!(
        !offered(&core),
        "revoking the registration withdraws the tool"
    );
}
