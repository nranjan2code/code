import {mkdir, readFile, writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';

const root = resolve(import.meta.dirname, '../../');
const videoId = process.env.VAK_VIDEO_ID ?? 'vak-architecture';
const audioPath = resolve(root, `video/${videoId}/public/audio/narration.mp3`);
const outputDir = resolve(root, `video/${videoId}/public/captions`);
const outputPath = resolve(outputDir, 'narration.json');

if (!process.env.OPENAI_API_KEY) throw new Error('OPENAI_API_KEY is required');
const audio = await readFile(audioPath);
const form = new FormData();
form.append('file', new Blob([audio], {type: 'audio/mpeg'}), 'narration.mp3');
form.append('model', process.env.VAK_STT_MODEL ?? 'gpt-4o-transcribe');
form.append('response_format', 'verbose_json');
form.append('timestamp_granularities[]', 'word');

const response = await fetch('https://api.openai.com/v1/audio/transcriptions', {
  method: 'POST',
  headers: {Authorization: `Bearer ${process.env.OPENAI_API_KEY}`},
  body: form,
});
if (!response.ok) throw new Error(`Transcription failed: ${response.status} ${await response.text()}`);

const data = await response.json();
const captions = (data.words ?? []).map((word) => ({
  text: `${word.word} `,
  startMs: Math.round(word.start * 1000),
  endMs: Math.round(word.end * 1000),
  timestampMs: Math.round(word.start * 1000),
  confidence: null,
}));
await mkdir(outputDir, {recursive: true});
await writeFile(outputPath, JSON.stringify(captions, null, 2));
console.log(`Wrote ${outputPath}`);
