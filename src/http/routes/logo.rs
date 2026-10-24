//! 台标库接口：公开取图与列表，面板上传与删除。
//!
//! 契约见 [`docs/design/03-http-contract.md`](../../../docs/design/03-http-contract.md) §3。
//! 证据：F-011 §6、F-012。

use std::{path::Path as FsPath, sync::Arc};

use axum::{
    extract::{Path, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use crate::{
    http::{error::ApiError, ext::PanelAuth},
    logo,
    state::AppState,
};

/// 列表响应：`{allowed_exts, count, logos}`。
fn list_body(cache: &str) -> Value {
    let logos = logo::list(cache);
    json!({
        "allowed_exts": logo::ALLOWED_EXTS,
        "count": logos.len(),
        "logos": logos,
    })
}

/// `GET /logo/api/list`——公开台标列表。
pub async fn public_list(State(state): State<Arc<AppState>>) -> Json<Value> {
    let config = state.snapshot();
    Json(list_body(&config.cache.disk_path))
}

/// `GET /logo/<file>`——公开取图，`public, max-age=86400`。
pub async fn file(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let Some(file) = logo::sanitize(&name) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let ext = file
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    if !logo::ALLOWED_EXTS.contains(&ext.as_str()) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let config = state.snapshot();
    let path = logo::dir(&config.cache.disk_path).join(&file);
    let Ok(body) = tokio::fs::read(&path).await else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let mime = mime_guess::from_path(FsPath::new(&file))
        .first_or_octet_stream()
        .to_string();
    let mut response = ([(header::CONTENT_TYPE, mime)], body).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    response
}

/// `GET /panel/api/logo/list`。
pub async fn panel_list(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
) -> Json<Value> {
    let config = state.snapshot();
    Json(list_body(&config.cache.disk_path))
}

/// `POST /panel/api/logo/upload`——`{"items":[{"name","ext","data":"data:image/…;base64,…"}]}`。
pub async fn upload(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let items = body
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad_request("items 必须是数组"))?;
    let cache = state.snapshot().cache.disk_path.clone();

    let mut added: Vec<Value> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    for item in items {
        let name = item.get("name").and_then(Value::as_str).unwrap_or("");
        let mut ext = item
            .get("ext")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if ext.is_empty() {
            ext = name
                .rsplit_once('.')
                .map(|(_, ext)| ext.to_string())
                .unwrap_or_default();
        }
        let data = item.get("data").and_then(Value::as_str).unwrap_or("");
        let bytes = match logo::decode_data_url(data) {
            Ok((_, bytes)) => bytes,
            Err(error) => {
                failed.push(json!({ "name": name, "error": format!("{error:#}") }));
                continue;
            }
        };
        let base = name.rsplit_once('.').map(|(base, _)| base).unwrap_or(name);
        match logo::write(&cache, base, &ext, &bytes) {
            Ok(entry) => added.push(json!(entry)),
            Err(error) => failed.push(json!({ "name": name, "error": format!("{error:#}") })),
        }
    }
    Ok(Json(
        json!({ "ok": true, "added": added, "failed": failed }),
    ))
}

/// `POST /panel/api/logo/delete`——`{"file"}`。
pub async fn delete(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let file = body
        .get("file")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if file.trim().is_empty() {
        return Err(ApiError::bad_request("file 不能为空"));
    }
    let cache = state.snapshot().cache.disk_path.clone();
    match logo::remove(&cache, &file) {
        Ok(true) => Ok(Json(json!({ "ok": true, "removed": 1 }))),
        Ok(false) => Err(ApiError::not_found("文件不存在")),
        Err(error) => Err(ApiError::internal(format!("{error:#}"))),
    }
}

/// `POST /panel/api/logo/rename`——`{"file","name"}`；保留原扩展名。
pub async fn rename(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let file = body
        .get("file")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if file.trim().is_empty() || name.trim().is_empty() {
        return Err(ApiError::bad_request("file 与 name 不能为空"));
    }
    let cache = state.snapshot().cache.disk_path.clone();
    let safe = logo::sanitize(&file).ok_or_else(|| ApiError::bad_request("文件名不合法"))?;
    if !logo::dir(&cache).join(&safe).is_file() {
        return Err(ApiError::not_found("文件不存在"));
    }
    match logo::rename(&cache, &safe, &name) {
        Ok(entry) => Ok(Json(json!({ "ok": true, "logo": entry }))),
        Err(error) => Err(ApiError::bad_request(format!("{error:#}"))),
    }
}

/// 频道当前的台标值（`logo` 优先，兼容导入的 `tvg_logo`）。
fn channel_logo(channel: &crate::config::model::Channel) -> Option<String> {
    ["logo", "tvg_logo"]
        .iter()
        .find_map(|key| channel.extra.get(*key).and_then(|value| value.as_str()))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// `GET /panel/api/logo/match-report`——缺台标、坏引用与未使用。
pub async fn match_report(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
) -> Json<Value> {
    let config = state.snapshot();
    let entries = logo::list(&config.cache.disk_path);
    let files: std::collections::HashSet<&str> =
        entries.iter().map(|entry| entry.file.as_str()).collect();
    let mut missing = Vec::new();
    let mut broken = Vec::new();
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut total = 0usize;

    for group in &config.groups {
        for channel in &group.channels {
            total += 1;
            let Some(value) = channel_logo(channel) else {
                missing.push(json!({
                    "gid": group.id,
                    "cid": channel.id,
                    "name": channel.name,
                }));
                continue;
            };
            if let Some(file) = value.strip_prefix("/logo/") {
                if files.contains(file) {
                    used.insert(file.to_string());
                } else {
                    broken.push(json!({
                        "gid": group.id,
                        "cid": channel.id,
                        "name": channel.name,
                        "logo": value,
                    }));
                }
            }
        }
    }
    let mut unused: Vec<String> = entries
        .iter()
        .filter(|entry| !used.contains(&entry.file))
        .map(|entry| entry.file.clone())
        .collect();
    unused.sort();

    Json(json!({
        "channels": total,
        "missing": missing.len(),
        "broken": broken.len(),
        "unused": unused.len(),
        "missing_channels": missing,
        "broken_logos": broken,
        "unused_logos": unused,
    }))
}

/// `POST /panel/api/logo/apply`——`{"gid","cid","file"}`；写入频道 `logo`。
pub async fn apply(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let gid = body
        .get("gid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let cid = body
        .get("cid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let file = body
        .get("file")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if gid.is_empty() || cid.is_empty() || file.is_empty() {
        return Err(ApiError::bad_request("gid、cid 与 file 不能为空"));
    }
    let cache = state.snapshot().cache.disk_path.clone();
    let safe = logo::sanitize(&file).ok_or_else(|| ApiError::bad_request("文件名不合法"))?;
    if !logo::dir(&cache).join(&safe).is_file() {
        return Err(ApiError::not_found("文件不存在"));
    }
    state
        .mutate(|config| {
            let group = config
                .groups
                .iter_mut()
                .find(|group| group.id == gid)
                .ok_or_else(|| anyhow::anyhow!("Group not found"))?;
            let channel = group
                .channels
                .iter_mut()
                .find(|channel| channel.id == cid)
                .ok_or_else(|| anyhow::anyhow!("Channel not found"))?;
            channel
                .extra
                .insert("logo".into(), toml::Value::String(format!("/logo/{safe}")));
            Ok(())
        })
        .map_err(|error| {
            let message = format!("{error:#}");
            if message.contains("not found") {
                ApiError::not_found(message)
            } else {
                ApiError::internal(message)
            }
        })?;
    Ok(Json(json!({
        "ok": true,
        "gid": gid,
        "cid": cid,
        "logo": format!("/logo/{safe}"),
    })))
}

/// `POST /panel/api/logo/suggest`——按名称相似度为缺台标频道推荐文件。
pub async fn suggest(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let threshold = body
        .get("threshold")
        .and_then(Value::as_f64)
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let limit = body
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(3)
        .clamp(1, 10) as usize;
    let config = state.snapshot();
    let entries = logo::list(&config.cache.disk_path);

    let mut channels = Vec::new();
    for group in &config.groups {
        for channel in &group.channels {
            if channel_logo(channel).is_some() || entries.is_empty() {
                continue;
            }
            let mut scored: Vec<(f64, &str)> = entries
                .iter()
                .filter_map(|entry| {
                    let score = logo::similarity(&channel.name, &entry.name);
                    if score >= threshold && score > 0.0 {
                        Some((score, entry.file.as_str()))
                    } else {
                        None
                    }
                })
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            channels.push(json!({
                "gid": group.id,
                "cid": channel.id,
                "name": channel.name,
                "suggest": scored
                    .into_iter()
                    .take(limit)
                    .map(|(score, file)| json!({ "file": file, "score": (score * 100.0).round() / 100.0 }))
                    .collect::<Vec<_>>(),
            }));
        }
    }
    Json(json!({
        "threshold": threshold,
        "count": channels.len(),
        "channels": channels,
    }))
}

/// `POST /panel/api/logo/restore`——用 `.orig/` 备份还原静态原图。
pub async fn restore(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let file = body
        .get("file")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if file.is_empty() {
        return Err(ApiError::bad_request("file 不能为空"));
    }
    let cache = state.snapshot().cache.disk_path.clone();
    match logo::restore(&cache, file) {
        Ok(true) => Ok(Json(json!({ "ok": true }))),
        Ok(false) => Err(ApiError::bad_request("没有找到原图备份")),
        Err(error) => Err(ApiError::internal(format!("{error:#}"))),
    }
}

/// `POST /panel/api/logo/animate`——生成动画 WebP 动效并覆盖原图。
pub async fn animate(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let file = body
        .get("file")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if file.is_empty() {
        return Err(ApiError::bad_request("file 不能为空"));
    }
    let effect = body
        .get("effect")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if effect.is_empty() {
        return Err(ApiError::bad_request("effect 不能为空"));
    }
    let quality = match body.get("quality") {
        Some(Value::String(value)) => value.as_str(),
        Some(_) => return Err(ApiError::bad_request("quality 必须是字符串")),
        None => "",
    };
    let config = state.snapshot();
    let max_size = extra_u64(&config, "max_size", 512).clamp(16, 2048) as u32;
    let frame_ms = extra_u64(&config, "anim_frame_ms", 80).clamp(20, 1_000);
    let default_quality = extra_u64(&config, "quality", 80).clamp(1, 100) as u8;
    let report = crate::logo_anim::animate(
        &config.cache.disk_path,
        file,
        effect,
        quality,
        max_size,
        frame_ms,
        default_quality,
    )
    .map_err(|error| {
        let message = format!("{error:#}");
        if message.contains("已经是动画")
            || message.contains("文件不存在")
            || message.contains("未知动效")
            || message.contains("quality")
            || message.contains("扩展名")
            || message.contains("文件名不合法")
        {
            ApiError::bad_request(message)
        } else {
            ApiError::internal(message)
        }
    })?;
    Ok(Json(json!({
        "ok": true,
        "frames": report.frames,
        "width": report.width,
        "height": report.height,
        "total_ms": report.total_ms,
        "bytes_before": report.bytes_before,
        "bytes_after": report.bytes_after,
        "dirty_ratio": report.dirty_ratio,
    })))
}

fn extra_u64(config: &crate::config::Config, key: &str, default: u64) -> u64 {
    config
        .extra
        .get("logo")
        .and_then(toml::Value::as_table)
        .and_then(|logo| logo.get(key))
        .and_then(toml::Value::as_integer)
        .map(|value| value.max(0) as u64)
        .unwrap_or(default)
}
