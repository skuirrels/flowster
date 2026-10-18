import { useCallback, useEffect, useRef, useState, type DragEvent } from "react";

import { CanvasView, PX_PER_INCH, type CanvasHandle } from "./canvas/CanvasView";
import { PackageView } from "./chrome/PackageView";
import type { EngineClient } from "./engine/client";
import type { DisplayList, DocInfo, OpenResult, PackageInfo, PageInfo, SyncState } from "./engine/protocol";
import { inTauri, pickDocument, type PickedFile } from "./host";

interface OpenDoc {
  open: OpenResult;
  pkg: PackageInfo;
  info: DocInfo;
  pages: PageInfo[];
}

interface UndoInfo {
  canUndo: boolean;
  canRedo: boolean;
  labels: string[];
}

type Mode = "drawing" | "package";

const ZOOM_STEPS = [0.1, 0.25, 0.5, 0.75, 1, 1.5, 2, 3, 4, 6, 8];

export function App({ engine }: { engine: EngineClient }) {
  const [doc, setDoc] = useState<OpenDoc | null>(null);
  const [page, setPage] = useState(0);
  const [dl, setDl] = useState<DisplayList | null>(null);
  const [mode, setMode] = useState<Mode>("drawing");
  const [zoom, setZoom] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [undo, setUndo] = useState<UndoInfo>({ canUndo: false, canRedo: false, labels: [] });
  const [sync, setSync] = useState<SyncState | null>(null);
  const [hash, setHash] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const canvas = useRef<CanvasHandle>(null);

  useEffect(
    () =>
      engine.on((e) => {
        if (e.kind === "undoState") setUndo({ canUndo: e.canUndo, canRedo: e.canRedo, labels: e.labels });
        if (e.kind === "syncState") setSync(e.state);
      }),
    [engine],
  );

  // Fetch the display list whenever the document or page changes.
  useEffect(() => {
    if (!doc || doc.pages.length === 0) {
      setDl(null);
      return;
    }
    let cancelled = false;
    engine
      .query(doc.open.doc, { kind: "pageDisplayList", page })
      .then((list) => !cancelled && setDl(list))
      .catch((e: unknown) => !cancelled && setError(e instanceof Error ? e.message : String(e)));
    return () => {
      cancelled = true;
    };
  }, [doc, page, engine]);

  const refreshInfo = useCallback(
    async (docId: string) => {
      const info = await engine.query(docId, { kind: "docInfo" });
      setDoc((d) => (d && d.open.doc === docId ? { ...d, info } : d));
    },
    [engine],
  );

  const openFile = useCallback(
    async (file: PickedFile) => {
      setBusy(true);
      setError(null);
      try {
        if (doc) await engine.close(doc.open.doc);
        const open = await engine.open(file.bytes, file.fileName);
        const [pkg, info, pages, derived] = await Promise.all([
          engine.query(open.doc, { kind: "packageParts" }),
          engine.query(open.doc, { kind: "docInfo" }),
          engine.query(open.doc, { kind: "pages" }),
          engine.query(open.doc, { kind: "derivedStateHash" }),
        ]);
        const first = pages.findIndex((p) => !p.background);
        setPage(first >= 0 ? first : 0);
        setDoc({ open, pkg, info, pages });
        setMode(pages.length > 0 ? "drawing" : "package");
        setHash(derived.hash);
      } catch (e) {
        setDoc(null);
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [doc, engine],
  );

  const onOpen = useCallback(async () => {
    try {
      const picked = await pickDocument();
      if (picked) await openFile(picked);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [openFile]);

  const onDrop = useCallback(
    async (ev: DragEvent) => {
      ev.preventDefault();
      setDragging(false);
      const file = ev.dataTransfer.files[0];
      if (file) await openFile({ fileName: file.name, bytes: await file.arrayBuffer() });
    },
    [openFile],
  );

  const exec = useCallback(
    async (cmd: Parameters<EngineClient["exec"]>[1]) => {
      if (!doc) return;
      try {
        await engine.exec(doc.open.doc, cmd);
        await refreshInfo(doc.open.doc);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [doc, engine, refreshInfo],
  );

  const stepZoom = useCallback(
    (dir: 1 | -1) => {
      const pct = zoom / PX_PER_INCH;
      const next = dir > 0 ? ZOOM_STEPS.find((s) => s > pct * 1.001) : [...ZOOM_STEPS].reverse().find((s) => s < pct * 0.999);
      if (next) canvas.current?.setZoom(next * PX_PER_INCH);
    },
    [zoom],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      const inInput = e.target instanceof HTMLInputElement;
      const key = e.key.toLowerCase();
      if (mod && key === "o") {
        e.preventDefault();
        void onOpen();
      } else if (mod && key === "z" && !inInput) {
        e.preventDefault();
        void exec({ kind: e.shiftKey ? "redo" : "undo" });
      } else if (mod && key === "y" && !inInput) {
        e.preventDefault();
        void exec({ kind: "redo" });
      } else if (mod && e.shiftKey && key === "w") {
        // Fit page to window (Visio: Ctrl+Shift+W).
        e.preventDefault();
        canvas.current?.fit();
      } else if (mod && (key === "=" || key === "+") && !inInput) {
        e.preventDefault();
        stepZoom(1);
      } else if (mod && key === "-" && !inInput) {
        e.preventDefault();
        stepZoom(-1);
      } else if (mod && (e.key === "PageDown" || e.key === "PageUp") && doc) {
        e.preventDefault();
        const n = doc.pages.length;
        setPage((p) => (p + (e.key === "PageDown" ? 1 : n - 1)) % n);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [doc, exec, onOpen, stepZoom]);

  return (
    <div
      className={`app${dragging ? " dragging" : ""}`}
      onDragOver={(e) => {
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={onDrop}
    >
      <header className="toolbar">
        <span className="brand">Flowster</span>
        <button type="button" className="primary" onClick={onOpen} disabled={busy} title="Open (Ctrl+O)">
          {busy ? "Opening…" : "Open…"}
        </button>
        <button type="button" onClick={() => exec({ kind: "undo" })} disabled={!undo.canUndo} title="Undo (Ctrl+Z)">
          Undo
        </button>
        <button type="button" onClick={() => exec({ kind: "redo" })} disabled={!undo.canRedo} title="Redo (Ctrl+Y)">
          Redo
        </button>
        {doc && (
          <div className="segmented" role="tablist" aria-label="View">
            <button type="button" role="tab" aria-selected={mode === "drawing"} onClick={() => setMode("drawing")}>
              Drawing
            </button>
            <button type="button" role="tab" aria-selected={mode === "package"} onClick={() => setMode("package")}>
              Package
            </button>
          </div>
        )}
        {doc && (
          <TitleField
            key={doc.open.doc + doc.info.meta.title}
            value={doc.info.meta.title ?? ""}
            onCommit={(v) => exec({ kind: "setDocProp", key: "title", value: v })}
          />
        )}
      </header>

      <main className="main">
        {error && (
          <div className="error" role="alert">
            {error}
            <button type="button" className="link" onClick={() => setError(null)}>
              Dismiss
            </button>
          </div>
        )}
        {!doc && !error && (
          <div className="empty">
            <p>Open a Visio drawing (.vsdx) to view it.</p>
            <p className="hint">Or drop a file anywhere in this window.</p>
          </div>
        )}
        {doc && mode === "package" && <PackageView open={doc.open} pkg={doc.pkg} info={doc.info} />}
        {doc && mode === "drawing" && (
          <div className="drawing">
            {dl ? (
              <CanvasView ref={canvas} dl={dl} onZoomChange={setZoom} />
            ) : (
              <div className="empty">
                <p>{doc.pages.length === 0 ? "This file has no drawing pages." : "Loading page…"}</p>
              </div>
            )}
            <div className="page-bar">
              <div className="page-tabs" role="tablist" aria-label="Pages">
                {doc.pages.map((p) => (
                  <button
                    type="button"
                    role="tab"
                    key={p.id}
                    aria-selected={p.index === page}
                    className={p.background ? "background" : undefined}
                    onClick={() => setPage(p.index)}
                    title={p.background ? "Background page" : undefined}
                  >
                    {p.name}
                  </button>
                ))}
              </div>
              <div className="zoom">
                <button type="button" onClick={() => stepZoom(-1)} title="Zoom out (Ctrl+-)" aria-label="Zoom out">
                  −
                </button>
                <span className="zoom-pct">{Math.round((zoom / PX_PER_INCH) * 100)}%</span>
                <button type="button" onClick={() => stepZoom(1)} title="Zoom in (Ctrl++)" aria-label="Zoom in">
                  +
                </button>
                <button type="button" onClick={() => canvas.current?.fit()} title="Fit page (Ctrl+Shift+W)">
                  Fit
                </button>
              </div>
            </div>
          </div>
        )}
      </main>

      <footer className="status">
        <span>{inTauri ? "Desktop" : "Browser"}</span>
        {dl && (
          <span>
            {dl.page.name}: {dl.page.width.toFixed(2)} × {dl.page.height.toFixed(2)} in, {dl.items.length} items
          </span>
        )}
        {sync && <span>{sync === "local" ? "Local document" : sync}</span>}
        {undo.labels[0] && <span>Undo: {undo.labels[0]}</span>}
        {dl && dl.warnings.length > 0 && <span title={dl.warnings.join("\n")}>{dl.warnings.length} warnings</span>}
        {hash && <span title="Derived-state determinism hash">State hash {hash}</span>}
      </footer>
    </div>
  );
}

function TitleField({ value, onCommit }: { value: string; onCommit: (v: string) => void }) {
  const [draft, setDraft] = useState(value);
  const commit = () => {
    if (draft !== value) onCommit(draft);
  };
  return (
    <label className="title-field">
      <span>Title</span>
      <input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && (e.currentTarget as HTMLInputElement).blur()}
      />
    </label>
  );
}
