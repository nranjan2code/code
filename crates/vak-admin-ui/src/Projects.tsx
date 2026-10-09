/// Configure › Projects (docs/design/74 A13). A project is what the model
/// calls a space: the thing a conversation's files and history belong to,
/// bound to a folder on each machine (docs/design/75 §7). It is named by an
/// id, never by its folder, so a project with no folder on this machine is
/// listed and says so.

import { createResource, createSignal, For, Show } from "solid-js";
import { api, type Project } from "./api";
import { pushToast } from "./store";

function lastOpened(seconds: number | null): string {
  return seconds ? ` · last opened ${new Date(seconds * 1000).toLocaleString()}` : "";
}

export default function Projects() {
  const [projects, { refetch }] = createResource(() => api.projects());
  const [editing, setEditing] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  const act = async (work: () => Promise<unknown>, done: string) => {
    if (busy()) return;
    setBusy(true);
    try {
      await work();
      await refetch();
      pushToast("info", done);
    } catch (error) {
      pushToast("alert", `${error}`);
    } finally {
      setBusy(false);
    }
  };

  // Erasing what Vakyartha keeps for a project (plan M7b-d): the counts
  // first, then the project's name typed. The server checks both again.
  const erase = async (project: Project) => {
    if (busy()) return;
    try {
      const { preview, confirm } = await api.projectErasurePreview(project.id);
      if (preview.held) {
        pushToast("alert", "Something kept for this project is on hold. Release the hold first.");
        return;
      }
      const typed = window.prompt(
        `Erase everything Vakyartha keeps for “${confirm}”? ${preview.conversations} ${preview.conversations === 1 ? "conversation" : "conversations"}, ${preview.documents} things remembered, ${preview.artifacts} files in the Library, ${preview.automations} automations and ${preview.workspace_files} working files are erased for good. The project's own folder is not touched. Type the project's name to go on.`,
      );
      if (typed === null) return;
      if (typed.trim() !== confirm.trim()) {
        pushToast("alert", "Not erased: the name did not match.");
        return;
      }
      await act(() => api.eraseProject(project.id, preview.digest, typed.trim()), "Everything kept for the project was erased. The receipt is under Conversations.");
    } catch (error) {
      pushToast("alert", `${error instanceof Error ? error.message : error}`);
    }
  };

  const rename = (project: Project) =>
    act(async () => {
      await api.patchProject(project.id, { name: draft().trim() });
      setEditing(null);
    }, `Project renamed to “${draft().trim()}”`);

  return (
    <div class="page">
      <header class="page-header">
        <div>
          <h1>Projects</h1>
          <p class="dim">Each project keeps its conversations, memory and settings. Hiding one removes it from lists; nothing it holds is touched. Erasing its data removes what Vakyartha keeps for it, never the folder.</p>
        </div>
      </header>
      <section class="panel project-list">
        <Show when={!projects.error} fallback={<p class="dim">Could not load projects: {`${projects.error}`}</p>}>
          <Show when={(projects()?.projects ?? []).length > 0} fallback={<p class="dim">No projects yet. Open a folder in Vakyartha and it appears here.</p>}>
            <For each={projects()?.projects ?? []}>
              {(project) => (
                <div class="project-row">
                  <div>
                    <strong>{project.name ?? "Unnamed project"}</strong>
                    <span class="dim">
                      {project.folder_here ? project.folder : "No folder on this machine"}
                      {" · "}
                      {project.trusted ? "Trusted" : "Not trusted"}
                      {lastOpened(project.last_opened)}
                      {project.hidden ? " · Hidden" : ""}
                    </span>
                  </div>
                  <Show
                    when={editing() !== project.id}
                    fallback={
                      <div class="project-edit">
                        <input value={draft()} onInput={(e) => setDraft(e.currentTarget.value)} />
                        <button class="small" disabled={!draft().trim() || busy()} onClick={() => void rename(project)}>Save</button>
                        <button class="ghost small" onClick={() => setEditing(null)}>Cancel</button>
                      </div>
                    }
                  >
                    <div class="project-edit">
                      <button class="ghost small" onClick={() => { setEditing(project.id); setDraft(project.name ?? ""); }}>Rename</button>
                      <button
                        class="ghost small"
                        disabled={busy()}
                        onClick={() => void act(() => api.patchProject(project.id, { hidden: !project.hidden }), project.hidden ? "Project shown" : "Project hidden")}
                      >
                        {project.hidden ? "Show" : "Hide"}
                      </button>
                      <Show when={!project.folder_here}>
                        <button
                          class="ghost small"
                          disabled={busy()}
                          onClick={() => {
                            const folder = window.prompt(`Where is “${project.name ?? "this project"}” on this machine? Give its folder. It was made on your other machine, where its folder is somewhere else.`);
                            if (folder && folder.trim()) void act(() => api.patchProject(project.id, { folder: folder.trim() }), "The project's folder on this machine is set.");
                          }}
                        >
                          Its folder here…
                        </button>
                      </Show>
                      <button class="ghost small danger" disabled={busy()} onClick={() => void erase(project)}>Erase its data…</button>
                    </div>
                  </Show>
                </div>
              )}
            </For>
          </Show>
        </Show>
      </section>
    </div>
  );
}
