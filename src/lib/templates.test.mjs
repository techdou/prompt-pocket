import { it } from "node:test";
import assert from "node:assert/strict";
import { templateFields, renderTemplate, missingTemplateFields } from "./templates.ts";
it("collects unique Chinese variables and defaults", () => {
  assert.deepEqual(templateFields("{{读者}} {{语气|自然}} {{读者}}"), [
    { name: "读者", defaultValue: "" }, { name: "语气", defaultValue: "自然" },
  ]);
});
it("renders multiline values literally without recursively replacing them", () => {
  assert.equal(renderTemplate("{{原文}} / {{语气|自然}}", { 原文: "第一行\n{{语气}} $&" }), "第一行\n{{语气}} $& / 自然");
  assert.deepEqual(missingTemplateFields("{{原文}} {{语气|自然}}", { 原文: "  " }), ["原文"]);
});
it("keeps ordinary prompts and escaped braces usable", () => {
  assert.equal(renderTemplate("普通内容", {}), "普通内容");
  assert.deepEqual(templateFields("\\{{示例}}"), []);
  assert.equal(renderTemplate("\\{{示例}}", {}), "{{示例}}");
});
