import { it } from "node:test";
import assert from "node:assert/strict";
import { readLibrary, writeLibrary, toggleFavorite, recordUse, moveLibraryEntry } from "./library.ts";
it("persists favorites and usage separately from prompt content", () => {
  let stored = null;
  const storage = { getItem: () => stored, setItem: (_, value) => { stored = value; } };
  let prefs = toggleFavorite({}, "写作/a.md");
  prefs = recordUse(prefs, "写作/a.md", 123);
  writeLibrary(storage, prefs);
  assert.deepEqual(readLibrary(storage), { "写作/a.md": { favorite: true, useCount: 1, lastUsed: 123 } });
  prefs = moveLibraryEntry(prefs, "写作/a.md", "编程/b.md");
  assert.equal(prefs["写作/a.md"], undefined);
  assert.equal(prefs["编程/b.md"].favorite, true);
});
it("tolerates corrupt or unavailable storage", () => {
  assert.deepEqual(readLibrary({ getItem: () => '{bad' }), {});
  assert.deepEqual(readLibrary({ getItem: () => '{"x":{"useCount":-1}}' }), {});
  assert.doesNotThrow(() => writeLibrary({ setItem: () => { throw new Error("blocked"); } }, {}));
});
