"""The subsystems: what the system does, cut across crates.

A crate says where code lives; a subsystem says what it is for. Doctor, for
example, is `crates/vak/src/doctor.rs`, `vak-core`'s health report, the
`/doctor` route, `vak doctor` and the admin Diagnostics screen.

Every unit of the tree belongs to exactly one subsystem, and the harness
checks it (blueprint/AGENTS.md, "Needs review"):
- a module: every source file under `crates/*/src` (Rust and TypeScript,
  tests left out);
- a part outside the crates: the public site, each plugin package, each
  entry of `scripts/`, `docker/` and `blueprint/`;
- every HTTP route the server registers, by the longest prefix that claims it;
- every `vak` subcommand.

A module pattern is a path relative to the repository root, where `*`
matches within a directory and `**` across them. The most specific pattern
wins: an exact path over a glob, then the longer literal prefix, so a
feature can claim one file out of a crate another subsystem holds. Two
subsystems claiming one unit equally is reported, as is a pattern, prefix,
verb or doc that matches nothing.
"""

# (id, name, role, modules, routes, cli verbs, design docs)
SUBSYSTEMS = [
    # ---- Surfaces ----------------------------------------------------------
    ("client", "Workspace client",
     "The SolidJS app shared by the desktop shell and /app: chat, sidebar, settings, the host seam",
     ["crates/vak-client-ui/src/**"],
     ["/app", "/host", "/stream", "/workspaces", "/fs", "/favicon.svg", "/version", "/commands"],
     [], ["48-web-client.md", "63-agent-first-conversations.md", "70-calm-agent-experience-implementation.md",
          "75-visual-refresh.md", "71-agent-character-system.md"]),
    ("desktop", "Desktop shell",
     "Tauri window, tray, single instance, PTY panes, native dialogs",
     ["crates/vak-desktop/src/**", "crates/vak-client-ui/src/components/TerminalPane.tsx"],
     ["/pty"], [], []),
    ("terminal", "Rich terminal",
     "`vak term`, a ratatui client of a running server",
     ["crates/vak-terminal/src/**"], [], ["term"], ["55-rich-terminal-surface.md"]),
    ("admin", "Admin console",
     "The /admin SPA and its data plane: sessions, capabilities, security, prompts, projects",
     ["crates/vak-admin-ui/src/**", "crates/vak-server/src/admin.rs", "crates/vak-server/src/admin_ui.rs",
      "crates/vak-server/src/embedded_ui.rs", "crates/vak-server/src/client_ui.rs"],
     ["/admin"], ["open"], ["33-admin-console.md", "58-admin-ia-and-ui-goal.md"]),
    ("site", "Public site and brand",
     "The website served at /, its build, and the brand media",
     ["crates/vak-server/site", "crates/vak-server/src/site.rs", "video", "scripts/video"],
     [], [], []),
    ("cli", "Command line",
     "The `vak` binary's parser, dispatch and shared output",
     ["crates/vak/src/main.rs", "crates/vak/src/cli.rs", "crates/vak/src/format.rs",
      "crates/vak/src/question_prompt.rs"],
     [], ["exec"], []),

    # ---- The turn ----------------------------------------------------------
    ("turn", "Turn admission",
     "Admitting a request: Agent and lifecycle checks, the live route, trace keys, the turn's knobs",
     ["crates/vak-core/src/lib.rs", "crates/vak-core/src/admission.rs", "crates/vak-core/src/run_services.rs", "crates/vak-core/src/routing.rs",
      "crates/vak-core/src/file_mentions.rs", "crates/vak-core/src/workspaces.rs",
      "crates/vak-core/src/measure.rs"],
     ["/sessions/{id}/run", "/sessions/{id}/cancel", "/sessions/{id}/steering", "/sessions/{id}/pause",
      "/sessions/{id}/resume", "/sessions/{id}/control-state", "/sessions/{id}/turns", "/sessions/{id}/side"],
     [], ["13-server.md", "52-outcome-directed-runtime.md", "85-turn-graph.md", "99-turns-that-do-not-poison.md"]),
    ("loop", "Agent loop",
     "Steps, retries, the breaker, tool waves, stop policy, spend gate, steering",
     ["crates/vak-agent/src/**"],
     [], [], ["03-agent-loop.md", "15-reliability.md"]),
    ("workers", "Workers and delegation",
     "The task tool, child runs, worker questions, the workers tool, write leases",
     ["crates/vak-agent/src/task.rs", "crates/vak-agent/src/questions.rs", "crates/vak-agent/src/workers_tool.rs",
      "crates/vak-agent/src/write_lease.rs", "crates/vak-client-ui/src/components/WorkersPanel.tsx"],
     ["/sessions/{id}/workers", "/sessions/{id}/subagents", "/sessions/{id}/questions", "/admin/api/questions"],
     [], ["84-worker-questions-and-control.md", "62-universal-delegation-experience.md"]),
    ("goal", "Goals and managed work",
     "Goal mode, audited completion, work contracts and their review",
     ["crates/vak-agent/src/goal.rs", "crates/vak-client-ui/src/components/WorkModal.tsx",
      "crates/vak-client-ui/src/components/RunControls.tsx"],
     ["/sessions/{id}/work", "/sessions/{id}/plan-change", "/sessions/{id}/outcome-review"],
     [], ["42-managed-work-contracts.md"]),
    ("context", "Context engine",
     "Capacity profiles, the working-set planner, prompt assembly, compaction",
     ["crates/vak-context/src/**", "crates/vak-core/src/indexed_history.rs"],
     ["/sessions/{id}/context", "/sessions/{id}/compact"],
     [], ["68-context-engine.md"]),
    ("prompt", "Prompt layers",
     "The seed prompt and its layered, per-turn resolution",
     ["crates/vak-core/src/prompts.rs", "crates/vak/src/prompts.rs"],
     ["/config/prompts"], ["prompts"],
     ["07-prompt.md", "45-prompt-layers.md", "83-prompt-system.md"]),
    ("intent", "Intent kernel",
     "Signals, strands, engagements and the narrowing Limits lattice",
     ["crates/vak-intent/src/**", "crates/vak-core/src/intent.rs", "crates/vak/src/intent.rs"],
     ["/intent", "/config/intent"], ["intent"], ["47-commitment-kernel.md"]),
    ("control", "Control plane and judges",
     "Decision points, the correction ladder, receipts to verdicts, the plan check",
     ["crates/vak-control/src/**", "crates/vak-judge/src/**", "crates/vak-tools/src/plan.rs"],
     ["/config/control", "/sessions/{id}/decisions", "/runs/{id}/decisions"],
     [], ["88-control-plane.md", "86-model-plans-runtime-checks.md"]),
    ("commitments", "Commitments",
     "Durable commitments, the satisfaction lattice and the portfolio scheduler",
     ["crates/vak-commit/src/**", "crates/vak-core/src/commitments.rs", "crates/vak-core/src/tools_commitments.rs",
      "crates/vak-client-ui/src/components/CommitmentsPanel.tsx"],
     ["/commitments"], ["commit"], []),

    # ---- Models and tools --------------------------------------------------
    ("llm", "Model providers and routing",
     "Provider wires, model discovery, the route ladder, capacity probes, work receipts",
     ["crates/vak-llm/src/**", "crates/vak-core/src/discovery.rs"],
     ["/providers", "/config/route", "/config/key", "/sessions/{id}/receipts"],
     [], ["01-llm.md", "31-network-resilience.md", "89-constrained-tool-arguments.md"]),
    ("tools", "Built-in tools and the worker",
     "File, shell and web tools, the argument reader, the broker worker protocol, egress",
     ["crates/vak-tools/src/**", "crates/vak-server/src/bin/vak-tool-worker.rs",
      "crates/vak-core/src/reach.rs"],
     [], [], ["04-tools.md", "50-call-and-evidence-contract.md", "51-retired-tools.md"]),
    ("recall", "Recall and history",
     "The recall tool, turn records, search across conversations",
     ["crates/vak-tools/src/recall.rs", "crates/vak-core/src/elsewhere.rs"],
     [], [], []),
    ("capabilities", "Capability registry",
     "The level-triggered registry, epochs, the tool surface, skills, custom commands",
     ["crates/vak-core/src/capability/**", "crates/vak-core/src/skills.rs",
      "crates/vak-core/src/custom_commands.rs", "crates/vak-core/src/packages.rs"],
     ["/skills", "/admin/api/capabilities"], ["skills"],
     ["41-capability-registry.md", "95-one-capability-index.md"]),
    ("abilities", "Self-evolving abilities",
     "Skill programs an Agent writes, tests and admits, and their review",
     ["crates/vak-tools/src/capability.rs", "crates/vak-core/src/skill_programs.rs",
      "crates/vak-core/src/ability_review.rs", "crates/vak-core/src/ability_sheet.rs",
      "crates/vak-client-ui/src/components/AbilitySheet.tsx"],
     ["/skills/abilities"], [], ["93-self-evolving-capabilities.md", "43-governed-self-evolution.md"]),
    ("mcp", "MCP integrations",
     "The lazy MCP client pool behind one meta-tool, each server behind its egress proxy",
     ["crates/vak-mcp/src/**", "crates/vak-client-ui/src/mcpNetwork.ts"],
     ["/config/mcp", "/config/integrations"], [], ["09-extensibility.md"]),
    ("plugins", "Plugins and packages",
     "Plugin packages, stores, catalog signatures, and the shipped packages",
     ["crates/vak-plugin/src/**", "crates/vak/src/plugins.rs", "packages/*"],
     ["/plugins"], ["plugins"], ["39-plugin-ecosystem.md"]),
    ("hooks", "Hooks",
     "Lifecycle hooks around tool use, stop and session start",
     ["crates/vak-hooks/src/**"], ["/config/hooks"], [], []),
    ("flows", "Flows and planner",
     "Static flow DAGs and the dynamic planner",
     ["crates/vak-flow/src/**"], ["/flows"], ["flow", "plan"], ["10-flows.md", "11-planner.md"]),

    # ---- Safety ------------------------------------------------------------
    ("permission", "Permissions and provenance",
     "Modes and rules over declared reach, approvals, the provenance gate",
     ["crates/vak-permission/src/**", "crates/vak-control/src/provenance.rs", "crates/vak-core/src/trust.rs"],
     ["/sessions/{id}/approvals", "/config/permissions", "/config/mode", "/admin/api/approvals"],
     [], ["08-permissions.md", "24-agent-security.md"]),
    ("sandbox", "Sandbox and execution",
     "Seatbelt, Landlock, Docker, execution roots, candidate promotion",
     ["crates/vak-sandbox/src/**", "crates/vak-core/src/sandbox_docker.rs", "crates/vak-tools/src/sandbox.rs",
      "crates/vak-tools/src/landlock.rs", "crates/vak-tools/src/sandbox_events.rs", "crates/vak-tools/src/drafts.rs",
      "crates/vak-server/src/sandbox_records.rs", "crates/vak-server/src/sandbox_output.rs",
      "crates/vak-client-ui/src/components/WorkbenchPanel.tsx"],
     ["/sandbox", "/sessions/{id}/sandbox", "/sessions/{id}/keep", "/sessions/{id}/discard", "/sessions/{id}/diff"],
     [], ["25-docker-sandbox.md", "54-task-environments-and-promotion.md"]),
    ("security", "Security events",
     "Security records and the security center",
     ["crates/vak-core/src/security_events.rs", "crates/vak-admin-ui/src/SecurityCenter.tsx"],
     ["/admin/api/security"], [], []),
    ("identity", "Owner identity and sign-in",
     "Passkeys, recovery, browser sessions, the gateway token, rate limits",
     ["crates/vak-server/src/auth_identity.rs", "crates/vak-server/src/rate_limit.rs",
      "crates/vak-core/src/gateway_token.rs", "crates/vak-client-ui/src/headlessAuth.ts",
      "crates/vak-client-ui/src/ownerRecovery.ts", "crates/vak-admin-ui/src/headlessAuth.ts",
      "crates/vak-admin-ui/src/ownerRecovery.ts"],
     ["/auth", "/config/owner"], [], ["78-headless-identity.md"]),
    ("grants", "Grants and sharing",
     "Who may reach an artifact or a conversation; guests",
     ["crates/vak-core/src/grants.rs", "crates/vak-session/src/grants.rs"],
     ["/shared", "/conversations/{id}/guests"], ["grant", "revoke"], []),

    # ---- Data --------------------------------------------------------------
    ("ledger", "Ledgers and records",
     "Sealed append-only ledgers, record chains, objects, fencing, documents, turns",
     ["crates/vak-session/src/**"],
     ["/sessions", "/conversations/{id}/gone"],
     ["sessions", "export"], ["02-sessions.md", "73-data-architecture-and-lifecycle.md"]),
    ("storage", "Storage substrate",
     "Keys, refs, encrypted objects, records and segments",
     ["crates/vak-storage/src/**", "crates/vak-core/src/keys.rs"],
     ["/data/keys"], [], []),
    ("runs", "Runs, effects and cursors",
     "Run records, at-most-once outbound effects, stream cursors",
     ["crates/vak-session/src/runs.rs", "crates/vak-session/src/effects.rs", "crates/vak-session/src/effects/projection.rs", "crates/vak-session/src/cursors.rs",
      "crates/vak-server/src/effects.rs", "crates/vak/src/effects.rs", "crates/vak/src/runs.rs",
      "crates/vak-admin-ui/src/Runs.tsx", "crates/vak-admin-ui/src/Effects.tsx"],
     ["/runs", "/effects"], ["runs", "effects"], []),
    ("catalog", "Catalog and search",
     "The SQLite catalog: nodes, lineage, full-text search",
     ["crates/vak-catalog/src/**", "crates/vak-core/src/catalog.rs", "crates/vak-server/src/catalog_api.rs",
      "crates/vak-admin-ui/src/Lineage.tsx", "crates/vak-client-ui/src/components/SearchModal.tsx"],
     ["/catalog", "/search", "/lineage", "/nodes"], [], []),
    ("state", "State registry and baseline",
     "Every durable path, the data baseline, integrity checks, `vak data`",
     ["crates/vak-core/src/state.rs", "crates/vak-core/src/baseline.rs", "crates/vak-core/src/integrity.rs",
      "crates/vak/src/data.rs", "crates/vak-admin-ui/src/Data.tsx"],
     ["/data/status", "/data/integrity", "/data/usage"], ["data"], []),
    ("lifecycle", "Lifecycle and erasure",
     "Retention rules, holds, the reconciler, the trash, crypto-shred erasure",
     ["crates/vak-lifecycle/src/**", "crates/vak-core/src/lifecycle.rs", "crates/vak-core/src/erasure.rs",
      "crates/vak-core/src/trash.rs", "crates/vak-session/src/conversation_state.rs",
      "crates/vak-admin-ui/src/ConversationLifecycle.tsx", "crates/vak-client-ui/src/components/YourData.tsx",
      "crates/vak-client-ui/src/components/EraseAgentSheet.tsx",
      "crates/vak-client-ui/src/components/EraseConversationSheet.tsx"],
     ["/data/lifecycle", "/data/rules", "/data/holds", "/data/erasure", "/conversations/{id}/erasure",
      "/conversations/{id}/hold", "/conversations/{id}/lifecycle", "/sessions/archived", "/sessions/{id}/archive",
      "/sessions/{id}/restore", "/library/trash"],
     [], ["74-lifecycle-and-data-administration.md"]),
    ("backup", "Backup and restore",
     "Encrypted backups and the restore that applies every erasure again",
     ["crates/vak-core/src/backup.rs", "crates/vak/src/backup.rs", "crates/vak-admin-ui/src/BackupModels.tsx"],
     ["/data/backups"], ["backup"], []),
    ("sync", "Sync",
     "A folder remote two machines take turns writing",
     ["crates/vak-core/src/sync.rs", "crates/vak/src/sync.rs"], ["/sync"], ["sync"], []),
    ("checkpoints", "Checkpoints and worktrees",
     "Workspace checkpoints, restore, git worktrees and PRs",
     ["crates/vak-core/src/checkpoints.rs", "crates/vak-core/src/worktree.rs",
      "crates/vak-client-ui/src/components/CheckpointsModal.tsx", "crates/vak-client-ui/src/components/PrPanel.tsx",
      "crates/vak-client-ui/src/components/DiffPane.tsx", "crates/vak-client-ui/src/diff.ts"],
     ["/sessions/{id}/checkpoints", "/sessions/{id}/pr"], ["checkpoints"], ["14-checkpoints.md"]),
    ("config", "Configuration and secrets",
     "Layered TOML, the credential store, paths, spaces and scopes",
     ["crates/vak-config/src/**", "crates/vak-server/src/restart.rs"],
     ["/config"], ["config"], ["05-config.md", "44-shared-config.md"]),
    ("telemetry", "Telemetry and logs",
     "Content-free structured telemetry and service logs",
     ["crates/vak-telemetry/src/**", "crates/vak-server/src/telemetry.rs"],
     ["/telemetry", "/runs/{id}/spans"], [], []),

    # ---- Presentation -------------------------------------------------------
    ("cards", "Cards and presentation",
     "Card forms and tools, packs, the closed primitives, projection to clients",
     ["crates/vak-presentation/src/**", "crates/vak-core/src/presentation_tools.rs",
      "crates/vak-core/src/presentation_store.rs", "crates/vak-agent/src/cards.rs", "crates/vak-agent/src/fences.rs",
      "crates/vak-server/src/projection.rs", "crates/vak-server/src/client_events.rs",
      "crates/vak-server/src/card_actions.rs", "crates/vak-session/src/card_actions.rs",
      "crates/vak-server/src/maps.rs", "crates/vak-client-ui/src/components/cards/**",
      "crates/vak-client-ui/src/components/presentation/**",
      "crates/vak-client-ui/src/components/PresentationRenderer.tsx",
      "crates/vak-client-ui/src/presentationHydration.ts", "crates/vak-client-ui/src/structured.ts",
      "crates/vak-client-ui/src/turnPairing.ts"],
     ["/presentations", "/sessions/{id}/presentation", "/sessions/{id}/card-actions", "/sessions/{id}/interface",
      "/sessions/{id}/results", "/maps", "/config/maps"],
     [], ["30-render-architecture.md", "57-adaptive-presentation-runtime.md", "67-presentation-renderer-guide.md",
          "87-composed-answers.md", "91-presentation-capabilities.md", "92-card-forms.md",
          "97-cards-end-the-answer.md", "98-answers-beyond-one-card.md", "96-answer-projection-gaps.md"]),
    ("delivery", "Delivery",
     "The semantic output contract, Markdown fallback and channel markup",
     ["crates/vak-delivery/src/**", "crates/vak-server/src/delivery.rs"],
     [], [], ["30-output-engineering.md"]),
    ("canvas", "Canvas and previews",
     "The artifact canvas, its viewers, live previews and the preview origin",
     ["crates/vak-server/src/canvas.rs", "crates/vak-server/src/preview.rs",
      "crates/vak-client-ui/src/components/canvas/**", "crates/vak-client-ui/src/components/ArtifactCanvas.tsx",
      "crates/vak-client-ui/src/components/LivePreviewPanel.tsx", "crates/vak-client-ui/src/canvas*.ts",
      "crates/vak-client-ui/src/preview*.ts", "crates/vak-client-ui/src/artifactPreview.ts"],
     ["/previews", "/sessions/{id}/canvas", "/sessions/{id}/launch"], [], ["66-immersive-artifact-canvas.md"]),
    ("library", "Library and file state",
     "Versioned artifacts, review, sharing, and what a file is now",
     ["crates/vak-core/src/artifacts.rs", "crates/vak-core/src/file_state.rs", "crates/vak-core/src/files.rs",
      "crates/vak-server/src/library.rs", "crates/vak-tools/src/artifact.rs",
      "crates/vak-client-ui/src/components/LibraryPage.tsx", "crates/vak-client-ui/src/components/fileState.tsx",
      "crates/vak-client-ui/src/components/SharedArtifact.tsx", "crates/vak-client-ui/src/candidateVersions.ts",
      "crates/vak-client-ui/src/draftVersions.ts", "crates/vak-client-ui/src/components/PageKeeper.tsx",
      "crates/vak-client-ui/src/components/PageView.tsx"],
     ["/library"], [], ["82-library.md", "90-file-state.md"]),

    # ---- Apps ---------------------------------------------------------------
    ("office", "Office and PDF documents",
     "The Open XML and PDF engines, doc_read, office_apply, Review and shared rooms",
     ["crates/vak-ooxml/src/**", "crates/vak-pdf/src/**", "crates/vak-tools/src/doc_read.rs",
      "crates/vak-tools/src/office_apply.rs", "crates/vak-tools/src/office_pdf.rs",
      "crates/vak-server/src/office_workspace.rs", "crates/vak/src/office.rs",
      "crates/vak-client-ui/src/components/Office*.tsx", "crates/vak-client-ui/src/office*.ts"],
     ["/sessions/{id}/office-workspaces", "/fs/office"], ["office"],
     ["72-openxml-documents.md", "77-pdf-documents.md"]),
    ("mail", "Mail and calendar",
     "Accounts, OAuth, the vault, provider adapters, reviewed sends and changes",
     ["crates/vak-mail-calendar/src/**", "crates/vak-core/src/mail_calendar.rs",
      "crates/vak-tools/src/mail_calendar.rs", "crates/vak-server/src/mail_calendar.rs",
      "crates/vak-client-ui/src/mailCalendar*", "crates/vak-client-ui/src/components/MailCalendar*.tsx",
      "crates/vak-client-ui/src/components/SyntheticMailCalendarDemoControl.tsx",
      "crates/vak-client-ui/src/components/canvas/*MailCalendar*.tsx"],
     ["/mail-calendar"], [], ["80-mail-and-calendar.md"]),
    ("intake", "Intake and feeds",
     "Sources, connectors parsed in the worker, detection that labels",
     ["crates/vak-intake/src/**", "crates/vak-core/src/intake.rs", "crates/vak-core/src/intake_alerts.rs",
      "crates/vak-server/src/intake.rs", "crates/vak-client-ui/src/components/SourcesPanel.tsx"],
     ["/intake"], [], ["76-intake-and-knowledge.md", "51-feed-system.md"]),
    ("social", "Social connectors",
     "LinkedIn, Reddit, X and YouTube connectors and their previews",
     ["crates/vak-server/src/social.rs", "crates/vak-server/src/social/**", "crates/vak-core/src/social/**"],
     ["/social"], [], []),
    ("voice", "Voice",
     "Speech providers, the live voice session, narration",
     ["crates/vak-voice/src/**", "crates/vak-server/src/voice.rs", "crates/vak-client-ui/src/voice-*.ts",
      "crates/vak-client-ui/src/components/VoiceControl.tsx", "crates/vak-client-ui/src/sound*.ts"],
     ["/voice"], [], ["38-voice-personality.md", "49-live-voice.md"]),
    ("memory", "Memory and learning",
     "Notes, recall of the owner's profile, the learning loop and skill proposals",
     ["crates/vak-core/src/memory.rs", "crates/vak-core/src/learning.rs", "crates/vak/src/memory.rs"],
     ["/memory", "/skills/proposals", "/skills/review"], ["memory"], ["23-memory.md", "26-learning.md"]),
    ("automations", "Automations and triggers",
     "The one trigger model, claims, schedules, routines and their runs",
     ["crates/vak-core/src/triggers.rs", "crates/vak-core/src/scheduled.rs", "crates/vak-core/src/tools_automations.rs",
      "crates/vak-server/src/automations.rs", "crates/vak/src/triggers.rs", "crates/vak-admin-ui/src/Automations.tsx",
      "crates/vak-client-ui/src/components/Automation*.tsx", "crates/vak-client-ui/src/components/SchedulePicker.tsx",
      "crates/vak-client-ui/src/automationWords.ts"],
     ["/triggers"], ["triggers"], ["94-scheduled-work-you-can-see.md"]),
    ("inbox", "Inbox and digest",
     "What needs the owner, and the periodic digest",
     ["crates/vak-core/src/inbox.rs", "crates/vak-core/src/digest.rs", "crates/vak-server/src/inbox.rs",
      "crates/vak/src/inbox.rs", "crates/vak/src/digest.rs", "crates/vak-admin-ui/src/Inbox.tsx",
      "crates/vak-client-ui/src/components/InboxPage.tsx", "crates/vak-client-ui/src/components/DigestCard.tsx",
      "crates/vak-client-ui/src/components/NeedsYou.tsx"],
     ["/inbox", "/digest"], ["inbox", "digest"], ["29-personal-os.md"]),
    ("agents", "Agents",
     "Agent definitions, lifecycle, the Agent network, per-Agent chats",
     ["crates/vak-core/src/agent_definitions.rs", "crates/vak-core/src/agent_network.rs",
      "crates/vak-server/src/agents.rs", "crates/vak-server/src/agent_chats.rs", "crates/vak/src/agents_cli.rs",
      "crates/vak-client-ui/src/components/Agent*.tsx", "crates/vak-client-ui/src/agent*.ts",
      "crates/vak-client-ui/src/components/RevokeAgentSheet.tsx", "crates/vak-client-ui/src/characterTier.ts"],
     ["/agents", "/agent-network", "/config/agents"], ["agents"], ["64-agent-owned-platform.md"]),
    ("coworking", "Shared conversations",
     "Guests in a conversation: invitations, presence, delegated approvals",
     ["crates/vak-server/src/coworking.rs", "crates/vak-client-ui/src/components/CoworkingShare.tsx",
      "crates/vak-client-ui/src/components/SharedConversation.tsx"],
     ["/sessions/{id}/coworking"], [], ["69-shared-conversation-coworking.md"]),

    # ---- Service and operations ---------------------------------------------
    ("server", "HTTP service",
     "The router, auth layers, the event bus, session handles and the Core pool",
     ["crates/vak-server/src/lib.rs", "crates/vak-server/src/web.rs", "crates/vak-server/src/events.rs",
      "crates/vak-server/src/stream.rs", "crates/vak-server/src/core_pool.rs", "crates/vak-server/src/turn_marks.rs",
      "crates/vak-session/src/turn_marks.rs", "crates/vak-server/src/traced_rows_tests.rs",
      "crates/vak-core/src/transcript_md.rs"],
     ["/traffic", "/admin/api/traffic", "/admin/api/events", "/admin/api/sessions", "/admin/api/config",
      "/admin/api/bestofn", "/admin/api/projects", "/config/server", "/sessions/{id}/events",
      "/sessions/{id}/attach", "/sessions/{id}/transcript", "/sessions/{id}/snapshot", "/sessions/{id}/bestofn"],
     ["serve"], ["13-server.md"]),
    ("gateway", "Gateway and channels",
     "Chat surfaces, bots, the allowlist, bindings, forwarded approvals, heartbeat",
     ["crates/vak-server/src/gateway.rs", "crates/vak-server/src/channels.rs", "crates/vak-server/src/surfaces/**",
      "crates/vak-server/src/heartbeat.rs"],
     ["/gateway", "/admin/api/gateway"], ["telegram", "discord", "slack"],
     ["22-gateway.md", "34-channel-onboarding.md"]),
    ("bus", "Distributed bus",
     "NATS JetStream fabric with causal lineage and encrypted envelopes",
     ["crates/vak-bus/src/**", "crates/vak-server/src/bus.rs"], ["/config/bus"], [], ["53-distributed-bus.md"]),
    ("operations", "Operations",
     "Service control, the Operations Center, incidents and action receipts",
     ["crates/vak-ops/src/**", "crates/vak-server/src/operations.rs", "crates/vak-server/src/service_control.rs",
      "crates/vak-admin-ui/src/OperationsCenter.tsx", "crates/vak-client-ui/src/components/OperationsPanel.tsx"],
     ["/ops"], [], ["28-operations.md"]),
    ("doctor", "Doctor and health",
     "The health report: `vak doctor`, `--repair`, /health, /doctor, diagnostics screens",
     ["crates/vak-core/src/health.rs", "crates/vak/src/doctor.rs", "crates/vak-admin-ui/src/Diagnostics.tsx"],
     ["/health", "/doctor"], ["doctor"], []),
    ("finops", "Cost and budgets",
     "The cost ledger, budget admission, alerts",
     ["crates/vak-core/src/finops.rs", "crates/vak-core/src/finops/projection.rs", "crates/vak-core/src/spend_services.rs", "crates/vak-core/src/model_call_budget.rs", "crates/vak-agent/src/spend.rs", "crates/vak-server/src/budget_recovery.rs",
      "crates/vak-client-ui/src/components/BudgetBanner.tsx"],
     ["/finops", "/finops/reservations", "/finops/reservations/{id}/reconcile"], [], []),
    ("onboarding", "Setup and onboarding",
     "First run, workspace trust, seeding, the setup wizard",
     ["crates/vak-core/src/onboarding.rs", "crates/vak-core/src/seed.rs", "crates/vak/src/setup.rs",
      "crates/vak-client-ui/src/components/SetupBanner.tsx", "crates/vak-client-ui/src/components/ConnectSheet.tsx",
      "crates/vak-client-ui/src/components/WorkspaceGate.tsx"],
     ["/onboarding"], ["setup"], ["36-first-run-onboarding.md", "46-stabilization-install-and-onboarding.md"]),
    ("install", "Install and update",
     "Self install, the update feed, manifests, transactional upgrades",
     ["crates/vak/src/install/**", "crates/vak/src/update_check.rs", "crates/vak-core/src/install.rs",
      "scripts/install.sh", "scripts/hosting", "scripts/upgrade-gate.sh", "scripts/merge-feeds.py",
      "scripts/integrity-manifest.py", "docker"],
     [], ["self"], ["37-distribution.md"]),
    ("eval", "Evaluation",
     "The deterministic suite, live mode, context and banned-token gates, replays",
     ["crates/vak-eval/src/**", "crates/vak/src/eval_poison.rs", "crates/vak/src/eval_ledgers.rs",
      "crates/vak-admin-ui/src/Measurements.tsx", "scripts/capability_coverage*", "scripts/measure_tool_form.py",
      "scripts/prompt_scenarios.py", "scripts/scenarios", "scripts/poison_replay.sh", "scripts/harness_500.py",
      "scripts/test_harness_500.py", "scripts/harness_selftest.py", "scripts/compound_regression.py"],
     ["/eval"], ["eval"], ["12-evals.md", "40-harness-engineering.md"]),
    ("engineering", "Build, gate and release",
     "Builds, the gate, version and record checks, the release, platform and smoke checks, this blueprint",
     ["scripts/build.sh", "scripts/build-amazonlinux-arm64.sh", "scripts/bump-version.sh", "scripts/gate.sh",
      "scripts/release.sh", "scripts/version.sh", "scripts/.version-reset", "scripts/verify-macos-release.sh",
      "scripts/check-*.sh", "scripts/check_*.py", "scripts/vak.sh", "scripts/brand",
      "scripts/*-check.sh", "scripts/linux-stack.sh", "scripts/landlock_smoke.sh", "scripts/ui_bundle_check.rs",
      "scripts/ui-acceptance-check.sh", "scripts/server_smoke.py", "scripts/telegram_smoke.sh",
      "blueprint", "scripts/mock_*.py", "scripts/fake_mcp_server.py", "scripts/fault_proxy.py", "scripts/chaos_driver.py",
      "scripts/resource_watch.py", "scripts/mail-soak", "scripts/sync-lab", "scripts/test-mail-calendar-mock-pack.sh"],
     [], [], ["32-release-engineering.md"]),
]

