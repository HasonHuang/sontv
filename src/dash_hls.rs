//! DASH → HLS：把 MPD 转成 HLS 主清单、媒体清单，并按需代理初始化段与切片。
//!
//! 证据：F-016。目标行为：
//! - `/{gid}/{cid}.m3u8` 返回 HLS master；媒体清单为 `/{gid}/{cid}/{track}/index.txt`；
//! - 轨道名 `video_{高}p` / `audio-{n}`；init 与切片伪装成图片扩展名；
//! - 支持 MPD 级 `<BaseURL>` 与 `SegmentTimeline`；
//! - 出口不含 `#EXT-X-KEY`（KID 只识别，不解密）。
//!
//! 本模块不调用 ffmpeg：MPD 解析后按 HLS 清单重写，切片字节原样代理。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::{
    dash::{self, Presentation, Stream},
    state::AppState,
};

/// 默认伪装扩展名。
const EXTS: &[&str] = &["jpg", "png", "gif", "bmp", "webp"];

/// 一个可代理的 DASH 文件。
#[derive(Debug, Clone)]
pub struct Entry {
    /// 上游候选地址；第一个为首选。
    pub urls: Vec<String>,
    /// 伪装扩展名。
    pub ext: String,
}

impl Entry {
    fn content_type(&self) -> &'static str {
        match self.ext.as_str() {
            "png" => "image/png",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            "webp" => "image/webp",
            "vtt" => "text/vtt; charset=utf-8",
            _ => "image/jpeg",
        }
    }
}

/// `/panel/api/dash/status` 的轨道摘要。目标字段见 F-016 §1。
#[derive(Debug, Clone, Serialize)]
pub struct TrackStatus {
    pub name: String,
    pub kind: String,
    pub lang: Option<String>,
    pub label: String,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub bandwidth: Option<u64>,
    pub codecs: Option<String>,
    pub frame_rate: Option<String>,
    pub audio_channels: Option<String>,
}

fn track_status(stream: &Stream) -> TrackStatus {
    let label = match stream.kind.as_str() {
        "video" => stream
            .height
            .map(|height| format!("{height}p"))
            .unwrap_or_else(|| stream.name.clone()),
        "audio" => audio_label(stream),
        "text" => stream.lang.clone().unwrap_or_else(|| stream.name.clone()),
        _ => stream.name.clone(),
    };
    TrackStatus {
        name: stream.name.clone(),
        kind: stream.kind.clone(),
        lang: stream.lang.clone(),
        label,
        width: stream.width,
        height: stream.height,
        bandwidth: stream.bandwidth,
        codecs: stream.codecs.clone(),
        frame_rate: stream.frame_rate.clone(),
        audio_channels: stream.audio_channels.clone(),
    }
}

fn track_statuses(presentation: &Presentation) -> Vec<TrackStatus> {
    presentation.streams.iter().map(track_status).collect()
}

fn audio_label(stream: &Stream) -> String {
    match stream.audio_channels.as_deref() {
        Some("1") => format!("{} (Mono)", stream.name),
        Some("2") => format!("{} (Stereo)", stream.name),
        Some(channels) => format!("{} ({}ch)", stream.name, channels),
        None => stream.name.clone(),
    }
}

#[derive(Debug)]
struct Session {
    upstream: String,
    keep_seconds: u64,
    pool_size: u32,
    entries: HashMap<String, Entry>,
    tracks: Vec<TrackStatus>,
    ready: bool,
    segment_count: usize,
    started: Instant,
    last: Instant,
}

/// DASH 会话管理器。
#[derive(Default)]
pub struct Manager {
    sessions: Mutex<HashMap<String, Session>>,
    janitor: AtomicBool,
}

impl Manager {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(gid: &str, cid: &str) -> String {
        format!("{gid}/{cid}")
    }

    fn settings(state: &AppState, gid: &str, cid: &str) -> Option<(String, u64, u32)> {
        let config = state.snapshot();
        let group = config.group(gid)?;
        let channel = group.channels.iter().find(|channel| channel.id == cid)?;
        Some((
            channel.url.clone(),
            group.mpd_keep_seconds,
            channel.url_pool_size.max(1),
        ))
    }

