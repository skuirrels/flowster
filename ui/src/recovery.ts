// AutoRecover: a copy of the open drawing with unsaved changes, kept in the
// WebView's IndexedDB (no host access needed) a few seconds after each change.
// It is cleared when the drawing is saved or deliberately discarded, so one that
// is still there at start-up was lost to a crash or a forced close.

export interface RecoveryCopy {
  /** The drawing's file name (what Save As will suggest). */
  fileName: string;
  title: string;
  /** When the copy was taken, ms since the epoch. */
  savedAt: number;
  /** The drawing as `.vsdx`. */
  bytes: ArrayBuffer;
}

const DB = "flowster";
const STORE = "recovery";
const KEY = "current";

function open(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function run<T>(mode: IDBTransactionMode, op: (s: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const db = await open();
  try {
    return await new Promise<T>((resolve, reject) => {
      const tx = db.transaction(STORE, mode);
      const req = op(tx.objectStore(STORE));
      tx.oncomplete = () => resolve(req.result);
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
  } finally {
    db.close();
  }
}

/** Storage can be unavailable (private windows, blocked site data): recovery then quietly does nothing. */
export async function keepRecoveryCopy(copy: RecoveryCopy): Promise<void> {
  try {
    await run("readwrite", (s) => s.put(copy, KEY));
  } catch {
    // Best effort.
  }
}

export async function recoveryCopy(): Promise<RecoveryCopy | null> {
  try {
    const copy = await run<RecoveryCopy | undefined>("readonly", (s) => s.get(KEY));
    return copy && copy.bytes instanceof ArrayBuffer ? copy : null;
  } catch {
    return null;
  }
}

export async function clearRecoveryCopy(): Promise<void> {
  try {
    await run("readwrite", (s) => s.delete(KEY));
  } catch {
    // Best effort.
  }
}
