//! Warm model discovery for turn routing (docs/design/15-reliability.md,
//! "How discovery stays warm").
//!
//! `Core::plan_route_ladder` admits a fallback leg only where discovery has
//! shown that a configured credential reaches the model, and it never talks
//! to a provider itself (invariants 7 and 25). Something has to keep
//! discovery warm ahead of the turns that read it. That used to be a person
//! opening a model list, on the one Core that served the list: every other
//! Core, each pooled gateway Core included, and any Core five minutes after
//! its last picker read planned primary-only ladders.
//!
//! The refresh is level-triggered, like the capability registry
//! (docs/design/41-capability-registry.md), in a loop of its own because a
//! capability pass is offline by construction and this one is not:
//!
//! * **Desired state.** Every credential of every keyed provider, as each
//!   Agent that planned a turn on this Core recently resolves it, has a
//!   catalogue younger than [`REFRESH_AFTER`].
//! * **One idempotent pass** fetches only what is due, and may run at any
//!   time. The ticker guarantees progress; hints (the first turn after a
//!   quiet spell, a key change) only make it run sooner.
//! * **A failure is a reason with a retry, never data.** It is recorded
//!   beside the last successful catalogue, never in place of it, and retried
//!   with capped backoff. Planning keeps that catalogue until
//!   [`PLAN_STALE_BOUND`]; nothing substitutes a static list (invariant 9).
//! * **A quiet Core stops asking.** Refreshing stops [`ACTIVE_WINDOW`] after
//!   its last planned turn, so an idle or evicted pooled Core does not poll
//!   providers indefinitely. Its next turn plans from the last catalogue and
//!   wakes the loop.
//!
//! A catalogue is keyed by `(provider, credential_id)`, and a credential id
//! fingerprints the base URL and the key together, so what one holds does
//! not depend on which workspace asked. The gateway's pool therefore shares
//! one [`ModelCatalogues`] among its Cores, and a channel Core built moments
//! ago plans from what the pool already knows. Sharing never widens
//! anything: a Core makes legs only from entries for credentials it resolves
//! itself.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use vak_llm::registry::ProviderAuth;
use vak_session::types::AgentIdentity;

use crate::{Core, CoreInner};

/// A catalogue this young is fresh: a model picker serves it without asking
/// the provider again.
pub const DISCOVERY_TTL: Duration = Duration::from_secs(300);

/// The refresh loop renews a catalogue once it is this old, so a Core in use
/// never plans from one past [`DISCOVERY_TTL`].
pub const REFRESH_AFTER: Duration = Duration::from_secs(240);

/// How long planning keeps the last successful catalogue when refreshes fail
/// or the Core has been quiet. A model withdrawn meanwhile costs one fast
/// failed dispatch on a fallback leg, which is reached only after the
/// primary failed; dropping the catalogue costs the whole fallback.
pub const PLAN_STALE_BOUND: Duration = Duration::from_secs(24 * 60 * 60);

/// A Core keeps refreshing for this long after its last planned turn.
pub const ACTIVE_WINDOW: Duration = Duration::from_secs(30 * 60);

/// How often the loop reconciles with no hint: the safety net that makes a
/// lost hint cost latency, never a cold ladder.
pub const RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

// A renewal lands within one tick of falling due, still inside the TTL.
const _: () =
    assert!(REFRESH_AFTER.as_secs() + RECONCILE_INTERVAL.as_secs() < DISCOVERY_TTL.as_secs());

/// The wait after a failed fetch. It doubles with each consecutive failure
/// up to [`RETRY_CAP`], and retrying never stops.
const RETRY_BASE: Duration = Duration::from_secs(15);
const RETRY_CAP: Duration = Duration::from_secs(300);

/// Hints are coalesced over this window.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// `(provider, credential_id)`.
pub(crate) type CatalogueKey = (String, String);

struct Catalogue {
    fetched_at: Instant,
    models: Arc<Vec<String>>,
}

struct Failure {
    reason: String,
    attempts: u32,
    failed_at: Instant,
    retry_at: Instant,
}

#[derive(Default)]
struct Store {
    catalogues: HashMap<CatalogueKey, Catalogue>,
    failures: HashMap<CatalogueKey, Failure>,
    in_flight: HashSet<CatalogueKey>,
}

/// The last successful model catalogue per credential, and why the latest
/// attempt failed where one did. A handle: clones share one store.
#[derive(Clone, Default)]
pub struct ModelCatalogues(Arc<Mutex<Store>>);

/// One credential's discovery, for diagnostics. It names the credential by
/// its fingerprint, never by the credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDiscoveryStatus {
    pub provider: String,
    pub credential_id: String,
    /// Models in the catalogue planning uses, if there is one.
    pub models: Option<usize>,
    /// How old that catalogue is.
    pub age: Option<Duration>,
    /// Why the latest attempt failed, while no later one has succeeded.
    pub failure: Option<String>,
    /// How long until the refresh loop tries again after that failure.
    pub retry_in: Option<Duration>,
}

