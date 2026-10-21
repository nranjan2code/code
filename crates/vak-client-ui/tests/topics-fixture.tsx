import { render } from "solid-js/web";
import { createSignal, Show } from "solid-js";
import Sidebar from "../src/components/Sidebar";
import WorkspaceHeader from "../src/components/WorkspaceHeader";
import TopicTurnControl from "../src/components/TopicTurnControl";
import { setActiveAgent, setActiveId, setBackend, setSessions, setSidebarOpen, setNarrowViewport, sidebarOpen } from "../src/store";
import type { SessionTopics } from "../src/api";

const agent = { id: "fixture-agent", name: "Sample agent", revision: 1, lifecycle: "active" as const, character: "mira" as const, personality: "", behaviour: "", responsibilities: "", instructions: "", animation: "subtle" as const, voice: "default" };
let snapshot: SessionTopics = {
  topics: [
    { id: "topic-one", title: "First sample topic", archived: false, pinned: true, created_by: "fixture-person", updated_by: "fixture-person" },
    { id: "topic-two", title: "Second sample topic", archived: false, pinned: false, created_by: "fixture-model", updated_by: "fixture-model" },
    { id: "topic-old", title: "Archived sample topic", archived: true, pinned: false },
  ],
  placements: [
    { turn_id: "11111111-1111-7111-8111-111111111111", outcome: "Compare the available choices", topic_ids: ["topic-one"], source: "model", actor: "fixture-model", trace: "fixture-trace-1" },
    { turn_id: "22222222-2222-7222-8222-222222222222", outcome: "Prepare a follow-up", topic_ids: ["topic-one", "topic-two"], source: "person", actor: "fixture-person", trace: "fixture-trace-2" },
  ],
  automatic: true,
  revision: 4,
};

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
window.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const path = new URL(String(input), window.location.href).pathname;
  if (path === "/agents") return json({ agents: [agent] });
  if (path === "/agents/fixture-agent/topics") return json({ topics: snapshot.topics.map((topic) => ({ ...topic, session_id: "fixture-session", placements: snapshot.placements })), next: null });
  if (path === "/sessions/fixture-session/topics" && init?.method === "POST") {
    const body = JSON.parse(String(init.body)) as Record<string, unknown>;
    if (body.revision !== snapshot.revision) return json({ error: "stale revision" }, 409);
    let createdTopicId: string | undefined;
    if (body.action === "create") { createdTopicId = `topic-${Date.now()}`; snapshot.topics.push({ id: createdTopicId, title: String(body.title), archived: false, pinned: false }); }
    if (body.action === "rename") { const topic = snapshot.topics.find((item) => item.id === body.topic_id); if (topic) topic.title = String(body.title); }
    if (body.action === "pin") { const topic = snapshot.topics.find((item) => item.id === body.topic_id); if (topic) topic.pinned = Boolean(body.pinned); }
    if (body.action === "archive" || body.action === "reopen") { const topic = snapshot.topics.find((item) => item.id === body.topic_id); if (topic) topic.archived = body.action === "archive"; }
    if (body.action === "automatic") snapshot.automatic = Boolean(body.automatic);
    if (body.action === "assign") {
      if (body.whole_turn) snapshot.placements = snapshot.placements.filter((item) => item.turn_id !== body.turn_id);
      const placement = snapshot.placements.find((item) => item.turn_id === body.turn_id && item.outcome === body.outcome);
      if (placement) { placement.topic_ids = body.topic_ids as string[]; placement.source = "person"; placement.whole_turn = Boolean(body.whole_turn); }
      else snapshot.placements.push({ turn_id: String(body.turn_id), outcome: String(body.outcome), topic_ids: body.topic_ids as string[], whole_turn: Boolean(body.whole_turn), source: "person", actor: "fixture-person", trace: "fixture-trace" });
    }
    snapshot.revision++;
    return json({ ...snapshot, ...(createdTopicId ? { created_topic_id: createdTopicId } : {}) });
  }
  if (path === "/sessions/fixture-session/topics") return json(snapshot);
  if (path === "/sessions/fixture-session/snapshot") {
    const around = new URL(String(input), window.location.href).searchParams.get("around");
    const turnId = around ?? "22222222-2222-7222-8222-222222222222";
    const cursor = around ? "focused-revision-1000" : "latest-revision";
    return json({
      session_id: "fixture-session", cursor, revision: cursor, running: false,
      page: { older: "fixture-before", newer: "fixture-after", end: cursor, total_turns: 1000, first_turn: around ? 1000 : 960, request: around ?? undefined },
      transcript: { messages: [{ role: "user", content: [{ type: "text", text: `Sample request ${turnId}` }] }], entries: [{ entry_id: "fixture-user-entry" }], set_aside: [] },
      presentation: { schema_version: 2, session_id: "fixture-session", cursor, items: [], diagnostics: [], answers: [] },
    });
  }
  const match = /^\/sessions\/fixture-session\/topics\/([^/]+)$/.exec(path);
  if (match) { const topic = snapshot.topics.find((item) => item.id === decodeURIComponent(match[1])); return topic ? json({ topic, placements: snapshot.placements.filter((placement) => placement.topic_ids.includes(topic.id)), automatic: snapshot.automatic, revision: snapshot.revision }) : json({ error: "missing" }, 404); }
  if (path === "/onboarding") return json({ core_ready: true, provider: { state: "ready" } });
  if (path === "/traffic") return json({ state: "normal", active: 0, queued: 0, retry_after_secs: null, observed_routes: 0, scope: "fixture" });
  if (path === "/inbox/unread_count") return json({ count: 0 });
  return json({ error: `Unmapped fixture request: ${path}` }, 404);
}) as typeof fetch;

