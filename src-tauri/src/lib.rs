// Prompt Pocket — Tauri 应用入口
//
// 架构：本地 Markdown + 手动 WebDAV / GitHub 同步
// - UI 读写走本地缓存（store.rs），瞬间响应
// - 本地写入与手动同步互斥，覆盖前保留恢复副本
// - 账号/路径存 config.json，应用密码存系统凭据库

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, LogicalPosition, LogicalSize, Manager, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

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
use crate::sync::{
    push_all_to_remote, CloudConfig, GitHubConfig, GitHubStore, RemoteStore, SyncStatus,
    WebDavStore,
};

const GLOBAL_HOTKEY: &str = "Ctrl+Alt+P";
/// 焦点回归轮询总预算：快路径（实测回归 30-50ms）就绪即返回零延迟；
/// 慢机器/重负载应用（Chromium 系激活 100ms+）需要更宽的兜底窗口——
/// 超时过窄会把可注入场景静默退化成纯复制
const FOCUS_RESTORE_TIMEOUT_MS: u64 = 800;
const FOCUS_RESTORE_POLL_MS: u64 = 10;
/// 首次命中目标前台后的稳定窗口：前台 HWND 已恢复 ≠ 应用内部焦点
/// 已恢复（网页 activeElement / 编辑器 caret / IME 上下文），留一小段
/// 时间让目标应用消化 WM_ACTIVATE 再注入，否则 Ctrl+V 会被过早丢弃
const FOCUS_STABLE_DELAY_MS: u64 = 40;
/// 注入前等待物理修饰键释放的预算：用户 Shift+Enter 调用时 Shift 往往
/// 还按着，直接注入会变成 Ctrl+Shift+V（≠ 粘贴）；等手指松开再注入
const MODIFIER_RELEASE_TIMEOUT_MS: u64 = 400;
const MODIFIER_RELEASE_POLL_MS: u64 = 25;
const CLOUD_PASSWORD_SERVICE: &str = "com.promptpocket.webdav";
const GITHUB_TOKEN_SERVICE: &str = "com.promptpocket.github";

// ────────────────────────────────────────────────────────────
// 配置持久化：config.json 存在 %APPDATA%/prompt-pocket/ 下
// ────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct PersistedConfig {
    username: Option<String>,
    /// 仅作手改配置文件的兜底通道；正常由系统凭据库接管后清出 JSON（不落明文）。
    /// 用 skip_serializing_if 而非 skip_serializing：迁移期间另一后端的明文
    /// 仍处于未迁移状态时，配置重写必须把它带回磁盘——skip_serializing 会
    /// 静默剥掉字段，让后迁移的一侧永远读不到自己的明文（凭据丢失）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    remote_root: Option<String>,
    enabled: Option<bool>,
    /// 旧版配置迁移用：本地目录（v0.x）— 现已弃用
    data_dir: Option<String>,
    // ── GitHub 存档（批次 B2）──
    /// 同步后端："webdav"（缺省）| "github"
    provider: Option<String>,
    gh_repo: Option<String>,
    gh_branch: Option<String>,
    gh_prefix: Option<String>,
    /// 仅作手改配置文件的兜底通道；正常由系统凭据库接管后清出 JSON（不落明文）
    /// 仅作手改配置文件的兜底通道；正常由系统凭据库接管后清出 JSON（不落明文）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gh_token: Option<String>,
}

/// 同步后端选择
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncProvider {
    WebDav,
    GitHub,
}

impl SyncProvider {
    fn parse(s: &str) -> Self {
        match s {
            "github" => Self::GitHub,
            _ => Self::WebDav,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::WebDav => "webdav",
            Self::GitHub => "github",
        }
    }
}

struct AppState {
    cloud: Mutex<CloudConfig>,
    github: Mutex<GitHubConfig>,
    provider: Mutex<SyncProvider>,
    local_dir: PathBuf,
    config_file: PathBuf,
    last_sync: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
    /// 本地写命令与同步命令的互斥闸（见 IoGate）
    io_gate: Mutex<IoGate>,
    /// 当前唤起会话：唤起代次 + 唤起前的前台窗口（粘贴目标），见 PasteSession
    paste_session: Mutex<PasteSession>,
    /// 粘贴请求正在隐藏主窗口：由此产生的失焦事件由请求自己负责收尾，
    /// 失焦处理不要插手（见 hide_and_claim）
    hiding_for_paste: AtomicBool,
    /// 窗口此刻是否持有焦点，由焦点事件原样维护、不区分是哪一次唤起
    window_focused: AtomicBool,
    hotkey: tokio::sync::Mutex<String>,
    preferences_file: PathBuf,
    interaction_locked: AtomicBool,
}

impl AppState {
    fn cloud_config(&self) -> CloudConfig {
        // 毒锁时取出内部数据而非 panic（读操作安全）
        self.cloud.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn paste_session(&self) -> std::sync::MutexGuard<'_, PasteSession> {
        self.paste_session.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 用户主动唤起主窗口（快捷键 / 托盘 / 单实例）：开启新的会话代次并登记
    /// 粘贴目标（托盘 / 单实例唤起没有"唤起前的窗口"，传 None）。代次一变，
    /// 还在途的旧粘贴请求就会作废。返回快照供前端记住"现在是第几次唤起"
    fn begin_paste_session(
        &self,
        app: &tauri::AppHandle,
        target: Option<PasteTarget>,
    ) -> PasteSessionSnapshot {
        // 只把"此刻窗口确实可见并持有焦点"算作已获焦：窗口开着时又唤起一次
        // （单实例、托盘显示），WebView2 不会再发获焦事件，现有的获焦状态要
        // 带进新会话，否则之后的真实失焦会被当成"还没获过焦"而忽略。
        // 从隐藏状态唤起时窗口不可见，即使上一次隐藏留下的失焦事件还没处理
        // （缓存的获焦位尚未清零），也一律从"等待首次获焦"开始。
        //
        // 判定优先直接问系统（Windows：窗口此刻就是前台窗口）。缓存的获焦位
        // 来自 WebView2 的异步回调，拖拽 / 缩放产生的伪失焦可能让它暂时是
        // false，而窗口其实一直在前台；非 Windows 没有等价判据，只能靠它
        let already_focused = app.get_webview_window("main").is_some_and(|win| {
            win.is_visible().unwrap_or(false)
                && (is_in_foreground(&win) || self.window_focused.load(Ordering::SeqCst))
        });
        // 首次启动的失焦豁免只属于启动时那一次展示；之后任何一次主动唤起
        // 都开启新的会话，豁免随之结束
        FIRST_RUN_SUPPRESS_BLUR_HIDE.store(false, Ordering::SeqCst);
        let mut session = self.paste_session();
        session.begin(target, already_focused);
        session.snapshot()
    }

    /// 用户主动收起主窗口（Esc / 快捷键 / 托盘 / 点击了别的窗口）：会话作废，
    /// 目标不残留到下一次唤起，途中的粘贴请求也一并作废——用户既然收起了
    /// 窗口，就不能再有内容被自动粘出去
    fn cancel_paste_session(&self) {
        self.paste_session().cancel();
    }

    fn set_cloud_config(&self, cfg: CloudConfig) -> Result<(), String> {
        let warning = persist_cloud_config(
            &self.config_file,
            &cfg,
            &SystemCloudSecretStore::new(CLOUD_PASSWORD_SERVICE),
        )?;
        {
            *self.cloud.lock().unwrap_or_else(|e| e.into_inner()) = cfg.clone();
        }
        // 降级（密码未持久化）时让前端能通过 sync_status 看到提示
        if let Some(w) = warning {
            *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(w);
        }
        Ok(())
    }

    fn github_config(&self) -> GitHubConfig {
        self.github
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn set_github_config(&self, cfg: GitHubConfig) -> Result<(), String> {
        let warning = persist_github_config(
            &self.config_file,
            &cfg,
            &SystemCloudSecretStore::new(GITHUB_TOKEN_SERVICE),
        )?;
        {
            *self.github.lock().unwrap_or_else(|e| e.into_inner()) = cfg;
        }
        if let Some(w) = warning {
            *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(w);
        }
        Ok(())
    }

    fn provider(&self) -> SyncProvider {
        *self.provider.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_provider(&self, p: SyncProvider) -> Result<(), String> {
        // 合并写：只改 provider 字段，保留两侧配置
        update_persisted_config(&self.config_file, |cfg| {
            cfg.provider = Some(p.as_str().to_string());
        })?;
        *self.provider.lock().unwrap_or_else(|e| e.into_inner()) = p;
        Ok(())
    }

    fn sync_status(&self) -> SyncStatus {
        // 状态跟随当前激活的后端
        let (configured, enabled) = match self.provider() {
            SyncProvider::WebDav => {
                let c = self.cloud_config();
                (c.is_configured(), c.enabled)
            }
            SyncProvider::GitHub => {
                let c = self.github_config();
                (c.is_configured(), c.enabled)
            }
        };
        SyncStatus {
            configured,
            enabled,
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
            syncing: self
                .io_gate
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .syncing,
        }
    }
}

/// 本地写命令与同步命令的互斥状态，由一把锁保护：
/// - 写命令在锁内检查 syncing==false 后**持有锁**做完磁盘写再释放，
///   同步命令想置位 syncing 必须先拿到同一把锁——彻底消除
///   "写命令检查通过 → 同步开始 → 两者互写同一文件"的 check-then-act 竞窗
/// - 同步命令在锁内置位 syncing 后**释放锁**做网络 IO（async 不能持 std 锁跨 await），
///   期间任何写命令进锁都能看到 syncing==true 而拒绝
#[derive(Default)]
struct IoGate {
    syncing: bool,
}

/// 写命令的磁盘 IO 许可：持锁期间同步命令无法启动。
/// 写命令是同步 fn（Tauri 线程池执行），持 std MutexGuard 跨磁盘 IO 是安全的；
/// 本地文件写为毫秒级，同步命令开头至多等待一个写命令的时长。
struct LocalIoPermit<'a> {
    _guard: std::sync::MutexGuard<'a, IoGate>,
}

fn begin_local_io(state: &AppState) -> Result<LocalIoPermit<'_>, String> {
    let guard = state.io_gate.lock().unwrap_or_else(|e| e.into_inner());
    if guard.syncing {
        return Err("SYNC_IN_PROGRESS".to_string());
    }
    Ok(LocalIoPermit { _guard: guard })
}

/// syncing 标志的 RAII 守卫：创建时在单次加锁内完成 check-and-set（消除 TOCTOU），
/// Drop 时自动复位——即使同步过程 panic，也不会永久卡在"同步中"需要重启。
struct SyncGuard<'a> {
    gate: &'a std::sync::Mutex<IoGate>,
}

