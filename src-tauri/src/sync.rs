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

pub mod github;
pub mod webdav;

pub use github::{GitHubConfig, GitHubStore};
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

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
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
// Send + Sync：tauri 命令层把 store 跨 await 持有，泛型 future 必须
// 可证跨线程安全，否则命令宏以「unsatisfied trait bounds」拒绝编译
pub trait RemoteStore: Send + Sync {
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
/// - 本地有、大小不同 → 先下临时文件，新内容完整到手才备份旧文件并替换
/// - 远程没有、本地有 → 备份到 .trash 后删除（clean_local_extra）；
///   列举不完整（部分目录失败）时跳过清理，避免误删未列出的文件
/// - 下载成功后把内容指纹回写账本：随后的 push 增量判定知道"这就是远端内容"，
///   不再整批重传刚下载的文件
pub async fn pull_from_remote<S: RemoteStore>(
    store: &S,
    local_dir: &Path,
    target_key: &str,
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
    // 列举完整性单独保留：errors 里还会混入下载失败等与完整性无关的条目
    let list_incomplete = !walk_errors.is_empty();
    errors.extend(walk_errors);

    // 账本按目标隔离；pull 回写下载内容的指纹
    let mut targets = load_sync_meta_targets(local_dir);
    let mut baseline = take_target_baseline(&mut targets, target_key);
    let mut meta_dirty = false;

    for file in files {
        remote_files.insert(file.rel.clone());

        let Some(local_path) = safe_join(local_dir, &file.rel) else {
            errors.push(format!("{}（非法远程路径，跳过）", file.rel));
            continue;
        };
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
            // 先下到临时文件：新内容完整到手才动旧文件。旧流程先备份再下载，
            // 下载失败时原文件已被挪进 .trash，列表"消失"要靠用户翻回收目录
            let file_name = local_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let tmp_path = local_path.with_file_name(format!("{file_name}.syncdl"));
            if let Some(parent) = local_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match store.download(&file.rel, &tmp_path).await {
                Ok(()) => {
                    if local_path.exists() {
                        if let Err(e) = crate::store::move_to_trash(local_dir, &local_path) {
                            errors.push(format!("{}（备份旧文件失败，保持原文件不动）: {e}", file.rel));
                            let _ = std::fs::remove_file(&tmp_path);
                            continue;
                        }
                    }
                    if let Err(e) = std::fs::rename(&tmp_path, &local_path) {
                        errors.push(format!("{}（落盘）: {e}", file.rel));
                        continue;
                    }
                    downloaded += 1;
                    // 回写指纹：push 增量判定据此跳过未变更的刚下载文件
                    if let Ok(content) = std::fs::read(&local_path) {
                        baseline.insert(file.rel.clone(), fnv1a_hash(&content));
                        meta_dirty = true;
                    }
                }
                Err(e) => {
                    errors.push(format!("{}: {e}", file.rel));
                    let _ = std::fs::remove_file(&tmp_path);
                }
            }
        } else {
            skipped += 1;
        }
    }

    // 清理本地多余文件（远程已删）—— 排除 .trash
    // 防御 1：remote_files 为空（可能列表请求/解析失败），不执行清理，
    // 避免把本地真实文件全部误移到 .trash
    // 防御 2：列举不完整（部分目录失败）同样不清理——未列出的文件会被误当
    // "远程已删"整批移走，一次局部网络失败等于丢一个分类
    let mut deleted = 0u32;
    let mut deleted_rels: Vec<String> = Vec::new();
    if remote_files.is_empty() {
        errors.push("警告：未获取到远程文件列表，跳过本地清理（可能网络或解析问题）".to_string());
    } else if list_incomplete {
        errors.push("警告：远程目录部分列举失败，跳过本地清理（避免误删未列出的文件）".to_string());
    } else {
        clean_local_extra(local_dir, &remote_files, Path::new(""), &mut deleted, &mut deleted_rels)?;
        for rel in &deleted_rels {
            if baseline.remove(rel).is_some() {
                meta_dirty = true;
            }
        }
    }

