import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { renderMarkdown, markdownToPlain } from "./markdown.ts";

/** 提取 marked 输出里是否有指定标签/属性，简化断言 */
const includes = (html, needle) => html.includes(needle);

describe("renderMarkdown - GFM 基础语法", () => {
  it("空内容返回占位", () => {
    assert.ok(includes(renderMarkdown(""), "empty-body"));
    assert.ok(includes(renderMarkdown("   "), "empty-body"));
  });

  it("标题渲染为 h2/h3", () => {
    const html = renderMarkdown("## 二级\n### 三级");
    assert.ok(includes(html, "<h2>二级</h2>"));
    assert.ok(includes(html, "<h3>三级</h3>"));
  });

  it("GFM 表格", () => {
    const html = renderMarkdown("| a | b |\n|---|---|\n| 1 | 2 |");
    assert.ok(includes(html, "<table>"));
    assert.ok(includes(html, "<th>a</th>"));
    assert.ok(includes(html, "<td>1</td>"));
  });

  it("引用块", () => {
    const html = renderMarkdown("> 引用文本");
    assert.ok(includes(html, "<blockquote>"));
  });

  it("分割线", () => {
    const html = renderMarkdown("上\n\n---\n\n下");
    assert.ok(includes(html, "<hr>"));
  });

  it("删除线", () => {
    const html = renderMarkdown("~~删除~~");
    assert.ok(includes(html, "<del>删除</del>"));
  });

  it("任务列表复选框", () => {
    const html = renderMarkdown("- [x] 完成\n- [ ] 未完成");
    assert.ok(includes(html, 'type="checkbox"'));
    assert.ok(includes(html, "checked"));
  });
});

describe("renderMarkdown - 扩展语法占位", () => {
  it("mermaid 代码块输出占位 div", () => {
    const html = renderMarkdown("```mermaid\ngraph TD; A-->B\n```");
    assert.ok(includes(html, 'class="mermaid"'));
    assert.ok(includes(html, "data-source="));
  });

  it("普通代码块保留 language-xxx class", () => {
    const html = renderMarkdown("```js\nvar x=1;\n```");
    assert.ok(includes(html, 'class="hljs language-js"'));
  });

  it("行内公式输出 katex 占位", () => {
    const html = renderMarkdown("能量 $E=mc^2$ 公式");
    assert.ok(includes(html, "katex-placeholder"));
    assert.ok(includes(html, 'data-tex="E=mc^2"'));
    assert.ok(includes(html, 'data-display="false"'));
  });

  it("块级公式输出 katex 占位（display=true）", () => {
    const html = renderMarkdown("$$\nE=mc^2\n$$");
    assert.ok(includes(html, "katex-block"));
    assert.ok(includes(html, 'data-display="true"'));
  });
});

describe("renderMarkdown - XSS 防护", () => {
  it("raw <script> 标签被转义，不执行", () => {
    const html = renderMarkdown("<script>alert(1)</script>");
    assert.ok(!includes(html, "<script>"));
    assert.ok(includes(html, "&lt;script&gt;"));
  });

  it("raw <img onerror> 被转义", () => {
    const html = renderMarkdown("<img src=x onerror=alert(1)>");
    assert.ok(!includes(html, "<img"));
    assert.ok(includes(html, "&lt;img"));
  });

  it("mermaid 占位属性里的双引号被转义", () => {
    const html = renderMarkdown('```mermaid\n" onclick="alert(1)\n```');
    assert.ok(includes(html, '&quot; onclick=&quot;alert(1)'));
    assert.ok(!includes(html, 'onclick="alert(1)"'));
  });

  it("KaTeX 占位属性里的双引号被转义", () => {
    const html = renderMarkdown('$" autofocus="autofocus$');
    assert.ok(includes(html, '&quot; autofocus=&quot;autofocus'));
    assert.ok(!includes(html, 'autofocus="autofocus"'));
  });

  it("javascript: 协议链接 href 被替换为 #", () => {
    const html = renderMarkdown("[点我](javascript:alert(1))");
    assert.ok(includes(html, 'href="#"'));
    assert.ok(!includes(html, "javascript:"));
  });

  it("javascript: 协议图片 src 被清空", () => {
    const html = renderMarkdown("![x](javascript:alert(1))");
    assert.ok(!includes(html, "javascript:"));
  });

  it("data:text/html 协议图片 src 被清空", () => {
    const html = renderMarkdown("![x](data:text/html;base64,PHNjcmlwdD4=)");
    assert.ok(!includes(html, "data:text/html"));
  });

  it("data:image/* 内嵌图片保留", () => {
    const html = renderMarkdown("![x](data:image/png;base64,iVBORw0KGgo=)");
    assert.ok(includes(html, "data:image/png;base64,"));
  });

  it("正常 https 链接保留", () => {
    const html = renderMarkdown("[官网](https://example.com)");
    assert.ok(includes(html, 'href="https://example.com"'));
  });
});

