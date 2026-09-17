import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { api, AuthRequired } from "./api";
import {
  CHANNEL_MODES, ICONS, Icon, MODES, PageHeader, PathCell, SEC_KINDS, SETUP_STEPS, StatCard,
  chatSurfaces, chatSurfacesError, confirmDestructive, modeLabel, providerLabel, secKindLabel, setChatSurfaces,
  setChatSurfacesError,
  surfaceIds, surfaceLabel,
} from "./display";
import { Home } from "./Home";
import { OperationsCenter } from "./OperationsCenter";
import { Commitments } from "./Commitments";
import { PromptsSection } from "./Prompts";
import { SecurityCenter } from "./SecurityCenter";
import { Inbox } from "./Inbox";
import { SessionsList, SessionForensics } from "./SessionForensics";
import { clock, shortId, timeAgo } from "./time";
import {
  AccessPicker, BUILTIN_TOOLS, MATCHER_TOOLS, MatcherBuilder, ScheduleBuilder,
  describeDuration, describeMatcher, describeSchedule, shortDuration, syncSelect,
} from "./controls";
import type { AccessOption } from "./controls";
import {
  authed, conn, connectEvents, disconnectEvents, navigate, pushToast, route, sessionsVersion,
  setAuthed, statsVersion, toasts,
} from "./store";
import type {
  AllowlistEntry, BestOfNRun, Bot, ChannelPolicy, ConfigInfo, OnboardingState, StepState, CorePoolEntry, DiscoveredModelsResponse,
  ConfigScope, ConfigLayer, FinOpsStatus, FinOpsDailyPoint, FinOpsRollupEntry, HookConfig, IntegrationStatus,
  GatewayApprovalPolicy, GatewayBinding, GatewayStatus, InboxEntry, McpServerConfig, MemoryItem, OpsStatus,
  PermissionMode, ProviderSummary, VoiceProviderSummary,
  SearchHit, SecurityEvent, SessionCheckpoint, SessionDiff, SessionListItem,
  ActiveSubagent, SkillItem, SkillProposal, TaskItem, TranscriptEntry, VoiceConfig, WorkReceipt,
} from "./types";

// Theme state: initialized from localStorage and synchronized to document root dataset
const [theme, setTheme] = createSignal<"warm" | "dark" | "contrast">(
  (localStorage.getItem("vak_admin_theme") as "warm" | "dark" | "contrast") || "warm"
);

createEffect(() => {
  const t = theme();
  localStorage.setItem("vak_admin_theme", t);
  document.documentElement.dataset.theme = t === "warm" ? "" : t;
});

// Unread inbox badge: polled lightly while signed in.
const [unread, setUnread] = createSignal(0);
const [configScope, setConfigScope] = createSignal<ConfigScope>(
  localStorage.getItem("vak_admin_config_scope") === "project" ? "project" : "user",
);

export interface AgentScopeItem {
  id: string;
  name: string;
  personality?: string;
  lifecycle?: string;
}

export const [adminAgents, setAdminAgents] = createSignal<AgentScopeItem[]>([
  { id: "vak", name: "Vak", personality: "General Purpose Assistant", lifecycle: "active" }
]);

export const [selectedAgentId, setSelectedAgentId] = createSignal<string>(
  localStorage.getItem("vak_admin_selected_agent") || "global"
);

export async function refreshAdminAgents() {
  try {
    const res = await api.agents();
    const list = res?.agents;
    if (Array.isArray(list) && list.length > 0) {
      const hasVak = list.some((a) => a.id === "vak");
      const full = hasVak ? list : [{ id: "vak", name: "Vak", personality: "General Purpose Assistant", lifecycle: "active" }, ...list];
      setAdminAgents(full);
    }
  } catch (err) {
    console.warn("Failed to load agent catalogue", err);
  }
}

function setConfigScopePersisted(scope: ConfigScope) {
  setConfigScope(scope);
  localStorage.setItem("vak_admin_config_scope", scope);
}

function PromptsPage() {
  return (
    <>
      <PageHeader
        title="Prompts"
        description="What the agent is told before every turn. Edit Global or Workspace settings; narrower settings inherit them."
      />
      <PromptsSection
        scope={configScope}
        pushToast={pushToast}
        onScopeChange={setConfigScopePersisted}
      />
    </>
  );
}

function ScopeControl() {
  const currentAgent = () => adminAgents().find((a) => a.id === selectedAgentId());

  const handleScopeChange = (val: string) => {
    setSelectedAgentId(val);
    localStorage.setItem("vak_admin_selected_agent", val);
    if (val === "global") {
      setConfigScopePersisted("user");
    } else {
      setConfigScopePersisted("project");
    }
  };

  const scopeDetail = () => {
    if (selectedAgentId() === "global") {
      return "Global platform defaults & fleet telemetry";
    }
    const ag = currentAgent();
    if (ag) {
      return `${ag.name} · dedicated workspace & ledger`;
    }
    return "Agent workspace & sessions";
  };

  return (
    <label class="scope-control" aria-label="Admin scope">
      <span class="scope-control-label">Agent & Scope</span>
      <select
        value={selectedAgentId()}
        onChange={(e) => handleScopeChange(e.currentTarget.value)}
      >
        <option value="global">🌐 Global Platform Defaults</option>
        <optgroup label="Specialist Agents">
          <For each={adminAgents()}>
            {(agent) => (
              <option value={agent.id}>
                ✦ {agent.name} ({agent.id})
              </option>
            )}
          </For>
        </optgroup>
      </select>
      <span>{scopeDetail()}</span>
    </label>
  );
}

function AdminContextBar() {
  const current = () => route().split("?", 1)[0] || "#/overview";
  const scope = () => routeScope(current());
  const layered = () => scope() === "layered";
  const agent = () => adminAgents().find((a) => a.id === selectedAgentId());

  return (
    <div class="admin-context-bar" aria-label="Admin viewing context">
      <span class="admin-context-kicker">VIEWING</span>
      <Show
        when={layered()}
        fallback={
          <>
            <span class={`scope-mark scope-mark-${scope()}`} aria-hidden="true" />
            <strong>
              {selectedAgentId() === "global"
                ? "Global Platform"
                : `Agent: ${agent()?.name || selectedAgentId()}`}
            </strong>
            <span class="admin-context-detail">
              {selectedAgentId() === "global"
                ? "system-wide evidence, fleet telemetry, and shared defaults"
                : "dedicated workspace, private ledger, and personality instructions"}
            </span>
            <Show when={scope() === "switchable"}>
              <button class="ghost small context-action" onClick={() => navigate("#/operations")}>
                Open filter
              </button>
            </Show>
          </>
        }
      >
        <strong>
          {selectedAgentId() === "global"
            ? "Global Platform Defaults"
            : `Agent: ${agent()?.name || selectedAgentId()}`}
        </strong>
        <span class="admin-context-detail">
          {selectedAgentId() === "global"
            ? "applies across all agents and channels"
            : `applies to ${agent()?.name || selectedAgentId()}'s dedicated workspace`}
        </span>
      </Show>
    </div>
  );
}

function LoadError(props: { message?: string; onRetry?: () => void }) {
  return (
    <div class="empty error-state" role="alert">
      <strong>Couldn’t load this section</strong>
      <p>{props.message ?? "The server did not return usable data. Your saved configuration is unchanged."}</p>
      <Show when={props.onRetry}>
        <button class="ghost small" onClick={() => props.onRetry?.()}>Try again</button>
      </Show>
    </div>
  );
}

// ---- filesystem paths ------------------------------------------------------


// ---- permission rules ------------------------------------------------------

type RuleDecision = "allow" | "ask" | "deny";

interface ParsedRule {
  /** The rule exactly as it appears in config, for display. */
  raw: string;
  /** Tool the rule binds to, lowercased (`bash`, `mcp`, `write`, …). */
  tool: string;
  /** Argument glob, or null for a blanket rule covering the whole tool. */
  pattern: string | null;
  decision: RuleDecision;
}

/// Mirrors `vak_permission::rules::Rule::parse`. A leading `+`/`?`/`-` picks
/// allow/ask/deny; a bare rule allows. `Tool(pattern)` scopes to matching
/// arguments, `Tool` covers every call. Anything that does not parse is
/// surfaced rather than silently dropped — an unparseable rule in config is
/// exactly the thing an operator needs to see.
function parseRule(raw: string, listDecision: RuleDecision): ParsedRule | null {
  const input = raw.trim();
  if (!input) return null;
  const prefix = input[0];
  const decision: RuleDecision =
    prefix === "+" ? "allow" : prefix === "-" ? "deny" : prefix === "?" ? "ask" : listDecision;
  const rest = "+-?".includes(prefix) ? input.slice(1) : input;
  const open = rest.indexOf("(");
  const close = rest.lastIndexOf(")");
  const [tool, pattern] =
    open >= 0 && close > open
      ? [rest.slice(0, open).trim(), rest.slice(open + 1, close).trim()]
      : [rest.trim(), null];
  if (!tool) return null;
  return {
    raw: input,
    tool: tool.toLowerCase(),
    pattern: pattern === "*" || pattern === "" ? null : pattern,
    decision,
  };
}

/// True when the server did not report its rule lists at all. An older
/// binary simply omits the field, and "the server did not say" must never be
/// rendered as "there are no rules" — that is the difference between an
/// unknown scope and an unrestricted one.
function rulesReported(config: ConfigInfo | undefined): boolean {
  return !!config && config.permissions !== undefined;
}

function parseRuleLists(rules: import("./types").PermissionRules | undefined): ParsedRule[] {
  if (!rules) return [];
  return (
    [
      ["allow", rules.allow],
      ["ask", rules.ask],
      ["deny", rules.deny],
    ] as const
  ).flatMap(([decision, list]) =>
    (list ?? []).map((r) => parseRule(r, decision)).filter((r): r is ParsedRule => r !== null),
  );
}

/// Glob match for the subset the rule syntax uses on a single path segment:
/// `*` spans anything, `?` one character. Enough to answer "does this MCP
/// pattern name this server", which is all it is used for.
function globMatch(pattern: string, value: string): boolean {
  const rx = new RegExp(
    `^${pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".")}$`,
  );
  return rx.test(value);
}

/// Rules that govern calls into one MCP server. The engine matches
/// `mcp(server/tool)` patterns against the literal `server/tool` string, so a
/// rule reaches this server when it is blanket (`mcp`) or when the pattern's
/// server half names it.
function rulesForMcpServer(rules: ParsedRule[], server: string): ParsedRule[] {
  return rules.filter((r) => {
    if (r.tool !== "mcp") return false;
    if (r.pattern === null) return true;
    const serverPart = r.pattern.split("/")[0];
    return globMatch(serverPart, server);
  });
}

// Reuses the chip tones the gateway tables already established, so a red
// chip means the same thing on every screen. `danger` is the vocabulary --
// there is no `alert` tone, and naming one would have rendered every deny
// rule in muted grey.
/// A decision keyword said as an outcome. The keyword itself is still the
/// config vocabulary and stays on hover wherever this is used.
const DECISION_WORDS: Record<RuleDecision, string> = {
  allow: "allowed",
  ask: "asks you first",
  deny: "blocked",
};

const DECISION_TONE: Record<RuleDecision, string> = {
  allow: "success",
  ask: "warning",
  deny: "danger",
};

/// A parsed rule, said in a sentence. The raw form stays on hover and in the
/// title — an operator who knows the syntax loses nothing, and one who
/// doesn't no longer has to learn it to read the page.
function describeRule(rule: ParsedRule): string {
  const noun =
    rule.tool === "mcp"
      ? rule.pattern
        ? `the connected app ${rule.pattern.split("/")[0]}`
        : "connected apps"
      : (BUILTIN_TOOLS.find((t) => t.value === rule.tool)?.label ?? rule.tool);
  if (rule.tool !== "mcp" && rule.pattern) return `${noun} matching ${rule.pattern}`;
  return noun;
}

function RuleChip(props: { rule: ParsedRule }) {
  return (
    <span
      class={`chip chip-phrase chip-tone-${DECISION_TONE[props.rule.decision]}`}
      title={`${props.rule.decision}: ${props.rule.raw}`}
    >
      {describeRule(props.rule)}
    </span>
  );
}

// ---- Login -----------------------------------------------------------------

