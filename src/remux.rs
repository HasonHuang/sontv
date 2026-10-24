//! Remux：把连续 TS 流转封装为本地 HLS 切片（纯 Rust，不调用 ffmpeg）。
//!
//! 证据：F-009 §1。目标行为：
//! - 冷启动 `503 retry-after: 2`（`remux warming up`）；
//! - 出口清单 `#EXT-X-VERSION:3`、`TARGETDURATION:5`，约 4 条切片；
//! - 切片按关键帧对齐，扩展名轮流伪装为图片。
//!
//! 本实现按 PTS 计时，按视频 PES 的 IDR/SPS（H264）或 IRAP/VPS/SPS（HEVC）切分。

use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::PathBuf,
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
use tracing::{info, warn};

use crate::{http::routes::stream::channel_headers, state::AppState};

/// 空闲多久后回收会话。
const IDLE: Duration = Duration::from_secs(90);
/// 伪装扩展名轮换。
const EXTS: &[&str] = &["png", "gif", "bmp", "jpg", "webp"];

/// 一条已完成的切片。
#[derive(Debug, Clone)]
struct Seg {
    name: String,
    duration: f64,
}

#[derive(Debug, Default)]
struct Inner {
    ready: bool,
    next_seq: u64,
    window: VecDeque<Seg>,
}

/// Remux 会话。
pub struct Session {
    gid: String,
    cid: String,
    dir: PathBuf,
    inner: Mutex<Inner>,
    last: Mutex<Instant>,
    stop: AtomicBool,
    target_secs: f64,
    window: usize,
    backoff_ms: Vec<u64>,
}

impl Session {
    fn touch(&self) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
    }

    /// 清单正文；未产出切片时返回 `None`。
    pub fn playlist(&self) -> Option<String> {
        self.touch();
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.window.is_empty() {
            return None;
        }
        let first = inner.next_seq.saturating_sub(inner.window.len() as u64);
        let mut body = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:{first}\n",
            self.target_secs as u64
        );
        for seg in &inner.window {
            body.push_str(&format!(
                "#EXTINF:{:.3},\n/{}/{}/{}\n",
                seg.duration, self.gid, self.cid, seg.name
            ));
        }
        Some(body)
    }

    /// 读取切片；名字不匹配返回 `None`。
    pub fn segment(&self, name: &str) -> Option<Vec<u8>> {
        if !valid_name(name) {
            return None;
        }
        self.touch();
        fs::read(self.dir.join(name)).ok()
    }

    /// 是否为本会话的切片名（无论文件是否存在）。
    pub fn owns(name: &str) -> bool {
        valid_name(name)
    }
}

fn valid_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    EXTS.contains(&ext)
        && stem.len() == 16
        && stem.starts_with("seg_")
        && stem[4..].bytes().all(|byte| byte.is_ascii_digit())
}

fn content_type(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        _ => "image/jpeg",
    }
}

/// 会话管理器。
#[derive(Default)]
pub struct Manager {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    janitor: AtomicBool,
}

