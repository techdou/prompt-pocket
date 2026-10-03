// Prompt Pocket — Tauri 应用入口
//
// 架构：本地 Markdown + 手动坚果云 WebDAV 同步
// - UI 读写走本地缓存（store.rs），瞬间响应
// - 所有内容写入与云同步共享存储锁，覆盖前保留恢复副本
// - 账号/路径存 config.json，应用密码存系统凭据库

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, LogicalPosition, LogicalSize, Manager, WindowEvent,
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

mod recovery;
mod store;
mod sync;
use crate::store::{
    create_category as create_category_disk, create_prompt as create_prompt_disk,
    delete_prompt as delete_prompt_disk, read_prompt as read_prompt_disk,
    rename_category as rename_category_disk, rename_prompt as rename_prompt_disk,
    reorder_category as reorder_category_disk, save_prompt as save_prompt_disk,
    scan_prompts as scan_disk, Prompt, PromptContent, SaveRequest, ScanResult,
};
use crate::sync::{push_all_to_remote, CloudConfig, SyncStatus};

const GLOBAL_HOTKEY: &str = "Ctrl+Alt+P";
const FOCUS_RESTORE_TIMEOUT_MS: u64 = 120;
const FOCUS_RESTORE_POLL_MS: u64 = 10;
const CLOUD_PASSWORD_SERVICE: &str = "com.promptpocket.webdav";

// ────────────────────────────────────────────────────────────
// 配置持久化：config.json 存在 %APPDATA%/prompt-pocket/ 下
// ────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct PersistedConfig {
    username: Option<String>,
    #[serde(default, skip_serializing)]
    password: Option<String>,
    remote_root: Option<String>,
    enabled: Option<bool>,
    /// 旧版配置迁移用：本地目录（v0.x）— 现已弃用
    data_dir: Option<String>,
}

struct AppState {
    cloud: Mutex<CloudConfig>,
    local_dir: PathBuf,
    config_file: PathBuf,
    last_sync: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
    syncing: Mutex<bool>,
    paste_target: Mutex<Option<PasteTarget>>,
    storage_gate: tokio::sync::Mutex<()>,
    hotkey: tokio::sync::Mutex<String>,
    preferences_file: PathBuf,
    interaction_locked: AtomicBool,
}

impl AppState {
    fn cloud_config(&self) -> CloudConfig {
        // 毒锁时取出内部数据而非 panic（读操作安全）
        self.cloud.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_paste_target(&self, target: Option<PasteTarget>) {
        *self.paste_target.lock().unwrap_or_else(|e| e.into_inner()) = target;
    }

    fn take_paste_target(&self) -> Option<PasteTarget> {
        self.paste_target
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    fn set_cloud_config(&self, cfg: CloudConfig) -> Result<(), String> {
        persist_cloud_config(&self.config_file, &cfg, &SystemCloudSecretStore)?;
        {
            *self.cloud.lock().map_err(|e| e.to_string())? = cfg.clone();
        }
        Ok(())
    }

    fn sync_status(&self) -> SyncStatus {
        let cfg = self.cloud_config();
        SyncStatus {
            configured: cfg.is_configured(),
            enabled: cfg.enabled,
            last_sync: self
                .last_sync
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            last_error: self
                .last_error
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            syncing: *self.syncing.lock().unwrap_or_else(|e| e.into_inner()),
        }
    }
}

/// 本地缓存目录：%APPDATA%/com.promptpocket.app/PromptPocket/
fn resolve_local_dir(app: &tauri::AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    dir.join("PromptPocket")
}

fn resolve_config_file(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("config.json")
}

fn parse_hotkey(value: &str) -> Result<Shortcut, String> {
    let shortcut = value
        .trim()
        .parse::<Shortcut>()
        .map_err(|e| format!("无效快捷键: {e}"))?;
    if shortcut.mods.is_empty() {
        return Err("快捷键必须包含 Ctrl、Alt、Shift 或 Super 修饰键".into());
    }
    Ok(shortcut)
}

fn load_hotkey(path: &Path) -> String {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("hotkey")
                .and_then(|key| key.as_str())
                .map(str::to_owned)
        })
        .filter(|value| parse_hotkey(value).is_ok())
        .unwrap_or_else(|| GLOBAL_HOTKEY.into())
}

fn save_hotkey_preferences(path: &Path, hotkey: &str) -> Result<(), String> {
    let mut preferences = match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| e.to_string())?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(error.to_string()),
    };
    let object = preferences.as_object_mut().ok_or("偏好文件格式错误")?;
    object.insert("hotkey".into(), serde_json::Value::String(hotkey.into()));
    let bytes = serde_json::to_vec_pretty(&preferences).map_err(|e| e.to_string())?;
    recovery::atomic_write(path, &bytes).map_err(|e| e.to_string())
}

fn change_hotkey_transaction(
    current: &mut String,
    next: &str,
    old_registered: bool,
    mut register: impl FnMut(Shortcut) -> Result<(), String>,
    mut unregister: impl FnMut(Shortcut) -> Result<(), String>,
    mut persist: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    let next = next.trim();
    let new_key = parse_hotkey(next)?;
    let old_key = parse_hotkey(current)?;
    if old_registered && new_key == old_key {
        return Ok(());
    }
    register(new_key)?;
    if let Err(error) = persist(next) {
        let rollback = unregister(new_key);
        return Err(format!(
            "快捷键未更改，保存失败: {error}{}",
            rollback
                .err()
                .map(|e| format!("；撤销新键失败: {e}"))
                .unwrap_or_default()
        ));
    }
    if old_registered {
        if let Err(error) = unregister(old_key) {
            let disk_rollback = persist(current);
            let new_rollback = unregister(new_key);
            return Err(format!(
                "保留旧快捷键，撤销旧键失败: {error}{}{}",
                disk_rollback
                    .err()
                    .map(|e| format!("；恢复偏好失败: {e}"))
                    .unwrap_or_default(),
                new_rollback
                    .err()
                    .map(|e| format!("；撤销新键失败: {e}"))
                    .unwrap_or_default()
            ));
        }
    }
    *current = next.into();
    Ok(())
}

fn register_hotkey(app: &tauri::AppHandle, shortcut: Shortcut) -> Result<(), String> {
    app.global_shortcut()
        .on_shortcut(shortcut, |app, _, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let visible = app
                .get_webview_window("main")
                .and_then(|win| win.is_visible().ok())
                .unwrap_or(false);
            let target = if visible {
                None
            } else {
                foreground_identity().filter(|_| foreground_has_text_input_focus())
            };
            app.state::<AppState>().set_paste_target(target);
            toggle_main_window(app);
        })
        .map_err(|e| format!("快捷键注册失败（可能被占用）: {e}"))
}

