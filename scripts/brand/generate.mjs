import { readFile, writeFile, mkdir, readdir, mkdtemp, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const require = createRequire(import.meta.url);
const sharp = require('sharp');
const fontkit = require('fontkit');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const check = process.argv.includes('--check');
const master = await readFile(join(root, 'docs/brand/mark/vakyartha-songbird.svg'), 'utf8');
if (!master.includes('viewBox="80 80 440 440"') || /<image\b/.test(master)) {
  throw new Error('Expected the vector Songbird master; review export framing if it changes');
}
const ink = '#101D3D';
const paper = '#F6F5F1';
const svg = (body, box = '0 0 608 608') =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${box}">${body}</svg>\n`;
const bird = master.slice(master.indexOf('<path'), master.lastIndexOf('</svg>'));
const artwork = (colour = ink, accent = '#F5A400', size = 608) => `<svg width="${size}" height="${size}" viewBox="80 80 440 440">${bird.replaceAll(ink, colour).replaceAll('#F5A400', accent)}</svg>`;
const tileBird = `<g transform="translate(54.72 54.72)">${artwork(paper, '#F5A400', 498.56)}</g>`;
const tile = `<rect width="608" height="608" rx="114" fill="${ink}"/>${tileBird}`;
const source = svg(tile);
const flat = svg(`<rect width="608" height="608" fill="${ink}"/>${tileBird}`);
// The bird fits within the central safe circle of maskable/adaptive icons.
const maskableBird = `<g transform="translate(109.44 109.44)">${artwork(paper, '#F5A400', 389.12)}</g>`;
const maskable = svg(`<rect width="608" height="608" fill="${ink}"/>${maskableBird}`);
const template = svg(artwork('#000000', '#000000'));
const foreground = svg(maskableBird);
const dock = svg(`<g transform="translate(59.375 59.375) scale(.8046875)">${tile}</g>`);
const round = svg(`<circle cx="304" cy="304" r="304" fill="${ink}"/>${maskableBird}`);
let count = 0;
const failures = [];

async function save(path, data) {
  const destination = join(root, path);
  if (check) {
    // Hash the expected result rather than holding two large image buffers.
    // This keeps a full platform export check usable on ordinary laptops.
    const current = await readFile(destination).catch(() => null);
    if (!current || !current.equals(Buffer.from(data))) failures.push(path);
  } else {
    await mkdir(dirname(destination), { recursive: true });
    await writeFile(destination, data);
  }
  count++;
}

async function png(art, size, opaque = false) {
  const render = sharp(Buffer.from(art), { density: 384 }).resize(size, size);
  return (opaque ? render.removeAlpha() : render.ensureAlpha()).png().toBuffer();
}

const icons = 'crates/vak-desktop/icons';
// Outlined lettering is identical in browsers, raster exports and print: no
// installed font or network font request can change the public wordmark.
const font = fontkit.openSync(join(root, 'docs/brand/fonts/Manrope.ttf')).getVariation({ wght: 600 });
const run = font.layout('Vakyartha');
const scale = 128 / font.unitsPerEm;
let advance = 0;
const letters = run.glyphs.map((glyph, i) => {
  const position = run.positions[i];
  const path = `<path transform="translate(${advance + position.xOffset} ${position.yOffset})" d="${glyph.path.toSVG()}"/>`;
  advance += position.xAdvance;
  return path;
}).join('');
const wordWidth = Math.ceil(advance * scale);
const word = (colour) => `<g fill="${colour}" transform="translate(0 126) scale(${scale} ${-scale})">${letters}</g>`;
const lockup = (colour, accent) => svg(`${artwork(colour, accent, 176)}<g transform="translate(194 0)">${word(colour)}</g>`, `0 0 ${wordWidth + 206} 176`);
for (const [name, colour, accent] of [['colour', ink, '#F5A400'], ['reverse', paper, '#F5A400'], ['black', '#000000', '#000000'], ['white', '#FFFFFF', '#FFFFFF']]) {
  const symbol = svg(artwork(colour, accent));
  const horizontal = lockup(colour, accent);
  const wordmark = svg(word(colour), `0 0 ${wordWidth + 2} 162`);
  for (const [kind, content] of [['songbird', symbol], ['lockup', horizontal], ['wordmark', wordmark]]) {
    await save(`docs/brand/exports/vakyartha-${kind}-${name}.svg`, content);
  }
  await save(`docs/brand/exports/vakyartha-songbird-${name}.png`, await png(symbol, 2048));
  await save(`docs/brand/exports/vakyartha-lockup-${name}.png`, await sharp(Buffer.from(horizontal), { density: 384 }).resize({ width: 2400 }).png().toBuffer());
  for (const app of ['vak-client-ui', 'vak-admin-ui']) {
    await save(`crates/${app}/public/assets/brand/songbird-${name}.svg`, symbol);
    await save(`crates/${app}/public/assets/brand/wordmark-${name}.svg`, wordmark);
  }
}
// Print-safe, editable 3.5 × 2 in card artwork with 0.125 in bleed.
// Trim area is inset by 9 SVG units; keep the lockup and contact details inside.
const card = (back, content) => svg(`<rect width="3456" height="2088" fill="${back}"/>${content}`, '0 0 3456 2088').replace('<svg ', '<svg width="3.75in" height="2.25in" ');
const logoColour = lockup(ink, '#F5A400');
const logoReverse = lockup(paper, '#F5A400');
const trimMarks = `<g fill="none" stroke="#8a8a8a" stroke-width="2"><path d="M108 108h48M108 108v48M3348 108h-48M3348 108v48M108 1980h48M108 1980v-48M3348 1980h-48M3348 1980v-48"/></g>`;
await save('docs/brand/exports/vakyartha-card-front.svg', card(ink, `<g transform="translate(300 814) scale(3.8)">${logoReverse.replace(/^.*?<svg[^>]*>|<\/svg>\s*$/gs, '')}</g><rect x="300" y="1240" width="240" height="18" rx="9" fill="#F5A400"/>${trimMarks}`));
await save('docs/brand/exports/vakyartha-card-back.svg', card(paper, `<g transform="translate(300 570) scale(2.15)">${logoColour.replace(/^.*?<svg[^>]*>|<\/svg>\s*$/gs, '')}</g><text x="310" y="1265" fill="#101D3D" font-family="Manrope, sans-serif" font-size="220" font-weight="600">Name</text><text x="310" y="1450" fill="#526079" font-family="Manrope, sans-serif" font-size="150">Role</text><path d="M310 1535H1750" stroke="#D8D6CF" stroke-width="3"/><text x="310" y="1655" fill="#101D3D" font-family="Manrope, sans-serif" font-size="136">your@email.com</text><text x="310" y="1800" fill="#101D3D" font-family="Manrope, sans-serif" font-size="136">vakyartha.com</text>${trimMarks}`));
// A flexible wordmark mask for UI headers: keep the actual artwork in the SVG
// and let each surface tint it with its accessible foreground colour.
for (const app of ['vak-client-ui', 'vak-admin-ui']) {
  await save(`crates/${app}/public/assets/brand/wordmark-mask.svg`, svg(word('#000000'), `0 0 ${wordWidth + 2} 162`));
}
await save('docs/brand/exports/vakyartha-app-icon.svg', source);
await save('docs/brand/exports/vakyartha-app-icon.png', await png(source, 1024));
await save('docs/brand/exports/vakyartha-lockup-web.svg', lockup(ink, '#F5A400'));
await save('docs/assets/visual-refresh-2026/img/vak-icon.png', await png(source, 512));
await save('crates/vak-desktop/app-icon.png', await png(dock, 1024));
await save(`${icons}/icon.png`, await png(dock, 512));
for (const size of [32, 64, 128]) await save(`${icons}/${size}x${size}.png`, await png(source, size));
await save(`${icons}/128x128@2x.png`, await png(source, 256));
for (const size of [30, 44, 71, 89, 107, 142, 150, 284, 310]) {
  await save(`${icons}/Square${size}x${size}Logo.png`, await png(source, size));
}
await save(`${icons}/StoreLogo.png`, await png(source, 50));
await save(`${icons}/tray-template.png`, await png(template, 36));
await save(`${icons}/tray-color.png`, await png(source, 32));

for (const app of ['vak-client-ui', 'vak-admin-ui']) {
  await save(`crates/${app}/public/vak-icon.png`, await png(source, 512));
}
await save('crates/vak-admin-ui/public/favicon.svg', source);
await save('crates/vak-client-ui/public/favicon.svg', source);
await save('crates/vak-client-ui/public/manifest.webmanifest', JSON.stringify({
  name: 'Vakyartha', short_name: 'Vakyartha', start_url: '/app', display: 'standalone',
  background_color: paper, theme_color: ink,
  icons: [
    { src: '/app/assets/brand/icon-192.png', sizes: '192x192', type: 'image/png' },
    { src: '/app/assets/brand/icon-512.png', sizes: '512x512', type: 'image/png' },
    { src: '/app/assets/brand/icon-maskable-512.png', sizes: '512x512', type: 'image/png', purpose: 'maskable' },
  ],
}, null, 2) + '\n');
for (const size of [192, 512]) {
  await save(`crates/vak-client-ui/public/assets/brand/icon-${size}.png`, await png(source, size));
}
await save('crates/vak-client-ui/public/assets/brand/icon-maskable-512.png', await png(maskable, 512));
await save('crates/vak-client-ui/public/assets/brand/apple-touch-icon.png', await png(flat, 180, true));

for (const file of await readdir(join(root, icons, 'ios'))) {
  const match = file.match(/^AppIcon-(\d+(?:\.\d+)?)(?:x[\d.]+)?@(\d)x(?:-\d)?\.png$/);
  if (match) await save(`${icons}/ios/${file}`, await png(flat, Number(match[1]) * Number(match[2]), true));
}
for (const [density, size] of [['mdpi', 48], ['hdpi', 72], ['xhdpi', 96], ['xxhdpi', 144], ['xxxhdpi', 192]]) {
  const dir = `${icons}/android/mipmap-${density}`;
  await save(`${dir}/ic_launcher.png`, await png(source, size));
  await save(`${dir}/ic_launcher_round.png`, await png(round, size));
  await save(`${dir}/ic_launcher_foreground.png`, await png(foreground, size * 2.25));
}
await save(`${icons}/android/values/ic_launcher_background.xml`, `<?xml version="1.0" encoding="utf-8"?>\n<resources>\n  <color name="ic_launcher_background">${ink}</color>\n</resources>\n`);

const icoSizes = [16, 24, 32, 48, 64, 128, 256];
const frames = await Promise.all(icoSizes.map(size => png(source, size)));
const ico = Buffer.alloc(6 + 16 * frames.length);
ico.writeUInt16LE(1, 2);
ico.writeUInt16LE(frames.length, 4);
let offset = ico.length;
frames.forEach((frame, i) => {
  const p = 6 + i * 16;
  ico[p] = ico[p + 1] = icoSizes[i] === 256 ? 0 : icoSizes[i];
  ico.writeUInt16LE(1, p + 4);
  ico.writeUInt16LE(32, p + 6);
  ico.writeUInt32LE(frame.length, p + 8);
  ico.writeUInt32LE(offset, p + 12);
  offset += frame.length;
});
await save(`${icons}/icon.ico`, Buffer.concat([ico, ...frames]));

// Agent characters (docs/design/71): the 512px portraits and 1024x512
// expression atlases in docs/brand/characters are the source art. The client
// loads only these WebP copies, the smallest that covers the mark's size at
// the screen's pixel ratio, instead of the full sources for a 24px mark.
const characters = ['vak', 'mira', 'moss', 'nori', 'pip', 'lumi', 'tavi', 'beni'];
const webp = { quality: 88, alphaQuality: 100, effort: 6, smartSubsample: true };
for (const id of characters) {
  const portrait = await readFile(join(root, `docs/brand/characters/${id}.png`));
  const atlas = await readFile(join(root, `docs/brand/characters/${id}-atlas.png`));
  const [p, a] = await Promise.all([sharp(portrait).metadata(), sharp(atlas).metadata()]);
  if (p.width !== 512 || p.height !== 512 || !p.hasAlpha || a.width !== 1024 || a.height !== 512 || !a.hasAlpha) {
    throw new Error(`${id}: expected a 512px RGBA portrait and a 1024x512 RGBA atlas`);
  }
  for (const size of [64, 128, 256]) {
    for (const [label, source, width, height] of [
      ['portrait', portrait, size, size],
      ['atlas', atlas, size * 4, size * 2],
    ]) {
      const path = `crates/vak-client-ui/public/characters/${id}${label === 'atlas' ? '-atlas' : ''}-${size}.webp`;
      if (check) {
        const output = await readFile(join(root, path)).catch(() => null);
        if (!output) failures.push(path);
        else {
          const metadata = await sharp(output).metadata();
          if (metadata.width !== width || metadata.height !== height || metadata.format !== 'webp') failures.push(path);
        }
      } else {
        const resized = await sharp(source).resize(width, height, { kernel: 'lanczos3' }).webp(webp).toBuffer();
        await save(path, resized);
      }
    }
  }
}

// iconutil emits the native small-size ICNS representations as well as Retina PNGs.
if (process.platform !== 'darwin') throw new Error('ICNS generation/check requires macOS iconutil');
const temporary = await mkdtemp(join(tmpdir(), 'vak-brand-'));
try {
  const iconset = join(temporary, 'Vakyartha.iconset');
  await mkdir(iconset);
  for (const size of [16, 32, 128, 256, 512]) {
    for (const scale of [1, 2]) {
      await writeFile(join(iconset, `icon_${size}x${size}${scale === 2 ? '@2x' : ''}.png`), await png(dock, size * scale));
    }
  }
  const icns = join(temporary, 'icon.icns');
  execFileSync('iconutil', ['-c', 'icns', iconset, '-o', icns]);
  await save(`${icons}/icon.icns`, await readFile(icns));
} finally {
  await rm(temporary, { recursive: true, force: true });
}
if (failures.length) throw new Error(`Stale brand exports:\n${failures.join('\n')}`);
console.log(`${check ? 'Verified' : 'Generated'} ${count} brand assets from the Vakyartha Songbird vector master and the character sources`);
