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
  CopyResult,
  PasteSessionSnapshot,
  RecoveryEntry,
  SyncReport,
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
  const { copy_mode, ...rest } = req;
  return normalizePrompt(
    await invoke<BackendPrompt>("save_prompt", {
      path,
      req: {
        ...rest,
        copyMode: normalizeCopyMode(copy_mode),
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

/** 分类拖拽排序：重写 .category-order.json */
export async function reorderCategories(names: string[]): Promise<void> {
  return invoke<void>("reorder_categories", { names });
}

// ── 剪贴板 / 窗口 ──

export async function copyText(text: string): Promise<void> {
  return invoke<void>("copy_text", { text });
}

/**
 * 智能复制/注入：写剪贴板 + 隐藏窗口，按"快捷键唤起时记住的原窗口"判定注入。
 * 有记忆目标 → 等焦点回原窗口后模拟 Ctrl+V 注入；否则纯复制到剪贴板。
 *
 * generation 是触发这次调用的那次唤起的代次（见 getPasteSession）。后端据此
 * 识别请求是否已经过时：期间用户重新唤起或收起过窗口，请求作废且不碰任何东西。
 */
export async function copyOrPaste(
  text: string,
  mode: CopyMode,
  hide = true,
  generation: number | null = null,
): Promise<CopyResult> {
  return invoke<CopyResult>("copy_or_paste", {
    text,
    mode: normalizeCopyMode(mode),
    hide,
    generation,
  });
}

/** 当前唤起会话的快照：Enter 会粘贴还是只复制，以及这次唤起的代次 */
export const getPasteSession = () =>
  invoke<PasteSessionSnapshot>("get_paste_session");

/**
 * 自动粘贴失败时的系统通知：主窗口此刻已隐藏，应用内 toast 用户看不到。
 * 通知权限未授予时尝试请求一次；失败静默（剪贴板已有内容，重新唤起后
 * 仍能看到应用内 toast 兜底）。
 */
export async function notifyPasteFailed(title: string, body: string): Promise<void> {
  try {
    const { isPermissionGranted, requestPermission, sendNotification } =
      await import("@tauri-apps/plugin-notification");
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    if (granted) sendNotification({ title, body });
  } catch {
    // 通知链路异常不影响主流程
  }
}

export const getHotkey = () => invoke<string>("get_hotkey");
export const setHotkey = (shortcut: string) => invoke<void>("set_hotkey", { shortcut });
export const listRecovery = () => invoke<RecoveryEntry[]>("list_recovery");
export const restoreRecovery = (id: string) => invoke<string>("restore_recovery", { id });
export const readRecovery = (id: string) => invoke<string>("read_recovery", { id });
export const setWindowMode = (mode: "quick" | "manage") => invoke<void>("set_window_mode", { mode });
export const setInteractionLock = (locked: boolean) => invoke<void>("set_interaction_lock", { locked });

/**
 * 收起主窗口（Esc）。generation 是按下 Esc 时所属的唤起代次：上一次唤起里
 * 按下的 Esc 晚到一步时，不能收起现在这一次唤起的窗口
 */
export async function hideWindow(generation: number | null = null): Promise<void> {
  return invoke<void>("hide_window", { generation });
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
export async function uploadAll(): Promise<SyncReport> {
  return invoke<SyncReport>("upload_all");
}

/** 下载到本地：从坚果云拉取并覆盖本地 */
export async function downloadAll(): Promise<SyncReport> {
  return invoke<SyncReport>("download_all");
}

export async function openUrl(url: string): Promise<void> {
  return invoke<void>("open_url", { url });
}

// ── GitHub 存档 ──

export async function testGithubConnection(
  repo: string,
  token: string,
  branch: string,
  prefix: string,
): Promise<void> {
  return invoke<void>("test_github_connection", { repo, token, branch, prefix });
}

export async function saveGithubConfig(
  repo: string,
  token: string,
  branch: string,
  prefix: string,
): Promise<void> {
  return invoke<void>("save_github_config", { repo, token, branch, prefix });
}

/** 切换同步后端（"webdav" | "github"）；两侧配置互不影响，立即持久化 */
export async function setSyncProvider(provider: string): Promise<void> {
  return invoke<void>("set_sync_provider", { provider });
}


// ── 开机自启动 ──

/** 读取系统真实自启动状态（注册表/LaunchAgent），不读配置文件 */
export async function getAutostart(): Promise<boolean> {
  return invoke<boolean>("get_autostart");
}

export async function setAutostart(enabled: boolean): Promise<void> {
  return invoke<void>("set_autostart", { enabled });
}
