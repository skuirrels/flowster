// The Engine Worker (SPEC §5.1): hosts flowster-core compiled to wasm32 and
// owns the live documents. The UI thread talks to it only through messages.

import init, { WasmEngine } from "./wasm/flowster_wasm.js";
import type { EngineEvent, WorkerMessage } from "./protocol";

interface WorkerScope {
  postMessage(message: EngineEvent): void;
  onmessage: ((ev: MessageEvent<WorkerMessage>) => void) | null;
}
const scope = self as unknown as WorkerScope;

const engine: Promise<WasmEngine> = init().then(() => new WasmEngine());

function handle(e: WasmEngine, { id, req }: WorkerMessage): EngineEvent[] {
  if (req.kind === "open") {
    return e.open(id, req.fileName, new Uint8Array(req.bytes)) as EngineEvent[];
  }
  return e.request(id, req) as EngineEvent[];
}

scope.onmessage = async (ev) => {
  let events: EngineEvent[];
  try {
    events = handle(await engine, ev.data);
  } catch (err) {
    events = [{ kind: "error", id: ev.data.id, message: `engine failure: ${String(err)}` }];
  }
  for (const out of events) scope.postMessage(out);
};
