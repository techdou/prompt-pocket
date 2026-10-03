// GitHub 仓库存档传输实现（RemoteStore 的 GitHub 版本）
//
// 模型：Contents API 单文件读写 + Trees API 一次拿全树。
// 与 WebDAV 的"单文件增量"模型同构：上传 = PUT contents、删除 = DELETE contents、
// 下载 = GET contents(raw)、列树 = GET git/trees?recursive=1。
// 路径扁平：PUT "a/b/c.md" 天然携带中间路径，无需像 WebDAV 逐级建目录。
//
// 已知边界：
// - 每个文件一个 commit（存档语义下可接受：每条 prompt 有独立变更历史；
//   一次同步合并成一个 commit 需 Git Data API 四步操作，留作后续增强）
// - 认证后 5000 次请求/小时；上传/删除要先 GET 拿 sha（更新语义要求），
//   每文件 2 次请求，本工具规模（几十个文件）远够
// - Contents API 单文件读取上限 1MB（base64 JSON 形态），提示词场景不触发；
//   下载走 raw media type，不受此限

use base64::Engine;

use super::webdav::encode_segments;
use super::{is_sync_file, RemoteContent, RemoteFile, RemoteStore};

const GH_API: &str = "https://api.github.com";
const GH_API_VERSION: &str = "2022-11-28";

/// GitHub 存档配置（repo/branch/prefix 从 config.json 加载；token 走系统凭据库）
#[derive(Debug, Clone, Default)]
pub struct GitHubConfig {
    pub repo: String,   // "owner/name"
    pub branch: String, // 缺省 main
    pub prefix: String, // 仓库内子目录前缀，"" = 仓库根
    pub token: String,  // PAT（建议 fine-grained，单仓库 Contents 读写）
    pub enabled: bool,
}

impl GitHubConfig {
    pub fn is_configured(&self) -> bool {
        self.enabled && !self.repo.is_empty() && !self.token.is_empty()
    }
}

/// GitHub 远程存储：持带认证头的 HTTP 客户端 + 规范化配置
pub struct GitHubStore {
    http: reqwest::Client,
    #[cfg(test)]
    api: Option<String>,
    repo: String,
    branch: String,
    /// 规范化后的前缀：无首尾斜杠，空串 = 仓库根
    prefix: String,
}

impl GitHubStore {
    fn api_base(&self) -> &str {
        #[cfg(test)]
        if let Some(api) = &self.api {
            return api;
        }
        GH_API
    }
    pub fn new(cfg: &GitHubConfig) -> Result<Self, String> {
        validate_repo(&cfg.repo)?;
        let mut headers = reqwest::header::HeaderMap::new();
        let auth = format!("Bearer {}", cfg.token);
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&auth)
                .map_err(|_| "PAT 含非法字符".to_string())?,
        );
        // GitHub 要求所有请求带 User-Agent
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static("prompt-pocket"),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            reqwest::header::HeaderValue::from_static(GH_API_VERSION),
        );
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(60))
            .default_headers(headers)
            .build()
            .map_err(|e| format!("客户端构建失败: {e}"))?;
        Ok(Self {
            http,
            #[cfg(test)]
            api: None,
            repo: cfg.repo.clone(),
            branch: if cfg.branch.is_empty() {
                "main".to_string()
            } else {
                cfg.branch.clone()
            },
            prefix: {
                let prefix = cfg.prefix.trim_matches('/');
                if !prefix.is_empty() && !super::is_sync_dir(prefix) {
                    return Err("GitHub 子目录包含不允许的路径".into());
                }
                prefix.to_string()
            },
        })
    }

    /// 相对路径 → 仓库内完整路径（加前缀）
    fn full_path(&self, rel: &str) -> String {
        if self.prefix.is_empty() {
            rel.to_string()
        } else {
            format!("{}/{rel}", self.prefix)
        }
    }

    /// Contents API URL（路径逐段 percent-encode，中文/空格/特殊字符安全）
    fn contents_url(&self, rel: &str) -> String {
        format!(
            "{}/repos/{}/contents/{}",
            self.api_base(),
            self.repo,
            encode_segments(&self.full_path(rel))
        )
    }

    /// 仓库是否为空（无任何分支）。用于区分"空仓库"与"分支名写错"两种 404
    async fn repo_is_empty(&self) -> Result<bool, String> {
        let resp = self
            .http
            .get(format!(
                "{}/repos/{}/branches?per_page=1",
                self.api_base(),
                self.repo
            ))
            .send()
            .await
            .map_err(|e| format!("查询分支列表失败: {e}"))?;
        let branches: Vec<serde_json::Value> = check_status(resp, "查询分支列表")?
            .json()
            .await
            .map_err(|e| format!("解析分支列表失败: {e}"))?;
        Ok(branches.is_empty())
    }
}

