//! 台标库：本地文件列表、写入、删除。
//!
//! 文件放在 `<cache>/logos/`（F-011 §6）。公开取图走 `/logo/<file>`。
//! 只接受白名单扩展名；文件名去掉路径成分。

use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{bail, Context, Result};
use serde::Serialize;

/// 允许的图片扩展名。
pub const ALLOWED_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "avif"];

/// 台标目录。
pub fn dir(cache_dir: &str) -> PathBuf {
    Path::new(cache_dir).join("logos")
}

/// 原图备份目录。
pub fn orig_dir(cache_dir: &str) -> PathBuf {
    dir(cache_dir).join(".orig")
}

/// 列表项。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Entry {
    pub file: String,
    pub name: String,
    pub ext: String,
    pub bytes: u64,
    pub mtime: u64,
    pub url: String,
    pub has_original: bool,
    pub animated: bool,
}

/// 文件名只保留安全字符；拒绝空名、`..` 与隐藏文件。
pub fn sanitize(name: &str) -> Option<String> {
    let name = name.trim().rsplit(['/', '\\']).next().unwrap_or("").trim();
    if name.is_empty() || name.starts_with('.') || name.contains("..") {
        return None;
    }
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.trim_matches('.').is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// 扩展名小写；不在白名单时返回 `None`。
pub fn check_ext(ext: &str) -> Option<String> {
    let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if ALLOWED_EXTS.contains(&ext.as_str()) {
        Some(ext)
    } else {
        None
    }
}

/// 列出目录中的台标，按文件名排序。
pub fn list(cache_dir: &str) -> Vec<Entry> {
    let directory = dir(cache_dir);
    let Ok(read) = fs::read_dir(&directory) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = read
        .filter_map(|item| item.ok())
        .filter(|item| item.file_type().map(|kind| kind.is_file()).unwrap_or(false))
        .filter_map(|item| {
            let file = item.file_name().to_string_lossy().to_string();
            if file.starts_with('.') {
                return None;
            }
            let ext = file
                .rsplit_once('.')
                .map(|(_, ext)| ext.to_ascii_lowercase())?;
            if !ALLOWED_EXTS.contains(&ext.as_str()) {
                return None;
            }
            let meta = item.metadata().ok()?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|value| value.as_secs())
                .unwrap_or(0);
            Some(Entry {
                name: file
                    .rsplit_once('.')
                    .map(|(name, _)| name)
                    .unwrap_or(&file)
                    .to_string(),
                has_original: orig_dir(cache_dir).join(&file).is_file(),
                animated: is_animated(&directory.join(&file)),
                file: file.clone(),
                ext,
                bytes: meta.len(),
                mtime,
                url: format!("/logo/{file}"),
            })
        })
        .collect();
    entries.sort_by(|a, b| a.file.cmp(&b.file));
    entries
}

