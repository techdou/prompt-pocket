// 坚果云 WebDAV 同步模块
//
// 架构：本地缓存 + 手动双向同步（用户显式点上传/下载）。
// - 所有 UI 读写仍走本地缓存（store.rs），保证瞬间响应
// - 本模块负责本地缓存 ↔ 坚果云的双向同步
//
// 同步语义（2026-08 对账后）：
// - 上传（push）：本地为准。上传变更文件 + 删除传播
//   （.sync_deleted.json 里的 tombstone 逐条发远程 DELETE）
// - 下载（pull）：远程为准，但尊重本地删除。
//   tombstone 命中且远程大小未变 → 跳过（防"删了又复活"）；
//   远程大小变了 → 视为远程新版本，下载。覆盖本地文件前先备份到 .trash/
// - 排序文件（.order.json / .category-order.json）双向都同步
//
// 速率限制：坚果云约 600 次/30 分钟，本工具规模（几十个文件）够用。

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
    client
        .list(&format!("/{}/", encode_segments(&root)), Depth::Number(0))
        .await
        .map_err(|e| format!("连接失败，请检查账号/应用密码/路径: {e}"))?;
    Ok(())
}

/// 全量拉取：把远程的文件同步到本地缓存
/// 策略：远程为准，但尊重本地删除（tombstone）。
/// - 远程有、本地无 → 下载；但 tombstone 命中且远程大小未变 → 跳过（防复活）
/// - 本地有、大小不同 → 下载覆盖，覆盖前把本地旧文件备份到 .trash/
/// - 远程没有、本地有 → 备份到 .trash 后删除（clean_local_extra）
pub async fn pull_from_remote(cfg: &CloudConfig, local_dir: &Path) -> Result<SyncReport, String> {
    let client = build_client(cfg).map_err(|e| format!("客户端构建失败: {e}"))?;
    let root = sanitize_remote_path(&cfg.remote_root);

    // 确保远程根目录存在
    let _ = client.mkcol(&format!("/{root}")).await;

    let mut remote_files: HashSet<String> = HashSet::new();
    let mut downloaded = 0u32;
    let mut skipped = 0u32;
    let mut errors: Vec<String> = Vec::new();

    // 本地删除记录：命中且远程大小未变的不下载（防复活）
    let mut tombstones = crate::store::load_tombstones(local_dir);

    // 关键修复：坚果云 WebDAV 不支持 Depth::Infinity（静默降级为只返回一层），
    // 必须用 Depth::Number(1) 逐层递归遍历（与 Obsidian Remotely Save / rclone 同策略）。
    // walk_remote 返回 (文件列表, 错误列表)，文件已是去 .trash、解码后的相对路径。
    let files = walk_remote(&client, &root, &mut errors).await;

    for file in files {
        remote_files.insert(file.rel.clone());

        let local_path = local_dir.join(&file.rel);
        let local_size = std::fs::metadata(&local_path).ok().map(|m| m.len() as i64);

        // tombstone 判定：本地曾主动删除该文件
        if let Some(tomb) = tombstones.get(&file.rel) {
            if tomb.size as i64 == file.content_length {
                // 远程还是删除时的那个版本 → 不复活，跳过
                skipped += 1;
                continue;
            }
            // 远程内容变了（其它设备推了新版本）→ 视为新内容，下载并清除 tombstone
            let _ = crate::store::remove_tombstone(local_dir, &file.rel);
            tombstones.remove(&file.rel);
        }

        // 内容校对：大小不同或本地不存在才下载。
        // 注意：同字节数但内容不同（改一字删一字）无法检出，属已知取舍——
        // 完整解决需要远程 ETag/mtime 指纹，坚果云对两者的支持都不稳定。
        let need_download = match local_size {
            Some(size) => size != file.content_length,
            None => true, // 本地不存在
        };

        if need_download {
            // 覆盖前先备份本地旧文件（本地可能有未上传的修改）
            if local_path.exists() {
                let _ = crate::store::move_to_trash(local_dir, &local_path);
            }
            if let Err(e) = download_file(&client, &root, &file.rel, &local_path).await {
                errors.push(format!("{}: {e}", file.rel));
                continue;
            }
            downloaded += 1;
        } else {
            skipped += 1;
        }
    }

    // 清理本地多余文件（远程已删）—— 排除 .trash
    // 防御：如果 remote_files 为空（可能 PROPFIND 解析失败），不执行清理，
    // 避免把本地真实文件全部误移到 .trash
    let mut deleted = 0u32;
    if !remote_files.is_empty() {
        clean_local_extra(local_dir, &remote_files, Path::new(""), &mut deleted)?;
    } else {
        // 远程列表为空：记录警告，让用户知道可能有连接/解析问题
        errors.push("警告：未获取到远程文件列表，跳过本地清理（可能网络或解析问题）".to_string());
    }

    Ok(SyncReport {
        downloaded,
        skipped,
        deleted,
        uploaded: 0,
        deleted_remote: 0,
        errors,
    })
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub downloaded: u32,
    pub skipped: u32,
    pub deleted: u32,
    pub uploaded: u32,
    /// push 时传播到云端的删除数（tombstone → 远程 DELETE）
    pub deleted_remote: u32,
    pub errors: Vec<String>,
}