impl Manager {
    /// 取得或创建会话。
    pub fn ensure(
        self: &Arc<Self>,
        state: &Arc<AppState>,
        gid: &str,
        cid: &str,
        url: &str,
    ) -> Arc<Session> {
        let key = format!("{gid}/{cid}");
        if let Some(session) = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .cloned()
        {
            session.touch();
            return session;
        }
        let dir = state.cache_dir.join("remux").join(gid).join(cid);
        let _ = fs::create_dir_all(&dir);
        let remux = state.snapshot().cache.remux.clone();
        let target_secs = remux.target_duration_secs.max(1.0);
        let window = (remux.window_segments as usize)
            .saturating_sub(remux.publish_delay_segments as usize)
            .max(1);
        let session = Arc::new(Session {
            gid: gid.to_string(),
            cid: cid.to_string(),
            dir,
            inner: Mutex::new(Inner::default()),
            last: Mutex::new(Instant::now()),
            stop: AtomicBool::new(false),
            target_secs,
            window,
            backoff_ms: if remux.reconnect_backoff_ms.is_empty() {
                vec![1_000]
            } else {
                remux.reconnect_backoff_ms.clone()
            },
        });
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, session.clone());
        info!(gid, cid, "remux 会话开始");
        spawn_reader(session.clone(), state.clone(), url.to_string());
        self.ensure_janitor();
        session
    }

    /// 交给请求路径：返回 `Some` 表示已处理。
    pub fn serve(&self, gid: &str, cid: &str, name: &str) -> Option<Response> {
        if !Session::owns(name) {
            return None;
        }
        let session = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&format!("{gid}/{cid}"))
            .cloned();
        let Some(session) = session else {
            return Some((StatusCode::NOT_FOUND, "Segment not found").into_response());
        };
        let Some(bytes) = session.segment(name) else {
            return Some((StatusCode::NOT_FOUND, "Segment not found").into_response());
        };
        let ext = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("png");
        let mut response = (StatusCode::OK, Body::from(bytes)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(content_type(ext)),
        );
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=30"),
        );
        Some(response)
    }

    fn ensure_janitor(self: &Arc<Self>) {
        if self.janitor.swap(true, Ordering::SeqCst) {
            return;
        }
        let manager = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let reaped: Vec<(String, Arc<Session>)> = {
                    let mut sessions = manager.sessions.lock().unwrap_or_else(|e| e.into_inner());
                    let keys: Vec<String> = sessions
                        .iter()
                        .filter(|(_, session)| session.idle_for() > IDLE)
                        .map(|(key, _)| key.clone())
                        .collect();
                    keys.into_iter()
                        .filter_map(|key| sessions.remove(&key).map(|session| (key, session)))
                        .collect()
                };
                for (key, session) in reaped {
                    session.stop.store(true, Ordering::SeqCst);
                    let _ = fs::remove_dir_all(&session.dir);
                    info!(key, "remux 会话回收");
                }
            }
        });
    }
}

/// 写入一条切片。
fn cut(session: &Session, data: Vec<u8>, duration: f64) {
    if data.is_empty() {
        return;
    }
    let _ = fs::create_dir_all(&session.dir);
    let mut inner = session.inner.lock().unwrap_or_else(|e| e.into_inner());
    let seq = inner.next_seq;
    let ext = EXTS[(seq as usize) % EXTS.len()];
    let name = format!("seg_{seq:012}.{ext}");
    let path = session.dir.join(&name);
    if let Err(error) = fs::write(&path, &data) {
        warn!(path = %path.display(), %error, "写 remux 切片失败");
        return;
    }
    inner.next_seq += 1;
    inner.ready = true;
    inner.window.push_back(Seg {
        name: name.clone(),
        duration: duration.max(0.1),
    });
    while inner.window.len() > session.window {
        if let Some(old) = inner.window.pop_front() {
            let _ = fs::remove_file(session.dir.join(old.name));
        }
    }
    // 兜底：删除不合法残留。
    let keep: Vec<String> = inner.window.iter().map(|seg| seg.name.clone()).collect();
    drop(inner);
    if let Ok(read) = fs::read_dir(&session.dir) {
        for item in read.flatten() {
            let name = item.file_name().to_string_lossy().to_string();
            if valid_name(&name) && !keep.contains(&name) {
                let _ = fs::remove_file(item.path());
            }
        }
    }
}

