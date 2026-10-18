import { render } from "solid-js/web";
import { canvasSaveState, retryCanvas, startCanvasSync } from "../src/canvasSync";
import { canvasStacks, changeCanvas, setActiveId } from "../src/store";
import { fileSubject } from "../src/canvasSubject";
import CanvasSaveNotice from "../src/components/canvas/CanvasSaveNotice";
import "../src/styles.css";

// Synthetic server key for this fixture only. The product retrieves its key
// from the owner-authenticated endpoint and never stores it in the browser.
const key = btoa(String.fromCharCode(...new Uint8Array(32).fill(42)));
const sid = "canvas-reload-fixture";
const note = "Unsent reload recovery probe";
const mode = sessionStorage.getItem("fixture.canvasRecovery.mode") ?? "seed";
let gets = 0, puts = 0;
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { "Content-Type": "application/json" } });
document.documentElement.dataset.theme = new URLSearchParams(location.search).get("theme") === "dark" ? "dark" : "light";
window.fetch = async (input, init) => {
  if (String(input).endsWith("/recovery-key")) return mode === "locked" ? json({ error: "Sign in" }, 401) : json({ key, identity: "0192a3c0-0000-7000-8000-000000000001" });
  if (init?.method === "PUT") {
    puts++;
    const body = JSON.parse(String(init.body));
    return mode === "online" ? json({ ...body, schema: 1, revision: body.revision + 1 }) : json({ error: "Unavailable" }, 503);
  }
  gets++;
  return json({ schema: 1, revision: 7, entries: [], active: null });
};
setActiveId(sid);
startCanvasSync();
const draft = () => canvasStacks()[sid]?.entries[0]?.draft ?? "";
render(() => <main style="max-width:640px;margin:auto;padding:16px">
  <h1>Canvas reload recovery</h1>
  <p>Fixture mode: {mode}</p>
  <textarea aria-label="Unsent note" value={draft()} onInput={(event) => {
    const entry = canvasStacks()[sid]?.entries[0];
    if (entry) changeCanvas(sid, { op: "update", key: entry.key, patch: { draft: event.currentTarget.value } });
  }} />
  <CanvasSaveNotice status={canvasSaveState(sid)} onRetry={() => retryCanvas(sid)} />
  <output id="result" />
</main>, document.getElementById("root")!);

async function rows(): Promise<any[]> {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open("vak.canvas-recovery.v1", 1);
    open.onsuccess = () => {
      const db = open.result;
      const tx = db.transaction("pending");
      const request = tx.objectStore("pending").getAll();
      request.onsuccess = () => resolve(request.result);
      tx.oncomplete = () => db.close();
      request.onerror = () => reject(new Error("Could not inspect ciphertext"));
    };
    open.onerror = () => reject(new Error("No recovery database"));
  });
}
async function until(test: () => boolean | Promise<boolean>) {
  for (let i = 0; i < 100; i++) {
    if (await test()) return;
    await new Promise((resolve) => setTimeout(resolve, 30));
  }
  throw new Error("Recovery did not reach the expected state");
}
const check = (value: unknown, message: string) => { if (!value) throw new Error(message); };
(window as any).flipRecoveryCiphertext = async () => {
  await new Promise<void>((resolve, reject) => {
    const open = indexedDB.open("vak.canvas-recovery.v1", 1);
    open.onsuccess = () => {
      const db = open.result, tx = db.transaction("pending", "readwrite");
      const cursor = tx.objectStore("pending").openCursor();
      cursor.onsuccess = () => {
        const row = cursor.result;
        if (!row) return;
        const value = row.value;
        new Uint8Array(value.bytes)[0] ^= 1;
        row.update(value);
      };
      tx.oncomplete = () => { db.close(); resolve(); };
      tx.onerror = tx.onabort = () => { db.close(); reject(new Error("Could not mutate fixture ciphertext")); };
    };
  });
};
(window as any).setRecoveryMode = (next: string) => sessionStorage.setItem("fixture.canvasRecovery.mode", next);
(window as any).runChecks = async () => {
  if (mode === "seed") {
    await until(() => gets > 0);
    changeCanvas(sid, { op: "open", subject: fileSubject("recovery.txt", { sessionId: sid }, "Notes") });
    changeCanvas(sid, { op: "update", key: canvasStacks()[sid].active, patch: { draft: note } });
    await until(() => canvasSaveState(sid).state === "error" && canvasSaveState(sid).recovery === "ready");
    const saved = await rows();
    check(saved.length > 0, "Failed server write has a durable checkpoint");
    check(saved.every((row) => row.bytes instanceof ArrayBuffer && !JSON.stringify(row).includes(note) && !("key" in row)), "IndexedDB contains ciphertext and no note or key");
    check(draft() === note, "Pending note remains visible");
  } else if (mode === "locked" || mode === "corrupt") {
    await until(() => canvasSaveState(sid).reason === "recovery");
    check(gets === 0 && puts === 0, "Locked recovery prevents network state from overwriting it");
    check((await rows()).length > 0, "Authentication failure preserves ciphertext");
    check(document.querySelector('[role="status"]')?.textContent?.includes("sign in"), "Locked recovery explains how to retry");
  } else {
    await until(async () => draft() === note && canvasSaveState(sid).state === "saved" && (await rows()).length === 0);
    check(puts > 0, "Recovered changes are acknowledged by the server");
    check(draft() === note, "Reload restored the exact unsent note");
  }
  document.getElementById("result")!.textContent = `PASS: ${mode}`;
  return { mode, draft: draft(), gets, puts, status: canvasSaveState(sid), records: (await rows()).length };
};
