import { createMemo, createResource, createSignal, For, Show } from "solid-js";
import * as api from "../api";
import type { ArtifactDetail, ArtifactShare, ArtifactSummary, ArtifactVersion, ShareRole } from "../api";
import { host } from "../host";
import { libraryFocus, setLibraryFocus, setLibraryOpen, technicalDetails } from "../store";
import { relAgo } from "../time";
import { activate, openAgentChat } from "../App";
import { attachArtifact } from "../attachFiles";
import Icon from "./Icon";
import OfficeView from "./OfficeView";

/** Plain words for an artifact's kind (doc 75 §7). */
const KIND: Record<ArtifactSummary["kind"], string> = {
  file: "File",
  document: "Document",
  changeset: "Code changes",
  card: "Saved card",
};

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
  return version.from === "person" ? "You" : "Vakyartha";
}

/** Files small enough and plain enough to show as text. */
function isText(artifact: ArtifactSummary): boolean {
  return artifact.kind === "file"
    && (artifact.size ?? 0) <= 256 * 1024
    && /\.(md|txt|csv|tsv|json|ya?ml|toml|html?|css|js|ts|tsx|py|rs|go|sh|sql|xml|ini)$/i.test(artifact.path);
}

/** An Office file or PDF, which the Library previews as a document. */
function isDocument(artifact: ArtifactSummary): boolean {
  return /\.(docx|docm|dotx|xlsx|xlsm|xltx|pptx|pptm|potx|vsdx|pdf)$/i.test(artifact.path);
}

function ArtifactPage(props: { id: string; onBack: () => void; onChanged: () => void }) {
  const [detail, { refetch }] = createResource(() => props.id, (id) => api.libraryArtifact(id));
  const [chosen, setChosen] = createSignal<string | null>(null);
  const [renaming, setRenaming] = createSignal(false);
  const [sharing, setSharing] = createSignal(false);
  const [editing, setEditing] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  let putBackInput!: HTMLInputElement;
  const [note, setNote] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const version = () => chosen() ?? detail()?.head ?? null;
  const [text] = createResource(
    () => {
      const found = detail();
      const at = version();
      return found && at && isText(found) ? [found.id, at] as const : null;
    },
    async ([id, at]) => new TextDecoder().decode(await api.libraryVersionBytes(id, at)),
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
      await host.saveFile(found.path.split("/").pop() || found.name, bytes, "application/octet-stream");
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
          <div class="library-head">
            <button type="button" class="btn sm library-back" onClick={props.onBack}><Icon name="chevron" size={13} /> Library</button>
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
              </form>
            </Show>
          </div>
          <p class="library-meta">
            {KIND[found().kind]} · {found().agent === "vak" ? "Vakyartha" : found().agent} · changed {relAgo(found().updated_at)}
            <Show when={technicalDetails()}> · <span class="mono">{found().path}</span></Show>
          </p>
          <Show when={found().summary}><p>{found().summary}</p></Show>
          <Show when={error()}><div class="worker-error" role="alert">{error()}</div></Show>
          <div class="library-actions">
            <button class="btn sm primary" onClick={() => void workOn(found(), "continue")}>Continue working</button>
            <button class="btn sm" onClick={() => void workOn(found(), "another")}>Make another</button>
            <Show when={version()}>{(at) => <button class="btn sm" onClick={() => void download(found(), at())}>Download</button>}</Show>
            <Show when={found().history.some((v) => v.session)}>
              <button class="btn sm" onClick={() => openConversation(found())}>Open conversation</button>
            </Show>
            <button class="btn sm" onClick={() => void act(() => api.libraryChange(found().id, "star", { on: !found().starred }))}>
              {found().starred ? "Unstar" : "Star"}
            </button>
            <Show when={isText(found()) && text() !== undefined}>
              <button class="btn sm" onClick={() => setEditing(text() ?? "")}>Edit</button>
            </Show>
            <button class="btn sm" onClick={() => putBackInput.click()}>Put back</button>
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
            <button class="btn sm" onClick={() => setSharing(true)}>Share</button>
            <button class="btn sm" onClick={() => setRenaming(true)}>Rename</button>
            <button class="btn sm" onClick={() => void act(() => api.libraryChange(found().id, "archive", { on: !found().archived }))}>
              {found().archived ? "Restore" : "Archive"}
            </button>
            <button class="btn sm has-tooltip" data-tooltip={found().held ? "Let it be deleted again" : "Keep it from being deleted, by anyone or any rule"} onClick={() => void act(() => api.libraryChange(found().id, "hold", { on: !found().held }))}>
              {found().held ? "Release hold" : "Hold"}
            </button>
          </div>

          <Show when={sharing()}>
            <SharePanel artifact={found()} onClose={() => setSharing(false)} />
          </Show>

          <Show when={notice()}><p class="library-note" role="status">{notice()}</p></Show>
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
          <Show
            when={isText(found())}
            fallback={
              <Show
                when={isDocument(found()) && version()}
                fallback={<div class="library-preview dock-empty">Download to open this {KIND[found().kind].toLowerCase()}.</div>}
              >
                {(at) => (
                  <div class="library-preview library-document">
                    <OfficeView source={{ path: found().path, version: { artifact: found().id, version: at() } }} fileName={found().path.split("/").pop() ?? found().path} />
                  </div>
                )}
              </Show>
            }
          >
            <pre class="library-preview">{text() ?? "Loading…"}</pre>
          </Show>
          </Show>

          <h3>Versions</h3>
          <Show when={siblings().size > 1}>
            <p class="library-note">Edits were made from the same version at once; each is kept below until you choose.</p>
          </Show>
          <ol class="library-versions">
            <For each={[...found().history].reverse()}>
              {(v) => {
                const number = found().history.findIndex((other) => other.id === v.id) + 1;
                return (
                  <li classList={{ chosen: version() === v.id }}>
                    <button type="button" class="library-version" onClick={() => setChosen(v.id)}>
                      <strong>Version {number}</strong>
                      <span>{maker(v)} · {relAgo(v.at)}</span>
                      <Show when={siblings().has(v.id) && siblings().size > 1}><span class="badge">Side by side</span></Show>
                      <Show when={v.promoted}><span class="badge">Accepted</span></Show><Show when={v.removed}><span class="badge">File removed</span></Show>
                      <Show when={v.saved}><span class="badge">Kept</span></Show>
                      <Show when={v.erased}><span class="badge">Deleted for good</span></Show>
                      <Show when={v.trashed_at && !v.erased}><span class="badge">In the trash</span></Show>
                      <Show when={v.draft_until}>{(until) => <span class="library-fade">{fadesIn(until())}</span>}</Show>
                    </button>
                    <Show when={v.trashed_at && !v.erased}>
                      <button class="btn sm" onClick={() => void act(() => api.setDraftTrashed(found().id, v.id, false))}>Restore</button>
                    </Show>
                    <Show when={!v.saved && !v.erased && !v.trashed_at}>
                      <button class="btn sm" onClick={() => void act(() => api.librarySave(found().id, v.id))}>Keep</button>
                    </Show>
                    <Show when={v.draft_until}>
                      <button class="icon-button subtle danger has-tooltip" data-tooltip="Move to trash" aria-label={`Move version ${number} to the trash`} onClick={() => void act(() => api.setDraftTrashed(found().id, v.id, true))}><Icon name="trash" size={14} /></button>
                    </Show>
                  </li>
                );
              }}
            </For>
          </ol>

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
      )}
    </Show>
  );
}

