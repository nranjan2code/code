import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal } from "solid-js";
import { api } from "./api";
import { headlessAuth } from "./headlessAuth";
import { MODES, PageHeader, SEC_KINDS, confirmDestructive, secKindLabel } from "./display";
import { timeAgo } from "./time";
import { navigate, pushToast, route, selectedAgentId, selectedAgentIdOrUndefined } from "./store";
import type {
  ConfigInfo,
  ConfigScope,
  GatewayApprovalPolicy,
  HealthInfo,
  PermissionMode,
  SecurityEvent,
} from "./types";

export type RuleDecision = "allow" | "ask" | "deny";

export interface ParsedRule {
  raw: string;
  tool: string;
  pattern: string | null;
  decision: RuleDecision;
}

const DECISION_TONE: Record<RuleDecision, string> = {
  allow: "success",
  ask: "warning",
  deny: "danger",
};

export const RULE_SECTIONS: { decision: RuleDecision; title: string; empty: string }[] = [
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

export function parseRule(raw: string, listDecision: RuleDecision): ParsedRule | null {
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

export function parseRuleLists(rules: import("./types").PermissionRules | undefined): ParsedRule[] {
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

export function globMatch(pattern: string, value: string): boolean {
  const rx = new RegExp(
    `^${pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\\*/g, ".*").replace(/\\?/g, ".")}$`,
  );
  return rx.test(value);
}

function describeRule(rule: ParsedRule): string {
  if (rule.tool === "mcp") {
    return rule.pattern ? `the connected app ${rule.pattern.split("/")[0]}` : "connected apps";
  }
  if (rule.pattern) return `${rule.tool} matching ${rule.pattern}`;
  return rule.tool;
}

export function RuleChip(props: { rule: ParsedRule }) {
  return (
    <span
      class={`chip chip-phrase chip-tone-${DECISION_TONE[props.rule.decision]}`}
      title={`${props.rule.decision}: ${props.rule.raw}`}
    >
      {describeRule(props.rule)}
    </span>
  );
}

function approverOptions(policy: GatewayApprovalPolicy): string[] {
  const out = [...(policy.candidates ?? [])];
  if (policy.approver && !out.includes(policy.approver)) out.unshift(policy.approver);
  return out;
}

export function ApprovalForwarding() {
  const [policy, { refetch }] = createResource(() => api.gatewayApprovals());
  const [target, setTarget] = createSignal("");
  const [busy, setBusy] = createSignal(false);

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
            When Vakyartha running in a chat hits something that needs approval, it either refuses on
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
                  chat — Vakyartha will say so instead of trying.
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
              <Show
                when={(p().candidates?.length ?? 0) > 0 || p().approver}
                fallback={
                  <p class="dim">
                    No chats are approved yet. Add one under <a href="#/gateway">Chats</a> first —
                    a request can only be sent somewhere Vakyartha is already allowed to talk.
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

export function RuleEditor(props: { scope: ConfigScope; onSaved?: () => void | Promise<void> }) {
  const [view, { refetch }] = createResource(
    () => props.scope,
    (scope: ConfigScope) => api.permissionRules(scope, selectedAgentIdOrUndefined()),
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
      await api.setPermissionRules(props.scope, { [d]: next }, selectedAgentIdOrUndefined());
      await Promise.all([refetch(), props.onSaved?.()]);
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

interface SimAction {
  label: string;
  tool: string;
  arg: string;
  desc: string;
}

const PRESET_SIM_ACTIONS: SimAction[] = [
  { label: "Bash: rm -rf *", tool: "bash", arg: "rm -rf *", desc: "Dangerous recursive file deletion in shell" },
  { label: "Write: .env", tool: "write", arg: ".env", desc: "Modify workspace root environment secrets" },
  { label: "Write: src/main.rs", tool: "write", arg: "src/main.rs", desc: "Standard code edit within workspace" },
  { label: "Read: /etc/passwd", tool: "read", arg: "/etc/passwd", desc: "Attempted read of system root file" },
  { label: "Read: src/main.rs", tool: "read", arg: "src/main.rs", desc: "Safe inspection of workspace file" },
  { label: "Webfetch: github.com", tool: "webfetch", arg: "https://api.github.com", desc: "Outbound HTTPS network call" },
  { label: "Mcp: filesystem/read", tool: "mcp", arg: "filesystem/read", desc: "Invoking external MCP plugin" },
];

function runGateSimulation(
  tool: string,
  arg: string,
  rules: ParsedRule[],
  mode: string,
  approvalMode: string,
): {
  verdict: "allow" | "ask" | "deny";
  reason: string;
  trace: { hit: boolean; text: string }[];
} {
  const t = tool.toLowerCase().trim();
  const a = arg.trim();

  const matches = (r: ParsedRule) => {
    if (r.tool !== t) return false;
    if (!r.pattern) return true;
    return globMatch(r.pattern, a);
  };

  const denyRule = rules.find((r) => r.decision === "deny" && matches(r));
  if (denyRule) {
    return {
      verdict: "deny",
      reason: `Matched explicit Deny rule: "${denyRule.raw}". Never allowed rules take absolute precedence over modes and approval settings.`,
      trace: [
        { hit: true, text: `Step 1: Check Deny rulebook → MATCHED "${denyRule.raw}" (Denied)` },
        { hit: false, text: "Step 2: Check Ask rulebook (Skipped)" },
        { hit: false, text: "Step 3: Check Allow rulebook (Skipped)" },
        { hit: false, text: "Step 4: Check Execution Authority mode (Skipped)" },
      ],
    };
  }

  const askRule = rules.find((r) => r.decision === "ask" && matches(r));
  if (askRule) {
    return {
      verdict: "ask",
      reason: `Matched explicit Ask rule: "${askRule.raw}". Must pause for human approval before execution.`,
      trace: [
        { hit: false, text: "Step 1: Check Deny rulebook → No deny match" },
        { hit: true, text: `Step 2: Check Ask rulebook → MATCHED "${askRule.raw}" (Escalated)` },
        { hit: false, text: "Step 3: Check Allow rulebook (Skipped)" },
        { hit: false, text: "Step 4: Check Execution Authority mode (Skipped)" },
      ],
    };
  }

  const allowRule = rules.find((r) => r.decision === "allow" && matches(r));
  if (allowRule) {
    return {
      verdict: "allow",
      reason: `Matched explicit Allow rule: "${allowRule.raw}". Pre-approved by security rulebook.`,
      trace: [
        { hit: false, text: "Step 1: Check Deny rulebook → No deny match" },
        { hit: false, text: "Step 2: Check Ask rulebook → No ask match" },
        { hit: true, text: `Step 3: Check Allow rulebook → MATCHED "${allowRule.raw}" (Pre-approved)` },
        { hit: false, text: "Step 4: Check Execution Authority mode (Skipped)" },
      ],
    };
  }

  const trace: { hit: boolean; text: string }[] = [
    { hit: false, text: "Step 1: Check Deny rulebook → No match" },
    { hit: false, text: "Step 2: Check Ask rulebook → No match" },
    { hit: false, text: "Step 3: Check Allow rulebook → No match" },
  ];

  const m = mode.toLowerCase();
  if (m === "full-access" || m === "fullaccess") {
    trace.push({ hit: true, text: "Step 4: Mode is FullAccess → Unconstrained execution permitted" });
    return {
      verdict: "allow",
      reason: "FullAccess mode active: All actions permitted without sandbox constraints (Invariant 13 trust in effect).",
      trace,
    };
  }

  if (m === "read-only" || m === "readonly") {
    const isReadTool = ["read", "glob", "grep", "search", "list_dir", "view_file"].includes(t);
    if (!isReadTool) {
      trace.push({ hit: true, text: `Step 4: Mode is ReadOnly → Non-read tool "${t}" refused` });
      return {
        verdict: "deny",
        reason: "ReadOnly mode strictly refuses file modifications and subprocess executions (Invariant 10).",
        trace,
      };
    }
    const isExternal = a.startsWith("/") || a.startsWith("..");
    if (isExternal) {
      trace.push({ hit: true, text: `Step 4: Mode is ReadOnly → Path "${a}" escapes workspace root` });
      return {
        verdict: "deny",
        reason: "Workspace-rooted containment (Invariant 10): Traversal outside workspace fails closed.",
        trace,
      };
    }
    trace.push({ hit: true, text: `Step 4: Mode is ReadOnly → Read-only tool "${t}" inside workspace allowed` });
    return {
      verdict: "allow",
      reason: "ReadOnly mode permits reading files inside the workspace root.",
      trace,
    };
  }

  // WorkspaceWrite (default)
  const isRead = ["read", "glob", "grep", "search", "list_dir", "view_file"].includes(t);
  const isWrite = ["write", "edit", "create", "multi_replace_file_content", "replace_file_content", "write_to_file"].includes(t);
  const isExternal = a.startsWith("/") || a.startsWith("..");

  if (isRead) {
    if (isExternal) {
      trace.push({ hit: true, text: `Step 4: WorkspaceWrite → Path "${a}" outside workspace root fails closed` });
      return {
        verdict: "deny",
        reason: "Restricted filesystem access is workspace-rooted (Invariant 10). Traversal outside workspace fails closed.",
        trace,
      };
    }
    trace.push({ hit: true, text: "Step 4: WorkspaceWrite → Safe workspace read permitted" });
    return {
      verdict: "allow",
      reason: "WorkspaceWrite permits safe read access within the workspace.",
      trace,
    };
  }

  if (isWrite) {
    if (isExternal) {
      trace.push({ hit: true, text: `Step 4: WorkspaceWrite → External write to "${a}" requires approval` });
      return {
        verdict: "ask",
        reason: "Writing outside canonical workspace requires explicit human approval (Invariant 10/16).",
        trace,
      };
    }
    trace.push({ hit: true, text: "Step 4: WorkspaceWrite → Write within workspace permitted" });
    return {
      verdict: "allow",
      reason: "WorkspaceWrite permits modifying files inside the canonical workspace.",
      trace,
    };
  }

  if (t === "bash") {
    if (approvalMode === "auto-approve") {
      trace.push({ hit: true, text: "Step 4: Approval mode is Auto-Approve → Subprocess approved within sandbox" });
      return {
        verdict: "allow",
        reason: "Auto-approve resolves ordinary Ask decisions; seatbelt sandbox containment remains active.",
        trace,
      };
    }
    trace.push({ hit: true, text: "Step 4: WorkspaceWrite → Subprocess shell execution requires operator approval" });
    return {
      verdict: "ask",
      reason: "Subprocess shell execution requires operator approval under WorkspaceWrite mode.",
      trace,
    };
  }

  if (approvalMode === "auto-approve") {
    trace.push({ hit: true, text: `Step 4: Approval mode is Auto-Approve → Tool "${t}" allowed` });
    return {
      verdict: "allow",
      reason: "Auto-approve resolved tool invocation; OS sandbox policy still applies.",
      trace,
    };
  }
  trace.push({ hit: true, text: `Step 4: Tool "${t}" requires operator approval` });
  return {
    verdict: "ask",
    reason: "External connector and network invocation require operator approval.",
    trace,
  };
}

export function GateSimulator(props: {
  rules: ParsedRule[];
  mode: string;
  approvalMode: string;
}) {
  const [selectedPreset, setSelectedPreset] = createSignal<number>(0);
  const [customTool, setCustomTool] = createSignal("");
  const [customArg, setCustomArg] = createSignal("");
  const [isCustom, setIsCustom] = createSignal(false);

  const activeAction = () => {
    if (isCustom()) {
      return {
        label: "Custom Action",
        tool: customTool().trim() || "bash",
        arg: customArg().trim(),
        desc: "Custom operator test action",
      };
    }
    return PRESET_SIM_ACTIONS[selectedPreset()] ?? PRESET_SIM_ACTIONS[0];
  };

  const simulation = createMemo(() => {
    const act = activeAction();
    return runGateSimulation(act.tool, act.arg, props.rules, props.mode, props.approvalMode);
  });

  return (
    <div class="sim-container panel" style={{ "margin-bottom": "16px" }}>
      <div class="panel-title-row">
        <div>
          <h2>Live Gate Simulator</h2>
          <p class="dim">
            Test how the 4-step authorization engine resolves tool actions against active rules and permission boundaries in real time.
          </p>
        </div>
        <span class={`chip chip-tone-${simulation().verdict === "allow" ? "success" : simulation().verdict === "ask" ? "warning" : "danger"}`}>
          {simulation().verdict.toUpperCase()}
        </span>
      </div>

      <div class="eyebrow">Action Presets</div>
      <div class="sim-actions-grid">
        <For each={PRESET_SIM_ACTIONS}>
          {(action, idx) => (
            <button
              type="button"
              class="sim-action-btn"
              classList={{ active: !isCustom() && selectedPreset() === idx() }}
              onClick={() => {
                setIsCustom(false);
                setSelectedPreset(idx());
              }}
            >
              <span class="sim-tool">{action.tool}</span>
              <span class="sim-arg">{action.arg}</span>
              <span class="dim" style={{ "font-size": "10.5px" }}>{action.desc}</span>
            </button>
          )}
        </For>
      </div>

      <div class="form-row" style={{ "margin-top": "8px" }}>
        <div style={{ "display": "flex", "align-items": "center", "gap": "8px" }}>
          <button
            type="button"
            class="ghost small"
            onClick={() => {
              setIsCustom(!isCustom());
              if (!customTool()) setCustomTool("bash");
              if (!customArg()) setCustomArg("whoami");
            }}
          >
            {isCustom() ? "Use preset actions" : "Test custom action"}
          </button>
        </div>
        <Show when={isCustom()}>
          <div class="row-gap" style={{ "margin-top": "8px" }}>
            <input
              placeholder="tool (e.g. bash, write, read, webfetch)"
              value={customTool()}
              onInput={(e) => setCustomTool(e.currentTarget.value)}
              style={{ "max-width": "180px" }}
            />
            <input
              placeholder="target / argument (e.g. rm -rf *, src/lib.rs)"
              value={customArg()}
              onInput={(e) => setCustomArg(e.currentTarget.value)}
            />
          </div>
        </Show>
      </div>

      <div class={`sim-verdict-box ${simulation().verdict}`}>
        <div class={`sim-verdict-title ${simulation().verdict}`}>
          <span>
            {simulation().verdict === "allow" ? "ALLOW (Pre-approved / Within Bounds)" : simulation().verdict === "ask" ? "ASK (Human Approval Required)" : "DENY (Blocked by Security Policy)"}
          </span>
        </div>
        <p style={{ "margin": "0", "font-size": "12.5px" }}>{simulation().reason}</p>

        <div class="sim-trace">
          <div class="eyebrow" style={{ "margin-top": "8px" }}>Evaluation Trace:</div>
          <For each={simulation().trace}>
            {(step) => (
              <div class="sim-trace-step" classList={{ hit: step.hit }}>
                <span class="step-mark">{step.hit ? "▶" : "·"}</span>
                <span>{step.text}</span>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}

function SandboxArchitectureDeck(props: {
  health: () => HealthInfo | undefined;
  config: () => ConfigInfo | undefined;
  permissionMode: string;
}) {
  const sandboxName = () => props.health()?.sandbox || props.config()?.sandbox || "Seatbelt";
  const cwd = () => props.health()?.cwd || "—";
  const isFullAccess = () => props.permissionMode === "FullAccess";

  return (
    <section class="panel">
      <div class="panel-title-row">
        <div>
          <h2>OS Sandbox & Boundary Architecture</h2>
          <p class="dim">
            Kernel-level process isolation, filesystem containment, and execution barriers (AGENTS.md Invariants 10, 12, 14).
          </p>
        </div>
        <span
          class="chip"
          classList={{
            "chip-tone-danger": isFullAccess(),
            "chip-tone-success": !isFullAccess(),
          }}
        >
          {isFullAccess() ? "FullAccess (Unsandboxed)" : `${sandboxName()} Active`}
        </span>
      </div>

      <div class="sandbox-architecture-deck">
        {/* Layer 1 */}
        <div class="sandbox-layer-card" classList={{ "active-jail": !isFullAccess() }}>
          <div class="sandbox-layer-top">
            <div class="sandbox-layer-title-wrap">
              <span class="sandbox-layer-num">L1</span>
              <strong class="sandbox-layer-name">OS Kernel Process Sandbox</strong>
            </div>
            <span
              class="chip"
              classList={{
                "chip-tone-danger": isFullAccess(),
                "chip-tone-info": !isFullAccess(),
              }}
            >
              {isFullAccess() ? "bypassed" : sandboxName()}
            </span>
          </div>
          <p class="sandbox-layer-desc">
            {isFullAccess()
              ? "FullAccess explicitly bypasses OS containment. Child processes run with full ambient host permissions."
              : "Subprocesses run under Apple Seatbelt (deny default) profile / Linux Landlock ABI. Socket creation, system binary writes, and daemon launches are denied at the kernel syscall boundary."}
          </p>
          <div class="sandbox-layer-meta">
            <span>engine: {sandboxName()}</span>
            <span>·</span>
            <span>scope: all child processes</span>
          </div>
        </div>

        {/* Layer 2 */}
        <div class="sandbox-layer-card" classList={{ "active-jail": !isFullAccess() }}>
          <div class="sandbox-layer-top">
            <div class="sandbox-layer-title-wrap">
              <span class="sandbox-layer-num">L2</span>
              <strong class="sandbox-layer-name">Workspace Filesystem Root Containment</strong>
            </div>
            <span class="chip chip-phrase mono">{cwd()}</span>
          </div>
          <p class="sandbox-layer-desc">
            Invariant 10: Automatic read, write, edit, glob, and grep tools resolve strictly within this canonical directory. Parent traversals (<code>../</code>) and escaping symlinks fail closed.
          </p>
          <div class="sandbox-layer-meta">
            <span>invariant: 10</span>
            <span>·</span>
            <span>enforcement: path_in_workspace</span>
          </div>
        </div>

        {/* Layer 3 */}
        <div class="sandbox-layer-card">
          <div class="sandbox-layer-top">
            <div class="sandbox-layer-title-wrap">
              <span class="sandbox-layer-num">L3</span>
              <strong class="sandbox-layer-name">Quarantined Scratch Sandbox</strong>
            </div>
            <span class="chip chip-tone-info mono">.vak/scratch/</span>
          </div>
          <p class="sandbox-layer-desc">
            2026 Unified Sandboxed Runtime: Ephemeral bash executions, compiler artifacts, and test runners run isolated in <code>.vak/scratch/</code>. Modified files require explicit operator promotion before altering the project workspace.
          </p>
          <div class="sandbox-layer-meta">
            <span>isolation: quarantined scratch</span>
            <span>·</span>
            <span>telemetry: 500ms process monitor</span>
          </div>
        </div>

        {/* Layer 4 */}
        <div class="sandbox-layer-card">
          <div class="sandbox-layer-top">
            <div class="sandbox-layer-title-wrap">
              <span class="sandbox-layer-num">L4</span>
              <strong class="sandbox-layer-name">Operational Environment Allowlist</strong>
            </div>
            <span class="chip chip-tone-success">Scrubbed (No Passthrough)</span>
          </div>
          <p class="sandbox-layer-desc">
            Invariant 12: Secrets are never ambient tool state. LLM provider keys, gateway credentials, and session tokens are scrubbed from child process environments. Only explicit variables (<code>PATH</code>, <code>HOME</code>, <code>LANG</code>, <code>TMPDIR</code>) pass through.
          </p>
          <div class="sandbox-layer-meta">
            <span>invariant: 12</span>
            <span>·</span>
            <span>credentials: recipient-scoped only</span>
          </div>
        </div>

        {/* Layer 5 */}
        <div class="sandbox-layer-card">
          <div class="sandbox-layer-top">
            <div class="sandbox-layer-title-wrap">
              <span class="sandbox-layer-num">L5</span>
              <strong class="sandbox-layer-name">Brokered Worker Process Group</strong>
            </div>
            <span class="chip chip-tone-info mono">__tool_worker</span>
          </div>
          <p class="sandbox-layer-desc">
            Invariant 14: Built-in tools and MCP servers execute across a brokered boundary in disposable process groups with process-group kill capability (<code>setpgid</code>). Workers never hold policy engines, session stores, or control-plane handles.
          </p>
          <div class="sandbox-layer-meta">
            <span>invariant: 14</span>
            <span>·</span>
            <span>boundary: brokered RPC</span>
          </div>
        </div>
      </div>
    </section>
  );
}

function PrecedenceFlowchartCard() {
  return (
    <div class="precedence-flowchart-card">
      <div class="panel-title-row">
        <div>
          <h3>5-Stage Authorization Precedence</h3>
          <p class="dim">
            The deterministic cascade evaluated by the PermissionEngine on every action before tool dispatch.
          </p>
        </div>
        <span class="chip chip-tone-info">Deterministic Cascade</span>
      </div>
      <div class="precedence-steps-grid">
        <div class="precedence-step">
          <div class="precedence-step-head">
            <span class="precedence-step-num">STAGE 1</span>
            <span class="chip chip-tone-danger" style={{ "font-size": "9.5px" }}>Deny</span>
          </div>
          <strong class="precedence-step-title">Explicit Deny</strong>
          <p class="precedence-step-desc">Checked first. If tool matches any <code>deny</code> pattern, refused immediately.</p>
          <div class="precedence-step-outcome">
            <span class="dim" style={{ "font-size": "10.5px" }}>⇒ Fails closed</span>
          </div>
        </div>

        <div class="precedence-step">
          <div class="precedence-step-head">
            <span class="precedence-step-num">STAGE 2</span>
            <span class="chip chip-tone-warning" style={{ "font-size": "9.5px" }}>Ask</span>
          </div>
          <strong class="precedence-step-title">Explicit Ask</strong>
          <p class="precedence-step-desc">If tool matches any <code>ask</code> pattern, forces approval gate escalation.</p>
          <div class="precedence-step-outcome">
            <span class="dim" style={{ "font-size": "10.5px" }}>⇒ Raises Ask</span>
          </div>
        </div>

        <div class="precedence-step">
          <div class="precedence-step-head">
            <span class="precedence-step-num">STAGE 3</span>
            <span class="chip chip-tone-success" style={{ "font-size": "9.5px" }}>Allow</span>
          </div>
          <strong class="precedence-step-title">Explicit Allow</strong>
          <p class="precedence-step-desc">If tool matches any <code>allow</code> pattern, executes without prompting.</p>
          <div class="precedence-step-outcome">
            <span class="dim" style={{ "font-size": "10.5px" }}>⇒ Dispatches</span>
          </div>
        </div>

        <div class="precedence-step">
          <div class="precedence-step-head">
            <span class="precedence-step-num">STAGE 4</span>
            <span class="chip chip-phrase" style={{ "font-size": "9.5px" }}>Mode</span>
          </div>
          <strong class="precedence-step-title">Authority Mode</strong>
          <p class="precedence-step-desc">No rule match. Mode decides: ReadOnly refuses writes; WorkspaceWrite permits workspace writes.</p>
          <div class="precedence-step-outcome">
            <span class="dim" style={{ "font-size": "10.5px" }}>⇒ Baseline rule</span>
          </div>
        </div>

        <div class="precedence-step">
          <div class="precedence-step-head">
            <span class="precedence-step-num">STAGE 5</span>
            <span class="chip chip-phrase" style={{ "font-size": "9.5px" }}>Gate</span>
          </div>
          <strong class="precedence-step-title">Approval Strategy</strong>
          <p class="precedence-step-desc">Resolves raised Ask: auto-approve, approve-safe, or forwards to human chat approver.</p>
          <div class="precedence-step-outcome">
            <span class="dim" style={{ "font-size": "10.5px" }}>⇒ Final verdict</span>
          </div>
        </div>
      </div>
    </div>
  );
}

export function SecurityCenter(props: { scope: () => ConfigScope }) {
  const [account, { refetch: refetchAccount }] = createResource(() => headlessAuth.account().catch(() => null));
  const [accountBusy, setAccountBusy] = createSignal(false);
  const [accountError, setAccountError] = createSignal("");
  const addOwnerPasskey = async () => {
    setAccountBusy(true); setAccountError("");
    try { await headlessAuth.addPasskey(); await refetchAccount(); pushToast("info", "Passkey added"); }
    catch (error) { setAccountError(error instanceof Error ? error.message : String(error)); }
    finally { setAccountBusy(false); }
  };
  const signOutEverywhere = async () => {
    if (!confirmDestructive("Sign out every browser session on this server?")) return;
    setAccountBusy(true); setAccountError("");
    try { await headlessAuth.revokeAll(); window.location.reload(); }
    catch (error) { setAccountError(error instanceof Error ? error.message : String(error)); setAccountBusy(false); }
  };
  type SecTab = "execution" | "rules" | "approvals" | "audit";
  const [activeTab, setActiveTab] = createSignal<SecTab>("execution");

  createEffect(() => {
    const r = route().split("?", 1)[0];
    if (r === "#/security/rules") setActiveTab("rules");
    else if (r === "#/security/approvals") setActiveTab("approvals");
    else if (r === "#/security/audit") setActiveTab("audit");
    else if (r === "#/security/execution" || r === "#/security") setActiveTab("execution");
  });

  const switchTab = (tab: SecTab) => {
    setActiveTab(tab);
    navigate(`#/security/${tab}`);
  };

  const [config, { refetch: refetchConfig }] = createResource(selectedAgentId, () => api.config(selectedAgentIdOrUndefined()));
  const [layer, { refetch: refetchLayer }] = createResource(props.scope, (scope) => api.configLayer(scope, selectedAgentIdOrUndefined()));
  const [gatewayPolicy, { refetch: refetchPolicy }] = createResource(() => api.gatewayApprovals());
  const [health, { refetch: refetchHealth }] = createResource(() => api.health());

  // Audit Events
  const [kind, setKind] = createSignal("");
  const [searchQuery, setSearchQuery] = createSignal("");
  const [selectedEvent, setSelectedEvent] = createSignal<SecurityEvent | null>(null);
  const [events, { refetch: refetchEvents }] = createResource(kind, (k) => api.security(500, k || undefined));

  const rules = createMemo(() => parseRuleLists(config()?.permissions));

  const selectedPermissionMode = () => {
    const fromLayer = layer()?.permission_mode;
    if (fromLayer) {
      if (fromLayer === "read-only") return "ReadOnly";
      if (fromLayer === "workspace-write") return "WorkspaceWrite";
      if (fromLayer === "full-access") return "FullAccess";
    }
    const mode = config()?.permission_mode;
    if (!mode) return "WorkspaceWrite";
    if (mode === "read-only") return "ReadOnly";
    if (mode === "workspace-write") return "WorkspaceWrite";
    if (mode === "full-access") return "FullAccess";
    return mode;
  };

  const inheritedPermissionMode = () => !layer()?.permission_mode;

  const selectedApprovalMode = () => {
    const fromLayer = layer()?.approval_mode;
    if (fromLayer) return fromLayer;
    return config()?.approval_mode ?? "ask";
  };

  const inheritedApprovalMode = () => !layer()?.approval_mode;

  const setPermissionModeWithConfirmation = async (targetMode: "ReadOnly" | "WorkspaceWrite" | "FullAccess") => {
    const apiModeValue = targetMode === "ReadOnly" ? "read-only" : targetMode === "WorkspaceWrite" ? "workspace-write" : "full-access";
    if (targetMode === "FullAccess") {
      const confirmed = confirmDestructive(
        "Invariant 13: Explicit Human Trust Confirmation\n\nEnabling FullAccess grants unconstrained, arbitrary execution across your entire host filesystem without OS sandbox containment.\n\nAll active capability leases and side-runs will be revoked (Invariant 11).\n\nDo you explicitly trust this environment to run with FullAccess?",
      );
      if (!confirmed) return;
    }
    try {
      await api.patchConfigScope(props.scope(), { permission_mode: apiModeValue }, selectedAgentIdOrUndefined());
      await Promise.all([refetchConfig(), refetchLayer()]);
      pushToast("info", `Execution authority set to “${targetMode}” (${props.scope() === "user" ? "Global" : "Workspace"})`);
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const setApprovalMode = async (targetApproval: "ask" | "approve-safe" | "auto-approve") => {
    try {
      await api.patchConfigScope(props.scope(), { approval_mode: targetApproval }, selectedAgentIdOrUndefined());
      await Promise.all([refetchConfig(), refetchLayer()]);
      pushToast("info", `Approval policy set to “${targetApproval}”`);
    } catch (err) {
      pushToast("alert", `${err}`);
    }
  };

  const rulesFor = (decision: RuleDecision) => rules().filter((r) => r.decision === decision);

  const filteredEvents = createMemo(() => {
    const list = events()?.events ?? [];
    const q = searchQuery().trim().toLowerCase();
    if (!q) return list;
    return list.filter((e) =>
      e.label.toLowerCase().includes(q) ||
      e.detail.toLowerCase().includes(q) ||
      e.kind.toLowerCase().includes(q) ||
      (e.ip && e.ip.toLowerCase().includes(q)),
    );
  });

  return (
    <div class="view">
      <PageHeader
        title="Permissions & Security"
        description="Unified authority engine, sandbox telemetry, multi-tier rulebooks, gate forwarding, and append-only forensic audit trail."
      />

      <Show when={account()}>
        {(owner) => <section class="card">
          <h2>Owner sign-in</h2>
          <p>{owner().passkeys} passkey{owner().passkeys === 1 ? "" : "s"} · {owner().recovery_codes_remaining} recovery codes left</p>
          <p>Passkeys sign you in to this server. Save recovery codes outside Vakyartha.</p>
          <button type="button" disabled={accountBusy()} onClick={() => void addOwnerPasskey()}>Add passkey</button>
          <button type="button" disabled={accountBusy()} onClick={() => void signOutEverywhere()}>Sign out all browsers</button>
          <Show when={accountError()}><p role="alert">{accountError()}</p></Show>
        </section>}
      </Show>

      {/* Executive Posture Deck */}
      <div class="security-posture-deck">
        <div class="stat-card">
          <span class="stat-card-label">Authority Mode</span>
          <div style={{ "display": "flex", "align-items": "center", "gap": "6px", "margin": "4px 0" }}>
            <span class={`chip chip-tone-${selectedPermissionMode() === "FullAccess" ? "danger" : selectedPermissionMode() === "ReadOnly" ? "info" : "success"}`}>
              {selectedPermissionMode()}
            </span>
            <Show when={inheritedPermissionMode()}>
              <span class="dim" style={{ "font-size": "10.5px" }}>inherited</span>
            </Show>
          </div>
          <span class="stat-card-hint">
            {props.scope() === "user" ? "Global Scope" : "Workspace Rooted"}
          </span>
        </div>

        <div class="stat-card">
          <span class="stat-card-label">Approval Strategy</span>
          <div style={{ "display": "flex", "align-items": "center", "gap": "6px", "margin": "4px 0" }}>
            <span class="chip chip-tone-info">
              {selectedApprovalMode() === "ask" ? "Ask" : selectedApprovalMode() === "approve-safe" ? "Approve Safe" : "Auto-Approve"}
            </span>
          </div>
          <span class="stat-card-hint">Escalation Policy</span>
        </div>

        <div class="stat-card">
          <span class="stat-card-label">OS Sandbox Engine</span>
          <div style={{ "display": "flex", "align-items": "center", "gap": "6px", "margin": "4px 0" }}>
            <span class={`chip chip-tone-${selectedPermissionMode() === "FullAccess" ? "danger" : "success"}`}>
              {selectedPermissionMode() === "FullAccess" ? "Bypassed (Unsandboxed)" : (config()?.sandbox ?? "Seatbelt")}
            </span>
          </div>
          <span class="stat-card-hint">
            {selectedPermissionMode() === "FullAccess" ? "Full Host Execution" : "Process Group Isolation"}
          </span>
        </div>

        <div class="stat-card">
          <span class="stat-card-label">Forensic Audit Log</span>
          <div style={{ "display": "flex", "align-items": "center", "gap": "6px", "margin": "4px 0" }}>
            <strong style={{ "font-size": "18px", "font-weight": "700" }}>
              {events()?.events.length ?? 0}
            </strong>
            <span class="dim" style={{ "font-size": "12px" }}>recorded events</span>
          </div>
          <span class="stat-card-hint">Append-only JSONL</span>
        </div>
      </div>

      {/* Safety Invariants Strip */}
      <div class="security-invariants-bar">
        <span class="eyebrow" style={{ "margin": "0" }}>Active Invariant Guardrails:</span>
        <span class="chip chip-phrase chip-tone-success">Invariant 13: Explicit Human Trust</span>
        <span class="chip chip-phrase chip-tone-info">Invariant 11: Mode Lease Revocation</span>
        <span class="chip chip-phrase chip-tone-info">Invariant 10 & 16: Universal Boundary Check</span>
        <span class="chip chip-phrase chip-tone-info">Invariant 12: Secret Hygiene</span>
      </div>

      {/* Subnav Tabs */}
      <div class="security-tab-bar">
        <button
          type="button"
          class="security-tab-btn"
          classList={{ active: activeTab() === "execution" }}
          onClick={() => switchTab("execution")}
        >
          Execution & Sandbox
        </button>
        <button
          type="button"
          class="security-tab-btn"
          classList={{ active: activeTab() === "rules" }}
          onClick={() => switchTab("rules")}
        >
          Security Rulebook & Simulator
        </button>
        <button
          type="button"
          class="security-tab-btn"
          classList={{ active: activeTab() === "approvals" }}
          onClick={() => switchTab("approvals")}
        >
          Gateways & Approvals
        </button>
        <button
          type="button"
          class="security-tab-btn"
          classList={{ active: activeTab() === "audit" }}
          onClick={() => switchTab("audit")}
        >
          Audit Ledger & Forensics
          <Show when={events()?.events.length}>
            <span class="nav-badge" style={{ "margin-left": "4px" }}>{events()?.events.length}</span>
          </Show>
        </button>
      </div>

      {/* Tab 1: Execution & Sandbox */}
      <Show when={activeTab() === "execution"}>
        <div class="two-col">
          <div class="stack">
            <section class="panel">
              <div class="panel-title-row">
                <div>
                  <h2>Execution Authority Mode</h2>
                  <p class="dim">
                    Governs workspace-rooted file mutation and host command isolation.
                  </p>
                </div>
              </div>
              <div class="mode-grid">
                <For each={MODES}>
                  {(m) => (
                    <button
                      class="mode-btn"
                      classList={{ active: selectedPermissionMode() === m.value }}
                      onClick={() => void setPermissionModeWithConfirmation(m.value as "ReadOnly" | "WorkspaceWrite" | "FullAccess")}
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
                      onClick={() => void setApprovalMode(m.value)}
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
            </section>
          </div>

          <div class="stack">
            <SandboxArchitectureDeck
              health={health}
              config={config}
              permissionMode={selectedPermissionMode()}
            />
          </div>
        </div>
      </Show>

      {/* Tab 2: Security Rulebook & Simulator */}
      <Show when={activeTab() === "rules"}>
        <PrecedenceFlowchartCard />

        <GateSimulator
          rules={rules()}
          mode={config()?.permission_mode ?? "workspace-write"}
          approvalMode={config()?.approval_mode ?? "ask"}
        />

        <section class="panel">
          <div class="panel-title-row">
            <div>
              <h2>Security Rulebook</h2>
              <p class="dim">
                Explicit pre-approved (allow), escalated (ask), and blocked (deny) patterns.
              </p>
            </div>
          </div>

          <div class="rulebook-grid">
            <div class="rulebook-col deny">
              <div class="rulebook-col-head">
                <span>Never allowed (Deny)</span>
                <span class="chip chip-tone-danger">{rulesFor("deny").length}</span>
              </div>
              <Show
                when={rulesFor("deny").length > 0}
                fallback={<p class="dim" style={{ "font-size": "11.5px" }}>Nothing blocked outright beyond mode defaults.</p>}
              >
                <div class="chip-stack">
                  <For each={rulesFor("deny")}>{(r) => <RuleChip rule={r} />}</For>
                </div>
              </Show>
            </div>

            <div class="rulebook-col ask">
              <div class="rulebook-col-head">
                <span>Always ask first (Ask)</span>
                <span class="chip chip-tone-warning">{rulesFor("ask").length}</span>
              </div>
              <Show
                when={rulesFor("ask").length > 0}
                fallback={<p class="dim" style={{ "font-size": "11.5px" }}>Nothing forced to ask beyond mode defaults.</p>}
              >
                <div class="chip-stack">
                  <For each={rulesFor("ask")}>{(r) => <RuleChip rule={r} />}</For>
                </div>
              </Show>
            </div>

            <div class="rulebook-col allow">
              <div class="rulebook-col-head">
                <span>Always allowed (Allow)</span>
                <span class="chip chip-tone-success">{rulesFor("allow").length}</span>
              </div>
              <Show
                when={rulesFor("allow").length > 0}
                fallback={<p class="dim" style={{ "font-size": "11.5px" }}>Nothing pre-approved beyond mode defaults.</p>}
              >
                <div class="chip-stack">
                  <For each={rulesFor("allow")}>{(r) => <RuleChip rule={r} />}</For>
                </div>
              </Show>
            </div>
          </div>

          <div style={{ "margin-top": "16px" }}>
            <RuleEditor scope={props.scope()} onSaved={() => void refetchConfig()} />
          </div>
        </section>
      </Show>

      {/* Tab 3: Gateways & Approvals */}
      <Show when={activeTab() === "approvals"}>
        <div class="stack">
          <ApprovalForwarding />

          <section class="panel">
            <div class="panel-title-row">
              <div>
                <h2>Unattended Surfaces & Escalation Policy</h2>
                <p class="dim">AGENTS.md Invariant 15 enforcement.</p>
              </div>
            </div>
            <div style={{ "display": "flex", "flex-direction": "column", "gap": "8px", "font-size": "12.5px", "color": "var(--text-soft)" }}>
              <p>
                Under Invariant 15, unattended surfaces always fail closed. Chat-driven turns auto-deny escalations unless an explicitly configured approver chat answers within the timeout window. Silence, network timeout, or missing delivery credentials resolve to a strict denial.
              </p>
              <p class="dim">
                To manage connected chat bots and channel allowlists, visit <a href="#/gateway">Chats</a>.
              </p>
            </div>
          </section>
        </div>
      </Show>

      {/* Tab 4: Audit Ledger & Forensics */}
      <Show when={activeTab() === "audit"}>
        <div class="panel">
          <div class="panel-title-row">
            <div>
              <h2>Forensic Audit Ledger</h2>
              <p class="dim">Immutable, append-only record of every sign-in, blocked action, key mutation, and gate decision.</p>
            </div>
          </div>

          <div class="toolbar" style={{ "margin-bottom": "14px", "gap": "10px", "flex-wrap": "wrap" }}>
            <input
              placeholder="Search audit trail by keyword, ip, detail..."
              value={searchQuery()}
              onInput={(e) => setSearchQuery(e.currentTarget.value)}
              style={{ "min-width": "260px", "flex": "1" }}
            />
            <div class="chips">
              <button
                class="chip-btn"
                classList={{ active: kind() === "" }}
                onClick={() => setKind("")}
              >
                All
              </button>
              <For each={SEC_KINDS}>
                {(k) => (
                  <button
                    class="chip-btn"
                    classList={{ active: kind() === k.value }}
                    onClick={() => setKind(k.value)}
                    title={k.value}
                  >
                    {k.label}
                  </button>
                )}
              </For>
            </div>
            <button class="ghost" onClick={() => refetchEvents()}>Refresh</button>
          </div>

          <Show when={!events.loading} fallback={<div class="empty">Loading audit events…</div>}>
            <Show
              when={filteredEvents().length > 0}
              fallback={<div class="empty">No audit events match your filter. Quiet is good.</div>}
            >
              <table class="table">
                <thead>
                  <tr>
                    <th>when</th>
                    <th>what</th>
                    <th>summary</th>
                    <th>details</th>
                    <th>from</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={filteredEvents()}>
                    {(e: SecurityEvent) => (
                      <tr
                        data-kind={e.kind}
                        style={{ "cursor": "pointer" }}
                        onClick={() => setSelectedEvent(selectedEvent() === e ? null : e)}
                      >
                        <td title={e.ts}>{timeAgo(e.ts)}</td>
                        <td>
                          <span class="chip chip-phrase" data-kind={e.kind} title={e.kind}>
                            {secKindLabel(e.kind)}
                          </span>
                        </td>
                        <td><strong>{e.label}</strong></td>
                        <td class="mono dim wrap">{e.detail}</td>
                        <td class="mono dim">{e.ip ?? "—"}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </Show>
          </Show>

          {/* Event Detail Modal / Expander */}
          <Show when={selectedEvent()}>
            {(e) => (
              <div class="panel" style={{ "margin-top": "16px", "border": "1px solid var(--border-strong)", "background": "var(--surface)" }}>
                <div class="panel-title-row">
                  <div>
                    <h3>Event Forensics: {e().label}</h3>
                    <p class="dim" style={{ "margin": "2px 0 0" }}>{e().ts} (Origin: {e().ip ?? "local"})</p>
                  </div>
                  <button class="ghost small" onClick={() => setSelectedEvent(null)}>Close</button>
                </div>
                <div style={{ "display": "grid", "grid-template-columns": "120px 1fr", "gap": "8px", "margin-top": "10px", "font-size": "12px" }}>
                  <span class="dim">Kind:</span>
                  <span><span class="chip chip-phrase" data-kind={e().kind}>{e().kind}</span></span>
                  <span class="dim">Timestamp:</span>
                  <span class="mono">{e().ts}</span>
                  <span class="dim">IP Address:</span>
                  <span class="mono">{e().ip ?? "None (Internal / Broker)"}</span>
                  <span class="dim">Raw Detail:</span>
                  <pre style={{ "background": "var(--surface-raised)", "padding": "8px", "border-radius": "4px", "margin": "0", "white-space": "pre-wrap", "word-break": "break-all" }}>
                    {e().detail}
                  </pre>
                </div>
              </div>
            )}
          </Show>
        </div>
      </Show>
    </div>
  );
}
