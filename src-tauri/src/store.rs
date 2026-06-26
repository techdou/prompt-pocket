// 数据存储层：一 prompt 一 Markdown 文件 + YAML frontmatter。
// 不引入数据库，靠文件系统 + 云盘客户端做同步。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// frontmatter 元数据，与前端 PromptMeta 对应（serde camelCase）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptMeta {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_copy_mode")]
    pub copy_mode: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub updated: String,
}

fn default_copy_mode() -> String {
    "markdown".to_string()
}

impl Default for PromptMeta {
    fn default() -> Self {
        let now = now_iso();
        Self {
            title: String::new(),
            tags: vec![],
            copy_mode: default_copy_mode(),
            pinned: false,
            created: now.clone(),
            updated: now,
        }
    }
}

/// 单条 prompt 的精简视图（列表用）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    pub id: String,
    pub title: String,
    pub category: String,
    pub path: String,
    pub abs_path: String,
    pub meta: PromptMeta,
}

/// 分类计数
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryCount {
    pub name: String,
    pub count: usize,
}

/// 应用配置（init_app 返回给前端）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub data_dir: String,
    pub hotkey: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub prompts: Vec<Prompt>,
    pub categories: Vec<CategoryCount>,
}

/// 解析单个 .md 文件：返回 (frontmatter 原文, 正文, 解析后的元数据)
pub fn parse_markdown(content: &str) -> (String, String, PromptMeta) {
    let trimmed = content.trim_start_matches('\u{feff}');
    let body_start = trimmed.strip_prefix("---\n").or_else(|| trimmed.strip_prefix("---\r\n"));

    if let Some(rest) = body_start {
        // 寻找闭合的 ---
        if let Some(end) = find_frontmatter_end(rest) {
            let fm_raw = rest[..end].to_string();
            let body = rest[end..]
                .trim_start_matches("---")
                .trim_start_matches(['\n', '\r', ' '])
                .to_string();
            let meta = parse_yaml_frontmatter(&fm_raw);
            return (fm_raw, body, meta);
        }
    }

    // 没有 frontmatter，整体当正文
    (String::new(), trimmed.to_string(), PromptMeta::default())
}

/// 在 frontmatter 内容中寻找闭合分隔符 `---` 所在的字符偏移。
/// frontmatter 可能含 `---` 开头的元素（如 yaml 文档标记），这里只认行首。
fn find_frontmatter_end(s: &str) -> Option<usize> {
    let mut pos = 0;
    for line in s.split_inclusive('\n') {
        let line_trim = line.trim_end_matches(['\n', '\r']);
        if line_trim == "---" {
            return Some(pos);
        }
        pos += line.len();
    }
    None
}

fn parse_yaml_frontmatter(fm_raw: &str) -> PromptMeta {
    // 尝试用结构化解析；失败则退回默认值（容错：用户手写 yaml 可能格式不对）
    match serde_yaml::from_str::<PromptMeta>(fm_raw) {
        Ok(m) => m,
        Err(_) => {
            // 尝试解析为通用 map，至少把 title/tags 救回来
            if let Ok(generic) = serde_yaml::from_str::<serde_yaml::Value>(fm_raw) {
                let mut meta = PromptMeta::default();
                if let Some(map) = generic.as_mapping() {
                    if let Some(t) = map.get(&serde_yaml::Value::String("title".into())) {
                        if let Some(s) = t.as_str() {
                            meta.title = s.to_string();
                        }
                    }
                    if let Some(t) = map.get(&serde_yaml::Value::String("tags".into())) {
                        if let Some(seq) = t.as_sequence() {
                            meta.tags = seq
                                .iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect();
                        }
                    }
                }
                meta
            } else {
                PromptMeta::default()
            }
        }
    }
}

