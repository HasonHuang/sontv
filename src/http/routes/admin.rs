//! 运维接口：缓存状态与清空、运行指标、运行日志（M4）。
//!
//! 契约见 [`docs/design/03-http-contract.md`](../../../docs/design/03-http-contract.md) §6.4。
//! 响应形状为本实现自定（目标未逐字段采证）。

use std::{path::Path, sync::Arc};

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::{
    http::{error::ApiError, ext::PanelAuth},
    state::AppState,
};

/// 清空缓存时清理的目录（`dash/` 保留，S-CACHE-06）。
const FLUSH_DIRS: [&str; 4] = ["m3u8", "seg_url", "seg_bytes", "sub_url"];

/// `GET /panel/api/cache/stats`。
pub async fn cache_stats(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
) -> Json<Value> {
    let mut entries = serde_json::Map::new();
    let mut bytes = 0u64;
    for name in ["m3u8", "seg_url", "seg_bytes", "sub_url", "dash"] {
        let (files, size) = dir_usage(&state.cache_dir.join(name));
        entries.insert(name.to_string(), json!(files));
        bytes += size;
    }
    Json(json!({
        "backend": "disk",
        "disk_path": state.cache_dir.display().to_string(),
        "entries": Value::Object(entries),
        "bytes": bytes,
        "playlist_cache_entries": state.playlist_cache_entries(),
    }))
}

/// `POST /panel/api/cache/flush`。
pub async fn cache_flush(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let mut removed = 0u64;
    for name in FLUSH_DIRS {
        removed += dir_usage(&state.cache_dir.join(name)).0;
        let dir = state.cache_dir.join(name);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(ApiError::internal(format!(
                    "清理 {} 失败: {err}",
                    dir.display()
                )))
            }
        }
        if let Err(err) = std::fs::create_dir_all(&dir) {
            return Err(ApiError::internal(format!(
                "重建 {} 失败: {err}",
                dir.display()
            )));
        }
    }
    // 同步丢弃内存索引与清单缓存，避免清空后仍命中旧数据。
    state.segments.clear();
    state.playlists.clear();
    state.sources.clear();
    tracing::info!(removed, "缓存已清空");
    Ok(Json(json!({ "ok": true, "removed": removed })))
}

/// `GET /panel/api/stats`。
pub async fn stats(PanelAuth(_): PanelAuth, State(state): State<Arc<AppState>>) -> Json<Value> {
    let snapshot = state.stats.snapshot();
    let config = state.snapshot();
    Json(json!({
        "version": state.version(),
        "uptime_secs": state.uptime_secs(),
        "requests": snapshot.requests,
        "playlists": snapshot.playlists,
        "segments": snapshot.segments,
        "segment_hits": snapshot.segment_hits,
        "upstream_failures": snapshot.upstream_failures,
        "online_window_secs": config.stats.online_window_secs,
        "groups": snapshot
            .groups
            .iter()
            .map(|(id, requests)| json!({ "id": id, "requests": requests }))
            .collect::<Vec<_>>(),
    }))
}

/// `GET /panel/api/logs?limit=n`。
pub async fn logs(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let limit = query
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(200)
        .min(500);
    let lines = state.logs.tail(limit);
    Json(json!({
        "total": state.logs.len(),
        "lines": lines,
    }))
}

/// 统计中间件：记总请求数，并按首个路径段记分组。
pub async fn count_request(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let gid = group_of(request.uri().path()).unwrap_or_default();
    state.stats.record_request(&gid);
    next.run(request).await
}

/// 从路径取分组 id。面板、静态资源与公开文件不算。
fn group_of(path: &str) -> Option<String> {
    let first = path.trim_start_matches('/').split('/').next()?;
    if first.is_empty() || first.contains('.') {
        return None;
    }
    if matches!(
        first,
        "panel" | "__assets" | "epg" | "logo" | "unban" | "api" | "health"
    ) {
        return None;
    }
    Some(first.to_string())
}

/// 统计目录内的文件数与总字节数。
fn dir_usage(dir: &Path) -> (u64, u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    let mut files = 0u64;
    let mut bytes = 0u64;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_file() {
            files += 1;
            bytes += meta.len();
        } else if meta.is_dir() {
            let (sub_files, sub_bytes) = dir_usage(&entry.path());
            files += sub_files;
            bytes += sub_bytes;
        }
    }
    (files, bytes)
}
