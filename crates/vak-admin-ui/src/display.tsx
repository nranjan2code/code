/// The console's shared presentation vocabulary.
///
/// Every screen has to print the same handful of things — a provider name, a
/// permission mode, a chat surface, a filesystem path, an event from the hub.
/// When each screen owned its own copy of those labels they drifted, and a
/// mode read one way on Overview and another way on Gateway. There is one
/// spelling of each here and every view imports it.
import { Show, createSignal } from "solid-js";
import type { ChatSurface, OnboardingState, PermissionMode, SystemEvent } from "./types";

// ---- chat surfaces ---------------------------------------------------------

// Chat surfaces with a bridge (crates/vak-server's InboundChannel impls) and
// a matching Core::bot_token_env entry. A routing key's surface prefix is
// validated against this list — the bare colon check it replaced accepted
// a pasted bot token (itself "digits:secret"-shaped) as a plausible key.
// No channel list lives in the console. The server owns it
// (`vak_core::Core::SURFACES`), so adding a transport is one edit there and
// no channel becomes the implicit default by being the one a UI hardcoded.
const [chatSurfaces, setChatSurfaces] = createSignal<ChatSurface[]>([]);
const [chatSurfacesError, setChatSurfacesError] = createSignal<string | null>(null);
export { chatSurfaces, chatSurfacesError, setChatSurfaces, setChatSurfacesError };
export const surfaceIds = () => chatSurfaces().map((s) => s.id);
export const surfaceLabel = (id: string) => chatSurfaces().find((s) => s.id === id)?.label ?? id;

// ---- icons (inline, stroke style) ------------------------------------------

export const Icon = (props: { d: string; size?: number }) => (
  <svg
    width={props.size ?? 16}
    height={props.size ?? 16}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path d={props.d} />
  </svg>
);

export const ICONS = {
  prompts: "M4 4h16v16H4z M8 9h8 M8 13h8 M8 17h5",
  overview: "M3 3v18h18M7 15l4-6 4 4 5-8",
  operations: "M4 6h16M4 12h16M4 18h16 M8 6v12 M16 6v12",
  runs: "M5 4l7 8-7 8 M13 4l7 8-7 8",
  library: "M4 5h6v14H4z M10 5h4v14h-4z M15 6l4-1 3 13-4 1z",
  diagnostics: "M3 12h4l3-8 4 16 3-8h4",
  automations: "M12 3v3 M12 18v3 M3 12h3 M18 12h3 M12 8a4 4 0 1 0 0 8a4 4 0 1 0 0-8",
  commitments: "M5 3h11l3 3v15H5z M9 8h6 M9 12h6 M9 16.5l1.7 1.7L14 15",
  sessions: "M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM23 21v-2a4 4 0 0 0-3-3.87",
  integrations: "M16.5 9.4 7.55 4.24a1.78 1.78 0 0 0-2.5 1.55v12.42a1.78 1.78 0 0 0 2.5 1.55L16.5 14.6a1.78 1.78 0 0 0 0-3.2z M21 12h-3 M3 12h1",
  gateway: "M4 4h16v12H4z M8 20h8 M12 16v4 M8 8h.01 M12 8h4 M8 12h8",
  memory: "M4 19.5A2.5 2.5 0 0 1 6.5 17H20 M4 4.5A2.5 2.5 0 0 1 6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15z",
  feeds: "M4 11a9 9 0 0 1 9 9M4 4a16 16 0 0 1 16 16M5 21a1 1 0 1 0 0-2 1 1 0 0 0 0 2z",
  search: "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35",
  inbox: "M22 12h-6l-2 3h-4l-2-3H2M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
  security: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z",
  projects: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
  finops: "M12 1v22 M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6",
  setup: "M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z",
  settings: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
};

// ---- providers and permission modes ---------------------------------------

// Cosmetic labels only — the actual set of selectable providers comes from
// `GET /providers` (`vak_core::Core::provider_names`, backed by the
// registration list in `vak-llm/src/registry.rs`). A name missing here
// prints as its bare id rather than disappearing.
export const PROVIDER_LABELS: Record<string, string> = {
  anthropic: "Anthropic (Claude)",
  openai: "OpenAI (Completions)",
  "openai-responses": "OpenAI (Responses)",
  google: "Google (Gemini)",
  openrouter: "OpenRouter",
  "openrouter-responses": "OpenRouter (Responses API)",
  "opencode-zen": "OpenCode Zen",
  nvidia: "NVIDIA NIM",
  bedrock: "Amazon Bedrock",
  ollama: "Ollama (local)",
};
export const providerLabel = (id: string) => PROVIDER_LABELS[id] ?? id;

