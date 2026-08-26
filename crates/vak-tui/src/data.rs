use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

type CommandCache = Vec<(String, String, String)>;

use tokio::sync::Mutex;
use vak_client::types::*;

/// Wraps `vak_client::Client` and provides the data-access interface the
/// TUI needs. All state lives on the server; this is a thin proxy with
/// local caches for config and paths.
#[derive(Clone)]
pub struct ClientData {
    client: vak_client::Client,
    config: Arc<Mutex<Option<ConfigResponse>>>,
    health: Arc<Mutex<Option<HealthResponse>>>,
    custom_commands: Arc<StdMutex<Option<CommandCache>>>,
}

impl ClientData {
    fn lock_cache(
        slot: &StdMutex<Option<CommandCache>>,
    ) -> std::sync::MutexGuard<'_, Option<CommandCache>> {
        slot.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn with_cache<T>(
        slot: &StdMutex<Option<CommandCache>>,
        f: impl FnOnce(&mut Option<CommandCache>) -> T,
    ) -> T {
        let mut guard = Self::lock_cache(slot);
        f(&mut guard)
    }

    pub async fn connect(url: &str, token: &str) -> Result<Self, String> {
        let client = vak_client::Client::new(url, token);
        let cfg = client
            .config()
            .await
            .map_err(|e| format!("config fetch failed: {e}"))?;
        Ok(Self {
            client,
            config: Arc::new(Mutex::new(Some(cfg))),
            health: Arc::new(Mutex::new(None)),
            custom_commands: Arc::new(StdMutex::new(None)),
        })
    }

    pub fn client(&self) -> &vak_client::Client {
        &self.client
    }

    // ── Config ───────────────────────────────────────────────────

    pub async fn config(&self) -> ConfigResponse {
        let mut guard = self.config.lock().await;
        if guard.is_none()
            && let Ok(cfg) = self.client.config().await
        {
            *guard = Some(cfg);
        }
        guard.clone().unwrap_or_default()
    }

    pub async fn refresh_config(&self) -> Result<ConfigResponse, String> {
        let cfg = self.client.config().await.map_err(|e| e.to_string())?;
        *self.config.lock().await = Some(cfg.clone());
        Ok(cfg)
    }

    pub async fn patch_config(&self, patch: &PatchConfigRequest) -> Result<(), String> {
        self.client
            .patch_config(patch)
            .await
            .map_err(|e| e.to_string())?;
        // Refresh cached config
        let _ = self.refresh_config().await;
        Ok(())
    }

    // ── Health ───────────────────────────────────────────────────

    pub async fn health(&self) -> Result<HealthResponse, String> {
        let h = self.client.health().await.map_err(|e| e.to_string())?;
        *self.health.lock().await = Some(h.clone());
        Ok(h)
    }

    // ── Sessions ─────────────────────────────────────────────────

    pub async fn list_sessions(&self) -> Result<SessionListResponse, String> {
        self.client.list_sessions().await.map_err(|e| e.to_string())
    }

    pub async fn create_session(&self, cwd: Option<&str>) -> Result<String, String> {
        let resp = self
            .client
            .create_session(cwd)
            .await
            .map_err(|e| e.to_string())?;
        Ok(resp.session_id)
    }

