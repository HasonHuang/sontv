//! 播放链路：入口清单 → 媒体清单 → 伪装切片（M2）与直连模式（M3）。
//!
//! 契约见 [`docs/design/03-http-contract.md`](../../../docs/design/03-http-contract.md) §4。

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    extract::{Path, Query, RawQuery, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use url::Url;

use crate::{
    config::{model::Group, Config},
    proxy,
    state::AppState,
};

/// proxy 模式 MPD 缓存头（目标实测 1 秒）。
const MPD_CACHE: &str = "public, max-age=1, s-maxage=1";
/// MPD 正文类型（目标带 UTF-8 字符集）。
const MPD_CONTENT_TYPE: &str = "application/dash+xml; charset=utf-8";
/// Rewrite 模式缓存头。
const REWRITE_CACHE: &str = "no-cache, must-revalidate";
/// 主清单入口缓存头：变体清单每次都要重取（S-PROXY-08）。
const MASTER_CACHE: &str = "no-cache, no-store";
/// 清单缺 `#EXT-X-TARGETDURATION` 时的缓存窗口（秒）。
const DEFAULT_TARGET_DURATION: u64 = 6;
/// 切片缓存头；`[cache.ttl].segment_secs` 控制时长。
fn segment_cache_control(state: &AppState, first: bool) -> String {
    let secs = state.snapshot().cache.ttl.segment_secs.max(1);
    if first {
        format!("public, max-age={secs}")
    } else {
        format!("private, max-age={secs}")
    }
}

/// `GET /{gid}/{cid}.m3u8` 与 `/{gid}/{cid}.mpd`。
pub async fn entry(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((gid, file)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let config = state.snapshot();
    let self_base = self_base(&headers);
    let Some((cid, ext)) = split_entry(&file) else {
        return placeholder(&config);
    };
    let Some(group) = config.group(&gid) else {
        return unknown_endpoint(&state, &config, &query);
    };
    // 归档组停播（F-009）。
    if group.access == "archived" {
        return placeholder(&config);
    }
    let Some(channel) = group.channels.iter().find(|channel| channel.id == cid) else {
        return unknown_endpoint(&state, &config, &query);
    };
    if !token_ok(&state, &config, group, &query) {
        return redirect_asset(&config, &config.auth.token_failure_url);
    }
    // 上游解析脚本：每次播放前算地址（S-RES-01）。
    let resolved = match crate::resolver::resolve(&state, &gid, &cid, channel).await {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(gid = %gid, cid = %cid, "解析脚本失败: {error:#}");
            return redirect_asset(&config, &config.server.error_url);
        }
    };
    let base = resolved.unwrap_or_else(|| channel.url.clone());
    // 回看：只把 `playseek` 原样追加到上游（S-EP-04），其余回看参数丢弃。
    let mut upstream = with_playseek(&base, &query);
    // 本地回看：点亮 `livestream` 的频道优先读缓冲；未命中落回实时（S-TS-01）。
    if channel.livestream || channel.dash_timeshift {
        if let Some((start, end)) =
            crate::timeshift::range(&query, config.server.timezone_offset_hours as i64)
        {
            if let Some(body) = state.timeshift.playlist(&state, &gid, &cid, start, end) {
                return crate::timeshift::vad_response(body);
            }
            upstream = base.clone();
        }
    }
    // Passthrough 原样下发上游地址（S-MODE-11）。
    if group.mode == "passthrough" {
        return redirect(&upstream);
    }

    match group.mode.as_str() {
        // 302 直连上游；缓存解析出的地址（S-MODE-08/13）。
        "redirect" => redirect_entry(&state, &config, group, &cid, &upstream).await,
        // 清单绝对化，客户端直连上游切片（S-MODE-09/14）。
        "rewrite" => rewrite_entry(&state, &config, group, &cid, &upstream).await,
        // 连续流转封装：纯 Rust 切片；未产出前 503（S-MODE-16/18）。
        "remux" => {
            let session = state.remux.ensure(&state, &gid, &cid, &upstream);
            match session.playlist() {
                Some(body) => remux_playlist(body),
                None => warming_up(),
            }
        }
        // 就绪门：30 秒内拿到上游主清单，否则 503（S-MODE-15）。
        "dash" => dash_entry(&state, group, &gid, &cid, &upstream, &self_base).await,
        // 代理：上游是主清单则自造单变体主清单；上游是媒体清单则直接改写。
        _ => proxy_entry(&state, &config, &gid, &cid, &upstream, &self_base, &ext).await,
    }
}

/// 回看参数：只保留 `playseek`，按上游已有查询串拼接（S-EP-04）。
fn with_playseek(url: &str, query: &HashMap<String, String>) -> String {
    let Some(value) = query
        .get("playseek")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return url.to_string();
    };
    let encoded: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}playseek={encoded}")
}

/// 缓存键里的地址标签：同一频道不同回看区间不互相覆盖。
fn url_tag(url: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(&Sha256::digest(url.as_bytes())[..4])
}

/// Redirect 模式：302 到上游；`解析最佳画质` 时先取主清单挑最高码率。
async fn redirect_entry(
    state: &Arc<AppState>,
    config: &Config,
    group: &Group,
    cid: &str,
    channel_url: &str,
) -> Response {
    let ttl = proxy::modes::effective_redirect_ttl(group.redirect_ttl);
    let key = format!("redirect:{}:{cid}:{}", group.id, url_tag(channel_url));
    if let Some(url) = state.modes.get(&key, ttl) {
        return redirect_asset(config, &url);
    }
    let resolved = if proxy::modes::wants_best_quality(&group.redirect_mode) {
        match fetch_playlist(state, channel_url, &group.id, cid).await {
            Ok(text) => proxy::modes::best_variant(&text, channel_url)
                .unwrap_or_else(|| channel_url.to_string()),
            // 失败不缓存（S-MODE-08）。
            Err(err) => {
                tracing::warn!(url = %channel_url, error = %err, "解析上游主清单失败");
                return redirect_asset(&config, &config.server.error_url);
            }
        }
    } else {
        channel_url.to_string()
    };
    state.modes.put(&key, resolved.clone());
    redirect_asset(config, &resolved)
}

/// Rewrite 模式：取一次清单，把相对地址改绝对，切片客户端直连（S-MODE-09）。
async fn rewrite_entry(
    state: &Arc<AppState>,
    config: &Config,
    group: &Group,
    cid: &str,
    channel_url: &str,
) -> Response {
    let key = format!("rw:{}:{cid}:{}", group.id, url_tag(channel_url));
    if let Some(body) = state.playlists.get(&key) {
        return rewrite_response(body);
    }
    let upstream = match fetch_playlist(state, channel_url, &group.id, cid).await {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!(url = %channel_url, error = %err, "抓取上游清单失败");
            return redirect_asset(config, &config.server.error_url);
        }
    };
    let rendered = proxy::rewrite_media(&upstream, channel_url, &group.id, cid, &[], false, "");
    state.playlists.put(&key, rendered.body.clone());
    rewrite_response(rendered.body)
}