/// 写入一个台标；返回落盘项。
pub fn write(cache_dir: &str, name: &str, ext: &str, data: &[u8]) -> Result<Entry> {
    let name = sanitize(name).context("文件名不合法")?;
    let ext = check_ext(ext).context("扩展名不在白名单")?;
    if data.is_empty() {
        bail!("内容为空");
    }
    let directory = dir(cache_dir);
    fs::create_dir_all(&directory).with_context(|| format!("建目录 {}", directory.display()))?;
    let file = format!("{name}.{ext}");
    let path = directory.join(&file);
    fs::write(&path, data).with_context(|| format!("写 {}", path.display()))?;
    let mtime = fs::metadata(&path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
        .unwrap_or(0);
    Ok(Entry {
        url: format!("/logo/{file}"),
        has_original: false,
        animated: false,
        file,
        name,
        ext,
        bytes: data.len() as u64,
        mtime,
    })
}

/// 是否为 APNG（含 `acTL` 块）。
fn is_animated(path: &Path) -> bool {
    fs::read(path)
        .map(|data| data.windows(4).any(|window| window == b"acTL"))
        .unwrap_or(false)
}

/// 用 `.orig/` 备份覆盖当前台标；没有备份时返回 `Ok(false)`。
pub fn restore(cache_dir: &str, file: &str) -> Result<bool> {
    let file = sanitize(file).context("文件名不合法")?;
    let original = orig_dir(cache_dir).join(&file);
    if !original.is_file() {
        return Ok(false);
    }
    let target = dir(cache_dir).join(&file);
    fs::copy(&original, &target).with_context(|| format!("还原 {}", target.display()))?;
    Ok(true)
}

/// 删除一个台标；文件不存在时返回 `Ok(false)`。
pub fn remove(cache_dir: &str, file: &str) -> Result<bool> {
    let file = sanitize(file).context("文件名不合法")?;
    let path = dir(cache_dir).join(file);
    if !path.is_file() {
        return Ok(false);
    }
    fs::remove_file(&path).with_context(|| format!("删除 {}", path.display()))?;
    Ok(true)
}

/// 解析 `data:image/png;base64,...`；没有前缀时按纯 base64 处理。
pub fn decode_data_url(data: &str) -> Result<(Option<String>, Vec<u8>)> {
    let (mime, payload) = match data.split_once(',') {
        Some((head, payload)) if head.starts_with("data:") => {
            let mime = head
                .trim_start_matches("data:")
                .split(';')
                .next()
                .map(str::to_string);
            (mime, payload)
        }
        _ => (None, data),
    };
    Ok((mime, base64_decode(payload)?))
}

/// 标准与 URL 安全字母表的 base64 解码；忽略空白。
pub fn base64_decode(input: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => bail!("base64 字符不合法"),
        } as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_traversal() {
        assert_eq!(sanitize("cctv1.png").as_deref(), Some("cctv1.png"));
        assert_eq!(sanitize("../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(sanitize("a b.png").as_deref(), Some("a_b.png"));
        assert_eq!(sanitize("台标 1.png").as_deref(), Some("台标_1.png"));
        assert_eq!(sanitize(""), None);
        assert_eq!(sanitize(".."), None);
        assert_eq!(sanitize(".hidden"), None);
    }

    #[test]
    fn ext_whitelist() {
        assert_eq!(check_ext("PNG").as_deref(), Some("png"));
        assert_eq!(check_ext(".jpg").as_deref(), Some("jpg"));
        assert_eq!(check_ext("exe"), None);
    }

    #[test]
    fn decode_base64_and_data_url() {
        // "PNG" 的 base64 是 "UE5H"。
        assert_eq!(base64_decode("UE5H").unwrap(), b"PNG");
        assert_eq!(base64_decode("UE5H\n").unwrap(), b"PNG");
        let (mime, bytes) = decode_data_url("data:image/png;base64,UE5H").unwrap();
        assert_eq!(mime.as_deref(), Some("image/png"));
        assert_eq!(bytes, b"PNG");
        assert!(base64_decode("!!").is_err());
    }

    #[test]
    fn similarity_scores() {
        assert_eq!(similarity("CCTV-1", "cctv1"), 1.0);
        assert!(similarity("CCTV-1 综合", "cctv1") >= 0.9);
        assert!(similarity("湖南卫视", "浙江卫视") > 0.0);
        assert!(similarity("湖南卫视", "浙江卫视") < similarity("湖南卫视", "湖南卫视"));
        assert_eq!(similarity("", "x"), 0.0);
    }

    #[test]
    fn rename_keeps_extension() {
        let dir = std::env::temp_dir().join(format!("logo-rename-{}", uuid::Uuid::new_v4()));
        let cache = dir.to_string_lossy().to_string();
        let entry = write(&cache, "old", "png", b"img").expect("写入");
        let renamed = rename(&cache, &entry.file, "新台标").expect("重命名");
        assert_eq!(renamed.file, "新台标.png");
        assert!(!dir.join("logos").join("old.png").exists());
        assert!(dir.join("logos").join("新台标.png").is_file());
        write(&cache, "other", "png", b"x").expect("写入");
        assert!(
            rename(&cache, "other.png", "新台标").is_err(),
            "目标已存在应报错"
        );
        assert!(rename(&cache, "missing.png", "x").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn allowed_exts_match_target() {
        assert_eq!(ALLOWED_EXTS, &["png", "jpg", "jpeg", "webp", "bmp", "avif"]);
        for ext in ["png", "jpg", "jpeg", "webp", "bmp", "avif"] {
            assert_eq!(check_ext(ext).as_deref(), Some(ext));
        }
        for ext in ["gif", "svg", "txt"] {
            assert!(check_ext(ext).is_none(), "{ext}");
        }
    }

    #[test]
    fn write_list_and_remove() {
        let dir = std::env::temp_dir().join(format!("logo-test-{}", uuid::Uuid::new_v4()));
        let cache = dir.to_string_lossy().to_string();
        let entry = write(&cache, "台标 1", "png", b"data").expect("应写入");
        assert_eq!(entry.file, "台标_1.png");
        assert_eq!(entry.url, "/logo/台标_1.png");
        let entries = list(&cache);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].bytes, 4);
        assert!(remove(&cache, &entry.file).expect("应删除"));
        assert!(!remove(&cache, &entry.file).expect("再删应返回 false"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_uses_original_backup() {
        let root = std::env::temp_dir().join(format!("logo-orig-{}", uuid::Uuid::new_v4()));
        let cache = root.to_string_lossy().to_string();
        let entry = write(&cache, "c1", "png", b"animated").expect("写入");
        fs::create_dir_all(orig_dir(&cache)).expect("建原图目录");
        fs::write(orig_dir(&cache).join(&entry.file), b"static").expect("写原图");
        assert!(list(&cache)[0].has_original);
        assert!(restore(&cache, &entry.file).expect("还原"));
        assert_eq!(
            fs::read(dir(&cache).join(&entry.file)).expect("读"),
            b"static"
        );
        assert!(!restore(&cache, "missing.png").expect("无备份"));
        let _ = fs::remove_dir_all(&root);
    }
}

/// 重命名台标：保留原扩展名；目标已存在时报错。
pub fn rename(cache_dir: &str, file: &str, name: &str) -> Result<Entry> {
    let file = sanitize(file).context("文件名不合法")?;
    let Some((_, ext)) = file.rsplit_once('.') else {
        bail!("缺少扩展名");
    };
    let ext = check_ext(ext).context("扩展名不在白名单")?;
    let name = sanitize(name).context("文件名不合法")?;
    let directory = dir(cache_dir);
    let source = directory.join(&file);
    if !source.is_file() {
        bail!("文件不存在");
    }
    let target_name = format!("{name}.{ext}");
    let target = directory.join(&target_name);
    if target == source {
        return Ok(Entry {
            name,
            has_original: orig_dir(cache_dir).join(&target_name).is_file(),
            animated: is_animated(&target),
            file: target_name.clone(),
            ext,
            bytes: fs::metadata(&source).map(|meta| meta.len()).unwrap_or(0),
            mtime: 0,
            url: format!("/logo/{target_name}"),
        });
    }
    if target.exists() {
        bail!("目标已存在");
    }
    fs::rename(&source, &target).with_context(|| format!("重命名 {}", source.display()))?;
    let original = orig_dir(cache_dir).join(&file);
    if original.is_file() {
        let target_original = orig_dir(cache_dir).join(&target_name);
        let _ = fs::rename(original, target_original);
    }
    let meta = fs::metadata(&target).ok();
    Ok(Entry {
        url: format!("/logo/{target_name}"),
        has_original: orig_dir(cache_dir).join(&target_name).is_file(),
        animated: is_animated(&target),
        file: target_name,
        name,
        ext,
        bytes: meta.as_ref().map(|meta| meta.len()).unwrap_or(0),
        mtime: meta
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_secs())
            .unwrap_or(0),
    })
}

/// 名称相似度（0.0–1.0）：字符集合的 Jaccard 系数，忽略大小写与非字母数字。
pub fn similarity(name: &str, stem: &str) -> f64 {
    let left = normalize(name);
    let right = normalize(stem);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    if left.contains(&right) || right.contains(&left) {
        return 0.9;
    }
    let a: std::collections::HashSet<char> = left.chars().collect();
    let b: std::collections::HashSet<char> = right.chars().collect();
    let inter = a.intersection(&b).count() as f64;
    let union = a.union(&b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn normalize(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}
