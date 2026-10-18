// OS services. In the Tauri app they go through the host's commands (SPEC §5.4,
// §21); in a plain browser (`pnpm dev`) they fall back to web APIs.

export interface PickedFile {
  fileName: string;
  bytes: ArrayBuffer;
}

export const VISIO_EXTENSIONS = [".vsdx", ".vssx", ".vstx", ".vsdm", ".vssm", ".vstm", ".vsd"];

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Shows the platform's open dialog and reads the chosen file, or returns null if cancelled. */
export async function pickDocument(): Promise<PickedFile | null> {
  return inTauri ? pickViaHost() : pickViaInput();
}

async function pickViaHost(): Promise<PickedFile | null> {
  const { invoke } = await import("@tauri-apps/api/core");
  // The host shows the dialog itself and only ever reads the path the user picked.
  const picked = await invoke<{ token: number; fileName: string } | null>("pick_document");
  if (!picked) return null;
  const bytes = await invoke<ArrayBuffer>("read_picked_document", { token: picked.token });
  return { fileName: picked.fileName, bytes };
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
