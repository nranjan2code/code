import { render } from 'solid-js/web';
import { Settings } from '../src/App';
import '../src/styles.css';
let release: (() => void) | undefined;
let delayAlpha = false;
let unknownBedrock = false;
window.fetch = async (input) => {
  const path = new URL(String(input), location.origin).pathname;
  let body: unknown = {};
  if (path === '/providers') body = { providers: ['alpha', 'beta', 'bedrock'].map((name) => ({ name, configured: true, requires_key: true })), current: 'alpha', current_model: 'custom-saved' };
  else if (['/config', '/config/global', '/config/project'].includes(path)) body = { provider: 'alpha', model: 'custom-saved', max_turns: 40, permissions: {}, route: { fallback_models: [] } };
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
(window as any).runChecks = async () => {
  await tick(); const passed: string[] = [];
  const check = (ok: unknown, description: string) => { if (!ok) throw new Error(description); passed.push(description); };
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
