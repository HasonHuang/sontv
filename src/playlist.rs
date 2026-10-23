//! 订阅（M3U）渲染。
//!
//! 契约见 [`docs/design/03-http-contract.md`](../docs/design/03-http-contract.md) §5。
//! 证据：F-004、F-009、S-URL-02/04/07。

use std::collections::HashSet;

use crate::{
    config::{
        model::{Channel, Group, PlaylistSource},
        Config,
    },
    subs::GroupRules,
};

/// 渲染失败的原因。
#[derive(Debug, PartialEq, Eq)]
pub enum PlaylistError {
    /// 分组已归档（`access = "archived"`）：分组订阅返回 404（F-009）。
    GroupArchived,
}

impl PlaylistError {
    /// 客户端可见的错误串。
    pub fn message(&self) -> &'static str {
        match self {
            PlaylistError::GroupArchived => "Group archived",
        }
    }
}

/// 渲染总订阅：只含 `access = "public"` 的分组（F-009）。
pub fn render_total(config: &Config, base: &str, token: Option<&str>) -> String {
    let mut body = header(config, base);
    for group in &config.groups {
        if group.access != "public" {
            continue;
        }
        for channel in &group.channels {
            push_entry(&mut body, group, channel, base, token);
        }
    }
    body
}

/// 渲染分组订阅。归档组报错（F-009）。
pub fn render_group(
    config: &Config,
    group: &Group,
    base: &str,
    token: Option<&str>,
) -> Result<String, PlaylistError> {
    if group.access == "archived" {
        return Err(PlaylistError::GroupArchived);
    }
    let mut body = header(config, base);
    for channel in &group.channels {
        push_entry(&mut body, group, channel, base, token);
    }
    Ok(body)
}

/// `#EXTM3U`、EPG 发现头与自定义头行（S-URL-07、S-EPG-01）。
fn header(config: &Config, base: &str) -> String {
    let mut body = if config.epg.enabled {
        format!(
            "#EXTM3U x-tvg-url=\"{}/epg.xml.gz\"\n",
            base.trim_end_matches('/')
        )
    } else {
        String::from("#EXTM3U\n")
    };
    for line in &config.server.m3u_header_lines {
        body.push_str(line);
        body.push('\n');
    }
    body
}

fn push_entry(
    body: &mut String,
    group: &Group,
    channel: &Channel,
    base: &str,
    token: Option<&str>,
) {
    let tvg_id = string_field(channel, "tvg_id").unwrap_or_else(|| channel.id.clone());
    let tvg_name = string_field(channel, "tvg_name").unwrap_or_else(|| channel.name.clone());
    let logo = string_field(channel, "logo")
        .or_else(|| string_field(channel, "tvg_logo"))
        .unwrap_or_default();
    let group_title = if group.name.is_empty() {
        group.id.clone()
    } else {
        group.name.clone()
    };

    body.push_str(&format!(
        "#EXTINF:-1 tvg-id=\"{}\" tvg-name=\"{}\" tvg-logo=\"{}\" group-title=\"{}\",{}\n",
        tvg_id, tvg_name, logo, group_title, channel.name
    ));

    // Passthrough 直接给上游原始地址，且不套 Token（S-URL-02、S-MODE-11）。
    if group.mode == "passthrough" {
        body.push_str(&channel.url);
        body.push('\n');
        return;
    }

    let ext = if group.format == "mpd" { "mpd" } else { "m3u8" };
    let url = format!(
        "{}/{}/{}.{}",
        base.trim_end_matches('/'),
        group.id,
        channel.id,
        ext
    );

    // `open` 组免 Token，条目不带参数（F-009）。
    match token {
        Some(value) if group.access != "open" => {
            body.push_str(&format!("{url}?token={value}\n"));
        }
        _ => {
            body.push_str(&url);
            body.push('\n');
        }
    }
}

