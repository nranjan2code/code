import { createResource, createSignal, For, Show } from "solid-js";
import * as api from "../api";
import { abilitySheetOpen, openInLibrary, sessions, setAbilitySheetOpen, setLibraryOpen, technicalDetails } from "../store";
import { activate } from "../App";
import { relAgo } from "../time";
import Sheet from "./Sheet";
import Icon from "./Icon";
import { Pill } from "./fileState";

/** An ability's state in plain words (docs/design/93 §9.3). */
export const ABILITY_STATE: Record<api.Ability["state"], { word: string; mark: string }> = {
  working: { word: "Working", mark: "accepted" },
  failing: { word: "Failing", mark: "changed" },
  offered: { word: "Ready for every Agent", mark: "current" },
  waiting: { word: "Waiting for you", mark: "waiting" },
  turned_off: { word: "Turned off", mark: "undone" },
};

const VERSION_STATE: Record<api.AbilityVersion["state"], { word: string; mark: string }> = {
  in_use: { word: "In use", mark: "accepted" },
  waiting: { word: "Waiting for you", mark: "waiting" },
  failed_tests: { word: "Its tests failed", mark: "changed" },
  failed_in_use: { word: "Failed when used", mark: "changed" },
  earlier: { word: "Used before", mark: "current" },
  not_used: { word: "Never used", mark: "undone" },
};

const PART: Record<string, string> = {
  instructions: "its instructions",
  declaration: "what it accepts and reaches",
  tests: "its tests",
  presentation: "how it shows results",
  program: "its program",
};

/** What a version changed from the one before, in words: by part, never
 *  by file name (file names show with technical details). */
function changeWords(version: api.AbilityVersion): string {
  if (version.number === 1) return "The first version.";
  if (version.changes.length === 0) return "The same as the version before.";
  const parts = [...new Set(version.changes.map((change) => PART[change.part] ?? PART.program))];
  const list = parts.length > 1 ? `${parts.slice(0, -1).join(", ")} and ${parts.at(-1)}` : parts[0];
  return `Changed ${list}.`;
}

function testWords(tests: NonNullable<api.AbilityVersion["tests"]>): string {
  const total = tests.cases.length;
  if (total === 0) return tests.passed ? "Its tests passed." : "Its tests failed.";
  const passed = tests.cases.filter((test) => test.passed).length;
  if (passed === total) return total === 1 ? "Its one test passed." : `All ${total} tests passed.`;
  return `${passed} of ${total} tests passed.`;
}

/** A conversation's name as the sidebar knows it. */
function conversationName(id?: string | null, title?: string | null): string {
  const known = id ? sessions().find((session) => session.session_id === id) : undefined;
  return known?.title || title || (known?.agent?.name ? `${known.agent.name}'s conversation` : "A conversation");
}

/** How often the uses went, as one sentence. */
function usesWords(uses: api.AbilitySheet["uses"], since?: number | null): string {
  const failed = uses.filter((use) => use.outcome === "failed").length;
  const count = uses.length === 1 ? "once" : `${uses.length} times`;
  const outcome = failed > 0 ? `${failed} failed` : "none failed";
  return since ? `Since version ${since}: used ${count}, ${outcome}.` : `Used ${count}, ${outcome}.`;
}

