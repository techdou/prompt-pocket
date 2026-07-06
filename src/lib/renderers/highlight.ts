// 代码高亮：CDN 按需加载 highlight.js + atom-one-dark 主题。
// 扫描 <pre><code class="language-xxx">，用 highlightElement 着色。
// 失败什么都不做——<code> 自带等宽字体仍可读，这就是降级。

import { loadCss, loadScript } from "./loadRemote";

const HL_JS =
  "https://cdn.jsdelivr.net/npm/@highlightjs/cdn-assets@11.11.1/highlight.min.js";
const HL_CSS =
  "https://cdn.jsdelivr.net/npm/@highlightjs/cdn-assets@11.11.1/styles/atom-one-dark.min.css";

interface HljsApi {
  highlightElement: (el: HTMLElement) => void;
  getLanguage: (name: string) => unknown;
}

let hljsPromise: Promise<HljsApi> | null = null;

/** 加载 highlight.js（幂等，并行触发 JS + 主题 CSS） */
function ensureHighlight(): Promise<HljsApi> {
  if (hljsPromise) return hljsPromise;
  loadCss(HL_CSS);
  hljsPromise = loadScript<HljsApi>(HL_JS, "highlight.js", "hljs");
  hljsPromise.catch(() => {
    hljsPromise = null;
  });
  return hljsPromise;
}

/**
 * 高亮容器内所有 <pre><code>。
 * 用 dataset.rendered 防重复高亮。
 */
export async function renderCode(root: HTMLElement): Promise<void> {
  const blocks = Array.from(
    root.querySelectorAll<HTMLElement>("pre code:not([data-rendered])"),
  );
  if (blocks.length === 0) return;

  try {
    const hljs = await ensureHighlight();
    blocks.forEach((el) => {
      el.dataset.rendered = "yes";
      hljs.highlightElement(el);
    });
  } catch (e) {
    console.warn("[highlight.js] 加载失败，代码以纯文本展示", e);
  }
}