impl RemoteStore for GitHubStore {
    /// Trees API 一次请求拿全树（顺带返回每个 blob 的 SHA 和大小，
    /// 比 WebDAV 逐层 PROPFIND 高效；SHA 做内容指纹的增强留待后续）
    async fn list_all(&self) -> Result<(Vec<RemoteFile>, Vec<String>), String> {
        // 先取分支头 sha；分支 404 有两种含义：仓库是空的（还没任何提交）→ 空列表；
        // 或仓库非空但分支名写错 → 明确报错（静默当空列表会让 pull 报
        // "未获取到远程文件列表"，用户摸不着头脑）
        let branch_url = format!(
            "{}/repos/{}/branches/{}",
            self.api_base(),
            self.repo,
            super::webdav::urlencoding_encode(&self.branch)
        );
        let resp = self
            .http
            .get(&branch_url)
            .send()
            .await
            .map_err(|e| format!("查询分支失败: {e}"))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            if self.repo_is_empty().await? {
                return Ok((Vec::new(), Vec::new()));
            }
            return Err(format!(
                "分支 {} 不存在：仓库非空但没有这个分支（检查配置的分支名）",
                self.branch
            ));
        }
        let branch: BranchResp = check_status(resp, "查询分支")?
            .json()
            .await
            .map_err(|e| format!("解析分支信息失败: {e}"))?;

