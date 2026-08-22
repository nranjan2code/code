import { invoke } from "@tauri-apps/api/core";
import type {
  AgentEvent,
  BackendInfo,
  DiffResponse,
  Health,
  Message,
  SessionSummary,
} from "./types";

let base = "";
let token = "";

export function backendUrl(): string {
  return base;
}

export async function initBackend(): Promise<BackendInfo> {
  const info = await invoke<BackendInfo>("backend_info");
  if (info.ready && info.base_url && info.token) {
    base = info.base_url;
    token = info.token;
  }
  return info;
}

export function isBackendReady(): boolean {
  return !!base;
}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${base}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
      ...(init?.headers ?? {}),
    },
  });
  const text = await res.text();
  let parsed: unknown = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = text;
  }
  if (!res.ok) {
    const msg =
      (parsed as { error?: string })?.error ?? `${res.status} ${res.statusText}`;
    throw new Error(msg);
  }
  return parsed as T;
}

// ---- sessions ---------------------------------------------------------------

export function listSessions(): Promise<{ sessions: SessionSummary[] }> {
  return req("/sessions");
}

export function createSession(): Promise<{ session_id: string }> {
  return req("/sessions", { method: "POST", body: "{}" });
}

export function attachSession(id: string): Promise<{ session_id: string }> {
  return req(`/sessions/${id}/attach`, {
    method: "POST",
    body: JSON.stringify({ session_id: id }),
  });
}

export function transcript(id: string): Promise<{
  count: number;
  usage: Record<string, number>;
  messages: Message[];
}> {
  return req(`/sessions/${id}/transcript`);
}

export function runPrompt(id: string, prompt: string): Promise<void> {
  return req(`/sessions/${id}/run`, {
    method: "POST",
    body: JSON.stringify({ prompt }),
  });
}

export function steer(id: string, text: string): Promise<void> {
  return req(`/sessions/${id}/steering`, {
    method: "POST",
    body: JSON.stringify({ text }),
  });
}

export function cancelRun(id: string): Promise<void> {
  return req(`/sessions/${id}/cancel`, { method: "POST" });
}

export function runSide(id: string, question: string): Promise<void> {
  return req(`/sessions/${id}/side`, {
    method: "POST",
    body: JSON.stringify({ question }),
  });
}

export function cancelSide(id: string): Promise<void> {
  return req(`/sessions/${id}/side/cancel`, { method: "POST" });
}

export function answerApproval(
  id: string,
  requestId: string,
  approve: boolean,
): Promise<unknown> {
  return req(`/sessions/${id}/approvals/${requestId}`, {
    method: "POST",
    body: JSON.stringify({ approve }),
  });
}

export function health(): Promise<Health> {
  // /health is intentionally open; still send the header for consistency.
  return req("/health");
}

export function setPermissionMode(mode: string): Promise<void> {
  return req("/config/mode", { method: "POST", body: JSON.stringify({ mode }) });
}

export function readDiff(id: string): Promise<DiffResponse> {
  return req(`/sessions/${id}/diff`);
}

export function readFile(path: string): Promise<{ path: string; content: string }> {
  return req(`/fs/file?path=${encodeURIComponent(path)}`);
}

export function writeFile(path: string, content: string): Promise<unknown> {
  return req("/fs/file", { method: "PUT", body: JSON.stringify({ path, content }) });
}

export function fsTree(limit = 400): Promise<{ files: string[]; truncated: boolean }> {
  return req(`/fs/tree?limit=${limit}`);
}

export function startBestOfN(
  anchorId: string,
  prompt: string,
  n: number,
): Promise<{ runs: { session_id: string; branch: string; path: string }[] }> {
  return req(`/sessions/${anchorId}/bestofn`, {
    method: "POST",
    body: JSON.stringify({ prompt, n }),
  });
}

export function keepRun(childId: string): Promise<{ kept: string }> {
  return req(`/sessions/${childId}/keep`, { method: "POST" });
}

export function discardRun(childId: string): Promise<{ discarded: string }> {
  return req(`/sessions/${childId}/discard`, { method: "POST" });
}

export function getPr(id: string): Promise<import("./types").PrStatus> {
  return req(`/sessions/${id}/pr`);
}

export function mergePr(
  id: string,
  number: number,
  method: "squash" | "merge" | "rebase" = "squash",
): Promise<{ merging: number }> {
  return req(`/sessions/${id}/pr/merge`, {
    method: "POST",
    body: JSON.stringify({ number, method }),
  });
}

// ---- SSE ---------------------------------------------------------------------

export function openEventStream(
  id: string,
  onEvent: (ev: AgentEvent) => void,
  onError?: () => void,
): EventSource {
  const es = new EventSource(`${base}/sessions/${id}/events?token=${encodeURIComponent(token)}`);
  es.onmessage = (m) => {
    try {
      onEvent(JSON.parse(m.data) as AgentEvent);
    } catch {
      // ignore keep-alive/comment frames
    }
  };
  es.onerror = () => onError?.();
  return es;
}

export function openSideStream(
  id: string,
  onEvent: (ev: AgentEvent) => void,
): EventSource {
  const es = new EventSource(
    `${base}/sessions/${id}/side/events?token=${encodeURIComponent(token)}`,
  );
  es.onmessage = (m) => {
    try {
      onEvent(JSON.parse(m.data) as AgentEvent);
    } catch {
      // ignore keep-alive frames
    }
  };
  return es;
}

export function listTasks(): Promise<{ tasks: import("./types").TaskDef[] }> {
  return req("/tasks");
}

export function createTask(name: string, prompt: string, intervalSecs: number): Promise<unknown> {
  return req("/tasks", {
    method: "POST",
    body: JSON.stringify({ name, prompt, interval_secs: intervalSecs }),
  });
}

export function patchTask(id: string, patch: Partial<{ enabled: boolean; name: string; prompt: string; interval_secs: number }>): Promise<unknown> {
  return req(`/tasks/${id}`, { method: "PATCH", body: JSON.stringify(patch) });
}

export function deleteTask(id: string): Promise<unknown> {
  return req(`/tasks/${id}`, { method: "DELETE" });
}

export function runTaskNow(id: string): Promise<unknown> {
  return req(`/tasks/${id}/run-now`, { method: "POST" });
}

export function getLaunch(id: string): Promise<{
  servers: { name: string; cmd: string; args: string[]; port: number | null; running: boolean }[];
  error?: string;
}> {
  return req(`/sessions/${id}/launch`);
}

export function startLaunch(id: string, name: string): Promise<{ started: boolean; listening: boolean; error?: string }> {
  return req(`/sessions/${id}/launch/start`, { method: "POST", body: JSON.stringify({ name }) });
}

export function stopLaunch(id: string, name: string): Promise<unknown> {
  return req(`/sessions/${id}/launch/stop`, { method: "POST", body: JSON.stringify({ name }) });
}

export function launchLogs(id: string, name: string): Promise<{ lines: string[] }> {
  return req(`/sessions/${id}/launch/logs?name=${encodeURIComponent(name)}`);
}
