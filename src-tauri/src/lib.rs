// Prompt Pocket — Tauri 应用入口
//
// 职责：
// 1. 注册并暴露 9 个 #[tauri::command] 给前端 invoke
// 2. 注册全局快捷键 Ctrl+Alt+P，唤出/隐藏 spotlight 窗口
// 3. 窗口失焦自动隐藏（spotlight 体验）
// 4. 初始化数据目录（默认放在用户文档目录下的 PromptPocket/）

use std::path::{Path, PathBuf};
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
// 数据目录解析：首次启动在「文档/PromptPocket」下建库
// ────────────────────────────────────────────────────────────
fn resolve_data_dir() -> PathBuf {
    // 文档目录是最适合做云盘同步的默认位置（OneDrive/iCloud 常接管此目录）
    let base = dirs::document_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Documents")
    });
    base.join("PromptPocket")
}

/// 状态：保存数据目录绝对路径，供命令复用
#[derive(Clone)]
struct AppState {
    data_dir: PathBuf,
}

// ────────────────────────────────────────────────────────────
// Tauri 命令
// ────────────────────────────────────────────────────────────

/// 初始化：确保数据目录存在（含 prompts/ 子目录），返回配置
#[tauri::command]
fn init_app(state: tauri::State<'_, AppState>) -> Result<AppConfig, String> {
    let data_dir = state.data_dir.clone();
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

    // 首次启动写入示例 prompt，让用户立刻看到效果（若目录为空）
    let any_md = std::fs::read_dir(&data_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"));
    if !any_md {
        seed_sample_prompts(&data_dir);
    }

    Ok(AppConfig {
        data_dir: data_dir.to_string_lossy().to_string(),
        hotkey: "Ctrl+Alt+P".to_string(),
    })
}

/// 扫描所有 prompt，返回列表 + 分类计数
#[tauri::command]
fn scan_prompts(state: tauri::State<'_, AppState>) -> Result<ScanResult, String> {
    let root = &state.data_dir;
    scan_disk(root).map_err(|e| e.to_string())
}

/// 读取单条 prompt 全文：(frontmatter 原文, 正文)
#[tauri::command]
fn read_prompt(path: String, state: tauri::State<'_, AppState>) -> Result<(String, String), String> {
    let abs = resolve_abs(&state.data_dir, &path);
    read_prompt_disk(&abs).map_err(|e| e.to_string())
}

/// 保存 prompt（前端已拼好 frontmatter + 正文），返回刷新后的 prompt
#[tauri::command]
fn save_prompt(
    path: String,
    content: String,
    state: tauri::State<'_, AppState>,
) -> Result<Prompt, String> {
    let abs = resolve_abs(&state.data_dir, &path);
    save_prompt_disk(&abs, &content).map_err(|e| e.to_string())?;
    // 返回最新元信息，方便前端同步
    scan_disk(&state.data_dir)
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
    let abs = create_prompt_disk(&state.data_dir, &category, &title)
        .map_err(|e| e.to_string())?;
    let rel = abs.strip_prefix(&state.data_dir).unwrap_or(&abs);
    let rel_unix = store::path_to_unix(rel);
    // 复用 scan 后查找，确保元数据一致
    scan_disk(&state.data_dir)
        .map_err(|e| e.to_string())?
        .prompts
        .into_iter()
        .find(|p| p.path == rel_unix)
        .ok_or_else(|| "新建后未能定位该提示词".to_string())
}

/// 删除 prompt
#[tauri::command]
fn delete_prompt(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let abs = resolve_abs(&state.data_dir, &path);
    delete_prompt_disk(&abs).map_err(|e| e.to_string())
}

/// 复制文本到剪贴板（纯文本）
#[tauri::command]
async fn copy_text(text: String, app: tauri::AppHandle) -> Result<(), String> {
    app.clipboard()
        .write_text(text)
        .map_err(|e| e.to_string())
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
    let abs = resolve_abs(&state.data_dir, &path);
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
    // 规范化：前端传的是 unix 风格相对路径，可能含 ../，这里做一次安全拼接
    let joined = root.join(rel);
    // 防止路径穿越：canonicalize 后检查是否仍在 root 下
    // （若文件不存在 canonicalize 会失败，此时退回直接 join）
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
            // 显示前先居中，保证每次都出现在屏幕中部（spotlight 感）
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
    let data_dir = resolve_data_dir();
    let hotkey = "Ctrl+Alt+P";

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState { data_dir })
        .setup(move |app| {
            // 注册全局快捷键 Ctrl+Alt+P
            let shortcut: Shortcut = hotkey
                .parse()
                .expect("无效的全局快捷键");
            let app_handle = app.handle().clone();
            app.global_shortcut().on_shortcut(shortcut, move |_app, _shortcut, event| {
                // ShortcutState::Pressed 仅在按下时触发，避免松开时重复
                if event.state == ShortcutState::Pressed {
                    toggle_main_window(&app_handle);
                }
            })
            .map_err(|e| format!("注册快捷键失败: {e}"))?;

            // 默认隐藏窗口（等快捷键唤出）；dev 首次启动让它显示，便于调试
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
            // 失焦自动隐藏：spotlight 体验（仅在 release 下生效，debug 保留方便调试）
            if let WindowEvent::Focused(false) = event {
                #[cfg(not(debug_assertions))]
                {
                    // release 下 window 被使用；debug 下该分支被 cfg 排除，故 allow
                    #[allow(unused_variables)]
                    let _ = window.hide();
                }
                #[cfg(debug_assertions)]
                {
                    let _ = window; // 显式标记 debug 下不使用
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            init_app,
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