impl<'a> SyncGuard<'a> {
    fn acquire(gate: &'a std::sync::Mutex<IoGate>) -> Result<Self, String> {
        // 等待在飞的本地写完成（写命令持锁写盘，这里阻塞到它释放），
        // 然后在同一临界区内置位 syncing——之后进来的写命令都会被拒绝。
        // 毒锁恢复（into_inner）：写命令 panic 中断同步不应让后续同步永久失灵
        let mut guard = gate.lock().unwrap_or_else(|e| e.into_inner());
        if guard.syncing {
            return Err("SYNC_IN_PROGRESS".to_string());
        }
        guard.syncing = true;
        drop(guard);
        Ok(Self { gate })
    }
}

impl Drop for SyncGuard<'_> {
    fn drop(&mut self) {
        let mut guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        guard.syncing = false;
    }
}

/// 本地缓存目录：%APPDATA%/com.promptpocket.app/PromptPocket/
fn resolve_local_dir(app: &tauri::AppHandle) -> PathBuf {
    let dir = app.path().app_config_dir().unwrap_or_else(|e| {
        // 极端环境（无家目录）下落回 CWD：记录到 stderr 而非完全静默
        eprintln!("[init] 无法解析系统配置目录，数据将写入当前目录: {e}");
        PathBuf::from(".")
    });
    dir.join("PromptPocket")
}

fn resolve_config_file(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|e| {
            eprintln!("[init] 无法解析系统配置目录，配置将写入当前目录: {e}");
            PathBuf::from(".")
        })
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
            on_ui_thread(app, |app| {
                if is_main_window_visible(app) {
                    // 可见时按快捷键 = 收起
                    hide_main_window(app);
                    return;
                }
                // 隐藏时按快捷键 = 新的一次唤起。粘贴目标必须在窗口显示之前记下：
                // 显示之后前台就是本窗口，再拿不到"唤起前在哪个应用"了
                let target = foreground_window_handle();
                summon_main_window(app, target);
            });
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

/// 启动时加载配置：两侧后端配置 + 当前选中的 provider
fn load_cloud_config(config_file: &std::path::Path) -> (CloudConfig, GitHubConfig, SyncProvider) {
    let webdav = load_cloud_config_with_store(
        config_file,
        &SystemCloudSecretStore::new(CLOUD_PASSWORD_SERVICE),
    );
    let github = load_github_config_with_store(
        config_file,
        &SystemCloudSecretStore::new(GITHUB_TOKEN_SERVICE),
    );
    let provider = std::fs::read_to_string(config_file)
        .ok()
        .and_then(|s| serde_json::from_str::<PersistedConfig>(&s).ok())
        .and_then(|p| p.provider)
        .map(|s| SyncProvider::parse(&s))
        .unwrap_or(SyncProvider::WebDav);
    (webdav, github, provider)
}

/// 系统凭据库读写。service 区分凭据归属（坚果云应用密码 / GitHub PAT 各自独立）
trait CloudSecretStore {
    fn read_password(&self, username: &str) -> Result<Option<String>, String>;
    fn write_password(&self, username: &str, password: &str) -> Result<(), String>;
}

struct SystemCloudSecretStore {
    service: &'static str,
}

impl SystemCloudSecretStore {
    fn new(service: &'static str) -> Self {
        Self { service }
    }
}

impl CloudSecretStore for SystemCloudSecretStore {
    fn read_password(&self, username: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(self.service, username)
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
        let entry = keyring::Entry::new(self.service, username)
            .map_err(|e| format!("打开系统凭据库失败: {e}"))?;
        entry
            .set_password(password)
            .map_err(|e| format!("写入系统凭据失败: {e}"))
    }
}

fn write_persisted_config(config_file: &Path, cfg: &PersistedConfig) -> Result<(), String> {
    if let Some(parent) = config_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    store::write_atomic(config_file, json.as_bytes()).map_err(|e| e.to_string())
}

/// 合并式更新 config.json：读出现有内容、只改指定字段再写回。
/// 两侧后端的配置共存于同一文件——保存坚果云时绝不能把 GitHub 段抹掉，反之亦然
fn update_persisted_config(
    config_file: &Path,
    mutate: impl FnOnce(&mut PersistedConfig),
) -> Result<(), String> {
    let mut cfg = std::fs::read_to_string(config_file)
        .ok()
        .and_then(|s| serde_json::from_str::<PersistedConfig>(&s).ok())
        .unwrap_or_default();
    mutate(&mut cfg);
    write_persisted_config(config_file, &cfg)
}

/// 持久化坚果云配置。返回 Ok(Some(警告)) 表示降级成功（密码未持久化）。
fn persist_cloud_config(
    config_file: &Path,
    cfg: &CloudConfig,
    secret_store: &impl CloudSecretStore,
) -> Result<Option<String>, String> {
    // keyring 不可用（如 Linux 无 secret-service）时降级而非整体失败：
    // 非密字段照常持久化，密码仅留内存（本次会话可用，重启需重填）
    let warning = match secret_store.write_password(&cfg.username, &cfg.password) {
        Ok(()) => None,
        Err(e) => {
            eprintln!("[cloud] 系统凭据库不可用，密码仅本次会话保留: {e}");
            Some("系统凭据库不可用：密码仅本次会话保留，重启后需重新填写".to_string())
        }
    };
    update_persisted_config(config_file, |p| {
        p.username = Some(cfg.username.clone());
        p.password = None;
        p.remote_root = Some(cfg.remote_root.clone());
        p.enabled = Some(cfg.enabled);
    })?;
    Ok(warning)
}

/// 持久化 GitHub 配置（token 进系统凭据库，key 用仓库名）。返回 Ok(Some(警告)) 表降级。
fn persist_github_config(
    config_file: &Path,
    cfg: &GitHubConfig,
    secret_store: &impl CloudSecretStore,
) -> Result<Option<String>, String> {
    let warning = match secret_store.write_password(&cfg.repo, &cfg.token) {
        Ok(()) => None,
        Err(e) => {
            eprintln!("[github] 系统凭据库不可用，PAT 仅本次会话保留: {e}");
            Some("系统凭据库不可用：PAT 仅本次会话保留，重启后需重新填写".to_string())
        }
    };
    update_persisted_config(config_file, |p| {
        p.gh_repo = Some(cfg.repo.clone());
        p.gh_branch = Some(cfg.branch.clone());
        p.gh_prefix = Some(cfg.prefix.clone());
        p.enabled = Some(cfg.enabled);
        p.gh_token = None;
    })?;
    Ok(warning)
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

    // JSON 里的明文密码（旧版遗留或手改）只要凭据库已接管就必须清出：
    // 不能因凭据库已有副本而留着明文不动
    if let Some(legacy) = legacy_password.as_ref() {
        let in_keyring = matches!(&stored_password, Ok(Some(_)));
        let migrated = !in_keyring
            && !username.is_empty()
            && secret_store.write_password(&username, legacy).is_ok();
        if in_keyring || migrated {
            let _ = update_persisted_config(config_file, |p| p.password = None);
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

/// 加载 GitHub 配置：非密字段来自 config.json，token 来自系统凭据库（key = 仓库名）。
/// gh_token 明文兜底仅服务手改配置文件的场景，加载后迁移进凭据库并清出 JSON。
fn load_github_config_with_store(
    config_file: &std::path::Path,
    secret_store: &impl CloudSecretStore,
) -> GitHubConfig {
    let parsed = std::fs::read_to_string(config_file)
        .ok()
        .and_then(|s| serde_json::from_str::<PersistedConfig>(&s).ok());
    let repo = parsed
        .as_ref()
        .and_then(|p| p.gh_repo.clone())
        .unwrap_or_default();
    let legacy_token = parsed.as_ref().and_then(|p| p.gh_token.clone());
    let stored_token = if repo.is_empty() {
        Ok(None)
    } else {
        secret_store.read_password(&repo)
    };
    let token = stored_token
        .as_ref()
        .ok()
        .and_then(|t| t.clone())
        .or_else(|| legacy_token.clone())
        .unwrap_or_default();

    // 手填明文 token 只要凭据库已接管（本就有，或本次迁移成功）就从 JSON 清出
    if let Some(legacy) = legacy_token.as_ref() {
        let in_keyring = matches!(&stored_token, Ok(Some(_)));
        let migrated =
            !in_keyring && !repo.is_empty() && secret_store.write_password(&repo, legacy).is_ok();
        if in_keyring || migrated {
            let _ = update_persisted_config(config_file, |p| p.gh_token = None);
        }
    }

    GitHubConfig {
        repo,
        branch: parsed
            .as_ref()
            .and_then(|p| p.gh_branch.clone())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "main".to_string()),
        prefix: parsed
            .as_ref()
            .and_then(|p| p.gh_prefix.clone())
            .unwrap_or_default(),
        token,
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

/// 同步进行中拒绝本地写：写命令持 IoGate 锁写盘（begin_local_io），
/// 与下载覆盖互斥——两者互写同一文件会互相吞掉（后写赢、无提示）
#[tauri::command]
fn save_prompt(
    path: String,
    req: SaveRequest,
    state: tauri::State<'_, AppState>,
) -> Result<Prompt, String> {
    let _io = begin_local_io(&state)?;
    let abs = resolve_abs(&state.local_dir, &path)?;
    // save_prompt 现在返回新路径（可能因标题重命名而变化）
    let new_abs = save_prompt_disk(&state.local_dir, &abs, &req).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "FILE_NOT_FOUND".to_string()
        } else {
            e.to_string()
        }
    })?;
    // 单文件组装返回值（旧版全量 scan_disk 找单条：一次保存放大成 O(N)
    // 递归遍历+全量解析，且全程持 IoGate 阻塞同步）
    store::build_prompt(&state.local_dir, &new_abs, None)
        .ok_or_else(|| "保存后未能重新定位该提示词".to_string())
}

#[tauri::command]
fn rename_prompt(
    path: String,
    new_title: String,
    new_category: String,
    state: tauri::State<'_, AppState>,
) -> Result<Prompt, String> {
    let _io = begin_local_io(&state)?;
    let old_abs = resolve_abs(&state.local_dir, &path)?;
    let new_abs = rename_prompt_disk(&state.local_dir, &old_abs, &new_title, &new_category)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "FILE_NOT_FOUND".to_string()
            } else {
                e.to_string()
            }
        })?;

    // 单文件组装返回值（同 save_prompt：不做全量扫描）
    store::build_prompt(&state.local_dir, &new_abs, None)
        .ok_or_else(|| "重命名后未能定位该提示词".to_string())
}

