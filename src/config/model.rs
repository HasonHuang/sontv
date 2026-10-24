//! 配置模型。
//!
//! 字段与目标 v0.8.2 的 `config.toml` 对齐（`docs/spec/08-target-spec.md` §11）。
//! 未建模的段与键落在 `extra`，保存时原样写回。这样不丢第三方配置。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 顶层配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerCfg,
    pub panel: PanelCfg,
    pub cache: CacheCfg,
    pub auth: AuthCfg,
    pub epg: EpgCfg,
    pub playlist: PlaylistCfg,
    pub share: ShareCfg,
    pub health: HealthCfg,
    pub stats: StatsCfg,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
    /// 未建模的段与键。
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Config {
    /// 按 id 找分组。
    pub fn group(&self, gid: &str) -> Option<&Group> {
        self.groups.iter().find(|group| group.id == gid)
    }

    /// 按 id 找可变分组。
    pub fn group_mut(&mut self, gid: &str) -> Option<&mut Group> {
        self.groups.iter_mut().find(|group| group.id == gid)
    }

    /// `[assets].map`：原始 URL → 本地 HLS 入口。
    pub fn asset_map(&self) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        let Some(table) = self
            .extra
            .get("assets")
            .and_then(toml::Value::as_table)
            .and_then(|assets| assets.get("map"))
            .and_then(toml::Value::as_table)
        else {
            return out;
        };
        for (url, local) in table {
            if let Some(local) = local.as_str() {
                out.insert(url.clone(), local.to_string());
            }
        }
        out
    }

    /// 查本地 HLS 入口。
    pub fn asset_local(&self, url: &str) -> Option<&str> {
        self.extra
            .get("assets")
            .and_then(toml::Value::as_table)
            .and_then(|assets| assets.get("map"))
            .and_then(toml::Value::as_table)
            .and_then(|map| map.get(url))
            .and_then(toml::Value::as_str)
    }

    /// 频道的有效上游代理：频道级优先，其次分组级。
    pub fn proxy_for(&self, gid: &str, cid: &str) -> Option<&ProxyCfg> {
        let group = self.group(gid)?;
        group
            .channels
            .iter()
            .find(|channel| channel.id == cid)
            .and_then(|channel| channel.proxy.as_ref())
            .or(group.proxy.as_ref())
    }
}

/// `[server]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerCfg {
    pub bind: String,
    pub not_found_url: String,
    pub error_url: String,
    pub home_url: String,
    pub m3u_header_lines: Vec<String>,
    pub timezone_offset_hours: i32,
    pub catchup_tpl: String,
    pub log_level: String,
    pub trust_proxy: Vec<String>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for ServerCfg {
    fn default() -> Self {
        Self {
            bind: "[::]:19899".into(),
            not_found_url: "https://material.yang-1989.eu.org/v/404.mp4".into(),
            error_url: "https://material.yang-1989.eu.org/v/mistake.mp4".into(),
            home_url: "https://yang-1989.eu.org/".into(),
            m3u_header_lines: Vec::new(),
            timezone_offset_hours: 8,
            catchup_tpl: "guonei".into(),
            log_level: "info".into(),
            trust_proxy: Vec::new(),
            extra: toml::Table::new(),
        }
    }
}

/// `[panel]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelCfg {
    pub enabled: bool,
    pub username: String,
    /// 口令的 SHA-256 十六进制。默认口令 `admin`。
    pub password_sha256: String,
    pub session_ttl: u64,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for PanelCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            username: "admin".into(),
            // sha256("admin")
            password_sha256: "8c6976e5b5410415bde908bd4dee15dfb167a9c873fc4bb8a81f6f2ab448a918"
                .into(),
            session_ttl: 86400,
            extra: toml::Table::new(),
        }
    }
}

/// `[cache]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheCfg {
    pub backend: String,
    pub disk_path: String,
    pub m3u8_max_entries: u64,
    pub segment_max_entries: u64,
    pub disk_max_mb: u64,
    pub memory_max_mb: u64,
    pub segment_cache_max_mb: u64,
    pub prefetch: CachePrefetchCfg,
    pub segments: CacheSegmentsCfg,
    pub ttl: CacheTtlCfg,
    pub dash: CacheDashCfg,
    pub remux: CacheRemuxCfg,
    pub disguise: DisguiseCfg,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for CacheCfg {
    fn default() -> Self {
        Self {
            backend: "disk".into(),
            disk_path: "./cache".into(),
            m3u8_max_entries: 500,
            segment_max_entries: 2000,
            disk_max_mb: 0,
            memory_max_mb: 0,
            segment_cache_max_mb: 50,
            prefetch: CachePrefetchCfg::default(),
            segments: CacheSegmentsCfg::default(),
            ttl: CacheTtlCfg::default(),
            dash: CacheDashCfg::default(),
            remux: CacheRemuxCfg::default(),
            disguise: DisguiseCfg::default(),
            extra: toml::Table::new(),
        }
    }
}