/// DASH 模式：MPD→HLS。30 秒就绪门与看门狗沿用目标行为（S-MODE-15/17）。
async fn dash_entry(
    state: &Arc<AppState>,
    group: &Group,
    gid: &str,
    cid: &str,
    channel_url: &str,
    self_base: &str,
) -> Response {
    let key = format!("dash:{}:{cid}", group.id);
    let dash = state.snapshot().cache.dash.clone();
    let watchdog_max = dash.watchdog_rebuild_max.max(1);
    let watchdog_cooldown = std::time::Duration::from_secs(dash.watchdog_cooldown_secs);
    if state.watchdog.is_cooling(&key) {
        return not_ready();
    }
    match state
        .dash_hls
        .master(state, gid, cid, channel_url, self_base)
        .await
    {
        Ok(body) => {
            state.watchdog.record_success(&key);
            let mut response = Response::new(Body::from(body));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/vnd.apple.mpegurl"),
            );
            insert_header(&mut response, header::CACHE_CONTROL, MASTER_CACHE);
            response
        }
        Err(error) => {
            tracing::warn!(url = %channel_url, error = %error, "DASH 主清单失败");
            state.stats.record_upstream_failure();
            if state
                .watchdog
                .record_failure_with(&key, watchdog_max, watchdog_cooldown)
            {
                tracing::warn!(
                    group = %group.id,
                    %cid,
                    max_failures = watchdog_max,
                    cooldown_secs = watchdog_cooldown.as_secs(),
                    "DASH 连续失败，进入冷却"
                );
            }
            if error.starts_with("MPD 解析失败") {
                (StatusCode::BAD_GATEWAY, "Upstream Error").into_response()
            } else {
                not_ready()
            }
        }
    }
}

/// Rewrite 响应：`text/plain` + `no-cache, must-revalidate`（S-MODE-09）。
fn rewrite_response(body: String) -> Response {
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    insert_header(&mut response, header::CACHE_CONTROL, REWRITE_CACHE);
    response
}

/// DASH 未就绪响应（S-MODE-15）。
fn not_ready() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "timeout waiting for master playlist",
    )
        .into_response()
}

/// Remux 清单响应：`text/plain`，禁止缓存。
fn remux_playlist(body: String) -> Response {
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    insert_header(
        &mut response,
        header::CACHE_CONTROL,
        "no-cache, must-revalidate",
    );
    response
}

/// Remux 预热响应（S-MODE-16）。
fn warming_up() -> Response {
    let mut response = (StatusCode::SERVICE_UNAVAILABLE, "remux warming up").into_response();
    insert_header(&mut response, header::RETRY_AFTER, "2");
    insert_header(&mut response, header::CACHE_CONTROL, "no-store");
    response
}

/// `GET /{gid}/{cid}/*path`：`video/index.txt`、`index.m3u8` 或伪装切片。
pub async fn wildcard(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((gid, cid, rest)): Path<(String, String, String)>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let rest = rest.trim_start_matches('/').to_string();
    if let Some(name) = rest.strip_prefix("timeshift/") {
        return state
            .timeshift
            .serve(&gid, &cid, name)
            .unwrap_or_else(|| (StatusCode::NOT_FOUND, "Segment not found").into_response());
    }
    if let Some(response) = state.remux.serve(&gid, &cid, &rest) {
        return response;
    }
    {
        let config = state.snapshot();
        if let Some((group, _)) = resolve(&config, &gid, &cid) {
            if group.mode == "dash" {
                if let Some(response) = state
                    .dash_hls
                    .serve(&state, &gid, &cid, &rest, &self_base(&headers))
                    .await
                {
                    return response;
                }
            }
        }
    }
    if let Some(path) = rest.strip_prefix("mpd/") {
        return mpd_segment(&state, &gid, &cid, path, raw_query.as_deref()).await;
    }
    match rest.as_str() {
        // 变体媒体清单：只有上游是主清单时存在（S-PROXY-07）。
        "video/index.txt" => variant_playlist(&state, &gid, &cid, &self_base(&headers)).await,
        // 兼容别名：与入口同义。
        "index.m3u8" => {
            let config = state.snapshot();
            let Some((group, url)) = resolve(&config, &gid, &cid) else {
                return placeholder(&config);
            };
            if group.mode == "remux" {
                let session = state.remux.ensure(&state, &gid, &cid, &url);
                match session.playlist() {
                    Some(body) => remux_playlist(body),
                    None => warming_up(),
                }
            } else if group.mode == "dash" {
                dash_entry(&state, &group, &gid, &cid, &url, &self_base(&headers)).await
            } else if group.mode == "proxy" {
                proxy_entry(
                    &state,
                    &config,
                    &gid,
                    &cid,
                    &url,
                    &self_base(&headers),
                    "m3u8",
                )
                .await
            } else {
                segment(&state, &gid, &cid, &rest).await
            }
        }
        _ => segment(&state, &gid, &cid, &rest).await,
    }
}

/// 代理模式入口。
///
/// - 上游是主清单 → 自造单变体主清单（S-PROXY-06）。
/// - 上游是媒体清单 → 直接改写为伪装切片地址（S-PROXY-01）。
async fn proxy_entry(
    state: &Arc<AppState>,
    config: &Config,
    gid: &str,
    cid: &str,
    channel_url: &str,
    self_base: &str,
    ext: &str,
) -> Response {
    let upstream = match upstream_playlist(state, channel_url, gid, cid).await {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!(url = %channel_url, error = %err, "抓取上游清单失败");
            state.stats.record_upstream_failure();
            return redirect_asset(config, &config.server.error_url);
        }
    };
    if proxy::modes::looks_like_mpd(&upstream) {
        let keep = config
            .group(gid)
            .map(|group| group.mpd_keep_seconds)
            .unwrap_or(0);
        return proxy_mpd(&upstream, gid, cid, channel_url, self_base, keep);
    }
    if proxy::media::is_master(&upstream) {
        return master_entry(&upstream, channel_url, gid, cid, self_base, ext);
    }
    media_entry(state, config, gid, cid, channel_url, &upstream, self_base)
}

/// 自造主清单：保留最佳变体的属性，只指向本服务的变体媒体清单。
fn master_entry(
    upstream: &str,
    channel_url: &str,
    gid: &str,
    cid: &str,
    self_base: &str,
    ext: &str,
) -> Response {
    let attrs = proxy::media::best_variant(upstream, channel_url)
        .map(|(attrs, _)| proxy::media::normalize_variant_attrs(&attrs))
        .unwrap_or_else(|| "BANDWIDTH=1000000".to_string());
    let body = format!(
        "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-STREAM-INF:{attrs}\n{self_base}/{gid}/{cid}/video/index.txt\n"
    );
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(match ext {
            "mpd" => "application/dash+xml",
            _ => "application/vnd.apple.mpegurl",
        }),
    );
    insert_header(&mut response, header::CACHE_CONTROL, MASTER_CACHE);
    response
}

