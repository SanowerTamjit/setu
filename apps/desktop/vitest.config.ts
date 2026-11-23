import { defineConfig } from "vitest/config";

// Unit tests for the UI's logic. They run in Node without a webview: code
// that calls Rust through `invoke` is tested with it mocked.
export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"],
    environment: "node",
  },
});