fn age(now: Instant, then: Instant) -> Duration {
    now.saturating_duration_since(then)
}

fn retry_delay(attempts: u32) -> Duration {
    let factor = 1u32
        .checked_shl(attempts.saturating_sub(1))
        .unwrap_or(u32::MAX);
    RETRY_BASE.saturating_mul(factor).min(RETRY_CAP)
}

impl ModelCatalogues {
    fn store(&self) -> MutexGuard<'_, Store> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether `other` is a handle on this same store.
    pub fn same_store(&self, other: &ModelCatalogues) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// `key`'s catalogue while it is younger than [`DISCOVERY_TTL`].
    pub(crate) fn fresh(&self, key: &CatalogueKey, now: Instant) -> Option<Arc<Vec<String>>> {
        self.store()
            .catalogues
            .get(key)
            .filter(|catalogue| age(now, catalogue.fetched_at) < DISCOVERY_TTL)
            .map(|catalogue| catalogue.models.clone())
    }

    /// Every catalogue planning may use: younger than [`PLAN_STALE_BOUND`].
    pub(crate) fn usable(&self, now: Instant) -> Vec<(CatalogueKey, Arc<Vec<String>>)> {
        self.store()
            .catalogues
            .iter()
            .filter(|(_, catalogue)| age(now, catalogue.fetched_at) < PLAN_STALE_BOUND)
            .map(|(key, catalogue)| (key.clone(), catalogue.models.clone()))
            .collect()
    }

    /// Whether a pass should fetch `key` at `now`: never fetched, or older
    /// than [`REFRESH_AFTER`], and neither in flight nor waiting out a
    /// failure's retry.
    fn due(&self, key: &CatalogueKey, now: Instant) -> bool {
        let store = self.store();
        !store.in_flight.contains(key)
            && store
                .failures
                .get(key)
                .is_none_or(|failure| now >= failure.retry_at)
            && store
                .catalogues
                .get(key)
                .is_none_or(|catalogue| age(now, catalogue.fetched_at) >= REFRESH_AFTER)
    }

    /// Mark `key` in flight unless it already is. Dropping the claim
    /// releases it however the fetch ended: a claim outliving a cancelled
    /// fetch would stop that credential refreshing for good.
    fn claim(&self, key: &CatalogueKey) -> Option<Claim> {
        self.store().in_flight.insert(key.clone()).then(|| Claim {
            catalogues: self.clone(),
            key: key.clone(),
        })
    }

    /// Replace `key`'s catalogue and clear its failure. Returns whether it
    /// had been failing, so a recovery is reported once.
    pub(crate) fn record_success(
        &self,
        key: &CatalogueKey,
        models: Vec<String>,
        now: Instant,
    ) -> bool {
        let mut store = self.store();
        store.catalogues.insert(
            key.clone(),
            Catalogue {
                fetched_at: now,
                models: Arc::new(models),
            },
        );
        store.failures.remove(key).is_some()
    }

    /// Record why fetching `key` failed and when to try again, leaving the
    /// last successful catalogue as it was. Returns the wait before the next
    /// attempt, and whether this failure starts a failing streak.
    pub(crate) fn record_failure(
        &self,
        key: &CatalogueKey,
        reason: &str,
        now: Instant,
    ) -> (Duration, bool) {
        let mut store = self.store();
        let failure = store.failures.entry(key.clone()).or_insert(Failure {
            reason: String::new(),
            attempts: 0,
            failed_at: now,
            retry_at: now,
        });
        let started = failure.attempts == 0;
        failure.attempts = failure.attempts.saturating_add(1);
        let wait = retry_delay(failure.attempts);
        failure.reason = reason.to_string();
        failure.failed_at = now;
        failure.retry_at = now.checked_add(wait).unwrap_or(now);
        (wait, started)
    }

    /// Forget `keys` entirely, so the next read or pass asks again.
    pub(crate) fn forget(&self, keys: &[CatalogueKey]) {
        let mut store = self.store();
        for key in keys {
            store.catalogues.remove(key);
            store.failures.remove(key);
        }
    }

    /// Drop what planning can no longer use.
    fn prune(&self, now: Instant) {
        let mut store = self.store();
        store
            .catalogues
            .retain(|_, catalogue| age(now, catalogue.fetched_at) < PLAN_STALE_BOUND);
        store
            .failures
            .retain(|_, failure| age(now, failure.failed_at) < PLAN_STALE_BOUND);
    }