    fn upsert(&self, gid: &str, cid: &str, upstream: String, keep: u64, pool: u32) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .entry(Self::key(gid, cid))
            .and_modify(|session| {
                session.upstream = upstream.clone();
                session.keep_seconds = keep;
                session.pool_size = pool;
                session.last = Instant::now();
            })
            .or_insert_with(|| Session {
                upstream,
                keep_seconds: keep,
                pool_size: pool,
                entries: HashMap::new(),
                tracks: Vec::new(),
                ready: false,
                segment_count: 0,
                started: Instant::now(),
                last: Instant::now(),
            });
    }

    fn touch(&self, gid: &str, cid: &str) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = sessions.get_mut(&Self::key(gid, cid)) {
            session.last = Instant::now();
        }
    }

    fn set_entries(&self, gid: &str, cid: &str, entries: Vec<(String, Entry)>) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let session = sessions
            .entry(Self::key(gid, cid))
            .or_insert_with(|| Session {
                upstream: String::new(),
                keep_seconds: 0,
                pool_size: 1,
                entries: HashMap::new(),
                tracks: Vec::new(),
                ready: false,
                segment_count: 0,
                started: Instant::now(),
                last: Instant::now(),
            });
        session.entries.extend(entries);
        session.last = Instant::now();
    }

    fn set_ready(&self, gid: &str, cid: &str, segment_count: usize) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = sessions.get_mut(&Self::key(gid, cid)) {
            session.ready = true;
            session.segment_count = segment_count;
            session.last = Instant::now();
        }
    }

    fn set_tracks(&self, gid: &str, cid: &str, tracks: Vec<TrackStatus>) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = sessions.get_mut(&Self::key(gid, cid)) {
            session.tracks = tracks;
            session.last = Instant::now();
        }
    }

    /// `/panel/api/dash/status` 摘要。目标形状见 F-016 §1。
    pub fn status(&self) -> Vec<serde_json::Value> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .iter()
            .map(|(key, session)| {
                let mut parts = key.splitn(2, '/');
                let gid = parts.next().unwrap_or("");
                let cid = parts.next().unwrap_or("");
                serde_json::json!({
                    "stream_key": format!("{gid}_{cid}"),
                    "source_url": session.upstream,
                    "phase": if session.ready { "ready" } else { "starting" },
                    "segment_count": session.segment_count,
                    "last_error": serde_json::Value::Null,
                    "checked_ms": 0,
                    "uptime_secs": session.started.elapsed().as_secs(),
                    "active_tracks": session.tracks,
                    "group_id": gid,
                    "channel_id": cid,
                })
            })
            .collect()
    }

    fn upstream(&self, gid: &str, cid: &str) -> Option<String> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .get(&Self::key(gid, cid))
            .map(|session| session.upstream.clone())
            .filter(|upstream| !upstream.is_empty())
    }

    fn entry(&self, gid: &str, cid: &str, path: &str) -> Option<Entry> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .get(&Self::key(gid, cid))
            .and_then(|session| session.entries.get(path).cloned())
    }

    fn ensure_janitor(self: &Arc<Self>, state: &Arc<AppState>) {
        if self.janitor.swap(true, Ordering::SeqCst) {
            return;
        }
        let manager = self.clone();
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let idle = Duration::from_secs(
                    state
                        .snapshot()
                        .cache
                        .dash
                        .ondemand_idle_secs
                        .clamp(5, 86_400),
                );
                let mut sessions = manager.sessions.lock().unwrap_or_else(|e| e.into_inner());
                sessions.retain(|_, session| session.last.elapsed() <= idle);
            }
        });
    }

    /// 生成 HLS 主清单，并登记会话。
    pub async fn master(
        self: &Arc<Self>,
        state: &Arc<AppState>,
        gid: &str,
        cid: &str,
        upstream: &str,
        self_base: &str,
    ) -> Result<String, String> {
        let (_, keep, pool) =
            Self::settings(state, gid, cid).unwrap_or_else(|| (upstream.to_string(), 0, 1));
        let dash = state.snapshot().cache.dash.clone();
        let keep = effective_keep(keep, dash.playlist_min_secs);
        let xml = fetch_mpd(state, gid, cid, upstream, &dash).await?;
        let presentation = dash::parse_presentation(&xml, upstream)
            .map_err(|error| format!("MPD 解析失败: {error:#}"))?;
        let body = render_master(&presentation, gid, cid, self_base);
        self.upsert(gid, cid, upstream.to_string(), keep, pool);
        self.set_tracks(gid, cid, track_statuses(&presentation));
        self.set_ready(
            gid,
            cid,
            presentation
                .streams
                .iter()
                .map(|stream| stream.segments.len())
                .max()
                .unwrap_or(0),
        );
        self.ensure_janitor(state);
        info!(
            gid,
            cid,
            streams = presentation.streams.len(),
            "DASH 主清单就绪"
        );
        Ok(body)
    }

    /// 处理 DASH 子路径：媒体清单、init、切片。
    ///
    /// `rest` 形如 `video_720p/index.txt`、`video_720p/init.webp`。
    pub async fn serve(
        self: &Arc<Self>,
        state: &Arc<AppState>,
        gid: &str,
        cid: &str,
        rest: &str,
        self_base: &str,
    ) -> Option<Response> {
        let (configured, keep, pool) = Self::settings(state, gid, cid)?;
        let segment_cache = format!(
            "public, max-age={}",
            state.snapshot().cache.ttl.segment_secs.max(1)
        );
        let upstream = self.upstream(gid, cid).unwrap_or(configured);
        let (track, file) = rest.split_once('/')?;
        if track.is_empty() || file.is_empty() {
            return None;
        }
        if file == "index.txt" || file == "index.m3u8" {
            let dash = state.snapshot().cache.dash.clone();
            let xml = match fetch_mpd(state, gid, cid, &upstream, &dash).await {
                Ok(xml) => xml,
                Err(error) => {
                    warn!(gid, cid, error = %error, "DASH 媒体清单抓取失败");
                    return Some((StatusCode::BAD_GATEWAY, "Upstream Error").into_response());
                }
            };
            let presentation = match dash::parse_presentation(&xml, &upstream) {
                Ok(value) => value,
                Err(error) => {
                    warn!(gid, cid, error = %error, "DASH MPD 解析失败");
                    return Some((StatusCode::BAD_GATEWAY, "Upstream Error").into_response());
                }
            };
            let Some(stream) = presentation
                .streams
                .iter()
                .find(|stream| stream.name == track)
            else {
                return Some((StatusCode::NOT_FOUND, "Segment not found").into_response());
            };
            let formats = disguise_formats(state);
            let window = effective_keep(keep, dash.playlist_min_secs);
            let start_offset = if presentation.is_live {
                dash.start_time_offset_secs.max(0.0)
            } else {
                0.0
            };
            let rendered = render_media(
                &presentation,
                stream,
                MediaOptions {
                    gid,
                    cid,
                    self_base,
                    keep_seconds: window,
                    pool_size: pool,
                    formats: &formats,
                    track_index: presentation
                        .streams
                        .iter()
                        .position(|candidate| candidate.name == track)
                        .unwrap_or(0),
                    start_offset,
                },
            );
            self.upsert(gid, cid, upstream, keep, pool);
            self.set_tracks(gid, cid, track_statuses(&presentation));
            self.set_ready(gid, cid, rendered.segment_count);
            self.set_entries(gid, cid, rendered.entries);
            self.touch(gid, cid);
            let mut response = Response::new(Body::from(rendered.body));
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
            let cache = if presentation.is_live {
                "public, max-age=2, s-maxage=2"
            } else {
                "public, max-age=5, s-maxage=5"
            };
            insert_header(&mut response, header::CACHE_CONTROL, cache);
            return Some(response);
        }

        // init / 媒体切片：先查登记表；缺失时重取 MPD 再登记一次。
        let mut entry = self.entry(gid, cid, rest);
        if entry.is_none() {
            if let Ok(xml) = crate::http::routes::stream::fetch(state, &upstream, gid, cid).await {
                if let Ok(presentation) = dash::parse_presentation(&xml, &upstream) {
                    let dash = state.snapshot().cache.dash.clone();
                    let formats = disguise_formats(state);
                    let window = effective_keep(keep, dash.playlist_min_secs);
                    let start_offset = if presentation.is_live {
                        dash.start_time_offset_secs.max(0.0)
                    } else {
                        0.0
                    };
                    let mut segment_count = 0;
                    for (index, stream) in presentation.streams.iter().enumerate() {
                        let rendered = render_media(
                            &presentation,
                            stream,
                            MediaOptions {
                                gid,
                                cid,
                                self_base,
                                keep_seconds: window,
                                pool_size: pool,
                                formats: &formats,
                                track_index: index,
                                start_offset,
                            },
                        );
                        segment_count = segment_count.max(rendered.segment_count);
                        self.set_entries(gid, cid, rendered.entries);
                    }
                    self.set_ready(gid, cid, segment_count);
                }
            }
            entry = self.entry(gid, cid, rest);
        }
        let Some(entry) = entry else {
            return Some((StatusCode::NOT_FOUND, "Segment not found").into_response());
        };
        let path = cache_path(&state.cache_dir, gid, cid, rest);
        if let Ok(bytes) = std::fs::read(&path) {
            self.touch(gid, cid);
            return Some(bytes_response(bytes, &entry, &segment_cache));
        }
        let mut last_error = None;
        let dash = state.snapshot().cache.dash.clone();
        let timeout = Duration::from_secs(dash.segment_timeout_secs.clamp(1, 120));
        let attempts = dash.retry_max.min(32);
        for url in &entry.urls {
            for attempt in 0..=attempts {
                match tokio::time::timeout(
                    timeout,
                    crate::http::routes::stream::fetch_bytes(state, url, gid, cid),
                )
                .await
                {
                    Ok(Ok(bytes)) => {
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::write(&path, &bytes);
                        self.touch(gid, cid);
                        return Some(bytes_response(bytes, &entry, &segment_cache));
                    }
                    Ok(Err(error)) => {
                        last_error = Some(error.to_string());
                    }
                    Err(_) => {
                        last_error = Some("segment timeout".to_string());
                    }
                }
                if attempt < attempts {
                    tokio::time::sleep(retry_delay(&dash, attempt)).await;
                }
            }
        }
        warn!(
            gid,
            cid,
            path = rest,
            error = last_error.as_deref().unwrap_or("no url"),
            "DASH 切片抓取失败"
        );
        state.stats.record_upstream_failure();
        Some((StatusCode::BAD_GATEWAY, "Upstream Error").into_response())
    }
}

