// Menus that pop up from a point: toolbar drop-downs and right-click menus.
// They share the menu bar's entries and look; arrows/Enter/Escape navigate,
// and a click elsewhere, a scroll or a resize closes them.

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";

import { Icon, shortcut, type IconName } from "./icons";
import type { MenuEntry } from "./MenuBar";

export function PopupMenu({
  items,
  at,
  label,
  onClose,
  anchor,
}: {
  items: MenuEntry[];
  /** Viewport point for the menu's top-left corner; it shifts to stay on screen. */
  at: [number, number];
  label: string;
  onClose: () => void;
  /** The control that opened the menu: pressing it again toggles rather than reopens. */
  anchor?: RefObject<HTMLElement | null>;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState(at);
  const [active, setActive] = useState(-1);

  useLayoutEffect(() => {
    const el = panel.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const x = Math.max(4, Math.min(at[0], window.innerWidth - r.width - 4));
    // Opens upward when there is no room below (menus from the status bar).
    const y = at[1] + r.height > window.innerHeight - 4 ? Math.max(4, at[1] - r.height) : at[1];
    setPos([x, y]);
    el.focus();
  }, [at]);

  useEffect(() => {
    const inside = (el: HTMLElement | null | undefined, t: EventTarget | null) => !!el?.contains(t as Node);
    const away = (e: PointerEvent) => !inside(panel.current, e.target) && !inside(anchor?.current, e.target) && onClose();
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("resize", onClose);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("resize", onClose);
      window.removeEventListener("blur", onClose);
    };
  }, [anchor, onClose]);

  const enabled = items.flatMap((it, i) => (it !== "separator" && !it.disabled ? [i] : []));
  const run = (it: MenuEntry) => {
    if (it === "separator" || it.disabled) return;
    onClose();
    it.run();
  };

  return (
    <div
      className="menu-panel popup"
      role="menu"
      aria-label={label}
      tabIndex={-1}
      ref={panel}
      style={{ left: pos[0], top: pos[1] }}
      onContextMenu={(e) => e.preventDefault()}
      onKeyDown={(e) => {
        const p = enabled.indexOf(active);
        if (e.key === "Escape") onClose();
        else if (e.key === "ArrowDown") setActive(enabled[(p + 1) % enabled.length] ?? -1);
        else if (e.key === "ArrowUp") setActive(enabled[(p - 1 + enabled.length) % enabled.length] ?? -1);
        else if (e.key === "Enter" && active >= 0) run(items[active]);
        else return;
        e.preventDefault();
        e.stopPropagation();
      }}
    >
      {items.map((it, i) =>
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
            onClick={() => run(it)}
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
  );
}

/**
 * A toolbar button with a drop-down. With `onClick` it is a split button: the
 * icon runs the main action and the arrow opens the menu.
 */
export function DropButton({
  icon,
  label,
  keys,
  items,
  onClick,
  disabled,
}: {
  icon: IconName;
  label: string;
  keys?: string;
  items: MenuEntry[];
  onClick?: () => void;
  disabled?: boolean;
}) {
  const [at, setAt] = useState<[number, number] | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const close = useCallback(() => setAt(null), []);
  const open = () => {
    const r = box.current!.getBoundingClientRect();
    setAt(at ? null : [r.left, r.bottom + 4]);
  };
  const tip = keys ? `${label}  ${shortcut(keys)}` : label;
  let main: ReactNode;
  if (onClick) {
    main = (
      <button type="button" className="tb split-main" aria-label={label} data-tip={tip} onClick={onClick} disabled={disabled}>
        <Icon name={icon} />
      </button>
    );
  }
  return (
    <div className={`drop${onClick ? " split" : ""}`} ref={box}>
      {main}
      <button
        type="button"
        className={onClick ? "tb split-arrow" : "tb drop-btn"}
        aria-label={onClick ? `More ${label.toLowerCase()} options` : label}
        aria-haspopup="menu"
        aria-expanded={at !== null}
        data-tip={onClick ? undefined : tip}
        disabled={disabled}
        onClick={open}
      >
        {!onClick && <Icon name={icon} />}
        <Icon name="chevron" size={12} />
      </button>
      {at && <PopupMenu items={items} at={at} label={label} anchor={box} onClose={close} />}
    </div>
  );
}
