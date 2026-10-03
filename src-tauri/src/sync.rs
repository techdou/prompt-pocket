// 坚果云 WebDAV 同步模块
//
// 架构：本地缓存 + 后台同步。
// - 所有 UI 读写仍走本地缓存（store.rs），保证瞬间响应
// - 本模块负责本地缓存 ↔ 坚果云的双向同步
//
// 同步策略：手动定向上传/下载，精确内容共同基线，冲突保留双方版本。
// 本地独有文件默认保留；远程覆盖通过条件请求保护。

use reqwest_dav::types::list_cmd::ListEntity;
use reqwest_dav::{Auth, Client, ClientBuilder, Depth, Error as DavError};
use std::collections::HashSet;
use std::path::Path;

/// 坚果云 WebDAV 端点
const JIANGUO_HOST: &str = "https://dav.jianguoyun.com/dav";

/// 同步配置（从 config.json 加载）
#[derive(Debug, Clone, Default)]
pub struct CloudConfig {
    pub username: String,
    pub password: String,    // 应用密码（App Password）
    pub remote_root: String, // 远程根路径，如 "PromptPocket"
    pub enabled: bool,
}

impl CloudConfig {
    pub fn is_configured(&self) -> bool {
        self.enabled && !self.username.is_empty() && !self.password.is_empty()
    }
}

/// 同步状态（暴露给前端展示）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub configured: bool,
    pub enabled: bool,
    pub last_sync: Option<String>,
    pub last_error: Option<String>,
    pub syncing: bool,
}

/// 构造 WebDAV 客户端（带超时，避免坚果云慢响应时无限期挂起）
pub fn build_client(cfg: &CloudConfig) -> Result<Client, DavError> {
    let agent = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(DavError::Reqwest)?;
    ClientBuilder::new()
        .set_agent(agent)
        .set_host(JIANGUO_HOST.to_string())
        .set_auth(Auth::Basic(cfg.username.clone(), cfg.password.clone()))
        .build()
}

/// 测试连接：PROPFIND 远程根目录，验证凭据 + 路径可访问
pub async fn test_connection(cfg: &CloudConfig) -> Result<(), String> {
    let client = build_client(cfg).map_err(|e| format!("客户端构建失败: {e}"))?;
    let root = sanitize_remote_path(&cfg.remote_root);
    validate_root(&root)?;
    client
        .list(&remote_path(&root, ""), Depth::Number(0))
        .await
        .map_err(|e| format!("连接失败，请检查账号/应用密码/路径: {e}"))?;
    Ok(())
}

/// 拉取完整远程清单并比较实际内容。本地独有文件永不因下载被删除。
pub async fn pull_from_remote(cfg: &CloudConfig, local_dir: &Path) -> Result<SyncReport, String> {
    let client = build_client(cfg).map_err(|e| format!("客户端构建失败: {e}"))?;
    pull_with_client(&client, &sanitize_remote_path(&cfg.remote_root), local_dir).await
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub downloaded: u32,
    pub skipped: u32,
    pub deleted: u32,
    pub uploaded: u32,
    pub conflicts: u32,
    pub errors: Vec<String>,
}

// A baseline means both sides were observed to contain these exact bytes. Old
// upload-only hash records cannot establish that invariant and are not trusted.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SyncMeta {
    version: u8,
    scope: String,
    files: std::collections::BTreeMap<String, String>,
}

impl SyncMeta {
    fn new(scope: &str) -> Self {
        Self {
            version: 2,
            scope: scope.into(),
            files: Default::default(),
        }
    }
}

fn sync_scope(client: &Client, root: &str) -> String {
    let user = match &client.auth {
        Auth::Basic(user, _) | Auth::Digest(user, _) => user.as_str(),
        Auth::Anonymous => "",
    };
    serde_json::to_string(&(&client.host, user, root)).expect("strings serialize")
}

fn load_sync_meta(local_dir: &Path, scope: &str) -> Result<SyncMeta, String> {
    let path = local_dir.join(".sync_meta.json");
    match std::fs::symlink_metadata(&path) {
        Ok(info) if !info.is_file() || info.file_type().is_symlink() => {
            return Err("同步基线不是普通文件".into())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SyncMeta::new(scope))
        }
        Err(error) => return Err(error.to_string()),
        _ => {}
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("同步基线损坏: {e}"))?;
    if value.get("version").and_then(serde_json::Value::as_u64) != Some(2) {
        return Ok(SyncMeta::new(scope));
    }
    let meta: SyncMeta = serde_json::from_value(value).map_err(|e| format!("同步基线损坏: {e}"))?;
    Ok(if meta.scope == scope {
        meta
    } else {
        SyncMeta::new(scope)
    })
}