#[tauri::command]
fn rename_category(
    old_name: String,
    new_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _io = begin_local_io(&state)?;
    rename_category_disk(&state.local_dir, &old_name, &new_name).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn create_category(name: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _io = begin_local_io(&state)?;
    create_category_disk(&state.local_dir, &name).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn create_prompt(
    category: String,
    title: String,
    state: tauri::State<'_, AppState>,
) -> Result<Prompt, String> {
    let _io = begin_local_io(&state)?;
    let abs = create_prompt_disk(&state.local_dir, &category, &title).map_err(|e| e.to_string())?;
    // 单文件组装返回值（同 save_prompt：不做全量扫描）
    store::build_prompt(&state.local_dir, &abs, None)
        .ok_or_else(|| "新建后未能定位该提示词".to_string())
}

#[tauri::command]
fn delete_prompt(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _io = begin_local_io(&state)?;
    let abs = resolve_abs(&state.local_dir, &path)?;
    delete_prompt_disk(&state.local_dir, &abs).map_err(|e| e.to_string())?;
    Ok(())
}

/// 拖拽排序：重写某分类的顺序到 .order.json（纯本地，手动上传时才同步）
#[tauri::command]
fn reorder(
    category: String,
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _io = begin_local_io(&state)?;
    reorder_category_disk(&state.local_dir, &category, &paths).map_err(|e| e.to_string())?;
    Ok(())
}

/// 分类拖拽排序：重写 .category-order.json
#[tauri::command]
fn reorder_categories(names: Vec<String>, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _io = begin_local_io(&state)?;
    store::save_category_order(&state.local_dir, &names).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_recovery(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<recovery::RecoveryEntry>, String> {
    let _io = begin_local_io(&state)?;
    recovery::list_recovery(&state.local_dir).map_err(|e| e.to_string())
}

#[tauri::command]
fn read_recovery(id: String, state: tauri::State<'_, AppState>) -> Result<String, String> {
    let _io = begin_local_io(&state)?;
    recovery::read_recovery(&state.local_dir, &id).map_err(|e| e.to_string())
}

#[tauri::command]
fn restore_recovery(id: String, state: tauri::State<'_, AppState>) -> Result<String, String> {
    let _io = begin_local_io(&state)?;
    let restored = recovery::restore_recovery(&state.local_dir, &id).map_err(|e| e.to_string())?;
    let rel = relative_library_path(&state.local_dir, &restored)?;
    store::remove_tombstone(&state.local_dir, &rel).map_err(|e| e.to_string())?;
    Ok(rel)
}

#[tauri::command]
async fn copy_text(text: String, app: tauri::AppHandle) -> Result<(), String> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

/// 当前唤起会话的快照：hasTarget 决定 Enter 是"隐藏并注入回原窗口"还是"仅复制"
#[tauri::command]
fn notify_paste_failed(title: String, body: String, app: tauri::AppHandle) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_paste_session(state: tauri::State<'_, AppState>) -> PasteSessionSnapshot {
    state.paste_session().snapshot()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PasteTarget {
    window: isize,
    process_id: u32,
}

/// 一次"唤起会话"：用户主动唤起主窗口时开启。
///
/// 只记窗口句柄不够：同一个窗口可以被反复唤起，只比较句柄分不清"这次请求
/// 属于哪一次唤起"。generation 每次唤起、每次用户收起都递增；粘贴请求记住
/// 自己发起时的代次，之后任何一处发现代次变了就立即作废——否则旧请求会在
/// 用户已经收起窗口之后，才把内容粘回原窗口。
///
/// 只有两种转换会让代次变化：用户唤起（begin）、用户收起（cancel）。
/// 粘贴请求自己隐藏窗口不算收起——那是请求在正常推进，由它认领目标后继续。
///
/// focused 记录"本次唤起的窗口已经获得过焦点"。Windows 上 tauri 的失焦事件来自
/// WebView2 的 LostFocus 回调、经事件循环异步送达，一条上一次隐藏留下的失焦
/// 可能晚于新一次显示才到。新会话在真正获焦之前不响应失焦收起，旧事件就
/// 伤不到它
#[derive(Debug, Default)]
struct PasteSession {
    generation: u64,
    /// 唤起前的前台窗口；托盘 / 单实例唤起、已被认领或会话已作废时为 None
    target: Option<PasteTarget>,
    focused: bool,
}

impl PasteSession {
    /// 新的一次唤起：代次递增，登记本次的粘贴目标。
    ///
    /// window_already_focused：唤起的那一刻窗口是否已经可见并持有焦点（窗口
    /// 开着时又触发了一次单实例 / 托盘显示）。是则把现成的获焦状态带进新会话
    /// ——对已获焦的窗口再 set_focus 不会再产生获焦事件，从 false 开始的话之后
    /// 真实的失焦会被当成"还没获过焦"忽略，窗口不再自动收起。从隐藏状态唤起
    /// 时传 false，等显示之后的首次获焦
    fn begin(&mut self, target: Option<PasteTarget>, window_already_focused: bool) {
        self.generation += 1;
        self.target = target;
        self.focused = window_already_focused;
    }

    /// 用户主动收起窗口：会话作废。代次也递增——途中的粘贴请求必须一起作废，
    /// 否则用户按了 Esc 之后，还没走完的请求仍会把内容粘出去
    fn cancel(&mut self) {
        self.generation += 1;
        self.target = None;
        self.focused = false;
    }

    /// 粘贴请求隐藏了窗口之后认领目标，从此只有它可以注入。只有代次与目标
    /// 都还是它发起时读到的那一份才成功；失败说明期间用户收起过窗口、
    /// 重新唤起过、或目标已被别的请求认领，调用方必须就此停下
    fn claim(&mut self, target: PasteTarget, generation: u64) -> bool {
        if self.generation == generation && self.target == Some(target) {
            self.target = None;
            true
        } else {
            false
        }
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    /// 窗口获得了焦点（本次唤起以来的第一次，或用户点回窗口）
    fn note_focused(&mut self) {
        self.focused = true;
    }

    /// 本次唤起的窗口是否获得过焦点。没有获过焦就收到的失焦，不是本次的
    fn been_focused(&self) -> bool {
        self.focused
    }

    fn snapshot(&self) -> PasteSessionSnapshot {
        PasteSessionSnapshot {
            generation: self.generation,
            has_target: self.target.is_some(),
        }
    }
}

/// 给前端的会话快照。前端把 generation 随粘贴请求带回：用户在第 N 次唤起的
/// 窗口里按的 Enter，即使指令晚到一步，也不会被当成第 N+1 次唤起的请求
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PasteSessionSnapshot {
    generation: u64,
    has_target: bool,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CopyStatus {
    /// 纯复制（无注入目标，或 hide=false 的界面内复制按钮）
    Copied,
    /// 注入指令已完整发出（SendInput 4 事件全部送达系统队列）。
    /// 不代表目标应用已插入——UIPI / 目标忽略快捷键不可观测
    PasteSent,
    /// 自动粘贴失败，剪贴板里已有内容，可手动 Ctrl+V
    PasteFailed { reason: PasteFailReason },
    /// 等待期间发生了新的唤起，本请求作废：未注入，也无需提示
    Superseded,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum PasteFailReason {
    /// 隐藏主窗口失败（极少见）
    HideFailed,
    /// 注入瞬间无法确认前台是目标窗口：等焦点回归超时，或等修饰键期间又切走了
    FocusLost,
    /// 物理修饰键（Ctrl/Alt/Shift/Win）一直未释放，放弃注入防误触
    ModifiersHeld,
    /// SendInput 未完整送出 4 个事件
    InjectFailed,
}

#[derive(Debug, serde::Serialize)]
struct CopyResult {
    /// flatten + 内部 tag：JSON 形状为 {"status":"copied"} /
    /// {"status":"paste_failed","reason":"focus_lost"}，与前端
    /// CopyResult 联合类型逐字段对齐（externally tagged 会嵌套一层
    /// {"status":{"paste_failed":…}}，前端判不出失败）
    #[serde(flatten)]
    status: CopyStatus,
}

/// 粘贴请求隐藏主窗口这一步的结果
enum HidePhase {
    /// 请求已过时（期间用户重新唤起或收起过窗口）：不再碰任何东西
    Superseded,
    /// 窗口没能隐藏：会话保持，用户可以重试
    HideFailed,
    /// 没有粘贴目标：隐藏窗口后纯复制
    Copied,
    /// 已认领目标，接下来等待注入
    Claimed(PasteTarget),
}

/// 在 UI 线程里一口气完成"核对代次 → 隐藏窗口 → 认领目标"，并等它执行完。
///
/// 快捷键、托盘、失焦这些同样会改变会话的处理都跑在 UI 线程上，排不进这一步
/// 的中间：过时的请求不会把新唤起的窗口藏掉，也认领不到别的会话的目标。
/// 之所以必须去 UI 线程：在后台线程调 hide() 只是把隐藏消息排队，实际执行
/// 可以晚于其后的一次唤起。
async fn hide_and_claim(app: tauri::AppHandle, generation: u64) -> HidePhase {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let ui_thread_app = app.clone();
    let dispatched = app.run_on_main_thread(move || {
        let _ = tx.send(hide_and_claim_on_ui_thread(&ui_thread_app, generation));
    });
    if dispatched.is_err() {
        return HidePhase::HideFailed;
    }
    // 闭包没跑（应用正在退出）或中途 panic，接收端都会得到 Err
    rx.await.unwrap_or(HidePhase::HideFailed)
}

/// hide_and_claim 的 UI 线程部分，同步执行
fn hide_and_claim_on_ui_thread(app: &tauri::AppHandle, generation: u64) -> HidePhase {
    let state = app.state::<AppState>();
    let target = {
        let session = state.paste_session();
        if !session.is_current(generation) {
            return HidePhase::Superseded;
        }
        session.target
    };
    // 窗口会因这次隐藏而失焦：告诉失焦处理"这是粘贴请求造成的"，它只负责
    // 用户主动离开的情形，不要在这里结束会话。标志用守卫复位——hide() 万一
    // panic，也不能让失焦处理从此永远被跳过
    struct ResetOnDrop<'a>(&'a AtomicBool);
    impl Drop for ResetOnDrop<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _reset_hiding = ResetOnDrop(&state.hiding_for_paste);
    state.hiding_for_paste.store(true, Ordering::SeqCst);
    let hidden = app
        .get_webview_window("main")
        .is_some_and(|win| win.hide().is_ok());
    drop(_reset_hiding);
    if !hidden {
        return HidePhase::HideFailed;
    }
    let Some(target) = target else {
        return HidePhase::Copied;
    };
    if state.paste_session().claim(target, generation) {
        HidePhase::Claimed(target)
    } else {
        HidePhase::Superseded
    }
}

/// 智能复制/注入：写剪贴板 → 隐藏窗口 → 等焦点回到唤起前的前台窗口 → 注入。
/// - 注入目标来自唤起会话（PasteSession）：快捷键唤起时记下的外部前台窗口。
///   按窗口身份判定而非检测"是否文本输入"——UIA/caret 探测对 Chromium/WinUI3/
///   Electron 应用恒 false，会把可注入场景误判为纯复制
/// - 无目标（托盘/单实例唤起、非 Windows）：隐藏窗口后纯复制
/// - hide=false（主窗口内按钮触发）：只写剪贴板，窗口保持可见，也不注入——
///   焦点还在本窗口，注入会粘错地方；且不触碰会话
/// - 请求绑定发起时的会话代次（generation 由前端带回）：此后只要用户重新
///   唤起或收起过窗口，请求立即作废（Superseded）——既不会把新唤起的窗口
///   藏起来，也不会在用户收起之后才把内容粘出去
///
/// 注入链路（有目标时）：UI 线程里核对代次 + 隐藏 + 认领 → 等前台回归
/// （≤800ms，首次命中后 +40ms 稳定复核）→ 等物理修饰键释放（≤400ms，防
/// Shift+Enter 注成 Ctrl+Shift+V）→ 持会话锁做最终核对并原子注入。
///
/// mode 参数当前未区分转换（前端传 editingBody 原文），保留以兼容现有调用契约。
#[tauri::command]
async fn copy_or_paste(
    text: String,
    mode: String,
    hide: Option<bool>,
    generation: Option<u64>,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<CopyResult, String> {
    let _ = mode;
    if !hide.unwrap_or(true) {
        // 界面内的纯复制（以及快照未到时的退化路径）不涉及会话：
        // 不需要代次，也不核对
        app.clipboard()
            .write_text(&text)
            .map_err(|e| e.to_string())?;
        return Ok(CopyResult {
            status: CopyStatus::Copied,
        });
    }
    // 会收起窗口的请求必须说明自己属于哪一次唤起。说不清（没带代次）的一律
    // 不作数——否则旧窗口里按的 Enter 晚到一步，就会认领到新一次唤起的目标
    let Some(generation) = generation else {
        return Ok(CopyResult {
            status: CopyStatus::Superseded,
        });
    };
    // 代次对不上：按下 Enter 之后用户已经重新唤起或收起过，这次请求只属于上一次
    if !state.paste_session().is_current(generation) {
        return Ok(CopyResult {
            status: CopyStatus::Superseded,
        });
    }
    let superseded = || !state.paste_session().is_current(generation);
    app.clipboard()
        .write_text(&text)
        .map_err(|e| e.to_string())?;
    let target = match hide_and_claim(app.clone(), generation).await {
        HidePhase::Superseded => {
            return Ok(CopyResult {
                status: CopyStatus::Superseded,
            })
        }
        HidePhase::HideFailed => {
            // 窗口没藏起来：会话未结束，目标保留供用户重试。有没有目标都按
            // hide_failed 报——用户需要知道"窗口没按预期收起"
            eprintln!("[粘贴] 隐藏主窗口失败");
            return Ok(CopyResult {
                status: CopyStatus::PasteFailed {
                    reason: PasteFailReason::HideFailed,
                },
            });
        }
        HidePhase::Copied => {
            return Ok(CopyResult {
                status: CopyStatus::Copied,
            })
        }
        HidePhase::Claimed(target) => target,
    };
    let focus_returned = wait_for_foreground_window(
        target,
        std::time::Duration::from_millis(FOCUS_RESTORE_TIMEOUT_MS),
        std::time::Duration::from_millis(FOCUS_RESTORE_POLL_MS),
        &superseded,
    )
    .await;
    let Some(focus_returned) = focus_returned else {
        return Ok(CopyResult {
            status: CopyStatus::Superseded,
        });
    };
    let modifiers_released = if focus_returned {
        wait_for_modifier_release(
            std::time::Duration::from_millis(MODIFIER_RELEASE_TIMEOUT_MS),
            std::time::Duration::from_millis(MODIFIER_RELEASE_POLL_MS),
            &superseded,
        )
        .await
    } else {
        Some(false)
    };
    let Some(modifiers_released) = modifiers_released else {
        return Ok(CopyResult {
            status: CopyStatus::Superseded,
        });
    };
    // 最终提交：核对代次、核对前台与修饰键、发出按键，全程持有会话锁。新的
    // 唤起要么排在这之前（被 is_current 发现，请求作废），只能排在这之后——
    // 不会夹在"核对完"和"发出去"之间
    let mut inject_error = None;
    let (session_current, focus_on_target, modifiers_ok, status) = {
        let session = state.paste_session();
        let session_current = session.is_current(generation);
        // SendInput 无法定向到 HWND，只能发给此刻的前台窗口，所以注入前最后
        // 核对一次：等修饰键期间用户切走了就放弃——宁可让用户手动 Ctrl+V，
        // 也不粘进别的窗口。修饰键同样紧贴注入再查一次
        let focus_on_target = foreground_is_target_window(target);
        let modifiers_ok = modifiers_released && physical_modifiers_released();
        let status = final_verdict(
            session_current,
            focus_returned,
            modifiers_ok,
            focus_on_target,
            // 锁内只记下错误文本，日志到锁外面再写
            || {
                simulate_paste().map_err(|e| {
                    inject_error = Some(e.to_string());
                    e.to_string()
                })
            },
        );
        (session_current, focus_on_target, modifiers_ok, status)
    };
    // 诊断日志（tauri dev 控制台可见）：只记原因与预算，不记提示词内容
    if let Some(error) = inject_error {
        eprintln!("[粘贴] {error}");
    } else if !session_current {
        eprintln!("[粘贴] 等待期间用户重新唤起或收起了窗口，本请求作废");
    } else if !focus_returned {
        eprintln!("[粘贴] 焦点未在 {FOCUS_RESTORE_TIMEOUT_MS}ms 内回到目标窗口，未注入");
    } else if !modifiers_released {
        eprintln!("[粘贴] 物理修饰键 {MODIFIER_RELEASE_TIMEOUT_MS}ms 内未释放，未注入");
    } else if !modifiers_ok {
        // 等待时松开了、注入前又被按下（比如用户开始按下一次快捷键）
        eprintln!("[粘贴] 注入前修饰键又被按下，未注入");
    } else if !focus_on_target {
        eprintln!("[粘贴] 等待修饰键期间前台窗口已切换，未注入");
    }
    Ok(CopyResult { status })
}

/// 注入前的最终裁定（纯函数，覆盖真实流程里窗口隐藏之后的全部归因）。
/// 按发生顺序短路：请求已过时 → 焦点没回来 → 修饰键没松 → 注入瞬间前台已不是
/// 目标 → 才真正注入。"已过时"排在最前：用户都已经重新操作了，这个请求的
/// 超时与否不值得再提示
fn final_verdict(
    session_current: bool,
    focus_returned: bool,
    modifiers_released: bool,
    focus_on_target: bool,
    inject: impl FnOnce() -> Result<(), String>,
) -> CopyStatus {
    if !session_current {
        return CopyStatus::Superseded;
    }
    if !focus_returned {
        return CopyStatus::PasteFailed {
            reason: PasteFailReason::FocusLost,
        };
    }
    if !modifiers_released {
        return CopyStatus::PasteFailed {
            reason: PasteFailReason::ModifiersHeld,
        };
    }
    if !focus_on_target {
        return CopyStatus::PasteFailed {
            reason: PasteFailReason::FocusLost,
        };
    }
    match inject() {
        Ok(()) => CopyStatus::PasteSent,
        Err(_) => CopyStatus::PasteFailed {
            reason: PasteFailReason::InjectFailed,
        },
    }
}

/// 轮询等待系统前台窗口回到 target（或其子窗口）。
/// - 首次命中后等满 FOCUS_STABLE_DELAY_MS 再复核一次：前台 HWND 恢复只代表
///   窗口激活，应用内部焦点（网页 activeElement / 编辑器 caret）可能尚未
///   就绪；复核失败回到轮询，避免注入过早被目标丢弃
/// - 剩余预算不够一次完整复核时按超时处理，不拿缩短的复核冒充成功
/// - superseded() 为 true 表示等待期间发生了新的唤起：立即放弃，返回 None
async fn wait_for_foreground_window(
    target: PasteTarget,
    timeout: std::time::Duration,
    poll: std::time::Duration,
    superseded: impl Fn() -> bool,
) -> Option<bool> {
    let deadline = std::time::Instant::now() + timeout;
    let stable = std::time::Duration::from_millis(FOCUS_STABLE_DELAY_MS);
    loop {
        if superseded() {
            return None;
        }
        let now = std::time::Instant::now();
        if foreground_is_target_window(target) {
            if deadline.saturating_duration_since(now) < stable {
                return Some(false);
            }
            tokio::time::sleep(stable).await;
            if superseded() {
                return None;
            }
            // 复核通过且没有因调度延迟越过截止时间才算恢复
            if foreground_is_target_window(target) && std::time::Instant::now() <= deadline {
                return Some(true);
            }
            // 复核失败（前台闪到别的窗口又回来）：继续轮询，不重置计时
        } else if now >= deadline {
            return Some(false);
        }
        tokio::time::sleep(
            deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(poll),
        )
        .await;
    }
}

/// 取当前系统前台窗口句柄；无有效前台窗口（罕见）返回 None。
#[cfg(windows)]
fn foreground_window_handle() -> Option<PasteTarget> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return None;
    }
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    (process_id != 0).then_some(PasteTarget {
        window: hwnd.0 as isize,
        process_id,
    })
}

#[cfg(not(windows))]
fn foreground_window_handle() -> Option<PasteTarget> {
    None
}

/// 前台是否为目标窗口（或其子窗口——焦点回归时前台可能是目标应用的
/// 弹出子窗口，如输入法候选窗、悬浮面板）。
#[cfg(windows)]
fn foreground_is_target_window(target: PasteTarget) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsChild,
    };
    let expected_pid = target.process_id;
    let target = HWND(target.window as *mut _);
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_invalid() || target.is_invalid() {
        return false;
    }
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(target, Some(&mut process_id));
    }
    if process_id != expected_pid {
        return false;
    }
    let is_same = fg == target;
    let is_child = unsafe { IsChild(target, fg) }.as_bool();
    foreground_matches_window(is_same, is_child)
}

#[cfg(not(windows))]
fn foreground_is_target_window(_target: PasteTarget) -> bool {
    false
}

/// 判断指定 Tauri 窗口当前是否为 Win32 前台窗口。
///
/// 用途：拖拽 / 缩放窗口时，WebView 可能误报 `Focused(false)`，
/// 但此时主窗口仍处于系统的 move/size 模态、仍是 GetForegroundWindow() 的结果。
/// 这种情况下不应该 hide，否则窗口会"闪一下消失"。
#[cfg(windows)]
fn is_window_in_foreground(win: &tauri::Window) -> bool {
    // hwnd() 在 Windows 上返回 Result<HWND>，Err 说明窗口未就绪
    win.hwnd().is_ok_and(hwnd_is_foreground)
}

/// 该窗口句柄（或其子窗口）当前是否为系统前台窗口
#[cfg(windows)]
fn hwnd_is_foreground(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsChild};

    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_invalid() {
        return false;
    }
    let is_same = foreground == hwnd;
    let is_child = unsafe { IsChild(hwnd, foreground).as_bool() };
    foreground_matches_window(is_same, is_child)
}

/// 主窗口此刻是否就是系统前台窗口。开启新会话时用它判断"窗口是否已经获焦"：
/// 这是不受 WebView 异步焦点回调影响的实时判据——拖拽 / 缩放造成的伪失焦
/// 可能让缓存的获焦位暂时是 false，而窗口其实一直在前台。
/// 非 Windows 没有等价的可靠判据，恒为 false（调用方会回退到缓存的获焦位）
fn is_in_foreground(win: &tauri::WebviewWindow) -> bool {
    #[cfg(windows)]
    {
        win.hwnd().is_ok_and(hwnd_is_foreground)
    }
    #[cfg(not(windows))]
    {
        let _ = win;
        false
    }
}

#[cfg(not(windows))]
fn is_window_in_foreground(_win: &tauri::Window) -> bool {
    false
}

#[cfg(any(windows, test))]
fn foreground_matches_window(is_same_window: bool, is_child_window: bool) -> bool {
    is_same_window || is_child_window
}

/// Windows 键盘注入（Win32 SendInput）。
///
/// 独立成模块是为了让"发出去的到底是什么事件"成为可单测的纯函数：
/// 事件 type 填错时 SendInput 照样返回成功，运行期根本发现不了。
///
/// 为什么不用 enigo 的 `Key::Unicode('v')`：它映射成功时强制附加
/// KEYEVENTF_SCANCODE（按前台窗口的键盘布局解释扫描码），映射失败时回退
/// KEYEVENTF_UNICODE / VK_PACKET（多数应用不认 Ctrl+VK_PACKET）——两种情况
/// 都会被部分应用整次吞掉。这里只发显式 VK_V 虚拟键码。
#[cfg(target_os = "windows")]
mod paste_input {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VIRTUAL_KEY, VK_CONTROL, VK_V,
    };

    /// 一个键盘事件。INPUT_KEYBOARD = 1（INPUT_MOUSE 才是 0）：类型错填会被
    /// 系统按鼠标事件解释，返回值照样是 4，但按键从未发生
    fn keyboard_input(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// 一次 Ctrl+V 的完整序列：Ctrl↓ V↓ V↑ Ctrl↑
    pub(super) fn paste_events() -> [INPUT; 4] {
        [
            keyboard_input(VK_CONTROL, KEYBD_EVENT_FLAGS::default()),
            keyboard_input(VK_V, KEYBD_EVENT_FLAGS::default()),
            keyboard_input(VK_V, KEYEVENTF_KEYUP),
            keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
        ]
    }

    /// 提交整批按键并按送达数善后。`send` 可替换，便于单测 0~4 各种送达数。
    ///
    /// 返回值 < 4 说明有事件没进系统队列（典型是 UIPI：目标进程权限更高）：
    /// - 0 个送达：没有键被按下，直接报错
    /// - 部分送达：已送达的是数组前缀，补发其中还没抬起的键。先放 Ctrl 再放 V
    ///   ——Ctrl 卡在按下态的后果（点选变多选、滚轮变缩放）比 V 严重
    pub(super) fn dispatch(send: impl Fn(&[INPUT]) -> u32) -> Result<(), String> {
        let events = paste_events();
        let sent = send(&events) as usize;
        if sent == events.len() {
            return Ok(());
        }
        if sent == 0 {
            return Err("SendInput 0/4：注入被系统拒绝（目标进程权限更高？）".into());
        }
        let release = [
            keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
            keyboard_input(VK_V, KEYEVENTF_KEYUP),
        ];
        // 已送达的是数组前缀，按其中还按着的键补发释放：
        // 1 个 = 只有 Ctrl↓；2 个 = Ctrl↓ V↓；3 个 = V 已抬起，只差 Ctrl↑
        let to_release = if sent == 2 { 2 } else { 1 };
        let released = send(&release[..to_release]) as usize;
        Err(format!(
            "SendInput 仅送达 {sent}/{} 个事件，补发释放 {released}/{to_release}",
            events.len()
        ))
    }

    /// 在同一次 SendInput 调用里提交整个序列（原子）：分批注入会被目标应用的
    /// 消息循环 / 全局钩子交错，出现"只按了 Ctrl"的半序列
    pub(super) fn send_paste() -> Result<(), String> {
        let size = std::mem::size_of::<INPUT>() as i32;
        dispatch(|batch| unsafe { SendInput(batch, size) })
    }
}

#[cfg(target_os = "windows")]
fn simulate_paste() -> Result<(), Box<dyn std::error::Error>> {
    paste_input::send_paste().map_err(Into::into)
}

/// 非 Windows 平台不可达：foreground_window_handle 恒返回 None，
/// copy_or_paste 永远走纯复制分支。保留签名让主流程无条件编译。
#[cfg(not(target_os = "windows"))]
fn simulate_paste() -> Result<(), Box<dyn std::error::Error>> {
    Err("自动粘贴仅支持 Windows".into())
}

/// 物理修饰键（Ctrl/Alt/Shift/Win）当前是否全部处于释放态。
/// 用 GetAsyncKeyState 而非消息队列：注入前的物理键状态只有异步状态表知道。
#[cfg(target_os = "windows")]
fn physical_modifiers_released() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    let pressed = |vk: i32| unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 };
    !(pressed(VK_CONTROL.0 as i32)
        || pressed(VK_MENU.0 as i32)
        || pressed(VK_SHIFT.0 as i32)
        || pressed(VK_LWIN.0 as i32)
        || pressed(VK_RWIN.0 as i32))
}