#[tauri::command]
async fn get_hotkey(state: tauri::State<'_, AppState>) -> Result<String, String> {
    Ok(state.hotkey.lock().await.clone())
}

#[tauri::command]
async fn set_hotkey(
    shortcut: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut current = state.hotkey.lock().await;
    let registered = app.global_shortcut().is_registered(parse_hotkey(&current)?);
    change_hotkey_transaction(
        &mut current,
        &shortcut,
        registered,
        |key| register_hotkey(&app, key),
        |key| {
            app.global_shortcut()
                .unregister(key)
                .map_err(|e| e.to_string())
        },
        |value| save_hotkey_preferences(&state.preferences_file, value),
    )
}

/// 启动时加载配置
fn load_cloud_config(config_file: &std::path::Path) -> CloudConfig {
    load_cloud_config_with_store(config_file, &SystemCloudSecretStore)
}

trait CloudSecretStore {
    fn read_password(&self, username: &str) -> Result<Option<String>, String>;
    fn write_password(&self, username: &str, password: &str) -> Result<(), String>;
}

struct SystemCloudSecretStore;

impl CloudSecretStore for SystemCloudSecretStore {
    fn read_password(&self, username: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(CLOUD_PASSWORD_SERVICE, username)
            .map_err(|e| format!("打开系统凭据库失败: {e}"))?;
        match entry.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("读取系统凭据失败: {e}")),
        }
    }

    fn write_password(&self, username: &str, password: &str) -> Result<(), String> {
        if username.is_empty() || password.is_empty() {
            return Ok(());
        }
        let entry = keyring::Entry::new(CLOUD_PASSWORD_SERVICE, username)
            .map_err(|e| format!("打开系统凭据库失败: {e}"))?;
        entry
            .set_password(password)
            .map_err(|e| format!("写入系统凭据失败: {e}"))
    }
}

fn persisted_from_cloud_config(cfg: &CloudConfig) -> PersistedConfig {
    PersistedConfig {
        username: Some(cfg.username.clone()),
        password: None,
        remote_root: Some(cfg.remote_root.clone()),
        enabled: Some(cfg.enabled),
        data_dir: None,
    }
}

fn write_persisted_config(config_file: &Path, cfg: &PersistedConfig) -> Result<(), String> {
    if let Some(parent) = config_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    recovery::atomic_write(config_file, json.as_bytes()).map_err(|e| e.to_string())
}

fn persist_cloud_config(
    config_file: &Path,
    cfg: &CloudConfig,
    secret_store: &impl CloudSecretStore,
) -> Result<(), String> {
    secret_store.write_password(&cfg.username, &cfg.password)?;
    write_persisted_config(config_file, &persisted_from_cloud_config(cfg))
}

fn load_cloud_config_with_store(
    config_file: &std::path::Path,
    secret_store: &impl CloudSecretStore,
) -> CloudConfig {
    let raw = std::fs::read_to_string(config_file).ok();
    let parsed = raw.and_then(|s| serde_json::from_str::<PersistedConfig>(&s).ok());
    let username = parsed
        .as_ref()
        .and_then(|p| p.username.clone())
        .unwrap_or_default();
    let legacy_password = parsed.as_ref().and_then(|p| p.password.clone());
    let stored_password = if username.is_empty() {
        Ok(None)
    } else {
        secret_store.read_password(&username)
    };
    let password = stored_password
        .as_ref()
        .ok()
        .and_then(|p| p.clone())
        .or_else(|| legacy_password.clone())
        .unwrap_or_default();

    if let (Ok(None), Some(legacy)) = (&stored_password, legacy_password.as_ref()) {
        if !username.is_empty() && secret_store.write_password(&username, legacy).is_ok() {
            let cfg = PersistedConfig {
                username: Some(username.clone()),
                password: None,
                remote_root: parsed.as_ref().and_then(|p| p.remote_root.clone()),
                enabled: parsed.as_ref().and_then(|p| p.enabled),
                data_dir: None,
            };
            let _ = write_persisted_config(config_file, &cfg);
        }
    }

    CloudConfig {
        username,
        password,
        remote_root: parsed
            .as_ref()
            .and_then(|p| p.remote_root.clone())
            .unwrap_or_else(|| "PromptPocket".to_string()),
        enabled: parsed.as_ref().and_then(|p| p.enabled).unwrap_or(false),
    }
}

// ────────────────────────────────────────────────────────────
// 同步辅助：后台 spawn 异步任务（不阻塞 UI）
// ────────────────────────────────────────────────────────────

// ────────────────────────────────────────────────────────────
// Tauri 命令
// ────────────────────────────────────────────────────────────

#[tauri::command]
async fn init_app(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _storage = state.storage_gate.lock().await;
    std::fs::create_dir_all(&state.local_dir).map_err(|e| e.to_string())?;
    let any_md = walkdir_has_md(&state.local_dir);
    if !any_md {
        seed_sample_prompts(&state.local_dir)?;
    }
    Ok(())
}

fn walkdir_has_md(dir: &std::path::Path) -> bool {
    for entry in walkdir::WalkDir::new(dir)
        .min_depth(1)
        .into_iter()
        .flatten()
    {
        if entry.path().extension().and_then(|e| e.to_str()) == Some("md") {
            return true;
        }
    }
    false
}

/// 扫描本地缓存
#[tauri::command]
fn scan_prompts(state: tauri::State<'_, AppState>) -> Result<ScanResult, String> {
    scan_disk(&state.local_dir).map_err(|e| e.to_string())
}

#[tauri::command]
fn read_prompt(path: String, state: tauri::State<'_, AppState>) -> Result<PromptContent, String> {
    let abs = resolve_abs(&state.local_dir, &path)?;
    read_prompt_disk(&abs).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "FILE_NOT_FOUND".to_string()
        } else {
            e.to_string()
        }
    })
}

#[tauri::command]
async fn save_prompt(
    path: String,
    req: SaveRequest,
    state: tauri::State<'_, AppState>,
) -> Result<Prompt, String> {
    let _storage = state.storage_gate.lock().await;
    let abs = resolve_abs(&state.local_dir, &path)?;
    // save_prompt 现在返回新路径（可能因标题重命名而变化）
    let new_abs = save_prompt_disk(&state.local_dir, &abs, &req).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "FILE_NOT_FOUND".to_string()
        } else {
            e.to_string()
        }
    })?;
    // 用新路径查找返回的 prompt（路径可能变了，必须用 new_abs）
    let result = scan_disk(&state.local_dir)
        .map_err(|e| e.to_string())?
        .prompts
        .into_iter()
        .find(|p| same_disk_path(Path::new(&p.abs_path), &new_abs))
        .ok_or_else(|| "保存后未能重新定位该提示词".to_string())?;
    Ok(result)
}

