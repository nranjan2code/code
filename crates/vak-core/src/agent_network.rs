//! Capability-checked communication between isolated agent workspaces.
//!
//! The optional Unix transport is the only supported transport. It keeps
//! isolated Docker tasks off the network while preserving the same capability
//! checks as in-process callers.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use uuid::Uuid;

const DEFAULT_MESSAGE_LIMIT: usize = 256 * 1024;
const MAX_QUEUE_MESSAGES: usize = 256;
const MAX_QUEUE_BYTES: usize = 16 * 1024 * 1024;
const MAX_FRAME_BYTES: usize = 1024 * 1024;
const CAPABILITY_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceNetworkPolicy {
    pub enabled: bool,
    pub allowed_peers: HashSet<String>,
    pub max_message_bytes: usize,
}

impl Default for WorkspaceNetworkPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_peers: HashSet::new(),
            max_message_bytes: DEFAULT_MESSAGE_LIMIT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessage {
    pub from_workspace: String,
    pub to_workspace: String,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerCapability {
    pub workspace: String,
    token: String,
}

impl BrokerCapability {
    pub fn from_parts(workspace: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
            token: token.into(),
        }
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

#[derive(Clone)]
pub struct AgentNetworkBroker {
    inner: Arc<Mutex<BrokerState>>,
}

pub struct AgentNetworkTool {
    broker: AgentNetworkBroker,
    workspace: String,
}

impl AgentNetworkTool {
    pub fn new(broker: AgentNetworkBroker, workspace: impl Into<String>) -> Self {
        Self {
            broker,
            workspace: workspace.into(),
        }
    }
}

#[async_trait::async_trait]
impl vak_tools::Tool for AgentNetworkTool {
    fn name(&self) -> &str {
        "agent_network"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["messaging"]
    }

    fn description(&self) -> &str {
        "Send a bounded message to, or receive a message from, an explicitly authorized agent workspace."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["send", "receive"]},
                "destination_workspace": {"type": "string"},
                "message": {"type": "string", "maxLength": MAX_MESSAGE_SCHEMA_CHARS}
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &vak_tools::ToolContext,
    ) -> vak_tools::ToolOutput {
        let Some(capability) = self.broker.capability_for(&self.workspace) else {
            return vak_tools::ToolOutput::error("agent network is not enabled for this workspace");
        };
        match args.get("action").and_then(serde_json::Value::as_str) {
            Some("send") => {
                let Some(destination) = args
                    .get("destination_workspace")
                    .and_then(serde_json::Value::as_str)
                else {
                    return vak_tools::ToolOutput::error(
                        "agent_network send requires destination_workspace",
                    );
                };
                let Some(message) = args.get("message").and_then(serde_json::Value::as_str) else {
                    return vak_tools::ToolOutput::error("agent_network send requires message");
                };
                match self
                    .broker
                    .send_to(&capability, destination, message.as_bytes().to_vec())
                {
                    Ok(()) => vak_tools::ToolOutput::ok("message queued"),
                    Err(error) => vak_tools::ToolOutput::error(error),
                }
            }
            Some("receive") => match self.broker.receive(&capability) {
                Ok(Some(message)) => vak_tools::ToolOutput::ok(
                    serde_json::json!({
                        "from_workspace": message.from_workspace,
                        "message": String::from_utf8_lossy(&message.body)
                    })
                    .to_string(),
                ),
                Ok(None) => vak_tools::ToolOutput::ok("no messages available"),
                Err(error) => vak_tools::ToolOutput::error(error),
            },
            _ => vak_tools::ToolOutput::error("agent_network action must be send or receive"),
        }
    }

    fn claims(&self, _args: &serde_json::Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            exclusive: true,
            ..Default::default()
        }
    }
}

const MAX_MESSAGE_SCHEMA_CHARS: usize = 262_144;

impl Default for AgentNetworkBroker {
    fn default() -> Self {
        static BROKER: std::sync::OnceLock<Arc<Mutex<BrokerState>>> = std::sync::OnceLock::new();
        Self {
            inner: BROKER
                .get_or_init(|| Arc::new(Mutex::new(BrokerState::default())))
                .clone(),
        }
    }
}

#[derive(Default)]
struct BrokerState {
    policies: HashMap<String, WorkspaceNetworkPolicy>,
    capabilities: HashMap<String, CapabilityRecord>,
    queues: HashMap<String, VecDeque<AgentMessage>>,
}

struct CapabilityRecord {
    token: String,
    expires_at: std::time::Instant,
}

