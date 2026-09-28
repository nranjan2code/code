import { createMemo, createResource, createSignal, For, Show } from "solid-js";
import { api, AuthRequired } from "./api";
import { providerLabel } from "./display";
import { pushToast, setAuthed } from "./store";
import type { ConfigScope, ModelRefView } from "./types";

const spell = (member: ModelRefView) => `${member.provider}/${member.model}`;

// The config spelling splits at the FIRST slash: model ids often contain
// one (`openrouter/anthropic/…`), provider names never do.
const parse = (spelling: string): ModelRefView | null => {
  const at = spelling.indexOf("/");
  return at > 0 && at < spelling.length - 1
    ? { provider: spelling.slice(0, at), model: spelling.slice(at + 1) }
    : null;
};

function MemberChip(props: { member: ModelRefView; chosen: boolean }) {
  return (
    <span class={`chip chip-phrase${props.chosen ? " chip-tone-success" : ""}`} title={spell(props.member)}>
      {providerLabel(props.member.provider)} · <span class="mono">{props.member.model}</span>
    </span>
  );
}

/// Backups for a model route (docs/design/15-reliability.md). Two lists: the
/// same model at other services, which a person confirms once for every
/// agent, and other models allowed for this scope. Nothing here changes which
/// model answers normally; a backup is tried only when the chosen model stops
/// responding.
export function BackupModels(props: { scope: ConfigScope; agent?: string; provider: string; model: string }) {
  // Which ids name one model is a fact about the services, not a preference,
  // so confirmed groups always live in the shared layer.
  const [shared, { refetch: refetchShared }] = createResource(() => api.configLayer("user"));
  // Other backup models follow the selected scope, like the route itself.
  const [layer, { refetch: refetchLayer }] = createResource(
    () => ({ scope: props.scope, agent: props.agent }),
    ({ scope, agent }) => api.configLayer(scope, agent),
  );
  const [suggested, setSuggested] = createSignal<ModelRefView[][] | null>(null);
  const [unticked, setUnticked] = createSignal<string[]>([]);
  const [unreadable, setUnreadable] = createSignal<string[]>([]);
  const [finding, setFinding] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [draft, setDraft] = createSignal("");

  // Each write replaces exactly the list the layer read returned (invariant 21).
  const sharedGroups = () => shared()?.route?.same_model ?? [];
  const groups = createMemo(() =>
    sharedGroups().map((group) => group.map(parse).filter((member): member is ModelRefView => member !== null)),
  );
  const backups = () => layer()?.route?.fallback_models ?? [];
  const inheritedBackups = () => (props.scope === "project" ? shared()?.route?.fallback_models ?? [] : []);
  const chosen = (member: ModelRefView) => member.provider === props.provider && member.model === props.model;
  const ticked = (group: ModelRefView[]) => group.map(spell).filter((spelling) => !unticked().includes(spelling));

  const write = async (scope: ConfigScope, patch: Record<string, unknown>, done: string) => {
    setBusy(true);
    try {
      await api.patchConfigScope(scope, patch, props.agent);
      await Promise.all([refetchShared(), refetchLayer()]);
      pushToast("info", done);
      return true;
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
      return false;
    } finally {
      setBusy(false);
    }
  };

  const find = async () => {
    setFinding(true);
    setSuggested(null);
    setUnreadable([]);
    setUnticked([]);
    try {
      const result = await api.sameModelSuggestions(spell({ provider: props.provider, model: props.model }), props.agent);
      setSuggested(result.groups);
      setUnreadable(result.errors.map((problem) => providerLabel(problem.provider)));
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setFinding(false);
    }
  };

  const confirm = async (group: ModelRefView[]) => {
    const members = ticked(group);
    const saved = await write("user", { route_same_model: [...sharedGroups(), members] }, `Saved: ${members.length} names for one model`);
    if (!saved) return;
    // A handled proposal leaves; with none left there is nothing to say.
    const rest = (suggested() ?? []).filter((candidate) => candidate !== group);
    setSuggested(rest.length > 0 ? rest : null);
  };

  const allow = async () => {
    const model = draft().trim();
    if (!model) return;
    if (await write(props.scope, { route_fallback_models: [...backups(), model] }, `${model} allowed as a backup`)) setDraft("");
  };

  return (
    <section class="panel" data-testid="backup-models">
      <div class="panel-title-row">
        <div>
          <h2>Backup models</h2>
          <p class="dim">Tried only when the chosen model stops responding. Nothing here changes which model answers normally.</p>
        </div>
      </div>

      <h3>Same model at other services</h3>
      <p class="dim">
        Each service names a model its own way, so Vakyartha never assumes two names are the same model. Confirm the ones
        that are; a confirmation applies to every agent.
      </p>
      <Show when={!shared.loading} fallback={<p class="dim">Loading…</p>}>
        <Show when={groups().length > 0} fallback={<p class="dim">None confirmed yet.</p>}>
          <div class="stack">
            <For each={groups()}>
              {(group, index) => (
                <div class="row-gap">
                  <div class="chip-stack">
                    <For each={group}>{(member) => <MemberChip member={member} chosen={chosen(member)} />}</For>
                  </div>
                  <button
                    class="ghost small"
                    disabled={busy()}
                    onClick={() => void write("user", { route_same_model: sharedGroups().filter((_, i) => i !== index()) }, "Removed")}
                  >
                    Remove
                  </button>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Show>
      <div class="row-gap">
        <button class="ghost small" disabled={finding() || busy() || !props.provider || !props.model} onClick={() => void find()}>
          {finding() ? "Checking your services…" : props.model ? `Find ${props.model} at other services` : "Choose a model first"}
        </button>
      </div>
      <Show when={suggested()}>
        {(found) => (
          <Show
            when={found().length > 0}
            fallback={<p class="dim">No new matches for {props.model} at the other services you have keys for.</p>}
          >
            <For each={found()}>
              {(group) => (
                <div class="stack">
                  <p class="dim">These look like the same model. Untick any that are not, then confirm.</p>
                  <div>
                    <For each={group}>
                      {(member) => (
                        <label class="inherit-toggle">
                          <input
                            type="checkbox"
                            checked={!unticked().includes(spell(member))}
                            onChange={(e) => {
                              const spelling = spell(member);
                              const keep = e.currentTarget.checked;
                              setUnticked((current) => (keep ? current.filter((s) => s !== spelling) : [...current, spelling]));
                            }}
                          />
                          <MemberChip member={member} chosen={chosen(member)} />
                        </label>
                      )}
                    </For>
                  </div>
                  <div class="row-gap">
                    <button disabled={busy() || ticked(group).length < 2} onClick={() => void confirm(group)}>
                      Confirm as the same model
                    </button>
                  </div>
                </div>
              )}
            </For>
          </Show>
        )}
      </Show>
      <Show when={unreadable().length > 0}>
        <p class="dim">Could not read the model list from {unreadable().join(", ")}.</p>
      </Show>

      <h3>Other backup models</h3>
      <p class="dim">
        {props.scope === "user" ? "Allowed for every agent." : "Allowed for this agent, on top of the shared defaults."} Tried after
        the same model at other services, and only where one of your keys reaches them.
      </p>
      <Show when={backups().length > 0} fallback={<p class="dim">None allowed here.</p>}>
        <div class="chip-stack">
          <For each={backups()}>
            {(model) => (
              <button
                class="chip chip-phrase"
                disabled={busy()}
                title={`Remove ${model}`}
                onClick={() => void write(props.scope, { route_fallback_models: backups().filter((m) => m !== model) }, `Removed ${model}`)}
              >
                <span class="mono">{model}</span> <span aria-hidden="true">×</span>
              </button>
            )}
          </For>
        </div>
      </Show>
      <Show when={inheritedBackups().length > 0}>
        <p class="dim">Also allowed in shared defaults: {inheritedBackups().join(", ")}.</p>
      </Show>
      <div class="form-row">
        <label for="backup-model-id">Model ID</label>
        <input
          id="backup-model-id"
          class="mono"
          placeholder="Exact ID, as the service lists it"
          value={draft()}
          onInput={(e) => setDraft(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void allow();
          }}
        />
        <button class="ghost small" disabled={busy() || !draft().trim()} onClick={() => void allow()}>
          Allow as backup
        </button>
      </div>
    </section>
  );
}
