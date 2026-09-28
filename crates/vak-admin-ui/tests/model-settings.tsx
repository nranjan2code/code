import { render } from 'solid-js/web';
import { Settings } from '../src/App';
import '../src/styles.css';
let release: (() => void) | undefined;
let delayAlpha = false;
let unknownBedrock = false;
const patches: { path: string; body: any }[] = [];
let sharedRoute: { same_model: string[][]; fallback_models: string[] } = { same_model: [['alpha/one', 'beta/one-copy']], fallback_models: ['shared-b'] };
window.fetch = async (input, init) => {
  const path = new URL(String(input), location.origin).pathname;
  let body: unknown = {};
  if (init?.method === 'PATCH') {
    const sent = JSON.parse(String(init.body));
    patches.push({ path, body: sent });
    if (path === '/config/global') sharedRoute = { same_model: sent.route_same_model ?? sharedRoute.same_model, fallback_models: sent.route_fallback_models ?? sharedRoute.fallback_models };
  }
  else if (path === '/providers') body = { providers: ['alpha', 'beta', 'bedrock'].map((name) => ({ name, configured: true, requires_key: true })), current: 'alpha', current_model: 'custom-saved' };
  else if (path === '/config/global') body = { provider: 'alpha', model: 'custom-saved', max_turns: 40, permissions: {}, route: sharedRoute };
  else if (['/config', '/config/project'].includes(path)) body = { provider: 'alpha', model: 'custom-saved', max_turns: 40, permissions: {}, route: { same_model: [], fallback_models: [] } };
  else if (path === '/config/route/suggestions') body = { groups: [[{ provider: 'alpha', model: 'custom-saved' }, { provider: 'beta', model: 'vendor/custom-saved' }, { provider: 'gamma', model: 'custom-saved-v1:0' }]], errors: [{ provider: 'bedrock', error: 'unreachable' }] };
  else if (path.endsWith('/models')) {
    const provider = path.split('/')[2];
    if (provider === 'alpha' && delayAlpha) await new Promise<void>((resolve) => { release = resolve; });
    body = provider === 'bedrock' ? { provider, models: ['blocked', 'allowed'], ...(unknownBedrock ? { availability_error: 'AWS check unavailable' } : { availability: [{ model_id: 'blocked', invokable: false }, { model_id: 'allowed', invokable: true }] }) } : { provider, models: [`${provider}-one`, `${provider}-two`] };
  }
  return new Response(JSON.stringify(body), { headers: { 'Content-Type': 'application/json' } });
};
render(() => <Settings />, document.getElementById('root')!);
const tick = () => new Promise((resolve) => setTimeout(resolve, 70));
const select = (label: string, value: string) => { const el = document.querySelector<HTMLSelectElement>(`select[aria-label="${label}"]`)!; el.value = value; el.dispatchEvent(new Event('change', { bubbles: true })); };
const save = () => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes('Save Model Route'))!;
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes(text));
(window as any).runChecks = async () => {
  await tick(); const passed: string[] = [];
  const check = (ok: unknown, description: string) => { if (!ok) throw new Error(description); passed.push(description); };
  const panel = () => document.querySelector<HTMLElement>('[data-testid="backup-models"]')!;
  const boxes = () => [...panel().querySelectorAll<HTMLInputElement>('input[type="checkbox"]')];
  check(panel()?.textContent?.includes('one-copy') && panel().textContent?.includes('shared-b'), 'backup panel shows confirmed groups and allowed backups');
  check(boxes().length === 0 && patches.length === 0, 'nothing is proposed or written on open');
  button('Find custom-saved at other services')!.click(); await tick();
  check(boxes().length === 3 && boxes().every((b) => b.checked), 'a proposal lists every member, all ticked');
  check(panel().textContent?.includes('Could not read the model list from Amazon Bedrock'), 'an unreadable service is named by its label, not guessed at');
  boxes()[1].click(); boxes()[2].click(); await tick();
  check(button('Confirm as the same model')!.disabled, 'one ticked name is not a group');
  boxes()[1].click(); await tick();
  button('Confirm as the same model')!.click(); await tick();
  check(JSON.stringify(patches.at(-1)) === JSON.stringify({ path: '/config/global', body: { route_same_model: [['alpha/one', 'beta/one-copy'], ['alpha/custom-saved', 'beta/vendor/custom-saved']] } }), 'confirming writes the shared list plus only the ticked names');
  check(boxes().length === 0 && panel().textContent?.includes('vendor/custom-saved'), 'a confirmed proposal moves to the confirmed list');
  const draft = panel().querySelector<HTMLInputElement>('#backup-model-id')!;
  draft.value = ' model-z '; draft.dispatchEvent(new Event('input', { bubbles: true })); await tick();
  button('Allow as backup')!.click(); await tick();
  check(JSON.stringify(patches.at(-1)) === JSON.stringify({ path: '/config/global', body: { route_fallback_models: ['shared-b', 'model-z'] } }), 'allowing a backup writes this layer\'s list with the new id');
  check(document.querySelector<HTMLSelectElement>('select[aria-label="Model"]')?.value === 'custom-saved', 'refresh preserves an exact saved ID missing from catalogue');
  delayAlpha = true;
  [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes('Refresh list'))!.click();
  await tick(); select('Provider', 'beta'); await tick(); release?.(); delayAlpha = false; await tick();
  check([...document.querySelector<HTMLSelectElement>('select[aria-label="Model"]')!.options].some((o) => o.value === 'beta-one'), 'new provider models remain after delayed old response');
  check(![...document.querySelector<HTMLSelectElement>('select[aria-label="Model"]')!.options].some((o) => o.value === 'alpha-one'), 'stale catalogue discarded');
  check(save().disabled, 'provider switch requires deliberate model choice');
  select('Provider', 'bedrock'); await tick();
  const options = [...document.querySelector<HTMLSelectElement>('select[aria-label="Model"]')!.options];
  check(options.find((o) => o.value === 'blocked')?.disabled && !options.find((o) => o.value === 'allowed')?.disabled, 'Bedrock authorization updates option states');
  select('Model', 'allowed'); await tick(); check(!save().disabled, 'confirmed Bedrock model can save');
  unknownBedrock = true; [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes('Refresh list'))!.click(); await tick();
  check(save().disabled, 'unknown Bedrock access blocks saving even a previous choice');
  return { passed: passed.length, checks: passed };
};
