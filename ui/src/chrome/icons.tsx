// Flowster's icon set: 20 px line icons on a 24-unit grid, drawn for this app.

import type { ReactNode, SVGProps } from "react";

export type IconName =
  | "logo"
  | "new"
  | "open"
  | "save"
  | "undo"
  | "redo"
  | "pointer"
  | "rectangle"
  | "ellipse"
  | "line"
  | "trash"
  | "zoomIn"
  | "zoomOut"
  | "fit"
  | "text"
  | "panel"
  | "package"
  | "drawing"
  | "keyboard"
  | "info"
  | "check"
  | "chevron"
  | "selectAll"
  | "close"
  | "page"
  | "connector"
  | "shapes"
  | "bold"
  | "italic"
  | "underline"
  | "alignLeft"
  | "alignCenter"
  | "alignRight"
  | "fill"
  | "lineColor"
  | "lineWeight"
  | "dash"
  | "valignTop"
  | "valignMiddle"
  | "valignBottom"
  | "search"
  | "noColor"
  | "plus"
  | "grid"
  | "ruler"
  | "autoFit"
  | "rename"
  | "eye"
  | "eyeOff"
  | "shapeAlignLeft"
  | "shapeAlignCenter"
  | "shapeAlignRight"
  | "shapeAlignTop"
  | "shapeAlignMiddle"
  | "shapeAlignBottom"
  | "distributeH"
  | "distributeV"
  | "rotateLeft"
  | "rotateRight"
  | "flipH"
  | "flipV"
  | "position"
  | "front"
  | "back"
  | "forward"
  | "backward"
  | "group"
  | "ungroup"
  | "copy"
  | "cut"
  | "paste"
  | "duplicate"
  | "brush"
  | "command"
  | "magnet"
  | "guides"
  | "map"
  | "focus"
  | "export"
  | "print";

