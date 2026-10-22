import { createEffect, createMemo, createResource, createSignal, For, on, Show } from "solid-js";
import * as api from "../api";
import type { ArtifactDetail, ArtifactShare, ArtifactSummary, ArtifactVersion, FileNeeds, ReadInput, ShareRole, VersionCompare } from "../api";
import { host } from "../host";
import { abilitySheetOpen, openArtifactCanvas, libraryAbilities, libraryFocus, openAbilitySheet, setLibraryAbilities, setLibraryFocus, setLibraryOpen, technicalDetails } from "../store";
import { relAgo } from "../time";
import { activate, openAgentChat } from "../App";
import { attachArtifact } from "../attachFiles";
import { compareTexts, type Run } from "../wordDiff";
import Icon, { type IconName } from "./Icon";
import LibraryPreview from "./LibraryPreview";
import OfficeChangeList from "./OfficeChangeList";
import PageView, { readPage } from "./PageView";
import PageKeeper from "./PageKeeper";
import { Pill, stateOf } from "./fileState";
import { ABILITY_STATE } from "./AbilitySheet";

/** Plain words for an artifact's kind (doc 75 §7). */
const KIND: Record<ArtifactSummary["kind"], string> = {
  file: "File",
  document: "Document",
  changeset: "Code changes",
  page: "Page",
  skill: "Ability",
};

/** A display hint, never a replacement for the artifact's declared kind. */
function kindName(artifact: ArtifactSummary): string {
  if (artifact.kind !== "file" && artifact.kind !== "document") return KIND[artifact.kind];
  const ext = artifact.path.split(".").pop()?.toLowerCase() ?? "";
  if (/^(html?|xhtml)$/.test(ext)) return "Web page";
  if (/^(png|jpe?g|gif|webp|svg|ico|bmp|avif)$/.test(ext)) return "Image";
  if (/^(mp3|wav|ogg|m4a|aac|flac)$/.test(ext)) return "Audio";
  if (/^(mp4|webm|mov|m4v)$/.test(ext)) return "Video";
  if (/^(xlsx|xlsm|xltx)$/.test(ext)) return "Spreadsheet";
  if (/^(pptx|pptm|potx)$/.test(ext)) return "Slides";
  if (/^(csv|tsv)$/.test(ext)) return "Data table";
  if (/^(docx|docm|dotx|md|markdown|pdf)$/.test(ext)) return "Document";
  return KIND[artifact.kind];
}

const CHANGED: { value: string; label: string; days: number }[] = [
  { value: "any", label: "Any time", days: Infinity },
  { value: "today", label: "Today", days: 1 },
  { value: "week", label: "This week", days: 7 },
  { value: "month", label: "This month", days: 31 },
];

const ROLE: Record<ShareRole, string> = {
  viewer: "Can view",
  commenter: "Can comment",
  editor: "Can edit",
};