#[tauri::command]
async fn rename_prompt(
    path: String,
    new_title: String,
    new_category: String,
    state: tauri::State<'_, AppState>,
) -> Result<Prompt, String> {
    let _storage = state.storage_gate.lock().await;
    let old_abs = resolve_abs(&state.local_dir, &path)?;
    let new_abs = rename_prompt_disk(&state.local_dir, &old_abs, &new_title, &new_category)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "FILE_NOT_FOUND".to_string()
            } else {
                e.to_string()
            }
        })?;

    scan_disk(&state.local_dir)
        .map_err(|e| e.to_string())?
        .prompts
        .into_iter()
        .find(|p| same_disk_path(Path::new(&p.abs_path), &new_abs))
        .ok_or_else(|| "重命名后未能定位该提示词".to_string())
}

#[tauri::command]
async fn rename_category(
    old_name: String,
    new_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _storage = state.storage_gate.lock().await;
    rename_category_disk(&state.local_dir, &old_name, &new_name).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn create_category(name: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _storage = state.storage_gate.lock().await;
    create_category_disk(&state.local_dir, &name).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn create_prompt(
    category: String,
    title: String,
    state: tauri::State<'_, AppState>,
) -> Result<Prompt, String> {
    let _storage = state.storage_gate.lock().await;
    let abs = create_prompt_disk(&state.local_dir, &category, &title).map_err(|e| e.to_string())?;
    let rel_unix = relative_library_path(&state.local_dir, &abs)?;
    scan_disk(&state.local_dir)
        .map_err(|e| e.to_string())?
        .prompts
        .into_iter()
        .find(|p| p.path == rel_unix)
        .ok_or_else(|| "新建后未能定位该提示词".to_string())
}

#[tauri::command]
async fn delete_prompt(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _storage = state.storage_gate.lock().await;
    let abs = resolve_abs(&state.local_dir, &path)?;
    delete_prompt_disk(&state.local_dir, &abs).map_err(|e| e.to_string())?;
    Ok(())
}

/// 拖拽排序：重写某分类的顺序到 .order.json（纯本地，手动上传时才同步）
#[tauri::command]
async fn reorder(
    category: String,
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _storage = state.storage_gate.lock().await;
    reorder_category_disk(&state.local_dir, &category, &paths).map_err(|e| e.to_string())?;
    Ok(())
}

/// 分类拖拽排序：重写 .category-order.json
#[tauri::command]
async fn reorder_categories(
    names: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _storage = state.storage_gate.lock().await;
    store::save_category_order(&state.local_dir, &names).map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_recovery(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<recovery::RecoveryEntry>, String> {
    let _storage = state.storage_gate.lock().await;
    recovery::list_recovery(&state.local_dir).map_err(|e| e.to_string())
}

#[tauri::command]
async fn read_recovery(id: String, state: tauri::State<'_, AppState>) -> Result<String, String> {
    let _storage = state.storage_gate.lock().await;
    recovery::read_recovery(&state.local_dir, &id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn restore_recovery(id: String, state: tauri::State<'_, AppState>) -> Result<String, String> {
    let _storage = state.storage_gate.lock().await;
    let restored = recovery::restore_recovery(&state.local_dir, &id).map_err(|e| e.to_string())?;
    relative_library_path(&state.local_dir, &restored)
}

#[tauri::command]
async fn copy_text(text: String, app: tauri::AppHandle) -> Result<(), String> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PasteTarget {
    window: usize,
    process_id: u32,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum CopyStatus {
    Copied,
    Pasted,
    PasteFailed,
}

#[derive(Debug, serde::Serialize)]
struct CopyResult {
    status: CopyStatus,
}

/// Formatting is resolved in the frontend. Once clipboard writing succeeds,
/// focus/hide/injection failures are returned as a partial-success outcome.
#[tauri::command]
async fn copy_or_paste(
    text: String,
    mode: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<CopyResult, String> {
    let _ = mode;
    app.clipboard()
        .write_text(&text)
        .map_err(|e| e.to_string())?;
    let target = state.take_paste_target();
    let hidden = app
        .get_webview_window("main")
        .is_some_and(|win| win.hide().is_ok());
    let restored = if let Some(target) = target.filter(|_| hidden) {
        wait_for_text_input_focus(
            target,
            std::time::Duration::from_millis(FOCUS_RESTORE_TIMEOUT_MS),
            std::time::Duration::from_millis(FOCUS_RESTORE_POLL_MS),
        )
        .await
            && matches_paste_target(
                target,
                foreground_identity(),
                foreground_has_text_input_focus(),
            )
    } else {
        false
    };
    Ok(finish_copy(
        target.is_some(),
        hidden,
        restored,
        simulate_paste,
    ))
}

fn finish_copy(
    should_paste: bool,
    hidden: bool,
    restored: bool,
    inject: impl FnOnce() -> Result<(), String>,
) -> CopyResult {
    let status = if !should_paste {
        CopyStatus::Copied
    } else if !hidden || !restored || inject().is_err() {
        CopyStatus::PasteFailed
    } else {
        CopyStatus::Pasted
    };
    CopyResult { status }
}

fn matches_paste_target(
    expected: PasteTarget,
    foreground: Option<PasteTarget>,
    has_input: bool,
) -> bool {
    has_input && foreground == Some(expected)
}

#[cfg(windows)]
fn foreground_identity() -> Option<PasteTarget> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        if process_id == 0 {
            return None;
        }
        Some(PasteTarget {
            window: hwnd.0 as usize,
            process_id,
        })
    }
}

#[cfg(not(windows))]
fn foreground_identity() -> Option<PasteTarget> {
    None
}

async fn wait_for_text_input_focus(
    target: PasteTarget,
    timeout: std::time::Duration,
    poll: std::time::Duration,
) -> bool {
    let started = std::time::Instant::now();
    loop {
        if matches_paste_target(
            target,
            foreground_identity(),
            foreground_has_text_input_focus(),
        ) {
            return true;
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return false;
        }
        tokio::time::sleep(std::cmp::min(poll, timeout - elapsed)).await;
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TextFocusSignal {
    None,
    GuiCaret,
    UiaTextInput,
}

#[cfg(any(windows, test))]
fn is_text_input_signal(signal: TextFocusSignal) -> bool {
    !matches!(signal, TextFocusSignal::None)
}

#[cfg(any(windows, test))]
fn is_uia_text_input_candidate(
    is_keyboard_focusable: bool,
    is_edit_control: bool,
    is_document_control: bool,
    has_value_pattern: bool,
    has_text_pattern: bool,
    has_text_edit_pattern: bool,
) -> bool {
    if !is_keyboard_focusable {
        return false;
    }

    is_edit_control
        || has_text_edit_pattern
        || (is_document_control && (has_value_pattern || has_text_pattern))
        || (has_value_pattern && has_text_pattern)
}

#[cfg(windows)]
fn foreground_has_text_input_focus() -> bool {
    is_text_input_signal(foreground_text_focus_signal())
}

#[cfg(not(windows))]
fn foreground_has_text_input_focus() -> bool {
    false
}

#[cfg(windows)]
fn foreground_text_focus_signal() -> TextFocusSignal {
    if uia_focused_element_is_text_input() == Some(true) {
        return TextFocusSignal::UiaTextInput;
    }
    if foreground_has_caret() {
        return TextFocusSignal::GuiCaret;
    }
    TextFocusSignal::None
}

#[cfg(windows)]
fn uia_focused_element_is_text_input() -> Option<bool> {
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, UIA_DocumentControlTypeId, UIA_EditControlTypeId,
        UIA_TextEditPatternId, UIA_TextPatternId, UIA_ValuePatternId,
    };

    unsafe {
        let coinit = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let should_uninitialize = coinit.is_ok();
        if coinit.is_err() && coinit != RPC_E_CHANGED_MODE {
            return None;
        }

        let result = (|| -> windows::core::Result<bool> {
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            let element = automation.GetFocusedElement()?;
            let is_keyboard_focusable = element.CurrentIsKeyboardFocusable()?.as_bool();
            let control_type = element.CurrentControlType()?;
            let is_edit_control = control_type == UIA_EditControlTypeId;
            let is_document_control = control_type == UIA_DocumentControlTypeId;
            let has_value_pattern = element.GetCurrentPattern(UIA_ValuePatternId).is_ok();
            let has_text_pattern = element.GetCurrentPattern(UIA_TextPatternId).is_ok();
            let has_text_edit_pattern = element.GetCurrentPattern(UIA_TextEditPatternId).is_ok();

            Ok(is_uia_text_input_candidate(
                is_keyboard_focusable,
                is_edit_control,
                is_document_control,
                has_value_pattern,
                has_text_pattern,
                has_text_edit_pattern,
            ))
        })();

        if should_uninitialize {
            CoUninitialize();
        }

        result.ok()
    }
}

/// 检测前台窗口是否正聚焦在一个有文本光标(caret)的控件上。
/// 用 GetGUIThreadInfo 查询前台窗口所属线程的 caret 信息。
#[cfg(windows)]
fn foreground_has_caret() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
    };

    unsafe {
        let hwnd = GetForegroundWindow();
        // hwnd 为 0/无效说明无前台窗口（罕见）
        if hwnd.is_invalid() {
            return false;
        }
        let thread_id = GetWindowThreadProcessId(hwnd, None);
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(thread_id, &mut info).is_err() {
            return false;
        }
        // hwndCaret 非空 → 前台窗口里有个正在编辑的文本控件
        !info.hwndCaret.is_invalid()
    }
}

/// 判断指定 Tauri 窗口当前是否为 Win32 前台窗口。
///
/// 用途：拖拽 / 缩放窗口时，WebView 可能误报 `Focused(false)`，
/// 但此时主窗口仍处于系统的 move/size 模态、仍是 GetForegroundWindow() 的结果。
/// 这种情况下不应该 hide，否则窗口会"闪一下消失"。
#[cfg(windows)]
fn is_window_in_foreground(win: &tauri::Window) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsChild};

    // hwnd() 在 Windows 上返回 Result<HWND>，Err 说明窗口未就绪
    let Ok(our_hwnd) = win.hwnd() else {
        return false;
    };
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_invalid() {
        return false;
    }
    let is_same = foreground == our_hwnd;
    let is_child = unsafe { IsChild(our_hwnd, foreground).as_bool() };
    foreground_matches_app_window(is_same, is_child)
}

#[cfg(not(windows))]
fn is_window_in_foreground(_win: &tauri::Window) -> bool {
    false
}

#[cfg(any(windows, test))]
fn foreground_matches_app_window(is_same_window: bool, is_child_window: bool) -> bool {
    is_same_window || is_child_window
}

#[cfg(windows)]
fn simulate_paste() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    enigo
        .key(Key::Control, Direction::Press)
        .map_err(|e| e.to_string())?;
    let pasted = enigo.key(Key::Unicode('v'), Direction::Click);
    let released = enigo.key(Key::Control, Direction::Release);
    pasted.and(released).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn simulate_paste() -> Result<(), String> {
    Err("此平台仅支持复制".into())
}

#[tauri::command]
fn hide_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        win.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn set_window_mode(mode: String, app: tauri::AppHandle) -> Result<(), String> {
    let (width, height) = match mode.as_str() {
        "quick" => (680.0, 500.0),
        "manage" => (960.0, 640.0),
        _ => return Err("未知窗口模式".into()),
    };
    let window = app.get_webview_window("main").ok_or("主窗口未就绪")?;
    window
        .set_min_size(Some(LogicalSize::new(640.0, 440.0)))
        .map_err(|e| e.to_string())?;
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_interaction_lock(locked: bool, state: tauri::State<'_, AppState>) {
    state.interaction_locked.store(locked, Ordering::SeqCst);
}

#[tauri::command]
fn reveal_in_finder(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let abs = resolve_abs(&state.local_dir, &path)?;
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .args(["/select,", &abs.to_string_lossy()])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &abs.to_string_lossy()])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        let dir = abs.parent().unwrap_or(std::path::Path::new("."));
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ────────────────────────────────────────────────────────────
// 云同步命令
// ────────────────────────────────────────────────────────────

#[tauri::command]
fn get_sync_status(state: tauri::State<'_, AppState>) -> SyncStatus {
    state.sync_status()
}

#[tauri::command]
fn get_cloud_config(state: tauri::State<'_, AppState>) -> serde_json::Value {
    let cfg = state.cloud_config();
    serde_json::json!({
        "username": cfg.username,
        "remoteRoot": cfg.remote_root,
        "enabled": cfg.enabled,
        "hasPassword": !cfg.password.is_empty()
    })
}

#[tauri::command]
async fn test_cloud_connection(
    username: String,
    password: String,
    remote_root: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let password = resolve_cloud_password(&username, &password, &state.cloud_config())?;
    let cfg = CloudConfig {
        username,
        password,
        remote_root,
        enabled: true,
    };
    sync::test_connection(&cfg).await
}

#[tauri::command]
fn save_cloud_config(
    username: String,
    password: String,
    remote_root: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let final_password = resolve_cloud_password(&username, &password, &state.cloud_config())?;

    let cfg = CloudConfig {
        username,
        password: final_password,
        remote_root,
        enabled: true,
    };
    state.set_cloud_config(cfg)?;
    Ok(())
}

fn resolve_cloud_password(
    username: &str,
    password: &str,
    stored: &CloudConfig,
) -> Result<String, String> {
    if password != "__KEEP__" {
        return Ok(password.into());
    }
    if username != stored.username || stored.password.is_empty() {
        return Err("账号已变化或没有已保存的密码，请重新输入应用密码".into());
    }
    Ok(stored.password.clone())
}

struct SyncGuard<'a>(&'a Mutex<bool>);
impl<'a> SyncGuard<'a> {
    fn begin(flag: &'a Mutex<bool>) -> Result<Self, String> {
        let mut syncing = flag.lock().map_err(|e| e.to_string())?;
        if *syncing {
            return Err("正在同步中，请稍候".into());
        }
        *syncing = true;
        Ok(Self(flag))
    }
}
impl Drop for SyncGuard<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = false;
    }
}