function Login() {
  const [token, setToken] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");

  const submit = async (e: SubmitEvent) => {
    e.preventDefault();
    if (!token().trim() || busy()) return;
    setBusy(true);
    setError("");
    try {
      await api.login(token().trim());
      setAuthed(true);
      connectEvents();
      navigate("#/overview");
    } catch (err) {
      setError(err instanceof AuthRequired || `${err}`.includes("401") ? "That token doesn\u2019t match. Check the one vak serve printed." : `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="login-wrap">
      <form class="login-card" onSubmit={submit}>
        <div class="login-logo"><img src="/admin/vak-icon.png" alt="" /></div>
        <h1>vak admin</h1>
        <p class="hint">Paste the token printed by <code>vak serve</code></p>
        <input
          type="password"
          placeholder="access token"
          aria-label="Access token"
          autocomplete="current-password"
          autofocus
          value={token()}
          onInput={(e) => setToken(e.currentTarget.value)}
          classList={{ shake: !!error() }}
        />
        <Show when={error()}>
          <div class="login-error">{error()}</div>
        </Show>
        <button type="submit" disabled={busy() || !token().trim()}>
          {busy() ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </div>
  );
}

// ---- Sessions list & Forensics (delegated to SessionForensics.tsx suite) ----

function Sessions() {
  return <SessionsList />;
}

function Transcript(props: { sessionId: string }) {
  return <SessionForensics sessionId={props.sessionId} />;
}

// ---- Extensions (MCP servers, skills, hooks, scheduled tasks) ---------------

/// Everything that widens what the agent can do, and what each thing is
/// permitted to do once loaded. Four sub-routes, one per extension point,
/// so a destination is nameable from the nav rather than found by scrolling
/// — the same shape the Gateway section uses.
const EXTENSION_TABS = [
  { hash: "#/integrations", label: "Connected apps" },
  { hash: "#/integrations/plugins", label: "Plugins" },
  { hash: "#/integrations/skills", label: "Skills" },
  { hash: "#/integrations/hooks", label: "Automations" },
  { hash: "#/integrations/tasks", label: "Scheduled tasks" },
] as const;

const KNOWLEDGE_TABS = [
  { hash: "#/memory", label: "Memory" },
  { hash: "#/feeds", label: "Feeds" },
  { hash: "#/search", label: "Search" },
] as const;

function knowledgeTab(): string {
  const r = route().split("?", 1)[0];
  return KNOWLEDGE_TABS.find((t) => r === t.hash || r.startsWith(`${t.hash}/`))?.hash ?? "#/memory";
}

function extensionsTab(): string {
  const r = route();
  return EXTENSION_TABS.slice(1).find((t) => r.startsWith(t.hash))?.hash ?? "#/integrations";
}

interface ExtensionsCtx {
  scope: () => ConfigScope;
  plugins: () => import("./types").PluginItem[];
  pluginsLoading: () => boolean;
  refetchPlugins: () => void | Promise<unknown>;
  mcp: () => Record<string, McpServerConfig>;
  mcpLoading: () => boolean;
  mcpError: () => unknown;
  refetchMcp: () => void | Promise<unknown>;
  hooks: () => HookConfig[];
  hooksLoading: () => boolean;
  hooksError: () => unknown;
  refetchHooks: () => void | Promise<unknown>;
  skills: () => SkillItem[];
  skillsLoading: () => boolean;
  proposals: () => SkillProposal[];
  proposalsLoading: () => boolean;
  refetchSkills: () => void | Promise<unknown>;
  tasks: () => TaskItem[];
  tasksLoading: () => boolean;
  refetchTasks: () => void | Promise<unknown>;
  /** Parsed allow/ask/deny rules from the running config. */
  rules: () => ParsedRule[];
  /** Effective permission mode, which decides everything no rule covers. */
  mode: () => string;
  /** Whether the server reported its rule lists at all — see rulesReported. */
  rulesKnown: () => boolean;
}

/// Stands in for the rule chips when the server never sent its rule lists.
/// Says so, rather than letting an empty list read as "unrestricted".
function RulesUnknown() {
  return (
    <span class="chip chip-phrase" title="This version of the server does not report its permission rules.">
      not reported
    </span>
  );
}

/// What happens to a tool call that no rule matches. Mirrors the mode arms
/// of `vak_permission::PermissionEngine::evaluate`: full-access allows,
/// read-only denies anything that is not a read tool, and workspace-write
/// asks. `mcp` and `task` are neither read nor write tools, so this is the
/// whole answer for them.
function modeDefaultDecision(mode: string): RuleDecision {
  if (mode === "FullAccess") return "allow";
  if (mode === "ReadOnly") return "deny";
  return "ask";
}

function ModeDefaultChip(props: { mode: string }) {
  const decision = () => modeDefaultDecision(props.mode);
  return (
    <span
      class={`chip chip-tone-${DECISION_TONE[decision()]}`}
      title={`No rule covers this, so the "${modeLabel(props.mode)}" setting decides: ${decision()}`}
    >
      {DECISION_WORDS[decision()]} by default
    </span>
  );
}

// ---- Extensions › MCP servers ----------------------------------------------

/** `KEY=VALUE` per line, the same shape a `.env` file uses — matches how
 * these values are described elsewhere ("Names live in .vak/config.toml").
 * Blank lines and lines without `=` are ignored rather than rejected, so a
 * stray trailing newline never blocks a save. */
function parseEnvLines(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const idx = line.indexOf("=");
    if (idx <= 0) continue;
    const key = line.slice(0, idx).trim();
    if (key) out[key] = line.slice(idx + 1).trim();
  }
  return out;
}
function envToLines(env: Record<string, string> | undefined): string {
  return Object.entries(env ?? {})
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
}

function McpServersView(props: { ctx: ExtensionsCtx }) {
  const [expanded, setExpanded] = createSignal("");
  const [editing, setEditing] = createSignal("");
  const [editCmd, setEditCmd] = createSignal("");
  const [editArgs, setEditArgs] = createSignal("");
  const [editNetwork, setEditNetwork] = createSignal(false);
  const [editEnv, setEditEnv] = createSignal("");
  const [newName, setNewName] = createSignal("");
  const [newCmd, setNewCmd] = createSignal("");
  const [newArgs, setNewArgs] = createSignal("");
  const [newNetwork, setNewNetwork] = createSignal(false);
  const [newEnv, setNewEnv] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [integrationKeys, setIntegrationKeys] = createSignal<Record<string, string>>({});
  const [integrationBusy, setIntegrationBusy] = createSignal("");
  const [catalog, catalogActions] = createResource(
    () => props.ctx.scope(),
    (scope) => api.integrationCatalog(scope),
  );

  const servers = createMemo(() => Object.entries(props.ctx.mcp()) as [string, McpServerConfig][]);

  const addServer = async () => {
    const name = newName().trim();
    const cmd = newCmd().trim();
    if (!name || !cmd || busy()) return;
    setBusy(true);
    try {
      const env = parseEnvLines(newEnv());
      await api.putMcpServers({
        ...props.ctx.mcp(),
        [name]: {
          command: cmd,
          args: newArgs().trim() ? newArgs().trim().split(/\s+/) : [],
          network: newNetwork(),
          ...(Object.keys(env).length > 0 ? { env } : {}),
        },
      }, props.ctx.scope());
      await props.ctx.refetchMcp();
      pushToast("info", `Connected ‘${name}’`);
      setNewName("");
      setNewCmd("");
      setNewArgs("");
      setNewNetwork(false);
      setNewEnv("");
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const removeServer = async (name: string) => {
    if (!confirmDestructive(`Disconnect “${name}”? Vak stops using its tools.`)) return;
    const next = { ...props.ctx.mcp() };
    delete next[name];
    try {
      await api.putMcpServers(next, props.ctx.scope());
      await props.ctx.refetchMcp();
      pushToast("info", `Disconnected ‘${name}’`);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  const beginEdit = (name: string, server: McpServerConfig) => {
    setEditing(name);
    setEditCmd(server.command);
    setEditArgs((server.args ?? []).join(" "));
    setEditNetwork(!!server.network);
    setEditEnv(envToLines(server.env));
  };

  const saveEdit = async (name: string) => {
    if (!editCmd().trim() || busy()) return;
    setBusy(true);
    try {
      const env = parseEnvLines(editEnv());
      await api.putMcpServers({
        ...props.ctx.mcp(),
        [name]: {
          command: editCmd().trim(),
          args: editArgs().trim() ? editArgs().trim().split(/\s+/) : [],
          network: editNetwork(),
          ...(Object.keys(env).length > 0 ? { env } : {}),
        },
      }, props.ctx.scope());
      await props.ctx.refetchMcp();
      pushToast("info", `Updated ‘${name}’`);
      setEditing("");
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const enableCatalogEntry = async (entry: IntegrationStatus) => {
    if (integrationBusy()) return;
    const key = integrationKeys()[entry.id]?.trim();
    if (entry.key_required && !key && !entry.key_effective) return;
    setIntegrationBusy(entry.id);
    try {
      await api.enableIntegration(entry.id, props.ctx.scope(), key);
      setIntegrationKeys((current) => ({ ...current, [entry.id]: "" }));
      await Promise.all([catalogActions.refetch(), props.ctx.refetchMcp()]);
      pushToast("info", `${entry.label} enabled in ${props.ctx.scope() === "user" ? "Global" : "Workspace"}`);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setIntegrationBusy("");
    }
  };

  const removeCatalogEntry = async (entry: IntegrationStatus) => {
    if (integrationBusy()) return;
    setIntegrationBusy(entry.id);
    try {
      await api.removeIntegration(entry.id, props.ctx.scope());
      await Promise.all([catalogActions.refetch(), props.ctx.refetchMcp()]);
      pushToast("info", props.ctx.scope() === "project" && entry.inherited
        ? `${entry.label} workspace override cleared`
        : `${entry.label} removed from ${props.ctx.scope() === "user" ? "Global" : "Workspace"}`);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setIntegrationBusy("");
    }
  };

  return (
    <>
      <div class="toolbar">
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refetchMcp()}>Refresh</button>
      </div>

      <section class="panel" style="margin-bottom:14px">
        <div class="panel-title-row">
          <div>
            <h2>Curated MCP integrations</h2>
            <p class="dim">
              These definitions run real upstream MCP packages. Keys are stored in the selected
              scope’s private secret file and are never returned to the browser.
            </p>
          </div>
        </div>
        <Show when={!catalog.loading} fallback={<div class="cred-list"><span class="skel skel-block" /><span class="skel skel-block" /></div>}>
          <div class="integration-catalog">
            <For each={catalog()?.integrations ?? []}>
              {(entry) => (
                <article class="integration-row">
                  <div class="integration-summary">
                    <div>
                      <strong>{entry.label}</strong>
                      <p>{entry.description}</p>
                      <code>{entry.command} {entry.args.join(" ")}</code>
                    </div>
                    <span class={`chip chip-tone-${entry.effective ? "success" : "ask"}`}>
                      {entry.configured_here ? "set here" : entry.inherited ? "inherited" : "off"}
                    </span>
                  </div>
                  <Show when={entry.env_var}>
                    <div class="form-row integration-key-row">
                      <label for={`${entry.id}-key`}>{entry.env_var}</label>
                      <input
                        id={`${entry.id}-key`}
                        type="password"
                        autocomplete="off"
                        placeholder={entry.key_here ? "Key saved in this scope" : entry.key_inherited ? "Using inherited key — enter to override" : "Enter API key"}
                        value={integrationKeys()[entry.id] ?? ""}
                        onInput={(event) => setIntegrationKeys((current) => ({ ...current, [entry.id]: event.currentTarget.value }))}
                      />
                    </div>
                  </Show>
                  <div class="row-gap">
                    <button
                      disabled={integrationBusy() === entry.id || (entry.key_required && !(integrationKeys()[entry.id]?.trim()) && !entry.key_effective)}
                      onClick={() => void enableCatalogEntry(entry)}
                    >
                      {integrationBusy() === entry.id ? "Saving…" : entry.configured_here ? "Update key" : entry.inherited ? "Override here" : "Enable"}
                    </button>
                    <Show when={entry.configured_here}>
                      <button class="danger small" disabled={!!integrationBusy()} onClick={() => void removeCatalogEntry(entry)}>
                        {props.ctx.scope() === "project" ? "Reset to Global" : "Remove"}
                      </button>
                    </Show>
                    <a class="ghost small button-link" href={entry.documentation_url} target="_blank" rel="noreferrer noopener">Upstream docs</a>
                  </div>
                </article>
              )}
            </For>
          </div>
        </Show>
      </section>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Connected apps</h2>
            <p class="dim">
              Each row is a program vak starts when it needs the tools that app provides. What it is
              then allowed to do comes from the rules under{" "}
              <a href="#/settings">Settings</a>, and where no rule reaches it, from the setting that
              decides everything else.
            </p>
            <p class="dim">This list shows only the selected scope. Use the Global/Workspace selector to inspect or edit the other scope.</p>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.mcpLoading()}>
            <table class="table">
              <thead>
                <tr><th>app</th><th>starts with</th><th>internet</th><th>secrets</th><th>rules that apply</th><th /></tr>
              </thead>
              <tbody><SkeletonRows cols={6} /></tbody>
            </table>
          </Match>

          <Match when={props.ctx.mcpError()}>
            <div class="error-state" role="alert">Could not load connected apps: {String(props.ctx.mcpError())} <button class="ghost small" type="button" onClick={() => props.ctx.refetchMcp()}>Retry</button></div>
          </Match>

          <Match when={servers().length === 0}>
            <div class="empty empty-teach">
              <strong>No apps are connected yet.</strong>
              <p>
A connected app hands vak extra tools — a GitHub client, a database, a browser. Connect one
                below and it starts up the first time something actually uses it; until then it costs
                you nothing.
              </p>
            </div>
          </Match>

          <Match when={servers().length > 0}>
            <table class="table">
              <thead>
                <tr><th>app</th><th>starts with</th><th>internet</th><th>secrets</th><th>rules that apply</th><th /></tr>
              </thead>
              <tbody>
                <For each={servers()}>
                  {([name, server]) => {
                    const matching = createMemo(() => rulesForMcpServer(props.ctx.rules(), name));
                    const envKeys = () => Object.keys(server.env ?? {});
                    const open = () => expanded() === name;
                    return (
                      <>
                        <tr tabindex="0" classList={{ "row-open": open() }} onClick={() => setExpanded(open() ? "" : name)} onKeyDown={(e) => e.key === "Enter" && setExpanded(open() ? "" : name)}>
                          <td class="mono bold">{name}</td>
                          <td class="dim col-command">
                            <span class="path" title={`${server.command} ${(server.args ?? []).join(" ")}`}>
                              {server.command} {(server.args ?? []).join(" ")}
                            </span>
                          </td>
                          <td>
                            <span class={`chip chip-tone-${server.network ? "warning" : "success"}`}>
                              {server.network ? "can reach the internet" : "no internet"}
                            </span>
                          </td>
                          <td class="dim">
                            {envKeys().length > 0 ? `${envKeys().length} secret${envKeys().length === 1 ? "" : "s"}` : "—"}
                          </td>
                          <td>
                            <Switch fallback={<ModeDefaultChip mode={props.ctx.mode()} />}>
                              <Match when={!props.ctx.rulesKnown()}><RulesUnknown /></Match>
                              <Match when={matching().length > 0}>
                                <div class="chip-stack">
                                  <For each={matching()}>{(r) => <RuleChip rule={r} />}</For>
                                </div>
                              </Match>
                            </Switch>
                          </td>
                          <td><span class="chev">{open() ? "⌄" : "›"}</span></td>
                        </tr>
                        <Show when={open()}>
                          <tr class="row-detail">
                            <td colspan={6}>
                              <div class="detail-grid">
                                <div>
                                  <span class="eyebrow">What gets run</span>
                                  <pre class="mono detail-pre">{server.command} {(server.args ?? []).join(" ")}</pre>
                                </div>
                                <div>
                                  <span class="eyebrow">Secrets passed in</span>
                                  <Show
                                    when={envKeys().length > 0}
                                    fallback={<p class="dim">Nothing beyond vak's own environment.</p>}
                                  >
                                    <div class="chip-stack">
                                      <For each={envKeys()}>{(k) => <span class="chip mono">{k}</span>}</For>
                                    </div>
                                    <p class="dim">
                                      Stored in <code>.vak/config.toml</code> — usually a{" "}
                                      <code>{"${VAR}"}</code> reference resolved from <code>.env</code> at
                                      launch rather than a literal secret, but whatever is written there is
                                      what "Edit command" below will show and can change.
                                    </p>
                                  </Show>
                                </div>
                                <div>
                                  <span class="eyebrow">What it is allowed to do</span>
                                  <Show when={!props.ctx.rulesKnown()}>
                                    <p class="dim">
This version of the server doesn’t report its rules, so what governs calls
                                      to <code>{name}</code> can’t be shown here. Open the workspace’s{" "}
                                      <code>config.toml</code> to read them, or update vak.
                                    </p>
                                  </Show>
                                  <Show when={props.ctx.rulesKnown()}>
                                  <Show
                                    when={matching().length > 0}
                                    fallback={
                                      <p class="dim">
No rule mentions this app, so every call to it is decided by the
                                        setting under Settings — “{modeLabel(props.ctx.mode())}”, which means{" "}
                                        <strong>{DECISION_WORDS[modeDefaultDecision(props.ctx.mode())]}</strong>.
                                        Add an <code>mcp({name}/*)</code> rule to the config file to
                                        change that for this app alone.
                                      </p>
                                    }
                                  >
                                    <div class="chip-stack">
                                      <For each={matching()}>{(r) => <RuleChip rule={r} />}</For>
                                    </div>
                                    <p class="dim">
When several rules match the same call, the strictest one wins — blocked beats
                                      ask beats allowed, and the order they appear in never matters. A call none
                                      of these cover falls back to “{modeLabel(props.ctx.mode())}”, which means{" "}
                                      <strong>{DECISION_WORDS[modeDefaultDecision(props.ctx.mode())]}</strong>.
                                    </p>
                                  </Show>
                                  </Show>
                                </div>
                              </div>
                              <div class="row-gap" style="margin-top:12px">
                                <Show when={editing() !== name}>
                                  <button class="ghost small" onClick={() => beginEdit(name, server)}>Edit</button>
                                </Show>
                                <button class="danger small" onClick={() => removeServer(name)}>
                                  Disconnect
                                </button>
                              </div>
                              <Show when={editing() === name}>
                                <div class="edit-form compact-edit">
                                  <div class="form-row"><label>Program to run</label><input class="mono" value={editCmd()} onInput={(e) => setEditCmd(e.currentTarget.value)} /></div>
                                  <div class="form-row"><label>Arguments</label><input class="mono" value={editArgs()} onInput={(e) => setEditArgs(e.currentTarget.value)} /></div>
                                  <label class="inherit-toggle">
                                    <input type="checkbox" checked={editNetwork()} onChange={(e) => setEditNetwork(e.currentTarget.checked)} />
                                    This app needs internet access
                                  </label>
                                  <div class="form-row">
                                    <label>Secrets it needs</label>
                                    <textarea class="mono" rows={3} placeholder={"KEY=value, one per line"} value={editEnv()} onInput={(e) => setEditEnv(e.currentTarget.value)} />
                                  </div>
                                  <div class="row-gap">
                                    <button disabled={busy() || !editCmd().trim()} onClick={() => void saveEdit(name)}>Save changes</button>
                                    <button class="ghost small" onClick={() => setEditing("")}>Cancel</button>
                                  </div>
                                </div>
                              </Show>
                            </td>
                          </tr>
                        </Show>
                      </>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>

        <details class="advanced">
          <summary>Connect a custom MCP server</summary>
          <p class="dim">Use an exact local command you have reviewed. Nothing starts until a turn invokes one of its tools.</p>
          <div class="form-row">
            <label>Name</label>
            <input placeholder="github, filesystem, postgres…" value={newName()} onInput={(e) => setNewName(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>Program to run</label>
            <input class="mono" placeholder="npx, uvx, python3…" value={newCmd()} onInput={(e) => setNewCmd(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>Arguments</label>
            <input class="mono" placeholder="-y @modelcontextprotocol/server-github" value={newArgs()} onInput={(e) => setNewArgs(e.currentTarget.value)} />
          </div>
          <label class="inherit-toggle">
            <input type="checkbox" checked={newNetwork()} onChange={(e) => setNewNetwork(e.currentTarget.checked)} />
            This app needs internet access
          </label>
          <div class="form-row">
            <label>Secrets it needs</label>
            <textarea class="mono" rows={3} placeholder={"GITHUB_TOKEN=${GITHUB_TOKEN}, one per line"} value={newEnv()} onInput={(e) => setNewEnv(e.currentTarget.value)} />
          </div>
          <p class="dim">
            One <code>NAME=value</code> per line. Write <code>{"${NAME}"}</code> as the value to
            pass a variable already in your environment through, instead of pasting the secret here.
          </p>
          <div class="row-gap" style="margin-top:12px">
            <button disabled={busy() || !newName().trim() || !newCmd().trim()} onClick={() => void addServer()}>
              {busy() ? "Connecting…" : "Connect"}
            </button>
          </div>
        </details>
      </section>
    </>
  );
}

function PluginsView(props: { ctx: ExtensionsCtx }) {
  const [path, setPath] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [sourcePath, setSourcePath] = createSignal("");
  const [sourceLabel, setSourceLabel] = createSignal("");
  const [keyId, setKeyId] = createSignal("");
  const [publicKey, setPublicKey] = createSignal("");
  const [signature, setSignature] = createSignal("");
  const [catalogQuery, setCatalogQuery] = createSignal("");
  const pluginScope = () => props.ctx.scope() === "user" ? "user" as const : "workspace" as const;
  const [sources, { refetch: refetchSources }] = createResource(
    pluginScope,
    (scope) => api.pluginSources(scope),
  );
  const [catalog, { refetch: refetchCatalog }] = createResource(
    () => [catalogQuery(), pluginScope()] as const,
    ([query, scope]) => api.pluginCatalog(query, scope),
  );
  const refresh = props.ctx.refetchPlugins;
  const install = async (update: boolean) => {
    const value = path().trim();
    if (!value || busy()) return;
    setBusy(true);
    try {
      if (update) await api.pluginUpdate(value, pluginScope());
      else await api.pluginInstall(value, pluginScope());
      await Promise.all([refetchSources(), refetchCatalog()]);
      pushToast("info", update ? "Plugin generation staged" : "Plugin installed disabled");
      setPath("");
      refresh();
    } catch (error) {
      pushToast("alert", `${error}`);
    } finally {
      setBusy(false);
    }
  };
  const action = async (name: string, operation: "enable" | "disable" | "rollback" | "remove") => {
    if (busy()) return;
    setBusy(true);
    try {
      await api.pluginAction(name, operation, pluginScope());
      pushToast("info", `${name} ${operation}d`);
      refresh();
    } catch (error) {
      pushToast("alert", `${error}`);
    } finally {
      setBusy(false);
    }
  };
  return <section class="stack plugin-stack">
    <div class="panel">
      <div class="panel-title-row"><div><h2>Install a reviewed package</h2><p>Packages are inspected, content-addressed, and installed disabled in {props.ctx.scope() === "user" ? "Global" : "Workspace"} until you enable them.</p></div></div>
      <div class="form-row"><label>Local package directory</label><input class="mono" placeholder="/path/to/plugin" value={path()} onInput={(e) => setPath(e.currentTarget.value)} /></div>
      <div class="row-gap"><button disabled={busy() || !path().trim()} onClick={() => void install(false)}>Install disabled</button><button class="ghost" disabled={busy() || !path().trim()} onClick={() => void install(true)}>Stage update</button></div>
    </div>
    <div class="panel">
      <div class="panel-title-row"><div><h2>Catalog sources</h2><p>Register a local marketplace snapshot for review. Sources start disabled and are revalidated before activation.</p></div></div>
      <Show when={sources.error || catalog.error}>
        <div class="error-state" role="alert">
          Catalog data is unavailable; empty results are unknown, not absent.
          <Show when={sources.error}> Sources: {String(sources.error)} <button class="ghost small" type="button" onClick={() => void refetchSources()}>Retry sources</button></Show>
          <Show when={catalog.error}> Catalog: {String(catalog.error)} <button class="ghost small" type="button" onClick={() => void refetchCatalog()}>Retry catalog</button></Show>
        </div>
      </Show>
      <div class="form-row"><label>Catalog directory<input class="mono" value={sourcePath()} onInput={(e) => setSourcePath(e.currentTarget.value)} placeholder="/path/to/catalog" /></label></div>
      <div class="form-row"><label>Label<input value={sourceLabel()} onInput={(e) => setSourceLabel(e.currentTarget.value)} placeholder="Team catalog" /></label></div>
      <details class="advanced"><summary>Detached Ed25519 evidence (optional)</summary><div class="form-row"><label>Key ID<input class="mono" value={keyId()} onInput={(e) => setKeyId(e.currentTarget.value)} /></label><label>Public key (base64)<input class="mono" value={publicKey()} onInput={(e) => setPublicKey(e.currentTarget.value)} /></label><label>Signature (base64)<input class="mono" value={signature()} onInput={(e) => setSignature(e.currentTarget.value)} /></label></div></details>
      <button type="button" disabled={busy() || !sourcePath().trim()} onClick={async () => { setBusy(true); try { const signed = keyId() && publicKey() && signature() ? { key_id: keyId(), public_key: publicKey(), signature: signature() } : undefined; await api.pluginRegisterSource(sourcePath(), sourceLabel() || "Local catalog", pluginScope(), signed); await refetchSources(); setSourcePath(""); setSourceLabel(""); setKeyId(""); setPublicKey(""); setSignature(""); pushToast("info", "Catalog source registered disabled"); } catch (error) { pushToast("alert", `${error}`); } finally { setBusy(false); } }}>Register source</button>
      <Show when={(sources()?.sources ?? []).length > 0}><div class="capability-list" style={{ "margin-top": "12px" }}><For each={sources()?.sources ?? []}>{(source) => <div class="capability-item"><div class="panel-title-row"><span><strong>{source.label}</strong><small>{source.format} · {source.trust} · {source.enabled ? "Enabled" : "Disabled"} · {source.signature ? (source.signature.verified ? "Signed" : "Signature invalid") : "Unsigned"}</small></span><code title={source.catalog_digest}>sha256:{source.catalog_digest.slice(0, 12)}</code></div><code>{source.trace_id}</code><div class="settings-actions"><button class="settings-button" disabled={busy()} onClick={async () => { setBusy(true); try { await api.pluginSourceAction(source.id, source.enabled ? "disable" : "enable", pluginScope()); await refetchSources(); } catch (error) { pushToast("alert", `${error}`); } finally { setBusy(false); } }}>{source.enabled ? "Disable source" : "Enable source"}</button><Show when={source.signature}><button class="settings-button danger" disabled={busy()} onClick={async () => { setBusy(true); try { await api.pluginKeyAction(source.signature!.key_id, source.signature!.revoked ? "restore" : "revoke", pluginScope()); await refetchSources(); } catch (error) { pushToast("alert", `${error}`); } finally { setBusy(false); } }}>{source.signature!.revoked ? "Restore key" : "Revoke key"}</button></Show></div></div>}</For></div></Show>
      <div class="form-row"><label>Search catalog entries<input value={catalogQuery()} onInput={(e) => setCatalogQuery(e.currentTarget.value)} placeholder="frontend, testing, release…" /></label></div>
      <Show when={(catalog()?.entries ?? []).length > 0} fallback={<p class="dim">No catalog entries match yet. Enable a verified source only after reviewing it.</p>}>
        <div class="capability-list"><For each={catalog()?.entries ?? []}>{(entry) => <div class="capability-item"><div class="panel-title-row"><span><strong>{entry.name}</strong><small>{entry.source_label} · {entry.source_enabled ? "Source enabled" : "Source disabled"}{entry.version ? ` · v${entry.version}` : ""}</small></span><code title={entry.catalog_digest}>sha256:{entry.catalog_digest.slice(0, 12)}</code></div><p>{entry.description || "No description"}</p><small>{entry.license ? `License: ${entry.license}` : "License: not declared"}</small></div>}</For></div>
      </Show>
    </div>
    <Show when={!props.ctx.pluginsLoading()} fallback={<div class="panel"><div class="spin" /></div>}>
      <For each={props.ctx.plugins()} fallback={<div class="panel empty"><h2>No installed plugins</h2><p>Install a local package after reviewing its publisher, license, capabilities, and digest.</p></div>}>
        {(plugin) => <article class="panel">
          <div class="panel-title-row"><div><h2>{plugin.name}</h2><p>{plugin.description || "No description"}</p></div><span class={`chip ${plugin.enabled ? "chip-tone-allow" : "chip-tone-ask"}`}>{plugin.enabled ? "Enabled" : "Disabled"}</span></div>
          <div class="meta-grid"><span>v{plugin.version}</span><span>{plugin.scope}</span><span>{plugin.format}</span><span class="mono" title={plugin.digest}>sha256:{plugin.digest.slice(0, 12)}</span></div>
          <p class="dim mono" title={plugin.trace_id}>trace {plugin.trace_id}</p>
          <div class="row-gap"><button onClick={() => void action(plugin.name, plugin.enabled ? "disable" : "enable")}>{plugin.enabled ? "Disable" : "Enable"}</button><button class="ghost" onClick={() => void action(plugin.name, "rollback")}>Rollback</button><button class="danger" onClick={() => void action(plugin.name, "remove")}>Remove</button></div>
        </article>}
      </For>
    </Show>
  </section>;
}

// ---- Extensions › Skills ---------------------------------------------------

function SkillsView(props: { ctx: ExtensionsCtx }) {
  const [busyId, setBusyId] = createSignal("");
  const [query, setQuery] = createSignal("");
  const visibleSkills = createMemo(() => {
    const needle = query().trim().toLowerCase();
    return props.ctx.skills().filter((skill) => {
      const inScope = props.ctx.scope() === "user" ? skill.scope === "user" : skill.scope === "workspace";
      return inScope && (!needle || skill.name.toLowerCase().includes(needle) || (skill.description ?? "").toLowerCase().includes(needle));
    });
  });

  const act = async (id: string, promote: boolean) => {
    if (!promote && !confirmDestructive("Reject this skill proposal?")) return;
    setBusyId(id);
    try {
      if (promote) await api.promoteProposal(id);
      else await api.rejectProposal(id);
      await props.ctx.refetchSkills();
      pushToast("info", promote ? "Promoted to an active skill" : "Proposal rejected");
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusyId("");
    }
  };

  return (
    <>
      <div class="toolbar">
        <input class="search-input" aria-label="Filter skills" placeholder="Filter skills…" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} />
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refetchSkills()}>Refresh</button>
      </div>

      <Show when={props.ctx.proposals().length > 0}>
        <section class="panel panel-alert" style="margin-bottom:14px">
          <div class="panel-title-row">
            <div>
              <h2>{props.ctx.proposals().length} skill{props.ctx.proposals().length === 1 ? "" : "s"} proposed</h2>
              <p class="dim">
                Written by the agent from its own runs. Nothing loads until you promote it.
              </p>
            </div>
          </div>
          <ul class="hit-list">
            <For each={props.ctx.proposals()}>
              {(p) => (
                <li class="inbox-item">
                  <div class="hit-meta"><strong class="mono">{p.name}</strong></div>
                  <div class="hit-snippet">{p.description}</div>
                  <div class="row-gap" style="margin-top:8px">
                    <button class="approve small" disabled={busyId() === p.id} onClick={() => void act(p.id, true)}>
                      Promote
                    </button>
                    <button class="danger small" disabled={busyId() === p.id} onClick={() => void act(p.id, false)}>
                      Reject
                    </button>
                  </div>
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Loaded skills</h2>
            <p class="dim">
              This is the {props.ctx.scope() === "user" ? "Global" : "Workspace"} skill set. A skill is instructions, not capability: it tells the agent how to approach a job, and
              every tool it then reaches for is gated the same as any other call. What it changes is
              which files the agent is told to read — so where a skill comes from is the thing worth
              watching.
            </p>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.skillsLoading()}>
            <table class="table skill-table">
              <thead><tr><th>skill</th><th>available in</th><th>what it does</th><th>file</th></tr></thead>
              <tbody><SkeletonRows cols={4} /></tbody>
            </table>
          </Match>

          <Match when={props.ctx.skills().length === 0}>
            <div class="empty empty-teach">
              <strong>No skills are loaded.</strong>
              <p>
                Skills are discovered from <code>.vak/skills/</code> in this workspace and from the
                Global skills directory (<code>~/vak-home/.vak/skills</code>). Each is a folder with a <code>SKILL.md</code> inside.
              </p>
            </div>
          </Match>

          <Match when={props.ctx.skills().length > 0}>
            <table class="table skill-table">
              <thead><tr><th>skill</th><th>available in</th><th>what it does</th><th>file</th></tr></thead>
              <tbody>
              <For each={visibleSkills()}>
                  {(s) => (
                    <tr class="row-static">
                      <td class="mono bold">{s.name}{s.shadowed ? <span class="chip chip-tone-ask" style={{ "margin-left": "6px" }}>shadowed</span> : null}</td>
                      <td>
                        <span class={`chip ${s.scope === "workspace" ? "chip-tool" : "chip-mode"}`} title={s.scope ?? "user"}>
                          {s.scope === "workspace" ? "Workspace" : "Global"}
                        </span>
                      </td>
                      <td class="dim">{s.description || "No description written for this skill."}</td>
                      <td class="dim col-path">
                        <Show when={s.path} fallback={<span class="dim">—</span>}>
                          <PathCell path={s.path!} />
                        </Show>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>
      </section>
    </>
  );
}

// ---- Extensions › Hooks ----------------------------------------------------

const HOOK_EVENTS = ["pre_tool_use", "post_tool_use", "session_start", "stop"] as const;

/// The same four events, said the way an operator thinks about them. The
/// wire value never changes — only what the console prints.
const HOOK_EVENT_LABELS: Record<string, string> = {
  pre_tool_use: "Before the agent uses a tool",
  post_tool_use: "After the agent uses a tool",
  session_start: "When a session starts",
  stop: "When a run finishes",
};

/// Milliseconds are the wire unit; seconds are the unit an operator has an
/// intuition for. The stored value is untouched for anything not on the list.
function TimeoutPicker(props: { value: string; onChange: (ms: string) => void }) {
  const CHOICES = ["1000", "5000", "10000", "30000", "60000"];
  const label = (ms: string) => `${Math.round(parseInt(ms, 10) / 100) / 10} seconds`;
  return (
    <select value={props.value} onChange={(e) => props.onChange(e.currentTarget.value)}>
      <For each={CHOICES.includes(props.value) ? CHOICES : [props.value, ...CHOICES]}>
        {(ms) => <option value={ms}>{label(ms)}</option>}
      </For>
    </select>
  );
}

function HooksView(props: { ctx: ExtensionsCtx }) {
  const [event, setEvent] = createSignal<string>("pre_tool_use");
  const [matcher, setMatcher] = createSignal("");
  const [command, setCommand] = createSignal("");
  const [timeout, setTimeoutMs] = createSignal("10000");
  const [failureMode, setFailureMode] = createSignal<"open" | "closed">("open");
  const [busy, setBusy] = createSignal(false);
  const [busyIndex, setBusyIndex] = createSignal(-1);
  const [editingIndex, setEditingIndex] = createSignal(-1);
  const [editEvent, setEditEvent] = createSignal<string>("pre_tool_use");
  const [editMatcher, setEditMatcher] = createSignal("");
  const [editCommand, setEditCommand] = createSignal("");
  const [editTimeout, setEditTimeout] = createSignal("10000");
  const [editFailureMode, setEditFailureMode] = createSignal<"open" | "closed">("open");

  const addHook = async () => {
    const cmd = command().trim();
    if (!cmd || busy()) return;
    setBusy(true);
    try {
      await api.putHooks([
        ...props.ctx.hooks(),
        {
          event: event(),
          matcher: matcher().trim() || null,
          command: cmd,
          timeout_ms: parseInt(timeout(), 10) || 10_000,
          enabled: true,
          failure_mode: failureMode(),
        },
      ], props.ctx.scope());
      await props.ctx.refetchHooks();
      pushToast("info", `Added — runs ${(HOOK_EVENT_LABELS[event()] ?? event()).toLowerCase()}`);
      setCommand("");
      setMatcher("");
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const removeHook = async (index: number) => {
    if (!confirmDestructive("Remove this automation? The script itself stays on disk.")) return;
    const next = [...props.ctx.hooks()];
    next.splice(index, 1);
    try {
      await api.putHooks(next, props.ctx.scope());
      await props.ctx.refetchHooks();
      pushToast("info", "Automation removed");
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  // `PUT /config/hooks` replaces the whole list — there is no per-hook
  // endpoint — so a toggle, an edit, and a delete are all "patch this one
  // element, write back the array."
  const replaceHook = async (index: number, patch: Partial<HookConfig>, ok: string) => {
    setBusyIndex(index);
    const next = [...props.ctx.hooks()];
    const current = next[index];
    if (!current) {
      setBusyIndex(-1);
      return;
    }
    next[index] = { ...current, ...patch };
    try {
      await api.putHooks(next, props.ctx.scope());
      await props.ctx.refetchHooks();
      pushToast("info", ok);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusyIndex(-1);
    }
  };

  const beginEdit = (index: number, hook: HookConfig) => {
    // Fields first: `<Show ... keyed>` mounts the form the moment the index
    // is set, and MatcherBuilder reads its value at construction.
    setEditEvent(hook.event);
    setEditMatcher(hook.matcher ?? "");
    setEditCommand(hook.command);
    setEditTimeout(String(hook.timeout_ms));
    setEditFailureMode(hook.failure_mode ?? "open");
    setEditingIndex(index);
  };

  const saveEdit = async () => {
    const index = editingIndex();
    const cmd = editCommand().trim();
    if (index < 0 || !cmd) return;
    await replaceHook(
      index,
      {
        event: editEvent(),
        matcher: editMatcher().trim() || null,
        command: cmd,
        timeout_ms: parseInt(editTimeout(), 10) || 10_000,
        failure_mode: editFailureMode(),
      },
      "Automation updated",
    );
    setEditingIndex(-1);
  };

  return (
    <>
      <div class="toolbar">
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refetchHooks()}>Refresh</button>
      </div>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Automations</h2>
            <p class="dim">
              An automation runs one of your own scripts at a point in the turn. Treat these as
              powerful: a script that runs before a tool can stop that tool outright, and every one
              of them runs as vak itself — outside the sandbox, and outside every permission rule on
              this page. Only what you choose under “Runs on” narrows when one fires.
            </p>
            <p class="dim">
              This lists only the automations set up for this workspace. One set up for your whole
              account still runs here, but isn’t shown or editable from this page, so saving here
              can never quietly copy it into this workspace.
            </p>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.hooksLoading()}>
            <table class="table automation-table">
              <thead><tr><th>on</th><th>when</th><th>fires on</th><th>runs</th><th>give up after</th><th /></tr></thead>
              <tbody><SkeletonRows cols={6} /></tbody>
            </table>
          </Match>

          <Match when={props.ctx.hooksError()}>
            <div class="error-state" role="alert">Could not load automations: {String(props.ctx.hooksError())} <button class="ghost small" type="button" onClick={() => props.ctx.refetchHooks()}>Retry</button></div>
          </Match>

          <Match when={props.ctx.hooks().length === 0}>
            <div class="empty empty-teach">
              <strong>No automations are set up.</strong>
              <p>
Nothing runs alongside your turns. Add one to keep a record of what vak does, to stop a
                particular command before it runs, or to give every new session some standing context.
              </p>
            </div>
          </Match>

          <Match when={props.ctx.hooks().length > 0}>
            <table class="table automation-table">
              <thead><tr><th>on</th><th>when</th><th>fires on</th><th>runs</th><th>give up after</th><th /></tr></thead>
              <tbody>
                <For each={props.ctx.hooks()}>
                  {(hook, index) => {
                    const scope = createMemo(() =>
                      hook.matcher ? parseRule(hook.matcher, "allow") : null,
                    );
                    const isBusy = () => busyIndex() === index();
                    return (
                      <tr class="row-static" classList={{ dim: !hook.enabled }}>
                        <td>
                          <label class="toggle" title={hook.enabled ? "Turn it off without removing it" : "Turn it back on"}>
                            <input
                              type="checkbox"
                              checked={hook.enabled}
                              disabled={isBusy()}
                              onChange={(e) =>
                                void replaceHook(
                                  index(),
                                  { enabled: e.currentTarget.checked },
                                  e.currentTarget.checked ? "Automation on" : "Automation off",
                                )
                              }
                            />
                          </label>
                        </td>
                        <td><span class="chip chip-phrase chip-mode" title={hook.event}>{HOOK_EVENT_LABELS[hook.event] ?? hook.event}</span></td>
                        <td>
                          <Switch>
                            <Match when={!hook.matcher}>
                              <span class="chip chip-tone-warning">every tool call</span>
                            </Match>
                            {/* Said plainly, with the verbatim matcher on
                                hover: the stored string's glob half is
                                case-significant, so it stays reachable. The
                                parse only decides whether the engine will
                                accept it at all. */}
                            <Match when={scope()}>
                              <span class="chip chip-phrase" title={hook.matcher ?? ""}>{describeMatcher(hook.matcher)}</span>
                            </Match>
                            <Match when={!scope()}>
                              <span class="chip chip-tone-danger mono" title="Vak cannot read this pattern, so this automation will not run. Edit it to fix.">
                                {hook.matcher} · not valid
                              </span>
                            </Match>
                          </Switch>
                        </td>
                        <td class="dim col-command">
                          <span class="path" title={hook.command}>{hook.command}</span>
                        </td>
                        <td class="dim">{hook.timeout_ms < 1000 ? `${hook.timeout_ms}ms` : shortDuration(hook.timeout_ms / 1000)}</td>
                        <td>
                          <div class="row-gap">
                            <button class="ghost small" disabled={isBusy()} onClick={() => beginEdit(index(), hook)}>
                              Edit
                            </button>
                            <button class="danger small" disabled={isBusy()} onClick={() => void removeHook(index())}>
                              Remove
                            </button>
                          </div>
                        </td>
                      </tr>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>

        <Show when={editingIndex() >= 0 ? String(editingIndex()) : ""} keyed>
          <div class="edit-form" aria-label="Edit automation">
            <div class="panel-title-row">
              <div>
                <h3>Edit automation</h3>
                <p class="dim">Changes apply to the next matching call.</p>
              </div>
              <button class="ghost small" onClick={() => setEditingIndex(-1)}>Cancel</button>
            </div>
            <div class="form-row">
              <label>When</label>
              <select value={editEvent()} onChange={(e) => setEditEvent(e.currentTarget.value)}>
                <For each={HOOK_EVENTS}>{(ev) => <option value={ev}>{HOOK_EVENT_LABELS[ev] ?? ev}</option>}</For>
              </select>
            </div>
            <MatcherBuilder value={editMatcher()} onChange={setEditMatcher} tools={MATCHER_TOOLS} />
            <div class="form-row">
              <label>Script to run</label>
              <input class="mono" value={editCommand()} onInput={(e) => setEditCommand(e.currentTarget.value)} />
            </div>
            <div class="form-row">
              <label>Give up after</label>
              <TimeoutPicker value={editTimeout()} onChange={setEditTimeout} />
            </div>
            <div class="form-row">
              <label>On hook failure</label>
              <select value={editFailureMode()} onChange={(e) => setEditFailureMode(e.currentTarget.value as "open" | "closed")}>
                <option value="open">Continue and report</option>
                <option value="closed">Block the operation</option>
              </select>
            </div>
            <button disabled={busyIndex() === editingIndex() || !editCommand().trim()} onClick={() => void saveEdit()}>
              {busyIndex() === editingIndex() ? "Saving…" : "Save changes"}
            </button>
          </div>
        </Show>

        <details class="advanced">
          <summary>Add an automation</summary>
          <p class="dim">
            A hook runs your own script at a point in the turn. Pick when it fires and which calls
            it should see; the script decides the rest.
          </p>
          <div class="form-row">
            <label>When</label>
            <select value={event()} onChange={(e) => setEvent(e.currentTarget.value)}>
              <For each={HOOK_EVENTS}>{(ev) => <option value={ev}>{HOOK_EVENT_LABELS[ev] ?? ev}</option>}</For>
            </select>
          </div>
          <MatcherBuilder value={matcher()} onChange={setMatcher} tools={MATCHER_TOOLS} />
          <div class="form-row">
            <label>Script to run</label>
            <input class="mono" placeholder="/path/to/script.sh" value={command()} onInput={(e) => setCommand(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>Give up after</label>
            <TimeoutPicker value={timeout()} onChange={setTimeoutMs} />
          </div>
          <div class="form-row">
            <label>On hook failure</label>
            <select value={failureMode()} onChange={(e) => setFailureMode(e.currentTarget.value as "open" | "closed")}>
              <option value="open">Continue and report</option>
              <option value="closed">Block the operation</option>
            </select>
          </div>
          <div class="row-gap" style="margin-top:12px">
            <button disabled={busy() || !command().trim()} onClick={() => void addHook()}>
              {busy() ? "Adding…" : "Add automation"}
            </button>
          </div>
        </details>
      </section>
    </>
  );
}

// ---- Extensions › Scheduled tasks ------------------------------------------

function TasksView(props: { ctx: ExtensionsCtx }) {
  const [name, setName] = createSignal("");
  const [prompt, setPrompt] = createSignal("");
  const [schedule, setSchedule] = createSignal("");
  const [modelPin, setModelPin] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [busyId, setBusyId] = createSignal("");
  const [editingId, setEditingId] = createSignal("");
  const [editName, setEditName] = createSignal("");
  const [editPrompt, setEditPrompt] = createSignal("");
  const [editSchedule, setEditSchedule] = createSignal("");
  const [editModelPin, setEditModelPin] = createSignal("");
  /// Set only for a task that runs on a plain interval rather than a cron.
  const [editInterval, setEditInterval] = createSignal<number | null>(null);

  const guard = async (id: string, work: () => Promise<void>, ok: string) => {
    setBusyId(id);
    try {
      await work();
      await props.ctx.refetchTasks();
      pushToast("info", ok);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusyId("");
    }
  };

  const createTask = async () => {
    if (!name().trim() || !prompt().trim() || busy()) return;
    setBusy(true);
    try {
      await api.createTask({
        name: name().trim(),
        prompt: prompt().trim(),
        schedule: schedule().trim() || undefined,
        model_pin: modelPin().trim() || undefined,
      });
      pushToast("info", `Scheduled ‘${name().trim()}’`);
      setName("");
      setPrompt("");
      setSchedule("");
      setModelPin("");
      await props.ctx.refetchTasks();
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const beginEdit = (task: TaskItem) => {
    // Fields before the id, for the same reason as the hooks editor above.
    setEditName(task.name);
    setEditPrompt(task.prompt ?? "");
    setEditSchedule(task.schedule ?? "");
    setEditInterval(task.schedule ? null : (task.interval_secs ?? null));
    setEditModelPin(task.model_pin ?? "");
    setEditingId(task.id);
  };

  const saveEdit = async () => {
    const id = editingId();
    if (!id || !editName().trim() || !editPrompt().trim() || busyId()) return;
    setBusyId(id);
    try {
      await api.patchTask(id, {
        name: editName().trim(),
        prompt: editPrompt().trim(),
        schedule: editSchedule().trim() || null,
        model_pin: editModelPin().trim() || null,
      });
      await props.ctx.refetchTasks();
      pushToast("info", "Task updated");
      setEditingId("");
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setBusyId("");
    }
  };

  return (
    <>
      <div class="toolbar">
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refetchTasks()}>Refresh</button>
      </div>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Scheduled tasks</h2>
            <p class="dim">
              Runs vak starts on its own, with nobody watching. Each one runs under the same
              permission mode as any other turn — <strong>{props.ctx.mode()}</strong> — so a task
              that needs an approval simply waits for one.
            </p>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.tasksLoading()}>
            <table class="table">
              <thead><tr><th>task</th><th>type</th><th>runs</th><th>model</th><th>last run</th><th>state</th><th /></tr></thead>
              <tbody><SkeletonRows cols={7} /></tbody>
            </table>
          </Match>

          <Match when={props.ctx.tasks().length === 0}>
            <div class="empty empty-teach">
              <strong>Nothing is scheduled.</strong>
              <p>
A scheduled task is something you ask vak to do on a repeating schedule — a nightly
                digest, a weekly check — with the result filed in your Inbox.
              </p>
            </div>
          </Match>

          <Match when={props.ctx.tasks().length > 0}>
            <table class="table">
              <thead><tr><th>task</th><th>type</th><th>runs</th><th>model</th><th>last run</th><th>state</th><th /></tr></thead>
              <tbody>
                <For each={props.ctx.tasks()}>
                  {(t) => (
                    <tr class="row-static">
                      <td class="bold">{t.name}</td>
                      <td><span class={`chip ${t.script ? "chip-tool" : "chip-mode"}`}>{t.script ? "runs a script" : "asks vak"}</span></td>
                      <td class="dim" title={t.schedule ?? ""}>
                        {t.schedule ? describeSchedule(t.schedule) : `Every ${describeDuration(t.interval_secs ?? 3600)}`}
                      </td>
                      <td class="dim">{t.model_pin ?? "Workspace default"}</td>
                      <td class="dim" title={t.last_run_at ?? ""}>{t.last_run_at ? timeAgo(t.last_run_at) : "never"}</td>
                      <td>
                        <span class={t.enabled ? "chip chip-tone-success" : "chip"}>
                          {t.enabled ? "on" : "paused"}
                        </span>
                      </td>
                      <td>
                        <div class="row-gap">
                          <button
                            class="ghost small"
                            disabled={busyId() === t.id}
                            onClick={() => void guard(t.id, () => api.runTaskNow(t.id), `Ran ‘${t.name}’`)}
                          >
                            Run now
                          </button>
                          <button
                            class="ghost small"
                            disabled={busyId() === t.id}
                            onClick={() =>
                              void guard(
                                t.id,
                                () => api.patchTask(t.id, { enabled: !t.enabled }),
                                t.enabled ? "Task paused" : "Task enabled",
                              )
                            }
                          >
                            {t.enabled ? "Pause" : "Enable"}
                          </button>
                          <button class="ghost small" disabled={busyId() === t.id} onClick={() => beginEdit(t)}>
                            Edit
                          </button>
                          <button
                            class="danger small"
                            disabled={busyId() === t.id}
                            onClick={() => {
                              if (confirmDestructive(`Delete task “${t.name}”?`)) {
                                void guard(t.id, () => api.deleteTask(t.id), "Task deleted");
                              }
                            }}
                          >
                            Delete
                          </button>
                        </div>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>

        <Show when={editingId()} keyed>
          <div class="edit-form" aria-label="Edit scheduled task">
            <div class="panel-title-row">
              <div>
                <h3>Edit task</h3>
                <p class="dim">Changes apply to the next run. Existing session history is preserved.</p>
              </div>
              <button class="ghost small" onClick={() => setEditingId("")}>Cancel</button>
            </div>
            <div class="form-row"><label>Name</label><input value={editName()} onInput={(e) => setEditName(e.currentTarget.value)} /></div>
            <div class="form-row"><label>What to do</label><textarea rows={3} value={editPrompt()} onInput={(e) => setEditPrompt(e.currentTarget.value)} /></div>
            <Show when={editInterval() != null && !editSchedule().trim()}>
              <p class="dim">
                This task runs every {describeDuration(editInterval())} at the moment. Leave the
                schedule alone to keep that, or pick one below to switch it over.
              </p>
            </Show>
            <ScheduleBuilder value={editSchedule()} onChange={setEditSchedule} />
            <div class="form-row"><label>Model</label><input class="mono" placeholder="Workspace default" value={editModelPin()} onInput={(e) => setEditModelPin(e.currentTarget.value)} /></div>
            <button disabled={busyId() === editingId() || !editName().trim() || !editPrompt().trim()} onClick={() => void saveEdit()}>
              {busyId() === editingId() ? "Saving…" : "Save changes"}
            </button>
          </div>
        </Show>

        <details class="advanced">
          <summary>Schedule a task</summary>
          <div class="form-row">
            <label>Name</label>
            <input placeholder="Nightly digest, dependency audit…" value={name()} onInput={(e) => setName(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>What to do</label>
            <textarea rows={3} placeholder="What should vak do each time this runs?" value={prompt()} onInput={(e) => setPrompt(e.currentTarget.value)} />
          </div>
          <ScheduleBuilder value={schedule()} onChange={setSchedule} />
          <div class="form-row">
            <label>Model</label>
            <input class="mono" placeholder="Optional — otherwise the workspace default" value={modelPin()} onInput={(e) => setModelPin(e.currentTarget.value)} />
          </div>
          <div class="row-gap" style="margin-top:12px">
            <button disabled={busy() || !name().trim() || !prompt().trim()} onClick={() => void createTask()}>
              {busy() ? "Scheduling…" : "Schedule task"}
            </button>
          </div>
        </details>
      </section>
    </>
  );
}

// ---- Extensions section shell ----------------------------------------------

function ExtensionsSection() {
  const [config] = createResource(() => api.config());
  const [mcp, mcpActions] = createResource(configScope, (scope) => api.mcpServers(scope));
  const [hooks, hooksActions] = createResource(configScope, (scope) => api.hooks(scope));
  const [skills, skillsActions] = createResource(() => api.skills());
  const [plugins, pluginsActions] = createResource(configScope, (scope) => api.plugins(scope === "user" ? "user" : "workspace"));
  const [proposals, proposalsActions] = createResource(() => api.skillProposals());
  const [tasks, tasksActions] = createResource(() => api.tasks());

  const ctx: ExtensionsCtx = {
    scope: configScope,
    plugins: () => plugins()?.plugins ?? [],
    pluginsLoading: () => plugins.loading,
    refetchPlugins: async () => { await pluginsActions.refetch(); },
    mcp: () => mcp()?.servers ?? {},
    mcpLoading: () => mcp.loading,
    mcpError: () => mcp.error,
    refetchMcp: async () => { await mcpActions.refetch(); },
    hooks: () => hooks()?.hooks ?? [],
    hooksLoading: () => hooks.loading,
    hooksError: () => hooks.error,
    refetchHooks: async () => { await hooksActions.refetch(); },
    skills: () => skills()?.skills ?? [],
    skillsLoading: () => skills.loading,
    proposals: () => proposals()?.proposals ?? [],
    proposalsLoading: () => proposals.loading,
    refetchSkills: async () => { await Promise.all([skillsActions.refetch(), proposalsActions.refetch()]); },
    tasks: () => tasks()?.tasks ?? [],
    tasksLoading: () => tasks.loading,
    refetchTasks: async () => { await tasksActions.refetch(); },
    rules: createMemo(() => parseRuleLists(config()?.permissions)),
    mode: () => config()?.permission_mode ?? "WorkspaceWrite",
    rulesKnown: () => rulesReported(config()),
  };

  const count = (n: number, loading: boolean) => (loading ? "" : ` ${n}`);

  // An errored fetch and a genuinely empty extension point look identical
  // once the resource settles, and "nothing is configured" is the more
  // alarming of the two to report wrongly. Name the failure instead.
  const failure = createMemo(() => {
    for (const [what, res] of [
      ["configuration", config],
      ["connected apps", mcp],
      ["automations", hooks],
      ["skills", skills],
      ["plugins", plugins],
      ["skill proposals", proposals],
      ["scheduled tasks", tasks],
    ] as const) {
      if (res.error) return { what, error: `${res.error}` };
    }
    return null;
  });

  createEffect(() => {
    if (failure() && (config.error instanceof AuthRequired || mcp.error instanceof AuthRequired)) {
      setAuthed(false);
    }
  });

  return (
    <div class="view extension-shell">
      <PageHeader
        title="Extensions"
        description="Give vak better ways to work — with clear boundaries, visible provenance, and one place to manage every capability."
      />
      <div class="extension-overview" aria-label="Extension summary">
        <div><span class="extension-overline">CAPABILITY HUB</span><strong>Everything vak can reach</strong><span>Connected apps, instructions, automations, and background work.</span></div>
        <div class="extension-stat"><strong>{Object.keys(ctx.mcp()).length}</strong><span>connected apps</span></div>
        <div class="extension-stat"><strong>{ctx.skills().length}</strong><span>loaded skills</span></div>
        <div class="extension-stat"><strong>{ctx.hooks().length + ctx.tasks().length}</strong><span>automations</span></div>
      </div>
      <Show when={failure()}>
        <section class="panel panel-alert callout" style="margin-bottom:14px">
          <div>
            <strong>Could not read {failure()!.what}</strong>
            <p class="dim">
              {failure()!.error} — what is shown below may be incomplete, so treat an empty list as
              unknown rather than as nothing configured.
            </p>
          </div>
        </section>
      </Show>

      <div class="tab-bar extension-tabs" aria-label="Extension types">
        <For each={EXTENSION_TABS}>
          {(t) => (
            <a class="tab-btn" href={t.hash} classList={{ active: extensionsTab() === t.hash }}>
              {t.label}
              <Switch>
                <Match when={t.hash === "#/integrations"}>
                  <span class="tab-count">{count(Object.keys(ctx.mcp()).length, ctx.mcpLoading())}</span>
                </Match>
                <Match when={t.hash === "#/integrations/skills"}>
                  <span class="tab-count">{count(ctx.skills().length, ctx.skillsLoading())}</span>
                  <Show when={ctx.proposals().length > 0}>
                    <span class="nav-badge">{ctx.proposals().length}</span>
                  </Show>
                </Match>
                <Match when={t.hash === "#/integrations/hooks"}>
                  <span class="tab-count">{count(ctx.hooks().length, ctx.hooksLoading())}</span>
                </Match>
                <Match when={t.hash === "#/integrations/tasks"}>
                  <span class="tab-count">{count(ctx.tasks().length, ctx.tasksLoading())}</span>
                </Match>
              </Switch>
            </a>
          )}
        </For>
      </div>

      <Switch>
        <Match when={extensionsTab() === "#/integrations"}><McpServersView ctx={ctx} /></Match>
        <Match when={extensionsTab() === "#/integrations/plugins"}><PluginsView ctx={ctx} /></Match>
        <Match when={extensionsTab() === "#/integrations/skills"}><SkillsView ctx={ctx} /></Match>
        <Match when={extensionsTab() === "#/integrations/hooks"}><HooksView ctx={ctx} /></Match>
        <Match when={extensionsTab() === "#/integrations/tasks"}><TasksView ctx={ctx} /></Match>
      </Switch>
    </div>
  );
}

// ---- Memory & Recall -------------------------------------------------------

/// Age → lifecycle bucket for the freshness dot on each note card. Memory
/// notes never expire on their own (docs/design/23-memory.md), so this is
/// purely a "how long has vak been carrying this" signal, not a TTL.
function noteAgeBucket(ts: string): "fresh" | "aging" | "stale" {
  const days = (Date.now() - new Date(ts).getTime()) / 86_400_000;
  if (days < 7) return "fresh";
  if (days < 30) return "aging";
  return "stale";
}

function KnowledgeSubNav(props: { active: "#/memory" | "#/feeds" | "#/search" }) {
  return (
    <div class="knowledge-subnav">
      <a
        href="#/memory"
        class="knowledge-tab-btn"
        classList={{ active: props.active === "#/memory" }}
      >
        Memory & Profile
      </a>
      <a
        href="#/feeds"
        class="knowledge-tab-btn"
        classList={{ active: props.active === "#/feeds" }}
      >
        Scheduled Feeds
      </a>
      <a
        href="#/search"
        class="knowledge-tab-btn"
        classList={{ active: props.active === "#/search" }}
      >
        Global Search
      </a>
    </div>
  );
}

const MEMORY_PRESETS = [
  {
    label: "Tone & Brevity",
    tag: "tone",
    text: "Prefer direct, dense, and structured prose. Skip conversational pleasantries, flattery, and apologies.",
  },
  {
    label: "Tech Stack & Tools",
    tag: "stack",
    text: "This workspace uses Rust 2024, SolidJS, and Vanilla CSS. Do not introduce TailwindCSS.",
  },
  {
    label: "Security & Secrets",
    tag: "security",
    text: "Never print, log, or commit API keys or auth tokens. Ask for human confirmation before deleting files.",
  },
  {
    label: "Architecture Constraint",
    tag: "architecture",
    text: "Preserve append-only ledger; do not rewrite or delete historical session records.",
  },
] as const;

function MemoryView() {
  const [memoryData, { refetch }] = createResource(() => api.memory());
  const [configData, { refetch: refetchConfig }] = createResource(() => api.config());
  const [layer, { refetch: refetchLayer }] = createResource(configScope, (scope) => api.configLayer(scope));
  const [scope, setScope] = createSignal<"profile" | "project">("project");
  const [tag, setTag] = createSignal("");
  const [noteText, setNoteText] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [editing, setEditing] = createSignal<string | null>(null);
  const [editText, setEditText] = createSignal("");
  const [cleaning, setCleaning] = createSignal(false);
  const [togglingFlag, setTogglingFlag] = createSignal<string | null>(null);
  const [filterScope, setFilterScope] = createSignal<"all" | "profile" | "workspace">("all");
  const [filterFreshness, setFilterFreshness] = createSignal<"all" | "fresh" | "stale">("all");
  const [query, setQuery] = createSignal("");

  const notes = createMemo(() => memoryData()?.notes ?? []);
  const profileCount = createMemo(() => notes().filter((n) => n.scope === "profile").length);
  const projectCount = createMemo(() => notes().filter((n) => n.scope !== "profile").length);
  const staleCount = createMemo(() => notes().filter((n) => noteAgeBucket(n.ts) === "stale").length);
  const oldestAgo = createMemo(() => {
    const list = notes();
    if (!list.length) return "—";
    const oldest = list.reduce((a, b) => (new Date(a.ts) < new Date(b.ts) ? a : b));
    return timeAgo(oldest.ts);
  });
  const visibleNotes = createMemo(() => {
    const q = query().trim().toLowerCase();
    return notes().filter((n) => {
      if (filterScope() !== "all") {
        const bucket = n.scope === "profile" ? "profile" : "workspace";
        if (bucket !== filterScope()) return false;
      }
      if (filterFreshness() !== "all") {
        const age = noteAgeBucket(n.ts);
        if (filterFreshness() === "fresh" && age === "stale") return false;
        if (filterFreshness() === "stale" && age !== "stale") return false;
      }
      if (q && !n.text.toLowerCase().includes(q) && !(n.tag ?? "").toLowerCase().includes(q)) return false;
      return true;
    });
  });

  // Real toggles, not a status readout: each flips exactly one [memory]
  // config key, persisted to .vak/config.toml and applied to the live
  // Core immediately (docs/design/23-memory.md) — no restart needed, and
  // no other flag is touched by this call.
  const toggleMemoryFlag = async (
    key: "memory_search_enabled" | "memory_write_enabled" | "memory_reflection" | "memory_skill_proposals",
    next: boolean,
  ) => {
    setTogglingFlag(key);
    try {
      await api.patchConfigScope(configScope(), { [key]: next });
      await Promise.all([refetchConfig(), refetchLayer()]);
      pushToast("info", `${next ? "Enabled" : "Disabled"} in ${configScope() === "user" ? "Global" : "Workspace"}`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setTogglingFlag(null);
    }
  };

  const cleanArtifacts = async () => {
    if (!confirmDestructive("Remove only abandoned memory lock/temp files and empty workspace folders? Notes will not be deleted.")) return;
    setCleaning(true);
    try {
      const report = await api.cleanupMemory();
      pushToast("info", `Cleaned ${report.removed_locks} locks, ${report.removed_temps} temp files, ${report.removed_empty_dirs} empty folders`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setCleaning(false);
    }
  };

  const addNote = async () => {
    if (!noteText().trim() || busy()) return;
    setBusy(true);
    try {
      await api.addMemory(scope(), noteText().trim(), tag().trim() || undefined);
      await refetch();
      pushToast("info", "Vak will remember that");
      setNoteText("");
      setTag("");
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const forget = async (id: string) => {
    if (!confirmDestructive("Forget this note? Vak stops taking it into account.")) return;
    try {
      const note = memoryData()?.notes?.find((m) => m.id === id);
      await api.forgetMemory(id, note?.scope === "profile" ? "profile" : "workspace");
      await refetch();
      pushToast("info", "Forgotten");
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const flagCards: Array<{
    key: "memory_search_enabled" | "memory_write_enabled" | "memory_reflection" | "memory_skill_proposals";
    on: () => boolean;
    setHere: () => boolean;
    title: string;
    desc: string;
  }> = [
    { key: "memory_search_enabled", on: () => configData()?.memory?.search_enabled ?? false, setHere: () => layer()?.memory?.search_enabled === true, title: "Search past sessions", desc: "Lets vak look up earlier conversations mid-run." },
    { key: "memory_write_enabled", on: () => configData()?.memory?.write_enabled ?? false, setHere: () => layer()?.memory?.write_enabled === true, title: "Write notes", desc: "Lets vak save what it learns mid-run, not just what you add here." },
    { key: "memory_reflection", on: () => configData()?.memory?.reflection ?? false, setHere: () => layer()?.memory?.reflection === true, title: "Reflect after each run", desc: "A short pass proposing notes/skills from what just happened." },
    { key: "memory_skill_proposals", on: () => configData()?.memory?.skill_proposals ?? false, setHere: () => layer()?.memory?.skill_proposals === true, title: "Propose skills", desc: "Lets reflection suggest new skills for you to review." },
  ];

  return (
    <div class="view">
      <KnowledgeSubNav active="#/memory" />
      <PageHeader
        title="Memory"
        description="Things vak should keep in mind between sessions — about you, or about this workspace."
        actions={<button class="ghost small" disabled={cleaning()} onClick={() => void cleanArtifacts()}>{cleaning() ? "Cleaning…" : "Clean artifacts"}</button>}
      />

      <div class="stat-strip">
        <StatCard label="Total memories" value={notes().length} />
        <StatCard label="About me" value={profileCount()} sub="carried across every workspace" />
        <StatCard label="About this workspace" value={projectCount()} />
        <StatCard label="Oldest note" value={oldestAgo()} tone={staleCount() > 0 ? "warn" : undefined} sub={staleCount() > 0 ? `${staleCount()} unrevisited 30+ days` : undefined} />
      </div>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Governance</h2>
            <p class="dim" style="margin-top:-4px">What vak is allowed to do with memory, live — no restart needed. Writing here affects <strong>{configScope() === "user" ? "Global" : "Workspace"}</strong> scope.</p>
          </div>
        </div>
        <div class="toggle-card-grid" style="margin-top:10px">
          <For each={flagCards}>
            {(f) => (
              <div class="toggle-card" data-on={f.on()}>
                <div class="toggle-card-body">
                  <strong>{f.title}</strong>
                  <span>{f.desc}</span>
                  <Show when={() => f.setHere()}>
                    <span class="chip chip-tone-success" style="margin-top:4px">set here</span>
                  </Show>
                  <Show when={() => !f.setHere()}>
                    <span class="chip chip-tone-warning" style="margin-top:4px">inherited</span>
                  </Show>
                </div>
                <label class="toggle">
                  <input
                    type="checkbox"
                    aria-label={f.title}
                    checked={f.on()}
                    disabled={configData.loading || togglingFlag() === f.key}
                    onChange={(e) => void toggleMemoryFlag(f.key, e.currentTarget.checked)}
                  />
                </label>
              </div>
            )}
          </For>
        </div>
      </section>

      <div class="two-col">
        <section class="panel">
          <h2>What vak remembers ({notes().length})</h2>
          <div class="memory-toolbar">
            <input class="search-input" placeholder="Filter by text or label…" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} />
            <div class="scope-tabs">
              <button classList={{ active: filterScope() === "all" }} onClick={() => setFilterScope("all")}>All</button>
              <button classList={{ active: filterScope() === "profile" }} onClick={() => setFilterScope("profile")}>About me</button>
              <button classList={{ active: filterScope() === "workspace" }} onClick={() => setFilterScope("workspace")}>This workspace</button>
            </div>
            <div class="scope-tabs" style="margin-left: 4px;">
              <button classList={{ active: filterFreshness() === "all" }} onClick={() => setFilterFreshness("all")}>All time</button>
              <button classList={{ active: filterFreshness() === "fresh" }} onClick={() => setFilterFreshness("fresh")}>Fresh</button>
              <button classList={{ active: filterFreshness() === "stale" }} onClick={() => setFilterFreshness("stale")}>Stale (30d+)</button>
            </div>
          </div>
          <Show when={!memoryData.loading} fallback={<div class="empty">Loading…</div>}>
            <Show when={!memoryData.error} fallback={<LoadError message={`${memoryData.error}`} onRetry={() => refetch()} />}>
            <Show when={notes().length > 0} fallback={<div class="empty">Nothing remembered yet. Add a note on the right.</div>}>
            <Show when={visibleNotes().length > 0} fallback={<div class="empty">No memories match that filter.</div>}>
              <ul class="hit-list">
                <For each={visibleNotes()}>
                  {(m: MemoryItem) => (
                    <li class={`note-card ${m.scope === "profile" ? "note-profile" : "note-workspace"}`}>
                      <div class="note-head">
                        <span class="note-freshness" data-age={noteAgeBucket(m.ts)} title={`Last touched ${timeAgo(m.ts)}`} />
                        <span class={`chip ${m.scope === "profile" ? "chip-mode" : "chip-tool"}`} title={m.scope}>{m.scope === "profile" ? "about me" : "about this workspace"}</span>
                        <Show when={m.tag}><strong class="mono">{m.tag}</strong></Show>
                        <span class="dim" style="margin-left:auto; font-size:11px">{timeAgo(m.ts)}</span>
                      </div>
                      <Show when={editing() === m.id} fallback={<div class="note-text">{m.text}</div>}>
                        <textarea value={editText()} onInput={(e) => setEditText(e.currentTarget.value)} />
                        <button class="small" onClick={async () => {
                          if (!editText().trim()) return;
                          try {
                            await api.amendMemory(m.id, m.scope === "profile" ? "profile" : "workspace", editText().trim());
                            await refetch();
                            setEditing(null);
                            pushToast("info", "Memory amended");
                          } catch (err) {
                            pushToast("alert", `${err}`);
                          }
                        }}>Save</button>
                      </Show>
                      <div class="note-foot">
                        <Show when={editing() !== m.id}>
                          <button class="small" onClick={() => { setEditing(m.id); setEditText(m.text); }}>Amend</button>
                        </Show>
                        <Show when={m.session_id}>
                          <button class="ghost small" onClick={() => navigate(`#/sessions/${m.session_id}`)}>From this session</button>
                        </Show>
                        <span class="spacer" />
                        <button class="danger small" onClick={() => forget(m.id)}>Forget</button>
                      </div>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
            </Show>
            </Show>
          </Show>
        </section>

        <section class="panel">
          <div class="panel-title-row">
            <div>
              <h2>Add a note</h2>
              <p class="dim">Vak reads these at the start of every session.</p>
            </div>
          </div>
          <div class="form-row">
            <label>Applies to</label>
              <select aria-label="Memory note scope" value={scope()} onChange={(e) => setScope(e.currentTarget.value as "profile" | "project")}>
              <option value="project">Workspace</option>
              <option value="profile">Me, in every workspace</option>
            </select>
          </div>
          <div class="form-row">
            <label>Label</label>
            <input placeholder="Optional label, e.g. billing" value={tag()} onInput={(e) => setTag(e.currentTarget.value)} />
          </div>
          <div class="form-row">
            <label>Note</label>
            <textarea rows={4} placeholder="What should vak remember?" value={noteText()} onInput={(e) => setNoteText(e.currentTarget.value)} />
          </div>

          {/* Quick Preset Templates */}
          <div class="memory-presets">
            <span class="dim small">Quick-insert preset templates:</span>
            <div class="memory-preset-strip">
              <For each={MEMORY_PRESETS}>
                {(preset) => (
                  <button
                    type="button"
                    class="memory-preset-btn"
                    onClick={() => {
                      setTag(preset.tag);
                      setNoteText(preset.text);
                    }}
                  >
                    + {preset.label}
                  </button>
                )}
              </For>
            </div>
          </div>

          <div class="row-gap" style="margin-top:14px">
            <button disabled={busy() || !noteText().trim()} onClick={addNote}>
              {busy() ? "Recording…" : "Save Note"}
            </button>
          </div>
        </section>
      </div>
    </div>
  );
}

// ---- FinOps -----------------------------------------------------------------

/// A self-contained SVG bar chart for the 14-day spend trend with interactive
/// hover tooltips, baseline average, and budget cap guides.
function SpendTrendChart(props: { points: FinOpsDailyPoint[]; capUsd: number | null }) {
  const width = 640;
  const height = 160;
  const padding = { top: 16, right: 14, bottom: 24, left: 44 };
  const plotW = width - padding.left - padding.right;
  const plotH = height - padding.top - padding.bottom;

  const [hovered, setHovered] = createSignal<{ date: string; usd: number; x: number; y: number } | null>(null);

  const total14d = createMemo(() => props.points.reduce((a, p) => a + p.usd, 0));
  const avgDaily = createMemo(() => (props.points.length ? total14d() / props.points.length : 0));
  const peakPoint = createMemo(() =>
    props.points.reduce((max, p) => (p.usd > max.usd ? p : max), { date: "—", usd: 0 })
  );

  const maxUsd = createMemo(() => {
    const max = Math.max(...props.points.map((p) => p.usd), props.capUsd ?? 0);
    return max > 0 ? max * 1.18 : 0.01;
  });

  const barW = createMemo(() => (props.points.length ? plotW / props.points.length : 0));
  const yFor = (usd: number) => padding.top + plotH * (1 - Math.min(usd, maxUsd()) / maxUsd());
  const capY = createMemo(() => (props.capUsd != null && props.capUsd <= maxUsd() ? yFor(props.capUsd) : null));
  const avgY = createMemo(() => (avgDaily() > 0 ? yFor(avgDaily()) : null));

  return (
    <div class="chart-container" onMouseLeave={() => setHovered(null)}>
      <div class="chart-meta-row" style="margin-bottom:10px">
        <span class="chart-badge">14d Spend: <strong>${total14d().toFixed(4)}</strong></span>
        <span class="chart-badge">Daily Avg: <strong>${avgDaily().toFixed(4)}</strong></span>
        <Show when={peakPoint().usd > 0}>
          <span class="chart-badge">Peak: <strong>${peakPoint().usd.toFixed(4)}</strong> ({peakPoint().date.slice(5)})</span>
        </Show>
        <Show when={props.capUsd != null}>
          <span class="chart-badge">Daily Cap: <strong>${props.capUsd!.toFixed(2)}</strong></span>
        </Show>
      </div>

      <Show when={hovered()}>
        {(h) => (
          <div
            class="chart-tooltip"
            style={{
              left: `${(h().x / width) * 100}%`,
              top: `${(h().y / height) * 100}%`,
            }}
          >
            <div style="font-weight:600; margin-bottom:2px">{h().date}</div>
            <div style="color:var(--text-soft)">Estimated: <strong>${h().usd.toFixed(4)}</strong></div>
            <Show when={props.capUsd != null && props.capUsd! > 0}>
              <div style="font-size:10px; color:var(--faint); margin-top:1px">
                {((h().usd / props.capUsd!) * 100).toFixed(1)}% of daily cap
              </div>
            </Show>
          </div>
        )}
      </Show>

      <svg
        viewBox={`0 0 ${width} ${height}`}
        width="100%"
        height={height}
        role="img"
        aria-label="Daily spend, last 14 days"
        style="overflow:visible"
      >
        {/* Baseline */}
        <line
          x1={padding.left}
          y1={padding.top + plotH}
          x2={width - padding.right}
          y2={padding.top + plotH}
          stroke="var(--border)"
          stroke-width="1"
        />

        {/* Avg dashed line */}
        <Show when={avgY() != null}>
          <line
            x1={padding.left}
            y1={avgY()!}
            x2={width - padding.right}
            y2={avgY()!}
            stroke="var(--faint)"
            stroke-width="1"
            stroke-dasharray="2 3"
            opacity="0.4"
          />
          <text
            x={padding.left + 4}
            y={avgY()! - 3}
            fill="var(--faint)"
            font-size="9"
            opacity="0.8"
          >
            avg
          </text>
        </Show>

        {/* Daily Cap dashed line */}
        <Show when={capY() != null}>
          <line
            x1={padding.left}
            y1={capY()!}
            x2={width - padding.right}
            y2={capY()!}
            stroke="var(--accent)"
            stroke-width="1"
            stroke-dasharray="4 3"
            opacity="0.75"
          />
          <text
            x={width - padding.right}
            y={capY()! - 4}
            text-anchor="end"
            fill="var(--accent)"
            font-size="10"
            font-weight="500"
          >
            cap ${props.capUsd!.toFixed(2)}
          </text>
        </Show>

        {/* Bars */}
        <For each={props.points}>
          {(p, i) => {
            const x = padding.left + i() * barW() + barW() * 0.15;
            const w = barW() * 0.7;
            const y = yFor(p.usd);
            const h = Math.max(padding.top + plotH - y, 2);
            const label = p.date.slice(5); // MM-DD
            const isHovered = () => hovered()?.date === p.date;

            return (
              <g
                style="cursor:pointer"
                onMouseEnter={() => setHovered({ date: p.date, usd: p.usd, x: x + w / 2, y })}
              >
                <rect
                  x={x}
                  y={y}
                  width={w}
                  height={h}
                  rx="2"
                  fill="var(--accent)"
                  opacity={isHovered() ? 1 : p.usd > 0 ? 0.85 : 0.25}
                  stroke={isHovered() ? "var(--text)" : "none"}
                  stroke-width="1"
                />
                <Show when={i() % 2 === 0 || props.points.length <= 8}>
                  <text
                    x={x + w / 2}
                    y={height - 6}
                    text-anchor="middle"
                    fill={isHovered() ? "var(--text)" : "var(--faint)"}
                    font-size="9.5"
                    font-weight={isHovered() ? "600" : "normal"}
                  >
                    {label}
                  </text>
                </Show>
              </g>
            );
          }}
        </For>
      </svg>
    </div>
  );
}

function SpendAllocationCard(props: { byProvider: FinOpsRollupEntry[]; byModel: FinOpsRollupEntry[] }) {
  const [mode, setMode] = createSignal<"provider" | "model">("provider");
  const items = createMemo(() => {
    const list = mode() === "provider" ? props.byProvider : props.byModel;
    return [...list].sort((a, b) => b.usd - a.usd || b.calls - a.calls);
  });
  const totalUsd = createMemo(() => items().reduce((a, r) => a + r.usd, 0));
  const maxUsd = createMemo(() => Math.max(...items().map((r) => r.usd), 0.00001));

  return (
    <section class="panel" style="display:flex; flex-direction:column">
      <div class="panel-title-row">
        <div>
          <h2>Spend allocation</h2>
          <p class="dim">Distribution across active dispatches today.</p>
        </div>
      </div>

      <div class="finops-tab-nav">
        <button
          class={`finops-tab-btn ${mode() === "provider" ? "active" : ""}`}
          onClick={() => setMode("provider")}
        >
          By Provider ({props.byProvider.length})
        </button>
        <button
          class={`finops-tab-btn ${mode() === "model" ? "active" : ""}`}
          onClick={() => setMode("model")}
        >
          By Model ({props.byModel.length})
        </button>
      </div>

      <div style="flex:1">
        <Show
          when={items().length > 0}
          fallback={<p class="dim" style="padding:16px 0">No dispatches recorded today.</p>}
        >
          <div class="allocation-list">
            <For each={items().slice(0, 5)}>
              {(item) => {
                const sharePct = totalUsd() > 0 ? (item.usd / totalUsd()) * 100 : 0;
                const barPct = (item.usd / maxUsd()) * 100;
                const totalTokens = item.input_tokens + item.output_tokens;
                return (
                  <div class="allocation-item">
                    <div class="allocation-header">
                      <span class="allocation-name" title={item.name}>{item.name || "(unknown)"}</span>
                      <div class="allocation-val">
                        <span>${item.usd.toFixed(4)}</span>
                        <span class="dim" style="margin-left:6px">({sharePct.toFixed(0)}%)</span>
                      </div>
                    </div>
                    <div class="allocation-bar-track">
                      <div
                        class="allocation-bar-fill"
                        style={{ width: `${Math.max(barPct, item.calls > 0 ? 3 : 0)}%` }}
                      />
                    </div>
                    <div style="display:flex; justify-content:space-between; font-size:10.5px; color:var(--faint)">
                      <span>{item.calls} {item.calls === 1 ? "call" : "calls"}</span>
                      <span>{totalTokens > 0 ? `${(totalTokens / 1000).toFixed(1)}k tokens` : "0 tokens"}</span>
                    </div>
                  </div>
                );
              }}
            </For>
          </div>
        </Show>
      </div>
    </section>
  );
}

function FinOpsRollupTable(props: { title: string; rows: FinOpsRollupEntry[] }) {
  const total = createMemo(() => props.rows.reduce((a, r) => a + r.usd, 0));
  return (
    <div>
      <div class="panel-title-row" style="margin-bottom:8px">
        <div>
          <h2>{props.title}</h2>
          <p class="dim">Usage and token breakdown today.</p>
        </div>
      </div>
      <Show when={props.rows.length > 0} fallback={<p class="dim">No dispatches today.</p>}>
        <div style="overflow-x:auto">
          <table class="table">
            <thead>
              <tr>
                <th>{props.title === "By provider" ? "provider" : "model"}</th>
                <th>calls</th>
                <th>input</th>
                <th>output</th>
                <th>cached</th>
                <th>est. usd</th>
                <th>share</th>
              </tr>
            </thead>
            <tbody>
              <For each={[...props.rows].sort((a, b) => b.usd - a.usd || b.calls - a.calls)}>
                {(r) => {
                  const share = total() > 0 ? (r.usd / total()) * 100 : 0;
                  return (
                    <tr>
                      <td class="mono" style="max-width:180px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap" title={r.name}>
                        {r.name || "(unknown)"}
                      </td>
                      <td>{r.calls}</td>
                      <td class="mono">{r.input_tokens.toLocaleString()}</td>
                      <td class="mono">{r.output_tokens.toLocaleString()}</td>
                      <td class="mono dim">
                        {r.cache_read_tokens > 0 ? (
                          <span style="color:var(--accent)">⚡ {r.cache_read_tokens.toLocaleString()}</span>
                        ) : (
                          "—"
                        )}
                      </td>
                      <td class="mono">${r.usd.toFixed(4)}</td>
                      <td>
                        <div style="display:flex; align-items:center; gap:6px">
                          <div style="width:36px; height:4px; background:var(--surface-raised); border-radius:2px; overflow:hidden">
                            <div style={{ width: `${Math.min(100, Math.max(0, share))}%`, height: "100%", background: "var(--accent)" }} />
                          </div>
                          <span class="dim" style="font-size:11px; min-width:28px">
                            {total() > 0 ? `${share.toFixed(0)}%` : "—"}
                          </span>
                        </div>
                      </td>
                    </tr>
                  );
                }}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </div>
  );
}

function FinOpsView() {
  const [data, { refetch }] = createResource(
    () => statsVersion(),
    () => api.finops()
  );
  const [runCapInput, setRunCapInput] = createSignal("");
  const [dayCapInput, setDayCapInput] = createSignal("");
  const [savingCaps, setSavingCaps] = createSignal(false);
  const [refreshing, setRefreshing] = createSignal(false);

  // Background polling every 15s when active tab
  onMount(() => {
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") {
        void refetch();
      }
    }, 15_000);
    onCleanup(() => window.clearInterval(timer));
  });

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      await refetch();
    } finally {
      setTimeout(() => setRefreshing(false), 450);
    }
  };

  let initialized = false;
  createEffect(() => {
    const d = data();
    if (d && !initialized) {
      initialized = true;
      setRunCapInput(d.run_cap_usd != null ? String(d.run_cap_usd) : "");
      setDayCapInput(d.day_cap_usd != null ? String(d.day_cap_usd) : "");
    }
  });

  const parseCap = (raw: string): number | null | undefined => {
    const trimmed = raw.trim();
    if (trimmed === "") return null; // explicit clear
    const n = Number(trimmed);
    return Number.isFinite(n) && n >= 0 ? n : undefined; // undefined = invalid, don't send
  };

  const saveCaps = async () => {
    const run = parseCap(runCapInput());
    const day = parseCap(dayCapInput());
    if (run === undefined || day === undefined) {
      pushToast("alert", "A budget cap must be a non-negative number, or blank to clear it.");
      return;
    }
    setSavingCaps(true);
    try {
      await api.patchFinops({ max_run_usd: run, max_day_usd: day });
      pushToast("info", "Budget caps saved");
      await refetch();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setSavingCaps(false);
    }
  };

  const dayProgress = createMemo(() => {
    const d = data();
    if (!d || !d.day_cap_usd || d.day_cap_usd <= 0) return null;
    return Math.min(100, (d.day_usd / d.day_cap_usd) * 100);
  });

  return (
    <div class="view">
      <PageHeader
        title="FinOps"
        description="What every dispatch is estimated to cost, where it goes, and active guardrails."
        actions={
          <div class="finops-header-actions">
            <div class="finops-live-badge" title="Real-time sync via SSE events and live telemetry pulse">
              <span class="finops-live-dot" />
              <span>Live</span>
            </div>
            <button
              class="ghost small"
              disabled={refreshing() || data.loading}
              onClick={() => void handleRefresh()}
              title="Refresh FinOps telemetry"
            >
              <span style={{ display: "inline-block", transform: refreshing() ? "rotate(180deg)" : "none", transition: "transform 0.5s ease" }}>↻</span>
              <span>{refreshing() ? "Refreshing…" : "Refresh"}</span>
            </button>
          </div>
        }
      />
      <Show when={!data.loading} fallback={<div class="cred-list"><span class="skel skel-block" /><span class="skel skel-block" /></div>}>
        <Show when={!data.error} fallback={<LoadError message={`${data.error}`} onRetry={() => refetch()} />}>
          {/* Executive KPI Strip (Compact 5-column responsive grid) */}
          <div class="stats-grid">
            <StatCard
              label="Spent today"
              value={`$${(data()?.day_usd ?? 0).toFixed(4)}`}
              progress={dayProgress() ?? undefined}
              sub={data()?.day_cap_usd ? `of $${data()!.day_cap_usd!.toFixed(2)} daily budget (${dayProgress()?.toFixed(0)}%)` : "No daily budget set"}
              tone={dayProgress() != null && dayProgress()! > 90 ? "warn" : undefined}
            />
            <StatCard
              label="Budget guardrails"
              value={data()?.run_cap_usd != null ? `Run: $${data()!.run_cap_usd!.toFixed(2)}` : "No run cap"}
              sub={data()?.day_cap_usd != null ? `Day limit: $${data()!.day_cap_usd!.toFixed(2)}` : "Checked before every dispatch"}
            />
            <StatCard
              label="Dispatches today"
              value={(data()?.by_provider ?? []).reduce((a, p) => a + p.calls, 0)}
              sub={`${data()?.total_rows ?? 0} total in cost ledger`}
            />
            <StatCard
              label="Tokens today"
              value={`${((data()?.day_input_tokens ?? 0) + (data()?.day_output_tokens ?? 0)).toLocaleString()}`}
              sub={`↑ ${(data()?.day_input_tokens ?? 0).toLocaleString()} · ↓ ${(data()?.day_output_tokens ?? 0).toLocaleString()}${(data()?.day_cache_read_tokens ?? 0) > 0 ? ` · ⚡ ${(data()?.day_cache_read_tokens ?? 0).toLocaleString()} cached` : ""}`}
            />
            <StatCard
              label="Pricing coverage"
              value={(data()?.unknown_rows ?? 0) === 0 ? "100% priced" : `${data()?.unknown_rows} unpriced`}
              sub={(data()?.unknown_rows ?? 0) === 0 ? "All models have price mappings" : "Cost is unknown for unpriced models"}
              tone={(data()?.unknown_rows ?? 0) > 0 ? "warn" : undefined}
            />
          </div>

          {/* Spend Dynamics: Trend Chart + Allocation Breakdown */}
          <div class="finops-main-grid">
            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Spend, last 14 days</h2>
                  <p class="dim">Estimated USD per day from the cost ledger. Hover bars for daily details.</p>
                </div>
              </div>
              <SpendTrendChart points={data()?.daily ?? []} capUsd={data()?.day_cap_usd ?? null} />
            </section>

            <SpendAllocationCard
              byProvider={data()?.by_provider ?? []}
              byModel={data()?.by_model ?? []}
            />
          </div>

          {/* Governance: Budget Caps & Alert History */}
          <div class="two-col" style="margin-bottom:14px">
            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Budget policy & caps</h2>
                  <p class="dim">Checked before every dispatch. Leave a field blank to remove that cap.</p>
                </div>
                <Show when={dayProgress() != null}>
                  <span class={`finops-stat-badge ${dayProgress()! >= 100 ? "alert" : dayProgress()! >= 80 ? "warn" : "good"}`}>
                    {dayProgress()! >= 100 ? "Limit reached" : dayProgress()! >= 80 ? "Approaching limit" : "Healthy pace"}
                  </span>
                </Show>
              </div>
              <div class="form-row">
                <label>Per-run cap (USD)</label>
                <input class="mono" placeholder="No limit" value={runCapInput()} onInput={(e) => setRunCapInput(e.currentTarget.value)} />
              </div>
              <div class="form-row">
                <label>Per-day cap (USD)</label>
                <input class="mono" placeholder="No limit" value={dayCapInput()} onInput={(e) => setDayCapInput(e.currentTarget.value)} />
              </div>
              <div class="row-gap" style="margin-top:12px; align-items:center">
                <button disabled={savingCaps()} onClick={() => void saveCaps()}>
                  {savingCaps() ? "Saving…" : "Save caps"}
                </button>
                <Show when={data()?.day_cap_usd}>
                  <span class="dim" style="font-size:11px">
                    ${(data()?.day_usd ?? 0).toFixed(4)} spent of ${data()!.day_cap_usd!.toFixed(2)} limit ({dayProgress()?.toFixed(1)}%)
                  </span>
                </Show>
              </div>
            </section>

            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Recent budget alerts</h2>
                  <p class="dim">Fired once per threshold per day — 80% and 100% of the daily cap.</p>
                </div>
              </div>
              <Show when={(data()?.recent_alerts ?? []).length > 0} fallback={<p class="dim">No alerts recorded.</p>}>
                <ul class="hit-list">
                  <For each={data()?.recent_alerts ?? []}>
                    {(a) => (
                      <li class="inbox-item">
                        <div class="hit-meta">
                          <span class={`chip chip-tone-${a.level === "full" ? "danger" : "warning"}`}>{a.level === "full" ? "100% Cap" : "80% Cap"}</span>
                          <span class="when">{timeAgo(a.ts)}</span>
                        </div>
                        <div class="hit-snippet">Day spend was ${a.day_total_usd.toFixed(4)} · session {shortId(a.session_id)}</div>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
            </section>
          </div>

          {/* Granular Rollup Tables */}
          <div class="two-col" style="margin-bottom:14px">
            <section class="panel"><FinOpsRollupTable title="By provider" rows={data()?.by_provider ?? []} /></section>
            <section class="panel"><FinOpsRollupTable title="By model" rows={data()?.by_model ?? []} /></section>
          </div>

          {/* Runtime Activity Today */}
          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>Runtime activity today</h2>
                <p class="dim">Only observed executions are counted. Duration is shown only when the execution boundary reports it.</p>
              </div>
            </div>
            <Show when={(data()?.activity ?? []).length > 0} fallback={<p class="dim">No instrumented runtime executions have been recorded today.</p>}>
              <div style="overflow-x:auto">
                <table class="table">
                  <thead>
                    <tr>
                      <th>kind</th>
                      <th>activity</th>
                      <th>plugin</th>
                      <th>calls</th>
                      <th>success rate</th>
                      <th>duration</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={data()?.activity ?? []}>
                      {(row) => {
                        const successRate = row.calls > 0 ? (row.successes / row.calls) * 100 : 100;
                        return (
                          <tr>
                            <td><span class="chip" style="font-size:10.5px">{row.kind}</span></td>
                            <td class="mono">{row.name}</td>
                            <td>{row.plugin ?? "—"}</td>
                            <td>{row.calls}</td>
                            <td>
                              <span class={`finops-stat-badge ${successRate === 100 ? "good" : successRate >= 75 ? "warn" : "alert"}`}>
                                {successRate.toFixed(0)}% ({row.successes}/{row.calls})
                              </span>
                            </td>
                            <td class="mono">
                              {row.duration_ms > 0 ? (
                                <span class={row.duration_ms > 2000 ? "dim" : ""}>
                                  {row.duration_ms >= 1000 ? `${(row.duration_ms / 1000).toFixed(2)}s` : `${row.duration_ms.toLocaleString()} ms`}
                                </span>
                              ) : (
                                <span class="dim">not measured</span>
                              )}
                            </td>
                          </tr>
                        );
                      }}
                    </For>
                  </tbody>
                </table>
              </div>
            </Show>
          </section>
        </Show>
      </Show>
    </div>
  );
}

// ---- Search ----------------------------------------------------------------

function highlight(snippet: string): string {
  // FTS snippets contain only <b> emphasis markers, but the surrounding text
  // is still untrusted transcript content. Escape it before using innerHTML.
  const escaped = snippet
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
  return escaped.replaceAll("&lt;b&gt;", "<mark>").replaceAll("&lt;/b&gt;", "</mark>");
}

const ROLE_FILTERS = [
  { id: "", label: "All roles" },
  { id: "user", label: "User" },
  { id: "assistant", label: "Assistant" },
  { id: "system", label: "System" },
  { id: "tool", label: "Tool" },
];

function SearchView() {
  const [q, setQ] = createSignal("");
  const [role, setRole] = createSignal("");
  const [hits, setHits] = createSignal<SearchHit[] | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");

  let timer: ReturnType<typeof setTimeout> | undefined;
  const runSearch = (query: string) => {
    clearTimeout(timer);
    if (!query.trim()) {
      setHits(null);
      return;
    }
    setBusy(true);
    timer = setTimeout(async () => {
      try {
        const res = await api.search(query.trim(), { role: role() || undefined, limit: 50 });
        setHits(res.hits);
        setError("");
      } catch (err) {
        if (err instanceof AuthRequired) setAuthed(false);
        else setError(`${err}`);
      } finally {
        setBusy(false);
      }
    }, 250);
  };

  return (
    <div class="view">
      <KnowledgeSubNav active="#/search" />
      <PageHeader title="Search" description="Look through every conversation vak has had, and jump straight to where something was said." />
      <div class="search-hero-deck">
        <div class="search-hero">
          <input
            class="search-big"
            placeholder="Search everything you’ve ever asked…"
            value={q()}
            onInput={(e) => {
              setQ(e.currentTarget.value);
              runSearch(e.currentTarget.value);
            }}
          />
          <select value={role()} onChange={(e) => { setRole(e.currentTarget.value); runSearch(q()); }}>
            <For each={ROLE_FILTERS}>{(f) => <option value={f.id}>{f.label}</option>}</For>
          </select>
        </div>
        <div class="search-role-pills">
          <span class="dim small" style="margin-right: 4px;">Role filter:</span>
          <For each={ROLE_FILTERS}>
            {(f) => (
              <button
                class="search-role-btn"
                classList={{ active: role() === f.id }}
                onClick={() => {
                  setRole(f.id);
                  runSearch(q());
                }}
              >
                {f.label}
              </button>
            )}
          </For>
          <Show when={hits()}>
            <span class="chip chip-kind" style="margin-left: auto;">
              {hits()!.length} match{hits()!.length === 1 ? "" : "es"}
            </span>
          </Show>
        </div>
      </div>
      <Show when={error()}>
        <div class="login-error">{error()}</div>
      </Show>
      <Switch>
        <Match when={busy() && !hits()}>
          <div class="empty">Searching…</div>
        </Match>
        <Match when={hits() === null}>
          <div class="empty">Start typing to search every conversation. Put a phrase in "quotes" to match it exactly, or end a word with * to match anything that starts with it.</div>
        </Match>
        <Match when={hits()?.length === 0}>
          <div class="empty">Nothing found.</div>
        </Match>
        <Match when={hits()}>
          <ul class="hit-list">
            <For each={hits() ?? []}>
              {(h) => (
                <li onClick={() => navigate(`#/sessions/${h.session_id}`)}>
                  <div class="hit-meta">
                    <span class="mono">{shortId(h.session_id)}</span>
                    <span class="chip chip-kind">{h.kind}</span>
                    <Show when={h.role}><span class="chip chip-role">{h.role}</span></Show>
                    <Show when={h.tool_name}><span class="chip chip-tool mono">{h.tool_name}</span></Show>
                    <span class="when">{timeAgo(h.ts)}</span>
                  </div>
                  <div class="hit-snippet" innerHTML={highlight(h.snippet)} />
                </li>
              )}
            </For>
          </ul>
        </Match>
      </Switch>
    </div>
  );
}

// ---- Security --------------------------------------------------------------

function Security() {
  return <SecurityCenter scope={configScope} />;
}

// ---- Inbox (delegated to Inbox.tsx suite) ----------------------------------

// ---- Settings & Governance -------------------------------------------------

const ALLOWLIST_CHIP_TONE: Record<string, string> = {
  allowed: "success",
  pending: "warning",
  denied: "danger",
};

// Per-surface badge so a mixed Telegram+Discord+Slack deployment reads at
// a glance (docs/design/34 Phase 3). Everything else in these panels is
// already surface-agnostic — it keys off `surface:chat` alone.

function SurfaceBadge(props: { channelKey: string }) {
  const surface = () => props.channelKey.split(":")[0] ?? "";
  return (
    <span class="chip chip-surface" data-surface={surface()} title={`${surfaceLabel(surface())} chat`}>
      {surfaceLabel(surface())}
    </span>
  );
}

const CUSTOM_WORKSPACE = "__custom__";

/// Workspace picker: known workspaces first (paths vak has actually run
/// in), with a custom-path fallback that says plainly it is unverified
/// until a Core starts there — docs/design/34 open question 4. A typo'd
/// path is what put a channel in the wrong workspace in the first place.
function WorkspacePicker(props: {
  value: string;
  onChange: (value: string) => void;
  known: string[];
  corePool: CorePoolEntry[];
  catalog?: Array<{ path: string; name: string }>;
}) {
  const options = createMemo(() => {
    const seen = props.known.filter(Boolean);
    // A value already saved on the entry stays selectable even when it is
    // not (or no longer) in the known list.
    if (props.value && !seen.includes(props.value)) return [props.value, ...seen];
    return seen;
  });
  const isCustom = () => !!props.value && !options().includes(props.value);
  const [custom, setCustom] = createSignal(isCustom());
  const warm = createMemo(() => props.corePool.some((e) => e.workspace === props.value.trim()));
  const label = (path: string) => props.catalog?.find((entry) => entry.path === path)?.name || path.split("/").filter(Boolean).at(-1) || path;

  return (
    <div style="margin-bottom:8px">
      <label class="inherit-toggle" style="margin-top:2px">
        Workspace
        <span class="chip chip-phrase" data-on={warm()} style="margin-left:6px">
          {warm() ? "loaded and ready" : "loads on the next message"}
        </span>
      </label>
      <select
        class="mono"
        style="width:100%"
        value={custom() ? CUSTOM_WORKSPACE : props.value}
        onChange={(e) => {
          const next = e.currentTarget.value;
          if (next === CUSTOM_WORKSPACE) {
            setCustom(true);
            return;
          }
          setCustom(false);
          props.onChange(next);
        }}
      >
        <For each={options()}>{(w) => <option value={w}>{label(w)} · {w}</option>}</For>
        <option value={CUSTOM_WORKSPACE}>Another folder…</option>
      </select>
      <Show when={custom()}>
        <input
          class="mono"
          value={props.value}
          onInput={(e) => props.onChange(e.currentTarget.value)}
          placeholder="/full/path/to/the/workspace"
          style="width:100%;margin-top:6px"
        />
        <div class="binding-meta">
Nothing has run here yet, so this path isn’t checked until the first message arrives. A
          typo shows up later as a failure — run “Check for problems” under Settings to catch one.
        </div>
      </Show>
    </div>
  );
}

function WorkspaceNames(props: { ctx: GatewayCtx }) {
  const [editing, setEditing] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal("");
  const [saving, setSaving] = createSignal(false);
  const entries = () => props.ctx.status()?.workspace_catalog ?? [];
  const begin = (entry: { path: string; name: string }) => { setEditing(entry.path); setDraft(entry.name); };
  const save = async () => {
    const path = editing();
    const name = draft().trim();
    if (!path || !name || saving()) return;
    setSaving(true);
    try {
      await api.patchWorkspaceName(path, name);
      await props.ctx.refresh();
      pushToast("info", `Workspace renamed to “${name}”`);
      setEditing(null);
    } catch (error) { pushToast("alert", `Could not rename workspace: ${error}`); }
    finally { setSaving(false); }
  };
  return <section class="workspace-names">
    <div class="panel-title-row"><div><h3>Workspace names</h3><p class="dim">Names are shared across the account. The canonical path remains the security identity.</p></div></div>
    <Show when={entries().length > 0} fallback={<p class="dim">No workspaces have been discovered yet.</p>}>
      <For each={entries()}>{(entry) => <div class="workspace-name-row"><div><strong>{entry.name}</strong><span class="mono dim">{entry.path}</span></div><Show when={editing() !== entry.path} fallback={<div class="workspace-name-edit"><input value={draft()} onInput={(e) => setDraft(e.currentTarget.value)} /><button class="small" disabled={!draft().trim() || saving()} onClick={() => void save()}>{saving() ? "Saving…" : "Save"}</button><button class="ghost small" onClick={() => setEditing(null)}>Cancel</button></div>}><button class="ghost small" onClick={() => begin(entry)}>Rename</button></Show></div>}</For>
    </Show>
  </section>;
}

function GatewayBindingEditor(props: {
  binding: GatewayBinding;
  providers: ProviderSummary[];
  allowlistEntry: AllowlistEntry | undefined;
  knownWorkspaces: string[];
  corePool: CorePoolEntry[];
  bots: Bot[];
  refresh: () => void;
}) {
  const [editing, setEditing] = createSignal(false);
  const [inherit, setInherit] = createSignal(!props.binding.override);
  const [provider, setProvider] = createSignal(
    props.binding.override?.provider ?? props.binding.effective_route.provider,
  );
  const [model, setModel] = createSignal(
    props.binding.override?.model ?? props.binding.effective_route.model,
  );
  const [models, setModels] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);

  createEffect(async () => {
    if (inherit()) return;
    try {
      const found = (await api.models(provider())).models ?? [];
      setModels(found);
      if (found.length && !found.includes(model())) setModel(found[0]);
    } catch {
      setModels([]);
    }
  });

  const act = async (operation: () => Promise<void>, message: string) => {
    setBusy(true);
    try {
      await operation();
      await props.refresh();
      pushToast("info", message);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <article class="binding-card" classList={{ "binding-stale": props.binding.stale }}>
      <div class="binding-head">
        <div>
          <strong class="mono">{props.binding.target}</strong>
          <div class="binding-meta">
            {props.binding.session_id ? `session ${shortId(props.binding.session_id)}` : "starts a new session on the next message"}
          </div>
        </div>
        <div class="row-gap" style="gap:6px">
          <SurfaceBadge channelKey={props.binding.target} />
          <Show when={props.allowlistEntry}>
            <span class={`chip chip-tone-${ALLOWLIST_CHIP_TONE[props.allowlistEntry!.status] ?? ""}`}>
              {props.allowlistEntry!.status}
            </span>
          </Show>
          <span class={`chip ${props.binding.stale ? "chip-tone-warning" : "chip-tone-success"}`}>
            {props.binding.stale ? "new session next" : "up to date"}
          </span>
        </div>
      </div>

      <div class="route-compare">
        <div>
          <span class="eyebrow">Answering with</span>
          <strong>{providerLabel(props.binding.effective_route.provider)}</strong>
          <code>{props.binding.effective_route.model}</code>
          <span class="binding-meta">from {sourceLabel(props.binding.effective_route.source)}</span>
        </div>
        {/* The permission counterpart of "Effective route": what this
            channel actually runs as, and where that came from. A pin that
            the workspace's own mode capped is called out rather than shown
            as if the wider grant were live. */}
        <Show when={props.allowlistEntry?.effective_permission_mode}>
          <div>
            <span class="eyebrow">Allowed to</span>
            <strong>{modeLabel(props.allowlistEntry!.effective_permission_mode)}</strong>
            <span class="binding-meta">
              {props.allowlistEntry!.permission_capped
                ? `reduced — you asked for "${modeLabel(props.allowlistEntry!.permission_mode)}", which is more than this workspace allows`
                : props.allowlistEntry!.permission_mode
                  ? "set for this chat"
                  : "follows the workspace"}
            </span>
          </div>
        </Show>
        <div>
          <span class="eyebrow">Current conversation</span>
          <Show when={props.binding.session_contract} fallback={<span class="dim">Not started yet</span>}>
            <strong>{providerLabel(props.binding.session_contract!.provider)}</strong>
            <code>{props.binding.session_contract!.model}</code>
            <span class="binding-meta">v{props.binding.session_contract!.app_version}</span>
          </Show>
        </div>
      </div>

      <Show when={props.binding.stale_reasons.length}>
        <div class="warning">The next message starts a fresh conversation, because this changed: {props.binding.stale_reasons.join(", ").replaceAll("_", " ")}</div>
      </Show>

      <label class="inherit-toggle">
        <input type="checkbox" checked={inherit()} onChange={(e) => setInherit(e.currentTarget.checked)} />
        Use the workspace’s model (untick to choose one for this chat)
      </label>
      <Show when={!inherit()}>
        <div class="binding-controls">
          <select
            value={provider()}
            ref={(el) => syncSelect(el, provider, () => props.providers)}
            onChange={(e) => setProvider(e.currentTarget.value)}
          >
            <For each={props.providers}>{(p) => <option value={p.name}>{providerLabel(p.name)}</option>}</For>
          </select>
          <Show when={models().length} fallback={
            <input class="mono" value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="model id" />
          }>
            <select
              class="mono"
              value={model()}
              ref={(el) => syncSelect(el, model, models)}
              onChange={(e) => setModel(e.currentTarget.value)}
            >
              <For each={models()}>{(m) => <option value={m}>{m}</option>}</For>
            </select>
          </Show>
        </div>
      </Show>
      <div class="row-gap">
        <button disabled={busy() || (!inherit() && !model().trim())} onClick={() => act(
          () => api.patchGatewayBinding(props.binding.target, inherit() ? {} : { provider: provider(), model: model() }),
          `Model updated for ${props.binding.target}`,
        )}>Save model</button>
        <button class="ghost" disabled={busy() || !props.binding.session_id} onClick={() => act(
          () => api.rotateGatewayBinding(props.binding.target),
          `Started a fresh conversation for ${props.binding.target}`,
        )}>Start fresh</button>
        <span class="spacer" />
        <Show when={props.allowlistEntry?.status === "allowed"}>
          <button class="ghost small" disabled={busy()} onClick={() => setEditing((v) => !v)}>
            {editing() ? "Close" : "Edit what it can do"}
          </button>
          <button class="danger small" disabled={busy()} onClick={() => {
            if (window.confirm(`Disconnect ${props.binding.target}? The next message from this chat is turned away, and it goes back to waiting for your approval.`)) {
              void act(() => api.revokeGatewayAllowlist(props.binding.target), `Disconnected ${props.binding.target}`);
            }
          }}>Disconnect</button>
        </Show>
        <button class="danger small" disabled={busy()} onClick={() => {
          if (window.confirm(`Remove ${props.binding.target} and the model chosen for it? The conversation history is kept.`)) {
            void act(() => api.deleteGatewayBinding(props.binding.target), `Removed ${props.binding.target}`);
          }
        }}>Remove</button>
      </div>
      <Show when={editing() && props.allowlistEntry?.status === "allowed"}>
        <ChannelAccessEditor
          entry={props.allowlistEntry!}
          providers={props.providers}
          knownWorkspaces={props.knownWorkspaces}
          corePool={props.corePool}
          bots={props.bots}
          refresh={() => {
            setEditing(false);
            props.refresh();
          }}
        />
      </Show>
    </article>
  );
}

function defaultChannelPolicy(): ChannelPolicy {
  return {
    tools_allow: null,
    tools_deny: [],
    mcp_allow: null,
    mcp_deny: [],
    skills_allow: null,
    skills_deny: [],
    hooks_allow: null,
    hooks_deny: [],
    mcp_network_deny: [],
  };
}

/// Optional per-channel permission pin, parallel to the "Pin a specific
/// provider / model" checkbox. Unchecked means inherit the workspace's own
/// configured mode, which is the default and today's behavior.
function ChannelPermissionPicker(props: {
  pinned: boolean;
  setPinned: (v: boolean) => void;
  mode: PermissionMode;
  setMode: (m: PermissionMode) => void;
  /// The workspace's own mode, when known — the ceiling a pin is capped to.
  ceiling?: PermissionMode | null;
  /// What this pin belongs to, for the toggle's label — "channel" (a chat)
  /// or "bot". Defaults to "channel", the original caller's wording.
  subject?: string;
}) {
  // A pin above the workspace's own mode is not an error, it is simply
  // reduced at dispatch; say so plainly rather than letting the operator
  // believe they granted access the workspace never permits.
  const capped = () => {
    const c = props.ceiling;
    if (!props.pinned || !c) return null;
    const rank = (m: PermissionMode) => CHANNEL_MODES.findIndex((x) => x.value === m);
    return rank(props.mode) > rank(c) ? c : null;
  };
  return (
    <>
      <label class="inherit-toggle">
        <input
          type="checkbox"
          checked={props.pinned}
          onChange={(e) => props.setPinned(e.currentTarget.checked)}
        />
        Give this {props.subject ?? "channel"} its own limits (otherwise it follows the workspace)
      </label>
      <Show when={props.pinned}>
        <div class="mode-grid">
          <For each={CHANNEL_MODES}>
            {(m) => (
              <button
                class="mode-btn"
                classList={{ active: props.mode === m.value }}
                onClick={() => props.setMode(m.value)}
              >
                <span class="mode-name">{m.label}</span>
                <span class="mode-desc">{m.desc}</span>
              </button>
            )}
          </For>
        </div>
        <Show when={capped()}>
          {(c) => (
            <div class="binding-meta">
              This workspace is set to “{modeLabel(c())}”. A channel can match that or ask for less,
              never more — so this choice will take effect as “{modeLabel(c())}”.
            </div>
          )}
        </Show>
      </Show>
    </>
  );
}

/// Optional per-bot/per-chat voice + persona pin, parallel to
/// `ChannelPermissionPicker`. Unchecked means inherit up the same
/// `route`/`permission_mode` chain — `chat.voice.or(bot.voice)`, gated by
/// `inherit_bot_policy`. A controlled component: the parent panel owns the
/// signals and folds them into its own single Save call, exactly like
/// `pinRoute`/`pinPerm` do today.
function VoiceConfigEditor(props: {
  pinned: boolean;
  setPinned: (v: boolean) => void;
  voiceName: string;
  provider?: string;
  setProvider?: (v: string) => void;
  setVoiceName: (v: string) => void;
  transcriptionModel: string;
  setTranscriptionModel: (v: string) => void;
  synthesisModel: string;
  setSynthesisModel: (v: string) => void;
  realtimeModel: string;
  setRealtimeModel: (v: string) => void;
  persona: string;
  setPersona: (v: string) => void;
  /// What this pin belongs to, for the toggle's label — "bot" or "chat".
  subject: "bot" | "chat";
}) {
  const [previewing, setPreviewing] = createSignal(false);
  const [voiceProviders] = createResource(() => api.voiceProviders());
  const discoveredVoices = createMemo(() =>
    (voiceProviders()?.providers ?? []).flatMap((provider: VoiceProviderSummary) => provider.voices),
  );
  let audioEl: HTMLAudioElement | undefined;

  const preview = async () => {
    setPreviewing(true);
    try {
      const blob = await api.speak({
        text: "Hi, this is a preview of my voice.",
        voice_override: {
          voice_name: props.voiceName || null,
          transcription_model: props.transcriptionModel || null,
          synthesis_model: props.synthesisModel || null,
          realtime_model: props.realtimeModel || null,
          persona: props.persona || null,
        },
      });
      if (!audioEl) audioEl = new Audio();
      audioEl.src = URL.createObjectURL(blob);
      await audioEl.play();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setPreviewing(false);
    }
  };

  return (
    <>
      <label class="inherit-toggle">
        <input
          type="checkbox"
          checked={props.pinned}
          onChange={(e) => props.setPinned(e.currentTarget.checked)}
        />
        Give this {props.subject === "bot" ? "bot" : "chat"} its own voice (otherwise it follows{" "}
        {props.subject === "bot" ? "its parent" : "the bot"})
      </label>
      <Show when={props.pinned}>
        <div class="binding-controls">
          <Show when={voiceProviders.error}>
            <div class="error-state" role="alert">
              Voice discovery unavailable: {String(voiceProviders.error)}. Entering an exact voice id is still possible.
            </div>
          </Show>
          <Show when={props.setProvider}>
            <select value={props.provider ?? ""} onChange={(e) => props.setProvider?.(e.currentTarget.value)} aria-label="Voice provider"><option value="">Inherit voice provider</option><For each={voiceProviders()?.providers ?? []}>{(p) => <option value={p.name}>{p.name}</option>}</For></select>
          </Show>
          <input
            list="vak-discovered-voices"
            value={props.voiceName}
            onInput={(e) => props.setVoiceName(e.currentTarget.value)}
            placeholder="Provider default or discovered voice id"
            aria-label="Voice identifier"
          />
          <datalist id="vak-discovered-voices">
            <For each={discoveredVoices()}>{(v) => <option value={v} />}</For>
          </datalist>
          <input value={props.transcriptionModel} onInput={(e) => props.setTranscriptionModel(e.currentTarget.value)} placeholder="Transcription model (inherit if empty)" aria-label="Transcription model" />
          <input value={props.synthesisModel} onInput={(e) => props.setSynthesisModel(e.currentTarget.value)} placeholder="Synthesis model (inherit if empty)" aria-label="Synthesis model" />
          <input value={props.realtimeModel} onInput={(e) => props.setRealtimeModel(e.currentTarget.value)} placeholder="Realtime model (inherit if empty)" aria-label="Realtime model" />
          <button disabled={previewing()} onClick={() => void preview()}>
            {previewing() ? "Playing…" : "Preview"}
          </button>
        </div>
        <textarea
          value={props.persona}
          onInput={(e) => props.setPersona(e.currentTarget.value)}
          placeholder="warm, upbeat, and enthusiastic"
          rows={2}
        />
      </Show>
    </>
  );
}

function ChannelCapabilityPolicy(props: {
  value: ChannelPolicy;
  onChange: (value: ChannelPolicy) => void;
}) {
  const [mcp] = createResource(() => api.mcpServers());
  const [skills] = createResource(() => api.skills());
  const [hooks] = createResource(() => api.hooks());
  const update = (patch: Partial<ChannelPolicy>) => props.onChange({ ...props.value, ...patch });

  // A server is matched as `name/*` (see `Core::filter_mcp`), a skill by its
  // own name, a hook by `event/command`. The picker offers exactly those
  // values so nothing here has to be typed.
  const mcpOptions = createMemo<AccessOption[]>(() =>
    Object.keys(mcp()?.servers ?? {}).map((name) => ({
      value: `${name}/*`,
      label: name,
      hint: "all of its tools",
    })),
  );
  const skillOptions = createMemo<AccessOption[]>(() =>
    (skills()?.skills ?? []).map((s) => ({
      value: s.name,
      label: s.name,
      hint: s.description?.slice(0, 60),
    })),
  );
  const hookOptions = createMemo<AccessOption[]>(() =>
    (hooks()?.hooks ?? []).map((h) => ({
      value: `${h.event}/${h.command}`,
      label: h.command.split("/").pop() || h.command,
      hint: `on ${HOOK_EVENT_LABELS[h.event] ?? h.event}`,
    })),
  );

  const networkServers = createMemo(
    () => Object.entries(mcp()?.servers ?? {}) as [string, McpServerConfig][],
  );
  const netOff = (name: string) => props.value.mcp_network_deny.includes(`${name}/*`);
  const toggleNet = (name: string) => {
    const pattern = `${name}/*`;
    update({
      mcp_network_deny: netOff(name)
        ? props.value.mcp_network_deny.filter((v) => v !== pattern)
        : [...props.value.mcp_network_deny, pattern],
    });
  };

  return (
    <section class="channel-capabilities">
      <Show when={mcp.error || skills.error || hooks.error}>
        <div class="error-state" role="alert">
          Capability discovery is incomplete. Empty options below are unknown, not denied.
          <Show when={mcp.error}> MCP: {String(mcp.error)}</Show>
          <Show when={skills.error}> Skills: {String(skills.error)}</Show>
          <Show when={hooks.error}> Hooks: {String(hooks.error)}</Show>
        </div>
      </Show>
      <div class="binding-meta">
        By default this channel gets whatever the workspace allows. Anything you change here can
        only take access away, never add it.
      </div>
      <div class="capability-grid">
        <AccessPicker
          title="What it can do on this machine"
          help="The built-in tools — reading files, editing them, running commands."
          noun="tool"
          inheritLabel="Everything the workspace allows"
          limitLabel="Only what I pick"
          noneLabel="No built-in tools"
          options={BUILTIN_TOOLS}
          allow={props.value.tools_allow}
          deny={props.value.tools_deny}
          onChange={(next) => update({ tools_allow: next.allow, tools_deny: next.deny })}
          patternHint="e.g. web*"
        />
        <AccessPicker
          title="Connected apps"
          help="Tools borrowed from the apps connected under Extensions."
          noun="server"
          inheritLabel="Every connected app"
          limitLabel="Only the apps I pick"
          noneLabel="No connected apps"
          options={mcpOptions()}
          loading={mcp.loading}
          allow={props.value.mcp_allow}
          deny={props.value.mcp_deny}
          onChange={(next) => update({ mcp_allow: next.allow, mcp_deny: next.deny })}
          patternHint="e.g. github/*"
        />
        <AccessPicker
          title="Skills"
          help="Which sets of instructions the agent can see on this channel."
          noun="skill"
          inheritLabel="Every skill"
          limitLabel="Only the skills I pick"
          noneLabel="No skills"
          options={skillOptions()}
          loading={skills.loading}
          allow={props.value.skills_allow}
          deny={props.value.skills_deny}
          onChange={(next) => update({ skills_allow: next.allow, skills_deny: next.deny })}
          patternHint="e.g. research*"
        />
        <AccessPicker
          title="Automations"
          help="The hooks an admin has set to run around each turn."
          noun="automation"
          inheritLabel="Every automation"
          limitLabel="Only the ones I pick"
          noneLabel="No automations"
          options={hookOptions()}
          loading={hooks.loading}
          allow={props.value.hooks_allow}
          deny={props.value.hooks_deny}
          onChange={(next) => update({ hooks_allow: next.allow, hooks_deny: next.deny })}
          patternHint="e.g. pre_tool_use/*"
        />
      </div>

      <Show when={networkServers().length > 0}>
        <section class="access-picker">
          <header class="access-head">
            <div>
              <h4>Internet access for connected apps</h4>
              <p class="dim">
                Turn the internet off for an app on this channel only. It stays on everywhere else.
                An app that has no internet to begin with cannot be given it here.
              </p>
            </div>
          </header>
          <div class="pick-grid">
            <For each={networkServers()}>
              {([name, server]) => (
                <label class="pick deny" classList={{ on: netOff(name) }}>
                  <input type="checkbox" checked={netOff(name)} onChange={() => toggleNet(name)} />
                  <span>
                    <strong>{name}</strong>
                    <em>{server.network ? "has internet — uncheck to keep it" : "already offline"}</em>
                  </span>
                </label>
              )}
            </For>
          </div>
        </section>
      </Show>
    </section>
  );
}

/// Re-point an already-allowed channel's workspace/route in place
/// (`PATCH .../allowlist/{key}`) instead of revoke-and-re-approve, which
/// would lose the added_at/added_by provenance and 403 the channel in
/// between. The same form the approve flow uses, pre-filled.
function ChannelAccessEditor(props: {
  entry: AllowlistEntry;
  providers: ProviderSummary[];
  knownWorkspaces: string[];
  corePool: CorePoolEntry[];
  bots: Bot[];
  refresh: () => void;
}) {
  const [workspace, setWorkspace] = createSignal(props.entry.workspace ?? "");
  const [agentId, setAgentId] = createSignal(props.entry.agent_id || "vak");
  const [pinRoute, setPinRoute] = createSignal(!!props.entry.route);
  const [provider, setProvider] = createSignal(
    props.entry.route?.provider ?? props.providers[0]?.name ?? "",
  );
  const [model, setModel] = createSignal(props.entry.route?.model ?? "");
  const [models, setModels] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [pinPerm, setPinPerm] = createSignal(!!props.entry.permission_mode);
  const [perm, setPerm] = createSignal<PermissionMode>(
    props.entry.permission_mode ?? props.entry.workspace_permission_mode ?? "workspace-write",
  );
  const [policy, setPolicy] = createSignal<ChannelPolicy>(props.entry.policy ?? defaultChannelPolicy());
  // Multi-bot-per-channel: which bot (if any) this chat is bound to, and
  // whether it inherits that bot's policy/permission/route tier or breaks
  // inheritance and resolves purely against the workspace.
  const surface = () => props.entry.key.split(":")[0];
  const surfaceBots = () => props.bots.filter((b) => b.surface === surface());
  const [botId, setBotId] = createSignal(props.entry.bot_id ?? "");
  const [inheritBot, setInheritBot] = createSignal(props.entry.inherit_bot_policy);
  const [pinVoice, setPinVoice] = createSignal(!!props.entry.voice);
  const [voiceProvider, setVoiceProvider] = createSignal(props.entry.voice?.provider ?? "");
  const [voiceName, setVoiceName] = createSignal(props.entry.voice?.voice_name ?? "");
  const [voicePersona, setVoicePersona] = createSignal(props.entry.voice?.persona ?? "");
  const [transcriptionModel, setTranscriptionModel] = createSignal(props.entry.voice?.transcription_model ?? "");
  const [synthesisModel, setSynthesisModel] = createSignal(props.entry.voice?.synthesis_model ?? "");
  const [realtimeModel, setRealtimeModel] = createSignal(props.entry.voice?.realtime_model ?? "");

  createEffect(async () => {
    if (!pinRoute() || !provider()) return;
    try {
      const found = (await api.models(provider())).models ?? [];
      setModels(found);
      if (found.length && !found.includes(model())) setModel(found[0]);
    } catch {
      setModels([]);
    }
  });

  const save = async () => {
    setBusy(true);
    try {
      await api.patchGatewayAllowlist(props.entry.key, {
        workspace: workspace().trim() || undefined,
        agent_id: agentId() || "vak",
        route: pinRoute() && provider() && model() ? { provider: provider(), model: model() } : {},
        // "" clears the pin back to inheriting the workspace default.
        permission_mode: pinPerm() ? perm() : "",
        policy: policy(),
        bot_id: botId() || null,
        inherit_bot_policy: inheritBot(),
        voice: pinVoice() ? { provider: voiceProvider() || null, voice_name: voiceName() || null, persona: voicePersona() || null, transcription_model: transcriptionModel() || null, synthesis_model: synthesisModel() || null, realtime_model: realtimeModel() || null } : null,
      });
      await props.refresh();
      pushToast("info", `Updated ${props.entry.key} — the next message rotates to a fresh session`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="pending-card" style="margin-top:10px">
      <div class="binding-meta" style="margin-bottom:6px">
        Approved {timeAgo(props.entry.added_at)} by {props.entry.added_by} — editing keeps that
        provenance. A changed workspace or route rotates to a new frozen session on the next
        message; the existing ledger is preserved.
      </div>
      <WorkspacePicker
        value={workspace()}
        onChange={setWorkspace}
        known={props.knownWorkspaces}
        corePool={props.corePool}
      />
      <label class="inherit-toggle">
        Target Agent
        <select value={agentId()} onChange={(e) => setAgentId(e.currentTarget.value)}>
          <option value="vak">✦ Vak (Default Assistant)</option>
          <For each={adminAgents().filter((a) => a.id !== "vak")}>
            {(a) => <option value={a.id}>✦ {a.name} ({a.id})</option>}
          </For>
        </select>
      </label>
      <Show when={surfaceBots().length > 0}>
        <label class="inherit-toggle">
          Bot
          <select value={botId()} onChange={(e) => setBotId(e.currentTarget.value)}>
            <option value="">No bot (workspace only)</option>
            <For each={surfaceBots()}>{(b) => <option value={b.id}>{b.label}</option>}</For>
          </select>
        </label>
        <Show when={botId()}>
          <label class="inherit-toggle">
            <input type="checkbox" checked={inheritBot()} onChange={(e) => setInheritBot(e.currentTarget.checked)} />
            Inherit this bot's policy, permission mode, and model (uncheck to resolve against the workspace only)
          </label>
        </Show>
      </Show>
      <VoiceConfigEditor
        pinned={pinVoice()}
        setPinned={setPinVoice}
        provider={voiceProvider()}
        setProvider={setVoiceProvider}
        voiceName={voiceName()}
        setVoiceName={setVoiceName}
        transcriptionModel={transcriptionModel()}
        setTranscriptionModel={setTranscriptionModel}
        synthesisModel={synthesisModel()}
        setSynthesisModel={setSynthesisModel}
        realtimeModel={realtimeModel()}
        setRealtimeModel={setRealtimeModel}
        persona={voicePersona()}
        setPersona={setVoicePersona}
        subject="chat"
      />
      <label class="inherit-toggle">
        <input type="checkbox" checked={pinRoute()} onChange={(e) => setPinRoute(e.currentTarget.checked)} />
        Give this channel its own model (otherwise it follows the workspace)
      </label>
      <Show when={pinRoute()}>
        <div class="binding-controls">
          <select
            value={provider()}
            ref={(el) => syncSelect(el, provider, () => props.providers)}
            onChange={(e) => setProvider(e.currentTarget.value)}
          >
            <For each={props.providers}>{(p) => <option value={p.name}>{providerLabel(p.name)}</option>}</For>
          </select>
          <Show when={models().length} fallback={
            <input class="mono" value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="model id" />
          }>
            <select
              class="mono"
              value={model()}
              ref={(el) => syncSelect(el, model, models)}
              onChange={(e) => setModel(e.currentTarget.value)}
            >
              <For each={models()}>{(m) => <option value={m}>{m}</option>}</For>
            </select>
          </Show>
        </div>
      </Show>
      <ChannelPermissionPicker
        pinned={pinPerm()}
        setPinned={setPinPerm}
        mode={perm()}
        setMode={setPerm}
        ceiling={props.entry.workspace_permission_mode}
      />
      <ChannelCapabilityPolicy value={policy()} onChange={setPolicy} />
      <div class="row-gap">
        <button disabled={busy()} onClick={save}>Save changes</button>
      </div>
    </div>
  );
}

function PendingChannelCard(props: {
  entry: AllowlistEntry;
  providers: ProviderSummary[];
  defaultWorkspace: string;
  knownWorkspaces: string[];
  corePool: CorePoolEntry[];
  refresh: () => void;
}) {
  const [workspace, setWorkspace] = createSignal(props.defaultWorkspace);
  const [agentId, setAgentId] = createSignal("vak");
  const [pinRoute, setPinRoute] = createSignal(false);
  const [provider, setProvider] = createSignal(props.providers[0]?.name ?? "");
  const [model, setModel] = createSignal("");
  const [models, setModels] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [pinPerm, setPinPerm] = createSignal(false);
  const [perm, setPerm] = createSignal<PermissionMode>("workspace-write");
  const [policy, setPolicy] = createSignal<ChannelPolicy>(props.entry.policy ?? defaultChannelPolicy());

  createEffect(async () => {
    if (!pinRoute() || !provider()) return;
    try {
      const found = (await api.models(provider())).models ?? [];
      setModels(found);
      if (found.length && !found.includes(model())) setModel(found[0]);
    } catch {
      setModels([]);
    }
  });

  const approve = async () => {
    setBusy(true);
    try {
      const entry = await api.approveGatewayAllowlist(props.entry.key, {
        workspace: workspace().trim() || undefined,
        agent_id: agentId() || "vak",
        route: pinRoute() && provider() && model() ? { provider: provider(), model: model() } : undefined,
        permission_mode: pinPerm() ? perm() : undefined,
        policy: policy(),
      });
      // Surface a capped grant at the moment it happens: the server
      // reduces an over-broad pin to the workspace's own mode, and an
      // operator who is not told would believe the wider grant is live.
      if (entry.permission_capped) {
        pushToast(
          "alert",
          `${props.entry.key}: "${modeLabel(entry.permission_mode)}" is more than that workspace allows, so it was reduced to "${modeLabel(entry.effective_permission_mode)}".`,
        );
      }
      await props.refresh();
      pushToast("info", `Approved ${props.entry.key}${entry.workspace ? ` — working in ${entry.workspace}` : ", but no workspace was set"}`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const deny = async () => {
    setBusy(true);
    try {
      await api.denyGatewayAllowlist(props.entry.key);
      await props.refresh();
      pushToast("info", `Turned away ${props.entry.key}`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <article class="binding-card pending-card">
      <div class="binding-head">
        <div>
          <strong class="mono">{props.entry.key}</strong>
          <div class="binding-meta">first messaged {timeAgo(props.entry.added_at)}</div>
        </div>
        <div class="row-gap" style="gap:6px">
          <SurfaceBadge channelKey={props.entry.key} />
          <span class="chip chip-tone-warning">pending</span>
        </div>
      </div>

      <Show when={props.entry.first_seen_text}>
        <div class="pending-first-seen">{props.entry.first_seen_text}</div>
      </Show>

      <WorkspacePicker
        value={workspace()}
        onChange={setWorkspace}
        known={props.knownWorkspaces}
        corePool={props.corePool}
      />

      <label class="inherit-toggle">
        Target Agent
        <select value={agentId()} onChange={(e) => setAgentId(e.currentTarget.value)}>
          <option value="vak">✦ Vak (Default Assistant)</option>
          <For each={adminAgents().filter((a) => a.id !== "vak")}>
            {(a) => <option value={a.id}>✦ {a.name} ({a.id})</option>}
          </For>
        </select>
      </label>

      <label class="inherit-toggle">
        <input type="checkbox" checked={pinRoute()} onChange={(e) => setPinRoute(e.currentTarget.checked)} />
        Give this channel its own model (otherwise it follows the workspace)
      </label>
      <Show when={pinRoute()}>
        <div class="binding-controls">
          <select
            value={provider()}
            ref={(el) => syncSelect(el, provider, () => props.providers)}
            onChange={(e) => setProvider(e.currentTarget.value)}
          >
            <For each={props.providers}>{(p) => <option value={p.name}>{providerLabel(p.name)}</option>}</For>
          </select>
          <Show when={models().length} fallback={
            <input class="mono" value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="model id" />
          }>
            <select
              class="mono"
              value={model()}
              ref={(el) => syncSelect(el, model, models)}
              onChange={(e) => setModel(e.currentTarget.value)}
            >
              <For each={models()}>{(m) => <option value={m}>{m}</option>}</For>
            </select>
          </Show>
        </div>
      </Show>

      <ChannelPermissionPicker
        pinned={pinPerm()}
        setPinned={setPinPerm}
        mode={perm()}
        setMode={setPerm}
      />
      <ChannelCapabilityPolicy value={policy()} onChange={setPolicy} />

      <div class="row-gap">
        <button disabled={busy()} onClick={approve}>Approve</button>
        <button class="danger small" disabled={busy()} onClick={deny}>Deny</button>
      </div>
    </article>
  );
}

/// Every bot on every surface. A bot is the only credential identity there
/// is (AGENTS.md invariant 23): the anonymous per-surface token slot that
/// used to sit beside this list could describe just one bot per transport,
/// so it is gone. Each row is its own credential *and* its own
/// policy/permission/route tier a chat can inherit from (see
/// `ChannelPolicy::merge` server-side).
/// A bot's own defaults — workspace, model, permission mode, and tool/skill/
/// automation policy — exactly the same controls and the same components
/// (`WorkspacePicker`, `ChannelPermissionPicker`, `ChannelCapabilityPolicy`)
/// as `ChannelAccessEditor` uses for one chat, so an operator sees one
/// consistent settings UI rather than two different ones depending on
/// whether they are editing a bot or a chat. A chat bound to this bot
/// inherits these unless it opts out (`inherit_bot_policy`) or pins its
/// own values, same as `ChannelAccessEditor`'s "Bot" picker already offers.
function BotAccessEditor(props: {
  bot: Bot;
  providers: ProviderSummary[];
  knownWorkspaces: string[];
  corePool: CorePoolEntry[];
  refresh: () => void;
}) {
  const [workspace, setWorkspace] = createSignal(props.bot.workspace ?? "");
  const [pinRoute, setPinRoute] = createSignal(!!props.bot.route);
  const [provider, setProvider] = createSignal(
    props.bot.route?.provider ?? props.providers[0]?.name ?? "",
  );
  const [model, setModel] = createSignal(props.bot.route?.model ?? "");
  const [models, setModels] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [pinPerm, setPinPerm] = createSignal(!!props.bot.permission_mode);
  const [perm, setPerm] = createSignal<PermissionMode>(props.bot.permission_mode ?? "workspace-write");
  const [policy, setPolicy] = createSignal<ChannelPolicy>(props.bot.policy ?? defaultChannelPolicy());
  const [pinVoice, setPinVoice] = createSignal(!!props.bot.voice);
  const [voiceName, setVoiceName] = createSignal(props.bot.voice?.voice_name ?? "");
  const [voiceProvider, setVoiceProvider] = createSignal(props.bot.voice?.provider ?? "");
  const [voicePersona, setVoicePersona] = createSignal(props.bot.voice?.persona ?? "");
  const [transcriptionModel, setTranscriptionModel] = createSignal(props.bot.voice?.transcription_model ?? "");
  const [synthesisModel, setSynthesisModel] = createSignal(props.bot.voice?.synthesis_model ?? "");
  const [realtimeModel, setRealtimeModel] = createSignal(props.bot.voice?.realtime_model ?? "");

  createEffect(async () => {
    if (!pinRoute() || !provider()) return;
    try {
      const found = (await api.models(provider())).models ?? [];
      setModels(found);
      if (found.length && !found.includes(model())) setModel(found[0]);
    } catch {
      setModels([]);
    }
  });

  const save = async () => {
    setBusy(true);
    try {
      await api.updateBot(props.bot.id, {
        workspace: workspace().trim() || null,
        route: pinRoute() && provider() && model() ? { provider: provider(), model: model() } : null,
        permission_mode: pinPerm() ? perm() : null,
        policy: policy(),
        voice: pinVoice() ? { provider: voiceProvider() || null, voice_name: voiceName() || null, persona: voicePersona() || null, transcription_model: transcriptionModel() || null, synthesis_model: synthesisModel() || null, realtime_model: realtimeModel() || null } : null,
      });
      await props.refresh();
      pushToast("info", `${props.bot.label} updated — chats bound to it pick this up on their next message`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      <p class="binding-meta" style="margin-bottom:6px">
        These are this bot's defaults across every chat bound to it. A chat can still override them
        for itself, or opt out of inheriting them entirely, from its own row under Chats.
      </p>
      <WorkspacePicker
        value={workspace()}
        onChange={setWorkspace}
        known={props.knownWorkspaces}
        corePool={props.corePool}
      />
      <label class="inherit-toggle">
        <input type="checkbox" checked={pinRoute()} onChange={(e) => setPinRoute(e.currentTarget.checked)} />
        Give this bot its own model (otherwise it follows the workspace)
      </label>
      <Show when={pinRoute()}>
        <div class="binding-controls">
          <select
            value={provider()}
            ref={(el) => syncSelect(el, provider, () => props.providers)}
            onChange={(e) => setProvider(e.currentTarget.value)}
          >
            <For each={props.providers}>{(p) => <option value={p.name}>{providerLabel(p.name)}</option>}</For>
          </select>
          <Show when={models().length} fallback={
            <input class="mono" value={model()} onInput={(e) => setModel(e.currentTarget.value)} placeholder="model id" />
          }>
            <select
              class="mono"
              value={model()}
              ref={(el) => syncSelect(el, model, models)}
              onChange={(e) => setModel(e.currentTarget.value)}
            >
              <For each={models()}>{(m) => <option value={m}>{m}</option>}</For>
            </select>
          </Show>
        </div>
      </Show>
      <ChannelPermissionPicker pinned={pinPerm()} setPinned={setPinPerm} mode={perm()} setMode={setPerm} subject="bot" />
      <VoiceConfigEditor
        pinned={pinVoice()}
        setPinned={setPinVoice}
        voiceName={voiceName()}
        setVoiceName={setVoiceName}
        transcriptionModel={transcriptionModel()}
        setTranscriptionModel={setTranscriptionModel}
        synthesisModel={synthesisModel()}
        setSynthesisModel={setSynthesisModel}
        realtimeModel={realtimeModel()}
        setRealtimeModel={setRealtimeModel}
        persona={voicePersona()}
        setPersona={setVoicePersona}
        subject="bot"
      />
      <ChannelCapabilityPolicy value={policy()} onChange={setPolicy} />
      <div class="row-gap">
        <button disabled={busy()} onClick={() => void save()}>{busy() ? "Saving…" : "Save changes"}</button>
      </div>
    </div>
  );
}

function BotRow(props: {
  bot: Bot;
  providers: ProviderSummary[];
  knownWorkspaces: string[];
  corePool: CorePoolEntry[];
  refresh: () => void;
}) {
  const [editing, setEditing] = createSignal(false);
  const [draft, setDraft] = createSignal("");
  const [busy, setBusy] = createSignal<"save" | "remove" | "delete" | "settings" | null>(null);
  const [settingsOpen, setSettingsOpen] = createSignal(false);

  const save = async () => {
    const token = draft().trim();
    if (!token) return;
    setBusy("save");
    try {
      await api.putBotIdToken(props.bot.id, token);
      await props.refresh();
      setEditing(false);
      setDraft("");
      pushToast("info", `${props.bot.label} token saved`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(null);
    }
  };

  const removeToken = async () => {
    if (!window.confirm(`Remove ${props.bot.label}'s token? Its chats go quiet until you set a new one.`)) return;
    setBusy("remove");
    try {
      await api.removeBotIdToken(props.bot.id);
      await props.refresh();
      pushToast("info", `${props.bot.label} token removed`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(null);
    }
  };

  const deleteBot = async () => {
    if (!window.confirm(`Delete the bot "${props.bot.label}"? Chats bound to it fall back to no bot tier.`)) return;
    setBusy("delete");
    try {
      await api.deleteBot(props.bot.id);
      await props.refresh();
      pushToast("info", `${props.bot.label} deleted`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div class="cred-block">
      <div class="cred-row">
        <div class="cred-id">
          <SurfaceBadge channelKey={`${props.bot.surface}:`} />
          <span class="binding-meta">{props.bot.label}</span>
          <code class="binding-meta">{props.bot.token_env}</code>
        </div>
        <span class="spacer" />
        <Show
          when={editing()}
          fallback={
            <div class="row-gap">
              <button class="ghost small" onClick={() => setSettingsOpen((v) => !v)}>
                {settingsOpen() ? "Hide settings" : "Settings"}
              </button>
              <button class="ghost small" onClick={() => setEditing(true)}>Set token</button>
              <button class="danger small" disabled={busy() === "remove"} onClick={() => void removeToken()}>Remove token</button>
              <button class="danger small" disabled={busy() === "delete"} onClick={() => void deleteBot()}>Delete bot</button>
            </div>
          }
        >
          <div class="cred-edit">
            <input
              type="password"
              autocomplete="off"
              spellcheck={false}
              autofocus
              placeholder={`Paste ${props.bot.label}'s token`}
              value={draft()}
              onInput={(e) => setDraft(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void save();
                if (e.key === "Escape") setEditing(false);
              }}
            />
            <button disabled={busy() === "save" || !draft().trim()} onClick={() => void save()}>
              {busy() === "save" ? "Saving…" : "Save"}
            </button>
            <button class="ghost small" onClick={() => { setEditing(false); setDraft(""); }}>Cancel</button>
          </div>
        </Show>
      </div>

      <Show when={settingsOpen()}>
        <div class="cred-settings">
          <BotAccessEditor
            bot={props.bot}
            providers={props.providers}
            knownWorkspaces={props.knownWorkspaces}
            corePool={props.corePool}
            refresh={props.refresh}
          />
        </div>
      </Show>
    </div>
  );
}

/// Every bot, plus the form to add one. No cap per surface: a transport
/// can carry as many independently-policed bot identities as you create.
function ExtraBotsList(props: { ctx: GatewayCtx }) {
  const [adding, setAdding] = createSignal(false);
  const [id, setId] = createSignal("");
  // Defaults to whatever the server lists first, so this file still names
  // no channel.
  const [surface, setSurface] = createSignal<string>("");
  createEffect(() => {
    if (!surface() && chatSurfaces().length > 0) setSurface(chatSurfaces()[0].id);
  });
  const [label, setLabel] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  const create = async () => {
    if (!id().trim()) return;
    setBusy(true);
    try {
      await api.createBot(id().trim(), surface(), label().trim() || id().trim());
      await props.ctx.refresh();
      pushToast("info", `Bot "${id().trim()}" added — set its token below`);
      setAdding(false);
      setId("");
      setLabel("");
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="cred-list" style={{ "margin-top": "0.75rem" }}>
      <Show when={props.ctx.bots().length > 0}>
        <For each={props.ctx.bots()}>
          {(bot) => (
            <BotRow
              bot={bot}
              providers={props.ctx.providers()}
              knownWorkspaces={props.ctx.status()?.known_workspaces ?? []}
              corePool={props.ctx.status()?.core_pool.entries ?? []}
              refresh={props.ctx.refresh}
            />
          )}
        </For>
      </Show>
      <Show
        when={adding()}
        fallback={
          <button class="ghost small" onClick={() => setAdding(true)}>+ Add another bot</button>
        }
      >
        <div class="cred-edit">
          <select value={surface()} onInput={(e) => setSurface(e.currentTarget.value)}>
            <For each={chatSurfaces()}>{(s) => <option value={s.id}>{surfaceLabel(s.id)}</option>}</For>
          </select>
          <input
            placeholder="id, e.g. telegram-sales"
            value={id()}
            onInput={(e) => setId(e.currentTarget.value)}
          />
          <input
            placeholder="Label (optional)"
            value={label()}
            onInput={(e) => setLabel(e.currentTarget.value)}
          />
          <button disabled={busy() || !id().trim()} onClick={() => void create()}>
            {busy() ? "Adding…" : "Add"}
          </button>
          <button class="ghost small" onClick={() => setAdding(false)}>Cancel</button>
        </div>
      </Show>
    </div>
  );
}

/// Everything the four gateway screens read, fetched once at the section
/// level so a refresh on one screen is a refresh on all of them.
interface GatewayCtx {
  status: () => GatewayStatus | undefined;
  statusLoading: () => boolean;
  entries: () => AllowlistEntry[];
  pending: () => AllowlistEntry[];
  providers: () => ProviderSummary[];
  /// Every bot identity. A bot owns its credential, policy, permission
  /// mode, and route; a surface is a transport, never a credential slot.
  bots: () => Bot[];
  botsLoading: () => boolean;
  refresh: () => void;
}

function SkeletonRows(props: { rows?: number; cols: number }) {
  return (
    <For each={Array.from({ length: props.rows ?? 3 })}>
      {() => (
        <tr class="skel-row">
          <For each={Array.from({ length: props.cols })}>{() => <td><span class="skel" /></td>}</For>
        </tr>
      )}
    </For>
  );
}

// ---- Gateway › Channels ----------------------------------------------------

/// The settled, scan-first view: every approved channel, what it routes
/// to, what permission it runs with. Editing is a row expansion, so the
/// page reads as a list of channels first and a form only on demand.
function ChannelsView(props: { ctx: GatewayCtx }) {
  const [filter, setFilter] = createSignal("");
  const [expanded, setExpanded] = createSignal("");
  const [manualKey, setManualKey] = createSignal("");
  const [registering, setRegistering] = createSignal(false);

  const byKey = createMemo(() => {
    const map = new Map<string, AllowlistEntry>();
    for (const e of props.ctx.entries()) map.set(e.key, e);
    return map;
  });

  // Multiple bots can share one surface (multi-bot-per-channel), so a chat
  // list with no bot column reads as if every Telegram chat talks to the
  // same bot. This resolves the label the row detail already lets you set
  // (`GatewayBindingEditor`'s bot picker) so the list itself shows it too.
  const botsById = createMemo(() => {
    const map = new Map<string, Bot>();
    for (const b of props.ctx.bots()) map.set(b.id, b);
    return map;
  });
  const botLabel = (botId: string | null | undefined) =>
    botId ? (botsById().get(botId)?.label ?? botId) : null;

  // Multi-bot-per-channel: a chat bound to a specific bot has a three-part
  // key (`surface:address:bot_id`) so the same physical chat served by
  // several bots gets independent entries. The bot id already has its own
  // column (`botLabel` above); repeating it in the chat id itself would
  // just be visual noise, so this strips that segment for display only —
  // `binding.target` (used for every API call, filter match, and the row
  // key) is never touched.
  const displayChatId = (target: string) => {
    const parts = target.split(":");
    return parts.length > 2 ? parts.slice(0, 2).join(":") : target;
  };

  const rows = createMemo(() => {
    const needle = filter().trim().toLowerCase();
    if (!needle) return props.ctx.status()?.bindings ?? [];
    return (props.ctx.status()?.bindings ?? []).filter((b) => {
      const label = botLabel(byKey().get(b.target)?.bot_id)?.toLowerCase() ?? "";
      return (
        b.target.toLowerCase().includes(needle) ||
        b.workspace.toLowerCase().includes(needle) ||
        label.includes(needle)
      );
    });
  });

  const registerManually = async () => {
    const key = manualKey().trim();
    const surface = key.split(":", 1)[0];
    if (!key.includes(":") || !surfaceIds().includes(surface)) {
      pushToast(
        "alert",
        `That doesn't look like a chat id. Write it as app:chat-id — for example telegram:12345 — where the app is one of ${surfaceIds().join(", ")}. Bot tokens go under Credentials, never here.`,
      );
      return;
    }
    setRegistering(true);
    try {
      await api.patchGatewayBinding(key, {});
      setManualKey("");
      await props.ctx.refresh();
      pushToast("info", `Added ${key}. It uses the workspace defaults until you change them.`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setRegistering(false);
    }
  };

  return (
    <>
      <div class="toolbar">
        <input
          class="search-input"
          placeholder="Find a chat or workspace…"
          value={filter()}
          onInput={(e) => setFilter(e.currentTarget.value)}
        />
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refresh()}>Refresh</button>
        <button onClick={() => navigate("#/gateway/connect")}>Connect a chat</button>
      </div>

      <Show when={props.ctx.pending().length > 0}>
        <section class="panel panel-alert callout">
          <div>
            <strong>
              {props.ctx.pending().length} chat{props.ctx.pending().length === 1 ? "" : "s"} waiting for review
            </strong>
            <p class="dim">
              Unrecognized chats are rejected on arrival and recorded — nothing is allowed in silently.
            </p>
          </div>
          <button class="ghost" onClick={() => navigate("#/gateway/connect")}>Review</button>
        </section>
      </Show>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Connected chats</h2>
            <Show when={(props.ctx.status()?.bindings.length ?? 0) > 0}>
              <p class="dim">
One row per approved chat: which workspace it works in, which model answers, and what it
                is allowed to do. Click a row to change any of that, or to disconnect it.
              </p>
            </Show>
          </div>
        </div>

        <Switch>
          <Match when={props.ctx.statusLoading()}>
            <table class="table">
              <thead>
                <tr><th>chat</th><th>app</th><th>agent</th><th>bot</th><th>workspace</th><th>model</th><th>can do</th><th>status</th><th /></tr>
              </thead>
              <tbody><SkeletonRows cols={9} /></tbody>
            </table>
          </Match>

          <Match when={(props.ctx.status()?.bindings.length ?? 0) === 0}>
            <div class="empty empty-teach">
              <strong>No chats are connected yet.</strong>
              <p>
A connected chat — a Telegram group, a Discord channel, a Slack conversation — lets people
                talk to vak from where they already are. It takes three steps: add the bot token,
                message the bot from that chat, then approve it here.
              </p>
              <button onClick={() => navigate("#/gateway/connect")}>Connect a chat</button>
            </div>
          </Match>

          <Match when={rows().length === 0}>
            <div class="empty">
              Nothing matches “{filter()}”.{" "}
              <button class="ghost small" onClick={() => setFilter("")}>Clear filter</button>
            </div>
          </Match>

          <Match when={rows().length > 0}>
            <table class="table">
              <thead>
                <tr><th>chat</th><th>app</th><th>agent</th><th>bot</th><th>workspace</th><th>model</th><th>can do</th><th>status</th><th /></tr>
              </thead>
              <tbody>
                <For each={rows()}>
                  {(binding) => {
                    const entry = () => byKey().get(binding.target);
                    const open = () => expanded() === binding.target;
                    return (
                      <>
                        <tr
                          tabindex="0"
                          classList={{ "row-open": open() }}
                          onClick={() => setExpanded(open() ? "" : binding.target)}
                          onKeyDown={(e) => e.key === "Enter" && setExpanded(open() ? "" : binding.target)}
                        >
                          <td class="mono">{displayChatId(binding.target)}</td>
                          <td><SurfaceBadge channelKey={binding.target} /></td>
                          <td>
                            <span class="chip chip-tone-info">
                              ✦ {entry()?.agent_id || "vak"}
                            </span>
                          </td>
                          <td>
                            <Show when={botLabel(entry()?.bot_id)} fallback={<span class="dim">—</span>}>
                              <span class="chip chip-mode">{botLabel(entry()?.bot_id)}</span>
                            </Show>
                          </td>
                          <td class="dim col-path"><PathCell path={binding.workspace} /></td>
                          <td class="mono dim">
                            {binding.effective_route.provider} / {binding.effective_route.model}
                            <Show when={binding.override}>
                              <span class="chip chip-mode" style="margin-left:6px">pinned</span>
                            </Show>
                          </td>
                          <td>
                            <Show when={entry()?.effective_permission_mode} fallback={<span class="dim">—</span>}>
                              <span class="chip chip-phrase chip-kind">{modeLabel(entry()!.effective_permission_mode)}</span>
                              <Show when={entry()!.permission_capped}>
                                <span class="chip chip-tone-warning" style="margin-left:6px" title="You asked for more than the workspace allows, so it was reduced to this.">reduced</span>
                              </Show>
                            </Show>
                          </td>
                          <td>
                            <Show when={entry()}>
                              <span class={`chip chip-tone-${ALLOWLIST_CHIP_TONE[entry()!.status] ?? ""}`}>
                                {entry()!.status}
                              </span>
                            </Show>
                            <Show when={binding.stale}>
                              <span class="chip chip-tone-warning" style="margin-left:6px" title="A setting changed. The next message starts a fresh session with the new one.">new session next</span>
                            </Show>
                          </td>
                          <td><span class="chev">{open() ? "⌄" : "›"}</span></td>
                        </tr>
                        <Show when={open()}>
                          <tr class="row-detail">
                            <td colspan={9}>
                              <GatewayBindingEditor
                                binding={binding}
                                providers={props.ctx.providers()}
                                allowlistEntry={entry()}
                                knownWorkspaces={props.ctx.status()?.known_workspaces ?? []}
                                corePool={props.ctx.status()?.core_pool.entries ?? []}
                                bots={props.ctx.bots()}
                                refresh={props.ctx.refresh}
                              />
                            </td>
                          </tr>
                        </Show>
                      </>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>

        <details class="advanced">
          <summary>Add a chat by its id</summary>
          <p class="dim">
            Only needed if you already know the chat’s id. Normally a chat adds itself the first time
            it messages the bot. Write it as <code>app:chat-id</code> — for example{" "}
            <code>telegram:12345</code>. This is never a bot token; those go under Credentials.
          </p>
          <div class="add-binding">
            <input
              class="mono"
              value={manualKey()}
              placeholder="telegram:12345"
              onInput={(e) => setManualKey(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && void registerManually()}
            />
            <button disabled={registering() || !manualKey().trim()} onClick={() => void registerManually()}>
              {registering() ? "Registering…" : "Register"}
            </button>
          </div>
        </details>
      </section>
    </>
  );
}

// ---- Gateway › Connect -----------------------------------------------------

/// Onboarding, as the sequence it actually is. Each step reports its own
/// state from the live backend rather than asking the operator to keep
/// track: a token either is set or is not, a chat either has knocked or
/// has not.
function ConnectView(props: { ctx: GatewayCtx }) {
  const credentialDone = () => props.ctx.bots().some((b) => b.token_configured);
  const knockDone = () => props.ctx.pending().length > 0 || (props.ctx.status()?.bindings.length ?? 0) > 0;
  const current = () => (!credentialDone() ? 1 : !props.ctx.pending().length ? 2 : 3);

  const stepState = (n: number) => {
    const done = n === 1 ? credentialDone() : n === 2 ? knockDone() : false;
    if (done && current() !== n) return "done";
    return current() === n ? "current" : "waiting";
  };

  return (
    <>
      <div class="toolbar">
        <div>
          <h2 class="view-title">Connect a chat</h2>
          <p class="dim">Three steps, from a bot with no token to a chat that can talk to vak.</p>
        </div>
        <span class="spacer" />
        <button class="ghost" onClick={() => props.ctx.refresh()}>Refresh</button>
        <button class="ghost" onClick={() => navigate("#/gateway")}>All chats</button>
      </div>

      <ol class="steps">
        <li class="step" data-state={stepState(1)}>
          <span class="step-index">1</span>
          <div class="step-body">
            <h3>Add the bot’s token</h3>
            <p class="dim">
              Get it from BotFather (Telegram), the Discord Developer Portal, or your Slack app’s
              “Bot User OAuth Token”. It is saved to your private <code>.env</code> and never shown
              again.
            </p>
            <ExtraBotsList ctx={props.ctx} />
          </div>
        </li>

        <li class="step" data-state={stepState(2)}>
          <span class="step-index">2</span>
          <div class="step-body">
            <h3>Message the bot from the chat you want to connect</h3>
            <p class="dim">
The first message is turned away on purpose. Vak notes the chat down instead of letting it
              in, and that is what puts it in front of you in step 3.
            </p>
            <Show
              when={props.ctx.pending().length > 0}
              fallback={
                <div class="waiting-line">
                  <span class={`dot dot-${conn()}`} />
                  <Show when={credentialDone()} fallback={<span class="dim">Waiting on step 1 — no bot token is set yet.</span>}>
                    <span class="dim">Listening. Send the bot any message and the chat appears in step 3.</span>
                  </Show>
                </div>
              }
            >
              <div class="waiting-line">
                <span class="chip chip-tone-success">
                  {props.ctx.pending().length} chat{props.ctx.pending().length === 1 ? "" : "s"} waiting
                </span>
              </div>
            </Show>
          </div>
        </li>

        <li class="step" data-state={stepState(3)}>
          <span class="step-index">3</span>
          <div class="step-body">
            <h3>Review and approve</h3>
            <p class="dim">
Approving connects the chat to a workspace, and lets you give it its own model and its own
              limits if you want to. Anything you leave alone follows the workspace, and all of it can
              be changed later from Channels.
            </p>
            <Show
              when={props.ctx.pending().length > 0}
              fallback={
                <div class="empty">
                  Nothing is waiting for review.{" "}
                  <Show when={(props.ctx.status()?.bindings.length ?? 0) > 0}>
                    <button class="ghost small" onClick={() => navigate("#/gateway")}>See connected chats</button>
                  </Show>
                </div>
              }
            >
              <div class="binding-list">
                <For each={props.ctx.pending()}>
                  {(entry) => (
                    <PendingChannelCard
                      entry={entry}
                      providers={props.ctx.providers()}
                      defaultWorkspace={props.ctx.status()?.workspace ?? ""}
                      knownWorkspaces={props.ctx.status()?.known_workspaces ?? []}
                      corePool={props.ctx.status()?.core_pool.entries ?? []}
                      refresh={props.ctx.refresh}
                    />
                  )}
                </For>
              </div>
            </Show>
          </div>
        </li>
      </ol>
    </>
  );
}

// ---- Gateway › Credentials -------------------------------------------------

function CredentialsView(props: { ctx: GatewayCtx }) {
  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>Bot tokens</h2>
          <p class="dim">
            One token per app — how vak signs in to Telegram, Discord, or Slack. Tokens are saved to
            your private <code>.env</code>; this page can set or clear one, but never shows it again.
            A chat id (like <code>telegram:12345</code>) is a different thing entirely and lives
            under Chats.
          </p>
        </div>
        <button class="ghost small" onClick={() => props.ctx.refresh()}>Refresh</button>
      </div>
      <ExtraBotsList ctx={props.ctx} />
    </section>
  );
}

// ---- Gateway › Routing & pool ----------------------------------------------

/// Where a default came from, as reported by `default_route.*_source`. An
/// unrecognised source still prints its own id rather than vanishing.
const ROUTE_SOURCES: Record<string, string> = {
  config: "the config file",
  admin: "set here in the console",
  default: "vak's built-in default",
  env: "an environment variable",
  cli: "a command-line flag",
};
const sourceLabel = (source: string | undefined) =>
  source ? (ROUTE_SOURCES[source] ?? source) : "—";

function GatewayHealthView(props: { ctx: GatewayCtx }) {
  const pool = () => props.ctx.status()?.core_pool;
  const [workspace, setWorkspace] = createSignal("");
  const [workspaceDirty, setWorkspaceDirty] = createSignal(false);
  createEffect(() => {
    const current = props.ctx.status()?.workspace;
    if (current && !workspaceDirty()) setWorkspace(current);
  });
  const saveWorkspace = async () => {
    const result = await api.patchGatewayWorkspace(workspace());
    await props.ctx.refresh();
    setWorkspaceDirty(false);
    if (result.restart_required) {
      window.setTimeout(() => window.location.reload(), 1200);
    }
  };
  return (
    <>
      <section class="panel gateway-summary">
        <div class="panel-title-row">
          <div>
            <h2>Defaults for connected chats</h2>
            <p class="dim">
              What a chat gets when it hasn’t been given anything of its own. Changing a default
              reaches every chat that follows it — each one picks it up on its next message rather
              than mid-conversation.
            </p>
          </div>
          <button class="ghost small" onClick={() => props.ctx.refresh()}>Refresh</button>
        </div>
        <Show
          when={!props.ctx.statusLoading()}
          fallback={<div class="route-summary-grid"><span class="skel skel-block" /><span class="skel skel-block" /><span class="skel skel-block" /><span class="skel skel-block" /></div>}
        >
          <div class="route-summary-grid">
            <div><span class="eyebrow">Gateway</span><strong>{props.ctx.status()?.enabled ? "Enabled" : "Disabled"}</strong></div>
            <div><span class="eyebrow">Workspace</span><PathCell path={props.ctx.status()?.workspace ?? ""} budget={46} /></div>
            <div><span class="eyebrow">Default model</span><strong>{providerLabel(props.ctx.status()?.default_route.provider ?? "")}</strong><code>{props.ctx.status()?.default_route.model}</code></div>
            <div>
              <span class="eyebrow">Where that comes from</span>
              <strong>{sourceLabel(props.ctx.status()?.default_route.provider_source)}</strong>
              <span class="binding-meta" title={`${props.ctx.status()?.default_route.provider_source} + ${props.ctx.status()?.default_route.model_source} · ${props.ctx.status()?.default_route.revision}`}>
                model: {sourceLabel(props.ctx.status()?.default_route.model_source)}
              </span>
            </div>
          </div>
        </Show>
        <div class="panel-subsection" style="margin-top:14px">
          <div class="panel-title-row">
            <div>
              <h3>Default workspace</h3>
              <p class="dim">New gateway chats start at the canonical <code>~/vak-home</code> unless you choose another folder. Bot and chat overrides remain independent below.</p>
            </div>
            <button disabled={!workspaceDirty()} onClick={() => void saveWorkspace()}>Save folder</button>
          </div>
          <WorkspacePicker
            value={workspace() || props.ctx.status()?.workspace || ""}
            onChange={(value) => { setWorkspace(value); setWorkspaceDirty(true); }}
            known={props.ctx.status()?.known_workspaces ?? []}
            corePool={pool()?.entries ?? []}
            catalog={props.ctx.status()?.workspace_catalog}
          />
          <button class="ghost small" onClick={() => { setWorkspace(props.ctx.status()?.canonical_default_workspace ?? ""); setWorkspaceDirty(true); }}>
            Use canonical ~/vak-home
          </button>
        </div>
      </section>

      <section class="panel" style="margin-top:14px"><WorkspaceNames ctx={props.ctx} /></section>

      <section class="panel" style="margin-top:14px">
        <div class="panel-title-row">
          <div>
            <h2>Workspaces loaded right now</h2>
            <p class="dim">
              A workspace stays loaded and ready after it is used. One that isn’t listed simply loads
              on the next message — nothing is lost either way. Up to {pool()?.max ?? "—"} stay
              loaded, and one drops off after{" "}
              {pool()?.idle_secs == null ? "—" : describeDuration(pool()!.idle_secs)} with nothing to
              do.
            </p>
          </div>
        </div>
        <Switch>
          <Match when={props.ctx.statusLoading()}>
            <div class="cred-list"><span class="skel skel-block" /><span class="skel skel-block" /></div>
          </Match>
          <Match when={(pool()?.entries.length ?? 0) === 0}>
            <div class="empty empty-teach">
              <strong>Nothing is loaded at the moment.</strong>
              <p>
                Nothing is wrong. A workspace loads on the first message to one of its chats and stays
                ready here until it has been idle for{" "}
                {pool()?.idle_secs == null ? "—" : describeDuration(pool()!.idle_secs)}.
              </p>
            </div>
          </Match>
          <Match when={(pool()?.entries.length ?? 0) > 0}>
            <table class="table">
              <thead>
                <tr><th>workspace</th><th>can do</th><th>idle for</th><th /></tr>
              </thead>
              <tbody>
                <For each={pool()?.entries}>
                  {(entry) => (
                    <tr class="row-static">
                      <td class="col-path"><PathCell path={entry.workspace} /></td>
                      <td><span class="chip chip-phrase chip-kind">{modeLabel(entry.effective_permission_mode)}</span></td>
                      <td class="dim">{shortDuration(entry.idle_secs)}</td>
                      <td>
                        <Show when={entry.is_default}><span class="chip chip-mode">default</span></Show>
                        <span class="chip chip-tone-success" style="margin-left:6px">ready</span>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>
      </section>
    </>
  );
}

// ---- Gateway section shell -------------------------------------------------

const GATEWAY_TABS = [
  { hash: "#/gateway", label: "Chats" },
  { hash: "#/gateway/connect", label: "Connect" },
  { hash: "#/gateway/credentials", label: "Credentials" },
  { hash: "#/gateway/routing", label: "Defaults & status" },
] as const;

function gatewayTab(): string {
  const r = route();
  return GATEWAY_TABS.slice(1).find((t) => r.startsWith(t.hash))?.hash ?? "#/gateway";
}

// ---- Setup wizard (docs/design/46, Part IV) --------------------------------


/// The three safety postures, in plain words.
///
/// Presets are a *projection* of the existing permission modes, never a
/// fourth mode (doc 46, Step 4). Full access is reachable and clearly
/// separated, and is never the recommendation.
const POSTURES: { mode: string; title: string; blurb: string; recommended?: boolean; danger?: boolean }[] = [
  { mode: "read-only", title: "Inspect only", blurb: "Read and search inside the folder. Changes nothing." },
  { mode: "workspace-write", title: "Work with approval", blurb: "Edit inside the folder; ask before shell commands.", recommended: true },
  { mode: "full-access", title: "Unrestricted", blurb: "Full access to this machine, unsandboxed.", danger: true },
];

/// The actions a step offers, inline.
///
/// The wizard *composes*; it does not reimplement the panels. Integrations
/// and bots already have complete screens, and a second implementation of
/// either here would be two contracts that must agree forever (AGENTS.md
/// invariant 30) — so those steps link, and the rest act in place.
function SetupActions(props: { step: keyof OnboardingState; done: () => void }) {
  const [busy, setBusy] = createSignal(false);
  const [key, setKey] = createSignal("");
  const [chosenProvider, setChosenProvider] = createSignal("");
  const [models, { refetch: refetchModels }] = createResource(
    () => chosenProvider() || undefined,
    (name: string) => api.models(name),
  );
  const [providers, { refetch: refetchProviders }] = createResource(() => api.providers());

  const run = async (what: string, f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
      pushToast("info", what);
      props.done();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Switch>
      <Match when={props.step === "capabilities"}>
        <button disabled={busy()} onClick={() => void run("Starter skills installed", () => api.seedCapabilities())}>
          {busy() ? "Installing…" : "Install starter skills"}
        </button>
      </Match>

      <Match when={props.step === "provider" || props.step === "route"}>
        <div class="setup-action-grid">
          <Show when={providers.error}>
            <div class="error-state" role="alert">
              Could not discover providers: {String(providers.error)}
              <button class="ghost small" type="button" onClick={() => void refetchProviders()}>Retry</button>
            </div>
          </Show>
          <select value={chosenProvider()} onChange={(e) => setChosenProvider(e.currentTarget.value)}>
            <option value="">Choose a service…</option>
            <For each={providers()?.providers ?? []}>
              {(p) => <option value={p.name}>{p.name}{p.configured ? " — key saved" : ""}</option>}
            </For>
          </select>
          <Show when={chosenProvider()}>
            <Show when={models.error}>
              <div class="error-state" role="alert">
                Could not discover models for {chosenProvider()}: {String(models.error)}
                <button class="ghost small" type="button" onClick={() => void refetchModels()}>Retry</button>
              </div>
            </Show>
            <input
              type="password"
              autocomplete="off"
              placeholder="Paste the API key (leave empty if already saved)"
              value={key()}
              onInput={(e) => setKey(e.currentTarget.value)}
            />
            <button
              disabled={busy()}
              onClick={() =>
                void run("Key saved — pick a model below", async () => {
                  if (key().trim()) await api.setProviderKey(chosenProvider(), key().trim(), "user");
                  setKey("");
                  // A stored key is not success: the route is only
                  // verified once the provider tells us what it can run.
                  await refetchModels();
                })
              }
            >
              Save key and list models
            </button>
            <Show when={(models()?.models ?? []).length > 0}>
              <select
                onChange={(e) =>
                  void run("Model saved", () =>
                    api.patchConfigScope("project", { provider: chosenProvider(), model: e.currentTarget.value }),
                  )
                }
              >
                <option value="">Choose a model…</option>
                <For each={models()?.models ?? []}>{(m) => <option value={m}>{m}</option>}</For>
              </select>
            </Show>
          </Show>
        </div>
      </Match>

      <Match when={props.step === "permission"}>
        <div class="setup-postures">
          <For each={POSTURES}>
            {(p) => (
              <button
                class={p.danger ? "danger small" : "ghost small"}
                disabled={busy()}
                onClick={() => {
                  // Unrestricted is a deliberate human decision and is
                  // never selected on someone's behalf (invariant 13).
                  if (p.danger && !window.confirm("Unrestricted means vak can reach anything on this machine, unsandboxed. Continue?")) return;
                  void run(`Set to ${p.title}`, () => api.setMode(p.mode));
                }}
              >
                <strong>{p.title}</strong>
                <span class="dim">{p.blurb}</span>
                <Show when={p.recommended}><span class="chip chip-ok">recommended</span></Show>
              </button>
            )}
          </For>
        </div>
      </Match>

      <Match when={props.step === "first_result"}>
        <button
          disabled={busy()}
          onClick={() =>
            void run("Starter session created — open it to watch it run", async () => {
              const started = await api.startFirstTask();
              location.hash = `#/sessions/${started.session_id}`;
            })
          }
        >
          {busy() ? "Starting…" : "Run the starter task"}
        </button>
        <p class="dim">
          It reads and explains this codebase. It cannot change anything: the run is
          capped to read-only even if you chose a more permissive setting above.
        </p>
      </Match>

      <Match when={props.step === "integrations"}>
        <a class="ghost small" href="#/integrations">Choose connected apps</a>
      </Match>

      <Match when={props.step === "channels"}>
        <a class="ghost small" href="#/gateway">Add a chat bot</a>
      </Match>
    </Switch>
  );
}

const SETUP_PHASES: {
  id: string;
  name: string;
  subtitle: string;
  keys: (keyof OnboardingState)[];
}[] = [
  {
    id: "foundation",
    name: "Phase 1: Environment & Host Foundation",
    subtitle: "App binaries, local dependencies, workspace folder containment, and configuration trust.",
    keys: ["install", "dependencies", "workspace", "trust"],
  },
  {
    id: "dispatch",
    name: "Phase 2: AI Dispatch & Boundary Sandbox",
    subtitle: "LLM credentials, active model route, permission ceiling, and OS sandbox containment.",
    keys: ["provider", "route", "permission", "sandbox"],
  },
  {
    id: "extensions",
    name: "Phase 3: Capabilities & Integrations",
    subtitle: "Starter skills, connected external apps (MCP / search), and inbound messaging bots.",
    keys: ["capabilities", "integrations", "channels"],
  },
  {
    id: "runtime",
    name: "Phase 4: Runtime Activation & First Flight",
    subtitle: "Service manager background daemons and initial read-only verification flight.",
    keys: ["services", "first_result"],
  },
];

function SetupWizard() {
  const [state, { refetch }] = createResource(() => api.onboarding());
  const [activating, setActivating] = createSignal(false);

  const step = (key: keyof OnboardingState) => state()?.[key] as StepState | undefined;

  const satisfiedCount = createMemo(() => {
    const s = state();
    if (!s) return 0;
    return SETUP_STEPS.filter((m) => step(m.key)?.state === "satisfied").length;
  });

  const progressPct = createMemo(() => {
    const total = SETUP_STEPS.length;
    if (total === 0) return 0;
    return Math.round((satisfiedCount() / total) * 100);
  });

  const activate = async () => {
    setActivating(true);
    try {
      const res = await api.activateServices();
      const failed = res.units.filter((u) => u.error);
      if (failed.length === 0) {
        pushToast("info", `Activated ${res.units.length} service(s)`);
      } else {
        pushToast("alert", `${failed.length} service(s) failed: ${failed.map((u) => u.name).join(", ")}`);
      }
      await refetch();
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setActivating(false);
    }
  };

  return (
    <div class="view setup-center-view">
      <PageHeader
        title="Setup & Readiness"
        description="Verify host prerequisites, AI model dispatch, sandbox containment, and platform service registration."
        actions={
          <div class="row-gap">
            <button class="ghost small" onClick={() => void refetch()}>
              Re-check Readiness
            </button>
            <button class="small" disabled={activating()} onClick={() => void activate()}>
              {activating() ? "Activating…" : "Activate Background Services"}
            </button>
          </div>
        }
      />

      <Show when={state()} fallback={<div class="skel skel-block" style={{ height: "240px", margin: "1rem 0" }} />}>
        {/* Executive Readiness KPI Deck */}
        <div class="setup-readiness-deck">
          <div class="setup-kpi-card">
            <span class="setup-kpi-label">Core Readiness</span>
            <div class="setup-kpi-value-row">
              <span class={`dot ${state()?.core_ready ? "dot-connected" : "dot-disconnected"}`} />
              <strong class="setup-kpi-val">{state()?.core_ready ? "Ready for Work" : "Action Needed"}</strong>
            </div>
            <span class="setup-kpi-sub">
              {state()?.core_ready ? "All prerequisites satisfied to execute tasks" : "Essential steps pending below"}
            </span>
          </div>

          <div class="setup-kpi-card">
            <span class="setup-kpi-label">Unattended Daemons</span>
            <div class="setup-kpi-value-row">
              <span class={`chip ${state()?.unattended_ready ? "chip-tone-success" : "chip-tone-neutral"}`}>
                {state()?.unattended_ready ? "Active & Healthy" : "Interactive Only"}
              </span>
            </div>
            <span class="setup-kpi-sub">
              {state()?.unattended_ready ? "Launchd / systemd services registered" : "Click Activate to enable background runs"}
            </span>
          </div>

          <div class="setup-kpi-card">
            <span class="setup-kpi-label">Requirements Verified</span>
            <div class="setup-kpi-value-row">
              <strong class="setup-kpi-val">{satisfiedCount()} / {SETUP_STEPS.length}</strong>
              <span class="setup-kpi-pct mono">{progressPct()}%</span>
            </div>
            <div class="setup-progress-meter">
              <div class="setup-progress-bar" style={{ width: `${progressPct()}%` }} />
            </div>
          </div>

          <div class="setup-kpi-card">
            <span class="setup-kpi-label">Target Workspace</span>
            <div class="setup-kpi-value-row">
              <span class="mono setup-workspace-tag">
                {step("workspace")?.state === "satisfied"
                  ? (step("workspace") as { detail: string }).detail
                  : "Not configured"}
              </span>
            </div>
            <span class="setup-kpi-sub">
              Canonical working directory for sessions & state
            </span>
          </div>
        </div>

        {/* 4-Phase Step Hierarchy */}
        <div class="setup-phases-container">
          <For each={SETUP_PHASES}>
            {(phase) => {
              const phaseSteps = () =>
                SETUP_STEPS.filter((m) => phase.keys.includes(m.key));
              const phaseSatisfied = () =>
                phaseSteps().filter((m) => step(m.key)?.state === "satisfied").length;

              return (
                <section class="setup-phase-section">
                  <div class="setup-phase-header">
                    <div class="setup-phase-titles">
                      <div class="setup-phase-eyebrow-row">
                        <span class="setup-phase-tag">{phase.id.toUpperCase()}</span>
                        <span class="setup-phase-counter">
                          {phaseSatisfied()} of {phaseSteps().length} satisfied
                        </span>
                      </div>
                      <h3 class="setup-phase-title">{phase.name}</h3>
                      <p class="setup-phase-subtitle dim">{phase.subtitle}</p>
                    </div>
                  </div>

                  <div class="setup-checklist">
                    <For each={phaseSteps()}>
                      {(meta) => {
                        const st = () => step(meta.key);
                        const sState = () => st()?.state ?? "incomplete";

                        return (
                          <div class="setup-item-row" data-state={sState()}>
                            <div class="setup-item-status-col">
                              <span class={`setup-item-indicator setup-item-${sState()}`} title={sState()}>
                                <Switch>
                                  <Match when={sState() === "satisfied"}>
                                    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="3">
                                      <polyline points="20 6 9 17 4 12" />
                                    </svg>
                                  </Match>
                                  <Match when={sState() === "not_applicable"}>
                                    <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2.5">
                                      <line x1="5" y1="12" x2="19" y2="12" />
                                    </svg>
                                  </Match>
                                  <Match when={sState() === "incomplete"}>
                                    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.5">
                                      <circle cx="12" cy="12" r="9" />
                                      <line x1="12" y1="8" x2="12" y2="12" />
                                      <line x1="12" y1="15" x2="12.01" y2="15" />
                                    </svg>
                                  </Match>
                                </Switch>
                              </span>
                            </div>

                            <div class="setup-item-main">
                              <div class="setup-item-head">
                                <span class="setup-item-title">{meta.title}</span>
                                <span class="setup-item-why">{meta.why}</span>
                              </div>

                              <div class="setup-item-body">
                                <Switch>
                                  <Match when={sState() === "satisfied"}>
                                    <div class="setup-item-satisfied">
                                      <span class="setup-item-detail mono">
                                        {(st() as { detail: string }).detail}
                                      </span>
                                      <Show when={(st() as { provenance?: string | null }).provenance}>
                                        {(p) => <span class="setup-item-prov">· {p()}</span>}
                                      </Show>
                                    </div>
                                  </Match>

                                  <Match when={sState() === "not_applicable"}>
                                    <div class="setup-item-na">
                                      <span class="dim">{(st() as { reason: string }).reason}</span>
                                    </div>
                                  </Match>

                                  <Match when={sState() === "incomplete"}>
                                    <div class="setup-item-problem">
                                      <div class="setup-problem-desc">
                                        <strong>{(st() as { what: string }).what}</strong>
                                        <Show when={(st() as { preserved: string }).preserved}>
                                          {(p) => <span class="dim"> — {p()}</span>}
                                        </Show>
                                      </div>
                                      <div class="setup-problem-repair-line">
                                        <span class="repair-tag">FIX</span> {(st() as { repair: string }).repair}
                                      </div>
                                      <Show when={(st() as { detail?: string | null }).detail}>
                                        {(d) => (
                                          <details class="setup-diag-details">
                                            <summary class="dim">Diagnostic details</summary>
                                            <pre class="mono">{d()}</pre>
                                          </details>
                                        )}
                                      </Show>
                                    </div>
                                  </Match>
                                </Switch>

                                <Show when={sState() !== "satisfied"}>
                                  <div class="setup-item-inline-actions">
                                    <SetupActions step={meta.key} done={() => refetch()} />
                                  </div>
                                </Show>
                              </div>
                            </div>
                          </div>
                        );
                      }}
                    </For>
                  </div>
                </section>
              );
            }}
          </For>
        </div>

        {/* Operational Footer Notice */}
        <div class="setup-operational-footer">
          <div class="setup-footer-content">
            <strong>Service Manager Registration Contract</strong>
            <p class="dim">
              Configuration files remain inert until background services are explicitly synchronized with launchd/systemd.
              Activating writes durable service units from the canonical workspace.
            </p>
          </div>
          <div class="setup-footer-actions">
            <button class="small" disabled={activating()} onClick={() => void activate()}>
              {activating() ? "Activating…" : "Activate Background Services"}
            </button>
          </div>
        </div>
      </Show>
    </div>
  );
}

function ChannelTopologyMatrix(props: { ctx: GatewayCtx }) {
  const allowedCount = createMemo(() => props.ctx.entries().filter((e) => e.status === "allowed").length);
  const pendingCount = createMemo(() => props.ctx.entries().filter((e) => e.status === "pending").length);
  const deniedCount = createMemo(() => props.ctx.entries().filter((e) => e.status === "denied").length);
  const bots = () => props.ctx.bots();
  const st = () => props.ctx.status();
  const surfaces = () => chatSurfaces();

  return (
    <section class="channel-topology-panel">
      <div class="panel-title-row">
        <div>
          <span class="eyebrow">Architecture & Delivery Flow</span>
          <h2>Channel & Multi-Bot Topology Matrix</h2>
          <p class="dim">
            4-tier inheritance chain (Global → Workspace → Bot → Chat). A chat can only narrow access, never escalate beyond its workspace ceiling (docs/design/34).
          </p>
        </div>
        <span
          class="chip"
          classList={{
            "chip-tone-success": st()?.enabled,
            "chip-phrase": !st()?.enabled,
          }}
        >
          {st()?.enabled ? "Gateway Active" : "Gateway Disabled"}
        </span>
      </div>

      <div class="channel-topology-matrix">
        {/* Tier 1: Global Baseline */}
        <div class="topology-tier-card">
          <div class="topology-tier-header">
            <span class="topology-tier-tag">Tier 1</span>
            <span class="chip chip-phrase">Global</span>
          </div>
          <strong class="topology-tier-title">Global Baseline</strong>
          <div class="topology-tier-body">
            <div>
              <span class="dim">Default Route:</span>{" "}
              <strong class="mono" style={{ "color": "var(--text)" }}>
                {st()?.default_route ? `${st()!.default_route.provider} / ${st()!.default_route.model}` : "Default"}
              </strong>
            </div>
            <div>
              <span class="dim">Chat Allowlist Mode:</span>{" "}
              <span class={`chip ${st()?.chat_allowlist_open ? "chip-tone-warning" : "chip-tone-success"}`}>
                {st()?.chat_allowlist_open ? "Open (Allow All)" : "Strict (Review Required)"}
              </span>
            </div>
            <p class="dim" style={{ "font-size": "11px", "margin": "4px 0 0" }}>
              Baseline route and security floor inherited by every workspace.
            </p>
          </div>
        </div>

        {/* Tier 2: Workspace Policy */}
        <div class="topology-tier-card">
          <div class="topology-tier-header">
            <span class="topology-tier-tag">Tier 2</span>
            <span class="chip chip-phrase">Workspace</span>
          </div>
          <strong class="topology-tier-title">Workspace Policy</strong>
          <div class="topology-tier-body">
            <div>
              <span class="dim">Root Directory:</span>
              <span class="mono wrap" style={{ "display": "block", "font-size": "11px", "color": "var(--text)" }}>
                {st()?.workspace || "—"}
              </span>
            </div>
            <div>
              <span class="dim">CorePool Occupancy:</span>{" "}
              <strong class="mono" style={{ "color": "var(--text)" }}>
                {st()?.core_pool ? `${st()!.core_pool.entries.length} warm / ${st()!.core_pool.max} max` : "—"}
              </strong>
            </div>
            <p class="dim" style={{ "font-size": "11px", "margin": "4px 0 0" }}>
              Isolates concurrent tenants into warm Core instances with distinct memory.
            </p>
          </div>
        </div>

        {/* Tier 3: Bot Identities */}
        <div class="topology-tier-card">
          <div class="topology-tier-header">
            <span class="topology-tier-tag">Tier 3</span>
            <span class="chip chip-tone-info">{bots().length} bot{bots().length === 1 ? "" : "s"}</span>
          </div>
          <strong class="topology-tier-title">Bot Identities</strong>
          <div class="topology-tier-body">
            <Show
              when={bots().length > 0}
              fallback={<p class="dim" style={{ "font-size": "11.5px" }}>No bots configured yet. Add a bot in the Bots tab.</p>}
            >
              <div class="topology-entity-list">
                <For each={bots()}>
                  {(b) => (
                    <div class="topology-entity-item">
                      <div>
                        <strong>{b.label}</strong>
                        <span class="dim mono" style={{ "font-size": "10.5px", "margin-left": "4px" }}>({b.surface})</span>
                      </div>
                      <span class={`chip chip-tone-${b.token_configured ? "success" : "warning"}`} style={{ "font-size": "10px" }}>
                        {b.token_configured ? "token set" : "no token"}
                      </span>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>
        </div>

        {/* Tier 4: Chat Endpoints */}
        <div class="topology-tier-card">
          <div class="topology-tier-header">
            <span class="topology-tier-tag">Tier 4</span>
            <span class="chip chip-phrase">{props.ctx.entries().length} registered</span>
          </div>
          <strong class="topology-tier-title">Chat Endpoints</strong>
          <div class="topology-tier-body">
            <div style={{ "display": "flex", "align-items": "center", "gap": "6px", "flex-wrap": "wrap" }}>
              <span class="chip chip-tone-success">{allowedCount()} allowed</span>
              <Show when={pendingCount() > 0}>
                <a href="#/gateway/connect" class="chip chip-tone-warning" style={{ "text-decoration": "none" }}>
                  {pendingCount()} pending review
                </a>
              </Show>
              <Show when={deniedCount() > 0}>
                <span class="chip chip-tone-danger">{deniedCount()} denied</span>
              </Show>
            </div>
            <p class="dim" style={{ "font-size": "11px", "margin": "4px 0 0" }}>
              Keys are <code>surface:chat:bot_id</code>. Policy chains: chat capped by bot capped by workspace.
            </p>
          </div>
        </div>
      </div>

      {/* Surface Transports Strip */}
      <div class="topology-surfaces-bar">
        <span class="eyebrow" style={{ "margin": "0" }}>Discovered Chat Transports:</span>
        <Show
          when={surfaces().length > 0}
          fallback={<span class="dim" style={{ "font-size": "11.5px" }}>No channel transports discovered.</span>}
        >
          <For each={surfaces()}>
            {(s) => (
              <span class="topology-surface-pill">
                <span class="topology-surface-dot" />
                <span>{s.label}</span>
              </span>
            )}
          </For>
        </Show>
      </div>
    </section>
  );
}

function GatewaySection() {
  // The transport list comes from the server, once, and every channel
  // control in this section reads it. Nothing here names a channel.
  createResource(() =>
    api
      .chatSurfaces()
      .then((surfaces) => {
        setChatSurfaces(surfaces);
        setChatSurfacesError(null);
        return surfaces;
      })
      .catch((error) => {
        setChatSurfacesError(error instanceof Error ? error.message : String(error));
        return [];
      }),
  );
  const [status, { refetch: refetchStatus }] = createResource(() => api.gatewayStatus());
  const [allowlist, { refetch: refetchAllowlist }] = createResource(() => api.gatewayAllowlist());
  const [providers] = createResource(() => api.providers());
  const [bots, { refetch: refetchBots }] = createResource(() => api.listBots());

  const refresh = () => {
    refetchStatus();
    refetchAllowlist();
    refetchBots();
  };

  const entries = createMemo(() => allowlist()?.entries ?? []);
  const ctx: GatewayCtx = {
    status: () => status(),
    statusLoading: () => status.loading,
    entries,
    pending: createMemo(() => entries().filter((e) => e.status === "pending")),
    providers: () => providers()?.providers ?? [],
    bots: () => bots()?.bots ?? [],
    botsLoading: () => bots.loading,
    refresh,
  };

  return (
    <div class="view">
      <PageHeader
        title="Channels"
        description="Manage the shared/workspace baseline, then narrow it per bot and per chat. A chat can never escalate beyond its workspace permission ceiling."
        actions={<a class="ghost small button-link" href="#/settings">Edit workspace baseline</a>}
      />
      <Show when={chatSurfacesError()}>
        <div class="error-state" role="alert">
          Channel discovery is unavailable. Empty channel options below are unknown,
          not unconfigured. <button class="ghost small" onClick={() => location.reload()}>Retry</button>
          <details><summary>Details</summary><pre class="mono">{chatSurfacesError()}</pre></details>
        </div>
      </Show>
      <Show when={providers.error || bots.error}>
        <section class="panel panel-alert callout" role="alert" style="margin-bottom:14px">
          <strong>Some channel configuration is unavailable</strong>
          <p class="dim">
            {providers.error ? `Provider discovery failed: ${String(providers.error)}. ` : ""}
            {bots.error ? `Bot discovery failed: ${String(bots.error)}.` : ""}
            Empty sections below must not be treated as unconfigured.
          </p>
        </section>
      </Show>
      <ChannelTopologyMatrix ctx={ctx} />
      <div class="tab-bar">
        <For each={GATEWAY_TABS}>
          {(t) => (
            <a
              class="tab-btn"
              href={t.hash}
              classList={{ active: gatewayTab() === t.hash }}
            >
              {t.label}
              <Show when={t.hash === "#/gateway/connect" && ctx.pending().length > 0}>
                <span class="nav-badge">{ctx.pending().length}</span>
              </Show>
            </a>
          )}
        </For>
      </div>

      <Switch>
        <Match when={gatewayTab() === "#/gateway"}><ChannelsView ctx={ctx} /></Match>
        <Match when={gatewayTab() === "#/gateway/connect"}><ConnectView ctx={ctx} /></Match>
        <Match when={gatewayTab() === "#/gateway/credentials"}><CredentialsView ctx={ctx} /></Match>
        <Match when={gatewayTab() === "#/gateway/routing"}><GatewayHealthView ctx={ctx} /></Match>
      </Switch>
    </div>
  );
}

/// The three rule lists, titled and explained the way they read rather than
/// by the keyword they use in config.
const RULE_SECTIONS: { decision: RuleDecision; title: string; empty: string }[] = [
  {
    decision: "deny",
    title: "Never allowed",
    empty: "Nothing is blocked outright beyond what the setting above already decides.",
  },
  {
    decision: "ask",
    title: "Always ask me first",
    empty: "Nothing is forced to check with you beyond what the setting above already decides.",
  },
  {
    decision: "allow",
    title: "Always allowed",
    empty: "Nothing is pre-approved beyond what the setting above already decides.",
  },
];

const MODE_COPY: Record<string, string> = {
  ReadOnly: "It can read and search this workspace, and nothing else. Every change is refused.",
  WorkspaceWrite: "It can change files inside this workspace on its own. Anything outside asks you first.",
  FullAccess: "Nothing is checked with you first. Only for a workspace you trust completely.",
};

const APPROVAL_MODES: { value: "ask" | "approve-safe" | "auto-approve"; label: string; desc: string }[] = [
  { value: "ask", label: "Ask for approval", desc: "Pause before actions that need approval." },
  { value: "approve-safe", label: "Approve safe actions", desc: "Automatically approve reads and actions inside the restricted sandbox; still ask for network and external access." },
  { value: "auto-approve", label: "Auto-approve", desc: "Automatically resolve ordinary Ask decisions. Explicit rules and circuit-breaker stops still require approval; permission denies and the sandbox still apply." },
];

/// Every chat that can be chosen as the approver: the approved ones, plus
/// whatever is currently set even when it is not among them.
function approverOptions(policy: GatewayApprovalPolicy): string[] {
  const out = [...(policy.candidates ?? [])];
  if (policy.approver && !out.includes(policy.approver)) out.unshift(policy.approver);
  return out;
}

/// Whether an approval gate raised on a chat surface reaches a human.
///
/// This is the setting that was readable everywhere and writable nowhere.
/// With `deny` — the default — every `Ask` on a chat surface is a foregone
/// denial, so `webfetch`, `browse`, and every MCP server are dropped from
/// the turn before the model sees them, and the only remedy the system
/// could print was "edit config.toml by hand and restart".
function ApprovalForwarding() {
  const [policy, { refetch }] = createResource(() => api.gatewayApprovals());
  const [target, setTarget] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  // Seed the input from the live policy, or from the only candidate when
  // there is exactly one — the common case, and typing a chat address from
  // memory is not something anyone should be asked to do.
  createEffect(() => {
    const p = policy();
    if (!p || target()) return;
    const only = p.candidates?.length === 1 ? p.candidates[0] : "";
    setTarget(p.approver ?? only);
  });

  const save = async (mode: "deny" | "forward") => {
    setBusy(true);
    try {
      const next = await api.setGatewayApprovals(
        mode === "forward" ? { mode, approver: target().trim() } : { mode },
      );
      await refetch();
      pushToast(
        "info",
        next.mode === "forward"
          ? `Gates now go to ${next.approver} for a yes or no`
          : "Gates on chat surfaces are refused without asking",
      );
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>Can a chat ask you first?</h2>
          <p class="dim">
            When vak running in a chat hits something that needs approval, it either refuses on
            the spot or asks you in a chat you choose.
          </p>
        </div>
        <Show when={policy()}>
          {(p) => (
            <span class={`chip ${p().forwarding ? "chip-ok" : "chip-warn"}`}>
              {p().forwarding ? "asks you" : "refuses"}
            </span>
          )}
        </Show>
      </div>

      <Show when={policy()} fallback={<div class="skel skel-block" />}>
        {(p) => (
          <>
            <Show when={p().gateway_enabled === false || p().enabled === false}>
              <p class="dim">
                Chat surfaces are switched off, so nothing here takes effect yet. Turn them on
                under <a href="#/gateway">Chats</a>.
              </p>
            </Show>

            <div class="mode-grid">
              <button
                class="mode-btn"
                classList={{ active: p().mode === "deny" }}
                disabled={busy() || p().mode === "deny"}
                onClick={() => void save("deny")}
              >
                <span class="mode-name">
                  Refuse without asking
                  <Show when={p().mode === "deny"}>
                    <span class="chip chip-tone-success">current</span>
                  </Show>
                </span>
                <span class="mode-desc">
                  Safest, and it means the web, the browser, and connected apps are unavailable in
                  chat — vak will say so instead of trying.
                </span>
              </button>
              <button
                class="mode-btn"
                classList={{ active: p().mode === "forward" }}
                disabled={busy() || !target().trim()}
                onClick={() => void save("forward")}
              >
                <span class="mode-name">
                  Ask me in a chat
                  <Show when={p().mode === "forward"}>
                    <span class="chip chip-tone-success">current</span>
                  </Show>
                </span>
                <span class="mode-desc">
                  The request is sent to the chat below; reply “yes” or “no”. No answer within{" "}
                  {Math.round(p().timeout_secs / 60)} minutes counts as no.
                </span>
              </button>
            </div>

            <div class="form-row" style={{ "margin-top": "12px" }}>
              <label>Ask me here</label>
              {/* A target set by hand in config.toml need not be one of this
                  gateway's approved chats. Offering only the candidates
                  would hide it, so the current value is always an option —
                  an operator must be able to see what is set before
                  deciding whether to change it. */}
              <Show
                when={(p().candidates?.length ?? 0) > 0 || p().approver}
                fallback={
                  <p class="dim">
                    No chats are approved yet. Add one under <a href="#/gateway">Chats</a> first —
                    a request can only be sent somewhere vak is already allowed to talk.
                  </p>
                }
              >
                <select value={target()} onChange={(e) => setTarget(e.currentTarget.value)}>
                  <option value="">Choose a chat…</option>
                  <For each={approverOptions(p())}>{(c) => <option value={c}>{c}</option>}</For>
                </select>
              </Show>
              <Show when={p().approver && !(p().candidates ?? []).includes(p().approver!)}>
                <p class="dim">
                  <code>{p().approver}</code> is not one of this gateway’s approved chats. It was
                  set outside the console, and a request sent there may not reach anyone.
                </p>
              </Show>
            </div>
          </>
        )}
      </Show>
    </section>
  );
}

/// Add and remove permission rules in one config layer.
///
/// The console already sets the permission mode and the approval mode, both
/// of which are strictly broader powers than a single rule, so the old copy
/// here — "can't be changed from a browser" — drew a line the rest of this
/// page had already crossed. What actually needs care is validation and
/// scope, both of which the server enforces: a malformed rule is rejected
/// whole rather than half-written, and a learned Allow can never shadow an
/// explicit Deny because the engine aggregates by severity.
function RuleEditor(props: { scope: ConfigScope; onSaved: () => void | Promise<void> }) {
  const [view, { refetch }] = createResource(
    () => props.scope,
    (scope: ConfigScope) => api.permissionRules(scope),
  );
  const [open, setOpen] = createSignal(false);
  const [decision, setDecision] = createSignal<RuleDecision>("deny");
  const [pattern, setPattern] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  const listFor = (d: RuleDecision) => {
    const layer = view()?.layer;
    if (!layer) return [] as string[];
    return d === "allow" ? layer.allow : d === "ask" ? layer.ask : layer.deny;
  };

  const write = async (d: RuleDecision, next: string[], message: string) => {
    setBusy(true);
    try {
      await api.setPermissionRules(props.scope, { [d]: next });
      await Promise.all([refetch(), props.onSaved()]);
      pushToast("info", message);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setBusy(false);
    }
  };

  const add = () => {
    const spec = pattern().trim();
    if (!spec) return;
    const d = decision();
    if (listFor(d).includes(spec)) {
      pushToast("alert", "That rule is already set in this scope");
      return;
    }
    void write(d, [...listFor(d), spec], `Added ${d} rule ${spec}`).then(() => setPattern(""));
  };

  return (
    <div class="rule-editor">
      <div class="row-gap">
        <button type="button" class="ghost small" onClick={() => setOpen(!open())}>
          {open() ? "Done editing" : "Edit rules"}
        </button>
        <span class="dim">
          Editing the {props.scope === "user" ? "Global" : "Workspace"} settings.
        </span>
      </div>

      <Show when={open()}>
        <div class="form-row" style={{ "margin-top": "10px" }}>
          <label>Add a rule</label>
          <div class="row-gap">
            <select
              value={decision()}
              onChange={(e) => setDecision(e.currentTarget.value as RuleDecision)}
            >
              <option value="deny">Never allow</option>
              <option value="ask">Always ask first</option>
              <option value="allow">Always allow</option>
            </select>
            <input
              placeholder="Bash(git *)"
              value={pattern()}
              onInput={(e) => setPattern(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && add()}
            />
            <button disabled={busy() || !pattern().trim()} onClick={() => add()}>
              Add
            </button>
          </div>
        </div>
        <p class="dim">
          A rule is a tool name, optionally with a pattern in brackets:{" "}
          <code>Bash(git *)</code>, <code>Edit(src/**)</code>, <code>Mcp(tavily/*)</code>, or just{" "}
          <code>Webfetch</code> for every use of it. “Never allow” always wins over the other two.
        </p>

        <Show when={view()}>
          <div class="rule-lists">
            <For each={RULE_SECTIONS}>
              {({ decision: d, title }) => (
                <div>
          <span class="eyebrow">{title} — set in this scope</span>
                  <Show
                    when={listFor(d).length > 0}
                    fallback={<p class="dim">Nothing set here.</p>}
                  >
                    <div class="chip-stack">
                      <For each={listFor(d)}>
                        {(spec) => (
                          <button
                            class="chip chip-phrase"
                            disabled={busy()}
                            title={`Remove ${spec}`}
                            onClick={() =>
                              void write(
                                d,
                                listFor(d).filter((s) => s !== spec),
                                `Removed ${spec}`,
                              )
                            }
                          >
                            {spec} <span aria-hidden="true">×</span>
                          </button>
                        )}
                      </For>
                    </div>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Show>
    </div>
  );
}

/// One page, six self-contained panels. Each answers a single question about
/// how this instance is configured; nothing here is a summary of a screen
/// that already exists elsewhere.
function Settings() {
  const [config, { refetch: refetchConfig }] = createResource(() => api.config());
  const [layer, { refetch: refetchLayer }] = createResource(configScope, (scope) => api.configLayer(scope));
  const [providersData, { refetch: refetchProviders }] = createResource(() => api.providers());
  const [rebuilding, setRebuilding] = createSignal(false);
  const [doctorReport, setDoctorReport] = createSignal<string | null>(null);
  const [runningDoctor, setRunningDoctor] = createSignal(false);

  const [selectedProvider, setSelectedProvider] = createSignal("anthropic");
  const [selectedModel, setSelectedModel] = createSignal("");
  const [discoveredModels, setDiscoveredModels] = createSignal<string[]>([]);
  const [bedrockAvailability, setBedrockAvailability] = createSignal<import("./types").BedrockModelAvailability[]>([]);
  const [loadingModels, setLoadingModels] = createSignal(false);
  const [modelError, setModelError] = createSignal("");
  const [modelSearch, setModelSearch] = createSignal("");
  const [providerKeyInput, setProviderKeyInput] = createSignal("");
  const [savingKey, setSavingKey] = createSignal(false);
  const [maxTurnsInput, setMaxTurnsInput] = createSignal("");
  const [savingMaxTurns, setSavingMaxTurns] = createSignal(false);
  const [togglingSubagents, setTogglingSubagents] = createSignal(false);
  const [probing, setProbing] = createSignal(false);
  const [probeResult, setProbeResult] = createSignal<{
    ok: boolean;
    latency_ms: number;
    model_count: number;
    has_active_model: boolean;
    message: string;
  } | null>(null);

  const testProviderConnection = async () => {
    const prov = selectedProvider();
    const mod = selectedModel();
    setProbing(true);
    const start = performance.now();
    try {
      const res = await api.models(prov);
      const latency = Math.round(performance.now() - start);
      const models = res.models ?? [];
      const hasActive = models.includes(mod);
      setProbeResult({
        ok: true,
        latency_ms: latency,
        model_count: models.length,
        has_active_model: hasActive,
        message: hasActive
          ? `Successfully reached ${providerLabel(prov)} API (${latency}ms) and verified active model "${mod}".`
          : `Reached ${providerLabel(prov)} API (${models.length} models discovered), but selected model "${mod}" was not listed in the catalogue.`,
      });
    } catch (err) {
      const latency = Math.round(performance.now() - start);
      setProbeResult({
        ok: false,
        latency_ms: latency,
        model_count: 0,
        has_active_model: false,
        message: `Connection failed: ${err}`,
      });
    } finally {
      setProbing(false);
    }
  };

  const [busData, { refetch: refetchBus }] = createResource(() => api.busConfig());
  const [busUrl, setBusUrl] = createSignal("");
  const [busJwt, setBusJwt] = createSignal("");
  const [busNkey, setBusNkey] = createSignal("");
  const [busSecretEnv, setBusSecretEnv] = createSignal("");
  const [savingBus, setSavingBus] = createSignal(false);

  const [activeTab, setActiveTab] = createSignal<"models" | "permissions" | "infrastructure" | "preferences">("models");

  createEffect(() => {
    const r = route().split("?", 1)[0];
    if (r.startsWith("#/settings/perm") || r.startsWith("#/settings/sec")) setActiveTab("permissions");
    else if (r.startsWith("#/settings/infra") || r.startsWith("#/settings/bus")) setActiveTab("infrastructure");
    else if (r.startsWith("#/settings/pref") || r.startsWith("#/settings/sys") || r.startsWith("#/settings/diag")) setActiveTab("preferences");
    else setActiveTab("models");
  });

  // Re-initialize the form whenever the layer resource resolves to a NEW value
  let initializedLayer: ConfigLayer | undefined;
  createEffect(() => {
    const c = config();
    const selectedLayer = layer();
    const scope = configScope();
    if (c && selectedLayer && selectedLayer !== initializedLayer) {
      initializedLayer = selectedLayer;
      setSelectedProvider(selectedLayer.provider || (scope === "project" ? c.provider : "anthropic"));
      setSelectedModel(selectedLayer.model || (scope === "project" ? c.model : ""));
      setMaxTurnsInput(String(selectedLayer.max_turns ?? (scope === "project" ? c.max_turns : "")));
    }
  });

  const discover = async (provider: string) => {
    setLoadingModels(true);
    setModelError("");
    try {
      const res = await api.models(provider);
      const models = res.models ?? [];
      setDiscoveredModels(models);
      setBedrockAvailability(res.availability ?? []);
      setModelError(res.availability_error ?? "");
      if (models.length > 0 && !models.includes(selectedModel())) setSelectedModel(models[0]);
    } catch (err) {
      setDiscoveredModels([]);
      setBedrockAvailability([]);
      setModelError(`${err}`);
    } finally {
      setLoadingModels(false);
    }
  };

  createEffect(() => {
    const provider = selectedProvider();
    setModelSearch("");
    if (provider) void discover(provider);
  });

  const filteredDiscoveredModels = createMemo(() => {
    const q = modelSearch().trim().toLowerCase();
    const list = discoveredModels();
    if (!q) return list;
    return list.filter((m) => m.toLowerCase().includes(q));
  });

  const keyConfigured = createMemo(
    () => providersData()?.providers?.find((p) => p.name === selectedProvider())?.configured ?? false,
  );

  const guard = async (work: () => Promise<void>, ok: string) => {
    try {
      await work();
      await Promise.all([refetchConfig(), refetchLayer()]);
      pushToast("info", ok);
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  const saveBus = async () => {
    if (savingBus()) return;
    setSavingBus(true);
    try {
      const body: Record<string, string> = {};
      if (busUrl().trim()) body.nats_url = busUrl().trim();
      if (busJwt().trim()) body.nats_credentials_jwt = busJwt().trim();
      if (busNkey().trim()) body.nats_nkey_seed = busNkey().trim();
      if (busSecretEnv().trim()) body.workspace_secret_env = busSecretEnv().trim();
      await api.putBusConfig(body);
      await refetchBus();
      pushToast("info", "Bus configuration saved — takes effect on next server restart");
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setSavingBus(false);
    }
  };

  const clearBus = async () => {
    if (!confirmDestructive("Remove all NATS credentials from the workspace .env? The bus connection will stop on restart.")) return;
    try {
      await api.deleteBusConfig();
      await refetchBus();
      pushToast("info", "Bus credentials removed");
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    }
  };

  const saveKey = async () => {
    if (!providerKeyInput().trim() || savingKey()) return;
    setSavingKey(true);
    try {
      await api.setProviderKey(selectedProvider(), providerKeyInput().trim(), configScope());
      await refetchProviders();
      pushToast("info", `Key saved for ${providerLabel(selectedProvider())} in ${configScope() === "user" ? "Global" : "Workspace"}`);
      setProviderKeyInput("");
      await discover(selectedProvider());
    } catch (err) {
      if (err instanceof AuthRequired) setAuthed(false);
      else pushToast("alert", `${err}`);
    } finally {
      setSavingKey(false);
    }
  };

  const rebuild = async () => {
    setRebuilding(true);
    try {
      const stats = await api.rebuild();
      if (stats.ok) pushToast("info", `Reindexed ${stats.entries_indexed} entries from ${stats.files_scanned} files`);
      else pushToast("alert", `Rebuild failed: ${stats.error}`);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setRebuilding(false);
    }
  };

  const runDoctor = async () => {
    setRunningDoctor(true);
    try {
      setDoctorReport((await api.doctor()).report);
    } catch (err) {
      pushToast("alert", `${err}`);
    } finally {
      setRunningDoctor(false);
    }
  };

  const signOut = async () => {
    await api.logout();
    disconnectEvents();
    setAuthed(false);
    navigate("#/overview");
  };

  const rules = createMemo(() => parseRuleLists(config()?.permissions));
  const rulesFor = (decision: RuleDecision) => rules().filter((r) => r.decision === decision);
  const selectedPermissionMode = () => layer()?.permission_mode ?? config()?.permission_mode;
  const selectedApprovalMode = () => layer()?.approval_mode ?? config()?.approval_mode;
  const inheritedPermissionMode = () => !layer()?.permission_mode;
  const inheritedApprovalMode = () => !layer()?.approval_mode;

  return (
    <div class="view">
      <PageHeader
        title="Settings & Configuration"
        description="Configure LLM routing, runtime permissions, event fabric, and local preferences."
      />
      <Show when={config.error || layer.error || providersData.error}>
        <div class="error-state" role="alert">
          <strong>Some settings could not be loaded.</strong>
          <p>{String(config.error || layer.error || providersData.error)}</p>
          <button class="ghost small" type="button" onClick={() => { void refetchConfig(); void refetchLayer(); void refetchProviders(); }}>Retry settings</button>
        </div>
      </Show>

      <div class="settings-nav-bar">
        <button
          type="button"
          class={`settings-tab-btn ${activeTab() === "models" ? "active" : ""}`}
          onClick={() => { setActiveTab("models"); navigate("#/settings"); }}
        >
          <span class="tab-icon">✦</span> Models & AI
          <span class="tab-pill mono">{selectedModel() || "Default"}</span>
        </button>
        <button
          type="button"
          class={`settings-tab-btn ${activeTab() === "permissions" ? "active" : ""}`}
          onClick={() => { setActiveTab("permissions"); navigate("#/settings/permissions"); }}
        >
          <span class="tab-icon">🛡</span> Permissions & Governance
          <span class="tab-pill">{modeLabel(selectedPermissionMode())}</span>
        </button>
        <button
          type="button"
          class={`settings-tab-btn ${activeTab() === "infrastructure" ? "active" : ""}`}
          onClick={() => { setActiveTab("infrastructure"); navigate("#/settings/infrastructure"); }}
        >
          <span class="tab-icon">⚡</span> Event Bus & Fabric
          <span class={`tab-pill ${busData()?.runtime?.connected ? "pill-ok" : ""}`}>
            {busData()?.runtime?.backend === "nats" ? "NATS" : "In-Process"}
          </span>
        </button>
        <button
          type="button"
          class={`settings-tab-btn ${activeTab() === "preferences" ? "active" : ""}`}
          onClick={() => { setActiveTab("preferences"); navigate("#/settings/preferences"); }}
        >
          <span class="tab-icon">⚙</span> System & Maintenance
          <span class="tab-pill">{theme()}</span>
        </button>
      </div>

      <Show when={activeTab() === "models"}>
        <div class="two-col">
          <div class="stack">
            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Model Route</h2>
                  <p class="dim">Primary provider and model answering turns, planned fresh per turn ladder.</p>
                </div>
              </div>
              <Show when={!config.loading} fallback={<div class="cred-list"><span class="skel skel-block" /><span class="skel skel-block" /></div>}>
                <div class="form-row">
                  <label>Provider</label>
                  <select
                    aria-label="Provider"
                    value={selectedProvider()}
                    ref={(el) => syncSelect(el, selectedProvider, () => providersData()?.providers)}
                    onChange={(e) => setSelectedProvider(e.currentTarget.value)}
                  >
                    <For each={providersData()?.providers ?? []}>
                      {(p) => <option value={p.name}>
                        {providerLabel(p.name)} {p.configured ? "✓ (configured)" : p.name === "ollama" ? "(local endpoint)" : "(key required)"}
                      </option>}
                    </For>
                  </select>
                </div>

                <div class="form-row">
                  <label>Model</label>
                  <Show
                    when={discoveredModels().length > 0}
                    fallback={
                      <input
                        class="mono"
                        aria-label="Model"
                        placeholder={loadingModels() ? "Asking the provider…" : "e.g. claude-sonnet-4-5, gemma4:e2b-mlx"}
                        value={selectedModel()}
                        onInput={(e) => setSelectedModel(e.currentTarget.value)}
                      />
                    }
                  >
                    <div style="display:flex; flex-direction:column; gap:6px; width:100%">
                      <div class="model-search-box">
                        <span class="model-search-icon">🔍</span>
                        <input
                          type="text"
                          class="model-search-input"
                          placeholder={`Filter ${discoveredModels().length} models or type custom name…`}
                          value={modelSearch()}
                          onInput={(e) => setModelSearch(e.currentTarget.value)}
                        />
                        <Show when={modelSearch()}>
                          <button class="model-search-clear" type="button" onClick={() => setModelSearch("")}>×</button>
                        </Show>
                      </div>

                      <select
                        aria-label="Model"
                        value={selectedModel()}
                        ref={(el) => syncSelect(el, selectedModel, filteredDiscoveredModels)}
                        onChange={(e) => setSelectedModel(e.currentTarget.value)}
                      >
                        <For each={filteredDiscoveredModels()}>{(m) => {
                          const status = bedrockAvailability().find((item) => item.model_id === m);
                          return <option value={m} disabled={selectedProvider() === "bedrock" && !!status && !status.invokable}>
                            {status && !status.invokable ? `${m} (not invokable)` : m}
                          </option>;
                        }}</For>
                      </select>

                      <Show when={modelSearch().trim() && !discoveredModels().includes(modelSearch().trim())}>
                        <button
                          type="button"
                          class="custom-model-chip"
                          onClick={() => setSelectedModel(modelSearch().trim())}
                        >
                          + Set model to custom tag: "{modelSearch().trim()}"
                        </button>
                      </Show>
                    </div>
                  </Show>
                </div>

                <div class="model-selection-badge">
                  <span class="dim">Active Selection:</span>
                  <strong class="mono">{providerLabel(selectedProvider())}</strong>
                  <span class="mono">· {selectedModel() || "None"}</span>
                </div>

                <Show when={modelError() && discoveredModels().length === 0}>
                  <p class="dim">
                    Couldn’t list models for {providerLabel(selectedProvider())} — usually because
                    its key isn’t stored yet. Add the key below, or type a model name in by hand.
                    ({modelError()})
                  </p>
                </Show>
                <Show when={selectedProvider() === "bedrock" && bedrockAvailability().length > 0}>
                  <div class="dim" style={{ "margin-top": "8px" }}>
                    <p>
                      <span class="chip chip-tone-success">{bedrockAvailability().filter((item) => item.invokable).length} invokable</span>{" "}
                      <span class="chip chip-tone-warning">{bedrockAvailability().filter((item) => !item.invokable).length} require entitlement/agreement</span>
                    </p>
                  </div>
                </Show>
                <Show when={selectedProvider() === "bedrock" && modelError()}>
                  <p class="dim">Bedrock authorization status could not be checked: {modelError()}</p>
                </Show>

                <div class="row-gap" style="margin-top:10px">
                  <button
                    disabled={!selectedModel().trim()}
                    onClick={() =>
                      void guard(
                        () => api.patchConfigScope(configScope(), { provider: selectedProvider(), model: selectedModel() }),
                        `Now using ${providerLabel(selectedProvider())} — ${selectedModel()}`,
                      )
                    }
                  >
                    Save Model Route
                  </button>
                  <button class="ghost small" disabled={loadingModels()} onClick={() => void discover(selectedProvider())}>
                    {loadingModels() ? "Checking…" : `Refresh list (${discoveredModels().length})`}
                  </button>
                </div>

                <div class="route-probe-panel">
                  <div class="route-probe-header">
                    <div>
                      <strong style={{ "font-size": "12.5px" }}>Live Connection Probe</strong>
                      <span class="dim" style={{ "font-size": "11px", "display": "block" }}>
                        Direct round-trip authentication and model availability ping.
                      </span>
                    </div>
                    <button
                      type="button"
                      class="button ghost small"
                      disabled={probing()}
                      onClick={() => void testProviderConnection()}
                    >
                      {probing() ? "Probing Route…" : "Test Provider Connection"}
                    </button>
                  </div>

                  <Show when={probeResult()}>
                    {(res) => (
                      <div class="route-probe-results">
                        <div class="route-probe-stat">
                          <span class="route-probe-label">Status</span>
                          <span class={`route-probe-val ${res().ok ? "text-good" : "text-bad"}`}>
                            {res().ok ? "Reachable (200 OK)" : "Auth/Network Error"}
                          </span>
                        </div>
                        <div class="route-probe-stat">
                          <span class="route-probe-label">Round-Trip Latency</span>
                          <span class="route-probe-val">{res().latency_ms} ms</span>
                        </div>
                        <div class="route-probe-stat">
                          <span class="route-probe-label">Catalogue Verification</span>
                          <span class="route-probe-val">
                            {res().model_count} models {res().has_active_model ? "✓ verified" : "—"}
                          </span>
                        </div>
                      </div>
                    )}
                  </Show>
                  <Show when={probeResult()?.message}>
                    <p class="dim" style={{ "font-size": "11px", "margin": "4px 0 0" }}>
                      {probeResult()!.message}
                    </p>
                  </Show>
                </div>
              </Show>
            </section>

            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Provider Key Vault</h2>
                  <p class="dim">
                    Authentication credential for {providerLabel(selectedProvider())}. Written to the selected
                    scope’s private <code>.env</code> and never shown again.
                  </p>
                </div>
                <span class={`chip chip-tone-${keyConfigured() ? "success" : "warning"}`}>
                  {keyConfigured() ? `saved (${providersData()?.providers?.find((p) => p.name === selectedProvider())?.key_source ?? "configured"})` : "not saved yet"}
                </span>
              </div>
              <div class="form-row">
                <label>API Key / Bearer Token</label>
                <input
                  type="password"
                  autocomplete="off"
                  placeholder="sk-… or Bearer token"
                  value={providerKeyInput()}
                  onInput={(e) => setProviderKeyInput(e.currentTarget.value)}
                />
              </div>
              <div class="row-gap" style="margin-top:10px">
                <button disabled={savingKey() || !providerKeyInput().trim()} onClick={() => void saveKey()}>
                  {savingKey() ? "Saving…" : "Save key"}
                </button>
                <Show when={keyConfigured()}>
                  <button
                    class="danger small"
                    onClick={() =>
                      void guard(async () => {
                        if (!confirmDestructive(`Delete the stored ${providerLabel(selectedProvider())} key?`)) return;
                        await api.deleteProviderKey(selectedProvider(), configScope());
                        await refetchProviders();
                        setDiscoveredModels([]);
                      }, `Key deleted for ${providerLabel(selectedProvider())}`)
                    }
                  >
                    Revoke
                  </button>
                </Show>
              </div>
            </section>
          </div>

          <div class="stack">
            <section class="panel" data-testid="voice-settings">
              <div class="panel-title-row">
                <div>
                  <h2>Voice & Speech Engine</h2>
                  <p class="dim">Voice conversation defaults for this scope. Bots and chats inherit these unless overridden.</p>
                </div>
                <span class="chip">{config()?.voice?.source ?? (configScope() === "user" ? "Global" : "Workspace")}</span>
              </div>
              <Show when={config()?.voice} fallback={<div class="dim">Voice configuration unavailable.</div>}>
                {(voice) => <>
                  <label class="inherit-toggle">
                    <input
                      type="checkbox"
                      checked={voice().enabled}
                      onChange={(e) => void guard(() => api.patchConfigScope(configScope(), { voice_enabled: e.currentTarget.checked }), e.currentTarget.checked ? "Voice enabled" : "Voice disabled")}
                    /> Enable voice conversations
                  </label>
                  <div class="voice-grid-2x2">
                    <div class="form-row">
                      <label for="voice-provider">Provider</label>
                      <input id="voice-provider" value={voice().provider ?? ""} placeholder="Configured default" onChange={(e) => void guard(() => api.patchConfigScope(configScope(), { voice_provider: e.currentTarget.value.trim() || null }), "Voice provider saved")} />
                    </div>
                    <div class="form-row">
                      <label for="voice-transcription-model">Transcription Model</label>
                      <input id="voice-transcription-model" value={voice().transcription_model ?? ""} placeholder="Provider default" onChange={(e) => void guard(() => api.patchConfigScope(configScope(), { voice_transcription_model: e.currentTarget.value.trim() || null }), "Transcription model saved")} />
                    </div>
                    <div class="form-row">
                      <label for="voice-synthesis-model">Synthesis Model</label>
                      <input id="voice-synthesis-model" value={voice().synthesis_model ?? ""} placeholder="Provider default" onChange={(e) => void guard(() => api.patchConfigScope(configScope(), { voice_synthesis_model: e.currentTarget.value.trim() || null }), "Synthesis model saved")} />
                    </div>
                    <div class="form-row">
                      <label for="voice-realtime-model">Realtime Streaming Model</label>
                      <input id="voice-realtime-model" value={voice().realtime_model ?? ""} placeholder="Provider default" onChange={(e) => void guard(() => api.patchConfigScope(configScope(), { voice_realtime_model: e.currentTarget.value.trim() || null }), "Realtime model saved")} />
                    </div>
                  </div>
                </>}
              </Show>
            </section>

            <div class="settings-info-box">
              <h3>✦ Multi-Provider Ladder Contract</h3>
              <p>
                In accordance with <code>AGENTS.md</code> Invariant 7, the route ladder is planned fresh on every turn
                from the live evidence ledger, session belief state, warm discovery cache, and your effective route.
                Failed requests fall back across candidate models automatically without rewriting your primary route.
              </p>
            </div>
          </div>
        </div>
      </Show>

      <Show when={activeTab() === "permissions"}>
        <div class="two-col">
          <div class="stack">
            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Execution Authority</h2>
                  <p class="dim">Permission mode governing file and tool execution boundaries.</p>
                </div>
              </div>
              <div class="mode-grid">
                <For each={MODES}>
                  {(m) => (
                    <button
                      class="mode-btn"
                      classList={{ active: selectedPermissionMode() === m.value }}
                      onClick={() => void guard(() => api.patchConfigScope(configScope(), { permission_mode: m.value === "ReadOnly" ? "read-only" : m.value === "WorkspaceWrite" ? "workspace-write" : "full-access" }), `Now set to \u201C${m.label}\u201D`)}
                      disabled={selectedPermissionMode() === m.value}
                    >
                      <span class="mode-name">
                        {m.label}
                        <Show when={selectedPermissionMode() === m.value}>
                          <span class="chip chip-tone-success">
                            {inheritedPermissionMode() ? "in force (inherited)" : "set here"}
                          </span>
                        </Show>
                      </span>
                      <span class="mode-desc">{MODE_COPY[m.value]}</span>
                    </button>
                  )}
                </For>
              </div>

              <div class="panel-title-row" style={{ "margin-top": "20px" }}>
                <div>
                  <h3>How should approvals be handled?</h3>
                  <p class="dim">Controls how Ask decisions are resolved without altering the OS sandbox.</p>
                </div>
              </div>
              <div class="mode-grid">
                <For each={APPROVAL_MODES}>
                  {(m) => (
                    <button
                      class="mode-btn"
                      classList={{ active: selectedApprovalMode() === m.value }}
                      onClick={() => void guard(() => api.patchConfigScope(configScope(), { approval_mode: m.value }), `Approval mode set to “${m.label}”`)}
                      disabled={selectedApprovalMode() === m.value}
                    >
                      <span class="mode-name">
                        {m.label}
                        <Show when={selectedApprovalMode() === m.value}>
                          <span class="chip chip-tone-success">
                            {inheritedApprovalMode() ? "in force (inherited)" : "set here"}
                          </span>
                        </Show>
                      </span>
                      <span class="mode-desc">{m.desc}</span>
                    </button>
                  )}
                </For>
              </div>
              <p class="dim" style={{ "margin-top": "12px" }}>
                Effective sandbox: <span class="chip chip-tone-info" style={{ "letter-spacing": "normal" }}>{config()?.sandbox ?? "none"}</span>
              </p>
            </section>
          </div>

          <div class="stack">
            <ApprovalForwarding />

            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Security Rulebook</h2>
                  <p class="dim">Specific pattern exceptions and pre-approved tools.</p>
                </div>
              </div>
              <Show
                when={rulesReported(config())}
                fallback={
                  <Show when={!config.loading}>
                    <div class="rule-lists">
                      <p class="dim">
                        Specific rules not reported by this server build. Rules in <code>config.toml</code> remain active.
                      </p>
                    </div>
                  </Show>
                }
              >
                <div class="rule-lists">
                  <For each={RULE_SECTIONS}>
                    {({ decision, title, empty }) => (
                      <div>
                        <span class="eyebrow">{title}</span>
                        <Show
                          when={rulesFor(decision).length > 0}
                          fallback={<p class="dim">{empty}</p>}
                        >
                          <div class="chip-stack">
                            <For each={rulesFor(decision)}>{(r) => <RuleChip rule={r} />}</For>
                          </div>
                        </Show>
                      </div>
                    )}
                  </For>
                </div>
                <p class="dim" style="margin-top:10px">
                  Hover a rule to see its pattern. To inspect apps, open <a href="#/integrations">Extensions</a>.
                </p>
                <RuleEditor scope={configScope()} onSaved={() => void refetchConfig()} />
              </Show>
            </section>
          </div>
        </div>
      </Show>

      <Show when={activeTab() === "infrastructure"}>
        <div class="two-col">
          <div class="stack">
            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Event Bus Fabric</h2>
                  <p class="dim">
                    Distributed event fabric (<code>crates/vak-bus</code>, docs/design/53). NATS Core + JetStream
                    enables multi-server synchronization; leave blank for in-process memory backend.
                  </p>
                </div>
                <Show when={busData()}>
                  <span class={`chip chip-tone-${busData()?.runtime?.connected ? "success" : "warning"}`}>
                    {busData()?.runtime?.backend === "nats" ? "connected (nats)" : busData()?.runtime?.backend === "memory" ? "in-process (memory)" : "not connected"}
                  </span>
                </Show>
              </div>
              <Show when={busData()} fallback={
                <div class="error-state" role="alert">
                  <strong>Event bus configuration unavailable.</strong>
                  <p>{busData.error ? String(busData.error) : "No configuration response was returned."}</p>
                  <button class="ghost small" type="button" onClick={() => void refetchBus()}>Retry bus settings</button>
                </div>
              }>
                <div class="form-row">
                  <label>NATS URL</label>
                  <input
                    type="url"
                    placeholder="nats://localhost:4222"
                    value={busData()?.nats_url ?? ""}
                    onInput={(e) => setBusUrl(e.currentTarget.value)}
                  />
                </div>
                <div class="form-row">
                  <label>Credentials JWT (optional)</label>
                  <input
                    type="password"
                    autocomplete="off"
                    placeholder="ey..."
                    value={busJwt()}
                    onInput={(e) => setBusJwt(e.currentTarget.value)}
                  />
                </div>
                <div class="form-row">
                  <label>NKey Seed (optional)</label>
                  <input
                    type="password"
                    autocomplete="off"
                    placeholder="SU..."
                    value={busNkey()}
                    onInput={(e) => setBusNkey(e.currentTarget.value)}
                  />
                </div>
                <div class="form-row">
                  <label>Workspace Secret Env Var Name</label>
                  <input
                    type="text"
                    class="mono"
                    placeholder="VAK_ENCRYPTION_SECRET"
                    value={busSecretEnv()}
                    onInput={(e) => setBusSecretEnv(e.currentTarget.value)}
                  />
                </div>
                <Show when={busData()?.encrypted}>
                  <p class="dim" style="margin-top:8px">
                    <span class="chip chip-tone-success">Envelope Encrypted</span>{" "}
                    AES-256-GCM envelope security is active for this workspace.
                  </p>
                </Show>
                <div class="row-gap" style="margin-top:12px">
                  <button disabled={savingBus()} onClick={() => void saveBus()}>
                    {savingBus() ? "Saving…" : "Save bus config"}
                  </button>
                  <Show when={busData()?.encrypted || (busData()?.nats_url ?? "").trim() !== ""}>
                    <button class="danger small" onClick={() => void clearBus()}>
                      Remove credentials
                    </button>
                  </Show>
                </div>
                <p class="dim" style="margin-top:8px">
                  Credentials written to <code>.vak/env</code>. Takes effect on next server restart.
                </p>
              </Show>
            </section>
          </div>

          <div class="stack">
            <Show when={configScope() === "project"}>
              <section class="panel">
                <div class="panel-title-row">
                  <div>
                    <h2>Inherited Capabilities</h2>
                    <p class="dim">Workspace inherits Global capabilities. Toggle off to scope to this workspace only.</p>
                  </div>
                </div>
                <div class="capability-inheritance-list">
                  <For each={[
                    ["inherit_mcp", "Connected apps (MCP)"],
                    ["inherit_hooks", "Automations (hooks)"],
                    ["inherit_skills", "Skills"],
                    ["inherit_plugins", "Plugins"],
                  ] as const}>
                    {([key, label]) => {
                      const inherited = () => layer()?.capabilities?.[key] !== false;
                      return (
                        <label class="inherit-toggle capability-inheritance-row">
                          <input
                            type="checkbox"
                            checked={inherited()}
                            onChange={(event) => void guard(
                              () => api.patchConfigScope("project", { [key]: event.currentTarget.checked }),
                              event.currentTarget.checked ? `${label} now inherit Global` : `${label} inheritance disabled`,
                            )}
                          />
                          <span><strong>{label}</strong><small>{inherited() ? "Inherited from Global" : "Workspace-only"}</small></span>
                        </label>
                      );
                    }}
                  </For>
                </div>
              </section>
            </Show>

            <div class="settings-info-box">
              <h3>Distributed Fabric Topology</h3>
              <p>
                The distributed bus handles multi-tenant events with Merkle/W3C causal lineage and dead-letter queues.
                Local nodes fall back to high-throughput shared-memory channels when NATS is not configured.
              </p>
            </div>
          </div>
        </div>
      </Show>

      <Show when={activeTab() === "preferences"}>
        <div class="two-col">
          <div class="stack">
            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Appearance</h2>
                  <p class="dim">Theme preferences for this browser session.</p>
                </div>
              </div>
              <div class="theme-grid">
                <For
                  each={[
                    { id: "warm", label: "Warm dark", class: "" },
                    { id: "dark", label: "Midnight", class: "dark" },
                    { id: "contrast", label: "High contrast", class: "contrast" },
                  ] as const}
                >
                  {(t) => (
                    <button class="theme-choice" classList={{ active: theme() === t.id }} onClick={() => setTheme(t.id)}>
                      <span class={`theme-preview ${t.class}`}><i /><i /><i /></span>
                      <strong>{t.label}</strong>
                    </button>
                  )}
                </For>
              </div>
            </section>

            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Execution Limits</h2>
                  <p class="dim">Run ceiling and delegation boundaries.</p>
                </div>
              </div>
              <Show when={!config.loading} fallback={<div class="cred-list"><span class="skel skel-block" /></div>}>
                <div class="form-row">
                  <label>Max turns per run</label>
                  <input
                    class="mono"
                    type="number"
                    min="1"
                    max="1000"
                    value={maxTurnsInput()}
                    onInput={(e) => setMaxTurnsInput(e.currentTarget.value)}
                  />
                </div>
                <div class="row-gap" style="margin-top:10px; margin-bottom:14px">
                  <button
                    disabled={
                      savingMaxTurns() ||
                      !maxTurnsInput().trim() ||
                      !(Number(maxTurnsInput()) >= 1 && Number(maxTurnsInput()) <= 1000)
                    }
                    onClick={() => {
                      setSavingMaxTurns(true);
                      void guard(
                        () => api.patchConfigScope(configScope(), { max_turns: Number(maxTurnsInput()) }),
                        `Max turns set to ${maxTurnsInput()}`,
                      ).finally(() => setSavingMaxTurns(false));
                    }}
                  >
                    {savingMaxTurns() ? "Saving…" : "Save Limit"}
                  </button>
                </div>
                <label class="inherit-toggle">
                  <input
                    type="checkbox"
                    checked={config()?.subagents ?? false}
                    disabled={togglingSubagents()}
                    onChange={(e) => {
                      const next = e.currentTarget.checked;
                      setTogglingSubagents(true);
                      void guard(
                        () => api.patchConfigScope(configScope(), { subagents: next }),
                        next ? "Sub-agents enabled" : "Sub-agents disabled",
                      ).finally(() => setTogglingSubagents(false));
                    }}
                  />
                  Sub-agents — allow delegating tasks to child workers
                </label>
              </Show>
            </section>
          </div>

          <div class="stack">
            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Housekeeping & Diagnostics</h2>
                  <p class="dim">Routine system integrity checks and search re-indexing.</p>
                </div>
              </div>
              <div class="row-gap">
                <button class="ghost" disabled={runningDoctor()} onClick={() => void runDoctor()}>
                  {runningDoctor() ? "Checking…" : "Run Doctor Check"}
                </button>
                <button class="ghost" disabled={rebuilding()} onClick={() => void rebuild()}>
                  {rebuilding() ? "Rebuilding…" : "Rebuild Search Index"}
                </button>
              </div>
              <Show when={doctorReport()}>
                <pre class="mono report-pre">{doctorReport()}</pre>
                <div class="row-gap" style="margin-top:8px">
                  <button class="ghost small" onClick={() => setDoctorReport(null)}>Dismiss report</button>
                </div>
              </Show>
            </section>

            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Session & Authentication</h2>
                  <p class="dim">
                    Signing out ends this browser's session. Background daemons continue uninterrupted.
                  </p>
                </div>
              </div>
              <div class="row-gap">
                <button class="danger" onClick={() => void signOut()}>Sign out</button>
              </div>
            </section>
          </div>
        </div>
      </Show>
    </div>
  );
}

// ---- Shell -----------------------------------------------------------------

interface NavItem {
  hash: string;
  label: string;
  icon: string;
  group: "Overview" | "Work" | "Operate" | "Configure" | "System";
  scope: "global" | "project" | "switchable" | "layered";
  badge?: () => string;
  /// Sub-destinations, rendered in the sidebar while the section is open.
  children?: readonly { readonly hash: string; readonly label: string }[];
  /// Which child is current. A section with children owns this, because only
  /// it knows how its sub-routes resolve (a bare section hash is its own
  /// first tab, not "no tab").
  activeChild?: () => string;
}

const OPERATIONS_TABS = [
  { hash: "#/operations", label: "Posture" },
  { hash: "#/operations/work", label: "Live work" },
  { hash: "#/operations/runtime", label: "Runtime & pools" },
  { hash: "#/operations/channels", label: "Channels & delivery" },
  { hash: "#/operations/automations", label: "Automations" },
  { hash: "#/operations/providers", label: "Providers" },
  { hash: "#/operations/incidents", label: "Incidents" },
  { hash: "#/operations/sandbox", label: "Sandbox" },
] as const;

const operationsTab = () => {
  const current = route().split("?", 1)[0] || "#/overview";
  const exact = OPERATIONS_TABS.find((tab) => current === tab.hash);
  if (exact) return exact.hash;
  return OPERATIONS_TABS.slice(1).find((tab) => current.startsWith(`${tab.hash}/`))?.hash ?? "#/operations";
};

const SETTINGS_TABS = [
  { hash: "#/settings", label: "Model & Keys" },
  { hash: "#/settings/permissions", label: "Access & Security" },
  { hash: "#/settings/infrastructure", label: "Infrastructure" },
  { hash: "#/settings/preferences", label: "Preferences" },
] as const;

const settingsTab = () => {
  const current = route().split("?", 1)[0] || "#/settings";
  const exact = SETTINGS_TABS.find((tab) => current === tab.hash);
  if (exact) return exact.hash;
  return SETTINGS_TABS.slice(1).find((tab) => current.startsWith(`${tab.hash}/`))?.hash ?? "#/settings";
};

function navHref(hash: string): string {
  if (!hash.startsWith("#/operations")) return hash;
  const queryIndex = route().indexOf("?");
  return queryIndex >= 0 ? `${hash}${route().slice(queryIndex)}` : hash;
}

const NAV: NavItem[] = [
  { group: "Overview", hash: "#/overview", label: "Home", icon: ICONS.overview, scope: "global" },
  { group: "Overview", hash: "#/inbox", label: "Inbox", icon: ICONS.inbox, scope: "global", badge: () => unread().toString() || "" },
  { group: "Work", hash: "#/sessions", label: "Sessions", icon: ICONS.sessions, scope: "global" },
  { group: "Work", hash: "#/commitments", label: "Commitments", icon: ICONS.commitments, scope: "project" },
  {
    group: "Operate",
    hash: "#/operations",
    label: "Operations",
    icon: ICONS.operations,
    scope: "switchable",
    children: OPERATIONS_TABS,
    activeChild: operationsTab,
  },
  // Extensions are four distinct governance questions — what external
  // processes can be started, what instructions are loaded, what intercepts
  // a run, what runs unattended — and they read as four screens for the
  // same reason the Gateway does.
  {
    group: "Configure",
    hash: "#/integrations",
    label: "Extensions",
    icon: ICONS.integrations,
    scope: "layered",
    children: EXTENSION_TABS,
    activeChild: extensionsTab,
  },
  // The gateway is four distinct jobs, not one page: watch the channels
  // you have, walk a new one in, handle credentials, check routing and
  // pool health. The sub-rows expand in place when the section is open so
  // the destination is nameable from the nav rather than found by
  // scrolling one long view.
  {
    group: "Configure",
    hash: "#/gateway",
    label: "Channels",
    icon: ICONS.gateway,
    scope: "global",
    children: GATEWAY_TABS,
    activeChild: gatewayTab,
  },
  { group: "Configure", hash: "#/security", label: "Permissions & security", icon: ICONS.security, scope: "global" },
  { group: "Configure", hash: "#/prompts", label: "Prompts", icon: ICONS.prompts, scope: "layered" },
  {
    group: "Configure",
    hash: "#/memory",
    label: "Knowledge",
    icon: ICONS.memory,
    scope: "project",
    children: KNOWLEDGE_TABS,
    activeChild: knowledgeTab,
  },
  {
    group: "Configure",
    hash: "#/settings",
    label: "Model & providers",
    icon: ICONS.settings,
    scope: "layered",
    children: SETTINGS_TABS,
    activeChild: settingsTab,
  },
  { group: "System", hash: "#/setup", label: "Setup", icon: ICONS.setup, scope: "project" },
  { group: "System", hash: "#/finops", label: "FinOps", icon: ICONS.finops, scope: "project" },
];

function routeScope(current: string): NavItem["scope"] {
  if (current === "#/feeds" || current.startsWith("#/feeds/") || current === "#/search" || current.startsWith("#/search/")) {
    return "project";
  }
  if (current === "#/sessions" || current.startsWith("#/sessions/")) return "global";
  return NAV.find((item) => current === item.hash || current.startsWith(`${item.hash}/`))?.scope ?? "global";
}

/// Source types arrive as the registry's own ids. Print the name people use.
const FEED_TYPE_LABELS: Record<string, string> = {
  rss: "RSS / Atom",
  youtube: "YouTube",
  hacker_news: "Hacker News",
  reddit: "Reddit",
  custom_http: "Custom",
};

function FeedsSection() {
  const [tab, setTab] = createSignal<"overview" | "sources" | "reader" | "alerts" | "workbench">("overview");
  const [searchQuery, setSearchQuery] = createSignal("");
  const [searchResults, setSearchResults] = createSignal<import("./types").FeedSearchResponse | null>(null);
  const [searching, setSearching] = createSignal(false);
  const [wizardOpen, setWizardOpen] = createSignal(false);
  const [alertFormOpen, setAlertFormOpen] = createSignal(false);
  const [editingSource, setEditingSource] = createSignal<string | null>(null);
  const [editInterval, setEditInterval] = createSignal("");
  const [editTrust, setEditTrust] = createSignal("");

  // Errors are surfaced (via LoadError), not swallowed to an empty/null
  // value — a script/MCP failure here used to render as an indefinite
  // "Loading…" with no way to tell a slow fetch from a broken backend.
  const [stats, { refetch: refetchStats }] = createResource(() => api.feedStats());
  const [sourceTypes] = createResource(() => api.feedSourceTypes());
  const [alerts, { refetch: refetchAlerts }] = createResource(() => api.feedAlerts());
  const [configuredSources, { refetch: refetchConfigured }] = createResource(() =>
    api.feedConfiguredSources(),
  );
  const [runs, { refetch: refetchRuns }] = createResource(() => api.feedRuns());
  const [quarantine, { refetch: refetchQuarantine }] = createResource(() => api.feedQuarantine());

  const refetchAll = async () => {
    await Promise.all([refetchStats(), refetchConfigured(), refetchRuns(), refetchQuarantine()]);
  };

  const doSearch = async () => {
    const q = searchQuery().trim();
    if (!q) return;
    setSearching(true);
    try {
      const res = await api.feedSearch({ q, limit: 10 });
      setSearchResults(res);
    } catch (e) {
      pushToast("alert", `Search failed: ${e}`);
    }
    setSearching(false);
  };

  const toggleSource = async (src: import("./types").ConfiguredFeedSource) => {
    try {
      await api.feedUpdateSource(src.id, { enabled: !src.enabled }, src.scope);
      await refetchConfigured();
      pushToast("info", `${src.name} ${src.enabled ? "disabled" : "enabled"}`);
    } catch (e) {
      pushToast("alert", `Could not update "${src.name}": ${e}`);
    }
  };

  const removeSource = async (source: import("./types").ConfiguredFeedSource) => {
    const name = source.name;
    if (!confirm(`Remove source "${name}"? This stops it from being checked, but keeps items already collected.`)) return;
    try {
      await api.feedDeleteSource(source.id, source.scope);
      await refetchAll();
      pushToast("info", `Source "${name}" removed`);
    } catch (e) {
      pushToast("alert", `Could not remove "${name}": ${e}`);
    }
  };

  const startEditSource = (src: import("./types").ConfiguredFeedSource) => {
    setEditingSource(src.id);
    setEditInterval(src.check_interval || "1h");
    setEditTrust(src.trust || "medium");
  };

  const saveEditSource = async (source: import("./types").ConfiguredFeedSource) => {
    const name = source.name;
    try {
      await api.feedUpdateSource(source.id, { interval: editInterval(), trust: editTrust() }, source.scope);
      await refetchConfigured();
      pushToast("info", `"${name}" updated`);
      setEditingSource(null);
    } catch (e) {
      pushToast("alert", `Could not update "${name}": ${e}`);
    }
  };

  const removeAlert = async (alert: import("./types").FeedAlertRule) => {
    const name = alert.name;
    if (!confirm(`Delete alert "${name}"?`)) return;
    try {
      await api.feedDeleteAlert(name, alert.scope);
      await refetchAlerts();
      pushToast("info", `Alert "${name}" deleted`);
    } catch (e) {
      pushToast("alert", `Could not delete alert "${name}": ${e}`);
    }
  };

  const triggerIngest = async () => {
    try {
      const res = await api.feedIngest();
      await refetchAll();
      pushToast("info", `Found ${res.new_items} new item${res.new_items === 1 ? "" : "s"} across ${res.sources_ingested} source${res.sources_ingested === 1 ? "" : "s"}`);
    } catch (e) {
      pushToast("alert", `Could not check for new items: ${e}`);
    }
  };

  return (
    <div class="view">
      <KnowledgeSubNav active="#/feeds" />
      <PageHeader
        title="Feeds"
        description="Sources vak reads on a schedule — blogs, YouTube channels, Reddit, Hacker News — so it can answer from them."
        actions={
          <button class="small" onClick={triggerIngest}>Check for new items</button>
        }
      />
      <div class="tab-bar">
        <button class="tab-btn" classList={{ active: tab() === "overview" }} onClick={() => setTab("overview")}>Overview</button>
        <button class="tab-btn" classList={{ active: tab() === "sources" }} onClick={() => setTab("sources")}>Sources</button>
        <button class="tab-btn" classList={{ active: tab() === "reader" }} onClick={() => setTab("reader")}>Reader</button>
        <button class="tab-btn" classList={{ active: tab() === "alerts" }} onClick={() => setTab("alerts")}>Alerts</button>
        <button class="tab-btn" classList={{ active: tab() === "workbench" }} onClick={() => setTab("workbench")}>Workbench</button>
      </div>

      <Show when={tab() === "overview"}>
        <Show when={!stats.error} fallback={<LoadError message={`${stats.error}`} onRetry={() => refetchStats()} />}>
        <Show when={stats()} fallback={<div class="empty">Loading…</div>}>
          <div class="feed-stats-row">
            <div class="feed-stat">
              <div class="value">{stats()!.total_items}</div>
              <div class="label">Items collected</div>
            </div>
            <div class="feed-stat">
              <div class="value">{stats()!.total_feeds}</div>
              <div class="label">Sources</div>
            </div>
            <div class="feed-stat">
              <div class="value">{stats()!.items_today}</div>
              <div class="label">New today</div>
            </div>
            <div class="feed-stat">
              <div class="value">{stats()!.total_alerts}</div>
              <div class="label">Alerts set up</div>
            </div>
            <div class="feed-stat">
              <div class="value">{stats()!.quarantined_items ?? 0}</div>
              <div class="label">Quarantined</div>
            </div>
          </div>
          <Show when={stats()!.sources && stats()!.sources!.length > 0}>
            <section class="panel">
              <div class="panel-title-row"><div><h2>Sources</h2></div></div>
              <For each={stats()!.sources}>
                {(src) => (
                  <div class="feed-source-card">
                    <div class="icon">{src.type === "hacker_news" ? "🔥" : src.type === "youtube" ? "▶" : "📡"}</div>
                    <div class="info">
                      <div class="name">{src.name}</div>
                      <div class="meta">{FEED_TYPE_LABELS[src.type] ?? src.type} · {src.item_count} items</div>
                    </div>
                  </div>
                )}
              </For>
            </section>
          </Show>
        </Show>
        </Show>
        <section class="panel">
          <div class="panel-title-row"><div><h2>Search</h2></div></div>
          <div class="toolbar">
            <input
              class="search-input"
              placeholder="Search everything you follow…"
              value={searchQuery()}
              onInput={(e) => setSearchQuery(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && doSearch()}
            />
            <button onClick={doSearch} disabled={searching()}>{searching() ? "Searching…" : "Search"}</button>
          </div>
          <Show when={searchResults()}>
            <Show when={searchResults()!.answer}>
              <div class="feed-answer-box">{searchResults()!.answer}</div>
            </Show>
            <Show when={searchResults()!.follow_up_questions && searchResults()!.follow_up_questions!.length > 0}>
              <div class="feed-follow-ups">
                <For each={searchResults()!.follow_up_questions}>
                  {(q) => (
                    <span class="feed-follow-up" onClick={() => { setSearchQuery(q); doSearch(); }}>{q}</span>
                  )}
                </For>
              </div>
            </Show>
            <For each={searchResults()!.results}>
              {(item) => (
                <div class="feed-search-result">
                  <div class="score-bar"><div class="fill" style={{ width: `${Math.round(item.score * 100)}%` }} /></div>
                  <div class="title"><a href={item.url} target="_blank">{item.title}</a></div>
                  <Show when={item.content}>
                    <div class="excerpt">{item.content.slice(0, 200)}</div>
                  </Show>
                  <div class="evidence">
                    <Show when={item.source_name}><span>{item.source_name}</span></Show>
                    <Show when={item.published_date}><span>{item.published_date}</span></Show>
                    <Show when={item.evidence?.freshness_hours != null}><span>{item.evidence!.freshness_hours}h ago</span></Show>
                  </div>
                </div>
              )}
            </For>
          </Show>
        </section>
        <section class="panel">
          <div class="panel-title-row"><div><h2>Quarantine</h2><p class="dim">External content held from search and alerts until reviewed.</p></div></div>
          <Show when={quarantine()} fallback={<div class="empty">No quarantined items.</div>}>
            <For each={quarantine()!.items}>
              {(item) => <div class="feed-source-card"><div class="info"><div class="name">{item.title}</div><div class="meta">{item.source_name} · {item.security_detail || "security review required"}</div></div><button type="button" class="small" onClick={async () => { try { await api.feedReleaseItem(item.id); await refetchQuarantine(); pushToast("info", "Item released"); } catch (e) { pushToast("alert", `Could not release item: ${e}`); } }}>Release</button></div>}
            </For>
          </Show>
        </section>
      </Show>

      <Show when={tab() === "sources"}>
        <section class="panel">
          <div class="panel-title-row">
            <div><h2>Your sources</h2><p class="dim">What vak is currently checking, grouped by tag.</p></div>
            <button class="small" onClick={() => setWizardOpen(true)}>Add a source</button>
          </div>
          <Show when={!configuredSources.error} fallback={<LoadError message={`${configuredSources.error}`} onRetry={() => refetchConfigured()} />}>
          <Show
            when={configuredSources() && configuredSources()!.sources.length > 0}
            fallback={<div class="empty">No sources yet. Add one below to get started.</div>}
          >
            <For each={configuredSources()!.sources}>
              {(src) => (
                <div class="feed-source-card">
                  <div class="icon">{src.source_type === "hacker_news" ? "🔥" : src.source_type === "youtube" ? "▶" : "📡"}</div>
                  <div class="info">
                    <div class="name">{src.name}</div>
                    <Show
                        when={editingSource() === src.id}
                      fallback={
                        <>
                          <div class="meta">
                            {FEED_TYPE_LABELS[src.source_type] ?? src.source_type}
                            <Show when={src.check_interval}> · every {src.check_interval}</Show>
                            <Show when={src.trust}> · {src.trust} trust</Show>
                            <span> · {src.scope === "workspace" ? "workspace" : "global"}</span>
                            <Show when={src.url}> · <a href={src.url} target="_blank" rel="noreferrer noopener">{src.url}</a></Show>
                          </div>
                          <Show when={src.next_due_at}>
                            <div class="meta">Next check: {src.next_due_at!.slice(0, 19)}</div>
                          </Show>
                          <Show when={src.last_status && src.last_status !== "never_run"}>
                            <div class="meta">Last check: {src.last_status}{src.last_error ? ` · ${src.last_error}` : ""}</div>
                          </Show>
                        </>
                      }
                    >
                      <div class="toolbar" style={{ "margin-top": "6px" }}>
                        <select value={editInterval()} onChange={(e) => setEditInterval(e.currentTarget.value)}>
                          <option value="5m">Every 5 minutes</option>
                          <option value="15m">Every 15 minutes</option>
                          <option value="30m">Every 30 minutes</option>
                          <option value="1h">Every hour</option>
                          <option value="6h">Every 6 hours</option>
                          <option value="1d">Daily</option>
                        </select>
                        <select value={editTrust()} onChange={(e) => setEditTrust(e.currentTarget.value)}>
                          <option value="high">High trust</option>
                          <option value="medium">Medium trust</option>
                          <option value="low">Low trust</option>
                        </select>
                        <button class="small" onClick={() => saveEditSource(src)}>Save</button>
                        <button class="ghost small" onClick={() => setEditingSource(null)}>Cancel</button>
                      </div>
                    </Show>
                  </div>
                  <div class="actions" style={{ display: "flex", gap: "6px", "align-items": "center" }}>
                    <span class={`chip ${src.enabled ? "chip-tone-success" : "chip-tone-danger"}`}>
                      {src.enabled ? "on" : "off"}
                    </span>
                    <button class="ghost small" onClick={() => startEditSource(src)}>Edit</button>
                    <button class="ghost small" onClick={() => toggleSource(src)}>{src.enabled ? "Disable" : "Enable"}</button>
                    <button class="ghost small" onClick={() => removeSource(src)}>Remove</button>
                  </div>
                </div>
              )}
            </For>
          </Show>
          </Show>
        </section>

        <section class="panel">
          <div class="panel-title-row">
            <div><h2>What you can follow</h2><p class="dim">Pick any of these when adding a source.</p></div>
          </div>
          <Show when={sourceTypes()}>
            <For each={sourceTypes()!.source_types}>
              {(st) => (
                <div class="feed-source-card">
                  <div class="icon">{st.icon === "rss" ? "📡" : st.icon === "youtube" ? "▶" : st.icon === "fire" ? "🔥" : st.icon === "reddit" ? "📱" : "🌐"}</div>
                  <div class="info">
                    <div class="name">{st.name}</div>
                    <div class="meta">{st.description} · checked every {st.default_interval} by default</div>
                  </div>
                </div>
              )}
            </For>
          </Show>
        </section>
      </Show>

      <Show when={tab() === "reader"}>
        <section class="panel">
          <div class="panel-title-row"><div><h2>Reader</h2><p class="dim">Everything collected so far, newest first.</p></div></div>
          <FeedReader />
        </section>
      </Show>

      <Show when={tab() === "alerts"}>
        <section class="panel">
          <div class="panel-title-row">
            <div><h2>Alerts</h2><p class="dim">Tell vak to flag an item when it mentions something you care about.</p></div>
            <button class="small" onClick={() => setAlertFormOpen(true)}>New alert</button>
          </div>
          <Show when={alertFormOpen()}>
            <AlertForm
              onClose={() => setAlertFormOpen(false)}
              onAdded={async () => { await refetchAlerts(); setAlertFormOpen(false); }}
            />
          </Show>
          <Show when={!alerts.error} fallback={<LoadError message={`${alerts.error}`} onRetry={() => refetchAlerts()} />}>
          <Show when={alerts.loading}>
            <div class="empty">Loading…</div>
          </Show>
          <Show when={!alerts.loading && (alerts()?.alerts.length ?? 0) === 0}>
            <div class="empty">No alerts set up yet.</div>
          </Show>
          <For each={alerts()?.alerts ?? []}>
            {(alert) => (
              <div class="feed-alert-card">
                <div class="info">
                  <div class="name">{alert.name}</div>
                  <div class="match">
                    <Show when={alert.match_config?.keywords?.length}>mentions {alert.match_config!.keywords!.join(", ")}</Show>
                    <Show when={alert.match_config?.tags?.length}> · tagged {alert.match_config!.tags!.join(", ")}</Show>
                    <Show when={alert.match_config?.sources?.length}> · from {alert.match_config!.sources!.join(", ")}</Show>
                  </div>
                </div>
                <div style={{ display: "flex", gap: "6px", "align-items": "center" }}>
                  <span class={`chip ${alert.enabled ? "chip-tone-success" : "chip-tone-danger"}`}>
                    {alert.enabled ? "on" : "off"}
                  </span>
                  <button class="ghost small" onClick={() => removeAlert(alert)}>Delete</button>
                </div>
              </div>
            )}
          </For>
          </Show>
        </section>
      </Show>

      <Show when={tab() === "workbench"}>
        <section class="panel">
          <div class="panel-title-row"><div><h2>Workbench</h2><p class="dim">Run a search and see the raw result vak works from — useful for checking why something did or didn’t come back.</p></div></div>
          <div class="form-row">
            <label>Try a search</label>
            <div class="toolbar">
              <input
                class="search-big"
                placeholder="Type a search to try…"
                value={searchQuery()}
                onInput={(e) => setSearchQuery(e.currentTarget.value)}
                onKeyDown={(e) => e.key === "Enter" && doSearch()}
              />
              <button onClick={doSearch} disabled={searching()}>Test</button>
            </div>
          </div>
          <Show when={searchResults()}>
            <pre style={{ "font-size": "11px", "max-height": "400px", overflow: "auto", "white-space": "pre-wrap" }}>
              {JSON.stringify(searchResults(), null, 2)}
            </pre>
          </Show>
        </section>
        <section class="panel">
          <div class="panel-title-row"><div><h2>Recent runs</h2><p class="dim">Durable ingestion receipts for this workspace.</p></div></div>
          <Show when={runs.error} fallback={<Show when={runs()} fallback={<div class="empty">Loading…</div>}>
            <For each={runs()!.runs}>
              {(run) => <div class="feed-source-card"><div class="info"><div class="name">{run.status} · {run.started_at.slice(0, 19)}</div><div class="meta">{run.scope} · {run.sources_succeeded}/{run.sources_seen} sources · {run.items_added} new items</div><Show when={run.error}><div class="meta">{run.error}</div></Show></div></div>}
            </For>
          </Show>}>
            <div class="empty">Could not load ingestion receipts.</div>
          </Show>
        </section>
      </Show>

      <Show when={wizardOpen()}>
        <FeedWizard onClose={() => setWizardOpen(false)} onAdded={async () => { await refetchAll(); setWizardOpen(false); }} />
      </Show>
    </div>
  );
}

function AlertForm(props: { onClose: () => void; onAdded: () => void | Promise<void> }) {
  const [name, setName] = createSignal("");
  const [keywords, setKeywords] = createSignal("");
  const [tags, setTags] = createSignal("");
  const [deliverTo, setDeliverTo] = createSignal("");
  const [scope, setScope] = createSignal("workspace");
  const [saving, setSaving] = createSignal(false);

  const canSubmit = () => name().trim() && (keywords().trim() || tags().trim());

  const submit = async () => {
    setSaving(true);
    try {
      await api.feedAddAlert({
        name: name().trim(),
        scope: scope(),
        keywords: keywords().trim() ? keywords().split(",").map((k) => k.trim()).filter(Boolean) : [],
        tags: tags().trim() ? tags().split(",").map((t) => t.trim()).filter(Boolean) : [],
        deliver_to: deliverTo().trim() || undefined,
      });
      pushToast("info", `Alert "${name()}" created`);
      await props.onAdded();
    } catch (e) {
      pushToast("alert", `Could not create alert: ${e}`);
    }
    setSaving(false);
  };

  return (
    <div class="panel" style={{ "margin-bottom": "12px", background: "var(--panel-alt, rgba(255,255,255,0.03))" }}>
      <div class="form-row">
        <label>Name</label>
        <input value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="Funding rounds" />
      </div>
      <div class="form-row">
        <label>Keywords</label>
        <input value={keywords()} onInput={(e) => setKeywords(e.currentTarget.value)} placeholder="series a, seed round — comma separated" />
      </div>
      <div class="form-row">
        <label>Tags</label>
        <input value={tags()} onInput={(e) => setTags(e.currentTarget.value)} placeholder="funding, startups — comma separated" />
      </div>
      <div class="form-row">
        <label>Deliver to</label>
        <input value={deliverTo()} onInput={(e) => setDeliverTo(e.currentTarget.value)} placeholder="Chat key, webhook, or leave blank" />
      </div>
      <div class="form-row">
        <label>Availability</label>
        <select value={scope()} onChange={(e) => setScope(e.currentTarget.value)}>
          <option value="workspace">Workspace</option>
          <option value="global">Global</option>
        </select>
      </div>
      <div style={{ display: "flex", gap: "8px", "margin-top": "8px" }}>
        <button type="button" class="ghost" onClick={props.onClose}>Cancel</button>
        <button type="button" onClick={() => void submit()} disabled={!canSubmit() || saving()}>{saving() ? "Saving…" : "Create alert"}</button>
      </div>
    </div>
  );
}

function FeedReader() {
  const [items, setItems] = createSignal<import("./types").FeedItem[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);

  const loadItems = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await api.feedItems({ limit: 50 });
      setItems(res.items || []);
    } catch (e) {
      setError(String(e));
    }
    setLoading(false);
  };

  createEffect(() => loadItems());

  return (
    <Show when={!loading()} fallback={<div class="empty">Loading…</div>}>
      <Show when={!error()} fallback={<LoadError message={error()!} onRetry={loadItems} />}>
      <Show when={items().length === 0}>
        <div class="empty">Nothing collected yet. Add a source, then press “Check for new items”.</div>
      </Show>
      <For each={items()}>
        {(item) => (
          <div class="feed-item-card">
            <div class="title"><a href={item.url} target="_blank">{item.title}</a></div>
            <Show when={item.summary}>
              <div class="summary">{item.summary!.slice(0, 200)}</div>
            </Show>
            <div class="meta">
              <Show when={item.source_name}><span class="source">{item.source_name}</span></Show>
              <Show when={item.published_at}><span class="date">{item.published_at!.slice(0, 10)}</span></Show>
              <Show when={item.source_trust}><span class="trust">{item.source_trust}</span></Show>
            </div>
            <Show when={item.tags && item.tags.length > 0}>
              <div class="tags">
                <For each={item.tags!}>
                  {(tag) => <span class="chip chip-tone-info">{tag}</span>}
                </For>
              </div>
            </Show>
          </div>
        )}
      </For>
      </Show>
    </Show>
  );
}

function FeedWizard(props: { onClose: () => void; onAdded: () => void | Promise<void> }) {
  const [step, setStep] = createSignal(1);
  const [selectedType, setSelectedType] = createSignal<string | null>(null);
  const [name, setName] = createSignal("");
  const [url, setUrl] = createSignal("");
  const [interval, setInterval] = createSignal("1h");
  const [tags, setTags] = createSignal("");
  const [trust, setTrust] = createSignal("medium");
  const [scope, setScope] = createSignal<"workspace" | "global">("workspace");
  const [sourceTypes] = createResource(() => api.feedSourceTypes());

  const typeOptions = () => {
    const types = sourceTypes();
    if (!types || !types.source_types) {
      return [
        { id: "rss", label: "RSS / Atom", desc: "Any RSS or Atom feed URL" },
        { id: "youtube", label: "YouTube", desc: "Follow a channel's uploads" },
        { id: "hacker_news", label: "Hacker News", desc: "Top stories, best new, Ask HN" },
        { id: "reddit", label: "Reddit", desc: "Follow subreddits" },
        { id: "custom_http", label: "Something else", desc: "Any web address that returns data" },
      ];
    }
    return types.source_types.map((t) => ({
      id: t.id,
      label: t.name || t.id.charAt(0).toUpperCase() + t.id.slice(1).replace(/_/g, " "),
      desc: t.description || `Fetch from ${t.id}`,
    }));
  };

  const needsUrl = () => {
    const t = selectedType();
    return t && (t === "rss" || t === "youtube" || t === "custom_http");
  };

  const canSubmit = () => name() && (!needsUrl() || url());

  const submit = async () => {
    const source: import("./types").FeedSource = {
      name: name(),
      type: selectedType()!,
      url: url() || undefined,
      interval: interval(),
      tags: tags() ? tags().split(",").map((t) => t.trim()) : [],
      trust: trust(),
      scope: scope(),
    };
    try {
      await api.feedAddSource(source);
      await props.onAdded();
      pushToast("info", `Source "${name()}" added`);
    } catch (e) {
      pushToast("alert", `Could not add the source: ${e}`);
    }
  };

  return (
      <div class="feed-wizard-overlay" onClick={props.onClose}>
        <div class="feed-wizard" onClick={(e) => e.stopPropagation()}>
        <div class="feed-wizard-header"><div><h2>Add a source</h2><p class="dim">Choose a source, then set how it should be checked.</p></div><button class="icon-button" aria-label="Close" onClick={props.onClose}>×</button></div>
        <Show when={step() === 1}>
          <div class="step">
            <div class="step-label">What do you want to follow?</div>
            <div class="type-grid">
              <For each={typeOptions()}>
                {(opt) => (
                  <button
                    type="button"
                    class={`type-option ${selectedType() === opt.id ? "selected" : ""}`}
                    onClick={() => { setSelectedType(opt.id); setStep(2); }}
                  >
                    <div class="label">{opt.label}</div>
                    <div class="desc">{opt.desc}</div>
                  </button>
                )}
              </For>
            </div>
          </div>
        </Show>
        <Show when={step() === 2}>
          <div class="step">
            <div class="step-label">Details</div>
            <div class="form-row">
              <label>Name</label>
              <input value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="My Feed" />
            </div>
            <Show when={needsUrl()}>
              <div class="form-row">
                <label>URL</label>
                <input value={url()} onInput={(e) => setUrl(e.currentTarget.value)} placeholder="https://example.com/feed.xml" />
              </div>
            </Show>
            <div class="form-row">
              <label>Availability</label>
              <select value={scope()} onChange={(e) => setScope(e.currentTarget.value as "workspace" | "global")}>
                <option value="workspace">Workspace</option>
                <option value="global">Global</option>
              </select>
            </div>
            <div class="form-row">
              <label>Check for new items</label>
              <select value={interval()} onChange={(e) => setInterval(e.currentTarget.value)}>
                <option value="5m">Every 5 minutes</option>
                <option value="15m">Every 15 minutes</option>
                <option value="30m">Every 30 minutes</option>
                <option value="1h">Every hour</option>
                <option value="6h">Every 6 hours</option>
                <option value="1d">Daily</option>
              </select>
            </div>
            <div class="form-row">
              <label>Tags</label>
              <input value={tags()} onInput={(e) => setTags(e.currentTarget.value)} placeholder="tech, news — separate with commas" />
            </div>
            <div class="form-row">
              <label>Trust level</label>
              <select value={trust()} onChange={(e) => setTrust(e.currentTarget.value)}>
                <option value="high">High — weighs in more on search/alerts</option>
                <option value="medium">Medium</option>
                <option value="low">Low — noted but discounted</option>
              </select>
            </div>
            <div style={{ "margin-top": "12px", display: "flex", gap: "8px" }}>
              <button class="ghost" onClick={() => setStep(1)}>Back</button>
              <button onClick={() => submit()} disabled={!canSubmit()}>Add source</button>
            </div>
          </div>
        </Show>
        <div style={{ "margin-top": "12px", "text-align": "right" }}>
          <button class="ghost" onClick={props.onClose}>Cancel</button>
        </div>
      </div>
    </div>
  );
}

export default function App() {
  /** Ask the canonical endpoint, not a data route.
   *
   * This used to probe by calling `api.config()` and reading the 401,
   * which works but cannot be *granted* a session — so on loopback the
   * console showed a token form to reach the machine the operator was
   * already sitting at. `/auth/session` answers the same question and,
   * on loopback, hands over the cookie instead of asking for it. */
  const probeAuth = () => {
    api.session()
      .then((s) => {
        setAuthed(s.authenticated);
        if (s.authenticated) connectEvents();
      })
      .catch(() => setAuthed(false));
  };

  createEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const linkToken = params.get("token");
    if (!linkToken) {
      probeAuth();
      return;
    }
    const url = new URL(window.location.href);
    url.searchParams.delete("token");
    window.history.replaceState(null, "", url.toString());
    api.login(linkToken)
      .then(() => {
        setAuthed(true);
        connectEvents();
      })
      .catch(() => probeAuth());
  });

  createEffect(() => {
    if (authed() !== true) return;
    refreshAdminAgents();
    let alive = true;
    const tick = async () => {
      try {
        const { count } = await api.unreadCount();
        if (alive) setUnread(count);
      } catch (err) {
        // A cookie left over from before a server restart (a fresh
        // token, or a full self uninstall --purge / reinstall) fails
        // every request from here on, but nothing about the page looks
        // broken -- it just quietly stops updating. Bounce back to the
        // login screen instead of polling a dead session forever.
        if (err instanceof AuthRequired) setAuthed(false);
      }
    };
    void tick();
    const t = setInterval(tick, 30_000);
    onCleanup(() => {
      alive = false;
      clearInterval(t);
    });
  });

  const currentRoute = () => {
    const r = route().split("?", 1)[0] || "#/overview";
    if (r === "#/setup") return "#/setup";
    if (r.startsWith("#/sessions/")) return "transcript";
    if (r === "#/feeds" || r.startsWith("#/feeds/") || r === "#/search" || r.startsWith("#/search/")) return r;
    // Operations owns a real subtree. Resolve it before the generic
    // top-level prefix matcher so nested routes never fall through to a
    // different screen when the hash carries a query or detail segment.
    if (r === "#/operations" || r.startsWith("#/operations/")) return "#/operations";
    return NAV.find((n) => r === n.hash || r.startsWith(`${n.hash}/`))?.hash ?? "#/overview";
  };

  const operationsSection = () => {
    const r = route().split("?", 1)[0];
    if (r === "#/operations/work" || r.startsWith("#/operations/work/")) return "work" as const;
    if (r === "#/operations/runtime" || r.startsWith("#/operations/runtime/")) return "runtime" as const;
    if (r === "#/operations/channels" || r.startsWith("#/operations/channels/")) return "channels" as const;
    if (r === "#/operations/automations" || r.startsWith("#/operations/automations/")) return "automations" as const;
    if (r === "#/operations/providers" || r.startsWith("#/operations/providers/")) return "providers" as const;
    if (r === "#/operations/incidents" || r.startsWith("#/operations/incidents/")) return "incidents" as const;
    return "overview" as const;
  };

  const navItemActive = (item: NavItem) => {
    const current = currentRoute();
    if (item.hash === "#/memory" && (current === "#/feeds" || current === "#/search")) return true;
    return current === item.hash;
  };

  return (
    <Switch>
      <Match when={authed() === null}>
        <div class="boot"><div class="spin" /></div>
      </Match>
      <Match when={authed() === false}>
        <Login />
      </Match>
      <Match when={authed() === true}>
        <div class="shell">
          <aside class="sidebar">
            <div class="brand"><span class="brand-mark"><img src="/admin/vak-icon.png" alt="" /></span> vak</div>
            <div class="sidebar-scope">
              <ScopeControl />
            </div>
            <nav aria-label="Primary">
              <For each={NAV}>
                {(item, index) => (
                  <>
                    <Show when={index() === 0 || NAV[index() - 1].group !== item.group}>
                      <div class="nav-group-label">{item.group}</div>
                    </Show>
                    <a href={navHref(item.hash)} classList={{ active: navItemActive(item) }}>
                      <Icon d={item.icon} />
                      {item.label}
                      <Show when={"badge" in item && item.badge?.() && Number(item.badge!()) > 0}>
                        <span class="nav-badge">{item.badge!()}</span>
                      </Show>
                    </a>
                    <Show when={item.children && navItemActive(item)}>
                      <div class="nav-sub">
                        <For each={item.children ?? []}>
                          {(child) => (
                            <a
                              class="sub"
                              href={navHref(child.hash)}
                              classList={{ active: item.activeChild?.() === child.hash }}
                            >
                              {child.label}
                            </a>
                          )}
                        </For>
                      </div>
                    </Show>
                  </>
                )}
              </For>
            </nav>
            <div class="sidebar-foot">
              <span class={`dot dot-${conn()}`} />
              <span class="conn-label">{conn()}</span>
            </div>
          </aside>
          <main class="main">
            <AdminContextBar />
            <Switch>
              <Match when={currentRoute() === "#/setup"}><SetupWizard /></Match>
              <Match when={currentRoute() === "#/overview"}><Home /></Match>
              <Match when={currentRoute() === "#/sessions"}><Sessions /></Match>
              <Match when={currentRoute() === "#/commitments"}><Commitments /></Match>
              <Match when={currentRoute() === "#/operations"}><OperationsCenter section={operationsSection()} /></Match>
              <Match when={currentRoute() === "transcript"}>
                <Transcript sessionId={route().slice("#/sessions/".length)} />
              </Match>
              <Match when={currentRoute() === "#/integrations"}><ExtensionsSection /></Match>
              <Match when={currentRoute() === "#/gateway"}><GatewaySection /></Match>
              <Match when={currentRoute() === "#/memory"}><MemoryView /></Match>
              <Match when={currentRoute() === "#/feeds"}><FeedsSection /></Match>
              <Match when={currentRoute() === "#/search"}><SearchView /></Match>
              <Match when={currentRoute() === "#/inbox"}><Inbox /></Match>
              <Match when={currentRoute() === "#/finops"}><FinOpsView /></Match>
              <Match when={currentRoute() === "#/security"}><Security /></Match>
              <Match when={currentRoute() === "#/prompts"}><PromptsPage /></Match>
              <Match when={currentRoute() === "#/settings"}><Settings /></Match>
            </Switch>
          </main>
          <div class="toasts">
            <For each={toasts()}>
              {(t) => <div class={`toast toast-${t.kind}`}>{t.text}</div>}
            </For>
          </div>
        </div>
      </Match>
    </Switch>
  );
}
