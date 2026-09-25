import { createEffect, createSignal, Show } from "solid-js";
import { backend, setAgentCreateOpen, setTechnicalDetails, technicalDetails } from "../store";
import * as api from "../api";
import Icon from "./Icon";
import AgentMark from "./AgentMark";
import Sheet from "./Sheet";

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
 * with Vakyartha and never notice the sidebar's "+ Agent" entry.
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
      <Sheet
        size="narrow"
        title="Meet Vakyartha"
        onClose={dismiss}
        footer={<>
          <button type="button" class="btn" onClick={dismiss}>Start with Vakyartha</button>
          <button type="button" class="btn primary" onClick={() => { dismiss(); setAgentCreateOpen(true); }}>
            <Icon name="add" size={14} /> Create your first agent
          </button>
        </>}
      >
        <div class="onboarding-mark"><AgentMark character="vak" size={44} /></div>
        <p class="onboarding-copy">
          Vakyartha is ready to help out of the box. You can also build your own specialists — an agent with its own
          personality, instructions, and workspace — for the things you do often.
        </p>
        <label class="onboarding-technical"><input type="checkbox" checked={technicalDetails()} onChange={(event) => setTechnicalDetails(event.currentTarget.checked)} /> I build software: show technical details</label>
      </Sheet>
    </Show>
  );
}