/// 读取频道上的未建模字符串字段（`tvg_id`、`tvg_name`、`logo`）。
fn string_field(channel: &Channel, key: &str) -> Option<String> {
    channel
        .extra
        .get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::Config;

    fn config_with(groups: Vec<Group>) -> Config {
        Config {
            groups,
            ..Config::default()
        }
    }

    fn group(id: &str, access: &str, mode: &str) -> Group {
        Group {
            id: id.into(),
            name: id.into(),
            access: access.into(),
            mode: mode.into(),
            channels: vec![Channel {
                id: "c1".into(),
                name: "C1".into(),
                url: "http://upstream/x.m3u8".into(),
                ..Channel::default()
            }],
            ..Group::default()
        }
    }

    #[test]
    fn total_keeps_only_public() {
        let config = config_with(vec![
            group("g1", "public", "proxy"),
            group("g2", "hidden", "proxy"),
            group("g3", "open", "proxy"),
            group("g4", "archived", "proxy"),
        ]);
        let body = render_total(&config, "http://host", Some("tok"));
        assert!(body.contains("http://host/g1/c1.m3u8?token=tok"));
        assert!(!body.contains("/g2/"));
        assert!(!body.contains("/g3/"), "open 组不进总订阅");
        assert!(!body.contains("/g4/"), "archived 组不进总订阅");
    }

    #[test]
    fn open_group_entry_has_no_token() {
        let config = config_with(vec![group("g3", "open", "proxy")]);
        let body = render_group(&config, &config.groups[0], "http://host", Some("tok")).unwrap();
        assert!(body.contains("http://host/g3/c1.m3u8\n"));
        assert!(!body.contains("token="));
    }

    #[test]
    fn archived_group_is_rejected() {
        let config = config_with(vec![group("g4", "archived", "proxy")]);
        let error = render_group(&config, &config.groups[0], "http://host", None).unwrap_err();
        assert_eq!(error, PlaylistError::GroupArchived);
    }

    #[test]
    fn passthrough_uses_upstream_url() {
        let config = config_with(vec![group("g5", "public", "passthrough")]);
        let body = render_group(&config, &config.groups[0], "http://host", Some("tok")).unwrap();
        assert!(body.contains("http://upstream/x.m3u8"));
        assert!(!body.contains("/g5/c1"));
    }

    #[test]
    fn epg_header_is_emitted_when_enabled() {
        let mut config = config_with(vec![group("g1", "public", "proxy")]);
        config.epg.enabled = true;
        let body = render_total(&config, "http://host/", None);
        let first = body.lines().next().unwrap();
        assert_eq!(first, "#EXTM3U x-tvg-url=\"http://host/epg.xml.gz\"");
    }

    #[test]
    fn epg_header_is_absent_when_disabled() {
        let config = config_with(vec![group("g1", "public", "proxy")]);
        let body = render_total(&config, "http://host", None);
        let first = body.lines().next().unwrap();
        assert_eq!(first, "#EXTM3U");
    }

    #[test]
    fn header_lines_are_emitted() {
        let mut config = config_with(vec![group("g1", "public", "proxy")]);
        config.server.m3u_header_lines = vec!["#EXTVLCOPT:http-user-agent=Rea/1.0".into()];
        let body = render_total(&config, "http://host", None);
        let first = body.lines().nth(1).unwrap();
        assert_eq!(first, "#EXTVLCOPT:http-user-agent=Rea/1.0");
    }

    #[test]
    fn mpd_format_uses_mpd_ext() {
        let mut config = config_with(vec![group("g6", "public", "dash")]);
        config.groups[0].format = "mpd".into();
        let body = render_total(&config, "http://host", None);
        assert!(body.contains("http://host/g6/c1.mpd"));
    }
}

/// 聚合源过滤 + 提取条目（F-013 §5）。
///
/// 保留 `#EXTINF` 及其附属 `#` 行和原始 URL；不改写地址，也不加 Token。
/// 过滤词：`group:<文本>` 匹配 `group-title`，`id:<文本>` 匹配 `tvg-id`，
/// 其余按频道名匹配。都不区分大小写，空列表表示不过滤。
pub fn aggregate(source: &PlaylistSource, global_exclude: &[String], body: &str) -> String {
    let mut seen = HashSet::new();
    aggregate_seen(source, global_exclude, body, &mut seen).0
}