/// 改写媒体清单并登记切片。
fn media_entry(
    state: &Arc<AppState>,
    config: &Config,
    gid: &str,
    cid: &str,
    channel_url: &str,
    upstream: &str,
    self_base: &str,
) -> Response {
    let disguise = config.cache.disguise.enabled && config.cache.disguise.playlist;
    let rendered = proxy::rewrite_media(
        upstream,
        channel_url,
        gid,
        cid,
        &config.cache.disguise.formats,
        disguise,
        self_base,
    );
    state.segments.register(gid, cid, &rendered.segments);
    proxy::save_index(&state.cache_dir, gid, cid, &rendered.segments);
    spawn_prefetch(
        state.clone(),
        config.cache.prefetch.clone(),
        gid.to_string(),
        cid.to_string(),
        rendered.segments.clone(),
    );
    state.stats.record_playlist();
    media_response(rendered.body, upstream)
}

/// 按 `[cache.prefetch]` 预取媒体清单切片；失败只记日志。
fn spawn_prefetch(
    state: Arc<AppState>,
    config: crate::config::model::CachePrefetchCfg,
    gid: String,
    cid: String,
    mut entries: Vec<(String, String, String)>,
) {
    if !config.enabled || entries.is_empty() {
        return;
    }
    let skip_tail = config.skip_tail as usize;
    if skip_tail > 0 && entries.len() > skip_tail {
        entries.truncate(entries.len() - skip_tail);
    }
    if config.prefetch_limit > 0 {
        entries.truncate(config.prefetch_limit as usize);
    }
    if entries.is_empty() {
        return;
    }
    let timeout = std::time::Duration::from_secs(config.timeout.clamp(1, 120));
    let concurrency = config.concurrency.clamp(1, 32) as usize;
    tokio::spawn(async move {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let mut tasks = tokio::task::JoinSet::new();
        for (suffix, url, ext) in entries {
            let path = proxy::segment_path(&state.cache_dir, &gid, &cid, &suffix, &ext);
            if path.is_file() {
                continue;
            }
            let Ok(permit) = semaphore.clone().acquire_owned().await else {
                break;
            };
            let state = state.clone();
            let gid = gid.clone();
            let cid = cid.clone();
            tasks.spawn(async move {
                let _permit = permit;
                let Ok(result) =
                    tokio::time::timeout(timeout, fetch_bytes(&state, &url, &gid, &cid)).await
                else {
                    return;
                };
                let Ok(bytes) = result else {
                    return;
                };
                if let Some(parent) = path.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                let _ = tokio::fs::write(path, bytes).await;
            });
        }
        while tasks.join_next().await.is_some() {}
    });
}

/// 媒体清单响应：VOD 取上游 `#EXT-X-TARGETDURATION`，直播禁止缓存（S-PROXY-03）。
fn media_response(body: String, upstream: &str) -> Response {
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    insert_header(
        &mut response,
        header::CACHE_CONTROL,
        &media_cache_control(upstream),
    );
    response
}

/// v0.8.2 实测：VOD 清单 `public, max-age=<TARGETDURATION>`；直播清单 `no-cache, no-store`。
fn media_cache_control(upstream: &str) -> String {
    let vod = upstream.contains("#EXT-X-PLAYLIST-TYPE:VOD") || upstream.contains("#EXT-X-ENDLIST");
    if vod {
        let seconds = proxy::media::target_duration(upstream).unwrap_or(DEFAULT_TARGET_DURATION);
        format!("public, max-age={seconds}, s-maxage={seconds}")
    } else {
        "no-cache, no-store".to_string()
    }
}

/// 变体媒体清单：抓最佳变体的清单再改写。
async fn variant_playlist(
    state: &Arc<AppState>,
    gid: &str,
    cid: &str,
    self_base: &str,
) -> Response {
    let config = state.snapshot();
    let Some((group, channel_url)) = resolve(&config, gid, cid) else {
        return (StatusCode::NOT_FOUND, "Channel not found").into_response();
    };
    if group.access == "archived" {
        return placeholder(&config);
    }
    let upstream = match upstream_playlist(state, &channel_url, gid, cid).await {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!(%gid, %cid, error = %err, "抓取上游清单失败");
            state.stats.record_upstream_failure();
            return redirect_asset(&config, &config.server.error_url);
        }
    };
    // 上游是媒体清单时没有变体清单这条路（S-PROXY-07）。
    let Some((_attrs, variant_url)) = proxy::media::best_variant(&upstream, &channel_url) else {
        return redirect_asset(&config, &config.server.error_url);
    };
    let text = match fetch_playlist(state, &variant_url, gid, cid).await {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!(url = %variant_url, error = %err, "抓取变体清单失败");
            state.stats.record_upstream_failure();
            return redirect_asset(&config, &config.server.error_url);
        }
    };
    if proxy::modes::looks_like_mpd(&text) {
        return dash_body(text);
    }
    media_entry(state, &config, gid, cid, &variant_url, &text, self_base)
}

/// 上游清单：进程内短暂复用，减少突发请求（S-CACHE-02）。
async fn upstream_playlist(
    state: &AppState,
    url: &str,
    gid: &str,
    cid: &str,
) -> Result<String, reqwest::Error> {
    let key = format!("upstream:{gid}:{cid}");
    if let Some(text) = state.playlists.get(&key) {
        return Ok(text);
    }
    let text = fetch_playlist(state, url, gid, cid).await?;
    state.playlists.put(&key, text.clone());
    Ok(text)
}

/// `proxy` 模式的 MPD：保留正文，把所有 `<BaseURL>` 指到本服务的 `mpd/` 路径。
///
/// 目标实测：绝对、相对与缺失 `<BaseURL>` 都会落成本地反代地址；本地路径带
/// 令牌和 Base64 编码的原始 BaseURL，切片请求再还原上游地址（F-021）。
fn proxy_mpd(
    body: &str,
    gid: &str,
    cid: &str,
    mpd_url: &str,
    self_base: &str,
    keep_seconds: u64,
) -> Response {
    let body = trim_mpd_window(body, keep_seconds);
    let body = rewrite_mpd_base_urls(&body, gid, cid, mpd_url, self_base);
    let body = rewrite_minimum_update_period(&body);
    let body = rewrite_publish_time(&body);
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(MPD_CONTENT_TYPE),
    );
    insert_header(&mut response, header::CACHE_CONTROL, MPD_CACHE);
    response
}

/// 目标 v0.8.2 把 `minimumUpdatePeriod` 统一改成 `PT1S`（F-022）。
///
/// 只改已有属性；缺失时不主动插入，避免改变上游声明的刷新模式。
fn rewrite_minimum_update_period(xml: &str) -> String {
    rewrite_attribute(xml, "minimumUpdatePeriod", "PT1S")
}

/// 目标每次代理 MPD 时把已有 `publishTime` 改成当前 UTC 时间（F-022）。
fn rewrite_publish_time(xml: &str) -> String {
    rewrite_attribute(xml, "publishTime", &format_utc(now_secs()))
}

