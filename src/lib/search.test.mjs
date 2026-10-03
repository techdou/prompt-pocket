import { it } from "node:test";
import assert from "node:assert/strict";
import { filterPrompts, searchExcerpt, highlightParts } from "./search.ts";

const prompt = (path, title, body = "", category = "写作") => ({ path, title, body, category, meta: {} });
it("finds words in the body and requires every query term", () => {
  const a = prompt("a", "周报", "项目复盘和行动计划");
  const b = prompt("b", "复盘", "其他内容");
  assert.deepEqual(filterPrompts([a, b], "复盘 计划"), [a]);
});
it("ranks exact titles above frequently used body matches", () => {
  const a = prompt("a", "其他", "复盘");
  const b = prompt("b", "复盘");
  assert.equal(filterPrompts([a, b], "复盘", { a: { useCount: 999999 } })[0].path, "b");
  assert.deepEqual(filterPrompts([a, b], ""), [a, b]);
});
it("returns a snippet around a late body hit and plain text highlight parts", () => {
  assert.match(searchExcerpt("前".repeat(200) + "目标内容" + "后".repeat(200), "目标"), /目标内容/);
  const parts = highlightParts('<script>alert("x")</script>', "script");
  assert.equal(parts.map((p) => p.text).join(""), '<script>alert("x")</script>');
  assert.ok(parts.some((p) => p.match && p.text === "script"));
});
