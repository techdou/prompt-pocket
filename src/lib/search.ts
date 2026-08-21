import type { Prompt } from "./types";

/**
 * 轻量级筛选 + 模糊匹配，不引入 fuse.js。
 * prompt 量级小（通常 < 500），全内存扫描足够快（亚毫秒）。
 *
 * 规则：
 * - 空查询：返回全部（按后端排好的顺序）
 * - 多关键词（空格分隔）：每段都需命中（标题/分类/标签/正文 任一）
 * - 匹配范围分两档：元信息（标题/分类/标签）> 正文
 * - 计分：元信息子串命中（1000 档）> 正文子串命中（500 档）> 元信息字符级模糊（个位数）
 * - 正文不做字符级模糊：长文本里任何字符序列都能"跳跃"凑出来，命中全是噪音
 */

/** 搜索文本归一化：小写 + 折叠空白（让跨行的多词查询也能子串命中） */
const norm = (s: string): string => s.toLowerCase().replace(/\s+/g, " ");

interface Row {
  p: Prompt;
  /** 归一化后的元信息搜索串：标题 + 分类 + 标签 */
  meta: string;
  /** 归一化后的正文搜索串 */
  body: string;
}

const scored = (row: Row, q: string): number => {
  // 元信息子串直接命中 → 最高分，位置越靠前越高
  const metaIdx = row.meta.indexOf(q);
  if (metaIdx >= 0) return 1000 - metaIdx;

  // 正文子串命中 → 中分（低于一切元信息命中，高于模糊命中），位置越靠前越高
  const bodyIdx = row.body.indexOf(q);
  if (bodyIdx >= 0) return 500 - Math.min(bodyIdx, 400);

  // 元信息字符级模糊（连续给高分），作为兜底
  let qi = 0;
  let score = 0;
  let lastIdx = -2;
  for (let i = 0; i < row.meta.length && qi < q.length; i++) {
    if (row.meta[i] === q[qi]) {
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
    // 无查询：保持后端返回的顺序（已按 category → order → updated 排好）
    return [...prompts];
  }

  // 每条 prompt 的搜索串只归一化一次（而非每个关键词算一遍）
  const rows: Row[] = prompts.map((p) => ({
    p,
    meta: norm(p.title + " " + p.category + " " + (p.meta.tags?.join(" ") ?? "")),
    body: norm(p.body),
  }));

  // 多关键词 AND
  const terms = norm(trimmed).split(" ").filter(Boolean);
  const results = rows
    .map((row) => {
      let total = 0;
      for (const t of terms) {
        const s = scored(row, t);
        if (s < 0) return null;
        total += s;
      }
      return { p: row.p, total };
    })
    .filter((x): x is { p: Prompt; total: number } => x !== null)
    .sort((a, b) => b.total - a.total);

  return results.map((r) => r.p);
}

/**
 * 正文命中摘录：当关键词只命中正文、标题/分类/标签解释不了"为什么搜到它"时，
 * 返回命中词上下文的一行摘录（供列表展示，帮用户认出目标）。
 * 元信息能解释或正文没命中 → 返回 null（列表保持原样）。
 */
export function bodyMatchSnippet(prompt: Prompt, query: string): string | null {
  const terms = norm(query.trim()).split(" ").filter(Boolean);
  if (terms.length === 0) return null;

  const meta = norm(
    prompt.title + " " + prompt.category + " " + (prompt.meta.tags?.join(" ") ?? ""),
  );
  // 摘录展示用折叠空白后的原文（保留大小写），索引用其小写副本定位
  const flat = prompt.body.replace(/\s+/g, " ").trim();
  const flatLower = flat.toLowerCase();

  for (const t of terms) {
    if (meta.includes(t)) continue; // 标题/分类/标签能解释这条结果，无需摘录
    const idx = flatLower.indexOf(t);
    if (idx < 0) continue;

    // 命中词前文最多带 20 字符、后文连词共 60 字符，两端截断处补省略号
    const start = Math.max(0, idx - 20);
    const end = Math.min(flat.length, idx + t.length + 40);
    const prefix = start > 0 ? "…" : "";
    const suffix = end < flat.length ? "…" : "";
    return prefix + flat.slice(start, end) + suffix;
  }
  return null;
}
