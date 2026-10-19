// The application menu bar (File, Edit, View, …): click or Alt to open, hover
// to move between menus while one is open, arrows/Enter/Escape to navigate.

import { useEffect, useRef, useState } from "react";

import { Icon, shortcut, type IconName } from "./icons";

export type MenuEntry =
  | {
      label: string;
      icon?: IconName;
      keys?: string;
      disabled?: boolean;
      checked?: boolean;
      run: () => void;
    }
  | "separator";

export interface Menu {
  label: string;
  items: MenuEntry[];
}

export function MenuBar({ menus }: { menus: Menu[] }) {
  const [open, setOpen] = useState<number | null>(null);
  const [active, setActive] = useState(-1);
  const bar = useRef<HTMLDivElement>(null);

  // Close on a click elsewhere.
  useEffect(() => {
    if (open === null) return;
    const away = (e: PointerEvent) => {
      if (!bar.current?.contains(e.target as Node)) setOpen(null);
    };
    window.addEventListener("pointerdown", away);
    return () => window.removeEventListener("pointerdown", away);
  }, [open]);

  // Alt alone focuses the first menu, as desktop apps do.
  useEffect(() => {
    let alone = false;
    const down = (e: KeyboardEvent) => {
      alone = e.key === "Alt";
    };
    const up = (e: KeyboardEvent) => {
      if (e.key === "Alt" && alone) {
        e.preventDefault();
        setOpen((o) => (o === null ? 0 : null));
        setActive(-1);
      }
      alone = false;
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    };
  }, []);

  const enabled = (m: number) =>
    menus[m].items.flatMap((it, i) => (it !== "separator" && !it.disabled ? [i] : []));

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (open === null) return;
    const items = enabled(open);
    const pos = items.indexOf(active);
    if (e.key === "Escape") {
      setOpen(null);
      (document.activeElement as HTMLElement | null)?.blur();
    } else if (e.key === "ArrowDown") {
      setActive(items[(pos + 1) % items.length] ?? -1);
    } else if (e.key === "ArrowUp") {
      setActive(items[(pos - 1 + items.length) % items.length] ?? -1);
    } else if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
      const next = (open + (e.key === "ArrowRight" ? 1 : menus.length - 1)) % menus.length;
      setOpen(next);
      setActive(-1);
      bar.current?.querySelectorAll<HTMLElement>(".menu-title")[next]?.focus();
    } else if (e.key === "Enter" && active >= 0) {
      const it = menus[open].items[active];
      if (it !== "separator" && !it.disabled) {
        setOpen(null);
        it.run();
      }
    } else {
      return;
    }
    e.preventDefault();
    e.stopPropagation();
  };

  return (
    <div className="menubar" role="menubar" ref={bar} onKeyDown={onKeyDown}>
      {menus.map((m, mi) => (
        <div className="menu" key={m.label}>
          <button
            type="button"
            role="menuitem"
            aria-haspopup="menu"
            aria-expanded={open === mi}
            className="menu-title"
            onClick={() => {
              setOpen(open === mi ? null : mi);
              setActive(-1);
            }}
            onPointerEnter={() => {
              if (open !== null && open !== mi) {
                setOpen(mi);
                setActive(-1);
              }
            }}
          >
            {m.label}
          </button>
          {open === mi && (
            <div className="menu-panel" role="menu" aria-label={m.label}>
              {m.items.map((it, i) =>
                it === "separator" ? (
                  <div className="menu-sep" role="separator" key={`sep${i}`} />
                ) : (
                  <button
                    type="button"
                    role={it.checked === undefined ? "menuitem" : "menuitemcheckbox"}
                    aria-checked={it.checked}
                    key={it.label}
                    className={`menu-item${active === i ? " active" : ""}`}
                    disabled={it.disabled}
                    onPointerEnter={() => setActive(i)}
                    onClick={() => {
                      setOpen(null);
                      (document.activeElement as HTMLElement | null)?.blur();
                      it.run();
                    }}
                  >
                    <span className="menu-icon">
                      {it.checked ? <Icon name="check" size={16} /> : it.icon && <Icon name={it.icon} size={16} />}
                    </span>
                    <span className="menu-label">{it.label}</span>
                    {it.keys && <kbd className="menu-keys">{shortcut(it.keys)}</kbd>}
                  </button>
                ),
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