/// 起上游读取任务。
fn spawn_reader(session: Arc<Session>, state: Arc<AppState>, url: String) {
    tokio::spawn(async move {
        let mut parser = Parser::new();
        let mut attempt = 0usize;
        loop {
            if session.stop.load(Ordering::SeqCst) || session.idle_for() > IDLE {
                break;
            }
            let mut request = state.http_for(&session.gid, &session.cid).get(&url);
            for (name, value) in channel_headers(&state, &session.gid, &session.cid) {
                request = request.header(name, value);
            }
            let mut response = match request.send().await {
                Ok(response) if response.status().is_success() => response,
                Ok(response) => {
                    warn!(url = %url, status = %response.status(), "remux 上游状态异常");
                    let delay = session
                        .backoff_ms
                        .get(attempt.min(session.backoff_ms.len() - 1))
                        .copied()
                        .unwrap_or(1_000);
                    attempt = attempt.saturating_add(1);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    continue;
                }
                Err(error) => {
                    warn!(url = %url, %error, "remux 上游连接失败");
                    let delay = session
                        .backoff_ms
                        .get(attempt.min(session.backoff_ms.len() - 1))
                        .copied()
                        .unwrap_or(1_000);
                    attempt = attempt.saturating_add(1);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    continue;
                }
            };
            attempt = 0;
            loop {
                if session.stop.load(Ordering::SeqCst) {
                    return;
                }
                match response.chunk().await {
                    Ok(Some(chunk)) => parser.push(&chunk, &session),
                    Ok(None) => break,
                    Err(error) => {
                        warn!(url = %url, %error, "remux 上游读取失败");
                        break;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
}

/// TS 解析与切片状态机。
struct Parser {
    buffer: Vec<u8>,
    pmt_pid: Option<u16>,
    video_pid: Option<u16>,
    video_type: Option<u8>,
    current: Vec<u8>,
    started: bool,
    start_pts: Option<u64>,
    pending: Option<u64>,
    last_pts: Option<u64>,
    last_pat: Option<[u8; 188]>,
    last_pmt: Option<[u8; 188]>,
}

impl Parser {
    fn new() -> Self {
        Self {
            buffer: Vec::new(),
            pmt_pid: None,
            video_pid: None,
            video_type: None,
            current: Vec::new(),
            started: false,
            start_pts: None,
            pending: None,
            last_pts: None,
            last_pat: None,
            last_pmt: None,
        }
    }

    /// 新切片前缀：重复最近一组 PAT/PMT。
    ///
    /// 目标在关键帧处切分，但前后切片都包含同一组 PAT/PMT；实测第二片
    /// 从关键帧前的 PAT 包开始。返回空表示尚未看到 PAT/PMT。
    fn boundary_prefix(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(376);
        if let Some(packet) = self.last_pat {
            out.extend_from_slice(&packet);
        }
        if let Some(packet) = self.last_pmt {
            out.extend_from_slice(&packet);
        }
        out
    }

    fn push(&mut self, chunk: &[u8], session: &Session) {
        self.buffer.extend_from_slice(chunk);
        let mut offset = 0;
        while self.buffer.len() - offset >= 188 {
            if self.buffer[offset] != 0x47 {
                offset += 1;
                continue;
            }
            let packet: [u8; 188] = self.buffer[offset..offset + 188].try_into().unwrap();
            offset += 188;
            self.packet(&packet, session);
        }
        if offset > 0 {
            self.buffer.drain(..offset);
        }
        if self.buffer.len() > 4096 {
            // 长时间无同步字节：只保留尾部，避免无界增长。
            let keep = self.buffer.len() - 188;
            self.buffer.drain(..keep);
        }
    }

    fn packet(&mut self, packet: &[u8], session: &Session) {
        let pid = (((packet[1] & 0x1f) as u16) << 8) | packet[2] as u16;
        let pusi = packet[1] & 0x40 != 0;
        let control = (packet[3] >> 4) & 0x3;
        if control == 0 {
            return;
        }
        let mut payload_start = 4;
        if control == 2 || control == 3 {
            payload_start += 1 + packet[4] as usize;
        }
        let has_payload = control != 2 && payload_start < 188;
        let payload = if has_payload {
            &packet[payload_start..]
        } else {
            &[][..]
        };

        if !self.started {
            self.started = true;
        }

        if has_payload && pid == 0 && pusi {
            self.parse_pat(payload);
            self.last_pat = Some(packet.try_into().unwrap_or([0; 188]));
        } else if has_payload && Some(pid) == self.pmt_pid && pusi {
            self.parse_pmt(payload);
            self.last_pmt = Some(packet.try_into().unwrap_or([0; 188]));
        }

        if has_payload && pusi && Some(pid) == self.video_pid {
            if let Some((pts, key)) = video_pes(payload, self.video_type == Some(0x24)) {
                let pts = pts.filter(|value| *value > 0);
                if let Some(pts) = pts {
                    if self.last_pts.map(|last| pts > last).unwrap_or(true) {
                        self.last_pts = Some(pts);
                    }
                    self.pending = Some(pts);
                }
                if key {
                    let boundary = self.pending;
                    let elapsed = match (self.start_pts, boundary) {
                        (Some(start), Some(now)) if now >= start => (now - start) as f64 / 90_000.0,
                        _ => 0.0,
                    };
                    if boundary.is_some()
                        && !self.current.is_empty()
                        && elapsed >= session.target_secs
                    {
                        let data = std::mem::take(&mut self.current);
                        cut(session, data, elapsed);
                        self.start_pts = boundary;
                        self.current = self.boundary_prefix();
                    } else if self.start_pts.is_none() {
                        self.start_pts = boundary;
                    }
                }
            }
        }

        if self.started {
            self.current.extend_from_slice(packet);
        }
    }

    fn parse_pat(&mut self, payload: &[u8]) {
        if payload.len() < 9 {
            return;
        }
        let pointer = payload[0] as usize;
        let section = &payload[1 + pointer..];
        if section.len() < 8 || section[0] != 0x00 {
            return;
        }
        let length = (((section[1] & 0x0f) as usize) << 8) | section[2] as usize;
        if length + 3 > section.len() {
            return;
        }
        let mut index = 8;
        while index + 4 <= 3 + length - 4 {
            let program = u16::from_be_bytes([section[index], section[index + 1]]);
            let pid = u16::from_be_bytes([section[index + 2], section[index + 3]]) & 0x1fff;
            if program != 0 {
                self.pmt_pid = Some(pid);
                return;
            }
            index += 4;
        }
    }

    fn parse_pmt(&mut self, payload: &[u8]) {
        if payload.len() < 12 {
            return;
        }
        let pointer = payload[0] as usize;
        let section = &payload[1 + pointer..];
        if section.len() < 12 || section[0] != 0x02 {
            return;
        }
        let length = (((section[1] & 0x0f) as usize) << 8) | section[2] as usize;
        if length + 3 > section.len() {
            return;
        }
        let info_len = (((section[10] & 0x0f) as usize) << 8) | section[11] as usize;
        let mut index = 12 + info_len;
        let end = 3 + length - 4;
        while index + 5 <= end {
            let stream_type = section[index];
            let pid = u16::from_be_bytes([section[index + 1], section[index + 2]]) & 0x1fff;
            let es_len =
                (((section[index + 3] & 0x0f) as usize) << 8) | section[index + 4] as usize;
            if matches!(stream_type, 0x01 | 0x02 | 0x1b | 0x24) {
                self.video_pid = Some(pid);
                self.video_type = Some(stream_type);
                return;
            }
            index += 5 + es_len;
        }
    }
}

/// 解析视频 PES：返回 `(PTS, 是否关键帧)`。
fn video_pes(payload: &[u8], hevc: bool) -> Option<(Option<u64>, bool)> {
    if payload.len() < 9 || payload[0..3] != [0x00, 0x00, 0x01] {
        return None;
    }
    let flags = (payload[7] >> 6) & 0x3;
    let header_len = payload[8] as usize;
    let pts = if flags >= 2 && payload.len() >= 14 {
        Some(
            (((payload[9] >> 1) & 0x07) as u64) << 30
                | (payload[10] as u64) << 22
                | ((payload[11] >> 1) as u64) << 15
                | (payload[12] as u64) << 7
                | ((payload[13] >> 1) as u64),
        )
    } else {
        None
    };
    let start = 9 + header_len;
    if payload.len() <= start + 4 {
        return Some((pts, false));
    }
    // 扫描前 512 字节内的 Annex B NAL：AUD/SEI 可能排在 IDR 之前。
    let es = &payload[start..payload.len().min(start + 512)];
    let mut key = false;
    let mut index = 0;
    while index + 4 <= es.len() {
        if es[index] == 0 && es[index + 1] == 0 && es[index + 2] == 1 {
            let nal = es[index + 3];
            let is_key = if hevc {
                matches!((nal >> 1) & 0x3f, 16 | 17 | 18 | 19 | 20 | 21 | 32 | 33)
            } else {
                matches!(nal & 0x1f, 5 | 7)
            };
            if is_key {
                key = true;
                break;
            }
            index += 4;
        } else {
            index += 1;
        }
    }
    Some((pts, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            gid: "g".into(),
            cid: "c".into(),
            dir: std::env::temp_dir().join(format!("remux-{}", uuid::Uuid::new_v4())),
            inner: Mutex::new(Inner::default()),
            last: Mutex::new(Instant::now()),
            stop: AtomicBool::new(false),
            target_secs: 5.0,
            window: 4,
            backoff_ms: vec![200, 1_000, 2_000],
        }
    }

    #[test]
    fn names_and_content_types() {
        assert!(valid_name("seg_000000000000.png"));
        assert!(!valid_name("seg_1.png"));
        assert!(!valid_name("seg_000000000000.exe"));
        assert_eq!(content_type("gif"), "image/gif");
        assert_eq!(content_type("webp"), "image/webp");
    }

    #[test]
    fn playlist_uses_relative_paths_and_sequence() {
        let session = session();
        let data = vec![0x47u8; 188];
        for _ in 0..6 {
            cut(&session, data.clone(), 5.0);
        }
        let body = session.playlist().expect("清单");
        assert!(body.contains("#EXT-X-TARGETDURATION:5"));
        assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:2"));
        assert!(body.contains("/g/c/seg_000000000002.bmp"));
        // 窗口只保留 4 条。
        assert_eq!(body.matches("#EXTINF").count(), 4);
        let _ = fs::remove_dir_all(&session.dir);
    }

    #[test]
    fn parser_cuts_on_keyframes() {
        fn packet(pid: u16, pusi: bool, payload: &[u8]) -> Vec<u8> {
            let mut out = vec![
                0x47,
                if pusi { 0x40 } else { 0 } | ((pid >> 8) as u8 & 0x1f),
                pid as u8,
                0x10,
            ];
            out.extend_from_slice(payload);
            out.resize(188, 0xff);
            out
        }
        fn pts_bytes(pts: u64) -> [u8; 5] {
            [
                0x21 | (((pts >> 30) as u8 & 0x07) << 1),
                (pts >> 22) as u8,
                0x01 | ((pts >> 14) as u8 & 0xfe),
                (pts >> 7) as u8,
                0x01 | ((pts << 1) as u8 & 0xfe),
            ]
        }
        let session = session();
        std::fs::create_dir_all(&session.dir).expect("建目录");
        let mut parser = Parser::new();
        // 源流开头的 SDT 应进入首片；后续切片从 PAT/PMT 开始。
        parser.push(
            &packet(0x11, true, &[0x42, 0xF0, 0x25, 0x00, 0x01]),
            &session,
        );
        let pat = [
            0x00u8, 0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xE1, 0x00, 0, 0,
            0, 0,
        ];
        parser.push(&packet(0, true, &pat), &session);
        let pmt = [
            0x00u8, 0x02, 0xB0, 0x12, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xE1, 0x01, 0xF0, 0x00, 0x1B,
            0xE1, 0x01, 0xF0, 0x00, 0, 0, 0, 0,
        ];
        parser.push(&packet(0x100, true, &pmt), &session);
        for index in 0..3u64 {
            let pts = (index + 1) * 90_000 * 5;
            let mut pes = vec![0x00, 0x00, 0x01, 0xE0, 0x00, 0x00, 0x80, 0x80, 0x05];
            pes.extend_from_slice(&pts_bytes(pts));
            pes.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x65]);
            pes.extend(std::iter::repeat(0x88).take(40));
            parser.push(&packet(0x101, true, &pes), &session);
        }
        assert!(session.playlist().is_some(), "应有切片");
        let pid = |packet: &[u8]| (((packet[1] & 0x1f) as u16) << 8) | packet[2] as u16;
        let first = fs::read(session.dir.join("seg_000000000000.png")).expect("首片");
        assert_eq!(pid(&first[..188]), 0x11, "首片以源流 SDT 开头");
        let second = fs::read(session.dir.join("seg_000000000001.gif")).expect("第二片");
        assert_eq!(pid(&second[..188]), 0x0, "第二片先复制 PAT");
        assert_eq!(pid(&second[188..376]), 0x100, "第二片再复制 PMT");
        let _ = fs::remove_dir_all(&session.dir);
    }

    #[test]
    fn video_pes_reads_pts_and_idr() {
        let mut pes = vec![0, 0, 1, 0xe0, 0, 0, 0x80, 0x80, 5];
        let pts: u64 = 90_000;
        pes.extend_from_slice(&[
            0x21 | (((pts >> 30) as u8 & 0x07) << 1),
            (pts >> 22) as u8,
            0x01 | (((pts >> 15) as u8 & 0x7f) << 1),
            (pts >> 7) as u8,
            0x01 | (((pts as u8 & 0x7f) as u8) << 1),
        ]);
        pes.extend_from_slice(&[0, 0, 0, 1, 0x65, 0x88]);
        let (parsed, key) = video_pes(&pes, false).expect("PES");
        assert!(key);
        assert_eq!(parsed, Some(90_000));
    }
}