# The bands the views group subsystems in, in order.
GROUPS = [
    ("Surfaces", ["client", "desktop", "terminal", "admin", "site", "cli"]),
    ("The turn", ["turn", "loop", "workers", "goal", "context", "prompt", "intent", "control", "commitments"]),
    ("Models and tools", ["llm", "tools", "recall", "capabilities", "abilities", "mcp", "plugins", "hooks", "flows"]),
    ("Safety", ["permission", "sandbox", "security", "identity", "grants"]),
    ("Data", ["ledger", "storage", "runs", "catalog", "state", "lifecycle", "backup", "sync", "checkpoints",
              "config", "telemetry"]),
    ("Presentation", ["cards", "delivery", "canvas", "library"]),
    ("Apps", ["office", "mail", "intake", "social", "voice", "memory", "automations", "inbox", "agents", "coworking"]),
    ("Service and operations", ["server", "gateway", "bus", "operations", "doctor", "finops", "onboarding",
                                "install", "eval", "engineering"]),
]

# What counts as a unit outside the crates' sources: each is claimed whole.
OUTSIDE = ["crates/vak-server/site", "packages/*", "scripts/*", "docker", "video", "blueprint"]
SOURCE_EXT = (".rs", ".ts", ".tsx", ".mts", ".mjs")


