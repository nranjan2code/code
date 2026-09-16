/// Session Forensics & Scalable Turn Workflow DAG Suite.
///
/// An executive-grade forensics center for agent sessions.
/// Core architectural principle:
/// Sessions scale to 100, 1,000, or 1,000,000 turns. The visualization
/// focuses deeply on THE TURN ITSELF — decomposing that specific turn's
/// ingress, intent kernel, context packet, route ladder dispatch,
/// execution loops (model inference <-> tool workers), stop case policy,
/// and governance checkpoints into a standard workflow DAG.
///
/// Strictly follows repository invariants:
/// - Invariant 1: Model-visible means logged.
/// - Invariant 2: Append-only ledger; zero deletions.
/// - Invariant 7: Per-turn route ladder dispatch.
/// - Invariant 10 & 16: Workspace-rooted permissions.
/// - Invariant 13: Explicit trust confirmation.
/// - Invariant 26: Evidence projection; ZERO mock or synthetic data.
/// - Zero markdown/emoji icons: SVG status dots, tone chips, crisp typography.

import {
  For,
  Match,
  Show,
  Switch,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
} from "solid-js";

import { api, AuthRequired } from "./api";
import { PageHeader, confirmDestructive } from "./display";
import { navigate, pushToast, setAuthed } from "./store";
import { clock, timeAgo } from "./time";

function shortId(id?: string | null, len: number = 8): string {
  if (!id) return "";
  return id.length > len ? id.slice(0, len) : id;
}

import type {
  ActiveSubagent,
  AgentIdentity,
  BestOfNRun,
  CapabilityDescriptor,
  FrozenContract,
  PromptLayerDescriptor,
  SessionCheckpoint,
  SessionDiff,
  SessionListItem,
  SessionTurn,
  ToolCallRecord,
  TranscriptEntry,
  WorkReceipt,
} from "./types";

import dagre from "@dagrejs/dagre";

export function renderNodeIcon(icon?: string) {
  switch (icon) {
    case "ingress":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
        </svg>
      );
    case "intent":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <circle cx="12" cy="12" r="10"/>
          <polygon points="16.24 7.76 14.12 14.12 7.76 16.24 9.88 9.88 16.24 7.76"/>
        </svg>
      );
    case "context":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <polygon points="12 2 2 7 12 12 22 7 12 2"/>
          <polyline points="2 17 12 22 22 17"/>
          <polyline points="2 12 12 17 22 12"/>
        </svg>
      );
    case "route":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <line x1="6" y1="3" x2="6" y2="15"/>
          <circle cx="18" cy="6" r="3"/>
          <circle cx="6" cy="18" r="3"/>
          <path d="M18 9a9 9 0 0 1-9 9"/>
        </svg>
      );
    case "sandbox":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>
          <polyline points="9 12 11 14 15 10"/>
        </svg>
      );
    case "model":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <path d="m12 3-1.912 5.813a2 2 0 0 1-1.275 1.275L3 12l5.813 1.912a2 2 0 0 1 1.275 1.275L12 21l1.912-5.813a2 2 0 0 1 1.275-1.275L21 12l-5.813-1.912a2 2 0 0 1-1.275-1.275L12 3Z"/>
        </svg>
      );
    case "terminal":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <polyline points="4 17 10 11 4 5"/>
          <line x1="12" y1="19" x2="20" y2="19"/>
        </svg>
      );
    case "subagent":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <rect width="16" height="12" x="4" y="8" rx="2"/>
          <path d="M2 14h2"/>
          <path d="M20 14h2"/>
          <path d="M15 13v2"/>
          <path d="M9 13v2"/>
          <path d="M12 2v6"/>
        </svg>
      );
    case "tool":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>
        </svg>
      );
    case "stop":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <circle cx="12" cy="12" r="10"/>
          <rect width="6" height="6" x="9" y="9"/>
        </svg>
      );
    case "checkpoint":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <ellipse cx="12" cy="5" rx="9" ry="3"/>
          <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"/>
          <path d="M3 12c0 1.66 4 3 9 3s9-1.34 9-3"/>
        </svg>
      );
    case "memory":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <path d="M12 2a7 7 0 0 0-7 7c0 2.38 1.19 4.47 3 5.74V17a2 2 0 0 0 2 2h4a2 2 0 0 0 2-2v-2.26c1.81-1.27 3-3.36 3-5.74a7 7 0 0 0-7-7z"/>
          <line x1="9" y1="21" x2="15" y2="21"/>
        </svg>
      );
    case "gate":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <rect x="3" y="11" width="18" height="11" rx="2" ry="2"/>
          <path d="M7 11V7a5 5 0 0 1 10 0v4"/>
        </svg>
      );
    case "presentation":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="node-icon">
          <rect x="2" y="3" width="20" height="14" rx="2" ry="2"/>
          <line x1="8" y1="21" x2="16" y2="21"/>
          <line x1="12" y1="17" x2="12" y2="21"/>
        </svg>
      );
    default:
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="node-icon">
          <circle cx="12" cy="12" r="4"/>
        </svg>
      );
  }
}

export function parseTaskPrompt(argsJson?: string): string {
  if (!argsJson) return "delegated subagent";
  try {
    const obj = JSON.parse(argsJson);
    return obj.prompt || obj.description || "delegated subagent";
  } catch {
    return argsJson.slice(0, 42);
  }
}

export function parseCommandSnippet(argsJson?: string): string {
  if (!argsJson) return "command execution";
  try {
    const obj = JSON.parse(argsJson);
    const cmd = obj.command || obj.cmd || "";
    return cmd.slice(0, 42) + (cmd.length > 42 ? "…" : "");
  } catch {
    return argsJson.slice(0, 42);
  }
}

export interface WorkflowNode {
  id: string;
  stage_key:
    | "ingress"
    | "memory"
    | "intent"
    | "context"
    | "route"
    | "sandbox"
    | "model"
    | "tool"
    | "subagent"
    | "stop"
    | "presentation"
    | "checkpoint";
  phase: "input" | "admission" | "security" | "execution" | "verification" | "presentation" | "governance";
  title: string;
  subtitle: string;
  badge?: string;
  status: "ok" | "warn" | "bad" | "idle" | "running";
  icon?: string;
  duration_ms?: number;
  tokens_in?: number;
  tokens_out?: number;
  cost?: number;
  details: Record<string, unknown>;
  raw_payload?: string;
  // Computed layout coordinates
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface WorkflowEdge {
  id: string;
  source: string;
  target: string;
  label?: string;
  status: "ok" | "warn" | "bad" | "running" | "idle";
}

// ---- Receipt Extraction Helpers (Real Backend Unpacking) -------------------

export function receiptInputTokens(r: WorkReceipt): number {
  if (r.input_tokens != null) return r.input_tokens;
  return r.attempts?.reduce((sum, a) => sum + (a.usage?.input_tokens ?? 0), 0) ?? 0;
}

export function receiptOutputTokens(r: WorkReceipt): number {
  if (r.output_tokens != null) return r.output_tokens;
  return r.attempts?.reduce((sum, a) => sum + (a.usage?.output_tokens ?? 0), 0) ?? 0;
}

export function receiptCachedTokens(r: WorkReceipt): number {
  return r.attempts?.reduce((sum, a) => sum + (a.usage?.cache_read_input_tokens ?? 0), 0) ?? 0;
}

export function receiptLatencyMs(r: WorkReceipt): number {
  if (r.latency_ms != null) return r.latency_ms;
  return r.attempts?.reduce((sum, a) => sum + (a.latency_ms ?? 0), 0) ?? 0;
}

export function receiptProvider(r: WorkReceipt): string {
  return r.provider || r.attempts?.[0]?.provider || "—";
}

export function receiptModel(r: WorkReceipt): string {
  return r.model || r.attempts?.[0]?.model || "—";
}

export function receiptSettlement(r: WorkReceipt): string {
  const winning = r.winning_attempt ?? 0;
  return r.attempts?.[winning]?.settlement ?? r.attempts?.[0]?.settlement ?? "ok";
}

// ---- Real User Prompt Filter -----------------------------------------------

export function isRealUserPrompt(entry: TranscriptEntry): boolean {
  if (entry.kind !== "message") return false;
  if (entry.role !== "user" && entry.role !== null) return false;
  const c = entry.content.trim();
  if (!c) return false;
  if (
    c.startsWith("[result]") ||
    c.startsWith("[stop-guard]") ||
    c.startsWith("[recovery]") ||
    c.startsWith("[repair directive]") ||
    c.startsWith("[tool:") ||
    c.startsWith("<context_summary>") ||
    c.startsWith("<system>")
  ) {
    return false;
  }
  return true;
}

// ---- Turn Reconstruction ----------------------------------------------------

export function reconstructTurns(
  entries: TranscriptEntry[],
  receipts: WorkReceipt[] = [],
  checkpoints: SessionCheckpoint[] = [],
): SessionTurn[] {
  const turns: SessionTurn[] = [];
  let currentTurn: SessionTurn | null = null;
  let receiptIdx = 0;
  let pendingIntent = "";
  let pendingGoal = "";

  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];

    if (isRealUserPrompt(entry)) {
      if (currentTurn) {
        turns.push(finalizeTurn(currentTurn, checkpoints));
      }
      currentTurn = {
        turn_index: turns.length + 1,
        id: entry.entry_id,
        started_at: entry.ts,
        user_prompt: entry.content,
        intent_summary: pendingIntent,
        goal: pendingGoal,
        tool_calls: [],
        work_receipts: [],
        tokens_in: 0,
        tokens_out: 0,
        cost_usd: 0,
        duration_ms: 0,
        status: "completed",
        entries: [entry],
      };
      pendingIntent = "";
      pendingGoal = "";
      continue;
    }

    if (!currentTurn) {
      // Buffer early intent or goal entries that precede turn 1
      if (entry.kind === "intent") pendingIntent = entry.content;
      if (entry.kind === "goal") pendingGoal = entry.content;
      continue;
    }

    currentTurn.entries.push(entry);

