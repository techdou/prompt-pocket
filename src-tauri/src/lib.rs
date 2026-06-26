// Prompt Pocket — Tauri 应用入口
//
// 职责：
// 1. 注册并暴露 #[tauri::command] 给前端 invoke
// 2. 注册全局快捷键 Ctrl+Alt+P，唤出/隐藏 spotlight 窗口
// 3. 窗口失焦自动隐藏（spotlight 体验）
// 4. 数据目录可配置、持久化（config.json），支持运行时切换（云同步用）

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Manager, WindowEvent};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

mod store;
use crate::store::{
    create_prompt as create_prompt_disk, delete_prompt as delete_prompt_disk,
    read_prompt as read_prompt_disk, save_prompt as save_prompt_disk, scan_prompts as scan_disk,
    AppConfig, Prompt, ScanResult,
};

// ────────────────────────────────────────────────────────────
// 配置持久化：config.json 存在 %APPDATA%/prompt-pocket/ 下
// ────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct PersistedConfig {
    /// 用户选择的数据目录；None 时回退到默认（~/Documents/PromptPocket）
    data_dir: Option<String>,
}

/// 运行时配置：可被设置界面动态修改，用 Mutex 保护
#[derive(Clone)]
struct RuntimeConfig {
    data_dir: PathBuf,
}

struct AppState {
    config: Mutex<RuntimeConfig>,
    config_file: PathBuf,
}

impl AppState {
    /// 读取当前数据目录（加锁）
    fn data_dir(&self) -> PathBuf {
        self.config.lock().unwrap().data_dir.clone()
    }

    /// 修改数据目录并持久化
    fn set_data_dir(&self, new_dir: PathBuf) -> Result<(), String> {
        {
            let mut cfg = self.config.lock().unwrap();
            cfg.data_dir = new_dir.clone();
        }
        self.persist(PersistedConfig {
            data_dir: Some(new_dir.to_string_lossy().to_string()),
        })
    }

    fn persist(&self, cfg: PersistedConfig) -> Result<(), String> {
        if let Some(parent) = self.config_file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
        std::fs::write(&self.config_file, json).map_err(|e| e.to_string())
    }
}

/// 配置文件路径：%APPDATA%/prompt-pocket/config.json
fn resolve_config_file(app: &tauri::AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    dir.join("config.json")
}

/// 默认数据目录：~/Documents/PromptPocket（云盘常接管此目录）
fn default_data_dir() -> PathBuf {
    let base = dirs::document_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Documents")
    });
    base.join("PromptPocket")
}

/// 启动时加载配置：读 config.json，没有则用默认目录
fn load_config(config_file: &Path) -> RuntimeConfig {
    let data_dir = std::fs::read_to_string(config_file)
        .ok()
        .and_then(|s| serde_json::from_str::<PersistedConfig>(&s).ok())
        .and_then(|c| c.data_dir)
        .map(PathBuf::from)
        .unwrap_or_else(default_data_dir);
    RuntimeConfig { data_dir }
}

// ────────────────────────────────────────────────────────────
// Tauri 命令
// ────────────────────────────────────────────────────────────

/// 初始化：确保数据目录存在，首次启动写入示例 prompt，返回配置
#[tauri::command]
fn init_app(state: tauri::State<'_, AppState>) -> Result<AppConfig, String> {
    let data_dir = state.data_dir();
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

    // 首次启动写入示例 prompt（仅当目录里还没有任何 .md）
    let any_md = walkdir_has_md(&data_dir);
    if !any_md {
        seed_sample_prompts(&data_dir);
    }

    Ok(AppConfig {
        data_dir: data_dir.to_string_lossy().to_string(),
        hotkey: "Ctrl+Alt+P".to_string(),
    })
}

fn walkdir_has_md(dir: &Path) -> bool {
    for entry in walkdir::WalkDir::new(dir).min_depth(1).into_iter().flatten() {
        if entry.path().extension().and_then(|e| e.to_str()) == Some("md") {
            return true;
        }
    }
    false
}