fn disguise_formats(state: &AppState) -> Vec<String> {
    let config = state.snapshot();
    if config.cache.disguise.enabled {
        let formats = &config.cache.disguise.formats;
        if !formats.is_empty() {
            return formats.clone();
        }
    }
    EXTS.iter().map(|ext| ext.to_string()).collect()
}

struct RenderedMedia {
    body: String,
    entries: Vec<(String, Entry)>,
    segment_count: usize,
}

/// 生成 HLS master。
pub fn render_master(presentation: &Presentation, gid: &str, cid: &str, self_base: &str) -> String {
    let audio: Vec<&Stream> = presentation
        .streams
        .iter()
        .filter(|stream| stream.kind == "audio")
        .collect();
    let text: Vec<&Stream> = presentation
        .streams
        .iter()
        .filter(|stream| stream.kind == "text")
        .collect();
    let mut body = String::from("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    for (index, stream) in audio.iter().enumerate() {
        body.push_str(&format!(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",LANGUAGE=\"{}\",NAME=\"{}\",DEFAULT={},AUTOSELECT=YES",
            escape(stream.lang.as_deref().unwrap_or("und")),
            escape(&audio_label(stream)),
            if index == 0 { "YES" } else { "NO" },
        ));
        if let Some(channels) = channels(stream) {
            body.push_str(&format!(",CHANNELS=\"{channels}\""));
        }
        body.push_str(&format!(
            ",URI=\"{}\"\n",
            media_url(self_base, gid, cid, &stream.name)
        ));
    }
    for (index, stream) in text.iter().enumerate() {
        body.push_str(&format!(
            "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",LANGUAGE=\"{}\",NAME=\"{}\",DEFAULT={},AUTOSELECT=YES,URI=\"{}\"\n",
            escape(stream.lang.as_deref().unwrap_or("und")),
            escape(&audio_label(stream)),
            if index == 0 { "YES" } else { "NO" },
            media_url(self_base, gid, cid, &stream.name)
        ));
    }

    let variants: Vec<&Stream> = if presentation
        .streams
        .iter()
        .any(|stream| stream.kind == "video")
    {
        presentation
            .streams
            .iter()
            .filter(|stream| stream.kind == "video")
            .collect()
    } else {
        presentation
            .streams
            .iter()
            .filter(|stream| stream.kind != "text")
            .take(1)
            .collect()
    };
    for stream in variants {
        let mut attrs = vec![format!(
            "BANDWIDTH={}",
            stream.bandwidth.unwrap_or(1).max(1)
        )];
        if let Some(codecs) = codecs(stream, audio.first().copied()) {
            attrs.push(format!("CODECS=\"{}\"", escape(&codecs)));
        }
        if let (Some(width), Some(height)) = (stream.width, stream.height) {
            attrs.push(format!("RESOLUTION={width}x{height}"));
        }
        if let Some(frame_rate) = &stream.frame_rate {
            if let Some(value) = frame_rate_number(frame_rate) {
                attrs.push(format!("FRAME-RATE={value}"));
            }
        }
        if !audio.is_empty() {
            attrs.push("AUDIO=\"audio\"".to_string());
        }
        if !text.is_empty() {
            attrs.push("SUBTITLES=\"subs\"".to_string());
        }
        body.push_str(&format!(
            "#EXT-X-STREAM-INF:{}\n{}\n",
            attrs.join(","),
            media_url(self_base, gid, cid, &stream.name)
        ));
    }
    body
}

