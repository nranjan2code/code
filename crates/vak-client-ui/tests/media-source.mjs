import assert from "node:assert/strict";
import { mediaSource } from "../src/mediaSource.ts";

// A web address is drawn from the web.
assert.deepEqual(mediaSource("https://example.com/a.png"), { kind: "web", url: "https://example.com/a.png" });
assert.deepEqual(mediaSource("data:image/png;base64,AAAA"), { kind: "web", url: "data:image/png;base64,AAAA" });

// A path with no scheme is a workspace file, whatever its name looks like
// (a live card named `gradient.png` and drew nothing, 2026-10-16).
assert.deepEqual(mediaSource("gradient.png"), { kind: "workspace", path: "gradient.png" });
assert.deepEqual(mediaSource("media/My clip.mp4"), { kind: "workspace", path: "media/My clip.mp4" });
assert.deepEqual(mediaSource(".vak/scratch/vak/e1/chart.png"), { kind: "workspace", path: ".vak/scratch/vak/e1/chart.png" });

// Every other scheme, and nothing at all, is refused.
for (const address of ["javascript:alert(1)", "file:///etc/passwd", "//host/a.png", "blob:x", "data:text/html,x", "  "]) {
  assert.deepEqual(mediaSource(address), { kind: "refused" }, address);
}
console.log("media-source: ok");