setBackend({ ready: true, cwd: "/sample/workspace", recent_workspaces: [] });
setActiveId("fixture-session");
setActiveAgent({ id: agent.id, name: agent.name, revision: agent.revision, character: agent.character, animation: agent.animation, voice: agent.voice });
setSessions([{ session_id: "fixture-session", cwd: "/sample/workspace", agent: { id: agent.id, name: agent.name, revision: 1, character: agent.character }, running: false, title: "Sample conversation" }]);
setSidebarOpen(true);
window.history.replaceState({}, "", `${window.location.pathname}${window.location.search}`);
const [fixtureConflictStatus, setFixtureConflictStatus] = createSignal("");
const simulateConflict = () => {
  snapshot.revision++;
  const placement = snapshot.placements.find((item) => item.turn_id === "22222222-2222-7222-8222-222222222222");
  if (placement) { placement.topic_ids = ["topic-two"]; placement.source = "person"; }
  setFixtureConflictStatus(`Concurrent edit applied · revision ${snapshot.revision} · only Second sample topic is selected`);
};

const root = document.querySelector<HTMLElement>("#root")!;
render(() => <div class="app" classList={{ "sidebar-collapsed": !sidebarOpen() }} style="--sidebar-width:278px;--dock-width:0px"><Sidebar /><div class="main"><WorkspaceHeader /><main class="topics-fixture-content"><h1>Sample conversation</h1><p>This fixture mounts the real Agent sidebar, workspace header, Topics sheet and per-request placement control against editable sample state.</p><p>Try the topic navigation, pinning, archiving, automatic organization and per-answer placement controls.</p><section class="fixture-sample-request" aria-label="Sample source request"><div><strong>Compare the available choices</strong><p>Review the options and prepare a follow-up recommendation.</p><Show when={fixtureConflictStatus()}><p role="status">{fixtureConflictStatus()}</p></Show></div><TopicTurnControl sessionId="fixture-session" turnId="22222222-2222-7222-8222-222222222222" request="Review the options and prepare a follow-up recommendation." /><button type="button" onClick={simulateConflict}>Simulate another person's edit</button></section></main></div></div>, root);

const syncViewport = () => {
  const phone = document.querySelector<HTMLSelectElement>("#fixture-width")?.value === "phone";
  document.documentElement.dataset.fixtureWidth = phone ? "phone" : "wide";
  setNarrowViewport(phone);
  setSidebarOpen(!phone);
};
document.querySelector<HTMLSelectElement>("#fixture-theme")?.addEventListener("change", (event) => { document.documentElement.dataset.theme = (event.currentTarget as HTMLSelectElement).value; });
document.querySelector<HTMLSelectElement>("#fixture-width")?.addEventListener("change", syncViewport);
syncViewport();