/// 远程文件（已解码、已去根前缀、已过滤 .trash 的相对路径）
struct RemoteFile {
    rel: String,
    content_length: i64,
}

/// 用 Depth::Number(1) 递归遍历远程目录树。
///
/// 关键背景：坚果云 WebDAV 不支持 Depth::Infinity——发 infinity 时服务端
/// 静默降级成只返回一层（实测 infinity 与 depth=1 返回字节完全一致），
/// 导致 `pull_from_remote` 永远看不到任何 .md 文件。
///
/// 解法（与 Obsidian Remotely Save / rclone 一致）：逐层 PROPFIND depth=1，
/// 遇到文件夹就递归再列一层，把整棵树走完。坚果云 600 次/30 分钟的限速
/// 对本工具的规模（几个分类、几十个文件）完全够用。
///
/// - 跳过 `.trash` 目录（含其所有后代）
/// - 跳过根目录自身（depth=1 会把被列目录自己也返回一次）
/// - 单个目录列举失败不中断整树：记录到 errors，继续其它目录
async fn walk_remote(client: &Client, root: &str, errors: &mut Vec<String>) -> Vec<RemoteFile> {
    let mut files: Vec<RemoteFile> = Vec::new();
    // 待访问的远程相对目录路径队列（相对 root，空串表示根目录）
    let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    queue.push_back(String::new());

    while let Some(rel_dir) = queue.pop_front() {
        // 该层的请求路径：根用 "/{root}/"，子目录用 "/{root}/{rel_dir}/"（逐段 URL 编码）
        let req_path = if rel_dir.is_empty() {
            format!("/{}/", encode_segments(&root))
        } else {
            format!("/{}/{}/", encode_segments(&root), encode_segments(&rel_dir))
        };

        let entities = match client.list(&req_path, Depth::Number(1)).await {
            Ok(es) => es,
            Err(e) => {
                // 单层列举失败：记录后继续其它分支，不让整次同步崩溃
                let label = if rel_dir.is_empty() {
                    "/".to_string()
                } else {
                    format!("{rel_dir}/")
                };
                errors.push(format!("列出远程目录 {label} 失败: {e}"));
                continue;
            }
        };

        for entity in entities {
            match entity {
                ListEntity::File(file) => {
                    let Some(rel) = extract_rel_path(&file.href, root) else {
                        continue;
                    };
                    // 过滤 .trash 及任何 . 开头的目录（防御）
                    if is_trash_or_hidden_rel(&rel) {
                        continue;
                    }
                    files.push(RemoteFile {
                        rel,
                        content_length: file.content_length,
                    });
                }
                ListEntity::Folder(folder) => {
                    let Some(rel) = extract_rel_path(&folder.href, root) else {
                        continue;
                    };
                    // 跳过根自身（depth=1 会把被列目录自身作为 Folder 返回一次）
                    if rel == rel_dir || rel.is_empty() {
                        continue;
                    }
                    // 过滤 .trash / 隐藏目录，不递归进去
                    if is_trash_or_hidden_rel(&rel) {
                        continue;
                    }
                    queue.push_back(rel);
                }
            }
        }
    }

    files
}

