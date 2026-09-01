export type StoryBeat = {
  id: string;
  eyebrow: string;
  title: string;
  line: string;
  asset?: string;
  tint: string;
};

export const beats: StoryBeat[] = [
  {id: 'hook', eyebrow: 'A REQUEST ARRIVES', title: 'The dangerous part is not the answer.', line: 'It is everything that happens before the answer can be trusted.', tint: '#df795f'},
  {id: 'map', eyebrow: 'THE WHOLE MACHINE', title: 'Behind one command is an operating system.', line: 'Fifteen crates. One governed path. A record you can inspect after the work is done.', asset: 'overview', tint: '#df795f'},
  {id: 'journey', eyebrow: 'THE JOURNEY', title: 'Follow the request, not the crate names.', line: 'A message enters, earns an identity, crosses trust, runs through policy, and returns as meaning.', asset: 'request', tint: '#7c9fc9'},
  {id: 'entry', eyebrow: '01 / FIND THE DOOR', title: 'One request. Many surfaces.', line: 'CLI, desktop, HTTP, Telegram, Discord, Slack — the entry changes. The contract does not.', asset: 'gateway', tint: '#7c9fc9'},
  {id: 'config', eyebrow: '02 / RESOLVE THE CONTRACT', title: 'Configuration decides what this run is.', line: 'Global defaults, project truth, scoped pins, secrets, route provenance — provider and model travel together.', asset: 'config', tint: '#d4a85d'},
  {id: 'core', eyebrow: '03 / ENTER THE CORE', title: 'vak-core is the boundary between intent and effect.', line: 'It assembles the workspace, freezes the session, and coordinates the same runtime across every surface.', asset: 'core', tint: '#ee9278'},
  {id: 'trust', eyebrow: '04 / EARN THE RIGHT', title: 'Before action, VAK asks: should this happen?', line: 'Identity, workspace, policy, capability, permission. The model never gets to skip the gate.', asset: 'permission', tint: '#d4a85d'},
  {id: 'agency', eyebrow: '03 / LET THE AGENT WORK', title: 'The model proposes. The system stays in charge.', line: 'Context comes from the ledger. Tools cross the broker. Effects run inside explicit boundaries.', asset: 'agent', tint: '#73a982'},
  {id: 'durability', eyebrow: '05 / CHOOSE THE DEPTH', title: 'A quick turn — or a managed mission.', line: 'Normal flow keeps the conversation fast. Managed Flow adds work items, checkpoints, evidence, and verification.', asset: 'flow', tint: '#ee9278'},
  {id: 'orchestration', eyebrow: '06 / MAKE PLANS EXECUTABLE', title: 'Plans are candidates until they survive the rules.', line: 'Static flows stay deterministic. Dynamic plans validate, repair, replan within bounds, and fail closed.', asset: 'flows', tint: '#d4a85d'},
  {id: 'proof', eyebrow: '07 / LEAVE RECEIPTS', title: 'Nothing important disappears into a chat bubble.', line: 'The append-only ledger makes the turn reconstructable: what was asked, what ran, what changed, and why.', asset: 'ledger', tint: '#7c9fc9'},
  {id: 'memory', eyebrow: '08 / REMEMBER CAREFULLY', title: 'Memory can grow without becoming authority.', line: 'Search and reflection propose context. User-approved memory persists. Policy still governs every write.', asset: 'memory', tint: '#73a982'},
  {id: 'result', eyebrow: '09 / RETURN MEANING', title: 'One semantic result. Every surface gets its own safe view.', line: 'Timeline, Markdown, channel message, SSE, retryable delivery — meaning stays consistent as presentation adapts.', asset: 'delivery', tint: '#df795f'},
  {id: 'surfaces', eyebrow: '10 / MEET THE USER', title: 'The UI is a window, not the authority.', line: 'Desktop, admin, tray, CLI, and extensions expose the runtime without bypassing its boundaries.', asset: 'desktop', tint: '#7c9fc9'},
  {id: 'doctor', eyebrow: '11 / WHEN REALITY BREAKS', title: 'Doctor diagnoses. Repair only does what is known.', line: 'Facts stay visible, mechanical fixes are explicit, and unresolved problems remain with the operator.', asset: 'doctor', tint: '#d4a85d'},
  {id: 'after', eyebrow: '12 / OPERATE WITH EVIDENCE', title: 'Operations is not a dashboard of guesses.', line: 'Live state, receipts, incidents, delivery, and service-manager truth form the operational picture.', asset: 'ops', tint: '#73a982'},
  {id: 'handoff', eyebrow: 'THE HANDOFF', title: 'The answer is only the last frame.', line: 'The real product is a system that can explain its work, constrain its power, and recover from failure.', tint: '#df795f'},
  {id: 'end', eyebrow: 'VAK', title: 'Power with a paper trail.', line: 'A coding agent you can inspect, constrain, and extend.', tint: '#df795f'},
];
