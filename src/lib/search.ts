import type { Prompt } from "./types";

const normalize = (text: string) => text.toLocaleLowerCase().replace(/\s+/g, " ");
type SearchRow = { title: string; category: string; tags: string[]; body: string };
const rows = new WeakMap<Prompt, SearchRow>();
function rowOf(prompt: Prompt): SearchRow {
  let row = rows.get(prompt);
  if (!row) {
    row = { title: normalize(prompt.title), category: normalize(prompt.category), tags: (prompt.meta.tags ?? []).map(normalize), body: normalize(prompt.body ?? "") };
    rows.set(prompt, row);
  }
  return row;
}
type Usage = Record<string, { useCount?: number }>;
const termsFor = (query: string) => query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
function score(prompt: Prompt, term: string): number {
  const row = rowOf(prompt);
  const title = row.title;
  if (title === term) return 2000;
  if (title.includes(term)) return 1000;
  if (row.category.includes(term)) return 300;
  if (row.tags.some((tag) => tag.includes(term))) return 200;
  if (row.body.includes(term)) return 100;
  let position = 0;
  for (const character of title) if (character === term[position]) position++;
  return position === term.length ? 40 : -1;
}
/** Empty queries preserve manual order; usage is a bounded relevance tie-breaker. */
export function filterPrompts(prompts: Prompt[], query: string, usage: Usage = {}): Prompt[] {
  const terms = termsFor(query);
  if (!terms.length) return [...prompts];
  return prompts.map((prompt) => {
    const scores = terms.map((term) => score(prompt, term));
    return { prompt, score: scores.some((value) => value < 0) ? -1 :
      scores.reduce((sum, value) => sum + value, 0) + Math.min(30, Math.log2(1 + Math.max(0, usage[prompt.path]?.useCount ?? 0))) };
  }).filter((result) => result.score >= 0).sort((a, b) => b.score - a.score).map((result) => result.prompt);
}
export function searchExcerpt(body: string, query: string, length = 110): string {
  const text = body.replace(/\s+/g, " ").trim();
  const lower = text.toLocaleLowerCase();
  const hits = termsFor(query).map((term) => lower.indexOf(term)).filter((index) => index >= 0);
  const start = Math.max(0, (hits.length ? Math.min(...hits) : 0) - 24);
  return (start ? "…" : "") + text.slice(start, start + length) + (start + length < text.length ? "…" : "");
}
/** Return text nodes, never HTML, so query/content cannot inject markup. */
export function highlightParts(text: string, query: string): { text: string; match: boolean }[] {
  const lower = text.toLocaleLowerCase();
  const flags = new Uint8Array(text.length);
  for (const term of termsFor(query)) {
    let offset = 0;
    while ((offset = lower.indexOf(term, offset)) >= 0) {
      flags.fill(1, offset, offset + term.length);
      offset += Math.max(1, term.length);
    }
  }
  const parts: { text: string; match: boolean }[] = [];
  for (let index = 0; index < text.length; index++) {
    const match = !!flags[index];
    if (parts.length && parts[parts.length - 1].match === match) parts[parts.length - 1].text += text[index];
    else parts.push({ text: text[index], match });
  }
  return parts;
}

const flatCache = new WeakMap<Prompt, { flat: string; flatLower: string }>();

export function bodyMatchSnippet(prompt: Prompt, query: string): string | null {
  const terms = normalize(query.trim()).split(" ").filter(Boolean);
  if (terms.length === 0) return null;

  const row = rowOf(prompt);
  const meta = row.title + " " + row.category;
  let cached = flatCache.get(prompt);
  if (!cached) {
    // 摘录展示用折叠空白后的原文（保留大小写），索引用其小写副本定位
    const flat = (prompt.body ?? "").replace(/\s+/g, " ").trim();
    cached = { flat, flatLower: flat.toLowerCase() };
    flatCache.set(prompt, cached);
  }
  const { flat, flatLower } = cached;

  for (const t of terms) {
    if (meta.includes(t)) continue; // 标题/分类能解释这条结果，无需摘录
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
