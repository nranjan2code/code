// Settings › Your data (docs/design/74 §6.3 C7, data-architecture plan
// M7b-g): what Vakyartha has stored, how long it keeps each kind, how it is
// protected, and the ways to take it out or remove it.

import { createResource, createSignal, For, Show } from "solid-js";
import * as api from "../api";
import type { SettingsPageId } from "../store";

// The kinds a person meets, in their words. A kind not named here is
// internal working data and is counted under "Working files and logs".
const KINDS: Record<string, [string, string]> = {
  trash: ["Things in the trash", "then erased for good"],
  draft_version: ["Drafts nobody kept", "then moved to the trash"],
  inbox_entry: ["Inbox entries", "then removed"],
  activity_segment: ["Activity history", "then removed"],
  document_history: ["Earlier versions of notes and settings", "then removed"],
};

function size(bytes: number): string {
  const units = ["bytes", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(1)} ${units[unit]}`;
}

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export default function YourData(props: { open: (page: SettingsPageId) => void }) {
  const [data, { refetch }] = createResource(() => api.yourData());
  const [copy, { refetch: refetchCopy }] = createResource(() => api.secondCopy().catch(() => ({ configured: false }) as api.SecondCopy));
  const copyLine = (found: api.SecondCopy) => {
    if (found.role === "standing_by" && !found.held_elsewhere) return "This machine handed the work over and is standing by. Take over on your other machine, or take it back here.";
    if (found.role === "standing_by" && found.released) return "Your other machine handed the work over. Take over here to work on this machine.";
    if (found.role === "standing_by") return "Your other machine holds the work. This one is standing by, and nothing can be started here until you take over.";
    if (found.role === "lost") return "Your other machine took the work over. Taking over here replaces what this machine never copied.";
    const when = found.synced_at ? `Last copied ${new Date(found.synced_at).toLocaleString()}.` : "Nothing has been copied yet.";
    const owed = (found.unpushed ?? 0) > 0 ? ` ${count(found.unpushed ?? 0, "file has", "files have")} changed since; ${found.unpushed === 1 ? "it is" : "they are"} copied between tasks.` : "";
    const trouble = found.last_error ? " The folder could not be reached last time; nothing is lost, and it is tried again." : "";
    return `${when}${owed}${trouble}`;
  };
  const copyNow = async () => {
    setBusy(true);
    setSaid("");
    try {
      await api.copyNow();
      setSaid("The second copy is up to date.");
      void refetchCopy();
    } catch (error) {
      setSaid(`${error instanceof Error ? error.message : error}`);
    } finally {
      setBusy(false);
    }
  };
  const takeOver = async (discard: boolean) => {
    setBusy(true);
    setSaid("");
    try {
      const done = await api.takeOverHere(discard);
      setSaid(done.restart_required ? "This machine holds the work now. Close Vakyartha and open it again before using it." : "This machine holds the work now.");
      void refetchCopy();
    } catch (error) {
      setSaid(`${error instanceof Error ? error.message : error}`);
    } finally {
      setBusy(false);
    }
  };
  const [said, setSaid] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [erasing, setErasing] = createSignal(false);
  const [typed, setTyped] = createSignal("");
  const [receipt, setReceipt] = createSignal<api.InstallErasureReceipt | null>(null);

  const rotate = async () => {
    setBusy(true);
    setSaid("");
    try {
      await api.rotateKeys();
      setSaid("A new key is in use, and everything is protected again under it.");
      void refetch();
    } catch (error) {
      setSaid(`${error instanceof Error ? error.message : error}`);
    } finally {
      setBusy(false);
    }
  };

  const erase = async () => {
    const found = data();
    if (!found) return;
    setBusy(true);
    setSaid("");
    try {
      setReceipt((await api.eraseEverything(found.digest, typed().trim())).receipt);
    } catch (error) {
      setSaid(`${error instanceof Error ? error.message : error}`);
    } finally {
      setBusy(false);
    }
  };

  const file = () => `data:application/json;charset=utf-8,${encodeURIComponent(JSON.stringify(receipt(), null, 2))}`;

  return (
    <>
      <header><h1>Your data</h1><p>Everything Vakyartha keeps is on this computer, and it is yours to back up, clear out or erase.</p></header>
      <Show when={receipt()}>
        {(done) => (
          <section class="settings-group" role="status">
            <h2>Everything was erased</h2>
            <div class="settings-card">
              <div class="setting-row"><div class="setting-copy"><strong>Vakyartha has stopped</strong><span>{count(done().conversations, "conversation", "conversations")} and everything else it stored are gone. Start it again to set it up as new.</span></div><a class="settings-button" href={file()} download={`vakyartha-erasure-${done().id}.json`}>Save the receipt</a></div>
              <For each={done().not_reached}>{(line) => <div class="setting-row"><div class="setting-copy"><strong>Not reached</strong><span>{line}</span></div></div>}</For>
            </div>
          </section>
        )}
      </Show>
      <Show when={!receipt()}>
        <Show when={said()}><p role="status" class="settings-group-copy">{said()}</p></Show>
        <Show when={!data.error} fallback={<p class="settings-group-copy">Could not read what is stored. Try again in a moment.</p>}>
          <Show when={data()} fallback={<p class="settings-group-copy">Counting…</p>}>
            {(found) => (
              <>
                <section class="settings-group">
                  <h2>What is kept</h2>
                  <div class="settings-card">
                    <div class="setting-row"><div class="setting-copy"><strong>Conversations</strong><span>{count(found().conversations, "conversation", "conversations")}, kept until you remove them.</span></div><button type="button" class="settings-button" onClick={() => props.open("archived")}>Archive and trash</button></div>
                    <div class="setting-row"><div class="setting-copy"><strong>Files in the Library</strong><span>{count(found().artifacts, "file", "files")}, with every version.</span></div></div>
                    <div class="setting-row"><div class="setting-copy"><strong>Space used</strong><span>{size(found().bytes)} on this computer, including working files and logs.</span></div></div>
                  </div>
                </section>
                <section class="settings-group">
                  <h2>How long things are kept</h2>
                  <div class="settings-card">
                    <For each={found().keep.filter((rule) => KINDS[rule.class])}>
                      {(rule) => <div class="setting-row"><div class="setting-copy"><strong>{KINDS[rule.class][0]}</strong><span>{count(rule.days, "day", "days")}, {KINDS[rule.class][1]}.</span></div></div>}
                    </For>
                    <div class="setting-row"><div class="setting-copy"><strong>Working files and logs</strong><span>Cleared on their own schedule. Nothing you made is among them.</span></div></div>
                  </div>
                </section>
                <section class="settings-group">
                  <h2>How it is protected</h2>
                  <div class="settings-card">
                    <div class="setting-row"><div class="setting-copy"><strong>Encrypted on this computer</strong><span>{found().keys.kept_in === "keychain" ? "The key is in this computer's keychain." : "The key is in an encrypted file beside the data, because no keychain could be reached."} {found().keys.rotations.length === 0 ? "It has never been changed." : `It was last changed on ${new Date(found().keys.rotations[found().keys.rotations.length - 1].at).toLocaleDateString()}.`}</span></div><button type="button" class="settings-button" disabled={busy()} onClick={() => void rotate()}>Change the key</button></div>
                    <Show when={copy()?.configured} fallback={<div class="setting-row"><div class="setting-copy"><strong>Second copy</strong><span>Not set up. A folder can hold a second, encrypted copy so another of your machines can take the work over. It is set up in the admin console, under Data.</span></div></div>}>
                      <div class="setting-row"><div class="setting-copy"><strong>Second copy</strong><span>{copyLine(copy() as api.SecondCopy)}</span></div>
                        <Show when={copy()?.role === "holder" || copy()?.role === "unset"}><button type="button" class="settings-button" disabled={busy()} onClick={() => void copyNow()}>Copy now</button></Show>
                        <Show when={copy()?.role === "standing_by"}><button type="button" class="settings-button" disabled={busy()} onClick={() => void takeOver(false)}>Take over here</button></Show>
                        <Show when={copy()?.role === "lost"}><button type="button" class="settings-button danger" disabled={busy()} onClick={() => void takeOver(true)}>Take over and replace</button></Show>
                      </div>
                    </Show>
                    <div class="setting-row"><div class="setting-copy"><strong>Back up</strong><span>Copy everything to a folder you choose.</span></div><button type="button" class="settings-button" onClick={() => props.open("storage")}>Storage and backup</button></div>
                  </div>
                </section>
                <section class="settings-group">
                  <h2>Erase everything</h2>
                  <div class="settings-card">
                    <div class="setting-row danger"><div class="setting-copy"><strong>Remove all that Vakyartha has stored</strong><span>Conversations, files, memory, agents, automations, connected accounts and saved keys. Your own folders are never touched. This cannot be undone.{found().held > 0 ? ` ${count(found().held, "thing is", "things are")} on hold; release every hold first.` : ""}</span></div><Show when={!erasing()}><button type="button" class="settings-button danger" disabled={found().held > 0} onClick={() => setErasing(true)}>Erase everything…</button></Show></div>
                    <Show when={erasing()}>
                      <div class="setting-row danger"><div class="setting-copy"><strong>Type “{found().confirm}” to go on</strong><span>Vakyartha will stop when it is done and leave one signed receipt.</span></div>
                        <span class="key-edit">
                          <input class="settings-input" aria-label={`Type ${found().confirm}`} value={typed()} onInput={(event) => setTyped(event.currentTarget.value)} />
                          <button type="button" class="settings-button" onClick={() => { setErasing(false); setTyped(""); }}>Cancel</button>
                          <button type="button" class="settings-button danger" disabled={busy() || typed().trim().toLowerCase() !== found().confirm} onClick={() => void erase()}>Erase everything</button>
                        </span>
                      </div>
                    </Show>
                  </div>
                </section>
              </>
            )}
          </Show>
        </Show>
      </Show>
    </>
  );
}
