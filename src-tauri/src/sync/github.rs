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
use std::path::Path;

use super::webdav::encode_segments;
use super::{is_trash_or_hidden_rel, RemoteFile, RemoteStore};

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
    repo: String,
    branch: String,
    /// 规范化后的前缀：无首尾斜杠，空串 = 仓库根
    prefix: String,
}

impl GitHubStore {
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
            repo: cfg.repo.clone(),
            branch: if cfg.branch.is_empty() {
                "main".to_string()
            } else {
                cfg.branch.clone()
            },
            prefix: cfg.prefix.trim_matches('/').to_string(),
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
            "{GH_API}/repos/{}/contents/{}",
            self.repo,
            encode_segments(&self.full_path(rel))
        )
    }

    /// 查文件当前 sha（更新/删除的前提）；404 = 文件不存在
    async fn get_sha(&self, rel: &str) -> Result<Option<String>, String> {
        let resp = self
            .http
            .get(format!("{}?ref={}", self.contents_url(rel), self.branch))
            .send()
            .await
            .map_err(|e| format!("查询文件状态失败: {e}"))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp = check_status(resp, "查询文件状态")?;
        let meta: ContentMeta = resp
            .json()
            .await
            .map_err(|e| format!("解析文件元数据失败: {e}"))?;
        Ok(Some(meta.sha))
    }
}

impl RemoteStore for GitHubStore {
    /// Trees API 一次请求拿全树（顺带返回每个 blob 的 SHA 和大小，
    /// 比 WebDAV 逐层 PROPFIND 高效；SHA 做内容指纹的增强留待后续）
    async fn list_all(&self) -> Result<(Vec<RemoteFile>, Vec<String>), String> {
        // 先取分支头 sha；分支 404 = 空仓库（还没任何提交）→ 空列表
        let branch_url = format!("{GH_API}/repos/{}/branches/{}", self.repo, self.branch);
        let resp = self
            .http
            .get(&branch_url)
            .send()
            .await
            .map_err(|e| format!("查询分支失败: {e}"))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok((Vec::new(), Vec::new()));
        }
        let branch: BranchResp = check_status(resp, "查询分支")?
            .json()
            .await
            .map_err(|e| format!("解析分支信息失败: {e}"))?;

        let tree_url = format!(
            "{GH_API}/repos/{}/git/trees/{}?recursive=1",
            self.repo, branch.commit.sha
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

    /// 下载：raw media type 直接拿文件字节（绕过 1MB base64 限制）
    async fn download(&self, rel: &str, local_path: &Path) -> Result<(), String> {
        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let resp = self
            .http
            .get(format!("{}?ref={}", self.contents_url(rel), self.branch))
            .header(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/vnd.github.raw"),
            )
            .send()
            .await
            .map_err(|e| format!("GET 失败: {e}"))?;
        let bytes = check_status(resp, "下载文件")?
            .bytes()
            .await
            .map_err(|e| format!("读取响应失败: {e}"))?;
        crate::store::write_atomic(local_path, &bytes).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 上传：已存在需带当前 sha（更新语义），不存在则创建。每文件最多 2 次请求。
    async fn upload(&self, rel: &str, content: Vec<u8>) -> Result<(), String> {
        let sha = self.get_sha(rel).await?;
        let mut body = serde_json::json!({
            "message": format!("prompt-pocket: update {rel}"),
            "content": base64::engine::general_purpose::STANDARD.encode(&content),
            "branch": self.branch,
        });
        if let Some(sha) = sha {
            body["sha"] = serde_json::Value::String(sha);
        }
        let resp = self
            .http
            .put(self.contents_url(rel))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("PUT 失败: {e}"))?;
        check_status(resp, "上传文件")?;
        Ok(())
    }

    /// 删除：需当前 sha；文件本就不存在视为成功（删除传播语义：目的已达）
    async fn delete(&self, rel: &str) -> Result<(), String> {
        let Some(sha) = self.get_sha(rel).await? else {
            return Ok(());
        };
        let body = serde_json::json!({
            "message": format!("prompt-pocket: delete {rel}"),
            "branch": self.branch,
            "sha": sha,
        });
        let resp = self
            .http
            .delete(self.contents_url(rel))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("DELETE 失败: {e}"))?;
        check_status(resp, "删除文件")?;
        Ok(())
    }

    /// 自检：GET 仓库信息——仓库存在且 PAT 可读即通过
    /// （Contents 写权限只有真实写操作才能验证，测试连接不制造 commit）
    async fn test(&self) -> Result<(), String> {
        let resp = self
            .http
            .get(format!("{GH_API}/repos/{}", self.repo))
            .send()
            .await
            .map_err(|e| format!("连接失败: {e}"))?;
        check_status(resp, "测试连接")?;
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
            if is_trash_or_hidden_rel(&rel) {
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
        403 => format!("{what}失败：权限不足或触发限速——检查 PAT 是否有该仓库 Contents 读写权限（403）"),
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
                repo: "a/b".to_string(),
                branch: "main".to_string(),
                prefix: prefix.trim_matches('/').to_string(),
            }
        };
        assert_eq!(store_prefix("").full_path("写作/a.md"), "写作/a.md");
        assert_eq!(store_prefix("archive").full_path("写作/a.md"), "archive/写作/a.md");
        // 首尾斜杠在 new 里已规范化
        assert_eq!(store_prefix("/archive/").full_path("a.md"), "archive/a.md");
    }

    #[test]
    fn contents_url_encodes_segments() {
        let store = GitHubStore {
            http: reqwest::Client::new(),
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
            entry("archive/写作", "tree"),              // 目录条目不收
            entry("archive/.trash/old.md", "blob"),     // .trash 过滤
            entry("archive/.order.json", "blob"),       // 排序白名单放行
            entry("other/x.md", "blob"),                // 前缀外的不收
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
}