    pub async fn delete_session(&self, id: &str) -> Result<(), String> {
        self.client
            .delete_session(id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn attach_session(&self, id: &str) -> Result<String, String> {
        self.client
            .attach_session(id)
            .await
            .map_err(|e| e.to_string())
    }

    // ── Run / Steering ───────────────────────────────────────────

    pub async fn run_prompt(&self, session_id: &str, req: &RunRequest) -> Result<(), String> {
        self.client
            .run_prompt(session_id, req)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn steer(&self, session_id: &str, text: &str) -> Result<(), String> {
        self.client
            .steer(session_id, text)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn cancel_run(&self, session_id: &str) -> Result<(), String> {
        self.client
            .cancel_run(session_id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ── Subagents ────────────────────────────────────────────────

    pub async fn subagents_list(&self, session_id: &str) -> Result<SubagentsResponse, String> {
        self.client
            .subagents_list(session_id)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn subagent_steer(
        &self,
        session_id: &str,
        child: &str,
        text: &str,
    ) -> Result<(), String> {
        self.client
            .subagent_steer(session_id, child, text)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn events(
        &self,
        session_id: &str,
    ) -> impl futures::Stream<Item = Result<AgentEvent, String>> + '_ {
        use futures::StreamExt;
        let stream = self.client.events(session_id);
        async_stream::stream! {
            tokio::pin!(stream);
            while let Some(item) = stream.next().await {
                yield item.map_err(|e| e.to_string());
            }
        }
    }

    // ── Transcript ───────────────────────────────────────────────

    pub async fn transcript(&self, session_id: &str) -> Result<TranscriptResponse, String> {
        self.client
            .transcript(session_id)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn transcript_md(&self, session_id: &str) -> Result<String, String> {
        let path = format!("/sessions/{session_id}/transcript.md");
        let (status, body) = self
            .client
            .get_raw(&path)
            .await
            .map_err(|e| e.to_string())?;
        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("HTTP {status}: {body}"))
        }
    }

    // ── Search ───────────────────────────────────────────────────

    pub async fn search(
        &self,
        query: &str,
        limit: u32,
        all: bool,
        exclude: Option<&str>,
    ) -> Result<SearchResponse, String> {
        self.client
            .search(query, limit, all, exclude)
            .await
            .map_err(|e| e.to_string())
    }

    // ── FinOps ───────────────────────────────────────────────────

    pub async fn finops(&self) -> Result<FinopsResponse, String> {
        self.client.finops().await.map_err(|e| e.to_string())
    }

    // ── Checkpoints ──────────────────────────────────────────────

    pub async fn checkpoints(&self, session_id: &str) -> Result<Vec<Checkpoint>, String> {
        self.client
            .checkpoints(session_id)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn restore_checkpoint(&self, session_id: &str, seq: u32) -> Result<(), String> {
        self.client
            .checkpoint_restore(session_id, seq)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    // ── Compaction ───────────────────────────────────────────────

    pub async fn compact(&self, session_id: &str) -> Result<serde_json::Value, String> {
        self.client
            .compact(session_id)
            .await
            .map_err(|e| e.to_string())
    }

    // ── Permission mode / sandbox ────────────────────────────────

    pub async fn set_permission_mode(&self, mode: &str) -> Result<(), String> {
        self.client
            .set_permission_mode(mode)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub async fn sandbox_info(&self) -> Result<serde_json::Value, String> {
        self.client.sandbox_info().await.map_err(|e| e.to_string())
    }

    pub async fn set_sandbox_backend(&self, backend: Option<&str>) -> Result<(), String> {
        self.client
            .set_sandbox_backend(backend)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    // ── Skills ───────────────────────────────────────────────────

    pub async fn skills(&self) -> Result<Vec<Skill>, String> {
        self.client.skills().await.map_err(|e| e.to_string())
    }

    // ── Custom commands ──────────────────────────────────────────

    pub fn custom_commands(&self) -> Vec<(String, String)> {
        Self::with_cache(&self.custom_commands, |cache| {
            cache
                .as_ref()
                .map(|v| v.iter().map(|c| (c.0.clone(), c.1.clone())).collect())
                .unwrap_or_default()
        })
    }

    pub async fn refresh_custom_commands(&self) {
        let fetched = self
            .client
            .custom_commands()
            .await
            .map(|cmds| {
                cmds.into_iter()
                    .map(|c| (format!("/{}", c.name), c.description, c.template))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self::with_cache(&self.custom_commands, |slot| *slot = Some(fetched));
    }

    /// Expands `/name args` against the cached custom-command templates.
    /// Returns None when the input is not a known custom command.
    pub fn expand_custom(&self, input: &str) -> Option<String> {
        let trimmed = input.trim();
        let rest = trimmed.strip_prefix('/')?;
        let mut parts = rest.splitn(2, char::is_whitespace);
        let name = parts.next()?;
        let args = parts.next().unwrap_or("").trim();
        let cache = Self::lock_cache(&self.custom_commands);
        let (_, _, template) = cache
            .as_ref()?
            .iter()
            .find(|(n, _, _)| n == &format!("/{name}"))?;
        Some(template.replace("$ARGUMENTS", args))
    }

    // ── Tools / breaker introspection ────────────────────────────

    pub async fn tools(&self) -> Result<Vec<String>, String> {
        self.client.tools().await.map_err(|e| e.to_string())
    }

    pub async fn breaker(&self) -> Result<serde_json::Value, String> {
        self.client.breaker().await.map_err(|e| e.to_string())
    }

    // ── Memory ───────────────────────────────────────────────────

    pub async fn memory(&self) -> Result<Vec<MemoryNote>, String> {
        self.client.memory().await.map_err(|e| e.to_string())
    }

    pub async fn append_memory(&self, text: &str, tier: &str) -> Result<(), String> {
        let body = serde_json::json!({ "text": text, "tier": tier });
        self.client
            .post_json_status("/memory", &body)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn amend_memory(&self, note_id: &str, text: &str) -> Result<(), String> {
        let body = serde_json::json!({ "text": text });
        self.client
            .patch_json(&format!("/memory/{note_id}"), &body)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn forget_memory(&self, note_id: &str) -> Result<(), String> {
        self.client
            .delete(&format!("/memory/{note_id}"))
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ── Tasks ────────────────────────────────────────────────────

    pub async fn tasks(&self) -> Result<Vec<Task>, String> {
        self.client.tasks().await.map_err(|e| e.to_string())
    }

    pub async fn task_enable(&self, id: &str) -> Result<(), String> {
        self.client
            .task_enable(id)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub async fn task_disable(&self, id: &str) -> Result<(), String> {
        self.client
            .task_disable(id)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub async fn task_run_now(&self, id: &str) -> Result<(), String> {
        self.client
            .task_run_now(id)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    // ── Proposals ────────────────────────────────────────────────

    pub async fn proposals_list(&self) -> Result<Vec<ProposalInfo>, String> {
        self.client
            .proposals_list()
            .await
            .map(|r| r.proposals)
            .map_err(|e| e.to_string())
    }

    pub async fn proposal_promote(&self, id: &str) -> Result<String, String> {
        let v = self
            .client
            .proposal_promote(id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(v["promoted"].as_str().unwrap_or_default().to_string())
    }

    pub async fn proposal_reject(&self, id: &str) -> Result<(), String> {
        self.client
            .proposal_reject(id)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    // ── Inbox (list) ─────────────────────────────────────────────

    pub async fn inbox_list(&self, limit: u32, unread_only: bool) -> Result<InboxResponse, String> {
        self.client
            .inbox_list(limit, unread_only)
            .await
            .map_err(|e| e.to_string())
    }

    // ── Providers ────────────────────────────────────────────────

    pub async fn providers(&self) -> Result<Vec<ProviderInfo>, String> {
        self.client.providers().await.map_err(|e| e.to_string())
    }

    pub async fn discover_models(&self, provider: &str) -> Result<Vec<ModelInfo>, String> {
        self.client
            .discover_models(provider)
            .await
            .map_err(|e| e.to_string())
    }

    // ── Approvals ────────────────────────────────────────────────

    pub async fn answer_approval(
        &self,
        session_id: &str,
        approval_id: &str,
        approve: bool,
    ) -> Result<(), String> {
        self.client
            .answer_approval(session_id, approval_id, approve)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ── Doctor ───────────────────────────────────────────────────

    pub async fn doctor(&self) -> Result<serde_json::Value, String> {
        self.client.doctor().await.map_err(|e| e.to_string())
    }

    // ── Ops ──────────────────────────────────────────────────────

    pub async fn ops_status(&self) -> Result<serde_json::Value, String> {
        self.client.ops_status().await.map_err(|e| e.to_string())
    }

    // ── File read (confined to workspace) ────────────────────────

    pub async fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        let url = format!("/fs/file?path={path}");
        let (status, body) = self.client.get_raw(&url).await.map_err(|e| e.to_string())?;
        if status.is_success() {
            Ok(body.into_bytes())
        } else {
            Err(format!("HTTP {status}: {body}"))
        }
    }

    // ── Inbox ────────────────────────────────────────────────────

    pub async fn inbox_unread_count(&self) -> Result<u32, String> {
        let resp = self
            .client
            .inbox_unread_count()
            .await
            .map_err(|e| e.to_string())?;
        Ok(resp.count)
    }

    pub async fn inbox_ack(&self, id: &str) -> Result<bool, String> {
        self.client
            .inbox_ack(id)
            .await
            .map(|r| r.acked)
            .map_err(|e| e.to_string())
    }

    // ── Convenience ──────────────────────────────────────────────

    pub async fn cwd_async(&self) -> PathBuf {
        let cfg = self.config().await;
        PathBuf::from(cfg.paths.cwd)
    }

    pub async fn sessions_home(&self) -> PathBuf {
        let cfg = self.config().await;
        PathBuf::from(cfg.paths.sessions_home)
    }

    pub async fn provider_configured(&self, name: &str) -> bool {
        match self.client.providers().await {
            Ok(providers) => providers.iter().any(|p| p.name == name && p.configured),
            Err(_) => false,
        }
    }
}
