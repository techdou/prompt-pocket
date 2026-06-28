// Prompt Pocket — Tauri 应用入口（v1.0.0 坚果云同步版）
//
// 架构：本地缓存 + 坚果云 WebDAV 后台同步
// - UI 读写走本地缓存（store.rs），瞬间响应
// - 启动时从坚果云拉取，保存后异步推送
// - 凭据存 %APPDATA%/config.json

use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Emitter, LogicalPosition, Manager, WindowEvent};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

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

// ────────────────────────────────────────────────────────────
// 配置持久化：config.json 存在 %APPDATA%/prompt-pocket/ 下
// ────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct PersistedConfig {
    username: Option<String>,
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
}

impl AppState {
    fn cloud_config(&self) -> CloudConfig {
        self.cloud.lock().unwrap().clone()
    }

    fn set_cloud_config(&self, cfg: CloudConfig) -> Result<(), String> {
        {
            *self.cloud.lock().unwrap() = cfg.clone();
        }
        let persisted = PersistedConfig {
            username: Some(cfg.username),
            password: Some(cfg.password),
            remote_root: Some(cfg.remote_root),
            enabled: Some(cfg.enabled),
            data_dir: None,
        };
        self.persist(persisted)
    }

    fn persist(&self, cfg: PersistedConfig) -> Result<(), String> {
        if let Some(parent) = self.config_file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
        std::fs::write(&self.config_file, json).map_err(|e| e.to_string())
    }