def _regex(pattern):
    import re
    out = ""
    i = 0
    while i < len(pattern):
        if pattern.startswith("**", i):
            out += ".*"
            i += 2
        elif pattern[i] == "*":
            out += "[^/]*"
            i += 1
        else:
            out += re.escape(pattern[i])
            i += 1
    return re.compile(out + "$")


def _specificity(pattern):
    wild = pattern.find("*")
    literal = len(pattern.replace("*", ""))
    return (1, len(pattern), literal) if wild < 0 else (0, wild, literal)


def units(ms):
    """Every unit the subsystems must cover: (path, lines or None)."""
    import fnmatch
    import os
    files = ms.run("git", "ls-files", "--cached", "--others", "--exclude-standard").split("\n")
    out = {}
    for f in files:
        if not f or not os.path.exists(os.path.join(ms.ROOT, f)):
            continue
        parts = f.split("/")
        if (len(parts) > 3 and parts[0] == "crates" and parts[2] == "src" and f.endswith(SOURCE_EXT)
                and ".test." not in f and "node_modules" not in parts):
            out[f] = ms.lines_of([os.path.join(ms.ROOT, f)])
            continue
        for root in OUTSIDE:
            depth = root.count("/") + 1
            head = "/".join(parts[:depth])
            if fnmatch.fnmatchcase(head, root) and "__pycache__" not in parts:
                out.setdefault(head, None)
                break
    return out