/// 上传到坚果云：本地所有文件推送到云端（只增不删）
#[tauri::command]
async fn upload_all(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let cfg = state.cloud_config();
    if !cfg.is_configured() {
        return Err("未配置坚果云同步".to_string());
    }
    let _sync = SyncGuard::begin(&state.syncing)?;
    let _storage = state.storage_gate.lock().await;
    let result = push_all_to_remote(&cfg, &state.local_dir).await;
    // Clear before emitting the status update; Drop also covers cancelled tasks.
    drop(_sync);
    match result {
        Ok(report) => {
            let mut msg = format!(
                "上传完成：共 {} 个文件，保留冲突 {}",
                report.uploaded, report.conflicts
            );
            if !report.errors.is_empty() {
                msg.push_str(&format!("，{} 个失败", report.errors.len()));
                *state.last_error.lock().map_err(|e| e.to_string())? =
                    Some(report.errors.join("; "));
            } else {
                *state.last_error.lock().map_err(|e| e.to_string())? = None;
            }
            *state.last_sync.lock().map_err(|e| e.to_string())? = Some(msg.clone());
            let _ = app.emit("sync-finished", ());
            Ok(msg)
        }
        Err(e) => {
            *state.last_error.lock().map_err(|e| e.to_string())? = Some(e.clone());
            Err(e)
        }
    }
}

