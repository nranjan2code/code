import { render } from "solid-js/web";
import ConnectSheet from "../src/components/ConnectSheet";
import { openConnect, setActiveAgent, setConnectOpen } from "../src/store";
import "../src/styles.css";

const requests: { path: string; method: string; body: any }[] = [];
let failDiscovery = false;
let failSave = false;
let resolveDiscovery: (() => void) | undefined;
let delayed = false;
const providers = [
  { name: "alpha", label: "Alpha AI", env_var: "ALPHA_KEY", requires_key: true, configured: true },
  { name: "beta", label: "Beta AI", env_var: "BETA_KEY", requires_key: true, configured: false },
  { name: "ollama", label: "Ollama", requires_key: false, configured: true },
  { name: "bedrock", label: "Amazon Bedrock", requires_key: true, configured: true },
];
window.fetch = async (input, init) => {
  const url = new URL(String(input), location.origin);
  const path = url.pathname;
  const method = init?.method || "GET";
  const body = init?.body ? JSON.parse(String(init.body)) : null;
  requests.push({ path: url.pathname + url.search, method, body });
  let result: unknown = {};
  let status = 200;
  if (path === "/providers") result = { providers, current: "alpha", current_model: "saved-model" };
  else if (path === "/config" && method === "GET") result = { provider: "alpha", model: "saved-model" };
  else if (path === "/config/global" && method === "GET") result = { provider: "beta", model: "shared-model" };
  else if (path.endsWith("/models")) {
    if (delayed) await new Promise<void>((resolve) => { resolveDiscovery = resolve; });
    if (failDiscovery) { result = { error: "Service is unavailable. Try again." }; status = 502; }
    else if (path.includes("bedrock")) result = { provider: "bedrock", models: ["blocked", "allowed"], availability: [{ model_id: "blocked", invokable: false }, { model_id: "allowed", invokable: true }] };
    else result = { provider: path.split('/')[2], models: ["first-model", "saved-model", "shared-model"] };
  } else if (method === "PATCH" && failSave) { result = { error: "Could not save settings" }; status = 500; }
  else if (path === "/health") result = { status: "ok" };
  return new Response(JSON.stringify(result), { status, headers: { "Content-Type": "application/json" } });
};
document.documentElement.dataset.theme = new URLSearchParams(location.search).get("theme") || "light";
setActiveAgent({ id: "agent-a", name: "Agent A" } as any);
render(() => <><button onClick={() => openConnect()}>AI service and model</button><ConnectSheet /></>, document.getElementById("root")!);
openConnect();
const tick = () => new Promise((resolve) => setTimeout(resolve, 60));
const service = () => document.querySelector<HTMLSelectElement>('.connect-field select')!;
const model = () => [...document.querySelectorAll<HTMLSelectElement>('.connect-field select')].at(-1)!;
const primary = () => document.querySelector<HTMLButtonElement>('.sheet-footer .primary')!;
const select = (element: HTMLSelectElement, value: string) => { element.value = value; element.dispatchEvent(new Event('change', { bubbles: true })); };
const inputKey = () => { const input = document.querySelector<HTMLInputElement>('input[type=password]')!; input.value = 'fixture-key'; input.dispatchEvent(new Event('input', { bubbles: true })); };
(window as any).runChecks = async () => {
  const passed: string[] = [];
  const check = (ok: unknown, description: string) => { if (!ok) throw new Error(description); passed.push(description); };
  await tick();
  check(document.body.textContent?.includes('saved-model'), 'daily flow shows saved model before discovery');
  check(requests.filter((r) => r.method !== 'GET').length === 0, 'opening makes no writes');
  delayed = true; primary().click(); await tick();
  check(service().disabled && primary().disabled, 'in-flight check locks service and submit');
  setActiveAgent({ id: 'agent-b', name: 'Agent B' } as any); resolveDiscovery?.(); delayed = false; await tick();
  check(model().value === 'saved-model', 'discovery preserves choice despite changed ordering');
  primary().click(); await tick();
  check(requests.some((r) => r.method === 'PATCH' && r.body.agent === 'agent-a' && r.body.provider === 'alpha' && r.body.model === 'saved-model'), 'save captures agent and atomically writes service/model');
  openConnect(); await tick(); select(service(), 'beta'); inputKey(); select(service(), 'alpha'); await tick();
  check(document.querySelector<HTMLInputElement>('input[type=password]')?.value === '', 'switching service clears secret draft');
  select(service(), 'beta'); inputKey(); failDiscovery = true; primary().click(); await tick();
  check(document.body.textContent?.includes('Account key saved'), 'key success survives discovery failure');
  check(!primary().disabled && document.querySelector<HTMLInputElement>('input[type=password]')?.value === '', 'retry needs no secret re-entry');
  failDiscovery = false; primary().click(); await tick();
  check(model().value === '' && primary().disabled, 'multiple alternatives require deliberate selection');
  select(model(), 'first-model'); failSave = true; primary().click(); await tick();
  check(!!document.querySelector('[role=alert]') && !primary().disabled, 'save failure retains choice and retry');
  failSave = false; setConnectOpen(false); openConnect('user'); await tick();
  check(service().value === 'beta' && document.body.textContent?.includes('shared-model'), 'shared flow reads shared layer');
  inputKey(); primary().click(); await tick(); primary().click(); await tick();
  check(requests.some((r) => r.path === '/config/global' && r.method === 'PATCH' && r.body.model === 'shared-model' && !r.body.agent), 'shared write uses shared endpoint');
  openConnect(); await tick(); select(service(), 'bedrock'); primary().click(); await tick();
  check(![...model().options].some((o) => o.value === 'blocked') && model().value === 'allowed', 'unavailable Bedrock model cannot be chosen');
  check(!document.body.textContent?.includes('Private and free'), 'no unverified local privacy promise');
  check(document.documentElement.scrollWidth <= innerWidth, 'no horizontal overflow');
  setConnectOpen(false); openConnect(); await tick();
  return { passed: passed.length, checks: passed };
};
