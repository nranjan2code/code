/// Operate › Data (plan M7a-c, docs/design/74 §6.2 A7 and A8): what
/// Vakyartha keeps, measured, and what its retention would remove now.
/// Retention only observes at this stage: the plan is the one it would
/// run, shown and not carried out, and a kind of data nothing watches yet
/// is named as such rather than counted as nothing due.

import { createResource, createSignal, For, Show } from "solid-js";
import { api } from "./api";
import type { DataIntegrity, DataPlan, DataStatus, DataTransition, DataUsage, ErasureReceipt, KeyStatus, SyncStatus } from "./types";

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
    if (!window.confirm("Start a new key and protect everything again under it? Nothing is lost, and everything stays readable. Earlier keys are kept so that older backups still open, until you retire them.")) return;
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
  const retire = async () => {
    if (!window.confirm("Destroy every earlier main key? Everything this install holds stays readable. A backup or key file made before the last rotation will no longer open here. This cannot be undone.")) return;
    setBusy(true);
    setSaid("");
    try {
      const { retirement } = await api.retireDataKeys();
      setSaid(`Done. ${retirement.retired} earlier ${retirement.retired === 1 ? "key was" : "keys were"} destroyed.`);
      void refetch();
    } catch (error) {
      setSaid(`${error instanceof Error ? error.message : error}`);
    } finally {
      setBusy(false);
    }
  };
  const rotations = () => (status()?.rotations ?? []).filter((row) => !row.retired);
  const canRetire = () => {
    const found = status();
    return !!found && found.version > found.retired;
  };
  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>Keys</h2>
          <p class="dim">Everything Vakyartha stores is encrypted. Each conversation and file has a key of its own, and one main key protects those. Rotating starts a new main key and protects every other key again under it.</p>
        </div>
        <div class="lifecycle-actions">
          <button class="ghost small" disabled={busy() || status.loading} onClick={() => void rotate()}>{busy() ? "Working…" : "Rotate the key"}</button>
          <Show when={canRetire()}>
            <button class="ghost small danger" disabled={busy() || status.loading} onClick={() => void retire()}>Retire earlier keys</button>
          </Show>
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
                <dd>{rotations().length === 0 ? "Never." : `${day(rotations()[rotations().length - 1].at)} (${rotations().length} ${rotations().length === 1 ? "time" : "times"} in all).`}</dd>
                <dt>Earlier keys</dt>
                <dd>{found().version === 0 ? "None." : found().retired >= found().version ? "All destroyed. Backups made before the last rotation no longer open here." : `${found().version - found().retired} kept, so that older backups still open.`}</dd>
              </dl>
              <Show when={found().rotations.length > 0}>
                <div class="ops-table-wrap">
                  <table class="ops-table">
                    <thead><tr><th>When</th><th>Key</th><th>What happened</th></tr></thead>
                    <tbody>
                      <For each={[...found().rotations].reverse()}>{(rotation) => <tr><td>{day(rotation.at)}</td><td>{rotation.version + 1}</td><td>{rotation.retired ? `${rotation.retired} earlier ${rotation.retired === 1 ? "key" : "keys"} destroyed` : `Rotated; ${rotation.rewrapped} keys protected again`}</td></tr>}</For>
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

const STANDS: Record<string, string> = {
  holder: "This machine holds the work. What it does is copied to the folder as it goes.",
  standing_by: "This machine is standing by. Your other machine holds the work, and nothing can be started here until you take over.",
  lost: "Your other machine took the work over from this one. Nothing can be started here until this machine is brought up to the copy.",
  unset: "Nothing has been copied from this machine yet.",
};

