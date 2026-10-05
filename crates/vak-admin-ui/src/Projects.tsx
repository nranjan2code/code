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
          <p class="dim">Each project keeps its conversations, memory and settings. Hiding one removes it from lists; nothing it holds is touched.</p>
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
