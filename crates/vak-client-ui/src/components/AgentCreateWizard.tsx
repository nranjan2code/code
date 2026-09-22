import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { trapFocus } from "../focusTrap";
import { agentCreateOpen, setAgentCreateOpen } from "../store";
import { openAgentChat } from "../App";
import * as api from "../api";
import { AGENT_CHARACTERS, AGENT_CHARACTER_IDS, type AgentCharacter } from "../agentGlyph";
import { playCharacterCue } from "../characterSound";
import AgentMark from "./AgentMark";
import Icon from "./Icon";

type Character = AgentCharacter;

const SCRATCH: api.AgentTemplate = {
  template_id: "__scratch__",
  domain: "custom",
  name: "",
  description: "Start with a blank agent and define everything yourself.",
  character: "mira",
  personality: "",
  behaviour: "Deliver high-quality outcomes with clear reasoning.",
  responsibilities: "",
  instructions: "",
  animation: "subtle",
  voice: "default",
};

export default function AgentCreateWizard() {
  const [step, setStep] = createSignal<1 | 2 | 3>(1);
  const [templates, setTemplates] = createSignal<api.AgentTemplate[]>([]);
  const [agents, setAgents] = createSignal<api.Agent[]>([]);
  const [chosen, setChosen] = createSignal<api.AgentTemplate>(SCRATCH);
  const [error, setError] = createSignal("");
  const [creating, setCreating] = createSignal(false);

  const [id, setId] = createSignal("");
  const [idTouched, setIdTouched] = createSignal(false);
  const [name, setName] = createSignal("");
  const [character, setCharacter] = createSignal<Character>("mira");
  const [personality, setPersonality] = createSignal("");
  const [instructions, setInstructions] = createSignal("");

  const slugify = (s: string) => s.trim().toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "");

  createEffect(() => {
    if (!idTouched()) setId(slugify(name()));
  });

  const reset = () => {
    setStep(1);
    setChosen(SCRATCH);
    setId("");
    setIdTouched(false);
    setName("");
    setCharacter("mira");
    setPersonality("");
    setInstructions("");
    setError("");
  };

  createEffect(() => {
    if (agentCreateOpen()) {
      reset();
      void api.listAgentTemplates().then((r) => setTemplates(r.templates)).catch(() => setTemplates([]));
      // This wizard writes the Shared layer, so seed the replacement from
      // that exact layer rather than copying workspace overrides into it.
      void api.listAgents("user").then((r) => setAgents(r.agents)).catch(() => setAgents([]));
    }
  });

  const applyTemplate = (tmpl: api.AgentTemplate) => {
    setChosen(tmpl);
    setName(tmpl.name);
    setIdTouched(false);
    setCharacter(tmpl.character);
    setPersonality(tmpl.personality);
    setInstructions(tmpl.instructions);
    setStep(2);
  };

  const idInUse = createMemo(() => id() === "vak" || agents().some((a) => a.id === id()));

  const canAdvanceStep2 = createMemo(() => name().trim().length > 0 && id().trim().length > 0 && !idInUse());

  const close = () => {
    if (creating()) return;
    setAgentCreateOpen(false);
  };

  const create = async () => {
    if (!canAdvanceStep2()) return;
    setCreating(true);
    setError("");
    try {
      const tmpl = chosen();
      const agent: api.Agent = {
        id: id(),
        revision: 1,
        lifecycle: "active",
        name: name().trim(),
        character: character(),
        personality: personality().trim(),
        behaviour: tmpl.behaviour || "Deliver high-quality outcomes with clear reasoning.",
        responsibilities: tmpl.responsibilities || "",
        instructions: instructions().trim(),
        animation: "subtle",
        voice: "default",
      };
      await api.saveAgents([...agents(), agent], "user");
      setAgentCreateOpen(false);
      await openAgentChat(agent.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to create agent.");
    } finally {
      setCreating(false);
    }
  };

  return (
    <Show when={agentCreateOpen()}>
      <div class="modal-back" onClick={close}>
        <div
          class="modal"
          style="max-width: 560px; width: 92vw;"
          role="dialog"
          aria-modal="true"
          aria-labelledby="agent-create-title"
          onClick={(e) => e.stopPropagation()}
          use:trapFocus
        >
          <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 12px;">
            <div>
              <h3 id="agent-create-title" style="margin: 0; font-size: 16px; font-weight: 600;">
                New Agent
              </h3>
              <span style="color: var(--muted); font-size: 12px;">
                Step {step()} of 3 — {step() === 1 ? "Choose a starting point" : step() === 2 ? "Name it" : "Give it instructions"}
              </span>
            </div>
            <button type="button" class="icon-button subtle" aria-label="Close" onClick={close}>
              <Icon name="close" size={14} />
            </button>
          </div>

          <Show when={error()}><p class="worker-error" role="alert" style="margin-bottom: 10px;">{error()}</p></Show>

          {/* STEP 1: TEMPLATE */}
          <Show when={step() === 1}>
            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 8px;">
              <For each={templates()}>
                {(tmpl) => (
                  <button
                    type="button"
                    class="button subtle"
                    style="text-align: left; padding: 10px 12px; height: auto; display: flex; flex-direction: column; gap: 4px; align-items: flex-start;"
                    onClick={() => applyTemplate(tmpl)}
                  >
                    <span style="display: inline-flex; align-items: center; gap: 7px;"><AgentMark character={tmpl.character} size={23} /> {tmpl.name}</span>
                    <span style="font-size: 11.5px; color: var(--muted); font-weight: 400;">{tmpl.description}</span>
                  </button>
                )}
              </For>
              <button
                type="button"
                class="button subtle"
                style="text-align: left; padding: 10px 12px; height: auto; display: flex; flex-direction: column; gap: 4px; align-items: flex-start; border-style: dashed;"
                onClick={() => applyTemplate(SCRATCH)}
              >
                <span style="font-size: 15px;">+ Start from scratch</span>
                <span style="font-size: 11.5px; color: var(--muted); font-weight: 400;">{SCRATCH.description}</span>
              </button>
            </div>
          </Show>

          {/* STEP 2: IDENTITY */}
          <Show when={step() === 2}>
            <div style="display: flex; flex-direction: column; gap: 12px;">
              <div>
                <label style="display: block; font-size: 12px; font-weight: 500; margin-bottom: 4px;">Name</label>
                <input
                  type="text"
                  required
                  autofocus
                  placeholder="e.g. Security Reviewer"
                  value={name()}
                  onInput={(e) => setName(e.currentTarget.value)}
                  style="width: 100%; padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--border); background: var(--surface); color: var(--text);"
                />
              </div>

              <div>
                <label style="display: block; font-size: 12px; font-weight: 500; margin-bottom: 4px;">Agent ID</label>
                <input
                  type="text"
                  value={id()}
                  onInput={(e) => { setIdTouched(true); setId(slugify(e.currentTarget.value)); }}
                  style="width: 100%; padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--border); background: var(--surface); color: var(--text); font-family: var(--mono); font-size: 12.5px;"
                />
                <Show when={idInUse()}>
                  <p style="margin: 4px 0 0; font-size: 11px; color: var(--red);">That ID is already taken — pick another.</p>
                </Show>
              </div>

              <div>
                <label style="display: block; font-size: 12px; font-weight: 500; margin-bottom: 4px;">Character</label>
                <div class="agent-companion-picker">
                  <For each={AGENT_CHARACTER_IDS}>
                    {(c) => (
                      <button
                        type="button"
                        class="agent-companion-choice"
                        classList={{ active: character() === c }}
                        aria-label={`Choose ${AGENT_CHARACTERS[c].name}, ${AGENT_CHARACTERS[c].kind}`}
                        onClick={() => { setCharacter(c); playCharacterCue(c); }}
                      >
                        <AgentMark character={c} size={56} state={character() === c ? "listening" : "idle"} interactive />
                        <strong>{AGENT_CHARACTERS[c].name}</strong>
                        <small>{AGENT_CHARACTERS[c].kind}</small>
                      </button>
                    )}
                  </For>
                </div>
                <p class="agent-companion-note">Choose a companion to preview its movement and sound. Sounds follow your app setting.</p>
              </div>

              <div>
                <label style="display: block; font-size: 12px; font-weight: 500; margin-bottom: 4px;">Personality & Demeanor</label>
                <input
                  type="text"
                  placeholder="e.g. Rigorous, cautious, and detail-obsessed."
                  value={personality()}
                  onInput={(e) => setPersonality(e.currentTarget.value)}
                  style="width: 100%; padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--border); background: var(--surface); color: var(--text);"
                />
              </div>

              <div style="display: flex; justify-content: space-between; margin-top: 4px;">
                <button type="button" class="button subtle" onClick={() => setStep(1)}>Back</button>
                <button type="button" class="button primary" disabled={!canAdvanceStep2()} onClick={() => setStep(3)}>Next</button>
              </div>
            </div>
          </Show>

          {/* STEP 3: INSTRUCTIONS + PREVIEW */}
          <Show when={step() === 3}>
            <div style="display: flex; flex-direction: column; gap: 12px;">
              <div>
                <label style="display: block; font-size: 12px; font-weight: 500; margin-bottom: 4px;">Core Instructions / System Prompt</label>
                <textarea
                  rows={4}
                  placeholder="Instructions that govern this agent's reasoning, tool use, and tone."
                  value={instructions()}
                  onInput={(e) => setInstructions(e.currentTarget.value)}
                  style="width: 100%; padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--border); background: var(--surface); color: var(--text); font-family: var(--sans); font-size: 12.5px; resize: vertical;"
                />
                <p style="margin: 4px 0 0; font-size: 11px; color: var(--muted);">This becomes the agent's system prompt — you can refine it later from its settings.</p>
              </div>

              <div style="padding: 10px 12px; background: var(--surface-raised); border: 1px solid var(--border); border-radius: var(--radius-sm);">
                <span style="font-size: 11px; color: var(--muted); text-transform: uppercase; letter-spacing: 0.05em; display: block; margin-bottom: 4px;">Preview</span>
                <div style="display: flex; align-items: center; gap: 8px;">
                  <AgentMark character={character()} size={28} />
                  <div>
                    <div style="font-weight: 600; font-size: 13.5px;">{name() || "Unnamed agent"}</div>
                    <div style="font-size: 11.5px; color: var(--muted);">{personality() || "No personality set yet"}</div>
                  </div>
                </div>
              </div>

              <div style="display: flex; justify-content: space-between; margin-top: 4px;">
                <button type="button" class="button subtle" onClick={() => setStep(2)}>Back</button>
                <button type="button" class="button primary" disabled={creating()} onClick={() => void create()}>
                  {creating() ? "Creating…" : "Create & Launch"}
                </button>
              </div>
            </div>
          </Show>
        </div>
      </div>
    </Show>
  );
}