#[cfg(not(target_os = "windows"))]
fn physical_modifiers_released() -> bool {
    true
}

/// 注入前等待物理修饰键释放：Shift+Enter 调用时 Shift 常仍按着，
/// 直接注入会变成 Ctrl+Shift+V（部分应用 = 无格式粘贴，部分 = 无动作）。
/// 超时放弃注入，绝不替用户释放物理键；超过截止时间才松开的不算数。
/// superseded() 为 true 表示等待期间发生了新的唤起：立即放弃，返回 None。
async fn wait_for_modifier_release(
    timeout: std::time::Duration,
    poll: std::time::Duration,
    superseded: impl Fn() -> bool,
) -> Option<bool> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if superseded() {
            return None;
        }
        if std::time::Instant::now() >= deadline {
            return Some(false);
        }
        if physical_modifiers_released() {
            return Some(true);
        }
        tokio::time::sleep(
            deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(poll),
        )
        .await;
    }
}

#[tauri::command]
fn hide_window(generation: Option<u64>, app: tauri::AppHandle) -> Result<(), String> {
    // Esc 必须说明自己是哪一次唤起里按下的，说不清的不执行：一次晚到的旧 Esc
    // 不能把现在这一次唤起的窗口收起
    let state = app.state::<AppState>();
    if !generation.is_some_and(|seen| state.paste_session().is_current(seen)) {
        return Ok(());
    }
    if let Some(win) = app.get_webview_window("main") {
        win.hide().map_err(|e| e.to_string())?;
        // Esc 收起 = 用户主动结束会话：清掉粘贴目标（下次托盘/单实例唤起不得
        // 把内容注进早已不相关的旧窗口），途中的粘贴请求一并作废
        state.cancel_paste_session();
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
// 开机自启动：开关状态读写系统真实状态（注册表 Run 键 / LaunchAgent），
// 不落 config.json——避免配置文件与系统状态双源打架
// ────────────────────────────────────────────────────────────

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())
    }
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
    let gh = state.github_config();
    // 两侧配置一起下发：前端切换后端或回显表单时不丢另一侧的已存配置。
    // 密钥永远不下发——hasPassword/hasToken 只告诉前端"有没有"，用于占位符显示
    serde_json::json!({
        "provider": state.provider().as_str(),
        "username": cfg.username,
        "remoteRoot": cfg.remote_root,
        "enabled": cfg.enabled,
        "hasPassword": !cfg.password.is_empty(),
        "ghRepo": gh.repo,
        "ghBranch": gh.branch,
        "ghPrefix": gh.prefix,
        "ghEnabled": gh.enabled,
        "hasToken": !gh.token.is_empty()
    })
}