describe("markdownToPlain - plain 复制模式", () => {
  it("去掉常见行内标记", () => {
    const plain = markdownToPlain("**加粗** *斜体* ~~删除~~ `代码` __粗__");
    assert.equal(plain, "加粗 斜体 删除 代码 粗");
  });

  it("链接保留文字去掉 URL，图片保留 alt", () => {
    const plain = markdownToPlain("见 [文档](https://a.b) 和 ![图](https://c.d/x.png)");
    assert.equal(plain, "见 文档 和 图");
  });

  it("去掉行首标记（标题/引用/任务列表/有序无序）", () => {
    const src = [
      "## 标题",
      "> 引用",
      "- [x] 完成",
      "- 未完成",
      "1. 第一",
    ].join("\n");
    const plain = markdownToPlain(src);
    assert.ok(!plain.includes("##"), `标题标记应去除: ${plain}`);
    assert.ok(!plain.includes("> 引用"), `引用标记应去除: ${plain}`);
    assert.equal(plain, "标题\n引用\n完成\n未完成\n第一");
  });

  it("围栏代码块保留内容", () => {
    const plain = markdownToPlain("说明\n```js\nvar x = 1;\n```");
    assert.ok(plain.includes("var x = 1;"));
    assert.ok(!plain.includes("```"));
  });

  it("水平线被移除", () => {
    const plain = markdownToPlain("上\n\n---\n\n下");
    assert.equal(plain, "上\n\n下");
  });

  it("无标记文本原样返回", () => {
    assert.equal(markdownToPlain("普通文本 123"), "普通文本 123");
    assert.equal(markdownToPlain(""), "");
  });

  it("行内代码与围栏块内的 dunder 标识符原样保留", () => {
    // 编程提示词高频场景：`__init__` 是字面标识符，不是粗体语法
    const inline = markdownToPlain("入口是 `python -m __main__`，构造器 __init__ 之外");
    assert.ok(inline.includes("__main__"), `行内代码应保留: ${inline}`);
    const fenced = markdownToPlain("```python\nif __name__ == \"__main__\":\n    pass\n```");
    assert.ok(fenced.includes("__main__"), `围栏块应保留: ${fenced}`);
    assert.ok(fenced.includes("__name__"), `围栏块应保留: ${fenced}`);
  });

  it("词中双下划线不剥（CommonMark 词中强调不生效）", () => {
    const plain = markdownToPlain("调用 foo__bar__baz 入口");
    assert.ok(plain.includes("foo__bar__baz"), `词中标识符不应被剥: ${plain}`);
  });

  it("空格包围的 __粗体__ 正常剥（合法 Markdown 语法）", () => {
    assert.equal(markdownToPlain("__重点__内容前"), "重点内容前");
    assert.equal(markdownToPlain("中文 __粗体__ 结尾"), "中文 粗体 结尾");
  });

  it("行内代码内的标记语法原样保留", () => {
    const plain = markdownToPlain("写 `**不是粗体**` 和 `[不是链接](x)`");
    assert.ok(plain.includes("**不是粗体**"), `行内代码内标记应保留: ${plain}`);
    assert.ok(plain.includes("[不是链接](x)"), `行内代码内标记应保留: ${plain}`);
  });

  it("链接 url 含一层嵌套括号时完整去除", () => {
    const plain = markdownToPlain("见 [wiki](https://en.wikipedia.org/wiki/Foo_(bar))");
    assert.equal(plain, "见 wiki");
  });
});
