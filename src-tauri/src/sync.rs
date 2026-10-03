// 同步算法与远程传输分离；按目标保存精确内容共同基线。
// 下载保留本地独有/修改内容；覆盖前快照，冲突保留双方版本。
pub mod github;
pub mod webdav;
pub use github::{GitHubConfig, GitHubStore};
use std::path::Path;
pub use webdav::{CloudConfig, WebDavStore};

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
    pub deleted_remote: u32,
    pub conflicts: u32,
    pub errors: Vec<String>,
}
pub struct RemoteFile {
    pub rel: String,
    pub content_length: i64,
}
pub struct RemoteContent {
    pub content: String,
    /// 强 WebDAV ETag / GitHub blob SHA，和返回内容属于同一版本。
    pub revision: Option<String>,
}
/// Both backends expose version-checked writes, keeping reconciliation independent
/// of transport. An existing file without a usable revision cannot be overwritten.
pub trait RemoteStore: Send + Sync {
    async fn list_all(&self) -> Result<(Vec<RemoteFile>, Vec<String>), String>;
    async fn fetch(&self, rel: &str) -> Result<Option<RemoteContent>, String>;
    async fn upload_checked(
        &self,
        rel: &str,
        content: Vec<u8>,
        expected: Option<&str>,
    ) -> Result<(), String>;
    async fn delete_checked(&self, rel: &str, expected: &str) -> Result<(), String>;
    async fn test(&self) -> Result<(), String>;
    async fn ensure_root(&self) -> Result<(), String> {
        Ok(())
    }
}

