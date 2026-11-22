// Setu's icon set: simple line icons drawn on a 24-unit grid, stroked with
// the current text colour so they follow every theme.

const PATHS = {
  // A bridge: Setu means bridge.
  logo: "M3 16h18M5 16V11M19 16V11M3 11c3-4 6-6 9-6s6 2 9 6M9 16V8.6M15 16V8.6M12 16V7",
  hosts: "M4 5h16v5H4zM4 14h16v5H4zM7.5 7.5h.01M7.5 16.5h.01M11 7.5h5M11 16.5h5",
  terminal: "M4 5h16v14H4zM7.5 10l3 2.5-3 2.5M12.5 15H16",
  key: "M14.5 9.5a4 4 0 1 1-8 0 4 4 0 0 1 8 0zM13.4 12.4 20 19M17.5 16.5 19.5 14.5M15.5 14.5l1.5-1.5",
  vault: "M12 3 5 6v5c0 4.5 3 8 7 10 4-2 7-5.5 7-10V6zM9.5 11.5h5v4h-5zM10.5 11.5V10a1.5 1.5 0 0 1 3 0v1.5",
  snippet: "M8 8 4 12l4 4M16 8l4 4-4 4M13.5 6l-3 12",
  settings:
    "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 13.5l1.6 1.2-2 3.4-1.9-.7a7.5 7.5 0 0 1-2 1.2L14.8 21h-4l-.3-2.4a7.5 7.5 0 0 1-2-1.2l-1.9.7-2-3.4 1.6-1.2a7.5 7.5 0 0 1 0-2.4L4.6 9.9l2-3.4 1.9.7a7.5 7.5 0 0 1 2-1.2L10.8 3h4l.3 2.4a7.5 7.5 0 0 1 2 1.2l1.9-.7 2 3.4-1.6 1.2a7.5 7.5 0 0 1 0 2.4z",
  plus: "M12 5v14M5 12h14",
  search: "M10.5 17a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13zM15.5 15.5 20 20",
  connect: "M5 12h12M13 7l5 5-5 5",
  files: "M8 4v13M4.5 13.5 8 17l3.5-3.5M16 20V7M12.5 10.5 16 7l3.5 3.5",
  edit: "M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4",
  import: "M12 4v11M7.5 10.5 12 15l4.5-4.5M5 19h14",
  map: "M5 6.5a1.5 1.5 0 1 0 0 .01M19 5.5a1.5 1.5 0 1 0 0 .01M19 18.5a1.5 1.5 0 1 0 0 .01M6.5 6.5h4.5l3 -1h3.5M11 6.5l3 12h3.5",
  jump: "M4 17c3 0 4-10 8-10s5 10 8 10M12 7V4",
  menu: "M4 7h16M4 12h16M4 17h16",
  sidebar: "M4 5h16v14H4zM9.5 5v14",
  command: "M9 6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3z",
  close: "M6 6l12 12M18 6 6 18",
  folder: "M3.5 7V18.5h17V8.5H11l-2-2.5H3.5z",
  chevronDown: "M7 10l5 5 5-5",
  chevronRight: "M10 7l5 5-5 5",
  pause: "M9 6v12M15 6v12",
  lock: "M6.5 11h11v9h-11zM8.5 11V8a3.5 3.5 0 0 1 7 0v3M12 14.5v2",
  folderPlus: "M3.5 7V18.5h17V8.5H11l-2-2.5H3.5zM12 11v5M9.5 13.5h5",
  rename: "M4 17.5V20h2.5L17 9.5 14.5 7zM13 8.5l2.5 2.5M14 20h6",
  trash: "M5 7h14M9.5 7V4.5h5V7M7 7l1 13h8l1-13M10.5 10.5v6M13.5 10.5v6",
  play: "M8 5.5v13l10.5-6.5z",
  layout: "M4 5h16v14H4zM9 5v14M9 10h11",
  splitRight: "M4 5h16v14H4zM12 5v14",
  splitDown: "M4 5h16v14H4zM4 12h16",
  window: "M9 4h11v11M20 4l-9 9M15 20H4V9",
  retry: "M4.5 12a7.5 7.5 0 1 0 2.2-5.3M4.5 4.5v4h4",
  // Three boxes joined by arrows: a workflow.
  workflow: "M3.5 4.5h6v5h-6zM14.5 4.5h6v5h-6zM9 17h6v4H9zM9.5 7h5M17.5 9.5v3.5H12V17M6.5 9.5v3.5H12",
} as const;

export type IconName = keyof typeof PATHS;

const SVG = "http://www.w3.org/2000/svg";

/** An inline SVG icon; decorative unless `label` is given. */
export function icon(name: IconName, size = 16, label?: string): SVGSVGElement {
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.75");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.classList.add("icon");
  if (label) {
    svg.setAttribute("role", "img");
    svg.setAttribute("aria-label", label);
  } else {
    svg.setAttribute("aria-hidden", "true");
  }
  const path = document.createElementNS(SVG, "path");
  path.setAttribute("d", PATHS[name]);
  svg.append(path);
  return svg;
}
