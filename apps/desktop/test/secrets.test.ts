import { describe, expect, it } from "vitest";
import { findSecrets, findSecretsInLines, keyBlockOpen } from "../src/secrets";

// Built at runtime so this file holds no token-shaped strings (the
// pre-commit secret check would rightly refuse them).
const AWS_ID = ["AKIA", "IOSFODNN7EXAMPLE"].join("");
const KEY_LABEL = ["OPENSSH", "PRIVATE", "KEY"].join(" ");

const covered = (line: string) => findSecrets(line).map((m) => line.slice(m.start, m.end));

describe("findSecrets", () => {
  it("covers well-known token formats", () => {
    expect(covered(`key ${AWS_ID} here`)).toEqual([AWS_ID]);
    expect(covered(`token ${"ghp"}_${"a".repeat(36)}`)).toEqual([`${"ghp"}_${"a".repeat(36)}`]);
    expect(covered(`${"glpat"}-${"x".repeat(20)}`)).toHaveLength(1);
    expect(covered("Authorization: Bearer abcdefghijklmnopqrstuvwxyz012345")).toEqual(["abcdefghijklmnopqrstuvwxyz012345"]);
  });

  it("covers only the secret part of assignments and URLs", () => {
    expect(covered("DB_PASSWORD=hunter22secret")).toEqual(["hunter22secret"]);
    expect(covered("export api_key: 'abcdef123456'")).toEqual(["abcdef123456"]);
    expect(covered("git clone https://me:s3cretpw@git.example.com/x")).toEqual(["s3cretpw"]);
    expect(covered("aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY")).toEqual([
      "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    ]);
  });

  it("leaves ordinary output alone", () => {
    for (const line of [
      "total 48",
      "drwxr-xr-x  2 root root 4096 Nov 22 18:00 passwords",
      "Password: ",
      "token=abc", // too short to be a real secret
      "https://example.com/path?x=1",
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl user@host",
    ]) {
      expect(covered(line), line).toEqual([]);
    }
  });

  it("returns sorted matches without overlaps", () => {
    const m = findSecrets(`password=${"ghp"}_${"b".repeat(36)} ${AWS_ID}`);
    for (let i = 1; i < m.length; i++) expect(m[i].start).toBeGreaterThanOrEqual(m[i - 1].end);
  });
});

describe("private key blocks", () => {
  const block = [`-----BEGIN ${KEY_LABEL}-----`, "b3BlbnNzaC1rZXktdjEAAAAABG5vbmU=", "  AAAAAAAAAAAAAAA  ", `-----END ${KEY_LABEL}-----`, "after"];

  it("covers every body line, and only the body", () => {
    const { matches, inKey } = findSecretsInLines(block);
    expect(matches[0]).toEqual([]);
    expect(matches[1]).toEqual([{ start: 0, end: block[1].length, kind: "Private key" }]);
    expect(matches[2]).toEqual([{ start: 2, end: 17, kind: "Private key" }]);
    expect(matches[3]).toEqual([]);
    expect(matches[4]).toEqual([]);
    expect(inKey).toBe(false);
  });

  it("carries an open block across calls", () => {
    const first = findSecretsInLines(block.slice(0, 2));
    expect(first.inKey).toBe(true);
    const rest = findSecretsInLines(block.slice(2), first.inKey);
    expect(rest.matches[0]).toHaveLength(1);
    expect(rest.inKey).toBe(false);
    expect(keyBlockOpen(block.slice(0, 3))).toBe(true);
    expect(keyBlockOpen(block)).toBe(false);
  });
});
