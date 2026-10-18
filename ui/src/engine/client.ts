// UI-thread side of the engine protocol: request/response correlation plus an
// event stream for panes. One client (and one worker) per window.

import type {
  Command,
  DocId,
  EngineEvent,
  EngineRequest,
  OpenResult,
  Query,
  QueryResult,
  WorkerMessage,
} from "./protocol";

type Pending = { resolve: (v: unknown) => void; reject: (e: Error) => void };
export type EngineListener = (event: EngineEvent) => void;

export class EngineError extends Error {}

export class EngineClient {
  private readonly worker: Worker;
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly listeners = new Set<EngineListener>();

  constructor() {
    this.worker = new Worker(new URL("./engine.worker.ts", import.meta.url), { type: "module", name: "flowster-engine" });
    this.worker.onmessage = (m: MessageEvent<EngineEvent>) => this.dispatch(m.data);
    this.worker.onerror = (e) => {
      const err = new EngineError(`engine worker failed: ${e.message}`);
      for (const p of this.pending.values()) p.reject(err);
      this.pending.clear();
    };
  }

  /** Subscribe to every event (model changes, undo state, sync state, …). */
  on(listener: EngineListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  open(bytes: ArrayBuffer, fileName: string): Promise<OpenResult> {
    return this.request<OpenResult>({ kind: "open", bytes, fileName }, [bytes]);
  }

  query<Q extends Query>(doc: DocId, q: Q): Promise<QueryResult<Q>> {
    return this.request<QueryResult<Q>>({ kind: "query", doc, q });
  }

  exec(doc: DocId, cmd: Command): Promise<void> {
    return this.request<void>({ kind: "exec", doc, cmd });
  }

  close(doc: DocId): Promise<void> {
    return this.request<void>({ kind: "close", doc });
  }

  request<T>(req: EngineRequest, transfer: Transferable[] = []): Promise<T> {
    const id = this.nextId++;
    const msg: WorkerMessage = { id, req };
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.worker.postMessage(msg, transfer);
    });
  }

  terminate(): void {
    this.worker.terminate();
  }

  private dispatch(event: EngineEvent): void {
    for (const l of this.listeners) l(event);
    if (event.kind === "reply" || (event.kind === "error" && event.id !== null)) {
      const p = this.pending.get(event.id as number);
      if (!p) return;
      this.pending.delete(event.id as number);
      if (event.kind === "reply") p.resolve(event.result);
      else p.reject(new EngineError(event.message));
    }
  }
}