/// 判断相对路径是否落在 .trash 或任意隐藏目录下（不参与同步）。
/// 例外放行：排序文件 .order.json / .category-order.json 要双向同步——
/// 旧版把 `.` 开头全部过滤，导致排序文件上传后任何设备都拉不回来。
fn is_trash_or_hidden_rel(rel: &str) -> bool {
    // 根目录下的排序白名单文件：放行
    if rel == crate::store::ORDER_FILE || rel == crate::store::CATEGORY_ORDER_FILE {
        return false;
    }
    rel.split('/')
        .any(|seg| seg == ".trash" || seg.starts_with('.'))
}

/// 全量上传：本地为准——上传变更文件 + 删除传播。
/// 1. 上传：本地 .md / .order.json / .category-order.json 与上次哈希比对，变了才 PUT
/// 2. 删除传播：.sync_deleted.json 里的 tombstone 逐条发远程 DELETE，
///    成功（或云端本就 404）后从清单移除
/// 内容校对（杜绝无限制重复上传）：
///   上传前算本地内容哈希(FNV-1a)，与 .sync_meta.json 里记录的「上次上传哈希」比对：
///   - 哈希相同 → 内容未变 → 跳过
///   - 哈希不同 或 无记录 → 上传，上传成功后更新记录
pub async fn push_all_to_remote(cfg: &CloudConfig, local_dir: &Path) -> Result<SyncReport, String> {
    let client = build_client(cfg).map_err(|e| format!("客户端构建失败: {e}"))?;
    let root = sanitize_remote_path(&cfg.remote_root);

    // 确保远程根目录存在
    let _ = client.mkcol(&format!("/{root}")).await;

    // 读取上次上传记录 { 路径: 哈希 }
    let mut sync_meta: std::collections::HashMap<String, u64> = load_sync_meta(local_dir);

    let mut uploaded = 0u32;
    let mut skipped = 0u32;
    let mut deleted_remote = 0u32;
    let mut errors: Vec<String> = Vec::new();

    // 遍历本地所有 .md 文件 + 两个排序文件
    for entry in walkdir::WalkDir::new(local_dir)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            // 排除 .trash / 同步元数据自身（tombstone 清单是本地状态，不上传）
            let name = e.file_name().to_string_lossy();
            name != crate::store::TRASH_DIR
                && name != ".sync_meta.json"
                && name != crate::store::TOMBSTONE_FILE
        })
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str());
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        let is_md = ext == Some("md");
        let is_order = name == crate::store::ORDER_FILE || name == crate::store::CATEGORY_ORDER_FILE;
        if !is_md && !is_order {
            continue;
        }
        if name.starts_with('~') {
            continue;
        }

        let rel = path.strip_prefix(local_dir).map_err(|e| e.to_string())?;
        let rel_unix = rel.to_string_lossy().replace('\\', "/");

        let local_content = match std::fs::read(path) {
            Ok(c) => c,
            Err(e) => {
                errors.push(format!("{rel_unix}（读文件）: {e}"));
                continue;
            }
        };

        // 内容校对：本地哈希 vs 上次上传记录的哈希
        let local_hash = fnv1a_hash(&local_content);
        if let Some(&last_hash) = sync_meta.get(&rel_unix) {
            if last_hash == local_hash {
                skipped += 1;
                continue; // 内容未变，跳过
            }
        }

        // 确保远程目录存在
        if let Err(e) = ensure_remote_dirs(&client, &root, &rel_unix).await {
            errors.push(format!("{rel_unix}（建目录）: {e}"));
            continue;
        }

        match client
            .put(&remote_url(&root, &rel_unix), local_content)
            .await
        {
            Ok(()) => {
                uploaded += 1;
                // 上传成功，更新记录
                sync_meta.insert(rel_unix, local_hash);
            }
            Err(e) => errors.push(format!("{rel_unix}: {e}")),
        }
    }

    // 删除传播：tombstone 清单逐条发远程 DELETE
    let mut tombstones = crate::store::load_tombstones(local_dir);
    if !tombstones.is_empty() {
        let rels: Vec<String> = tombstones.keys().cloned().collect();
        for rel in rels {
            match client.delete(&remote_url(&root, &rel)).await {
                Ok(()) => {
                    deleted_remote += 1;
                    tombstones.remove(&rel);
                    sync_meta.remove(&rel);
                }
                Err(e) => {
                    // 云端本就没有该文件（从未上传/已被其它设备删）→ 目的已达，清除记录
                    if e.to_string().contains("404") || e.to_string().contains("Not Found") {
                        tombstones.remove(&rel);
                        sync_meta.remove(&rel);
                    } else {
                        // 网络/权限错误：保留 tombstone，下次 push 重试
                        errors.push(format!("{rel}（删除传播）: {e}"));
                    }
                }
            }
        }
        let _ = crate::store::save_tombstones(local_dir, &tombstones);
    }

    // 持久化更新后的记录
    save_sync_meta(local_dir, &sync_meta);

    Ok(SyncReport {
        uploaded,
        skipped,
        deleted_remote,
        errors,
        ..Default::default()
    })
}

