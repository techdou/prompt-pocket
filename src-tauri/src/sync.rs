// 同步模块（算法层）：本地缓存 ↔ 远程的双向对账。
//
// 架构：同步算法（本文件）与传输层（RemoteStore 实现）解耦——
// 换后端（坚果云 WebDAV / GitHub Contents API）只换传输实现，对账逻辑不动。
// - 所有 UI 读写仍走本地缓存（store.rs），保证瞬间响应
// - 本模块负责本地缓存 ↔ 远程存储的双向同步
//
// 同步语义（2026-08 对账后，与后端无关）：
// - 上传（push）：本地为准。上传变更文件 + 删除传播
//   （.sync_deleted.json 里的 tombstone 逐条发远程删除）
// - 下载（pull）：远程为准，但尊重本地删除。
//   tombstone 命中且远程大小未变 → 跳过（防"删了又复活"）；
//   远程大小变了 → 视为远程新版本，下载。覆盖本地文件前先备份到 .trash/
// - 排序文件（.order.json / .category-order.json）双向都同步

pub mod webdav;

pub use webdav::{CloudConfig, WebDavStore};

use std::collections::HashSet;
use std::path::Path;

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

#[derive(Debug, Default)]
pub struct SyncReport {
    pub downloaded: u32,
    pub skipped: u32,
    pub deleted: u32,
    pub uploaded: u32,
    /// push 时传播到云端的删除数（tombstone → 远程删除）
    pub deleted_remote: u32,
    pub errors: Vec<String>,
}

/// 远程文件（已解码、已去根前缀、已过滤 .trash 的相对路径）
pub struct RemoteFile {
    pub rel: String,
    pub content_length: i64,
}

/// 远程存储传输层：同步算法（pull/push 对账）只面向这个 trait 编程。
/// WebDAV 与 GitHub 各自实现；后端差异（建目录、路径编码、404 语义）
/// 全部收敛在实现内部，算法层无感知。
pub trait RemoteStore {
    /// 列出远程全部文件，返回 (文件列表, 非致命错误列表)。
    /// WebDAV 逐层列举，允许单个目录失败（记入错误列表继续）；
    /// GitHub Trees 一次请求拿全树，失败只能整体 Err。
    async fn list_all(&self) -> Result<(Vec<RemoteFile>, Vec<String>), String>;
    /// 下载单个文件到本地绝对路径（本地父目录由实现方创建）
    async fn download(&self, rel: &str, local_path: &Path) -> Result<(), String>;
    /// 上传单个文件（远程中间目录由实现方负责；GitHub 路径扁平，天然无需建目录）
    async fn upload(&self, rel: &str, content: Vec<u8>) -> Result<(), String>;
    /// 删除远程文件；远程本就不存在视为成功（删除传播语义：目的已达）
    async fn delete(&self, rel: &str) -> Result<(), String>;
    /// 连通性 + 凭据 + 远程根路径综合自检
    async fn test(&self) -> Result<(), String>;
    /// 确保远程根存在（WebDAV 建根 collection；GitHub 仓库天然存在，空实现）
    async fn ensure_root(&self) -> Result<(), String> {
        Ok(())
    }
}

