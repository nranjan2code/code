import assert from "node:assert/strict";
import { createCanvasRecovery } from "../src/canvasRecovery.ts";

// A deterministic IndexedDB/lock adapter exercises real Web Crypto while
// keeping transaction failure and reload timing under the test's control.
const records = new Map(), session = new Map(), locks = new Set();
let quota = false, authorized = true, keyCalls = 0;
Object.defineProperty(globalThis, "sessionStorage", { value: {
  getItem: (key) => session.get(key) ?? null,
  setItem: (key, value) => session.set(key, value),
} });
Object.defineProperty(globalThis, "navigator", { value: { locks: {
  request: async (name, _options, callback) => {
    if (locks.has(name)) return callback(null);
    locks.add(name);
    return callback({ name });
  },
} } });
globalThis.indexedDB = {
  open() {
    const request = {};
    queueMicrotask(() => {
      request.result = {
        close() {},
        transaction() {
          const tx = {};
          const complete = () => queueMicrotask(() => tx.oncomplete?.());
          tx.objectStore = () => ({
            get(key) {
              const read = {};
              queueMicrotask(() => { read.result = structuredClone(records.get(key)); read.onsuccess?.(); complete(); });
              return read;
            },
            put(value, key) {
              queueMicrotask(() => {
                if (quota) { tx.onabort?.(); return; }
                records.set(key, structuredClone(value)); complete();
              });
            },
            delete(key) {
              queueMicrotask(() => {
                if (quota) { tx.onabort?.(); return; }
                records.delete(key); complete();
              });
            },
          });
          return tx;
        },
      };
      request.onsuccess?.();
    });
    return request;
  },
};
const checkpoint = { base: 3, pending: [{ draft: "private recovery note" }], snapshot: { draft: "private recovery note" } };
const journal = () => createCanvasRecovery({
  namespace: () => "test-origin",
  key: async () => {
    keyCalls++;
    if (!authorized) throw new Error("erased or signed out");
    return { identity: "server-conversation-identity", key: btoa(String.fromCharCode(...new Uint8Array(32).fill(19))) };
  },
  validate: (value) => value?.base === 3 && Array.isArray(value.pending),
});
const first = journal();
assert.equal(await first.load("c"), null);
await first.save("c", checkpoint);
assert.equal(keyCalls, 1, "successful initial read warms the memory-only key");
const recordKey = [...records.keys()][0], envelope = records.get(recordKey);
const originalIdentity = envelope.identity;
assert.ok(envelope.bytes instanceof ArrayBuffer);
assert.equal(new TextDecoder().decode(envelope.bytes).includes("private recovery note"), false);
assert.equal("key" in envelope, false);

// A second live document inheriting the same sessionStorage writer cannot
// read or overwrite the first document's checkpoint.
const duplicate = journal();
assert.equal(await duplicate.load("c"), null);
await duplicate.save("c", checkpoint);
assert.equal(records.size, 2);

// Reload destroys the old document's locks and keys but retains writer id.
session.set("vak.canvasRecovery.writer", JSON.parse(recordKey)[2]);
locks.clear(); authorized = false;
const reloaded = journal();
await assert.rejects(reloaded.load("c"));
assert.equal(records.has(recordKey), true, "locked recovery retains ciphertext");
authorized = true;
assert.deepEqual(await reloaded.load("c"), checkpoint);
const otherKeyParts = JSON.parse(recordKey); otherKeyParts[1] = "other";
const otherKey = JSON.stringify(otherKeyParts);
records.set(otherKey, structuredClone(records.get(recordKey)));
await assert.rejects(reloaded.load("other"), "AAD rejects moving ciphertext to another conversation");
records.delete(otherKey);
new Uint8Array(records.get(recordKey).bytes)[0] ^= 1;
await assert.rejects(reloaded.load("c"), "authenticated encryption rejects corrupt bytes");
new Uint8Array(records.get(recordKey).bytes)[0] ^= 1;
records.get(recordKey).identity = "different-install";
await assert.rejects(reloaded.load("c"), "installation identity cannot change");
records.get(recordKey).identity = originalIdentity;
assert.deepEqual(await reloaded.load("c"), checkpoint);
quota = true;
await assert.rejects(reloaded.save("c", null));
assert.equal(records.has(recordKey), true, "failed cleanup does not silently discard ciphertext");
quota = false;
await reloaded.save("c", null);
assert.equal(records.has(recordKey), false);
assert.equal(records.size, 1, "acknowledging one writer leaves the other writer untouched");
console.log("Canvas recovery encryption, reload, isolation and storage failures passed");