/// `[cache.prefetch]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CachePrefetchCfg {
    pub enabled: bool,
    pub concurrency: u32,
    pub timeout: u64,
    pub prefetch_limit: u64,
    pub skip_tail: u64,
    pub mpd_readahead: u64,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for CachePrefetchCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            concurrency: 4,
            timeout: 10,
            prefetch_limit: 0,
            skip_tail: 1,
            mpd_readahead: 1,
            extra: toml::Table::new(),
        }
    }
}

/// `[cache.segments]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheSegmentsCfg {
    pub global_max: u64,
    pub per_channel_max: u64,
    pub retain_count: u64,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for CacheSegmentsCfg {
    fn default() -> Self {
        Self {
            global_max: 0,
            per_channel_max: 0,
            retain_count: 20,
            extra: toml::Table::new(),
        }
    }
}

/// `[cache.ttl]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheTtlCfg {
    pub sub_url_secs: u64,
    pub segment_secs: u64,
    pub refresh_ratio: f64,
    pub fallback_mpd_secs: u64,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for CacheTtlCfg {
    fn default() -> Self {
        Self {
            sub_url_secs: 1_800,
            segment_secs: 300,
            refresh_ratio: 0.4,
            fallback_mpd_secs: 8,
            extra: toml::Table::new(),
        }
    }
}

/// `[cache.dash]`——目标 DASH 会话调参。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheDashCfg {
    pub mpd_refresh_min_secs: f64,
    pub mpd_refresh_max_secs: f64,
    pub mpd_timeout_secs: u64,
    pub segment_timeout_secs: u64,
    pub fetch_concurrency: u32,
    pub ondemand_idle_secs: u64,
    pub mpd_body_dedup: bool,
    pub whole_file_serve: String,
    pub whole_file_budget_mb: u64,
    pub obfuscate_filenames: bool,
    pub playlist_min_secs: f64,
    pub retain_min_secs: f64,
    pub fetch_extra_segments: u64,
    /// 快速起播窗口。目标 v0.8.2 无可观测语义；保留字段以兼容配置回写。
    pub fast_start_secs: f64,
    pub start_time_offset_secs: f64,
    /// 直播清单发布延迟；>0 时按切片时长向上取整压后末端。
    pub publish_delay_secs: f64,
    pub continuous_tail_min_ratio: f64,
    pub retry_max: u32,
    pub retry_initial_ms: u64,
    pub retry_backoff: f64,
    pub retry_max_ms: u64,
    pub min_request_interval_ms: u64,
    pub playlist_stall_secs: u64,
    pub watchdog_tick_secs: u64,
    pub watchdog_stall_secs: u64,
    pub watchdog_rebuild_max: u32,
    pub watchdog_cooldown_secs: u64,
    pub watchdog_rebuild_stagger_ms: u64,
    pub session_max_lifetime_secs: u64,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for CacheDashCfg {
    fn default() -> Self {
        Self {
            mpd_refresh_min_secs: 4.0,
            mpd_refresh_max_secs: 12.0,
            mpd_timeout_secs: 10,
            segment_timeout_secs: 8,
            fetch_concurrency: 20,
            ondemand_idle_secs: 60,
            mpd_body_dedup: true,
            whole_file_serve: "auto".into(),
            whole_file_budget_mb: 0,
            obfuscate_filenames: true,
            playlist_min_secs: 30.0,
            retain_min_secs: 90.0,
            fetch_extra_segments: 2,
            fast_start_secs: 12.0,
            start_time_offset_secs: 15.0,
            publish_delay_secs: 0.0,
            continuous_tail_min_ratio: 0.6,
            retry_max: 8,
            retry_initial_ms: 200,
            retry_backoff: 2.0,
            retry_max_ms: 2_000,
            min_request_interval_ms: 80,
            playlist_stall_secs: 0,
            watchdog_tick_secs: 15,
            watchdog_stall_secs: 45,
            watchdog_rebuild_max: 5,
            watchdog_cooldown_secs: 3_600,
            watchdog_rebuild_stagger_ms: 300,
            session_max_lifetime_secs: 0,
            extra: toml::Table::new(),
        }
    }
}

