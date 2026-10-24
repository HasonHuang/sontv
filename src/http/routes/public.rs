//! 公开面：健康检查、首页跳转、自助解封、兜底。

use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use crate::{auth::unban::MONTHLY_LIMIT, http::ext::client_ip_with_peer, state::AppState};

/// `302 Found`。目标用 302 做占位跳转（S-DIS-15）。
fn found(url: &str) -> Response {
    let mut response = StatusCode::FOUND.into_response();
    match HeaderValue::from_str(url) {
        Ok(location) => {
            response.headers_mut().insert(header::LOCATION, location);
        }
        Err(_) => {
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        }
    }
    response
}

/// `GET /health` → `200` `ok`（`text/plain`，S-EP-10）。
pub async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/plain; charset=utf-8")],
        "ok",
    )
}

/// `GET /` → 跳首页（`server.home_url`）。
pub async fn home(State(state): State<Arc<AppState>>) -> Response {
    let cfg = state.snapshot();
    found(&cfg.server.home_url)
}

/// 兜底：面板接口返回 404 JSON；其余路径返回 404 正文。
///
/// 目标把 `/status` 一类单段未知路径判为 404（S-EP-08）。
/// 两段及以上的播放形状由 `/{gid}/{file}` 与通配路由处理。
pub async fn fallback(State(_): State<Arc<AppState>>, uri: Uri) -> Response {
    if uri.path().starts_with("/panel/api/") {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({ "error": "接口不存在" })),
        )
            .into_response();
    }
    (StatusCode::NOT_FOUND, "not found").into_response()
}

/// `GET /unban/whoami`——返回调用方地址、封禁状态与本月配额。
///
/// 目标在 IP 管控关闭时只返回 `enabled` 与 `ip`。
pub async fn whoami(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Json<Value> {
    let ip = client_ip_with_peer(&headers, Some(peer));
    if !state.snapshot().auth.ip_guard_enabled {
        return Json(json!({ "enabled": false, "ip": ip }));
    }
    let used = state.unban_quota.used(&ip);
    Json(json!({
        "banned": state.ip_guard.is_banned(&ip),
        "enabled": true,
        "ip": ip,
        "max_per_month": MONTHLY_LIMIT,
        "remaining": MONTHLY_LIMIT.saturating_sub(used),
        "used": used,
    }))
}

/// `POST /unban`——调用方自助解封。
///
/// 免登录（S-EP-07）。每个 IP 每月最多 3 次（S-AUTH-07）。
/// 配额用完时目标仍返回 `200`，正文 `ok=false`。
pub async fn unban(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Json<Value> {
    if !state.snapshot().auth.ip_guard_enabled {
        return Json(json!({ "msg": "IP 管控未启用", "ok": false }));
    }
    let ip = client_ip_with_peer(&headers, Some(peer));
    if !state.ip_guard.is_banned(&ip) {
        return Json(json!({ "msg": "你的 IP 当前未被封禁", "ok": false }));
    }
    if !state.unban_quota.consume(&ip) {
        return Json(json!({
            "msg": "本月自助解封次数已用完（每月最多3次）",
            "ok": false,
        }));
    }
    state.ip_guard.unban(&ip);
    Json(json!({ "msg": "解封成功", "ok": true }))
}