#[tauri::command]
async fn test_cloud_connection(
    username: String,
    password: String,
    remote_root: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // __KEEP__：已配置用户密码不回显，测试连接复用已存值
    // （与 save_cloud_config 同语义，否则"测试连接"要求重输密码）
    let password = if password == "__KEEP__" {
        state.cloud_config().password
    } else {
        password
    };
    let cfg = CloudConfig {
        username,
        password,
        remote_root,
        enabled: true,
    };
    WebDavStore::new(&cfg)?.test().await
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

#[tauri::command]
async fn test_github_connection(
    repo: String,
    token: String,
    branch: String,
    prefix: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // __KEEP__：已配置用户 PAT 不回显，测试连接复用已存值
    // （与 save_github_config 同语义）
    let token = if token == "__KEEP__" {
        state.github_config().token
    } else {
        token
    };
    let cfg = GitHubConfig {
        repo,
        branch,
        prefix,
        token,
        enabled: true,
    };
    GitHubStore::new(&cfg)?.test().await
}

#[tauri::command]
fn save_github_config(
    repo: String,
    token: String,
    branch: String,
    prefix: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // __KEEP__ 占位符表示保留旧 PAT（用户未重新填写）
    let final_token = if token == "__KEEP__" {
        state.github_config().token
    } else {
        token
    };

    let cfg = GitHubConfig {
        repo,
        branch,
        prefix,
        token: final_token,
        enabled: true,
    };
    // 落盘前先校验（repo 形态 / PAT 非法字符），不构造网络请求
    GitHubStore::new(&cfg)?;
    state.set_github_config(cfg)?;
    Ok(())
}

/// 切换同步后端（"webdav" | "github"）。只改激活标记，两侧配置都保留
#[tauri::command]
fn set_sync_provider(provider: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    match provider.as_str() {
        "webdav" | "github" => state.set_provider(SyncProvider::parse(&provider)),
        _ => Err(format!("未知同步后端: {provider}")),
    }
}

/// 全量上传：本地所有文件推送到当前后端（坚果云 / GitHub）+ 删除传播（tombstone）
#[tauri::command]
async fn upload_all(app: tauri::AppHandle) -> Result<sync::SyncReport, String> {
    let state = app.state::<AppState>();
    let provider = state.provider();
    match provider {
        SyncProvider::WebDav if !state.cloud_config().is_configured() => {
            return Err("SYNC_NOT_CONFIGURED".to_string());
        }
        SyncProvider::GitHub if !state.github_config().is_configured() => {
            return Err("SYNC_NOT_CONFIGURED".to_string());
        }
        _ => {}
    }
    // 并发保护：IoGate 单锁互斥（等待在飞的本地写完成后置位 syncing）；
    // guard drop 自动复位（panic 也不卡死）
    let _guard = SyncGuard::acquire(&state.io_gate)?;
    let result = match provider {
        SyncProvider::WebDav => {
            let cfg = state.cloud_config();
            let store = WebDavStore::new(&cfg)?;
            let target = sync::target_key_webdav(&cfg);
            push_all_to_remote(&store, &state.local_dir, &target).await
        }
        SyncProvider::GitHub => {
            let cfg = state.github_config();
            let store = GitHubStore::new(&cfg)?;
            let target = sync::target_key_github(&cfg);
            push_all_to_remote(&store, &state.local_dir, &target).await
        }
    };
    match result {
        Ok(report) => {
            // 文案拼装在前端 i18n（英文界面不得穿帮中文）；last_sync 同理存码
            if !report.errors.is_empty() {
                *state.last_error.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(report.errors.join("; "));
            } else {
                *state.last_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
            *state.last_sync.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(format!("SYNC_UPLOADED:{}", report.uploaded));
            let _ = app.emit("sync-finished", ());
            Ok(report)
        }
        Err(e) => {
            *state.last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(e.clone());
            Err(e)
        }
    }
}

/// 全量下载：从当前后端拉取并覆盖本地（覆盖前备份 .trash，tombstone 防复活）
#[tauri::command]
async fn download_all(app: tauri::AppHandle) -> Result<sync::SyncReport, String> {
    let state = app.state::<AppState>();
    let provider = state.provider();
    match provider {
        SyncProvider::WebDav if !state.cloud_config().is_configured() => {
            return Err("SYNC_NOT_CONFIGURED".to_string());
        }
        SyncProvider::GitHub if !state.github_config().is_configured() => {
            return Err("SYNC_NOT_CONFIGURED".to_string());
        }
        _ => {}
    }
    // 并发保护：IoGate 单锁互斥；guard drop 自动复位
    let _guard = SyncGuard::acquire(&state.io_gate)?;
    let result = match provider {
        SyncProvider::WebDav => {
            let cfg = state.cloud_config();
            let store = WebDavStore::new(&cfg)?;
            let target = sync::target_key_webdav(&cfg);
            sync::pull_from_remote(&store, &state.local_dir, &target).await
        }
        SyncProvider::GitHub => {
            let cfg = state.github_config();
            let store = GitHubStore::new(&cfg)?;
            let target = sync::target_key_github(&cfg);
            sync::pull_from_remote(&store, &state.local_dir, &target).await
        }
    };
    match result {
        Ok(report) => {
            // 文案拼装在前端 i18n；last_sync 存码由前端翻译展示
            if !report.errors.is_empty() {
                *state.last_error.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(report.errors.join("; "));
            } else {
                *state.last_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
            *state.last_sync.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!(
                "SYNC_DOWNLOADED:{}/{}/{}",
                report.downloaded, report.skipped, report.deleted
            ));
            let _ = app.emit("sync-finished", ());
            Ok(report)
        }
        Err(e) => {
            *state.last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(e.clone());
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

/// 把 root.join(rel) 解析为绝对路径，并校验不逃逸 root。
/// 两条防线：
/// 1. 路径已存在 → canonicalize（解析符号链接/`..`）后校验前缀
/// 2. 路径不存在（新建文件场景）→ 手工展开 `.`/`..`（不依赖文件系统）后校验前缀
///
/// 注意顺序：root 先规范化到 canonical 形式，再 join rel。
/// 若反过来（先 join 再分别规范化），当 rel 目标不存在而 root 存在时，
/// 两边会拿到不同形式（macOS /var vs /private/var、Windows 短文件名），
/// 前缀校验会误判合法路径为越界——CI 上实测踩过。
///
/// 任何逃逸都返回 Err——旧版校验失败会回退到未校验的路径，等于没有防护：
/// `read_prompt("../../../etc/passwd")` 可读 root 外文件，delete 可删任意文件。
fn resolve_abs(root: &Path, rel: &str) -> Result<PathBuf, String> {
    recovery::checked_path(root, rel).map_err(|e| e.to_string())
}

#[cfg(test)]
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

#[cfg(all(windows, test))]
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

#[cfg(all(not(windows), test))]
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
        let sub = dir.join(cat);
        std::fs::create_dir_all(&sub).map_err(|e| format!("创建示例分类失败: {e}"))?;
        store::write_atomic(&sub.join(name), content.as_bytes())
            .map_err(|e| format!("写入示例文件失败: {e}"))?;
    }
    Ok(())
}

/// 把"唤起 / 收起"的判定与执行放到 UI 线程。各平台回调快捷键、单实例的线程
/// 并不相同（Windows 上两者都在 UI 线程；Linux 的快捷键、Linux / macOS 的
/// 单实例不在），统一排到这里，会话变更与窗口显隐才不会互相交错。
/// 调用方已在 UI 线程时立即执行，没有延迟
fn on_ui_thread(app: &tauri::AppHandle, task: impl FnOnce(&tauri::AppHandle) + Send + 'static) {
    let task_app = app.clone();
    let _ = app.run_on_main_thread(move || task(&task_app));
}

/// 唤起主窗口：开启新的唤起会话 → 显示 + 聚焦 + 通知前端（前端把焦点放进
/// 搜索框，兑现「唤出→直接打字」的核心链路；普通点击窗口获得焦点不发此事件）。
///
/// target 是唤起前的前台窗口（快捷键唤起时记录）；托盘 / 单实例唤起传 None。
/// 开启新会话的同时，上一轮还在等待的粘贴请求就此作废
fn show_main_window(app: &tauri::AppHandle, target: Option<PasteTarget>) {
    let snapshot = app.state::<AppState>().begin_paste_session(app, target);
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let _ = win.show();
    let _ = win.set_focus();
    // 快照随事件带给前端：这次唤起的代次（之后发起粘贴请求时带回）与
    // "Enter 会粘贴还是只复制"
    let _ = app.emit("window-shown", snapshot);
}

/// 快捷键、托盘左键的唤起：先定位到鼠标所在屏幕再显示
fn summon_main_window(app: &tauri::AppHandle, target: Option<PasteTarget>) {
    // 多屏跟随鼠标定位：找到鼠标所在的显示器，在该屏居中显示
    if let Some(win) = app.get_webview_window("main") {
        position_window_at_cursor(&win);
    }
    show_main_window(app, target);
}

/// 收起主窗口：隐藏成功才结束当前唤起会话。失败时窗口仍可见、会话保持，
/// 用户可以重试——不能在还不知道结果的时候就先把粘贴目标清掉
fn hide_main_window(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if win.hide().is_err() {
        eprintln!("[窗口] 隐藏主窗口失败");
        return;
    }
    app.state::<AppState>().cancel_paste_session();
}

fn is_main_window_visible(app: &tauri::AppHandle) -> bool {
    app.get_webview_window("main")
        .is_some_and(|win| win.is_visible().unwrap_or(false))
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
            // 用户再次双击 exe 时走到这里：聚焦到已有窗口。
            // 双击唤起没有"唤起前前台窗口"语义：不开粘贴目标（Enter 只纯复制）。
            // Windows 上这个回调就在 UI 线程，Linux / macOS 上不在：统一交给
            // on_ui_thread，两种情况都排进同一顺序
            on_ui_thread(app, |app| show_main_window(app, None));
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        // 自动粘贴失败时发系统通知：主窗口已隐藏，应用内 toast 用户看不到
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // 开机自启动：Windows 写注册表 Run 键 / macOS LaunchAgent；参数 None = 不带参数启动
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(move |app| {
            let local_dir = resolve_local_dir(app.handle());
            let config_file = resolve_config_file(app.handle());
            let (cloud, github, provider) = load_cloud_config(&config_file);
            let preferences_file = config_file.with_file_name("preferences.json");
            let hotkey = load_hotkey(&preferences_file);

            // 确保本地缓存目录存在
            let _ = std::fs::create_dir_all(&local_dir);

            app.manage(AppState {
                cloud: Mutex::new(cloud),
                github: Mutex::new(github),
                provider: Mutex::new(provider),
                local_dir: local_dir.clone(),
                config_file,
                last_sync: Mutex::new(None),
                last_error: Mutex::new(None),
                io_gate: Mutex::new(IoGate::default()),
                paste_session: Mutex::new(PasteSession::default()),
                hiding_for_paste: AtomicBool::new(false),
                window_focused: AtomicBool::new(false),
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
            let show_item = MenuItem::with_id(app, "show", "显示主界面 · Show", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出 · Quit", true, None::<&str>)?;
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
                        // 左键单击切换窗口显隐。显隐只在这里判定一次：收起要等
                        // 隐藏成功才结束会话，唤起才登记新会话（托盘唤起无原窗口）
                        on_ui_thread(tray.app_handle(), |app| {
                            if is_main_window_visible(app) {
                                hide_main_window(app);
                            } else {
                                summon_main_window(app, None);
                            }
                        });
                    }
                })
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "show" => {
                            on_ui_thread(app, |app| show_main_window(app, None));
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
                    // 首启聚焦由前端 bootstrap 完成后自行处理：此刻 webview 的
                    // JS 尚未执行、事件监听未注册，emit window-shown 必丢
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
            let state = window.app_handle().state::<AppState>();
            match event {
                WindowEvent::Focused(true) => {
                    state.window_focused.store(true, Ordering::SeqCst);
                    // 窗口可见时的获焦，才算"本次唤起的窗口已经获焦"
                    if window.is_visible().unwrap_or(false) {
                        state.paste_session().note_focused();
                    }
                }
                WindowEvent::Focused(false) => {
                    if state.interaction_locked.load(Ordering::SeqCst) {
                        return;
                    }
                    // 粘贴请求正在隐藏窗口：这次失焦是它自己造成的，会话由它
                    // 接着认领，这里不插手
                    if state.hiding_for_paste.load(Ordering::SeqCst) {
                        return;
                    }
                    // 本次唤起的窗口还没获得过焦点：这条失焦是上一次隐藏留下的
                    // 旧事件。tauri 在 Windows 上的失焦来自 WebView2 的回调、经
                    // 事件循环异步送达，可能晚于其后的新一次显示才到这里
                    if !state.paste_session().been_focused() {
                        return;
                    }
                    // 以下两种是"伪失焦"，不算用户离开，也不消耗首次启动的豁免。
                    // 拖拽 / 缩放窗口时 WebView 会误报失焦，但此时窗口仍为前台窗口
                    // （系统处于 move/size 模态）。这种情况不能 hide，否则窗口闪一下消失。
                    if is_window_in_foreground(window) {
                        return;
                    }
                    // 窗口已经不可见：这次失焦来自本进程自己的某次隐藏（Esc、快捷键
                    // 收起等），那条路径已经结束了会话
                    if !window.is_visible().unwrap_or(true) {
                        return;
                    }
                    // 走到这里说明是一次真实的失焦，才清掉缓存的获焦位。上面那些
                    // 提前返回的都是"不算用户离开"的事件（粘贴请求自己隐藏、旧事件、
                    // 拖拽 / 缩放的伪失焦、窗口已不可见），窗口此刻多半仍在前台，
                    // 清掉会让下一次"窗口开着时再唤起"误判成还没获焦
                    state.window_focused.store(false, Ordering::SeqCst);
                    // 首次启动豁免：第一次失焦不隐藏，让用户看清界面、可以自由点别处。
                    // 之后的失焦恢复正常的"贴入式"行为（点外部即收起）。
                    if FIRST_RUN_SUPPRESS_BLUR_HIDE.swap(false, Ordering::SeqCst) {
                        return;
                    }
                    // 用户点了别的窗口：自动收起并结束会话——目标不残留，
                    // 途中的粘贴请求也一并作废
                    if window.hide().is_ok() {
                        state.cancel_paste_session();
                    }
                }
                _ => {}
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
            copy_text,
            copy_or_paste,
            notify_paste_failed,
            get_paste_session,
            hide_window,
            get_hotkey,
            set_hotkey,
            list_recovery,
            read_recovery,
            restore_recovery,
            set_window_mode,
            set_interaction_lock,
            reveal_in_finder,
            get_sync_status,
            get_cloud_config,
            test_cloud_connection,
            save_cloud_config,
            test_github_connection,
            save_github_config,
            set_sync_provider,
            upload_all,
            download_all,
            open_url,
            get_autostart,
            set_autostart,
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

    /// 注入内容的契约：4 个键盘类型事件、顺序 Ctrl↓ V↓ V↑ Ctrl↑、只带虚拟键码。
    /// 事件 type 填错（INPUT_MOUSE 而非 INPUT_KEYBOARD）时 SendInput 照样返回
    /// "成功"，实机上也只是"没粘贴"——这个单测是唯一能确定拦住它的地方
    #[test]
    #[cfg(windows)]
    fn paste_sequence_is_four_keyboard_events_with_virtual_key_codes_only() {
        use super::paste_input::paste_events;
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_KEYBOARD, INPUT_MOUSE, KEYEVENTF_KEYUP,
        };
        assert_eq!(INPUT_MOUSE.0, 0);
        assert_eq!(INPUT_KEYBOARD.0, 1);

        let detail = |event: &INPUT| unsafe {
            (
                event.r#type,
                event.Anonymous.ki.wVk.0,
                event.Anonymous.ki.wScan,
                event.Anonymous.ki.dwFlags.0,
            )
        };
        let events = paste_events();
        let (vk_control, vk_v, released) = (0x11u16, 0x56u16, KEYEVENTF_KEYUP.0);
        assert_eq!(detail(&events[0]), (INPUT_KEYBOARD, vk_control, 0, 0));
        assert_eq!(detail(&events[1]), (INPUT_KEYBOARD, vk_v, 0, 0));
        assert_eq!(detail(&events[2]), (INPUT_KEYBOARD, vk_v, 0, released));
        assert_eq!(
            detail(&events[3]),
            (INPUT_KEYBOARD, vk_control, 0, released)
        );
    }

    #[test]
    fn copy_result_json_matches_frontend_contract() {
        // 前端 CopyResult 是联合类型 {status:"copied"} | {status:"paste_sent"} |
        // {status:"paste_failed", reason:...}：serde 的 tag+flatten 输出必须
        // 逐字符对齐，任何嵌套形状（externally tagged 默认）前端都判不出失败
        let json = serde_json::to_string(&CopyResult {
            status: CopyStatus::Copied,
        })
        .unwrap();
        assert_eq!(json, r#"{"status":"copied"}"#);
        let json = serde_json::to_string(&CopyResult {
            status: CopyStatus::PasteSent,
        })
        .unwrap();
        assert_eq!(json, r#"{"status":"paste_sent"}"#);
        let json = serde_json::to_string(&CopyResult {
            status: CopyStatus::PasteFailed {
                reason: PasteFailReason::FocusLost,
            },
        })
        .unwrap();
        assert_eq!(json, r#"{"status":"paste_failed","reason":"focus_lost"}"#);
        let json = serde_json::to_string(&CopyResult {
            status: CopyStatus::Superseded,
        })
        .unwrap();
        assert_eq!(json, r#"{"status":"superseded"}"#);
        // 给前端的会话快照同样是 IPC 契约：字段名 camelCase，generation 是数字
        let json = serde_json::to_string(&PasteSessionSnapshot {
            generation: 7,
            has_target: true,
        })
        .unwrap();
        assert_eq!(json, r#"{"generation":7,"hasTarget":true}"#);
    }

    /// 窗口隐藏之后的归因顺序：任何一步不通过都不注入
    #[test]
    fn paste_verdict_short_circuits_in_order() {
        use PasteFailReason::{FocusLost, InjectFailed, ModifiersHeld};
        fn never() -> Result<(), String> {
            panic!("must not paste")
        }
        let verdict = |session_current, focus_returned, modifiers_released, focus_on_target| {
            final_verdict(
                session_current,
                focus_returned,
                modifiers_released,
                focus_on_target,
                never,
            )
        };
        // 请求已过时优先于一切超时归因：用户都已经重新操作了，
        // 这个请求当时是超时还是没松键都不值得再提示
        assert_eq!(verdict(false, false, false, false), CopyStatus::Superseded);
        assert_eq!(verdict(false, true, false, true), CopyStatus::Superseded);
        // 焦点没在预算内回到原窗口（此时不会再去等修饰键）
        assert_eq!(
            verdict(true, false, false, true),
            CopyStatus::PasteFailed { reason: FocusLost }
        );
        // 焦点回来了，但物理修饰键没松（或松开后、注入前又被按下）
        assert_eq!(
            verdict(true, true, false, true),
            CopyStatus::PasteFailed {
                reason: ModifiersHeld
            }
        );
        // 会话仍在，但注入瞬间前台已经不是目标窗口
        assert_eq!(
            verdict(true, true, true, false),
            CopyStatus::PasteFailed { reason: FocusLost }
        );
        // 全部通过才注入；注入指令没完整发出要如实上报
        assert_eq!(
            final_verdict(true, true, true, true, || Err("send failed".into())),
            CopyStatus::PasteFailed {
                reason: InjectFailed
            }
        );
        assert_eq!(
            final_verdict(true, true, true, true, || Ok(())),
            CopyStatus::PasteSent
        );
    }

    /// 会话代次要能分清三件事：用户收起窗口、同一个窗口被再次唤起、
    /// 粘贴请求自己隐藏窗口。前两者让在途请求作废，第三种不算
    #[test]
    fn paste_session_distinguishes_cancel_reinvoke_and_own_hide() {
        let a = PasteTarget {
            window: 42,
            process_id: 7,
        };
        let mut session = PasteSession::default();

        // 从 A 唤起，粘贴请求记下此时的代次与目标
        session.begin(Some(a), false);
        let request = session.generation;
        assert_eq!(
            session.snapshot(),
            PasteSessionSnapshot {
                generation: request,
                has_target: true,
            }
        );

        // 情形一：请求还在写剪贴板，用户就按 Esc 收起了窗口。
        // 请求之后必须发现自己已过时——认领不了，更不能注入
        session.cancel();
        assert!(!session.is_current(request), "用户收起后，在途请求必须作废");
        assert!(!session.claim(a, request), "收起后目标已清，认领必须失败");

        // 情形二：再从 A 唤起（同一个窗口，句柄完全相同）。旧代次来认领要被拒，
        // 且不能误删新会话的目标；新请求可以认领，且只认领一次
        session.begin(Some(a), false);
        let fresh = session.generation;
        assert!(fresh > request, "每次唤起 / 收起代次都递增");
        assert!(!session.claim(a, request), "旧代次不得认领新会话的目标");
        assert!(session.snapshot().has_target);
        assert!(session.claim(a, fresh), "本次唤起的请求可以认领");
        assert!(!session.snapshot().has_target, "目标只认领一次");
        // 请求自己隐藏窗口不是收起：认领之后代次不变，请求仍然有效
        assert!(session.is_current(fresh));
        assert!(!session.claim(a, fresh), "同一目标不能被认领两次");

        // 情形三：请求等待期间用户又按了快捷键（还是从 A）：旧请求作废，
        // 新会话带着自己的目标
        session.begin(Some(a), false);
        assert!(!session.is_current(fresh));
        assert!(session.snapshot().has_target);

        // 情形四：上一次隐藏留下的失焦事件，晚于新一次显示才被处理。
        // 新会话的窗口还没获过焦，这条失焦不是它的，不能据此把新窗口收起；
        // 获焦之后的失焦才算是本次的
        session.cancel();
        assert!(!session.been_focused());
        session.begin(Some(a), false);
        assert!(!session.been_focused(), "从隐藏状态唤起：窗口还没获焦");
        session.note_focused();
        assert!(session.been_focused(), "获焦之后，失焦才是本次的");

        // 情形五：窗口开着且已获焦时又唤起一次（单实例 / 托盘显示）。
        // WebView2 不会再发获焦事件，现成的获焦状态要带进新会话——
        // 否则之后真实的失焦会被当成"还没获过焦"忽略
        session.begin(Some(a), true);
        assert!(session.been_focused(), "已获焦的窗口再次唤起，获焦状态保留");
    }

    /// SendInput 只送达一部分时的善后：按已送达的前缀补发释放，Ctrl 优先
    #[test]
    #[cfg(windows)]
    fn partial_delivery_releases_ctrl_first() {
        use super::paste_input::{dispatch, paste_events};
        use std::cell::RefCell;
        let total = paste_events().len();
        for (delivered, expected_follow_up) in [
            // 全被拒：没有键按下，不补发
            (0, vec![]),
            // 只送达 Ctrl↓：只补 Ctrl↑
            (1, vec![(0x11u16, 2u32)]),
            // Ctrl↓ V↓ 已送达：补 Ctrl↑ V↑（先放 Ctrl）
            (2, vec![(0x11, 2), (0x56, 2)]),
            // V 已经抬起，只差 Ctrl↑：不重复发 V↑
            (3, vec![(0x11, 2)]),
        ] {
            let follow_up = RefCell::new(Vec::new());
            let result = dispatch(|batch| {
                if batch.len() == total {
                    return delivered as u32;
                }
                *follow_up.borrow_mut() = batch
                    .iter()
                    .map(|event| unsafe {
                        (event.Anonymous.ki.wVk.0, event.Anonymous.ki.dwFlags.0)
                    })
                    .collect();
                batch.len() as u32
            });
            assert!(result.is_err(), "送达 {delivered} 个应报错");
            assert_eq!(follow_up.into_inner(), expected_follow_up);
        }
        // 整批送达：成功，且不再补发任何键
        let sent_events = RefCell::new(0);
        assert!(dispatch(|batch| {
            *sent_events.borrow_mut() += batch.len();
            batch.len() as u32
        })
        .is_ok());
        assert_eq!(sent_events.into_inner(), total);
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
    fn resolve_abs_allows_normal_relative_path() {
        let root = std::env::temp_dir().join("pp_test_resolve_root");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("写作")).unwrap();

        let ok = resolve_abs(&root, "写作/a.md").unwrap();
        // 两侧均保留 canonical 拼写：macOS 解析 /var 别名，Windows 保留 \\?\ 前缀。
        // 仅从一侧去掉 Windows 前缀，会把同一库内路径误判为越界。
        let root_canon = std::fs::canonicalize(&root).unwrap();
        assert_eq!(ok, root_canon.join("写作").join("a.md"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_abs_rejects_parent_traversal_existing() {
        // 路径存在时走 canonicalize 分支
        let base = std::env::temp_dir().join("pp_test_resolve_escape");
        let root = base.join("root");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&root).unwrap();
        // root 外放一个真实文件
        std::fs::write(base.join("secret.md"), "secret").unwrap();

        let result = resolve_abs(&root, "../secret.md");
        assert!(result.is_err(), "存在文件的 .. 逃逸必须被拒绝");

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn resolve_abs_rejects_parent_traversal_nonexistent() {
        // 路径不存在时走手工规范化分支（新建文件场景）
        let root = std::env::temp_dir().join("pp_test_resolve_escape2");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let result = resolve_abs(&root, "../../evil/new.md");
        assert!(result.is_err(), "不存在路径的 .. 逃逸也必须被拒绝");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn resolve_abs_rejects_absolute_path_injection() {
        // join 传入绝对路径会整体替换 root——必须拒绝
        let root = std::env::temp_dir().join("pp_test_resolve_abs_inj");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let result = resolve_abs(&root, "C:\\Windows\\Temp\\evil.md");
        assert!(result.is_err(), "绝对路径注入必须被拒绝");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn keyring_failure_degrades_to_memory_only() {
        // keyring 不可用时配置保存不整体失败：非密字段落盘 + 返回警告
        struct FailingSecretStore;
        impl CloudSecretStore for FailingSecretStore {
            fn read_password(&self, _username: &str) -> Result<Option<String>, String> {
                Err("凭据库不可用".to_string())
            }
            fn write_password(&self, _username: &str, _password: &str) -> Result<(), String> {
                Err("凭据库不可用".to_string())
            }
        }

        let dir = std::env::temp_dir().join("pp_test_keyring_degrade");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.json");

        let warning = persist_cloud_config(
            &config_file,
            &CloudConfig {
                username: "user@example.com".into(),
                password: "secret".into(),
                remote_root: "PromptPocket".into(),
                enabled: true,
            },
            &FailingSecretStore,
        )
        .unwrap();

        assert!(warning.is_some(), "keyring 失败应返回降级警告");
        let json = std::fs::read_to_string(&config_file).unwrap();
        assert!(json.contains("user@example.com"), "非密字段应照常持久化");
        assert!(!json.contains("secret"), "密码不应落入 JSON");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn focus_restore_polling_is_short_and_bounded() {
        let poll_ms = FOCUS_RESTORE_POLL_MS;
        let timeout_ms = FOCUS_RESTORE_TIMEOUT_MS;
        assert!(poll_ms <= 10);
        // 800ms：Chromium 系大应用激活可能超 400ms，预算过窄会把可注入
        // 场景静默退化为纯复制（吞字根因 R5）；上限仍需有界防无限等
        assert!(timeout_ms <= 1000);
        assert!(timeout_ms >= poll_ms);
    }

    #[test]
    fn global_hotkey_stays_ctrl_alt_p() {
        assert_eq!(GLOBAL_HOTKEY, "Ctrl+Alt+P");
    }

    #[test]
    fn persisted_config_migration_keeps_pending_secret_and_omits_cleared() {
        // 迁移期间（password/gh_token 仍为 Some）：配置重写必须把未迁移一侧的
        // 明文带回磁盘——skip_serializing 会静默剥字段，让后迁移的一侧丢凭据。
        // 迁移完成（清成 None）后字段自然省略，最终态不落明文
        let pending = PersistedConfig {
            username: Some("user@example.com".into()),
            password: Some("secret-app-password".into()),
            gh_token: Some("ghp_secret-token".into()),
            remote_root: Some("PromptPocket".into()),
            enabled: Some(true),
            ..Default::default()
        };
        let json = serde_json::to_string(&pending).unwrap();
        assert!(
            json.contains("secret-app-password"),
            "未迁移明文必须保留写回"
        );
        assert!(
            json.contains("ghp_secret-token"),
            "另一后端未迁移的 token 不得被剥掉"
        );

        // 清理完成后的最终态：两个字段都不出现
        let cleared = PersistedConfig {
            password: None,
            gh_token: None,
            ..pending
        };
        let json = serde_json::to_string(&cleared).unwrap();
        assert!(!json.contains("secret-app-password"));
        assert!(!json.contains("\"password\""));
        assert!(!json.contains("ghp_secret-token"));
        assert!(!json.contains("\"gh_token\""));
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
    fn github_config_roundtrip_preserves_webdav_fields() {
        // 共存性：保存 GitHub 配置不得抹掉同文件里的坚果云段（合并写回归测试）
        let dir = std::env::temp_dir().join("pp_test_github_config_merge");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.json");
        let store = MemorySecretStore::default();

        persist_cloud_config(
            &config_file,
            &CloudConfig {
                username: "user@example.com".into(),
                password: "webdav-secret".into(),
                remote_root: "PromptPocket".into(),
                enabled: true,
            },
            &store,
        )
        .unwrap();
        persist_github_config(
            &config_file,
            &GitHubConfig {
                repo: "someone/prompts".into(),
                branch: "main".into(),
                prefix: "archive".into(),
                token: "ghp_secret".into(),
                enabled: true,
            },
            &store,
        )
        .unwrap();

        let json = std::fs::read_to_string(&config_file).unwrap();
        assert!(json.contains("user@example.com"), "坚果云段必须保留");
        assert!(json.contains("someone/prompts"));
        assert!(!json.contains("ghp_secret"), "PAT 不应落入 JSON");

        let gh = load_github_config_with_store(&config_file, &store);
        assert_eq!(gh.repo, "someone/prompts");
        assert_eq!(gh.branch, "main");
        assert_eq!(gh.prefix, "archive");
        assert_eq!(gh.token, "ghp_secret");
        let webdav = load_cloud_config_with_store(&config_file, &store);
        assert_eq!(webdav.username, "user@example.com");
        assert_eq!(webdav.password, "webdav-secret");
    }

    #[test]
    fn legacy_github_token_is_migrated_out_of_json() {
        // 手改配置文件的明文 token：加载后迁入凭据库并从 JSON 清出
        let dir = std::env::temp_dir().join("pp_test_github_token_migrate");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.json");
        std::fs::write(
            &config_file,
            r#"{
  "gh_repo": "someone/prompts",
  "gh_token": "ghp_legacy",
  "enabled": true
}"#,
        )
        .unwrap();
        let store = MemorySecretStore::default();

        let cfg = load_github_config_with_store(&config_file, &store);

        assert_eq!(cfg.token, "ghp_legacy");
        assert_eq!(cfg.branch, "main", "branch 缺省应为 main");
        assert_eq!(
            store.read_password("someone/prompts").unwrap().as_deref(),
            Some("ghp_legacy")
        );
        let json = std::fs::read_to_string(&config_file).unwrap();
        assert!(!json.contains("ghp_legacy"));
        assert!(!json.contains("\"gh_token\""));
    }

    #[test]
    fn plaintext_secret_is_scrubbed_even_when_keyring_already_has_it() {
        // 凭据库已有密钥、JSON 又出现明文（手改配置）：明文也必须清出
        let dir = std::env::temp_dir().join("pp_test_plaintext_scrub");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config_file = dir.join("config.json");
        std::fs::write(
            &config_file,
            r#"{
  "username": "user@example.com",
  "password": "stray-plaintext",
  "gh_repo": "someone/prompts",
  "gh_token": "stray-token"
}"#,
        )
        .unwrap();
        let store = MemorySecretStore::default();
        store
            .write_password("user@example.com", "keyring-secret")
            .unwrap();
        store
            .write_password("someone/prompts", "keyring-token")
            .unwrap();

        let webdav = load_cloud_config_with_store(&config_file, &store);
        let github = load_github_config_with_store(&config_file, &store);

        // 凭据库优先：加载到的是 keyring 里的值
        assert_eq!(webdav.password, "keyring-secret");
        assert_eq!(github.token, "keyring-token");
        let json = std::fs::read_to_string(&config_file).unwrap();
        assert!(!json.contains("stray-plaintext"), "明文密码必须清出");
        assert!(!json.contains("stray-token"), "明文 token 必须清出");
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
        assert!(foreground_matches_window(true, false));
        assert!(foreground_matches_window(false, true));
        assert!(!foreground_matches_window(false, false));
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
