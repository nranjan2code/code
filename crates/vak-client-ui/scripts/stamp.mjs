// Record which sources produced dist/, so a stale bundle is a build error
// rather than a silent 404 (docs/design/32-release-engineering.md).
//
// `vak-server` embeds dist/ with `include_dir!` at Cargo compile time. Its
// build.rs already reruns when dist/ changes, but nothing connected dist/
// back to src/: editing a component and running `cargo build` without
// `npm run build` produced a binary serving the previous bundle, with no
// error anywhere to explain it. release.sh catches that before a release;
// this catches it on the very next `cargo build`.
//
// The manifest lists sha256 per source file rather than one combined hash,
// so the Rust side re-verifies with its own sha256 and there is no hashing
// scheme to keep in sync across two languages.

import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function walk(dir, out = []) {
  for (const name of readdirSync(dir).sort()) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) walk(full, out);
    else out.push(full);
  }
  return out;
}

const files = [...walk(join(root, "src")), join(root, "index.html")]
  .map((f) => relative(root, f).split("\\").join("/"))
  .sort();

const lines = files.map((rel) => {
  const digest = createHash("sha256").update(readFileSync(join(root, rel))).digest("hex");
  return `${digest}  ${rel}`;
});

// Which bundle we just built — `dist` (Tauri) or `dist-web` (the copy
// vak-server embeds under /app). Both are stamped against the same sources.
const dist = process.argv[2] ?? "dist";

writeFileSync(join(root, dist, ".src-manifest"), `${lines.join("\n")}\n`);
process.stdout.write(`stamped ${lines.length} source files into ${dist}/.src-manifest\n`);
