// 数据存储层：一 prompt 一 Markdown 文件 + YAML frontmatter。
// 不引入数据库，靠文件系统 + 云盘客户端做同步。
//
// 关键设计（修复"保存后内容不可见"）：
// 不再让前端拼接裸 frontmatter 文本往返，而是 read/save 都走结构化元数据对象。
// 写文件时由 serde_yaml 规范序列化 frontmatter，杜绝多次保存后格式漂移。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::recovery;

/// frontmatter 元数据，与前端 PromptMeta 对应（serde camelCase）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptMeta {
    #[serde(default)]
    pub title: String,
    #[serde(default = "default_copy_mode")]
    pub copy_mode: String,
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
            copy_mode: default_copy_mode(),
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
    pub body: String,
    /// 在分类内的排序权重（来自 .order.json），None 表示未定义（排末尾）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
}

/// 分类计数
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryCount {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub prompts: Vec<Prompt>,
    pub categories: Vec<CategoryCount>,
}

/// read_prompt 的返回：结构化元数据 + 正文
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptContent {
    pub meta: PromptMeta,
    pub body: String,
}

/// save_prompt 接收的结构化参数（前端表单直接传）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRequest {
    pub title: String,
    #[serde(default = "default_copy_mode")]
    pub copy_mode: String,
    pub body: String,
    #[serde(default)]
    pub category: Option<String>,
}

/// ────────────────────────────────────────────────
/// 解析：把 .md 文件内容拆成 (结构化元数据, 正文)
/// ────────────────────────────────────────────────
pub fn parse_markdown(content: &str) -> (PromptMeta, String) {
    let trimmed = content.trim_start_matches('\u{feff}');
    let body_start = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"));

    if let Some(rest) = body_start {
        if let Some(end) = find_frontmatter_end(rest) {
            let fm_raw = &rest[..end];
            let body = rest[end..]
                .trim_start_matches("---")
                .trim_start_matches(['\n', '\r', ' '])
                .to_string();
            let mut meta = parse_yaml_frontmatter(fm_raw);
            // 保留原始 created（解析到的），updated 留待保存时刷新
            if meta.created.is_empty() {
                meta.created = now_iso();
            }
            return (meta, body);
        }
    }

    // 没有 frontmatter，整体当正文
    (PromptMeta::default(), trimmed.to_string())
}

/// 在 frontmatter 内容中寻找闭合分隔符 `---` 所在的字符偏移。
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
    match serde_yaml::from_str::<PromptMeta>(fm_raw) {
        Ok(m) => m,
        Err(_) => {
            // 容错：解析失败时退回默认值，至少尝试救回 title
            if let Ok(generic) = serde_yaml::from_str::<serde_yaml::Value>(fm_raw) {
                let mut meta = PromptMeta::default();
                if let Some(map) = generic.as_mapping() {
                    if let Some(t) = map.get(serde_yaml::Value::String("title".into())) {
                        if let Some(s) = t.as_str() {
                            meta.title = s.to_string();
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

/// 把结构化元数据序列化成规范的 frontmatter 文本块（不含外层 ---）
fn serialize_frontmatter(meta: &PromptMeta) -> String {
    // 用 serde_yaml 序列化，保证格式规范、不漂移
    match serde_yaml::to_string(meta) {
        Ok(yaml) => yaml.trim_end().to_string(),
        Err(_) => format!("title: {}\n", meta.title), // 极端兜底
    }
}

/// ────────────────────────────────────────────────
/// 扫描：构建 prompt 列表 + 分类计数
/// 关键修复（Bug1）：先扫描所有一级子目录作为分类（含空目录），
/// 再统计每个分类下的 .md 文件数。这样新建空分类也能立刻显示。
/// ────────────────────────────────────────────────
pub fn scan_prompts(root: &Path) -> io::Result<ScanResult> {
    let mut prompts: Vec<Prompt> = Vec::new();
    let mut cat_counts: BTreeMap<String, usize> = BTreeMap::new();

    // 先把所有一级子目录登记为分类（count=0），含空目录
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() && !file_type.is_symlink() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if !name.starts_with('.') && !name.starts_with('~') {
                        cat_counts.entry(name.to_string()).or_insert(0);
                    }
                }
            }
        }
    }

    // 读取 .order.json：{ 分类名: [相对路径, ...] }
    let order_map = load_order_map(root);

    // 扫描所有 .md 文件
    // 关键修复：用 filter_entry 剪掉 .trash 及所有隐藏目录，避免备份文件泄漏进列表
    // （clean_local_extra 会把被删的 prompt 备份到 .trash/，若这里不剪掉，
    //  walkdir 会钻进去把备份当成正常 prompt 扫出来，分类显示成 ".trash"）
    for entry in WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            // 剪掉 .trash、.sync_meta.json 及所有以 . 开头的隐藏目录/文件
            !(name.starts_with('.') || name.starts_with('~'))
        })
        .filter_map(|e| e.ok())
    {
        let file_type = entry.file_type();
        if file_type.is_symlink() || !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') || name.starts_with('~') {
                continue;
            }
        }

        // 计算 order：取该文件所在分类，在 order_map[分类] 里找路径索引
        let rel_str = path_to_unix(path.strip_prefix(root).unwrap_or(path));
        let category_name = if let Some(idx) = rel_str.find('/') {
            rel_str[..idx].to_string()
        } else {
            "未分类".to_string()
        };
        let order = order_map
            .get(&category_name)
            .and_then(|paths| paths.iter().position(|p| p == &rel_str))
            .map(|idx| idx as i32);

        if let Some(prompt) = build_prompt(root, path, order) {
            *cat_counts.entry(prompt.category.clone()).or_insert(0) += 1;
            prompts.push(prompt);
        }
    }

    // 排序：category（字母序）→ order（升序，None 排后）→ updated（倒序）
    prompts.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| {
                // None 视为 i32::MAX，排到末尾
                let oa = a.order.unwrap_or(i32::MAX);
                let ob = b.order.unwrap_or(i32::MAX);
                oa.cmp(&ob)
            })
            .then_with(|| b.meta.updated.cmp(&a.meta.updated))
    });

    let categories = build_category_counts(cat_counts, root);

    Ok(ScanResult {
        prompts,
        categories,
    })
}