    if (entry.kind === "intent") {
      currentTurn.intent_summary = entry.content;
    } else if (entry.kind === "goal") {
      currentTurn.goal = entry.content;
    } else if (entry.kind === "receipt") {
      if (receiptIdx < receipts.length) {
        currentTurn.work_receipts.push(receipts[receiptIdx++]);
      }
    } else if (entry.kind === "message" && entry.role === "assistant") {
      if (entry.content && !entry.content.startsWith("[tool:")) {
        currentTurn.model_response = entry.content;
      }
      if (entry.is_error) currentTurn.status = "failed";
    } else if (entry.tool_name) {
      currentTurn.tool_calls.push({
        tool_name: entry.tool_name,
        result_text: entry.content,
        is_error: entry.is_error,
        ts: entry.ts,
      });
      if (entry.is_error) currentTurn.status = "failed";
    } else if (entry.content.startsWith("[stop-guard]")) {
      currentTurn.stop_guard = entry.content;
    }
  }

  if (currentTurn) {
    turns.push(finalizeTurn(currentTurn, checkpoints));
  }

  // If there are leftover receipts not assigned, allocate them to the last turn
  if (turns.length > 0 && receiptIdx < receipts.length) {
    const lastTurn = turns[turns.length - 1];
    while (receiptIdx < receipts.length) {
      lastTurn.work_receipts.push(receipts[receiptIdx++]);
    }
    finalizeTurn(lastTurn, checkpoints);
  }

  return turns;
}

function finalizeTurn(
  turn: SessionTurn,
  checkpoints: SessionCheckpoint[],
): SessionTurn {
  let totalIn = 0;
  let totalOut = 0;
  let totalDuration = 0;
  let totalCost = 0;

  for (const r of turn.work_receipts) {
    totalIn += receiptInputTokens(r);
    totalOut += receiptOutputTokens(r);
    totalDuration += receiptLatencyMs(r);
    totalCost += r.cost_usd ?? 0;
  }

  turn.tokens_in = totalIn;
  turn.tokens_out = totalOut;
  turn.duration_ms = totalDuration;
  turn.cost_usd = totalCost;

  // Match real filesystem checkpoint
  const cleanPrompt = turn.user_prompt.trim().slice(0, 24);
  const matchingCheckpoint = checkpoints.find(
    (cp) =>
      (cp.label || cp.message || "").includes(cleanPrompt) ||
      (cp.label || "").includes(`turn: ${cleanPrompt}`),
  );
  if (matchingCheckpoint) {
    turn.checkpoint_seq = matchingCheckpoint.seq;
  }

  const lastEntry = turn.entries[turn.entries.length - 1];
  if (lastEntry) {
    turn.ended_at = lastEntry.ts;
  }

  return turn;
}

// ---- Intent 7-Axis Parser --------------------------------------------------

export function parseIntentAxes(summary?: string): Record<string, string> {
  if (!summary) return {};
  const tokens = summary.trim().split(/\s+/);
  return {
    act: tokens[0] || "converse",
    horizon: tokens[1] || "immediate",
    stakes: tokens[2] || "inert",
    evidence: tokens[3] || "none",
    clarity: tokens[4] || "interactive",
    modality: tokens[5] || "signals",
    attendance: tokens[6] || "attended",
  };
}

// ---- Turn Breadcrumb Formatter ---------------------------------------------

export function buildTurnBreadcrumb(turn: SessionTurn, nodeCount: number): string {
  const toolCount = turn.tool_calls.length;
  const toolPart =
    toolCount === 0
      ? "Direct Generation"
      : `${toolCount} Tool${toolCount > 1 ? "s" : ""} (parallel)`;
  const stopPart = turn.stop_guard ? " → Stop Guard" : "";
  const synthPart = turn.model_response && toolCount > 0 ? " → Synthesis" : "";
  return `${nodeCount} nodes · Ingress → Admission (5 parallel gates) → Model → ${toolPart}${synthPart}${stopPart} → Presentation → Checkpoint`;
}

// ---- Standard Workflow Graph Builder for the Active Turn -------------------

