// Engine ↔ UI protocol (SPEC §5.4). Mirrors crates/flowster-core/src/protocol.rs.
//
// UI → worker: `{ id, req }`. Worker → UI: a stream of EngineEvents; the last
// event for a request is `reply` or `error` carrying the same id.
// `open` transfers the file's ArrayBuffer (no copy).

export type DocId = string;
export type ShapeRef = { page: number; id: number };

// ---- commands (the command bus, §5.5) --------------------------------------

/** A library shape id (`rectangle`, `process`, `line`, `pool`, …; see the `library` query). */
export type ShapeKind = string;

/** One cell to set: a single cell, or a `Character`/`Paragraph` row cell (all rows unless `row`). */
export interface CellEdit {
  section?: string;
  row?: number;
  cell: string;
  value: string;
}

export type Command =
  | { kind: "setDocProp"; key: string; value: string }
  /** 2-D shapes fill (x, y)–(x + w, y + h) in page inches; a line runs between those points. */
  | { kind: "addShape"; page: number; shape: ShapeKind; x: number; y: number; w: number; h: number }
  | { kind: "moveShapes"; page: number; shapes: number[]; dx: number; dy: number }
  | { kind: "setBounds"; page: number; shape: number; pinX: number; pinY: number; width: number; height: number }
  | {
      kind: "setEndpoints";
      page: number;
      shape: number;
      beginX: number;
      beginY: number;
      endX: number;
      endY: number;
      /** Connectors: the shapes the ends glue to (absent = unglued). */
      beginGlue?: number;
      endGlue?: number;
    }
  | { kind: "setCells"; page: number; shapes: number[]; cells: CellEdit[] }
  | { kind: "addConnector"; page: number; from: number | null; to: number | null; x0: number; y0: number; x1: number; y1: number }
  | { kind: "autoConnect"; page: number; shape: number; direction: "up" | "down" | "left" | "right" }
  | { kind: "deleteShapes"; page: number; shapes: number[] }
  | { kind: "setText"; page: number; shape: number; text: string }
  /** Aligns to the first shape (the primary selection). */
  | { kind: "alignShapes"; page: number; shapes: number[]; align: Align }
  | { kind: "distributeShapes"; page: number; shapes: number[]; axis: Axis }
  | { kind: "rotateShapes"; page: number; shapes: number[]; direction: "left" | "right" }
  | { kind: "flipShapes"; page: number; shapes: number[]; axis: Axis }
  | { kind: "reorderShapes"; page: number; shapes: number[]; to: "front" | "back" | "forward" | "backward" }
  | { kind: "groupShapes"; page: number; shapes: number[] }
  | { kind: "ungroupShapes"; page: number; shapes: number[] }
  /** A blank foreground page after `after` (after the last foreground page when absent). */
  | { kind: "addPage"; after?: number }
  | { kind: "deletePage"; page: number }
  | { kind: "renamePage"; page: number; name: string }
  | { kind: "setPageHidden"; page: number; hidden: boolean }
  /** Pastes shapes from the `copyShapes` query, moved by (dx, dy) inches; they get new ids. */
  | { kind: "insertShapes"; page: number; shapes: ClipShape[]; dx: number; dy: number }
  | { kind: "undo" }
  | { kind: "redo" };

export type Align = "left" | "center" | "right" | "top" | "middle" | "bottom";
export type Axis = "horizontal" | "vertical";

// ---- queries ---------------------------------------------------------------

export type Query =
  | { kind: "packageParts" }
  | { kind: "docInfo" }
  | { kind: "derivedStateHash" }
  | { kind: "pages" }
  | { kind: "pageDisplayList"; page: number }
  | { kind: "library" }
  | { kind: "copyShapes"; page: number; shapes: number[] };

/** A shape as model data, for the clipboard (opaque to the UI). */
export type ClipShape = Record<string, unknown>;

