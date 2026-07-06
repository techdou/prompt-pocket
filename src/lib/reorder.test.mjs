import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  canReorderPromptList,
  getHorizontalCategoryDropTarget,
  getReorderDisabledReason,
  getReorderCategory,
  moveCategoryOrder,
  movePathOrder,
} from "./reorder.ts";

const prompt = (category, path) => ({ category, path });

describe("reorder helpers", () => {
  it("allows the default all view when it only contains one category", () => {
    const prompts = [
      prompt("写作", "写作/a.md"),
      prompt("写作", "写作/b.md"),
    ];

    assert.equal(getReorderCategory("__all__", prompts), "写作");
    assert.equal(canReorderPromptList("", "__all__", prompts), true);
  });

  it("disables reorder when fewer than two prompts are visible", () => {
    const prompts = [prompt("绘图", "绘图/image2清晰.md")];

    assert.equal(getReorderCategory("绘图", prompts), "绘图");
    assert.equal(canReorderPromptList("", "绘图", prompts), false);
    assert.equal(
      getReorderDisabledReason("", "绘图", prompts),
      "至少需要 2 条提示词才能排序",
    );
  });

  it("keeps the all view disabled when multiple categories are visible", () => {
    const prompts = [
      prompt("写作", "写作/a.md"),
      prompt("编程", "编程/b.md"),
    ];

    assert.equal(getReorderCategory("__all__", prompts), null);
    assert.equal(canReorderPromptList("", "__all__", prompts), false);
    assert.equal(
      getReorderDisabledReason("", "__all__", prompts),
      "切到单个分类后可拖拽排序",
    );
  });

  it("keeps search results disabled so hidden prompts do not lose their order", () => {
    const prompts = [
      prompt("写作", "写作/a.md"),
      prompt("写作", "写作/b.md"),
    ];

    assert.equal(canReorderPromptList("a", "写作", prompts), false);
    assert.equal(getReorderDisabledReason("a", "写作", prompts), "搜索结果不支持拖拽排序");
  });

  it("converts a drag source and insertion point into a path order", () => {
    const prompts = [
      prompt("写作", "写作/a.md"),
      prompt("写作", "写作/b.md"),
      prompt("写作", "写作/c.md"),
    ];

    assert.deepEqual(movePathOrder(prompts, 0, 3), [
      "写作/b.md",
      "写作/c.md",
      "写作/a.md",
    ]);
    assert.equal(movePathOrder(prompts, 1, 2), null);
  });

  it("moves a category by insertion point", () => {
    const categories = ["写作", "编程", "翻译"];

    assert.deepEqual(moveCategoryOrder(categories, 0, 3), ["编程", "翻译", "写作"]);
    assert.deepEqual(moveCategoryOrder(categories, 2, 0), ["翻译", "写作", "编程"]);
    assert.equal(moveCategoryOrder(categories, 1, 2), null);
  });

  it("keeps horizontal category drop targets valid in tab gaps and trailing space", () => {
    const tabs = [
      { tabIdx: -1, left: 0, right: 58 },
      { tabIdx: 0, left: 64, right: 124 },
      { tabIdx: 1, left: 130, right: 190 },
      { tabIdx: 2, left: 196, right: 256 },
    ];

    assert.deepEqual(getHorizontalCategoryDropTarget(tabs, 61), {
      lineIndex: -1,
      lineBefore: false,
      toIndex: 0,
    });
    assert.deepEqual(getHorizontalCategoryDropTarget(tabs, 127), {
      lineIndex: 0,
      lineBefore: false,
      toIndex: 1,
    });
    assert.deepEqual(getHorizontalCategoryDropTarget(tabs, 280), {
      lineIndex: 2,
      lineBefore: false,
      toIndex: 3,
    });
    assert.equal(getHorizontalCategoryDropTarget(tabs, 10), null);
  });
});