/// 下载到本地：依据共同基线更新，保留本地独有内容与冲突副本。
#[tauri::command]
async fn download_all(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let cfg = state.cloud_config();
    if !cfg.is_configured() {
        return Err("未配置坚果云同步".to_string());
    }
    let _sync = SyncGuard::begin(&state.syncing)?;
    let _storage = state.storage_gate.lock().await;
    let result = sync::pull_from_remote(&cfg, &state.local_dir).await;
    // Clear before emitting the status update; Drop also covers cancelled tasks.
    drop(_sync);
    match result {
        Ok(report) => {
            let mut msg = format!(
                "下载完成：更新 {}，跳过 {}，清理 {}，保留冲突 {}",
                report.downloaded, report.skipped, report.deleted, report.conflicts
            );
            if !report.errors.is_empty() {
                msg.push_str(&format!("，{} 个失败", report.errors.len()));
                *state.last_error.lock().map_err(|e| e.to_string())? =
                    Some(report.errors.join("; "));
            } else {
                *state.last_error.lock().map_err(|e| e.to_string())? = None;
            }
            *state.last_sync.lock().map_err(|e| e.to_string())? = Some(msg.clone());
            let _ = app.emit("sync-finished", ());
            Ok(msg)
        }
        Err(e) => {
            *state.last_error.lock().map_err(|e| e.to_string())? = Some(e.clone());
            Err(e)
        }
    }
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    // 打开外部链接（坚果云帮助页等）。URL 先做协议白名单，再交给平台 opener。
    let url = url.trim();
    if !is_allowed_external_url(url) {
        return Err("不支持打开该链接协议".to_string());
    }
    open_external_url(url)
}

fn open_external_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use windows::core::HSTRING;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let file = HSTRING::from(url);
        let result = unsafe { ShellExecuteW(None, None, &file, None, None, SW_SHOWNORMAL) };
        if (result.0 as isize) <= 32 {
            return Err(format!(
                "打开链接失败: ShellExecuteW 返回 {}",
                result.0 as isize
            ));
        }
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn is_allowed_external_url(url: &str) -> bool {
    let scheme = url
        .trim()
        .split_once(':')
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    matches!(scheme.as_deref(), Some("https" | "http" | "mailto"))
}

// ────────────────────────────────────────────────────────────
// 辅助
// ────────────────────────────────────────────────────────────

fn resolve_abs(root: &Path, rel: &str) -> Result<PathBuf, String> {
    recovery::checked_path(root, rel).map_err(|e| e.to_string())
}

fn same_disk_path(left: &Path, right: &Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    strip_unc_prefix(&left) == strip_unc_prefix(&right)
}

fn relative_library_path(root: &Path, path: &Path) -> Result<String, String> {
    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let rel = path
        .strip_prefix(&root)
        .map_err(|_| "路径不在提示词库内".to_string())?;
    Ok(store::path_to_unix(rel))
}

#[cfg(windows)]
fn strip_unc_prefix(path: &std::path::Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{stripped}"))
    } else if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

#[cfg(not(windows))]
fn strip_unc_prefix(path: &std::path::Path) -> PathBuf {
    path.to_path_buf()
}

fn seed_sample_prompts(dir: &std::path::Path) -> Result<(), String> {
    let samples = [
        (
            "写作",
            "改写润色.md",
            "---\ntitle: 改写润色\ncopy_mode: markdown\ncreated: 2026-06-27T00:00:00Z\nupdated: 2026-06-27T00:00:00Z\n---\n\n请把下面这段文字改写得更**简洁、专业**：\n\n> 待改写的内容\n\n要求：\n- 保持原意\n- 消除口语化表达\n- 控制在原长度以内\n",
        ),
        (
            "写作",
            "周报模板.md",
            "---\ntitle: 周报模板\ncopy_mode: markdown\ncreated: 2026-06-27T00:00:00Z\nupdated: 2026-06-27T00:00:00Z\n---\n\n请帮我生成本周工作周报：\n\n## 本周完成\n- \n\n## 进行中\n- \n\n## 下周计划\n- \n\n## 风险与求助\n- \n",
        ),
        (
            "编程",
            "代码审查.md",
            "---\ntitle: 代码审查\ncopy_mode: markdown\ncreated: 2026-06-27T00:00:00Z\nupdated: 2026-06-27T00:00:00Z\n---\n\n请审查以下代码，从这些维度给出改进建议：\n\n1. **可读性**：命名、注释、结构\n2. **正确性**：边界条件、潜在 bug\n3. **性能**：时间/空间复杂度\n\n```\n// 待审查代码\n```\n",
        ),
    ];
    for (cat, name, content) in samples {
        let path =
            recovery::checked_path(dir, Path::new(cat).join(name)).map_err(|e| e.to_string())?;
        recovery::atomic_write(&path, content.as_bytes())
            .map_err(|e| format!("写入示例文件失败: {e}"))?;
    }
    Ok(())
}

