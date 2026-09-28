import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import * as api from "../api";
import { loadHealth } from "../App";
import { host } from "../host";
import { availableModels, initialModel } from "../modelChoices";
import { activeAgentId, connectOpen, connectScope, setConnectOpen, setNotice, setProviders, setSetupEpoch } from "../store";
import type { ProviderInfo } from "../types";
import Sheet from "./Sheet";

const message = (error: unknown) => error instanceof Error ? error.message : String(error);

export default function ConnectSheet() {
  return <Show when={connectOpen()}><ConnectForm /></Show>;
}

function ConnectForm() {
  // The destination cannot follow a background change of agent or settings scope.
  const agent = activeAgentId();
  const scope = connectScope();
  const [providers, setList] = createSignal<ProviderInfo[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [provider, setProvider] = createSignal("");
  const [key, setKey] = createSignal("");
  const [models, setModels] = createSignal<string[]>([]);
  const [model, setModel] = createSignal("");
  const [search, setSearch] = createSignal("");
  const [checked, setChecked] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [savedKey, setSavedKey] = createSignal(false);
  let current = { provider: "", model: "" };
  let alive = true;
  onCleanup(() => { alive = false; });
  const chosen = () => providers().find((p) => p.name === provider());
  const resetChoice = (name: string) => {
    setProvider(name); setKey(""); setSavedKey(false); setModels([]);
    setModel(""); setSearch(""); setChecked(false); setError(null);
  };
  // A provider that is already usable (has a key on file, or needs none) can
  // show its model list immediately: the extra "Check service" click only
  // earns its keep when a key still needs to be entered or verified.
  const selectProvider = (name: string) => {
    resetChoice(name);
    const p = providers().find((entry) => entry.name === name);
    if (p && (!p.requires_key || p.configured)) void check();
  };
  const load = async () => {
    setLoading(true); setError(null);
    try {
      const [list, config] = await Promise.all([api.listProviders(), scope === "user" ? api.getGlobalRoute() : api.getConfig(agent)]);
      if (!alive) return;
      setList(list.providers);
      current = { provider: config.provider || "", model: config.model || "" };
      selectProvider(list.providers.some((p) => p.name === current.provider) ? current.provider : "");
    } catch (e) { if (alive) setError(message(e)); }
    finally { if (alive) setLoading(false); }
  };
  onMount(() => void load());

  const check = async () => {
    if (busy() || !chosen()) return;
    const name = provider();
    setBusy(true); setError(null);
    try {
      if (key().trim()) {
        await api.putProviderKey(name, key().trim());
        if (!alive) return;
        setKey(""); setSavedKey(true);
        setList((list) => list.map((p) => p.name === name ? { ...p, configured: true } : p));
        setSetupEpoch((n) => n + 1);
      }
      const result = await api.discoverModels(name);
      if (!alive) return;
      const found = availableModels(result);
      if (!found.length) throw new Error(result.availability_error || (name === "bedrock"
        ? "No models have confirmed access. Check AWS access in the admin portal, then try again."
        : "No models are available from this service. Check your account or model server, then try again."));
      setModels(found);
      setModel(initialModel(found, current.provider === name ? current.model : ""));
      setChecked(true);
    } catch (e) { if (alive) setError(message(e)); }
    finally { if (alive) setBusy(false); }
  };

  const finish = async () => {
    if (busy() || !checked() || !models().includes(model())) return;
    const name = provider();
    const selected = model();
    setBusy(true); setError(null);
    try {
      if (scope === "user") await api.patchGlobalConfig({ provider: name, model: selected });
      else await api.patchConfig({ provider: name, model: selected }, agent);
    } catch (e) { if (alive) { setError(message(e)); setBusy(false); } return; }
    setSetupEpoch((n) => n + 1);
    setConnectOpen(false);
    setNotice({ kind: "info", text: `${api.providerLabel(providers(), name)} · ${selected} saved${scope === "user" ? " as the shared default" : " for this agent"}. Used from the next message; work already running keeps its current choice.` });
    void loadHealth();
    void api.listProviders().then(setProviders).catch(() => {});
  };

  const KeyFields = () => <div class="connect-form">
    <Show when={chosen()?.key_in_process || chosen()?.key_in_project}><p class="connect-note">A server or workspace key can take priority over a shared key saved here. Manage those keys in admin.</p></Show>
    <label class="connect-field"><span>Account key{chosen()?.configured ? " (optional)" : ""}</span>
      <input type="password" autocomplete="off" spellcheck={false} disabled={busy()} value={key()} placeholder={chosen()?.configured ? "Leave blank to keep the available key" : "Paste your API key"} onInput={(e) => setKey(e.currentTarget.value)} />
    </label>
    <p class="connect-note">Saved securely where Vakyartha runs. Replacing a shared key affects other agents using this account.</p>
    <details><summary>Where do I find a key?</summary><p class="connect-note">In your AI service’s API or developer settings. An API key lets Vakyartha use your account. A chat subscription may not include API usage; check the service’s billing settings.</p></details>
  </div>;

  return <Sheet class="connect-sheet" title="AI service and model" busy={busy()} onClose={() => setConnectOpen(false)}
    subtitle={scope === "user" ? "Shared default for agents that have not chosen their own service and model." : "Choose what this agent uses for its next message. Work already running keeps its current choice."}
    footer={<>
      <button type="button" class="settings-button" disabled={busy()} onClick={() => { setConnectOpen(false); void host.openAdmin("#/settings/models"); }}>Advanced settings in admin</button>
      <Show when={providers().length > 0}>
        <button type="button" class="btn primary" disabled={busy() || !provider() || (checked() ? !model() : !!chosen()?.requires_key && !chosen()?.configured && !key().trim())}
          onClick={() => void (checked() ? finish() : check())}>{busy() ? "Checking and saving…" : checked() ? "Save choice" : key().trim() ? "Save key and check" : "Check service"}</button>
      </Show>
    </>}>
    <Show when={!loading()} fallback={<p class="connect-note" role="status">Loading your AI services…</p>}>
      <Show when={providers().length > 0} fallback={<button class="btn" onClick={() => void load()}>Try again</button>}>
        <div class="connect-form">
          <Show when={current.provider && current.model}><p class="connect-note">Current choice: {api.providerLabel(providers(), current.provider)} · {current.model}</p></Show>
          <label class="connect-field"><span>AI service</span>
            <select value={provider()} disabled={busy()} onChange={(e) => selectProvider(e.currentTarget.value)}>
              <option value="">Choose a service…</option>
              <For each={providers()}>{(p) => <option value={p.name}>{p.label}{p.configured && p.requires_key ? " (key available)" : ""}</option>}</For>
            </select>
          </label>
          <Show when={chosen()?.requires_key && !checked()}>
            <p class="connect-note">Messages go to this service. Usage is billed to your AI service account.</p>
            <Show when={chosen()?.configured} fallback={<KeyFields />}>
              <details class="connect-key-details"><summary>Replace account key</summary><KeyFields /></details>
            </Show>
          </Show>
          <Show when={chosen() && !chosen()?.requires_key}>
            <p class="connect-note">Uses your configured model server without an account key. Where messages go depends on that server’s address; manage it in admin.</p>
          </Show>
          <Show when={savedKey()}><p class="connect-note" role="status">Account key saved. You can retry the check without pasting it again.</p></Show>
          <Show when={checked()}>
            <Show when={models().length > 12}><label class="connect-field"><span>Find a model</span><input type="search" value={search()} onInput={(e) => setSearch(e.currentTarget.value)} placeholder="Search available models" /></label></Show>
            <label class="connect-field"><span>Model</span>
              <select value={model()} disabled={busy()} onChange={(e) => setModel(e.currentTarget.value)}>
                <option value="">Choose a model…</option>
                <For each={models().filter((m) => m === model() || m.toLowerCase().includes(search().trim().toLowerCase()))}>{(m) => <option value={m}>{m}</option>}</For>
              </select>
            </label>
            <p class="connect-note">Models differ in capability, speed and price. This list comes from your service; it is not ranked. Checking the list does not test an answer.</p>
            <button type="button" class="settings-button" disabled={busy()} onClick={() => { setChecked(false); setError(null); }}>Recheck service or replace key</button>
          </Show>
        </div>
      </Show>
    </Show>
    <Show when={error()}>{(text) => <p class="connect-error" role="alert">Could not complete this step. {text()} Your model choice has not changed.</p>}</Show>
  </Sheet>;
}
