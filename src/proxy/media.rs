//! 清单改写：把上游媒体清单改成本地伪装地址。
//!
//! - 切片地址 → `{self_base}/{gid}/{cid}/<P>_<S>.<ext>`（绝对地址，S-DIS-04）。
//! - `<S>` = `sha256(上游绝对 URL)` 前 32 位十六进制。
//! - `<P>` = `sha256("<gid>:<cid>")` 前 16 位十六进制（自研，F-017 未解）。
//! - 主清单（含 `#EXT-X-STREAM-INF`）只把变体地址绝对化。

use sha2::{Digest, Sha256};
use url::Url;

/// 改写产物。
#[derive(Debug, PartialEq, Eq)]
pub struct Rendered {
    /// 改写后的清单正文。
    pub body: String,
    /// 上游是主清单（变体清单，不是切片）。
    pub is_master: bool,
    /// 新登记的切片：`(S, 上游绝对 URL, 扩展名)`。
    pub segments: Vec<(String, String, String)>,
}

/// 计算伪装前缀 `<P>`。
pub fn prefix(gid: &str, cid: &str) -> String {
    let digest = Sha256::digest(format!("{gid}:{cid}").as_bytes());
    hex::encode(digest)[..16].to_string()
}

/// 计算伪装后缀 `<S>`。
pub fn suffix(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    hex::encode(digest)[..32].to_string()
}

/// 解析切片文件名：返回 `(S, 扩展名)`。前缀缺失时也接受 `<S>.<ext>`。
pub fn parse_segment(name: &str) -> Option<(String, String)> {
    let (stem, ext) = name.rsplit_once('.')?;
    if ext.is_empty() {
        return None;
    }
    let suffix_part = stem.rsplit('_').next()?;
    if suffix_part.len() != 32 || !suffix_part.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some((suffix_part.to_ascii_lowercase(), ext.to_ascii_lowercase()))
}

/// 校验前缀。空前缀（伪装关闭）视为通过。
pub fn prefix_matches(expected: &str, name: &str) -> bool {
    match name.split_once('_') {
        Some((got, _)) => got.eq_ignore_ascii_case(expected),
        None => true,
    }
}

/// 每频道固定的扩展名轮换起点。
pub fn ext_offset(gid: &str, cid: &str) -> usize {
    Sha256::digest(format!("{gid}:{cid}").as_bytes())[0] as usize
}

/// 清单是否为主清单（含变体）。
pub fn is_master(body: &str) -> bool {
    body.contains("#EXT-X-STREAM-INF")
}

/// 清单的 `#EXT-X-TARGETDURATION`（秒）。缺失时返回 `None`。
pub fn target_duration(body: &str) -> Option<u64> {
    body.lines().find_map(|line| {
        line.trim()
            .strip_prefix("#EXT-X-TARGETDURATION:")?
            .trim()
            .parse()
            .ok()
    })
}

/// 主清单 `#EXT-X-STREAM-INF` 的属性白名单。
///
/// v0.8.2 实测只保留这 4 项；`PROGRAM-ID`、`NAME`、`AUDIO`、`SUBTITLES` 等均丢弃。
const VARIANT_ATTRS: [&str; 4] = ["BANDWIDTH", "CODECS", "RESOLUTION", "FRAME-RATE"];

/// 按 v0.8.2 白名单过滤 `#EXT-X-STREAM-INF` 属性，保持原顺序。
pub fn normalize_variant_attrs(attrs: &str) -> String {
    let kept: Vec<&str> = split_attrs(attrs)
        .into_iter()
        .filter(|part| {
            part.split_once('=')
                .map(|(name, _)| VARIANT_ATTRS.contains(&name.trim()))
                .unwrap_or(false)
        })
        .collect();
    if kept.is_empty() {
        "BANDWIDTH=1000000".to_string()
    } else {
        kept.join(",")
    }
}

