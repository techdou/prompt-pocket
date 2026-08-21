import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { filterPrompts, bodyMatchSnippet } from "./search.ts";

// 最小 Prompt  stub：search.ts 只读 title / category / meta.tags / body
const mk = (title, body, { category = "默认", path } = {}) => ({
  title,
  body,
  category,
  path: path ?? `${category}/${title}.md`,
  meta: {},
});

describe("filterPrompts 内容搜索", () => {
  it("关键词只出现在正文里也能搜到（标题遗忘场景）", () => {
    const prompts = [
      mk("周报模板", "请把以下内容整理成结构化周报"),
      mk("翻译助手", "将文本翻译为英文"),
    ];
    const hits = filterPrompts(prompts, "结构化");
    assert.deepEqual(hits.map((p) => p.title), ["周报模板"]);
  });

  it("标题命中排在正文命中前面", () => {
    const prompts = [
      mk("无关标题A", "正文里包含角色扮演的要求"),
      mk("角色扮演生成器", "完全不同的正文"),
    ];
    const hits = filterPrompts(prompts, "角色扮演");
    assert.equal(hits.length, 2);
    assert.equal(hits[0].title, "角色扮演生成器", "标题命中应排第一");
  });

  it("多关键词 AND：可以一个中标题、一个中正文", () => {
    const prompts = [
      mk("代码审查", "关注并发安全和错误处理"),
      mk("代码生成", "根据需求输出可运行代码"),
    ];
    assert.deepEqual(
      filterPrompts(prompts, "代码 并发").map((p) => p.title),
      ["代码审查"],
    );
    // 任一关键词不命中则整条排除
    assert.equal(filterPrompts(prompts, "代码 不存在词").length, 0);
  });

  it("正文不做字符级模糊：跳跃凑出的字符序列不算命中", () => {
    // 正文按顺序含 a...b...c，但无连续子串 "abc"
    const prompts = [mk("标题", "a 很长很长的间隔 b 再隔很远 c")];
    assert.equal(filterPrompts(prompts, "abc").length, 0);
  });

  it("跨行的连续词语折叠空白后仍能命中", () => {
    const prompts = [mk("标题", "第一行有个词\n第二行紧接着接上")];
    assert.equal(filterPrompts(prompts, "个词 第二行").length, 1);
  });

  it("大小写不敏感", () => {
    const prompts = [mk("Title", "Use Chain-of-Thought here")];
    assert.equal(filterPrompts(prompts, "chain-of-thought").length, 1);
  });

  it("空查询返回全部且保持原顺序", () => {
    const prompts = [mk("B", "x"), mk("A", "y")];
    assert.deepEqual(
      filterPrompts(prompts, "  ").map((p) => p.title),
      ["B", "A"],
    );
  });
});

describe("bodyMatchSnippet 正文命中摘录", () => {
  it("只命中正文时返回命中词上下文", () => {
    const p = mk("周报", "前半段铺垫文字，结构化输出是核心要求，后半段其他内容");
    const s = bodyMatchSnippet(p, "结构化");
    assert.ok(s.includes("结构化"), "摘录应含命中词");
    assert.ok(s.includes("铺垫"), "摘录应带前文上下文");
  });

  it("标题能解释命中时不需要摘录", () => {
    const p = mk("结构化周报", "正文里也提到结构化");
    assert.equal(bodyMatchSnippet(p, "结构化"), null);
  });

  it("正文没命中时返回 null", () => {
    const p = mk("标题", "正文");
    assert.equal(bodyMatchSnippet(p, "不存在"), null);
    assert.equal(bodyMatchSnippet(p, ""), null);
  });

  it("命中位置靠后时前文截断补省略号", () => {
    const p = mk("标题", "很".repeat(100) + "关键词" + "后".repeat(100));
    const s = bodyMatchSnippet(p, "关键词");
    assert.ok(s.startsWith("…"), "前文截断应以省略号开头");
    assert.ok(s.endsWith("…"), "后文截断应以省略号结尾");
    assert.ok(s.includes("关键词"));
  });

  it("摘录把换行折叠成空格（一行展示）", () => {
    const p = mk("标题", "上文\n\n关键词\n下文");
    const s = bodyMatchSnippet(p, "关键词");
    assert.ok(!s.includes("\n"), "摘录不应含换行");
    assert.equal(s, "上文 关键词 下文");
  });
});