/// 媒体清单渲染参数。
struct MediaOptions<'a> {
    gid: &'a str,
    cid: &'a str,
    self_base: &'a str,
    keep_seconds: u64,
    pool_size: u32,
    formats: &'a [String],
    track_index: usize,
    start_offset: f64,
}

/// 生成媒体清单并登记 init/切片。
fn render_media(
    presentation: &Presentation,
    stream: &Stream,
    options: MediaOptions<'_>,
) -> RenderedMedia {
    let MediaOptions {
        gid,
        cid,
        self_base,
        keep_seconds,
        pool_size,
        formats,
        track_index,
        start_offset,
    } = options;
    let mut picked: Vec<&dash::Segment> = stream.segments.iter().collect();
    if keep_seconds > 0 {
        let mut total = 0.0;
        let mut start = picked.len();
        for (index, segment) in picked.iter().enumerate().rev() {
            total += segment.duration.max(0.001);
            start = index;
            if total >= keep_seconds as f64 {
                break;
            }
        }
        picked = picked.split_off(start);
    }
    if picked.is_empty() {
        picked = stream.segments.iter().rev().take(1).collect();
        picked.reverse();
    }
    let target = picked
        .iter()
        .map(|segment| segment.duration.ceil() as u64)
        .max()
        .unwrap_or(1)
        .max(1);
    let first = picked.first().map(|segment| segment.number).unwrap_or(0);
    let is_live = presentation.is_live;
    let mut body = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:{first}\n#EXT-X-DISCONTINUITY-SEQUENCE:0\n"
    );
    if is_live && start_offset > 0.0 {
        body.push_str(&format!("#EXT-X-START:TIME-OFFSET=-{start_offset:.3}\n"));
    }
    body.push_str("#EXT-X-INDEPENDENT-SEGMENTS\n");
    let mut entries = Vec::new();
    let init_ext = pick_ext(formats, track_index);
    if !stream.init.is_empty() {
        let init_name = format!("init.{init_ext}");
        let path = format!("{}/{init_name}", stream.name);
        entries.push((
            path.clone(),
            Entry {
                urls: choose_urls(&stream.init, &stream.kind, 0, pool_size),
                ext: init_ext.clone(),
            },
        ));
        body.push_str(&format!(
            "#EXT-X-MAP:URI=\"{}\"\n",
            media_file_url(self_base, gid, cid, &path)
        ));
    }
    for (index, segment) in picked.iter().enumerate() {
        if segment.urls.is_empty() {
            continue;
        }
        let ext = pick_ext(formats, track_index + index + 1);
        let name = format!(
            "{}_{}.{}",
            segment.number,
            short_hash(&segment.urls[0]),
            ext
        );
        let path = format!("{}/{name}", stream.name);
        entries.push((
            path.clone(),
            Entry {
                urls: choose_urls(&segment.urls, &stream.kind, index, pool_size),
                ext,
            },
        ));
        body.push_str(&format!(
            "#EXTINF:{:.6},\n{}\n",
            segment.duration.max(0.001),
            media_file_url(self_base, gid, cid, &path)
        ));
    }
    if !is_live {
        body.push_str("#EXT-X-ENDLIST\n");
    }
    RenderedMedia {
        body,
        entries,
        segment_count: picked.len(),
    }
}

