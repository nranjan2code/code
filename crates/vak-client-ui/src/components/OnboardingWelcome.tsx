import { createEffect, createSignal, Show } from "solid-js";
import { backend, setAgentCreateOpen } from "../store";
import * as api from "../api";
import Icon from "./Icon";
import AgentMark from "./AgentMark";

const SEEN_KEY = "vak.onboarded";

function markSeen() {
  try { localStorage.setItem(SEEN_KEY, "1"); } catch { /* ignore */ }
}

function alreadySeen(): boolean {
  try { return localStorage.getItem(SEEN_KEY) === "1"; } catch { return false; }
}

/**
 * A one-time welcome for a fresh install: nothing steers a first-time user
 * toward making an agent otherwise, so they'd land straight on an empty chat
 * with Vak and never notice the sidebar's "+ Agent" entry.
 */
export default function OnboardingWelcome() {
  const [dismissed, setDismissed] = createSignal(alreadySeen());
  const [checked, setChecked] = createSignal(false);
  const [hasCustomAgents, setHasCustomAgents] = createSignal(true);

  createEffect(() => {
    if (!backend().ready || dismissed() || checked()) return;
    setChecked(true);
    void api.listAgents().then((r) => setHasCustomAgents(r.agents.length > 0)).catch(() => setHasCustomAgents(true));
  });

  const visible = () => backend().ready && !dismissed() && checked() && !hasCustomAgents();

  const dismiss = () => { markSeen(); setDismissed(true); };

  return (
    <Show when={visible()}>
      <div class="modal-back" onClick={dismiss}>
        <div class="modal" style="max-width: 420px;" role="dialog" aria-modal="true" aria-labelledby="onboarding-title" onClick={(e) => e.stopPropagation()}>
          <div style="margin-bottom: 12px;"><AgentMark character="spark" size={44} /></div>
          <h2 id="onboarding-title" style="margin: 0 0 6px;">Meet Vak</h2>
          <p style="margin: 0 0 18px; color: var(--muted); font-size: 13.5px; line-height: 1.5;">
            Vak is ready to help out of the box. You can also build your own specialists — an agent with its own
            personality, instructions, and workspace — for the things you do often.
          </p>
          <div style="display: flex; gap: 8px;">
            <button type="button" class="button primary" onClick={() => { dismiss(); setAgentCreateOpen(true); }}>
              <Icon name="add" size={14} /> Create your first agent
            </button>
            <button type="button" class="button subtle" onClick={dismiss}>Start with Vak</button>
          </div>
        </div>
      </div>
    </Show>
  );
}
