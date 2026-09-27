import { createResource, Show } from "solid-js";
import { host } from "../host";
import * as api from "../api";
import { openConnect, setupEpoch } from "../store";
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

/** Everyday words for a step, keyed by the step's id and never by its text.
 * Steps without an entry keep the server's own wording. */
const PLAIN_STEP: Partial<Record<(typeof SETUP_ORDER)[number], { what: string; repair: string; action: string }>> = {
  provider: {
    what: "Connect an AI service to start",
    repair: "Vakyartha needs a model to think with. It takes about a minute, and you can change it later.",
    action: "Connect",
  },
  route: {
    what: "Choose a model to start",
    repair: "Your AI service is connected. Pick the model Vakyartha should think with.",
    action: "Choose",
  },
};

/** The steps the in-app Connect sheet completes; the rest stay in the one
 * setup wizard (docs/design/46 D7). */
const IN_APP: ReadonlySet<string> = new Set(["provider", "route"]);

/**
 * Says when setup is incomplete. The AI-service steps open the in-app
 * Connect sheet, which makes the wizard's own calls (ConnectSheet.tsx);
 * every other step opens the one wizard (docs/design/46 D7), which the
 * desktop serves from the backend it embeds, as the browser does.
 *
 * Replaces `SetupCard`, which asked for a provider and a key and stopped
 * there, never discovering a model or freezing a route, and was a second
 * implementation of setup that disagreed with the wizard about "ready".
 */
export default function SetupBanner(props: { inGreeting?: boolean }) {
  // Derived on every read, like every other consumer of this projection:
  // a key removed elsewhere has to make setup incomplete again here too.
  const [state, { refetch }] = createResource(setupEpoch, () => api.onboarding());

  // The projection serializes one field per step; "which is owed" is
  // derived here rather than shipped, so there is no second list to keep
  // in step with the Rust one.
  const missing = () => {
    const s = state();
    if (!s || s.core_ready) return null;
    for (const key of SETUP_ORDER) {
      const step = s[key];
      if (step && step.state === "incomplete") {
        const plain = PLAIN_STEP[key];
        return { key, what: plain?.what ?? step.what, repair: plain?.repair ?? step.repair, action: plain?.action ?? "Finish setup" };
      }
    }
    return null;
  };

  return (
    <Show when={state.error} fallback={<Show when={missing()}>
      {(step) => (
        <div class="setup-banner" classList={{ "in-greeting": props.inGreeting }} role="status">
          <span class="setup-banner-mark"><Icon name="plug" size={20} /></span>
          <div class="setup-banner-text">
            <strong>{step().what}</strong>
            <span>{step().repair}</span>
          </div>
          <button
            class="btn primary"
            onClick={() => (IN_APP.has(step().key) ? openConnect() : host.openAdmin("#/setup"))}
          >
            {step().action}
          </button>
        </div>
      )}
    </Show>}>
      <div class="setup-banner" classList={{ "in-greeting": props.inGreeting }} role="alert">
        <span class="setup-banner-mark"><Icon name="warning" size={20} /></span>
        <div class="setup-banner-text">
          <strong>Setup status unavailable</strong>
          <span>{String(state.error)}</span>
        </div>
        <button class="btn primary" type="button" onClick={() => void refetch()}>Retry</button>
      </div>
    </Show>
  );
}
