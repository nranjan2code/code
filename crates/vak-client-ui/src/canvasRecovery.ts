import type { CanvasCheckpoint } from "./canvasSyncState";

/** Only ciphertext lives in IndexedDB. The server holds the recovery key
 * under the conversation's erasure key; the browser key is memory-only. */
interface Envelope { version: 1; identity: string; iv: Uint8Array; bytes: ArrayBuffer }
interface KeyAnswer { identity: string; key: string }
interface Unlocked { identity: string; key: CryptoKey }

const DATABASE = "vak.canvas-recovery.v1";
const STORE = "pending";
const LIMIT = 6 * 1024 * 1024;

function uuid7(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  let time = Date.now();
  for (let i = 5; i >= 0; i--) { bytes[i] = time % 256; time = Math.floor(time / 256); }
  bytes[6] = 0x70 | (bytes[6] & 15); bytes[8] = 0x80 | (bytes[8] & 63);
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function database(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(new Error("Recovery storage is unavailable"));
    request.onblocked = () => reject(new Error("Recovery storage is blocked"));
  });
}

async function stored(key: string): Promise<Envelope | undefined> {
  const db = await database();
  try {
    return await new Promise((resolve, reject) => {
      const tx = db.transaction(STORE, "readonly");
      const request = tx.objectStore(STORE).get(key);
      let value: Envelope | undefined;
      request.onsuccess = () => { value = request.result; };
      tx.oncomplete = () => resolve(value);
      tx.onabort = tx.onerror = () => reject(new Error("Recovery could not be read"));
    });
  } finally { db.close(); }
}

async function replace(key: string, value: Envelope | null): Promise<void> {
  const db = await database();
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE, "readwrite");
      if (value) tx.objectStore(STORE).put(value, key);
      else tx.objectStore(STORE).delete(key);
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () => reject(new Error("Recovery could not be saved"));
    });
  } finally { db.close(); }
}

export function createCanvasRecovery<Op, Snapshot, Stored>(options: {
  namespace: () => string;
  key: (conversation: string) => Promise<KeyAnswer>;
  validate: (value: unknown) => value is CanvasCheckpoint<Op, Snapshot, Stored>;
}) {
  const keys = new Map<string, Promise<Unlocked>>();
  const chains = new Map<string, Promise<void>>();
  let writer: Promise<string> | undefined;

  // sessionStorage holds a writer identifier, never a key or content. A
  // duplicated tab may inherit it, so an origin Web Lock reserves each writer.
  // A duplicate gets a fresh writer; a reload can reclaim its released lock.
  function writerId(): Promise<string> {
    return writer ??= new Promise((resolve, reject) => {
      if (!navigator.locks) { reject(new Error("Recovery needs browser locking")); return; }
      let id: string;
      try {
        const previous = sessionStorage.getItem("vak.canvasRecovery.writer");
        id = previous && /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(previous) ? previous : uuid7();
      } catch { reject(new Error("Recovery writer is unavailable")); return; }
      const claim = (candidate: string) => {
        void navigator.locks.request(`vak.canvas-recovery:${candidate}`, { ifAvailable: true }, async (lock) => {
          if (!lock) { claim(uuid7()); return; }
          try { sessionStorage.setItem("vak.canvasRecovery.writer", candidate); }
          catch { reject(new Error("Recovery writer is unavailable")); return; }
          resolve(candidate);
          // The browser releases the lock when this document is destroyed.
          await new Promise<void>(() => undefined);
        }).catch(() => reject(new Error("Recovery writer is unavailable")));
      };
      claim(id);
    });
  }

  async function recordId(conversation: string): Promise<string> {
    return JSON.stringify([options.namespace(), conversation, await writerId()]);
  }
  function unlock(conversation: string): Promise<Unlocked> {
    const scope = JSON.stringify([options.namespace(), conversation]);
    let pending = keys.get(scope);
    if (!pending) {
      pending = options.key(conversation).then(async (answer) => {
        const raw = Uint8Array.from(atob(answer.key), (c) => c.charCodeAt(0));
        if (raw.length !== 32 || !answer.identity) throw new Error("Recovery key is invalid");
        try {
          return { identity: answer.identity, key: await crypto.subtle.importKey("raw", raw, "AES-GCM", false, ["encrypt", "decrypt"]) };
        } finally { raw.fill(0); }
      }).catch((error) => { keys.delete(scope); throw error; });
      keys.set(scope, pending);
    }
    return pending;
  }
  const aad = (id: string, identity: string) => new TextEncoder().encode(JSON.stringify([DATABASE, id, identity]));

  return {
    async load(conversation: string): Promise<CanvasCheckpoint<Op, Snapshot, Stored> | null> {
      const id = await recordId(conversation);
      // Finish this document's writes before reading its journal again.
      await chains.get(id)?.catch(() => undefined);
      const value = await stored(id);
      if (!value) {
        // Provision while connected so later offline edits can be encrypted.
        // A failed warmup never blocks the canonical initial Canvas read.
        void unlock(conversation).catch(() => undefined);
        return null;
      }
      if (value.version !== 1 || !(value.iv instanceof Uint8Array) || value.iv.length !== 12
        || !(value.bytes instanceof ArrayBuffer) || value.bytes.byteLength > LIMIT) throw new Error("Recovery format is invalid");
      const key = await unlock(conversation);
      if (value.identity !== key.identity) throw new Error("Recovery belongs to another installation");
      const plain = await crypto.subtle.decrypt({ name: "AES-GCM", iv: new Uint8Array(value.iv), additionalData: aad(id, key.identity) }, key.key, value.bytes);
      const parsed: unknown = JSON.parse(new TextDecoder().decode(plain));
      if (!options.validate(parsed)) throw new Error("Recovery content is invalid");
      return parsed;
    },
    save(conversation: string, checkpoint: CanvasCheckpoint<Op, Snapshot, Stored> | null): Promise<void> {
      // Capture synchronously: a later edit must not change this checkpoint
      // while its encryption awaits a key or an earlier transaction.
      const plain = checkpoint === null ? null : new TextEncoder().encode(JSON.stringify(checkpoint));
      if (plain && plain.byteLength > LIMIT - 16) return Promise.reject(new Error("Recovery is too large"));
      return recordId(conversation).then((id) => {
        const next = (chains.get(id) ?? Promise.resolve()).catch(() => undefined).then(async () => {
          if (plain === null) { await replace(id, null); return; }
          const key = await unlock(conversation);
          const iv = crypto.getRandomValues(new Uint8Array(12));
          const bytes = await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: aad(id, key.identity) }, key.key, plain);
          await replace(id, { version: 1, identity: key.identity, iv, bytes });
        });
        chains.set(id, next);
        return next;
      });
    },
  };
}