/** Who may open an artifact, and a new link for one person (doc 82 §8). */
function SharePanel(props: { artifact: ArtifactDetail; onClose: () => void }) {
  const [shares, { refetch }] = createResource(() => props.artifact.id, (id) => api.libraryShares(id).then((res) => res.shares));
  const [name, setName] = createSignal("");
  const [role, setRole] = createSignal<ShareRole>("viewer");
  const [days, setDays] = createSignal(7);
  const [historyFrom, setHistoryFrom] = createSignal("");
  const [made, setMade] = createSignal<{ name: string; token: string } | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const link = () => `${window.location.origin}/app/?shared=artifact`;

  const create = async () => {
    setError(null);
    try {
      const res = await api.libraryShare(props.artifact.id, {
        name: name().trim(),
        role: role(),
        expires_in_hours: days() * 24,
        history_from: historyFrom() || undefined,
      });
      setMade({ name: res.share.name, token: res.token });
      setName("");
      await refetch();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const stop = async (share: ArtifactShare) => {
    setError(null);
    try {
      await api.libraryUnshare(props.artifact.id, share.id);
      await refetch();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <section class="library-share" aria-label="Share">
      <div class="library-head">
        <h3>Share</h3>
        <button type="button" class="btn sm" onClick={props.onClose}>Done</button>
      </div>
      <p class="library-note">A link opens only this, never the conversations that made it. Earlier versions are shown only if you include them.</p>
      <Show when={error()}><div class="worker-error" role="alert">{error()}</div></Show>
      <form class="library-share-form" onSubmit={(e) => { e.preventDefault(); void create(); }}>
        <input placeholder="Who is it for?" value={name()} onInput={(e) => setName(e.currentTarget.value)} aria-label="Who is it for" />
        <select value={role()} onChange={(e) => setRole(e.currentTarget.value as ShareRole)} aria-label="What they can do">
          <For each={Object.entries(ROLE)}>{([value, label]) => <option value={value}>{label}</option>}</For>
        </select>
        <select value={historyFrom()} onChange={(e) => setHistoryFrom(e.currentTarget.value)} aria-label="Versions shown">
          <option value="">Only the current version</option>
          <For each={props.artifact.history}>{(v, index) => <option value={v.id}>From version {index() + 1} on</option>}</For>
        </select>
        <select value={String(days())} onChange={(e) => setDays(Number(e.currentTarget.value))} aria-label="Lasts">
          <option value="1">For a day</option>
          <option value="7">For a week</option>
          <option value="30">For 30 days</option>
        </select>
        <button class="btn sm primary" type="submit" disabled={!name().trim()}>Create link</button>
      </form>
      <Show when={made()}>
        {(created) => (
          <div class="library-share-made">
            <p>Send {created().name} this address and code. The code is shown only now.</p>
            <code>{link()}</code>
            <code>{created().token}</code>
          </div>
        )}
      </Show>
      <ul class="library-shares">
        <For each={shares() ?? []}>
          {(share) => (
            <li>
              <span>{share.name} · {ROLE[share.role]}{share.status === "active" ? "" : ` · ${share.status === "revoked" ? "stopped" : "expired"}`}</span>
              <Show when={share.status === "active"}>
                <button class="btn sm" onClick={() => void stop(share)}>Stop sharing</button>
              </Show>
            </li>
          )}
        </For>
      </ul>
    </section>
  );
}

/** Who made a version, read from its record (doc 82 §5). */
/** When a draft nobody keeps goes to the trash. */
function fadesIn(until: string): string {
  const days = Math.ceil((new Date(until).getTime() - Date.now()) / 86_400_000);
  if (days <= 0) return "Goes to the trash soon unless you keep it";
  return `Goes to the trash in ${days} day${days === 1 ? "" : "s"} unless you keep it`;
}

function maker(version: ArtifactVersion): string {
  if (version.from === "workspace") return "You, outside Vak";
  return version.from === "person" ? "You" : "Vakyartha";
}

/** Files small enough and plain enough to show as text. */
function isText(artifact: ArtifactSummary): boolean {
  return artifact.kind === "file"
    && (artifact.size ?? 0) <= 256 * 1024
    && /\.(md|txt|csv|tsv|json|ya?ml|toml|html?|css|js|ts|tsx|py|rs|go|sh|sql|xml|ini)$/i.test(artifact.path);
}

/** The icon for a file, by its kind. */
function kindIcon(path: string): IconName {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  if (["docx", "docm", "dotx"].includes(ext)) return "word";
  if (["xlsx", "xlsm", "xltx", "csv", "tsv"].includes(ext)) return "sheet";
  if (["pptx", "pptm", "potx"].includes(ext)) return "slides";
  if (ext === "vsdx") return "diagram";
  if (ext === "pdf") return "pdf";
  if (/^(mp3|wav|ogg|m4a|aac|flac)$/.test(ext)) return "mic";
  if (/^(mp4|webm|mov|m4v)$/.test(ext)) return "monitor";
  if (["html", "htm", "xhtml"].includes(ext)) return "preview";
  if (["css", "js", "ts", "tsx", "py", "rs", "go", "sh", "json", "sql"].includes(ext)) return "code";
  return "file";
}

/** Closes the menu a menu item sits in, returning focus to its button. */
function closeMenu(event: MouseEvent) {
  const menu = event.currentTarget instanceof HTMLElement ? event.currentTarget.closest("details") : null;
  menu?.removeAttribute("open");
  menu?.querySelector<HTMLElement>("summary")?.focus({ preventScroll: true });
}

/** On the folder's own line, or a branch of drafts beside it. */
function onFolderLine(v: ArtifactVersion): boolean {
  return ["in_folder", "accepted", "written", "found_outside", "undone"].includes(v.now?.is ?? "");
}

/** What the file waits on the person for, as pills (doc 90 §5). */
function FileStates(props: { needs?: FileNeeds }) {
  return (
    <Show when={props.needs}>
      {(needs) => (
        <div class="file-states">
          <Show when={needs().changed_outside}><Pill mark="changed">Changed outside Vak</Pill></Show>
          <Show when={needs().missing}><Pill mark="undone">Not in your folder now</Pill></Show>
          <Show when={needs().waiting > 0}>
            <Pill mark="waiting">{needs().waiting === 1 ? "1 draft waits for you" : `${needs().waiting} drafts wait for you`}</Pill>
          </Show>
        </div>
      )}
    </Show>
  );
}

/** What a turn read first, in the words a person uses: a file by its name,
 *  an earlier draft as one, a page by its site. The path itself only with
 *  technical details on (doc 75). */
function readWords(input: ReadInput): { text: string; href?: string; detail: string } {
  if (input.kind === "page") {
    let site = input.url;
    try {
      site = new URL(input.url).hostname.replace(/^www\./, "");
    } catch {
      // An address that does not parse is shown as it was fetched.
    }
    return { text: site, href: input.url, detail: input.url };
  }
  const name = input.path.split("/").pop() || input.path;
  const draft = input.path.includes(".vak/scratch/");
  return { text: draft ? `an earlier draft of ${name}` : name, detail: input.path };
}

/** The version graph, newest first, and where the chosen version came from
 *  (doc 90 §5, §5.1). */
function History(props: {
  found: ArtifactDetail;
  chosen: string | null;
  onChoose: (id: string) => void;
  act: (work: () => Promise<unknown>) => Promise<void>;
  onOpenConversation: (session: string) => void;
}) {
  const number = (id?: string | null) => props.found.history.findIndex((v) => v.id === id) + 1;
  /** A version's name in a sentence: its own title, else its place. */
  const label = (id?: string | null) => {
    const found = props.found.history.find((v) => v.id === id);
    return found?.title ? `“${found.title}”` : `version ${number(id)}`;
  };
  const picked = () => props.found.history.find((v) => v.id === props.chosen) ?? null;
  const [origin] = createResource(
    () => {
      const v = picked();
      return v?.session && v.turn ? ([props.found.id, v.id] as const) : null;
    },
    ([id, version]) => api.libraryVersionOrigin(id, version).catch(() => null),
  );
  const [compared, setCompared] = createSignal<{ of: string; result: VersionCompare } | null>(null);
  const [comparing, setComparing] = createSignal(false);
  const [compareError, setCompareError] = createSignal<string | null>(null);
  const compare = async (version: string) => {
    setComparing(true);
    setCompareError(null);
    try {
      setCompared({ of: version, result: await api.libraryVersionCompare(props.found.id, version) });
    } catch {
      setCompareError("Your folder could not be read just now.");
    } finally {
      setComparing(false);
    }
  };
  const comparison = () => {
    const shown = compared();
    return shown && shown.of === picked()?.id ? shown.result : null;
  };
  return (
    <aside class="library-history" aria-label="History">
      <h3>History</h3>
      <ol class="file-graph" aria-label="Versions, newest first">
        <For each={[...props.found.history].reverse()}>
          {(v) => {
            const state = stateOf(v.now, v.from);
            const behind = v.now?.is === "waiting" && v.now.behind;
            return (
              <li classList={{ chosen: props.chosen === v.id, branch: !onFolderLine(v) }}>
                <span class="rail" aria-hidden="true"><span class={`pin st-${state?.mark ?? "accepted"}`} /></span>
                <button type="button" class="file-node" onClick={() => props.onChoose(v.id)}>
                  <span class="t"><strong>{v.title ?? `Version ${number(v.id)}`}</strong><Show when={state}>{(st) => <Pill mark={st().mark}>{st().word}</Pill>}</Show></span>
                  <span class="d">
                    {maker(v)} · {relAgo(v.at)}
                    <Show when={v.parent}> · from {label(v.parent)}</Show>
                    <Show when={v.removed}> · removes the file</Show>
                  </span>
                  <Show when={behind}><span class="warn">Made before the copy in your folder</span></Show>
                  <Show when={v.draft_until}>{(until) => <span class="library-fade">{fadesIn(until())}</span>}</Show>
                </button>
              </li>
            );
          }}
        </For>
      </ol>
      <Show when={picked()}>
        {(v) => (
          <section class="file-origin" aria-label={`Where ${label(v().id)} came from`}>
            <h4>Where {label(v().id)} came from</h4>
            <dl>
              <dt>Made by</dt>
              <dd>{maker(v())} · {relAgo(v().at)}</dd>
              <Show when={v().session}>
                {(session) => (
                  <>
                    <dt>Conversation</dt>
                    <dd><button type="button" class="file-link" onClick={() => props.onOpenConversation(session())}>Open the conversation that made it</button></dd>
                  </>
                )}
              </Show>
              <Show when={origin()?.asked}>
                {(asked) => (
                  <>
                    <dt>You asked</dt>
                    <dd class="file-asked">“{asked()}”</dd>
                  </>
                )}
              </Show>
              <Show when={(v().derived_from ?? []).length > 0}>
                <dt>It read first</dt>
                <dd>
                  <For each={v().derived_from}>
                    {(input, index) => {
                      const words = readWords(input);
                      return (
                        <>
                          <Show when={index() > 0}> · </Show>
                          <Show when={words.href} fallback={<span title={technicalDetails() ? words.detail : undefined}>{words.text}</span>}>
                            {(href) => <a class="file-link" href={href()} target="_blank" rel="noopener noreferrer">{words.text}</a>}
                          </Show>
                          <Show when={technicalDetails() && !words.href}> <span class="file-detail">({words.detail})</span></Show>
                        </>
                      );
                    }}
                  </For>
                </dd>
              </Show>
              <Show when={v().parent}>
                <dt>Made from</dt>
                <dd>
                  <button type="button" class="file-link" onClick={() => props.onChoose(v().parent!)}>{label(v().parent)}</button>
                  <Show when={v().now?.is === "waiting" && (v().now as { behind: boolean }).behind}>, not the copy in your folder</Show>
                </dd>
              </Show>
            </dl>
            <div class="library-actions">
              <Show when={!v().erased && !v().removed}>
                <button class="btn sm" disabled={comparing()} onClick={() => void compare(v().id)}>{comparing() ? "Comparing…" : "Compare with your folder"}</button>
              </Show>
              <Show when={v().trashed_at && !v().erased}>
                <button class="btn sm" onClick={() => void props.act(() => api.setDraftTrashed(props.found.id, v().id, false))}>Restore</button>
              </Show>
              <Show when={!v().saved && !v().erased && !v().trashed_at}>
                <button class="btn sm" onClick={() => void props.act(() => api.librarySave(props.found.id, v().id))}>Keep</button>
              </Show>
              <Show when={v().draft_until}>
                <button class="btn sm danger" onClick={() => void props.act(() => api.setDraftTrashed(props.found.id, v().id, true))}>Move to trash</button>
              </Show>
            </div>
            <Show when={compareError()}>{(message) => <p class="file-compare-note" role="status">{message()}</p>}</Show>
            <Show when={comparison()}>
              {(result) => (
                <div class="file-compare" role="region" aria-label="Compared with your folder">
                  <p class="file-compare-note" role="status">
                    <Show when={result().folder === "absent"}>This file is not in your folder now.</Show>
                    <Show when={result().folder === "same"}>Your folder holds this version.</Show>
                    <Show when={result().folder === "different"}>
                      Your folder holds{" "}
                      <Show when={result().folder_version} fallback={<>a copy no version matches.</>}>
                        {(other) => <button type="button" class="file-link" onClick={() => props.onChoose(other())}>{label(other())}</button>}
                      </Show>
                      <Show when={result().folder_version}>.</Show>
                    </Show>
                  </p>
                  <Show when={result().kind === "text" && result().folder === "different"}>
                    {(() => {
                      const sides = compareTexts(result().version_text ?? "", result().folder_text ?? "");
                      const side = (runs: Run[], mark: string) => (
                        <pre><For each={runs}>{(run) => (run.changed ? <mark class={mark}>{run.text}</mark> : run.text)}</For></pre>
                      );
                      return (
                        <div class="file-compare-columns">
                          <div><strong>This version</strong>{side(sides.before, "file-diff-only-here")}</div>
                          <div><strong>Your folder</strong>{side(sides.after, "file-diff-only-there")}</div>
                        </div>
                      );
                    })()}
                  </Show>
                  <Show when={result().review}>
                    {(review) => <OfficeChangeList review={review()} />}
                  </Show>
                  <Show when={result().review_error}>
                    <p class="file-compare-note" role="status">The changes could not be worked out just now.</p>
                  </Show>
                </div>
              )}
            </Show>
          </section>
        )}
      </Show>
    </aside>
  );
}

function ArtifactPage(props: { id: string; onBack: () => void; onChanged: () => void }) {
  const [detail, { refetch }] = createResource(() => props.id, (id) => api.libraryArtifact(id));
  const [chosen, setChosen] = createSignal<string | null>(null);
  const [section, setSection] = createSignal<"preview" | "history" | "comments">("preview");
  const [renaming, setRenaming] = createSignal(false);
  const [sharing, setSharing] = createSignal(false);
  const [editing, setEditing] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  let putBackInput!: HTMLInputElement;
  const [note, setNote] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const version = () => chosen() ?? detail()?.head ?? null;
  const [page] = createResource(
    () => {
      const found = detail();
      const at = version();
      return found && at && found.kind === "page" ? [found.id, at] as const : null;
    },
    async ([id, at]) => {
      const page = readPage(await api.libraryVersionBytes(id, at));
      return page ? { page, version: at } : null;
    },
  );
  const siblings = createMemo(() => {
    // A version in the trash or deleted is not one of the current ones.
    const history = (detail()?.history ?? []).filter((v) => !v.trashed_at && !v.erased);
    return new Set(history.filter((v) => !history.some((child) => child.parent === v.id)).map((v) => v.id));
  });

  const act = async (work: () => Promise<unknown>) => {
    setError(null);
    try {
      await work();
      await refetch();
      props.onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const download = async (found: ArtifactDetail, at: string) => {
    await act(async () => {
      const bytes = await api.libraryVersionBytes(found.id, at);
      const name = found.path.split("/").pop() || found.name;
      // A page leaves as its text form: its cards' text, then the reply.
      const page = found.kind === "page" ? readPage(bytes) : null;
      if (page) await host.saveFile(name.replace(/\.json$/, ".md"), new TextEncoder().encode(page.text), "text/markdown");
      else await host.saveFile(name, bytes, "application/octet-stream");
    });
  };

  /** Continue working and Make another (doc 82 §6): the authoring Agent's
   *  own conversation, with the artifact attached in the composer. Nothing
   *  is sent until the person sends. */
  const workOn = async (found: ArtifactDetail, mode: "continue" | "another") => {
    setLibraryOpen(false);
    await openAgentChat(found.agent);
    attachArtifact({ id: found.id, name: found.name, mode });
    if (mode === "another") {
      window.dispatchEvent(new CustomEvent("vak:edit-prompt", { detail: { text: `Make another one like ${found.name}, as a new file.` } }));
    }
  };

  /** The authoring Agent's conversation with the artifact attached and the
   *  request written, not sent. */
  const redo = async (found: ArtifactDetail) => {
    await workOn(found, "continue");
    window.dispatchEvent(new CustomEvent("vak:edit-prompt", { detail: { text: `Redo the waiting draft of ${found.name} from the copy in my folder, keeping what I changed there.` } }));
  };

  const openConversation = (found: ArtifactDetail) => {
    const made = found.history.find((v) => v.id === version()) ?? found.history[found.history.length - 1];
    if (made?.session) {
      setLibraryOpen(false);
      void activate(made.session);
    }
  };

  return (
    <Show when={detail()} fallback={<div class="dock-empty">{detail.error ? String(detail.error) : "Opening…"}</div>}>
      {(found) => (
        <div class="library-artifact">
          <header class="artifact-hero">
            <button type="button" class="artifact-back" onClick={props.onBack}><Icon name="chevron" size={13} /> Library</button>
            <div class="artifact-title">
              <span class="artifact-kind" aria-hidden="true"><Icon name={kindIcon(found().path)} size={20} /></span>
              <div class="artifact-title-text">
                <Show
                  when={renaming()}
                  fallback={<h2>{found().name}</h2>}
                >
                  <form
                    class="library-rename"
                    onSubmit={(e) => {
                      e.preventDefault();
                      const title = new FormData(e.currentTarget).get("title")?.toString() ?? "";
                      setRenaming(false);
                      void act(() => api.libraryChange(found().id, "rename", { title }));
                    }}
                  >
                    <input name="title" value={found().name} aria-label="Name" />
                    <button class="btn sm primary" type="submit">Save</button>
                    <button class="btn sm" type="button" onClick={() => setRenaming(false)}>Cancel</button>
                  </form>
                </Show>
                <p class="library-meta">
                  {kindName(found())} · {found().agent === "vak" ? "Vakyartha" : found().agent} · changed {relAgo(found().updated_at)}
                  <Show when={found().starred}> · Starred</Show>
                  <Show when={found().held}> · On hold</Show>
                  <Show when={found().archived}> · Archived</Show>
                  <Show when={technicalDetails()}> · <span class="mono">{found().path}</span></Show>
                </p>
              </div>
            </div>
            <FileStates needs={found().needs} />
            <div class="artifact-toolbar" role="toolbar" aria-label="Artifact actions">
              <button class="btn primary" onClick={() => void workOn(found(), "continue")}>Continue working</button>
              <Show when={version()}>{(at) => <button class="btn" onClick={() => void download(found(), at())}><Icon name="download" size={14} />Download</button>}</Show>
              <button class="btn" onClick={() => setSharing(true)}><Icon name="link" size={14} />Share</button>
              <Show when={found().kind === "page"}>
                <button class="btn" onClick={() => openArtifactCanvas({ kind: "page", title: found().name, artifactId: found().id })}>Open in Canvas</button>
                <PageKeeper
                  artifactId={found().id}
                  title={found().name}
                  request={(!page.error && page()?.page.items.find((item) => item.role === "user")?.fallback_text.trim()) || null}
                />
              </Show>
              <details class="workspace-more" data-menu>
                <summary class="btn" aria-label="More actions"><Icon name="more" size={14} />More</summary>
                <div class="workspace-more-menu" role="menu">
                  <button type="button" role="menuitem" onClick={(e) => { closeMenu(e); void workOn(found(), "another"); }}><Icon name="copy" />Make another</button>
                  <Show when={found().history.some((v) => v.session)}>
                    <button type="button" role="menuitem" onClick={(e) => { closeMenu(e); openConversation(found()); }}><Icon name="chat" />Open conversation</button>
                  </Show>
                  <Show when={isText(found()) && version()}>
                    <button type="button" role="menuitem" onClick={(e) => { closeMenu(e); const at = version(); if (at) void act(async () => { const bytes = await api.libraryVersionBytes(found().id, at); setChosen(at); setEditing(new TextDecoder().decode(bytes)); setSection("preview"); }); }}><Icon name="pencil" />Edit</button>
                  </Show>
                  <button type="button" role="menuitem" onClick={(e) => { closeMenu(e); putBackInput.click(); }}><Icon name="restore" />Put back a copy</button>
                  <button type="button" role="menuitem" onClick={(e) => { closeMenu(e); setRenaming(true); }}><Icon name="tag" />Rename</button>
                  <button type="button" role="menuitem" onClick={(e) => { closeMenu(e); void act(() => api.libraryChange(found().id, "star", { on: !found().starred })); }}><Icon name="pin" />{found().starred ? "Remove star" : "Star"}</button>
                  <button type="button" role="menuitem" onClick={(e) => { closeMenu(e); void act(() => api.libraryChange(found().id, "archive", { on: !found().archived })); }}><Icon name="archive" />{found().archived ? "Restore from archive" : "Archive"}</button>
                  <button type="button" role="menuitem" title={found().held ? "Let it be deleted again" : "Keep it from being deleted, by anyone or any rule"} onClick={(e) => { closeMenu(e); void act(() => api.libraryChange(found().id, "hold", { on: !found().held })); }}><Icon name="lock" />{found().held ? "Release hold" : "Hold"}</button>
                </div>
              </details>
            <input
              ref={putBackInput}
              type="file"
              hidden
              onChange={async (e) => {
                const file = e.currentTarget.files?.[0];
                e.currentTarget.value = "";
                if (!file) return;
                const bytes = new Uint8Array(await file.arrayBuffer());
                let binary = "";
                for (const byte of bytes) binary += String.fromCharCode(byte);
                await act(async () => {
                  const made = await api.libraryPersonVersion(found().id, { data: btoa(binary) });
                  setNotice(made.sibling
                    ? "Saved as your version beside the changes made since you downloaded it."
                    : "Saved as your version, and the file in the folder now matches it.");
                  setChosen(made.version);
                });
              }}
            />
            </div>
          </header>
          <Show when={found().summary}><details class="artifact-summary"><summary>About this file</summary><p>{found().summary}</p></details></Show>
          <Show when={(found().needs?.behind ?? 0) > 0}>
            <div class="file-notice" role="status">
              <Icon name="warning" size={16} />
              <div>
                <strong>{found().needs!.behind === 1 ? "A waiting draft was made before the copy in your folder." : "Some waiting drafts were made before the copy in your folder."}</strong>
                <span> Accepting one as it is would drop what changed in your folder since.</span>
                <div class="library-actions">
                  <button class="btn sm primary" onClick={() => void redo(found())}>Ask Vak to redo it from your copy</button>
                </div>
              </div>
            </div>
          </Show>
          <Show when={error()}><div class="worker-error" role="alert">{error()}</div></Show>

          <Show when={sharing()}>
            <SharePanel artifact={found()} onClose={() => setSharing(false)} />
          </Show>

          <Show when={notice()}><p class="library-note" role="status">{notice()}</p></Show>
          <div class="library-content-bar">
            <div class="library-content-tabs" role="group" aria-label="Artifact view">
              <button class="btn sm" aria-pressed={section() === "preview"} onClick={() => setSection("preview")}>Content</button>
              <button class="btn sm" aria-pressed={section() === "history"} onClick={() => setSection("history")}>History <span>{found().history.length}</span></button>
              <button class="btn sm" aria-pressed={section() === "comments"} onClick={() => setSection("comments")}>Comments <span>{found().comments?.length ?? 0}</span></button>
            </div>
            <Show when={version()}>
              <label class="library-version-picker">Viewing
                <select aria-label="Version to view" value={version() ?? ""} onChange={e => { setChosen(e.currentTarget.value); setEditing(null); }}>
                  <For each={[...found().history].reverse()}>{v => <option value={v.id}>Version {found().history.findIndex(item => item.id === v.id) + 1}{v.id === found().head ? " · Latest" : ""}{v.erased ? " · Deleted" : v.trashed_at ? " · In trash" : ""}</option>}</For>
                </select>
              </label>
            </Show>
          </div>
          <div class="library-content-section" hidden={section() !== "preview"}>
          <Show when={editing() !== null}>
            <form
              class="library-edit"
              onSubmit={(e) => {
                e.preventDefault();
                const content = editing() ?? "";
                const parent = version() ?? undefined;
                setEditing(null);
                void act(async () => {
                  const made = await api.libraryPersonVersion(found().id, { text: content, parent });
                  setNotice(made.sibling
                    ? "Saved as your version beside the other changes; choose between them when you are ready."
                    : "Saved as your version.");
                  setChosen(made.version);
                });
              }}
            >
              <textarea value={editing() ?? ""} onInput={(e) => setEditing(e.currentTarget.value)} aria-label="Edit" />
              <div class="library-actions">
                <button class="btn sm primary" type="submit">Save</button>
                <button class="btn sm" type="button" onClick={() => setEditing(null)}>Cancel</button>
              </div>
            </form>
          </Show>
          <Show when={editing() === null}>
            <Show when={found().kind === "page"} fallback={
              <Show when={version()} fallback={<p class="library-note">No saved version to show yet.</p>}>
                {at => <Show keyed when={`${found().id}:${at()}`}>
                  <LibraryPreview artifact={found()} version={at()} />
                </Show>}
              </Show>
            }>
              <div class="library-preview library-page-view">
                <Show when={!page.error} fallback={<p role="alert">This page could not be opened. Try reopening it or download this version.</p>}>
                  <Show when={page()} fallback={<p class="library-note">{page.loading ? "Opening page…" : "No page to show in this version."}</p>}>
                    {shown => <PageView page={shown().page} versionId={shown().version} />}
                  </Show>
                </Show>
              </div>
            </Show>
          </Show>
          </div>

          <Show when={section() === "history"}>

          <History
            found={found()}
            chosen={version()}
            onChoose={id => { setChosen(id); setEditing(null); setSection("preview"); }}
            act={act}
            onOpenConversation={(session) => { setLibraryOpen(false); void activate(session); }}
          />
          <Show when={siblings().size > 1}>
            <p class="library-note">Edits were made from the same version at once; each is kept until you choose.</p>
          </Show>

          </Show>

          <Show when={section() === "comments"}>
          <div class="library-comments-block">
          <h3>Comments</h3>
          <Show when={(found().comments ?? []).length > 0} fallback={<p class="library-note">No comments yet.</p>}>
            <ul class="library-comments">
              <For each={found().comments ?? []}>
                {(comment) => {
                  const number = found().history.findIndex((v) => v.id === comment.version) + 1;
                  return (
                    <li>
                      <strong>{comment.author_name}</strong>
                      <span class="library-note"> on version {number} · {relAgo(comment.at)}</span>
                      <p>{comment.text}</p>
                    </li>
                  );
                }}
              </For>
            </ul>
          </Show>
          <Show when={version()}>
            {(at) => (
              <form
                class="library-share-form"
                onSubmit={(e) => {
                  e.preventDefault();
                  const text = note().trim();
                  if (!text) return;
                  setNote("");
                  void act(() => api.commentOnVersion(found().id, at(), text));
                }}
              >
                <input placeholder="Add a comment on this version" value={note()} onInput={(e) => setNote(e.currentTarget.value)} aria-label="Comment" />
                <button class="btn sm" type="submit" disabled={!note().trim()}>Comment</button>
              </form>
            )}
          </Show>
          </div>
          </Show>
        </div>
      )}
    </Show>
  );
}

/** One learned ability as a card; it opens the ability's sheet, where
 *  everything about it is decided (docs/design/93 §9.1-§9.3). */
function AbilityCard(props: { ability: api.Ability; agent?: string; agentName?: string }) {
  const a = () => props.ability;
  return (
    <button type="button" class="library-card library-ability" onClick={() => openAbilitySheet(a().name, props.agent)}>
      <span class="library-card-top">
        <span class="artifact-kind sm" aria-hidden="true"><Icon name="spark" size={16} /></span>
        <span class="library-card-kind">Ability</span>
        <Pill mark={ABILITY_STATE[a().state].mark}>{ABILITY_STATE[a().state].word}</Pill>
      </span>
      <strong>{a().name}</strong>
      <Show when={a().description}><span class="library-card-summary">{a().description}</span></Show>
      <Show when={a().why}><span class="library-card-summary">{a().state === "waiting" ? "Waiting" : "A new version waits"} because {a().why}.</span></Show>
      <Show when={a().reaches.length > 0}>
        <ul class="library-ability-reaches" aria-label="What it can do">
          <For each={a().reaches}>{(line) => <li>{line}</li>}</For>
        </ul>
      </Show>
      <span class="library-card-meta">
        {[props.agentName ? `Learned by ${props.agentName}` : "", a().versions > 1 ? `${a().versions} versions` : ""].filter(Boolean).join(" · ")}
      </span>
    </button>
  );
}

const ABILITY_FILTERS: { value: "any" | api.Ability["state"]; label: string }[] = [
  { value: "any", label: "Every state" },
  { value: "working", label: "Working" },
  { value: "failing", label: "Failing" },
  { value: "waiting", label: "Waiting for you" },
  { value: "offered", label: "Ready for every Agent" },
  { value: "turned_off", label: "Turned off" },
];

/** What the Agents learned (docs/design/93 §9.3): one card per ability,
 *  found by name, Agent and state. */
function AbilitiesView(props: { abilities: api.Ability[] | undefined; loading: boolean; error: unknown; agent: string; onAgent: (agent: string) => void; agents: api.Agent[] }) {
  const [query, setQuery] = createSignal("");
  const [state, setState] = createSignal<"any" | api.Ability["state"]>("any");
  const shown = createMemo(() => {
    const words = query().trim().toLowerCase();
    return (props.abilities ?? []).filter((ability) =>
      (state() === "any" || ability.state === state())
      && (!words || `${ability.name} ${ability.description}`.toLowerCase().includes(words)));
  });
  const agentName = () => props.agents.find((agent) => agent.id === props.agent)?.name;
  return (
    <>
      <div class="library-filters" role="group" aria-label="Ability filters">
        <input class="search-input" placeholder="Find by name…" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} aria-label="Find by name" />
        <Show when={props.agents.length > 1}>
          <select value={props.agent} onChange={(e) => props.onAgent(e.currentTarget.value)} aria-label="Agent">
            <option value="">This Agent</option>
            <For each={props.agents}>{(agent) => <option value={agent.id}>{agent.name}</option>}</For>
          </select>
        </Show>
        <select value={state()} onChange={(e) => setState(e.currentTarget.value as "any" | api.Ability["state"])} aria-label="State">
          <For each={ABILITY_FILTERS}>{(entry) => <option value={entry.value}>{entry.label}</option>}</For>
        </select>
      </div>
      <Show when={!props.error} fallback={<div class="worker-error">{String(props.error)}</div>}>
        <Show
          when={shown().length > 0}
          fallback={<div class="dock-empty">{props.loading ? "Loading…" : (props.abilities ?? []).length > 0 ? "No ability matches." : "Nothing learned yet. When an Agent makes itself an ability, it appears here."}</div>}
        >
          <div class="library-list">
            <For each={shown()}>{(ability) => <AbilityCard ability={ability} agent={props.agent || undefined} agentName={agentName()} />}</For>
          </div>
        </Show>
      </Show>
    </>
  );
}

export default function LibraryPage() {
  const [all, { refetch }] = createResource(() => api.library().then((res) => res.artifacts));
  const [open, setOpen] = createSignal<string | null>(libraryFocus());
  setLibraryFocus(null);
  const [kind, setKind] = createSignal("all");
  const [changed, setChanged] = createSignal("any");
  const [layout, setLayout] = createSignal<"list" | "grid">("list");
  /** Which files to show by their state (doc 90 §5). */
  const [state, setState] = createSignal<"all" | "needs" | "changed" | "starred" | "archived" | "abilities">(libraryAbilities() ? "abilities" : "all");
  setLibraryAbilities(false);
  const starred = () => state() === "starred";
  const archived = () => state() === "archived";
  const needsYou = (artifact: ArtifactSummary) => !!artifact.needs && (artifact.needs.waiting > 0 || artifact.needs.missing);
  const [query, setQuery] = createSignal("");
  // The learned abilities of the chosen Agent ("" is the one this
  // conversation is with); refreshed when an ability's sheet closes.
  const [abilityAgent, setAbilityAgent] = createSignal("");
  const [abilities, { refetch: refetchAbilities }] = createResource(abilityAgent, (agent) => api.listAbilities(agent || undefined).then((res) => res.abilities));
  const [agents] = createResource(() => api.listAgents().then((res) => res.agents).catch(() => [] as api.Agent[]));
  createEffect(on(abilitySheetOpen, (now, before) => { if (before && !now) void refetchAbilities(); }, { defer: true }));
  const abilityNeedsYou = (ability: api.Ability) => ability.state === "waiting" || ability.state === "offered" || !!ability.proposal;
  const waitingAbilities = () => (abilities() ?? []).filter(abilityNeedsYou);

  const shown = createMemo(() => {
    const days = CHANGED.find((entry) => entry.value === changed())?.days ?? Infinity;
    const since = Date.now() - days * 86_400_000;
    const words = query().trim().toLowerCase();
    return (all() ?? []).filter((artifact) =>
      (kind() === "all" || artifact.kind === kind())
      && (!starred() || artifact.starred)
      && artifact.archived === archived()
      && (state() !== "needs" || needsYou(artifact))
      && (state() !== "changed" || !!artifact.needs?.changed_outside)
      && new Date(artifact.updated_at).getTime() >= since
      && (!words || `${artifact.name} ${artifact.summary ?? ""}`.toLowerCase().includes(words)));
  });

  return (
    <div class="library-page">
      <Show
        when={open()}
        fallback={
          <>
            <header class="library-top">
              <button type="button" class="artifact-back" onClick={() => setLibraryOpen(false)}><Icon name="chevron" size={13} /> Back to chat</button>
              <h2>Library</h2>
              <p class="library-meta">Everything Vakyartha made for you, with every version.</p>
            </header>
            <div class="library-states" role="tablist" aria-label="Show">
              <For each={[
                { value: "all", label: "All" },
                { value: "needs", label: "Needs you", count: (all() ?? []).filter(needsYou).length + waitingAbilities().length },
                { value: "changed", label: "Changed outside Vak", count: (all() ?? []).filter((a) => a.needs?.changed_outside).length },
                { value: "starred", label: "Starred" },
                { value: "archived", label: "Archived" },
                { value: "abilities", label: "Learned abilities" },
              ] as const}>
                {(chip) => (
                  <button type="button" role="tab" class="library-chip" aria-selected={state() === chip.value} onClick={() => setState(chip.value)}>
                    {chip.label}
                    <Show when={"count" in chip && chip.count > 0}><span class="library-chip-count">{"count" in chip ? chip.count : 0}</span></Show>
                  </button>
                )}
              </For>
            </div>
            <Show when={state() === "abilities"}>
              <AbilitiesView abilities={abilities()} loading={abilities.loading} error={abilities.error} agent={abilityAgent()} onAgent={setAbilityAgent} agents={agents() ?? []} />
            </Show>
            <Show when={state() === "needs" && waitingAbilities().length > 0}>
              <h3 class="library-section-title">Abilities</h3>
              <div class="library-list">
                <For each={waitingAbilities()}>{(ability) => <AbilityCard ability={ability} agent={abilityAgent() || undefined} />}</For>
              </div>
              <h3 class="library-section-title">Files</h3>
            </Show>
            <Show when={state() !== "abilities"}>
            <div class="library-filters" role="group" aria-label="Library filters">
              <input class="search-input" placeholder="Find by name…" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} aria-label="Find by name" />
              <select value={kind()} onChange={(e) => setKind(e.currentTarget.value)} aria-label="Kind">
                <option value="all">Every kind</option>
                <For each={Object.entries(KIND)}>{([value, label]) => <option value={value}>{label}</option>}</For>
              </select>
              <select value={changed()} onChange={(e) => setChanged(e.currentTarget.value)} aria-label="Changed">
                <For each={CHANGED}>{(entry) => <option value={entry.value}>{entry.label}</option>}</For>
              </select>
              <div class="library-layout" role="group" aria-label="Library layout">
                <button type="button" aria-pressed={layout() === "list"} onClick={() => setLayout("list")}>List</button>
                <button type="button" aria-pressed={layout() === "grid"} onClick={() => setLayout("grid")}>Grid</button>
              </div>
            </div>
            <Show when={!all.error} fallback={<div class="worker-error">{String(all.error)}</div>}>
              <Show
                when={shown().length > 0}
                fallback={<div class="dock-empty">{all.loading ? "Loading…" : "Nothing here yet. When Vakyartha makes something for you, it appears here."}</div>}
              >
                <div class="library-list" classList={{ "is-list": layout() === "list" }}>
                  <For each={shown()}>
                    {(artifact) => (
                      <button type="button" class="library-card" onClick={() => setOpen(artifact.id)}>
                        <span class="library-card-top">
                          <span class="artifact-kind sm" aria-hidden="true"><Icon name={kindIcon(artifact.path)} size={16} /></span>
                          <span class="library-card-kind">{kindName(artifact)}</span>
                          <Show when={artifact.starred}><span class="library-card-star" aria-label="Starred">★</span></Show>
                        </span>
                        <strong>{artifact.name}</strong>
                        <Show when={artifact.summary}><span class="library-card-summary">{artifact.summary}</span></Show>
                        <FileStates needs={artifact.needs} />
                        <span class="library-card-meta">
                          {artifact.agent === "vak" ? "Vakyartha" : artifact.agent} · changed {relAgo(artifact.updated_at)}
                          <Show when={artifact.versions > 1}> · {artifact.versions} versions</Show>
                        </span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </Show>
            </Show>
          </>
        }
      >
        {(id) => <ArtifactPage id={id()} onBack={() => setOpen(null)} onChanged={() => void refetch()} />}
      </Show>
    </div>
  );
}