/// 按自定义顺序构建分类计数列表。
/// - 在 category_order 里的：按该顺序排列
/// - 不在里面的（含新建分类）：按字母序追加到末尾
fn build_category_counts(cat_counts: BTreeMap<String, usize>, root: &Path) -> Vec<CategoryCount> {
    let order = load_category_order(root);
    let mut ordered: Vec<CategoryCount> = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();

    // 先按自定义顺序收集
    for name in &order {
        if let Some(&count) = cat_counts.get(name) {
            ordered.push(CategoryCount {
                name: name.clone(),
                count,
            });
            seen.insert(name.as_str());
        }
    }
    // 未列入的按字母序（BTreeMap 已是字母序）追加
    for (name, count) in &cat_counts {
        if !seen.contains(name.as_str()) {
            ordered.push(CategoryCount {
                name: name.clone(),
                count: *count,
            });
        }
    }
    ordered
}

/// .order.json 文件名
pub const ORDER_FILE: &str = ".order.json";

/// .category-order.json 文件名：分类的自定义显示顺序（纯数组）
pub const CATEGORY_ORDER_FILE: &str = ".category-order.json";

/// 加载 order 映射：{ 分类名: [相对路径, ...] }
fn load_order_map(root: &Path) -> std::collections::HashMap<String, Vec<String>> {
    let path = root.join(ORDER_FILE);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 重写某分类的顺序到 .order.json
pub fn reorder_category(root: &Path, category: &str, ordered_paths: &[String]) -> io::Result<()> {
    let path = root.join(ORDER_FILE);
    let mut map: std::collections::HashMap<String, Vec<String>> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    map.insert(category.to_string(), ordered_paths.to_vec());
    let json = serde_json::to_string_pretty(&map).map_err(io::Error::other)?;
    recovery::atomic_write(&path, json.as_bytes())
}

/// 读取分类自定义顺序（.category-order.json）。失败或不存在返回空 Vec。
pub fn load_category_order(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join(CATEGORY_ORDER_FILE))
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

/// 写入分类顺序到 .category-order.json
pub fn save_category_order(root: &Path, names: &[String]) -> io::Result<()> {
    let path = root.join(CATEGORY_ORDER_FILE);
    let json = serde_json::to_string_pretty(names).map_err(io::Error::other)?;
    recovery::atomic_write(&path, json.as_bytes())
}

fn build_prompt(root: &Path, abs: &Path, order: Option<i32>) -> Option<Prompt> {
    let content = fs::read_to_string(abs).ok()?;
    let (mut meta, body) = parse_markdown(&content);

    let rel = abs.strip_prefix(root).ok()?;
    let rel_str = path_to_unix(rel);
    let file_stem = abs
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string();

    if meta.title.is_empty() {
        meta.title = file_stem.clone();
    }

    let category = rel
        .parent()
        .and_then(|p| p.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("未分类")
        .to_string();

    let id = rel_str.trim_end_matches(".md").to_string();

    Some(Prompt {
        id,
        title: meta.title.clone(),
        category,
        path: rel_str,
        abs_path: abs.to_string_lossy().to_string(),
        meta,
        body: body.trim_end_matches(['\n', '\r']).to_string(),
        order,
    })
}

/// ────────────────────────────────────────────────
/// 读取单条 prompt：返回结构化元数据 + 正文
/// ────────────────────────────────────────────────
pub fn read_prompt(abs: &Path) -> io::Result<PromptContent> {
    let content = fs::read_to_string(abs)?;
    let (meta, body) = parse_markdown(&content);
    Ok(PromptContent {
        meta,
        // trim 尾部换行，避免 save 时附加的 \n 在多次 round-trip 后累积
        body: body.trim_end_matches(['\n', '\r']).to_string(),
    })
}

/// ────────────────────────────────────────────────
/// 保存：接收结构化字段，用 serde_yaml 规范序列化 frontmatter
/// 若标题与当前文件名不一致，自动重命名文件（让文件名反映标题，便于同步比对）
/// 返回（新路径, 新 Prompt）—— 路径可能因重命名而变化
/// ────────────────────────────────────────────────
pub fn save_prompt(root: &Path, abs: &Path, req: &SaveRequest) -> io::Result<PathBuf> {
    let checked_abs = checked_existing_prompt_path(root, abs)?;
    let abs = checked_abs.as_path();
    // 读取旧文件以保留 created 时间戳
    let old_created = fs::read_to_string(abs)
        .ok()
        .map(|c| parse_markdown(&c).0.created)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(now_iso);

    let meta = PromptMeta {
        title: req.title.clone(),
        copy_mode: req.copy_mode.clone(),
        created: old_created,
        updated: now_iso(),
    };

    let fm = serialize_frontmatter(&meta);
    let content = format!("---\n{}\n---\n\n{}\n", fm, req.body);

    // 自动重命名：若标题有意义的部分与文件名不同，则重命名
    let final_path = maybe_rename_to_title(root, abs, &req.title, req.category.as_deref())?;

    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if abs.exists() {
        recovery::snapshot(root, abs, "history")?;
    }
    if final_path.exists() && final_path != abs {
        recovery::snapshot(root, &final_path, "history")?;
    }
    recovery::atomic_write(&final_path, content.as_bytes())?;
    if final_path != abs && abs.exists() {
        finalize_prompt_move(root, abs, &final_path)?;
    }
    Ok(final_path)
}

/// 若标题与当前文件名差异较大，重命名文件为「标题.md」
/// 规则：标题 sanitize 后若与当前文件 stem 不同且非空，则重命名（避免重名追加序号）
fn maybe_rename_to_title(
    root: &Path,
    abs: &Path,
    title: &str,
    category: Option<&str>,
) -> io::Result<PathBuf> {
    let safe_title = sanitize_filename::sanitize(title);
    if safe_title.is_empty() {
        return Ok(abs.to_path_buf());
    }
    let current_stem = abs.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    // 标题与当前文件名一致，无需重命名
    if safe_title == current_stem && category.is_none() {
        return Ok(abs.to_path_buf());
    }
    // 时间戳命名的（14 位纯数字，如 20260628T153000）一定重命名为标题
    let is_timestamp = current_stem
        .split('-')
        .next()
        .unwrap_or("")
        .chars()
        .all(|c| c.is_ascii_digit() || c == 'T')
        && current_stem.len() >= 13;

    let parent = if let Some(category) = category {
        category_dir(root, category)?
    } else {
        abs.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let mut new_name = format!("{}.md", safe_title);
    let mut n = 1;
    while parent.join(&new_name).exists() && parent.join(&new_name) != abs {
        new_name = format!("{}-{}.md", safe_title, n);
        n += 1;
    }
    let candidate = parent.join(&new_name);
    if candidate != abs || is_timestamp {
        Ok(candidate)
    } else {
        Ok(abs.to_path_buf())
    }
}

/// 新建 prompt 文件，返回其绝对路径
/// 文件名用时间戳保证唯一，frontmatter 的 title 用传入标题
/// （避免"新提示词-1.md"这种无意义命名；真正有意义的标题在保存时自动同步到文件名）
pub fn create_prompt(root: &Path, category: &str, title: &str) -> io::Result<PathBuf> {
    let dir = category_dir(root, category)?;
    fs::create_dir_all(&dir)?;

    // 用紧凑时间戳作文件名，避免重名 + 避免无意义的"新提示词-N"
    let stamp = now_compact();
    let mut file_name = format!("{}.md", stamp);
    let mut n = 1;
    while dir.join(&file_name).exists() {
        file_name = format!("{}-{}.md", stamp, n);
        n += 1;
    }

    let path = dir.join(&file_name);
    let now = now_iso();
    let meta = PromptMeta {
        title: title.to_string(),
        copy_mode: default_copy_mode(),
        created: now.clone(),
        updated: now,
    };
    let fm = serialize_frontmatter(&meta);
    let content = format!("---\n{}\n---\n\n\n", fm);
    recovery::atomic_write(&path, content.as_bytes())?;
    Ok(path)
}

/// ────────────────────────────────────────────────
/// 重命名 + 移动分类（问题2）
/// 改文件名（sanitize）+ 移动到新分类目录 + 更新 frontmatter title
/// ────────────────────────────────────────────────
pub fn rename_prompt(
    root: &Path,
    old_abs: &Path,
    new_title: &str,
    new_category: &str,
) -> io::Result<PathBuf> {
    let safe_title = sanitize_filename::sanitize(new_title);
    if safe_title.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "标题无效"));
    }
    let checked_abs = checked_existing_prompt_path(root, old_abs)?;
    let old_abs = checked_abs.as_path();
    let new_dir = category_dir(root, new_category)?;
    fs::create_dir_all(&new_dir)?;

    // 目标文件名，避免重名
    let mut file_name = format!("{}.md", safe_title);
    let mut n = 1;
    while new_dir.join(&file_name).exists() && new_dir.join(&file_name) != old_abs {
        file_name = format!("{}-{}.md", safe_title, n);
        n += 1;
    }
    let new_abs = new_dir.join(&file_name);

    // 重写 frontmatter 的 title（保留其余字段 + 正文）
    let (mut meta, body) = {
        let content = fs::read_to_string(old_abs)?;
        parse_markdown(&content)
    };
    meta.title = new_title.to_string();
    meta.updated = now_iso();
    let fm = serialize_frontmatter(&meta);
    let content = format!("---\n{}\n---\n\n{}\n", fm, body);

    recovery::snapshot(root, old_abs, "history")?;
    if new_abs.exists() && new_abs != old_abs {
        recovery::snapshot(root, &new_abs, "history")?;
    }
    recovery::atomic_write(&new_abs, content.as_bytes())?;

    // 如果路径变了，删除旧文件
    if new_abs != old_abs {
        finalize_prompt_move(root, old_abs, &new_abs)?;
    }

    Ok(new_abs)
}