    if meta_dirty {
        targets.insert(target_key.to_string(), baseline);
        save_sync_meta_targets(local_dir, &targets);
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
    target_key: &str,
) -> Result<SyncReport, String> {
    // 确保远程根目录存在（GitHub 为空操作）
    let _ = store.ensure_root().await;

    // 读取上次上传记录（按远程目标隔离的账本）
    let mut targets = load_sync_meta_targets(local_dir);
    let mut sync_meta = take_target_baseline(&mut targets, target_key);

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
        // 空分类占位文件随同步传输（跨设备保留分类骨架）
        let is_keep = name == ".gitkeep";
        if !is_md && !is_order && !is_keep {
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

    // 删除传播：tombstone 清单逐条发远程删除。
    // 按目标标记确认：一个目标删完不能消费整个记录——多目标场景下另一目标
    // 的副本会因此永远残留；已确认过的目标跳过（幂等）
    let mut tombstones = crate::store::load_tombstones(local_dir);
    if !tombstones.is_empty() {
        let rels: Vec<String> = tombstones.keys().cloned().collect();
        for rel in rels {
            if tombstones[&rel].done_targets.contains(target_key) {
                continue; // 本目标已确认过，不重复发删除
            }
            match store.delete(&rel).await {
                Ok(()) => {
                    deleted_remote += 1;
                    let _ = crate::store::mark_tombstone_done(local_dir, &rel, target_key);
                    tombstones = crate::store::load_tombstones(local_dir);
                    sync_meta.remove(&rel);
                }
                Err(e) => {
                    // 网络/权限错误：保留 tombstone，下次 push 重试
                    errors.push(format!("{rel}（删除传播）: {e}"));
                }
            }
        }
    }

    // 持久化更新后的记录（保留其他目标的账本切片一起写回）
    targets.insert(target_key.to_string(), sync_meta);
    save_sync_meta_targets(local_dir, &targets);

    Ok(SyncReport {
        uploaded,
        skipped,
        deleted_remote,
        errors,
        ..Default::default()
    })
}

/// 远程目标标识：账本按目标隔离——切后端 / 换仓库 / 换远程目录各自独立基线，
/// 否则 A 目标的上传记录会让 B 目标误跳过未上传的文件（切换后显示"完成"实则缺文件），
/// 删除记录也会被第一个消费的目标顺手带走
pub fn target_key_webdav(cfg: &CloudConfig) -> String {
    format!(
        "webdav:{}@{}",
        cfg.username,
        cfg.remote_root.trim_matches('/')
    )
}

pub fn target_key_github(cfg: &GitHubConfig) -> String {
    format!(
        "github:{}@{}:{}",
        cfg.repo,
        cfg.branch,
        cfg.prefix.trim_matches('/')
    )
}

/// 账本文件名（v2.3 起结构升级：按远程目标分命名空间）
const SYNC_META_FILE: &str = ".sync_meta.json";
/// 旧版（≤v2.2.1）平铺账本的收编标记
const LEGACY_TARGET: &str = "__legacy__";

type SyncMetaTargets = std::collections::HashMap<String, std::collections::HashMap<String, u64>>;

/// 读取账本全量目标表。
/// 兼容旧平铺格式 `{rel: hash}`（无法归属目标）：整体标记为 __legacy__，
/// 由 take_target_baseline 收编进首个发起同步的目标（单后端用户无缝迁移）
fn load_sync_meta_targets(local_dir: &Path) -> SyncMetaTargets {
    let path = local_dir.join(SYNC_META_FILE);
    let Ok(s) = std::fs::read_to_string(&path) else {
        return Default::default();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else {
        return Default::default();
    };
    let Some(obj) = v.as_object() else {
        return Default::default();
    };
    if !obj.is_empty() && obj.values().all(|x| x.is_number()) {
        // 旧平铺格式
        let legacy = obj
            .iter()
            .filter_map(|(k, v)| v.as_u64().map(|h| (k.clone(), h)))
            .collect();
        let mut m = SyncMetaTargets::new();
        m.insert(LEGACY_TARGET.to_string(), legacy);
        return m;
    }
    serde_json::from_value(v).unwrap_or_default()
}

/// 写入账本（原子写，防同步中断留下半截 JSON）
fn save_sync_meta_targets(local_dir: &Path, targets: &SyncMetaTargets) {
    let path = local_dir.join(SYNC_META_FILE);
    if let Ok(json) = serde_json::to_string_pretty(targets) {
        let _ = crate::store::write_atomic(&path, json.as_bytes());
    }
}

/// 取出目标自己的基线。旧格式（≤v2.2.1 平铺）数据无法证明归属哪个目标——
/// 收编给当前目标会把 A 的基线错配给 B，导致 B 漏传整批文件且错误基线被
/// 持久化。直接丢弃：升级后首次 push 全量重传一次（一次性代价，绝对正确）
fn take_target_baseline(
    targets: &mut SyncMetaTargets,
    target_key: &str,
) -> std::collections::HashMap<String, u64> {
    targets.remove(LEGACY_TARGET);
    targets.remove(target_key).unwrap_or_default()
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

/// 同步层的受限路径拼接：远程相对路径先做词法校验——拒绝反斜杠、盘符
/// 冒号和 `..` 越界段，杜绝异常/被投毒的远端路径把写盘目标逃逸出数据目录
/// （命令层有 resolve_abs，同步层此前直接 join，是校验缺口）
pub(crate) fn safe_join(local_dir: &Path, rel: &str) -> Option<std::path::PathBuf> {
    const BS: char = '\\';
    if rel.contains(BS) || rel.contains(':') {
        return None;
    }
    let mut out = local_dir.to_path_buf();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => return None,
            s => out.push(s),
        }
    }
    Some(out)
}

/// 判断相对路径是否落在 .trash 或任意隐藏目录下（不参与同步）。
/// 例外放行：排序文件 .order.json / .category-order.json 要双向同步——
/// 旧版把 `.` 开头全部过滤，导致排序文件上传后任何设备都拉不回来。
pub(crate) fn is_trash_or_hidden_rel(rel: &str) -> bool {
    // 根目录下的排序白名单文件：放行
    if rel == crate::store::ORDER_FILE || rel == crate::store::CATEGORY_ORDER_FILE {
        return false;
    }
    // 空分类占位文件：放行（分类目录下的 .gitkeep 要随同步传输）
    if rel.split('/').next_back() == Some(".gitkeep") {
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
    deleted_rels: &mut Vec<String>,
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
            clean_local_extra(local_dir, remote_files, &sub_rel, deleted, deleted_rels)?;
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
                // 上报删除清单：调用方据此同步清理账本记录
                deleted_rels.push(rel_unix);
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
        let mut deleted_rels: Vec<String> = Vec::new();
        clean_local_extra(&dir, &remote_files, std::path::Path::new(""), &mut deleted, &mut deleted_rels).unwrap();

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
        let mut deleted_rels: Vec<String> = Vec::new();
        clean_local_extra(&dir, &remote_files, std::path::Path::new(""), &mut deleted, &mut deleted_rels).unwrap();

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

    /// 账本按目标隔离读写；旧平铺格式迁移收编进首个同步目标
    #[test]
    fn sync_meta_targets_isolation_and_legacy_migration() {
        let dir = std::env::temp_dir().join("pp_test_sync_meta_v2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 旧平铺格式（v≤2.2.1）：无法证明归属，直接丢弃——首个目标从零基线
        // 全量上传，绝不能把旧基线错配给任意目标导致漏传
        std::fs::write(dir.join(".sync_meta.json"), r#"{"写作/a.md": 12345}"#).unwrap();
        let mut targets = load_sync_meta_targets(&dir);
        let baseline = take_target_baseline(&mut targets, "webdav:u@root");
        assert!(baseline.is_empty(), "旧格式账本必须丢弃，不得错配给任何目标");

        // 写入两个目标，各自隔离
        let mut b1 = baseline;
        b1.insert("x.md".to_string(), 1);
        targets.insert("webdav:u@root".to_string(), b1);
        let mut b2 = std::collections::HashMap::new();
        b2.insert("y.md".to_string(), 2);
        targets.insert("github:r@main".to_string(), b2);
        save_sync_meta_targets(&dir, &targets);

        let re = load_sync_meta_targets(&dir);
        assert!(re.contains_key("webdav:u@root") && re.contains_key("github:r@main"));
        // 第二个目标取不到第一个目标的基线
        let mut re2 = re.clone();
        let b = take_target_baseline(&mut re2, "github:r@main");
        assert!(!b.contains_key("x.md"));

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

    /// 内存版 RemoteStore：验证同步算法（pull 回写 / 目标隔离 / 清理门 / 下载失败）
    struct MockStore {
        files: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
        list_errors: Vec<String>,
        /// 匹配该 rel 的下载直接失败（模拟网络中断）
        fail_download: Option<String>,
        uploads: std::sync::atomic::AtomicUsize,
    }

    impl MockStore {
        fn new() -> Self {
            Self {
                files: std::sync::Mutex::new(Default::default()),
                list_errors: Vec::new(),
                fail_download: None,
                uploads: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl RemoteStore for MockStore {
        async fn list_all(&self) -> Result<(Vec<RemoteFile>, Vec<String>), String> {
            let files = self.files.lock().unwrap();
            Ok((
                files
                    .iter()
                    .map(|(rel, c)| RemoteFile {
                        rel: rel.clone(),
                        content_length: c.len() as i64,
                    })
                    .collect(),
                self.list_errors.clone(),
            ))
        }
        async fn download(&self, rel: &str, local_path: &Path) -> Result<(), String> {
            if self.fail_download.as_deref() == Some(rel) {
                return Err("mock download failure".to_string());
            }
            let files = self.files.lock().unwrap();
            match files.get(rel) {
                Some(c) => std::fs::write(local_path, c).map_err(|e| e.to_string()),
                None => Err("404".to_string()),
            }
        }
        async fn upload(&self, rel: &str, content: Vec<u8>) -> Result<(), String> {
            self.uploads
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.files.lock().unwrap().insert(rel.to_string(), content);
            Ok(())
        }
        async fn delete(&self, rel: &str) -> Result<(), String> {
            self.files.lock().unwrap().remove(rel);
            Ok(())
        }
        async fn test(&self) -> Result<(), String> {
            Ok(())
        }
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(f)
    }

    /// pull 下载的内容回写账本：pull 完立即 push 不应整批重传（#7）
    #[test]
    fn pull_then_push_skips_unchanged() {
        let a = std::env::temp_dir().join("pp_test_pull_push_a");
        let b = std::env::temp_dir().join("pp_test_pull_push_b");
        for d in [&a, &b] {
            let _ = std::fs::remove_dir_all(d);
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::create_dir_all(a.join("cat")).unwrap();
        std::fs::write(a.join("cat").join("x.md"), "hello").unwrap();

        let store = MockStore::new();
        let key = "webdav:u@root";
        block_on(push_all_to_remote(&store, &a, key)).unwrap();
        let n1 = store.uploads.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(n1, 1, "首轮 push 应上传 1 个文件");

        // 设备 B 拉取，然后立即回推——账本已回写，不应重传
        block_on(pull_from_remote(&store, &b, key)).unwrap();
        block_on(push_all_to_remote(&store, &b, key)).unwrap();
        let n2 = store.uploads.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(n2, n1, "pull 回写账本后，未变更文件不应重复上传");

        for d in [&a, &b] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }

    /// 账本按远程目标隔离：换目标后同批文件重新全量上传（#8）
    #[test]
    fn push_targets_are_isolated() {
        let dir = std::env::temp_dir().join("pp_test_push_isolated");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cat")).unwrap();
        std::fs::write(dir.join("cat").join("x.md"), "hello").unwrap();

        let store = MockStore::new();
        block_on(push_all_to_remote(&store, &dir, "webdav:u@root")).unwrap();
        let n1 = store.uploads.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(n1, 1);

        // 换目标（GitHub）：新基线为空，同批文件应重新上传
        block_on(push_all_to_remote(&store, &dir, "github:r@main")).unwrap();
        let n2 = store.uploads.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(n2, 2, "目标隔离：新目标应有自己独立的基线");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 列举不完整（部分目录失败）时禁止本地清理（#2）
    #[test]
    fn pull_with_partial_listing_keeps_local() {
        let dir = std::env::temp_dir().join("pp_test_pull_partial");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cat")).unwrap();
        std::fs::write(dir.join("cat").join("keep.md"), "old").unwrap();

        let mut store = MockStore::new();
        store
            .files
            .lock()
            .unwrap()
            .insert("cat/remote.md".to_string(), b"remote".to_vec());
        store.list_errors = vec!["some-dir 列举失败".to_string()];

        let report = block_on(pull_from_remote(&store, &dir, "webdav:u@root")).unwrap();
        assert!(
            dir.join("cat").join("keep.md").exists(),
            "列举不完整时本地文件不得被清理"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|e| e.contains("跳过本地清理")),
            "应有清理跳过警告: {:?}",
            report.errors
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 多目标删除传播互不吞：A 目标确认后 B 目标仍会删除自己的副本（#8）
    #[test]
    fn tombstone_deletion_propagates_per_target() {
        let dir = std::env::temp_dir().join("pp_test_tomb_multi");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cat")).unwrap();
        std::fs::write(dir.join("cat").join("x.md"), "hello").unwrap();

        let store = MockStore::new();
        // 上传到两个目标
        block_on(push_all_to_remote(&store, &dir, "webdav:u@root")).unwrap();
        block_on(push_all_to_remote(&store, &dir, "github:r@main")).unwrap();

        // 本地删除（tombstone 带真实 size）
        crate::store::delete_prompt(&dir, &dir.join("cat").join("x.md")).unwrap();
        let tombs = crate::store::load_tombstones(&dir);
        assert_eq!(tombs["cat/x.md"].size, 5, "tombstone 必须带删除时刻的真实大小");

        // 目标 A 推送删除；目标 B 的副本仍在
        block_on(push_all_to_remote(&store, &dir, "webdav:u@root")).unwrap();
        {
            let files = store.files.lock().unwrap();
            assert!(!files.contains_key("cat/x.md"), "A 目标应已删除");
        }
        // 目标 B 推送：仍会执行自己的删除（A 的确认不得消费记录）
        block_on(push_all_to_remote(&store, &dir, "github:r@main")).unwrap();
        {
            let files = store.files.lock().unwrap();
            assert!(!files.contains_key("cat/x.md"), "B 目标也应完成删除传播");
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 下载失败时本地原文件原位不动、无临时文件残留（#1 临时文件流程）
    #[test]
    fn pull_download_failure_keeps_original() {
        let dir = std::env::temp_dir().join("pp_test_pull_dlfail");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cat")).unwrap();
        std::fs::write(dir.join("cat").join("x.md"), "old-content").unwrap();

        let mut store = MockStore::new();
        // 远端同路径放不同长度的内容，触发 need_download
        store
            .files
            .lock()
            .unwrap()
            .insert("cat/x.md".to_string(), b"new-remote-content".to_vec());
        store.fail_download = Some("cat/x.md".to_string());

        block_on(pull_from_remote(&store, &dir, "webdav:u@root")).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join("cat").join("x.md")).unwrap(),
            "old-content",
            "下载失败时原文件必须原位未动"
        );
        assert!(
            !dir.join("cat").join("x.md.syncdl").exists(),
            "临时文件应被清理"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