def build(ms, crate_layer):
    """The subsystems with what each holds, and every coverage problem."""
    import os
    review = []
    by_id = {s[0]: s for s in SUBSYSTEMS}
    grouped = [i for _, ids in GROUPS for i in ids]
    review += [f"subsystem {i} is in no group (subsystems.GROUPS)" for i in by_id if i not in grouped]
    review += [f"group names unknown subsystem {i}" for i in grouped if i not in by_id]
    review += [f"subsystem {i} is in two groups" for i in set(grouped) if grouped.count(i) > 1]

    unit_lines = units(ms)
    owner, used = {}, set()
    patterns = [(sid, p, _regex(p), _specificity(p)) for sid, *_ in SUBSYSTEMS for p in by_id[sid][3]]
    for u in unit_lines:
        hits = [(spec, sid, p) for sid, p, rx, spec in patterns if rx.match(u)]
        if not hits:
            review.append(f"module {u} has no subsystem")
            continue
        hits.sort(reverse=True)
        best = hits[0][0]
        tied = sorted({sid for spec, sid, _ in hits if spec == best})
        if len(tied) > 1:
            review.append(f"module {u} is claimed equally by {', '.join(tied)}")
        owner[u] = hits[0][1]
        used.add((hits[0][1], hits[0][2]))
    for sid, p, rx, _ in patterns:
        if not any(rx.match(u) for u in unit_lines):
            review.append(f"subsystem {sid}: pattern {p} matches nothing")

    route_owner = {}
    prefixes = [(sid, r) for sid, *_ in SUBSYSTEMS for r in by_id[sid][4]]
    all_routes = ms.routes()
    for route in all_routes:
        hits = [(len(r), sid) for sid, r in prefixes
                if route == r or (route.startswith(r) and route[len(r)] in "/.") or (r.endswith("/") and route.startswith(r))]
        if not hits:
            review.append(f"route {route} has no subsystem")
            continue
        hits.sort(reverse=True)
        tied = sorted({sid for n, sid in hits if n == hits[0][0]})
        if len(tied) > 1:
            review.append(f"route {route} is claimed equally by {', '.join(tied)}")
        route_owner[route] = hits[0][1]
    for sid, r in prefixes:
        if not any(route == r or route.startswith(r) for route in all_routes):
            review.append(f"subsystem {sid}: route prefix {r} matches nothing")

    verbs = ms.cli_verbs()
    claimed = [(v, sid) for sid, *_ in SUBSYSTEMS for v in by_id[sid][5]]
    for v in verbs:
        n = [sid for c, sid in claimed if c == v]
        if not n:
            review.append(f"vak {v} has no subsystem")
        elif len(n) > 1:
            review.append(f"vak {v} is claimed by {', '.join(n)}")
    review += [f"subsystem {sid}: vak {v} is not a subcommand" for v, sid in claimed if v not in verbs]

    design = os.path.join(ms.ROOT, "docs/design")
    docs_cited = set()
    for sid, *_ in SUBSYSTEMS:
        for d in by_id[sid][6]:
            docs_cited.add(d)
            if not os.path.exists(os.path.join(design, d)):
                review.append(f"subsystem {sid}: doc {d} does not exist")

    out = []
    for group, ids in GROUPS:
        for sid in ids:
            if sid not in by_id:
                continue
            _, name, role, _, _, cli, docs = by_id[sid]
            mine = sorted(u for u, o in owner.items() if o == sid)
            homes = []
            for u in mine:
                parts = u.split("/")
                home = parts[1] if parts[0] == "crates" and len(parts) > 2 and parts[2] == "src" else (
                    "site" if u.startswith("crates/vak-server/site") else parts[0])
                if home not in homes:
                    homes.append(home)
            lines = sum(unit_lines[u] or 0 for u in mine)
            largest = max(((unit_lines[u] or 0, u) for u in mine), default=(0, ""))
            out.append({
                "id": sid, "name": name, "group": group, "role": role,
                "homes": sorted(homes, key=lambda h: -sum(unit_lines[u] or 0 for u in mine if f"/{h}/" in f"/{u}/")),
                "layers": sorted({crate_layer.get(h, "") for h in homes if crate_layer.get(h)}),
                "modules": len([u for u in mine if unit_lines[u] is not None]),
                "parts": [u for u in mine if unit_lines[u] is None],
                "lines": lines, "largest": list(largest),
                "routes": sorted(r for r, o in route_owner.items() if o == sid),
                "cli": cli, "docs": docs, "units": mine,
            })
    uncited = []
    for f in sorted(os.listdir(design)):
        if f.endswith(".md") and f != "INDEX.md" and f not in docs_cited:
            uncited.append(f)
    coverage = {"units": len(unit_lines), "claimed": len(owner), "routes": len(all_routes),
                "routes_claimed": len(route_owner), "verbs": len(verbs),
                "verbs_claimed": len([v for v in verbs if any(c == v for c, _ in claimed)]),
                "docs_uncited": uncited}
    return out, coverage, review, owner, route_owner

