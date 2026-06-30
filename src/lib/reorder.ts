import type { Prompt } from "./types";

export const ALL_CATEGORY = "__all__";

export function getReorderCategory(
  selectedCategory: string,
  prompts: Pick<Prompt, "category">[],
): string | null {
  if (selectedCategory !== ALL_CATEGORY) return selectedCategory;
  const firstCategory = prompts[0]?.category;
  if (!firstCategory) return null;
  return prompts.every((prompt) => prompt.category === firstCategory)
    ? firstCategory
    : null;
}

export function getReorderDisabledReason(
  query: string,
  selectedCategory: string,
  prompts: Pick<Prompt, "category">[],
): string {
  if (query.trim()) return "搜索结果不支持拖拽排序";
  if (prompts.length < 2) return "至少需要 2 条提示词才能排序";
  if (getReorderCategory(selectedCategory, prompts) === null) {
    return "切到单个分类后可拖拽排序";
  }
  return "";
}

export function canReorderPromptList(
  query: string,
  selectedCategory: string,
  prompts: Pick<Prompt, "category">[],
): boolean {
  return (
    !query.trim() &&
    prompts.length >= 2 &&
    getReorderCategory(selectedCategory, prompts) !== null
  );
}

export function movePathOrder(
  prompts: Pick<Prompt, "path">[],
  from: number,
  to: number,
): string[] | null {
  if (from < 0 || from >= prompts.length || to < 0 || to > prompts.length) {
    return null;
  }
  if (to === from || to === from + 1) {
    return null;
  }

  const next = [...prompts];
  const [moved] = next.splice(from, 1);
  const insertAt = to > from ? to - 1 : to;
  next.splice(insertAt, 0, moved);
  return next.map((prompt) => prompt.path);
}
