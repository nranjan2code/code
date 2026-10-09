/// Operate › Data (plan M7a-c, docs/design/74 §6.2 A7 and A8): what
/// Vakyartha keeps, measured, and what its retention would remove now.
/// Retention only observes at this stage: the plan is the one it would
/// run, shown and not carried out, and a kind of data nothing watches yet
/// is named as such rather than counted as nothing due.

import { createResource, createSignal, For, Show } from "solid-js";
import { api } from "./api";
import type { DataIntegrity, DataPlan, DataStatus, DataTransition, DataUsage, ErasureReceipt, KeyStatus } from "./types";

function size(bytes: number): string {
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${bytes} B` : `${value.toFixed(1)} ${units[unit]}`;
}

const words = (name: string) => name.replaceAll("_", " ");
const day = (iso: string) => new Date(iso).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });

const DOES: Record<string, string> = { remove: "Remove", trash: "Move to trash" };
const WHY: Record<string, string> = { age: "Past its keep time", size: "Over its size limit", count: "More than a conversation keeps" };
const KEPT: Record<string, string> = { held: "On hold", live: "In use", kept: "Kept by a person" };
const KEEP_DAYS = (secs?: number) => (secs ? `${Math.round(secs / 86400)} days` : "");

/// The install's keep times, which the owner edits (plan M7b-a). There is
/// one set for the whole install. A shorter time says what it would
/// remove and asks before it is saved.
function Rules(props: { status?: DataStatus; onSaved: () => void }) {
  const [rules, { refetch }] = createResource(() => api.dataRules());
  const [draft, setDraft] = createSignal<Record<string, number>>({});
  const [busy, setBusy] = createSignal(false);
  const [said, setSaid] = createSignal("");
  const daysOf = (secs?: number) => (secs ? Math.round(secs / 86400) : 0);
  const shown = (rule: { class: string; delete_after_secs?: number }) => draft()[rule.class] ?? daysOf(rule.delete_after_secs);
  const defaultDays = (kind: string) => daysOf(rules()?.defaults.rules.find((rule) => rule.class === kind)?.delete_after_secs);
  // What is sent: every keep time that differs from its default.
  const wanted = () => {
    const out: Record<string, number> = {};
    for (const rule of rules()?.label.rules ?? []) {
      if (!rule.delete_after_secs) continue;
      const days = shown(rule);
      if (days !== defaultDays(rule.class)) out[rule.class] = days;
    }
    return out;
  };
  const changed = () => Object.keys(draft()).length > 0;
  const save = async (keepDays: Record<string, number>) => {
    setBusy(true);
    setSaid("");
    try {
      const { preview } = await api.previewDataRules(keepDays);
      let digest: string | undefined;
      if (preview.shortened.length > 0) {
        const loses = preview.newly_due.length === 0
          ? "Nothing is past the new times yet."
          : `The next pass would remove ${preview.newly_due.map((impact) => `${impact.items} of ${words(impact.class)} (${size(impact.bytes)})`).join(", ")} that is kept now.`;
        if (!window.confirm(`Shorter keep times for: ${preview.shortened.map(words).join(", ")}. ${loses} This cannot be undone once the pass has run. Save them?`)) {
          setBusy(false);
          return;
        }
        digest = preview.digest;
      }
      await api.setDataRules(keepDays, digest);
      setDraft({});
      setSaid("Keep times saved.");
      void refetch();
      props.onSaved();
    } catch (error) {
      setSaid(`Could not save the keep times: ${error instanceof Error ? error.message : error}`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>How long each kind is kept</h2>
          <p class="dim">One set of keep times for everything on this machine. A shorter time removes things sooner, so it says what it would remove and asks first.</p>
        </div>
        <div class="lifecycle-actions">
          <button class="ghost small" disabled={busy() || rules()?.label.id === "default"} onClick={() => void save({})}>Back to the defaults</button>
          <button class="ghost small" disabled={busy() || !changed()} onClick={() => void save(wanted())}>{busy() ? "Saving…" : "Save keep times"}</button>
        </div>
      </div>
      <Show when={said()}><p role="status">{said()}</p></Show>
      <Show when={rules()} fallback={<p class="dim">{rules.error ? `Could not read the keep times: ${rules.error}` : "Reading…"}</p>}>
        {(read) => (
          <div class="ops-table-wrap">
            <table class="ops-table">
              <thead><tr><th>Kind</th><th>Kept for (days)</th><th>Default</th><th>Limit</th><th>Then</th><th>Watched</th></tr></thead>
              <tbody>
                <For each={read().label.rules}>
                  {(rule) => (
                    <tr>
                      <td>{words(rule.class)}</td>
                      <td>
                        <Show when={rule.delete_after_secs} fallback="">
                          <input
                            class="rules-days"
                            type="number"
                            min="1"
                            max={read().max_days}
                            aria-label={`Days ${words(rule.class)} is kept`}
                            value={shown(rule)}
                            onInput={(event) => {
                              const days = Math.round(Number(event.currentTarget.value));
                              if (Number.isFinite(days) && days >= 1) setDraft({ ...draft(), [rule.class]: days });
                            }}
                          />
                        </Show>
                      </td>
                      <td>{KEEP_DAYS(read().defaults.rules.find((other) => other.class === rule.class)?.delete_after_secs)}</td>
                      <td>{rule.max_bytes ? size(rule.max_bytes) : rule.keep_newest ? `first and newest ${rule.keep_newest}` : ""}</td>
                      <td>{DOES[rule.on_expiry] ?? rule.on_expiry}</td>
                      <td>{props.status ? (props.status.observed.includes(rule.class) ? "Yes" : "Not yet") : ""}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        )}
      </Show>
      <p class="dim">Conversations and their files have no keep time: they stay until a person removes them.</p>
    </section>
  );
}

// Erasing everything Vakyartha stored (plan M7b-f): the counts first, then
// the words typed. The server checks both again, and stops once it has
// answered, so the receipt is shown here and offered as a file.
function EraseEverything() {
  const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const [busy, setBusy] = createSignal(false);
  const [said, setSaid] = createSignal("");
  const [receipt, setReceipt] = createSignal<ErasureReceipt | null>(null);
  const erase = async () => {
    if (busy()) return;
    setBusy(true);
    setSaid("");
    try {
      const { preview, confirm } = await api.installErasurePreview();
      if (preview.held > 0) {
        setSaid(`${preview.held} ${preview.held === 1 ? "thing is" : "things are"} on hold. Release every hold first.`);
        return;
      }
      const typed = window.prompt(
        `Erase everything Vakyartha has stored on this machine? ${count(preview.conversations, "conversation", "conversations")}, ${count(preview.artifacts, "file", "files")} in the Library, every Agent, automation, connected account and saved key. Your own folders are not touched. This cannot be undone. Type “${confirm}” to go on.`,
      );
      if (typed === null) return;
      if (typed.trim().toLowerCase() !== confirm) {
        setSaid("Not erased: the words did not match.");
        return;
      }
      setReceipt((await api.eraseInstall(preview.digest, typed.trim())).receipt);
    } catch (error) {
      setSaid(`${error instanceof Error ? error.message : error}`);
    } finally {
      setBusy(false);
    }
  };
  const file = () => `data:application/json;charset=utf-8,${encodeURIComponent(JSON.stringify(receipt(), null, 2))}`;
  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>Erase everything</h2>
          <p class="dim">Removes all that Vakyartha has stored on this machine: conversations, files, memory, Agents, automations, connected accounts and saved keys. Your own folders are never touched. It leaves one signed receipt.</p>
        </div>
        <Show when={!receipt()}>
          <div class="lifecycle-actions">
            <button class="ghost small danger" disabled={busy()} onClick={() => void erase()}>{busy() ? "Working…" : "Erase everything…"}</button>
          </div>
        </Show>
      </div>
      <Show when={said()}><p role="status">{said()}</p></Show>
      <Show when={receipt()}>
        {(done) => (
          <div role="status">
            <p>Everything was erased, and Vakyartha has stopped. Start it again to set it up as new.</p>
            <p class="dim">{count(done().conversations, "conversation", "conversations")} and {count(done().keys_destroyed, "key", "keys")} were destroyed.</p>
            <p><a class="ghost small" href={file()} download={`vakyartha-erasure-${done().id}.json`}>Save the receipt</a></p>
            <ul class="dim">
              <For each={done().not_reached}>{(line) => <li>Not reached: {line}</li>}</For>
            </ul>
          </div>
        )}
      </Show>
    </section>
  );
}

function Retention() {
  const [status, { refetch: refetchStatus }] = createResource<DataStatus>(() => api.dataStatus());
  const [plan, { refetch }] = createResource<DataPlan>(() => api.dataPlan());
  const [made, { refetch: reread }] = createResource<{ transitions: DataTransition[] }>(() => api.dataTransitions());
  const [running, setRunning] = createSignal(false);
  const [ran, setRan] = createSignal("");
  const acting = () => status()?.mode === "commit";
  const runNow = async () => {
    setRunning(true);
    setRan("");
    try {
      const tick = await api.dataTick();
      setRan(
        tick.mode === "commit"
          ? `Removed ${tick.committed.length} ${tick.committed.length === 1 ? "item" : "items"}, ${size(tick.reclaimed_bytes)}.${tick.failed.length ? ` ${tick.failed.length} could not be removed.` : ""}`
          : "Looked again. Nothing was removed.",
      );
      void refetch();
      void reread();
    } catch (error) {
      setRan(`Could not run retention: ${error}`);
    } finally {
      setRunning(false);
    }
  };
  const done = () => (made()?.transitions ?? []).filter((row) => row.state !== "started");
  return (
    <>
      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>What retention would do now</h2>
            <Show
              when={acting()}
              fallback={<p class="dim">Retention is observing only: this is the plan it would run. Nothing here has been removed.</p>}
            >
              <p class="dim">
                Retention removes what is due for: {(status()?.committed ?? []).map(words).join(", ")}. It runs every ten minutes; other kinds are shown and left alone.
              </p>
            </Show>
          </div>
          <button class="ghost small" disabled={plan.loading || running()} onClick={() => void runNow()}>
            {acting() ? "Run now" : "Look again"}
          </button>
        </div>
        <Show when={ran()}><p role="status">{ran()}</p></Show>
        <Show when={!plan.error} fallback={<p class="dim">Could not read the plan: {`${plan.error}`}</p>}>
          <Show when={plan()} fallback={<p class="dim">Reading the data home…</p>}>
            {(planned) => (
              <>
                <p>
                  <strong>{planned().actions.length}</strong> {planned().actions.length === 1 ? "item is" : "items are"} due, {size(planned().reclaimable_bytes)} in all.
                  {" "}<strong>{planned().guarded.length}</strong> past their time are kept back.
                </p>
                <Show when={planned().actions.length > 0}>
                  <div class="ops-table-wrap">
                    <table class="ops-table">
                      <thead><tr><th>Would</th><th>Kind</th><th>Why</th><th>Due since</th><th>Size</th><th>Item</th></tr></thead>
                      <tbody>
                        <For each={planned().actions}>
                          {(action) => (
                            <tr>
                              <td>{DOES[action.does] ?? action.does}</td>
                              <td>{words(action.class)}</td>
                              <td>{WHY[action.reason] ?? action.reason}</td>
                              <td>{day(action.due)}</td>
                              <td>{size(action.bytes)}</td>
                              <td class="mono">{action.item}</td>
                            </tr>
                          )}
                        </For>
                      </tbody>
                    </table>
                  </div>
                </Show>
                <Show when={planned().guarded.length > 0}>
                  <h3>Kept back</h3>
                  <div class="ops-table-wrap">
                    <table class="ops-table">
                      <thead><tr><th>Kind</th><th>Why it stays</th><th>Item</th></tr></thead>
                      <tbody>
                        <For each={planned().guarded}>
                          {(kept) => (
                            <tr><td>{words(kept.class)}</td><td>{KEPT[kept.guard] ?? kept.guard}</td><td class="mono">{kept.item}</td></tr>
                          )}
                        </For>
                      </tbody>
                    </table>
                  </div>
                </Show>
                <Show when={planned().unobserved.length > 0}>
                  <p class="dim">Not watched yet, so nothing is planned for them: {planned().unobserved.map(words).join(", ")}.</p>
                </Show>
              </>
            )}
          </Show>
        </Show>
      </section>
      <Show when={done().length > 0}>
        <section class="panel">
          <h2>What retention has removed</h2>
          <div class="ops-table-wrap">
            <table class="ops-table">
              <thead><tr><th>When</th><th>Result</th><th>Kind</th><th>Why</th><th>Size</th><th>Item</th></tr></thead>
              <tbody>
                <For each={done()}>
                  {(row) => (
                    <tr>
                      <td>{new Date(row.at).toLocaleString()}</td>
                      <td>{row.state === "committed" ? "Removed" : `Could not remove${row.error_kind ? ` (${row.error_kind})` : ""}`}</td>
                      <td>{words(row.class)}</td>
                      <td>{WHY[row.reason] ?? row.reason}</td>
                      <td>{size(row.bytes)}</td>
                      <td class="mono">{row.item}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </section>
      </Show>
      <Rules status={status()} onSaved={() => { void refetchStatus(); void refetch(); }} />
      <EraseEverything />
    </>
  );
}

function Limit() {
  const [status] = createResource<DataStatus>(() => api.dataStatus());
  return (
    <Show when={status()?.quota}>
      {(quota) => (
        <p role={quota().state === "hard" ? "alert" : undefined}>
          <Show when={quota().limit_bytes} fallback={<span class="dim">No storage limit is set.</span>}>
            {(limit) => (
              <>
                <strong>{size(quota().kept_bytes)}</strong> kept of a {size(limit())} limit.
                <Show when={quota().state === "hard"}> Storage is full: new work is refused until space is freed or the limit is raised. Nothing is removed to make room.</Show>
                <Show when={quota().state === "soft"}> Storage is nearly full.</Show>
              </>
            )}
          </Show>
        </p>
      )}
    </Show>
  );
}

function Storage() {
  const [usage, { refetch }] = createResource<DataUsage>(() => api.dataUsage());
  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>What is stored</h2>
          <p class="dim">Measured from the files on this machine when the page loaded. Each file is counted once.</p>
        </div>
        <button class="ghost small" disabled={usage.loading} onClick={() => void refetch()}>Measure again</button>
      </div>
      <Show when={!usage.error} fallback={<p class="dim">Could not measure storage: {`${usage.error}`}</p>}>
        <Show when={usage()} fallback={<p class="dim">Measuring…</p>}>
          {(read) => (
            <>
              <p><strong>{size(read().bytes)}</strong> in {read().files} files.</p>
              <Limit />
              <div class="ops-table-wrap">
                <table class="ops-table">
                  <thead><tr><th>Where</th><th>Kind</th><th>Part of Vakyartha</th><th>Files</th><th>Size</th></tr></thead>
                  <tbody>
                    <For each={[...read().rows].sort((a, b) => b.bytes - a.bytes)}>
                      {(row) => (
                        <tr><td>{row.root}</td><td>{row.class}</td><td>{row.owner}</td><td>{row.files}</td><td>{size(row.bytes)}</td></tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </>
          )}
        </Show>
      </Show>
    </section>
  );
}

const SEARCH: Record<string, string> = {
  current: "Up to date with the records.",
  behind: "Behind the records. Rebuild search brings it up to date.",
  unreadable: "Could not be read. Rebuild search makes it again from the records.",
};

/// Operate › Data › Integrity (docs/design/74 §6.2 A9). Checking reads
/// every stored record once, so it runs when asked, never on a timer.
function Integrity() {
  const [report, { refetch }] = createResource<DataIntegrity>(() => api.dataIntegrity());
  const [rebuilding, setRebuilding] = createSignal(false);
  const [said, setSaid] = createSignal("");
  const rebuild = async () => {
    setRebuilding(true);
    setSaid("");
    try {
      await api.rebuild();
      setSaid("Search was rebuilt from the records.");
      void refetch();
    } catch (error) {
      setSaid(`Could not rebuild search: ${error}`);
    } finally {
      setRebuilding(false);
    }
  };
  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>Integrity</h2>
          <p class="dim">Every conversation and record is checked against a running fingerprint of what was written, without reading what it says. A changed, missing or reordered record is found and named.</p>
        </div>
        <div class="lifecycle-actions">
          <button class="ghost small" disabled={report.loading} onClick={() => void refetch()}>{report.loading ? "Checking…" : "Check again"}</button>
          <button class="ghost small" disabled={rebuilding()} onClick={() => void rebuild()}>{rebuilding() ? "Rebuilding…" : "Rebuild search"}</button>
        </div>
      </div>
      <Show when={said()}><p role="status">{said()}</p></Show>
      <Show when={!report.error} fallback={<p class="dim">Could not check the data home: {`${report.error}`}</p>}>
        <Show when={report()} fallback={<p class="dim">Checking every stored record…</p>}>
          {(found) => (
            <>
              <p>
                <strong>{found().broken.length === 0 && found().receipts_unverified === 0 ? "Nothing is damaged." : "Something is damaged."}</strong>{" "}
                {found().conversations} conversations and {found().chains} other record logs were checked: {found().records} records, as of {day(found().at)}.
              </p>
              <Show when={found().broken.length > 0}>
                <div class="ops-table-wrap">
                  <table class="ops-table">
                    <thead><tr><th>Damaged</th><th>Part</th></tr></thead>
                    <tbody>
                      <For each={found().broken}>{(broken) => <tr><td class="mono">{broken.at}</td><td>{broken.segment}</td></tr>}</For>
                    </tbody>
                  </table>
                </div>
              </Show>
              <dl class="lifecycle-facts">
                <dt>Keys</dt>
                <dd>{found().keys} in use, {found().keys_destroyed} destroyed by an erasure, {found().keys_held} on hold.</dd>
                <dt>Erasure receipts</dt>
                <dd>{found().receipts} kept{found().receipts_unverified > 0 ? `; ${found().receipts_unverified} with a signature that does not verify` : found().receipts > 0 ? ", every signature verifies" : ""}.</dd>
                <dt>Search</dt>
                <dd>{SEARCH[found().search] ?? found().search}</dd>
                <Show when={found().torn_tails > 0}>
                  <dt>Unfinished writes</dt>
                  <dd>{found().torn_tails}, left by a stop mid-write. The next write trims them; nothing is lost.</dd>
                </Show>
                <Show when={found().fenced}>
                  <dt>Restored</dt>
                  <dd>This data home was restored from a backup. Start Vakyartha again before it writes.</dd>
                </Show>
              </dl>
            </>
          )}
        </Show>
      </Show>
    </section>
  );
}

/// Operate › Data › Keys (docs/design/74 §6.2 A12, plan M7b-g): where the
/// keys that protect everything are kept, and rotation. No key is shown.
function Keys() {
  const [status, { refetch }] = createResource<KeyStatus>(() => api.dataKeys());
  const [busy, setBusy] = createSignal(false);
  const [said, setSaid] = createSignal("");
  const rotate = async () => {
    if (!window.confirm("Start a new key and protect everything again under it? Nothing is lost, and everything stays readable. Earlier keys are kept so that older backups still open.")) return;
    setBusy(true);
    setSaid("");
    try {
      const { rotation } = await api.rotateDataKeys();
      setSaid(`Done. Key ${rotation.version + 1} is in use; ${rotation.rewrapped} stored keys were protected again under it.`);
      void refetch();
    } catch (error) {
      setSaid(`${error instanceof Error ? error.message : error}`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>Keys</h2>
          <p class="dim">Everything Vakyartha stores is encrypted. Each conversation and file has a key of its own, and one main key protects those. Rotating starts a new main key and protects every other key again under it.</p>
        </div>
        <div class="lifecycle-actions">
          <button class="ghost small" disabled={busy() || status.loading} onClick={() => void rotate()}>{busy() ? "Rotating…" : "Rotate the key"}</button>
        </div>
      </div>
      <Show when={said()}><p role="status">{said()}</p></Show>
      <Show when={!status.error} fallback={<p class="dim">Could not read the keys: {`${status.error}`}</p>}>
        <Show when={status()} fallback={<p class="dim">Reading…</p>}>
          {(found) => (
            <>
              <dl class="lifecycle-facts">
                <dt>Kept in</dt>
                <dd>{found().kept_in === "keychain"
                  ? "This computer's keychain."
                  : "An encrypted file in the data home, because no keychain could be reached. Anyone who can read the data home and that file can read your data, so keep both private."}</dd>
                <dt>Main key in use</dt>
                <dd>Key {found().version + 1}{found().oldest_in_use < found().version ? `; some things are still protected by key ${found().oldest_in_use + 1}, and rotating again moves them` : ""}.</dd>
                <dt>Keys it protects</dt>
                <dd>{found().keys} in use, {found().destroyed} destroyed by an erasure, {found().held} on hold.</dd>
                <dt>Last rotated</dt>
                <dd>{found().rotations.length === 0 ? "Never." : `${day(found().rotations[found().rotations.length - 1].at)} (${found().rotations.length} ${found().rotations.length === 1 ? "time" : "times"} in all).`}</dd>
              </dl>
              <Show when={found().rotations.length > 0}>
                <div class="ops-table-wrap">
                  <table class="ops-table">
                    <thead><tr><th>Rotated</th><th>Key</th><th>Keys protected again</th></tr></thead>
                    <tbody>
                      <For each={[...found().rotations].reverse()}>{(rotation) => <tr><td>{day(rotation.at)}</td><td>{rotation.version + 1}</td><td>{rotation.rewrapped}</td></tr>}</For>
                    </tbody>
                  </table>
                </div>
              </Show>
            </>
          )}
        </Show>
      </Show>
      <p class="dim">No key is ever shown here or sent anywhere.</p>
    </section>
  );
}

export default function Data(props: { section: "retention" | "storage" | "integrity" | "keys" }) {
  return (
    <div class="data-page">
      <Show when={props.section !== "keys"} fallback={<Keys />}>
        <Show when={props.section !== "integrity"} fallback={<Integrity />}>
          <Show when={props.section === "storage"} fallback={<Retention />}><Storage /></Show>
        </Show>
      </Show>
    </div>
  );
}