# What lies outside the repository, and which subsystems reach it. Declared
# by hand, since the code reaches these through the network, the OS or a
# person; each named subsystem must exist.
EXTERNAL = [
    ("owner", "You", "The owner, at the desktop app, a browser, a terminal or a phone",
     ["client", "desktop", "terminal", "admin", "cli"]),
    ("chat", "Chat platforms", "Telegram, Discord and Slack", ["gateway"]),
    ("models", "Model APIs and local models", "Hosted providers and local model servers", ["llm", "voice"]),
    ("mailproviders", "Mail and calendar providers", "Google, Microsoft and Apple accounts", ["mail"]),
    ("web", "The web", "Pages, feeds and social sites a tool or connector fetches", ["tools", "intake", "social"]),
    ("mcpservers", "MCP servers", "Integration servers the owner configured", ["mcp"]),
    ("keychain", "Keychain", "The OS secret service, or the encrypted-file fallback", ["config"]),
    ("ossandbox", "OS sandbox and Docker", "Seatbelt, Landlock and seccomp, and the optional Docker backend", ["sandbox"]),
    ("services", "launchd and systemd", "The service managers that start and restart Vak", ["operations", "install"]),
    ("nats", "NATS", "The optional distributed bus", ["bus"]),
]

# How work moves through the system: each flow is a sequence of steps
# (from, to, what moves). Every step must ride a measured connection
# (connections.py), in either direction, or the harness reports it, so a
# drawn flow cannot outlive the code it describes.
FLOWS = [
    ("message", "A message becomes an answer", [
        ("owner", "client", "type a message"),
        ("client", "server", "POST the message"),
        ("server", "turn", "run the turn"),
        ("turn", "intent", "read the request"),
        ("turn", "capabilities", "bind the tool surface"),
        ("turn", "runs", "open the run"),
        ("turn", "loop", "start the loop"),
        ("loop", "context", "plan the working set"),
        ("loop", "llm", "stream the model"),
        ("llm", "models", "call the provider"),
        ("loop", "permission", "authorize each call"),
        ("loop", "tools", "run the calls"),
        ("tools", "sandbox", "in a sandboxed worker"),
        ("loop", "ledger", "record results"),
        ("loop", "control", "check the plan"),
        ("loop", "cards", "record cards"),
        ("cards", "server", "project what a person sees"),
        ("server", "client", "stream events"),
        ("client", "owner", "the answer, card by card"),
    ]),
    ("channel", "A chat message from a phone", [
        ("chat", "gateway", "a message arrives"),
        ("gateway", "permission", "check the allowlist"),
        ("gateway", "turn", "admit as the bound Agent"),
        ("turn", "loop", "run the loop"),
        ("loop", "cards", "record cards"),
        ("cards", "delivery", "render for the channel"),
        ("delivery", "gateway", "channel markup"),
        ("gateway", "runs", "send once as an effect"),
        ("gateway", "chat", "the reply"),
    ]),
    ("automation", "A scheduled automation runs", [
        ("automations", "runs", "claim the slot"),
        ("automations", "turn", "run the turn"),
        ("turn", "loop", "run the loop"),
        ("loop", "ledger", "record the work"),
        ("automations", "inbox", "report how it ended"),
    ]),
    ("office", "An Office edit goes through Review", [
        ("loop", "tools", "call office_apply"),
        ("tools", "office", "edit in the worker"),
        ("office", "sandbox", "write a draft"),
        ("office", "library", "record a version"),
        ("client", "library", "the owner reviews"),
        ("library", "sandbox", "promote, with undo"),
    ]),
    ("mail", "Mail is sent after review", [
        ("client", "mail", "approve the candidate"),
        ("mail", "runs", "send once as an effect"),
        ("mail", "mailproviders", "the provider sends it"),
    ]),
    ("delegate", "A worker is delegated", [
        ("loop", "workers", "call task"),
        ("workers", "turn", "run a child turn"),
        ("workers", "ledger", "keep the child's ledger"),
    ]),
    ("recall", "An older turn is recalled", [
        ("loop", "recall", "call recall"),
        ("recall", "catalog", "search the catalog"),
        ("catalog", "ledger", "open the turn"),
    ]),
    ("intake", "A source is read", [
        ("automations", "intake", "poll a source"),
        ("intake", "web", "fetch the source"),
        ("intake", "tools", "parse in the worker"),
        ("intake", "catalog", "index what was accepted"),
        ("intake", "inbox", "alert the owner"),
    ]),
    ("erasure", "Data leaves by rule", [
        ("lifecycle", "ledger", "find what is due"),
        ("lifecycle", "grants", "end its shares"),
        ("lifecycle", "catalog", "remove what was derived"),
        ("lifecycle", "storage", "shred the scope key"),
    ]),
    ("sync", "Two machines take turns", [
        ("sync", "ledger", "push and pull records"),
        ("sync", "storage", "blobs and the writer epoch"),
    ]),
    ("doctor", "The doctor checks the install", [
        ("owner", "cli", "run vak doctor"),
        ("cli", "doctor", "collect the report"),
        ("admin", "doctor", "the Diagnostics screen"),
        ("doctor", "config", "read the configuration"),
        ("doctor", "llm", "reach the providers"),
        ("doctor", "state", "check the data"),
    ]),
]


