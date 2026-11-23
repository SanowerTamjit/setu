// The two window layouts. Classic is the rail and tab bar Setu has had since
// the redesign; Studio puts open sessions in a navigator on the left, a search
// box for everything in a title bar, and the session's actions above the
// terminal. Both show the same tabs, hosts and dialogs: switching keeps every
// session running.

import type { LayoutName } from "./settings";

export interface LayoutInfo {
  id: LayoutName;
  name: string;
  description: string;
}

export const LAYOUTS: readonly LayoutInfo[] = [
  {
    id: "classic",
    name: "Classic",
    description: "Icon rail, tabs across the top, status bar at the bottom.",
  },
  {
    id: "studio",
    name: "Studio",
    description: "Futuristic: glass panels and neon edges, sessions on the left, search for everything at the top.",
  },
];