/// 替换一个双引号或单引号属性的值；保留引号、空白和其余正文。
fn rewrite_attribute(xml: &str, key: &str, value: &str) -> String {
    let mut output = String::with_capacity(xml.len());
    let mut cursor = 0;
    while let Some(offset) = xml[cursor..].find(key) {
        let key_start = cursor + offset;
        output.push_str(&xml[cursor..key_start]);
        let after_key = key_start + key.len();
        let Some(eq_offset) = xml[after_key..].find('=') else {
            output.push_str(&xml[key_start..]);
            return output;
        };
        let eq = after_key + eq_offset;
        let value_start = eq + 1;
        let rest = &xml[value_start..];
        let trimmed = rest.trim_start();
        let whitespace = rest.len() - trimmed.len();
        let Some(quote) = trimmed.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            output.push_str(&xml[key_start..value_start + whitespace]);
            cursor = value_start + whitespace;
            continue;
        };
        let after_quote = &trimmed[1..];
        let Some(end) = after_quote.find(quote) else {
            output.push_str(&xml[key_start..]);
            return output;
        };
        let value_start = value_start + whitespace + 1;
        output.push_str(&xml[key_start..value_start]);
        output.push_str(value);
        cursor = value_start + end;
    }
    output.push_str(&xml[cursor..]);
    output
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

fn format_utc(secs: u64) -> String {
    let days = secs / 86_400;
    let remainder = secs % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        remainder / 3_600,
        remainder % 3_600 / 60,
        remainder % 60
    )
}

/// Howard Hinnant 的 civil_from_days 算法；仅用于 MPD 时间戳。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    (year, month as u32, day as u32)
}

/// 按 `mpd_keep_seconds` 裁掉 MPD 时间线中最旧的切片。
///
/// 只处理带 `SegmentTimeline` 的 `SegmentTemplate` 与 `SegmentList`。裁剪后
/// 若模板使用 `$Number$`，同步增加 `startNumber`，避免播放器请求错位。
fn trim_mpd_window(xml: &str, keep_seconds: u64) -> String {
    if keep_seconds == 0 {
        return xml.to_string();
    }
    let Ok(document) = roxmltree::Document::parse(xml) else {
        return xml.to_string();
    };
    let keep = keep_seconds as f64;
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for parent in document.descendants().filter(|node| node.is_element()) {
        match parent.tag_name().name() {
            "SegmentTemplate" => {
                let Some(timeline) = parent
                    .children()
                    .find(|node| node.is_element() && node.tag_name().name() == "SegmentTimeline")
                else {
                    continue;
                };
                let timescale = parent
                    .attribute("timescale")
                    .and_then(|value| value.parse::<f64>().ok())
                    .unwrap_or(1.0)
                    .max(1.0);
                let spans: Vec<(roxmltree::Node<'_, '_>, f64, usize, f64)> = timeline
                    .children()
                    .filter(|node| node.is_element() && node.tag_name().name() == "S")
                    .filter_map(|node| {
                        let duration = node
                            .attribute("d")
                            .and_then(|value| value.parse::<f64>().ok())
                            .unwrap_or(0.0);
                        let repeat = node
                            .attribute("r")
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0);
                        (duration > 0.0).then_some((node, duration / timescale, repeat + 1, 0.0))
                    })
                    .collect();
                if spans.is_empty() {
                    continue;
                }
                let mut starts = Vec::with_capacity(spans.len());
                let mut cursor = 0.0;
                for (node, duration, repeats, _) in &spans {
                    let start = node
                        .attribute("t")
                        .and_then(|value| value.parse::<f64>().ok())
                        .map(|value| value / timescale)
                        .unwrap_or(cursor);
                    starts.push(start);
                    cursor = start + duration * *repeats as f64;
                }
                let mut spans: Vec<_> = spans
                    .into_iter()
                    .zip(starts)
                    .map(|((node, duration, repeats, _), start)| (node, duration, repeats, start))
                    .collect();
                let total: f64 = spans
                    .iter()
                    .map(|(_, duration, repeats, _)| duration * *repeats as f64)
                    .sum();
                if total <= keep {
                    continue;
                }
                let mut remaining = total;
                let mut removed_segments = 0usize;
                for index in 0..spans.len() {
                    if remaining <= keep {
                        break;
                    }
                    let (node, duration, repeats, start) = spans[index].clone();
                    let span_duration = duration * repeats as f64;
                    if index + 1 < spans.len() && remaining - span_duration >= keep {
                        edits.push((node.range(), String::new()));
                        removed_segments += repeats;
                        remaining -= span_duration;
                        continue;
                    }
                    if repeats <= 1 {
                        if index + 1 < spans.len() {
                            edits.push((node.range(), String::new()));
                            removed_segments += 1;
                            remaining -= span_duration;
                            continue;
                        }
                        break;
                    }
                    let cut =
                        (((remaining - keep) / duration).ceil() as usize).clamp(1, repeats - 1);
                    let new_repeats = repeats - cut;
                    let new_start = start + duration * cut as f64;
                    edits.push((
                        node.range(),
                        rewrite_s_node(&node, new_start * timescale, new_repeats),
                    ));
                    removed_segments += cut;
                    spans[index].2 = new_repeats;
                    break;
                }
                if removed_segments == 0 {
                    continue;
                }
                let media = parent.attribute("media").unwrap_or("");
                if media.contains("$Number$") {
                    let start = parent
                        .attribute("startNumber")
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(1)
                        + removed_segments;
                    match parent.attribute_node("startNumber") {
                        Some(attribute) => {
                            edits.push((attribute.range_value(), start.to_string()));
                        }
                        None => {
                            let start_tag = parent.range().start;
                            if let Some(offset) = xml[start_tag..].find('>') {
                                let at = start_tag + offset;
                                edits.push((at..at, format!(" startNumber=\"{start}\"")));
                            }
                        }
                    }
                }
            }
            "SegmentList" => {
                let timescale = parent
                    .attribute("timescale")
                    .and_then(|value| value.parse::<f64>().ok())
                    .unwrap_or(1.0)
                    .max(1.0);
                let duration = parent
                    .attribute("duration")
                    .and_then(|value| value.parse::<f64>().ok())
                    .unwrap_or(0.0);
                if duration <= 0.0 {
                    continue;
                }
                let segments: Vec<_> = parent
                    .children()
                    .filter(|node| node.is_element() && node.tag_name().name() == "SegmentURL")
                    .collect();
                if segments.len() < 2 {
                    continue;
                }
                let keep_count = ((keep * timescale / duration).ceil() as usize).max(1);
                if keep_count >= segments.len() {
                    continue;
                }
                for segment in &segments[..segments.len() - keep_count] {
                    edits.push((segment.range(), String::new()));
                }
            }
            _ => {}
        }
    }
    if edits.is_empty() {
        return xml.to_string();
    }
    edits.sort_by_key(|(range, _)| range.start);
    let mut output = xml.to_string();
    for (range, replacement) in edits.into_iter().rev() {
        if range.end <= output.len() {
            output.replace_range(range, &replacement);
        }
    }
    output
}

