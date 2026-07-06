// 富 Markdown 渲染核心：marked 打包，离线可用 GFM 全语法。
// XSS 防护不依赖 DOMPurify——raw HTML 块一律转义显示，危险协议链接替换为 #。
// mermaid / 代码高亮走 DOM 阶段 CDN 加载（见 ./renderers）。

import { Marked } from "marked";
import { markedHighlight } from "marked-highlight";

const esc = (s: string): string =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

// 链接协议白名单：仅放行 http(s)/mailto/ftp/相对路径/锚点
const SAFE_HREF = /^(https?:|mailto:|ftp:|\/|#|\.\/|\.\.\/)/i;

const marked = new Marked();

// ── KaTeX tokenizer（手写，不依赖 katex 包本体）──
// 规则取自 marked-katex-extension：$...$ 行内、$$...$$/$...$ 块级。
// renderer 只产出占位标记，真正的渲染交给 DOM 阶段 CDN 加载的 katex（见 renderers/katex.ts）。
const inlineKatexRule =
  /^(\${1,2})(?!\$)((?:\\.|[^\\\n])*?(?:\\.|[^\\\n$]))\1(?=[\s?!.,:？！。，：]|$)/;
const blockKatexRule = /^(\${1,2})\n((?:\\[^]|[^\\])+?)\n\1(?:\n|$)/;

/** 把 tex 源码包成占位元素，data-* 供 DOM 阶段读取 */
function katexPlaceholder(tex: string, displayMode: boolean): string {
  // 用 <code> 兜底：katex 未加载时至少显示等宽原文，不裸露 $ 符号干扰阅读
  const mode = displayMode ? "block" : "inline";
  return `<code class="katex-placeholder katex-${mode}" data-tex="${esc(tex)}" data-display="${displayMode}">${esc(tex)}</code>`;
}

// marked-highlight：代码高亮 hook。mermaid 语言原样返回，留给 DOM 阶段处理。
// 其余语言目前不在解析阶段着色（highlight.js 走 CDN，DOM 阶段统一处理），
// 这里只负责给 code 元素打上 language-xxx class，方便后续 highlightElement() 定位。
marked.use(
  markedHighlight({
    langPrefix: "hljs language-",
    highlight(code, lang) {
      if ((lang || "").trim() === "mermaid") return code;
      return false; // 不在解析阶段着色
    },
  }),
);

marked.use({
  // raw HTML 块/行内 HTML 一律转义，杜绝 <script>/<iframe>/<img onerror> 注入
  renderer: {
    html({ text }: { text: string }): string {
      return esc(text);
    },
    // mermaid 代码块输出占位 div，交给 DOM 阶段 renderMermaid() 渲染
    code({ text, lang }: { text: string; lang?: string }): string | false {
      if ((lang || "").trim() === "mermaid") {
        return `<div class="mermaid" data-source="${esc(text)}">${esc(text)}</div>`;
      }
      return false; // 回退默认 renderer（产出 <pre><code class="language-xxx">）
    },
  },
  walkTokens(token) {
    // 危险协议（javascript:/data:text-html 等）链接 → href 替换为 #
    if (token.type === "link" && token.href && !SAFE_HREF.test(token.href)) {
      token.href = "#";
    }
  },
  gfm: true,
  breaks: false,
});

// 注册 KaTeX 扩展：tokenizer 复用标准规则，renderer 输出占位（不打包 katex 本体）
marked.use({
  extensions: [
    {
      name: "inlineKatex",
      level: "inline",
      start(src: string) {
        return src.indexOf("$");
      },
      tokenizer(src: string) {
        const match = src.match(inlineKatexRule);
        if (match) {
          return {
            type: "inlineKatex",
            raw: match[0],
            text: match[2].trim(),
            displayMode: match[1].length === 2,
          };
        }
        return undefined;
      },
      renderer(token) {
        const t = token as { text: string; displayMode: boolean };
        return katexPlaceholder(t.text, t.displayMode);
      },
    },
    {
      name: "blockKatex",
      level: "block",
      tokenizer(src: string) {
        const match = src.match(blockKatexRule);
        if (match) {
          return {
            type: "blockKatex",
            raw: match[0],
            text: match[2].trim(),
            displayMode: match[1].length === 2,
          };
        }
        return undefined;
      },
      renderer(token) {
        const t = token as { text: string; displayMode: boolean };
        return katexPlaceholder(t.text, t.displayMode) + "\n";
      },
    },
  ],
});

/**
 * 同步渲染 Markdown → HTML。GFM 全语法（表格/删除线/任务列表/引用/分割线）离线可用。
 * mermaid/代码高亮/KaTeX 留给 DOM 阶段的 renderRich() 异步增强。
 */
export function renderMarkdown(src: string): string {
  if (!src || !src.trim()) return '<p class="empty-body">（无内容）</p>';
  return marked.parse(src, { async: false }) as string;
}
