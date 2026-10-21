import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent } from "react";

import { CanvasView, gridStep, PX_PER_INCH, type CanvasHandle, type Tool } from "./canvas/CanvasView";
import { pageBounds } from "./canvas/edit";
import { CommandPalette, type PaletteItem } from "./chrome/CommandPalette";
import { fmt, FormatBar, hex } from "./chrome/FormatBar";
import { Icon, shortcut, type IconName } from "./chrome/icons";
import { Inspector } from "./chrome/Inspector";
import { MenuBar, type Menu, type MenuEntry } from "./chrome/MenuBar";
import { PackageView } from "./chrome/PackageView";
import { DropButton, PopupMenu } from "./chrome/Popup";
import { ShapesPanel } from "./chrome/ShapesPanel";
import { AboutDialog, ShortcutsDialog, Welcome } from "./chrome/Welcome";
import type { EngineClient } from "./engine/client";
import type {
  Align,
  Axis,
  CellEdit,
  ChangeSummary,
  ClipShape,
  Command,
  DisplayList,
  DocInfo,
  LibraryEntry,
  OpenResult,
  PackageInfo,
  PageInfo,
  ShapeFormat,
  SyncState,
} from "./engine/protocol";
import { ExportDialog, RecoveryNotice, UnsavedDialog, type ExportChoice } from "./chrome/FileDialogs";
import { pageImage, pagePng } from "./canvas/pageImage";
import {
  canExportPdf,
  checkForUpdates,
  exportFile,
  exportPdf,
  inTauri,
  pickDocument,
  saveDocument,
  setDocumentState,
  type PickedFile,
  type SaveHandle,
} from "./host";
import { clearRecoveryCopy, keepRecoveryCopy, recoveryCopy, type RecoveryCopy } from "./recovery";

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

/** How far each paste or duplicate is offset, inches (right and down). */
const PASTE_STEP = 0.25;

/** The cell edits that give a shape the formatting `f` (Paste style). */
function styleEdits(f: ShapeFormat): CellEdit[] {
  return [
    ...fmt.fill(hex(f.fill)),
    ...fmt.line(hex(f.line)),
    ...(f.line ? [...fmt.weight(f.lineWeight), ...fmt.pattern(f.linePattern)] : []),
    ...fmt.rounding(f.rounding),
    ...fmt.font(f.font),
    ...fmt.size(f.size),
    ...fmt.textColor(hex(f.textColor) ?? "#000000"),
    ...fmt.style(f, {}),
    ...fmt.align(f.align),
    ...fmt.valign(f.valign),
    ...fmt.beginArrow(f.beginArrow),
    ...fmt.endArrow(f.endArrow),
  ];
}

