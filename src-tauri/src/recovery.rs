use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

const RECOVERY_DIR: &str = ".recovery";
const RECORD_EXT: &str = "json";
const CONTENT_EXT: &str = "bak";
const ALLOWED_HIDDEN_FILES: &[&str] = &[".order.json", ".category-order.json"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryEntry {
    pub id: String,
    pub original_path: String,
    pub created_at: String,
    pub kind: String,
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "目标路径缺少父目录"))?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "目标必须是普通文件",
            ));
        }
    }
    fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "目标文件名无效"))?;
    let tmp_name = format!(".{}.{}.tmp", file_name, unique_id());
    let tmp_path = parent.join(tmp_name);

    let write_result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_with_written_temp(&tmp_path, path)?;
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    write_result
}

fn replace_with_written_temp(tmp_path: &Path, path: &Path) -> io::Result<()> {
    // std::fs::rename replaces an existing file on supported desktop platforms.
    // Never move the destination aside: failure must leave the original at its path.
    fs::rename(tmp_path, path)
}

pub fn snapshot(root: &Path, path: &Path, kind: &str) -> io::Result<RecoveryEntry> {
    let (source, rel) = checked_existing_path_and_rel(root, path)?;
    if !source.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "只能为文件创建恢复快照",
        ));
    }

    let rel = path_to_unix(&rel);
    let id = unique_id();
    let entry = RecoveryEntry {
        id: id.clone(),
        original_path: rel,
        created_at: now_iso(),
        kind: kind.to_string(),
    };

    let dir = safe_recovery_dir(root)?;
    fs::create_dir_all(&dir)?;
    atomic_write(
        &dir.join(format!("{id}.{CONTENT_EXT}")),
        &fs::read(&source)?,
    )?;
    let json = serde_json::to_vec_pretty(&entry).map_err(io::Error::other)?;
    atomic_write(&dir.join(format!("{id}.{RECORD_EXT}")), &json)?;
    Ok(entry)
}

pub fn list_recovery(root: &Path) -> io::Result<Vec<RecoveryEntry>> {
    let dir = safe_recovery_dir(root)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(RECORD_EXT) {
            continue;
        }
        if entry.file_type()?.is_symlink() {
            continue;
        }
        let Ok(raw) = fs::read_to_string(path) else {
            continue;
        };
        if let Ok(record) = serde_json::from_str::<RecoveryEntry>(&raw) {
            entries.push(record);
        }
    }
    entries.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(entries)
}

pub fn restore_recovery(root: &Path, id: &str) -> io::Result<PathBuf> {
    validate_id(id)?;
    let dir = safe_recovery_dir(root)?;
    let record_path = dir.join(format!("{id}.{RECORD_EXT}"));
    let content_path = dir.join(format!("{id}.{CONTENT_EXT}"));
    reject_symlink(&record_path)?;
    reject_symlink(&content_path)?;
    let raw = fs::read_to_string(record_path)?;
    let record: RecoveryEntry = serde_json::from_str(&raw).map_err(io::Error::other)?;
    let target = checked_path(root, &record.original_path)?;
    let restore_target = unique_restore_path(&target);

    if let Some(parent) = restore_target.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = fs::read(content_path)?;
    atomic_write(&restore_target, &bytes)?;
    Ok(restore_target)
}

pub fn checked_path(root: &Path, rel: impl AsRef<Path>) -> io::Result<PathBuf> {
    let rel = rel.as_ref();
    validate_relative_path(rel)?;

    let root = canonical_root(root)?;
    let candidate = root.join(rel);
    validate_no_symlink_prefix(&root, &candidate)?;
    Ok(candidate)
}

fn checked_existing_path_and_rel(root: &Path, path: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let rel = relative_to_root(root, path)?;
    validate_relative_path(&rel)?;

    let root = canonical_root(root)?;
    let candidate = root.join(&rel);
    validate_no_symlink_prefix(&root, &candidate)?;
    if !candidate.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "路径不存在"));
    }
    Ok((candidate, rel))
}

fn relative_to_root(root: &Path, path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        let canonical = canonical_root(root)?;
        path.strip_prefix(root)
            .or_else(|_| path.strip_prefix(&canonical))
            .map(Path::to_path_buf)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "路径不在库内"))
    } else {
        Ok(path.to_path_buf())
    }
}

fn validate_relative_path(rel: &Path) -> io::Result<()> {
    if rel.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "路径必须是相对路径",
        ));
    }
    if rel.as_os_str().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "路径不能为空"));
    }

    let count = rel.components().count();
    for (index, component) in rel.components().enumerate() {
        match component {
            Component::Normal(name) => {
                let Some(name) = name.to_str() else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "路径必须是 UTF-8",
                    ));
                };
                if name.is_empty() || name.starts_with('~') {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "路径包含隐藏或临时项",
                    ));
                }
                if name.starts_with('.')
                    && !(index + 1 == count && ALLOWED_HIDDEN_FILES.contains(&name))
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "路径包含隐藏项",
                    ));
                }
            }
            Component::CurDir => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "路径不能逃逸出库目录",
                ));
            }
        }
    }
    Ok(())
}

fn validate_no_symlink_prefix(root: &Path, candidate: &Path) -> io::Result<()> {
    let relative = candidate
        .strip_prefix(root)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "路径不在库内"))?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        let Component::Normal(name) = part else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "路径不能逃逸出库目录",
            ));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "路径包含符号链接",
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn canonical_root(root: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(root)?;
    fs::canonicalize(root)
}

fn reject_symlink(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "恢复路径不能是符号链接",
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
fn safe_recovery_dir(root: &Path) -> io::Result<PathBuf> {
    let dir = canonical_root(root)?.join(RECOVERY_DIR);
    reject_symlink(&dir)?;
    Ok(dir)
}
pub fn read_recovery(root: &Path, id: &str) -> io::Result<String> {
    validate_id(id)?;
    let path = safe_recovery_dir(root)?.join(format!("{id}.{CONTENT_EXT}"));
    reject_symlink(&path)?;
    fs::read_to_string(path)
}

fn unique_restore_path(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }

    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let stem = target
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("restored");
    let ext = target.extension().and_then(|s| s.to_str());
    for n in 1.. {
        let name = match ext {
            Some(ext) if !ext.is_empty() => format!("{stem}-恢复副本-{n}.{ext}"),
            _ => format!("{stem}-恢复副本-{n}"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn validate_id(id: &str) -> io::Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "恢复记录 ID 无效",
        ));
    }
    Ok(())
}

fn path_to_unix(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn unique_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{}", std::process::id())
}

fn now_iso() -> String {
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
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, m, s)
}

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
