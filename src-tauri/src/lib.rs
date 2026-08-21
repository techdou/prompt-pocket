// Prompt Pocket — Tauri 应用入口（v1.0.0 坚果云同步版）
//
// 架构：本地缓存 + 坚果云 WebDAV 后台同步
// - UI 读写走本地缓存（store.rs），瞬间响应
// - 启动时从坚果云拉取，保存后异步推送
// - 账号/路径存 config.json，应用密码存系统凭据库

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, LogicalPosition, Manager, WindowEvent,
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
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
use crate::sync::{
    push_all_to_remote, CloudConfig, GitHubConfig, GitHubStore, RemoteStore, SyncStatus,
    WebDavStore,
};

const GLOBAL_HOTKEY: &str = "Ctrl+Alt+P";
const FOCUS_RESTORE_TIMEOUT_MS: u64 = 120;
const FOCUS_RESTORE_POLL_MS: u64 = 10;
const CLOUD_PASSWORD_SERVICE: &str = "com.promptpocket.webdav";
const GITHUB_TOKEN_SERVICE: &str = "com.promptpocket.github";

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
    // ── GitHub 存档（批次 B2）──
    /// 同步后端："webdav"（缺省）| "github"
    provider: Option<String>,
    gh_repo: Option<String>,
    gh_branch: Option<String>,
    gh_prefix: Option<String>,
    /// 仅作手改配置文件的兜底通道；正常由系统凭据库接管后清出 JSON（不落明文）
    #[serde(default, skip_serializing)]
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
    last_hotkey_had_text_input: Mutex<bool>,
}