/// 重建 `S` 节点，保留其它属性并覆盖 `t`/`r`。
fn rewrite_s_node(node: &roxmltree::Node<'_, '_>, start: f64, repeats: usize) -> String {
    let mut attrs = Vec::new();
    for attribute in node.attributes() {
        if matches!(attribute.name(), "t" | "r") {
            continue;
        }
        attrs.push(format!(
            "{}=\"{}\"",
            attribute.name(),
            xml_escape(attribute.value())
        ));
    }
    attrs.push(format!("t=\"{}\"", format_mpd_number(start)));
    if repeats > 1 {
        attrs.push(format!("r=\"{}\"", repeats - 1));
    }
    format!("<S {}/>", attrs.join(" "))
}

fn format_mpd_number(value: f64) -> String {
    if (value - value.round()).abs() < f64::EPSILON {
        (value.round() as i64).to_string()
    } else {
        format!("{value}")
    }
}

/// 本地 MPD 反代前缀。
fn local_mpd_prefix(gid: &str, cid: &str, self_base: &str) -> String {
    if self_base.is_empty() {
        format!("/{gid}/{cid}/mpd/")
    } else {
        format!("{}/{gid}/{cid}/mpd/", self_base.trim_end_matches('/'))
    }
}

/// 本地 BaseURL 令牌；防止任意改写后的地址直接访问本服务。
fn mpd_base_token(gid: &str, cid: &str, base: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"rea-iptv/mpd-v1\0");
    hasher.update(gid.as_bytes());
    hasher.update([0]);
    hasher.update(cid.as_bytes());
    hasher.update([0]);
    hasher.update(base.as_bytes());
    hex::encode(&hasher.finalize()[..16])
}

/// 把上游 BaseURL 编成本地路径：`mpd/{token}/{base64}/`。
fn local_mpd_base(gid: &str, cid: &str, self_base: &str, base: &Url) -> String {
    let token = mpd_base_token(gid, cid, base.as_str());
    let encoded = URL_SAFE_NO_PAD.encode(base.as_str());
    format!(
        "{}{token}/{encoded}/",
        local_mpd_prefix(gid, cid, self_base)
    )
}

/// 解析一个 `<BaseURL>`：支持绝对、相对与协议相对地址。
fn resolve_mpd_base(mpd_url: &str, value: &str) -> Option<Url> {
    if let Ok(url) = Url::parse(value) {
        return matches!(url.scheme(), "http" | "https").then_some(url);
    }
    let joined = Url::parse(mpd_url).ok()?.join(value).ok()?;
    matches!(joined.scheme(), "http" | "https").then_some(joined)
}

/// MPD 所在目录；用于缺失 `<BaseURL>` 时的默认基准。
fn mpd_directory(mpd_url: &str) -> Option<Url> {
    let mut url = Url::parse(mpd_url).ok()?;
    url.set_query(None);
    url.set_fragment(None);
    let path = url.path().to_string();
    let cut = path.rfind('/').map(|index| index + 1).unwrap_or(1);
    url.set_path(&path[..cut]);
    Some(url)
}

fn rewrite_mpd_base_urls(
    xml: &str,
    gid: &str,
    cid: &str,
    mpd_url: &str,
    self_base: &str,
) -> String {
    let Ok(document) = roxmltree::Document::parse(xml) else {
        return xml.to_string();
    };
    let mut replacements = Vec::new();
    for node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "BaseURL")
    {
        let value = node.text().unwrap_or("").trim();
        let Some(base) = resolve_mpd_base(mpd_url, value) else {
            continue;
        };
        let local = local_mpd_base(gid, cid, self_base, &base);
        replacements.push((
            node.range(),
            format!("<BaseURL>{}</BaseURL>", xml_escape(&local)),
        ));
    }
    if replacements.is_empty() {
        let Some(directory) = mpd_directory(mpd_url) else {
            return xml.to_string();
        };
        let local = local_mpd_base(gid, cid, self_base, &directory);
        let Some(insert_at) = xml
            .find("<MPD")
            .and_then(|start| xml[start..].find('>').map(|end| start + end + 1))
        else {
            return xml.to_string();
        };
        let mut output = String::with_capacity(xml.len() + local.len() + 18);
        output.push_str(&xml[..insert_at]);
        output.push_str(&format!("<BaseURL>{}</BaseURL>", xml_escape(&local)));
        output.push_str(&xml[insert_at..]);
        return output;
    }
    let mut output = xml.to_string();
    for (range, replacement) in replacements.into_iter().rev() {
        output.replace_range(range, &replacement);
    }
    output
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 代理 `proxy` 模式 MPD 的相对切片。路径来自改写后的 `<BaseURL>`。
async fn mpd_segment(
    state: &Arc<AppState>,
    gid: &str,
    cid: &str,
    path: &str,
    query: Option<&str>,
) -> Response {
    let path = path.trim_start_matches('/');
    if path.is_empty() || path.split('/').any(|part| part == "..") {
        return (StatusCode::BAD_REQUEST, "Bad path").into_response();
    }
    let config = state.snapshot();
    let Some(group) = config.group(gid) else {
        return (StatusCode::NOT_FOUND, "Channel not found").into_response();
    };
    if group.mode != "proxy" {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let Some(channel) = group.channels.iter().find(|channel| channel.id == cid) else {
        return (StatusCode::NOT_FOUND, "Channel not found").into_response();
    };
    let channel_url = match crate::resolver::resolve(state, gid, cid, channel).await {
        Ok(Some(url)) => url,
        Ok(None) => channel.url.clone(),
        Err(error) => {
            tracing::warn!(gid, cid, error = %error, "解析脚本失败");
            return (StatusCode::BAD_GATEWAY, "Upstream Error").into_response();
        }
    };
    let (base, relative, inherited_query) = if let Some((token, rest)) =
        path.split_once('/').filter(|(token, _)| {
            token.len() == 32 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
        let (encoded, relative) = rest.split_once('/').unwrap_or((rest, ""));
        let Some(decoded) = URL_SAFE_NO_PAD
            .decode(encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
        else {
            return (StatusCode::NOT_FOUND, "Not found").into_response();
        };
        if mpd_base_token(gid, cid, &decoded) != token {
            // 目标：本地令牌错误 → 302 error_url（F-022）。
            return redirect_asset(&config, &config.server.error_url);
        }
        let Ok(base) = Url::parse(&decoded) else {
            return (StatusCode::BAD_GATEWAY, "Upstream Error").into_response();
        };
        if !matches!(base.scheme(), "http" | "https") {
            return (StatusCode::BAD_REQUEST, "Bad path").into_response();
        }
        let inherited = base.query().map(str::to_string);
        (base, relative.to_string(), inherited)
    } else {
        let Ok(channel_base) = Url::parse(&channel_url) else {
            return (StatusCode::BAD_GATEWAY, "Upstream Error").into_response();
        };
        (channel_base, path.to_string(), None)
    };
    let target = if let Some(root) = relative.strip_prefix("root/") {
        base.join(&format!("/{root}"))
    } else {
        base.join(&relative)
    };
    let Ok(mut target) = target else {
        return (StatusCode::BAD_REQUEST, "Bad path").into_response();
    };
    if let Some(query) = query.filter(|query| !query.is_empty()) {
        target.set_query(Some(query));
    } else if let Some(query) = inherited_query {
        target.set_query(Some(&query));
    }
    let bytes = match fetch_bytes(state, target.as_str(), gid, cid).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(url = %target, error = %error, "代理 MPD 切片失败");
            state.stats.record_upstream_failure();
            return (StatusCode::BAD_GATEWAY, "Upstream Error").into_response();
        }
    };
    let mime = match target.path().rsplit_once('.').map(|(_, ext)| ext) {
        Some("m4s" | "mp4" | "m4v") => "video/mp4",
        Some("m4a") => "audio/mp4",
        Some("vtt") => "text/vtt",
        Some("ttml" | "xml") => "application/ttml+xml",
        _ => "application/octet-stream",
    };
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    insert_header(
        &mut response,
        header::CACHE_CONTROL,
        &segment_cache_control(state, true),
    );
    response
}

/// MPD 正文透传（转封装属 P1）。
fn dash_body(body: String) -> Response {
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(MPD_CONTENT_TYPE),
    );
    insert_header(&mut response, header::CACHE_CONTROL, MPD_CACHE);
    response
}