/// 全量拉取：把远程的文件同步到本地缓存
/// 策略：远程为准，但尊重本地删除（tombstone）。
/// - 远程有、本地无 → 下载；但 tombstone 命中且远程大小未变 → 跳过（防复活）
/// - 本地有、大小不同 → 下载覆盖，覆盖前把本地旧文件备份到 .trash/
/// - 远程没有、本地有 → 备份到 .trash 后删除（clean_local_extra）
pub async fn pull_from_remote<S: RemoteStore>(
    store: &S,
    local_dir: &Path,
) -> Result<SyncReport, String> {
    // 确保远程根目录存在（GitHub 为空操作）
    let _ = store.ensure_root().await;

    let mut remote_files: HashSet<String> = HashSet::new();
    let mut downloaded = 0u32;
    let mut skipped = 0u32;
    let mut errors: Vec<String> = Vec::new();

    // 本地删除记录：命中且远程大小未变的不下载（防复活）
    let mut tombstones = crate::store::load_tombstones(local_dir);

    // 远程全树：文件列表 + 非致命错误（单目录列举失败等）
    let (files, walk_errors) = store.list_all().await?;
    errors.extend(walk_errors);

    for file in files {
        remote_files.insert(file.rel.clone());

        let local_path = local_dir.join(&file.rel);
        let local_size = std::fs::metadata(&local_path).ok().map(|m| m.len() as i64);

        // tombstone 判定：本地曾主动删除该文件
        if let Some(tomb) = tombstones.get(&file.rel) {
            // size=0 是"记录时文件已不存在"的失真值（旧版 rename_category 产生）：
            // 与远程大小必然不等，会被误判成"远程新版本"而复活。
            // 保守跳过（防复活优先），由下次 push 的删除传播收敛：删除成功后清除记录。
            if tomb.size == 0 || tomb.size as i64 == file.content_length {
                // 远程还是删除时的那个版本（或大小未知）→ 不复活，跳过
                skipped += 1;
                continue;
            }
            // 远程内容变了（其它设备推了新版本）→ 视为新内容，下载并清除 tombstone
            let _ = crate::store::remove_tombstone(local_dir, &file.rel);
            tombstones.remove(&file.rel);
        }

        // 内容校对：大小不同或本地不存在才下载。
        // 注意：同字节数但内容不同（改一字删一字）无法检出，属已知取舍——
        // 完整解决需要远程内容指纹（ETag/SHA），坚果云对 ETag/mtime 的支持都不稳定；
        // GitHub 实现天然有 blob SHA，留作后续增强点。
        let need_download = match local_size {
            Some(size) => size != file.content_length,
            None => true, // 本地不存在
        };

        if need_download {
            // 覆盖前先备份本地旧文件（本地可能有未上传的修改）
            if local_path.exists() {
                let _ = crate::store::move_to_trash(local_dir, &local_path);
            }
            if let Err(e) = store.download(&file.rel, &local_path).await {
                errors.push(format!("{}: {e}", file.rel));
                continue;
            }
            downloaded += 1;
        } else {
            skipped += 1;
        }
    }

    // 清理本地多余文件（远程已删）—— 排除 .trash
    // 防御：如果 remote_files 为空（可能列表请求/解析失败），不执行清理，
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

/// 全量上传：本地为准——上传变更文件 + 删除传播。
/// 1. 上传：本地 .md / .order.json / .category-order.json 与上次哈希比对，变了才传
/// 2. 删除传播：.sync_deleted.json 里的 tombstone 逐条发远程删除，
///    成功后从清单移除（远程本就没有时由实现方吞掉 404 视为成功）
///
/// 内容校对（杜绝无限制重复上传）：
///   上传前算本地内容哈希(FNV-1a)，与 .sync_meta.json 里记录的「上次上传哈希」比对：
///   - 哈希相同 → 内容未变 → 跳过
///   - 哈希不同 或 无记录 → 上传，上传成功后更新记录
pub async fn push_all_to_remote<S: RemoteStore>(
    store: &S,
    local_dir: &Path,
) -> Result<SyncReport, String> {
    // 确保远程根目录存在（GitHub 为空操作）
    let _ = store.ensure_root().await;

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

        // 上传（远程中间目录由实现方负责）
        match store.upload(&rel_unix, local_content).await {
            Ok(()) => {
                uploaded += 1;
                // 上传成功，更新记录
                sync_meta.insert(rel_unix, local_hash);
            }
            Err(e) => errors.push(format!("{rel_unix}: {e}")),
        }
    }

    // 删除传播：tombstone 清单逐条发远程删除
    let mut tombstones = crate::store::load_tombstones(local_dir);
    if !tombstones.is_empty() {
        let rels: Vec<String> = tombstones.keys().cloned().collect();
        for rel in rels {
            match store.delete(&rel).await {
                Ok(()) => {
                    deleted_remote += 1;
                    tombstones.remove(&rel);
                    sync_meta.remove(&rel);
                }
                Err(e) => {
                    // 网络/权限错误：保留 tombstone，下次 push 重试
                    errors.push(format!("{rel}（删除传播）: {e}"));
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

/// 判断相对路径是否落在 .trash 或任意隐藏目录下（不参与同步）。
/// 例外放行：排序文件 .order.json / .category-order.json 要双向同步——
/// 旧版把 `.` 开头全部过滤，导致排序文件上传后任何设备都拉不回来。
pub(crate) fn is_trash_or_hidden_rel(rel: &str) -> bool {
    // 根目录下的排序白名单文件：放行
    if rel == crate::store::ORDER_FILE || rel == crate::store::CATEGORY_ORDER_FILE {
        return false;
    }
    rel.split('/')
        .any(|seg| seg == ".trash" || seg.starts_with('.'))
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
                // 备份到 .trash/ 后删除（避免永久丢失）；与 store::move_to_trash
                // 共用同一实现（含同秒同名备份的防覆盖序号）
                let _ = crate::store::move_to_trash(local_dir, &path);
                *deleted += 1;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
