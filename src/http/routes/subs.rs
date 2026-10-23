//! 订阅源预检端点：`/subs/status`、`/subs/groups`、`/subs/preview`。
//!
//! 证据：S-GAP-07。响应形状自定，字段保留目标的 `trace` 结构。
//! 这组端点公开；只接受 `http`/`https` 源地址。

use std::sync::Arc;

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::{
    http::{error::ApiError, ext::PanelAuth},
    m3u,
    state::AppState,
    subs::{self, PreviewSource},
};

/// `GET /subs/status`——列出聚合源最近一次抓取状态。
pub async fn status(State(state): State<Arc<AppState>>) -> Json<Value> {
    let config = state.snapshot();
    let statuses = state
        .source_status
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let sources: Vec<Value> = config
        .playlist
        .sources
        .iter()
        .map(|source| {
            let status = statuses.get(&source.id).cloned().unwrap_or_default();
            json!({
                "id": source.id,
                "name": source.name,
                "enabled": source.enabled,
                "error": status.error,
                "channels": status.channels,
                "last_ok": status.last_ok,
                "last_try": status.last_try,
                "ttl": crate::subs::effective_ttl_secs(source.ttl_secs),
            })
        })
        .collect();
    Json(json!({ "sources": sources }))
}

/// `POST /subs/groups`——抓取源并列出分组。
pub async fn groups(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let url = body
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| ApiError::bad_request("缺少 url"))?;
    validate_url(url)?;
    let text = fetch(&state, url).await?;
    let entries = m3u::parse(&text);
    let total = entries.len();
    let groups: Vec<Value> = subs::group_counts(&entries)
        .into_iter()
        .map(|count| json!({ "name": count.name, "count": count.count }))
        .collect();
    Ok(Json(json!({ "groups": groups, "total": total })))
}

/// `POST /subs/preview`——按映射与过滤规则预检源。
pub async fn preview(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let input = body.get("source").cloned().unwrap_or_else(|| body.clone());
    let config = state.snapshot();

    // `id` 命中已配置源时，用配置补齐空字段。
    let configured = input
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| config.playlist.sources.iter().find(|item| item.id == id));

    let url = input
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .or_else(|| configured.map(|source| source.url.trim()))
        .filter(|url| !url.is_empty())
        .ok_or_else(|| ApiError::bad_request("缺少 url"))?;
    validate_url(url)?;

    let mut source = configured
        .map(|configured| PreviewSource {
            group_map: configured
                .group_map
                .iter()
                .map(|(from, to)| (from.clone(), to.clone()))
                .collect(),
            group_mode: configured.group_mode.clone(),
            merge_group: configured.merge_group.clone(),
            unlisted_groups: configured.unlisted_groups.clone(),
            include: configured.include.clone(),
            exclude: configured.exclude.clone(),
        })
        .unwrap_or_else(|| PreviewSource {
            group_mode: "keep".into(),
            unlisted_groups: "drop".into(),
            ..PreviewSource::default()
        });

    if let Some(value) = input.get("unlisted_groups").and_then(Value::as_str) {
        source.unlisted_groups = value.to_string();
    }
    if let Some(value) = input.get("group_mode").and_then(Value::as_str) {
        source.group_mode = value.to_string();
    }
    if let Some(value) = input.get("merge_group").and_then(Value::as_str) {
        source.merge_group = value.to_string();
    }
    if input.get("include").is_some() {
        source.include = strings(&input, "include");
    }
    if input.get("exclude").is_some() {
        source.exclude = strings(&input, "exclude");
    }
    if let Some(map) = input.get("group_map") {
        let map = map
            .as_object()
            .ok_or_else(|| ApiError::bad_request("source.group_map 必须是对象"))?;
        source.group_map.clear();
        for (from, value) in map {
            let to = value
                .as_str()
                .ok_or_else(|| ApiError::bad_request("source.group_map 的值必须是字符串"))?;
            source.group_map.insert(from.clone(), to.to_string());
        }
    }

    let text = fetch(&state, url).await?;
    let entries = m3u::parse(&text);
    let result = subs::preview(
        &entries,
        &source,
        &config.playlist.global_exclude,
        SAMPLE_LIMIT,
    );

    let sample: Vec<Value> = result
        .sample
        .iter()
        .map(|item| {
            json!({
                "name": item.name,
                "group": item.group,
                "tvg_id": item.tvg_id,
            })
        })
        .collect();
    let out_groups: Vec<Value> = result
        .out_groups
        .iter()
        .map(|count| json!({ "name": count.name, "count": count.count }))
        .collect();

    Ok(Json(json!({
        "kept": result.kept,
        "sample": sample,
        "trace": {
            "total": result.trace.total,
            "after_group": result.trace.after_group,
            "after_global_exclude": result.trace.after_global_exclude,
            "after_include": result.trace.after_include,
            "after_exclude": result.trace.after_exclude,
            "dropped_groups": result.dropped_groups,
            "out_groups": out_groups,
        },
    })))
}

/// `GET /panel/api/subs/status`——目标面板路径，需要会话。
pub async fn panel_status(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
) -> Json<Value> {
    status(State(state)).await
}

/// `POST /panel/api/subs/groups`——目标面板路径，需要会话。
pub async fn panel_groups(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    groups(State(state), Json(body)).await
}

/// `POST /panel/api/subs/preview`——目标面板路径，需要会话。
pub async fn panel_preview(
    PanelAuth(_): PanelAuth,
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    preview(State(state), Json(body)).await
}

/// 抽样条数上限。
const SAMPLE_LIMIT: usize = 5;

fn strings(input: &Value, key: &str) -> Vec<String> {
    input
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn validate_url(url: &str) -> Result<(), ApiError> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(())
    } else {
        Err(ApiError::bad_request("url 必须是 http(s) 地址"))
    }
}

async fn fetch(state: &Arc<AppState>, url: &str) -> Result<String, ApiError> {
    let response = state
        .http
        .get(url)
        .send()
        .await
        .map_err(|error| ApiError::bad_gateway(format!("抓取失败: {error}")))?;
    if !response.status().is_success() {
        return Err(ApiError::bad_gateway(format!(
            "上游返回 {}",
            response.status()
        )));
    }
    response
        .text()
        .await
        .map_err(|error| ApiError::bad_gateway(format!("读取失败: {error}")))
}
