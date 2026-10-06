import { createSignal, For, Show } from "solid-js";
import { relAgo } from "../time";

interface SharedVersion {
  id: string;
  number: number;
  by: string;
  at: string;
  size: number;
}

interface SharedView {
  name: string;
  kind: string;
  file_name?: string | null;
  role: "viewer" | "commenter" | "editor";
  you: string;
  versions: SharedVersion[];
  comments: { version: string; name: string; text: string; at: string }[];
}

/**
 * What a person a Library artifact was shared with sees (plan M8.4a,
 * docs/design/82-library.md §8): the artifact, the versions the owner
 * chose to show, and, for a commenter, a place to comment. Never the
 * conversations that made it.
 */
export default function SharedArtifact() {
  const [code, setCode] = createSignal("");
  const [token, setToken] = createSignal<string | null>(null);
  const [view, setView] = createSignal<SharedView | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [note, setNote] = createSignal("");

  const call = async (path: string, init?: RequestInit) => {
    const response = await fetch(path, {
      ...init,
      credentials: "omit",
      cache: "no-store",
      referrerPolicy: "no-referrer",
      headers: { Authorization: `Bearer ${token()}`, "Content-Type": "application/json" },
    });
    if (response.status === 401) throw new Error("This link has stopped working or the code is wrong.");
    if (!response.ok) throw new Error(`The request was refused (${response.status}).`);
    return response;
  };

  const load = async () => {
    setError(null);
    try {
      setView(await (await call("/shared/artifact")).json() as SharedView);
    } catch (e) {
      setView(null);
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const download = async (version: SharedVersion) => {
    try {
      const blob = await (await call(`/shared/artifact/versions/${encodeURIComponent(version.id)}`)).blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = view()?.file_name || view()?.name || "file";
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const comment = async (version: string) => {
    const text = note().trim();
    if (!text) return;
    try {
      await call("/shared/artifact/comments", { method: "POST", body: JSON.stringify({ version, text }) });
      setNote("");
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <main class="library-page shared-artifact">
      <Show
        when={view()}
        fallback={
          <form
            class="library-share-form"
            onSubmit={(e) => {
              e.preventDefault();
              setToken(code().trim());
              void load();
            }}
          >
            <h2>Open a shared item</h2>
            <input placeholder="Paste the code you were sent" value={code()} onInput={(e) => setCode(e.currentTarget.value)} aria-label="Code" />
            <button class="btn sm primary" type="submit" disabled={!code().trim()}>Open</button>
            <Show when={error()}><div class="worker-error" role="alert">{error()}</div></Show>
          </form>
        }
      >
        {(shared) => {
          const current = () => shared().versions[shared().versions.length - 1];
          return (
            <>
              <div class="library-head"><h2>{shared().name}</h2></div>
              <p class="library-meta">Shared with {shared().you}</p>
              <Show when={error()}><div class="worker-error" role="alert">{error()}</div></Show>
              <h3>Versions</h3>
              <ol class="library-versions">
                <For each={[...shared().versions].reverse()}>
                  {(version) => (
                    <li>
                      <span class="library-version">
                        <strong>Version {version.number}</strong>
                        <span>{version.by} · {relAgo(version.at)}</span>
                      </span>
                      <button class="btn sm" onClick={() => void download(version)}>Download</button>
                    </li>
                  )}
                </For>
              </ol>
              <h3>Comments</h3>
              <Show when={shared().comments.length > 0} fallback={<p class="library-note">No comments yet.</p>}>
                <ul class="library-comments">
                  <For each={shared().comments}>
                    {(c) => <li><strong>{c.name}</strong><span class="library-note"> · {relAgo(c.at)}</span><p>{c.text}</p></li>}
                  </For>
                </ul>
              </Show>
              <Show when={shared().role !== "viewer" && current()}>
                {(version) => (
                  <form class="library-share-form" onSubmit={(e) => { e.preventDefault(); void comment(version().id); }}>
                    <input placeholder="Add a comment" value={note()} onInput={(e) => setNote(e.currentTarget.value)} aria-label="Comment" />
                    <button class="btn sm" type="submit" disabled={!note().trim()}>Comment</button>
                  </form>
                )}
              </Show>
            </>
          );
        }}
      </Show>
    </main>
  );
}
