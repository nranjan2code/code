//! The one place the server touches the platform service manager.
//!
//! Two rules, both learned from defects this module exists to make
//! impossible:
//!
//! **1. Every call runs on a blocking worker.** `launchctl` and
//! `systemctl` are subprocesses; `vak_ops`'s health probe is
//! `reqwest::blocking`. Calling either straight from an async handler
//! stalls a tokio worker for as long as the service manager takes to
//! answer — which, for `launchctl bootstrap`, can be indefinitely
//! (AGENTS.md invariant 26). Handlers `await` these functions instead of
//! calling `vak_ops` directly, and nothing in `lib.rs` may reach past
//! this module to the service manager.
//!
//! **2. A record is not an activation.** Creating a bot, renaming it,
//! setting its token, or revoking it writes `bots.json` / `.env` and
//! nothing else. Registering an OS service is a separate, explicit act —
//! [`reconcile`] — for the same reason `vak self install` no longer starts
//! services (`docs/design/46-stabilization-install-and-onboarding.md` D6):
//! configuring something and activating it are different decisions, and
//! collapsing them means a routine edit quietly mutates the machine's
//! service manager. It also meant a test that created a bot wrote real
//! launchd plists pointing at the test binary.
//!
//! Revocation is deliberately **not** an exception to this. Making a
//! cleared token effective by bouncing a process would put orchestration
//! back in the request path — and it did: an API handler shelled out to
//! `launchctl`, which blocked. A bridge watches its own credential instead
//! (`surfaces::CredentialWatch`), so revoking one is a fact about `.env`
//! that takes effect within a poll cycle, with nothing orchestrating it.

use serde::Serialize;

/// Run a service-manager call on a blocking worker.
///
/// A panic inside the closure is reported as a failed operation rather
/// than taking down the handler: the service manager's opinion is never
/// worth a 500 on an unrelated request.
async fn blocking<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("service manager call did not complete: {e}"))
}

/// What reconciling produced, per unit.
#[derive(Debug, Clone, Serialize)]
pub struct UnitOutcome {
    pub name: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Bring the platform service manager in line with configuration:
/// the fixed services, plus one bridge unit per configured bot.
///
/// This is the **only** thing that registers, updates, or removes units,
/// and it is always called deliberately — by `vak setup`'s activation
/// step, by `vak self services-sync`, or by an operator pressing a button
/// in the admin console. Nothing reconciles as a side effect of an edit.
pub async fn reconcile(core: &vak_core::Core, port: u16) -> Result<Vec<UnitOutcome>, String> {
    // Units exec the running binary. In the gateway service that *is* the
    // installed binary; in a test or a dev build it is not, and writing
    // units that point at a test harness is how a test comes to mutate the
    // developer's machine. Refuse rather than guess.
    let bin_path =
        std::env::current_exe().map_err(|e| format!("cannot locate the running binary: {e}"))?;
    // A bridge unit authenticates against the gateway with this, so it has
    // to exist *before* the unit is registered — otherwise the unit is
    // written, starts, and crash-loops on "gateway token missing".
    vak_core::gateway_token::ensure_gateway_token()?;
    let data_home = core.sessions_home();
    let gateway_url = vak_ops::OpsConfig { port }.base_url();

    blocking(move || {
        let mut outcomes: Vec<UnitOutcome> = Vec::new();
        let names = vak_ops::services::default_service_names(&bin_path);
        for outcome in vak_ops::services::services_sync(
            &bin_path,
            &names,
            &vak_ops::services::Paths::default(),
            &vak_ops::services::SystemRunner,
        ) {
            outcomes.push(outcome.into());
        }
        for outcome in vak_ops::sync_bots(
            &bin_path,
            &data_home,
            &gateway_url,
            &vak_ops::services::Paths::default(),
            &vak_ops::services::SystemRunner,
        ) {
            outcomes.push(outcome.into());
        }
        outcomes
    })
    .await
}

impl From<vak_ops::services::SyncOutcome> for UnitOutcome {
    fn from(outcome: vak_ops::services::SyncOutcome) -> Self {
        match outcome.action {
            vak_ops::services::SyncAction::Failed(error) => UnitOutcome {
                name: outcome.name,
                action: "failed".into(),
                error: Some(error),
            },
            action => UnitOutcome {
                name: outcome.name,
                action: format!("{action:?}").to_lowercase(),
                error: None,
            },
        }
    }
}

/// How far configuration has run ahead of what is actually registered.
///
/// A bot that exists in `bots.json` with no unit is not broken — it is
/// **configured but not activated**, which is a state the operator chose
/// and must therefore be able to see. Before this was reported, a bot
/// created in the console looked identical to one that was live.
#[derive(Debug, Clone, Serialize)]
pub struct ActivationDrift {
    /// Bots configured in `bots.json`.
    pub configured_bots: usize,
    /// Bot bridge units the service manager actually has.
    pub registered_units: usize,
    /// Bot ids with no unit registered for them.
    pub awaiting_activation: Vec<String>,
}

/// Compare configured bots against registered units.
pub async fn activation_drift(core: &vak_core::Core) -> Result<ActivationDrift, String> {
    let data_home = core.sessions_home();
    blocking(move || {
        let configured = vak_ops::services::configured_bot_service_names_all(&data_home);
        let paths = vak_ops::services::Paths::default();
        let awaiting: Vec<String> = configured
            .iter()
            .filter(|name| !vak_ops::services::unit_is_registered(name, &paths))
            .cloned()
            .collect();
        ActivationDrift {
            configured_bots: configured.len(),
            registered_units: configured.len() - awaiting.len(),
            awaiting_activation: awaiting,
        }
    })
    .await
}

/// Start, stop, or restart a service, on a blocking worker.
pub async fn act(
    service: vak_ops::Service,
    action: Action,
    cfg: vak_ops::OpsConfig,
) -> Result<bool, String> {
    blocking(move || match action {
        Action::Start => vak_ops::start(service, &cfg),
        Action::Stop => vak_ops::stop(service, &cfg),
        Action::Restart => vak_ops::restart(service, &cfg),
    })
    .await
}

/// Probe a service's state, on a blocking worker.
pub async fn status(
    service: vak_ops::Service,
    cfg: vak_ops::OpsConfig,
) -> Result<vak_ops::State, String> {
    blocking(move || vak_ops::status(service, &cfg)).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Configured and registered are different counts, and the gap is the
    /// thing an operator needs to see.
    #[test]
    fn drift_separates_what_is_configured_from_what_is_registered() {
        let drift = ActivationDrift {
            configured_bots: 2,
            registered_units: 1,
            awaiting_activation: vec!["com.vak.discord-ops".into()],
        };
        assert_eq!(drift.configured_bots - drift.registered_units, 1);
        assert_eq!(drift.awaiting_activation, vec!["com.vak.discord-ops"]);
    }

    #[tokio::test]
    async fn a_blocking_call_returns_its_value_rather_than_stalling_the_runtime() {
        assert_eq!(blocking(|| 7).await.unwrap(), 7);
    }
}