/** Drawing tools, with Visio's shortcuts. */
const TOOLS: { tool: Tool; label: string; key: string; icon: IconName }[] = [
  { tool: "select", label: "Pointer", key: "1", icon: "pointer" },
  { tool: "text", label: "Text", key: "2", icon: "text" },
  { tool: "connector", label: "Connector", key: "3", icon: "connector" },
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
  /** Where File > Save writes without asking (the .vsdx the drawing came from or was last saved to). */
  const [saveHandle, setSaveHandle] = useState<SaveHandle | undefined>(undefined);
  /** The unsaved-changes question, while it is open: how to answer the caller. */
  const [unsaved, setUnsaved] = useState<((proceed: boolean) => void) | null>(null);
  /** A drawing lost to a crash, found at start-up. */
  const [recovery, setRecovery] = useState<RecoveryCopy | null>(null);
  const [exporting, setExporting] = useState(false);
  /** Pages laid out for printing (image URL and size in inches) while the print dialog is up. */
  const [printPages, setPrintPages] = useState<{ url: string; width: number; height: number }[] | null>(null);
  /** A short confirmation ("Exported Plan.pdf"), cleared after a few seconds. */
  const [notice, setNotice] = useState<string | null>(null);
  const [inspector, setInspector] = useState(true);
  const [shapesPanel, setShapesPanel] = useState(true);
  const [library, setLibrary] = useState<LibraryEntry[]>([]);
  /** A shape to start editing as soon as it appears (a new text box). */
  const [pendingEdit, setPendingEdit] = useState<number | null>(null);
  const [dialog, setDialog] = useState<"shortcuts" | "about" | null>(null);
  const [grid, setGrid] = useState(true);
  const [rulers, setRulers] = useState(true);
  /** Refit the page when the window (or the canvas) changes size; zooming or panning turns it off. */
  const [autoFit, setAutoFit] = useState(true);
  const [snapGrid, setSnapGrid] = useState(true);
  const [smartGuides, setSmartGuides] = useState(true);
  const [minimap, setMinimap] = useState(true);
  const snap = useMemo(() => ({ grid: snapGrid, guides: smartGuides }), [snapGrid, smartGuides]);
  /** The command palette, or the quick-add shape search at a point. */
  const [palette, setPalette] = useState<{ kind: "commands" } | { kind: "shapes"; client: [number, number]; at: [number, number] } | null>(null);
  const closePalette = useCallback(() => setPalette(null), []);
  /** Copied shapes (model data, so cut shapes can still be pasted) and how often they have been pasted. */
  const clip = useRef<{ shapes: ClipShape[]; page: number; pastes: number } | null>(null);
  const [hasClip, setHasClip] = useState(false);
  /** Copied formatting (Copy style). */
  const [styleClip, setStyleClip] = useState<ShapeFormat | null>(null);
  /** The page tab menu: the page it is for (null: the hidden-pages list) and where it opens. */
  const [pageMenu, setPageMenu] = useState<{ page: number | null; at: [number, number] } | null>(null);
  /** The page whose tab is being renamed. */
  const [renaming, setRenaming] = useState<number | null>(null);
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
  // (Keyed on the document's id: its info refreshing after an edit is not a change.)
  const docId = doc?.open.doc;
  const hasPages = (doc?.pages.length ?? 0) > 0;
  useEffect(() => {
    if (!docId || !hasPages) {
      setDl(null);
      return;
    }
    let cancelled = false;
    engine
      .query(docId, { kind: "pageDisplayList", page })
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
  }, [docId, hasPages, page, engine, version]);

  // The shape library, once a document is open.
  useEffect(() => {
    if (!doc || library.length > 0) return;
    engine
      .query(doc.open.doc, { kind: "library" })
      .then(setLibrary)
      .catch(() => {});
  }, [doc, engine, library.length]);

  /** Re-reads the document's info and page list after a change. */
  const refreshInfo = useCallback(
    async (docId: string) => {
      const [info, pages] = await Promise.all([
        engine.query(docId, { kind: "docInfo" }),
        engine.query(docId, { kind: "pages" }),
      ]);
      setDoc((d) => (d && d.open.doc === docId ? { ...d, info, pages } : d));
      return pages;
    },
    [engine],
  );

  /** Shows a document the engine has just opened or created (`handle`: where Save writes it back). */
  const showDoc = useCallback(
    async (open: OpenResult, handle?: SaveHandle) => {
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
      setSaveHandle(handle);
      setDoc({ open, pkg, info, pages });
      setMode(pages.length > 0 ? "drawing" : "package");
      setHash(derived.hash);
    },
    [engine],
  );

  /** The drawing's file name as last opened or saved. */
  const fileName = savedAs ?? doc?.info.fileName ?? "Drawing.vsdx";
  const stem = fileName.replace(/\.[^.]+$/, "");
  /** Edits since the last save started, so a save that finishes after more edits stays dirty. */
  const versionRef = useRef(version);
  versionRef.current = version;

  /** Saves to the drawing's own file (or asks where, the first time, or with `asNew`). True once saved. */
  const saveTo = useCallback(
    async (asNew: boolean): Promise<boolean> => {
      if (!doc) return false;
      setError(null);
      try {
        const at = versionRef.current;
        const bytes = await engine.save(doc.open.doc);
        const saved = await saveDocument(`${stem}.vsdx`, bytes, asNew ? undefined : saveHandle);
        if (!saved) return false;
        setSavedAs(saved.fileName);
        setSaveHandle(saved.handle);
        if (versionRef.current === at) {
          setDirty(false);
          void clearRecoveryCopy();
        }
        return true;
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        return false;
      }
    },
    [doc, engine, saveHandle, stem],
  );
  const onSave = useCallback(() => saveTo(false), [saveTo]);
  const onSaveAs = useCallback(() => saveTo(true), [saveTo]);

  /** Before the drawing is replaced: true to go ahead (saved, or the changes discarded). */
  const confirmUnsaved = useCallback(
    (): Promise<boolean> => (dirty ? new Promise((resolve) => setUnsaved(() => resolve)) : Promise.resolve(true)),
    [dirty],
  );

  const onNew = useCallback(async () => {
    setError(null);
    try {
      if (!(await confirmUnsaved())) return;
      if (doc) await engine.close(doc.open.doc);
      await showDoc(await engine.newDrawing());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [confirmUnsaved, doc, engine, showDoc]);

  /** Opens a file (the unsaved-changes question has been answered). */
  const openFile = useCallback(
    async (file: PickedFile) => {
      setBusy(true);
      setError(null);
      try {
        if (doc) await engine.close(doc.open.doc);
        await showDoc(await engine.open(file.bytes, file.fileName), file.handle);
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
      if (!(await confirmUnsaved())) return;
      const picked = await pickDocument();
      if (picked) await openFile(picked);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [confirmUnsaved, openFile]);

  const onDrop = useCallback(
    async (ev: DragEvent) => {
      ev.preventDefault();
      setDragging(false);
      const file = ev.dataTransfer.files[0];
      if (!file) return;
      const bytes = await file.arrayBuffer();
      if (await confirmUnsaved()) await openFile({ fileName: file.name, bytes });
    },
    [confirmUnsaved, openFile],
  );

  // AutoRecover: a copy a couple of seconds after the last change (at least
  // every 30 s while editing goes on), kept until the drawing is saved.
  const lastKept = useRef(0);
  useEffect(() => {
    if (!doc || !dirty) return;
    const wait = Date.now() - lastKept.current > 30_000 ? 0 : 2_000;
    const timer = setTimeout(async () => {
      try {
        const bytes = await engine.save(doc.open.doc);
        lastKept.current = Date.now();
        await keepRecoveryCopy({ fileName: `${stem}.vsdx`, title: stem, savedAt: lastKept.current, bytes });
      } catch {
        // A drawing the engine cannot save has nothing to recover.
      }
    }, wait);
    return () => clearTimeout(timer);
  }, [doc, dirty, engine, stem, version]);

  // A copy left behind by a crash is offered on the start screen.
  useEffect(() => {
    void recoveryCopy().then(setRecovery);
  }, []);

  const onRecover = useCallback(async () => {
    const copy = recovery;
    if (!copy) return;
    setRecovery(null);
    await openFile({ fileName: copy.fileName, bytes: copy.bytes.slice(0) });
    // Still unsaved: Save asks where, and the copy stays until then.
    setDirty(true);
  }, [openFile, recovery]);

  const onDiscardRecovery = useCallback(() => {
    setRecovery(null);
    void clearRecoveryCopy();
  }, []);

  // The host warns before the window closes on unsaved changes; a browser tab asks too.
  useEffect(() => {
    void setDocumentState(dirty, stem).catch(() => {});
    if (!dirty || inTauri) return;
    const ask = (e: BeforeUnloadEvent) => e.preventDefault();
    window.addEventListener("beforeunload", ask);
    return () => window.removeEventListener("beforeunload", ask);
  }, [dirty, stem]);

  useEffect(() => {
    if (!notice) return;
    const t = setTimeout(() => setNotice(null), 4000);
    return () => clearTimeout(t);
  }, [notice]);

  /** Visible foreground pages, in order: what Export and Print cover. */
  const outputPages = useMemo(() => (doc?.pages ?? []).filter((p) => !p.background && !p.hidden), [doc]);

  const onExport = useCallback(
    async (choice: ExportChoice) => {
      if (!doc || !dl) return;
      setExporting(false);
      setError(null);
      // One page of several gets its name in the file name.
      const one = outputPages.length > 1 ? `${stem} - ${dl.page.name}` : stem;
      try {
        let saved: string | null = null;
        if (choice.format === "pdf") {
          const pages = choice.allPages ? outputPages.map((p) => p.index) : [page];
          saved = await exportPdf(`${choice.allPages ? stem : one}.pdf`, await engine.save(doc.open.doc), pages);
        } else if (choice.format === "png") {
          saved = await exportFile(`${one}.png`, await pagePng(dl, choice.dpi), "png");
        } else {
          const svg = await engine.request<string>({ kind: "export", doc: doc.open.doc, format: "svg", opts: { page } });
          saved = await exportFile(`${one}.svg`, new TextEncoder().encode(svg).buffer as ArrayBuffer, "svg");
        }
        if (saved) setNotice(`Exported ${saved}`);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [dl, doc, engine, outputPages, page, stem],
  );

  /** Lays the visible pages out as images and opens the system print dialog. */
  const onPrint = useCallback(async () => {
    if (!doc) return;
    setError(null);
    try {
      const out = [];
      for (const p of outputPages) {
        const list = await engine.query(doc.open.doc, { kind: "pageDisplayList", page: p.index });
        const img = await pageImage(list, 200);
        out.push({ url: img.toDataURL("image/png"), width: list.page.width, height: list.page.height });
      }
      setPrintPages(out);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [doc, engine, outputPages]);

  // Print once the page images are in the document; tidy up afterwards.
  useEffect(() => {
    if (!printPages) return;
    const done = () => setPrintPages(null);
    window.addEventListener("afterprint", done, { once: true });
    const imgs = [...document.querySelectorAll<HTMLImageElement>(".print-pages img")];
    void Promise.all(imgs.map((i) => i.decode().catch(() => {}))).then(() => window.print());
    return () => window.removeEventListener("afterprint", done);
  }, [printPages]);

  const exec = useCallback(
    async (cmd: Command): Promise<ChangeSummary | null> => {
      if (!doc) return null;
      try {
        const changes = await engine.exec(doc.open.doc, cmd);
        // The page list first, so the page fetched next still exists.
        const pages = await refreshInfo(doc.open.doc);
        const next = changes?.page ?? Math.min(page, Math.max(0, pages.length - 1));
        if (next !== page) {
          setSelection([]);
          setPage(next);
        }
        setVersion((v) => v + 1);
        setDirty(true);
        if (changes?.created) setSelection([changes.created[1]]);
        if ((cmd.kind === "ungroupShapes" || cmd.kind === "insertShapes") && changes?.shapes) {
          setSelection(changes.shapes.map(Number));
        }
        return changes;
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        return null;
      }
    },
    [doc, engine, page, refreshInfo],
  );

  /** Runs a canvas command; a new text box opens for typing straight away. */
  const onCanvasCommand = useCallback(
    async (c: Command) => {
      // Back to the pointer as soon as a shape is drawn (not when the engine
      // replies, which could undo a tool picked meanwhile).
      if (c.kind === "addShape" || c.kind === "addConnector") setTool("select");
      const changes = await exec(c);
      if (c.kind === "addShape" && c.shape === "text" && changes?.created) setPendingEdit(changes.created[1]);
    },
    [exec],
  );

  const applyFormat = useCallback(
    (cells: CellEdit[]) => {
      if (selection.length > 0) void exec({ kind: "setCells", page, shapes: selection, cells });
    },
    [exec, page, selection],
  );

  /** Adds a library shape centred on a page point (the view centre by default). */
  const addFromLibrary = useCallback(
    (entry: LibraryEntry, at?: [number, number]) => {
      const c = at ?? canvas.current?.viewCentre() ?? [dl ? dl.page.width / 2 : 4, dl ? dl.page.height / 2 : 4];
      let [x, y] = [c[0] - entry.width / 2, c[1] - entry.height / 2];
      // A clicked tile lands in view and, where it fits, on the page.
      if (!at && dl) {
        x = Math.max(0, Math.min(x, dl.page.width - entry.width));
        y = Math.max(0, Math.min(y, dl.page.height - entry.height));
      }
      void onCanvasCommand({
        kind: "addShape",
        page,
        shape: entry.id,
        x,
        y,
        w: entry.width,
        h: entry.height,
      });
    },
    [dl, onCanvasCommand, page],
  );

  const deleteSelection = useCallback(() => {
    if (selection.length > 0) void exec({ kind: "deleteShapes", page, shapes: selection });
  }, [exec, page, selection]);

  // Clipboard. Shapes travel as model data, so cut shapes paste back and
  // copies paste onto other pages. Each paste lands a step further along.
  const copyOf = useCallback(
    async (ids: number[]) => (doc && ids.length > 0 ? engine.query(doc.open.doc, { kind: "copyShapes", page, shapes: ids }) : []),
    [doc, engine, page],
  );
  const onCopy = useCallback(async () => {
    const shapes = await copyOf(selection);
    if (shapes.length > 0) {
      clip.current = { shapes, page, pastes: 0 };
      setHasClip(true);
    }
  }, [copyOf, page, selection]);
  const onCut = useCallback(async () => {
    await onCopy();
    deleteSelection();
  }, [deleteSelection, onCopy]);
  const onPaste = useCallback(() => {
    const c = clip.current;
    if (!c) return;
    c.pastes += 1;
    // On another page the first paste lands where the shapes were.
    const step = (c.page === page ? c.pastes : c.pastes - 1) * PASTE_STEP;
    void exec({ kind: "insertShapes", page, shapes: c.shapes, dx: step, dy: -step });
  }, [exec, page]);
  /** Copies of shapes, moved by (dx, dy); the clipboard is left alone. */
  const duplicate = useCallback(
    async (ids: number[], dx = PASTE_STEP, dy = -PASTE_STEP) => {
      const shapes = await copyOf(ids);
      if (shapes.length > 0) await exec({ kind: "insertShapes", page, shapes, dx, dy });
    },
    [copyOf, exec, page],
  );
  const pasteStyle = useCallback(() => {
    if (styleClip) applyFormat(styleEdits(styleClip));
  }, [applyFormat, styleClip]);

  /** Moves the selection by grid steps (Shift: a major step). */
  const nudge = useCallback(
    (dx: number, dy: number, big: boolean) => {
      if (selection.length === 0) return;
      const step = gridStep(zoom || PX_PER_INCH);
      const k = big ? step.major : step.minor;
      void exec({ kind: "moveShapes", page, shapes: selection, dx: dx * k, dy: dy * k });
    },
    [exec, page, selection, zoom],
  );

  /** Zooms to the selection, or to all shapes when nothing is selected. */
  const zoomToSelection = useCallback(() => {
    const boxes = dl?.shapes.filter((b) => selection.length === 0 || selection.includes(b.id)) ?? [];
    if (boxes.length === 0) return canvas.current?.fit();
    const rs = boxes.map(pageBounds);
    canvas.current?.zoomTo([
      Math.min(...rs.map((r) => r[0])),
      Math.min(...rs.map((r) => r[1])),
      Math.max(...rs.map((r) => r[2])),
      Math.max(...rs.map((r) => r[3])),
    ]);
  }, [dl, selection]);

  // Arrange: commands on the selection (alignment is to the first selected shape).
  const arrange = useMemo(() => {
    const shapes = selection;
    const run = (c: Command) => void exec(c);
    const groups = shapes.filter((id) => dl?.shapes.find((b) => b.id === id)?.group);
    return {
      align: (align: Align) => run({ kind: "alignShapes", page, shapes, align }),
      distribute: (axis: Axis) => run({ kind: "distributeShapes", page, shapes, axis }),
      rotate: (direction: "left" | "right") => run({ kind: "rotateShapes", page, shapes, direction }),
      flip: (axis: Axis) => run({ kind: "flipShapes", page, shapes, axis }),
      reorder: (to: "front" | "back" | "forward" | "backward") => run({ kind: "reorderShapes", page, shapes, to }),
      group: () => run({ kind: "groupShapes", page, shapes }),
      ungroup: () => run({ kind: "ungroupShapes", page, shapes: groups }),
      canAlign: shapes.length >= 2,
      canDistribute: shapes.length >= 3,
      any: shapes.length > 0,
      canUngroup: groups.length > 0,
    };
  }, [dl, exec, page, selection]);

  const { alignItems, positionItems, frontItems, backItems, groupItems } = useMemo(() => {
    const alignItems: MenuEntry[] = [
      { label: "Align left", icon: "shapeAlignLeft", run: () => arrange.align("left"), disabled: !arrange.canAlign },
      { label: "Align centre", icon: "shapeAlignCenter", run: () => arrange.align("center"), disabled: !arrange.canAlign },
      { label: "Align right", icon: "shapeAlignRight", run: () => arrange.align("right"), disabled: !arrange.canAlign },
      "separator",
      { label: "Align top", icon: "shapeAlignTop", run: () => arrange.align("top"), disabled: !arrange.canAlign },
      { label: "Align middle", icon: "shapeAlignMiddle", run: () => arrange.align("middle"), disabled: !arrange.canAlign },
      { label: "Align bottom", icon: "shapeAlignBottom", run: () => arrange.align("bottom"), disabled: !arrange.canAlign },
    ];
    const positionItems: MenuEntry[] = [
      { label: "Distribute horizontally", icon: "distributeH", run: () => arrange.distribute("horizontal"), disabled: !arrange.canDistribute },
      { label: "Distribute vertically", icon: "distributeV", run: () => arrange.distribute("vertical"), disabled: !arrange.canDistribute },
      "separator",
      { label: "Rotate right 90°", icon: "rotateRight", keys: "Ctrl+R", run: () => arrange.rotate("right"), disabled: !arrange.any },
      { label: "Rotate left 90°", icon: "rotateLeft", keys: "Ctrl+L", run: () => arrange.rotate("left"), disabled: !arrange.any },
      { label: "Flip vertical", icon: "flipV", keys: "Ctrl+J", run: () => arrange.flip("vertical"), disabled: !arrange.any },
      { label: "Flip horizontal", icon: "flipH", keys: "Ctrl+H", run: () => arrange.flip("horizontal"), disabled: !arrange.any },
    ];
    const frontItems: MenuEntry[] = [
      { label: "Bring to front", icon: "front", keys: "Ctrl+Shift+F", run: () => arrange.reorder("front"), disabled: !arrange.any },
      { label: "Bring forward", icon: "forward", run: () => arrange.reorder("forward"), disabled: !arrange.any },
    ];
    const backItems: MenuEntry[] = [
      { label: "Send to back", icon: "back", keys: "Ctrl+Shift+B", run: () => arrange.reorder("back"), disabled: !arrange.any },
      { label: "Send backward", icon: "backward", run: () => arrange.reorder("backward"), disabled: !arrange.any },
    ];
    const groupItems: MenuEntry[] = [
      { label: "Group", icon: "group", keys: "Ctrl+G", run: arrange.group, disabled: !arrange.canAlign },
      { label: "Ungroup", icon: "ungroup", keys: "Ctrl+Shift+U", run: arrange.ungroup, disabled: !arrange.canUngroup },
    ];
    return { alignItems, positionItems, frontItems, backItems, groupItems };
  }, [arrange]);

  const closePageMenu = useCallback(() => setPageMenu(null), []);

  /** Shows a page from its tab (or the page menu). */
  const showPage = useCallback((index: number) => {
    setPage(index);
    setMode("drawing");
  }, []);

  /** The page tab menu's entries: for one page, or (`null`) the list of hidden pages. */
  const pageMenuItems = (index: number | null): MenuEntry[] => {
    const pages = doc?.pages ?? [];
    const visible = pages.filter((p) => !p.hidden && !p.background).length;
    const unhide: MenuEntry[] = pages
      .filter((p) => p.hidden)
      .map((p) => ({
        label: `Unhide “${p.name}”`,
        icon: "eye",
        run: () => void exec({ kind: "setPageHidden", page: p.index, hidden: false }),
      }));
    if (index === null) return unhide;
    const p = pages[index];
    const lastForeground = !p.background && pages.filter((q) => !q.background).length === 1;
    const items: MenuEntry[] = [
      { label: "Insert page", icon: "plus", run: () => void exec({ kind: "addPage", after: index }) },
      { label: "Rename", icon: "rename", run: () => setRenaming(index) },
      {
        label: "Hide",
        icon: "eyeOff",
        run: () => void exec({ kind: "setPageHidden", page: index, hidden: true }),
        disabled: !p.background && visible <= 1,
      },
      { label: "Delete", icon: "trash", run: () => void exec({ kind: "deletePage", page: index }), disabled: lastForeground },
    ];
    return unhide.length ? [...items, "separator", ...unhide] : items;
  };

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

  // Open the editor on a new text box once it is on screen.
  useEffect(() => {
    if (pendingEdit !== null && dl?.shapes.some((b) => b.id === pendingEdit)) {
      editText(pendingEdit);
      setPendingEdit(null);
    }
  }, [dl, editText, pendingEdit]);

  const selectedFormat = dl?.shapes.find((b) => b.id === selection[0])?.format ?? null;

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
        if (doc?.pages.length) void (e.shiftKey ? onSaveAs() : onSave());
      } else if (mod && e.shiftKey && key === "e" && drawing) {
        e.preventDefault();
        setExporting(true);
      } else if (mod && !e.shiftKey && key === "p" && drawing) {
        e.preventDefault();
        void onPrint();
      } else if (mod && key === "n") {
        e.preventDefault();
        void onNew();
      } else if (mod && (key === "k" || key === "/") && doc) {
        // Command palette (Ctrl+K, or Figma's Ctrl+/).
        e.preventDefault();
        setPalette((p) => (p ? null : { kind: "commands" }));
      } else if (mod && e.altKey && (e.code === "KeyC" || e.code === "KeyV") && drawing && !inInput) {
        // Copy and paste style (Figma's Ctrl+Alt+C / V). By code: Alt changes the key on a Mac.
        e.preventDefault();
        if (e.code === "KeyV") pasteStyle();
        else if (selectedFormat) setStyleClip(selectedFormat);
      } else if (mod && !e.altKey && !e.shiftKey && ["c", "x", "v", "d"].includes(key) && drawing && !inInput) {
        e.preventDefault();
        if (key === "c") void onCopy();
        else if (key === "x") void onCut();
        else if (key === "v") onPaste();
        else void duplicate(selection);
      } else if (!mod && !e.altKey && e.key.startsWith("Arrow") && drawing && !inInput && selection.length > 0) {
        // Nudge by a grid step; Shift for a big step.
        e.preventDefault();
        const [dx, dy] = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, 1], ArrowDown: [0, -1] }[e.key] ?? [0, 0];
        nudge(dx, dy, e.shiftKey);
      } else if (e.shiftKey && !mod && !e.altKey && (e.code === "Digit1" || e.code === "Digit2") && drawing && !inInput) {
        // Figma: Shift+1 shows everything, Shift+2 the selection.
        e.preventDefault();
        if (e.code === "Digit1") canvas.current?.fit();
        else zoomToSelection();
      } else if (toolKey && drawing && !inInput) {
        e.preventDefault();
        setTool(toolKey.tool);
      } else if (mod && drawing && !inInput && arrangeKey(key, e.shiftKey)) {
        // Arrange (Visio's keys). These shadow browser shortcuts (Ctrl+R, Ctrl+H …) only while drawing.
        e.preventDefault();
        if (arrange.any) arrangeKey(key, e.shiftKey)!();
      } else if (mod && !e.shiftKey && (key === "b" || key === "i" || key === "u") && drawing && !inInput && selectedFormat) {
        e.preventDefault();
        const k = key === "b" ? "bold" : key === "i" ? "italic" : "underline";
        const f = selectedFormat;
        const bits = ((k === "bold" ? !f.bold : f.bold) ? 1 : 0) | ((k === "italic" ? !f.italic : f.italic) ? 2 : 0) | ((k === "underline" ? !f.underline : f.underline) ? 4 : 0);
        applyFormat([{ section: "Character", cell: "Style", value: String(bits) }]);
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
        // Hidden pages are skipped.
        e.preventDefault();
        const n = doc.pages.length;
        const step = e.key === "PageDown" ? 1 : n - 1;
        let p = page;
        for (let i = 0; i < n; i++) {
          p = (p + step) % n;
          if (!doc.pages[p].hidden) break;
        }
        setPage(p);
      }
    };
    /** The arrange command for Ctrl+key (with Shift), if any. */
    function arrangeKey(key: string, shift: boolean): (() => void) | null {
      if (shift) {
        if (key === "f") return () => arrange.reorder("front");
        if (key === "b") return () => arrange.reorder("back");
        if (key === "u") return arrange.ungroup;
        return null;
      }
      if (key === "g") return arrange.group;
      if (key === "r") return () => arrange.rotate("right");
      if (key === "l") return () => arrange.rotate("left");
      if (key === "h") return () => arrange.flip("horizontal");
      if (key === "j") return () => arrange.flip("vertical");
      return null;
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [doc, drawing, exec, onOpen, onNew, onSave, stepZoom, deleteSelection, selectAll, editText, selection, selectedFormat, applyFormat, arrange, page,
    onCopy, onCut, onPaste, duplicate, pasteStyle, nudge, zoomToSelection, onSaveAs, onPrint]);

  const menus: Menu[] = useMemo(
    () => [
      {
        label: "File",
        items: [
          { label: "New drawing", icon: "new", keys: "Ctrl+N", run: () => void onNew() },
          { label: "Open…", icon: "open", keys: "Ctrl+O", run: () => void onOpen(), disabled: busy },
          "separator",
          { label: "Save", icon: "save", keys: "Ctrl+S", run: () => void onSave(), disabled: !doc?.pages.length },
          { label: "Save as…", keys: "Ctrl+Shift+S", run: () => void onSaveAs(), disabled: !doc?.pages.length },
          "separator",
          { label: "Export…", icon: "export", keys: "Ctrl+Shift+E", run: () => setExporting(true), disabled: !drawing },
          { label: "Print…", icon: "print", keys: "Ctrl+P", run: () => void onPrint(), disabled: !drawing },
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
          { label: "Cut", icon: "cut", keys: "Ctrl+X", run: () => void onCut(), disabled: !drawing || selection.length === 0 },
          { label: "Copy", icon: "copy", keys: "Ctrl+C", run: () => void onCopy(), disabled: !drawing || selection.length === 0 },
          { label: "Paste", icon: "paste", keys: "Ctrl+V", run: onPaste, disabled: !drawing || !hasClip },
          { label: "Duplicate", icon: "duplicate", keys: "Ctrl+D", run: () => void duplicate(selection), disabled: !drawing || selection.length === 0 },
          "separator",
          {
            label: "Copy style",
            icon: "brush",
            keys: "Ctrl+Alt+C",
            run: () => selectedFormat && setStyleClip(selectedFormat),
            disabled: !selectedFormat,
          },
          { label: "Paste style", icon: "brush", keys: "Ctrl+Alt+V", run: pasteStyle, disabled: !styleClip || selection.length === 0 },
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
          { label: "Zoom to selection", icon: "focus", keys: "Shift+2", run: zoomToSelection, disabled: !drawing },
          { label: "Fit page on resize", checked: autoFit, run: () => setAutoFit((v) => !v), disabled: !doc },
          "separator",
          { label: "Rulers", checked: rulers, run: () => setRulers((v) => !v), disabled: !doc },
          { label: "Grid", checked: grid, run: () => setGrid((v) => !v), disabled: !doc },
          { label: "Snap to grid", checked: snapGrid, run: () => setSnapGrid((v) => !v), disabled: !doc },
          { label: "Smart guides", checked: smartGuides, run: () => setSmartGuides((v) => !v), disabled: !doc },
          { label: "Minimap", checked: minimap, run: () => setMinimap((v) => !v), disabled: !doc },
          "separator",
          { label: "Shapes panel", checked: shapesPanel, run: () => setShapesPanel((v) => !v), disabled: !doc },
          { label: "Inspector", checked: inspector, run: () => setInspector((v) => !v), disabled: !doc },
          "separator",
          { label: "Drawing", checked: mode === "drawing", run: () => setMode("drawing"), disabled: !doc },
          { label: "Package parts", checked: mode === "package", run: () => setMode("package"), disabled: !doc },
        ],
      },
      {
        label: "Insert",
        items: [
          ...TOOLS.slice(1).map((t) => ({
            label: t.label,
            icon: t.icon,
            keys: `Ctrl+${t.key}`,
            run: () => {
              setMode("drawing");
              setTool(t.tool);
            },
            disabled: !doc,
          })),
          "separator",
          { label: "New page", icon: "plus", run: () => void exec({ kind: "addPage" }), disabled: !doc },
        ],
      },
      {
        label: "Arrange",
        items: [...alignItems, "separator", ...positionItems, "separator", ...frontItems, ...backItems, "separator", ...groupItems],
      },
      {
        label: "Help",
        items: [
          { label: "Command palette", icon: "command", keys: "Ctrl+K", run: () => setPalette({ kind: "commands" }), disabled: !doc },
          { label: "Keyboard shortcuts", icon: "keyboard", run: () => setDialog("shortcuts") },
          ...(inTauri
            ? [{ label: "Check for updates…", icon: "export" as const, run: () => void checkForUpdates().catch(() => {}) }]
            : []),
          { label: "About Flowster", icon: "info", run: () => setDialog("about") },
        ],
      },
    ],
    [busy, deleteSelection, doc, drawing, editText, exec, inspector, shapesPanel, mode, onNew, onOpen, onSave, selectAll, selection, stepZoom, undo,
      autoFit, rulers, grid, alignItems, positionItems, frontItems, backItems, groupItems,
      onCut, onCopy, onPaste, duplicate, pasteStyle, selectedFormat, styleClip, hasClip, zoomToSelection, snapGrid, smartGuides, minimap, onSaveAs, onPrint],
  );

  // The command palette lists every enabled menu command, then pages and shapes.
  const paletteItems = useMemo((): PaletteItem[] => {
    if (palette?.kind !== "commands") return [];
    const commands = menus.flatMap((m) =>
      m.items.flatMap((it): PaletteItem[] =>
        it === "separator" || it.disabled
          ? []
          : [
              {
                id: `${m.label}/${it.label}`,
                label: it.label,
                group: it.checked === undefined ? m.label : `${m.label} · ${it.checked ? "on" : "off"}`,
                icon: it.icon,
                keys: it.keys,
                run: it.run,
              },
            ],
      ),
    );
    const pages = (doc?.pages ?? []).map(
      (p): PaletteItem => ({ id: `page/${p.id}`, label: `Go to ${p.name}`, group: "Page", icon: "page", run: () => showPage(p.index) }),
    );
    const shapes = library.map(
      (e): PaletteItem => ({
        id: `shape/${e.id}`,
        label: `Add ${e.name}`,
        group: "Shape",
        shape: e,
        keywords: e.category,
        run: () => {
          setMode("drawing");
          addFromLibrary(e);
        },
      }),
    );
    return [...commands, ...pages, ...shapes];
  }, [palette, menus, doc, library, showPage, addFromLibrary]);

  // Quick add: the shapes only, dropped where the canvas was double-clicked.
  const quickAddItems = useMemo((): PaletteItem[] => {
    if (palette?.kind !== "shapes") return [];
    return library.map((e) => ({
      id: e.id,
      label: e.name,
      group: e.category,
      shape: e,
      run: () => addFromLibrary(e, palette.at),
    }));
  }, [palette, library, addFromLibrary]);

  // The saved file's name once there is one, else the drawing's title.
  const title = savedAs ? stem : doc?.info.meta.title || stem;

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
          {doc && mode === "drawing" && (
            <div className="panel-toggles">
              <ToolButton icon="shapes" label="Shapes panel" onClick={() => setShapesPanel((v) => !v)} pressed={shapesPanel} />
              <ToolButton icon="panel" label="Inspector" onClick={() => setInspector((v) => !v)} pressed={inspector} />
            </div>
          )}
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
            <ToolButton icon="save" label="Save" keys="Ctrl+S" onClick={() => void onSave()} disabled={!doc.pages.length} />
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
              <div className="tb-group" aria-label="Arrange">
                <DropButton icon="shapeAlignLeft" label="Align" items={alignItems} disabled={!arrange.any} />
                <DropButton icon="position" label="Position" items={positionItems} disabled={!arrange.any} />
                <DropButton
                  icon="front"
                  label="Bring to front"
                  keys="Ctrl+Shift+F"
                  onClick={() => arrange.reorder("front")}
                  items={frontItems}
                  disabled={!arrange.any}
                />
                <DropButton
                  icon="back"
                  label="Send to back"
                  keys="Ctrl+Shift+B"
                  onClick={() => arrange.reorder("back")}
                  items={backItems}
                  disabled={!arrange.any}
                />
                <DropButton icon="group" label="Group" items={groupItems} disabled={!arrange.canAlign && !arrange.canUngroup} />
              </div>
              {selectedFormat && <FormatBar format={selectedFormat} apply={applyFormat} />}
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
        {notice && (
          <div className="banner info" role="status">
            <Icon name="check" size={16} />
            <span>{notice}</span>
          </div>
        )}
        {!doc && (
          <Welcome
            onNew={() => void onNew()}
            onOpen={() => void onOpen()}
            busy={busy}
            notice={
              recovery && (
                <RecoveryNotice
                  title={recovery.title}
                  savedAt={recovery.savedAt}
                  onRecover={() => void onRecover()}
                  onDiscard={onDiscardRecovery}
                />
              )
            }
          />
        )}
        {doc && mode === "package" && <PackageView open={doc.open} pkg={doc.pkg} info={doc.info} />}
        {doc && mode === "drawing" && (
          <div className="stage">
            {shapesPanel && library.length > 0 && <ShapesPanel library={library} onAdd={(e) => addFromLibrary(e)} />}
            <div className="canvas-wrap">
              {dl ? (
                <CanvasView
                  ref={canvas}
                  dl={dl}
                  tool={tool}
                  selection={selection}
                  onSelection={setSelection}
                  onCommand={(c) => void onCanvasCommand(c)}
                  onEditText={editText}
                  onDropShape={(id, at) => {
                    const entry = library.find((e) => e.id === id);
                    if (entry) addFromLibrary(entry, at);
                  }}
                  onZoomChange={setZoom}
                  grid={grid}
                  rulers={rulers}
                  autoFit={autoFit}
                  onManualView={() => setAutoFit(false)}
                  snap={snap}
                  minimap={minimap}
                  onDuplicate={(ids, dx, dy) => void duplicate(ids, dx, dy)}
                  onQuickAdd={(client, at) => setPalette({ kind: "shapes", client, at })}
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
                  {tool === "connector"
                    ? "Drag from one shape to another to connect them"
                    : tool === "text"
                      ? "Click to add text, or drag a text box"
                      : `Drag to draw a ${TOOLS.find((t) => t.tool === tool)?.label.toLowerCase()}`}{" "}
                  · <kbd>Esc</kbd> to cancel
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
            {doc.pages
              .filter((p) => !p.hidden || p.index === page)
              .map((p) =>
                renaming === p.index ? (
                  <input
                    key={p.id}
                    className="page-rename"
                    aria-label="Page name"
                    defaultValue={p.name}
                    autoFocus
                    onFocus={(e) => e.currentTarget.select()}
                    onBlur={(e) => {
                      const name = e.currentTarget.value.trim();
                      setRenaming(null);
                      if (name && name !== p.name) void exec({ kind: "renamePage", page: p.index, name });
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") e.currentTarget.blur();
                      else if (e.key === "Escape") {
                        e.currentTarget.value = p.name;
                        e.currentTarget.blur();
                      }
                    }}
                  />
                ) : (
                  <button
                    type="button"
                    role="tab"
                    key={p.id}
                    aria-selected={p.index === page}
                    className={[p.background && "background", p.hidden && "hidden"].filter(Boolean).join(" ") || undefined}
                    onClick={() => showPage(p.index)}
                    onDoubleClick={() => setRenaming(p.index)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      showPage(p.index);
                      setPageMenu({ page: p.index, at: [e.clientX, e.clientY] });
                    }}
                    title={[p.background && "Background page", p.hidden && "Hidden page"].filter(Boolean).join(" · ") || undefined}
                  >
                    <Icon name={p.hidden ? "eyeOff" : "page"} size={14} />
                    {p.name}
                  </button>
                ),
              )}
            <button
              type="button"
              className="page-add"
              aria-label="Insert page"
              title="Insert page"
              onClick={() => void exec({ kind: "addPage" })}
            >
              <Icon name="plus" size={15} />
            </button>
            {doc.pages.some((p) => p.hidden) && (
              <button
                type="button"
                className="page-hidden"
                aria-label="Hidden pages"
                title="Hidden pages"
                onClick={(e) => {
                  const r = e.currentTarget.getBoundingClientRect();
                  setPageMenu({ page: null, at: [r.left, r.top - 4] });
                }}
              >
                <Icon name="eyeOff" size={14} />
                {doc.pages.filter((p) => p.hidden).length}
              </button>
            )}
          </div>
          {pageMenu && (
            <PopupMenu
              items={pageMenuItems(pageMenu.page)}
              at={pageMenu.at}
              label={pageMenu.page === null ? "Hidden pages" : "Page"}
              onClose={closePageMenu}
            />
          )}
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
            {mode === "drawing" && dl && (
              <div className="zoom-ctl">
                <button type="button" className="icon-btn" aria-label="Zoom out" onClick={() => stepZoom(-1)}>
                  <Icon name="zoomOut" size={15} />
                </button>
                <input
                  type="range"
                  aria-label="Zoom"
                  min={10}
                  max={400}
                  value={Math.round((zoom / PX_PER_INCH) * 100)}
                  onChange={(e) => canvas.current?.setZoom((Number(e.target.value) / 100) * PX_PER_INCH)}
                />
                <button type="button" className="icon-btn" aria-label="Zoom in" onClick={() => stepZoom(1)}>
                  <Icon name="zoomIn" size={15} />
                </button>
                <button type="button" className="zoom-pct" onClick={() => canvas.current?.fit()} title="Fit page (Ctrl+Shift+W)">
                  {Math.round((zoom / PX_PER_INCH) * 100)}%
                </button>
                <button type="button" className="icon-btn" aria-label="Fit page" onClick={() => canvas.current?.fit()}>
                  <Icon name="fit" size={15} />
                </button>
                <button
                  type="button"
                  className="icon-btn"
                  aria-label="Fit page on resize"
                  aria-pressed={autoFit}
                  title={autoFit ? "Fitting the page on resize (zoom or pan to stop)" : "Fit the page whenever the window resizes"}
                  onClick={() => setAutoFit((v) => !v)}
                >
                  <Icon name="autoFit" size={15} />
                </button>
              </div>
            )}
          </div>
        </footer>
      )}

      {palette?.kind === "commands" && (
        <CommandPalette
          items={paletteItems}
          placeholder="Type a command, a page or a shape…"
          label="Command palette"
          onClose={closePalette}
        />
      )}
      {palette?.kind === "shapes" && (
        <CommandPalette items={quickAddItems} placeholder="Add a shape…" label="Add a shape" at={palette.client} onClose={closePalette} />
      )}
      {dialog === "shortcuts" && <ShortcutsDialog onClose={() => setDialog(null)} />}
      {dialog === "about" && <AboutDialog onClose={() => setDialog(null)} />}
      {unsaved && (
        <UnsavedDialog
          title={stem}
          onSave={() => {
            setUnsaved(null);
            void saveTo(false).then(unsaved);
          }}
          onDiscard={() => {
            setUnsaved(null);
            void clearRecoveryCopy();
            unsaved(true);
          }}
          onCancel={() => {
            setUnsaved(null);
            unsaved(false);
          }}
        />
      )}
      {exporting && dl && (
        <ExportDialog
          pdf={canExportPdf}
          pageName={dl.page.name}
          pageCount={outputPages.length}
          onExport={(c) => void onExport(c)}
          onClose={() => setExporting(false)}
        />
      )}
      {printPages && (
        <div className="print-pages" aria-hidden="true">
          <style>{`@page { size: ${printPages[0]?.width ?? 11}in ${printPages[0]?.height ?? 8.5}in; margin: 0; }`}</style>
          {printPages.map((p, i) => (
            <img key={i} src={p.url} alt="" style={{ width: `${p.width}in`, height: `${p.height}in` }} />
          ))}
        </div>
      )}
    </div>
  );
}
