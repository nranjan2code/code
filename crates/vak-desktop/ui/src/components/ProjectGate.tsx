import { createEffect, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { backend, providers, setupNeeded } from "../store";
import { refreshBackend } from "../App";
import * as api from "../api";
import type { BackendInfo } from "../types";
import Icon from "./Icon";

/** Second gate step: pick provider + model and store a key (this machine only). */
/** Sentinel option that swaps the model select for a free-text field. */
const CUSTOM_MODEL = "\u0000custom";

function ConnectModel() {
  const [provider, setProvider] = createSignal(providers()?.current ?? "anthropic");
  const [model, setModel] = createSignal(providers()?.current_model ?? "");
  const [key, setKey] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const info = () => providers()?.providers.find((p) => p.name === provider());
  // A provider whose key is already on this machine (env or ~/.vakcoder/.env)
  // must not be asked for it again; offer the field only as an optional
  // override so a configured provider can be selected and saved directly.
  const configured = () => info()?.configured ?? false;
  const needsKey = () => (info()?.requires_key ?? true) && !configured();

  // Model list comes off the provider's own API using the configured key,
  // so it reflects what this key can actually reach.
  const [catalog, setCatalog] = createSignal<string[]>([]);
  const [catalogNote, setCatalogNote] = createSignal<string | null>(null);
  const [customModel, setCustomModel] = createSignal(false);
  createEffect(() => {
    const name = provider();
    setCatalog([]);
    setCatalogNote("discovering models…");
    void (async () => {
      try {
        const r = await api.discoverModels(name);
        if (name !== provider()) return; // a newer selection won
        setCatalog(r.models);
        setCatalogNote(r.models.length ? null : "provider returned no models");
        if (r.models.length && !r.models.includes(model())) setModel(r.models[0]);
      } catch (e) {
        if (name !== provider()) return;
        setCatalogNote(e instanceof Error ? e.message : String(e));
      }
    })();
  });

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      if (needsKey() && !key().trim()) {
        throw new Error(`enter the ${info()?.env_var} value`);
      }
      // Send a key only when one was actually typed: an already-configured
      // provider saves fine without re-entering it.
      if (key().trim()) {
        await api.putProviderKey(provider(), key().trim());
      }
      await api.patchConfig({ provider: provider(), model: model().trim() || undefined });
      // Re-derive everything; when configured, App flips to the workspace.
      await refreshBackend();
      if (!providers()?.current_configured) {
        setError("saved — but the credential did not register; try again");
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="connect">
      <h1>Connect a model</h1>
      <p class="gate-lead">Choose a provider and add its key. Keys are stored on this device only.</p>
      <div class="connect-form">
        <label class="connect-field">
          <span>Provider</span>
          <select
            value={provider()}
            onChange={(e) => setProvider(e.currentTarget.value)}
          >
            <For each={providers()?.providers ?? []}>
              {(p) => (
                <option value={p.name}>
                  {p.name}
                  {p.configured ? " ✓" : ""}
                </option>
              )}
            </For>
          </select>
        </label>
        <label class="connect-field">
          <span>Model</span>
          <Show
            when={!customModel()}
            fallback={
              <input
                value={model()}
                placeholder="exact model id"
                onInput={(e) => setModel(e.currentTarget.value)}
              />
            }
          >
            <select
              value={model()}
              onChange={(e) => {
                const next = e.currentTarget.value;
                if (next === CUSTOM_MODEL) setCustomModel(true);
                else setModel(next);
              }}
            >
              <Show when={model() && !catalog().includes(model())}>
                <option value={model()}>{model()}</option>
              </Show>
              <For each={catalog()}>{(m) => <option value={m}>{m}</option>}</For>
              <option value={CUSTOM_MODEL}>Enter a model id…</option>
            </select>
          </Show>
          <Show when={catalogNote()}>
            <small class="connect-note">{catalogNote()}</small>
          </Show>
        </label>
        <Show when={info()?.requires_key ?? true}>
          <label class="connect-field">
            <span>
              {provider()} — {info()?.env_var}
              {configured() ? " (already set on this device)" : ""}
            </span>
            <input
              type="password"
              autocomplete="off"
              spellcheck={false}
              aria-label={`${info()?.env_var ?? "API key"} for ${provider()}`}
              placeholder={configured() ? "leave blank to keep current key" : `paste ${info()?.env_var ?? "API key"}`}
              value={key()}
              onInput={(e) => setKey(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && void save()}
            />
          </label>
        </Show>
        <Show when={error()}>
          <div class="gate-err">{error()}</div>
        </Show>
        <button class="btn primary lg" disabled={busy()} onClick={() => void save()}>
          {busy() ? "Connecting…" : "Save & continue"}
        </button>
        <p class="gate-note">Written to ~/.vakcoder/.env with owner-only permissions. Never leaves this machine.</p>
      </div>
    </div>
  );
}

export default function ProjectGate() {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  // Belt and braces: while the gate is up, poll the shell for backend state.
  // The backend-ready event normally flips this view instantly; the poll
  // guarantees the transition even if that event is missed.
  //
  // This deliberately lives in onMount, not createEffect: the poll writes
  // backend(), so a tracking scope that also reads it would tear down and
  // rebuild the timer on every tick instead of polling steadily.
  onMount(() => {
    let stop = false;
    const tick = async () => {
      if (stop || backend().ready) return;
      await refreshBackend();
    };
    const t = setInterval(() => void tick(), 800);
    void tick();
    onCleanup(() => {
      stop = true;
      clearInterval(t);
    });
  });

  const readBootError = async () => {
    try {
      const info = await invoke<BackendInfo>("backend_info");
      if (!info.ready && info.boot_error) setError(info.boot_error);
    } catch (e) {
      console.error("readBootError: backend_info failed", e);
    }
  };

  const pick = async () => {
    setBusy(true);
    setError(null);
    try {
      const dir = await open({ directory: true, multiple: false, title: "Open a project" });
      if (typeof dir === "string") {
        await invoke("start_backend", { cwd: dir });
        // Flip proactively; do not trust the event alone.
        const ready = await refreshBackend();
        if (!ready) await readBootError();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      await readBootError();
    } finally {
      setBusy(false);
    }
  };

  // Show the picker while no project has been opened yet (providers()
  // hasn't loaded). Once a project's backend is up, fall through to
  // ConnectModel whenever it isn't configured yet.
  const showPicker = () => !providers();

  return (
    <div class="gate">
      <div class="gate-card">
        <div class="gate-mark"><Icon name="spark" size={28} /></div>
        <Show when={showPicker()} fallback={<ConnectModel />}>
          <h1>vakcoder</h1>
          <p class="gate-lead">Your code, your machine, your agent.</p>
          <div class="gate-features">
            <span><Icon name="check" size={15} /> Isolated tasks and worktrees</span>
            <span><Icon name="check" size={15} /> Review every change before keeping it</span>
            <span><Icon name="check" size={15} /> Local-first and fully inspectable</span>
          </div>
          <Show when={error() || backend().boot_error}>
            <div class="gate-err">{error() ?? backend().boot_error}</div>
          </Show>
          <button class="btn primary lg" onClick={() => void pick()} disabled={busy()}>
            <Icon name="folder" /> {busy() ? "Opening workspace…" : "Open a project"}
          </button>
          <p class="gate-note">Configuration and secrets remain on this device.</p>
        </Show>
      </div>
    </div>
  );
}