/// 新建分类（即创建文件夹）（问题3）
pub fn create_category(root: &Path, name: &str) -> io::Result<PathBuf> {
    let safe = sanitize_filename::sanitize(name);
    if !is_visible_segment(&safe) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "分类名无效"));
    }
    let _ = recovery::checked_path(root, Path::new(&safe))?;
    let dir = root.join(&safe);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 重命名分类（优化3）：重命名文件夹，内部所有 .md 文件随之移动
/// 返回受影响的 .md 文件新路径列表
pub fn rename_category(root: &Path, old_name: &str, new_name: &str) -> io::Result<()> {
    let safe_old = sanitize_filename::sanitize(old_name);
    let safe_new = sanitize_filename::sanitize(new_name);
    if !is_visible_segment(&safe_old) || !is_visible_segment(&safe_new) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "新分类名无效"));
    }
    let _ = recovery::checked_path(root, Path::new(&safe_old))?;
    let _ = recovery::checked_path(root, Path::new(&safe_new))?;
    let old_dir = root.join(&safe_old);
    let new_dir = root.join(&safe_new);

    if !old_dir.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "原分类文件夹不存在",
        ));
    }

    if new_dir.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "目标分类已存在，拒绝合并覆盖",
        ));
    }
    let original_order_map = load_order_map(root);
    let next_order_map =
        remapped_category_order_map(original_order_map.clone(), old_name, new_name);
    let original_category_order = load_category_order(root);
    let next_category_order =
        remapped_category_order_list(original_category_order.clone(), old_name, new_name);

    if next_order_map != original_order_map {
        write_order_map(root, &next_order_map)?;
    }
    if next_category_order != original_category_order {
        if let Err(err) = save_category_order(root, &next_category_order) {
            if next_order_map != original_order_map {
                let _ = write_order_map(root, &original_order_map);
            }
            return Err(err);
        }
    }

    if let Err(err) = fs::rename(&old_dir, &new_dir) {
        if next_order_map != original_order_map {
            let _ = write_order_map(root, &original_order_map);
        }
        if next_category_order != original_category_order {
            let _ = save_category_order(root, &original_category_order);
        }
        return Err(err);
    }
    Ok(())
}