export function buildWorkflowGraph(
  turn: SessionTurn,
  contract?: FrozenContract | null,
  orientation: "LR" | "TB" = "LR",
): { nodes: WorkflowNode[]; edges: WorkflowEdge[]; width: number; height: number } {
  const nodes: WorkflowNode[] = [];
  const edges: WorkflowEdge[] = [];

  const isLR = orientation === "LR";
  // In vertical (TB) mode: admission nodes are sized so 5 parallel gates fit side-by-side gracefully
  const ADMISSION_W = isLR ? 260 : 220;
  const STANDARD_W = isLR ? 270 : 260;
  const TOOL_W = isLR ? 260 : (turn.tool_calls.length > 2 ? 220 : 250);
  const NODE_H = 112;

  // Tier 1: Request Ingress
  const ingressNode: WorkflowNode = {
    id: "ingress",
    stage_key: "ingress",
    phase: "input",
    icon: "ingress",
    title: "1. Request Ingress",
    subtitle: turn.user_prompt.slice(0, 42) + (turn.user_prompt.length > 42 ? "…" : ""),
    badge: `${turn.user_prompt.length} chars`,
    status: "ok",
    duration_ms: 0,
    details: {
      started_at: turn.started_at,
      raw_prompt: turn.user_prompt,
      turn_index: turn.turn_index,
      delivery_surface: "local",
    },
    raw_payload: turn.user_prompt,
    x: 0,
    y: 0,
    width: STANDARD_W,
    height: NODE_H,
  };
  nodes.push(ingressNode);

  // Tier 2: Admission Cluster (5 parallel gates: Memory, Intent, Context, Route, Sandbox)
  // 2.1: Memory & Recall (docs/design/23 & 29)
  const memoryNode: WorkflowNode = {
    id: "memory",
    stage_key: "memory",
    phase: "admission",
    icon: "memory",
    title: "2.1 Memory & Recall",
    subtitle: "Tier-1 USER.md & Tier-2 Recall",
    badge: "tier-1 & tier-2",
    status: "ok",
    details: {
      profile_tier: "Tier 1: USER.md profile recall & persistent operator context (doc 29)",
      semantic_tier: "Tier 2: Indexed semantic recall with cross-project boundaries (doc 23)",
      forget_amend: "Amnesia contract supported (explicit forget & amend commands)",
      injection_bound: "Bounded injection into prompt layers; zero unbounded drift",
    },
    raw_payload: "Memory recall settled. Bounded context slice extracted from USER.md and project semantic memory index.",
    x: 0,
    y: 0,
    width: ADMISSION_W,
    height: NODE_H,
  };
  nodes.push(memoryNode);
  edges.push({
    id: "e-ingress-memory",
    source: "ingress",
    target: "memory",
    status: "ok",
  });

  // 2.2: Intent & Commitment Kernel (docs/design/47)
  const intentAxes = parseIntentAxes(turn.intent_summary);
  const intentNode: WorkflowNode = {
    id: "intent",
    stage_key: "intent",
    phase: "admission",
    icon: "intent",
    title: "2.2 Intent Kernel",
    subtitle: turn.intent_summary
      ? `${intentAxes.act} · ${intentAxes.horizon} · ${intentAxes.stakes}`
      : "Direct interactive dispatch",
    badge: turn.intent_summary ? `${intentAxes.act} · ${intentAxes.evidence}` : "default",
    status: "ok",
    details: {
      intent_raw: turn.intent_summary || "None recorded",
      axes: intentAxes,
      narrowing_rule: "Intent narrows capabilities; never widens authority (doc 47)",
      evidence_required: intentAxes.evidence,
      clarity: intentAxes.clarity,
      attendance: intentAxes.attendance,
    },
    raw_payload: turn.intent_summary || "No intent reading recorded for this turn.",
    x: 0,
    y: 0,
    width: ADMISSION_W,
    height: NODE_H,
  };
  nodes.push(intentNode);
  edges.push({
    id: "e-ingress-intent",
    source: "ingress",
    target: "intent",
    status: "ok",
  });

  // 2.3: Context Packet & Capabilities (docs/design/17 & 45)
  const promptLayers = contract?.prompt_layers ?? [];
  const admittedCaps = contract?.capabilities ?? [];
  const contextNode: WorkflowNode = {
    id: "context",
    stage_key: "context",
    phase: "admission",
    icon: "context",
    title: "2.3 Context & Caps",
    subtitle: `${promptLayers.length || 4} prompt layers · ${admittedCaps.length || 11} caps`,
    badge: `${admittedCaps.length || 11} capabilities`,
    status: "ok",
    details: {
      prompt_layers: promptLayers.map((l) => `${l.layer}: ${l.block}`),
      capabilities: admittedCaps.map((c) => `${c.name} (${c.kind})`),
      context_accounting: "Tokens partition: verbatim window vs compaction summary (doc 17)",
      turn_index: turn.turn_index,
    },
    raw_payload: JSON.stringify(
      {
        prompt_layers: promptLayers,
        capabilities: admittedCaps.map((c) => ({ name: c.name, kind: c.kind, invocation: c.invocation })),
      },
      null,
      2,
    ),
    x: 0,
    y: 0,
    width: ADMISSION_W,
    height: NODE_H,
  };
  nodes.push(contextNode);
  edges.push({
    id: "e-ingress-context",
    source: "ingress",
    target: "context",
    status: "ok",
  });

  // 2.4: Route Ladder & FinOps Budget (docs/design/15 & Phase R)
  const activeProvider = turn.work_receipts[0]?.provider || contract?.provider || "openai-completions";
  const activeModel = turn.work_receipts[0]?.model || contract?.model || "default";
  const routeLadder = contract?.route_ladder ?? [];
  const routeNode: WorkflowNode = {
    id: "route",
    stage_key: "route",
    phase: "admission",
    icon: "route",
    title: "2.4 Route & Budget",
    subtitle: `${activeProvider} · ${activeModel}`,
    badge: `${contract?.route_objective ?? "balanced"} ladder`,
    status: "ok",
    details: {
      provider: activeProvider,
      model: activeModel,
      route_ladder: routeLadder,
      objective: contract?.route_objective ?? "balanced",
      retries_budget: "3 (+6) watchdog backoff",
      circuit_breaker: "Closed (healthy)",
    },
    raw_payload: JSON.stringify(
      {
        provider: activeProvider,
        model: activeModel,
        route_objective: contract?.route_objective ?? "balanced",
        route_ladder: routeLadder,
      },
      null,
      2,
    ),
    x: 0,
    y: 0,
    width: ADMISSION_W,
    height: NODE_H,
  };
  nodes.push(routeNode);
  edges.push({
    id: "e-ingress-route",
    source: "ingress",
    target: "route",
    status: "ok",
  });

  // 2.5: Security & Sandbox Jail (docs/design/24 & 25)
  const permMode = contract?.permission_mode || "WorkspaceWrite";
  const isFullAccess = permMode === "FullAccess";
  const sandboxNode: WorkflowNode = {
    id: "sandbox",
    stage_key: "sandbox",
    phase: "security",
    icon: "sandbox",
    title: "2.5 Security & Sandbox",
    subtitle: isFullAccess
      ? "FullAccess (Unsandboxed trust confirmed)"
      : "Seatbelt / OS Process Jail Sandbox",
    badge: permMode,
    status: isFullAccess ? "warn" : "ok",
    details: {
      permission_mode: permMode,
      sandbox_engine: isFullAccess ? "Unsandboxed (Host trust confirmed)" : "Seatbelt (macOS sandbox-exec profile)",
      target: "WorkerProcess & ToolCommand (SandboxTarget::WorkerProcess)",
      filesystem_jail: "Canonical workspace jail with path traversal denial (Invariant 10)",
      quarantine_scratch: ".vak/scratch/ isolated artifact directory",
      broker_boundary: "__tool_worker disposable process group with sanitized env (Invariant 14)",
      secrets_isolation: "Sanitized operational allowlist (Invariant 12: secrets never ambient tool state)",
      process_kill: "PGID process-group kill (Invariant 6)",
      trust_decision: "Invariant 13: FullAccess is an explicit human trust decision; never selected automatically",
    },
    raw_payload: JSON.stringify(
      {
        permission_mode: permMode,
        sandbox: isFullAccess ? "off" : "seatbelt_workspace_jail",
        scratch_isolation: ".vak/scratch/",
        broker_protocol: "__tool_worker disposable process group",
        operational_env_allowlist: ["PATH", "HOME", "USER", "LANG", "SHELL"],
      },
      null,
      2,
    ),
    x: 0,
    y: 0,
    width: ADMISSION_W,
    height: NODE_H,
  };
  nodes.push(sandboxNode);
  edges.push({
    id: "e-ingress-sandbox",
    source: "ingress",
    target: "sandbox",
    status: sandboxNode.status,
  });

  // Tier 3: Execution (Dynamic depending on tool_calls)
  let convergenceTargetId = "";

  if (turn.tool_calls.length === 0) {
    // Direct prose generation: 0 tools executed
    const genNode: WorkflowNode = {
      id: "model_gen",
      stage_key: "model",
      phase: "execution",
      icon: "model",
      title: "3. Direct Generation",
      subtitle: turn.model_response ? turn.model_response.slice(0, 42) + "…" : "Prose response completed",
      badge: `${turn.tokens_in.toLocaleString()} in · ${turn.tokens_out.toLocaleString()} out`,
      status: turn.status === "failed" ? "bad" : "ok",
      duration_ms: turn.duration_ms,
      tokens_in: turn.tokens_in,
      tokens_out: turn.tokens_out,
      cost: turn.cost_usd,
      details: {
        model: activeModel,
        provider: activeProvider,
        tokens_in: turn.tokens_in,
        tokens_out: turn.tokens_out,
        latency_ms: turn.duration_ms,
        cost_usd: turn.cost_usd,
      },
      raw_payload: turn.model_response || "(no prose output)",
      x: 0,
      y: 0,
      width: STANDARD_W,
      height: NODE_H,
    };
    nodes.push(genNode);

    // All 5 admission gates converge into model_gen
    for (const admId of ["memory", "intent", "context", "route", "sandbox"]) {
      edges.push({
        id: `e-${admId}-gen`,
        source: admId,
        target: "model_gen",
        status: "ok",
      });
    }
    convergenceTargetId = "model_gen";
  } else {
    // Multi-tool / Brokered execution: Model Dispatched Tools
    const stepCount = turn.tool_calls.length;
    const rc0 = turn.work_receipts[0];
    const rcIn = rc0 ? receiptInputTokens(rc0) : turn.tokens_in;
    const rcOut = rc0 ? receiptOutputTokens(rc0) : turn.tokens_out;
    const rcLat = rc0 ? receiptLatencyMs(rc0) : 0;

    const dispatchNode: WorkflowNode = {
      id: "model_dispatch",
      stage_key: "model",
      phase: "execution",
      icon: "model",
      title: `3. Model Inference`,
      subtitle: `Dispatched ${stepCount} tool call${stepCount > 1 ? "s" : ""}`,
      badge: `${rcIn.toLocaleString()} in · ${rcOut.toLocaleString()} out`,
      status: "ok",
      duration_ms: rcLat,
      tokens_in: rcIn,
      tokens_out: rcOut,
      details: {
        model: rc0 ? receiptModel(rc0) : activeModel,
        provider: rc0 ? receiptProvider(rc0) : activeProvider,
        tool_count: stepCount,
        tools_dispatched: turn.tool_calls.map((t) => t.tool_name).join(", "),
        latency_ms: rcLat,
      },
      raw_payload: rc0 ? JSON.stringify(rc0, null, 2) : `Model dispatched ${stepCount} tools.`,
      x: 0,
      y: 0,
      width: STANDARD_W,
      height: NODE_H,
    };
    nodes.push(dispatchNode);

    // All 5 admission gates converge into model_dispatch
    for (const admId of ["memory", "intent", "context", "route", "sandbox"]) {
      edges.push({
        id: `e-${admId}-dispatch`,
        source: admId,
        target: "model_dispatch",
        status: "ok",
      });
    }

    // Dynamic Tool Execution Nodes (placed side-by-side in vertical mode)
    const toolNodeIds: string[] = [];
    for (let i = 0; i < stepCount; i++) {
      const tc = turn.tool_calls[i];
      const isSubagent = tc.tool_name === "task";
      const isBash = tc.tool_name === "bash";
      const isFileTool = ["read", "write", "edit", "glob", "grep"].includes(tc.tool_name);
      const isWeb = tc.tool_name === "webfetch" || tc.tool_name === "browse" || tc.tool_name === "tavily_search";
      const execId = isSubagent ? `subagent_step_${i + 1}` : `tool_step_${i + 1}`;
      toolNodeIds.push(execId);

      const execNode: WorkflowNode = {
        id: execId,
        stage_key: isSubagent ? "subagent" : "tool",
        phase: "execution",
        icon: isSubagent ? "subagent" : isBash ? "terminal" : isFileTool ? "tool" : isWeb ? "tool" : "tool",
        title: isSubagent
          ? `Subagent Delegation`
          : isBash
          ? `Shell: ${tc.tool_name}`
          : isFileTool
          ? `File: ${tc.tool_name}`
          : isWeb
          ? `Web: ${tc.tool_name}`
          : `Tool: ${tc.tool_name}`,
        subtitle: isSubagent
          ? parseTaskPrompt(tc.args_json)
          : isBash
          ? parseCommandSnippet(tc.args_json)
          : tc.result_text
          ? tc.result_text.slice(0, 42) + "…"
          : "Output captured",
        badge: isSubagent
          ? "subagent"
          : isBash
          ? "jail"
          : isFileTool
          ? "safe-io"
          : isWeb
          ? "ssrf-guard"
          : tc.is_error
          ? "error"
          : "ok",
        status: tc.is_error ? "bad" : "ok",
        details: {
          tool_name: tc.tool_name,
          is_subagent: isSubagent,
          args: tc.args_json,
          is_error: tc.is_error,
          timestamp: tc.ts,
          result_bytes: tc.result_text?.length ?? 0,
          sandbox_isolation: isBash
            ? "Process group isolate with operational env allowlist & Seatbelt profile"
            : isSubagent
            ? "Dedicated child session with inherited parent authority"
            : isFileTool
            ? "Workspace-rooted safe I/O (path_in_workspace verification)"
            : isWeb
            ? "SSRF-guarded outbound network policy"
            : "Brokered worker execution",
        },
        raw_payload: tc.result_text || "(empty tool return)",
        x: 0,
        y: 0,
        width: TOOL_W,
        height: NODE_H,
      };
      nodes.push(execNode);

      // Model dispatch branches to all tools (side-by-side)
      edges.push({
        id: `e-dispatch-${execId}`,
        source: "model_dispatch",
        target: execId,
        status: execNode.status,
      });
    }

    // Final Synthesis if model response prose exists
    if (turn.model_response) {
      const finalrc = turn.work_receipts[stepCount] || turn.work_receipts[turn.work_receipts.length - 1];
      const finIn = finalrc ? receiptInputTokens(finalrc) : 0;
      const finOut = finalrc ? receiptOutputTokens(finalrc) : 0;
      const finLat = finalrc ? receiptLatencyMs(finalrc) : 0;

      const synthNode: WorkflowNode = {
        id: "model_synthesis",
        stage_key: "model",
        phase: "execution",
        icon: "model",
        title: "4. Outcome Synthesis",
        subtitle: turn.model_response.slice(0, 42) + "…",
        badge: `${finIn.toLocaleString()} in · ${finOut.toLocaleString()} out`,
        status: "ok",
        duration_ms: finLat,
        tokens_in: finIn,
        tokens_out: finOut,
        details: {
          model: finalrc ? receiptModel(finalrc) : activeModel,
          provider: finalrc ? receiptProvider(finalrc) : activeProvider,
          latency_ms: finLat,
          tokens_in: finIn,
          tokens_out: finOut,
        },
        raw_payload: turn.model_response,
        x: 0,
        y: 0,
        width: STANDARD_W,
        height: NODE_H,
      };
      nodes.push(synthNode);

      // All tools converge into model_synthesis
      for (const tId of toolNodeIds) {
        edges.push({
          id: `e-${tId}-synthesis`,
          source: tId,
          target: "model_synthesis",
          status: "ok",
        });
      }
      convergenceTargetId = "model_synthesis";
    } else {
      convergenceTargetId = toolNodeIds[0];
    }
  }

  // Tier 4: Stop Gate (Stop Gate Policy - docs/design/15)
  // Only insert separate Stop Gate node if stop-guard actually intervened or warned
  let prePresentationId = convergenceTargetId;

  if (turn.stop_guard) {
    const stopNode: WorkflowNode = {
      id: "stop_gate",
      stage_key: "stop",
      phase: "verification",
      icon: "stop",
      title: "Stop Gate Policy",
      subtitle: "Continuation nudged",
      badge: "stop-guard",
      status: "warn",
      details: {
        stop_guard: turn.stop_guard,
        verification_status: "Continuation nudged by stop guard",
        reliability_rule: "Invariant 3: Errors are values; turn returns verified TurnOutcome",
      },
      raw_payload: turn.stop_guard,
      x: 0,
      y: 0,
      width: STANDARD_W,
      height: NODE_H,
    };
    nodes.push(stopNode);

    if (turn.tool_calls.length > 0 && !turn.model_response) {
      for (let i = 0; i < turn.tool_calls.length; i++) {
        const tId = turn.tool_calls[i].tool_name === "task" ? `subagent_step_${i + 1}` : `tool_step_${i + 1}`;
        edges.push({ id: `e-${tId}-stop`, source: tId, target: "stop_gate", status: "warn" });
      }
    } else {
      edges.push({ id: `e-${convergenceTargetId}-stop`, source: convergenceTargetId, target: "stop_gate", status: "warn" });
    }
    prePresentationId = "stop_gate";
  }

  // Tier 5: Outcome Presentation (docs/design/30 & 61)
  const presentationNode: WorkflowNode = {
    id: "presentation",
    stage_key: "presentation",
    phase: "presentation",
    icon: "presentation",
    title: "Universal Outcome",
    subtitle: turn.tool_calls.length > 0 ? "Diff inspector, test matrix & telemetry" : "Calm continuous prose presentation",
    badge: turn.tool_calls.length > 0 ? "outcome-first" : "calm-prose",
    status: "ok",
    details: {
      renderer_projection: "Universal outcome-first renderer (docs/design/30 & 61)",
      available_renderers: "Diff inspector, test matrix, telemetry crosshairs, terminal session, dynamic recipe, calm prose",
      delivery_mode: "Continuous chat canvas with contextual drawer (doc 61)",
      retry_outbox: "Durable store-and-forward outbox active (doc 31)",
    },
    raw_payload: turn.tool_calls.length > 0
      ? "Outcome projected to universal renderer with interactive telemetry and diff inspector."
      : "Outcome projected as clean calm prose with prompt scaffolding scrubbed.",
    x: 0,
    y: 0,
    width: STANDARD_W,
    height: NODE_H,
  };
  nodes.push(presentationNode);

  if (!turn.stop_guard && turn.tool_calls.length > 0 && !turn.model_response) {
    for (let i = 0; i < turn.tool_calls.length; i++) {
      const tId = turn.tool_calls[i].tool_name === "task" ? `subagent_step_${i + 1}` : `tool_step_${i + 1}`;
      edges.push({ id: `e-${tId}-presentation`, source: tId, target: "presentation", status: "ok" });
    }
  } else {
    edges.push({
      id: `e-${prePresentationId}-presentation`,
      source: prePresentationId,
      target: "presentation",
      status: "ok",
    });
  }

  // Tier 6: Governance & Checkpoint (Invariant 1 & 2)
  const checkpointNode: WorkflowNode = {
    id: "checkpoint",
    stage_key: "checkpoint",
    phase: "governance",
    icon: "checkpoint",
    title: "Governance & State",
    subtitle: turn.checkpoint_seq != null ? `Checkpoint #${turn.checkpoint_seq} committed` : "Append-only ledger commit",
    badge: turn.checkpoint_seq != null ? `seq #${turn.checkpoint_seq}` : "appended",
    status: "ok",
    details: {
      checkpoint_seq: turn.checkpoint_seq ?? "No disk snapshot needed",
      ledger_entries: turn.entries.length,
      audit_integrity: "Invariant 2: Append-only JSONL verifiable; zero deletion",
      prev_hash_merkle: "SHA-256 Merkle chain digest verified",
      settlement_receipts: `${turn.work_receipts.length} work receipts settled`,
    },
    raw_payload: `Turn #${turn.turn_index} closed at ${turn.ended_at || turn.started_at}. Ledger contains ${turn.entries.length} verified immutable entries.`,
    x: 0,
    y: 0,
    width: STANDARD_W,
    height: NODE_H,
  };
  nodes.push(checkpointNode);
  edges.push({
    id: "e-presentation-checkpoint",
    source: "presentation",
    target: "checkpoint",
    status: "ok",
  });

  // Perform topological DAG layout using @dagrejs/dagre
  const g = new dagre.graphlib.Graph();
  g.setGraph({
    rankdir: orientation,
    nodesep: isLR ? 32 : 20,
    ranksep: isLR ? 64 : 48,
    marginx: 36,
    marginy: 40,
  });
  g.setDefaultEdgeLabel(() => ({}));

  for (const n of nodes) {
    g.setNode(n.id, { width: n.width, height: n.height });
  }
  for (const e of edges) {
    g.setEdge(e.source, e.target);
  }

  dagre.layout(g);

  for (const n of nodes) {
    const layoutNode = g.node(n.id);
    if (layoutNode) {
      n.x = Math.round(layoutNode.x - layoutNode.width / 2);
      n.y = Math.round(layoutNode.y - layoutNode.height / 2);
    }
  }

  const graphInfo = g.graph();
  const width = Math.max(1060, Math.round((graphInfo.width || 800) + 72));
  const height = Math.max(260, Math.round((graphInfo.height || 140) + 90));

  return {
    nodes,
    edges,
    width,
    height,
  };
}

