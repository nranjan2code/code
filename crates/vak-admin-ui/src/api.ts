import type { ConfigResponse, Diagnostics, Event, Inbox, Memory, Project, Run, Session, Task } from "./types";

let auth: Promise<void> | undefined;

const adminRecovery = "Open a fresh Admin Console from the tray or run `vakcoder admin`.";

class RuntimeConnectionError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RuntimeConnectionError";
  }
}

function tokenFromFragment(): string {
  // URLSearchParams handles an additional fragment parameter safely and avoids
  // throwing on a malformed percent-encoded fragment.
  return new URLSearchParams(location.hash.slice(1)).get("token") ?? "";
}

function ensureAuth(): Promise<void> {
  if (auth) return auth;
  const token = tokenFromFragment();
  auth = token
    ? fetch("/auth/login", { method: "POST", credentials: "include", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ token }) }).then(async (response) => {
      if (!response.ok) throw new RuntimeConnectionError(`Runtime authentication failed. ${adminRecovery}`);
      // The URL is often opened from a shell or the tray. Remove the one-time
      // credential as soon as the HttpOnly session cookie has been established.
      history.replaceState(null, "", `${location.pathname}${location.search}`);
    })
    : Promise.resolve();
  return auth;
}

function transportError(error: unknown): RuntimeConnectionError {
  if (error instanceof RuntimeConnectionError) return error;
  if (error instanceof TypeError) {
    return new RuntimeConnectionError(`The Runtime gateway is unreachable. Check that VakCoder is running, then try again. ${adminRecovery}`);
  }
  return new RuntimeConnectionError(error instanceof Error ? error.message : "The Runtime request failed.");
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  try {
    await ensureAuth();
    const response = await fetch(path, { credentials: "include", ...init });
    const body = await response.text();
    if (!response.ok) {
      if (response.status === 401) {
        throw new RuntimeConnectionError(`Runtime authentication is missing or expired. ${adminRecovery}`);
      }
      throw new RuntimeConnectionError(body || `Runtime returned HTTP ${response.status}.`);
    }
    return body ? JSON.parse(body) as T : undefined as T;
  } catch (error) {
    throw transportError(error);
  }
}

export function retryAuthentication(): void {
  // A retry is meaningful after the gateway comes back. A new browser-login
  // link still supplies its token through the URL fragment on the new page.
  auth = undefined;
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
