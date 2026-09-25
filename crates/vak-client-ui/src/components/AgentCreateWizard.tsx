import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { agentCreateOpen, setAgentCreateOpen, technicalDetails } from "../store";
import { openAgentChat } from "../App";
import * as api from "../api";
import { AGENT_CHARACTERS, AGENT_CHARACTER_IDS, type AgentCharacter } from "../agentGlyph";
import { playCharacterCue } from "../characterSound";
import AgentMark from "./AgentMark";
import Sheet from "./Sheet";

type Character = AgentCharacter;

/** The mascot is the product's own character (docs/design/71); agents a
 * person creates choose among the companions. */
const COMPANIONS = AGENT_CHARACTER_IDS.filter((id) => id !== "vak");

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
  // Every agent in reach, not just the Shared layer this wizard writes:
  // the new agent's default character is one none of them wears yet.
  const [everyAgent, setEveryAgent] = createSignal<api.Agent[]>([]);
  const [chosen, setChosen] = createSignal<api.AgentTemplate>(SCRATCH);
  const [error, setError] = createSignal("");
  const [creating, setCreating] = createSignal(false);

  const [id, setId] = createSignal("");
  const [idTouched, setIdTouched] = createSignal(false);
  const [name, setName] = createSignal("");
  const [character, setCharacter] = createSignal<Character>(COMPANIONS[0]);
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
    setCharacter(COMPANIONS[0]);
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
      void api.listAgents().then((r) => setEveryAgent(r.agents)).catch(() => setEveryAgent([]));
    }
  });

  // A template's own character when it is free, otherwise the first
  // companion no agent wears; every character is taken only past seven.
  const freeCharacter = (preferred: string): Character => {
    const taken = new Set<string>(everyAgent().map((agent) => agent.character));
    if ((COMPANIONS as string[]).includes(preferred) && !taken.has(preferred)) return preferred as Character;
    return COMPANIONS.find((id) => !taken.has(id)) ?? ((COMPANIONS as string[]).includes(preferred) ? preferred as Character : COMPANIONS[0]);
  };

  const applyTemplate = (tmpl: api.AgentTemplate) => {
    setChosen(tmpl);
    setName(tmpl.name);
    setIdTouched(false);
    setCharacter(freeCharacter(tmpl.character));
    setPersonality(tmpl.personality);
    setInstructions(tmpl.instructions);
    setStep(2);
  };

  const idInUse = createMemo(() => id() === "vak" || everyAgent().some((a) => a.id === id()) || agents().some((a) => a.id === id()));

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
      <Sheet
        class="agent-create"
        title="New agent"
        subtitle={`Step ${step()} of 3: ${step() === 1 ? "choose a starting point" : step() === 2 ? "name it and pick a character" : "say how it should work"}`}
        onClose={close}
        busy={creating()}
        footer={step() === 1 ? undefined : <>
          <button type="button" class="btn" disabled={creating()} onClick={() => setStep(step() === 3 ? 2 : 1)}>Back</button>
          <Show when={step() === 2} fallback={<button type="button" class="btn primary" disabled={creating()} onClick={() => void create()}>{creating() ? "Creating…" : "Create agent"}</button>}>
            <button type="button" class="btn primary" disabled={!canAdvanceStep2()} onClick={() => setStep(3)}>Next</button>
          </Show>
        </>}
      >
        <Show when={error()}><p class="worker-error" role="alert">{error()}</p></Show>

        <Show when={step() === 1}>
          <div class="agent-create-starts">
            <For each={templates()}>
              {(tmpl) => (
                <button type="button" class="agent-create-start" onClick={() => applyTemplate(tmpl)}>
                  <span class="agent-create-start-name"><AgentMark character={freeCharacter(tmpl.character)} size={28} /> {tmpl.name}</span>
                  <span class="agent-create-start-text">{tmpl.description}</span>
                </button>
              )}
            </For>
            <button type="button" class="agent-create-start blank" onClick={() => applyTemplate(SCRATCH)}>
              <span class="agent-create-start-name">Start from scratch</span>
              <span class="agent-create-start-text">A blank agent you shape yourself.</span>
            </button>
          </div>
        </Show>

        <Show when={step() === 2}>
          <div class="agent-create-form">
            <label class="agent-identity-field"><span>Name</span>
              <input type="text" required autofocus placeholder="e.g. Trip Planner" value={name()} onInput={(e) => setName(e.currentTarget.value)} />
            </label>
            <Show when={technicalDetails()}>
              <label class="agent-identity-field"><span>Agent ID</span>
                <input type="text" class="mono" value={id()} onInput={(e) => { setIdTouched(true); setId(slugify(e.currentTarget.value)); }} />
              </label>
            </Show>
            <Show when={idInUse()}>
              <p class="agent-create-error" role="alert">{technicalDetails() ? "That ID is already taken. Pick another." : "You already have an agent with this name. Pick another name."}</p>
            </Show>
            <fieldset class="agent-identity-characters">
              <legend>Character</legend>
              <div class="agent-companion-picker">
                <For each={COMPANIONS}>
                  {(c) => (
                    <button
                      type="button"
                      class="agent-companion-choice"
                      classList={{ active: character() === c }}
                      aria-label={`Choose ${AGENT_CHARACTERS[c].name}, ${AGENT_CHARACTERS[c].kind}`}
                      aria-pressed={character() === c}
                      onClick={() => { setCharacter(c); playCharacterCue(c); }}
                    >
                      <AgentMark character={c} size={56} state={character() === c ? "listening" : "idle"} interactive />
                      <strong>{AGENT_CHARACTERS[c].name}</strong>
                      <small>{AGENT_CHARACTERS[c].kind}</small>
                    </button>
                  )}
                </For>
              </div>
            </fieldset>
            <label class="agent-identity-field"><span>Personality</span>
              <input type="text" placeholder="e.g. Friendly, careful and to the point." value={personality()} onInput={(e) => setPersonality(e.currentTarget.value)} />
            </label>
          </div>
        </Show>

        <Show when={step() === 3}>
          <div class="agent-create-form">
            <label class="agent-identity-field"><span>How it should work</span>
              <textarea rows={5} placeholder="What it helps with, what it should always or never do, and how it should sound." value={instructions()} onInput={(e) => setInstructions(e.currentTarget.value)} />
            </label>
            <p class="agent-create-hint">You can change this later in the agent's settings.</p>
            <div class="agent-create-preview">
              <AgentMark character={character()} size={40} />
              <div><strong>{name() || "Unnamed agent"}</strong><span>{personality() || AGENT_CHARACTERS[character()].personality}</span></div>
            </div>
          </div>
        </Show>
      </Sheet>
    </Show>
  );
}
