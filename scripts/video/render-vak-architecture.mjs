import {access, mkdir, writeFile} from 'node:fs/promises';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {resolve} from 'node:path';

const run = promisify(execFile);
const root = resolve(import.meta.dirname, '../../video/vak-architecture');
const asset = async (relative) => {
  try { await access(resolve(root, 'public', relative)); return relative; } catch { return null; }
};
await mkdir(resolve(root, 'out'), {recursive: true});
const props = {
  voiceoverSrc: await asset('audio/narration.mp3'),
  musicSrc: await asset('music/bed.mp3'),
  captionsSrc: await asset('captions/narration.json'),
};
const propsPath = resolve(root, 'out/render-props.json');
await writeFile(propsPath, JSON.stringify(props, null, 2));
const args = ['remotion', 'render', 'src/index.tsx', 'VakArchitecture', 'out/vak-architecture.mp4', `--props=${propsPath}`];
const {stdout, stderr} = await run('npx', args, {cwd: root, maxBuffer: 1024 * 1024 * 10});
process.stdout.write(stdout);
process.stderr.write(stderr);
