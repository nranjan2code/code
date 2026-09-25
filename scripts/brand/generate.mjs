import { readFile, writeFile, mkdir, readdir, mkdtemp, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const require = createRequire(import.meta.url);
const sharp = require('sharp');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const check = process.argv.includes('--check');
const master = await readFile(join(root, 'docs/brand/mark/vak-logo-master.png'));
const metadata = await sharp(master).metadata();
if (metadata.width !== 1254 || metadata.height !== 1254 || !metadata.hasAlpha) {
  throw new Error('Expected the approved 1254px RGBA master; review export bounds if it changes');
}
const paper = '#f4f1ea';
const svg = (body, box = '0 0 608 608') =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${box}">${body}</svg>\n`;
// Trim only the exterior canvas and fringe. The approved artwork is embedded
// unchanged: there is no tracing, redrawing, recolouring or new lighting.
const artwork = `<svg width="608" height="608" viewBox="73 73 1108 1108"><image width="1254" height="1254" href="data:image/png;base64,${master.toString('base64')}"/></svg>`;
const tile = `<defs><clipPath id="tile"><rect width="608" height="608" rx="114"/></clipPath></defs><g clip-path="url(#tile)">${artwork}</g>`;
const source = svg(tile);
const flat = svg(`<rect width="608" height="608" fill="${paper}"/>${tile}`);
const maskable = svg(`<rect width="608" height="608" fill="${paper}"/><g transform="translate(45.6 45.6) scale(.85)">${tile}</g>`);
// Only the macOS template discards colour and the paper ground. The alpha mask
// is derived from the approved raster, not from a separately drawn approximation.
const mask = `<defs><filter id="ink" color-interpolation-filters="sRGB"><feColorMatrix type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  -2.55 -8.58 -.87 10 0"/></filter></defs>`;
const template = svg(`${mask}<g filter="url(#ink)">${artwork}</g>`, '64 72 480 480');
// Adaptive launchers keep the original coloured mark; the pale ground is moved
// to the opaque background layer by a luminance mask over the same artwork.
const foreground = svg(`${mask}<defs><mask id="glyph" mask-type="alpha"><g filter="url(#ink)">${artwork}</g></mask></defs><g transform="translate(97.28 97.28) scale(.68)"><g mask="url(#glyph)">${artwork}</g></g>`);
const dock = svg(`<g transform="translate(59.375 59.375) scale(.8046875)">${tile}</g>`);
const round = svg(`<defs><clipPath id="circle"><circle cx="304" cy="304" r="304"/></clipPath></defs><g clip-path="url(#circle)"><rect width="608" height="608" fill="${paper}"/>${artwork}</g>`);
let count = 0;
const failures = [];

async function save(path, data) {
  const destination = join(root, path);
  if (check) {
    const current = await readFile(destination).catch(() => null);
    if (!current?.equals(Buffer.from(data))) failures.push(path);
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
await save('docs/market/vak-icon.png', await png(source, 512));
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
// The server's existing public favicon alias uses this file too.
const favicon = await png(source, 64);
await save('crates/vak-admin-ui/public/favicon.svg', svg(`<image width="608" height="608" href="data:image/png;base64,${favicon.toString('base64')}"/>`));
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
await save(`${icons}/android/values/ic_launcher_background.xml`, `<?xml version="1.0" encoding="utf-8"?>\n<resources>\n  <color name="ic_launcher_background">${paper}</color>\n</resources>\n`);

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

// iconutil emits the native small-size ICNS representations as well as Retina PNGs.
if (process.platform !== 'darwin') throw new Error('ICNS generation/check requires macOS iconutil');
const temporary = await mkdtemp(join(tmpdir(), 'vak-brand-'));
try {
  const iconset = join(temporary, 'Vak.iconset');
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
console.log(`${check ? 'Verified' : 'Generated'} ${count} brand assets from the approved vak-logo-master.png`);
