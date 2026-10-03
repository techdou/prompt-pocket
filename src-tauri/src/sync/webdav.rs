// 坚果云 WebDAV 传输实现（RemoteStore 的 WebDAV 版本）
//
// 速率限制：坚果云约 600 次/30 分钟，本工具规模（几十个文件）够用。

use reqwest_dav::types::list_cmd::ListEntity;
use reqwest_dav::{Auth, Client, ClientBuilder, Depth};
use std::collections::HashSet;

use super::{is_sync_dir, is_sync_file, RemoteContent, RemoteFile, RemoteStore};

/// 坚果云 WebDAV 端点
pub(super) const JIANGUO_HOST: &str = "https://dav.jianguoyun.com/dav";

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

/// WebDAV 远程存储：持构造好的客户端 + 规范化后的远程根路径
pub struct WebDavStore {
    client: Client,
    root: String,
    /// 本轮会话已确认存在的远程目录：同目录多文件上传不再逐文件重复 mkcol
    /// （100 个同分类文件从 ~200 次请求降到 ~101 次）
    ensured_dirs: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl WebDavStore {
    /// 构造客户端（带超时，避免坚果云慢响应时无限期挂起）
    pub fn new(cfg: &CloudConfig) -> Result<Self, String> {
        let agent = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| format!("客户端构建失败: {e}"))?;
        let client = ClientBuilder::new()
            .set_agent(agent)
            .set_host(JIANGUO_HOST.to_string())
            .set_auth(Auth::Basic(cfg.username.clone(), cfg.password.clone()))
            .build()
            .map_err(|e| format!("客户端构建失败: {e}"))?;
        let root = sanitize_remote_path(&cfg.remote_root);
        validate_root(&root)?;
        Ok(Self {
            client,
            root,
            ensured_dirs: std::sync::Mutex::new(std::collections::HashSet::new()),
        })
    }

    /// 用 Depth::Number(1) 递归遍历远程目录树。
    ///
    /// 关键背景：坚果云 WebDAV 不支持 Depth::Infinity——发 infinity 时服务端
    /// 静默降级成只返回一层（实测 infinity 与 depth=1 返回字节完全一致），
    /// 导致算法层永远看不到任何 .md 文件。
    ///
    /// 解法（与 Obsidian Remotely Save / rclone 一致）：逐层 PROPFIND depth=1，
    /// 遇到文件夹就递归再列一层，把整棵树走完。坚果云 600 次/30 分钟的限速
    /// 对本工具的规模（几个分类、几十个文件）完全够用。
    ///
    /// - 跳过 `.trash` 目录（含其所有后代）
    /// - 跳过根目录自身（depth=1 会把被列目录自己也返回一次）
    /// - 单个目录列举失败不中断整树：记录到 errors，继续其它目录
    async fn walk_remote(&self, errors: &mut Vec<String>) -> Vec<RemoteFile> {
        let mut files: Vec<RemoteFile> = Vec::new();
        // 待访问的远程相对目录路径队列（相对 root，空串表示根目录）
        let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        queue.push_back(String::new());

        let mut visited = HashSet::new();
        while let Some(rel_dir) = queue.pop_front() {
            if !visited.insert(rel_dir.clone()) {
                continue;
            }
            // 该层的请求路径：根用 "/{root}/"，子目录用 "/{root}/{rel_dir}/"（逐段 URL 编码）
            let req_path = if rel_dir.is_empty() {
                format!("/{}/", encode_segments(&self.root))
            } else {
                format!(
                    "/{}/{}/",
                    encode_segments(&self.root),
                    encode_segments(&rel_dir)
                )
            };

            let entities = match self.client.list(&req_path, Depth::Number(1)).await {
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
                        let Some(rel) = extract_rel_path(&file.href, &self.root) else {
                            continue;
                        };
                        // 过滤 .trash 及任何 . 开头的目录（防御）
                        if !is_sync_file(&rel) {
                            continue;
                        }
                        files.push(RemoteFile {
                            rel,
                            content_length: file.content_length,
                        });
                    }
                    ListEntity::Folder(folder) => {
                        let Some(rel) = extract_rel_path(&folder.href, &self.root) else {
                            continue;
                        };
                        // folder href 可能带尾斜杠，剥掉防止拼出 "//" 双斜杠请求路径
                        let rel = rel.trim_end_matches('/').to_string();
                        // 跳过根自身（depth=1 会把被列目录自身作为 Folder 返回一次）
                        if rel.is_empty() || rel == rel_dir {
                            continue;
                        }
                        // 过滤 .trash / 隐藏目录，不递归进去
                        if !is_sync_dir(&rel) {
                            continue;
                        }
                        queue.push_back(rel);
                    }
                }
            }
        }

        files
    }

    /// 逐级创建远程目录（如 写作/子目录/a.md 会先 mkcol 写作 再 mkcol 写作/子目录）
    async fn ensure_remote_dirs(&self, rel_unix: &str) -> Result<(), String> {
        // 取出文件所在的目录路径
        let parent = match rel_unix.rfind('/') {
            Some(i) => &rel_unix[..i],
            None => return Ok(()), // 文件在根目录，无需建目录
        };

        // 整条父链已确认：直接跳过（批量上传同目录的最常见场景）
        if self
            .ensured_dirs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(parent)
        {
            return Ok(());
        }

        // 逐级 mkcol（忽略"已存在"错误）；每级确认后入缓存，
        // 后续同目录文件零 mkcol
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
            if self
                .ensured_dirs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&acc)
            {
                continue;
            }
            let mkcol_result = self.client.mkcol(&remote_url(&self.root, &acc)).await;
            // 已存在（坚果云回 405）视为确认成功；网络等其他错误不缓存——
            // 否则一次瞬时失败会让本轮后续同目录文件全部跳过建目录而上传失败
            let confirmed = mkcol_result.is_ok()
                || mkcol_result
                    .as_ref()
                    .err()
                    .is_some_and(|e| e.to_string().contains("405"));
            if confirmed {
                self.ensured_dirs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(acc.clone());
            } else if let Err(error) = mkcol_result {
                return Err(format!("创建远程目录失败: {error}"));
            }
        }
        Ok(())
    }
}

