// Settings: for now, which Visio file types open in Flowster (desktop app only).

import { useEffect, useState } from "react";

import { fileAssociations, makeDefaultApp, type FileAssociations } from "../host";
import { Dialog } from "./Welcome";

const GROUPS: [string, string[]][] = [
  ["Drawings", ["vsdx", "vsdm"]],
  ["Stencils", ["vssx", "vssm"]],
  ["Templates", ["vstx", "vstm"]],
];

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  return (
    <Dialog title="Settings" onClose={onClose}>
      <OpenWith />
      <div className="dialog-actions">
        <span className="spacer" />
        <button type="button" className="btn primary" autoFocus onClick={onClose}>
          Done
        </button>
      </div>
    </Dialog>
  );
}

function OpenWith() {
  const [state, setState] = useState<FileAssociations | null>(null);
  /** Ticked types, by extension. Types Flowster already opens are ticked and greyed. */
  const [ticked, setTicked] = useState<Set<string>>(new Set(GROUPS.flatMap(([, exts]) => exts)));
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fileAssociations().then(setState, (e) => setError(String(e)));
  }, []);

  const isDefault = (ext: string) => state?.kinds.find((k) => k.ext === ext)?.isDefault === true;
  const wanted = [...ticked].filter((ext) => !isDefault(ext));
  const blocked = !state || !!state.unavailable || working;

  const toggle = (exts: string[], on: boolean) =>
    setTicked((t) => {
      const next = new Set(t);
      for (const ext of exts) {
        if (isDefault(ext)) continue;
        if (on) next.add(ext);
        else next.delete(ext);
      }
      return next;
    });

  const makeDefault = async (exts: string[]) => {
    setWorking(true);
    setError(null);
    try {
      setState(await makeDefaultApp(exts));
    } catch (e) {
      setError(String(e));
      // Some types may have changed before one was refused.
      setState(await fileAssociations().catch(() => state));
    } finally {
      setWorking(false);
    }
  };

  const list = GROUPS.flatMap(([, exts]) => exts.map((e) => `.${e}`)).join(", ");

  return (
    <fieldset className="choice-list open-with">
      <legend>Open with Flowster</legend>
      {state?.platform === "windows" ? (
        <>
          <p className="dialog-text">
            Flowster is offered for {list} files once it's installed. Windows asks you to choose the default app
            yourself, in Settings › Apps › Default apps.
          </p>
          <div className="open-with-actions">
            <button type="button" className="btn" onClick={() => void makeDefault([])} disabled={working}>
              Open Default Apps…
            </button>
          </div>
        </>
      ) : state?.platform === "linux" ? (
        <p className="dialog-text">
          To open {list} files in Flowster, choose it with your file manager's Open With, and tick "Always use".
        </p>
      ) : (
        <>
          <p className="dialog-text">
            Tick the file types, then make Flowster their default app. macOS asks you to confirm each kind of file.
          </p>
          <div className="assoc-groups">
            {GROUPS.map(([label, exts]) => {
              const all = exts.every((e) => ticked.has(e) || isDefault(e));
              const some = exts.some((e) => ticked.has(e) || isDefault(e));
              return (
                <div className="assoc-group" key={label}>
                  <label className="assoc-tick">
                    <input
                      type="checkbox"
                      checked={all}
                      ref={(el) => {
                        if (el) el.indeterminate = some && !all;
                      }}
                      disabled={blocked || exts.every(isDefault)}
                      onChange={(e) => toggle(exts, e.target.checked)}
                    />
                    <strong>{label}</strong>
                  </label>
                  {exts.map((ext) => (
                    <label className="assoc-tick" key={ext}>
                      <input
                        type="checkbox"
                        checked={ticked.has(ext) || isDefault(ext)}
                        disabled={blocked || isDefault(ext)}
                        onChange={(e) => toggle([ext], e.target.checked)}
                      />
                      <code>.{ext}</code>
                    </label>
                  ))}
                </div>
              );
            })}
          </div>
          <p className="dialog-text muted small">
            Greyed ticks are files Flowster already opens. To open them with another app, use Get Info › Open with ›
            Change All in Finder.
          </p>
          {state?.unavailable && <p className="dialog-text muted small">{state.unavailable}</p>}
          <div className="open-with-actions">
            <button
              type="button"
              className="btn"
              onClick={() => void makeDefault(wanted)}
              disabled={blocked || wanted.length === 0}
            >
              {working ? "Waiting for macOS…" : "Make Flowster the Default App"}
            </button>
          </div>
        </>
      )}
      {error && (
        <p className="dialog-text error-text" role="alert">
          {error}
        </p>
      )}
    </fieldset>
  );
}
