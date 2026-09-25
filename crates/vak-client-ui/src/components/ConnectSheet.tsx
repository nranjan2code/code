import { createSignal, For, onMount, Show } from "solid-js";
import * as api from "../api";
import { trapFocus } from "../focusTrap";
import { loadHealth } from "../App";
import { activeAgentId, connectOpen, setConnectOpen, setNotice, setSetupEpoch } from "../store";
import type { ProviderInfo } from "../types";
import Icon from "./Icon";

/** Names people know a service by. An id without an entry shows as itself. */
const SERVICE_NAMES: Record<string, string> = {
  anthropic: "Anthropic",
  bedrock: "Amazon Bedrock",
  openai: "OpenAI",
  "opencode-zen": "OpenCode Zen",
  google: "Google Gemini",
  openrouter: "OpenRouter",
  ollama: "Ollama",
};

const serviceName = (id: string) => SERVICE_NAMES[id] ?? id;
const message = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * The in-app "Connect an AI service" sheet (docs/design/75 §6.2).
 *
 * It is the wizard's provider and model steps, not a second setup: the
 * same calls with the same scopes (the key in the Shared scope, the
 * provider and model in the active Agent's project layer), so identical
 * choices produce identical configuration (docs/design/46 D7), and
 * `GET /onboarding` stays the only judge of "ready".
 */
export default function ConnectSheet() {
  return <Show when={connectOpen()}><Sheet /></Show>;
}

function Sheet() {
  const [providers, setProviders] = createSignal<ProviderInfo[]>([]);
  const [local, setLocal] = createSignal<{ provider: string; models: string[] } | null>(null);
  const [localModel, setLocalModel] = createSignal("");
  const [looking, setLooking] = createSignal(true);
  const [account, setAccount] = createSignal("");
  const [key, setKey] = createSignal("");
  const [models, setModels] = createSignal<string[]>([]);
  const [model, setModel] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const close = () => setConnectOpen(false);
  // Several ids can share one key (another API flavour of the same
  // account); a person has the key, so each key is offered once.
  const accounts = () => {
    const keys = new Set<string>();
    return providers().filter((p) => {
      if (!p.requires_key) return false;
      if (!p.env_var) return true;
      if (keys.has(p.env_var)) return false;
      keys.add(p.env_var);
      return true;
    });
  };
  const chosen = () => accounts().find((p) => p.name === account());

  onMount(() => void (async () => {
    try {
      const list = (await api.listProviders()).providers;
      setProviders(list);
      // A model already running on this computer is the one-step path.
      for (const p of list.filter((item) => !item.requires_key)) {
        try {
          const found = (await api.discoverModels(p.name)).models;
          if (found.length) {
            setLocal({ provider: p.name, models: found });
            setLocalModel(found[0]);
            break;
          }
        } catch { /* nothing is serving models here */ }
      }
    } catch (e) {
      setError(message(e));
    } finally {
      setLooking(false);
    }
  })());

  const run = async (step: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try { await step(); } catch (e) { setError(message(e)); } finally { setBusy(false); }
  };

  const finish = async (provider: string, chosenModel: string) => {
    await api.patchConfig({ provider, model: chosenModel }, activeAgentId());
    setSetupEpoch((n) => n + 1);
    await loadHealth();
    close();
    setNotice({ kind: "info", text: `Connected to ${serviceName(provider)}. Ask anything.` });
    window.dispatchEvent(new CustomEvent("vak:focus-composer"));
  };

  const checkAccount = () => run(async () => {
    const name = account();
    if (key().trim()) {
      await api.putProviderKey(name, key().trim());
      setKey("");
    }
    // A stored key is not success: the service has to say what it can run.
    const found = (await api.discoverModels(name)).models;
    if (!found.length) throw new Error(`${serviceName(name)} offered no models to this key.`);
    setModels(found);
    setModel(found[0]);
  });

  return (
    <div class="modal-back" onClick={close}>
      <div
        class="modal connect-sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="connect-title"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => { if (e.key === "Escape") close(); }}
        use:trapFocus
      >
        <div class="modal-header">
          <div>
            <h2 id="connect-title" class="connect-title">Connect an AI service</h2>
            <p class="connect-sub">Vakyartha needs a model to think with. You can change it later in Settings.</p>
          </div>
          <button type="button" class="icon-button" aria-label="Close" onClick={close}><Icon name="close" size={16} /></button>
        </div>
        <Show when={!looking()} fallback={<p class="connect-note" role="status">Looking for a model on this computer…</p>}>
          <Show when={local()}>
            {(found) => (
              <section class="connect-option">
                <span class="connect-option-icon"><Icon name="lock" size={18} /></span>
                <div class="connect-option-text">
                  <strong>Use the model on this computer</strong>
                  <span>Private and free: what you ask stays on this machine.</span>
                  <Show when={found().models.length > 1} fallback={<span class="connect-model">{localModel()}</span>}>
                    <select aria-label="Model on this computer" value={localModel()} onChange={(e) => setLocalModel(e.currentTarget.value)}>
                      <For each={found().models}>{(m) => <option value={m}>{m}</option>}</For>
                    </select>
                  </Show>
                </div>
                <button type="button" class="btn primary" disabled={busy()} onClick={() => void run(() => finish(found().provider, localModel()))}>Use this model</button>
              </section>
            )}
          </Show>
          <section class="connect-option">
            <span class="connect-option-icon"><Icon name="plug" size={18} /></span>
            <div class="connect-option-text">
              <strong>{local() ? "Or use an account" : "Use an account"}</strong>
              <span>Paste the key from your AI service account. It is kept on this device.</span>
              <select aria-label="AI service" value={account()} onChange={(e) => { setAccount(e.currentTarget.value); setModels([]); setModel(""); setError(null); }}>
                <option value="">Choose a service…</option>
                <For each={accounts()}>{(p) => <option value={p.name}>{serviceName(p.name)}{p.configured ? " (key saved)" : ""}</option>}</For>
              </select>
              <Show when={account() && models().length === 0}>
                <div class="connect-row">
                  <input
                    type="password"
                    autocomplete="off"
                    aria-label="Key"
                    placeholder={chosen()?.configured ? "Saved. Paste a new key to replace it" : "Paste your key"}
                    value={key()}
                    onInput={(e) => setKey(e.currentTarget.value)}
                  />
                  <button type="button" class="btn primary" disabled={busy() || (!key().trim() && !chosen()?.configured)} onClick={() => void checkAccount()}>
                    {busy() ? "Checking…" : "Continue"}
                  </button>
                </div>
              </Show>
              <Show when={models().length > 0}>
                <div class="connect-row">
                  <select aria-label="Model" value={model()} onChange={(e) => setModel(e.currentTarget.value)}>
                    <For each={models()}>{(m) => <option value={m}>{m}</option>}</For>
                  </select>
                  <button type="button" class="btn primary" disabled={busy()} onClick={() => void run(() => finish(account(), model()))}>Use this model</button>
                </div>
              </Show>
            </div>
          </section>
          <Show when={error()}>{(text) => <p class="connect-error" role="alert">{text()}</p>}</Show>
        </Show>
      </div>
    </div>
  );
}