/// 读取 .sync_meta.json（记录每个文件上次上传时的内容哈希）
fn load_sync_meta(local_dir: &Path) -> std::collections::HashMap<String, u64> {
    let path = local_dir.join(".sync_meta.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 写入 .sync_meta.json（原子写，防同步中断留下半截 JSON）
fn save_sync_meta(local_dir: &Path, meta: &std::collections::HashMap<String, u64>) {
    let path = local_dir.join(".sync_meta.json");
    if let Ok(json) = serde_json::to_string_pretty(meta) {
        let _ = crate::store::write_atomic(&path, json.as_bytes());
    }
}

/// FNV-1a 64 位哈希（轻量内容指纹，无外部依赖，对内容任何变化敏感）
fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

// ────────────────────────────────────────────────
// 辅助函数
// ────────────────────────────────────────────────

/// 规范化远程路径：去首尾斜杠
fn sanitize_remote_path(s: &str) -> String {
    s.trim_matches('/').to_string()
}

/// percent-encode 单段路径：保留 RFC 3986 unreserved（A-Za-z0-9 - _ . ~），
/// 其余逐字节转 %XX。与 urlencoding_decode 对称。
/// 标题里的空格、#、?、% 等字符不编码会在 URL 里产生歧义（# 被当 fragment 截断）。
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 对多段路径逐段编码（保留 / 分隔符）
fn encode_segments(path: &str) -> String {
    path.split('/')
        .map(urlencoding_encode)
        .collect::<Vec<_>>()
        .join("/")
}

/// 拼接远程文件 URL：root 和 rel 都逐段编码
fn remote_url(root: &str, rel: &str) -> String {
    format!("/{}/{}", encode_segments(root), encode_segments(rel))
}

/// 紧凑时间戳，用于备份文件名（如 20260628T153000）
fn now_iso_compact() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let rem = secs % 86400;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{:04}{:02}{:02}T{:02}{:02}{:02}", y, mo, d, h, m, s)
}

/// Howard Hinnant 的 days_from_civil 逆运算
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 从 WebDAV href 中提取相对于 remote_root 的路径
/// href 形如 /dav/PromptPocket/%E5%86%99%E4%BD%9C/a.md
/// 返回 写作/a.md（URL 解码 + 去掉根前缀）
fn extract_rel_path(href: &str, root: &str) -> Option<String> {
    // URL 解码
    let decoded = urlencoding_decode(href)?;
    // 找到 root 之后的部分
    let marker = format!("/{root}/");
    let idx = decoded.find(&marker)?;
    let after = &decoded[idx + marker.len()..];
    if after.is_empty() {
        return None;
    }
    Some(after.to_string())
}

/// 简单的 URL 解码（处理 %XX），正确处理多字节 UTF-8
fn urlencoding_decode(s: &str) -> Option<String> {
    let mut bytes_out: Vec<u8> = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                bytes_out.push(byte);
                i += 3;
                continue;
            }
        }
        bytes_out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(bytes_out).ok()
}

/// 下载单个远程文件到本地
async fn download_file(
    client: &Client,
    root: &str,
    rel: &str,
    local_path: &Path,
) -> Result<(), String> {
    if let Some(parent) = local_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let resp = client
        .get(&remote_url(root, rel))
        .await
        .map_err(|e| format!("GET 失败: {e}"))?;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    crate::store::write_atomic(local_path, &bytes).map_err(|e| e.to_string())?;
    Ok(())
}

