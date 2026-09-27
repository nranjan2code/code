import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const check = process.argv.includes('--check');
const pack = 'crates/vak-client-ui/public/assets/brand/dimensional';
const sourceRoot = 'docs/brand/library/dimensional/characters';
const entries = {
  vak: { source: `${sourceRoot}/vak-source.png`, head: [290, 0, 850, 850] },
  mira: { source: 'docs/brand/characters/mira.png', head: [76, 2, 360, 360] },
  moss: { source: 'docs/brand/characters/moss.png', head: [44, 0, 424, 424] },
  nori: { source: `${sourceRoot}/nori-source.png`, head: [170, 65, 925, 925] },
  pip: { source: `${sourceRoot}/pip-source.png`, head: [230, 0, 820, 820] },
  lumi: { source: `${sourceRoot}/lumi-source.png`, head: [185, 0, 900, 900] },
  tavi: { source: `${sourceRoot}/tavi-source.png`, head: [225, 0, 830, 830] },
  beni: { source: 'docs/brand/characters/beni.png', head: [20, 0, 472, 472] },
};
const webp = { quality: 90, alphaQuality: 100, effort: 6 };

async function save(path, buffer) {
  const target = join(root, path);
  if (check) {
    const previous = await readFile(target).catch(() => null);
    if (!previous || !previous.equals(buffer)) throw new Error(`${path}: stale dimensional export`);
  } else {
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, buffer);
  }
  console.log(`${check ? 'checked' : 'wrote'} ${path}`);
}

for (const [id, entry] of Object.entries(entries)) {
  const input = await readFile(join(root, entry.source));
  const metadata = await sharp(input).metadata();
  if (!metadata.hasAlpha || metadata.width < 512 || metadata.height < 512) {
    throw new Error(`${entry.source}: expected a transparent source of at least 512 px`);
  }
  const [left, top, width, height] = entry.head;
  if (left + width > metadata.width || top + height > metadata.height) {
    throw new Error(`${entry.source}: head crop exceeds source`);
  }
  for (const size of [64, 128, 256, 512]) {
    const portrait = await sharp(input).resize(size, size, { fit: 'contain', background: '#00000000' }).webp(webp).toBuffer();
    await save(`${pack}/characters/${id}-${size}.webp`, portrait);
  }
  for (const size of [32, 64, 128]) {
    const glyph = await sharp(input).extract({ left, top, width, height })
      .resize(size, size, { fit: 'contain', background: '#00000000' }).webp(webp).toBuffer();
    await save(`${pack}/glyphs/${id}-${size}.webp`, glyph);
  }
}

const bird = await readFile(join(root, 'docs/brand/library/art/songbird-3d-transparent.png'));
const iconBackground = Buffer.from('<svg width="1024" height="1024" xmlns="http://www.w3.org/2000/svg"><rect x="18" y="18" width="988" height="988" rx="216" fill="#F6F5F1"/><rect x="19" y="19" width="986" height="986" rx="215" fill="none" stroke="#D8D6CF" stroke-width="2"/></svg>');
const iconBird = await sharp(bird).resize(830, 830).png().toBuffer();
const icon = await sharp(iconBackground).composite([{ input: iconBird, left: 97, top: 97 }]).png().toBuffer();
await save(`${pack}/app-icon-1024.png`, icon);
for (const size of [32, 64, 192, 512]) {
  await save(`${pack}/app-icon-${size}.png`, await sharp(icon).resize(size, size).png().toBuffer());
}
await save(`${pack}/songbird-3d.png`, bird);

for (const [variant, wordmark] of [
  ['light', 'docs/brand/exports/vakyartha-wordmark-colour.svg'],
  ['dark', 'docs/brand/exports/vakyartha-wordmark-reverse.svg'],
]) {
  const birdImage = await sharp(bird).resize(330, 330).png().toBuffer();
  const lettering = await sharp(join(root, wordmark)).resize({ width: 1250 }).png().toBuffer();
  const letterMeta = await sharp(lettering).metadata();
  const layers = [];
  if (variant === 'dark') {
    layers.push({ input: Buffer.from('<svg width="340" height="340" xmlns="http://www.w3.org/2000/svg"><rect x="5" y="5" width="330" height="330" rx="76" fill="#F6F5F1"/></svg>'), left: 0, top: 0 });
  }
  layers.push({ input: birdImage, left: 5, top: 5 });
  layers.push({ input: lettering, left: 390, top: Math.round((340 - letterMeta.height) / 2) });
  const logo = await sharp({ create: { width: 1680, height: 340, channels: 4, background: '#00000000' } })
    .composite(layers).png().toBuffer();
  await save(`${pack}/logo-${variant}.png`, logo);
  await save(`${pack}/site-logo-${variant}.png`, logo);
}
