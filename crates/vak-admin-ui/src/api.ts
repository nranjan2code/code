import type { ConfigResponse, Diagnostics, Event, Inbox, Memory, Project, Run, Session, Task } from "./types";
let auth: Promise<void> | undefined;
function ensureAuth(): Promise<void> {
  if (auth) return auth;
  const token = location.hash.startsWith("#token=") ? decodeURIComponent(location.hash.slice(7)) : "";
  auth = token
    ? fetch("/auth/login", { method: "POST", credentials: "include", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ token }) }).then(async (response) => {
      if (!response.ok) throw new Error("Runtime authentication failed; open a fresh Admin Console from the tray or run `vakcoder admin`.");
      history.replaceState(null, "", location.pathname);
    })
    : Promise.resolve();
  return auth;
}
async function request<T>(path: string, init?: RequestInit): Promise<T> {
  await ensureAuth();
  const response = await fetch(path, { credentials: "include", ...init });
  const body = await response.text();
  if (!response.ok) {
    if (response.status === 401) {
      throw new Error("Runtime authentication expired; open a fresh Admin Console from the tray or run `vakcoder admin`.");
    }
    throw new Error(body || `HTTP ${response.status}`);
  }
  return body ? JSON.parse(body) as T : undefined as T;
}
export const api = {
  projects: () => request<{items: Project[]}>("/projects"),
  sessions: (projectId?: string) => request<{items: Session[]}>(`/sessions${projectId ? `?project_id=${encodeURIComponent(projectId)}` : ""}`),
  runs: (sessionId?: string) => request<{items: Run[]}>(`/runs${sessionId ? `?session_id=${encodeURIComponent(sessionId)}` : ""}`),
  tasks: () => request<{items: Task[]}>("/tasks"),
  memory: () => request<{items: Memory[]}>("/memory?scope=workspace"),
  inbox: () => request<{items: Inbox[]}>("/inbox?unread=false"),
  config: (projectId: string) => request<ConfigResponse>(`/config?project_id=${encodeURIComponent(projectId)}`),
  diagnostics: () => request<Diagnostics>("/diagnostics"),
  cancel: (id: string) => request<{cancelled: boolean}>(`/runs/${encodeURIComponent(id)}/cancel`, {method: "POST"}),
  events: (runId: string, onEvent: (event: Event) => void) => {
    let source: EventSource | undefined;
    let closed = false;
    void ensureAuth().then(() => {
      if (closed) return;
      source = new EventSource(`/events?run_id=${encodeURIComponent(runId)}`);
      const handle = (message: MessageEvent) => onEvent(JSON.parse(message.data) as Event);
      ["run.status_changed", "run.output", "run.finished"].forEach((name) => source?.addEventListener(name, handle));
    });
    return () => { closed = true; source?.close(); };
  },
};
