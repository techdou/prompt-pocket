import { invoke } from "@tauri-apps/api/core";
import type {
  CloudConfigView,
  CopyMode,
  Prompt,
  PromptMeta,
  PromptContent,
  SaveRequest,
  ScanResult,
  SyncStatus,
} from "./types";

type BackendPromptMeta = Partial<Omit<PromptMeta, "copy_mode">> & {
  copy_mode?: CopyMode;
  copyMode?: CopyMode;
};

type BackendPrompt = Omit<Prompt, "abs_path" | "meta"> & {
  abs_path?: string;
  absPath?: string;
  meta?: BackendPromptMeta;
};

type BackendPromptContent = Omit<PromptContent, "meta"> & {
  meta?: BackendPromptMeta;
};

type BackendScanResult = Omit<ScanResult, "prompts"> & {
  prompts: BackendPrompt[];
};

export function normalizeCopyMode(mode: unknown): CopyMode {
  return mode === "plain" ? "plain" : "markdown";
}

export function normalizePromptMeta(meta: BackendPromptMeta = {}): PromptMeta {
  return {
    title: meta.title ?? "",
    tags: meta.tags,
    copy_mode: normalizeCopyMode(meta.copy_mode ?? meta.copyMode),
    created: meta.created ?? "",
    updated: meta.updated ?? "",
  };
}

export function normalizePrompt(prompt: BackendPrompt): Prompt {
  return {
    ...prompt,
    abs_path: prompt.abs_path ?? prompt.absPath ?? "",
    meta: normalizePromptMeta(prompt.meta),
  };
}

export function normalizePromptContent(content: BackendPromptContent): PromptContent {
  return {
    ...content,
    meta: normalizePromptMeta(content.meta),
  };
}

function normalizeScanResult(result: BackendScanResult): ScanResult {
  return {
    ...result,
    prompts: result.prompts.map(normalizePrompt),
  };
}

// ── 提示词读写（本地缓存）──

export async function initApp(): Promise<void> {
  return invoke<void>("init_app");
}

export async function scanPrompts(): Promise<ScanResult> {
  return normalizeScanResult(await invoke<BackendScanResult>("scan_prompts"));
}

export async function readPrompt(path: string): Promise<PromptContent> {
  return normalizePromptContent(await invoke<BackendPromptContent>("read_prompt", { path }));
}

export async function savePrompt(path: string, req: SaveRequest): Promise<Prompt> {
  return normalizePrompt(
    await invoke<BackendPrompt>("save_prompt", {
      path,
      req: {
        ...req,
        copyMode: normalizeCopyMode(req.copy_mode),
      },
    }),
  );
}

export async function renamePrompt(
  path: string,
  newTitle: string,
  newCategory: string,
): Promise<Prompt> {
  return normalizePrompt(
    await invoke<BackendPrompt>("rename_prompt", { path, newTitle, newCategory }),
  );
}

export async function renameCategory(oldName: string, newName: string): Promise<void> {
  return invoke<void>("rename_category", { oldName, newName });
}

export async function createCategory(name: string): Promise<void> {
  return invoke<void>("create_category", { name });
}

export async function createPrompt(category: string, title: string): Promise<Prompt> {
  return normalizePrompt(await invoke<BackendPrompt>("create_prompt", { category, title }));
}

export async function deletePrompt(path: string): Promise<void> {
  return invoke<void>("delete_prompt", { path });
}

/** 拖拽排序：重写某分类的顺序 */
export async function reorderPrompts(
  category: string,
  paths: string[],
): Promise<void> {
  return invoke<void>("reorder", { category, paths });
}

// ── 剪贴板 / 窗口 ──

export async function copyText(text: string): Promise<void> {
  return invoke<void>("copy_text", { text });
}

/**
 * 智能复制/注入：写剪贴板 + 隐藏窗口 + 自动判断是否注入输入框。
 * 前台有文本光标 → 模拟 Ctrl+V 注入；否则纯复制到剪贴板。
 */
export async function copyOrPaste(
  text: string,
  mode: CopyMode,
): Promise<void> {
  return invoke<void>("copy_or_paste", { text, mode: normalizeCopyMode(mode) });
}

export async function hideWindow(): Promise<void> {
  return invoke<void>("hide_window");
}

export async function revealInFinder(path: string): Promise<void> {
  return invoke<void>("reveal_in_finder", { path });
}

// ── 坚果云同步 ──

export async function getSyncStatus(): Promise<SyncStatus> {
  return invoke<SyncStatus>("get_sync_status");
}

export async function getCloudConfig(): Promise<CloudConfigView> {
  return invoke<CloudConfigView>("get_cloud_config");
}

export async function testCloudConnection(
  username: string,
  password: string,
  remoteRoot: string,
): Promise<void> {
  return invoke<void>("test_cloud_connection", { username, password, remoteRoot });
}

export async function saveCloudConfig(
  username: string,
  password: string,
  remoteRoot: string,
): Promise<void> {
  return invoke<void>("save_cloud_config", { username, password, remoteRoot });
}

/** 上传到坚果云：本地所有文件推送到云端（只增不删） */
export async function uploadAll(): Promise<string> {
  return invoke<string>("upload_all");
}

/** 下载到本地：从坚果云拉取并覆盖本地 */
export async function downloadAll(): Promise<string> {
  return invoke<string>("download_all");
}

export async function openUrl(url: string): Promise<void> {
  return invoke<void>("open_url", { url });
}