fn choose_urls(urls: &[String], kind: &str, index: usize, pool_size: u32) -> Vec<String> {
    if urls.len() <= 1 || pool_size <= 1 {
        return urls.to_vec();
    }
    // 目标面板自述：视频固定 pool[0]，音频/字幕在 pool[1..] 轮询。
    let start = if kind == "video" {
        0
    } else {
        1 + index % (urls.len() - 1)
    };
    let mut out = Vec::with_capacity(urls.len());
    for offset in 0..urls.len() {
        out.push(urls[(start + offset) % urls.len()].clone());
    }
    out
}

fn pick_ext(formats: &[String], index: usize) -> String {
    if formats.is_empty() {
        return "jpg".to_string();
    }
    formats[index % formats.len()]
        .trim_start_matches('.')
        .to_ascii_lowercase()
}

fn short_hash(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    hex::encode(&digest[..8])
}

fn media_url(self_base: &str, gid: &str, cid: &str, track: &str) -> String {
    media_file_url(self_base, gid, cid, &format!("{track}/index.txt"))
}

fn media_file_url(self_base: &str, gid: &str, cid: &str, rest: &str) -> String {
    let base = self_base.trim_end_matches('/');
    if base.is_empty() {
        format!("/{gid}/{cid}/{rest}")
    } else {
        format!("{base}/{gid}/{cid}/{rest}")
    }
}