impl AgentNetworkBroker {
    fn capability_for(&self, workspace: &str) -> Option<BrokerCapability> {
        let state = self.lock();
        state
            .capabilities
            .get(workspace)
            .map(|record| BrokerCapability {
                workspace: workspace.to_string(),
                token: record.token.clone(),
            })
    }
    fn lock(&self) -> MutexGuard<'_, BrokerState> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn register(
        &self,
        workspace: impl Into<String>,
        policy: WorkspaceNetworkPolicy,
    ) -> BrokerCapability {
        let workspace = workspace.into();
        let token = Uuid::now_v7().to_string();
        let mut state = self.lock();
        state.policies.insert(workspace.clone(), policy);
        state.capabilities.insert(
            workspace.clone(),
            CapabilityRecord {
                token: token.clone(),
                expires_at: std::time::Instant::now() + CAPABILITY_TTL,
            },
        );
        state.queues.insert(workspace.clone(), VecDeque::new());
        BrokerCapability { workspace, token }
    }

    pub fn revoke(&self, capability: &BrokerCapability) -> bool {
        let mut state = self.lock();
        if state
            .capabilities
            .get(&capability.workspace)
            .is_some_and(|record| {
                record.token == capability.token && record.expires_at > std::time::Instant::now()
            })
        {
            state.capabilities.remove(&capability.workspace);
            state.policies.remove(&capability.workspace);
            state.queues.remove(&capability.workspace);
            true
        } else {
            false
        }
    }

    pub fn send(
        &self,
        sender: &BrokerCapability,
        destination: &BrokerCapability,
        body: Vec<u8>,
    ) -> Result<(), String> {
        let mut state = self.lock();
        validate_capability(&state, sender)?;
        validate_capability(&state, destination)?;
        if sender.workspace == destination.workspace {
            return Err("agent network requires a distinct destination workspace".into());
        }
        let source = state
            .policies
            .get(&sender.workspace)
            .ok_or_else(|| "sender workspace is not registered".to_string())?;
        let target = state
            .policies
            .get(&destination.workspace)
            .ok_or_else(|| "destination workspace is not registered".to_string())?;
        if !source.enabled || !target.enabled {
            return Err("agent network is disabled for one workspace".into());
        }
        if !source.allowed_peers.contains(&destination.workspace)
            || !target.allowed_peers.contains(&sender.workspace)
        {
            return Err("workspace-to-workspace edge is not authorized by both policies".into());
        }
        let limit = source.max_message_bytes.min(target.max_message_bytes);
        if body.len() > limit {
            return Err(format!("agent message exceeds {limit} byte limit"));
        }
        let queue = state
            .queues
            .entry(destination.workspace.clone())
            .or_default();
        let queued_bytes: usize = queue.iter().map(|message| message.body.len()).sum();
        if queue.len() >= MAX_QUEUE_MESSAGES
            || queued_bytes.saturating_add(body.len()) > MAX_QUEUE_BYTES
        {
            return Err("destination agent queue is full".into());
        }
        queue.push_back(AgentMessage {
            from_workspace: sender.workspace.clone(),
            to_workspace: destination.workspace.clone(),
            body,
        });
        Ok(())
    }

    pub fn send_to(
        &self,
        sender: &BrokerCapability,
        destination_workspace: &str,
        body: Vec<u8>,
    ) -> Result<(), String> {
        let destination = {
            let state = self.lock();
            let token = state
                .capabilities
                .get(destination_workspace)
                .ok_or_else(|| "destination workspace is not registered".to_string())?
                .token
                .clone();
            BrokerCapability::from_parts(destination_workspace, token)
        };
        self.send(sender, &destination, body)
    }

    pub fn receive(&self, receiver: &BrokerCapability) -> Result<Option<AgentMessage>, String> {
        let mut state = self.lock();
        validate_capability(&state, receiver)?;
        Ok(state
            .queues
            .get_mut(&receiver.workspace)
            .and_then(VecDeque::pop_front))
    }

    pub fn save_policies(&self, path: &std::path::Path) -> Result<(), String> {
        let state = self.lock();
        let policies = state.policies.clone();
        let parent = path
            .parent()
            .ok_or_else(|| "agent network policy file has no parent".to_string())?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("agent network policy directory: {error}"))?;
        let text = serde_json::to_vec_pretty(&policies)
            .map_err(|error| format!("serialize agent network policies: {error}"))?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text)
            .map_err(|error| format!("write agent network policies: {error}"))?;
        std::fs::rename(&temporary, path)
            .map_err(|error| format!("replace agent network policies: {error}"))?;
        Ok(())
    }

    pub fn load_policies(&self, path: &std::path::Path) -> Result<usize, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(format!("read agent network policies: {error}")),
        };
        let policies = serde_json::from_str::<HashMap<String, WorkspaceNetworkPolicy>>(&text)
            .map_err(|error| format!("parse agent network policies: {error}"))?;
        let mut loaded = 0;
        for (workspace, policy) in policies {
            let Some(workspace) = canonical_workspace(&workspace) else {
                continue;
            };
            if policy
                .allowed_peers
                .iter()
                .any(|peer| canonical_workspace(peer).is_none())
            {
                continue;
            }
            self.register(workspace, policy);
            loaded += 1;
        }
        Ok(loaded)
    }

    pub fn socket_path(sessions_home: &std::path::Path) -> std::path::PathBuf {
        sessions_home.join("agent-network").join("broker.sock")
    }

    #[cfg(unix)]
    pub async fn serve_unix(&self, socket: &std::path::Path) -> Result<(), String> {
        use std::os::unix::fs::FileTypeExt;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;

        let parent = socket
            .parent()
            .ok_or_else(|| "agent network socket has no parent".to_string())?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("agent network socket directory: {error}"))?;
        if socket.exists() {
            let metadata = tokio::fs::symlink_metadata(socket)
                .await
                .map_err(|error| format!("agent network socket metadata: {error}"))?;
            if !metadata.file_type().is_socket() {
                return Err("agent network socket path is not a socket".into());
            }
            if tokio::net::UnixStream::connect(socket).await.is_ok() {
                return Err("agent network broker is already running".into());
            }
            tokio::fs::remove_file(socket)
                .await
                .map_err(|error| format!("remove stale agent network socket: {error}"))?;
        }
        let listener = UnixListener::bind(socket)
            .map_err(|error| format!("bind agent network socket: {error}"))?;
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|error| format!("secure agent network socket: {error}"))?;

        loop {
            let (stream, _) = listener
                .accept()
                .await
                .map_err(|error| format!("accept agent network connection: {error}"))?;
            let broker = self.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut reader = BufReader::new(read);
                let mut line = Vec::with_capacity(8192);
                loop {
                    line.clear();
                    let mut complete = false;
                    loop {
                        let available = match reader.fill_buf().await {
                            Ok(available) if !available.is_empty() => available,
                            _ => break,
                        };
                        let take = available
                            .iter()
                            .position(|byte| *byte == b'\n')
                            .map_or(available.len(), |position| position + 1);
                        if line.len().saturating_add(take) > MAX_FRAME_BYTES {
                            return;
                        }
                        line.extend_from_slice(&available[..take]);
                        reader.consume(take);
                        if line.last() == Some(&b'\n') {
                            line.pop();
                            complete = true;
                            break;
                        }
                    }
                    if !complete {
                        break;
                    }
                    let Ok(line) = std::str::from_utf8(&line) else {
                        break;
                    };
                    let response = broker.handle_wire(line);
                    let mut encoded = match serde_json::to_vec(&response) {
                        Ok(value) => value,
                        Err(_) => break,
                    };
                    encoded.push(b'\n');
                    if write.write_all(&encoded).await.is_err() {
                        break;
                    }
                }
            });
        }
    }

    fn handle_wire(&self, line: &str) -> serde_json::Value {
        let Ok(request) = serde_json::from_str::<serde_json::Value>(line) else {
            return serde_json::json!({"ok": false, "error": "invalid JSON"});
        };
        match request.get("op").and_then(serde_json::Value::as_str) {
            Some("send") => self.handle_send_wire(&request),
            Some("receive") => self.handle_receive_wire(&request),
            _ => {
                serde_json::json!({"ok": false, "error": "operation is not available on the agent socket"})
            }
        }
    }

    fn handle_send_wire(&self, request: &serde_json::Value) -> serde_json::Value {
        use base64::Engine as _;
        let (Some(workspace), Some(token), Some(destination), Some(body)) = (
            request.get("sender_workspace").and_then(|v| v.as_str()),
            request.get("capability").and_then(|v| v.as_str()),
            request
                .get("destination_workspace")
                .and_then(|v| v.as_str()),
            request.get("body").and_then(|v| v.as_str()),
        ) else {
            return serde_json::json!({"ok": false, "error": "send fields are required"});
        };
        let Ok(body) = base64::engine::general_purpose::STANDARD.decode(body) else {
            return serde_json::json!({"ok": false, "error": "body must be standard base64"});
        };
        let sender = BrokerCapability::from_parts(workspace, token);
        match self.send_to(&sender, destination, body) {
            Ok(()) => serde_json::json!({"ok": true}),
            Err(error) => serde_json::json!({"ok": false, "error": error}),
        }
    }

    fn handle_receive_wire(&self, request: &serde_json::Value) -> serde_json::Value {
        use base64::Engine as _;
        let (Some(workspace), Some(token)) = (
            request.get("workspace").and_then(|v| v.as_str()),
            request.get("capability").and_then(|v| v.as_str()),
        ) else {
            return serde_json::json!({"ok": false, "error": "workspace and capability are required"});
        };
        let receiver = BrokerCapability::from_parts(workspace, token);
        match self.receive(&receiver) {
            Ok(Some(message)) => serde_json::json!({"ok": true, "message": {
                "from_workspace": message.from_workspace,
                "to_workspace": message.to_workspace,
                "body": base64::engine::general_purpose::STANDARD.encode(message.body)
            }}),
            Ok(None) => serde_json::json!({"ok": true, "message": null}),
            Err(error) => serde_json::json!({"ok": false, "error": error}),
        }
    }
}