/// `[cache.remux]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheRemuxCfg {
    pub target_duration_secs: f64,
    pub window_segments: u64,
    pub publish_delay_segments: u64,
    pub reconnect_backoff_ms: Vec<u64>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for CacheRemuxCfg {
    fn default() -> Self {
        Self {
            target_duration_secs: 5.0,
            window_segments: 6,
            publish_delay_segments: 2,
            reconnect_backoff_ms: vec![200, 1_000, 2_000],
            extra: toml::Table::new(),
        }
    }
}

/// `[cache.disguise]`——切片伪装（S-DIS-*）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisguiseCfg {
    pub enabled: bool,
    pub formats: Vec<String>,
    pub subtitle: bool,
    pub playlist: bool,
    pub master: bool,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for DisguiseCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            formats: ["jpg", "png", "gif", "bmp", "webp"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            subtitle: false,
            playlist: true,
            master: false,
            extra: toml::Table::new(),
        }
    }
}

/// `[auth]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthCfg {
    pub token_enabled: bool,
    pub token_ttl_hours: u64,
    pub token_failure_url: String,
    pub random_id_enabled: bool,
    pub ip_guard_enabled: bool,
    pub max_sessions_per_ip: u32,
    pub session_ttl_secs: u64,
    pub limit_url: String,
    pub ban_url: String,
    pub whitelist: Vec<String>,
    /// 登录失败统计窗口（秒）。
    pub auth_fail_window_secs: u64,
    /// 窗口内免费失败次数。
    pub auth_fail_free_per_window: u64,
    /// 超出免费次数后，再累计多少次触发锁定。
    pub auth_fail_ban_threshold: u64,
    /// 锁定秒数。
    pub auth_fail_lock_secs: u64,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for AuthCfg {
    fn default() -> Self {
        Self {
            token_enabled: false,
            token_ttl_hours: 24,
            token_failure_url: "https://material.yang-1989.eu.org/v/Failure.mp4".into(),
            random_id_enabled: false,
            ip_guard_enabled: false,
            max_sessions_per_ip: 1,
            session_ttl_secs: 30,
            limit_url: "https://material.yang-1989.eu.org/v/limit.mp4".into(),
            ban_url: "https://material.yang-1989.eu.org/v/Ban.mp4".into(),
            whitelist: Vec::new(),
            auth_fail_window_secs: 60,
            auth_fail_free_per_window: 15,
            auth_fail_ban_threshold: 6,
            auth_fail_lock_secs: 300,
            extra: toml::Table::new(),
        }
    }
}

/// `[health]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthCfg {
    pub enabled: bool,
    pub interval_secs: u64,
    pub timeout_secs: u64,
    pub concurrency: u32,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for HealthCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 300,
            timeout_secs: 10,
            concurrency: 3,
            extra: toml::Table::new(),
        }
    }
}

/// `[stats]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StatsCfg {
    pub enabled: bool,
    pub online_window_secs: u64,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for StatsCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            online_window_secs: 90,
            extra: toml::Table::new(),
        }
    }
}

/// 上游代理覆盖（分组或频道级）。
///
/// 证据：S-GAP-05。目标字段为 `proxy{kind,addr,user,pass}`；行为未验。
/// 本实现只用 `http`/`https`；其它 `kind` 忽略并记日志。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProxyCfg {
    pub kind: String,
    pub addr: String,
    pub user: String,
    pub pass: String,
}

impl Default for ProxyCfg {
    fn default() -> Self {
        Self {
            kind: "http".into(),
            addr: String::new(),
            user: String::new(),
            pass: String::new(),
        }
    }
}

