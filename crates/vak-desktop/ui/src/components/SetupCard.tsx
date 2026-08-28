import { createSignal, For, Show } from "solid-js";
import { providers, setNotice, setProviders, setSetupNeeded, setupNeeded } from "../store";
import { loadHealth } from "../App";
import * as api from "../api";
import Icon from "./Icon";

/**
 * First-run setup (docs/design/29): a dismissible card — never a gate. The
 * workspace stays usable for everything that needs no model (transcripts,
 * settings); saving a key re-checks readiness and the card retires itself.
 */
export default function SetupCard() {  const [dismissed, setDismissed] = createSignal(false);
  const [provider, setProvider] = createSignal(providers()?.current ?? "anthropic");
  const [key, setKey] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const info = () => providers()?.providers.find((p) => p.name === provider());
  const configured = () => info()?.configured ?? false;
  const needsKey = () => (info()?.requires_key ?? true) && !configured();

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      if (needsKey() && !key().trim()) {
        throw new Error(`enter the ${info()?.env_var ?? "API key"} value`);
      }
      if (key().trim()) {
        await api.putProviderKey(provider(), key().trim());
      }
      // Selecting a provider here also makes it the active one.
      await api.patchConfig({ provider: provider() });
      // Re-check readiness; App un-blocks nothing (it never blocked), but
      // setupNeeded dropping to false retires this card.
      try {
        const p = await api.listProviders();
        setProviders(p);
        setSetupNeeded(!p.current_configured);
      } catch {
        /* keep the previous snapshot; error below explains */
      }
      await loadHealth();
      if (!providers()?.current_configured) {
        setError("saved — but the credential did not register; try again");
      } else {
        setNotice({ kind: "info", text: `${provider()} connected — you are ready to run tasks.` });
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Show when={setupNeeded() && !dismissed()}>
      <section class="setup-card" role="region" aria-label="Set up a model provider">
        <div class="setup-card-head">
          <span class="setup-card-mark"><Icon name="spark" size={15} /></span>
          <strong>Connect a model to start running tasks</strong>
          <button
            class="icon-button subtle"
            aria-label="Dismiss setup card"
            title="Dismiss — you can add a key later in Settings → Agent"
            onClick={() => setDismissed(true)}
          >
            <Icon name="close" size={13} />
          </button>
        </div>
        <div class="setup-card-form">
          <label class="setup-field">
            <span>Provider</span>
            <select value={provider()} onChange={(e) => { setProvider(e.currentTarget.value); setKey(""); }}>
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
          <Show when={info()?.requires_key ?? true}>
            <label class="setup-field grow">
              <span>{info()?.env_var ?? "API key"}</span>
              <input
                type="password"
                autocomplete="off"
                spellcheck={false}
                placeholder={configured() ? "already set on this device — leave blank to keep" : "paste key…"}
                aria-label={`${info()?.env_var ?? "API key"} for ${provider()}`}
                value={key()}
                onInput={(e) => setKey(e.currentTarget.value)}
                onKeyDown={(e) => e.key === "Enter" && void save()}
              />
            </label>
          </Show>
          <button class="btn primary sm" disabled={busy()} onClick={() => void save()}>
            {busy() ? "Saving…" : "Save"}
          </button>
        </div>
        <Show when={error()}>
          <div class="gate-err setup-card-error">{error()}</div>
        </Show>
        <p class="setup-card-note">Stored in ~/.vak/.env on this device only.</p>
      </section>
    </Show>
  );
}