    fn sync_status(&self) -> SyncStatus {
        let cfg = self.cloud_config();
        SyncStatus {
            configured: cfg.is_configured(),
            enabled: cfg.enabled,
            last_sync: self.last_sync.lock().unwrap().clone(),
            last_error: self.last_error.lock().unwrap().clone(),
            syncing: *self.syncing.lock().unwrap(),
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

/// 启动时加载配置
fn load_cloud_config(config_file: &std::path::Path) -> CloudConfig {
    let raw = std::fs::read_to_string(config_file).ok();
    let parsed = raw
        .and_then(|s| serde_json::from_str::<PersistedConfig>(&s).ok());

    CloudConfig {
        username: parsed.as_ref().and_then(|p| p.username.clone()).unwrap_or_default(),
        password: parsed.as_ref().and_then(|p| p.password.clone()).unwrap_or_default(),
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
fn init_app(state: tauri::State<'_, AppState>) -> Result<(), String> {
    std::fs::create_dir_all(&state.local_dir).map_err(|e| e.to_string())?;
    let any_md = walkdir_has_md(&state.local_dir);
    if !any_md {
        seed_sample_prompts(&state.local_dir);
    }
    Ok(())
}

fn walkdir_has_md(dir: &std::path::Path) -> bool {
    for entry in walkdir::WalkDir::new(dir).min_depth(1).into_iter().flatten() {
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
fn read_prompt(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<PromptContent, String> {
    let abs = resolve_abs(&state.local_dir, &path);
    read_prompt_disk(&abs).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "FILE_NOT_FOUND".to_string()
        } else {
            e.to_string()
        }
    })
}

#[tauri::command]
fn save_prompt(
    path: String,
    req: SaveRequest,
    state: tauri::State<'_, AppState>,
) -> Result<Prompt, String> {
    let abs = resolve_abs(&state.local_dir, &path);
    save_prompt_disk(&abs, &req).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "FILE_NOT_FOUND".to_string()
        } else {
            e.to_string()
        }
    })?;
    let result = scan_disk(&state.local_dir)
        .map_err(|e| e.to_string())?
        .prompts
        .into_iter()
        .find(|p| p.abs_path == abs.to_string_lossy().to_string())
        .ok_or_else(|| "保存后未能重新定位该提示词".to_string())?;
    Ok(result)
}

#[tauri::command]
fn rename_prompt(
    path: String,
    new_title: String,
    new_category: String,
    state: tauri::State<'_, AppState>,
) -> Result<Prompt, String> {
    let old_abs = resolve_abs(&state.local_dir, &path);
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
        .find(|p| p.abs_path == new_abs.to_string_lossy().to_string())
        .ok_or_else(|| "重命名后未能定位该提示词".to_string())
}

#[tauri::command]
fn rename_category(
    old_name: String,
    new_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    rename_category_disk(&state.local_dir, &old_name, &new_name).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn create_category(
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    create_category_disk(&state.local_dir, &name).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn create_prompt(
    category: String,
    title: String,
    state: tauri::State<'_, AppState>,
) -> Result<Prompt, String> {
    let abs = create_prompt_disk(&state.local_dir, &category, &title).map_err(|e| e.to_string())?;
    let rel = abs.strip_prefix(&state.local_dir).unwrap_or(&abs);
    let rel_unix = store::path_to_unix(rel);
    scan_disk(&state.local_dir)
        .map_err(|e| e.to_string())?
        .prompts
        .into_iter()
        .find(|p| p.path == rel_unix)
        .ok_or_else(|| "新建后未能定位该提示词".to_string())
}

#[tauri::command]
fn delete_prompt(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let abs = resolve_abs(&state.local_dir, &path);
    delete_prompt_disk(&abs).map_err(|e| e.to_string())?;
    Ok(())
}

/// 拖拽排序：重写某分类的顺序到 .order.json（纯本地，手动上传时才同步）
#[tauri::command]
fn reorder(
    category: String,
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    reorder_category_disk(&state.local_dir, &category, &paths).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn copy_text(text: String, app: tauri::AppHandle) -> Result<(), String> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

#[tauri::command]
fn hide_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        win.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn reveal_in_finder(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let abs = resolve_abs(&state.local_dir, &path);
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
) -> Result<(), String> {
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
    // __KEEP__ 占位符表示保留旧密码（用户未重新填写）
    let final_password = if password == "__KEEP__" {
        state.cloud_config().password
    } else {
        password
    };

    let cfg = CloudConfig {
        username,
        password: final_password,
        remote_root,
        enabled: true,
    };
    state.set_cloud_config(cfg)?;
    Ok(())
}

/// 上传到坚果云：本地所有文件推送到云端（只增不删）
#[tauri::command]
async fn upload_all(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let cfg = state.cloud_config();
    if !cfg.is_configured() {
        return Err("未配置坚果云同步".to_string());
    }
    *state.syncing.lock().unwrap() = true;
    let result = push_all_to_remote(&cfg, &state.local_dir).await;
    *state.syncing.lock().unwrap() = false;
    match result {
        Ok(report) => {
            let msg = format!("上传完成：共 {} 个文件", report.uploaded);
            *state.last_sync.lock().unwrap() = Some(msg.clone());
            *state.last_error.lock().unwrap() = None;
            let _ = app.emit("sync-finished", ());
            Ok(msg)
        }
        Err(e) => {
            *state.last_error.lock().unwrap() = Some(e.clone());
            Err(e)
        }
    }
}

/// 下载到本地：从坚果云拉取并覆盖本地（清理本地多余文件）
#[tauri::command]
async fn download_all(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let cfg = state.cloud_config();
    if !cfg.is_configured() {
        return Err("未配置坚果云同步".to_string());
    }
    *state.syncing.lock().unwrap() = true;
    let result = sync::pull_from_remote(&cfg, &state.local_dir).await;
    *state.syncing.lock().unwrap() = false;
    match result {
        Ok(report) => {
            let msg = format!(
                "下载完成：更新 {}，跳过 {}，清理 {}",
                report.downloaded, report.skipped, report.deleted
            );
            *state.last_sync.lock().unwrap() = Some(msg.clone());
            *state.last_error.lock().unwrap() = None;
            let _ = app.emit("sync-finished", ());
            Ok(msg)
        }
        Err(e) => {
            *state.last_error.lock().unwrap() = Some(e.clone());
            Err(e)
        }
    }
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    // 打开外部链接（坚果云帮助页等）
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(&url).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(&url).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ────────────────────────────────────────────────────────────
// 辅助
// ────────────────────────────────────────────────────────────

fn resolve_abs(root: &std::path::Path, rel: &str) -> PathBuf {
    let joined = root.join(rel);
    match std::fs::canonicalize(&joined) {
        Ok(canon) => {
            let stripped = strip_unc_prefix(&canon);
            if stripped.starts_with(root) {
                stripped
            } else {
                joined
            }
        }
        Err(_) => joined,
    }
}

#[cfg(windows)]
fn strip_unc_prefix(path: &std::path::Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

#[cfg(not(windows))]
fn strip_unc_prefix(path: &std::path::Path) -> PathBuf {
    path.to_path_buf()
}

fn seed_sample_prompts(dir: &std::path::Path) {
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
        let sub = dir.join(cat);
        let _ = std::fs::create_dir_all(&sub);
        let _ = std::fs::write(sub.join(name), content);
    }
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
            let _ = win.show();
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let hotkey = "Ctrl+Alt+P";

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(move |app| {
            let local_dir = resolve_local_dir(app.handle());
            let config_file = resolve_config_file(app.handle());
            let cloud = load_cloud_config(&config_file);

            // 确保本地缓存目录存在
            let _ = std::fs::create_dir_all(&local_dir);

            app.manage(AppState {
                cloud: Mutex::new(cloud.clone()),
                local_dir,
                config_file,
                last_sync: Mutex::new(None),
                last_error: Mutex::new(None),
                syncing: Mutex::new(false),
            });

            // v1.0.1：同步改为纯手动，启动时不再自动拉取

            // 注册全局快捷键
            let shortcut: Shortcut = hotkey.parse().expect("无效的全局快捷键");
            let app_handle = app.handle().clone();
            app.global_shortcut()
                .on_shortcut(shortcut, move |_app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_main_window(&app_handle);
                    }
                })
                .map_err(|e| format!("注册快捷键失败: {e}"))?;

            if let Some(win) = app.get_webview_window("main") {
                #[cfg(debug_assertions)]
                {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
                #[cfg(not(debug_assertions))]
                {
                    let _ = win.hide();
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(false) = event {
                #[cfg(not(debug_assertions))]
                {
                    #[allow(unused_variables)]
                    let _ = window.hide();
                }
                #[cfg(debug_assertions)]
                {
                    let _ = window;
                }
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
            copy_text,
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