    fn status(&self, key: &CatalogueKey, now: Instant) -> ModelDiscoveryStatus {
        let store = self.store();
        let catalogue = store
            .catalogues
            .get(key)
            .filter(|catalogue| age(now, catalogue.fetched_at) < PLAN_STALE_BOUND);
        let failure = store.failures.get(key);
        ModelDiscoveryStatus {
            provider: key.0.clone(),
            credential_id: key.1.clone(),
            models: catalogue.map(|catalogue| catalogue.models.len()),
            age: catalogue.map(|catalogue| age(now, catalogue.fetched_at)),
            failure: failure.map(|failure| failure.reason.clone()),
            retry_in: failure.map(|failure| failure.retry_at.saturating_duration_since(now)),
        }
    }
}

struct Claim {
    catalogues: ModelCatalogues,
    key: CatalogueKey,
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.catalogues.store().in_flight.remove(&self.key);
    }
}

/// One Core's side of discovery: the store it plans from, the Agents that
/// planned turns on it recently, and the refresh loop's wake-up channel.
#[derive(Default)]
pub(crate) struct DiscoveryRuntime {
    catalogues: Mutex<ModelCatalogues>,
    /// Agent id → when it last planned a turn here, and the identity its
    /// credentials resolve as. Agent-private secret scopes mean two Agents
    /// on one Core can hold different keys.
    demand: Mutex<HashMap<String, (Instant, Option<AgentIdentity>)>>,
    /// Set once the loop runs. It lives and dies with the Core, and its
    /// closing is what ends the loop.
    hints: OnceLock<mpsc::UnboundedSender<()>>,
}

impl DiscoveryRuntime {
    fn catalogues(&self) -> ModelCatalogues {
        self.catalogues
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn hint(&self) {
        if let Some(hints) = self.hints.get() {
            let _ = hints.send(());
        }
    }

    /// Record that `agent` planned a turn at `now`. Returns whether it had
    /// been quiet for [`ACTIVE_WINDOW`] or longer, or had never planned one.
    fn note(&self, agent: Option<AgentIdentity>, now: Instant) -> bool {
        let id = agent.as_ref().map(|a| a.id.clone()).unwrap_or_default();
        let mut demand = self.demand.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = demand.get(&id).map(|(at, _)| *at);
        demand.insert(id, (previous.map_or(now, |at| at.max(now)), agent));
        previous.is_none_or(|at| age(now, at) >= ACTIVE_WINDOW)
    }

    /// The Agents that planned a turn within [`ACTIVE_WINDOW`] of `now`;
    /// the rest are forgotten until they plan again.
    fn active(&self, now: Instant) -> Vec<Option<AgentIdentity>> {
        let mut demand = self.demand.lock().unwrap_or_else(PoisonError::into_inner);
        demand.retain(|_, (at, _)| age(now, *at) < ACTIVE_WINDOW);
        demand.values().map(|(_, agent)| agent.clone()).collect()
    }
}

impl Core {
    /// The catalogues this Core plans from and refreshes into.
    pub fn model_catalogues(&self) -> ModelCatalogues {
        self.inner.discovery.catalogues()
    }

    /// Plan from, and refresh into, `catalogues` from now on. The gateway's
    /// pool hands every Core it builds the store of its default Core.
    pub fn share_model_catalogues(&self, catalogues: ModelCatalogues) {
        *self
            .inner
            .discovery
            .catalogues
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = catalogues;
    }

    /// Start this Core's background refresh. Idempotent; without a Tokio
    /// runtime it does nothing. Returns whether this call started it.
    ///
    /// Long-lived hosts call it: the server for its own Core, and the pool
    /// for each Core it builds. Starting counts as demand, so a host that
    /// expects turns has catalogues before the first one. The loop holds the
    /// Core only while a pass runs, so dropping the Core ends it.
    pub fn start_model_discovery(&self) -> bool {
        if tokio::runtime::Handle::try_current().is_err() {
            return false;
        }
        let (hints, receiver) = mpsc::unbounded_channel();
        if self.inner.discovery.hints.set(hints).is_err() {
            return false;
        }
        self.inner
            .discovery
            .note(self.agent_identity.clone(), Instant::now());
        tokio::spawn(refresh_loop(Arc::downgrade(&self.inner), receiver));
        true
    }

    /// Whether this Core's background refresh has been started.
    pub fn model_discovery_started(&self) -> bool {
        self.inner.discovery.hints.get().is_some()
    }

    /// Note that this Core's Agent is planning a turn. When it had been
    /// quiet, its catalogues may be older than the refresh would have kept
    /// them, so the loop is woken now rather than at its next tick.
    pub(crate) fn note_route_demand(&self) {
        if self
            .inner
            .discovery
            .note(self.agent_identity.clone(), Instant::now())
        {
            self.hint_model_discovery();
        }
    }

    pub(crate) fn hint_model_discovery(&self) {
        self.inner.discovery.hint();
    }