_COMPILED = []


def owner_of_path(path):
    """The subsystem a path belongs to by the module patterns, existing or not."""
    if not _COMPILED:
        _COMPILED.extend((p.split("*")[0], _regex(p), _specificity(p), s[0]) for s in SUBSYSTEMS for p in s[3])
    parts = path.split("/")
    heads = [path] + ["/".join(parts[:n]) for n in range(1, len(parts))]
    best = None
    for prefix, rx, spec, sid in _COMPILED:
        if path.startswith(prefix) and (best is None or spec > best[0]) and any(rx.match(h) for h in heads):
            best = (spec, sid)
    return best[1] if best else None


def _commits(ms):
    """(hash, date, subject, files) for every measured commit, newest first. Commits never change,
    so they are cached in the git dir (local, never committed) and only new
    ones are read."""
    import json
    import os
    git_dir = ms.run("git", "rev-parse", "--absolute-git-dir").strip()
    path = os.path.join(git_dir, "vak-blueprint-history.json")
    try:
        cache = json.load(open(path, encoding="utf-8"))
    except (OSError, ValueError):
        cache = {}
    wanted = ms.run("git", "rev-list", "HEAD", "--", ".", *ms.SELF).split()
    missing = [h for h in wanted if h not in cache]
    if missing:
        log = ms.run("git", "log", "--no-walk=unsorted", "--format=@@%H%x1f%ad%x1f%s", "--date=short", "--name-only", *missing)
        for chunk in log.split("@@")[1:]:
            head, _, body = chunk.partition("\n")
            h, day, subject = head.split("\x1f", 2)
            cache[h] = [day, subject, [f for f in body.split("\n") if f.strip()]]
        try:
            with open(path, "w", encoding="utf-8") as fh:
                json.dump(cache, fh)
        except OSError:
            pass
    return [(h[:9], *cache[h]) for h in wanted if h in cache]


