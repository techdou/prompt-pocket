// Mermaid 图表渲染：CDN 按需加载 ESM，securityLevel=strict 防 XSS。
// 失败把 .mermaid 块回填源码到 <pre class="render-fallback">，绝不白屏。

import { importEsm } from "./loadRemote";

const MERMAID_URL =
  "https://cdn.jsdelivr.net/npm/mermaid@11.20.0/dist/mermaid.esm.min.mjs";

interface MermaidApi {
  initialize: (config: Record<string, unknown>) => void;
  run: (opts?: { nodes?: Element[] | NodeListOf<Element> }) => Promise<void>;
}

let mermaidPromise: Promise<MermaidApi> | null = null;

/** 加载并初始化 mermaid（幂等，仅首次 initialize） */
function ensureMermaid(): Promise<MermaidApi> {
  if (mermaidPromise) return mermaidPromise;
  mermaidPromise = (async () => {
    const mermaid = await importEsm<MermaidApi>(MERMAID_URL, "mermaid");
    mermaid.initialize({
      startOnLoad: false, // 手动调 run()
      securityLevel: "strict", // 用户输入不可信，禁掉 html label 里的脚本
      theme: "default",
    });
    return mermaid;
  })();
  // 失败时清空 promise，允许下次重试
  mermaidPromise.catch(() => {
    mermaidPromise = null;
  });
  return mermaidPromise;
}

/**
 * 渲染容器内所有 .mermaid 块。
 * 加载/渲染失败时把源码回填进 <pre>，不抛错。
 */
export async function renderMermaid(root: HTMLElement): Promise<void> {
  const blocks = Array.from(root.querySelectorAll<HTMLElement>(".mermaid"));
  if (blocks.length === 0) return;

  let mermaid: MermaidApi;
  try {
    mermaid = await ensureMermaid();
  } catch (e) {
    blocks.forEach((b) => fallbackToSource(b, e));
    return;
  }

  // run() 会把 textContent 替换成 SVG，先备份源码
  const backups = new Map<Element, string>();
  blocks.forEach((b) => backups.set(b, b.getAttribute("data-source") ?? b.textContent ?? ""));

  try {
    await mermaid.run({ nodes: blocks });
  } catch {
    // run() 可能整体 reject；逐块检查，未渲染成功的回填源码
    blocks.forEach((b) => {
      if (!b.querySelector("svg")) {
        fallbackToSource(b, new Error("mermaid.run 渲染失败"));
      }
    });
  }
}

/** 把节点替换成 <pre class="render-fallback"> 显示原文 */
function fallbackToSource(el: HTMLElement, err: unknown): void {
  const msg = err instanceof Error ? err.message : String(err);
  const source = el.getAttribute("data-source") ?? el.textContent ?? "";
  const pre = document.createElement("pre");
  pre.className = "render-fallback";
  pre.dataset.lib = "mermaid";
  pre.dataset.error = msg;
  pre.textContent = source;
  el.replaceWith(pre);
}
