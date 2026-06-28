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
                    if let Some(t) = map.get(&serde_yaml::Value::String("title".into())) {
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
            if path.is_dir() {
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

    let categories = cat_counts
        .into_iter()
        .map(|(name, count)| CategoryCount { name, count })
        .collect();

    Ok(ScanResult { prompts, categories })
}

/// .order.json 文件名
pub const ORDER_FILE: &str = ".order.json";

/// 加载 order 映射：{ 分类名: [相对路径, ...] }
fn load_order_map(root: &Path) -> std::collections::HashMap<String, Vec<String>> {
    let path = root.join(ORDER_FILE);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 重写某分类的顺序到 .order.json
pub fn reorder_category(
    root: &Path,
    category: &str,
    ordered_paths: &[String],
) -> io::Result<()> {
    let path = root.join(ORDER_FILE);
    let mut map: std::collections::HashMap<String, Vec<String>> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    map.insert(category.to_string(), ordered_paths.to_vec());
    let json = serde_json::to_string_pretty(&map).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(&path, json)
}

fn build_prompt(root: &Path, abs: &Path, order: Option<i32>) -> Option<Prompt> {
    let content = fs::read_to_string(abs).ok()?;
    let (mut meta, _body) = parse_markdown(&content);

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
/// 关键：杜绝裸文本往返导致的格式漂移
/// ────────────────────────────────────────────────
pub fn save_prompt(abs: &Path, req: &SaveRequest) -> io::Result<()> {
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

    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(abs, content)
}

/// 新建 prompt 文件，返回其绝对路径
pub fn create_prompt(root: &Path, category: &str, title: &str) -> io::Result<PathBuf> {
    let safe_cat = sanitize_filename::sanitize(category);
    let safe_title = sanitize_filename::sanitize(title);
    let dir = if safe_cat.is_empty() || safe_cat == "未分类" {
        root.to_path_buf()
    } else {
        root.join(&safe_cat)
    };
    fs::create_dir_all(&dir)?;

    let mut file_name = format!("{}.md", safe_title);
    let mut n = 1;
    while dir.join(&file_name).exists() {
        file_name = format!("{}-{}.md", safe_title, n);
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
    fs::write(&path, content)?;
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
    let safe_cat = sanitize_filename::sanitize(new_category);
    let safe_title = sanitize_filename::sanitize(new_title);
    let new_dir = if safe_cat.is_empty() || safe_cat == "未分类" {
        root.to_path_buf()
    } else {
        root.join(&safe_cat)
    };
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

    fs::write(&new_abs, &content)?;

    // 如果路径变了，删除旧文件
    if new_abs != old_abs {
        let _ = fs::remove_file(old_abs);
    }

    Ok(new_abs)
}

/// 新建分类（即创建文件夹）（问题3）
pub fn create_category(root: &Path, name: &str) -> io::Result<PathBuf> {
    let safe = sanitize_filename::sanitize(name);
    if safe.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "分类名无效"));
    }
    let dir = root.join(&safe);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 重命名分类（优化3）：重命名文件夹，内部所有 .md 文件随之移动
/// 返回受影响的 .md 文件新路径列表
pub fn rename_category(root: &Path, old_name: &str, new_name: &str) -> io::Result<()> {
    let safe_old = sanitize_filename::sanitize(old_name);
    let safe_new = sanitize_filename::sanitize(new_name);
    if safe_new.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "新分类名无效"));
    }
    let old_dir = root.join(&safe_old);
    let new_dir = root.join(&safe_new);

    if !old_dir.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "原分类文件夹不存在",
        ));
    }

    // 目标已存在：合并（移动所有文件过去），否则重命名
    if new_dir.exists() {
        for entry in fs::read_dir(&old_dir)? {
            let entry = entry?;
            let from = entry.path();
            let to = new_dir.join(entry.file_name());
            if from != to {
                // 若目标已存在同名文件，覆盖
                let _ = fs::remove_file(&to);
                fs::rename(&from, &to)?;
            }
        }
        fs::remove_dir(&old_dir)?;
    } else {
        fs::rename(&old_dir, &new_dir)?;
    }
    Ok(())
}

pub fn delete_prompt(abs: &Path) -> io::Result<()> {
    fs::remove_file(abs)
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

fn format_iso_utc(secs: u64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        save_prompt(&abs, &req).unwrap();

        // 读回，验证 body 完整
        let content = read_prompt(&abs).unwrap();
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

        // 连续保存 3 次，每次改 body
        for i in 0..3 {
            let body = format!("第 {} 次的内容", i);
            let req = SaveRequest {
                title: format!("标题{}", i),
                copy_mode: "markdown".into(),
                body: body.clone(),
            };
            save_prompt(&abs, &req).unwrap();

            let content = read_prompt(&abs).unwrap();
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
            res.categories.iter().any(|c| c.name == "空分类" && c.count == 0),
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

        // 建两个 prompt（默认按文件名/时间排序）
        create_prompt(&dir, "写作", "甲").unwrap();
        create_prompt(&dir, "写作", "乙").unwrap();

        // 写入自定义顺序：乙 在 甲 前面
        reorder_category(
            &dir,
            "写作",
            &["写作/乙.md".to_string(), "写作/甲.md".to_string()],
        )
        .unwrap();

        let res = scan_prompts(&dir).unwrap();
        let cat_prompts: Vec<_> = res.prompts.iter().filter(|p| p.category == "写作").collect();
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

        create_prompt(&dir, "写作", "甲").unwrap();
        create_prompt(&dir, "写作", "乙").unwrap();
        create_prompt(&dir, "写作", "丙").unwrap();

        // order.json 只列了 甲（丙 未列入，应排末尾）
        reorder_category(&dir, "写作", &["写作/甲.md".to_string()]).unwrap();

        let res = scan_prompts(&dir).unwrap();
        let cat: Vec<_> = res.prompts.iter().filter(|p| p.category == "写作").collect();
        assert_eq!(cat[0].title, "甲");
        // 甲有 order=0，乙和丙 order=None 排其后
        assert_eq!(cat[0].order, Some(0));
        assert!(cat[1].order.is_none());

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
}