const paths: Record<IconName, ReactNode> = {
  logo: (
    <>
      <rect x="3" y="4" width="7" height="6" rx="1.5" />
      <rect x="14" y="14" width="7" height="6" rx="1.5" />
      <path d="M10 7h3.5a2.5 2.5 0 0 1 2.5 2.5V14" />
    </>
  ),
  new: (
    <>
      <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
      <path d="M14 3v5h5M12 11v6M9 14h6" />
    </>
  ),
  open: (
    <>
      <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v1" />
      <path d="M3 7v11a1 1 0 0 0 1 1h14.2a1 1 0 0 0 .97-.76L21.6 11.2a1 1 0 0 0-.97-1.2H7.3a1 1 0 0 0-.96.73L3 19" />
    </>
  ),
  save: (
    <>
      <path d="M5 3h11l3 3v13a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z" />
      <path d="M7 3v5h8V3M7 21v-7h10v7" />
    </>
  ),
  undo: (
    <>
      <path d="M9 14 4 9l5-5" />
      <path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11" />
    </>
  ),
  redo: (
    <>
      <path d="m15 14 5-5-5-5" />
      <path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13" />
    </>
  ),
  pointer: <path d="M5 3l14 7.5-6.2 1.6L10 18.5z M12.8 12.1 18 19" />,
  rectangle: <rect x="3.5" y="6" width="17" height="12" rx="2" />,
  ellipse: <ellipse cx="12" cy="12" rx="9" ry="6.5" />,
  line: (
    <>
      <path d="M5 19 19 5" />
      <circle cx="5" cy="19" r="1.6" />
      <circle cx="19" cy="5" r="1.6" />
    </>
  ),
  trash: (
    <>
      <path d="M4 7h16M10 11v6M14 11v6" />
      <path d="M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3" />
    </>
  ),
  zoomIn: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-4-4M11 8v6M8 11h6" />
    </>
  ),
  zoomOut: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-4-4M8 11h6" />
    </>
  ),
  fit: <path d="M4 9V5a1 1 0 0 1 1-1h4M15 4h4a1 1 0 0 1 1 1v4M20 15v4a1 1 0 0 1-1 1h-4M9 20H5a1 1 0 0 1-1-1v-4" />,
  text: <path d="M5 6V4h14v2M12 4v16M9 20h6" />,
  panel: (
    <>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M15 4v16" />
    </>
  ),
  package: (
    <>
      <path d="M21 8 12 3 3 8v8l9 5 9-5z" />
      <path d="m3 8 9 5 9-5M12 13v8" />
    </>
  ),
  drawing: (
    <>
      <rect x="3" y="3" width="8" height="7" rx="1.5" />
      <circle cx="17" cy="17" r="4" />
      <path d="M7 10v7h6" />
    </>
  ),
  keyboard: (
    <>
      <rect x="2.5" y="6" width="19" height="12" rx="2" />
      <path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10" />
    </>
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v6M12 7.5h.01" />
    </>
  ),
  check: <path d="m5 12.5 4.5 4.5L19 7.5" />,
  chevron: <path d="m6 9 6 6 6-6" />,
  selectAll: (
    <>
      <path d="M4 7V5a1 1 0 0 1 1-1h2M17 4h2a1 1 0 0 1 1 1v2M20 17v2a1 1 0 0 1-1 1h-2M7 20H5a1 1 0 0 1-1-1v-2" />
      <rect x="8" y="8" width="8" height="8" rx="1" />
    </>
  ),
  close: <path d="M6 6l12 12M18 6 6 18" />,
  connector: (
    <>
      <rect x="2.5" y="3" width="6" height="5" rx="1.2" />
      <rect x="15.5" y="16" width="6" height="5" rx="1.2" />
      <path d="M8.5 5.5H12v13h3.5M13.5 16.5l2 2-2 2" />
    </>
  ),
  shapes: (
    <>
      <rect x="3" y="3" width="8" height="8" rx="1.5" />
      <circle cx="17" cy="7" r="4" />
      <path d="M7 14l4.5 7h-9z" />
      <rect x="13.5" y="14" width="7" height="7" rx="1.5" transform="rotate(45 17 17.5)" />
    </>
  ),
  bold: <path d="M7 4h6a4 4 0 0 1 0 8H7zM7 12h7a4 4 0 0 1 0 8H7z" />,
  italic: <path d="M10 4h8M6 20h8M14 4l-4 16" />,
  underline: <path d="M7 4v7a5 5 0 0 0 10 0V4M5 20h14" />,
  alignLeft: <path d="M4 6h16M4 10h10M4 14h16M4 18h10" />,
  alignCenter: <path d="M4 6h16M7 10h10M4 14h16M7 18h10" />,
  alignRight: <path d="M4 6h16M10 10h10M4 14h16M10 18h10" />,
  fill: (
    <>
      <path d="m5 12 7-7 7 7-7 7z" />
      <path d="M12 5 9 2M19 12v0" />
      <path d="M20 16s1.5 1.8 1.5 3a1.5 1.5 0 0 1-3 0c0-1.2 1.5-3 1.5-3z" />
    </>
  ),
  lineColor: (
    <>
      <path d="M4 20h16" strokeWidth="3" />
      <path d="m14.5 4.5 4 4L9 18H5v-4z" />
    </>
  ),
  lineWeight: (
    <>
      <path d="M4 6h16" strokeWidth="1" />
      <path d="M4 11h16" strokeWidth="2" />
      <path d="M4 17h16" strokeWidth="3.5" />
    </>
  ),
  dash: <path d="M3 12h4M10 12h4M17 12h4" />,
  valignTop: <path d="M4 4h16M12 8v12M8 12l4-4 4 4" />,
  valignMiddle: <path d="M4 12h16M12 3v5M12 16v5M9 6l3 2 3-2M9 18l3-2 3 2" />,
  valignBottom: <path d="M4 20h16M12 4v12M8 12l4 4 4-4" />,
  search: (
    <>
      <circle cx="11" cy="11" r="6.5" />
      <path d="m20 20-4.2-4.2" />
    </>
  ),
  noColor: (
    <>
      <rect x="4" y="4" width="16" height="16" rx="2" />
      <path d="M5 19 19 5" />
    </>
  ),
  page: (
    <>
      <path d="M6 3h8l4 4v13a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z" />
      <path d="M14 3v4h4" />
    </>
  ),
  plus: <path d="M12 5v14M5 12h14" />,
  grid: (
    <>
      <rect x="4" y="4" width="16" height="16" rx="1.5" />
      <path d="M9.33 4v16M14.67 4v16M4 9.33h16M4 14.67h16" />
    </>
  ),
  ruler: (
    <>
      <path d="M3 8.5 8.5 3 21 15.5 15.5 21z" />
      <path d="m7 7 2 2M10 4.5l1.5 1.5M10 10l2 2M13 7.5l1.5 1.5M13 13l2 2M16 10.5l1.5 1.5" />
    </>
  ),
  autoFit: (
    <>
      <path d="M4 9V5a1 1 0 0 1 1-1h4M15 4h4a1 1 0 0 1 1 1v4M20 15v4a1 1 0 0 1-1 1h-4M9 20H5a1 1 0 0 1-1-1v-4" />
      <path d="M9.5 12h5M8 12l1.5-1.5M8 12l1.5 1.5M16 12l-1.5-1.5M16 12l-1.5 1.5" />
    </>
  ),
  rename: (
    <>
      <path d="M4 20h4L19 9a2.1 2.1 0 0 0-3-3L5 17z" />
      <path d="m14.5 7.5 3 3" />
    </>
  ),
  eye: (
    <>
      <path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z" />
      <circle cx="12" cy="12" r="3" />
    </>
  ),
  eyeOff: (
    <>
      <path d="M9.9 5.8A9.6 9.6 0 0 1 12 5.5C18 5.5 21.5 12 21.5 12a17 17 0 0 1-2.7 3.4M6.6 6.9C4 8.6 2.5 12 2.5 12S6 18.5 12 18.5a9.3 9.3 0 0 0 4.9-1.4" />
      <path d="M9.9 9.9a3 3 0 0 0 4.2 4.2M3 3l18 18" />
    </>
  ),
  shapeAlignLeft: (
    <>
      <path d="M4 3v18" />
      <rect x="7" y="5.5" width="13" height="5" rx="1" />
      <rect x="7" y="13.5" width="8" height="5" rx="1" />
    </>
  ),
  shapeAlignCenter: (
    <>
      <path d="M12 3v18" />
      <rect x="4.5" y="5.5" width="15" height="5" rx="1" />
      <rect x="7.5" y="13.5" width="9" height="5" rx="1" />
    </>
  ),
  shapeAlignRight: (
    <>
      <path d="M20 3v18" />
      <rect x="4" y="5.5" width="13" height="5" rx="1" />
      <rect x="9" y="13.5" width="8" height="5" rx="1" />
    </>
  ),
  shapeAlignTop: (
    <>
      <path d="M3 4h18" />
      <rect x="5.5" y="7" width="5" height="13" rx="1" />
      <rect x="13.5" y="7" width="5" height="8" rx="1" />
    </>
  ),
  shapeAlignMiddle: (
    <>
      <path d="M3 12h18" />
      <rect x="5.5" y="4.5" width="5" height="15" rx="1" />
      <rect x="13.5" y="7.5" width="5" height="9" rx="1" />
    </>
  ),
  shapeAlignBottom: (
    <>
      <path d="M3 20h18" />
      <rect x="5.5" y="4" width="5" height="13" rx="1" />
      <rect x="13.5" y="9" width="5" height="8" rx="1" />
    </>
  ),
  distributeH: (
    <>
      <path d="M3 4v16M21 4v16" />
      <rect x="6" y="8" width="4" height="8" rx="1" />
      <rect x="14" y="8" width="4" height="8" rx="1" />
    </>
  ),
  distributeV: (
    <>
      <path d="M4 3h16M4 21h16" />
      <rect x="8" y="6" width="8" height="4" rx="1" />
      <rect x="8" y="14" width="8" height="4" rx="1" />
    </>
  ),
  rotateLeft: (
    <>
      <path d="M4 4v5h5" />
      <path d="M4.6 9A8 8 0 1 1 6 16.9" />
    </>
  ),
  rotateRight: (
    <>
      <path d="M20 4v5h-5" />
      <path d="M19.4 9A8 8 0 1 0 18 16.9" />
    </>
  ),
  flipH: (
    <>
      <path d="M12 3v18" strokeDasharray="2 2.5" />
      <path d="M9 6 3 18h6zM15 6l6 12h-6z" />
    </>
  ),
  flipV: (
    <>
      <path d="M3 12h18" strokeDasharray="2 2.5" />
      <path d="M6 9 18 3v6zM6 15l12 6v-6z" />
    </>
  ),
  position: (
    <>
      <rect x="3.5" y="13.5" width="7" height="7" rx="1" />
      <rect x="13.5" y="3.5" width="7" height="7" rx="1" />
      <path d="M7 3v6M4 6h6M17 14v6M14 17h6" />
    </>
  ),
  front: (
    <>
      <rect x="3.5" y="3.5" width="10" height="10" rx="1.5" strokeDasharray="2 2" />
      <rect x="10.5" y="10.5" width="10" height="10" rx="1.5" fill="currentColor" fillOpacity="0.25" />
    </>
  ),
  back: (
    <>
      <rect x="10.5" y="10.5" width="10" height="10" rx="1.5" fill="currentColor" fillOpacity="0.25" />
      <rect x="3.5" y="3.5" width="10" height="10" rx="1.5" strokeDasharray="2 2" />
    </>
  ),
  forward: (
    <>
      <rect x="3.5" y="9.5" width="9" height="9" rx="1.5" strokeDasharray="2 2" />
      <rect x="9.5" y="5.5" width="9" height="9" rx="1.5" fill="currentColor" fillOpacity="0.25" />
      <path d="M20 21v-4M18 19l2-2 2 2" />
    </>
  ),
  backward: (
    <>
      <rect x="9.5" y="5.5" width="9" height="9" rx="1.5" fill="currentColor" fillOpacity="0.25" />
      <rect x="3.5" y="9.5" width="9" height="9" rx="1.5" strokeDasharray="2 2" />
      <path d="M20 17v4M18 19l2 2 2-2" />
    </>
  ),
  group: (
    <>
      <path d="M3 6V3h3M18 3h3v3M21 18v3h-3M6 21H3v-3" />
      <rect x="6.5" y="6.5" width="6" height="6" rx="1" />
      <rect x="11.5" y="11.5" width="6" height="6" rx="1" />
    </>
  ),
  ungroup: (
    <>
      <rect x="3.5" y="3.5" width="7" height="7" rx="1" />
      <rect x="13.5" y="13.5" width="7" height="7" rx="1" />
      <path d="M14 4h6v6M4 14v6h6" strokeDasharray="2 2" />
    </>
  ),
  copy: (
    <>
      <rect x="8.5" y="8.5" width="12" height="12" rx="2" />
      <path d="M15.5 8.5V5a1.5 1.5 0 0 0-1.5-1.5H5A1.5 1.5 0 0 0 3.5 5v9A1.5 1.5 0 0 0 5 15.5h3.5" />
    </>
  ),
  cut: (
    <>
      <circle cx="6.5" cy="17.5" r="3" />
      <circle cx="17.5" cy="17.5" r="3" />
      <path d="M8.6 15.4 18 4M15.4 15.4 6 4" />
    </>
  ),
  paste: (
    <>
      <path d="M9 4.5H6.5A1.5 1.5 0 0 0 5 6v13.5A1.5 1.5 0 0 0 6.5 21h11a1.5 1.5 0 0 0 1.5-1.5V6a1.5 1.5 0 0 0-1.5-1.5H15" />
      <rect x="9" y="3" width="6" height="3.5" rx="1" />
    </>
  ),
  duplicate: (
    <>
      <rect x="8.5" y="8.5" width="12" height="12" rx="2" />
      <path d="M15.5 8.5V5a1.5 1.5 0 0 0-1.5-1.5H5A1.5 1.5 0 0 0 3.5 5v9A1.5 1.5 0 0 0 5 15.5h3.5M14.5 11.5v6M11.5 14.5h6" />
    </>
  ),
  brush: (
    <>
      <path d="M18.5 3.5 10 12l2 2 8.5-8.5a1.4 1.4 0 0 0-2-2z" />
      <path d="M10 12c-2.5 0-4 1.5-4 4 0 1.5-.8 2.5-2.5 3 4.5 1.5 10.5.5 10.5-5z" />
    </>
  ),
  command: (
    <>
      <rect x="3" y="5" width="18" height="14" rx="2.5" />
      <path d="m7 10 3 2-3 2M12.5 14.5H17" />
    </>
  ),
  magnet: (
    <>
      <path d="M5 4h4v8a3 3 0 0 0 6 0V4h4v8a7 7 0 0 1-14 0z" />
      <path d="M5 8h4M15 8h4" />
    </>
  ),
  guides: (
    <>
      <path d="M12 2.5v19M2.5 12h19" strokeDasharray="2 2.5" />
      <rect x="5" y="5" width="7" height="7" rx="1" />
      <rect x="12" y="12" width="7" height="7" rx="1" />
    </>
  ),
  map: (
    <>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <rect x="12" y="12" width="7" height="6" rx="1" />
    </>
  ),
  focus: (
    <>
      <path d="M4 9V5a1 1 0 0 1 1-1h4M15 4h4a1 1 0 0 1 1 1v4M20 15v4a1 1 0 0 1-1 1h-4M9 20H5a1 1 0 0 1-1-1v-4" />
      <rect x="9" y="9" width="6" height="6" rx="1" />
    </>
  ),
  export: (
    <>
      <path d="M12 15V3.5M7.5 8 12 3.5 16.5 8" />
      <path d="M5 13v5.5A1.5 1.5 0 0 0 6.5 20h11a1.5 1.5 0 0 0 1.5-1.5V13" />
    </>
  ),
  print: (
    <>
      <path d="M7 8V3.5h10V8" />
      <rect x="3.5" y="8" width="17" height="8" rx="1.5" />
      <path d="M7 13.5h10V20.5H7z" />
    </>
  ),
};

export function Icon({ name, size = 18, ...rest }: { name: IconName; size?: number } & SVGProps<SVGSVGElement>) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...rest}
    >
      {paths[name]}
    </svg>
  );
}

/** Platform-appropriate shortcut text: "Ctrl+S" or "⌘S". */
export function shortcut(keys: string): string {
  const mac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);
  if (!mac) return keys;
  return keys.replace(/Ctrl\+/g, "⌘").replace(/Shift\+/g, "⇧").replace(/Alt\+/g, "⌥");
}
