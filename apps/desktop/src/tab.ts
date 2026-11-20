// What the tab bar needs from any tab: a terminal or a Files view.

export interface Tab {
  readonly element: HTMLElement;
  title: string;
  /** Shown on the left of the status bar. */
  detail: string;
  readonly tooltip: string;
  /** Shown in the status bar next to the detail (terminal size, item count). */
  readonly sizeLabel: string;
  readonly hasExited: boolean;
  show(): void;
  hide(): void;
  refit(): void;
  focus(): void;
  dispose(): Promise<void>;
}