function VersionRow(props: { version: api.AbilityVersion; busy: boolean; onUse: () => void }) {
  const v = () => props.version;
  return (
    <li class="ability-version">
      <div class="ability-version-head">
        <strong>Version {v().number}</strong>
        <Pill mark={VERSION_STATE[v().state].mark}>{VERSION_STATE[v().state].word}</Pill>
        <span class="ability-muted">{v().by_you ? "Chosen by you" : "Made by the Agent"} · {relAgo(v().at)}</span>
      </div>
      <p>{changeWords(v())}</p>
      <Show when={v().tests}>
        {(tests) => (
          <details class="ability-tests">
            <summary>{testWords(tests())}</summary>
            <ul>
              <For each={tests().cases}>
                {(test) => (
                  <li class={test.passed ? "passed" : "failed"}>
                    <Icon name={test.passed ? "check" : "warning"} size={13} />
                    <span>{test.name}<Show when={test.detail}>{(detail) => <>: {detail()}</>}</Show></span>
                  </li>
                )}
              </For>
            </ul>
          </details>
        )}
      </Show>
      <Show when={technicalDetails()}>
        <p class="ability-technical">
          {v().changes.map((change) => `${change.change} ${change.path}`).join(" · ") || "no file changes"}
          <br />digest {v().digest.slice(0, 12)}<Show when={v().tests}>{(tests) => <> · run {tests().run}</>}</Show>
        </p>
      </Show>
      <Show when={v().can_use}>
        <button type="button" class="settings-button" disabled={props.busy} onClick={props.onUse}>Use this version</button>
      </Show>
    </li>
  );
}

/** The capability sheet (docs/design/93 §9.1, §9.2): what an ability can
 *  do, why it waits, its versions with their tests, its recent uses, and
 *  what the owner can do about it. Review of an ability happens here. */