/// 清理本地缓存中"远程已不存在"的 .md 文件
/// 关键修复：
/// 1. 跳过 .trash 目录（不递归清理备份文件）
/// 2. 跳过隐藏文件（以 . 开头）
fn clean_local_extra(
    local_dir: &Path,
    remote_files: &HashSet<String>,
    current_rel: &Path,
    deleted: &mut u32,
) -> Result<(), String> {
    let scan_dir = if current_rel.as_os_str().is_empty() {
        local_dir.to_path_buf()
    } else {
        local_dir.join(current_rel)
    };

    let entries = match std::fs::read_dir(&scan_dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // 跳过 .trash 和所有隐藏目录/文件（不参与清理）
        if name.starts_with('.') {
            continue;
        }

        if path.is_dir() {
            let sub_rel = current_rel.join(&name);
            clean_local_extra(local_dir, remote_files, &sub_rel, deleted)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let rel_unix = path
                .strip_prefix(local_dir)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if !remote_files.contains(&rel_unix) {
                // 备份到 .trash/ 后删除（避免永久丢失）
                let trash_dir = local_dir.join(".trash");
                let _ = std::fs::create_dir_all(&trash_dir);
                let backup_name = format!(
                    "{}_{}.md",
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("untitled"),
                    now_iso_compact()
                );
                let backup_path = trash_dir.join(backup_name);
                if std::fs::rename(&path, &backup_path).is_err() {
                    let _ = std::fs::remove_file(&path);
                }
                *deleted += 1;
            }
        }
    }
    Ok(())
}

