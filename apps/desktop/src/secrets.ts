// Finding things that look like secrets in a line of terminal output.
// Kept free of xterm so it can be checked on its own.

export interface SecretMatch {
  /** Offset in the line, in characters. */
  start: number;
  end: number;
  /** What kind of secret, shown when you hover the bar. */
  kind: string;
}

interface Rule {
  kind: string;
  re: RegExp;
  /** The capture group holding the secret itself; 0 is the whole match. */
  group?: number;
}

// Every pattern is global so all matches in a line are found. Keep them
// specific: a bar over ordinary output is worse than a missed secret only
// for a moment, but it makes people turn the feature off.
const RULES: Rule[] = [
  { kind: "AWS access key", re: /\b(?:AKIA|ASIA)[0-9A-Z]{16}\b/g },
  {
    kind: "AWS secret key",
    re: /\baws_secret_access_key\b["']?\s*[=:]\s*["']?([A-Za-z0-9/+]{40})(?![A-Za-z0-9/+])/gi,
    group: 1,
  },
  { kind: "GitHub token", re: /\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}\b/g },
  { kind: "GitHub token", re: /\bgithub_pat_[A-Za-z0-9_]{60,}\b/g },
  { kind: "GitLab token", re: /\bglpat-[A-Za-z0-9_-]{20,}\b/g },
  { kind: "Slack token", re: /\bxox[abprs]-[A-Za-z0-9-]{10,}\b/g },
  { kind: "Stripe key", re: /\b[sr]k_live_[A-Za-z0-9]{20,}\b/g },
  { kind: "Google API key", re: /\bAIza[0-9A-Za-z_-]{35}(?![0-9A-Za-z_-])/g },
  { kind: "JSON web token", re: /\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}/g },
  { kind: "Bearer token", re: /\bBearer\s+([A-Za-z0-9._~+/-]{20,}=*)/g, group: 1 },
  { kind: "Password in URL", re: /\b[a-z][a-z0-9+.-]*:\/\/[^\s:@/]+:([^\s@/]+)@/gi, group: 1 },
  {
    kind: "Password or key",
    re: /(?<![A-Za-z0-9])(?:[A-Za-z0-9]+[_-])*(?:password|passwd|pass|pwd|secret|client_secret|api[_-]?key|access[_-]?token|auth[_-]?token|token)\b["']?\s*[=:]\s*["']?([^\s"',;]{6,})/gi,
    group: 1,
  },
];

const KEY_BEGIN = /-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----/;
const KEY_END = /-----END [A-Z0-9 ]*PRIVATE KEY-----/;

/** Secrets in one line, sorted and without overlaps. */
export function findSecrets(line: string): SecretMatch[] {
  const found: SecretMatch[] = [];
  for (const rule of RULES) {
    rule.re.lastIndex = 0;
    for (const m of line.matchAll(rule.re)) {
      const g = rule.group ?? 0;
      const text = m[g];
      if (!text) continue;
      // A captured secret is always at the end of its match.
      const at = (m.index ?? 0) + (g ? m[0].lastIndexOf(text) : 0);
      found.push({ start: at, end: at + text.length, kind: rule.kind });
    }
  }
  found.sort((a, b) => a.start - b.start || b.end - a.end);
  const out: SecretMatch[] = [];
  for (const f of found) {
    const last = out[out.length - 1];
    if (last && f.start < last.end) {
      last.end = Math.max(last.end, f.end);
      continue;
    }
    out.push({ ...f });
  }
  return out;
}

/**
 * Secrets across consecutive lines, which also catches the body of a
 * private key block. `inKey` says whether the first line is already inside
 * one; the result says whether the last line leaves it open.
 */
export function findSecretsInLines(lines: string[], inKey = false): { matches: SecretMatch[][]; inKey: boolean } {
  const matches = lines.map((line) => {
    if (inKey) {
      if (KEY_END.test(line)) {
        inKey = false;
        return [];
      }
      const text = line.trimEnd();
      const start = text.length - text.trimStart().length;
      return text.length > start ? [{ start, end: text.length, kind: "Private key" }] : [];
    }
    if (KEY_BEGIN.test(line) && !KEY_END.test(line)) {
      inKey = true;
      return [];
    }
    return findSecrets(line);
  });
  return { matches, inKey };
}

/** Whether the lines before a point leave a private key block open. */
export function keyBlockOpen(lines: string[]): boolean {
  let open = false;
  for (const line of lines) {
    if (KEY_BEGIN.test(line) && !KEY_END.test(line)) open = true;
    else if (KEY_END.test(line)) open = false;
  }
  return open;
}
