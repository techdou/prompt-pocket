import { invoke } from "@tauri-apps/api/core";
import type { AppConfig, Prompt, ScanResult } from "./types";

/**
 * 前端 → Rust 后端的命令桥。
 * 每个函数封装一次 invoke，集中处理错误与参数命名。
 */

/** 首次/启动时初始化：确定或创建数据目录，返回配置 */
export async function initApp(): Promise<AppConfig> {
  return invoke<AppConfig>("init_app");
}

/** 读取当前配置（设置界面用） */
export async function getConfig(): Promise<AppConfig> {
  return invoke<AppConfig>("get_config");
}

/** 设置数据目录并持久化，返回更新后的配置 */
export async function setDataDir(path: string): Promise<AppConfig> {
  return invoke<AppConfig>("set_data_dir", { path });
}

/** 弹出系统目录选择器，返回用户选的路径（取消则 null） */
export async function pickDataDir(): Promise<string | null> {
  return invoke<string | null>("pick_data_dir");
}

/** 在系统文件管理器中打开数据目录 */
export async function openDataDir(): Promise<void> {
  return invoke<void>("open_data_dir");
}

/** 扫描所有 prompt，返回列表与分类计数 */
export async function scanPrompts(): Promise<ScanResult> {
  return invoke<ScanResult>("scan_prompts");
}

/** 读取单个 prompt 的完整内容（frontmatter + 正文） */
export async function readPrompt(
  path: string,
): Promise<{ meta_raw: string; body: string }> {
  return invoke<{ meta_raw: string; body: string }>("read_prompt", { path });
}

/** 保存 prompt（写回文件，自动刷新 updated 时间戳） */
export async function savePrompt(
  path: string,
  content: string,
): Promise<Prompt> {
  return invoke<Prompt>("save_prompt", { path, content });
}

/** 新建 prompt：在指定分类下创建文件，返回新 prompt */
export async function createPrompt(
  category: string,
  title: string,
): Promise<Prompt> {
  return invoke<Prompt>("create_prompt", { category, title });
}

/** 删除 prompt 文件 */
export async function deletePrompt(path: string): Promise<void> {
  return invoke<void>("delete_prompt", { path });
}

/** 写入剪贴板（纯文本） */
export async function copyText(text: string): Promise<void> {
  return invoke<void>("copy_text", { text });
}

/** 隐藏主窗口（复制后调用，让用户回到原应用粘贴） */
export async function hideWindow(): Promise<void> {
  return invoke<void>("hide_window");
}

/** 在文件管理器中显示该文件（便于用 VSCode/Typora 编辑） */
export async function revealInFinder(path: string): Promise<void> {
  return invoke<void>("reveal_in_finder", { path });
}
