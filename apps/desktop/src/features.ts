// Feature flags: switches for features that are new, not tested on every
// system yet, or that change how Setu behaves, so they can ship turned off
// (or be turned off by anyone who hits a problem) without a new release.
//
// Every flag has a stage:
// - experimental: off by default; may change or go away.
// - beta: on by default; still collecting feedback (tested less widely).
// - stable: kept only for one release so people can still turn it off,
//   then the flag is removed and the feature is always on.
//
// Choices are saved in settings.json under "features". See docs/developer/feature-flags.md.

import { settings, updateSettings } from "./settings";

export type FlagStage = "experimental" | "beta" | "stable";

export interface FeatureFlag {
  id: string;
  title: string;
  description: string;
  stage: FlagStage;
  /** Whether it is on for people who never changed it. */
  default: boolean;
  /** The release that added the flag, to know when to retire it. */
  since: string;
}

export const FLAGS = [
  {
    id: "securityKeys",
    title: "YubiKey and FIDO2 security keys",
    description:
      "Make SSH keys on a security key (ed25519-sk, ecdsa-sk) and load keys stored on one, in the Keys view. Needs OpenSSH 8.2 or newer.",
    stage: "beta",
    default: true,
    since: "0.1.0",
  },
  {
    id: "remoteEdit",
    title: "Edit remote files in a local editor",
    description: "The Edit button (F4) in Files: copy a file to a private folder, open it in your editor, and write each save back to the server.",
    stage: "beta",
    default: true,
    since: "0.1.0",
  },
  {
    id: "pasteGuard",
    title: "Ask before pasting several lines",
    description:
      "When the shell does not support bracketed paste, each line break in pasted text runs a command at once. Show the text and ask first.",
    stage: "beta",
    default: true,
    since: "0.1.0",
  },
  {
    id: "workflows",
    title: "Workflows",
    description:
      "The Workflows view: draw steps that run commands on your saved hosts in order or in parallel, with approvals, failure branches and run history.",
    stage: "beta",
    default: true,
    since: "0.1.0",
  },
  {
    id: "workMemory",
    title: "Work memory and automation suggestions",
    description:
      "The Memory tab in Workflows: once you turn remembering on, Setu keeps the commands you run (on this device only, secrets masked) and suggests snippets and workflows for work you repeat. Off here hides it and stops all remembering.",
    stage: "beta",
    default: true,
    since: "0.1.0",
  },
  {
    id: "promptThemes",
    title: "Prompt themes (Powerlevel10k, Starship, Oh My Posh)",
    description:
      "Built-in Nerd Font symbols so prompt icons and Powerline arrows show with any font, Unicode 11 character widths so right-hand prompts line up, and colour schemes imported from other terminals.",
    stage: "beta",
    default: true,
    since: "0.1.0",
  },
  {
    id: "tunnels",
    title: "Tunnels",
    description:
      "The Tunnels view: saved port forwards (ssh -L, -R and a SOCKS proxy like -D) over a saved host's login, with live status and reconnect after a drop. Off here hides the view; tunnels already running keep going until you quit.",
    stage: "beta",
    default: true,
    since: "0.2.0",
  },
  {
    id: "networkTools",
    title: "Network tools",
    description:
      "The Network view: port check (a TCP ping), DNS lookup, ping and Wake-on-LAN, from this computer or through a saved host. Off here hides the view.",
    stage: "beta",
    default: true,
    since: "0.2.0",
  },
  {
    id: "monitoring",
    title: "Monitoring",
    description:
      "The Monitoring view: live CPU, memory, network and disk use for Linux hosts, read over SSH with nothing installed, with alerts above the limits you set. Off here hides the view.",
    stage: "beta",
    default: true,
    since: "0.2.0",
  },
  {
    id: "serverManagement",
    title: "Server management",
    description:
      "The Server view: a saved Linux host's systemd services with their status and logs (start, stop, restart, reload, enable, disable), its processes (stop one) and what fills its disks. Every change shows the exact command and asks first. Off here hides the view.",
    stage: "beta",
    default: true,
    since: "0.2.0",
  },
] as const satisfies readonly FeatureFlag[];

export type FlagId = (typeof FLAGS)[number]["id"];

const byId = new Map<string, FeatureFlag>(FLAGS.map((f) => [f.id, f]));

/** Whether a feature is on: the person's choice, else the flag's default. */
export function enabled(id: FlagId): boolean {
  const chosen = settings().features[id];
  if (typeof chosen === "boolean") return chosen;
  return byId.get(id)?.default ?? false;
}

/** Turns a feature on or off; `null` goes back to the default. */
export function setEnabled(id: FlagId, on: boolean | null) {
  const features = { ...settings().features };
  if (on === null || on === byId.get(id)?.default) delete features[id];
  else features[id] = on;
  updateSettings({ features });
}

/** Whether the person changed this flag from its default. */
export function isOverridden(id: FlagId): boolean {
  return typeof settings().features[id] === "boolean";
}