/// 扫描根目录下所有 .md 文件，构建 prompt 列表 + 分类计数
pub fn scan_prompts(root: &Path) -> std::io::Result<ScanResult> {
    let mut prompts: Vec<Prompt> = Vec::new();
    let mut cat_counts: BTreeMap<String, usize> = BTreeMap::new();

    for entry in WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        // 跳过隐藏文件（以 . 开头）和临时文件
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') || name.starts_with('~') {
                continue;
            }
        }

        if let Some(prompt) = build_prompt(root, path) {
            *cat_counts.entry(prompt.category.clone()).or_insert(0) += 1;
            prompts.push(prompt);
        }
    }

    // 排序：置顶优先，再按更新时间倒序
    prompts.sort_by(|a, b| {
        b.meta
            .pinned
            .cmp(&a.meta.pinned)
            .then_with(|| b.meta.updated.cmp(&a.meta.updated))
    });

    let categories = cat_counts
        .into_iter()
        .map(|(name, count)| CategoryCount { name, count })
        .collect();

    Ok(ScanResult { prompts, categories })
}

fn build_prompt(root: &Path, abs: &Path) -> Option<Prompt> {
    let content = fs::read_to_string(abs).ok()?;
    let (_fm_raw, _body, mut meta) = parse_markdown(&content);

    let rel = abs.strip_prefix(root).ok()?;
    let rel_str = path_to_unix(rel);
    let file_stem = abs
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string();

    // title 缺省取文件名
    if meta.title.is_empty() {
        meta.title = file_stem.clone();
    }

    // category 取父目录名；根目录下则归为"未分类"
    let category = rel
        .parent()
        .and_then(|p| p.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("未分类")
        .to_string();

    // id = 去扩展名的相对路径
    let id = rel_str.trim_end_matches(".md").to_string();

    Some(Prompt {
        id,
        title: meta.title.clone(),
        category,
        path: rel_str,
        abs_path: abs.to_string_lossy().to_string(),
        meta,
    })
}

/// 读取 prompt 全文：返回 (frontmatter 原文, 正文)
pub fn read_prompt(abs: &Path) -> std::io::Result<(String, String)> {
    let content = fs::read_to_string(abs)?;
    let (fm, body, _meta) = parse_markdown(&content);
    Ok((fm, body))
}

/// 保存 prompt 全文（前端已拼好 frontmatter + 正文）
pub fn save_prompt(abs: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(abs, content)
}

/// 新建 prompt 文件，返回其绝对路径
pub fn create_prompt(root: &Path, category: &str, title: &str) -> std::io::Result<PathBuf> {
    let safe_cat = sanitize_filename::sanitize(category);
    let safe_title = sanitize_filename::sanitize(title);
    let dir = if safe_cat.is_empty() || safe_cat == "未分类" {
        root.to_path_buf()
    } else {
        root.join(&safe_cat)
    };
    fs::create_dir_all(&dir)?;

    // 避免重名：若存在则追加数字
    let mut file_name = format!("{}.md", safe_title);
    let mut n = 1;
    while dir.join(&file_name).exists() {
        file_name = format!("{}-{}.md", safe_title, n);
        n += 1;
    }

    let path = dir.join(&file_name);
    let now = now_iso();
    let content = format!(
        "---\ntitle: {}\ntags: []\ncopy_mode: markdown\ncreated: {}\nupdated: {}\n---\n\n在这里写提示词内容…\n",
        title, now, now
    );
    fs::write(&path, content)?;
    Ok(path)
}

pub fn delete_prompt(abs: &Path) -> std::io::Result<()> {
    fs::remove_file(abs)
}

/// 把路径分隔符统一为正斜杠（用于前端跨平台一致 id）
pub fn path_to_unix(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

pub fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // 简单 RFC3339 近似：用秒级时间戳生成 ISO 字符串
    // 注：本地时区转换略复杂，这里用 UTC，前端展示时再格式化
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_iso_utc(secs)
}

fn format_iso_utc(secs: u64) -> String {
    // 基于 epoch 秒数算出 UTC 年月日时分秒（不引入 chrono，保持依赖最小）
    let days = secs / 86400;
    let rem = secs % 86400;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;

    let (y, mo, d) = civil_from_days(days as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, mo, d, h, m, s
    )
}

/// Howard Hinnant 的 days_from_civil 逆运算，把 epoch 起的天数转成 (年,月,日)
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