/// 逐级创建远程目录（如 写作/子目录/a.md 会先 mkcol 写作 再 mkcol 写作/子目录）
async fn ensure_remote_dirs(client: &Client, root: &str, rel_unix: &str) -> Result<(), String> {
    // 取出文件所在的目录路径
    let parent = match rel_unix.rfind('/') {
        Some(i) => &rel_unix[..i],
        None => return Ok(()), // 文件在根目录，无需建目录
    };

    // 逐级 mkcol（忽略"已存在"错误）
    let mut acc = String::new();
    for part in parent.split('/') {
        if part.is_empty() {
            continue;
        }
        // 逐级 mkcol（忽略"已存在"错误）；目录名逐段 URL 编码
        acc = if acc.is_empty() {
            part.to_string()
        } else {
            format!("{acc}/{part}")
        };
        let _ = client.mkcol(&remote_url(&root, &acc)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 验证 clean_local_extra 跳过 .trash 目录（不清理备份文件）
    #[test]
    fn clean_local_extra_skips_trash() {
        let dir = std::env::temp_dir().join("pp_test_clean_trash");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // .trash 里放一个备份文件
        let trash = dir.join(".trash");
        std::fs::create_dir_all(&trash).unwrap();
        std::fs::write(trash.join("backup.md"), "备份内容").unwrap();
        // 真实分类里放一个文件
        std::fs::create_dir_all(dir.join("写作")).unwrap();
        std::fs::write(dir.join("写作").join("真实.md"), "内容").unwrap();

        // remote_files 为空（远程没有任何文件），clean 应该只清理真实文件，不动 .trash
        let mut deleted = 0u32;
        let remote_files: HashSet<String> = HashSet::new();
        clean_local_extra(&dir, &remote_files, std::path::Path::new(""), &mut deleted).unwrap();

        // 真实文件被移到 .trash（删除计数 +1）
        assert_eq!(deleted, 1, "应只删除 1 个真实文件");
        // .trash 里的备份文件仍然存在
        assert!(trash.join("backup.md").exists(), ".trash 备份不应被清理");
        // 现在有 2 个文件在 .trash（原备份 + 移入的真实文件）
        let trash_count = std::fs::read_dir(&trash).unwrap().count();
        assert_eq!(trash_count, 2, ".trash 应有 2 个文件");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 验证 clean_local_extra 正确匹配远程文件（不误删）
    #[test]
    fn clean_local_extra_keeps_matched() {
        let dir = std::env::temp_dir().join("pp_test_clean_keep");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("写作")).unwrap();
        std::fs::write(dir.join("写作").join("保留.md"), "内容").unwrap();

        let mut remote_files: HashSet<String> = HashSet::new();
        remote_files.insert("写作/保留.md".to_string());

        let mut deleted = 0u32;
        clean_local_extra(&dir, &remote_files, std::path::Path::new(""), &mut deleted).unwrap();

        assert_eq!(deleted, 0, "远程存在的文件不应被删除");
        assert!(dir.join("写作").join("保留.md").exists(), "文件应保留");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 验证 FNV-1a 哈希对内容变化敏感（相同内容同哈希，不同内容不同哈希）
    #[test]
    fn fnv1a_hash_detects_changes() {
        let h1 = fnv1a_hash(b"hello world");
        let h2 = fnv1a_hash(b"hello world");
        let h3 = fnv1a_hash(b"hello world!");
        assert_eq!(h1, h2, "相同内容应有相同哈希");
        assert_ne!(h1, h3, "不同内容应有不同哈希");
    }

    /// 验证 .sync_meta.json 的读写
    #[test]
    fn sync_meta_roundtrip() {
        let dir = std::env::temp_dir().join("pp_test_sync_meta");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut meta: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        meta.insert("写作/a.md".to_string(), 12345);
        meta.insert("编程/b.md".to_string(), 67890);
        save_sync_meta(&dir, &meta);

        let loaded = load_sync_meta(&dir);
        assert_eq!(loaded.get("写作/a.md"), Some(&12345));
        assert_eq!(loaded.get("编程/b.md"), Some(&67890));
        assert!(dir.join(".sync_meta.json").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 验证 walk_remote 的路径过滤：.trash 及隐藏目录下的文件应被排除
    /// 这是 walk_remote 不递归进 .trash、不收录隐藏文件的核心防线。
    #[test]
    fn is_trash_or_hidden_rel_filters_correctly() {
        // 应被排除（true）
        assert!(is_trash_or_hidden_rel(".trash/x.md"));
        assert!(is_trash_or_hidden_rel("写作/.trash/b.md"));
        assert!(is_trash_or_hidden_rel(".cache/y.md"));
        assert!(is_trash_or_hidden_rel("a/.hidden/b.md"));
        // 根目录下的隐藏文件
        assert!(is_trash_or_hidden_rel(".sync_meta.json"));
        assert!(is_trash_or_hidden_rel(".sync_deleted.json"));

        // 应保留（false）：正常分类路径
        assert!(!is_trash_or_hidden_rel("写作/a.md"));
        assert!(!is_trash_or_hidden_rel("编程/子目录/b.md"));
        assert!(!is_trash_or_hidden_rel("root.md"));
        assert!(!is_trash_or_hidden_rel("web服务/html-read.md"));

        // 排序白名单：要双向同步，不能当隐藏文件过滤掉
        assert!(!is_trash_or_hidden_rel(".order.json"));
        assert!(!is_trash_or_hidden_rel(".category-order.json"));
    }

    /// URL 编码：特殊字符转义 + 与 decode 对称 round-trip
    #[test]
    fn urlencoding_encode_decode_roundtrip() {
        // 特殊字符必须编码
        assert_eq!(urlencoding_encode("a b#c?d%e"), "a%20b%23c%3Fd%25e");
        // unreserved 不编码
        assert_eq!(urlencoding_encode("a-b_c.d~e"), "a-b_c.d~e");
        // 中文按 UTF-8 字节编码
        assert_eq!(urlencoding_encode("写"), "%E5%86%99");

        // 对称性：encode 后 decode 必须还原
        let cases = ["写作/a b.md", "编程/#1 清单.md", "100%.md", "web服务/html-read.md"];
        for c in cases {
            let encoded = urlencoding_encode(c);
            let decoded = urlencoding_decode(&encoded).unwrap();
            assert_eq!(decoded, c, "round-trip 失败: {c}");
        }
    }

    /// remote_url：root 和 rel 都编码，保留路径分隔
    #[test]
    fn remote_url_encodes_all_segments() {
        assert_eq!(remote_url("PromptPocket", "写作/a.md"), "/PromptPocket/%E5%86%99%E4%BD%9C/a.md");
        assert_eq!(
            remote_url("My Root", "笔记/#1.md"),
            "/My%20Root/%E7%AC%94%E8%AE%B0/%231.md"
        );
    }
}