fn canonical_workspace(raw: &str) -> Option<String> {
    let path = std::path::Path::new(raw).canonicalize().ok()?;
    path.is_dir().then(|| path.display().to_string())
}

fn validate_capability(state: &BrokerState, capability: &BrokerCapability) -> Result<(), String> {
    if state
        .capabilities
        .get(&capability.workspace)
        .is_some_and(|record| {
            record.token == capability.token && record.expires_at > std::time::Instant::now()
        })
    {
        Ok(())
    } else {
        Err("agent network capability is invalid or revoked".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(peer: &str) -> WorkspaceNetworkPolicy {
        WorkspaceNetworkPolicy {
            enabled: true,
            allowed_peers: [peer.to_string()].into_iter().collect(),
            max_message_bytes: 8,
        }
    }

    #[test]
    fn requires_explicit_bidirectional_authorization() {
        let broker = AgentNetworkBroker::default();
        let a_name = format!("a-{}", Uuid::now_v7());
        let b_name = format!("b-{}", Uuid::now_v7());
        let a = broker.register(a_name, policy(&b_name));
        let b = broker.register(
            b_name,
            WorkspaceNetworkPolicy {
                enabled: true,
                ..WorkspaceNetworkPolicy::default()
            },
        );
        assert!(matches!(
            broker.send(&a, &b, b"hello".to_vec()),
            Err(error) if error.contains("both policies")
        ));
    }

    #[test]
    fn delivers_bounded_messages_and_revocation_closes_access() {
        let broker = AgentNetworkBroker::default();
        let a_name = format!("a-{}", Uuid::now_v7());
        let b_name = format!("b-{}", Uuid::now_v7());
        let a = broker.register(a_name.clone(), policy(&b_name));
        let b = broker.register(b_name, policy(&a.workspace));
        assert!(broker.send(&a, &b, b"hello".to_vec()).is_ok());
        let message = broker.receive(&b).ok().flatten();
        assert_eq!(message.map(|message| message.body), Some(b"hello".to_vec()));
        assert!(broker.send(&a, &b, vec![0; 9]).is_err());
        assert!(broker.revoke(&b));
        assert!(broker.receive(&b).is_err());
        assert!(broker.send(&a, &b, b"again".to_vec()).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_transport_preserves_broker_authorization() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixStream;

        let root = tempfile::tempdir().ok();
        let Some(root) = root else { return };
        let a_dir = tempfile::tempdir().ok();
        let b_dir = tempfile::tempdir().ok();
        let (Some(a_dir), Some(b_dir)) = (a_dir, b_dir) else {
            return;
        };
        let a = a_dir.path().display().to_string();
        let b = b_dir.path().display().to_string();
        let socket = root.path().join("broker.sock");
        let broker = AgentNetworkBroker::default();
        let a_cap = broker.register(a.clone(), policy(&b));
        let b_cap = broker.register(b.clone(), policy(&a));
        let server = broker.clone();
        let server_socket = socket.clone();
        let task = tokio::spawn(async move {
            let _ = server.serve_unix(&server_socket).await;
        });
        for _ in 0..50 {
            if socket.exists() {
                break;
            }
            tokio::task::yield_now().await;
        }
        let stream = match UnixStream::connect(&socket).await {
            Ok(stream) => stream,
            Err(_) => {
                task.abort();
                return;
            }
        };
        let mut reader = BufReader::new(stream);
        use base64::Engine as _;
        let send = serde_json::json!({
            "op": "send", "sender_workspace": a,
            "capability": a_cap.token(), "destination_workspace": b,
            "body": base64::engine::general_purpose::STANDARD.encode("hello")
        });
        let mut line = serde_json::to_vec(&send).unwrap_or_default();
        line.push(b'\n');
        assert!(reader.get_mut().write_all(&line).await.is_ok());
        let mut response = String::new();
        assert!(reader.read_line(&mut response).await.is_ok());
        assert!(response.contains("\"ok\":true"));

        let receive = serde_json::json!({
            "op": "receive", "workspace": b_cap.workspace,
            "capability": b_cap.token()
        });
        let mut line = serde_json::to_vec(&receive).unwrap_or_default();
        line.push(b'\n');
        assert!(reader.get_mut().write_all(&line).await.is_ok());
        response.clear();
        assert!(reader.read_line(&mut response).await.is_ok());
        assert!(response.contains("from_workspace") && response.contains("aGVsbG8="));
        task.abort();
    }
}
