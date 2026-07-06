// CDN 富内容编排器：并行调用 mermaid/katex/highlight，各库独立 try/catch 互不影响。

import { renderMermaid } from "./mermaid";
import { renderKatex } from "./katex";
import { renderCode } from "./highlight";

/**
 * 渲染 Markdown 容器内的全部富内容（mermaid 图表 / katex 公式 / 代码高亮）。
 * 调用时机：marked 同步产出 HTML 入 DOM 后（tick() 之后）。
 * 各库加载/渲染失败均自行降级，此函数永不 reject。
 */
export async function renderRich(root: HTMLElement): Promise<void> {
  await Promise.allSettled([
    renderMermaid(root),
    renderKatex(root),
    renderCode(root),
  ]);
}

export { RemoteLoadError } from "./loadRemote";