/** Every deliverable an Agent made, across conversations (docs/design/82-library.md §4). */
export default function LibraryPage() {
  const [all, { refetch }] = createResource(() => api.library().then((res) => res.artifacts));
  const [open, setOpen] = createSignal<string | null>(libraryFocus());
  setLibraryFocus(null);
  const [kind, setKind] = createSignal("all");
  const [changed, setChanged] = createSignal("any");
  const [starred, setStarred] = createSignal(false);
  const [archived, setArchived] = createSignal(false);
  const [query, setQuery] = createSignal("");

  const shown = createMemo(() => {
    const days = CHANGED.find((entry) => entry.value === changed())?.days ?? Infinity;
    const since = Date.now() - days * 86_400_000;
    const words = query().trim().toLowerCase();
    return (all() ?? []).filter((artifact) =>
      (kind() === "all" || artifact.kind === kind())
      && (!starred() || artifact.starred)
      && artifact.archived === archived()
      && new Date(artifact.updated_at).getTime() >= since
      && (!words || `${artifact.name} ${artifact.summary ?? ""}`.toLowerCase().includes(words)));
  });

  return (
    <div class="library-page">
      <Show
        when={open()}
        fallback={
          <>
            <div class="library-head">
              <button type="button" class="btn sm" onClick={() => setLibraryOpen(false)}>
                <Icon name="chat" size={14} /> Back to chat
              </button>
              <h2>Library</h2>
            </div>
            <div class="library-filters" role="group" aria-label="Library filters">
              <input class="search-input" placeholder="Find by name…" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} aria-label="Find by name" />
              <select value={kind()} onChange={(e) => setKind(e.currentTarget.value)} aria-label="Kind">
                <option value="all">Everything</option>
                <For each={Object.entries(KIND)}>{([value, label]) => <option value={value}>{label}</option>}</For>
              </select>
              <select value={changed()} onChange={(e) => setChanged(e.currentTarget.value)} aria-label="Changed">
                <For each={CHANGED}>{(entry) => <option value={entry.value}>{entry.label}</option>}</For>
              </select>
              <label class="library-toggle"><input type="checkbox" checked={starred()} onChange={(e) => setStarred(e.currentTarget.checked)} /> Starred</label>
              <label class="library-toggle"><input type="checkbox" checked={archived()} onChange={(e) => setArchived(e.currentTarget.checked)} /> Archived</label>
            </div>
            <Show when={!all.error} fallback={<div class="worker-error">{String(all.error)}</div>}>
              <Show
                when={shown().length > 0}
                fallback={<div class="dock-empty">{all.loading ? "Loading…" : "Nothing here yet. When Vakyartha makes something for you, it appears here."}</div>}
              >
                <div class="library-list">
                  <For each={shown()}>
                    {(artifact) => (
                      <button type="button" class="library-card" onClick={() => setOpen(artifact.id)}>
                        <span class="library-card-kind">{KIND[artifact.kind]}</span>
                        <strong>{artifact.starred ? "★ " : ""}{artifact.name}</strong>
                        <Show when={artifact.summary}><span class="library-card-summary">{artifact.summary}</span></Show>
                        <span class="library-card-meta">
                          {artifact.agent === "vak" ? "Vakyartha" : artifact.agent} · changed {relAgo(artifact.updated_at)}
                          <Show when={artifact.versions > 1}> · {artifact.versions} versions</Show>
                          <Show when={artifact.siblings > 1}> · needs a choice</Show>
                        </span>
                      </button>
                    )}
                  </For>
                </div>
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