fn save_sync_meta(local_dir: &Path, meta: &SyncMeta) -> Result<(), String> {
    let path = local_dir.join(".sync_meta.json");
    if std::fs::symlink_metadata(&path)
        .is_ok_and(|info| !info.is_file() || info.file_type().is_symlink())
    {
        return Err("同步基线不是普通文件".into());
    }
    let bytes = serde_json::to_vec_pretty(meta).map_err(|e| e.to_string())?;
    crate::recovery::atomic_write(&path, &bytes).map_err(|e| format!("保存同步基线失败: {e}"))
}

async fn pull_with_client(
    client: &Client,
    root: &str,
    local_dir: &Path,
) -> Result<SyncReport, String> {
    validate_root(root)?;
    let canonical_dir = std::fs::canonicalize(local_dir).map_err(|e| e.to_string())?;
    let local_dir = canonical_dir.as_path();
    let mut report = SyncReport::default();
    let mut meta = load_sync_meta(local_dir, &sync_scope(client, root))?;
    let files = walk_remote(client, root, &mut report.errors).await;
    for rel in files {
        let result = async {
            let path = crate::recovery::checked_path(local_dir, &rel).map_err(|e| e.to_string())?;
            let remote = fetch_remote(client, root, &rel)
                .await?
                .ok_or("远程文件在列举后已不存在")?;
            let local = read_optional(&path)?;
            if local.as_deref() == Some(remote.content.as_bytes()) {
                meta.files.insert(rel.clone(), remote.content);
                report.skipped += 1;
                return Ok::<(), String>(());
            }
            if let Some(local) = local.as_deref() {
                let base = meta.files.get(&rel).map(String::as_bytes);
                if base == Some(remote.content.as_bytes()) {
                    // Only the local side changed. Download is not permission to discard it.
                    report.skipped += 1;
                    return Ok(());
                }
                if base != Some(local) {
                    record_conflict(local_dir, &rel, &remote.content, &mut report)?;
                    return Ok(());
                }
            }
            write_download(local_dir, &rel, local.as_deref(), remote.content.as_bytes())?;
            meta.files.insert(rel.clone(), remote.content);
            report.downloaded += 1;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            report.errors.push(format!("{rel}: {error}"));
        }
    }
    // No deletion pass: neither an incomplete listing nor a remote deletion is
    // sufficient evidence to remove an independent or locally changed prompt.
    save_sync_meta(local_dir, &meta)?;
    Ok(report)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn write_download(
    root: &Path,
    rel: &str,
    expected: Option<&[u8]>,
    bytes: &[u8],
) -> Result<(), String> {
    let path = crate::recovery::checked_path(root, rel).map_err(|e| e.to_string())?;
    if read_optional(&path)?.as_deref() != expected {
        return Err("下载期间本地内容已变化，已保留本地文件，请重试".into());
    }
    if expected.is_some() {
        crate::recovery::snapshot(root, &path, "sync")
            .map_err(|e| format!("备份失败，未覆盖原文件: {e}"))?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Revalidate after creating parents / taking the snapshot.
    let path = crate::recovery::checked_path(root, rel).map_err(|e| e.to_string())?;
    if read_optional(&path)?.as_deref() != expected {
        return Err("下载期间本地内容已变化，已保留本地文件，请重试".into());
    }
    crate::recovery::atomic_write(&path, bytes).map_err(|e| e.to_string())
}

fn record_conflict(
    root: &Path,
    rel: &str,
    remote: &str,
    report: &mut SyncReport,
) -> Result<(), String> {
    use std::io::Write;
    let original = crate::recovery::checked_path(root, rel).map_err(|e| e.to_string())?;
    let parent = original.parent().ok_or("冲突文件缺少父目录")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("prompt")
        .trim_start_matches('.');
    let ext = original
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("md");
    // Content-derived names reuse an existing identical conflict; a collision
    // always compares bytes and picks another name, never overwrites a file.
    for index in 0..1000 {
        let name = format!(
            "{stem}.remote-conflict-{:016x}-{index}.{ext}",
            fnv1a_hash(remote.as_bytes())
        );
        let path = parent.join(name);
        let conflict_rel = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let path = crate::recovery::checked_path(root, &conflict_rel).map_err(|e| e.to_string())?;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                if original.exists() {
                    if let Err(error) = crate::recovery::snapshot(root, &original, "sync") {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        return Err(format!("冲突备份失败: {error}"));
                    }
                }
                if let Err(error) = file
                    .write_all(remote.as_bytes())
                    .and_then(|_| file.sync_all())
                {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(format!("保存冲突副本失败: {error}"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::fs::read(&path).map_err(|e| e.to_string())? != remote.as_bytes() {
                    continue;
                }
            }
            Err(error) => return Err(error.to_string()),
        }
        report.conflicts += 1;
        report.errors.push(format!(
            "{rel}: 保留本地内容，远程冲突副本已保存为 {conflict_rel}"
        ));
        return Ok(());
    }
    Err("无法分配冲突副本文件名".into())
}

struct RemoteContent {
    content: String,
    etag: Option<String>,
}

async fn fetch_remote(
    client: &Client,
    root: &str,
    rel: &str,
) -> Result<Option<RemoteContent>, String> {
    let response = client
        .get_raw(&remote_path(root, rel))
        .await
        .map_err(|e| format!("GET 失败: {e}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("GET 失败: {}", response.status()));
    }
    let response = response
        .error_for_status()
        .map_err(|e| format!("GET 失败: {e}"))?;
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .filter(|tag| tag.starts_with('"') && tag.ends_with('"'))
        .map(str::to_owned);
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    let content =
        String::from_utf8(bytes.to_vec()).map_err(|e| format!("远程文件不是有效 UTF-8: {e}"))?;
    Ok(Some(RemoteContent { content, etag }))
}

async fn walk_remote(client: &Client, root: &str, errors: &mut Vec<String>) -> Vec<String> {
    let mut files = std::collections::BTreeSet::new();
    let mut queue = std::collections::VecDeque::from([String::new()]);
    let mut visited = HashSet::new();
    while let Some(dir) = queue.pop_front() {
        if !visited.insert(dir.clone()) {
            continue;
        }
        let request = format!("{}/", remote_path(root, &dir).trim_end_matches('/'));
        let entities = match client.list(&request, Depth::Number(1)).await {
            Ok(entities) => entities,
            Err(error) => {
                errors.push(format!("列出远程目录 {dir}/ 失败: {error}"));
                continue;
            }
        };
        for entity in entities {
            match entity {
                ListEntity::File(file) => {
                    if let Some(rel) =
                        extract_rel_path(&file.href, root).filter(|rel| is_sync_file(rel))
                    {
                        files.insert(rel);
                    }
                }
                ListEntity::Folder(folder) => {
                    if let Some(rel) =
                        extract_rel_path(&folder.href, root).filter(|rel| is_sync_dir(rel))
                    {
                        if rel != dir && !visited.contains(&rel) {
                            queue.push_back(rel);
                        }
                    }
                }
            }
        }
    }
    files.into_iter().collect()
}

fn is_sync_dir(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.contains(['\\', ':', '\0'])
        && rel
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.starts_with('~'))
}

fn is_sync_file(rel: &str) -> bool {
    let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    if !parent.is_empty() && !is_sync_dir(parent) {
        return false;
    }
    if name.is_empty()
        || name.contains(['\\', ':', '\0'])
        || name.starts_with('~')
        || name.contains(".remote-conflict-")
    {
        return false;
    }
    name == ".order.json"
        || name == ".category-order.json"
        || (!name.starts_with('.') && name.ends_with(".md"))
}

/// 上传始终读取远端当前内容。只有共同基线未在远端变化时才允许覆盖，
/// 并使用强 ETag 的 If-Match（新文件使用 If-None-Match:*）保护 GET/PUT 竞争。
pub async fn push_all_to_remote(cfg: &CloudConfig, local_dir: &Path) -> Result<SyncReport, String> {
    let client = build_client(cfg).map_err(|e| format!("客户端构建失败: {e}"))?;
    push_with_client(&client, &sanitize_remote_path(&cfg.remote_root), local_dir).await
}

async fn push_with_client(
    client: &Client,
    root: &str,
    local_dir: &Path,
) -> Result<SyncReport, String> {
    validate_root(root)?;
    let canonical_dir = std::fs::canonicalize(local_dir).map_err(|e| e.to_string())?;
    let local_dir = canonical_dir.as_path();
    let mut report = SyncReport::default();
    let mut meta = load_sync_meta(local_dir, &sync_scope(client, root))?;
    ensure_remote_dirs(client, root, "").await?;
    for entry in walkdir::WalkDir::new(local_dir)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.file_type().is_symlink() {
                return false;
            }
            let rel = entry
                .path()
                .strip_prefix(local_dir)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            if entry.file_type().is_dir() {
                is_sync_dir(&rel)
            } else {
                is_sync_file(&rel)
            }
        })
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.errors.push(error.to_string());
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(local_dir)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let result = async {
            let path = crate::recovery::checked_path(local_dir, &rel).map_err(|e| e.to_string())?;
            let local = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let remote = fetch_remote(client, root, &rel).await?;
            if remote.as_ref().is_some_and(|r| r.content == local) {
                meta.files.insert(rel.clone(), local);
                report.skipped += 1;
                return Ok::<(), String>(());
            }
            if let Some(remote) = &remote {
                if meta.files.get(&rel) != Some(&remote.content) || remote.etag.is_none() {
                    record_conflict(local_dir, &rel, &remote.content, &mut report)?;
                    return Ok(());
                }
            } else if meta.files.contains_key(&rel) {
                // A remote deletion and a surviving local file need an explicit
                // decision; silently recreating it could undo another device.
                report.conflicts += 1;
                report.errors.push(format!(
                    "{rel}: 远程文件已删除，已保留本地文件，未自动重新上传"
                ));
                return Ok(());
            }
            ensure_remote_dirs(client, root, &rel).await?;
            let mut request = client
                .start_request(reqwest::Method::PUT, &remote_path(root, &rel))
                .await
                .map_err(|e| e.to_string())?;
            request = match remote.as_ref().and_then(|r| r.etag.as_ref()) {
                Some(etag) => request.header(reqwest::header::IF_MATCH, etag),
                None => request.header(reqwest::header::IF_NONE_MATCH, "*"),
            };
            let response = request
                .body(local.clone())
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if response.status() == reqwest::StatusCode::PRECONDITION_FAILED
                || response.status() == reqwest::StatusCode::CONFLICT
            {
                if let Some(latest) = fetch_remote(client, root, &rel).await? {
                    record_conflict(local_dir, &rel, &latest.content, &mut report)?;
                } else {
                    report.conflicts += 1;
                    report
                        .errors
                        .push(format!("{rel}: 上传期间远程文件已变化，未覆盖"));
                }
                return Ok(());
            }
            response
                .error_for_status()
                .map_err(|e| format!("PUT 失败: {e}"))?;
            meta.files.insert(rel.clone(), local);
            report.uploaded += 1;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            report.errors.push(format!("{rel}: {error}"));
        }
    }
    save_sync_meta(local_dir, &meta)?;
    Ok(report)
}