        let tree_url = format!(
            "{}/repos/{}/git/trees/{}?recursive=1",
            self.api_base(),
            self.repo,
            branch.commit.sha
        );
        let resp = self
            .http
            .get(&tree_url)
            .send()
            .await
            .map_err(|e| format!("获取文件树失败: {e}"))?;
        let tree: TreeResp = check_status(resp, "获取文件树")?
            .json()
            .await
            .map_err(|e| format!("解析文件树失败: {e}"))?;
        if tree.truncated {
            return Err("仓库文件树过大被 GitHub 截断，请联系开发者改用分页方案".to_string());
        }
        Ok((tree_to_files(tree.tree, &self.prefix), Vec::new()))
    }

    /// Content and blob revision come from one Contents response, so the SHA
    /// cannot describe a different version than the bytes checked by reconciliation.
    async fn fetch(&self, rel: &str) -> Result<Option<RemoteContent>, String> {
        let response = self
            .http
            .get(self.contents_url(rel))
            .query(&[("ref", self.branch.as_str())])
            .send()
            .await
            .map_err(|e| format!("GET 失败: {e}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let meta: ContentMeta = check_status(response, "读取文件")?
            .json()
            .await
            .map_err(|e| format!("解析文件内容失败: {e}"))?;
        if meta.encoding != "base64" {
            return Err("GitHub 文件超过内容读取限制或不是普通文件".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(meta.content.split_whitespace().collect::<String>())
            .map_err(|e| format!("解码文件内容失败: {e}"))?;
        let content =
            String::from_utf8(bytes).map_err(|e| format!("远程文件不是有效 UTF-8: {e}"))?;
        Ok(Some(RemoteContent {
            content,
            revision: Some(meta.sha),
        }))
    }
    async fn upload_checked(
        &self,
        rel: &str,
        content: Vec<u8>,
        expected: Option<&str>,
    ) -> Result<(), String> {
        let mut body = serde_json::json!({
            "message": format!("prompt-pocket: update {rel}"),
            "content": base64::engine::general_purpose::STANDARD.encode(content),
            "branch": self.branch,
        });
        if let Some(sha) = expected {
            body["sha"] = serde_json::Value::String(sha.into());
        }
        let response = self
            .http
            .put(self.contents_url(rel))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("PUT 失败: {e}"))?;
        check_status(response, "上传文件")?;
        Ok(())
    }
    async fn delete_checked(&self, rel: &str, expected: &str) -> Result<(), String> {
        let body = serde_json::json!({
            "message": format!("prompt-pocket: delete {rel}"),
            "branch": self.branch, "sha": expected,
        });
        let response = self
            .http
            .delete(self.contents_url(rel))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("DELETE 失败: {e}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        check_status(response, "删除文件")?;
        Ok(())
    }

    /// 自检：GET 仓库信息——仓库存在、PAT 可读，且响应里的 permissions
    /// 明确报告无写权限时直接报错（省得用户到上传时才发现 PAT 只读）。
    /// permissions 缺失（旧版 PAT 形态等）不阻塞，靠真实写操作暴露。
    async fn test(&self) -> Result<(), String> {
        let resp = self
            .http
            .get(format!("{}/repos/{}", self.api_base(), self.repo))
            .send()
            .await
            .map_err(|e| format!("连接失败: {e}"))?;
        let info: RepoInfo = check_status(resp, "测试连接")?
            .json()
            .await
            .map_err(|e| format!("解析仓库信息失败: {e}"))?;
        if let Some(perms) = info.permissions {
            if perms.push == Some(false) {
                return Err(
                    "PAT 对该仓库无写权限：请确认 PAT 勾选了此仓库的 Contents 读写权限".to_string(),
                );
            }
        }
        Ok(())
    }
}

/// 校验仓库格式：必须是 owner/name，字符集限 GitHub 合法命名字符
fn validate_repo(repo: &str) -> Result<(), String> {
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    let parts: Vec<&str> = repo.split('/').collect();
    match parts.as_slice() {
        [owner, name] if valid(owner) && valid(name) => Ok(()),
        _ => Err("仓库格式应为 owner/repo（如 techdou/prompts）".to_string()),
    }
}

/// 把 GitHub 文件树条目映射为同步用的远程文件列表：
/// 只收 blob、剥前缀、过滤 .trash/隐藏路径（与 WebDAV 腿同口径）
fn tree_to_files(tree: Vec<TreeEntry>, prefix: &str) -> Vec<RemoteFile> {
    tree.into_iter()
        .filter(|e| e.kind == "blob")
        .filter_map(|e| {
            let rel = if prefix.is_empty() {
                e.path
            } else {
                e.path.strip_prefix(&format!("{prefix}/"))?.to_string()
            };
            if !is_sync_file(&rel) {
                return None;
            }
            Some(RemoteFile {
                rel,
                content_length: e.size.unwrap_or(0),
            })
        })
        .collect()
}

/// HTTP 状态码 → 中文错误。401/403/404 翻译成人话，其余带原始状态码
fn check_status(resp: reqwest::Response, what: &str) -> Result<reqwest::Response, String> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    Err(status_error(resp.status().as_u16(), what))
}

fn status_error(status: u16, what: &str) -> String {
    match status {
        401 => format!("{what}失败：PAT 无效或已过期（401）"),
        403 => format!(
            "{what}失败：权限不足或触发限速——检查 PAT 是否有该仓库 Contents 读写权限（403）"
        ),
        404 => format!("{what}失败：仓库/分支/路径不存在，或 PAT 无权访问（404）"),
        409 => format!("{what}失败：分支冲突或仓库为空（409）"),
        s => format!("{what}失败：GitHub 返回 {s}"),
    }
}

#[derive(serde::Deserialize)]
struct BranchResp {
    commit: CommitRef,
}

#[derive(serde::Deserialize)]
struct CommitRef {
    sha: String,
}

#[derive(serde::Deserialize)]
struct TreeResp {
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(serde::Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    size: Option<i64>,
}

#[derive(serde::Deserialize)]
struct ContentMeta {
    sha: String,
    #[serde(default)]
    encoding: String,
    #[serde(default)]
    content: String,
}

#[derive(serde::Deserialize)]
struct RepoInfo {
    /// 认证用户的仓库权限；只在带认证的请求里返回，缺失时按未知处理
    permissions: Option<RepoPermissions>,
}

#[derive(serde::Deserialize)]
struct RepoPermissions {
    push: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_repo_accepts_legal_names() {
        assert!(validate_repo("techdou/prompts").is_ok());
        assert!(validate_repo("a-b_c.d/e-f_g.h").is_ok());
    }

    #[test]
    fn validate_repo_rejects_bad_shapes() {
        for bad in ["", "noslash", "a/b/c", "/b", "a/", "a b/c", "a/中 文"] {
            assert!(validate_repo(bad).is_err(), "应拒绝: {bad:?}");
        }
    }

    #[test]
    fn full_path_joins_prefix() {
        let store_prefix = |prefix: &str| {
            // full_path 只依赖 prefix 字段，直接构造结构体测纯逻辑
            GitHubStore {
                http: reqwest::Client::new(),
                api: None,
                repo: "a/b".to_string(),
                branch: "main".to_string(),
                prefix: prefix.trim_matches('/').to_string(),
            }
        };
        assert_eq!(store_prefix("").full_path("写作/a.md"), "写作/a.md");
        assert_eq!(
            store_prefix("archive").full_path("写作/a.md"),
            "archive/写作/a.md"
        );
        // 首尾斜杠在 new 里已规范化
        assert_eq!(store_prefix("/archive/").full_path("a.md"), "archive/a.md");
    }

    #[test]
    fn contents_url_encodes_segments() {
        let store = GitHubStore {
            http: reqwest::Client::new(),
            api: None,
            repo: "a/b".to_string(),
            branch: "main".to_string(),
            prefix: String::new(),
        };
        assert_eq!(
            store.contents_url("写作/我的 #1.md"),
            "https://api.github.com/repos/a/b/contents/%E5%86%99%E4%BD%9C/%E6%88%91%E7%9A%84%20%231.md"
        );
    }

    #[test]
    fn tree_to_files_filters_and_strips_prefix() {
        let entry = |path: &str, kind: &str| TreeEntry {
            path: path.to_string(),
            kind: kind.to_string(),
            size: Some(10),
        };
        let tree = vec![
            entry("archive/写作/a.md", "blob"),
            entry("archive/写作", "tree"),          // 目录条目不收
            entry("archive/.trash/old.md", "blob"), // .trash 过滤
            entry("archive/.order.json", "blob"),   // 排序白名单放行
            entry("other/x.md", "blob"),            // 前缀外的不收
        ];
        let files = tree_to_files(tree, "archive");
        let rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, vec!["写作/a.md", ".order.json"]);
        assert_eq!(files[0].content_length, 10);
    }

    #[test]
    fn tree_to_files_root_prefix_keeps_all_visible() {
        let entry = |path: &str| TreeEntry {
            path: path.to_string(),
            kind: "blob".to_string(),
            size: None, // size 缺失按 0 处理
        };
        let files = tree_to_files(vec![entry("a.md"), entry(".hidden/b.md")], "");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].rel, "a.md");
        assert_eq!(files[0].content_length, 0);
    }

    #[test]
    fn status_error_maps_to_human_messages() {
        assert!(status_error(401, "上传文件").contains("PAT 无效"));
        assert!(status_error(403, "上传文件").contains("权限不足"));
        assert!(status_error(404, "测试连接").contains("不存在"));
        assert!(status_error(500, "下载文件").contains("500"));
    }

    /// A loopback Contents API verifies the observed SHA is retained through the
    /// write. A concurrent edit must fail rather than refreshing SHA and overwriting.
    #[test]
    fn contents_sha_guards_concurrent_update_and_delete() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for expected_method in ["GET", "PUT", "DELETE"] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut input = Vec::new();
                let mut buffer = [0u8; 4096];
                let boundary = loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    input.extend_from_slice(&buffer[..n]);
                    if let Some(p) = input.windows(4).position(|w| w == b"\r\n\r\n") {
                        break p + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&input[..boundary]).to_string();
                assert!(headers.starts_with(expected_method), "{headers}");
                let length = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                    .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                    .unwrap_or(0);
                while input.len() < boundary + length {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    input.extend_from_slice(&buffer[..n]);
                }
                let (status, body) = if expected_method == "GET" {
                    assert!(headers.contains("ref=feature%2Ftest"), "{headers}");
                    (200, serde_json::json!({"sha":"observed-sha", "encoding":"base64", "content":"b2xk\n"}).to_string())
                } else {
                    let body: serde_json::Value =
                        serde_json::from_slice(&input[boundary..]).unwrap();
                    assert_eq!(body["sha"], "observed-sha");
                    assert_eq!(body["branch"], "feature/test");
                    if expected_method == "PUT" {
                        assert_eq!(body["content"], "bmV3");
                    }
                    // Server has already advanced to a different SHA.
                    (409, "{}".to_string())
                };
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let store = GitHubStore {
            http: reqwest::Client::new(),
            api: Some(api),
            repo: "owner/repo".into(),
            branch: "feature/test".into(),
            prefix: String::new(),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let fetched = store.fetch("a.md").await.unwrap().unwrap();
            assert_eq!(fetched.content, "old");
            assert_eq!(fetched.revision.as_deref(), Some("observed-sha"));
            assert!(store
                .upload_checked("a.md", b"new".to_vec(), fetched.revision.as_deref())
                .await
                .unwrap_err()
                .contains("409"));
            assert!(store
                .delete_checked("a.md", fetched.revision.as_deref().unwrap())
                .await
                .unwrap_err()
                .contains("409"));
        });
        worker.join().unwrap();
    }
}