fn toggle_main_window(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    match win.is_visible() {
        Ok(true) => {
            let _ = win.hide();
        }
        _ => {
            // 多屏跟随鼠标定位：找到鼠标所在的显示器，在该屏居中显示
            position_window_at_cursor(&win);
            if win.show().is_ok() {
                let _ = win.emit("window-shown", ());
            }
            let _ = win.set_focus();
        }
    }
}

/// 把窗口定位到鼠标光标所在的显示器中央（支持多屏）
fn position_window_at_cursor(win: &tauri::WebviewWindow) {
    // 获取鼠标当前位置（物理坐标，相对桌面左上角）
    let cursor = match win.cursor_position() {
        Ok(p) => p,
        Err(_) => {
            let _ = win.center();
            return;
        }
    };

    // 鼠标坐标统一转 i32（cursor_position 返回 f64，monitor 用 i32）
    let (cx, cy) = (cursor.x as i32, cursor.y as i32);

    // 遍历所有显示器，找到包含鼠标位置的那个
    let target_monitor = win
        .available_monitors()
        .ok()
        .into_iter()
        .flatten()
        .find(|m| {
            let pos = m.position();
            let size = m.size();
            cx >= pos.x
                && cx < pos.x + size.width as i32
                && cy >= pos.y
                && cy < pos.y + size.height as i32
        })
        .or_else(|| win.current_monitor().ok().flatten());

    let Some(monitor) = target_monitor else {
        let _ = win.center();
        return;
    };

    let mon_pos = monitor.position();
    let mon_size = monitor.size();
    let scale = monitor.scale_factor();

    // 窗口逻辑尺寸
    let (win_w, win_h) = match win.inner_size() {
        Ok(s) => (s.width as i32, s.height as i32),
        Err(_) => (960, 600),
    };

    // 在目标显示器中央定位（物理坐标），再转 logical 设置
    let center_x = mon_pos.x + (mon_size.width as i32 - win_w) / 2;
    let center_y = mon_pos.y + (mon_size.height as i32 - win_h) / 2;
    let logical_x = center_x as f64 / scale;
    let logical_y = center_y as f64 / scale;
    let _ = win.set_position(LogicalPosition::new(logical_x, logical_y));
}

// ────────────────────────────────────────────────────────────
// 启动
// ────────────────────────────────────────────────────────────

/// 判定是否首次启动：用本地数据目录下的 `.first_run_done` 标志文件。
/// 文件存在 → 已经跑过一次，走隐藏逻辑；不存在 → 首次启动，显示窗口并写入标志。
fn is_first_run(local_dir: &Path) -> bool {
    !local_dir.join(".first_run_done").exists()
}

fn mark_first_run_done(local_dir: &Path) {
    let _ = std::fs::write(local_dir.join(".first_run_done"), "");
}

fn should_mark_first_run_done(first_run: bool, window_was_shown: bool) -> bool {
    first_run && window_was_shown
}

