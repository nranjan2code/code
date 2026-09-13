import { For, Show, createMemo, createSignal, createEffect, onCleanup } from "solid-js";
import * as api from "../api";
import { settingsScope, backend } from "../store";

export type AgentProfile = {
  id: string;
  revision: number;
  name: string;
  character: "orb" | "leaf" | "sun" | "wave" | "spark";
  personality: string;
  behaviour: string;
  responsibilities: string;
  animation: "subtle" | "expressive" | "off";
  voice: string;
};

const presets: Array<{ id: AgentProfile["character"]; label: string; glyph: string }> = [
  { id: "orb", label: "Soft orb", glyph: "◌" },
  { id: "leaf", label: "Quiet leaf", glyph: "◒" },
  { id: "sun", label: "Little sun", glyph: "☼" },
  { id: "wave", label: "Gentle wave", glyph: "〰" },
  { id: "spark", label: "Bright spark", glyph: "✦" },
];

function normalizeProfiles(profiles: AgentProfile[]): AgentProfile[] {
  return profiles.map((profile) => ({ ...profile, responsibilities: typeof profile.responsibilities === "string" ? profile.responsibilities : "" }));
}

function makeProfile(): AgentProfile {
  return { id: crypto.randomUUID(), revision: 1, name: "", character: "orb", personality: "Warm, practical, and easy to talk to.", behaviour: "Take initiative on clear requests and explain the next useful step.", responsibilities: "", animation: "subtle", voice: "default" };
}

function defaultChoices(profile: AgentProfile): AgentProfile {
  return { ...profile, character: "orb", personality: "Warm, practical, and easy to talk to.", behaviour: "Take initiative on clear requests and explain the next useful step.", responsibilities: "", animation: "subtle", voice: "default" };
}