/// Wire values are the `Debug` form the server prints (`GET /config`
/// reports `format!("{:?}", mode)`); the labels are what an operator reads.
/// Ordered least to most permissive.
export const MODES: { value: string; label: string }[] = [
  { value: "ReadOnly", label: "Look, don't touch" },
  { value: "WorkspaceWrite", label: "Work inside this workspace" },
  { value: "FullAccess", label: "No limits" },
];

/// The three permission modes in wire (kebab-case) form, ordered least to
/// most permissive — the same order and the same `mode-btn` control the
/// Settings page's "Permission Mode" panel uses, so an operator sees one
/// vocabulary in both places.
export const CHANNEL_MODES: { value: PermissionMode; label: string; desc: string }[] = [
  { value: "read-only", label: "Look, don't touch", desc: "Reads and searches only; every change refused" },
  { value: "workspace-write", label: "Work inside this workspace", desc: "Changes files here; anything else asks first" },
  { value: "full-access", label: "No limits", desc: "Nothing is checked with you first" },
];

/// Same three words wherever a wire mode is printed back, in either casing
/// the server may use (`read-only` from the gateway, `ReadOnly` from
/// `GET /config`).
export function modeLabel(mode: string | null | undefined): string {
  if (!mode) return "the workspace default";
  const kebab = CHANNEL_MODES.find((m) => m.value === mode);
  if (kebab) return kebab.label;
  return MODES.find((m) => m.value === mode)?.label ?? mode;
}

// ---- security event kinds --------------------------------------------------

/// The audit ledger's own kind strings, each paired with what it means. The
/// value is what `GET /security?kind=` filters on and must not change; the
/// label is all the operator ever needs to read.
export const SEC_KINDS: { value: string; label: string }[] = [
  { value: "", label: "Everything" },
  { value: "auth_failure", label: "Failed sign-in" },
  { value: "rate_limit", label: "Rate limited" },
  { value: "chat_allowlist", label: "Chat allowed" },
  { value: "chat_pending", label: "Chat knocked" },
  { value: "chat_approved", label: "Chat approved" },
  { value: "chat_denied", label: "Chat refused" },
  { value: "chat_revoked", label: "Chat removed" },
  { value: "permission_denial", label: "Action blocked" },
  { value: "config_change", label: "Setting changed" },
  { value: "provider_key_change", label: "Provider key changed" },
  { value: "full_access_grant", label: "Full access granted" },
  { value: "full_access_revoke", label: "Full access removed" },
];

export const secKindLabel = (kind: string) =>
  SEC_KINDS.find((k) => k.value === kind)?.label ?? kind.replaceAll("_", " ");

// ---- hub events ------------------------------------------------------------

/// `SystemEvent`'s serde tag is a Rust variant name. Print what happened
/// instead, and keep the tag on hover for anyone matching it against a log.
export const EVENT_LABELS: Record<string, string> = {
  Agent: "Agent step",
  SessionCreated: "Session started",
  SessionEntryAppended: "Message recorded",
  ConfigChanged: "Setting changed",
  GatewayInbound: "Message from a chat",
  ApprovalRequested: "Approval requested",
  ApprovalGranted: "Approval granted",
  ApprovalDenied: "Approval refused",
  SecurityEvent: "Security event",
  ProviderError: "Provider error",
  RateLimit: "Rate limited",
  Heartbeat: "Still connected",
  Lagged: "Events skipped",
};

export function summarizeEvent(ev: SystemEvent): string {
  switch (ev.type) {
    case "Agent":
      return ev.data.summary + (ev.data.detail ? ` · ${ev.data.detail}` : "");
    case "SessionCreated":
      return ev.data.session_id.slice(0, 12);
    case "SessionEntryAppended":
      return `${ev.data.kind} in ${ev.data.session_id.slice(0, 12)}`;
    case "ConfigChanged":
      return `${ev.data.label}: ${ev.data.detail}`;
    case "GatewayInbound":
      return `${surfaceLabel(ev.data.surface)} · ${ev.data.who}: ${ev.data.preview}`;
    case "ApprovalGranted":
    case "ApprovalDenied":
      return `${ev.data.tool} (${ev.data.id.slice(0, 8)})`;
    case "SecurityEvent":
      return `${secKindLabel(ev.data.kind)} — ${ev.data.label}`;
    case "ProviderError":
      return `${ev.data.provider}/${ev.data.model}: ${ev.data.error}`;
    case "RateLimit":
      return ev.data.provider;
    default:
      return "";
  }
}

// ---- paths -----------------------------------------------------------------