/// 聚合单个源，并把已并入的 URL 记入 `seen`。
///
/// 目标按 URL 去重，范围覆盖同一源和多个源（F-020）。返回正文与实际并入条数。
pub fn aggregate_seen(
    source: &PlaylistSource,
    global_exclude: &[String],
    body: &str,
    seen: &mut HashSet<String>,
) -> (String, usize) {
    let rules = GroupRules {
        group_map: &source.group_map,
        group_mode: &source.group_mode,
        merge_group: &source.merge_group,
        unlisted_groups: &source.unlisted_groups,
    };
    let mut out = String::new();
    let mut added = 0usize;
    let mut pending: Vec<&str> = Vec::new();
    for line in body.lines() {
        let line = line.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            if !trimmed.starts_with("#EXTM3U") {
                pending.push(line);
            }
            continue;
        }
        let entry = pending.clone();
        pending.clear();
        let meta = entry
            .iter()
            .find(|line| line.starts_with("#EXTINF"))
            .map(|line| entry_meta(line))
            .unwrap_or_default();
        let original = if meta.1.trim().is_empty() {
            "(无分组)".to_string()
        } else {
            meta.1.trim().to_string()
        };
        let Some(group) = rules.target(&original) else {
            continue;
        };
        if (!source.include.is_empty() && !matches(&source.include, &meta, &original, &group))
            || matches(&source.exclude, &meta, &original, &group)
            || matches(global_exclude, &meta, &original, &group)
        {
            continue;
        }
        if !seen.insert(trimmed.to_string()) {
            continue;
        }
        for line in &entry {
            if line.starts_with("#EXTINF") && group != original {
                out.push_str(&set_group_title(line, &group));
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
        out.push_str(trimmed);
        out.push('\n');
        added += 1;
    }
    (out, added)
}

/// 改写 `#EXTINF` 的 `group-title`；缺属性时插入到名称逗号之前。
fn set_group_title(line: &str, group: &str) -> String {
    if let Some(start) = line.find("group-title=\"") {
        let value_start = start + "group-title=\"".len();
        if let Some(end) = line[value_start..].find('"') {
            let mut out = String::with_capacity(line.len() + group.len());
            out.push_str(&line[..value_start]);
            out.push_str(group);
            out.push_str(&line[value_start + end..]);
            return out;
        }
    }
    match line.rfind(',') {
        Some(index) => format!(
            "{} group-title=\"{}\"{}",
            &line[..index],
            group,
            &line[index..]
        ),
        None => format!("{line} group-title=\"{group}\""),
    }
}

/// 把聚合条目按位置拼到本地订阅上。
pub fn compose(local: String, extra: String, position: &str) -> String {
    if extra.is_empty() {
        return local;
    }
    let (head, rest) = match local.split_once('\n') {
        Some((first, rest)) => (format!("{first}\n"), rest.to_string()),
        None => (String::new(), local),
    };
    if position == "top" {
        format!("{head}{extra}{rest}")
    } else {
        format!("{head}{rest}{extra}")
    }
}

/// `(频道名, group-title, tvg-id)`。
fn entry_meta(line: &str) -> (String, String, String) {
    let name = line
        .rsplit_once(',')
        .map(|(_, name)| name.trim().to_string())
        .unwrap_or_default();
    (
        name,
        attr(line, "group-title").unwrap_or_default(),
        attr(line, "tvg-id").unwrap_or_default(),
    )
}

/// 取 `key="value"` 属性。
fn attr(line: &str, key: &str) -> Option<String> {
    let start = line.find(&format!("{key}=\""))? + key.len() + 2;
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// 是否有任一过滤词命中。`group:` 同时匹配原分组与映射后的分组。
fn matches(
    patterns: &[String],
    meta: &(String, String, String),
    original: &str,
    group: &str,
) -> bool {
    let (name, _, id) = meta;
    patterns
        .iter()
        .map(|pattern| pattern.trim())
        .filter(|pattern| !pattern.is_empty())
        .any(|pattern| {
            if let Some(want) = pattern.strip_prefix("group:") {
                let want = want.trim().to_lowercase();
                original.to_lowercase().contains(&want) || group.to_lowercase().contains(&want)
            } else if let Some(want) = pattern.strip_prefix("id:") {
                id.to_lowercase() == want.trim().to_lowercase()
            } else {
                name.to_lowercase().contains(&pattern.to_lowercase())
            }
        })
}

#[cfg(test)]
mod aggregate_tests {
    use super::*;

    const BODY: &str = "#EXTM3U\n#EXTINF:-1 tvg-id=\"a\" group-title=\"新闻\",CCTV-1\nhttp://a/1.m3u8\n#EXTINF:-1 tvg-id=\"b\" group-title=\"体育\",CCTV-5\nhttp://a/5.m3u8\n";

    fn source(include: &[&str], exclude: &[&str]) -> PlaylistSource {
        PlaylistSource {
            unlisted_groups: "keep".into(),
            include: include.iter().map(|s| s.to_string()).collect(),
            exclude: exclude.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn keeps_all_when_no_filter() {
        let out = aggregate(&source(&[], &[]), &[], BODY);
        assert!(out.contains("CCTV-1") && out.contains("CCTV-5"));
        assert!(out.starts_with("#EXTINF"));
        assert!(!out.contains("#EXTM3U"));
    }

    #[test]
    fn include_by_name_group_and_id() {
        assert!(!aggregate(&source(&["cctv-1"], &[]), &[], BODY).contains("CCTV-5"));
        assert!(aggregate(&source(&["group:体育"], &[]), &[], BODY).contains("CCTV-5"));
        assert!(!aggregate(&source(&["id:b"], &[]), &[], BODY).contains("CCTV-1"));
    }

    #[test]
    fn exclude_wins_and_global_applies() {
        let out = aggregate(&source(&[], &["cctv-5"]), &[], BODY);
        assert!(out.contains("CCTV-1") && !out.contains("CCTV-5"));
        let out = aggregate(&source(&[], &[]), &["group:体育".to_string()], BODY);
        assert!(!out.contains("CCTV-5"));
    }

    #[test]
    fn group_map_selects_and_rewrites_groups() {
        let mut source = source(&[], &[]);
        source.unlisted_groups = "drop".into();
        source.group_mode = "map".into();
        source.group_map.insert("新闻".into(), "映射新闻".into());
        let out = aggregate(&source, &[], BODY);
        assert!(out.contains("group-title=\"映射新闻\""), "{out}");
        assert!(!out.contains("CCTV-5"), "{out}");
    }

    #[test]
    fn merge_mode_renames_selected_and_unlisted_groups() {
        let mut source = source(&[], &[]);
        source.group_mode = "merge".into();
        source.merge_group = "合并".into();
        source.group_map.insert("新闻".into(), String::new());
        let out = aggregate(&source, &[], BODY);
        assert_eq!(out.matches("group-title=\"合并\"").count(), 2, "{out}");
    }

    #[test]
    fn duplicate_urls_are_dropped() {
        let body = "#EXTM3U\n#EXTINF:-1 tvg-id=\"a\" group-title=\"新闻\",A\nhttp://a/1.m3u8\n#EXTINF:-1 tvg-id=\"b\" group-title=\"新闻\",B\nhttp://a/1.m3u8\n#EXTINF:-1 tvg-id=\"c\" group-title=\"新闻\",C\nhttp://a/2.m3u8\n";
        let out = aggregate(&source(&[], &[]), &[], body);
        assert_eq!(out.matches("http://a/1.m3u8").count(), 1, "{out}");
        assert!(out.contains("http://a/2.m3u8"), "{out}");
    }

    #[test]
    fn empty_map_drops_everything_by_default() {
        let source = PlaylistSource::default();
        assert!(aggregate(&source, &[], BODY).is_empty());
    }

    #[test]
    fn compose_places_extra() {
        let local = "#EXTM3U\n#EXTINF:-1,本地\nhttp://self/1.m3u8\n".to_string();
        let extra = "#EXTINF:-1,远端\nhttp://up/1.m3u8\n".to_string();
        let top = compose(local.clone(), extra.clone(), "top");
        assert!(top.starts_with("#EXTM3U\n#EXTINF:-1,远端"));
        let bottom = compose(local, extra, "bottom");
        assert!(bottom.ends_with("#EXTINF:-1,远端\nhttp://up/1.m3u8\n"));
        assert_eq!(
            compose("#EXTM3U\n".into(), String::new(), "top"),
            "#EXTM3U\n"
        );
    }
}