pub async fn pull_from_remote<S: RemoteStore>(
    store: &S,
    local_dir: &Path,
    target_key: &str,
) -> Result<SyncReport, String> {
    let canonical_dir = std::fs::canonicalize(local_dir).map_err(|e| e.to_string())?;
    let local_dir = canonical_dir.as_path();
    let mut report = SyncReport::default();
    let mut targets = load_sync_meta_targets(local_dir)?;
    let mut baseline = take_target_baseline(&mut targets, target_key);
    let tombstones = crate::store::load_tombstones(local_dir);
    let (files, errors) = store.list_all().await?;
    report.errors.extend(errors);
    for file in files {
        let rel = file.rel;
        if !is_sync_file(&rel) {
            continue;
        }
        let result = async {
            let path = checked_sync_path(local_dir, &rel)?;
            let remote = store.fetch(&rel).await?.ok_or("远程文件在列举后已不存在")?;
            let local = read_optional(&path)?;
            let mut clear_tombstone = false;
            if let Some(tomb) = tombstones.get(&rel) {
                // Exact shared baseline detects even same-length remote edits.
                // Unknown older deletions remain conservative until explicit push.
                let unchanged = baseline.get(&rel).map_or(
                    tomb.size == 0 || tomb.size as i64 == file.content_length,
                    |base| base == &remote.content,
                );
                if unchanged {
                    report.skipped += 1;
                    return Ok::<(), String>(());
                }
                clear_tombstone = true;
            }
            if local.as_deref() == Some(remote.content.as_bytes()) {
                baseline.insert(rel.clone(), remote.content);
                report.skipped += 1;
                if clear_tombstone {
                    crate::store::remove_tombstone(local_dir, &rel).map_err(|e| e.to_string())?;
                }
                return Ok(());
            }
            if let Some(local) = local.as_deref() {
                let base = baseline.get(&rel).map(String::as_bytes);
                if base == Some(remote.content.as_bytes()) {
                    report.skipped += 1;
                    return Ok(());
                }
                if base != Some(local) {
                    record_conflict(local_dir, &rel, &remote.content, &mut report)?;
                    return Ok(());
                }
            }
            write_download(local_dir, &rel, local.as_deref(), remote.content.as_bytes())?;
            baseline.insert(rel.clone(), remote.content);
            report.downloaded += 1;
            if clear_tombstone {
                crate::store::remove_tombstone(local_dir, &rel).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            report.errors.push(format!("{rel}: {error}"));
        }
    }
    // Remote absence, including incomplete listings, never proves permission to
    // discard an independent local prompt. Explicit local deletion uses tombstones.
    targets.insert(target_key.to_string(), baseline);
    save_sync_meta_targets(local_dir, &targets)?;
    Ok(report)
}

pub async fn push_all_to_remote<S: RemoteStore>(
    store: &S,
    local_dir: &Path,
    target_key: &str,
) -> Result<SyncReport, String> {
    let canonical_dir = std::fs::canonicalize(local_dir).map_err(|e| e.to_string())?;
    let local_dir = canonical_dir.as_path();
    store.ensure_root().await?;
    let mut report = SyncReport::default();
    let mut targets = load_sync_meta_targets(local_dir)?;
    let mut baseline = take_target_baseline(&mut targets, target_key);
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
            let path = checked_sync_path(local_dir, &rel)?;
            let local = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let remote = store.fetch(&rel).await?;
            if remote.as_ref().is_some_and(|r| r.content == local) {
                baseline.insert(rel.clone(), local);
                report.skipped += 1;
                return Ok::<(), String>(());
            }
            if let Some(remote) = &remote {
                if baseline.get(&rel) != Some(&remote.content) || remote.revision.is_none() {
                    record_conflict(local_dir, &rel, &remote.content, &mut report)?;
                    return Ok(());
                }
            } else if baseline.contains_key(&rel) {
                report.conflicts += 1;
                report.errors.push(format!(
                    "{rel}: 远程文件已删除，已保留本地文件，未自动重新上传"
                ));
                return Ok(());
            }
            match store
                .upload_checked(
                    &rel,
                    local.clone().into_bytes(),
                    remote.as_ref().and_then(|r| r.revision.as_deref()),
                )
                .await
            {
                Ok(()) => {
                    baseline.insert(rel.clone(), local);
                    report.uploaded += 1;
                }
                Err(error) => {
                    // A rejected conditional write may mean another device won
                    // the race. Preserve the newly observed content as a conflict.
                    if let Some(latest) = store.fetch(&rel).await? {
                        if remote
                            .as_ref()
                            .map_or(true, |old| old.content != latest.content)
                        {
                            record_conflict(local_dir, &rel, &latest.content, &mut report)?;
                            return Ok(());
                        }
                    }
                    return Err(error);
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            report.errors.push(format!("{rel}: {error}"));
        }
    }
    // Keep each target's acknowledgement: one backend must not consume another
    // backend's pending deletion. Never delete a remotely revised file blindly.
    let tombstones = crate::store::load_tombstones(local_dir);
    for (rel, tomb) in tombstones {
        if tomb.done_targets.contains(target_key) {
            continue;
        }
        let result = async {
            let path = checked_sync_path(local_dir, &rel)?;
            // A restored or newly recreated local prompt supersedes its deletion.
            if path.exists() {
                return Ok::<(), String>(());
            }
            if let Some(remote) = store.fetch(&rel).await? {
                // Without an exact common baseline, equal byte length cannot
                // prove this is still the version the user deleted.
                let unchanged = baseline.get(&rel) == Some(&remote.content);
                let Some(revision) = remote.revision.as_deref().filter(|_| unchanged) else {
                    record_conflict(local_dir, &rel, &remote.content, &mut report)?;
                    return Ok(());
                };
                store.delete_checked(&rel, revision).await?;
            }
            crate::store::mark_tombstone_done(local_dir, &rel, target_key)
                .map_err(|e| e.to_string())?;
            baseline.remove(&rel);
            report.deleted_remote += 1;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            report.errors.push(format!("{rel}（删除传播）: {error}"));
        }
    }
    targets.insert(target_key.to_string(), baseline);
    save_sync_meta_targets(local_dir, &targets)?;
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
    let path = checked_sync_path(root, rel)?;
    if read_optional(&path)?.as_deref() != expected {
        return Err("下载期间本地内容已变化，已保留本地文件，请重试".into());
    }
    if expected.is_some() && !rel.ends_with(".gitkeep") {
        crate::recovery::snapshot(root, &path, "sync")
            .map_err(|e| format!("备份失败，未覆盖原文件: {e}"))?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Revalidate after creating parents / taking the snapshot.
    let path = checked_sync_path(root, rel)?;
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
    let original = checked_sync_path(root, rel)?;
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
                if original.exists() && !rel.ends_with(".gitkeep") {
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

pub fn target_key_webdav(cfg: &CloudConfig) -> String {
    format!(
        "webdav:{}",
        serde_json::to_string(&(
            webdav::JIANGUO_HOST,
            &cfg.username,
            cfg.remote_root.trim_matches('/')
        ))
        .expect("strings serialize")
    )
}
pub fn target_key_github(cfg: &GitHubConfig) -> String {
    // Empty branch is normalized to main by the transport as well.
    format!(
        "github:{}@{}:{}",
        cfg.repo,
        if cfg.branch.is_empty() {
            "main"
        } else {
            &cfg.branch
        },
        cfg.prefix.trim_matches('/')
    )
}
const SYNC_META_FILE: &str = ".sync_meta.json";
type SyncMetaTargets =
    std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>;
#[derive(serde::Serialize, serde::Deserialize)]
struct SyncMetaEnvelope {
    version: u8,
    targets: SyncMetaTargets,
}
fn load_sync_meta_targets(local_dir: &Path) -> Result<SyncMetaTargets, String> {
    let path = local_dir.join(SYNC_META_FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(info) if !info.is_file() || info.file_type().is_symlink() => {
            return Err("同步基线不是普通文件".into())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => return Err(error.to_string()),
        _ => {}
    }
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("同步基线损坏: {e}"))?;
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(3) => Ok(serde_json::from_value::<SyncMetaEnvelope>(value)
            .map_err(|e| format!("同步基线损坏: {e}"))?
            .targets),
        Some(2) => {
            // Preserve the previous exact-content WebDAV baseline in its account
            // namespace. Upload-only/hash ledgers cannot prove a common baseline.
            let scope = value
                .get("scope")
                .and_then(serde_json::Value::as_str)
                .ok_or("同步基线缺少目标")?;
            let files =
                serde_json::from_value(value.get("files").cloned().ok_or("同步基线缺少文件")?)
                    .map_err(|e| e.to_string())?;
            Ok(std::collections::BTreeMap::from([(
                format!("webdav:{scope}"),
                files,
            )]))
        }
        _ => Ok(Default::default()),
    }
}
fn save_sync_meta_targets(local_dir: &Path, targets: &SyncMetaTargets) -> Result<(), String> {
    let path = local_dir.join(SYNC_META_FILE);
    if std::fs::symlink_metadata(&path)
        .is_ok_and(|info| !info.is_file() || info.file_type().is_symlink())
    {
        return Err("同步基线不是普通文件".into());
    }
    let bytes = serde_json::to_vec_pretty(&SyncMetaEnvelope {
        version: 3,
        targets: targets.clone(),
    })
    .map_err(|e| e.to_string())?;
    crate::recovery::atomic_write(&path, &bytes).map_err(|e| format!("保存同步基线失败: {e}"))
}
fn take_target_baseline(
    targets: &mut SyncMetaTargets,
    target_key: &str,
) -> std::collections::BTreeMap<String, String> {
    targets.remove(target_key).unwrap_or_default()
}
fn fnv1a_hash(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
pub(crate) fn is_sync_dir(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.contains(['\\', ':', '\0'])
        && rel
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.starts_with('~'))
}
pub(crate) fn is_sync_file(rel: &str) -> bool {
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
        || name == ".gitkeep"
        || (!name.starts_with('.') && name.ends_with(".md"))
}
#[cfg(test)]
fn is_trash_or_hidden_rel(rel: &str) -> bool {
    !is_sync_file(rel) && !is_sync_dir(rel)
}
fn checked_sync_path(root: &Path, rel: &str) -> Result<std::path::PathBuf, String> {
    if !is_sync_file(rel) {
        return Err("非法同步路径".into());
    }
    if rel.rsplit('/').next() != Some(".gitkeep") {
        return crate::recovery::checked_path(root, rel).map_err(|e| e.to_string());
    }
    let parent = rel.rsplit_once('/').map_or("", |(parent, _)| parent);
    let path = if parent.is_empty() {
        std::fs::canonicalize(root).map_err(|e| e.to_string())?
    } else {
        crate::recovery::checked_path(root, parent).map_err(|e| e.to_string())?
    }
    .join(".gitkeep");
    if std::fs::symlink_metadata(&path)
        .is_ok_and(|info| !info.is_file() || info.file_type().is_symlink())
    {
        return Err("同步目标必须是普通文件".into());
    }
    Ok(path)
}
#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 FNV-1a 哈希对内容变化敏感（相同内容同哈希，不同内容不同哈希）
    #[test]
    fn fnv1a_hash_detects_changes() {
        let h1 = fnv1a_hash(b"hello world");
        let h2 = fnv1a_hash(b"hello world");
        let h3 = fnv1a_hash(b"hello world!");
        assert_eq!(h1, h2, "相同内容应有相同哈希");
        assert_ne!(h1, h3, "不同内容应有不同哈希");
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
        async fn fetch(&self, rel: &str) -> Result<Option<RemoteContent>, String> {
            if self.fail_download.as_deref() == Some(rel) {
                return Err("mock download failure".into());
            }
            Ok(self.files.lock().unwrap().get(rel).map(|c| RemoteContent {
                content: String::from_utf8(c.clone()).unwrap(),
                revision: Some(fnv1a_hash(c).to_string()),
            }))
        }
        async fn upload_checked(
            &self,
            rel: &str,
            content: Vec<u8>,
            expected: Option<&str>,
        ) -> Result<(), String> {
            let mut files = self.files.lock().unwrap();
            let revision = files.get(rel).map(|c| fnv1a_hash(c).to_string());
            if revision.as_deref() != expected {
                return Err("precondition failed".into());
            }
            self.uploads
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            files.insert(rel.to_string(), content);
            Ok(())
        }
        async fn delete_checked(&self, rel: &str, expected: &str) -> Result<(), String> {
            let mut files = self.files.lock().unwrap();
            if files
                .get(rel)
                .is_some_and(|c| fnv1a_hash(c).to_string() != expected)
            {
                return Err("precondition failed".into());
            }
            files.remove(rel);
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
        let second = MockStore::new();
        block_on(push_all_to_remote(&second, &dir, "github:r@main")).unwrap();
        let n2 = n1 + second.uploads.load(std::sync::atomic::Ordering::SeqCst);
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
        assert_eq!(report.deleted, 0);
        assert!(report.errors.iter().any(|e| e.contains("some-dir")));

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
        assert_eq!(
            tombs["cat/x.md"].size, 5,
            "tombstone 必须带删除时刻的真实大小"
        );

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

    #[test]
    fn exact_baselines_isolate_targets_and_migrate_previous_content_format() {
        let dir = std::env::temp_dir().join("pp_test_sync_content_meta");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let scope = "[\"host\",\"account\",\"root\"]";
        let old =
            serde_json::json!({ "version": 2, "scope": scope, "files": { "a.md": "exact bytes" } });
        std::fs::write(dir.join(SYNC_META_FILE), serde_json::to_vec(&old).unwrap()).unwrap();
        let mut targets = load_sync_meta_targets(&dir).unwrap();
        assert_eq!(targets[&format!("webdav:{scope}")]["a.md"], "exact bytes");
        assert!(take_target_baseline(&mut targets, "github:repo@main:root").is_empty());
        targets.insert(
            "github:repo@main:root".into(),
            std::collections::BTreeMap::from([("a.md".into(), "second".into())]),
        );
        save_sync_meta_targets(&dir, &targets).unwrap();
        let result = load_sync_meta_targets(&dir).unwrap();
        assert_eq!(result[&format!("webdav:{scope}")]["a.md"], "exact bytes");
        assert_eq!(result["github:repo@main:root"]["a.md"], "second");
        for hashes in [r#"{"a.md":1234}"#, r#"{"webdav:u@root":{"a.md":1234}}"#] {
            std::fs::write(dir.join(SYNC_META_FILE), hashes).unwrap();
            assert!(load_sync_meta_targets(&dir).unwrap().is_empty());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn same_length_remote_change_survives_pending_deletion() {
        let dir = std::env::temp_dir().join("pp_test_tomb_remote_change");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "old").unwrap();
        let store = MockStore::new();
        let key = "github:repo@main";
        block_on(push_all_to_remote(&store, &dir, key)).unwrap();
        crate::store::delete_prompt(&dir, &dir.join("a.md")).unwrap();
        store
            .files
            .lock()
            .unwrap()
            .insert("a.md".into(), b"new".to_vec());
        let report = block_on(push_all_to_remote(&store, &dir, key)).unwrap();
        assert_eq!(report.deleted_remote, 0);
        assert_eq!(report.conflicts, 1);
        assert_eq!(store.files.lock().unwrap()["a.md"], b"new");
        assert!(!crate::store::load_tombstones(&dir)["a.md"]
            .done_targets
            .contains(key));
        let pulled = block_on(pull_from_remote(&store, &dir, key)).unwrap();
        assert_eq!(pulled.downloaded, 1);
        assert_eq!(std::fs::read(dir.join("a.md")).unwrap(), b"new");
        assert!(!crate::store::load_tombstones(&dir).contains_key("a.md"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_category_markers_roundtrip_but_recovery_and_conflicts_do_not() {
        let a = std::env::temp_dir().join("pp_test_keep_roundtrip_a");
        let b = std::env::temp_dir().join("pp_test_keep_roundtrip_b");
        for d in [&a, &b] {
            let _ = std::fs::remove_dir_all(d);
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::create_dir_all(a.join("empty")).unwrap();
        std::fs::write(a.join("empty/.gitkeep"), "").unwrap();
        std::fs::create_dir_all(a.join(".recovery")).unwrap();
        std::fs::write(a.join(".recovery/private.md"), "private").unwrap();
        std::fs::write(a.join("a.remote-conflict-1.md"), "conflict").unwrap();
        let store = MockStore::new();
        let pushed = block_on(push_all_to_remote(&store, &a, "target")).unwrap();
        assert_eq!(pushed.uploaded, 1, "{pushed:?}");
        let pulled = block_on(pull_from_remote(&store, &b, "target")).unwrap();
        assert_eq!(pulled.downloaded, 1, "{pulled:?}");
        assert!(b.join("empty/.gitkeep").is_file());
        assert!(!b.join(".recovery").exists());
        for d in [&a, &b] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }
}