/// `[[groups]]`——工作模式属于分组（S-MODE-02）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Group {
    pub id: String,
    pub name: String,
    /// `proxy` / `remux` / `dash` / `redirect` / `rewrite` / `passthrough`。
    pub mode: String,
    /// `public` / `hidden` / `archived` / `open`。
    pub access: String,
    /// `hls` / `mpd`。
    pub format: String,
    pub remux_container: String,
    pub mpd_keep_seconds: u64,
    pub redirect_ttl: u64,
    pub redirect_mode: String,
    pub headers: Vec<toml::Value>,
    /// 分组级上游代理；频道级优先。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyCfg>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<Channel>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for Group {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            mode: "proxy".into(),
            access: "public".into(),
            format: "hls".into(),
            remux_container: "auto".into(),
            mpd_keep_seconds: 0,
            redirect_ttl: 0,
            redirect_mode: "direct".into(),
            headers: Vec::new(),
            proxy: None,
            channels: Vec::new(),
            extra: toml::Table::new(),
        }
    }
}

/// `[[groups.channels]]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub url: String,
    pub headers: Vec<toml::Value>,
    /// 频道级上游代理；覆盖分组级。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyCfg>,
    pub selected_tracks: Vec<String>,
    pub dash_mode: String,
    pub livestream: bool,
    pub timeshift_hours: u32,
    pub timeshift_max_gb: u64,
    pub timeshift_offset_secs: u64,
    pub dash_timeshift: bool,
    pub dash_timeshift_hours: u32,
    pub dash_timeshift_max_gb: u64,
    pub dash_timeshift_offset_secs: u64,
    pub resolver_catchup: bool,
    pub catchup_days: u32,
    pub url_pool_size: u32,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            url: String::new(),
            headers: Vec::new(),
            proxy: None,
            selected_tracks: Vec::new(),
            dash_mode: "continuous".into(),
            livestream: false,
            timeshift_hours: 4,
            timeshift_max_gb: 0,
            timeshift_offset_secs: 60,
            dash_timeshift: false,
            dash_timeshift_hours: 4,
            dash_timeshift_max_gb: 0,
            dash_timeshift_offset_secs: 60,
            resolver_catchup: false,
            catchup_days: 7,
            url_pool_size: 1,
            extra: toml::Table::new(),
        }
    }
}

/// `[epg]`——节目单（S-EP-05、F-008）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EpgCfg {
    pub enabled: bool,
    pub cron: String,
    pub public_viewer_enabled: bool,
    pub keep_days: i64,
    pub future_days: i64,
    pub keep_yesterday: bool,
    pub match_by_name: bool,
    pub allow_scripts: bool,
    pub python_bin: Option<String>,
    pub script_timeout_secs: u64,
    pub record_pad_before_secs: u64,
    pub record_pad_after_secs: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<EpgSource>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for EpgCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            cron: "0 */4 * * *".into(),
            public_viewer_enabled: true,
            keep_days: 3,
            future_days: 1,
            keep_yesterday: true,
            match_by_name: true,
            allow_scripts: true,
            python_bin: None,
            script_timeout_secs: 300,
            record_pad_before_secs: 60,
            record_pad_after_secs: 180,
            sources: Vec::new(),
            extra: toml::Table::new(),
        }
    }
}

/// `[[epg.sources]]`——采集源。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EpgSource {
    pub id: String,
    pub name: String,
    /// `xml`（按 `url` 拉 XMLTV）或 `script`（跑 Python，需外部依赖）。
    pub kind: String,
    pub enabled: bool,
    pub url: String,
    pub script: String,
    pub output_files: Vec<String>,
    pub cron: String,
    pub timeout_secs: u64,
    pub tz_shift_mins: i64,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for EpgSource {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "新源".into(),
            kind: "xml".into(),
            enabled: true,
            url: String::new(),
            script: String::new(),
            output_files: Vec::new(),
            cron: String::new(),
            timeout_secs: 0,
            tz_shift_mins: 0,
            extra: toml::Table::new(),
        }
    }
}

/// `[playlist]`——订阅聚合（F-013 §5）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaylistCfg {
    /// 聚合源缓存时长（秒）。
    pub merge_ttl_secs: u64,
    /// `top` 或 `bottom`。
    pub merge_position: String,
    pub global_exclude: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<PlaylistSource>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for PlaylistCfg {
    fn default() -> Self {
        Self {
            merge_ttl_secs: 300,
            merge_position: "bottom".into(),
            global_exclude: Vec::new(),
            sources: Vec::new(),
            extra: toml::Table::new(),
        }
    }
}

