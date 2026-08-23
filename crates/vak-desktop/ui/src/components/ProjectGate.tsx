import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { backend, providers } from "../store";
import { refreshBackend } from "../App";
import * as api from "../api";
import type { BackendInfo } from "../types";
import Icon from "./Icon";

/** Second gate step: pick provider + model and store a key (this machine only). */
function ConnectModel() {
  const [provider, setProvider] = createSignal(providers()?.current ?? "anthropic");
  const [model, setModel] = createSignal(providers()?.current_model ?? "");
  const [key, setKey] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const info = () => providers()?.providers.find((p) => p.name === provider());
  const needsKey = () => info()?.requires_key ?? true;

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      if (needsKey()) {
        if (!key().trim()) throw new Error(`enter the ${info()?.env_var} value`);
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
            onChange={(e) => {
              const next = e.currentTarget.value;
              setProvider(next);
              const first = providers()?.models[next]?.[0];
              if (first && !providers()?.models[next]?.includes(model())) setModel(first);
            }}
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
          <input
            list="gate-models"
            value={model()}
            placeholder="provider default"
            onInput={(e) => setModel(e.currentTarget.value)}
          />
          <datalist id="gate-models">
            <For each={providers()?.models[provider()] ?? []}>{(m) => <option value={m} />}</For>
          </datalist>
        </label>
        <Show when={needsKey()}>
          <label class="connect-field">
            <span>{info()?.env_var}</span>
            <input
              type="password"
              autocomplete="off"
              spellcheck={false}
              placeholder="paste API key"
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
  createEffect(() => {
    if (backend().ready) return;
    const t = setInterval(() => void refreshBackend(), 1500);
    onCleanup(() => clearInterval(t));
  });

  const readBootError = async () => {
    try {
      const info = await invoke<BackendInfo>("backend_info");
      if (!info.ready && info.boot_error) setError(info.boot_error);
    } catch {
      /* invoke failure already surfaced by refresh */
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

  return (
    <div class="gate">
      <div class="gate-card">
        <div class="gate-mark"><Icon name="spark" size={28} /></div>
        <Show
          when={!backend().ready || !providers()}
          fallback={<ConnectModel />}
        >
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
