import { createResource, Show } from "solid-js";
import { host } from "../host";
import * as api from "../api";
import Icon from "./Icon";

/// Presentation order, matching `OnboardingState::steps` server-side.
const SETUP_ORDER = [
  "install",
  "workspace",
  "trust",
  "provider",
  "route",
  "permission",
  "sandbox",
  "capabilities",
  "dependencies",
  "integrations",
  "channels",
  "services",
  "first_result",
] as const;


/**
 * Says when setup is incomplete, and opens the wizard.
 *
 * Replaces `SetupCard`, which asked for a provider and a key and stopped
 * there — it never discovered a model, never froze a route, and never
 * mentioned posture, sandbox, skills, channels, or services. Worse, it was
 * a *second* implementation of setup: the web wizard and the desktop card
 * could disagree about what "ready" meant, and did.
 *
 * There is one wizard now (docs/design/46 D7). The desktop opens the same
 * page the browser does, served by the backend it already embeds, so both
 * surfaces render one state machine and produce identical configuration
 * from identical choices.
 */
export default function SetupBanner() {
  // Derived on every read, like every other consumer of this projection:
  // a key removed elsewhere has to make setup incomplete again here too.
  const [state, { refetch }] = createResource(() => api.onboarding());

  // The projection serializes one field per step; "which is owed" is
  // derived here rather than shipped, so there is no second list to keep
  // in step with the Rust one.
  const missing = () => {
    const s = state();
    if (!s || s.core_ready) return null;
    for (const key of SETUP_ORDER) {
      const step = s[key];
      if (step && step.state === "incomplete") return step;
    }
    return null;
  };

  return (
    <Show when={state.error} fallback={<Show when={missing()}>
      {(step) => (
        <div class="setup-banner" role="status">
          <span class="setup-banner-mark"><Icon name="spark" size={15} /></span>
          <div class="setup-banner-text">
            <strong>{step().what}</strong>
            <span class="dim">{step().repair}</span>
          </div>
          <button
            class="btn primary sm"
            onClick={() => host.openAdmin("#/setup")}
          >
            Finish setup
          </button>
        </div>
      )}
    </Show>}>
      <div class="setup-banner" role="alert">
        <span class="setup-banner-mark"><Icon name="spark" size={15} /></span>
        <div class="setup-banner-text">
          <strong>Setup status unavailable</strong>
          <span class="dim">{String(state.error)}</span>
        </div>
        <button class="btn primary sm" type="button" onClick={() => void refetch()}>Retry</button>
      </div>
    </Show>
  );
}