export default function AgentProfilesPanel() {
  const [profiles, setProfiles] = createSignal<AgentProfile[]>([]);
  const [selected, setSelected] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal<AgentProfile | null>(null);
  const [saveError, setSaveError] = createSignal("");
  const current = createMemo(() => draft());
  const selectedProfile = createMemo(() => profiles().find((p) => p.id === selected()) ?? null);
  onCleanup(() => {
    if (typeof window !== "undefined" && "speechSynthesis" in window) window.speechSynthesis.cancel();
  });
  const [loading, setLoading] = createSignal(true);
  createEffect(() => {
    const scope = settingsScope();
    void backend().cwd;
    let disposed = false;
    setProfiles([]); setDraft(null); setSelected(null); setLoading(true); setSaveError("");
    void api.listAgentProfiles(scope).then((result) => {
      if (disposed) return;
      const normalized = normalizeProfiles(result.profiles);
      setProfiles(normalized);

    }).catch((error) => { if (!disposed) setSaveError(String(error)); }).finally(() => { if (!disposed) setLoading(false); });
    onCleanup(() => { disposed = true; });
  });
  const save = async () => {
    const value = current();
    if (!value || !value.name.trim()) return;
    setSaveError("");
    const next = profiles().some((p) => p.id === value.id)
      ? profiles().map((p) => p.id === value.id ? { ...value, name: value.name.trim() } : p)
      : [...profiles(), { ...value, name: value.name.trim() }];
    try {
      const saved = await api.saveAgentProfiles(next, settingsScope());
      const normalized = normalizeProfiles(saved.profiles);
      setProfiles(normalized);

    } catch (error) {
      setSaveError(error instanceof Error ? error.message : "Could not save this helper. Try again.");
      return;
    }
    setSelected(value.id);
    setDraft(null);
  };
  const remove = async (id: string) => {
    setSaveError("");
    try {
      const scheduled = (await api.listTasks()).tasks.filter((task) => task.agent_profile_id === id && task.enabled);
      if (scheduled.length > 0 && typeof window !== "undefined" && !window.confirm(`This helper is used by ${scheduled.length} scheduled task${scheduled.length === 1 ? "" : "s"}. Remove it and stop future runs?`)) return;
    } catch {
      // Profile removal remains available when the optional task view is offline.
    }
    const previous = profiles();
    const next = profiles().filter((p) => p.id !== id);
    try {
      const saved = await api.saveAgentProfiles(next, settingsScope());
      const normalized = normalizeProfiles(saved.profiles);
      setProfiles(normalized);

    } catch (error) {
      setProfiles(previous);
      setSaveError(error instanceof Error ? error.message : "Could not remove this helper. Try again.");
      return;
    }
    if (selected() === id) setSelected(null);
  };
  const update = <K extends keyof AgentProfile>(key: K, value: AgentProfile[K]) => {
    setDraft((value0) => value0 ? { ...value0, [key]: value } : value0);
  };
  const character = (id: AgentProfile["character"]) => presets.find((p) => p.id === id) ?? presets[0];

  return (
    <section class="agent-profiles">
      <Show when={loading()}><p role="status">Loading agents…</p></Show>
      <Show when={saveError()}><p role="alert">{saveError()}</p></Show>
      <div class="agent-profiles-heading">
        <div><h2>Your agents</h2><p>Create a named helper with its own look and manner. This is optional—Vak works beautifully on its own.</p></div>
        <button type="button" class="settings-button primary" disabled={loading() || !!saveError()} onClick={() => { const next = makeProfile(); setSelected(next.id); setDraft(next); }}>Create agent</button>
      </div>
      <Show when={profiles().length > 0} fallback={<div class="agent-profiles-empty"><span class="agent-empty-glyph">✦</span><div><strong>Make Vak feel like yours</strong><p>Name a helper, choose a character, and decide how it should work with you.</p></div></div>}>
        <div class="agent-profile-list"><For each={profiles()}>{(profile) => <button type="button" class="agent-profile-chip" title={`${profile.name} · saved revision ${profile.revision} · ${profile.id}`} classList={{ active: selected() === profile.id }} onClick={() => { setSelected(profile.id); setDraft(null); }}><span class={`agent-glyph ${profile.character}`}>{character(profile.character).glyph}</span><span>{profile.name}</span></button>}</For></div>
      </Show>
      <Show when={selectedProfile() && !draft()}>
        <div class="agent-profile-summary"><span class={`agent-avatar ${selectedProfile()!.character}`}>{character(selectedProfile()!.character).glyph}</span><div><strong>{selectedProfile()!.name}</strong><p>{selectedProfile()!.personality}</p><small>Saved revision {selectedProfile()!.revision} · open this agent from the sidebar. Existing chats retain their saved instructions.</small></div><button type="button" class="settings-button" onClick={() => setDraft({ ...selectedProfile()! })}>Edit</button><button type="button" class="settings-button" onClick={() => { const copy = { ...selectedProfile()!, id: crypto.randomUUID(), revision: 1, name: `${selectedProfile()!.name} copy` }; setSelected(copy.id); setDraft(copy); }}>Duplicate</button><button type="button" class="settings-button danger" onClick={() => remove(selectedProfile()!.id)}>Remove</button></div>
      </Show>
      <Show when={current()}>{(value) => <div class="agent-profile-editor">
        <div class="agent-preview"><span class={`agent-avatar ${value().character} ${value().animation}`}>{character(value().character).glyph}</span><div><strong>{value().name || "Your new agent"}</strong><span>{value().personality || "A personality that sounds like you want."}</span></div></div>
        <label>Name<input class="settings-input wide" value={value().name} placeholder="e.g. Pip, Mira, Atlas" onInput={(e) => update("name", e.currentTarget.value)} /></label>
        <div class="agent-character-picker"><span>Character</span><div><For each={presets}>{(preset) => <button type="button" class="agent-character-choice" classList={{ active: value().character === preset.id }} title={preset.label} onClick={() => update("character", preset.id)}><span class={`agent-glyph ${preset.id}`}>{preset.glyph}</span><small>{preset.label}</small></button>}</For></div></div>
        <label>Personality<textarea class="settings-textarea" rows={2} value={value().personality} placeholder="How should this agent sound?" onInput={(e) => update("personality", e.currentTarget.value)} /></label>
        <label>Working style<textarea class="settings-textarea" rows={2} value={value().behaviour} placeholder="How should it approach work?" onInput={(e) => update("behaviour", e.currentTarget.value)} /></label>
        <label>Useful for<textarea class="settings-textarea" rows={2} value={value().responsibilities} placeholder="What kinds of requests should this helper be useful for?" onInput={(e) => update("responsibilities", e.currentTarget.value)} /></label>
        <div class="agent-inline-fields"><label>Movement<select class="settings-input" value={value().animation} onChange={(e) => update("animation", e.currentTarget.value as AgentProfile["animation"])}><option value="subtle">Subtle</option><option value="expressive">Expressive</option><option value="off">Still</option></select></label><label>Voice<select class="settings-input" value={value().voice} onChange={(e) => update("voice", e.currentTarget.value)}><option value="default">Vak's voice</option><option value="calm">Calm</option><option value="bright">Bright</option><option value="quiet">Quiet</option></select></label><button type="button" class="settings-button" onClick={() => { if (typeof window === "undefined" || !("speechSynthesis" in window)) return; window.speechSynthesis.cancel(); const utterance = new SpeechSynthesisUtterance(`Hello, I'm ${value().name || "your helper"}. I'll keep things clear and useful.`); utterance.rate = value().voice === "quiet" ? 0.86 : value().voice === "bright" ? 1.08 : 1; window.speechSynthesis.speak(utterance); }}>Preview voice</button></div>
        <div class="agent-example-replies" aria-label="Personality examples"><span>How this helper might respond</span><div><p><strong>Greeting</strong> “What would you like a hand with?”</p><p><strong>Result</strong> “I found a clear next step and kept the details close by.”</p><p><strong>When unsure</strong> “I can continue once you choose between these two options.”</p></div></div>
        <p class="agent-profile-note">Personality, movement, and voice shape the experience. Permissions, privacy, and spending limits are always controlled separately.</p>
        <Show when={saveError()}><p class="agent-profile-error" role="alert">{saveError()} Your draft is still here.</p></Show>
        <div class="agent-editor-actions"><button type="button" class="settings-button" onClick={() => { setSaveError(""); setDraft(null); }}>Cancel</button><button type="button" class="settings-button" onClick={() => { setSaveError(""); setDraft(defaultChoices(value())); }}>Restore defaults</button><Show when={selectedProfile() && selectedProfile()!.id === value().id}><button type="button" class="settings-button" onClick={() => { setSaveError(""); setDraft({ ...selectedProfile()! }); }}>Reset changes</button></Show><button type="button" class="settings-button primary" disabled={!value().name.trim()} onClick={save}>Save agent</button></div>
      </div>}</Show>
    </section>
  );
}
