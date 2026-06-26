import type { Prompt } from "./types";

/**
 * 轻量级筛选 + 模糊匹配，不引入 fuse.js。
 * prompt 量级小（通常 < 500），全内存扫描足够快（亚毫秒）。
 *
 * 规则：
 * - 空查询：返回全部（按更新时间倒序）
 * - 多关键词（空格分隔）：每段都需命中（标题/分类/标签/正文 任一）
 * - 单字符命中率优先：连续匹配 > 跳跃匹配
 */

const scored = (prompt: Prompt, query: string): number => {
  const hay = (
    prompt.title +
    " " +
    prompt.category +
    " " +
    prompt.meta.tags.join(" ")
  ).toLowerCase();
  const q = query.toLowerCase();

  // 子串直接命中 → 最高分
  if (hay.includes(q)) return 1000 + hay.indexOf(q) * -1;

  // 否则做字符级模糊（连续给高分）
  let qi = 0;
  let score = 0;
  let lastIdx = -2;
  for (let i = 0; i < hay.length && qi < q.length; i++) {
    if (hay[i] === q[qi]) {
      score += i - lastIdx === 1 ? 5 : 1; // 连续匹配奖励
      lastIdx = i;
      qi++;
    }
  }
  // 全部字符都匹配上才算命中
  return qi === q.length ? score : -1;
};

export function filterPrompts(
  prompts: Prompt[],
  query: string,
): Prompt[] {
  const trimmed = query.trim();
  if (!trimmed) {
    // 无查询：置顶优先，再按更新时间倒序
    return [...prompts].sort((a, b) => {
      if (!!a.meta.pinned !== !!b.meta.pinned) {
        return a.meta.pinned ? -1 : 1;
      }
      return b.meta.updated.localeCompare(a.meta.updated);
    });
  }

  // 多关键词 AND
  const terms = trimmed.split(/\s+/);
  const results = prompts
    .map((p) => {
      let total = 0;
      for (const t of terms) {
        const s = scored(p, t);
        if (s < 0) return null;
        total += s;
      }
      return { p, total };
    })
    .filter((x): x is { p: Prompt; total: number } => x !== null)
    .sort((a, b) => b.total - a.total);

  return results.map((r) => r.p);
}
