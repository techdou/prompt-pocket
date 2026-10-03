import type { Prompt, PromptContent, RecoveryEntry } from "./lib/types";

/** Explicit, development-only browser fixture. Never reads or writes a real library. */
export function installPreview() {
  const sample = (title: string, category: string, body: string): Prompt => ({
    id: `${category}/${title}`, path: `${category}/${title}.md`, abs_path: "", title, category, body,
    meta: { title, copy_mode: "markdown", created: "2026-10-03T00:00:00Z", updated: "2026-10-03T00:00:00Z" },
  });
  let prompts = [
    sample("文章改写", "写作", "请把以下内容改写给 {{目标读者|普通读者}}，语气为 {{语气|自然}}：\n\n{{原文}}\n\n保留原意，优先使用短句。"),
    sample("项目复盘", "效率", "## 项目复盘\n\n请根据材料整理：\n\n1. 目标与完成情况\n2. 关键决策\n3. 行动计划与负责人"),
    sample("代码审查", "编程", "请检查代码的正确性、边界条件和可维护性。\n\n优先列出有具体证据的问题，并给出最小修复建议。"),
    sample("输出约定", "常用片段", "请用中文回答。先说结论，再解释必要的依据。不要编造数据。"),
    ...Array.from({ length: 14 }, (_, index) => sample(`参考提示词 ${index + 1}`, "参考", `第 ${index + 1} 条示例内容，用于验证长列表键盘导航。`)),
  ];
  let hotkey = "Ctrl+Alt+P";
  let sequence = 0;
  const recovery: (RecoveryEntry & { content: PromptContent })[] = [];
  const archive = (prompt: Prompt, kind: string) => recovery.unshift({
    id: String(++sequence), originalPath: prompt.path, createdAt: new Date().toISOString(), kind,
    content: { meta: { ...prompt.meta }, body: prompt.body ?? "" },
  });
  const find = (path: unknown) => {
    const prompt = prompts.find((entry) => entry.path === path);
    if (!prompt) throw new Error("FILE_NOT_FOUND");
    return prompt;
  };
  const internals = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
    transformCallback: () => ++sequence,
    unregisterCallback: () => {},
    invoke: async (command: string, args: Record<string, unknown> = {}): Promise<unknown> => {
      if (command.startsWith("plugin:event|")) return ++sequence;
      if (["init_app", "set_window_mode", "set_interaction_lock", "hide_window", "reveal_in_finder", "reorder", "reorder_categories"].includes(command)) return;
      if (command === "scan_prompts") return { prompts: structuredClone(prompts), categories: [...new Set(prompts.map((prompt) => prompt.category))].map((name) => ({ name, count: prompts.filter((prompt) => prompt.category === name).length })) };
      if (command === "read_prompt") {
        const prompt = structuredClone(find(args.path));
        await new Promise((resolve) => setTimeout(resolve, prompt.category === "写作" ? 140 : 30));
        return { meta: prompt.meta, body: prompt.body };
      }
      if (command === "copy_or_paste") return { status: "copied" };
      if (command === "get_sync_status") return { configured: false, enabled: false, lastSync: null, lastError: null, syncing: false };
      if (command === "get_cloud_config") return { username: "", remoteRoot: "PromptPocket", enabled: false, hasPassword: false };
      if (command === "get_hotkey") return hotkey;
      if (command === "set_hotkey") { hotkey = String(args.shortcut); return; }
      if (command === "list_recovery") return structuredClone(recovery);
      if (command === "read_recovery") return recovery.find((entry) => entry.id === args.id)?.content.body ?? "";
      if (command === "restore_recovery") {
        const entry = recovery.find((item) => item.id === args.id);
        if (!entry) throw new Error("Missing recovery entry");
        const restored = sample(`${entry.content.meta.title} 恢复副本`, entry.originalPath.split("/")[0], entry.content.body);
        prompts.push(restored); return restored.path;
      }
      if (command === "create_prompt") {
        const prompt = sample(String(args.title || ""), String(args.category), "");
        prompt.path = `${prompt.category}/new-${++sequence}.md`; prompts.push(prompt); return structuredClone(prompt);
      }
      if (command === "save_prompt") {
        const prompt = find(args.path);
        const req = args.req as { title: string; category: string; body: string; copyMode: "markdown" | "plain" };
        archive(prompt, "history");
        Object.assign(prompt, { title: req.title, category: req.category, body: req.body, path: `${req.category}/${req.title}.md`, meta: { ...prompt.meta, title: req.title, copy_mode: req.copyMode } });
        return structuredClone(prompt);
      }
      if (command === "delete_prompt") { archive(find(args.path), "deleted"); prompts = prompts.filter((prompt) => prompt.path !== args.path); return; }
      throw new Error(`浏览器预览不执行此系统操作 / Unavailable in preview: ${command}`);
    },
  };
  Object.assign(window, {
    __TAURI_INTERNALS__: internals,
    __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
  });
}