    /// One idempotent pass at `now` over `providers`: fetch every due
    /// catalogue among the credentials each recently active Agent resolves,
    /// concurrently, recording each result as it lands.
    pub(crate) async fn reconcile_model_discovery_at(&self, now: Instant, providers: &[String]) {
        let catalogues = self.model_catalogues();
        catalogues.prune(now);
        let mut due: Vec<(CatalogueKey, ProviderAuth)> = Vec::new();
        for agent in self.inner.discovery.active(now) {
            let mut resolver = self.clone();
            resolver.agent_identity = agent;
            for provider in providers {
                // Nothing resolves: the provider is not keyed, so there is
                // nothing to keep warm and nothing to report.
                let Ok(pool) = resolver.provider_auth_pool_for(provider) else {
                    continue;
                };
                for auth in pool {
                    let Some(credential_id) = auth.credential_id.clone() else {
                        continue;
                    };
                    let key = (provider.clone(), credential_id);
                    if !due.iter().any(|(queued, _)| *queued == key) && catalogues.due(&key, now) {
                        due.push((key, auth));
                    }
                }
            }
        }
        let fetches = due.into_iter().filter_map(|(key, auth)| {
            let claim = catalogues.claim(&key)?;
            let catalogues = catalogues.clone();
            Some(async move {
                let result = vak_llm::models::list_models(&claim.key.0, &auth).await;
                settle(&catalogues, &claim.key, result, now);
            })
        });
        futures::future::join_all(fetches).await;
    }

    /// Discovery for every credential this Core resolves: what planning can
    /// use, and why a refresh is failing where one is.
    pub fn model_discovery_status(&self) -> Vec<ModelDiscoveryStatus> {
        let now = Instant::now();
        let catalogues = self.model_catalogues();
        let mut statuses = Vec::new();
        for provider in self.provider_names() {
            let Ok(pool) = self.provider_auth_pool_for(&provider) else {
                continue;
            };
            for auth in pool {
                if let Some(credential_id) = auth.credential_id {
                    statuses.push(catalogues.status(&(provider.clone(), credential_id), now));
                }
            }
        }
        statuses
    }
}

/// Record one fetch. The log speaks on a change of state, the first failure
/// of a streak and the recovery, never on every retry.
fn settle(
    catalogues: &ModelCatalogues,
    key: &CatalogueKey,
    result: Result<Vec<String>, vak_llm::LlmError>,
    now: Instant,
) {
    let (provider, credential_id) = key;
    let credential = credential_id.get(..8).unwrap_or(credential_id);
    match result {
        Ok(models) => {
            if catalogues.record_success(key, models, now) {
                eprintln!("[discovery] {provider} ({credential}): listing models again");
            }
        }
        Err(error) => {
            let (wait, started) = catalogues.record_failure(key, &error.to_string(), now);
            if started {
                eprintln!(
                    "[discovery] {provider} ({credential}): could not list models: {error}; \
                     retrying in {}s",
                    wait.as_secs()
                );
            }
        }
    }
}

/// The background refresh. It upgrades its `Weak` only for the length of a
/// pass, and the hint sender lives in the Core, so the last handle dropping
/// ends the loop.
async fn refresh_loop(inner: Weak<CoreInner>, mut hints: mpsc::UnboundedReceiver<()>) {
    let mut ticker = tokio::time::interval(RECONCILE_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            hint = hints.recv() => {
                if hint.is_none() {
                    return;
                }
                tokio::time::sleep(DEBOUNCE).await;
                while hints.try_recv().is_ok() {}
            }
        }
        let Some(inner) = inner.upgrade() else {
            return;
        };
        let core = Core::from_inner(inner);
        let providers = core.provider_names();
        core.reconcile_model_discovery_at(Instant::now(), &providers)
            .await;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const MODEL: &str = "shared-model";

    /// What a fake `/models` endpoint answers.
    #[derive(Clone)]
    enum Reply {
        Catalogue(Vec<String>),
        Status(u16),
    }

    /// A loopback stand-in for one provider's model listing: it answers every
    /// request with `reply` and counts them. Plain blocking std I/O on its own
    /// thread, so it needs neither an HTTP dependency nor the test's runtime.
    struct ModelsEndpoint {
        base_url: String,
        requests: Arc<AtomicUsize>,
        reply: Arc<Mutex<Reply>>,
    }

    impl ModelsEndpoint {
        fn serving(models: &[&str]) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(AtomicUsize::new(0));
            let reply = Arc::new(Mutex::new(Reply::Catalogue(
                models.iter().map(|m| m.to_string()).collect(),
            )));
            let (count, answer) = (requests.clone(), reply.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => request.extend_from_slice(&chunk[..n]),
                        }
                    }
                    count.fetch_add(1, Ordering::SeqCst);
                    let (status, body) = match answer.lock().unwrap().clone() {
                        Reply::Catalogue(ids) => (
                            200,
                            serde_json::json!({
                                "data": ids.iter().map(|id| serde_json::json!({"id": id})).collect::<Vec<_>>()
                            })
                            .to_string(),
                        ),
                        Reply::Status(code) => (code, r#"{"error":"catalogue unavailable"}"#.into()),
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
                         content-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
            });
            ModelsEndpoint {
                base_url,
                requests,
                reply,
            }
        }

