import { useCallback, useEffect, useRef, useState, type DragEvent } from "react";

import { CanvasView, PX_PER_INCH, type CanvasHandle, type Tool } from "./canvas/CanvasView";
import { PackageView } from "./chrome/PackageView";
import type { EngineClient } from "./engine/client";
import type { Command, DisplayList, DocInfo, OpenResult, PackageInfo, PageInfo, SyncState } from "./engine/protocol";
import { inTauri, pickDocument, saveDocument, type PickedFile } from "./host";

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

/** Drawing tools, with Visio's shortcuts. */
const TOOLS: { tool: Tool; label: string; key: string }[] = [
  { tool: "select", label: "Pointer", key: "1" },
  { tool: "rectangle", label: "Rectangle", key: "8" },
  { tool: "ellipse", label: "Ellipse", key: "9" },
  { tool: "line", label: "Line", key: "6" },
];

interface TextEdit {
  id: number;
  text: string;
  rect: [number, number, number, number];
}

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
  const [tool, setTool] = useState<Tool>("select");
  const [selection, setSelection] = useState<number[]>([]);
  const [editing, setEditing] = useState<TextEdit | null>(null);
  const [savedAs, setSavedAs] = useState<string | null>(null);
  /** Bumped after every change so the page is fetched again. */
  const [version, setVersion] = useState(0);
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
      .then((list) => {
        if (cancelled) return;
        setDl(list);
        // Drop selected shapes that no longer exist (deleted, undone).
        setSelection((sel) => sel.filter((id) => list.shapes.some((b) => b.id === id)));
      })
      .catch((e: unknown) => !cancelled && setError(e instanceof Error ? e.message : String(e)));
    return () => {
      cancelled = true;
    };
  }, [doc, page, engine, version]);

  const refreshInfo = useCallback(
    async (docId: string) => {
      const info = await engine.query(docId, { kind: "docInfo" });
      setDoc((d) => (d && d.open.doc === docId ? { ...d, info } : d));
    },
    [engine],
  );

  /** Shows a document the engine has just opened or created. */
  const showDoc = useCallback(
    async (open: OpenResult) => {
      const [pkg, info, pages, derived] = await Promise.all([
        engine.query(open.doc, { kind: "packageParts" }),
        engine.query(open.doc, { kind: "docInfo" }),
        engine.query(open.doc, { kind: "pages" }),
        engine.query(open.doc, { kind: "derivedStateHash" }),
      ]);
      const first = pages.findIndex((p) => !p.background);
      setPage(first >= 0 ? first : 0);
      setSelection([]);
      setEditing(null);
      setTool("select");
      setDoc({ open, pkg, info, pages });
      setMode(pages.length > 0 ? "drawing" : "package");
      setHash(derived.hash);
    },
    [engine],
  );

  const onNew = useCallback(async () => {
    setError(null);
    try {
      if (doc) await engine.close(doc.open.doc);
      await showDoc(await engine.newDrawing());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [doc, engine, showDoc]);

  const openFile = useCallback(
    async (file: PickedFile) => {
      setBusy(true);
      setError(null);
      try {
        if (doc) await engine.close(doc.open.doc);
        const open = await engine.open(file.bytes, file.fileName);
        await showDoc(open);
      } catch (e) {
        setDoc(null);
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [doc, engine, showDoc],
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
    async (cmd: Command) => {
      if (!doc) return;
      try {
        const changes = await engine.exec(doc.open.doc, cmd);
        setVersion((v) => v + 1);
        if (changes?.created) setSelection([changes.created[1]]);
        await refreshInfo(doc.open.doc);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [doc, engine, refreshInfo],
  );

  const onSave = useCallback(async () => {
    if (!doc) return;
    setError(null);
    try {
      const bytes = await engine.save(doc.open.doc);
      const stem = doc.info.fileName.replace(/\.[^.]+$/, "");
      const saved = await saveDocument(`${stem}.vsdx`, bytes);
      if (saved) setSavedAs(saved);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [doc, engine]);

  const deleteSelection = useCallback(() => {
    if (selection.length > 0) void exec({ kind: "deleteShapes", page, shapes: selection });
  }, [exec, page, selection]);

  const editText = useCallback(
    (id: number) => {
      const rect = canvas.current?.screenBounds(id);
      const box = dl?.shapes.find((b) => b.id === id);
      if (!rect || !box) return;
      setSelection([id]);
      setEditing({ id, text: box.text ?? "", rect });
    },
    [dl],
  );

  const commitText = useCallback(() => {
    if (!editing) return;
    const box = dl?.shapes.find((b) => b.id === editing.id);
    if (box && (box.text ?? "") !== editing.text) {
      void exec({ kind: "setText", page, shape: editing.id, text: editing.text });
    }
    setEditing(null);
  }, [dl, editing, exec, page]);

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
      const inInput = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;
      const key = e.key.toLowerCase();
      const toolKey = mod ? TOOLS.find((t) => t.key === e.key) : undefined;
      if (mod && key === "o") {
        e.preventDefault();
        void onOpen();
      } else if (mod && key === "s") {
        e.preventDefault();
        void onSave();
      } else if (mod && key === "n") {
        e.preventDefault();
        void onNew();
      } else if (toolKey && doc && !inInput) {
        e.preventDefault();
        setTool(toolKey.tool);
      } else if ((e.key === "Delete" || e.key === "Backspace") && !inInput && doc) {
        e.preventDefault();
        deleteSelection();
      } else if (e.key === "Escape" && !inInput) {
        setTool("select");
        setSelection([]);
      } else if (e.key === "F2" && !inInput && selection.length === 1) {
        e.preventDefault();
        editText(selection[0]);
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
  }, [doc, exec, onOpen, onNew, onSave, stepZoom, deleteSelection, editText, selection]);

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
        <button type="button" onClick={onNew} title="New drawing (Ctrl+N)">
          New
        </button>
        <button type="button" onClick={onOpen} disabled={busy} title="Open (Ctrl+O)">
          {busy ? "Opening…" : "Open…"}
        </button>
        <button type="button" onClick={onSave} disabled={!doc || doc.pages.length === 0} title="Save as .vsdx (Ctrl+S)">
          Save
        </button>
        <button type="button" onClick={() => exec({ kind: "undo" })} disabled={!undo.canUndo} title="Undo (Ctrl+Z)">
          Undo
        </button>
        <button type="button" onClick={() => exec({ kind: "redo" })} disabled={!undo.canRedo} title="Redo (Ctrl+Y)">
          Redo
        </button>
        {doc && mode === "drawing" && (
          <div className="segmented tools" role="toolbar" aria-label="Tools">
            {TOOLS.map((t) => (
              <button
                type="button"
                key={t.tool}
                aria-pressed={tool === t.tool}
                onClick={() => setTool(t.tool)}
                title={`${t.label} (Ctrl+${t.key})`}
              >
                {t.label}
              </button>
            ))}
          </div>
        )}
        {doc && mode === "drawing" && (
          <button type="button" onClick={deleteSelection} disabled={selection.length === 0} title="Delete (Del)">
            Delete
          </button>
        )}
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
            <p>Create a new drawing, or open a Visio drawing (.vsdx).</p>
            <p className="empty-actions">
              <button type="button" className="primary" onClick={onNew}>
                New drawing
              </button>
              <button type="button" onClick={onOpen}>
                Open…
              </button>
            </p>
            <p className="hint">Or drop a file anywhere in this window.</p>
          </div>
        )}
        {doc && mode === "package" && <PackageView open={doc.open} pkg={doc.pkg} info={doc.info} />}
        {doc && mode === "drawing" && (
          <div className="drawing">
            {dl ? (
              <CanvasView
                ref={canvas}
                dl={dl}
                tool={tool}
                selection={selection}
                onSelection={setSelection}
                onCommand={(c) => {
                  // Back to the pointer as soon as a shape is drawn (not when the
                  // engine replies, which could undo a tool picked meanwhile).
                  if (c.kind === "addShape") setTool("select");
                  void exec(c);
                }}
                onEditText={editText}
                onZoomChange={setZoom}
              />
            ) : (
              <div className="empty">
                <p>{doc.pages.length === 0 ? "This file has no drawing pages." : "Loading page…"}</p>
              </div>
            )}
            {editing && (
              <textarea
                className="text-editor"
                autoFocus
                value={editing.text}
                style={{
                  left: editing.rect[0],
                  top: editing.rect[1],
                  width: Math.max(80, editing.rect[2]),
                  height: Math.max(32, editing.rect[3]),
                }}
                onChange={(e) => setEditing({ ...editing, text: e.target.value })}
                onBlur={commitText}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setEditing(null);
                  else if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    commitText();
                  }
                }}
              />
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
        {savedAs && <span>Saved as {savedAs}</span>}
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