/// Shorten a path by dropping WHOLE middle segments, never by breaking one.
/// Two workspaces differ in their tail (`…/Projects/vakcoder`), almost never
/// in the `/Users/<name>` prefix every one of them shares, so the head is
/// what gets spent and the tail is kept to the last possible character.
///
/// The budget is in characters rather than segments on purpose: a
/// segment-counted result can still overflow the column and get clipped by
/// CSS from the right, which throws away exactly the end that distinguishes
/// two workspaces. Fitting the budget here means the CSS ellipsis is only
/// ever a backstop for a single segment longer than the whole column.
export function truncatePath(path: string, budget = 38): string {
  if (path.length <= budget) return path;
  const absolute = path.startsWith("/");
  const segments = path.split("/").filter(Boolean);
  if (segments.length <= 2) return path;

  const head = `${absolute ? "/" : ""}${segments[0]}/…`;
  // Grow the tail one segment at a time while it still fits, always keeping
  // at least the final segment even when nothing fits.
  let tail = segments[segments.length - 1];
  for (let i = segments.length - 2; i >= 1; i--) {
    const candidate = `${segments[i]}/${tail}`;
    if (head.length + 1 + candidate.length > budget) break;
    tail = candidate;
  }
  return `${head}/${tail}`;
}

/// A filesystem path in a width-constrained cell. Never wraps: middle
/// segments are elided first, and anything still too wide for the column is
/// clipped with an ellipsis by CSS. The untruncated path is always on the
/// `title` and selectable, so nothing is actually lost.
export function PathCell(props: { path: string; budget?: number }) {
  // The CSS cap is derived from the same budget the text was fitted to, so
  // the two can never disagree and clip a tail that JS had already made room
  // for. `ch` is the right unit here because `.path` is monospaced.
  const budget = () => props.budget ?? 38;
  return (
    <span class="path" title={props.path} style={{ "max-width": `${budget() + 1}ch` }}>
      {truncatePath(props.path, budget())}
    </span>
  );
}

// ---- page furniture --------------------------------------------------------

export function confirmDestructive(message: string): boolean {
  return window.confirm(`${message}\n\nThis cannot be undone from the admin console.`);
}

export function PageHeader(props: { title: string; description: string; actions?: import("solid-js").JSX.Element }) {
  return (
    <header class="page-header">
      <div>
        <h1>{props.title}</h1>
        <p>{props.description}</p>
      </div>
      <Show when={props.actions}>
        <div class="page-actions">{props.actions}</div>
      </Show>
    </header>
  );
}

export function StatCard(props: { label: string; value: string | number; sub?: string; tone?: string; progress?: number }) {
  return (
    <div class="stat-card" data-tone={props.tone ?? "default"}>
      <div class="stat-value">{props.value}</div>
      <div class="stat-label">{props.label}</div>
      <Show when={props.progress != null}>
        <div class="progress-bar">
          <div
            class={`progress-fill ${props.progress! > 90 ? "alert" : props.progress! > 75 ? "warn" : ""}`}
            style={{ width: `${Math.min(100, Math.max(0, props.progress!))}%` }}
          />
        </div>
      </Show>
      <Show when={props.sub}>
        <div class="stat-sub">{props.sub}</div>
      </Show>
    </div>
  );
}

// ---- setup steps -----------------------------------------------------------

/// Plain-language names for the steps the projection reports.
///
/// The design contract (doc 46 D10) is that the *question* is in plain
/// words and the precise vocabulary lives behind disclosure — a reader who
/// has never heard of a permission mode or a sandbox backend must still
/// know what a screen is asking.
export const SETUP_STEPS: { key: keyof OnboardingState; title: string; why: string }[] = [
  { key: "install", title: "Installation", why: "The app's own files are present and unmodified." },
  { key: "dependencies", title: "Tools on this machine", why: "Optional helpers some features use." },
  { key: "workspace", title: "Where Vakyartha works", why: "The folder Vakyartha reads, writes, and remembers in." },
  { key: "trust", title: "Trusting this folder", why: "Whether settings inside the folder may grant it power." },
  { key: "provider", title: "The AI service", why: "Which company's model answers, and your key for it." },
  { key: "route", title: "The model", why: "Which specific model runs your work." },
  { key: "permission", title: "How much Vakyartha may do alone", why: "Read only, work with approval, or unrestricted." },
  { key: "sandbox", title: "Containment", why: "What stops a command reaching outside the folder." },
  { key: "capabilities", title: "Starter skills", why: "Instructions that make Vakyartha better at common jobs." },
  { key: "integrations", title: "Connected apps", why: "Optional services Vakyartha can call, like web search." },
  { key: "channels", title: "Chat bots", why: "Talking to Vakyartha from Telegram, Discord, or Slack." },
  { key: "services", title: "Running in the background", why: "Whether Vakyartha keeps working when you close this." },
  { key: "first_result", title: "Your first task", why: "One safe, read-only run so you can see a result." },
];
