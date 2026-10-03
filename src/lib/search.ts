import type { Prompt } from "./types";

type Usage = Record<string, { useCount?: number }>;
const termsFor = (query: string) => query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
function score(prompt: Prompt, term: string): number {
  const title = prompt.title.toLocaleLowerCase();
  if (title === term) return 2000;
  if (title.includes(term)) return 1000;
  if (prompt.category.toLocaleLowerCase().includes(term)) return 300;
  if (prompt.meta.tags?.some((tag) => tag.toLocaleLowerCase().includes(term))) return 200;
  if ((prompt.body ?? "").toLocaleLowerCase().includes(term)) return 100;
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