/** A Shapes panel entry; `preview` is an SVG path in a `width` × `height` box (y down). */
export interface LibraryEntry {
  id: string;
  name: string;
  category: string;
  width: number;
  height: number;
  preview: string;
}

export type PackageKind =
  | "drawing"
  | "stencil"
  | "template"
  | "drawingMacro"
  | "stencilMacro"
  | "templateMacro"
  | "unknown";

export interface PartInfo {
  name: string;
  contentType: string | null;
  size: number;
  compressedSize: number;
  compression: string;
  isRelationships: boolean;
}

export interface Relationship {
  source: string;
  id: string;
  relType: string;
  target: string;
  external: boolean;
  targetPart: string | null;
  targetExists: boolean;
}

export interface PackageInfo {
  kind: PackageKind;
  mainPart: string | null;
  parts: PartInfo[];
  relationships: Relationship[];
  contentTypes: { defaults: [string, string][]; overrides: [string, string][] };
  totalSize: number;
  warnings: string[];
}

export interface DocInfo {
  fileName: string;
  meta: Record<string, string>;
  unknownParts: number;
  snapshotBytes: number;
}

export interface DerivedStateReport {
  hash: string;
  values: number;
  shapes: number;
  seed: number;
}

export interface OpenResult {
  doc: DocId;
  fileName: string;
  kind: PackageKind;
  mainPart: string | null;
  partCount: number;
  pageCount: number;
}

// ---- display list (SPEC §8.5) ------------------------------------------------
// Page coordinates are inches with +Y up; the canvas flips Y.

/** [r, g, b, a], 0–255. */
export type Rgba = [number, number, number, number];
/** Affine [a, b, c, d, e, f]: x' = a·x + c·y + e, y' = b·x + d·y + f. */
export type Affine = [number, number, number, number, number, number];

export type Seg =
  | { k: "M"; x: number; y: number }
  | { k: "L"; x: number; y: number }
  | { k: "C"; x1: number; y1: number; x2: number; y2: number; x: number; y: number }
  | { k: "Z" };

export interface Stroke {
  color: Rgba;
  width: number;
  dash: number[];
  cap: CanvasLineCap;
}

export interface Run {
  text: string;
  font: string;
  size: number;
  color: Rgba;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
}

export interface Paragraph {
  align: number;
  lineSpacing: number;
  spaceBefore: number;
  spaceAfter: number;
  indentLeft: number;
  indentFirst: number;
  runs: Run[];
}

export interface Stop {
  offset: number;
  color: Rgba;
}

/** Fill paints. Gradient and pattern coordinates are in paint space, mapped to the page by `space`. */
export type Paint =
  | { type: "solid"; color: Rgba }
  | { type: "linear"; space: Affine; x0: number; y0: number; x1: number; y1: number; stops: Stop[] }
  | { type: "radial"; space: Affine; cx: number; cy: number; r: number; stops: Stop[] }
  /** Square isolines: offset = Chebyshev distance from (ox, oy) / extent. */
  | { type: "rect"; space: Affine; ox: number; oy: number; extent: number; stops: Stop[] }
  /** 8 × 8 bitmap (row 0 at the top, bit 7 on the left), one bit per paint unit. */
  | { type: "pattern"; space: Affine; bits: number[]; fg: Rgba; bg: Rgba };

export type Item =
  | { kind: "path"; shape: number; d: Seg[]; fill: Paint | null; stroke: Stroke | null; blur?: number }
  | {
      kind: "text";
      shape: number;
      transform: Affine;
      width: number;
      height: number;
      margins: [number, number, number, number];
      valign: number;
      background: Rgba | null;
      paragraphs: Paragraph[];
    }
  | {
      kind: "image";
      shape: number;
      transform: Affine;
      x: number;
      y: number;
      w: number;
      h: number;
      mime: string;
      data: string | null;
    };

export interface PageInfo {
  index: number;
  id: number;
  name: string;
  background: boolean;
  backPage: number | null;
  /** Hidden from the page tabs. */
  hidden: boolean;
  width: number;
  height: number;
}