function AbilitySheetBody(props: { name: string; agent?: string }) {
  const [sheet, { refetch }] = createResource(() => api.abilitySheet(props.name, props.agent));
  const [busy, setBusy] = createSignal(false);
  const [notice, setNotice] = createSignal<string | null>(null);
  const close = () => setAbilitySheetOpen(null);
  const act = async (work: () => Promise<unknown>, done: string, closeAfter = false) => {
    setBusy(true);
    try {
      await work();
      setNotice(done);
      if (closeAfter) close();
    } catch (error) {
      setNotice(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
    if (!closeAfter) void refetch();
  };
  const use = async (name: string, version: api.AbilityVersion) => {
    setBusy(true);
    try {
      const { in_use } = await api.useAbilityVersion(name, version.version, props.agent);
      setNotice(in_use === version.number ? `Version ${in_use} is in use again.` : `Version ${version.number} is in use again, kept as version ${in_use}.`);
    } catch (error) {
      setNotice(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
    void refetch();
  };
  const openConversation = (id: string) => {
    close();
    setLibraryOpen(false);
    void activate(id);
  };
  const waitingVersion = () => sheet()?.history.find((v) => v.state === "waiting");
  const footer = () => {
    const found = sheet();
    if (!found) return undefined;
    const name = found.name;
    return (
      <>
        <Show when={found.proposal}>
          {(id) => (
            <>
              <Show
                when={found.reaches.length > 0}
                fallback={<button type="button" class="btn primary" disabled={busy()} onClick={() => void act(() => api.promoteProposal(id(), props.agent), `${name} is now available to every Agent.`)}>Approve</button>}
              >
                <button type="button" class="btn primary" disabled={busy()} onClick={() => void act(() => api.approveProposal(id(), props.agent), `${name} is now this Agent's.`)}>Approve</button>
              </Show>
              <Show when={found.proposed_in}>
                {(conversation) => <button type="button" class="btn" disabled={busy()} onClick={() => openConversation(conversation())}>Ask for changes</button>}
              </Show>
              <button type="button" class="btn danger" disabled={busy()} onClick={() => void act(() => api.rejectProposal(id(), props.agent), `${name} was declined.`, true)}>Decline</button>
            </>
          )}
        </Show>
        <Show when={found.state === "offered"}>
          <button type="button" class="btn primary" disabled={busy()} onClick={() => void act(() => api.promoteAbility(name, props.agent), `${name} is now available to every Agent.`)}>Give to every Agent</button>
        </Show>
        <Show when={found.state === "working" || found.state === "failing" || found.state === "offered"}>
          <button type="button" class="btn" disabled={busy()} onClick={() => void act(() => api.turnOffAbility(name, props.agent), `${name} is turned off. Its versions stay here to use again.`)}>Turn off</button>
        </Show>
        <Show when={found.artifact}>
          {(artifact) => <button type="button" class="btn" onClick={() => { close(); openInLibrary(artifact()); }}>In the Library</button>}
        </Show>
      </>
    );
  };
  return (
    <Sheet
      class="ability-sheet"
      title={<><Icon name="spark" size={16} /> {props.name}</>}
      subtitle={sheet()?.description || undefined}
      onClose={close}
      busy={busy()}
      footer={footer()}
    >
      <Show when={!sheet.error} fallback={<div class="worker-error">This ability could not be read: {String(sheet.error?.message ?? sheet.error)}</div>}>
        <Show when={sheet()} fallback={<div class="dock-empty">Loading…</div>}>
          {(found) => (
            <div class="ability-sheet-body">
              <Show when={notice()}><p class="library-note" role="status">{notice()}</p></Show>
              <section class="ability-reach" aria-label="What it can do">
                <div class="ability-reach-head">
                  <Pill mark={ABILITY_STATE[found().state].mark}>{ABILITY_STATE[found().state].word}</Pill>
                </div>
                <Show when={found().reaches.length > 0} fallback={<p class="ability-muted">It has no program: it is instructions the Agent follows.</p>}>
                  <ul>
                    <For each={found().reaches}>{(line) => <li>{line}</li>}</For>
                  </ul>
                </Show>
                <Show when={found().why}><p class="ability-why">{found().state === "waiting" ? "It is waiting" : "A new version is waiting"} because {found().why}.</p></Show>
                <Show when={waitingVersion()}>
                  {(waiting) => <p class="ability-muted">Version {waiting().number}: {changeWords(waiting()).toLowerCase().replace(/\.$/, "")}; {waiting().tests ? testWords(waiting().tests!).toLowerCase() : "not tested."}</p>}
                </Show>
              </section>

              <section aria-label="Versions">
                <h3>Versions</h3>
                <Show when={found().history.length > 0} fallback={<p class="ability-muted">No versions are kept for it.</p>}>
                  <ol class="ability-versions">
                    <For each={found().history}>
                      {(version) => (
                        <VersionRow
                          version={version}
                          busy={busy()}
                          onUse={() => void use(found().name, version)}
                        />
                      )}
                    </For>
                  </ol>
                </Show>
              </section>

              <section aria-label="Recent uses">
                <h3>Recent uses</h3>
                <Show
                  when={found().uses.length > 0}
                  fallback={<p class="ability-muted">{found().uses_since ? `Not used since version ${found().uses_since}.` : "Not used yet."}</p>}
                >
                  <p class="ability-muted">{usesWords(found().uses, found().uses_since)}</p>
                  <ul class="ability-uses">
                    <For each={found().uses}>
                      {(use) => (
                        <li>
                          <Pill mark={use.outcome === "failed" ? "changed" : use.outcome === "done" ? "accepted" : "waiting"}>
                            {use.outcome === "failed" ? "Failed" : use.outcome === "done" ? "Worked" : "No answer yet"}
                          </Pill>
                          <Show when={use.conversation} fallback={<span>{conversationName(null, use.title)}</span>}>
                            {(conversation) => (
                              <button type="button" class="file-link" onClick={() => openConversation(conversation())}>{conversationName(conversation(), use.title)}</button>
                            )}
                          </Show>
                          <span class="ability-muted">{relAgo(use.at)}</span>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
              </section>

              <Show when={technicalDetails() && found().declaration}>
                <section aria-label="Declaration">
                  <h3>Declaration</h3>
                  <pre class="ability-declaration">{found().declaration}</pre>
                </section>
              </Show>
            </div>
          )}
        </Show>
      </Show>
    </Sheet>
  );
}

export default function AbilitySheet() {
  return (
    <Show when={abilitySheetOpen()} keyed>
      {(open) => <AbilitySheetBody name={open.name} agent={open.agent} />}
    </Show>
  );
}
