import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  normalizeCopyMode,
  normalizePrompt,
  normalizePromptContent,
} from "./api.ts";

describe("api normalization helpers", () => {
  it("keeps copy mode safe for Tauri command arguments", () => {
    assert.equal(normalizeCopyMode("plain"), "plain");
    assert.equal(normalizeCopyMode("markdown"), "markdown");
    assert.equal(normalizeCopyMode(undefined), "markdown");
    assert.equal(normalizeCopyMode("copyMode"), "markdown");
  });

  it("normalizes backend camelCase prompt fields for the Svelte app", () => {
    const prompt = normalizePrompt({
      id: "1",
      title: "Example",
      category: "Writing",
      path: "Writing/example.md",
      absPath: "C:\\Prompts\\Writing\\example.md",
      meta: {
        title: "Example",
        copyMode: "plain",
        created: "2026-07-05T00:00:00Z",
        updated: "2026-07-05T00:00:00Z",
      },
    });

    assert.equal(prompt.abs_path, "C:\\Prompts\\Writing\\example.md");
    assert.equal(prompt.meta.copy_mode, "plain");
  });

  it("defaults missing copy mode to markdown", () => {
    const content = normalizePromptContent({
      body: "body",
      meta: {
        title: "Untyped prompt",
        created: "2026-07-05T00:00:00Z",
        updated: "2026-07-05T00:00:00Z",
      },
    });

    assert.equal(content.meta.copy_mode, "markdown");
  });
});
