import { beforeEach, describe, expect, it, vi } from "vitest";

// What Rust was asked to keep. Remembering is on unless a test turns it off.
const recorded: { command: string; host: string | null }[] = [];
let recording = true;
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: { command: string; host: string | null }) => {
    if (cmd === "memory_view") return { settings: { recording }, snippets: [] };
    if (cmd === "memory_record" && args) recorded.push({ command: args.command, host: args.host });
    return null;
  }),
}));

const { CommandCapture, isSecretPrompt, refreshMemoryState } = await import("../src/work-memory");
type Terminal = import("@xterm/xterm").Terminal;

/** Just enough of xterm's buffer: rows of text and a cursor. */
class FakeTerm {
  rows: string[] = [""];
  wrapped = new Set<number>();
  y = 0;
  type: "normal" | "alternate" = "normal";
  /** Prints like a shell would: moves the cursor to the end of the row. */
  print(text: string) {
    this.rows[this.y] += text;
  }
  newLine(prompt = "") {
    this.rows.push(prompt);
    this.y = this.rows.length - 1;
  }
  get buffer() {
    const t = this;
    return {
      active: {
        get type() {
          return t.type;
        },
        baseY: 0,
        get cursorY() {
          return t.y;
        },
        get cursorX() {
          return t.rows[t.y].length;
        },
        getLine(y: number) {
          const row = t.rows[y];
          if (row === undefined) return undefined;
          return {
            isWrapped: t.wrapped.has(y),
            translateToString: (trim: boolean, start = 0, end?: number) => {
              const s = row.slice(start, end);
              return trim ? s.trimEnd() : s;
            },
          };
        },
      },
    };
  }
}

function setup(prompt = "$ ", host: string | null = null) {
  const term = new FakeTerm();
  term.print(prompt);
  const capture = new CommandCapture(term as unknown as Terminal, () => host);
  /** Types `text`; the shell echoes it unless `echo` is false. */
  const type = (text: string, echo = true) => {
    capture.input(text);
    if (echo) term.print(text);
  };
  const enter = () => {
    capture.input("\r");
    term.newLine("$ ");
  };
  return { term, capture, type, enter };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(async () => {
  recorded.length = 0;
  recording = true;
  await refreshMemoryState();
});

describe("CommandCapture", () => {
  it("keeps a typed command with its host, without the prompt", async () => {
    const { type, enter } = setup("deploy@web-1:~$ ", "web-1");
    type("systemctl status nginx");
    enter();
    await flush();
    expect(recorded).toEqual([{ command: "systemctl status nginx", host: "web-1" }]);
  });

  it("reads the line from the screen after tab completion or history", async () => {
    const { term, capture, enter } = setup();
    capture.input("git sta");
    term.print("git sta");
    capture.input("\t");
    term.print("tus"); // the shell completed it
    enter();
    capture.input("\x1b[A"); // up arrow: history recall
    term.print("git status");
    enter();
    await flush();
    expect(recorded.map((r) => r.command)).toEqual(["git status", "git status"]);
  });

  it("applies backspace and forgets a line cancelled with Ctrl+C", async () => {
    const { capture, type, enter } = setup();
    type("lss");
    capture.input("\x7f");
    type(" -la");
    enter();
    type("rm -rf build");
    capture.input("\x03");
    enter();
    await flush();
    expect(recorded.map((r) => r.command)).toEqual(["ls -la"]);
  });

  it("never keeps what is typed at a password prompt or not echoed", async () => {
    const { term, type, enter } = setup("[sudo] password for deploy: ");
    type("hunter2", false);
    enter();
    term.print(""); // a prompt we do not recognise, but the shell echoes nothing
    term.rows[term.y] = "Enter it: ";
    type("s3cret", false);
    enter();
    await flush();
    expect(recorded).toEqual([]);
  });

  it("skips full-screen programs and does nothing while remembering is off", async () => {
    const { term, type, enter } = setup();
    term.type = "alternate";
    type(":wq");
    enter();
    term.type = "normal";
    recording = false;
    await refreshMemoryState();
    type("uptime");
    enter();
    await flush();
    expect(recorded).toEqual([]);
  });

  it("joins wrapped rows of a long command", async () => {
    const { term, capture, enter } = setup();
    capture.input("\x1b[A");
    term.print("rsync -av ./dist/ ");
    term.newLine("deploy@web-1:/srv/app/");
    term.wrapped.add(term.y);
    term.y = 0; // the prompt row; Enter is read from there
    enter();
    await flush();
    expect(recorded.map((r) => r.command)).toEqual(["rsync -av ./dist/ deploy@web-1:/srv/app/"]);
  });
});

describe("isSecretPrompt", () => {
  it("knows password, passphrase, PIN, token and code prompts", () => {
    for (const p of [
      "Password: ",
      "[sudo] password for me: ",
      "Enter passphrase for key '/home/me/.ssh/id_ed25519': ",
      "Enter PIN for authenticator: ",
      "GitHub token? ",
      "Verification code: ",
    ]) {
      expect(isSecretPrompt(p), p).toBe(true);
    }
  });

  it("leaves shell prompts alone", () => {
    for (const p of ["$ ", "deploy@web-1:~/passwords$ x", "PS C:\\> ", "~/src/setu main ❯ "]) {
      expect(isSecretPrompt(p), p).toBe(false);
    }
  });
});