async fn ensure_remote_dirs(client: &Client, root: &str, rel: &str) -> Result<(), String> {
    let parent = rel.rsplit_once('/').map_or("", |(parent, _)| parent);
    let mut parts = vec![root.to_string()];
    let mut accumulated = String::new();
    for part in parent.split('/').filter(|part| !part.is_empty()) {
        if !accumulated.is_empty() {
            accumulated.push('/');
        }
        accumulated.push_str(part);
        parts.push(format!("{root}/{accumulated}"));
    }
    for path in parts {
        let response = client
            .mkcol_raw(&format!("/{}", encode_path(&path)))
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success()
            && response.status() != reqwest::StatusCode::METHOD_NOT_ALLOWED
        {
            return Err(format!("创建远程目录失败: {}", response.status()));
        }
    }
    Ok(())
}

fn validate_root(root: &str) -> Result<(), String> {
    if is_sync_dir(root) {
        Ok(())
    } else {
        Err("远程根路径不能为空或包含隐藏/上级目录".into())
    }
}

fn sanitize_remote_path(s: &str) -> String {
    s.trim_matches('/').to_string()
}

fn remote_path(root: &str, rel: &str) -> String {
    format!("/{}/{}", encode_path(root), encode_path(rel))
}

fn encode_path(path: &str) -> String {
    path.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn extract_rel_path(href: &str, root: &str) -> Option<String> {
    let path = if href.starts_with("http://") || href.starts_with("https://") {
        reqwest::Url::parse(href).ok()?.path().to_string()
    } else {
        href.to_string()
    };
    let decoded = urlencoding_decode(&path)?;
    let marker = format!("/{root}/");
    let index = decoded.find(&marker)?;
    let rel = decoded[index + marker.len()..].trim_end_matches('/');
    if rel.is_empty()
        || rel
            .split('/')
            .any(|part| part == "." || part == ".." || part.is_empty())
        || rel.contains(['\\', ':', '\0'])
    {
        None
    } else {
        Some(rel.into())
    }
}

fn urlencoding_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

// Used only to name conflict copies; all sync decisions compare exact content.
fn fnv1a_hash(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression contract for the new synchronizer. Tests use isolated temporary
    // libraries and a loopback-only DAV server; never the configured user account.
    #[test]
    fn equal_length_remote_edit_is_downloaded_and_old_version_is_recoverable() {
        let dir = TestDir::new();
        let server = MockDav::new(&[("a.md", b"old")]);
        run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        server.set("a.md", b"new");
        let report = run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        assert_eq!(report.downloaded, 1, "{report:?}");
        assert!(report.errors.is_empty(), "{report:?}");
        assert_eq!(std::fs::read(dir.0.join("a.md")).unwrap(), b"new");
        let recovered = crate::recovery::list_recovery(&dir.0).unwrap();
        assert!(
            recovered.iter().any(|entry| entry.original_path == "a.md"
                && entry.kind == "sync"
                && crate::recovery::read_recovery(&dir.0, &entry.id).unwrap() == "old"),
            "{recovered:?}"
        );
    }

    #[test]
    fn failed_snapshot_prevents_download_overwrite_and_baseline_advance() {
        let dir = TestDir::new();
        let server = MockDav::new(&[("a.md", b"base")]);
        let initial = run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        assert!(initial.errors.is_empty(), "{initial:?}");
        std::fs::write(dir.0.join(".recovery"), b"blocked directory").unwrap();
        server.set("a.md", b"new content");
        let report = run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        assert_eq!(report.downloaded, 0);
        assert!(
            report.errors.iter().any(|error| error.contains("备份失败")),
            "{report:?}"
        );
        assert_eq!(std::fs::read(dir.0.join("a.md")).unwrap(), b"base");
        let baseline =
            load_sync_meta(&dir.0, &sync_scope(&server.client(), "PromptPocket")).unwrap();
        assert_eq!(baseline.files["a.md"], "base");
    }

    #[test]
    fn incomplete_listing_and_failed_download_never_remove_local_files() {
        let dir = TestDir::new();
        let server = MockDav::new(&[("a.md", b"remote"), ("folder/b.md", b"remote")]);
        std::fs::write(dir.0.join("local.md"), b"only here").unwrap();
        server
            .state
            .lock()
            .unwrap()
            .failed_list
            .insert("folder".into());
        server
            .state
            .lock()
            .unwrap()
            .failed_get
            .insert("a.md".into());
        let report = run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        assert!(
            report.errors.iter().any(|error| error.contains("folder/")),
            "{report:?}, requests: {:?}",
            server.state.lock().unwrap().requests
        );
        assert!(report.errors.iter().any(|error| error.contains("a.md")));
        assert_eq!(report.deleted, 0);
        assert_eq!(std::fs::read(dir.0.join("local.md")).unwrap(), b"only here");
    }

    #[test]
    fn local_only_and_local_edits_survive_download() {
        let dir = TestDir::new();
        let server = MockDav::new(&[("a.md", b"base")]);
        run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        std::fs::write(dir.0.join("a.md"), b"my edit").unwrap();
        std::fs::write(dir.0.join("local.md"), b"only here").unwrap();
        let report = run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        assert_eq!(report.downloaded, 0);
        assert_eq!(std::fs::read(dir.0.join("a.md")).unwrap(), b"my edit");
        assert!(dir.0.join("local.md").exists());
    }

    #[test]
    fn diverged_or_unbased_files_preserve_both_versions_and_do_not_advance_baseline() {
        for establish_base in [false, true] {
            let dir = TestDir::new();
            let server = MockDav::new(&[("a.md", b"base")]);
            if establish_base {
                run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
            }
            std::fs::write(dir.0.join("a.md"), b"local edit").unwrap();
            server.set("a.md", b"remote edit");
            let report = run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
            assert_eq!(report.conflicts, 1);
            assert_eq!(std::fs::read(dir.0.join("a.md")).unwrap(), b"local edit");
            assert!(std::fs::read_dir(&dir.0)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .contains("remote-conflict")
                    && std::fs::read(entry.path()).ok().as_deref() == Some(b"remote edit")));
            let again = run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
            assert_eq!(again.conflicts, 1);
        }
    }

    #[test]
    fn order_metadata_roundtrips_but_hidden_files_and_symlinks_do_not_sync() {
        let dir = TestDir::new();
        let server = MockDav::new(&[
            (".category-order.json", b"[]"),
            ("folder/.order.json", b"[]"),
            (".hidden/a.md", b"secret"),
        ]);
        let report = run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        assert_eq!(
            report.downloaded,
            2,
            "{report:?}, requests: {:?}",
            server.state.lock().unwrap().requests
        );
        assert!(!dir.0.join(".hidden").exists());
        std::fs::write(dir.0.join(".category-order.json"), b"[\"folder\"]").unwrap();
        std::fs::write(dir.0.join("folder/.order.json"), b"[\"a.md\"]").unwrap();
        std::fs::create_dir(dir.0.join(".private")).unwrap();
        std::fs::write(dir.0.join(".private/secret.md"), b"private").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.0.join(".private/secret.md"), dir.0.join("link.md"))
            .unwrap();
        let report = run(push_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        assert_eq!(report.uploaded, 2);
        let state = server.state.lock().unwrap();
        assert_eq!(state.files[".category-order.json"], b"[\"folder\"]");
        assert_eq!(state.files["folder/.order.json"], b"[\"a.md\"]");
        assert!(!state.files.contains_key(".private/secret.md"));
        assert!(!state.files.contains_key("link.md"));
        assert!(!state.files.contains_key(".sync_meta.json"));
    }

    #[test]
    fn upload_uses_preconditions_and_preserves_a_concurrent_remote_edit() {
        let dir = TestDir::new();
        let server = MockDav::new(&[("a.md", b"base")]);
        run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        std::fs::write(dir.0.join("a.md"), b"local edit").unwrap();
        server.state.lock().unwrap().race_on_put = true;
        let report = run(push_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        assert_eq!(report.uploaded, 0);
        assert_eq!(report.conflicts, 1);
        assert_eq!(
            server.state.lock().unwrap().files["a.md"],
            b"concurrent edit"
        );
        assert_eq!(std::fs::read(dir.0.join("a.md")).unwrap(), b"local edit");
    }

    #[test]
    fn upload_without_etag_refuses_existing_remote_overwrite() {
        let dir = TestDir::new();
        let server = MockDav::new(&[("a.md", b"base")]);
        run(pull_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        std::fs::write(dir.0.join("a.md"), b"local edit").unwrap();
        server.state.lock().unwrap().omit_etag = true;
        let report = run(push_with_client(&server.client(), "PromptPocket", &dir.0)).unwrap();
        assert_eq!(report.uploaded, 0);
        assert_eq!(report.conflicts, 1);
        assert_eq!(server.state.lock().unwrap().files["a.md"], b"base");
    }

    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    struct TestDir(std::path::PathBuf);
    impl TestDir {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "pp-sync-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Default)]
    struct DavState {
        files: std::collections::BTreeMap<String, Vec<u8>>,
        requests: Vec<String>,
        failed_list: HashSet<String>,
        failed_get: HashSet<String>,
        race_on_put: bool,
        omit_etag: bool,
    }
    struct MockDav {
        host: String,
        state: std::sync::Arc<std::sync::Mutex<DavState>>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl MockDav {
        fn new(files: &[(&str, &[u8])]) -> Self {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let host = format!("http://{}", listener.local_addr().unwrap());
            let state = std::sync::Arc::new(std::sync::Mutex::new(DavState::default()));
            state
                .lock()
                .unwrap()
                .files
                .extend(files.iter().map(|(p, b)| (p.to_string(), b.to_vec())));
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let shared = state.clone();
            let stopped = stop.clone();
            let thread = std::thread::spawn(move || {
                while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(s) => s,
                        Err(_) => {
                            std::thread::sleep(std::time::Duration::from_millis(2));
                            continue;
                        }
                    };
                    // On macOS an accepted socket inherits O_NONBLOCK from the
                    // listener. Blocking reads are required for fragmented HTTP
                    // headers/body; otherwise a normal WouldBlock closes it.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                        .unwrap();
                    let mut input = Vec::new();
                    let mut buffer = [0u8; 4096];
                    let boundary = loop {
                        match stream.read(&mut buffer) {
                            Ok(0) | Err(_) => break None,
                            Ok(n) => input.extend_from_slice(&buffer[..n]),
                        }
                        if let Some(pos) = input.windows(4).position(|w| w == b"\r\n\r\n") {
                            break Some(pos + 4);
                        }
                    };
                    let Some(boundary) = boundary else {
                        continue;
                    };
                    let head = String::from_utf8_lossy(&input[..boundary]).into_owned();
                    let mut lines = head.lines();
                    let request: Vec<_> = lines.next().unwrap().split_whitespace().collect();
                    let headers: std::collections::BTreeMap<_, _> = lines
                        .filter_map(|line| line.split_once(':'))
                        .map(|(k, v)| (k.to_lowercase(), v.trim().to_string()))
                        .collect();
                    let length = headers
                        .get("content-length")
                        .and_then(|n| n.parse::<usize>().ok())
                        .unwrap_or(0);
                    while input.len() < boundary + length {
                        match stream.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => input.extend_from_slice(&buffer[..n]),
                        }
                    }
                    let path = urlencoding_decode(request[1]).unwrap();
                    let rel = path.trim_start_matches("/PromptPocket").trim_matches('/');
                    let mut state = shared.lock().unwrap();
                    state.requests.push(format!("{} {path}", request[0]));
                    let mut status = 200;
                    let mut extra = String::new();
                    let body = match request[0] {
                        "PROPFIND" if state.failed_list.contains(rel) => {
                            status = 500;
                            Vec::new()
                        }
                        "PROPFIND" => {
                            status = 207;
                            dav_listing(&state.files, rel).into_bytes()
                        }
                        "GET" if state.failed_get.contains(rel) => {
                            status = 500;
                            Vec::new()
                        }
                        "GET" => match state.files.get(rel) {
                            Some(bytes) => {
                                if !state.omit_etag {
                                    extra = format!("ETag: \"{}\"\r\n", fnv1a_hash(bytes));
                                }
                                bytes.clone()
                            }
                            None => {
                                status = 404;
                                Vec::new()
                            }
                        },
                        "PUT" => {
                            if state.race_on_put {
                                state.race_on_put = false;
                                state.files.insert(rel.into(), b"concurrent edit".to_vec());
                            }
                            let current = state
                                .files
                                .get(rel)
                                .map(|b| format!("\"{}\"", fnv1a_hash(b)));
                            let allowed = match headers.get("if-match") {
                                Some(tag) => current.as_ref() == Some(tag),
                                None => {
                                    headers.get("if-none-match").is_some_and(|tag| tag == "*")
                                        && current.is_none()
                                }
                            };
                            if allowed {
                                state.files.insert(rel.into(), input[boundary..].to_vec());
                                status = 201;
                            } else {
                                status = 412;
                            }
                            Vec::new()
                        }
                        "MKCOL" => {
                            status = 201;
                            Vec::new()
                        }
                        _ => {
                            status = 405;
                            Vec::new()
                        }
                    };
                    let response = format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n", body.len());
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(&body);
                }
            });
            Self {
                host,
                state,
                stop,
                thread: Some(thread),
            }
        }
        fn client(&self) -> Client {
            ClientBuilder::new()
                .set_host(self.host.clone())
                .set_auth(Auth::Anonymous)
                .build()
                .unwrap()
        }
        fn set(&self, rel: &str, bytes: &[u8]) {
            self.state
                .lock()
                .unwrap()
                .files
                .insert(rel.into(), bytes.to_vec());
        }
    }
    impl Drop for MockDav {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            self.thread.take().unwrap().join().unwrap();
        }
    }
    fn dav_listing(files: &std::collections::BTreeMap<String, Vec<u8>>, dir: &str) -> String {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut entries = std::collections::BTreeMap::new();
        for (path, bytes) in files {
            if let Some(tail) = path.strip_prefix(&prefix) {
                if let Some((folder, _)) = tail.split_once('/') {
                    entries.insert(format!("{prefix}{folder}/"), None);
                } else {
                    entries.insert(path.clone(), Some(bytes.len()));
                }
            }
        }
        let mut xml = String::from("<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\">");
        xml.push_str(&format!("<d:response><d:href>/PromptPocket/{prefix}</d:href><d:propstat><d:status>HTTP/1.1 200 OK</d:status><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>"));
        for (path, size) in entries {
            let resource = if size.is_none() {
                "<d:collection/>"
            } else {
                ""
            };
            xml.push_str(&format!("<d:response><d:href>/PromptPocket/{path}</d:href><d:propstat><d:status>HTTP/1.1 200 OK</d:status><d:prop><d:resourcetype>{resource}</d:resourcetype><d:getlastmodified>Wed, 10 Apr 2019 14:00:00 GMT</d:getlastmodified><d:getcontenttype>text/plain</d:getcontenttype><d:getcontentlength>{}</d:getcontentlength></d:prop></d:propstat></d:response>",size.unwrap_or(0)));
        }
        xml.push_str("</d:multistatus>");
        xml
    }

    #[test]
    fn test_sanitize_remote_path() {
        assert_eq!(sanitize_remote_path("/PromptPocket/"), "PromptPocket");
        assert_eq!(sanitize_remote_path("PromptPocket"), "PromptPocket");
        assert_eq!(sanitize_remote_path(""), "");
    }

    #[test]
    fn test_urlencoding_decode() {
        assert_eq!(
            urlencoding_decode("/dav/PromptPocket/%E5%86%99%E4%BD%9C/a.md"),
            Some("/dav/PromptPocket/写作/a.md".to_string())
        );
        assert_eq!(
            urlencoding_decode("/dav/a/b.md"),
            Some("/dav/a/b.md".to_string())
        );
    }

    #[test]
    fn test_extract_rel_path() {
        assert_eq!(
            extract_rel_path("/dav/PromptPocket/%E5%86%99%E4%BD%9C/a.md", "PromptPocket"),
            Some("写作/a.md".to_string())
        );
        assert_eq!(extract_rel_path("/dav/PromptPocket/", "PromptPocket"), None);
    }

    #[test]
    fn baseline_roundtrip_is_scoped_and_legacy_hashes_are_not_trusted() {
        let dir = TestDir::new();
        let mut meta = SyncMeta::new("account-a");
        meta.files.insert("写作/a.md".into(), "原文".into());
        save_sync_meta(&dir.0, &meta).unwrap();
        assert_eq!(
            load_sync_meta(&dir.0, "account-a").unwrap().files["写作/a.md"],
            "原文"
        );
        assert!(load_sync_meta(&dir.0, "account-b")
            .unwrap()
            .files
            .is_empty());
        std::fs::write(dir.0.join(".sync_meta.json"), r#"{"a.md":12345}"#).unwrap();
        assert!(load_sync_meta(&dir.0, "account-a")
            .unwrap()
            .files
            .is_empty());
    }

    #[test]
    fn sync_whitelist_and_remote_paths_reject_unsafe_or_internal_entries() {
        for rel in [
            "a.md",
            "写作/a.md",
            ".order.json",
            ".category-order.json",
            "写作/.order.json",
        ] {
            assert!(is_sync_file(rel), "{rel}");
        }
        for rel in [
            ".trash/a.md",
            ".cache/a.md",
            "a/.hidden.md",
            "../a.md",
            "a/../../b.md",
            "a\\b.md",
            "a/.sync_meta.json",
            "~a.md",
            "a.remote-conflict-1.md",
            "x.txt",
        ] {
            assert!(!is_sync_file(rel), "{rel}");
        }
        assert!(extract_rel_path("/dav/PromptPocket/%2e%2e/escape.md", "PromptPocket").is_none());
        assert!(extract_rel_path("/dav/PromptPocket/a%5Cb.md", "PromptPocket").is_none());
        assert_eq!(
            remote_path("PromptPocket", "写作/a #%.md"),
            "/PromptPocket/%E5%86%99%E4%BD%9C/a%20%23%25.md"
        );
    }
}