/** A top-level shape's frame, for selection and handles. */
export interface ShapeBox {
  id: number;
  /** Shape-local inches (origin bottom-left, +Y up) → page inches. */
  transform: Affine;
  width: number;
  height: number;
  /** Page positions of a 1-D shape's ends. */
  begin?: [number, number];
  end?: [number, number];
  text?: string;
  /** Shapes a connector's ends are glued to. */
  beginGlue?: number;
  endGlue?: number;
  connector?: boolean;
  group?: boolean;
  /** The `Angle` cell, radians counter-clockwise (absent: 0). */
  angle?: number;
  format: ShapeFormat;
}

/** A shape's resolved formatting. */
export interface ShapeFormat {
  fill: Rgba | null;
  line: Rgba | null;
  /** Points. */
  lineWeight: number;
  linePattern: number;
  rounding: number;
  font: string;
  /** Points. */
  size: number;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
  textColor: Rgba;
  align: number;
  valign: number;
  /** Visio arrowhead kinds (0 = none). */
  beginArrow: number;
  endArrow: number;
}

export interface DisplayList {
  page: PageInfo;
  items: Item[];
  /** The page's top-level shapes, back to front. */
  shapes: ShapeBox[];
  warnings: string[];
}

export type QueryResult<Q extends Query> = Q extends { kind: "packageParts" }
  ? PackageInfo
  : Q extends { kind: "docInfo" }
    ? DocInfo
    : Q extends { kind: "derivedStateHash" }
      ? DerivedStateReport
      : Q extends { kind: "pages" }
        ? PageInfo[]
        : Q extends { kind: "pageDisplayList" }
          ? DisplayList
          : Q extends { kind: "library" }
            ? LibraryEntry[]
            : Q extends { kind: "copyShapes" }
              ? ClipShape[]
              : never;

// ---- requests ---------------------------------------------------------------

/** Pointer stream for tools (M2). */
export type GestureEvent = { phase: "down" | "move" | "up"; x: number; y: number; buttons: number };
export type ExportFormat = "pdf" | "svg" | "png";

export type EngineRequest =
  | { kind: "open"; bytes: ArrayBuffer; fileName: string }
  | { kind: "new"; title?: string }
  | { kind: "exec"; doc: DocId; cmd: Command } // undoable
  | { kind: "gesture"; doc: DocId; g: GestureEvent }
  | { kind: "query"; doc: DocId; q: Query } // read-only
  | { kind: "save"; doc: DocId; format: "vsdx" | "vssx" | "vstx" }
  | { kind: "export"; doc: DocId; format: ExportFormat; opts: Record<string, unknown> }
  | { kind: "close"; doc: DocId };

export interface WorkerMessage {
  id: number;
  req: EngineRequest;
}

// ---- events -----------------------------------------------------------------

export interface ChangeSummary {
  meta?: string[];
  shapes?: string[];
  opened?: { fileName: string; format: string; partCount: number };
  /** The shape a command created: [page, id]. */
  created?: [number, number];
  /** The page list changed; the page to show. */
  page?: number;
}

export type SyncState = "local" | "connecting" | "live" | "offline";

export type EngineEvent =
  | { kind: "displayListPatch"; doc: DocId; page: number; patchLen: number }
  | { kind: "selectionChanged"; doc: DocId; sel: ShapeRef[] }
  | { kind: "modelChanged"; doc: DocId; changes: ChangeSummary } // drives panes
  | { kind: "overlay"; doc: DocId; items: unknown[] }
  | { kind: "undoState"; doc: DocId; canUndo: boolean; canRedo: boolean; labels: string[] }
  | { kind: "presence"; doc: DocId; peers: unknown[] }
  | { kind: "syncState"; doc: DocId; state: SyncState; pendingOps: number }
  | { kind: "reply"; id: number; result: unknown }
  | { kind: "error"; id: number | null; message: string };