impl RemoteStore for WebDavStore {
    async fn list_all(&self) -> Result<(Vec<RemoteFile>, Vec<String>), String> {
        let mut errors = Vec::new();
        let files = self.walk_remote(&mut errors).await;
        Ok((files, errors))
    }

    async fn fetch(&self, rel: &str) -> Result<Option<RemoteContent>, String> {
        let response = self
            .client
            .get_raw(&remote_url(&self.root, rel))
            .await
            .map_err(|e| format!("GET 失败: {e}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response
            .error_for_status()
            .map_err(|e| format!("GET 失败: {e}"))?;
        let revision = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .filter(|tag| tag.starts_with('"') && tag.ends_with('"'))
            .map(str::to_owned);
        let content =
            String::from_utf8(response.bytes().await.map_err(|e| e.to_string())?.to_vec())
                .map_err(|e| format!("远程文件不是有效 UTF-8: {e}"))?;
        Ok(Some(RemoteContent { content, revision }))
    }
    async fn upload_checked(
        &self,
        rel: &str,
        content: Vec<u8>,
        expected: Option<&str>,
    ) -> Result<(), String> {
        self.ensure_remote_dirs(rel).await?;
        let request = self
            .client
            .start_request(reqwest::Method::PUT, &remote_url(&self.root, rel))
            .await
            .map_err(|e| e.to_string())?;
        let request = match expected {
            Some(etag) => request.header(reqwest::header::IF_MATCH, etag),
            None => request.header(reqwest::header::IF_NONE_MATCH, "*"),
        };
        request
            .body(content)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| format!("PUT 失败: {e}"))?;
        Ok(())
    }
    async fn delete_checked(&self, rel: &str, expected: &str) -> Result<(), String> {
        let response = self
            .client
            .start_request(reqwest::Method::DELETE, &remote_url(&self.root, rel))
            .await
            .map_err(|e| e.to_string())?
            .header(reqwest::header::IF_MATCH, expected)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        response
            .error_for_status()
            .map_err(|e| format!("DELETE 失败: {e}"))?;
        Ok(())
    }

    /// 测试连接：PROPFIND 远程根目录，验证凭据 + 路径可访问
    async fn test(&self) -> Result<(), String> {
        self.client
            .list(
                &format!("/{}/", encode_segments(&self.root)),
                Depth::Number(0),
            )
            .await
            .map_err(|e| format!("连接失败，请检查账号/应用密码/路径: {e}"))?;
        Ok(())
    }

    /// 确保远程根目录存在
    async fn ensure_root(&self) -> Result<(), String> {
        let response = self
            .client
            .mkcol_raw(&format!("/{}", encode_segments(&self.root)))
            .await
            .map_err(|e| e.to_string())?;
        if response.status().is_success()
            || response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED
        {
            Ok(())
        } else {
            Err(format!("创建远程根目录失败: {}", response.status()))
        }
    }
}

// ────────────────────────────────────────────────
// 辅助函数
// ────────────────────────────────────────────────

/// 规范化远程路径：去首尾斜杠
fn sanitize_remote_path(s: &str) -> String {
    s.trim_matches('/').to_string()
}
fn validate_root(root: &str) -> Result<(), String> {
    if is_sync_dir(root) {
        Ok(())
    } else {
        Err("远程根路径不能为空或包含隐藏/上级目录".into())
    }
}

/// percent-encode 单段路径：保留 RFC 3986 unreserved（A-Za-z0-9 - _ . ~），
/// 其余逐字节转 %XX。与 urlencoding_decode 对称。
/// 标题里的空格、#、?、% 等字符不编码会在 URL 里产生歧义（# 被当 fragment 截断）。
/// 通用编码，github.rs 拼 Contents API URL 也复用。
pub(crate) fn urlencoding_encode(s: &str) -> String {
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
pub(crate) fn encode_segments(path: &str) -> String {
    path.split('/')
        .map(urlencoding_encode)
        .collect::<Vec<_>>()
        .join("/")
}

/// 拼接远程文件 URL：root 和 rel 都逐段编码
fn remote_url(root: &str, rel: &str) -> String {
    format!("/{}/{}", encode_segments(root), encode_segments(rel))
}

/// 从 WebDAV href 中提取相对于 remote_root 的路径
/// href 形如 /dav/PromptPocket/%E5%86%99%E4%BD%9C/a.md
/// 返回 写作/a.md（URL 解码 + 去掉根前缀）
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

#[cfg(test)]
mod transport_tests {
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
        let cases = [
            "写作/a b.md",
            "编程/#1 清单.md",
            "100%.md",
            "web服务/html-read.md",
        ];
        for c in cases {
            let encoded = urlencoding_encode(c);
            let decoded = urlencoding_decode(&encoded).unwrap();
            assert_eq!(decoded, c, "round-trip 失败: {c}");
        }
    }

    /// remote_url：root 和 rel 都编码，保留路径分隔
    #[test]
    fn remote_url_encodes_all_segments() {
        assert_eq!(
            remote_url("PromptPocket", "写作/a.md"),
            "/PromptPocket/%E5%86%99%E4%BD%9C/a.md"
        );
        assert_eq!(
            remote_url("My Root", "笔记/#1.md"),
            "/My%20Root/%E7%AC%94%E8%AE%B0/%231.md"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::{fnv1a_hash, is_sync_file};
    use super::*;
    use std::path::Path;
    fn sync_scope(client: &Client, root: &str) -> String {
        let user = match &client.auth {
            Auth::Basic(user, _) | Auth::Digest(user, _) => user.as_str(),
            Auth::Anonymous => "",
        };
        format!(
            "webdav:{}",
            serde_json::to_string(&(&client.host, user, root)).unwrap()
        )
    }
    fn test_store(client: &Client, root: &str) -> WebDavStore {
        WebDavStore {
            client: ClientBuilder::new()
                .set_host(client.host.clone())
                .set_auth(client.auth.clone())
                .build()
                .unwrap(),
            root: root.into(),
            ensured_dirs: std::sync::Mutex::new(Default::default()),
        }
    }
    async fn pull_with_client(
        client: &Client,
        root: &str,
        local: &Path,
    ) -> Result<super::super::SyncReport, String> {
        super::super::pull_from_remote(&test_store(client, root), local, &sync_scope(client, root))
            .await
    }
    async fn push_with_client(
        client: &Client,
        root: &str,
        local: &Path,
    ) -> Result<super::super::SyncReport, String> {
        super::super::push_all_to_remote(
            &test_store(client, root),
            local,
            &sync_scope(client, root),
        )
        .await
    }

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
        let baseline = super::super::load_sync_meta_targets(&dir.0).unwrap();
        assert_eq!(
            baseline[&sync_scope(&server.client(), "PromptPocket")]["a.md"],
            "base"
        );
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
            remote_url("PromptPocket", "写作/a #%.md"),
            "/PromptPocket/%E5%86%99%E4%BD%9C/a%20%23%25.md"
        );
    }
}