/// 首次启动豁免标志：首次启动显示窗口后，第一次失焦不应立即把窗口藏起来
/// （否则用户鼠标一点别的窗口，主界面就消失，体验很差）。
/// 用 atomic 而非 Mutex：只需要 set true / 读取后置 false，无需持锁。
static FIRST_RUN_SUPPRESS_BLUR_HIDE: AtomicBool = AtomicBool::new(false);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例锁必须最先注册：第二实例启动时唤起已有窗口，而不是新开进程
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 用户再次双击 exe 时走到这里：聚焦到已有窗口
            if let Some(win) = app.get_webview_window("main") {
                app.state::<AppState>().set_paste_target(None);
                if win.show().is_ok() { let _ = win.emit("window-shown", ()); }
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(move |app| {
            let local_dir = resolve_local_dir(app.handle());
            let config_file = resolve_config_file(app.handle());
            let cloud = load_cloud_config(&config_file);
            let preferences_file = config_file.with_file_name("preferences.json");
            let hotkey = load_hotkey(&preferences_file);

            // 确保本地缓存目录存在
            let _ = std::fs::create_dir_all(&local_dir);

            app.manage(AppState {
                cloud: Mutex::new(cloud.clone()),
                local_dir: local_dir.clone(),
                config_file,
                last_sync: Mutex::new(None),
                last_error: Mutex::new(None),
                syncing: Mutex::new(false),
                paste_target: Mutex::new(None),
                storage_gate: tokio::sync::Mutex::new(()),
                hotkey: tokio::sync::Mutex::new(hotkey.clone()),
                preferences_file,
                interaction_locked: AtomicBool::new(false),
            });

            // v1.0.1：同步改为纯手动，启动时不再自动拉取

            if let Err(error) = parse_hotkey(&hotkey).and_then(|key| register_hotkey(app.handle(), key)) {
                eprintln!("[启动] {error}");
                app.dialog().message(format!("全局快捷键 {hotkey} 注册失败。\n\n你仍可以通过托盘打开主界面，并在设置中修改快捷键。\n{error}"))
                    .kind(MessageDialogKind::Warning).title("快捷键注册失败").show(|_| {});
            }

            // 系统托盘：左键单击 toggle 窗口；右键菜单提供「显示 / 退出」
            let show_item = MenuItem::with_id(app, "show", "显示主界面", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;
            let mut tray_builder = TrayIconBuilder::with_id("tray-main").tooltip("Prompt Pocket");
            if let Some(icon) = app.default_window_icon().cloned() {
                tray_builder = tray_builder.icon(icon);
            } else {
                eprintln!("[启动] 未找到默认窗口图标，托盘将使用系统默认图标");
            }
            tray_builder
                .menu(&menu)
                .on_tray_icon_event(|tray, event| {
                    // 左键单击（按下后抬起）切换窗口显隐 —— 给用户一个稳定的可见入口
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        tray.app_handle().state::<AppState>().set_paste_target(None);
                        toggle_main_window(tray.app_handle());
                    }
                })
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "show" => {
                            app.state::<AppState>().set_paste_target(None);
                            if let Some(win) = app.get_webview_window("main") {
                                if win.show().is_ok() { let _ = win.emit("window-shown", ()); }
                                let _ = win.set_focus();
                            }
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            // 启动时的窗口显隐策略：
            // - 首次启动（无 .first_run_done）：必须显示窗口，给新用户一个看得见的入口
            // - 非首次：保持隐藏，等快捷键 / 托盘唤起（贴入式工具的常规行为）
            // 注：dev/release 行为保持一致，避免"dev 测不出 release bug"
            let first_run = is_first_run(&local_dir);
            let mut first_run_window_was_shown = false;
            if let Some(win) = app.get_webview_window("main") {
                if first_run {
                    first_run_window_was_shown = win.show().is_ok();
                    let _ = win.set_focus();
                    // 首次启动豁免一次失焦隐藏：避免新用户鼠标一点别的窗口主界面就消失
                    if first_run_window_was_shown {
                        FIRST_RUN_SUPPRESS_BLUR_HIDE.store(true, Ordering::SeqCst);
                    }
                } else {
                    let _ = win.hide();
                }
            }
            if should_mark_first_run_done(first_run, first_run_window_was_shown) {
                mark_first_run_done(&local_dir);
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(false) = event {
                if window.state::<AppState>().interaction_locked.load(Ordering::SeqCst) { return; }
                // 首次启动豁免：第一次失焦不隐藏，让用户看清界面、可以自由点别处。
                // 之后的失焦恢复正常的"贴入式"行为（点外部即收起）。
                if FIRST_RUN_SUPPRESS_BLUR_HIDE.swap(false, Ordering::SeqCst) {
                    return;
                }
                // 拖拽 / 缩放窗口时 WebView 会误报失焦，但此时窗口仍为前台窗口
                // （系统处于 move/size 模态）。这种情况不能 hide，否则窗口闪一下消失。
                if is_window_in_foreground(window) {
                    return;
                }
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            init_app,
            scan_prompts,
            read_prompt,
            save_prompt,
            rename_prompt,
            rename_category,
            create_category,
            create_prompt,
            delete_prompt,
            reorder,
            reorder_categories,
            list_recovery,
            read_recovery,
            restore_recovery,
            get_hotkey,
            set_hotkey,
            set_window_mode,
            set_interaction_lock,
            copy_text,
            copy_or_paste,
            hide_window,
            reveal_in_finder,
            get_sync_status,
            get_cloud_config,
            test_cloud_connection,
            save_cloud_config,
            upload_all,
            download_all,
            open_url,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn hotkey_registration_and_persistence_failures_keep_old_binding() {
        use std::cell::RefCell;
        for fail_registration in [false, true] {
            let mut current = GLOBAL_HOTKEY.to_string();
            let registered = RefCell::new(vec![GLOBAL_HOTKEY.parse::<Shortcut>().unwrap()]);
            let disk = RefCell::new(current.clone());
            let result = change_hotkey_transaction(
                &mut current,
                "Ctrl+Alt+K",
                true,
                |key| {
                    if fail_registration {
                        Err("occupied".into())
                    } else {
                        registered.borrow_mut().push(key);
                        Ok(())
                    }
                },
                |key| {
                    registered.borrow_mut().retain(|k| k != &key);
                    Ok(())
                },
                |value| {
                    if value == "Ctrl+Alt+K" {
                        Err("read-only preferences".into())
                    } else {
                        *disk.borrow_mut() = value.into();
                        Ok(())
                    }
                },
            );
            assert!(result.is_err());
            assert_eq!(current, GLOBAL_HOTKEY);
            assert_eq!(*disk.borrow(), GLOBAL_HOTKEY);
            assert_eq!(
                *registered.borrow(),
                vec![GLOBAL_HOTKEY.parse::<Shortcut>().unwrap()]
            );
        }
    }

    #[test]
    fn hotkey_change_persists_before_removing_old_binding() {
        use std::cell::RefCell;
        let mut current = GLOBAL_HOTKEY.into();
        let steps = RefCell::new(Vec::new());
        change_hotkey_transaction(
            &mut current,
            "Ctrl+Alt+K",
            true,
            |_| {
                steps.borrow_mut().push("register");
                Ok(())
            },
            |_| {
                steps.borrow_mut().push("unregister");
                Ok(())
            },
            |_| {
                steps.borrow_mut().push("persist");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*steps.borrow(), vec!["register", "persist", "unregister"]);
        assert_eq!(current, "Ctrl+Alt+K");
        assert!(parse_hotkey("invalid key").is_err());
        assert!(parse_hotkey("P").is_err());
        assert!(parse_hotkey(GLOBAL_HOTKEY).is_ok());
    }

    #[test]
    fn copy_success_is_preserved_when_hiding_focus_or_injection_fails() {
        assert_eq!(
            finish_copy(false, false, false, || panic!("must not paste")).status,
            CopyStatus::Copied
        );
        assert_eq!(
            finish_copy(true, false, true, || panic!("must not paste")).status,
            CopyStatus::PasteFailed
        );
        assert_eq!(
            finish_copy(true, true, false, || panic!("must not paste")).status,
            CopyStatus::PasteFailed
        );
        assert_eq!(
            finish_copy(true, true, true, || Err("injection failed".into())).status,
            CopyStatus::PasteFailed
        );
        assert_eq!(
            finish_copy(true, true, true, || Ok(())).status,
            CopyStatus::Pasted
        );
    }

    #[test]
    fn paste_target_requires_same_window_and_process() {
        let original = PasteTarget {
            window: 42,
            process_id: 7,
        };
        assert!(matches_paste_target(original, Some(original), true));
        assert!(!matches_paste_target(
            original,
            Some(PasteTarget {
                window: 43,
                process_id: 7
            }),
            true
        ));
        assert!(!matches_paste_target(
            original,
            Some(PasteTarget {
                window: 42,
                process_id: 8
            }),
            true
        ));
        assert!(!matches_paste_target(original, Some(original), false));
        assert!(!matches_paste_target(original, None, true));
    }

    #[test]
    fn keep_password_rejects_account_changes() {
        let stored = CloudConfig {
            username: "one".into(),
            password: "secret".into(),
            ..Default::default()
        };
        assert_eq!(
            resolve_cloud_password("one", "__KEEP__", &stored).unwrap(),
            "secret"
        );
        assert!(resolve_cloud_password("two", "__KEEP__", &stored).is_err());
        assert_eq!(
            resolve_cloud_password("two", "new", &stored).unwrap(),
            "new"
        );
    }

    #[test]
    fn native_path_resolution_rejects_escape_and_leaf_symlinks() {
        let dir = std::env::temp_dir().join(format!("pp-native-paths-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "body").unwrap();
        assert!(resolve_abs(&dir, "../escape.md").is_err());
        assert!(resolve_abs(&dir, dir.join("a.md").to_str().unwrap()).is_err());
        let resolved = resolve_abs(&dir, "a.md").unwrap();
        assert_eq!(relative_library_path(&dir, &resolved).unwrap(), "a.md");
        assert!(same_disk_path(&resolved, &dir.join("a.md")));
        let request = SaveRequest {
            title: "a".into(),
            body: "updated".into(),
            copy_mode: "markdown".into(),
            category: Some("未分类".into()),
        };
        let saved = save_prompt_disk(&dir, &resolved, &request).unwrap();
        assert!(same_disk_path(&saved, &resolved));
        assert_eq!(recovery::list_recovery(&dir).unwrap().len(), 1);
        assert!(!dir.join("a-1.md").exists());
        recovery::snapshot(&dir, &resolved, "history").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("a.md"), dir.join("link.md")).unwrap();
            assert!(resolve_abs(&dir, "link.md").is_err());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sync_guard_prevents_duplicates_and_releases_after_failure() {
        let flag = Mutex::new(false);
        let first = SyncGuard::begin(&flag).unwrap();
        assert!(SyncGuard::begin(&flag).is_err());
        drop(first);
        assert!(SyncGuard::begin(&flag).is_ok());
        assert!(!*flag.lock().unwrap());
    }

    #[test]
    fn hotkey_preferences_preserve_cloud_config_and_other_preferences() {
        let dir = std::env::temp_dir().join(format!("pp-native-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let preferences = dir.join("preferences.json");
        assert_eq!(load_hotkey(&preferences), GLOBAL_HOTKEY);
        std::fs::write(dir.join("config.json"), "cloud config").unwrap();
        std::fs::write(&preferences, r#"{"other":true}"#).unwrap();
        save_hotkey_preferences(&preferences, "Ctrl+Alt+K").unwrap();
        assert_eq!(load_hotkey(&preferences), "Ctrl+Alt+K");
        assert!(std::fs::read_to_string(&preferences)
            .unwrap()
            .contains("\"other\": true"));
        assert_eq!(
            std::fs::read_to_string(dir.join("config.json")).unwrap(),
            "cloud config"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[derive(Default)]
    struct MemorySecretStore {
        passwords: Mutex<HashMap<String, String>>,
    }

    impl CloudSecretStore for MemorySecretStore {
        fn read_password(&self, username: &str) -> Result<Option<String>, String> {
            Ok(self.passwords.lock().unwrap().get(username).cloned())
        }

        fn write_password(&self, username: &str, password: &str) -> Result<(), String> {
            self.passwords
                .lock()
                .unwrap()
                .insert(username.to_string(), password.to_string());
            Ok(())
        }
    }

    #[test]
    fn focus_restore_polling_is_short_and_bounded() {
        let poll_ms = FOCUS_RESTORE_POLL_MS;
        let timeout_ms = FOCUS_RESTORE_TIMEOUT_MS;
        assert!(poll_ms <= 10);
        assert!(timeout_ms <= 120);
        assert!(timeout_ms >= poll_ms);
    }

    #[test]
    fn text_focus_signal_accepts_uia_and_legacy_caret() {
        assert!(is_text_input_signal(TextFocusSignal::UiaTextInput));
        assert!(is_text_input_signal(TextFocusSignal::GuiCaret));
        assert!(!is_text_input_signal(TextFocusSignal::None));
    }

    #[test]
    fn uia_candidate_detects_modern_text_inputs_without_legacy_caret() {
        assert!(is_uia_text_input_candidate(
            true, true, false, false, false, false,
        ));
        assert!(is_uia_text_input_candidate(
            true, false, true, false, true, false,
        ));
        assert!(is_uia_text_input_candidate(
            true, false, false, false, false, true,
        ));
    }

    #[test]
    fn uia_candidate_rejects_non_focusable_or_weak_value_controls() {
        assert!(!is_uia_text_input_candidate(
            false, true, false, true, true, true,
        ));
        assert!(!is_uia_text_input_candidate(
            true, false, false, true, false, false,
        ));
        assert!(!is_uia_text_input_candidate(
            true, false, false, false, true, false,
        ));
    }

    #[test]
    fn global_hotkey_stays_ctrl_alt_p() {
        assert_eq!(GLOBAL_HOTKEY, "Ctrl+Alt+P");
    }

    #[test]
    fn persisted_cloud_config_never_serializes_password() {
        let persisted = PersistedConfig {
            username: Some("user@example.com".into()),
            password: Some("secret-app-password".into()),
            remote_root: Some("PromptPocket".into()),
            enabled: Some(true),
            data_dir: None,
        };

        let json = serde_json::to_string(&persisted).unwrap();

        assert!(!json.contains("secret-app-password"));
        assert!(!json.contains("\"password\""));
    }

    #[test]
    fn cloud_config_persists_password_to_secret_store_not_json() {
        let dir = std::env::temp_dir().join("pp_test_cloud_config_secure");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.json");
        let store = MemorySecretStore::default();

        persist_cloud_config(
            &config_file,
            &CloudConfig {
                username: "user@example.com".into(),
                password: "secret-app-password".into(),
                remote_root: "PromptPocket".into(),
                enabled: true,
            },
            &store,
        )
        .unwrap();

        let json = std::fs::read_to_string(&config_file).unwrap();
        assert!(!json.contains("secret-app-password"));
        assert!(!json.contains("\"password\""));
        assert_eq!(
            store.read_password("user@example.com").unwrap().as_deref(),
            Some("secret-app-password")
        );
    }

    #[test]
    fn legacy_cloud_config_password_is_migrated_out_of_json() {
        let dir = std::env::temp_dir().join("pp_test_cloud_config_migrate");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.json");
        std::fs::write(
            &config_file,
            r#"{
  "username": "user@example.com",
  "password": "legacy-secret",
  "remote_root": "PromptPocket",
  "enabled": true
}"#,
        )
        .unwrap();
        let store = MemorySecretStore::default();

        let cfg = load_cloud_config_with_store(&config_file, &store);

        assert_eq!(cfg.password, "legacy-secret");
        assert_eq!(
            store.read_password("user@example.com").unwrap().as_deref(),
            Some("legacy-secret")
        );
        let json = std::fs::read_to_string(&config_file).unwrap();
        assert!(!json.contains("legacy-secret"));
        assert!(!json.contains("\"password\""));
    }

    #[test]
    fn first_run_marker_is_written_only_after_visible_window() {
        assert!(should_mark_first_run_done(true, true));
        assert!(!should_mark_first_run_done(true, false));
        assert!(!should_mark_first_run_done(false, true));
        assert!(!should_mark_first_run_done(false, false));
    }

    #[test]
    fn foreground_match_accepts_webview_child_window() {
        assert!(foreground_matches_app_window(true, false));
        assert!(foreground_matches_app_window(false, true));
        assert!(!foreground_matches_app_window(false, false));
    }

    #[test]
    fn open_url_allows_only_external_safe_protocols() {
        assert!(is_allowed_external_url("https://example.com"));
        assert!(is_allowed_external_url("http://example.com"));
        assert!(is_allowed_external_url("HTTPS://example.com/?a=1&b=2"));
        assert!(is_allowed_external_url("mailto:hello@example.com"));
        assert!(!is_allowed_external_url("javascript:alert(1)"));
        assert!(!is_allowed_external_url(
            "file:///C:/Windows/System32/calc.exe"
        ));
        assert!(!is_allowed_external_url("./local-file.md"));
    }
}
