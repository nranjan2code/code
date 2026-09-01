export type StoryAsset = {
  id: string;
  source: string;
  role: string;
  alt: string;
};

export const assets: StoryAsset[] = [
  {id: 'overview', source: 'vak-architecture-overview.png', role: 'opening system map', alt: 'VAK architecture overview'},
  {id: 'request', source: 'vak-request-end-to-end.png', role: 'request journey', alt: 'One VAK request end to end'},
  {id: 'gateway', source: 'vak-gateway-channels-trust.png', role: 'trust boundary', alt: 'VAK gateway channels and trust'},
  {id: 'config', source: 'vak-config-routing-contract.png', role: 'configuration contract', alt: 'VAK configuration routing and runtime contract'},
  {id: 'core', source: 'vak-core-runtime-facade.png', role: 'runtime facade', alt: 'VAK core runtime facade'},
  {id: 'permission', source: 'vak-permission-tool-security.png', role: 'security boundary', alt: 'VAK permission and tool security boundary'},
  {id: 'agent', source: 'vak-inside-one-agent-turn.png', role: 'agent loop', alt: 'Inside one VAK agent turn'},
  {id: 'flow', source: 'vak-normal-vs-managed-flow.png', role: 'durability choice', alt: 'Normal flow versus managed flow'},
  {id: 'flows', source: 'vak-flows-planner-evaluation.png', role: 'orchestration', alt: 'VAK flows planner and evaluation'},
  {id: 'ledger', source: 'vak-session-ledger.png', role: 'evidence record', alt: 'VAK session ledger'},
  {id: 'memory', source: 'vak-memory-learning-automation.png', role: 'memory and automation', alt: 'VAK memory learning and automation'},
  {id: 'delivery', source: 'vak-semantic-delivery.png', role: 'result projection', alt: 'VAK semantic delivery'},
  {id: 'desktop', source: 'vak-desktop-admin-extensions.png', role: 'surfaces and extensions', alt: 'VAK desktop admin and extensions'},
  {id: 'doctor', source: 'vak-doctor.png', role: 'diagnosis and repair', alt: 'VAK doctor diagnosis repair and recheck'},
  {id: 'ops', source: 'vak-operations-center.png', role: 'operational proof', alt: 'VAK operations center'},
];

export const assetPath = (id: string) => {
  const asset = assets.find((candidate) => candidate.id === id);
  if (!asset) throw new Error(`Unknown story asset: ${id}`);
  return `assets/${asset.source}`;
};