def history(ms, last_date, days):
    """Per subsystem: commits, commits whose message names a fix, and commits
    in the `days` before the last measured commit; and those recent commits
    with the subsystems each touched, newest first."""
    from datetime import date, timedelta
    from evidence import FIX_WORDS as fix
    since = (date.fromisoformat(last_date) - timedelta(days=days)).isoformat() if last_date else ""
    cache, out, recent = {}, {}, []
    for h, day, subject, files in _commits(ms):
        touched = set()
        for f in files:
            if f not in cache:
                cache[f] = owner_of_path(f)
            if cache[f]:
                touched.add(cache[f])
        for sid in touched:
            row = out.setdefault(sid, {"commits": 0, "fixes": 0, "recent": 0})
            row["commits"] += 1
            row["fixes"] += bool(fix.search(subject))
            row["recent"] += day >= since
        if day >= since:
            recent.append({"hash": h, "date": day, "subject": subject, "subsystems": sorted(touched)})
    return out, since, recent


def check_flows(edges):
    """Review lines for flows that name an unknown subsystem or ride no
    measured or declared connection, and for outside parts that name an
    unknown subsystem."""
    ids = {s[0] for s in SUBSYSTEMS}
    linked = {frozenset((e["from"], e["to"])) for e in edges}
    review = []
    for xid, _, _, reach in EXTERNAL:
        for sid in reach:
            if sid not in ids:
                review.append(f"outside {xid}: unknown subsystem {sid}")
            linked.add(frozenset((xid, sid)))
    ids |= {x[0] for x in EXTERNAL}
    for fid, _, steps in FLOWS:
        for a, b, what in steps:
            for x in (a, b):
                if x not in ids:
                    review.append(f"flow {fid}: unknown subsystem {x}")
            if a in ids and b in ids and frozenset((a, b)) not in linked:
                review.append(f"flow {fid}: {a} → {b} ({what}) rides no measured connection")
    return review