impl AppState {
    fn cloud_config(&self) -> CloudConfig {
        // 毒锁时取出内部数据而非 panic（读操作安全）
        self.cloud.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_last_hotkey_had_text_input(&self, had_text_input: bool) {
        *self
            .last_hotkey_had_text_input
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = had_text_input;
    }

    fn take_last_hotkey_had_text_input(&self) -> bool {
        let mut guard = self
            .last_hotkey_had_text_input
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let had_text_input = *guard;
        *guard = false;
        had_text_input
    }

    fn set_cloud_config(&self, cfg: CloudConfig) -> Result<(), String> {
        let warning = persist_cloud_config(&self.config_file, &cfg, &SystemCloudSecretStore::new(CLOUD_PASSWORD_SERVICE))?;
        {
            *self.cloud.lock().map_err(|e| e.to_string())? = cfg.clone();
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
            *self.github.lock().map_err(|e| e.to_string())? = cfg;
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
        *self.provider.lock().map_err(|e| e.to_string())? = p;
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
    let guard = state
        .io_gate
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if guard.syncing {
        return Err("正在同步中，请稍候再编辑".to_string());
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
            return Err("正在同步中，请稍候".to_string());
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
    let dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|e| {
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

/// 启动时加载配置：两侧后端配置 + 当前选中的 provider
fn load_cloud_config(config_file: &std::path::Path) -> (CloudConfig, GitHubConfig, SyncProvider) {
    let webdav =
        load_cloud_config_with_store(config_file, &SystemCloudSecretStore::new(CLOUD_PASSWORD_SERVICE));
    let github =
        load_github_config_with_store(config_file, &SystemCloudSecretStore::new(GITHUB_TOKEN_SERVICE));
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

    // 旧版明文密码迁移进系统凭据库，并从 JSON 清出（合并写，保留其它字段）
    if let (Ok(None), Some(legacy)) = (&stored_password, legacy_password.as_ref()) {
        if !username.is_empty() && secret_store.write_password(&username, legacy).is_ok() {
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

    // 手填明文 token 的迁移：进凭据库 + 清出 JSON
    if let (Ok(None), Some(legacy)) = (&stored_token, legacy_token.as_ref()) {
        if !repo.is_empty() && secret_store.write_password(&repo, legacy).is_ok() {
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
    // 用新路径查找返回的 prompt（路径可能变了，必须用 new_abs）
    let result = scan_disk(&state.local_dir)
        .map_err(|e| e.to_string())?
        .prompts
        .into_iter()
        .find(|p| p.abs_path.as_str() == new_abs.to_string_lossy().as_ref())
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

    scan_disk(&state.local_dir)
        .map_err(|e| e.to_string())?
        .prompts
        .into_iter()
        .find(|p| p.abs_path.as_str() == new_abs.to_string_lossy().as_ref())
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
async fn copy_text(text: String, app: tauri::AppHandle) -> Result<(), String> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

/// 智能复制/注入：写剪贴板 → 隐藏窗口 → 等焦点回归 → 按快捷键来源决定是否注入。
/// - Ctrl+Alt+P 按下时外部前台有 caret：模拟 Ctrl+V 把内容注入原输入框
/// - Ctrl+Alt+P 按下时不在输入框：纯复制到剪贴板，不误粘贴
///
/// mode 参数当前未区分转换（前端传 editingBody 原文），保留以兼容现有调用契约。
#[tauri::command]
async fn copy_or_paste(
    text: String,
    mode: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // mode 当前未做格式转换（前端传原文），保留参数以兼容现有调用契约。
    let _ = mode;
    // 1. 写剪贴板（无论后续是否注入，剪贴板都得有内容）
    app.clipboard()
        .write_text(&text)
        .map_err(|e| e.to_string())?;

    let invoked_from_text_input = state.take_last_hotkey_had_text_input();

    // 2. 隐藏当前窗口，让 OS 焦点回归到用户原本聚焦的应用
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }

    // 3. 不来自输入框时立即结束：剪贴板已写好，不做任何粘贴尝试。
    if !invoked_from_text_input {
        return Ok(());
    }

    // 4. hide() 后焦点回归是异步的。短轮询比固定等待更快：
    //    输入框一恢复焦点就粘贴，最长只等一小段时间。
    let returned_to_text_input = wait_for_text_input_focus(
        std::time::Duration::from_millis(FOCUS_RESTORE_TIMEOUT_MS),
        std::time::Duration::from_millis(FOCUS_RESTORE_POLL_MS),
    )
    .await;
    if !should_inject_after_hotkey(invoked_from_text_input, returned_to_text_input) {
        return Ok(());
    }

    // 5. 模拟 Ctrl+V 注入
    simulate_paste().map_err(|e| format!("注入失败: {e}"))?;
    Ok(())
}

fn should_inject_after_hotkey(invoked_from_text_input: bool, returned_to_text_input: bool) -> bool {
    invoked_from_text_input && returned_to_text_input
}

async fn wait_for_text_input_focus(
    timeout: std::time::Duration,
    poll: std::time::Duration,
) -> bool {
    let started = std::time::Instant::now();
    loop {
        if foreground_has_text_input_focus() {
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

/// 用 enigo 模拟一次 Ctrl+V 粘贴（跨平台，macOS 需改 Meta，此处先支持 Windows/Linux）
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn simulate_paste() -> Result<(), Box<dyn std::error::Error>> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};

    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.key(Key::Control, Direction::Press)?;
    enigo.key(Key::Unicode('v'), Direction::Click)?;
    enigo.key(Key::Control, Direction::Release)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn simulate_paste() -> Result<(), Box<dyn std::error::Error>> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};

    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.key(Key::Meta, Direction::Press)?;
    enigo.key(Key::Unicode('v'), Direction::Click)?;
    enigo.key(Key::Meta, Direction::Release)?;
    Ok(())
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
) -> Result<(), String> {
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
) -> Result<(), String> {
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
async fn upload_all(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let provider = state.provider();
    match provider {
        SyncProvider::WebDav if !state.cloud_config().is_configured() => {
            return Err("未配置坚果云同步".to_string());
        }
        SyncProvider::GitHub if !state.github_config().is_configured() => {
            return Err("未配置 GitHub 存档（需仓库和 PAT）".to_string());
        }
        _ => {}
    }
    // 并发保护：IoGate 单锁互斥（等待在飞的本地写完成后置位 syncing）；
    // guard drop 自动复位（panic 也不卡死）
    let _guard = SyncGuard::acquire(&state.io_gate)?;
    let result = match provider {
        SyncProvider::WebDav => {
            let store = WebDavStore::new(&state.cloud_config())?;
            push_all_to_remote(&store, &state.local_dir).await
        }
        SyncProvider::GitHub => {
            let store = GitHubStore::new(&state.github_config())?;
            push_all_to_remote(&store, &state.local_dir).await
        }
    };
    match result {
        Ok(report) => {
            let mut msg = format!("上传完成：共 {} 个文件", report.uploaded);
            if report.deleted_remote > 0 {
                msg.push_str(&format!("，云端删除 {}", report.deleted_remote));
            }
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

/// 全量下载：从当前后端拉取并覆盖本地（覆盖前备份 .trash，tombstone 防复活）
#[tauri::command]
async fn download_all(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let provider = state.provider();
    match provider {
        SyncProvider::WebDav if !state.cloud_config().is_configured() => {
            return Err("未配置坚果云同步".to_string());
        }
        SyncProvider::GitHub if !state.github_config().is_configured() => {
            return Err("未配置 GitHub 存档（需仓库和 PAT）".to_string());
        }
        _ => {}
    }
    // 并发保护：IoGate 单锁互斥；guard drop 自动复位
    let _guard = SyncGuard::acquire(&state.io_gate)?;
    let result = match provider {
        SyncProvider::WebDav => {
            let store = WebDavStore::new(&state.cloud_config())?;
            sync::pull_from_remote(&store, &state.local_dir).await
        }
        SyncProvider::GitHub => {
            let store = GitHubStore::new(&state.github_config())?;
            sync::pull_from_remote(&store, &state.local_dir).await
        }
    };
    match result {
        Ok(report) => {
            let mut msg = format!(
                "下载完成：更新 {}，跳过 {}，清理 {}",
                report.downloaded, report.skipped, report.deleted
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
fn resolve_abs(root: &std::path::Path, rel: &str) -> Result<PathBuf, String> {
    let root_norm = match std::fs::canonicalize(root) {
        Ok(canon) => strip_unc_prefix(&canon),
        Err(_) => normalize_lexical(root),
    };
    let joined = root_norm.join(rel);
    let normalized = match std::fs::canonicalize(&joined) {
        Ok(canon) => strip_unc_prefix(&canon),
        Err(_) => normalize_lexical(&joined),
    };
    if normalized.starts_with(&root_norm) {
        Ok(normalized)
    } else {
        Err("路径越界：拒绝访问数据目录之外的文件".to_string())
    }
}

/// 不依赖文件系统的手工路径规范化：展开 `.` 和 `..`。
/// `..` 弹出到根组件后自然停止（不会 panic），越界结果交给调用方的前缀校验拒绝。
fn normalize_lexical(p: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
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
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
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
                last_hotkey_had_text_input: Mutex::new(false),
            });

            // v1.0.1：同步改为纯手动，启动时不再自动拉取

            // 注册全局快捷键（解析失败则降级为无快捷键，不 panic）
            let app_handle = app.handle().clone();
            match GLOBAL_HOTKEY.parse::<Shortcut>() {
                Ok(shortcut) => {
                    if let Err(e) = app.global_shortcut().on_shortcut(
                        shortcut,
                        move |_app, _shortcut, event| {
                            if event.state == ShortcutState::Pressed {
                                let window_is_visible = app_handle
                                    .get_webview_window("main")
                                    .and_then(|win| win.is_visible().ok())
                                    .unwrap_or(false);
                                let hotkey_had_text_input =
                                    !window_is_visible && foreground_has_text_input_focus();
                                app_handle
                                    .state::<AppState>()
                                    .set_last_hotkey_had_text_input(hotkey_had_text_input);
                                toggle_main_window(&app_handle);
                            }
                        },
                    ) {
                        eprintln!("[启动] 全局快捷键注册失败（可能被占用）: {e}");
                        // 弹窗提示用户：快捷键被占用，应用只能靠托盘/双击第二实例唤起
                        let app_handle = app.handle().clone();
                        let hotkey = GLOBAL_HOTKEY.to_string();
                        app_handle
                            .dialog()
                            .message(format!(
                                "全局快捷键 {hotkey} 注册失败（可能被其他软件占用）。\n\n\
                                 你仍可以点击右下角托盘图标来打开主界面，或在设置里关闭占用该快捷键的软件后重启本程序。"
                            ))
                            .kind(MessageDialogKind::Warning)
                            .title("快捷键注册失败")
                            .show(|_| {});
                    }
                }
                Err(e) => {
                    eprintln!("[启动] 全局快捷键解析失败: {e}");
                }
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
                        toggle_main_window(tray.app_handle());
                    }
                })
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "show" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
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
            copy_text,
            copy_or_paste,
            hide_window,
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
        // 返回值是 canonical 形式：macOS 上 temp_dir 是 /var（/private/var 的符号链接），
        // 不能直接和原始 root 比前缀，要和 canonical 后的 root 比
        let root_canon = strip_unc_prefix(&std::fs::canonicalize(&root).unwrap());
        assert!(ok.starts_with(&root_canon), "正常相对路径应解析到 root 内");
        assert!(ok.ends_with(Path::new("写作").join("a.md")));

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
    fn paste_injection_requires_hotkey_origin_and_returned_caret() {
        assert!(should_inject_after_hotkey(true, true));
        assert!(!should_inject_after_hotkey(false, true));
        assert!(!should_inject_after_hotkey(true, false));
        assert!(!should_inject_after_hotkey(false, false));
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
            gh_token: Some("ghp_secret-token".into()),
            remote_root: Some("PromptPocket".into()),
            enabled: Some(true),
            ..Default::default()
        };

        let json = serde_json::to_string(&persisted).unwrap();

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
