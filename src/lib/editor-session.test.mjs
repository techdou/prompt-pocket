import { it } from "node:test";
import assert from "node:assert/strict";
import { createRequestGate, draftChanged } from "./editor-session.ts";
it("ignores an older result after selecting another prompt", async () => {
  const gate = createRequestGate();
  let finish;
  let text = "";
  const first = gate.begin("a");
  const pending = new Promise((resolve) => { finish = resolve; }).then(() => {
    if (gate.accepts(first, "a")) text = "old";
  });
  const second = gate.begin("b");
  if (gate.accepts(second, "b")) text = "new";
  finish();
  await pending;
  assert.equal(text, "new");
  gate.invalidate();
  assert.equal(gate.accepts(second, "b"), false);
});
it("detects category-only changes as unsaved changes", () => {
  const a = { title: "title", body: "body", category: "A", copy_mode: "markdown" };
  assert.equal(draftChanged(a, { ...a }), false);
  assert.equal(draftChanged(a, { ...a, category: "B" }), true);
});
