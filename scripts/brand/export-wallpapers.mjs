import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const root = fileURLToPath(new URL('../..', import.meta.url));
const directory = join(root, 'docs/brand/library/wallpapers');
const check = process.argv.includes('--check');
const publicDirectory = join(root, 'crates/vak-server/site/src/assets/wallpapers');
if (!check) await mkdir(publicDirectory, { recursive: true });
const sizes = [
  [1920, 1080], // common Full HD monitor
  [2560, 1440], // QHD monitor
  [3840, 2160], // 4K monitor; enlarged from the generated master
  [2560, 1600], // 16:10 laptop
  [3840, 2400], // high-density 16:10 laptop; enlarged from master
];

const mobileSizes = [[1080, 2400], [1290, 2796], [1440, 3200]];

for (const [scene, expectedWidth, expectedHeight] of [['day', 1672, 941], ['dusk', 1586, 992], ['day-mobile', 841, 1870], ['dusk-mobile', 841, 1870]]) {
  const source = join(directory, `ensemble-${scene}-master.png`);
  const metadata = await sharp(source).metadata();
  if (metadata.width !== expectedWidth || metadata.height !== expectedHeight) {
    throw new Error(`${source}: expected the reviewed ${expectedWidth} × ${expectedHeight} master`);
  }
  for (const [width, height] of scene.endsWith('-mobile') ? mobileSizes : sizes) {
    const target = join(directory, `ensemble-${scene}-${width}x${height}.jpg`);
    const output = await sharp(source)
      .resize(width, height, { fit: 'cover', position: 'centre', kernel: 'lanczos3' })
      .jpeg({ quality: 92, mozjpeg: true, chromaSubsampling: '4:4:4' })
      .toBuffer();
    if (check) {
      const saved = await readFile(target);
      if (!saved.equals(output)) throw new Error(`${target}: stale export`);
    } else {
      await writeFile(target, output);
    }
    const publicTarget = join(publicDirectory, `ensemble-${scene}-${width}x${height}.jpg`);
    if (check) {
      if (!(await readFile(publicTarget)).equals(output)) throw new Error(`${publicTarget}: stale export`);
    } else {
      await writeFile(publicTarget, output);
    }
    console.log(`${check ? 'checked' : 'wrote'} ${target}`);
  }
  const preview = await sharp(source).resize({ width: scene.endsWith('-mobile') ? 480 : 960 }).webp({ quality: 82 }).toBuffer();
  const previewTarget = join(publicDirectory, `ensemble-${scene}-preview.webp`);
  if (check) {
    if (!(await readFile(previewTarget)).equals(preview)) throw new Error(`${previewTarget}: stale preview`);
  } else {
    await writeFile(previewTarget, preview);
  }
}