/// `[[playlist.sources]]`——聚合源。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaylistSource {
    pub id: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    /// 源正文缓存时长（秒）；`0` 表示不按时间失效。
    pub ttl_secs: u64,
    /// 源分组名 → 目标分组名；空值表示保留原名。
    pub group_map: BTreeMap<String, String>,
    /// `keep`（默认）、`map` 或 `merge`；未知值按 `keep`。
    pub group_mode: String,
    /// `merge` 模式的目标分组名；空值表示保留原名。
    pub merge_group: String,
    /// 未出现在 `group_map` 的分组：`drop`（默认）或 `keep`。
    pub unlisted_groups: String,
    /// 频道名白名单；支持 `group:` 与 `id:` 前缀。
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for PlaylistSource {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            url: String::new(),
            enabled: true,
            ttl_secs: 0,
            group_map: BTreeMap::new(),
            group_mode: "keep".into(),
            merge_group: String::new(),
            unlisted_groups: "drop".into(),
            include: Vec::new(),
            exclude: Vec::new(),
            extra: toml::Table::new(),
        }
    }
}

/// `[share]`——多 VPS 分享（F-013 §10）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShareCfg {
    pub enabled: bool,
    /// `master` 或 `follower`。
    pub role: String,
    /// 分享密钥；对端用 `?key=` 传入。
    pub key: String,
    /// 允许拉取的源地址（精确或 `a.b.c.0/24`）。
    pub trust_ips: Vec<String>,
    /// follower 拉取 master 的基地址。
    pub master_url: String,
    /// master 汇总的 follower 基地址。
    pub follower_urls: Vec<String>,
    /// 同步周期（秒）。
    pub poll_secs: u64,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for ShareCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            role: "master".into(),
            key: String::new(),
            trust_ips: Vec::new(),
            master_url: String::new(),
            follower_urls: Vec::new(),
            poll_secs: 300,
            extra: toml::Table::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_proxy_overrides_group_proxy() {
        let mut config = Config::default();
        let mut group = Group {
            id: "g".into(),
            ..Group::default()
        };
        group.proxy = Some(ProxyCfg {
            addr: "http://group:8080".into(),
            ..ProxyCfg::default()
        });
        group.channels.push(Channel {
            id: "c".into(),
            proxy: Some(ProxyCfg {
                addr: "http://channel:8080".into(),
                ..ProxyCfg::default()
            }),
            ..Channel::default()
        });
        config.groups.push(group);

        assert_eq!(
            config.proxy_for("g", "c").expect("频道代理").addr,
            "http://channel:8080"
        );
        assert_eq!(
            config.proxy_for("g", "other").expect("分组代理").addr,
            "http://group:8080"
        );
        assert!(config.proxy_for("missing", "c").is_none());
    }

    #[test]
    fn cache_subsections_round_trip() {
        let text = r#"
[cache]
disk_path = "cache"
[cache.prefetch]
enabled = false
concurrency = 2
[cache.segments]
global_max = 10
per_channel_max = 3
[cache.ttl]
segment_secs = 42
[cache.dash]
mpd_timeout_secs = 7
[cache.remux]
window_segments = 8
publish_delay_segments = 2
reconnect_backoff_ms = [100, 500]
"#;
        let config: Config = toml::from_str(text).expect("解析配置");
        assert!(!config.cache.prefetch.enabled);
        assert_eq!(config.cache.prefetch.concurrency, 2);
        assert_eq!(config.cache.segments.global_max, 10);
        assert_eq!(config.cache.segments.per_channel_max, 3);
        assert_eq!(config.cache.ttl.segment_secs, 42);
        assert_eq!(config.cache.dash.mpd_timeout_secs, 7);
        assert_eq!(config.cache.remux.window_segments, 8);
        let encoded = toml::to_string(&config).expect("序列化");
        let decoded: Config = toml::from_str(&encoded).expect("再解析");
        assert_eq!(decoded.cache.remux.reconnect_backoff_ms, vec![100, 500]);
    }

    #[test]
    fn proxy_is_parsed_from_toml() {
        let text = r#"
[[groups]]
id = "g"
name = "组"
[groups.proxy]
kind = "http"
addr = "http://127.0.0.1:8080"
user = "u"
pass = "p"
[[groups.channels]]
id = "c"
name = "频道"
url = "http://up/live.m3u8"
"#;
        let config: Config = toml::from_str(text).expect("解析配置");
        let proxy = config.proxy_for("g", "c").expect("代理");
        assert_eq!(proxy.kind, "http");
        assert_eq!(proxy.user, "u");
    }
}
