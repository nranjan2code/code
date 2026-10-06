import { createMemo, createResource, createSignal, For, Show } from "solid-js";
import * as api from "../api";
import type { ArtifactDetail, ArtifactSummary, ArtifactVersion } from "../api";
import { host } from "../host";
import { setLibraryOpen, technicalDetails } from "../store";
import { relAgo } from "../time";
import { activate } from "../App";
import Icon from "./Icon";

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

/** Who made a version, read from its record (doc 82 §5). */
function maker(version: ArtifactVersion): string {
  return version.from === "person" ? "You" : "Vakyartha";
}

/** Files small enough and plain enough to show as text. */
function isText(artifact: ArtifactSummary): boolean {
  return artifact.kind === "file"
    && (artifact.size ?? 0) <= 256 * 1024
    && /\.(md|txt|csv|tsv|json|ya?ml|toml|html?|css|js|ts|tsx|py|rs|go|sh|sql|xml|ini)$/i.test(artifact.path);
}

function ArtifactPage(props: { id: string; onBack: () => void; onChanged: () => void }) {
  const [detail, { refetch }] = createResource(() => props.id, (id) => api.libraryArtifact(id));
  const [chosen, setChosen] = createSignal<string | null>(null);
  const [renaming, setRenaming] = createSignal(false);
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
    const history = detail()?.history ?? [];
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
            <Show when={version()}>{(at) => <button class="btn sm primary" onClick={() => void download(found(), at())}>Download</button>}</Show>
            <Show when={found().history.some((v) => v.session)}>
              <button class="btn sm" onClick={() => openConversation(found())}>Open conversation</button>
            </Show>
            <button class="btn sm" onClick={() => void act(() => api.libraryChange(found().id, "star", { on: !found().starred }))}>
              {found().starred ? "Unstar" : "Star"}
            </button>
            <button class="btn sm" onClick={() => setRenaming(true)}>Rename</button>
            <button class="btn sm" onClick={() => void act(() => api.libraryChange(found().id, "archive", { on: !found().archived }))}>
              {found().archived ? "Restore" : "Archive"}
            </button>
          </div>

          <Show when={isText(found())} fallback={<div class="library-preview dock-empty">Download to open this {KIND[found().kind].toLowerCase()}.</div>}>
            <pre class="library-preview">{text() ?? "Loading…"}</pre>
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
                      <Show when={v.promoted}><span class="badge">Accepted</span></Show>
                      <Show when={v.saved}><span class="badge">Kept</span></Show>
                    </button>
                    <Show when={!v.saved}>
                      <button class="btn sm" onClick={() => void act(() => api.librarySave(found().id, v.id))}>Keep</button>
                    </Show>
                  </li>
                );
              }}
            </For>
          </ol>
        </div>
      )}
    </Show>
  );
}

/** Every deliverable an Agent made, across conversations (docs/design/82-library.md §4). */
export default function LibraryPage() {
  const [all, { refetch }] = createResource(() => api.library().then((res) => res.artifacts));
  const [open, setOpen] = createSignal<string | null>(null);
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
