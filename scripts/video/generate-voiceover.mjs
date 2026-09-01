import {readFile, mkdir, writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';

const root = resolve(import.meta.dirname, '../../');
const videoId = process.env.VAK_VIDEO_ID ?? 'vak-architecture';
const scriptPath = resolve(root, process.env.VAK_NARRATION_FILE ?? 'docs/video/narration.txt');
const outputDir = resolve(root, `video/${videoId}/public/audio`);
const outputPath = resolve(outputDir, 'narration.mp3');

if (!process.env.OPENAI_API_KEY) {
  throw new Error('OPENAI_API_KEY is required');
}

const input = (await readFile(scriptPath, 'utf8')).trim();
if (!input) throw new Error(`Narration script is empty: ${scriptPath}`);

const response = await fetch('https://api.openai.com/v1/audio/speech', {
  method: 'POST',
  headers: {
    Authorization: `Bearer ${process.env.OPENAI_API_KEY}`,
    'Content-Type': 'application/json',
  },
  body: JSON.stringify({
    model: process.env.VAK_TTS_MODEL ?? 'gpt-4o-mini-tts',
    voice: process.env.VAK_TTS_VOICE ?? 'marin',
    input,
    instructions: 'Warm, clear, confident technical narrator. Moderate pace. Brief pauses between sections. Pronounce VAK as one word and vak-core as vak core.',
    response_format: 'mp3',
  }),
});

if (!response.ok) throw new Error(`Speech generation failed: ${response.status} ${await response.text()}`);
await mkdir(outputDir, {recursive: true});
await writeFile(outputPath, Buffer.from(await response.arrayBuffer()));
console.log(`Wrote ${outputPath}`);