        fn requests(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }

        fn answer(&self, reply: Reply) {
            *self.reply.lock().unwrap() = reply;
        }
    }

    /// The endpoint behind every provider whose base URL is process-wide
    /// rather than per workspace (`VAK_OPENCODE_ZEN_BASE_URL`). Set once per
    /// test binary, so tests running in parallel all agree on it.
    fn shared_endpoint() -> &'static ModelsEndpoint {
        static ENDPOINT: OnceLock<ModelsEndpoint> = OnceLock::new();
        ENDPOINT.get_or_init(|| {
            let endpoint = ModelsEndpoint::serving(&[MODEL, "other-model"]);
            vak_config::set_override("VAK_OPENCODE_ZEN_BASE_URL", endpoint.base_url.clone());
            endpoint
        })
    }

    /// A trusted workspace whose project layer points Anthropic at `anthropic`
    /// and whose project secret scope holds `secrets`. Every secret a test
    /// stores through it is removed again on drop, so no test leaves a
    /// credential behind.
    struct Workspace {
        dir: tempfile::TempDir,
        secrets: Mutex<Vec<(std::path::PathBuf, &'static str)>>,
    }

    impl Workspace {
        fn new(anthropic: &ModelsEndpoint, secrets: &[(&'static str, &str)]) -> Self {
            crate::isolate_global_config();
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join(".vak")).unwrap();
            std::fs::write(
                dir.path().join(".vak/config.toml"),
                format!(
                    "anthropic_base_url = \"{}\"\nroute = {{ same_model = [[\"anthropic/shared-model\", \"opencode-zen/shared-model\"]] }}\n",
                    anthropic.base_url
                ),
            )
            .unwrap();
            let workspace = Workspace {
                dir,
                secrets: Mutex::new(Vec::new()),
            };
            let scope = workspace.dir.path().join(".env");
            for (var, value) in secrets {
                workspace.store_secret(&scope, var, value);
            }
            workspace
        }

        fn store_secret(&self, scope: &std::path::Path, var: &'static str, value: &str) {
            vak_config::upsert_env_file(scope, var, value).unwrap();
            self.secrets
                .lock()
                .unwrap()
                .push((scope.to_path_buf(), var));
        }

        /// A new Core over this workspace: its own shared state, exactly what
        /// the gateway's pool builds for a channel.
        fn core(&self) -> Core {
            let core = Core::new(self.dir.path().to_path_buf()).unwrap();
            core.set_shared_scope(vak_config::scope::SharedScope::new(
                self.dir.path().join("home"),
            ));
            core
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            for (scope, var) in self.secrets.lock().unwrap().iter() {
                let _ = vak_config::remove_env_file_key(scope, var);
            }
        }
    }

    fn primary_leg(core: &Core) -> vak_llm::RouteLeg {
        vak_llm::RouteLeg {
            provider: "anthropic".into(),
            model: MODEL.into(),
            dialect: vak_llm::EndpointDialect::for_provider("anthropic", true),
            credential_id: core
                .provider_auth_for_leg("anthropic", None)
                .unwrap()
                .credential_id,
        }
    }

    /// The reported defect, reproduced before anything else here existed. Two
    /// keyed providers serve the primary model, yet a turn's ladder has no
    /// fallback until something runs discovery, and planning never runs it.
    /// Before this module the only callers were a person opening a model
    /// list, on the one Core that served the list.
    #[tokio::test]
    async fn a_ladder_has_no_fallback_until_discovery_has_run() {
        shared_endpoint();
        let anthropic = ModelsEndpoint::serving(&[MODEL]);
        let workspace = Workspace::new(
            &anthropic,
            &[
                ("ANTHROPIC_API_KEY", "sk-ant-repro-primary"),
                ("OPENCODE_API_KEY", "oz-repro-backup"),
            ],
        );
        let core = workspace.core();
        let primary = primary_leg(&core);

        let cold = core.plan_route_ladder(primary.clone(), None);
        assert_eq!(
            cold.ladder,
            vec![primary.clone()],
            "no discovery, no fallback"
        );
        assert!(
            cold.annotations
                .iter()
                .any(|a| a.contains("single point of failure")),
            "{:?}",
            cold.annotations
        );

        let models = core.discover_models("opencode-zen").await.unwrap();
        assert!(models.contains(&MODEL.to_string()), "{models:?}");
        let warm = core.plan_route_ladder(primary.clone(), None);
        assert_eq!(warm.ladder.len(), 2, "{:?}", warm.ladder);
        assert_eq!(warm.ladder[0], primary);
        assert_eq!(
            (
                warm.ladder[1].provider.as_str(),
                warm.ladder[1].model.as_str()
            ),
            ("opencode-zen", MODEL)
        );

        // Another Core over the same workspace and keys, as the pool builds
        // one per channel workspace, with nothing sharing or refreshing its
        // catalogues: nobody opened a model list on it.
        let pooled = workspace.core();
        let pooled_plan = pooled.plan_route_ladder(primary_leg(&pooled), None);
        assert_eq!(pooled_plan.ladder.len(), 1, "{:?}", pooled_plan.ladder);
        assert_eq!(anthropic.requests(), 0, "planning never reaches a provider");
    }

    fn credential_keys(core: &Core, provider: &str) -> Vec<CatalogueKey> {
        core.provider_credential_ids(provider)
            .into_iter()
            .map(|id| (provider.to_string(), id))
            .collect()
    }

    fn note_turn(core: &Core, at: Instant) {
        core.inner.discovery.note(core.agent_identity.clone(), at);
    }

    fn anthropic_only() -> Vec<String> {
        vec!["anthropic".to_string()]
    }

    /// The fix, without any picker: one pass fetches every credential of
    /// every keyed provider, and the next plan has both fallbacks. A second
    /// pass finds nothing due and asks nobody.
    #[tokio::test]
    async fn a_pass_warms_every_keyed_credential_with_no_model_list_opened() {
        shared_endpoint();
        let anthropic = ModelsEndpoint::serving(&[MODEL]);
        let workspace = Workspace::new(
            &anthropic,
            &[
                ("ANTHROPIC_API_KEY", "sk-ant-pass-primary"),
                (
                    "ANTHROPIC_API_KEYS",
                    "sk-ant-pass-primary,sk-ant-pass-second",
                ),
                ("OPENCODE_API_KEY", "oz-pass-backup"),
            ],
        );
        let core = workspace.core();
        let primary = primary_leg(&core);
        let providers = vec!["anthropic".to_string(), "opencode-zen".to_string()];
        let now = Instant::now();
        note_turn(&core, now);

        core.reconcile_model_discovery_at(now, &providers).await;
        assert_eq!(anthropic.requests(), 2, "one listing per pooled credential");
        let plan = core.plan_route_ladder(primary.clone(), None);
        assert_eq!(plan.ladder.len(), 3, "{:?}", plan.ladder);
        assert_eq!(plan.ladder[0], primary);
        assert!(
            plan.ladder.iter().any(
                |leg| leg.provider == "anthropic" && leg.credential_id != primary.credential_id
            ),
            "the pool's second key serves as a fallback: {:?}",
            plan.ladder
        );
        assert!(
            plan.ladder.iter().any(|leg| leg.provider == "opencode-zen"),
            "{:?}",
            plan.ladder
        );
        assert!(
            !plan
                .annotations
                .iter()
                .any(|a| a.contains("single point of failure")),
            "{:?}",
            plan.annotations
        );

        core.reconcile_model_discovery_at(now, &providers).await;
        assert_eq!(anthropic.requests(), 2, "an unchanged world asks nobody");
    }

    #[tokio::test]
    async fn refreshes_ahead_of_the_ttl_and_keeps_the_last_catalogue_through_failures() {
        let anthropic = ModelsEndpoint::serving(&[MODEL]);
        let workspace = Workspace::new(
            &anthropic,
            &[
                ("ANTHROPIC_API_KEY", "sk-ant-refresh-primary"),
                ("ANTHROPIC_API_KEYS", "sk-ant-refresh-second"),
            ],
        );
        let core = workspace.core();
        let catalogues = core.model_catalogues();
        let backup = credential_keys(&core, "anthropic")[1].clone();
        let t0 = Instant::now();
        note_turn(&core, t0);

        core.reconcile_model_discovery_at(t0, &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 2);
        core.reconcile_model_discovery_at(t0 + Duration::from_secs(120), &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 2, "still fresh: nothing to do");

        // Renewed while still fresh, so neither a picker nor a plan finds a
        // lapsed catalogue.
        let renewed = t0 + REFRESH_AFTER;
        core.reconcile_model_discovery_at(renewed, &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 4);

        // The listing breaks. The pass records why and when it retries; the
        // catalogue planning uses is untouched.
        anthropic.answer(Reply::Status(503));
        let failed = renewed + REFRESH_AFTER;
        core.reconcile_model_discovery_at(failed, &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 6);
        let status = catalogues.status(&backup, failed);
        assert_eq!(status.models, Some(1), "{status:?}");
        assert!(
            status
                .failure
                .as_deref()
                .is_some_and(|reason| reason.contains("catalogue unavailable")),
            "{status:?}"
        );
        assert_eq!(status.retry_in, Some(RETRY_BASE));
        assert_eq!(catalogues.usable(failed).len(), 2);

        // Nothing is asked inside the backoff; the retry comes after it, and
        // a second failure waits twice as long.
        core.reconcile_model_discovery_at(failed + Duration::from_secs(5), &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 6);
        core.reconcile_model_discovery_at(failed + RETRY_BASE, &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 8);
        assert_eq!(
            catalogues.status(&backup, failed + RETRY_BASE).retry_in,
            Some(RETRY_BASE * 2)
        );
        let plan = core.plan_route_ladder(primary_leg(&core), None);
        assert_eq!(plan.ladder.len(), 2, "still planned: {:?}", plan.ladder);

        anthropic.answer(Reply::Catalogue(vec![MODEL.into(), "newer-model".into()]));
        let recovered = failed + RETRY_BASE * 3;
        core.reconcile_model_discovery_at(recovered, &anthropic_only())
            .await;
        let status = catalogues.status(&backup, recovered);
        assert_eq!((status.models, status.failure), (Some(2), None));
    }

    #[tokio::test]
    async fn a_failure_is_a_reason_with_a_retry_never_a_catalogue() {
        let anthropic = ModelsEndpoint::serving(&[]);
        anthropic.answer(Reply::Status(401));
        let workspace = Workspace::new(
            &anthropic,
            &[
                ("ANTHROPIC_API_KEY", "sk-ant-failing-primary"),
                ("ANTHROPIC_API_KEYS", "sk-ant-failing-second"),
            ],
        );
        let core = workspace.core();
        let catalogues = core.model_catalogues();
        let now = Instant::now();
        note_turn(&core, now);

        core.reconcile_model_discovery_at(now, &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 2);
        assert!(
            catalogues.usable(now).is_empty(),
            "a failure is not an empty catalogue"
        );
        for key in credential_keys(&core, "anthropic") {
            let status = catalogues.status(&key, now);
            assert_eq!(status.models, None, "{status:?}");
            assert!(status.failure.is_some(), "{status:?}");
            assert_eq!(status.retry_in, Some(RETRY_BASE));
        }
        let plan = core.plan_route_ladder(primary_leg(&core), None);
        assert_eq!(plan.ladder.len(), 1, "{:?}", plan.ladder);

        // A person asking is demand of its own: the picker asks straight
        // away, whatever the backoff, and reports the reason, not a list.
        assert!(core.discover_models("anthropic").await.is_err());
        assert_eq!(anthropic.requests(), 4);
    }

    #[tokio::test]
    async fn a_quiet_core_stops_asking_but_plans_from_its_last_catalogue() {
        let anthropic = ModelsEndpoint::serving(&[MODEL]);
        let workspace = Workspace::new(
            &anthropic,
            &[
                ("ANTHROPIC_API_KEY", "sk-ant-quiet-primary"),
                ("ANTHROPIC_API_KEYS", "sk-ant-quiet-second"),
            ],
        );
        let core = workspace.core();
        let catalogues = core.model_catalogues();
        let t0 = Instant::now();
        note_turn(&core, t0);
        core.reconcile_model_discovery_at(t0, &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 2);

        // No turn for the whole active window: the catalogues are due, and
        // nothing asks for them.
        let quiet = t0 + ACTIVE_WINDOW;
        core.reconcile_model_discovery_at(quiet, &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 2);
        assert_eq!(catalogues.usable(quiet).len(), 2, "planning still has them");
        assert!(
            catalogues.usable(t0 + PLAN_STALE_BOUND).is_empty(),
            "but not past the stale bound"
        );

        // The next turn is demand again, and wakes the loop for it.
        assert!(
            core.inner
                .discovery
                .note(core.agent_identity.clone(), quiet)
        );
        core.reconcile_model_discovery_at(quiet, &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 4);
    }

    /// The loop on its own: started once, it warms the Core with no pass
    /// called by hand, and dropping the Core ends it rather than the loop
    /// keeping the Core alive.
    #[tokio::test]
    async fn the_loop_warms_its_core_and_ends_with_it() {
        let anthropic = ModelsEndpoint::serving(&[MODEL]);
        let workspace = Workspace::new(
            &anthropic,
            &[
                ("ANTHROPIC_API_KEY", "sk-ant-loop-primary"),
                ("ANTHROPIC_API_KEYS", "sk-ant-loop-second"),
            ],
        );
        let core = workspace.core();
        assert!(!core.model_discovery_started());
        assert!(core.start_model_discovery());
        assert!(!core.start_model_discovery(), "idempotent");
        assert!(core.model_discovery_started());

        let keys = credential_keys(&core, "anthropic");
        let catalogues = core.model_catalogues();
        for _ in 0..1000 {
            if keys
                .iter()
                .all(|key| catalogues.fresh(key, Instant::now()).is_some())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let plan = core.plan_route_ladder(primary_leg(&core), None);
        assert_eq!(plan.ladder.len(), 2, "{:?}", plan.ladder);

        let weak = Arc::downgrade(&core.inner);
        drop(core);
        for _ in 0..2000 {
            if weak.upgrade().is_none() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the refresh loop kept its Core alive");
    }

    /// What the gateway's pool relies on: Cores for different workspaces
    /// share one store, a Core plans from another's discovery for a key it
    /// holds too, and never from a key it does not hold.
    #[tokio::test]
    async fn a_shared_store_serves_every_core_but_only_for_keys_it_holds() {
        let anthropic = ModelsEndpoint::serving(&[MODEL]);
        let keys = [
            ("ANTHROPIC_API_KEY", "sk-ant-shared-primary"),
            ("ANTHROPIC_API_KEYS", "sk-ant-shared-second"),
        ];
        let default_workspace = Workspace::new(&anthropic, &keys);
        let channel_workspace = Workspace::new(&anthropic, &keys);
        let other_workspace = Workspace::new(
            &anthropic,
            &[
                ("ANTHROPIC_API_KEY", "sk-ant-other-primary"),
                ("ANTHROPIC_API_KEYS", "sk-ant-other-second"),
            ],
        );
        let default = default_workspace.core();
        let channel = channel_workspace.core();
        let other = other_workspace.core();
        channel.share_model_catalogues(default.model_catalogues());
        other.share_model_catalogues(default.model_catalogues());
        assert!(
            channel
                .model_catalogues()
                .same_store(&default.model_catalogues())
        );

        let now = Instant::now();
        note_turn(&default, now);
        default
            .reconcile_model_discovery_at(now, &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 2);

        let plan = channel.plan_route_ladder(primary_leg(&channel), None);
        assert_eq!(plan.ladder.len(), 2, "{:?}", plan.ladder);
        let plan = other.plan_route_ladder(primary_leg(&other), None);
        assert_eq!(plan.ladder.len(), 1, "{:?}", plan.ladder);

        // The channel Core's own pass finds its credentials fresh.
        note_turn(&channel, now);
        channel
            .reconcile_model_discovery_at(now, &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 2);

        // A key change on one Core forgets that Core's credentials only.
        note_turn(&other, now);
        other
            .reconcile_model_discovery_at(now, &anthropic_only())
            .await;
        assert_eq!(anthropic.requests(), 4);
        other.invalidate_models_cache(Some("anthropic"));
        let usable: Vec<CatalogueKey> = default
            .model_catalogues()
            .usable(now)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(usable.len(), 2, "{usable:?}");
        assert!(
            credential_keys(&default, "anthropic")
                .iter()
                .all(|key| usable.contains(key))
        );
    }

    /// Agent-private secret scopes mean one Core can hold a different key per
    /// Agent: each Agent that planned a turn is refreshed with its own.
    #[tokio::test]
    async fn each_active_agent_is_refreshed_with_its_own_keys() {
        let anthropic = ModelsEndpoint::serving(&[MODEL]);
        let workspace = Workspace::new(
            &anthropic,
            &[("ANTHROPIC_API_KEY", "sk-ant-agent-workspace")],
        );
        let core = workspace.core();
        let specialist = core.clone().with_agent_identity(Some(agent("specialist")));
        let private = specialist.scope().env_file();
        workspace.store_secret(&private, "ANTHROPIC_API_KEY", "sk-ant-agent-private");
        workspace.store_secret(
            &private,
            "ANTHROPIC_API_KEYS",
            "sk-ant-agent-private-second",
        );

        let now = Instant::now();
        note_turn(&specialist, now);
        core.reconcile_model_discovery_at(now, &anthropic_only())
            .await;
        assert_eq!(
            anthropic.requests(),
            2,
            "the specialist's two keys, not the built-in Agent's, which planned nothing"
        );
        let plan = specialist.plan_route_ladder(primary_leg(&specialist), None);
        assert_eq!(plan.ladder.len(), 2, "{:?}", plan.ladder);
    }

    fn agent(id: &str) -> AgentIdentity {
        AgentIdentity {
            id: id.into(),
            revision: 1,
            name: id.into(),
            character: "vak".into(),
            personality: String::new(),
            animation: "subtle".into(),
            voice: "default".into(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
        }
    }

    #[test]
    fn retries_back_off_to_a_cap_and_never_stop() {
        let waits: Vec<u64> = (1..=8).map(|n| retry_delay(n).as_secs()).collect();
        assert_eq!(waits, vec![15, 30, 60, 120, 240, 300, 300, 300]);
        assert_eq!(retry_delay(u32::MAX), RETRY_CAP);
    }
}