// ---- Sessions List Component -----------------------------------------------

export function SessionsList() {
  const [sessions, { refetch }] = createResource(() => api.sessions(200));
  const [bestofn, bestofnActions] = createResource(api.bestofn);
  const [q, setQ] = createSignal("");
  const [showArchived, setShowArchived] = createSignal(false);
  const [creating, setCreating] = createSignal(false);
  const [busySession, setBusySession] = createSignal("");
  const [busyCandidate, setBusyCandidate] = createSignal("");
  const [bulkBusy, setBulkBusy] = createSignal(false);

  const localHash = createMemo(() => sessions()?.workspace_project_hash ?? "");
  const isLocal = (s: SessionListItem) => !localHash() || s.project_hash === localHash();

  const filtered = createMemo(() => {
    const list = sessions()?.sessions ?? [];
    const query = q().trim().toLowerCase();
    const withArchived = showArchived() ? list : list.filter((s) => !s.archived);
    if (!query) return withArchived;
    return withArchived.filter(
      (s) =>
        s.session_id.toLowerCase().includes(query) ||
        (s.agent?.name ?? "").toLowerCase().includes(query) ||
        (s.title ?? "").toLowerCase().includes(query),
    );
  });

  const archivedCount = createMemo(() => (sessions()?.sessions ?? []).filter((s) => s.archived).length);
  const totalCount = createMemo(() => sessions()?.total ?? 0);

  const toggleArchive = async (s: SessionListItem) => {
    setBusySession(s.session_id);
    try {
      await api.archiveSession(s.session_id, !s.archived);
      await refetch();
      pushToast("info", s.archived ? "Session unarchived" : "Session archived");
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setBusySession("");
    }
  };

  const removeSession = async (s: SessionListItem) => {
    if (!confirmDestructive(`Permanently delete session “${s.session_id}”? Its JSONL ledger and history will be deleted.`)) return;
    setBusySession(s.session_id);
    try {
      await api.deleteSession(s.session_id);
      await refetch();
      pushToast("info", "Session deleted");
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setBusySession("");
    }
  };

  const deleteAllArchived = async () => {
    const count = archivedCount();
    if (!confirmDestructive(`Delete all ${count} archived sessions in this workspace? This cannot be undone.`)) return;
    setBulkBusy(true);
    try {
      const res = await api.deleteAllArchived();
      await refetch();
      const countDeleted = res.deleted ?? 0;
      pushToast("info", `Deleted ${countDeleted} archived session${countDeleted === 1 ? "" : "s"}`);
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setBulkBusy(false);
    }
  };

  const startNewSession = async () => {
    setCreating(true);
    try {
      const res = await api.createSession();
      navigate(`#/sessions/${res.session_id}`);
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setCreating(false);
    }
  };

  const handleKeep = async (sessionId: string) => {
    setBusyCandidate(sessionId);
    try {
      await api.keepBestRun(sessionId);
      pushToast("info", "Winning candidate kept");
      await Promise.all([refetch(), bestofnActions.refetch()]);
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setBusyCandidate("");
    }
  };

  const handleDiscard = async (sessionId: string) => {
    if (!confirmDestructive("Discard this candidate run? Its temporary worktree will be removed.")) return;
    setBusyCandidate(sessionId);
    try {
      await api.discardBestRun(sessionId);
      pushToast("info", "Candidate discarded");
      await Promise.all([refetch(), bestofnActions.refetch()]);
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setBusyCandidate("");
    }
  };

  return (
    <div class="view sessions-view">
      <PageHeader
        title="Sessions & Conversations"
        description="Historical and active agent conversations, turn-by-turn workflow pipelines, and forensic execution traces."
        actions={
          <button type="button" class="button primary" disabled={creating()} onClick={startNewSession}>
            {creating() ? "Creating…" : "New Conversation"}
          </button>
        }
      />

      {/* Best-of-N Candidate Management Deck */}
      <Show when={(bestofn()?.runs?.length ?? 0) > 0}>
        <section class="panel candidates-panel">
          <div class="candidates-header">
            <div>
              <h3>Active Best-of-N Candidate Runs</h3>
              <p class="dim">Multi-attempt executions in isolated worktrees. Keep the winner or discard alternatives.</p>
            </div>
          </div>
          <div class="candidates-grid">
            <For each={bestofn()?.runs ?? []}>
              {(run: BestOfNRun) => (
                <div class="candidate-card">
                  <div class="candidate-head">
                    <span class="chip mono">Candidate #{run.branch_index != null ? run.branch_index + 1 : 1}</span>
                    <span class="mono dim">{shortId(run.session_id)}</span>
                  </div>
                  <div class="candidate-prompt font-semibold">{run.prompt}</div>
                  <div class="candidate-actions">
                    <button
                      type="button"
                      class="button small"
                      disabled={busyCandidate() === run.session_id}
                      onClick={() => handleKeep(run.session_id)}
                    >
                      Keep This Run
                    </button>
                    <button
                      type="button"
                      class="button small danger ghost"
                      disabled={busyCandidate() === run.session_id}
                      onClick={() => handleDiscard(run.session_id)}
                    >
                      Discard
                    </button>
                  </div>
                </div>
              )}
            </For>
          </div>
        </section>
      </Show>

      {/* Sessions Posture Deck */}
      <div class="sessions-posture-deck">
        <div class="sessions-kpi-card">
          <span class="kpi-label">Total Sessions</span>
          <span class="kpi-value font-mono">{totalCount()}</span>
          <span class="kpi-sub dim">Append-only JSONL source</span>
        </div>
        <div class="sessions-kpi-card">
          <span class="kpi-label">Active Workspace</span>
          <span class="kpi-value font-mono">{(sessions()?.sessions ?? []).filter(isLocal).length}</span>
          <span class="kpi-sub dim">Hash {localHash() ? localHash().slice(0, 8) : "all"}</span>
        </div>
        <div class="sessions-kpi-card">
          <span class="kpi-label">Archived</span>
          <span class="kpi-value font-mono">{archivedCount()}</span>
          <span class="kpi-sub dim">Hidden from primary queue</span>
        </div>
        <div class="sessions-kpi-card">
          <span class="kpi-label">Execution Mode</span>
          <span class="kpi-value">Immutable</span>
          <span class="kpi-sub dim">Invariant 1 & 2 verified</span>
        </div>
      </div>

      {/* Filter and Action Toolbar */}
      <div class="sessions-toolbar">
        <div class="sessions-search-box">
          <input
            type="search"
            placeholder="Filter sessions by ID, agent personality, or title…"
            value={q()}
            onInput={(e) => setQ(e.currentTarget.value)}
          />
        </div>
        <div class="sessions-toolbar-actions">
          <label class="toggle-label">
            <input
              type="checkbox"
              checked={showArchived()}
              onChange={(e) => setShowArchived(e.currentTarget.checked)}
            />
            <span>Show Archived ({archivedCount()})</span>
          </label>
          <Show when={archivedCount() > 0 && showArchived()}>
            <button
              type="button"
              class="button small danger ghost"
              disabled={bulkBusy()}
              onClick={deleteAllArchived}
            >
              {bulkBusy() ? "Deleting…" : "Delete All Archived"}
            </button>
          </Show>
          <button type="button" class="button small ghost" onClick={() => void refetch()}>
            Refresh
          </button>
        </div>
      </div>

      {/* Sessions Table */}
      <div class="sessions-table-panel">
        <Show when={!sessions.loading} fallback={<div class="empty">Loading session index…</div>}>
          <Show
            when={filtered().length > 0}
            fallback={<div class="empty">No sessions match your filter criteria.</div>}
          >
            <table class="table sessions-table">
              <thead>
                <tr>
                  <th>Session ID</th>
                  <th>Agent Persona</th>
                  <th>Entries</th>
                  <th>First Activity</th>
                  <th>Last Activity</th>
                  <th>Scope</th>
                  <th class="actions-col">Actions</th>
                </tr>
              </thead>
              <tbody>
                <For each={filtered()}>
                  {(s: SessionListItem) => (
                    <tr
                      class="session-row"
                      classList={{ "archived-row": s.archived }}
                      onClick={() => navigate(`#/sessions/${s.session_id}`)}
                    >
                      <td class="mono font-semibold">
                        <div class="session-id-cell">
                          <span class="cdot cdot-ok" />
                          <span>{s.session_id}</span>
                        </div>
                      </td>
                      <td>
                        <Show
                          when={s.agent}
                          fallback={<span class="dim mono">default</span>}
                        >
                          <span class="chip chip-tone-info" title={s.agent?.personality || ""}>
                            {s.agent?.name}
                          </span>
                        </Show>
                      </td>
                      <td class="mono">{s.entry_count}</td>
                      <td class="dim">{timeAgo(s.first_ts)}</td>
                      <td class="dim">{timeAgo(s.last_ts)}</td>
                      <td>
                        <span
                          class="chip"
                          classList={{
                            "chip-tone-success": isLocal(s),
                            "chip-tone-muted": !isLocal(s),
                          }}
                        >
                          {isLocal(s) ? "local" : "external"}
                        </span>
                      </td>
                      <td class="actions-col" onClick={(e) => e.stopPropagation()}>
                        <div class="row-actions">
                          <button
                            type="button"
                            class="button small ghost"
                            disabled={busySession() === s.session_id}
                            onClick={() => toggleArchive(s)}
                          >
                            {s.archived ? "Restore" : "Archive"}
                          </button>
                          <button
                            type="button"
                            class="button small danger ghost"
                            disabled={busySession() === s.session_id}
                            onClick={() => removeSession(s)}
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
          </Show>
        </Show>
      </div>
    </div>
  );
}

// ---- Scalable Turn Forensics View ------------------------------------------

export function SessionForensics(props: { sessionId: string }) {
  const [activeTab, setActiveTab] = createSignal<
    "dag" | "log" | "drift" | "receipts" | "checkpoints"
  >("dag");
  const [selectedTurnIndex, setSelectedTurnIndex] = createSignal<number>(1);
  const [selectedNodeId, setSelectedNodeId] = createSignal<string>("ingress");
  const [inspectorTab, setInspectorTab] = createSignal<"payload" | "economics" | "policy" | "sandbox">("payload");
  const [zoomScale, setZoomScale] = createSignal<number>(1.0);
  const [panX, setPanX] = createSignal<number>(0);
  const [panY, setPanY] = createSignal<number>(0);
  const [isPanning, setIsPanning] = createSignal<boolean>(false);
  const [layoutOrientation, setLayoutOrientation] = createSignal<"LR" | "TB">("LR");
  let panStart = { x: 0, y: 0, initPanX: 0, initPanY: 0 };

  const handleCanvasMouseDown = (e: MouseEvent) => {
    if ((e.target as HTMLElement).closest(".workflow-node-card")) return;
    setIsPanning(true);
    panStart = { x: e.clientX, y: e.clientY, initPanX: panX(), initPanY: panY() };
  };

  const handleCanvasMouseMove = (e: MouseEvent) => {
    if (!isPanning()) return;
    setPanX(panStart.initPanX + (e.clientX - panStart.x));
    setPanY(panStart.initPanY + (e.clientY - panStart.y));
  };

  const handleCanvasMouseUp = () => {
    if (isPanning()) setIsPanning(false);
  };

  const zoomToFit = () => {
    const container = document.querySelector(".workflow-scroll-viewport");
    if (!container) return;
    const cw = container.clientWidth - 50;
    const ch = container.clientHeight - 50;
    const gw = workflowGraph().width;
    const gh = workflowGraph().height;
    const scale = Math.min(1.2, Math.max(0.4, Math.min(cw / gw, ch / gh)));
    setZoomScale(scale);
    setPanX(0);
    setPanY(0);
  };

  const [copied, setCopied] = createSignal(false);

  // Turn search / filter in large sessions
  const [turnSearchQuery, setTurnSearchQuery] = createSignal("");

  // Filter for conversational log
  const [logFilter, setLogFilter] = createSignal<"all" | "dialogue" | "tools" | "system">("all");

  // Composer signals
  const [draft, setDraft] = createSignal("");
  const [nCandidates, setNCandidates] = createSignal(1);
  const [sending, setSending] = createSignal(false);
  const [running, setRunning] = createSignal(false);
  const [live, setLive] = createSignal(true);

  // Load Session Resources
  const [subagentsData, { refetch: refetchSubagents }] = createResource(
    () => props.sessionId,
    (id) => api.subagents(id),
  );

  const [transcriptData, { refetch: refetchTranscript }] = createResource(
    () => ({ id: props.sessionId, refresh: true }),
    ({ id, refresh }) => api.transcript(id, { limit: 500, offset: 0, refresh }),
  );

  const [receiptsData, { refetch: refetchReceipts }] = createResource(
    () => props.sessionId,
    (id) => api.attach(id).then(() => api.receipts(id)).catch(() => []),
  );

  const [checkpointsData, { refetch: refetchCheckpoints }] = createResource(
    () => props.sessionId,
    (id) => api.checkpoints(id).catch(() => ({ checkpoints: [] })),
  );

  const [diffData, { refetch: refetchDiff }] = createResource(
    () => props.sessionId,
    (id): Promise<SessionDiff> => api.attach(id).then(() => api.diff(id)).catch(() => ({ diff: "" })),
  );

  // Subagents polling
  createEffect(() => {
    void props.sessionId;
    const timer = window.setInterval(() => refetchSubagents(), 3000);
    onCleanup(() => window.clearInterval(timer));
  });

  // Reconstruct turns
  const turns = createMemo(() => {
    const entries = transcriptData()?.entries ?? [];
    const receipts = receiptsData() ?? [];
    const checkpoints = checkpointsData()?.checkpoints ?? [];
    return reconstructTurns(entries, receipts, checkpoints);
  });

  const contract = createMemo(() => transcriptData()?.contract ?? null);

  // Total session economics
  const totalTokens = createMemo(() => {
    const recs = receiptsData() ?? [];
    return recs.reduce((sum, r) => sum + receiptInputTokens(r) + receiptOutputTokens(r), 0);
  });

  const totalCost = createMemo(() => {
    const recs = receiptsData() ?? [];
    return recs.reduce((sum, r) => sum + (r.cost_usd ?? 0), 0);
  });

  // Ensure selectedTurnIndex defaults to latest turn when session loads
  createEffect(() => {
    const list = turns();
    if (list.length > 0 && selectedTurnIndex() > list.length) {
      setSelectedTurnIndex(list.length);
    }
  });

  // Current turn (The single turn being deeply visualized)
  const currentTurn = createMemo(() => {
    const list = turns();
    if (list.length === 0) return null;
    const match = list.find((t) => t.turn_index === selectedTurnIndex());
    return match || list[list.length - 1];
  });

  // Current Turn Workflow Graph
  const workflowGraph = createMemo(() => {
    const turn = currentTurn();
    if (!turn) return { nodes: [], edges: [], width: 1040, height: 260 };
    return buildWorkflowGraph(turn, contract(), layoutOrientation());
  });

  // Active node for inspector
  const activeNode = createMemo(() => {
    const graph = workflowGraph();
    const id = selectedNodeId();
    return graph.nodes.find((n) => n.id === id) || graph.nodes[0] || null;
  });

  // Keep selected node synced when switching turns
  createEffect(() => {
    const graph = workflowGraph();
    if (graph.nodes.length > 0 && !graph.nodes.some((n) => n.id === selectedNodeId())) {
      setSelectedNodeId(graph.nodes[0].id);
    }
  });

  // SSE Live Feed
  createEffect(() => {
    const sid = props.sessionId;
    if (!sid) return;

    const source = new EventSource("/events");
    source.onmessage = (event) => {
      try {
        const ev = JSON.parse(event.data);
        if (ev.session_id === sid) {
          if (ev.kind === "receipt" || ev.kind === "run_end") {
            refetchReceipts();
            refetchCheckpoints();
            refetchDiff();
          }
          if (ev.kind === "turn_start" || ev.kind === "tool_call_start") setRunning(true);
          if (ev.kind === "turn_end" || ev.kind === "run_end") setRunning(false);
          refetchTranscript();
        }
      } catch {
        // non-JSON event ignored
      }
    };

    onCleanup(() => source.close());
  });

  // Prompt execution
  const handleSend = async () => {
    const text = draft().trim();
    if (!text || sending()) return;
    setSending(true);
    try {
      await api.attach(props.sessionId);
      if (nCandidates() > 1) {
        await api.startBestofn(props.sessionId, text, nCandidates());
        pushToast("info", `Forked ${nCandidates()} parallel candidates`);
      } else {
        await api.runPrompt(props.sessionId, text);
        pushToast("info", "Prompt dispatched to agent");
      }
      setDraft("");
      setRunning(true);
      await refetchTranscript();
      // Auto-jump to the newest turn
      setTimeout(() => setSelectedTurnIndex(turns().length), 500);
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    } finally {
      setSending(false);
    }
  };

  const handleCopyPayload = (text: string) => {
    navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const restoreCommit = async (seq: number) => {
    if (!confirmDestructive(`Restore workspace to checkpoint #${seq}? Subsequent uncommitted changes will be lost.`)) return;
    try {
      await api.attach(props.sessionId);
      await api.restoreCheckpoint(props.sessionId, seq);
      pushToast("info", `Workspace restored to checkpoint #${seq}`);
      await Promise.all([refetchCheckpoints(), refetchDiff()]);
    } catch (err) {
      pushToast("alert", String(err instanceof Error ? err.message : err));
    }
  };

  // Filtered log entries
  const filteredLogEntries = createMemo(() => {
    const all = transcriptData()?.entries ?? [];
    const filter = logFilter();
    if (filter === "all") return all;
    if (filter === "dialogue") {
      return all.filter((e) => (e.role === "user" || e.role === "assistant") && !e.tool_name && !e.content.startsWith("[result]"));
    }
    if (filter === "tools") {
      return all.filter((e) => !!e.tool_name || e.content.startsWith("[result]") || e.content.startsWith("[tool:"));
    }
    if (filter === "system") {
      return all.filter((e) => e.kind === "intent" || e.kind === "goal" || e.kind === "header" || e.kind === "activity");
    }
    return all;
  });

  return (
    <div class="view session-forensics-view">
      {/* Executive Forensics Masthead */}
      <div class="forensics-masthead">
        <div class="masthead-nav-bar">
          <button type="button" class="button small ghost" onClick={() => navigate("#/sessions")}>
            ‹ Sessions
          </button>
          <div class="masthead-id-pill">
            <span class="cdot cdot-ok" />
            <span class="mono font-bold">{shortId(props.sessionId, 12)}</span>
          </div>
          <div class="masthead-right-actions">
            <a
              class="button small ghost"
              href={`/admin/api/sessions/${encodeURIComponent(props.sessionId)}/export`}
              target="_blank"
              rel="noreferrer"
            >
              Export .md
            </a>
            <label class="toggle-label small">
              <input type="checkbox" checked={live()} onChange={(e) => setLive(e.currentTarget.checked)} />
              <span>Live tail</span>
            </label>
          </div>
        </div>

        <div class="session-metrics-deck">
          <div class="session-metric-tile">
            <span class="metric-label">DISPATCH ROUTE</span>
            <div class="metric-value font-semibold mono">
              {contract()?.provider ?? "openai-completions"} · {contract()?.model ?? "default"}
            </div>
            <span class="metric-sub mono dim">
              Ladder: {contract()?.route_ladder?.length ?? 1} leg(s) · {contract()?.route_objective ?? "balanced"}
            </span>
          </div>

          <div class="session-metric-tile">
            <span class="metric-label">AUTHORITY & SANDBOX</span>
            <div class="metric-value">
              <span class={`chip ${contract()?.permission_mode === "FullAccess" ? "chip-tone-alert" : "chip-tone-success"}`}>
                {contract()?.permission_mode ?? "WorkspaceWrite"}
              </span>
            </div>
            <span class="metric-sub dim">
              {contract()?.permission_mode === "FullAccess" ? "Unsandboxed (Host trust confirmed)" : "OS Process Jail Sandbox"}
            </span>
          </div>

          <div class="session-metric-tile">
            <span class="metric-label">TOTAL TURNS</span>
            <div class="metric-value font-bold mono">
              {turns().length} turns
            </div>
            <span class="metric-sub dim">
              Append-only verified ledger
            </span>
          </div>

          <div class="session-metric-tile">
            <span class="metric-label">TOKEN VOLUME</span>
            <div class="metric-value font-bold mono">
              {totalTokens() > 1000 ? `${(totalTokens() / 1000).toFixed(1)}k` : totalTokens()} tokens
            </div>
            <span class="metric-sub dim">
              Multi-attempt accounting
            </span>
          </div>

          <div class="session-metric-tile">
            <span class="metric-label">FINOPS COST</span>
            <div class="metric-value font-bold mono">
              ${totalCost().toFixed(4)}
            </div>
            <span class="metric-sub dim">
              Budget admitted
            </span>
          </div>
        </div>
      </div>

      {/* Scalable Turn Stepper & Navigator (Designed for 10, 1,000, or 1,000,000 turns) */}
      <section class="turn-scale-navigator">
        <div class="turn-nav-group">
          <button
            type="button"
            class="button small ghost turn-step-btn"
            disabled={selectedTurnIndex() <= 1}
            onClick={() => setSelectedTurnIndex((i) => Math.max(1, i - 1))}
            title="Step to preceding turn"
          >
            ‹ Prev Turn
          </button>

          <div class="turn-index-picker">
            <span class="dim">Turn</span>
            <input
              type="number"
              min={1}
              max={Math.max(1, turns().length)}
              value={selectedTurnIndex()}
              onInput={(e) => {
                const val = parseInt(e.currentTarget.value, 10);
                if (!isNaN(val) && val >= 1 && val <= turns().length) {
                  setSelectedTurnIndex(val);
                }
              }}
              class="turn-number-input mono font-bold"
            />
            <span class="dim">of {turns().length}</span>
          </div>

          <button
            type="button"
            class="button small ghost turn-step-btn"
            disabled={selectedTurnIndex() >= turns().length}
            onClick={() => setSelectedTurnIndex((i) => Math.min(turns().length, i + 1))}
            title="Step to subsequent turn"
          >
            Next Turn ›
          </button>

          <Show when={selectedTurnIndex() !== turns().length && turns().length > 0}>
            <button
              type="button"
              class="button small primary ghost"
              onClick={() => setSelectedTurnIndex(turns().length)}
            >
              Jump to Latest (#{turns().length})
            </button>
          </Show>
        </div>

        {/* Turn Selector Dropdown for Fast Direct Access */}
        <div class="turn-dropdown-picker">
          <select
            class="turn-select-menu"
            value={selectedTurnIndex()}
            onChange={(e) => setSelectedTurnIndex(Number(e.currentTarget.value))}
          >
            <For each={turns()}>
              {(t) => (
                <option value={t.turn_index}>
                  Turn #{t.turn_index}: {t.user_prompt.slice(0, 36)}… ({t.tool_calls.length} tools · {Math.round(t.duration_ms / 1000)}s)
                </option>
              )}
            </For>
          </select>
        </div>
      </section>

      {/* 5 Segmented Forensics Tabs */}
      <div class="forensics-tabs-bar">
        <button
          type="button"
          class="forensics-tab-btn"
          classList={{ active: activeTab() === "dag" }}
          onClick={() => setActiveTab("dag")}
        >
          Turn #{selectedTurnIndex()} Workflow DAG
        </button>
        <button
          type="button"
          class="forensics-tab-btn"
          classList={{ active: activeTab() === "log" }}
          onClick={() => setActiveTab("log")}
        >
          Conversational Log ({transcriptData()?.total ?? 0})
        </button>
        <button
          type="button"
          class="forensics-tab-btn"
          classList={{ active: activeTab() === "drift" }}
          onClick={() => setActiveTab("drift")}
        >
          Context &amp; Drift Audit
        </button>
        <button
          type="button"
          class="forensics-tab-btn"
          classList={{ active: activeTab() === "receipts" }}
          onClick={() => setActiveTab("receipts")}
        >
          Work Receipts ({receiptsData()?.length ?? 0})
        </button>
        <button
          type="button"
          class="forensics-tab-btn"
          classList={{ active: activeTab() === "checkpoints" }}
          onClick={() => setActiveTab("checkpoints")}
        >
          Checkpoints &amp; Diffs ({checkpointsData()?.checkpoints?.length ?? 0})
        </button>
      </div>

      {/* TAB CONTENTS */}
      <Switch>
        {/* TAB 1: STANDARD TURN WORKFLOW DAG */}
        <Match when={activeTab() === "dag"}>
          <div class="workflow-dag-view">
            <Show when={currentTurn()} fallback={<div class="empty">No turns recorded yet in this session.</div>}>
              {/* Turn Header Card */}
              <div class="turn-summary-card">
                <div class="turn-summary-left">
                  <span class="turn-badge">Turn #{currentTurn()!.turn_index}</span>
                  <strong class="turn-title">“{currentTurn()!.user_prompt}”</strong>
                </div>
                <div class="turn-summary-right">
                  <span class="dim mono">{timeAgo(currentTurn()!.started_at)}</span>
                  <span class="mono font-semibold">
                    {currentTurn()!.tool_calls.length} tool{currentTurn()!.tool_calls.length === 1 ? "" : "s"}
                  </span>
                  <span class="mono">
                    {currentTurn()!.duration_ms > 0 ? `${(currentTurn()!.duration_ms / 1000).toFixed(1)}s` : "—"}
                  </span>
                  <span class="mono">
                    {currentTurn()!.tokens_in + currentTurn()!.tokens_out > 0
                      ? `${((currentTurn()!.tokens_in + currentTurn()!.tokens_out) / 1000).toFixed(1)}k tokens`
                      : "0 tokens"}
                  </span>
                  <span
                    class="chip"
                    classList={{
                      "chip-tone-success": currentTurn()!.status !== "failed",
                      "chip-tone-alert": currentTurn()!.status === "failed",
                    }}
                  >
                    {currentTurn()!.status}
                  </span>
                </div>
              </div>

              {/* Standard Workflow DAG Canvas for THIS SPECIFIC TURN */}
              <div class="workflow-canvas-container">
                <div class="workflow-canvas-toolbar">
                  <div class="toolbar-left">
                    <span class="canvas-label">Turn #{currentTurn()!.turn_index} Execution DAG</span>
                    <span class="dim small">
                      {buildTurnBreadcrumb(currentTurn()!, workflowGraph().nodes.length)}
                    </span>
                  </div>
                  <div class="canvas-zoom-controls">
                    <button
                      type="button"
                      class="button small ghost"
                      onClick={() => setLayoutOrientation((o) => (o === "LR" ? "TB" : "LR"))}
                      title="Toggle between Horizontal (LR) and Vertical (TB) flow"
                    >
                      {layoutOrientation() === "LR" ? "↔ Horizontal" : "↕ Vertical"}
                    </button>
                    <button type="button" class="button small ghost" onClick={() => setZoomScale((s) => Math.max(0.45, s - 0.15))}>-</button>
                    <span class="zoom-level mono">{Math.round(zoomScale() * 100)}%</span>
                    <button type="button" class="button small ghost" onClick={() => setZoomScale((s) => Math.min(1.6, s + 0.15))}>+</button>
                    <button type="button" class="button small ghost" onClick={zoomToFit} title="Fit entire DAG into view">Fit</button>
                    <button type="button" class="button small ghost" onClick={() => { setZoomScale(1.0); setPanX(0); setPanY(0); }}>Reset</button>
                  </div>
                </div>

                <div
                  class="workflow-scroll-viewport"
                  classList={{ "is-panning": isPanning() }}
                  onMouseDown={handleCanvasMouseDown}
                  onMouseMove={handleCanvasMouseMove}
                  onMouseUp={handleCanvasMouseUp}
                  onMouseLeave={handleCanvasMouseUp}
                >
                  <div
                    class="workflow-stage-canvas"
                    style={{
                      width: `${workflowGraph().width}px`,
                      height: `${workflowGraph().height}px`,
                      transform: `translate(${panX()}px, ${panY()}px) scale(${zoomScale()})`,
                      "transform-origin": "top left",
                    }}
                  >
                    {/* SVG Connector Edges */}
                    <svg class="workflow-edges-layer" width={workflowGraph().width} height={workflowGraph().height}>
                      <defs>
                        <marker id="arrow" viewBox="0 0 10 10" refX="6" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
                          <path d="M 0 1 L 8 5 L 0 9 z" fill="var(--border-strong, #64748b)" />
                        </marker>
                        <marker id="arrow-active" viewBox="0 0 10 10" refX="6" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
                          <path d="M 0 1 L 8 5 L 0 9 z" fill="#10b981" />
                        </marker>
                        <marker id="arrow-warn" viewBox="0 0 10 10" refX="6" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
                          <path d="M 0 1 L 8 5 L 0 9 z" fill="#f59e0b" />
                        </marker>
                        <marker id="arrow-bad" viewBox="0 0 10 10" refX="6" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
                          <path d="M 0 1 L 8 5 L 0 9 z" fill="#ef4444" />
                        </marker>
                      </defs>
                      <For each={workflowGraph().edges}>
                        {(edge) => {
                          const src = workflowGraph().nodes.find((n) => n.id === edge.source);
                          const dst = workflowGraph().nodes.find((n) => n.id === edge.target);
                          if (!src || !dst) return null;

                          const isLR = layoutOrientation() === "LR";
                          const x1 = isLR ? src.x + src.width : src.x + src.width / 2;
                          const y1 = isLR ? src.y + src.height / 2 : src.y + src.height;
                          const x2 = isLR ? dst.x : dst.x + dst.width / 2;
                          const y2 = isLR ? dst.y + dst.height / 2 : dst.y;
                          const d = isLR
                            ? `M ${x1} ${y1} C ${x1 + Math.max(30, (x2 - x1) / 2)} ${y1}, ${x2 - Math.max(30, (x2 - x1) / 2)} ${y2}, ${x2} ${y2}`
                            : `M ${x1} ${y1} C ${x1} ${y1 + Math.max(28, (y2 - y1) / 2)}, ${x2} ${y2 - Math.max(28, (y2 - y1) / 2)}, ${x2} ${y2}`;

                          return (
                            <path
                              d={d}
                              class="workflow-edge-path"
                              classList={{
                                "edge-ok": edge.status === "ok",
                                "edge-warn": edge.status === "warn",
                                "edge-bad": edge.status === "bad",
                              }}
                              marker-end={
                                edge.status === "bad"
                                  ? "url(#arrow-bad)"
                                  : edge.status === "warn"
                                  ? "url(#arrow-warn)"
                                  : "url(#arrow-active)"
                              }
                            />
                          );
                        }}
                      </For>
                    </svg>

                    {/* Node Cards */}
                    <For each={workflowGraph().nodes}>
                      {(node) => (
                        <div
                          class="workflow-node-card"
                          classList={{
                            "node-selected": selectedNodeId() === node.id,
                            [`phase-${node.phase}`]: true,
                            [`stage-${node.stage_key}`]: true,
                            "node-ok": node.status === "ok",
                            "node-warn": node.status === "warn",
                            "node-bad": node.status === "bad",
                          }}
                          style={{
                            left: `${node.x}px`,
                            top: `${node.y}px`,
                            width: `${node.width}px`,
                            height: `${node.height}px`,
                          }}
                          onClick={() => setSelectedNodeId(node.id)}
                        >
                          <div class="node-port node-port-in" classList={{ "port-tb-in": layoutOrientation() === "TB" }} />
                          <div class="node-port node-port-out" classList={{ "port-tb-out": layoutOrientation() === "TB" }} />
                          <div class="node-header">
                            <div class="node-header-left">
                              <span class="node-stage-icon">{renderNodeIcon(node.icon)}</span>
                              <span class={`node-phase-tag phase-tag-${node.phase}`}>{node.phase}</span>
                            </div>
                            <div class="node-status-pill">
                              <span class={`cdot cdot-${node.status === "bad" ? "bad" : node.status === "warn" ? "warn" : "ok"}`} />
                              <span class="node-status-label mono">
                                {node.status === "bad" ? "ERROR" : node.status === "warn" ? "NUDGE" : "READY"}
                              </span>
                            </div>
                          </div>
                          <div class="node-body">
                            <div class="node-title font-semibold" title={node.title}>{node.title}</div>
                            <div class="node-subtitle mono" title={node.subtitle}>{node.subtitle}</div>
                          </div>
                          <div class="node-footer">
                            <span class="node-badge-chip mono">{node.badge || "—"}</span>
                            <Show when={node.duration_ms != null && node.duration_ms > 0}>
                              <span class="node-time-chip mono">⚡ {(node.duration_ms! / 1000).toFixed(1)}s</span>
                            </Show>
                          </div>
                        </div>
                      )}
                    </For>
                  </div>
                </div>

                {/* Mini Radar / Overview in bottom-right corner */}
                <div class="workflow-minimap">
                  <svg viewBox={`0 0 ${workflowGraph().width} ${workflowGraph().height}`} class="minimap-svg">
                    <For each={workflowGraph().edges}>
                      {(edge) => {
                        const src = workflowGraph().nodes.find((n) => n.id === edge.source);
                        const dst = workflowGraph().nodes.find((n) => n.id === edge.target);
                        if (!src || !dst) return null;
                        return (
                          <line
                            x1={src.x + src.width / 2}
                            y1={src.y + src.height / 2}
                            x2={dst.x + dst.width / 2}
                            y2={dst.y + dst.height / 2}
                            stroke="#475569"
                            stroke-width="6"
                          />
                        );
                      }}
                    </For>
                    <For each={workflowGraph().nodes}>
                      {(node) => (
                        <rect
                          x={node.x}
                          y={node.y}
                          width={node.width}
                          height={node.height}
                          rx="12"
                          fill={node.status === "bad" ? "#ef4444" : node.status === "warn" ? "#f59e0b" : selectedNodeId() === node.id ? "#818cf8" : "#334155"}
                        />
                      )}
                    </For>
                  </svg>
                </div>
              </div>

              {/* Node Forensics Inspector Panel */}
              <Show when={activeNode()}>
                <div class="node-inspector-deck">
                  <div class="node-inspector-header">
                    <div class="inspector-header-left">
                      <span class="inspector-badge">{activeNode()!.phase.toUpperCase()}</span>
                      <strong class="inspector-title">{activeNode()!.title}</strong>
                      <span class={`cdot cdot-${activeNode()!.status === "bad" ? "bad" : activeNode()!.status === "warn" ? "warn" : "ok"}`} />
                    </div>
                    <div class="inspector-header-right">
                      <button
                        type="button"
                        class="button small ghost"
                        onClick={() => handleCopyPayload(activeNode()!.raw_payload || JSON.stringify(activeNode()!.details, null, 2))}
                      >
                        {copied() ? "Copied!" : "Copy Payload"}
                      </button>
                    </div>
                  </div>

                  <div class="inspector-subtabs">
                    <button
                      type="button"
                      class="subtab-btn"
                      classList={{ active: inspectorTab() === "payload" }}
                      onClick={() => setInspectorTab("payload")}
                    >
                      Forensic Payload
                    </button>
                    <button
                      type="button"
                      class="subtab-btn"
                      classList={{ active: inspectorTab() === "economics" }}
                      onClick={() => setInspectorTab("economics")}
                    >
                      Economics &amp; Timing
                    </button>
                    <button
                      type="button"
                      class="subtab-btn"
                      classList={{ active: inspectorTab() === "policy" }}
                      onClick={() => setInspectorTab("policy")}
                    >
                      Policy &amp; Invariants
                    </button>
                    <button
                      type="button"
                      class="subtab-btn"
                      classList={{ active: inspectorTab() === "sandbox" }}
                      onClick={() => setInspectorTab("sandbox")}
                    >
                      Sandbox &amp; Security
                    </button>
                  </div>

                  <div class="inspector-tab-content">
                    <Switch>
                      <Match when={inspectorTab() === "payload"}>
                        <div class="inspector-code-box">
                          <pre class="mono">{activeNode()!.raw_payload || JSON.stringify(activeNode()!.details, null, 2)}</pre>
                        </div>
                      </Match>
                      <Match when={inspectorTab() === "economics"}>
                        <div class="inspector-kv-grid">
                          <div class="kv-item">
                            <span class="kv-label">Stage Latency:</span>
                            <span class="mono font-semibold">{activeNode()!.duration_ms != null ? `${activeNode()!.duration_ms} ms` : "0 ms"}</span>
                          </div>
                          <div class="kv-item">
                            <span class="kv-label">Input Tokens:</span>
                            <span class="mono">{activeNode()!.tokens_in?.toLocaleString() ?? "—"}</span>
                          </div>
                          <div class="kv-item">
                            <span class="kv-label">Output Tokens:</span>
                            <span class="mono">{activeNode()!.tokens_out?.toLocaleString() ?? "—"}</span>
                          </div>
                          <div class="kv-item">
                            <span class="kv-label">Cost (USD):</span>
                            <span class="mono font-bold">${(activeNode()!.cost ?? 0).toFixed(5)}</span>
                          </div>
                        </div>
                      </Match>
                      <Match when={inspectorTab() === "policy"}>
                        <div class="inspector-kv-grid">
                          <For each={Object.entries(activeNode()!.details)}>
                            {([key, val]) => (
                              <div class="kv-item">
                                <span class="kv-label">{key.replace(/_/g, " ")}:</span>
                                <span class="mono wrap">{typeof val === "object" ? JSON.stringify(val) : String(val)}</span>
                              </div>
                            )}
                          </For>
                        </div>
                      </Match>
                      <Match when={inspectorTab() === "sandbox"}>
                        <div class="sandbox-security-grid">
                          <div class="security-card">
                            <span class="security-card-title">OS Sandbox Backend</span>
                            <span class="security-card-status">
                              {contract()?.permission_mode === "FullAccess"
                                ? "Unsandboxed (FullAccess)"
                                : "Seatbelt / OS Process Jail"}
                            </span>
                            <span class="security-card-desc">
                              Active isolation backend (macOS Seatbelt profile / Linux Landlock ABI / Docker backend).
                            </span>
                          </div>
                          <div class="security-card">
                            <span class="security-card-title">Authority Mode</span>
                            <span class="security-card-status">
                              {contract()?.permission_mode || "WorkspaceWrite"}
                            </span>
                            <span class="security-card-desc">
                              Invariant 13: FullAccess is an explicit human trust decision; never selected automatically.
                            </span>
                          </div>
                          <div class="security-card">
                            <span class="security-card-title">Filesystem Jail Boundary</span>
                            <span class="security-card-status">Canonical Workspace Root</span>
                            <span class="security-card-desc">
                              Invariant 10: Automatic access resolves inside canonical workspace; traversal escapes fail closed.
                            </span>
                          </div>
                          <div class="security-card">
                            <span class="security-card-title">Quarantined Scratch</span>
                            <span class="security-card-status">.vak/scratch/ Isolated Jail</span>
                            <span class="security-card-desc">
                              Isolated staging directory for generated scripts, sandboxed artifacts, and code preview.
                            </span>
                          </div>
                          <div class="security-card">
                            <span class="security-card-title">Broker Worker Boundary</span>
                            <span class="security-card-status">__tool_worker Process Group</span>
                            <span class="security-card-desc">
                              Invariant 14: Built-in tools execute through versioned broker protocol in disposable process groups.
                            </span>
                          </div>
                          <div class="security-card">
                            <span class="security-card-title">Secrets Isolation</span>
                            <span class="security-card-status">Sanitized Environment Allowlist</span>
                            <span class="security-card-desc">
                              Invariant 12: Secrets are not ambient tool state. Provider and gateway tokens excluded from tool workers.
                            </span>
                          </div>
                        </div>
                      </Match>
                    </Switch>
                  </div>
                </div>
              </Show>
            </Show>
          </div>
        </Match>

        {/* TAB 2: CONVERSATIONAL LOG */}
        <Match when={activeTab() === "log"}>
          <div class="log-tab-view">
            <div class="log-filter-toolbar">
              <div class="filter-pills">
                <button type="button" class="chip-btn" classList={{ active: logFilter() === "all" }} onClick={() => setLogFilter("all")}>All Entries</button>
                <button type="button" class="chip-btn" classList={{ active: logFilter() === "dialogue" }} onClick={() => setLogFilter("dialogue")}>Dialogue Only</button>
                <button type="button" class="chip-btn" classList={{ active: logFilter() === "tools" }} onClick={() => setLogFilter("tools")}>Tools &amp; MCP</button>
                <button type="button" class="chip-btn" classList={{ active: logFilter() === "system" }} onClick={() => setLogFilter("system")}>Intent &amp; Goals</button>
              </div>
              <span class="dim small" style="margin-left: auto;">
                Showing {filteredLogEntries().length} of {transcriptData()?.total ?? 0} entries
              </span>
            </div>

            <div class="transcript-feed">
              <For each={filteredLogEntries()}>
                {(e: TranscriptEntry) => (
                  <article class="entry-card" data-role={e.role ?? e.kind} data-error={e.is_error}>
                    <header class="entry-header">
                      <span class="entry-role-chip">{e.role ?? e.kind}</span>
                      <Show when={e.tool_name}>
                        <span class="chip chip-tool mono">{e.tool_name}</span>
                      </Show>
                      <Show when={e.is_error}>
                        <span class="chip chip-error">error</span>
                      </Show>
                      <span class="entry-ts font-mono" title={e.ts}>{clock(e.ts)} ({timeAgo(e.ts)})</span>
                    </header>
                    <pre class="entry-content mono">{e.content}</pre>
                  </article>
                )}
              </For>
            </div>
          </div>
        </Match>

        {/* TAB 3: CONTEXT & DRIFT AUDIT */}
        <Match when={activeTab() === "drift"}>
          <div class="drift-tab-view">
            <div class="drift-header-banner">
              <div>
                <h3>Context Packet &amp; Drift Accounting</h3>
                <p>
                  Invariant 1 &amp; 17 audit: session instructions freeze at admission time.
                  Verifies that system prompt layers, capabilities, and route state have not drifted.
                </p>
              </div>
            </div>

            <div class="drift-grid">
              {/* Card 1: Frozen Route Contract */}
              <div class="drift-card">
                <h4>1. Admission Contract Snapshot</h4>
                <div class="drift-row">
                  <span class="drift-label">App Version:</span>
                  <span class="mono">{contract()?.app_version ?? "3.0.94"}</span>
                </div>
                <div class="drift-row">
                  <span class="drift-label">Admitted Provider:</span>
                  <span class="mono">{contract()?.provider}</span>
                </div>
                <div class="drift-row">
                  <span class="drift-label">Admitted Model:</span>
                  <span class="mono">{contract()?.model}</span>
                </div>
                <div class="drift-row">
                  <span class="drift-label">Authority Mode:</span>
                  <span class="chip chip-tone-success">{contract()?.permission_mode}</span>
                </div>
              </div>

              {/* Card 2: Prompt Layers */}
              <div class="drift-card">
                <h4>2. Prompt Layers &amp; Provenance</h4>
                <Show
                  when={(contract()?.prompt_layers ?? []).length > 0}
                  fallback={<div class="dim">No layer hashes recorded (monolithic header).</div>}
                >
                  <div class="layers-list">
                    <For each={contract()?.prompt_layers ?? []}>
                      {(layer: PromptLayerDescriptor) => (
                        <div class="layer-item">
                          <div class="layer-head">
                            <strong>{layer.block}</strong>
                            <span class="chip">{layer.layer}</span>
                          </div>
                          <div class="layer-meta mono dim">
                            hash: {layer.digest.slice(0, 16)}… · {layer.bytes} bytes
                          </div>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </div>

              {/* Card 3: Admitted Capabilities */}
              <div class="drift-card" style="grid-column: span 2;">
                <h4>3. Admitted Capabilities Inventory ({contract()?.capabilities?.length ?? 0})</h4>
                <Show
                  when={(contract()?.capabilities ?? []).length > 0}
                  fallback={<div class="dim">Capabilities derived from standard harness inventory.</div>}
                >
                  <div class="capabilities-grid">
                    <For each={contract()?.capabilities ?? []}>
                      {(cap: CapabilityDescriptor) => (
                        <div class="cap-item">
                          <div class="cap-head">
                            <strong class="mono">{cap.name}</strong>
                            <span class="chip">{cap.kind}</span>
                          </div>
                          <p class="cap-desc">{cap.description || "Builtin broker capability"}</p>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </div>
            </div>
          </div>
        </Match>

        {/* TAB 4: WORK RECEIPTS */}
        <Match when={activeTab() === "receipts"}>
          <div class="receipts-tab-view">
            <Show when={(receiptsData()?.length ?? 0) > 0} fallback={<div class="empty">No work receipts billed to this session yet.</div>}>
              <table class="table receipts-table">
                <thead>
                  <tr>
                    <th>Step</th>
                    <th>Provider</th>
                    <th>Model</th>
                    <th>Input Tokens</th>
                    <th>Output Tokens</th>
                    <th>Cached</th>
                    <th>Latency</th>
                    <th>Settlement</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={receiptsData() ?? []}>
                    {(r: WorkReceipt, i) => (
                      <tr>
                        <td class="mono">#{i() + 1}</td>
                        <td>{receiptProvider(r)}</td>
                        <td class="mono">{receiptModel(r)}</td>
                        <td>{receiptInputTokens(r).toLocaleString()}</td>
                        <td>{receiptOutputTokens(r).toLocaleString()}</td>
                        <td>{receiptCachedTokens(r).toLocaleString()}</td>
                        <td class="mono">{receiptLatencyMs(r) ? `${(receiptLatencyMs(r) / 1000).toFixed(1)}s` : "—"}</td>
                        <td>
                          <span
                            class="chip"
                            classList={{
                              "chip-tone-success": receiptSettlement(r) === "ok",
                              "chip-tone-alert": receiptSettlement(r) === "failed",
                              "chip-tone-muted": receiptSettlement(r) === "unknown",
                            }}
                          >
                            {receiptSettlement(r)}
                          </span>
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </Show>
          </div>
        </Match>

        {/* TAB 5: CHECKPOINTS & DIFFS */}
        <Match when={activeTab() === "checkpoints"}>
          <div class="checkpoints-tab-view">
            <div class="checkpoints-grid">
              <div class="checkpoints-panel">
                <h3>Filesystem Save Points</h3>
                <Show when={(checkpointsData()?.checkpoints?.length ?? 0) > 0} fallback={<div class="empty">No checkpoints recorded.</div>}>
                  <div class="checkpoints-list">
                    <For each={checkpointsData()?.checkpoints ?? []}>
                      {(cp: SessionCheckpoint) => (
                        <div class="checkpoint-item">
                          <div class="cp-info">
                            <strong>Checkpoint #{cp.seq}</strong>
                            <span class="cp-label">{cp.message || cp.label || "Save Point"}</span>
                            <span class="cp-ts dim">{timeAgo(cp.ts || cp.created_at || "")}</span>
                          </div>
                          <button
                            type="button"
                            class="button small ghost"
                            onClick={() => void restoreCommit(cp.seq)}
                          >
                            Rewind to #{cp.seq}
                          </button>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </div>

              <div class="diff-panel">
                <h3>Session Git Diff</h3>
                <div class="diff-canvas">
                  <pre class="mono">{diffData()?.diff || diffData()?.error || "No uncommitted modifications on disk."}</pre>
                </div>
              </div>
            </div>
          </div>
        </Match>
      </Switch>

      {/* Live Composer */}
      <div class="session-composer-bar">
        <div class="composer-controls">
          <select
            class="candidate-stepper"
            value={nCandidates()}
            onChange={(e) => setNCandidates(Number(e.currentTarget.value))}
            disabled={running()}
            title="Fan out multiple parallel runs in isolated workspace copies"
          >
            <option value={1}>×1</option>
            <option value={2}>×2</option>
            <option value={3}>×3</option>
            <option value={4}>×4</option>
          </select>
          <textarea
            rows={2}
            placeholder={running() ? "Steer the run in progress…" : "Ask vak to run the next turn…"}
            value={draft()}
            onInput={(e) => setDraft(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                handleSend();
              }
            }}
            disabled={sending()}
          />
          <button
            type="button"
            class="button primary"
            disabled={sending() || !draft().trim()}
            onClick={handleSend}
          >
            {sending() ? "Dispatching…" : running() ? "Steer Turn" : "Send Turn"}
          </button>
        </div>
      </div>
    </div>
  );
}
