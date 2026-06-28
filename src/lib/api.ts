import { invoke } from "@tauri-apps/api/core";
import type {
  CloudConfigView,
  Prompt,
  PromptContent,
  SaveRequest,
  ScanResult,
  SyncStatus,
} from "./types";

// ── 提示词读写（本地缓存）──

export async function initApp(): Promise<void> {
  return invoke<void>("init_app");
}

export async function scanPrompts(): Promise<ScanResult> {
  return invoke<ScanResult>("scan_prompts");
}

export async function readPrompt(path: string): Promise<PromptContent> {
  return invoke<PromptContent>("read_prompt", { path });
}

export async function savePrompt(path: string, req: SaveRequest): Promise<Prompt> {
  return invoke<Prompt>("save_prompt", { path, req });
}

export async function renamePrompt(
  path: string,
  newTitle: string,
  newCategory: string,
): Promise<Prompt> {
  return invoke<Prompt>("rename_prompt", { path, newTitle, newCategory });
}

export async function renameCategory(oldName: string, newName: string): Promise<void> {
  return invoke<void>("rename_category", { oldName, newName });
}

export async function createCategory(name: string): Promise<void> {
  return invoke<void>("create_category", { name });
}

export async function createPrompt(category: string, title: string): Promise<Prompt> {
  return invoke<Prompt>("create_prompt", { category, title });
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
  enabled: boolean,
): Promise<void> {
  return invoke<void>("save_cloud_config", { username, password, remoteRoot, enabled });
}

export async function syncNow(): Promise<void> {
  return invoke<void>("sync_now");
}

export async function openUrl(url: string): Promise<void> {
  return invoke<void>("open_url", { url });
}
