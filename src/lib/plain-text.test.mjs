import { it } from "node:test";
import assert from "node:assert/strict";
import { markdownToPlain } from "./plain-text.ts";
it("copies readable text while preserving code and list structure", () => {
  assert.equal(markdownToPlain("# Title\n\n**Bold** [link](https://example.com)"), "Title\n\nBold link");
  assert.equal(markdownToPlain("```js\na * b\n```"), "a * b");
  assert.match(markdownToPlain("- first\n- second"), /• first\n• second/);
});
