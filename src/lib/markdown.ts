// 富 Markdown 渲染核心：marked 打包，离线可用 GFM 全语法。
// XSS 防护不依赖 DOMPurify——raw HTML 块一律转义显示，危险协议链接替换为 #。
// mermaid / 代码高亮 / KaTeX 走 DOM 阶段本地打包增强（见 ./renderers）。

import { Marked } from "marked";
import { markedHighlight } from "marked-highlight";

const escHtml = (s: string): string =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

const escAttr = (s: string): string => escHtml(s).replace(/"/g, "&quot;");

// 链接协议白名单：仅放行 http(s)/mailto/ftp/相对路径/锚点
const SAFE_HREF = /^(https?:|mailto:|ftp:|\/|#|\.\/|\.\.\/)/i;
// 图片额外放行 data:image/*（贴图常用 base64 内嵌）；data:text/html 等仍拦截
const SAFE_IMG = /^(https?:|ftp:|\/|#|\.\/|\.\.\/|data:image\/)/i;

const marked = new Marked();

// ── KaTeX tokenizer（手写，不依赖 katex 包本体）──
// 规则取自 marked-katex-extension：$...$ 行内、$$...$$/$...$ 块级。
// renderer 只产出占位标记，真正的渲染交给 DOM 阶段本地打包的 katex（见 renderers/katex.ts）。
const inlineKatexRule =
  /^(\${1,2})(?!\$)((?:\\.|[^\\\n])*?(?:\\.|[^\\\n$]))\1(?=[\s?!.,:？！。，：]|$)/;
const blockKatexRule = /^(\${1,2})\n((?:\\[^]|[^\\])+?)\n\1(?:\n|$)/;

/** 把 tex 源码包成占位元素，data-* 供 DOM 阶段读取 */
function katexPlaceholder(tex: string, displayMode: boolean): string {
  // 用 <code> 兜底：katex 未加载时至少显示等宽原文，不裸露 $ 符号干扰阅读
  const mode = displayMode ? "block" : "inline";
  return `<code class="katex-placeholder katex-${mode}" data-tex="${escAttr(tex)}" data-display="${displayMode}">${escHtml(tex)}</code>`;
}

// marked-highlight：只借它的 langPrefix 给 code 元素打 class，不在解析阶段着色。
// highlight 回调返回原文（=== token.text 时 marked-highlight 视为"无高亮"跳过转义标记），
// mermaid 由下方自定义 renderer 接管，highlight.js 留给 DOM 阶段本地处理。
marked.use(
  markedHighlight({
    langPrefix: "hljs language-",
    highlight(code, _lang) {
      return code;
    },
  }),
);

marked.use({
  // raw HTML 块/行内 HTML 一律转义，杜绝 <script>/<iframe>/<img onerror> 注入
  renderer: {
    html({ text }: { text: string }): string {
      return escHtml(text);
    },
    // mermaid 代码块输出占位 div，交给 DOM 阶段 renderMermaid() 渲染
    code({ text, lang }: { text: string; lang?: string }): string | false {
      if ((lang || "").trim() === "mermaid") {
        return `<div class="mermaid" data-source="${escAttr(text)}">${escHtml(text)}</div>`;
      }
      return false; // 回退默认 renderer（产出 <pre><code class="language-xxx">）
    },
  },
  walkTokens(token) {
    // 危险协议（javascript:/data:text-html 等）链接 → href 替换为 #
    if (token.type === "link" && token.href && !SAFE_HREF.test(token.href)) {
      token.href = "#";
    }
    // 图片同源防护：javascript:/data:text-html 等协议的 src 清空
    if (token.type === "image" && token.href && !SAFE_IMG.test(token.href)) {
      token.href = "";
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
        const t = token as unknown as { text: string; displayMode: boolean };
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
        const t = token as unknown as { text: string; displayMode: boolean };
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

/**
 * Markdown → 纯文本：copy_mode 为 plain 时去掉常见 Markdown 标记，
 * 让粘贴目标（不支持 Markdown 的输入框）拿到可读文本。
 * 轻量正则实现，目标是常见标记干净去除，不追求语义级还原；
 * 表格保留竖线结构（去掉标记后仍可读）。
 */
export function markdownToPlain(src: string): string {
  if (!src) return "";
  let out = src;
  // 围栏代码块：去围栏保留内容
  out = out.replace(/```[^\n]*\n([\s\S]*?)```/g, "$1");
  // 图片 ![alt](url) → alt；链接 [text](url) → text
  out = out.replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1");
  out = out.replace(/\[([^\]]*)\]\([^)]*\)/g, "$1");
  // 行内标记：**x** / __x__ / ~~x~~ / `x` / *x* → x
  out = out.replace(/\*\*([^*]+)\*\*/g, "$1");
  out = out.replace(/__([^_]+)__/g, "$1");
  out = out.replace(/~~([^~]+)~~/g, "$1");
  out = out.replace(/`([^`]+)`/g, "$1");
  out = out.replace(/\*([^*]+)\*/g, "$1");
  out = out.replace(/(?<!\w)_([^_]+)_(?!\w)/g, "$1");
  // 行首标记：标题 #、引用 >、任务列表 [ ]/[x]、无序 -/*/+、有序 1.
  out = out.replace(
    /^(\s*)(#{1,6}\s+|>\s?|[-*+]\s+\[[ xX]\]\s+|[-*+]\s+|\d+\.\s+)/gm,
    "$1",
  );
  // 水平线（--- / *** / ___）
  out = out.replace(/^\s*([-*_]\s*){3,}$/gm, "");
  return out.trim();
}
