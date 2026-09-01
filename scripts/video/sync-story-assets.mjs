import {copyFile, mkdir, readFile} from 'node:fs/promises';
import {dirname, resolve} from 'node:path';

const root = resolve(import.meta.dirname, '../..');
const manifestPath = resolve(root, 'video/vak-story/src/assetManifest.ts');
const manifest = await readFile(manifestPath, 'utf8');
const sources = [...manifest.matchAll(/source: '([^']+)'/g)].map((match) => match[1]);
const destination = resolve(root, 'video/vak-story/public/assets');
await mkdir(destination, {recursive: true});
for (const source of sources) {
  await copyFile(resolve(root, 'docs/tutor', source), resolve(destination, source));
}
console.log(`Synced ${sources.length} story assets to ${destination}`);