pub fn delete_prompt(root: &Path, abs: &Path) -> io::Result<()> {
    let checked_abs = checked_existing_prompt_path(root, abs)?;
    let abs = checked_abs.as_path();
    recovery::snapshot(root, abs, "deleted")?;
    fs::remove_file(abs)
}

fn checked_existing_prompt_path(root: &Path, abs: &Path) -> io::Result<PathBuf> {
    let canonical_root = fs::canonicalize(root)?;
    let candidate = if abs.is_absolute() {
        abs.to_path_buf()
    } else {
        canonical_root.join(abs)
    };
    let rel = candidate
        .strip_prefix(root)
        .or_else(|_| candidate.strip_prefix(&canonical_root))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "路径不在库内"))?;
    let mut current = canonical_root.clone();
    for component in rel.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "路径不能逃逸出库目录",
            ));
        };
        current.push(name);
        if fs::symlink_metadata(&current)?.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "路径包含符号链接",
            ));
        }
    }
    let canonical_abs = fs::canonicalize(&candidate)?;
    if !canonical_abs.starts_with(&canonical_root) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "路径不在库内"));
    }
    if canonical_abs.extension().and_then(|e| e.to_str()) != Some("md") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "只允许操作 Markdown 文件",
        ));
    }
    // Use the root's spelling consistently when comparing source and destination.
    // Native path checks may return /private/tmp or a Windows verbatim path.
    Ok(root.join(rel))
}

fn category_dir(root: &Path, category: &str) -> io::Result<PathBuf> {
    let safe_cat = sanitize_filename::sanitize(category);
    if safe_cat.is_empty() || safe_cat == "未分类" {
        Ok(root.to_path_buf())
    } else if is_visible_segment(&safe_cat) {
        let _ = recovery::checked_path(root, Path::new(&safe_cat))?;
        Ok(root.join(safe_cat))
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "分类名无效"))
    }
}

fn is_visible_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.starts_with('.')
        && !segment.starts_with('~')
        && !segment.contains('/')
        && !segment.contains('\\')
}

fn remap_prompt_order_path(root: &Path, old_abs: &Path, new_abs: &Path) -> io::Result<()> {
    let old_rel = path_to_unix(old_abs.strip_prefix(root).unwrap_or(old_abs));
    let new_rel = path_to_unix(new_abs.strip_prefix(root).unwrap_or(new_abs));
    if old_rel == new_rel {
        return Ok(());
    }
    let mut map = load_order_map(root);
    let old_category = old_rel
        .split_once('/')
        .map(|(cat, _)| cat.to_string())
        .unwrap_or_else(|| "未分类".to_string());
    let new_category = new_rel
        .split_once('/')
        .map(|(cat, _)| cat.to_string())
        .unwrap_or_else(|| "未分类".to_string());

    let mut old_index = None;
    if let Some(paths) = map.get_mut(&old_category) {
        if let Some(index) = paths.iter().position(|p| p == &old_rel) {
            paths.remove(index);
            old_index = Some(index);
        }
    }

    let paths = map.entry(new_category).or_default();
    if let Some(existing) = paths.iter().position(|p| p == &new_rel) {
        paths.remove(existing);
    }
    let insert_at = old_index.unwrap_or(paths.len()).min(paths.len());
    paths.insert(insert_at, new_rel);
    write_order_map(root, &map)
}

fn finalize_prompt_move(root: &Path, old_abs: &Path, new_abs: &Path) -> io::Result<()> {
    if let Err(err) = remap_prompt_order_path(root, old_abs, new_abs) {
        let _ = fs::remove_file(new_abs);
        return Err(err);
    }

    if let Err(err) = fs::remove_file(old_abs) {
        let _ = remap_prompt_order_path(root, new_abs, old_abs);
        let _ = fs::remove_file(new_abs);
        return Err(err);
    }

    Ok(())
}

fn remapped_category_order_map(
    mut map: std::collections::HashMap<String, Vec<String>>,
    old_name: &str,
    new_name: &str,
) -> std::collections::HashMap<String, Vec<String>> {
    let Some(paths) = map.remove(old_name) else {
        return map;
    };
    let old_prefix = format!("{old_name}/");
    let new_prefix = format!("{new_name}/");
    let remapped = paths
        .into_iter()
        .map(|path| {
            path.strip_prefix(&old_prefix)
                .map(|rest| format!("{new_prefix}{rest}"))
                .unwrap_or(path)
        })
        .collect();
    map.insert(new_name.to_string(), remapped);
    map
}

fn remapped_category_order_list(
    mut order: Vec<String>,
    old_name: &str,
    new_name: &str,
) -> Vec<String> {
    for name in &mut order {
        if name == old_name {
            *name = new_name.to_string();
        }
    }
    order
}

fn write_order_map(
    root: &Path,
    map: &std::collections::HashMap<String, Vec<String>>,
) -> io::Result<()> {
    let json = serde_json::to_string_pretty(map).map_err(io::Error::other)?;
    recovery::atomic_write(&root.join(ORDER_FILE), json.as_bytes())
}

/// 把路径分隔符统一为正斜杠（用于前端跨平台一致 id）
pub fn path_to_unix(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

pub fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_iso_utc(secs)
}

/// 紧凑时间戳，用于新建文件名（如 20260628T153000），保证唯一
pub fn now_compact() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
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