fn codecs(stream: &Stream, audio: Option<&Stream>) -> Option<String> {
    let mut values = Vec::new();
    if let Some(codecs) = &stream.codecs {
        values.push(codecs.clone());
    }
    if let Some(codecs) = audio.and_then(|stream| stream.codecs.as_ref()) {
        if !values.contains(codecs) {
            values.push(codecs.clone());
        }
    }
    if values.is_empty() {
        None
    } else {
        Some(values.join(","))
    }
}

fn channels(stream: &Stream) -> Option<&str> {
    let value = stream.audio_channels.as_deref()?;
    // `audioSamplingRate` 不是声道数；只在明显是 1/2/6/8 时输出。
    match value {
        "1" | "2" | "6" | "8" => Some(value),
        _ => None,
    }
}

fn frame_rate_number(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some((numerator, denominator)) = value.split_once('/') {
        let numerator: f64 = numerator.parse().ok()?;
        let denominator: f64 = denominator.parse().ok()?;
        if denominator == 0.0 {
            return None;
        }
        Some(format!("{:.3}", numerator / denominator))
    } else {
        let value: f64 = value.parse().ok()?;
        Some(format!("{value:.3}"))
    }
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn cache_path(root: &Path, gid: &str, cid: &str, rest: &str) -> PathBuf {
    root.join("dash")
        .join(safe(gid))
        .join(safe(cid))
        .join(safe_path(rest))
}

fn safe(value: &str) -> String {
    let mapped: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if mapped.is_empty() {
        "empty".to_string()
    } else {
        mapped
    }
}

fn safe_path(value: &str) -> String {
    value.split('/').map(safe).collect::<Vec<_>>().join("/")
}

fn bytes_response(bytes: Vec<u8>, entry: &Entry, cache: &str) -> Response {
    let mut response = Response::new(Body::from(bytes));
    if let Ok(value) = HeaderValue::from_str(entry.content_type()) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    insert_header(&mut response, header::CACHE_CONTROL, cache);
    response
}

fn insert_header(response: &mut Response, name: header::HeaderName, value: &str) {
    if let Ok(parsed) = HeaderValue::from_str(value) {
        response.headers_mut().insert(name, parsed);
    }
}

/// 计算 DASH 重试等待时间。
fn retry_delay(config: &crate::config::model::CacheDashCfg, attempt: u32) -> Duration {
    let initial = config.retry_initial_ms as f64;
    let factor = config.retry_backoff.max(1.0);
    let max = config.retry_max_ms.min(60_000) as f64;
    let delay = (initial * factor.powi(attempt as i32)).min(max);
    Duration::from_millis(delay.max(config.min_request_interval_ms as f64) as u64)
}

/// 计算 DASH 媒体清单窗口：分组 `mpd_keep_seconds` 优先，否则用 `playlist_min_secs`。
///
/// 目标实测：`playlist_min_secs=30`、4 秒切片 → 8 条；`=12` → 3 条。
fn effective_keep(group_keep: u64, playlist_min_secs: f64) -> u64 {
    if group_keep > 0 {
        group_keep
    } else if playlist_min_secs > 0.0 {
        playlist_min_secs.ceil() as u64
    } else {
        0
    }
}

/// 拉取 MPD，按 `[cache.dash]` 的超时与重试参数执行。
async fn fetch_mpd(
    state: &Arc<AppState>,
    gid: &str,
    cid: &str,
    url: &str,
    config: &crate::config::model::CacheDashCfg,
) -> Result<String, String> {
    let timeout = Duration::from_secs(config.mpd_timeout_secs.clamp(1, 120));
    let attempts = config.retry_max.min(32);
    let mut last_error = String::new();
    for attempt in 0..=attempts {
        match tokio::time::timeout(
            timeout,
            crate::http::routes::stream::fetch(state, url, gid, cid),
        )
        .await
        {
            Ok(Ok(body)) => return Ok(body),
            Ok(Err(error)) => last_error = error.to_string(),
            Err(_) => last_error = "MPD timeout".to_string(),
        }
        if attempt < attempts {
            tokio::time::sleep(retry_delay(config, attempt)).await;
        }
    }
    Err(format!("Upstream Error: {last_error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dash::{parse_presentation, Segment};

    fn presentation() -> Presentation {
        parse_presentation(
            r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT8S">
  <BaseURL>http://origin.example/dash/</BaseURL>
  <Period>
    <AdaptationSet contentType="video" maxWidth="1280" maxHeight="720">
      <SegmentTemplate timescale="1000" initialization="init-$RepresentationID$.mp4"
        media="chunk-$RepresentationID$-$Number%03d$.m4s" startNumber="1">
        <SegmentTimeline><S d="2000" r="3"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v0" codecs="avc1.4d401f" bandwidth="2500000" width="1280" height="720"/>
    </AdaptationSet>
    <AdaptationSet contentType="audio" lang="zh">
      <SegmentTemplate timescale="1000" initialization="a-init.mp4"
        media="a-$Number$.m4s" startNumber="1">
        <SegmentTimeline><S d="2000" r="3"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="a0" codecs="mp4a.40.2" bandwidth="128000"/>
    </AdaptationSet>
  </Period>
</MPD>"#,
            "http://origin.example/master.mpd",
        )
        .expect("MPD")
    }

    #[test]
    fn master_has_audio_and_video() {
        let body = render_master(&presentation(), "g", "c", "http://self");
        assert!(body.contains("#EXT-X-MEDIA:TYPE=AUDIO"));
        assert!(body.contains("video_720p/index.txt"));
        assert!(body.contains("audio-0/index.txt"));
        assert!(body.contains("RESOLUTION=1280x720"));
    }

    #[test]
    fn media_rewrites_segments_and_honors_keep_window() {
        let p = presentation();
        let stream = p.streams.iter().find(|s| s.kind == "video").unwrap();
        let rendered = render_media(
            &p,
            stream,
            MediaOptions {
                gid: "g",
                cid: "c",
                self_base: "http://self",
                keep_seconds: 4,
                pool_size: 1,
                formats: &["webp".to_string()],
                track_index: 0,
                start_offset: 0.0,
            },
        );
        assert!(rendered
            .body
            .contains("#EXT-X-MAP:URI=\"http://self/g/c/video_720p/init.webp\""));
        assert!(rendered.body.contains("#EXT-X-ENDLIST"));
        // 4 秒窗口只保留最后两段。
        assert_eq!(rendered.entries.len(), 1 + 2);
    }

    #[test]
    fn playlist_min_keeps_target_window() {
        let p = parse_presentation(
            r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic">
  <Period>
    <AdaptationSet contentType="video">
      <SegmentTemplate timescale="1000" initialization="init.mp4" media="s-$Number$.m4s" startNumber="1">
        <SegmentTimeline><S d="4000" r="9"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v0" width="640" height="360"/>
    </AdaptationSet>
  </Period>
</MPD>"#,
            "http://origin.example/live.mpd",
        )
        .expect("MPD");
        let stream = &p.streams[0];
        let rendered = render_media(
            &p,
            stream,
            MediaOptions {
                gid: "g",
                cid: "c",
                self_base: "",
                keep_seconds: 30,
                pool_size: 1,
                formats: &["jpg".to_string()],
                track_index: 0,
                start_offset: 0.0,
            },
        );
        assert_eq!(rendered.segment_count, 8);
        assert_eq!(rendered.body.matches("#EXTINF:").count(), 8);
        assert!(rendered.body.contains("#EXT-X-MEDIA-SEQUENCE:3"));
    }

    #[test]
    fn pool_rotates_audio_urls() {
        let urls = vec![
            "http://a/1".to_string(),
            "http://b/1".to_string(),
            "http://c/1".to_string(),
        ];
        assert_eq!(choose_urls(&urls, "video", 1, 3)[0], "http://a/1");
        assert_eq!(choose_urls(&urls, "audio", 0, 3)[0], "http://b/1");
        assert_eq!(choose_urls(&urls, "audio", 1, 3)[0], "http://c/1");
    }

    #[test]
    fn live_media_has_no_endlist() {
        let p = parse_presentation(
            r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" timeShiftBufferDepth="PT4S">
  <Period>
    <AdaptationSet contentType="video">
      <SegmentTemplate timescale="1000" media="s-$Number$.m4s" startNumber="1">
        <SegmentTimeline><S d="2000" r="-1"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v0" width="640" height="360"/>
    </AdaptationSet>
  </Period>
</MPD>"#,
            "http://origin.example/live.mpd",
        )
        .expect("MPD");
        let stream = &p.streams[0];
        let rendered = render_media(
            &p,
            stream,
            MediaOptions {
                gid: "g",
                cid: "c",
                self_base: "",
                keep_seconds: 0,
                pool_size: 1,
                formats: &["jpg".to_string()],
                track_index: 0,
                start_offset: 0.0,
            },
        );
        assert!(!rendered.body.contains("#EXT-X-ENDLIST"));
        assert_eq!(rendered.body.matches("#EXTINF:").count(), 2);
    }

    #[test]
    fn media_sequence_uses_mpd_number() {
        let p = presentation();
        let stream = p.streams.iter().find(|s| s.kind == "video").unwrap();
        assert_eq!(stream.segments[0].number, 1);
        let rendered = render_media(
            &p,
            stream,
            MediaOptions {
                gid: "g",
                cid: "c",
                self_base: "",
                keep_seconds: 0,
                pool_size: 1,
                formats: &["jpg".to_string()],
                track_index: 0,
                start_offset: 0.0,
            },
        );
        assert!(rendered.body.contains("#EXT-X-MEDIA-SEQUENCE:1"));
        assert!(rendered.body.contains("/g/c/video_720p/1_"));
    }

    #[test]
    fn group_keep_overrides_playlist_min() {
        assert_eq!(effective_keep(4, 30.0), 4);
        assert_eq!(effective_keep(0, 30.0), 30);
        assert_eq!(effective_keep(0, 0.0), 0);
    }

    #[test]
    fn live_media_uses_playlist_min_and_start_offset() {
        let p = presentation();
        let stream = p.streams.iter().find(|s| s.kind == "video").unwrap();
        let mut live = p.clone();
        live.is_live = true;
        let rendered = render_media(
            &live,
            stream,
            MediaOptions {
                gid: "g",
                cid: "c",
                self_base: "",
                keep_seconds: 0,
                pool_size: 1,
                formats: &["jpg".to_string()],
                track_index: 0,
                start_offset: 15.0,
            },
        );
        assert!(rendered.body.contains("#EXT-X-START:TIME-OFFSET=-15.000"));
        assert!(!rendered.body.contains("#EXT-X-ENDLIST"));
    }

    #[test]
    fn retry_delay_backs_off_and_caps() {
        let config = crate::config::model::CacheDashCfg::default();
        assert_eq!(retry_delay(&config, 0).as_millis(), 200);
        assert_eq!(retry_delay(&config, 1).as_millis(), 400);
        assert_eq!(retry_delay(&config, 8).as_millis(), 2_000);
        let mut config = config;
        config.min_request_interval_ms = 500;
        assert_eq!(retry_delay(&config, 0).as_millis(), 500);
    }

    #[test]
    fn segment_struct_is_public() {
        let segment = Segment {
            number: 1,
            time: 0,
            duration: 2.0,
            urls: vec!["http://a".into()],
        };
        assert_eq!(segment.duration, 2.0);
    }
}
