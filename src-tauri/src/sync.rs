// 坚果云 WebDAV 同步模块
//
// 架构：本地缓存 + 后台同步。
// - 所有 UI 读写仍走本地缓存（store.rs），保证瞬间响应
// - 本模块负责本地缓存 ↔ 坚果云的双向同步
//
// 同步策略（规避坚果云每30分钟600次的速率限制）：
// - 启动时：PROPFIND 远程目录树 → 拉取变更/新增 → 删除远程已删的本地文件
// - 保存/新建/删除后：异步推送单个文件（PUT/DELETE）
// - 重命名/移动：远程 MOVE

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
    pub password: String, // 应用密码（App Password）
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
        .map_err(|e| DavError::Reqwest(e))?;
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
        .list(&format!("/{root}/"), Depth::Number(0))
        .await
        .map_err(|e| format!("连接失败，请检查账号/应用密码/路径: {e}"))?;
    Ok(())
}

/// 全量拉取：把远程的文件同步到本地缓存
/// 策略：远程为准。远程有的下载，远程没有的本地 .md 备份到 .trash 后删除。
/// 关键修复：排除 .trash 目录，避免无限循环；只处理真实 prompt 文件。
pub async fn pull_from_remote(cfg: &CloudConfig, local_dir: &Path) -> Result<SyncReport, String> {
    let client = build_client(cfg).map_err(|e| format!("客户端构建失败: {e}"))?;
    let root = sanitize_remote_path(&cfg.remote_root);

    // 确保远程根目录存在
    let _ = client.mkcol(&format!("/{root}")).await;

    // 递归列出远程所有文件（Depth::Infinity）
    let entities = client
        .list(&format!("/{root}/"), Depth::Infinity)
        .await
        .map_err(|e| format!("列出远程文件失败: {e}"))?;

    let mut remote_files: HashSet<String> = HashSet::new();
    let mut downloaded = 0u32;
    let mut skipped = 0u32;
    let mut errors: Vec<String> = Vec::new();

    for entity in entities {
        if let ListEntity::File(file) = entity {
            let rel = match extract_rel_path(&file.href, &root) {
                Some(r) => r,
                None => continue,
            };
            // 排除 .trash 路径下的文件（备份目录不参与同步）
            if rel.starts_with(".trash/") || rel.contains("/.trash/") {
                continue;
            }
            remote_files.insert(rel.clone());

            let local_path = local_dir.join(&rel);
            // 内容校对：用文件大小 + 存在性判断是否需要下载
            let need_download = match std::fs::metadata(&local_path) {
                Ok(meta) => meta.len() as i64 != file.content_length,
                Err(_) => true, // 本地不存在
            };

            if need_download {
                if let Err(e) = download_file(&client, &root, &rel, &local_path).await {
                    errors.push(format!("{rel}: {e}"));
                    continue;
                }
                downloaded += 1;
            } else {
                skipped += 1;
            }
        }
    }

    // 清理本地多余文件（远程已删）—— 排除 .trash
    let mut deleted = 0u32;
    clean_local_extra(local_dir, &remote_files, Path::new(""), &mut deleted)?;

    Ok(SyncReport {
        downloaded,
        skipped,
        deleted,
        uploaded: 0,
        errors,
    })
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub downloaded: u32,
    pub skipped: u32,
    pub deleted: u32,
    pub uploaded: u32,
    pub errors: Vec<String>,
}

/// 全量上传：把本地所有文件推送到远程（只增不删，不删除云端多余文件）
/// 关键修复：
/// 1. 排除 .trash 目录（备份文件不参与上传，避免无限上传）
/// 2. 内容校对：先 PROPFIND 远程文件大小，相同则跳过，避免无限制重复上传
pub async fn push_all_to_remote(cfg: &CloudConfig, local_dir: &Path) -> Result<SyncReport, String> {
    let client = build_client(cfg).map_err(|e| format!("客户端构建失败: {e}"))?;
    let root = sanitize_remote_path(&cfg.remote_root);

    // 确保远程根目录存在
    let _ = client.mkcol(&format!("/{root}")).await;

    // 先列出远程所有文件及其大小，用于内容校对
    let remote_sizes: std::collections::HashMap<String, i64> = match client
        .list(&format!("/{root}/"), Depth::Infinity)
        .await
    {
        Ok(entities) => entities
            .into_iter()
            .filter_map(|e| match e {
                ListEntity::File(f) => {
                    let rel = extract_rel_path(&f.href, &root)?;
                    Some((rel, f.content_length))
                }
                _ => None,
            })
            .collect(),
        Err(_) => std::collections::HashMap::new(), // 远程为空或出错则全量上传
    };

    let mut uploaded = 0u32;
    let mut skipped = 0u32;
    let mut errors: Vec<String> = Vec::new();

    // 遍历本地所有 .md 文件 + .order.json
    for entry in walkdir::WalkDir::new(local_dir)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            // 关键：排除 .trash 目录及其内容
            let name = e.file_name().to_string_lossy();
            name != ".trash"
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
        let is_order = name == ".order.json";
        if !is_md && !is_order {
            continue;
        }
        if name.starts_with('~') {
            continue;
        }

        let rel = path.strip_prefix(local_dir).map_err(|e| e.to_string())?;
        let rel_unix = rel.to_string_lossy().replace('\\', "/");

        // 内容校对：读取本地文件大小，与远程比对，相同则跳过
        let local_content = match std::fs::read(path) {
            Ok(c) => c,
            Err(e) => {
                errors.push(format!("{rel_unix}（读文件）: {e}"));
                continue;
            }
        };
        let local_size = local_content.len() as i64;
        if let Some(&remote_size) = remote_sizes.get(&rel_unix) {
            if remote_size == local_size {
                skipped += 1;
                continue; // 大小相同，跳过上传
            }
        }

        // 确保远程目录存在
        if let Err(e) = ensure_remote_dirs(&client, &root, &rel_unix).await {
            errors.push(format!("{rel_unix}（建目录）: {e}"));
            continue;
        }

        match client
            .put(&format!("/{root}/{rel_unix}"), local_content)
            .await
        {
            Ok(()) => uploaded += 1,
            Err(e) => errors.push(format!("{rel_unix}: {e}")),
        }
    }

    Ok(SyncReport {
        uploaded,
        skipped,
        errors,
        ..Default::default()
    })
}


// ────────────────────────────────────────────────
// 辅助函数
// ────────────────────────────────────────────────

/// 规范化远程路径：去首尾斜杠
fn sanitize_remote_path(s: &str) -> String {
    s.trim_matches('/').to_string()
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
async fn download_file(client: &Client, root: &str, rel: &str, local_path: &Path) -> Result<(), String> {
    if let Some(parent) = local_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let resp = client
        .get(&format!("/{root}/{rel}"))
        .await
        .map_err(|e| format!("GET 失败: {e}"))?;
    let bytes = resp.bytes().await.map_err(|e| format!("读取响应失败: {e}"))?;
    std::fs::write(local_path, &bytes).map_err(|e| e.to_string())?;
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
                    path.file_stem().and_then(|s| s.to_str()).unwrap_or("untitled"),
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
        acc = if acc.is_empty() {
            part.to_string()
        } else {
            format!("{acc}/{part}")
        };
        let _ = client.mkcol(&format!("/{root}/{acc}")).await;
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
        assert_eq!(urlencoding_decode("/dav/a/b.md"), Some("/dav/a/b.md".to_string()));
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
}