/// 本服务对外基址：从 `Host` 头拼出绝对地址前缀。
fn self_base(headers: &HeaderMap) -> String {
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("http");
    match headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    {
        Some(host) if !host.is_empty() => format!("{scheme}://{host}"),
        _ => String::new(),
    }
}

/// 伪装切片：查索引、读缓存或抓上游、按命中改写缓存头。
async fn segment(state: &Arc<AppState>, gid: &str, cid: &str, name: &str) -> Response {
    let config = state.snapshot();
    let Some((group, _url)) = resolve(&config, gid, cid) else {
        // 子路径上的未知分组/频道等价于未知路径（对拍：目标 → `not_found_url`）。
        return placeholder(&config);
    };
    if group.access == "archived" {
        return placeholder(&config);
    }
    let expected = proxy::prefix(&group.id, cid);
    let Some((suffix, ext)) = proxy::parse_segment(name) else {
        return placeholder(&config);
    };
    if !proxy::prefix_matches(&expected, name) {
        // 前缀错值 → 302 占位（S-DIS-17）。
        return placeholder(&config);
    }
    let Some(entry) = state.segments.get(&state.cache_dir, gid, cid, &suffix) else {
        return (StatusCode::NOT_FOUND, "Segment not found").into_response();
    };

    let path = proxy::segment_path(&state.cache_dir, gid, cid, &suffix, &ext);
    if let Ok(bytes) = std::fs::read(&path) {
        state.stats.record_segment(true);
        return segment_response(bytes, &ext, &segment_cache_control(state, false));
    }
    state.stats.record_segment(false);
    stream_segment(
        state.clone(),
        gid.to_string(),
        cid.to_string(),
        entry.url,
        path,
        ext,
        true,
    )
    .await
}

/// 边转发边写缓存。
///
/// v0.8.2 实测：上游正文中断时，客户端仍收到 `200` 和已收到的部分字节，日志记
/// `片段下载中断`；只有完整正文才落盘。这里用通道维持背压，并在流结束后写缓存。
async fn stream_segment(
    state: Arc<AppState>,
    gid: String,
    cid: String,
    url: String,
    path: PathBuf,
    ext: String,
    first: bool,
) -> Response {
    let mut request = state.http_for(&gid, &cid).get(&url);
    for (name, value) in channel_headers(&state, &gid, &cid) {
        request = request.header(name, value);
    }
    let cache_control = segment_cache_control(&state, first);
    let response = match request.send().await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            // v0.8.2 实测：上游切片 404 也先回 200，正文为空；日志记片段拉取失败。
            tracing::warn!(url = %url, status = %response.status(), "片段拉取失败");
            state.stats.record_upstream_failure();
            return segment_response(Vec::new(), &ext, &cache_control);
        }
        Err(error) => {
            tracing::warn!(url = %url, error = %error, "片段拉取失败");
            state.stats.record_upstream_failure();
            return segment_response(Vec::new(), &ext, &cache_control);
        }
    };
    let mime = mime_guess::from_ext(&ext).first_or_octet_stream();
    let (tx, mut rx) = mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(8);
    let log_url = url;
    tokio::spawn(async move {
        let mut response = response;
        let mut cached = Vec::new();
        let mut complete = true;
        loop {
            match response.chunk().await {
                Ok(Some(bytes)) => {
                    cached.extend_from_slice(&bytes);
                    if tx.send(Ok(bytes)).await.is_err() {
                        // 客户端已断开；不再继续读上游，也不缓存半包。
                        return;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(url = %log_url, error = %error, "片段下载中断");
                    complete = false;
                    break;
                }
            }
        }
        if complete {
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            if let Err(error) = tokio::fs::write(&path, &cached).await {
                tracing::warn!(error = %error, "写切片缓存失败");
            }
        }
    });
    let body = Body::from_stream(futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx)));
    let mut response = Response::new(body);
    if let Ok(value) = HeaderValue::from_str(mime.as_ref()) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    insert_header(&mut response, header::CACHE_CONTROL, &cache_control);
    response
}

/// 解析分组与频道。
fn resolve(config: &Config, gid: &str, cid: &str) -> Option<(Group, String)> {
    let group = config.group(gid)?;
    let channel = group.channels.iter().find(|channel| channel.id == cid)?;
    Some((group.clone(), channel.url.clone()))
}

/// 拆入口文件名：`c1.m3u8` → `("c1", "m3u8")`。
fn split_entry(file: &str) -> Option<(String, String)> {
    let (cid, ext) = file.rsplit_once('.')?;
    if cid.is_empty() || !matches!(ext, "m3u8" | "mpd") {
        return None;
    }
    Some((cid.to_string(), ext.to_string()))
}

/// Token 仅校验入口；`open` 组免校验（F-007、F-009）。
fn token_ok(
    state: &AppState,
    config: &Config,
    group: &Group,
    query: &HashMap<String, String>,
) -> bool {
    if !config.auth.token_enabled || group.access == "open" {
        return true;
    }
    query
        .get("token")
        .map(|token| state.play_tokens.verify(token))
        .unwrap_or(false)
}

/// 未知分组/频道：Token 合法（或未启用 Token）→ 占位视频；否则 → Token 失效视频。
fn unknown_endpoint(
    state: &AppState,
    config: &Config,
    query: &HashMap<String, String>,
) -> Response {
    let verified = !config.auth.token_enabled
        || query
            .get("token")
            .map(|token| state.play_tokens.verify(token))
            .unwrap_or(false);
    if verified {
        placeholder(config)
    } else {
        redirect_asset(config, &config.auth.token_failure_url)
    }
}

/// 302 占位视频（未知频道、归档组、错前缀）。
fn placeholder(config: &Config) -> Response {
    redirect_asset(config, &config.server.not_found_url)
}

fn redirect_asset(config: &Config, url: &str) -> Response {
    redirect(config.asset_local(url).unwrap_or(url))
}