/// Operate › Data › Second copy (docs/design/74 §6.2 A18, plan M9): the
/// folder that holds a second copy of everything, which of two machines
/// holds the work, and the key file the other machine reads the copy with.
function Sync() {
  const [status, { refetch }] = createResource<SyncStatus>(() => api.syncStatus());
  const [busy, setBusy] = createSignal(false);
  const [said, setSaid] = createSignal("");
  const [folder, setFolder] = createSignal("");
  const [passphrase, setPassphrase] = createSignal("");
  const [keyFile, setKeyFile] = createSignal("");
  const [incoming, setIncoming] = createSignal("");
  const act = async (what: () => Promise<{ restart_required?: boolean; restarting?: boolean } | void>, done: string) => {
    setBusy(true);
    setSaid("");
    try {
      const result = await what();
      setSaid(result && result.restarting
        ? `${done} Vakyartha is starting again; this page reloads when it is back.`
        : result && result.restart_required ? `${done} Start Vakyartha again before using it.` : done);
      if (result && result.restarting) api.reloadWhenBack();
      void refetch();
    } catch (error) {
      setSaid(`${error instanceof Error ? error.message : error}`);
    } finally {
      setBusy(false);
    }
  };
  const takeOver = (force: boolean) => {
    const lost = status()?.role === "lost";
    if (force && !window.confirm("Take over without a hand-over? Do this only when the other machine is lost or cannot be reached. Whatever it never copied to the folder is not brought here.")) return;
    if (lost && !window.confirm("This machine holds work that was never copied to the folder. Taking over replaces it with the copy. Go on?")) return;
    void act(() => api.syncDo("takeover", { force, discard: lost }), "This machine holds the work now.");
  };
  const exportKey = async () => {
    setBusy(true);
    setSaid("");
    try {
      setKeyFile((await api.syncKeyExport(passphrase())).file);
      setPassphrase("");
      setSaid("The key file is ready. Save it, carry it to the other machine yourself, and delete it once it is imported there.");
    } catch (error) {
      setSaid(`${error instanceof Error ? error.message : error}`);
    } finally {
      setBusy(false);
    }
  };
  const fileHref = () => `data:text/plain;charset=utf-8,${encodeURIComponent(keyFile())}`;
  return (
    <>
      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Second copy</h2>
            <p class="dim">A folder that holds a second copy of everything Vakyartha stores, encrypted: an external drive, a network share, or a folder another tool keeps in step. Two of your machines can take turns on it. One holds the work; the other stands by.</p>
          </div>
          <Show when={status()?.configured}>
            <div class="lifecycle-actions">
              <Show when={status()?.role === "holder" || status()?.role === "unset"}>
                <button class="ghost small" disabled={busy()} onClick={() => void act(() => api.syncDo("now"), "The copy is up to date.")}>Copy now</button>
              </Show>
              <Show when={status()?.role === "holder"}>
                <button class="ghost small" disabled={busy()} onClick={() => void act(() => api.syncDo("handover"), "Handed over. This machine is standing by; take over on the other one.")}>Hand over</button>
              </Show>
              <Show when={status()?.role === "standing_by" || status()?.role === "lost"}>
                <button class="ghost small" disabled={busy()} onClick={() => takeOver(false)}>Take over here</button>
                <button class="ghost small danger" disabled={busy()} onClick={() => takeOver(true)}>Take over, the other is lost…</button>
              </Show>
            </div>
          </Show>
        </div>
        <Show when={said()}><p role="status">{said()}</p></Show>
        <Show when={!status.error} fallback={<p class="dim">Could not read the second copy: {`${status.error}`}</p>}>
          <Show when={status()} fallback={<p class="dim">Reading…</p>}>
            {(found) => (
              <Show
                when={found().configured}
                fallback={
                  <div class="lifecycle-actions">
                    <input class="rules-days" style={{ width: "28rem" }} aria-label="Folder for the second copy" placeholder="A folder that already exists, for example /Volumes/Backup/vakyartha" value={folder()} onInput={(event) => setFolder(event.currentTarget.value)} />
                    <button class="ghost small" disabled={busy() || !folder().trim()} onClick={() => void act(() => api.syncDo("setup", { folder: folder() }), "The folder is set. Copy now to make the first copy.")}>Use this folder</button>
                  </div>
                }
              >
                <dl class="lifecycle-facts">
                  <dt>Folder</dt>
                  <dd>{found().remote}{found().reachable ? "" : " (it cannot be reached right now)"}</dd>
                  <dt>This machine</dt>
                  <dd>{found().role === "standing_by" && !found().held_elsewhere
                    ? "This machine handed the work over and is standing by. Take over on your other machine, or take it back here."
                    : `${STANDS[found().role ?? "unset"]}${found().role === "standing_by" && found().released ? " It has handed over, so this machine can take over." : ""}`}</dd>
                  <dt>Last copied</dt>
                  <dd>{found().synced_at ? `${day(found().synced_at as string)}.` : "Never."}{(found().unpushed ?? 0) > 0 ? ` ${found().unpushed === 1 ? "1 file has" : `${found().unpushed} files have`} changed here since.` : found().synced_at ? " Nothing has changed here since." : ""}</dd>
                  <Show when={found().last_error}>
                    <dt>Last try</dt>
                    <dd>It did not go through: {`${(found().last_error as string).replace(/\.?\s*$/, "")}.`} Nothing is lost; it is tried again.</dd>
                  </Show>
                </dl>
                <p><button class="ghost small" disabled={busy()} onClick={() => { if (window.confirm("Stop using this folder? Nothing in it is touched.")) void act(() => api.syncDo("forget"), "This machine no longer uses a folder."); }}>Stop using this folder</button></p>
              </Show>
            )}
          </Show>
        </Show>
      </section>
      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Key file for your other machine</h2>
            <p class="dim">The folder holds only encrypted data, and no key is ever put in it. Your other machine reads it with a key file you make here under a passphrase and carry there yourself. After the key is changed (Keys), make a new one.</p>
          </div>
        </div>
        <div class="lifecycle-actions">
          <input class="rules-days" style={{ width: "20rem" }} type="password" autocomplete="new-password" aria-label="Passphrase for the key file" placeholder="A passphrase of at least 12 characters" value={passphrase()} onInput={(event) => setPassphrase(event.currentTarget.value)} />
          <button class="ghost small" disabled={busy() || passphrase().length < 12} onClick={() => void exportKey()}>Make a key file</button>
          <Show when={keyFile()}><a class="ghost small" href={fileHref()} download="vakyartha-key-file.txt">Save the key file</a></Show>
        </div>
        <p class="dim">On a new machine, or after the key was changed on the other one: paste the key file and its passphrase.</p>
        <div class="lifecycle-actions">
          <textarea class="rules-days" style={{ width: "28rem", height: "4.5rem" }} aria-label="Key file from your other machine" placeholder="The key file's text" value={incoming()} onInput={(event) => setIncoming(event.currentTarget.value)} />
          <button class="ghost small" disabled={busy() || !incoming().trim() || passphrase().length < 1} onClick={() => void act(async () => { const done = await api.syncKeyImport(incoming(), passphrase()); setIncoming(""); setPassphrase(""); return done; }, "This machine now holds your keys.")}>Take these keys</button>
        </div>
      </section>
    </>
  );
}

export default function Data(props: { section: "retention" | "storage" | "integrity" | "keys" | "sync" }) {
  return (
    <div class="data-page">
      <Show when={props.section !== "sync"} fallback={<Sync />}>
      <Show when={props.section !== "keys"} fallback={<Keys />}>
        <Show when={props.section !== "integrity"} fallback={<Integrity />}>
          <Show when={props.section === "storage"} fallback={<Retention />}><Storage /></Show>
        </Show>
      </Show>
      </Show>
    </div>
  );
}