/// 按逗号切分属性串；忽略引号内的逗号（如 `CODECS="avc1,mp4a"`）。
fn split_attrs(attrs: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut quoted = false;
    for (index, ch) in attrs.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                let part = attrs[start..index].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = attrs[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

/// 主清单里的最佳变体：返回 `(STREAM-INF 属性串, 绝对地址)`。
///
/// 按 `BANDWIDTH` 取最大；一条变体都没有时返回 `None`。
/// 属性串保持上游原样；对外输出前应经 [`normalize_variant_attrs`]。
pub fn best_variant(body: &str, base: &str) -> Option<(String, String)> {
    let mut attrs: Option<String> = None;
    let mut best: Option<(u64, String, String)> = None;
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("#EXT-X-STREAM-INF:") {
            attrs = Some(rest.to_string());
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(current) = attrs.take() else {
            continue;
        };
        let Ok(url) = absolute_url(base, trimmed) else {
            continue;
        };
        let bandwidth = bandwidth_of(&current);
        if best
            .as_ref()
            .map_or(true, |(known, _, _)| bandwidth > *known)
        {
            best = Some((bandwidth, current, url));
        }
    }
    best.map(|(_, attrs, url)| (attrs, url))
}

/// 解析 `BANDWIDTH=…`；缺失时按 0 处理。
fn bandwidth_of(attrs: &str) -> u64 {
    split_attrs(attrs)
        .into_iter()
        .find_map(|part| part.strip_prefix("BANDWIDTH="))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

/// 改写清单正文。
///
/// `base` 是清单自身的绝对 URL，用于解析相对地址。
/// `self_base` 是本服务的对外基址（`http://主机:端口`），用于拼绝对伪装地址。
/// `disguise` 关闭时切片地址改写为上游绝对地址。
pub fn rewrite_media(
    body: &str,
    base: &str,
    gid: &str,
    cid: &str,
    formats: &[String],
    disguise: bool,
    self_base: &str,
) -> Rendered {
    let is_master = is_master(body);
    let prefix = prefix(gid, cid);
    let offset = ext_offset(gid, cid);
    let mut out = String::with_capacity(body.len() + 256);
    let mut segments = Vec::new();
    let mut index = 0usize;

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            out.push_str(&rewrite_tags(rest, base));
            out.push('\n');
            continue;
        }
        let Ok(absolute) = absolute_url(base, trimmed) else {
            out.push_str(trimmed);
            out.push('\n');
            continue;
        };
        if is_master || !disguise {
            out.push_str(&absolute);
            out.push('\n');
            continue;
        }
        let ext = pick_ext(trimmed, formats, offset + index);
        let suffix = suffix(&absolute);
        out.push_str(&format!(
            "{self_base}/{gid}/{cid}/{prefix}_{suffix}.{ext}\n"
        ));
        segments.push((suffix, absolute, ext));
        index += 1;
    }

    Rendered {
        body: out,
        is_master,
        segments,
    }
}

/// 改写带 `URI="…"` 的标签（密钥、映射、备用音轨）。
fn rewrite_tags(tag: &str, base: &str) -> String {
    if !tag.contains("URI=\"") {
        return format!("#{tag}");
    }
    let mut out = String::from("#");
    let mut rest = tag;
    while let Some(start) = rest.find("URI=\"") {
        let (head, tail) = rest.split_at(start + 5);
        out.push_str(head);
        match tail.find('"') {
            Some(end) => {
                let (uri, after) = tail.split_at(end);
                let absolute = absolute_url(base, uri).unwrap_or_else(|_| uri.to_string());
                out.push_str(&absolute);
                rest = after;
            }
            None => {
                out.push_str(tail);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// 解析相对地址。
fn absolute_url(base: &str, reference: &str) -> Result<String, url::ParseError> {
    if reference.starts_with("http://") || reference.starts_with("https://") {
        return Ok(reference.to_string());
    }
    Url::parse(base).map(|base| base.join(reference).unwrap_or(base).to_string())
}

/// 选择伪装扩展名。
///
/// v0.8.2 实测：上游扩展名已在 `formats` 中时直接保留（如 `.jpg` 切片仍为 `.jpg`）；
/// 否则按频道序列轮换。伪装关闭或列表为空时用原扩展名。
fn pick_ext(reference: &str, formats: &[String], index: usize) -> String {
    let original = reference
        .split('?')
        .next()
        .and_then(|path| path.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase());
    if formats.is_empty() {
        return original.unwrap_or_else(|| "bin".to_string());
    }
    if let Some(ext) = &original {
        if formats
            .iter()
            .any(|format| format.trim_start_matches('.').eq_ignore_ascii_case(ext))
        {
            return ext.clone();
        }
    }
    formats[index % formats.len()]
        .trim_start_matches('.')
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formats() -> Vec<String> {
        ["jpg", "png", "gif", "bmp", "webp"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn prefix_is_stable_16_hex() {
        let first = prefix("g1", "c1");
        assert_eq!(first, prefix("g1", "c1"));
        assert_eq!(first.len(), 16);
        assert_ne!(first, prefix("g1", "c2"));
    }

    #[test]
    fn suffix_matches_sha256_prefix() {
        let url = "http://up.example/a/1.ts";
        let expected = hex::encode(Sha256::digest(url.as_bytes()))[..32].to_string();
        assert_eq!(suffix(url), expected);
    }

    #[test]
    fn relative_segments_are_absolutized_and_disguised() {
        let body = "#EXTM3U\n#EXTINF:4.0,\nseg1.ts\n#EXTINF:4.0,\n../b/seg2.ts\n";
        let rendered = rewrite_media(
            body,
            "http://up.example/live/a/index.m3u8",
            "g1",
            "c1",
            &formats(),
            true,
            "http://self",
        );
        assert!(!rendered.is_master);
        assert_eq!(rendered.segments.len(), 2);
        assert_eq!(rendered.segments[0].1, "http://up.example/live/a/seg1.ts");
        assert_eq!(rendered.segments[1].1, "http://up.example/live/b/seg2.ts");
        assert!(rendered
            .body
            .contains(&format!("http://self/g1/c1/{}_", prefix("g1", "c1"))));
        assert!(rendered
            .body
            .contains(&format!("_{}.", rendered.segments[0].0)));
        assert!(!rendered.body.contains("seg1.ts"));
    }

    #[test]
    fn master_playlist_keeps_variants_absolute() {
        let body = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\nv0/index.m3u8\n";
        let rendered = rewrite_media(
            body,
            "http://up.example/live/master.m3u8",
            "g1",
            "c1",
            &formats(),
            true,
            "http://self",
        );
        assert!(rendered.is_master);
        assert!(rendered.segments.is_empty());
        assert!(rendered
            .body
            .contains("http://up.example/live/v0/index.m3u8"));
    }

    #[test]
    fn target_duration_is_read() {
        assert_eq!(
            target_duration("#EXTM3U\n#EXT-X-TARGETDURATION:12\n"),
            Some(12)
        );
        assert_eq!(target_duration("#EXTM3U\n"), None);
    }

    #[test]
    fn variant_attrs_follow_target_allowlist() {
        let attrs = r#"PROGRAM-ID=1,BANDWIDTH=123456,AVERAGE-BANDWIDTH=100000,CODECS="avc1.64001f,mp4a.40.2",RESOLUTION=640x360,FRAME-RATE=25.000,NAME="hi",AUDIO="aud",SUBTITLES="sub""#;
        assert_eq!(
            normalize_variant_attrs(attrs),
            r#"BANDWIDTH=123456,CODECS="avc1.64001f,mp4a.40.2",RESOLUTION=640x360,FRAME-RATE=25.000"#
        );
        assert_eq!(normalize_variant_attrs("PROGRAM-ID=1"), "BANDWIDTH=1000000");
    }

    #[test]
    fn bandwidth_ignores_quoted_commas() {
        let attrs = r#"CODECS="avc1.64001f,mp4a.40.2",BANDWIDTH=800000"#;
        assert_eq!(bandwidth_of(attrs), 800000);
    }

    #[test]
    fn best_variant_takes_highest_bandwidth() {
        let body = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=640x360\n360/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1920x1080\n1080/index.m3u8\n";
        let (attrs, url) = best_variant(body, "http://up.example/live/master.m3u8").unwrap();
        assert_eq!(attrs, "BANDWIDTH=2000000,RESOLUTION=1920x1080");
        assert_eq!(url, "http://up.example/live/1080/index.m3u8");
        assert!(best_variant("#EXTM3U\nseg.ts\n", "http://up.example/a.m3u8").is_none());
    }

    #[test]
    fn key_uri_is_absolutized() {
        let body = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\nseg.ts\n";
        let rendered = rewrite_media(
            body,
            "http://up.example/live/a/index.m3u8",
            "g1",
            "c1",
            &formats(),
            true,
            "http://self",
        );
        assert!(rendered
            .body
            .contains("#EXT-X-KEY:METHOD=AES-128,URI=\"http://up.example/live/a/key.bin\""));
    }

    #[test]
    fn disguise_off_uses_absolute_urls() {
        let body = "#EXTM3U\n#EXTINF:4.0,\nseg1.ts\n";
        let rendered = rewrite_media(
            body,
            "http://up.example/live/a/index.m3u8",
            "g1",
            "c1",
            &formats(),
            false,
            "http://self",
        );
        assert!(rendered.body.contains("http://up.example/live/a/seg1.ts"));
        assert!(rendered.segments.is_empty());
    }

    #[test]
    fn segment_name_parsing() {
        let name = format!("{}_{}.jpg", prefix("g1", "c1"), suffix("http://x/1.ts"));
        let (parsed_suffix, ext) = parse_segment(&name).unwrap();
        assert_eq!(parsed_suffix, suffix("http://x/1.ts"));
        assert_eq!(ext, "jpg");
        assert!(prefix_matches(&prefix("g1", "c1"), &name));
        assert!(!prefix_matches(&prefix("g1", "c2"), &name));
        assert!(parse_segment("plain.ts").is_none());
    }

    #[test]
    fn extension_rotates_per_channel() {
        let body = "#EXTM3U\n#EXTINF:4.0,\n1.ts\n#EXTINF:4.0,\n2.ts\n";
        let rendered = rewrite_media(
            body,
            "http://up.example/a/index.m3u8",
            "g1",
            "c1",
            &formats(),
            true,
            "http://self",
        );
        let first = rendered.segments[0].2.clone();
        let second = rendered.segments[1].2.clone();
        assert_ne!(first, second, "同一频道内扩展名轮换");
    }

    #[test]
    fn known_upstream_extension_is_preserved() {
        let body = "#EXTM3U\n#EXTINF:4.0,\nseg1.jpg?token=x\n#EXTINF:4.0,\nseg2.jpg?token=y\n";
        let rendered = rewrite_media(
            body,
            "http://up.example/a/index.m3u8",
            "g1",
            "c1",
            &formats(),
            true,
            "http://self",
        );
        assert_eq!(rendered.segments[0].2, "jpg");
        assert_eq!(rendered.segments[1].2, "jpg");
        assert_eq!(
            rendered.segments[0].1,
            "http://up.example/a/seg1.jpg?token=x"
        );
    }
}
