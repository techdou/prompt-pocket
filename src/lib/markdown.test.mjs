import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { renderMarkdown } from "./markdown.ts";

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

  it("正常 https 链接保留", () => {
    const html = renderMarkdown("[官网](https://example.com)");
    assert.ok(includes(html, 'href="https://example.com"'));
  });
});