fn format_iso_utc(secs: u64) -> String {
    let days = secs / 86400;
    let rem = secs % 86400;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;

    let (y, mo, d) = civil_from_days(days as i64);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, m, s)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_dir(name: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("pp_test_{name}_{nanos}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 问题5核心验证：save → read round-trip，body 必须完整保留
    #[test]
    fn save_read_roundtrip_preserves_body() {
        let dir = std::env::temp_dir().join("pp_test_roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 先用 create_prompt 建一个
        let abs = create_prompt(&dir, "测试", "我的提示词").unwrap();

        // 用 save_prompt 写入结构化内容（模拟前端 doSave）
        let req = SaveRequest {
            title: "改过的标题".into(),
            copy_mode: "markdown".into(),
            body: "这是正文内容\n\n## 第二段\n\n- 列表项1\n- 列表项2".into(),
            category: None,
        };
        let new_abs = save_prompt(&dir, &abs, &req).unwrap();

        // 读回，验证 body 完整（用返回的新路径，可能因标题重命名）
        let content = read_prompt(&new_abs).unwrap();
        assert_eq!(content.meta.title, "改过的标题");
        assert!(
            content.body.contains("这是正文内容"),
            "body 应包含正文，实际: {}",
            content.body
        );
        assert!(content.body.contains("列表项1"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 多次保存后格式不漂移（问题5的根因场景）
    #[test]
    fn multiple_saves_stay_consistent() {
        let dir = std::env::temp_dir().join("pp_test_multi");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let abs = create_prompt(&dir, "未分类", "多次保存").unwrap();

        // 连续保存 3 次，每次改 body 和标题（标题变化会触发重命名，需追踪路径）
        let mut cur_abs = abs;
        for i in 0..3 {
            let body = format!("第 {} 次的内容", i);
            let req = SaveRequest {
                title: format!("标题{}", i),
                copy_mode: "markdown".into(),
                body: body.clone(),
                category: None,
            };
            cur_abs = save_prompt(&dir, &cur_abs, &req).unwrap();

            let content = read_prompt(&cur_abs).unwrap();
            assert_eq!(content.meta.title, format!("标题{}", i));
            assert_eq!(content.body, body, "第 {} 次保存后 body 不一致", i);
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Bug1 验证：新建空分类后 scan 能看到它（count=0）
    #[test]
    fn empty_category_appears_in_scan() {
        let dir = std::env::temp_dir().join("pp_test_empty_cat");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 创建一个空分类（只建文件夹，无 .md）
        create_category(&dir, "空分类").unwrap();

        // 扫描，空分类应该出现
        let res = scan_prompts(&dir).unwrap();
        assert!(
            res.categories
                .iter()
                .any(|c| c.name == "空分类" && c.count == 0),
            "空分类应出现在列表中，实际分类: {:?}",
            res.categories
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 验证 rename_category：重命名文件夹
    #[test]
    fn rename_category_moves_files() {
        let dir = std::env::temp_dir().join("pp_test_rename_cat");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 在旧分类下建一个 prompt
        create_prompt(&dir, "旧分类", "测试").unwrap();
        assert!(dir.join("旧分类").exists());

        // 重命名分类
        rename_category(&dir, "旧分类", "新分类").unwrap();

        // 旧目录应消失，新目录存在且含文件
        assert!(!dir.join("旧分类").exists(), "旧目录应已重命名");
        assert!(dir.join("新分类").exists());

        let res = scan_prompts(&dir).unwrap();
        assert!(res.categories.iter().any(|c| c.name == "新分类"));
        assert!(res.prompts.iter().any(|p| p.category == "新分类"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rename_category_rejects_existing_target_without_merging() {
        let dir = fresh_dir("cat_no_merge");

        let old_prompt = create_prompt(&dir, "旧分类", "旧文件").unwrap();
        let new_prompt = create_prompt(&dir, "新分类", "已有文件").unwrap();

        let err = rename_category(&dir, "旧分类", "新分类").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(old_prompt.exists(), "旧分类文件不能被删除或移动");
        assert!(new_prompt.exists(), "目标分类现有文件不能被覆盖");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 验证 frontmatter 解析容错：旧文件含 tags 字段也能正常解析（tags 被忽略）
    #[test]
    fn parse_legacy_with_tags_field() {
        let content = "---\ntitle: 旧格式\ntags: [a, b]\n---\n\n正文";
        let (meta, body) = parse_markdown(content);
        assert_eq!(meta.title, "旧格式");
        assert_eq!(body, "正文");
    }

    /// 验证 order.json：写入顺序后 scan 能按该顺序返回
    #[test]
    fn order_json_controls_sort_within_category() {
        let dir = std::env::temp_dir().join("pp_test_order");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 建两个 prompt，拿到真实路径（现在用时间戳命名）
        let p1 = create_prompt(&dir, "写作", "甲").unwrap();
        let p2 = create_prompt(&dir, "写作", "乙").unwrap();
        let p1_rel = path_to_unix(p1.strip_prefix(&dir).unwrap());
        let p2_rel = path_to_unix(p2.strip_prefix(&dir).unwrap());

        // 写入自定义顺序：乙 在 甲 前面
        reorder_category(&dir, "写作", &[p2_rel.clone(), p1_rel.clone()]).unwrap();

        let res = scan_prompts(&dir).unwrap();
        let cat_prompts: Vec<_> = res
            .prompts
            .iter()
            .filter(|p| p.category == "写作")
            .collect();
        assert_eq!(cat_prompts.len(), 2);
        assert_eq!(cat_prompts[0].title, "乙", "乙应在前面");
        assert_eq!(cat_prompts[1].title, "甲");
        assert_eq!(cat_prompts[0].order, Some(0));
        assert_eq!(cat_prompts[1].order, Some(1));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 验证不在 order.json 里的 prompt 排到分类末尾
    #[test]
    fn unlisted_prompt_goes_last() {
        let dir = std::env::temp_dir().join("pp_test_order_unlisted");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let p1 = create_prompt(&dir, "写作", "甲").unwrap();
        let _p2 = create_prompt(&dir, "写作", "乙").unwrap();
        let _p3 = create_prompt(&dir, "写作", "丙").unwrap();
        let p1_rel = path_to_unix(p1.strip_prefix(&dir).unwrap());

        // order.json 只列了 甲（乙丙 未列入，应排末尾）
        reorder_category(&dir, "写作", &[p1_rel]).unwrap();

        let res = scan_prompts(&dir).unwrap();
        let cat: Vec<_> = res
            .prompts
            .iter()
            .filter(|p| p.category == "写作")
            .collect();
        assert_eq!(cat[0].title, "甲");
        // 甲有 order=0，乙和丙 order=None 排其后
        assert_eq!(cat[0].order, Some(0));
        assert!(cat[1].order.is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 验证保存时按标题自动重命名文件（时间戳名 → 标题名）
    #[test]
    fn save_renames_file_to_title() {
        let dir = std::env::temp_dir().join("pp_test_rename_on_save");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 新建（文件名是时间戳）
        let abs = create_prompt(&dir, "写作", "初始标题").unwrap();
        let old_name = abs.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            old_name.ends_with(".md") && old_name.len() >= 15,
            "新建文件名应为时间戳，实际: {old_name}"
        );

        // 保存时改成有意义的标题
        let req = SaveRequest {
            title: "我的代码审查清单".into(),
            copy_mode: "markdown".into(),
            body: "正文内容".into(),
            category: None,
        };
        let new_abs = save_prompt(&dir, &abs, &req).unwrap();
        let new_name = new_abs.file_name().unwrap().to_string_lossy().to_string();

        assert_eq!(new_name, "我的代码审查清单.md", "保存后文件名应为标题");
        assert!(!abs.exists(), "旧时间戳文件应已删除");
        assert!(new_abs.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_creates_history_snapshot_and_updates_order_path() {
        let dir = fresh_dir("save_history_order");
        let first = create_prompt(&dir, "写作", "甲").unwrap();
        let second = create_prompt(&dir, "写作", "乙").unwrap();
        let first_rel = path_to_unix(first.strip_prefix(&dir).unwrap());
        let second_rel = path_to_unix(second.strip_prefix(&dir).unwrap());
        reorder_category(&dir, "写作", &[first_rel, second_rel.clone()]).unwrap();

        let req = SaveRequest {
            title: "甲改名".into(),
            copy_mode: "markdown".into(),
            body: "新版正文".into(),
            category: None,
        };
        let renamed = save_prompt(&dir, &first, &req).unwrap();
        let renamed_rel = path_to_unix(renamed.strip_prefix(&dir).unwrap());

        let order = load_order_map(&dir);
        assert_eq!(
            order.get("写作").unwrap(),
            &vec![renamed_rel.clone(), second_rel],
            "排序文件应映射到重命名后的路径"
        );

        let entries = recovery::list_recovery(&dir).unwrap();
        assert!(
            entries
                .iter()
                .any(|entry| entry.kind == "history" && entry.original_path.ends_with(".md")),
            "保存覆盖前应留下 history 恢复记录"
        );

        let res = scan_prompts(&dir).unwrap();
        let renamed_prompt = res.prompts.iter().find(|p| p.path == renamed_rel).unwrap();
        assert_eq!(renamed_prompt.body, "新版正文");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_fails_when_recovery_unavailable_and_keeps_original() {
        let dir = fresh_dir("save_recovery_unavailable");
        let abs = create_prompt(&dir, "写作", "原稿").unwrap();
        std::fs::write(&abs, "---\ntitle: 原稿\n---\n\n旧正文").unwrap();
        std::fs::write(dir.join(".recovery"), "not a directory").unwrap();

        let req = SaveRequest {
            title: "新稿".into(),
            copy_mode: "markdown".into(),
            body: "新正文".into(),
            category: None,
        };
        let err = save_prompt(&dir, &abs, &req).unwrap_err();
        assert!(
            err.kind() == io::ErrorKind::AlreadyExists || err.kind() == io::ErrorKind::InvalidInput
        );
        assert!(abs.exists(), "snapshot 失败时原稿必须保留");
        assert_eq!(read_prompt(&abs).unwrap().body, "旧正文");
        assert!(
            !dir.join("写作").join("新稿.md").exists(),
            "snapshot 失败时不能继续写入新文件"
        );

        let _ = std::fs::remove_file(dir.join(".recovery"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_order_failure_keeps_old_and_removes_new_file() {
        let dir = fresh_dir("save_order_failure");
        let abs = create_prompt(&dir, "写作", "原稿").unwrap();
        std::fs::write(&abs, "---\ntitle: 原稿\n---\n\n旧正文").unwrap();
        std::fs::create_dir(dir.join(ORDER_FILE)).unwrap();

        let req = SaveRequest {
            title: "新稿".into(),
            copy_mode: "markdown".into(),
            body: "新正文".into(),
            category: None,
        };
        let err = save_prompt(&dir, &abs, &req).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(abs.exists(), "排序写失败时旧文件仍应存在");
        assert_eq!(read_prompt(&abs).unwrap().body, "旧正文");
        assert!(
            !dir.join("写作").join("新稿.md").exists(),
            "排序写失败时应撤掉新文件"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn save_supports_tmp_alias_paths_without_rejecting_library_file() {
        let dir = PathBuf::from("/tmp").join(
            fresh_dir("tmp_alias")
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap(),
        );
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let abs = create_prompt(&dir, "写作", "原稿").unwrap();
        let req = SaveRequest {
            title: "新稿".into(),
            copy_mode: "markdown".into(),
            body: "通过 /tmp 别名保存".into(),
            category: None,
        };

        let new_abs = save_prompt(&dir, &abs, &req).unwrap();
        assert_eq!(read_prompt(&new_abs).unwrap().body, "通过 /tmp 别名保存");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_to_missing_new_category_directory_succeeds() {
        let dir = fresh_dir("save_missing_category");
        let abs = create_prompt(&dir, "未分类", "原稿").unwrap();
        let req = SaveRequest {
            title: "原稿".into(),
            copy_mode: "markdown".into(),
            body: "移动到新分类".into(),
            category: Some("新分类".into()),
        };

        let new_abs = save_prompt(&dir, &abs, &req).unwrap();
        assert!(new_abs.ends_with(Path::new("新分类").join("原稿.md")));
        assert_eq!(read_prompt(&new_abs).unwrap().body, "移动到新分类");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 验证 rename_prompt：改标题 + 移动分类
    #[test]
    fn rename_and_move_category() {
        let dir = std::env::temp_dir().join("pp_test_rename");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let abs = create_prompt(&dir, "旧分类", "旧标题").unwrap();
        let new_abs = rename_prompt(&dir, &abs, "新标题", "新分类").unwrap();

        // 旧文件应不存在
        assert!(!abs.exists(), "旧文件应已移动");
        // 新文件应在 新分类 目录下
        assert!(new_abs.to_string_lossy().contains("新分类"));
        assert!(new_abs.exists());

        // 内容正确
        let content = read_prompt(&new_abs).unwrap();
        assert_eq!(content.meta.title, "新标题");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rename_prompt_order_failure_keeps_old_and_removes_new_file() {
        let dir = fresh_dir("rename_order_failure");
        let abs = create_prompt(&dir, "旧分类", "旧标题").unwrap();
        std::fs::write(&abs, "---\ntitle: 旧标题\n---\n\n旧正文").unwrap();
        std::fs::create_dir(dir.join(ORDER_FILE)).unwrap();

        let err = rename_prompt(&dir, &abs, "新标题", "新分类").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(abs.exists(), "排序写失败时旧文件仍应存在");
        assert_eq!(read_prompt(&abs).unwrap().body, "旧正文");
        assert!(
            !dir.join("新分类").join("新标题.md").exists(),
            "排序写失败时应撤掉新文件"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_prompt_records_recoverable_snapshot() {
        let dir = fresh_dir("delete_recovery");
        let abs = create_prompt(&dir, "写作", "可恢复").unwrap();
        let req = SaveRequest {
            title: "可恢复".into(),
            copy_mode: "markdown".into(),
            body: "删除前正文".into(),
            category: None,
        };
        let abs = save_prompt(&dir, &abs, &req).unwrap();

        delete_prompt(&dir, &abs).unwrap();
        assert!(!abs.exists(), "删除后原文件应不存在");

        let entries = recovery::list_recovery(&dir).unwrap();
        let deleted = entries
            .iter()
            .find(|entry| entry.kind == "deleted")
            .unwrap();
        let restored = recovery::restore_recovery(&dir, &deleted.id).unwrap();
        assert!(restored.exists(), "恢复应写回文件");
        assert_eq!(read_prompt(&restored).unwrap().body, "删除前正文");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_fails_when_recovery_unavailable_and_keeps_file() {
        let dir = fresh_dir("delete_recovery_unavailable");
        let abs = create_prompt(&dir, "写作", "可恢复").unwrap();
        std::fs::write(&abs, "---\ntitle: 可恢复\n---\n\n删除前正文").unwrap();
        std::fs::write(dir.join(".recovery"), "not a directory").unwrap();

        let err = delete_prompt(&dir, &abs).unwrap_err();
        assert!(
            err.kind() == io::ErrorKind::AlreadyExists || err.kind() == io::ErrorKind::InvalidInput
        );
        assert!(abs.exists(), "snapshot 失败时不能删除原文件");
        assert_eq!(read_prompt(&abs).unwrap().body, "删除前正文");

        let _ = std::fs::remove_file(dir.join(".recovery"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn save_fails_when_recovery_dir_is_symlink_and_keeps_original() {
        use std::os::unix::fs::symlink;

        let dir = fresh_dir("save_recovery_symlink");
        let outside = fresh_dir("save_recovery_symlink_outside");
        let abs = create_prompt(&dir, "写作", "原稿").unwrap();
        std::fs::write(&abs, "---\ntitle: 原稿\n---\n\n旧正文").unwrap();
        symlink(&outside, dir.join(".recovery")).unwrap();

        let req = SaveRequest {
            title: "新稿".into(),
            copy_mode: "markdown".into(),
            body: "新正文".into(),
            category: None,
        };
        let err = save_prompt(&dir, &abs, &req).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(abs.exists());
        assert_eq!(read_prompt(&abs).unwrap().body, "旧正文");
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());

        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn restore_recovery_never_overwrites_existing_target() {
        let dir = fresh_dir("restore_no_overwrite");
        let abs = create_prompt(&dir, "写作", "原稿").unwrap();
        let _entry = recovery::snapshot(&dir, &abs, "history").unwrap();
        std::fs::write(&abs, "---\ntitle: 原稿\n---\n\n当前正文").unwrap();

        let entry = recovery::list_recovery(&dir).unwrap().remove(0);
        let restored = recovery::restore_recovery(&dir, &entry.id).unwrap();

        assert_ne!(restored, abs, "目标已存在时必须恢复为唯一副本");
        assert_eq!(read_prompt(&abs).unwrap().body, "当前正文");
        assert!(restored.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn restore_recovery_rejects_existing_leaf_symlink() {
        use std::os::unix::fs::symlink;

        let dir = fresh_dir("restore_leaf_symlink");
        let outside = fresh_dir("restore_leaf_symlink_outside");
        let abs = create_prompt(&dir, "写作", "原稿").unwrap();
        std::fs::write(&abs, "---\ntitle: 原稿\n---\n\n历史正文").unwrap();
        let entry = recovery::snapshot(&dir, &abs, "history").unwrap();
        std::fs::remove_file(&abs).unwrap();
        let outside_target = outside.join("target.md");
        std::fs::write(&outside_target, "outside").unwrap();
        symlink(&outside_target, &abs).unwrap();

        let err = recovery::restore_recovery(&dir, &entry.id).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(std::fs::read_to_string(&outside_target).unwrap(), "outside");

        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    /// Bug：scan_prompts 不能把 .trash/ 里的备份文件扫进列表。
    /// clean_local_extra 会把被删/被远程覆盖的 prompt 备份到 .trash/，
    /// 若 scan 不剪掉 .trash 目录，这些备份会以 category=".trash" 泄漏进列表，
    /// 造成"层级错乱"（截图里看到的 "改写润色 / .trash"）。
    #[test]
    fn scan_excludes_trash_directory() {
        let dir = std::env::temp_dir().join("pp_test_scan_trash");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 真实分类 + 真实 prompt
        create_prompt(&dir, "写作", "真实提示词").unwrap();

        // 模拟 clean_local_extra 产生的备份：.trash/改写润色_20260628T153000.md
        let trash = dir.join(".trash");
        std::fs::create_dir_all(&trash).unwrap();
        std::fs::write(
            trash.join("改写润色_20260628T153000.md"),
            "---\ntitle: 改写润色\n---\n\n正文",
        )
        .unwrap();

        // 隐藏目录里的文件也不应出现
        let hidden = dir.join(".cache");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(hidden.join(" leaked.md"), "---\ntitle: 泄漏\n---\n\nx").unwrap();

        let res = scan_prompts(&dir).unwrap();

        // 只有 1 个真实 prompt，.trash 和隐藏目录的都不算
        assert_eq!(
            res.prompts.len(),
            1,
            "应只扫到 1 个真实 prompt，实际: {:?}",
            res.prompts.iter().map(|p| &p.path).collect::<Vec<_>>()
        );
        assert_eq!(res.prompts[0].category, "写作");
        // 任何 prompt 的分类都不应是 .trash
        assert!(
            !res.prompts.iter().any(|p| p.category == ".trash"),
            ".trash 里的备份不应出现在列表"
        );
        // .trash 不应被注册为分类
        assert!(
            !res.categories.iter().any(|c| c.name == ".trash"),
            ".trash 不应是分类"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn scan_excludes_symlinked_prompts_and_categories() {
        use std::os::unix::fs::symlink;

        let dir = fresh_dir("scan_symlink");
        let outside = fresh_dir("scan_symlink_outside");
        create_prompt(&dir, "写作", "真实提示词").unwrap();
        std::fs::write(outside.join("leaked.md"), "---\ntitle: 泄漏\n---\n\nx").unwrap();
        symlink(outside.join("leaked.md"), dir.join("linked.md")).unwrap();
        symlink(&outside, dir.join("linked-category")).unwrap();

        let res = scan_prompts(&dir).unwrap();
        assert_eq!(res.prompts.len(), 1);
        assert_eq!(res.prompts[0].title, "真实提示词");
        assert!(
            !res.categories.iter().any(|c| c.name == "linked-category"),
            "符号链接目录不能被登记为分类"
        );

        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn category_creation_rejects_symlink_boundary() {
        use std::os::unix::fs::symlink;

        let dir = fresh_dir("category_symlink_boundary");
        let outside = fresh_dir("category_symlink_outside");
        symlink(&outside, dir.join("链接分类")).unwrap();

        let err = create_prompt(&dir, "链接分类", "不应写入").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(
            std::fs::read_dir(&outside).unwrap().next().is_none(),
            "符号链接分类边界不能被跟随写入"
        );

        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rename_category_order_write_failure_keeps_directory_names() {
        use std::os::unix::fs::symlink;

        let dir = fresh_dir("rename_category_order_failure");
        create_prompt(&dir, "旧名", "提示词").unwrap();
        let order_target = dir.join("order-target.json");
        std::fs::write(
            &order_target,
            serde_json::json!({ "旧名": ["旧名/example.md"] }).to_string(),
        )
        .unwrap();
        symlink(&order_target, dir.join(ORDER_FILE)).unwrap();

        let err = rename_category(&dir, "旧名", "新名").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(dir.join("旧名").exists(), "排序写失败前不能改名旧目录");
        assert!(!dir.join("新名").exists(), "排序写失败时不能留下新目录");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 验证分类自定义顺序：写入 .category-order.json 后 scan 按该顺序返回
    #[test]
    fn category_order_controls_display_sequence() {
        let dir = std::env::temp_dir().join("pp_test_cat_order");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        create_category(&dir, "写作").unwrap();
        create_category(&dir, "编程").unwrap();
        create_category(&dir, "翻译").unwrap();

        // 写入自定义顺序：翻译 → 编程 → 写作
        save_category_order(&dir, &["翻译".into(), "编程".into(), "写作".into()]).unwrap();

        let res = scan_prompts(&dir).unwrap();
        let names: Vec<_> = res.categories.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["翻译", "编程", "写作"], "应按自定义顺序排列");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 不在 order 里的分类按字母序追加到末尾
    #[test]
    fn unlisted_categories_appended_alphabetically() {
        let dir = std::env::temp_dir().join("pp_test_cat_order_unlisted");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        create_category(&dir, "alpha").unwrap();
        create_category(&dir, "beta").unwrap();
        create_category(&dir, "gamma").unwrap();
        create_category(&dir, "delta").unwrap();

        // order 只列了 gamma 和 alpha
        save_category_order(&dir, &["gamma".into(), "alpha".into()]).unwrap();

        let res = scan_prompts(&dir).unwrap();
        let names: Vec<_> = res.categories.iter().map(|c| c.name.as_str()).collect();
        // gamma、alpha 按 order；beta、delta 不在 order 里，按字母序追加
        assert_eq!(names, vec!["gamma", "alpha", "beta", "delta"]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 没有 .category-order.json 时，回退到全字母序
    #[test]
    fn category_order_missing_falls_back_to_alphabetical() {
        let dir = std::env::temp_dir().join("pp_test_cat_order_missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        create_category(&dir, "zebra").unwrap();
        create_category(&dir, "apple").unwrap();

        let res = scan_prompts(&dir).unwrap();
        let names: Vec<_> = res.categories.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["apple", "zebra"]);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 重命名分类时，.category-order.json 里的旧名同步更新
    #[test]
    fn rename_category_updates_order_file() {
        let dir = std::env::temp_dir().join("pp_test_cat_rename_order");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        create_category(&dir, "旧名").unwrap();
        create_category(&dir, "其他").unwrap();
        save_category_order(&dir, &["旧名".into(), "其他".into()]).unwrap();

        rename_category(&dir, "旧名", "新名").unwrap();

        let order = load_category_order(&dir);
        assert_eq!(order, vec!["新名", "其他"], "旧名应已替换为新名");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// .category-order.json 损坏时容错（返回空，回退字母序）
    #[test]
    fn category_order_corrupt_file_is_ignored() {
        let dir = std::env::temp_dir().join("pp_test_cat_order_corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        create_category(&dir, "beta").unwrap();
        create_category(&dir, "alpha").unwrap();
        // 写入损坏的 JSON
        std::fs::write(dir.join(CATEGORY_ORDER_FILE), "{ not valid json").unwrap();

        let res = scan_prompts(&dir).unwrap();
        let names: Vec<_> = res.categories.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"], "损坏文件应被忽略，回退字母序");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
