import { render } from 'solid-js/web';
import { Settings } from '../src/App';
import '../src/styles.css';

const theme = new URLSearchParams(location.search).get('theme') ?? 'light';
document.documentElement.dataset.theme = theme;
const voice = { enabled: false, provider: new URLSearchParams(location.search).get('case') === 'routed' ? 'openrouter' : 'openai', transcription_model: null as string | null, synthesis_model: null as string | null, voice_name: null as string | null, persona: null as string | null, max_session_secs: 900, max_concurrent: 2, max_audio_bytes: 16777216 };
const patches: Record<string, unknown>[] = [];
const speech: Record<string, unknown>[] = [];
const pack = { provider: voice.provider, transcription_model: 'account-transcribe', synthesis_model: 'account-mini-tts' };
const rejectDiscovery = new URLSearchParams(location.search).get('case') === 'discovery-error';
const config = () => ({ voice: { ...voice }, provider: 'alpha', model: 'alpha-model', permissions: {}, trusted_hosts: [], route: {same_model: [], fallback_models: []} });
window.fetch = async (input, init) => {
  const path = new URL(String(input), location.origin).pathname;
  const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), {status, headers: {'Content-Type': 'application/json'}});
  if (init?.method === 'PATCH') {
    const patch = JSON.parse(String(init.body)); patches.push(patch);
    for (const field of ['enabled', 'provider', 'transcription_model', 'synthesis_model', 'name', 'persona']) {
      if (`voice_${field}` in patch) (voice as any)[field === 'name' ? 'voice_name' : field] = patch[`voice_${field}`];
    }
    return json({saved: true});
  }
  if (path === '/voice/speak') { speech.push(JSON.parse(String(init?.body))); return json({error: 'Synthetic provider refusal'}, 503); }
  if (path === '/voice/providers') return json({providers: [{name:voice.provider, configured:true, formats:['wav'], credential_vars:[], readiness:{ready:false, detail:'Choose listening and speaking models'}}]});
  if (path === '/providers') return json({providers:[{name:'alpha', configured:true}], current:'alpha', current_model:'alpha-model'});
  if (path === '/voice/models') return rejectDiscovery ? json({error:'Discovery unavailable'}, 502) : json({provider:voice.provider, models:[pack.transcription_model,pack.synthesis_model], capabilities:{transcription:[pack.transcription_model],synthesis:[pack.synthesis_model]},voice_pack:voice.provider === 'openai' ? pack : undefined, voice_packs:voice.provider === 'openrouter' ? [{...pack,label:'Vendor',voice_name:'account-voice'}] : undefined});
  if (path.endsWith('/models')) return json({provider:'alpha', models:['alpha-model'], capabilities:{}});
  if (['/config','/admin/api/config','/config/global','/config/project'].includes(path)) return json(config());
  if (path === '/config/control') return json({control:{},effective:{},limits:{},shipped_limits:{}});
  if (path === '/config/route/suggestions') return json({groups:[],errors:[]});
  return json({});
};
render(() => <Settings />, document.getElementById('root')!);
const tick = () => new Promise(resolve => setTimeout(resolve, 100));
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find(item => item.textContent?.trim() === text)!;
(window as any).runChecks = async () => {
  const passed: string[] = [];
  const check = (value: unknown, message: string) => { if (!value) throw new Error(message); passed.push(message); };
  await tick();
  const panel = document.querySelector<HTMLElement>('[data-testid="voice-settings"]')!;
  check(panel, 'voice configuration is available while conversations are disabled');
  check(patches.length === 0, 'opening settings does not apply a suggestion');
  check((document.getElementById('voice-provider') as HTMLSelectElement).value === voice.provider, 'the selected service survives asynchronous catalogue loading');
  if (rejectDiscovery) {
    check(panel.textContent?.includes('Discovery unavailable'), 'discovery failure is shown');
    check(document.getElementById('voice-synthesis-model')?.tagName === 'INPUT', 'exact model entry remains available after discovery fails');
    return {theme, width:innerWidth, passed};
  }
  check(!panel.textContent?.includes('ready to use'), 'a connected key does not claim a verified voice route');
  button('Use everyday setup').click(); await tick();
  check(patches.length === 1 && patches[0].voice_provider === pack.provider && patches[0].voice_transcription_model === pack.transcription_model && patches[0].voice_synthesis_model === pack.synthesis_model, 'one write applies provider and both discovered models');
  check(!('voice_enabled' in patches[0]), 'applying a pair does not enable microphone conversations');
  const name = document.getElementById('voice-name') as HTMLInputElement;
  name.value = 'account-voice'; name.dispatchEvent(new Event('change', {bubbles:true})); await tick();
  check(patches.at(-1)?.voice_name === 'account-voice', 'voice name is saved in Agent configuration');
  const style = document.getElementById('voice-style') as HTMLInputElement;
  style.value = 'Calm and concise'; style.dispatchEvent(new Event('change', {bubbles:true})); await tick();
  check(patches.at(-1)?.voice_persona === 'Calm and concise', 'speaking directions are saved in configuration');
  button('Test voice').click(); await tick();
  check(speech.length === 1 && (speech[0].voice_override as any).synthesis_model === pack.synthesis_model, 'audition uses the configured pair while voice remains off');
  check(!voice.enabled && !button('Test voice').disabled, 'provider refusal leaves the microphone off and allows retry');
  const widths = [...panel.querySelectorAll<HTMLElement>('input,select,button')].filter(el => el.getClientRects().length).map(el => el.getBoundingClientRect());
  check(widths.every(rect => rect.left >= 0 && rect.right <= innerWidth + 1), 'voice controls fit the viewport');
  panel.scrollIntoView({block:'start'});
  return {theme, width:innerWidth, passed};
};
