// 代码高亮：本地打包（npm highlight.js 常用语言集），动态 import 按需加载。
// 扫描 <pre><code class="language-xxx">，用 highlightElement 着色。
// 失败什么都不做——<code> 自带等宽字体仍可读，这就是降级。

import "highlight.js/styles/atom-one-dark.css";

interface HljsApi {
  highlightElement: (el: HTMLElement) => void;
  getLanguage: (name: string) => unknown;
}

let hljsPromise: Promise<HljsApi> | null = null;

/** 加载 highlight.js（幂等，常用语言子集控制包体积） */
function ensureHighlight(): Promise<HljsApi> {
  if (hljsPromise) return hljsPromise;
  hljsPromise = (async () => {
    const mod = await import("highlight.js/lib/common");
    return (mod.default ?? mod) as unknown as HljsApi;
  })();
  hljsPromise.catch(() => {
    hljsPromise = null;
  });
  return hljsPromise;
}

/**
 * 高亮容器内所有 <pre><code>。
 * 用 dataset.rendered 防重复高亮——注意先执行成功再打标，
 * 否则 highlightElement 抛错时该块会永远失去重试机会。
 */
export async function renderCode(root: HTMLElement): Promise<void> {
  const blocks = Array.from(
    root.querySelectorAll<HTMLElement>("pre code:not([data-rendered])"),
  );
  if (blocks.length === 0) return;

  try {
    const hljs = await ensureHighlight();
    blocks.forEach((el) => {
      try {
        hljs.highlightElement(el);
        el.dataset.rendered = "yes";
      } catch {
        // 单块失败（未知语言等）：不打标，下次 renderRich 还会重试
      }
    });
  } catch (e) {
    console.warn("[highlight.js] 加载失败，代码以纯文本展示", e);
  }
}
