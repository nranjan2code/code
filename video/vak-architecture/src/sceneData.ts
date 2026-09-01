export type Scene = {
  title: string;
  kicker: string;
  body: string;
  accent: string;
  nodes: string[];
};

export const scenes: Scene[] = [
  {
    kicker: 'THE QUESTION',
    title: 'What happens after you ask VAK to do something?',
    body: 'One request becomes a governed journey through trust, planning, execution, evidence, and delivery.',
    accent: '#25b5a5',
    nodes: ['request', 'trust', 'agent', 'tools', 'evidence'],
  },
  {
    kicker: 'ONE SECURE ENTRY PATH',
    title: 'Every channel enters the same system',
    body: 'Desktop, CLI, web, Telegram, Discord, and Slack normalize into one request model.',
    accent: '#4c86e8',
    nodes: ['desktop', 'cli', 'web', 'telegram', 'discord', 'slack'],
  },
  {
    kicker: 'THE CORE',
    title: 'vak-core is the workspace-scoped runtime facade',
    body: 'It composes configuration, sessions, permissions, agents, tools, providers, persistence, and delivery.',
    accent: '#ff8a4c',
    nodes: ['config', 'session', 'permission', 'agent', 'provider', 'delivery'],
  },
  {
    kicker: 'THE AGENT LOOP',
    title: 'The model proposes; the system governs',
    body: 'Context is projected from the ledger, the model streams a response, and every effectful tool call crosses authorization.',
    accent: '#8b6be8',
    nodes: ['project', 'stream', 'authorize', 'broker', 'sandbox', 'result'],
  },
  {
    kicker: 'NORMAL OR MANAGED',
    title: 'Simple work stays fast. Complex work becomes durable.',
    body: 'Direct turns use the normal loop. Managed Flow adds contracts, work items, evidence, checkpoints, and verification.',
    accent: '#d79b21',
    nodes: ['direct', 'contract', 'items', 'owners', 'verify'],
  },
  {
    kicker: 'THE LEDGER',
    title: 'Everything important becomes reconstructable',
    body: 'Append-only JSONL stores the frozen contract, messages, receipts, work, goals, activity, and compaction facts.',
    accent: '#3da36a',
    nodes: ['header', 'messages', 'receipts', 'work', 'projection'],
  },
  {
    kicker: 'THE RESULT',
    title: 'One semantic result, many safe surfaces',
    body: 'vak-delivery projects meaning into UI timelines, Markdown, Telegram HTML, SSE, and retryable outbox jobs.',
    accent: '#e95b55',
    nodes: ['timeline', 'ast', 'capabilities', 'renderer', 'adapter'],
  },
  {
    kicker: 'THE OPERATING MODEL',
    title: 'VAK is a personal AI operating system',
    body: 'It remembers, plans, acts, verifies, and stays observable without giving the model authority over the system.',
    accent: '#25b5a5',
    nodes: ['remember', 'plan', 'act', 'verify', 'deliver'],
  },
];
