// 坚果云 WebDAV 传输实现（RemoteStore 的 WebDAV 版本）
//
// 速率限制：坚果云约 600 次/30 分钟，本工具规模（几十个文件）够用。

use reqwest_dav::types::list_cmd::ListEntity;
use reqwest_dav::{Auth, Client, ClientBuilder, Depth};
use std::path::Path;

use super::{is_trash_or_hidden_rel, RemoteFile, RemoteStore};

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

/// WebDAV 远程存储：持构造好的客户端 + 规范化后的远程根路径
pub struct WebDavStore {
    client: Client,
    root: String,
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
        Ok(Self {
            client,
            root: sanitize_remote_path(&cfg.remote_root),
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

        while let Some(rel_dir) = queue.pop_front() {
            // 该层的请求路径：根用 "/{root}/"，子目录用 "/{root}/{rel_dir}/"（逐段 URL 编码）
            let req_path = if rel_dir.is_empty() {
                format!("/{}/", encode_segments(&self.root))
            } else {
                format!("/{}/{}/", encode_segments(&self.root), encode_segments(&rel_dir))
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
                        if is_trash_or_hidden_rel(&rel) {
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

    /// 逐级创建远程目录（如 写作/子目录/a.md 会先 mkcol 写作 再 mkcol 写作/子目录）
    async fn ensure_remote_dirs(&self, rel_unix: &str) -> Result<(), String> {
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
            let _ = self.client.mkcol(&remote_url(&self.root, &acc)).await;
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

    /// 下载单个远程文件到本地
    async fn download(&self, rel: &str, local_path: &Path) -> Result<(), String> {
        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let resp = self
            .client
            .get(&remote_url(&self.root, rel))
            .await
            .map_err(|e| format!("GET 失败: {e}"))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("读取响应失败: {e}"))?;
        crate::store::write_atomic(local_path, &bytes).map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn upload(&self, rel: &str, content: Vec<u8>) -> Result<(), String> {
        // 先确保中间目录存在，再 PUT
        self.ensure_remote_dirs(rel)
            .await
            .map_err(|e| format!("建目录失败: {e}"))?;
        self.client
            .put(&remote_url(&self.root, rel), content)
            .await
            .map_err(|e| e.to_string())
    }

    async fn delete(&self, rel: &str) -> Result<(), String> {
        match self.client.delete(&remote_url(&self.root, rel)).await {
            Ok(()) => Ok(()),
            // 云端本就没有该文件（从未上传/已被其它设备删）→ 目的已达，视为成功
            Err(e) if e.to_string().contains("404") || e.to_string().contains("Not Found") => {
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// 测试连接：PROPFIND 远程根目录，验证凭据 + 路径可访问
    async fn test(&self) -> Result<(), String> {
        self.client
            .list(&format!("/{}/", encode_segments(&self.root)), Depth::Number(0))
            .await
            .map_err(|e| format!("连接失败，请检查账号/应用密码/路径: {e}"))?;
        Ok(())
    }

    /// 确保远程根目录存在
    async fn ensure_root(&self) -> Result<(), String> {
        let _ = self.client.mkcol(&format!("/{}", self.root)).await;
        Ok(())
    }
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
