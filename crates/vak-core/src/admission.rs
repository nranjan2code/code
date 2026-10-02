//! Where a run is admitted and its trace key is minted
//! (docs/design/73-data-architecture-and-lifecycle.md §4, plan M1).
//!
//! The key is minted once per admitted request, from the surface that made
//! it, and then travels by value: into the agent loop, each tool call's
//! context, the broker worker, the sandbox records and the bus. Nothing
//! downstream re-derives it from ambient state.

use crate::{Core, Surface};
use vak_session::ids::PrincipalId;
use vak_session::trace::{Cause, TraceKey, local};

/// What a surface knows about who asked and why, stamped on a `Core` handle
/// before admission. Clone-local like the surface itself. Anything left
/// `None` is derived from the handle's surface.
#[derive(Debug, Clone, Default)]
pub struct RunAdmission {
    pub cause: Option<Cause>,
    pub actor: Option<PrincipalId>,
    pub on_behalf_of: Option<PrincipalId>,
    /// A key the surface already minted for this request (it announced the
    /// request on the bus under it). The run adopts it, as a child span, so
    /// the announcement and the work are one trace.
    pub trace: Option<TraceKey>,
}

impl RunAdmission {
    pub fn cause(mut self, cause: Cause) -> Self {
        self.cause = Some(cause);
        self
    }

    pub fn trace(mut self, trace: TraceKey) -> Self {
        self.trace = Some(trace);
        self
    }

    pub fn actor(mut self, actor: PrincipalId) -> Self {
        self.actor = Some(actor);
        self
    }

    pub fn on_behalf_of(mut self, principal: PrincipalId) -> Self {
        self.on_behalf_of = Some(principal);
        self
    }
}

impl Core {
    /// Stamp the cause and actor the next admissions on this handle carry.
    pub fn with_run_admission(mut self, admission: RunAdmission) -> Self {
        self.run_admission = admission;
        self
    }

    /// The actor for runs this handle admits, unless the host already
    /// stamped one.
    pub fn with_default_actor(mut self, actor: PrincipalId) -> Self {
        if self.run_admission.actor.is_none() {
            self.run_admission.actor = Some(actor);
        }
        self
    }

    /// The pre-minted key (or none) the next admission adopts. A handle that
    /// outlives its request must clear it.
    pub fn with_admitted_trace(mut self, trace: Option<TraceKey>) -> Self {
        self.run_admission.trace = trace;
        self
    }

    /// The key the surface minted for the request this handle serves, if it
    /// stamped one. Write sites that hold only a `Core` use it to record the
    /// run they belong to; it is `None` for work no run caused.
    pub fn admitted_trace(&self) -> Option<&TraceKey> {
        self.run_admission.trace.as_ref()
    }

    /// Mint the trace key for one admitted request: a fresh run, the
    /// cause its surface implies (or the one the host stamped), and the
    /// principal that acted. `request_id` is the client's idempotency key.
    pub fn mint_trace(&self, request_id: Option<&str>) -> TraceKey {
        if let Some(admitted) = &self.run_admission.trace {
            return admitted.child();
        }
        let agent_id = self
            .agent_identity()
            .map_or("vak", |agent| agent.id.as_str());
        let request = request_id.map_or_else(|| uuid::Uuid::now_v7().to_string(), str::to_string);
        let cause = self
            .run_admission
            .cause
            .clone()
            .unwrap_or_else(|| self.cause_for_surface(request));
        let agent_principal = local::agent_principal(agent_id);
        let (actor, on_behalf_of) = match (&self.run_admission.actor, &cause) {
            (Some(actor), _) => (*actor, self.run_admission.on_behalf_of),
            (
                None,
                Cause::Schedule { .. }
                | Cause::Heartbeat
                | Cause::Trigger { .. }
                | Cause::Delegation { .. },
            ) => (
                agent_principal,
                self.run_admission
                    .on_behalf_of
                    .or(Some(local::local_owner())),
            ),
            (None, Cause::System { .. }) => (local::system_principal(), None),
            (None, Cause::Channel { .. }) => (self.audience_principal(), None),
            (None, _) => (local::local_owner(), self.run_admission.on_behalf_of),
        };
        TraceKey::root(
            local::tenant(),
            local::space(self.cwd()),
            local::agent(agent_id),
            cause,
        )
        .acting(actor, on_behalf_of)
    }

    fn cause_for_surface(&self, request_id: String) -> Cause {
        match self.surface() {
            Surface::Chat { channel } => Cause::Channel {
                endpoint: self.channel_endpoint(channel),
                request_id,
            },
            Surface::Background => Cause::Heartbeat,
            Surface::Worker => Cause::System {
                job: "worker".into(),
            },
            Surface::Unknown
            | Surface::Cli
            | Surface::Terminal
            | Surface::Desktop
            | Surface::Server
            | Surface::Web => Cause::User { request_id },
        }
    }

    /// `surface:address[:bot]` for the conversation this handle serves.
    fn channel_endpoint(&self, channel: &str) -> String {
        match self.conversation_context().and_then(|c| c.origin.as_ref()) {
            Some(origin) => match &origin.bot_id {
                Some(bot) => format!("{}:{}:{bot}", origin.surface, origin.address),
                None => format!("{}:{}", origin.surface, origin.address),
            },
            None => channel.to_string(),
        }
    }

    /// The conversation's verified audience as a principal: the channel
    /// sender the gateway resolved, when it stamped one.
    fn audience_principal(&self) -> PrincipalId {
        match self.conversation_context() {
            Some(context) => {
                let (surface, chat) = context
                    .origin
                    .as_ref()
                    .map_or(("", ""), |o| (o.surface.as_str(), o.address.as_str()));
                local::channel_sender(surface, chat, &context.audience_id)
            }
            None => local::system_principal(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn local_surfaces_are_user_runs_by_the_local_owner() {
        let dir = tempfile::tempdir().unwrap();
        vak_config::paths::isolate_home_for_tests();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        let key = core.with_surface(Surface::Cli).mint_trace(Some("req-1"));
        assert_eq!(
            key.cause,
            Cause::User {
                request_id: "req-1".into()
            }
        );
        assert_eq!(key.actor, Some(local::local_owner()));
        assert_eq!(key.on_behalf_of, None);
    }
}
