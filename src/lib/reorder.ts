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

// 禁用原因用 code 而非文案，调用方（App.svelte）按 code 映射 i18n key。
// 旧版直接返回中文文案再当中间格式翻译，语言切换时会漏译。
export type ReorderDisabledReason =
  | "searchDisabled"
  | "needTwo"
  | "singleCategory"
  | null;

export function getReorderDisabledReason(
  query: string,
  selectedCategory: string,
  prompts: Pick<Prompt, "category">[],
): ReorderDisabledReason {
  if (query.trim()) return "searchDisabled";
  if (prompts.length < 2) return "needTwo";
  if (getReorderCategory(selectedCategory, prompts) === null) {
    return "singleCategory";
  }
  return null;
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
  // 移动逻辑与 moveCategoryOrder 完全一致，委托去重（旧版两份实现已漂移过一次）
  const next = moveCategoryOrder(prompts, from, to);
  return next ? next.map((prompt) => prompt.path) : null;
}

export function moveCategoryOrder<T>(items: T[], from: number, to: number): T[] | null {
  if (from < 0 || from >= items.length || to < 0 || to > items.length) {
    return null;
  }
  if (to === from || to === from + 1) {
    return null;
  }

  const next = [...items];
  const [moved] = next.splice(from, 1);
  const insertAt = to > from ? to - 1 : to;
  next.splice(insertAt, 0, moved);
  return next;
}

export interface HorizontalCategoryTabRect {
  tabIdx: number;
  left: number;
  right: number;
}

export interface HorizontalCategoryDropTarget {
  lineIndex: number;
  lineBefore: boolean;
  toIndex: number;
}

function targetBeforeTab(
  tab: HorizontalCategoryTabRect,
): HorizontalCategoryDropTarget | null {
  if (tab.tabIdx === ALL_CATEGORY_TAB_INDEX) return null;
  return {
    lineIndex: tab.tabIdx,
    lineBefore: true,
    toIndex: tab.tabIdx,
  };
}

function targetAfterTab(tab: HorizontalCategoryTabRect): HorizontalCategoryDropTarget {
  return {
    lineIndex: tab.tabIdx,
    lineBefore: false,
    toIndex: tab.tabIdx === ALL_CATEGORY_TAB_INDEX ? 0 : tab.tabIdx + 1,
  };
}

export const ALL_CATEGORY_TAB_INDEX = -1;

export function getHorizontalCategoryDropTarget(
  tabs: HorizontalCategoryTabRect[],
  clientX: number,
): HorizontalCategoryDropTarget | null {
  const sortedTabs = [...tabs].sort((a, b) => a.left - b.left);
  if (sortedTabs.length === 0 || clientX < sortedTabs[0].left) {
    return null;
  }

  for (const tab of sortedTabs) {
    if (clientX >= tab.left && clientX <= tab.right) {
      const before = clientX - tab.left < (tab.right - tab.left) / 2;
      return before ? targetBeforeTab(tab) : targetAfterTab(tab);
    }
  }

  for (let i = 0; i < sortedTabs.length - 1; i += 1) {
    const leftTab = sortedTabs[i];
    const rightTab = sortedTabs[i + 1];
    if (clientX > leftTab.right && clientX < rightTab.left) {
      const gapMidpoint = (leftTab.right + rightTab.left) / 2;
      return clientX <= gapMidpoint
        ? targetAfterTab(leftTab)
        : targetBeforeTab(rightTab);
    }
  }

  return targetAfterTab(sortedTabs[sortedTabs.length - 1]);
}

export function getHorizontalCategoryDropTargetWithFallback(
  tabs: HorizontalCategoryTabRect[],
  clientX: number,
  fallback: HorizontalCategoryDropTarget | null,
  insideVerticalBounds = true,
): HorizontalCategoryDropTarget | null {
  if (!insideVerticalBounds) return fallback;
  return getHorizontalCategoryDropTarget(tabs, clientX) ?? fallback;
}
