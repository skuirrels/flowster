// OS services. In the Tauri app they go through the host's commands (SPEC §5.4,
// §21); in a plain browser (`pnpm dev`) they fall back to web APIs.

/**
 * Where File > Save writes without asking: the host's handle for a `.vsdx` the
 * user opened or saved (the app), or a File System Access handle (a browser
 * that has the API). The page never sees a path.
 */
export type SaveHandle = number | FileSystemFileHandle;

export interface PickedFile {
  fileName: string;
  bytes: ArrayBuffer;
  /** Set when the file can be saved back in place (a `.vsdx`). */
  handle?: SaveHandle;
}

export interface SavedFile {
  fileName: string;
  handle?: SaveHandle;
}

export type ExportFormat = "svg" | "png";

export const VISIO_EXTENSIONS = [".vsdx", ".vssx", ".vstx", ".vsdm", ".vssm", ".vstm", ".vsd"];

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const isVsdx = (name: string) => /\.vsdx$/i.test(name);

// The File System Access API (Chromium browsers), when there is one.
interface PickerWindow {
  showOpenFilePicker?: (opts: unknown) => Promise<FileSystemFileHandle[]>;
  showSaveFilePicker?: (opts: unknown) => Promise<FileSystemFileHandle>;
}
const picker = (typeof window !== "undefined" ? window : {}) as PickerWindow;

const TYPES = {
  vsdx: { description: "Visio drawing", accept: { "application/vnd.ms-visio.drawing": [".vsdx"] } },
  svg: { description: "SVG image", accept: { "image/svg+xml": [".svg"] } },
  png: { description: "PNG image", accept: { "image/png": [".png"] } },
  pdf: { description: "PDF document", accept: { "application/pdf": [".pdf"] } },
};

const isAbort = (e: unknown) => e instanceof DOMException && e.name === "AbortError";

/** Shows the platform's open dialog and reads the chosen file, or returns null if cancelled. */
export async function pickDocument(): Promise<PickedFile | null> {
  if (inTauri) return pickViaHost();
  if (picker.showOpenFilePicker) {
    try {
      const [handle] = await picker.showOpenFilePicker({
        types: [{ description: "Visio documents", accept: { "application/vnd.ms-visio.drawing": VISIO_EXTENSIONS } }],
      });
      const file = await handle.getFile();
      return { fileName: file.name, bytes: await file.arrayBuffer(), handle: isVsdx(file.name) ? handle : undefined };
    } catch (e) {
      if (isAbort(e)) return null;
      throw e;
    }
  }
  return pickViaInput();
}

async function pickViaHost(): Promise<PickedFile | null> {
  const { invoke } = await import("@tauri-apps/api/core");
  // The host shows the dialog itself and only ever reads the path the user picked.
  const picked = await invoke<{ token: number; fileName: string } | null>("pick_document");
  if (!picked) return null;
  const bytes = await invoke<ArrayBuffer>("read_picked_document", { token: picked.token });
  // The host keeps a .vsdx's token as its save handle.
  return { fileName: picked.fileName, bytes, handle: isVsdx(picked.fileName) ? picked.token : undefined };
}

/**
 * Saves a `.vsdx`: back to `handle` without asking when there is one (File >
 * Save), else where the user chooses (Save As). In a browser without the File
 * System Access API it is a download. Returns null if cancelled.
 */
export async function saveDocument(fileName: string, bytes: ArrayBuffer, handle?: SaveHandle): Promise<SavedFile | null> {
  if (inTauri) {
    const { invoke } = await import("@tauri-apps/api/core");
    const headers: Record<string, string> = { "x-file-name": fileName, "x-format": "vsdx" };
    if (typeof handle === "number") headers["x-handle"] = String(handle);
    return invoke<SavedFile | null>("save_document", new Uint8Array(bytes), { headers });
  }
  return saveInBrowser(fileName, bytes, "vsdx", handle);
}

/** Exports an SVG or PNG where the user chooses. Returns the file name, or null if cancelled. */
export async function exportFile(fileName: string, bytes: ArrayBuffer, format: ExportFormat): Promise<string | null> {
  if (inTauri) {
    const { invoke } = await import("@tauri-apps/api/core");
    const saved = await invoke<SavedFile | null>("save_document", new Uint8Array(bytes), {
      headers: { "x-file-name": fileName, "x-format": format },
    });
    return saved?.fileName ?? null;
  }
  return (await saveInBrowser(fileName, bytes, format))?.fileName ?? null;
}