/// 读取当前配置（设置界面用）
#[tauri::command]
fn get_config(state: tauri::State<'_, AppState>) -> Result<AppConfig, String> {
    let data_dir = state.data_dir();
    Ok(AppConfig {
        data_dir: data_dir.to_string_lossy().to_string(),
        hotkey: "Ctrl+Alt+P".to_string(),
    })
}

/// 直接设置数据目录（设置界面用）
#[tauri::command]
fn set_data_dir(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<AppConfig, String> {
    let new_dir = PathBuf::from(&path);
    if !new_dir.exists() {
        return Err(format!("目录不存在: {path}"));
    }
    state.set_data_dir(new_dir)?;
    // 确保新目录存在（若用户选了空目录，自动建 prompts 子结构）
    std::fs::create_dir_all(state.data_dir()).map_err(|e| e.to_string())?;
    let data_dir = state.data_dir();
    Ok(AppConfig {
        data_dir: data_dir.to_string_lossy().to_string(),
        hotkey: "Ctrl+Alt+P".to_string(),
    })
}

/// 弹出系统目录选择器，返回用户选的路径（取消则 None）
#[tauri::command]
async fn pick_data_dir(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel::<Option<PathBuf>>();
    app.dialog()
        .file()
        .set_title("选择提示词存储目录")
        .pick_folder(move |path| {
            let p = path.and_then(|p| p.as_path().map(|p| p.to_path_buf()));
            let _ = tx.send(p);
        });
    // pick_folder 是异步回调，这里等待结果
    let result = rx.recv().map_err(|e| e.to_string())?;
    Ok(result.map(|p| p.to_string_lossy().to_string()))
}

/// 在系统文件管理器中打开数据目录
#[tauri::command]
fn open_data_dir(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let dir = state.data_dir();
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 扫描所有 prompt，返回列表 + 分类计数
#[tauri::command]
fn scan_prompts(state: tauri::State<'_, AppState>) -> Result<ScanResult, String> {
    let root = state.data_dir();
    scan_disk(&root).map_err(|e| e.to_string())
}

/// 读取单条 prompt 全文：(frontmatter 原文, 正文)
#[tauri::command]
fn read_prompt(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(String, String), String> {
    let abs = resolve_abs(&state.data_dir(), &path);
    read_prompt_disk(&abs).map_err(|e| e.to_string())
}

/// 保存 prompt（前端已拼好 frontmatter + 正文），返回刷新后的 prompt
#[tauri::command]
fn save_prompt(
    path: String,
    content: String,
    state: tauri::State<'_, AppState>,
) -> Result<Prompt, String> {
    let root = state.data_dir();
    let abs = resolve_abs(&root, &path);
    save_prompt_disk(&abs, &content).map_err(|e| e.to_string())?;
    scan_disk(&root)
        .map_err(|e| e.to_string())?
        .prompts
        .into_iter()
        .find(|p| p.abs_path == abs.to_string_lossy().to_string())
        .ok_or_else(|| "保存后未能重新定位该提示词".to_string())
}

/// 新建 prompt：在指定分类下创建文件，返回新 prompt
#[tauri::command]
fn create_prompt(
    category: String,
    title: String,
    state: tauri::State<'_, AppState>,
) -> Result<Prompt, String> {
    let root = state.data_dir();
    let abs = create_prompt_disk(&root, &category, &title).map_err(|e| e.to_string())?;
    let rel = abs.strip_prefix(&root).unwrap_or(&abs);
    let rel_unix = store::path_to_unix(rel);
    scan_disk(&root)
        .map_err(|e| e.to_string())?
        .prompts
        .into_iter()
        .find(|p| p.path == rel_unix)
        .ok_or_else(|| "新建后未能定位该提示词".to_string())
}

/// 删除 prompt
#[tauri::command]
fn delete_prompt(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let abs = resolve_abs(&state.data_dir(), &path);
    delete_prompt_disk(&abs).map_err(|e| e.to_string())
}

/// 复制文本到剪贴板（纯文本）
#[tauri::command]
async fn copy_text(text: String, app: tauri::AppHandle) -> Result<(), String> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

/// 隐藏主窗口（复制后调用，让用户回到原应用粘贴）
#[tauri::command]
fn hide_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        win.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 在系统文件管理器中显示该文件（便于用 VSCode/Typora 编辑）
#[tauri::command]
fn reveal_in_finder(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let abs = resolve_abs(&state.data_dir(), &path);
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
        let dir = abs.parent().unwrap_or(Path::new("."));
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ────────────────────────────────────────────────────────────
// 辅助：把前端传来的相对路径解析为绝对路径，禁止越界
// ────────────────────────────────────────────────────────────
fn resolve_abs(root: &Path, rel: &str) -> PathBuf {
    let joined = root.join(rel);
    match std::fs::canonicalize(&joined) {
        Ok(canon) => {
            if canon.starts_with(root) {
                canon
            } else {
                joined
            }
        }
        Err(_) => joined,
    }
}

/// 首次启动写入示例 prompt，演示格式与分类
fn seed_sample_prompts(dir: &Path) {
    let samples = [
        (
            "写作",
            "改写润色.md",
            "---\ntitle: 改写润色\ntags: [写作, 润色]\ncopy_mode: markdown\ncreated: 2026-06-27T00:00:00Z\nupdated: 2026-06-27T00:00:00Z\n---\n\n请把下面这段文字改写得更**简洁、专业**：\n\n> 待改写的内容\n\n要求：\n- 保持原意\n- 消除口语化表达\n- 控制在原长度以内\n",
        ),
        (
            "写作",
            "周报模板.md",
            "---\ntitle: 周报模板\ntags: [写作, 工作]\ncopy_mode: markdown\ncreated: 2026-06-27T00:00:00Z\nupdated: 2026-06-27T00:00:00Z\n---\n\n请帮我生成本周工作周报，包含以下要素：\n\n## 本周完成\n- \n\n## 进行中\n- \n\n## 下周计划\n- \n\n## 风险与求助\n- \n",
        ),
        (
            "编程",
            "代码审查.md",
            "---\ntitle: 代码审查\ntags: [编程, review]\ncopy_mode: markdown\ncreated: 2026-06-27T00:00:00Z\nupdated: 2026-06-27T00:00:00Z\n---\n\n请审查以下代码，从这些维度给出改进建议：\n\n1. **可读性**：命名、注释、结构\n2. **正确性**：边界条件、潜在 bug\n3. **性能**：时间/空间复杂度\n4. **安全性**：输入校验、注入风险\n\n```\n// 待审查代码\n```\n",
        ),
    ];

    for (cat, name, content) in samples {
        let sub = dir.join(cat);
        let _ = std::fs::create_dir_all(&sub);
        let _ = std::fs::write(sub.join(name), content);
    }
}

// ────────────────────────────────────────────────────────────
// 窗口控制：快捷键 toggle + 失焦隐藏
// ────────────────────────────────────────────────────────────

fn toggle_main_window(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    match win.is_visible() {
        Ok(true) => {
            let _ = win.hide();
        }
        _ => {
            let _ = win.center();
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
}

// ────────────────────────────────────────────────────────────
// 应用启动
// ────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let hotkey = "Ctrl+Alt+P";

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(move |app| {
            // 加载持久化配置
            let config_file = resolve_config_file(app.handle());
            let runtime_config = load_config(&config_file);
            app.manage(AppState {
                config: Mutex::new(runtime_config),
                config_file,
            });

            // 注册全局快捷键 Ctrl+Alt+P
            let shortcut: Shortcut = hotkey.parse().expect("无效的全局快捷键");
            let app_handle = app.handle().clone();
            app.global_shortcut()
                .on_shortcut(shortcut, move |_app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_main_window(&app_handle);
                    }
                })
                .map_err(|e| format!("注册快捷键失败: {e}"))?;

            // dev 模式显示窗口便于调试；release 启动隐藏等快捷键
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
            get_config,
            set_data_dir,
            pick_data_dir,
            open_data_dir,
            scan_prompts,
            read_prompt,
            save_prompt,
            create_prompt,
            delete_prompt,
            copy_text,
            hide_window,
            reveal_in_finder,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
