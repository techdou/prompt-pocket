// 提示词变量占位符：{{变量名}}
// 复制时提取正文占位符让用户填空，替换后再进剪贴板。
// 语法约束：变量名为非空、不含花括号的任意文本（支持中文），
// 两侧空白被容忍（{{ a }} 与 {{a}} 同名）；留空的变量在替换时保留原样。

const PLACEHOLDER = /\{\{([^{}]*)\}\}/g;

/** 提取正文里全部占位符变量名，去重并保持首次出现顺序 */
export function extractVariables(body: string): string[] {
  const seen = new Set<string>();
  for (const match of body.matchAll(PLACEHOLDER)) {
    const name = match[1].trim();
    if (name) seen.add(name);
  }
  return [...seen];
}

/** 用填写的值替换占位符；未填写的占位符保留原文（含原空白格式） */
export function applyVariables(
  body: string,
  values: Record<string, string>,
): string {
  return body.replace(PLACEHOLDER, (match, rawName: string) => {
    const name = rawName.trim();
    // 只替换用户实际填写过的键：未出现的键和纯空白占位符原样保留
    return Object.hasOwn(values, name) ? values[name] : match;
  });
}