/** Whether PDF export is available: it needs the desktop host's fonts (a browser prints to PDF instead). */
export const canExportPdf = inTauri;

/** Exports pages (indices) of a document, given as `.vsdx` bytes, as a PDF made by the host. */
export async function exportPdf(fileName: string, vsdx: ArrayBuffer, pages: number[]): Promise<string | null> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string | null>("export_pdf", new Uint8Array(vsdx), {
    headers: { "x-file-name": fileName, "x-pages": pages.join(",") },
  });
}

/** Help > Check for Updates: the host checks the signed update feed and asks in its own dialogs. */
export async function checkForUpdates(): Promise<void> {
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("check_for_updates");
}

/**
 * Calls `onOpen` with a file the system asks Flowster to open (a double-click
 * in Finder or Explorer, Open With), including one that arrived before the page
 * was ready. The host only ever hands over files the user chose.
 */
export async function watchOpenedDocuments(onOpen: (file: PickedFile) => void, onError: (e: unknown) => void): Promise<void> {
  if (!inTauri) return;
  const { Channel, invoke } = await import("@tauri-apps/api/core");
  const channel = new Channel<{ token: number; fileName: string }>();
  channel.onmessage = (picked) =>
    void invoke<ArrayBuffer>("read_picked_document", { token: picked.token }).then(
      (bytes) => onOpen({ fileName: picked.fileName, bytes, handle: isVsdx(picked.fileName) ? picked.token : undefined }),
      onError,
    );
  await invoke("watch_opened_documents", { onOpen: channel });
}

/** Settings > Open with Flowster: the file types Flowster opens, and whether it is their default app. */
export interface FileAssociations {
  /** How the default app is chosen: here (macOS), in Windows Settings, or in the file manager (Linux). */
  platform: "macos" | "windows" | "linux";
  /** Why Flowster can't make itself the default (a development build, an old macOS). */
  unavailable: string | null;
  /** `isDefault` is null where the system doesn't say. */
  kinds: { ext: string; isDefault: boolean | null }[];
}

export async function fileAssociations(): Promise<FileAssociations> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<FileAssociations>("file_associations");
}

/**
 * Makes Flowster the default app for these extensions (no dots). macOS asks the
 * user to confirm each one; on Windows this opens Default apps in Settings.
 * Returns the state afterwards.
 */
export async function makeDefaultApp(extensions: string[]): Promise<FileAssociations> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<FileAssociations>("make_default_app", { extensions });
}

/** Tells the host whether there are unsaved changes, so closing the window can warn. */
export async function setDocumentState(dirty: boolean, title: string): Promise<void> {
  if (!inTauri) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("set_document_state", { dirty, title });
}

async function saveInBrowser(
  fileName: string,
  bytes: ArrayBuffer,
  format: keyof typeof TYPES,
  handle?: SaveHandle,
): Promise<SavedFile | null> {
  if (picker.showSaveFilePicker) {
    try {
      const target =
        handle && typeof handle !== "number"
          ? handle
          : await picker.showSaveFilePicker({ suggestedName: fileName, types: [TYPES[format]] });
      const out = await target.createWritable();
      await out.write(bytes);
      await out.close();
      return { fileName: target.name, handle: format === "vsdx" && isVsdx(target.name) ? target : undefined };
    } catch (e) {
      if (isAbort(e)) return null;
      throw e;
    }
  }
  const url = URL.createObjectURL(new Blob([bytes], { type: Object.keys(TYPES[format].accept)[0] }));
  const a = document.createElement("a");
  a.href = url;
  a.download = fileName;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
  return { fileName };
}

function pickViaInput(): Promise<PickedFile | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = VISIO_EXTENSIONS.join(",");
    input.onchange = async () => {
      const file = input.files?.[0];
      resolve(file ? { fileName: file.name, bytes: await file.arrayBuffer() } : null);
    };
    input.oncancel = () => resolve(null);
    input.click();
  });
}
