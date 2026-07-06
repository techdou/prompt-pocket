// KaTeX 公式渲染：CDN 按需加载 UMD bundle + CSS。
// 占位元素由 markdown.ts 的 katexPlaceholder() 产出，此模块扫描并替换。
// 失败保留占位的 <code> 原文，不抛错。

import { loadCss, loadScript } from "./loadRemote";

const KATEX_JS = "https://cdn.jsdelivr.net/npm/katex@0.16.22/dist/katex.min.js";
const KATEX_CSS = "https://cdn.jsdelivr.net/npm/katex@0.16.22/dist/katex.min.css";

interface KatexApi {
  renderToString: (
    tex: string,
    opts?: {
      displayMode?: boolean;
      throwOnError?: boolean;
      output?: string;
    },
  ) => string;
}

let katexPromise: Promise<KatexApi> | null = null;

/** 加载 katex（幂等，并行触发 JS + CSS） */
function ensureKatex(): Promise<KatexApi> {
  if (katexPromise) return katexPromise;
  loadCss(KATEX_CSS); // 不 await，CSS 慢一点也不阻塞 JS 渲染
  katexPromise = loadScript<KatexApi>(KATEX_JS, "katex", "katex");
  katexPromise.catch(() => {
    katexPromise = null; // 失败清空，允许重试
  });
  return katexPromise;
}

/**
 * 渲染容器内所有 .katex-placeholder 占位。
 * 加载失败时占位 <code> 原样保留（等宽字体显示 tex 源码）。
 */
export async function renderKatex(root: HTMLElement): Promise<void> {
  const placeholders = Array.from(
    root.querySelectorAll<HTMLElement>(".katex-placeholder"),
  );
  if (placeholders.length === 0) return;

  let katex: KatexApi;
  try {
    katex = await ensureKatex();
  } catch (e) {
    console.warn("[katex] 加载失败，公式以源码展示", e);
    return;
  }

  for (const el of placeholders) {
    const tex = el.getAttribute("data-tex") ?? "";
    const display = el.getAttribute("data-display") === "true";
    try {
      const html = katex.renderToString(tex, {
        displayMode: display,
        throwOnError: false, // 出错公式渲染成红色错误提示而非抛异常
        output: "htmlAndMathml",
      });
      const wrapper = document.createElement(display ? "div" : "span");
      wrapper.className = display ? "katex-block" : "katex-inline";
      wrapper.innerHTML = html;
      el.replaceWith(wrapper);
    } catch (e) {
      // 单条公式渲染失败不影响其他，占位保留
      console.warn("[katex] 公式渲染失败，保留源码:", tex, e);
    }
  }
}