/// 手写 302（axum 的 `Redirect::temporary` 是 307）。
fn redirect(url: &str) -> Response {
    if url.trim().is_empty() {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    match HeaderValue::from_str(url) {
        Ok(value) => {
            let mut response = StatusCode::FOUND.into_response();
            response.headers_mut().insert(header::LOCATION, value);
            response
        }
        Err(_) => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

/// 切片响应：按伪装扩展名给类型；忽略 `Range`，总是返回全量（S-DIS-07）。
fn segment_response(bytes: Vec<u8>, ext: &str, cache_control: &str) -> Response {
    let mime = mime_guess::from_ext(ext).first_or_octet_stream();
    let mut response = Response::new(Body::from(bytes));
    if let Ok(value) = HeaderValue::from_str(mime.as_ref()) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    insert_header(&mut response, header::CACHE_CONTROL, cache_control);
    response
}

fn insert_header(response: &mut Response, name: header::HeaderName, value: &str) {
    if let Ok(parsed) = HeaderValue::from_str(value) {
        response.headers_mut().insert(name, parsed);
    }
}

/// 上游自定义头：分组头与频道头合并，频道覆盖同名分组头。
///
/// 目标配置把 `headers` 存为 `[{name,value}]`；本实现同时接受旧的
/// `"名称: 值"` 字符串。`user_agent` 是独立字段，显式同名头优先。
pub(crate) fn channel_headers(state: &AppState, gid: &str, cid: &str) -> Vec<(String, String)> {
    let config = state.snapshot();
    let Some((group, _)) = resolve(&config, gid, cid) else {
        return Vec::new();
    };
    let Some(channel) = group.channels.iter().find(|channel| channel.id == cid) else {
        return Vec::new();
    };
    merge_upstream_headers(
        &group.headers,
        user_agent(&group.extra),
        &channel.headers,
        user_agent(&channel.extra),
    )
}

fn user_agent(extra: &toml::Table) -> Option<&str> {
    extra
        .get("user_agent")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// 纯函数形式，便于单测。
fn merge_upstream_headers(
    group_headers: &[toml::Value],
    group_agent: Option<&str>,
    channel_headers: &[toml::Value],
    channel_agent: Option<&str>,
) -> Vec<(String, String)> {
    let mut merged = Vec::new();
    for header in group_headers {
        if let Some((name, value)) = header_pair(header) {
            push_header(&mut merged, &name, &value);
        }
    }
    // 目标先取频道 UA、其次分组 UA；但分组显式 User-Agent 头优先。
    if let Some(agent) = channel_agent.or(group_agent) {
        push_header_if_absent(&mut merged, "User-Agent", agent);
    }
    for header in channel_headers {
        if let Some((name, value)) = header_pair(header) {
            push_header(&mut merged, &name, &value);
        }
    }
    merged
}

fn header_pair(value: &toml::Value) -> Option<(String, String)> {
    if let Some(text) = value.as_str() {
        let (name, value) = text.split_once(':')?;
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        return Some((name.to_string(), value.trim().to_string()));
    }
    let table = value.as_table()?;
    let name = table.get("name")?.as_str()?.trim();
    let value = table.get("value")?.as_str()?.trim();
    (!name.is_empty()).then(|| (name.to_string(), value.to_string()))
}

fn push_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    if let Some(existing) = headers
        .iter_mut()
        .find(|(existing, _)| existing.eq_ignore_ascii_case(name))
    {
        existing.0 = name.to_string();
        existing.1 = value.to_string();
    } else {
        headers.push((name.to_string(), value.to_string()));
    }
}

fn push_header_if_absent(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    if !headers
        .iter()
        .any(|(existing, _)| existing.eq_ignore_ascii_case(name))
    {
        headers.push((name.to_string(), value.to_string()));
    }
}

/// HLS 清单拉取重试次数（含首次请求）。v0.8.2 日志为 `第 1/4…3/4 次重试`。
const PLAYLIST_RETRY_MAX: usize = 4;

/// 抓上游清单；传输错误、正文解码错误与 5xx 按指数退避重试。
///
/// v0.8.2 实测：404 等 4xx 不重试，直接转 `error_url`；超时/正文中断最多 4 次。
async fn fetch_playlist(
    state: &AppState,
    url: &str,
    gid: &str,
    cid: &str,
) -> Result<String, reqwest::Error> {
    let mut attempt = 1usize;
    loop {
        match fetch(state, url, gid, cid).await {
            Ok(text) => return Ok(text),
            Err(error) => {
                if attempt >= PLAYLIST_RETRY_MAX || !retryable_playlist_error(&error) {
                    return Err(error);
                }
                let wait = std::time::Duration::from_millis(1_000u64 << (attempt - 1));
                tracing::debug!(
                    url = %url,
                    attempt,
                    total = PLAYLIST_RETRY_MAX,
                    wait_ms = wait.as_millis(),
                    "M3U8 拉取重试"
                );
                tokio::time::sleep(wait).await;
                attempt += 1;
            }
        }
    }
}

fn retryable_playlist_error(error: &reqwest::Error) -> bool {
    match error.status() {
        Some(status) => status.is_server_error() || matches!(status.as_u16(), 408 | 429),
        None => true,
    }
}

/// 抓上游文本。
pub(crate) async fn fetch(
    state: &AppState,
    url: &str,
    gid: &str,
    cid: &str,
) -> Result<String, reqwest::Error> {
    let mut request = state.http_for(gid, cid).get(url);
    for (name, value) in channel_headers(state, gid, cid) {
        request = request.header(name, value);
    }
    let response = request.send().await?.error_for_status()?;
    response.text().await
}

/// 抓上游字节。
pub(crate) async fn fetch_bytes(
    state: &AppState,
    url: &str,
    gid: &str,
    cid: &str,
) -> Result<Vec<u8>, reqwest::Error> {
    let mut request = state.http_for(gid, cid).get(url);
    for (name, value) in channel_headers(state, gid, cid) {
        request = request.header(name, value);
    }
    let response = request.send().await?.error_for_status()?;
    Ok(response.bytes().await?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::{
        format_utc, media_cache_control, merge_upstream_headers, mpd_base_token,
        rewrite_minimum_update_period, rewrite_mpd_base_urls, rewrite_publish_time,
        trim_mpd_window,
    };
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    #[test]
    fn media_cache_control_matches_vod_and_live_targets() {
        assert_eq!(
            media_cache_control("#EXTM3U\n#EXT-X-TARGETDURATION:11\n#EXT-X-PLAYLIST-TYPE:VOD\n"),
            "public, max-age=11, s-maxage=11"
        );
        assert_eq!(
            media_cache_control("#EXTM3U\n#EXT-X-TARGETDURATION:8\n#EXTINF:8,\nseg.ts\n"),
            "no-cache, no-store"
        );
        assert_eq!(
            media_cache_control("#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-ENDLIST\n"),
            "public, max-age=4, s-maxage=4"
        );
    }

    fn header(name: &str, value: &str) -> toml::Value {
        toml::Value::Table(
            [
                ("name".to_string(), toml::Value::String(name.into())),
                ("value".to_string(), toml::Value::String(value.into())),
            ]
            .into_iter()
            .collect(),
        )
    }

    #[test]
    fn merges_group_and_channel_headers_with_channel_winning() {
        let merged = merge_upstream_headers(
            &[header("X-Group", "g"), header("X-Both", "group")],
            None,
            &[header("X-Channel", "c"), header("X-Both", "channel")],
            None,
        );
        assert_eq!(
            merged,
            vec![
                ("X-Group".into(), "g".into()),
                ("X-Both".into(), "channel".into()),
                ("X-Channel".into(), "c".into()),
            ]
        );
    }

    #[test]
    fn user_agent_precedence_matches_target() {
        let group = [header("User-Agent", "GroupHeader")];
        let merged = merge_upstream_headers(&group, Some("GroupField"), &[], Some("ChannelField"));
        assert_eq!(merged, vec![("User-Agent".into(), "GroupHeader".into())]);

        let merged = merge_upstream_headers(&[], Some("GroupField"), &[], Some("ChannelField"));
        assert_eq!(merged, vec![("User-Agent".into(), "ChannelField".into())]);

        let merged = merge_upstream_headers(
            &[],
            Some("GroupField"),
            &[header("User-Agent", "ChannelHeader")],
            Some("ChannelField"),
        );
        assert_eq!(merged, vec![("User-Agent".into(), "ChannelHeader".into())]);
    }

    #[test]
    fn accepts_legacy_header_strings() {
        let merged = merge_upstream_headers(
            &[toml::Value::String("X-Token: abc".into())],
            None,
            &[toml::Value::String("Referer: http://ref/".into())],
            None,
        );
        assert_eq!(
            merged,
            vec![
                ("X-Token".into(), "abc".into()),
                ("Referer".into(), "http://ref/".into()),
            ]
        );
    }

    fn template_mpd() -> &'static str {
        r#"<MPD><Period><AdaptationSet><SegmentTemplate timescale="1000" startNumber="10" media="seg-$Number$.m4s"><SegmentTimeline><S t="0" d="2000"/><S t="2000" d="2000"/><S t="4000" d="2000"/><S t="6000" d="2000"/></SegmentTimeline></SegmentTemplate></AdaptationSet></Period></MPD>"#
    }

    #[test]
    fn rewrites_minimum_update_period_to_one_second() {
        let xml = r#"<MPD minimumUpdatePeriod="PT2.0S" type="dynamic"><Period/></MPD>"#;
        let output = rewrite_minimum_update_period(xml);
        assert_eq!(
            output,
            r#"<MPD minimumUpdatePeriod="PT1S" type="dynamic"><Period/></MPD>"#
        );
        let single = r#"<MPD minimumUpdatePeriod='PT4S'><Period/></MPD>"#;
        assert!(rewrite_minimum_update_period(single).contains("minimumUpdatePeriod='PT1S'"));
        let absent = r#"<MPD type="dynamic"><Period/></MPD>"#;
        assert_eq!(rewrite_minimum_update_period(absent), absent);
    }

    #[test]
    fn publish_time_uses_utc_and_keeps_absence() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        let xml = r#"<MPD publishTime="2020-01-01T00:00:00Z"><Period/></MPD>"#;
        let output = rewrite_publish_time(xml);
        assert!(!output.contains("2020-01-01"), "{output}");
        assert!(output.contains("publishTime=\""));
        let absent = r#"<MPD type="dynamic"><Period/></MPD>"#;
        assert_eq!(rewrite_publish_time(absent), absent);
    }

    #[test]
    fn trim_mpd_keeps_tail_and_advances_number() {
        let output = trim_mpd_window(template_mpd(), 5);
        assert!(!output.contains("t=\"0\""));
        assert!(!output.contains("t=\"2000\""));
        assert!(output.contains("t=\"4000\""));
        assert!(output.contains("t=\"6000\""));
        assert!(output.contains("startNumber=\"12\""), "{output}");
    }

    #[test]
    fn trim_mpd_handles_repeat_and_missing_start_number() {
        let xml = r#"<MPD><Period><AdaptationSet><SegmentTemplate timescale="1" media="s-$Number$.m4s"><SegmentTimeline><S t="0" d="1" r="4"/><S t="5" d="5"/></SegmentTimeline></SegmentTemplate></AdaptationSet></Period></MPD>"#;
        let output = trim_mpd_window(xml, 7);
        assert!(!output.contains("d=\"1\" r=\"4\""), "{output}");
        assert!(output.contains("startNumber=\"4\""), "{output}");
        assert!(output.contains("t=\"5\""));
    }

    #[test]
    fn trim_mpd_handles_segment_list_and_limits() {
        let xml = r#"<MPD><Period><AdaptationSet><SegmentList timescale="1" duration="2"><SegmentURL media="a.m4s"/><SegmentURL media="b.m4s"/><SegmentURL media="c.m4s"/><SegmentURL media="d.m4s"/></SegmentList></AdaptationSet></Period></MPD>"#;
        let output = trim_mpd_window(xml, 3);
        assert!(!output.contains("a.m4s"));
        assert!(!output.contains("b.m4s"));
        assert!(output.contains("c.m4s"));
        assert!(output.contains("d.m4s"));
        assert_eq!(trim_mpd_window(xml, 0), xml);
        assert_eq!(trim_mpd_window(xml, 100), xml);
        assert_eq!(trim_mpd_window("not xml", 5), "not xml");
    }

    fn decode_local(xml: &str) -> (String, String) {
        let start = xml.find("<BaseURL>").expect("BaseURL") + "<BaseURL>".len();
        let end = xml[start..].find("</BaseURL>").expect("end") + start;
        let local = &xml[start..end];
        let rest = local
            .strip_prefix("http://self/g/c/mpd/")
            .expect("local prefix");
        let (token, rest) = rest.split_once('/').expect("token");
        let (encoded, _) = rest.split_once('/').unwrap_or((rest, ""));
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded)
            .expect("base64")
            .into_iter()
            .map(char::from)
            .collect();
        (token.to_string(), decoded)
    }

    #[test]
    fn rewrites_absolute_base_url_through_local_token() {
        let xml = r#"<MPD><BaseURL>http://cdn.example/a/</BaseURL><Period/></MPD>"#;
        let output = rewrite_mpd_base_urls(
            xml,
            "g",
            "c",
            "http://origin.example/dash/live.mpd",
            "http://self",
        );
        assert!(!output.contains("http://cdn.example/a/"), "{output}");
        let (token, decoded) = decode_local(&output);
        assert_eq!(decoded, "http://cdn.example/a/");
        assert_eq!(mpd_base_token("g", "c", &decoded), token);
    }

    #[test]
    fn resolves_relative_and_missing_base_urls() {
        let relative = r#"<MPD><BaseURL>sub/</BaseURL><Period/></MPD>"#;
        let output = rewrite_mpd_base_urls(
            relative,
            "g",
            "c",
            "http://origin.example/dash/live.mpd",
            "http://self",
        );
        assert_eq!(decode_local(&output).1, "http://origin.example/dash/sub/");

        let missing = r#"<MPD><Period/></MPD>"#;
        let output = rewrite_mpd_base_urls(
            missing,
            "g",
            "c",
            "http://origin.example/dash/live.mpd",
            "http://self",
        );
        assert_eq!(decode_local(&output).1, "http://origin.example/dash/");
    }
}
