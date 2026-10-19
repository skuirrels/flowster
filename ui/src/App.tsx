import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent } from "react";

import { CanvasView, PX_PER_INCH, type CanvasHandle, type Tool } from "./canvas/CanvasView";
import { Icon, shortcut, type IconName } from "./chrome/icons";
import { Inspector } from "./chrome/Inspector";
import { MenuBar, type Menu } from "./chrome/MenuBar";
import { PackageView } from "./chrome/PackageView";
import { AboutDialog, ShortcutsDialog, Welcome } from "./chrome/Welcome";
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
const TOOLS: { tool: Tool; label: string; key: string; icon: IconName }[] = [
  { tool: "select", label: "Pointer", key: "1", icon: "pointer" },
  { tool: "rectangle", label: "Rectangle", key: "8", icon: "rectangle" },
  { tool: "ellipse", label: "Ellipse", key: "9", icon: "ellipse" },
  { tool: "line", label: "Line", key: "6", icon: "line" },
];

interface TextEdit {
  id: number;
  text: string;
  rect: [number, number, number, number];
}

/** A toolbar button: icon only, with a tooltip naming it and its shortcut. */
function ToolButton({
  icon,
  label,
  keys,
  onClick,
  disabled,
  pressed,
}: {
  icon: IconName;
  label: string;
  keys?: string;
  onClick: () => void;
  disabled?: boolean;
  pressed?: boolean;
}) {
  return (
    <button
      type="button"
      className="tb"
      aria-label={label}
      aria-pressed={pressed}
      data-tip={keys ? `${label}  ${shortcut(keys)}` : label}
      onClick={onClick}
      disabled={disabled}
    >
      <Icon name={icon} />
    </button>
  );
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
  const [dirty, setDirty] = useState(false);
  const [inspector, setInspector] = useState(true);
  const [dialog, setDialog] = useState<"shortcuts" | "about" | null>(null);
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

  // Fetch the display list whenever the document, page or content changes.
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
      setDirty(false);
      setSavedAs(null);
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
        await showDoc(await engine.open(file.bytes, file.fileName));
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
        setDirty(true);
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
      if (saved) {
        setSavedAs(saved);
        setDirty(false);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [doc, engine]);

  const deleteSelection = useCallback(() => {
    if (selection.length > 0) void exec({ kind: "deleteShapes", page, shapes: selection });
  }, [exec, page, selection]);

  const selectAll = useCallback(() => {
    if (dl) setSelection(dl.shapes.map((b) => b.id));
  }, [dl]);

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

  const drawing = !!doc && mode === "drawing" && !!dl;

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
      } else if (toolKey && drawing && !inInput) {
        e.preventDefault();
        setTool(toolKey.tool);
      } else if (mod && key === "a" && drawing && !inInput) {
        e.preventDefault();
        selectAll();
      } else if ((e.key === "Delete" || e.key === "Backspace") && !inInput && drawing) {
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
  }, [doc, drawing, exec, onOpen, onNew, onSave, stepZoom, deleteSelection, selectAll, editText, selection]);

  const menus: Menu[] = useMemo(
    () => [
      {
        label: "File",
        items: [
          { label: "New drawing", icon: "new", keys: "Ctrl+N", run: () => void onNew() },
          { label: "Open…", icon: "open", keys: "Ctrl+O", run: () => void onOpen(), disabled: busy },
          "separator",
          { label: "Save as .vsdx…", icon: "save", keys: "Ctrl+S", run: () => void onSave(), disabled: !doc?.pages.length },
        ],
      },
      {
        label: "Edit",
        items: [
          {
            label: undo.labels[0] ? `Undo ${undo.labels[0]}` : "Undo",
            icon: "undo",
            keys: "Ctrl+Z",
            run: () => void exec({ kind: "undo" }),
            disabled: !undo.canUndo,
          },
          { label: "Redo", icon: "redo", keys: "Ctrl+Y", run: () => void exec({ kind: "redo" }), disabled: !undo.canRedo },
          "separator",
          { label: "Select all", icon: "selectAll", keys: "Ctrl+A", run: selectAll, disabled: !drawing },
          { label: "Edit text", icon: "text", keys: "F2", run: () => editText(selection[0]), disabled: selection.length !== 1 },
          { label: "Delete", icon: "trash", keys: "Del", run: deleteSelection, disabled: selection.length === 0 },
        ],
      },
      {
        label: "View",
        items: [
          { label: "Zoom in", icon: "zoomIn", keys: "Ctrl++", run: () => stepZoom(1), disabled: !drawing },
          { label: "Zoom out", icon: "zoomOut", keys: "Ctrl+-", run: () => stepZoom(-1), disabled: !drawing },
          { label: "Fit page", icon: "fit", keys: "Ctrl+Shift+W", run: () => canvas.current?.fit(), disabled: !drawing },
          "separator",
          { label: "Inspector", checked: inspector, run: () => setInspector((v) => !v), disabled: !doc },
          "separator",
          { label: "Drawing", checked: mode === "drawing", run: () => setMode("drawing"), disabled: !doc },
          { label: "Package parts", checked: mode === "package", run: () => setMode("package"), disabled: !doc },
        ],
      },
      {
        label: "Insert",
        items: TOOLS.slice(1).map((t) => ({
          label: t.label,
          icon: t.icon,
          keys: `Ctrl+${t.key}`,
          run: () => {
            setMode("drawing");
            setTool(t.tool);
          },
          disabled: !doc,
        })),
      },
      {
        label: "Help",
        items: [
          { label: "Keyboard shortcuts", icon: "keyboard", run: () => setDialog("shortcuts") },
          { label: "About Flowster", icon: "info", run: () => setDialog("about") },
        ],
      },
    ],
    [busy, deleteSelection, doc, drawing, editText, exec, inspector, mode, onNew, onOpen, onSave, selectAll, selection, stepZoom, undo],
  );

  const title = doc?.info.meta.title || doc?.info.fileName.replace(/\.[^.]+$/, "") || "";

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
      <header className="titlebar">
        <div className="brand">
          <span className="brand-mark">
            <Icon name="logo" size={16} />
          </span>
          <span className="brand-name">Flowster</span>
        </div>
        <MenuBar menus={menus} />
        <div className="doc-title">
          {doc && (
            <>
              <span className="doc-name" title={doc.info.fileName}>
                {title}
              </span>
              <span className={`doc-state${dirty ? " dirty" : ""}`}>{dirty ? "Edited" : savedAs ? "Saved" : "Opened"}</span>
            </>
          )}
        </div>
        <div className="titlebar-end">
          {doc && (
            <div className="seg" role="tablist" aria-label="View">
              <button type="button" role="tab" aria-selected={mode === "drawing"} onClick={() => setMode("drawing")}>
                <Icon name="drawing" size={15} /> Drawing
              </button>
              <button type="button" role="tab" aria-selected={mode === "package"} onClick={() => setMode("package")}>
                <Icon name="package" size={15} /> Package
              </button>
            </div>
          )}
        </div>
      </header>

      {doc && (
        <div className="toolbar" role="toolbar" aria-label="Commands">
          <div className="tb-group">
            <ToolButton icon="new" label="New drawing" keys="Ctrl+N" onClick={() => void onNew()} />
            <ToolButton icon="open" label="Open" keys="Ctrl+O" onClick={() => void onOpen()} disabled={busy} />
            <ToolButton icon="save" label="Save as .vsdx" keys="Ctrl+S" onClick={() => void onSave()} disabled={!doc.pages.length} />
          </div>
          <div className="tb-group">
            <ToolButton
              icon="undo"
              label={undo.labels[0] ? `Undo ${undo.labels[0]}` : "Undo"}
              keys="Ctrl+Z"
              onClick={() => void exec({ kind: "undo" })}
              disabled={!undo.canUndo}
            />
            <ToolButton icon="redo" label="Redo" keys="Ctrl+Y" onClick={() => void exec({ kind: "redo" })} disabled={!undo.canRedo} />
          </div>
          {mode === "drawing" && (
            <>
              <div className="tb-group tools" aria-label="Tools">
                {TOOLS.map((t) => (
                  <ToolButton
                    key={t.tool}
                    icon={t.icon}
                    label={t.label}
                    keys={`Ctrl+${t.key}`}
                    pressed={tool === t.tool}
                    onClick={() => setTool(t.tool)}
                  />
                ))}
              </div>
              <div className="tb-group">
                <ToolButton icon="text" label="Edit text" keys="F2" onClick={() => editText(selection[0])} disabled={selection.length !== 1} />
                <ToolButton icon="trash" label="Delete" keys="Del" onClick={deleteSelection} disabled={selection.length === 0} />
              </div>
              <div className="tb-spacer" />
              <div className="tb-group zoom">
                <ToolButton icon="zoomOut" label="Zoom out" keys="Ctrl+-" onClick={() => stepZoom(-1)} />
                <button type="button" className="zoom-pct" data-tip="Fit page  Ctrl+Shift+W" onClick={() => canvas.current?.fit()}>
                  {Math.round((zoom / PX_PER_INCH) * 100)}%
                </button>
                <ToolButton icon="zoomIn" label="Zoom in" keys="Ctrl++" onClick={() => stepZoom(1)} />
                <ToolButton icon="fit" label="Fit page" keys="Ctrl+Shift+W" onClick={() => canvas.current?.fit()} />
              </div>
              <div className="tb-group">
                <ToolButton icon="panel" label="Inspector" onClick={() => setInspector((v) => !v)} pressed={inspector} />
              </div>
            </>
          )}
        </div>
      )}

      <main className="workspace">
        {error && (
          <div className="banner error" role="alert">
            <Icon name="info" size={16} />
            <span>{error}</span>
            <button type="button" className="icon-btn" aria-label="Dismiss" onClick={() => setError(null)}>
              <Icon name="close" size={14} />
            </button>
          </div>
        )}
        {!doc && <Welcome onNew={() => void onNew()} onOpen={() => void onOpen()} busy={busy} />}
        {doc && mode === "package" && <PackageView open={doc.open} pkg={doc.pkg} info={doc.info} />}
        {doc && mode === "drawing" && (
          <div className="stage">
            <div className="canvas-wrap">
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
                <div className="stage-empty">{doc.pages.length === 0 ? "This file has no drawing pages." : "Loading page…"}</div>
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
              {tool !== "select" && (
                <div className="tool-hint">
                  Drag to draw a {TOOLS.find((t) => t.tool === tool)?.label.toLowerCase()} · <kbd>Esc</kbd> to cancel
                </div>
              )}
            </div>
            {inspector && dl && (
              <Inspector
                dl={dl}
                info={doc.info}
                selection={selection}
                onCommand={(c) => void exec(c)}
                onTitle={(v) => void exec({ kind: "setDocProp", key: "title", value: v })}
                onDelete={deleteSelection}
              />
            )}
          </div>
        )}
      </main>

      {doc && (
        <footer className="statusbar">
          <div className="page-tabs" role="tablist" aria-label="Pages">
            {doc.pages.map((p) => (
              <button
                type="button"
                role="tab"
                key={p.id}
                aria-selected={p.index === page}
                className={p.background ? "background" : undefined}
                onClick={() => {
                  setPage(p.index);
                  setMode("drawing");
                }}
                title={p.background ? "Background page" : undefined}
              >
                <Icon name="page" size={14} />
                {p.name}
              </button>
            ))}
          </div>
          <div className="status-items">
            {selection.length > 0 && <span>{selection.length === 1 ? "1 shape selected" : `${selection.length} shapes selected`}</span>}
            {dl && (
              <span>
                {dl.page.width.toFixed(2)} × {dl.page.height.toFixed(2)} in
              </span>
            )}
            {dl && dl.warnings.length > 0 && (
              <span className="warn" title={dl.warnings.join("\n")}>
                {dl.warnings.length} warnings
              </span>
            )}
            <span>{sync === "local" || !sync ? "Local" : sync}</span>
            <span className="muted-mono" title="Derived-state determinism hash">
              {inTauri ? "Desktop" : "Browser"}
              {hash ? ` · ${hash.slice(0, 8)}` : ""}
            </span>
          </div>
        </footer>
      )}

      {dialog === "shortcuts" && <ShortcutsDialog onClose={() => setDialog(null)} />}
      {dialog === "about" && <AboutDialog onClose={() => setDialog(null)} />}
    </div>
  );
}
