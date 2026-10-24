//! 订阅端点：总订阅与分组订阅。
//!
//! 契约见 [`docs/design/03-http-contract.md`](../../../docs/design/03-http-contract.md) §5。

use std::{collections::HashSet, sync::Arc};

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};

use crate::{playlist, state::AppState};

/// `GET /playlist.m3u`。
pub async fn total_playlist(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let config = state.snapshot();
    let token = token_for(&state, &config);
    let local = playlist::render_total(&config, &base_url(&headers), token.as_deref());
    let extra = aggregate_sources(&state, &config).await;
    let body = playlist::compose(local, extra, &config.playlist.merge_position);
    m3u_response(body, "playlist.m3u")
}

/// 抓取并合并聚合源（F-013 §5）。
///
/// 单个源失败只记日志；本地订阅照常返回。
async fn aggregate_sources(state: &Arc<AppState>, config: &crate::config::Config) -> String {
    let mut extra = String::new();
    let mut seen = HashSet::new();
    for source in config
        .playlist
        .sources
        .iter()
        .filter(|source| source.enabled && !source.url.trim().is_empty())
    {
        let key = format!("sub:{}:{}", source.id, source.url);
        let ttl = std::time::Duration::from_secs(crate::subs::effective_ttl_secs(source.ttl_secs));
        let body = match state.sources.get_with_ttl(&key, ttl) {
            Some(body) => body,
            None => match fetch_source(state, source).await {
                Ok(text) => text,
                Err(error) => {
                    tracing::warn!(url = %source.url, error = %error, "抓取聚合源失败");
                    state.sources.mark_error(&source.id, error);
                    continue;
                }
            },
        };
        let (piece, channels) =
            playlist::aggregate_seen(source, &config.playlist.global_exclude, &body, &mut seen);
        extra.push_str(&piece);
        state.sources.set_channels(&source.id, channels);
    }
    extra
}

/// 抓取单个聚合源；成功时写入缓存与状态。
async fn fetch_source(
    state: &Arc<AppState>,
    source: &crate::config::model::PlaylistSource,
) -> Result<String, String> {
    let response = state
        .http
        .get(&source.url)
        .send()
        .await
        .map_err(|error| format!("抓取失败: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("上游返回 {}", response.status()));
    }
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取失败: {error}"))?;
    state
        .sources
        .put(&format!("sub:{}:{}", source.id, source.url), text.clone());
    state.sources.mark_success(&source.id);
    Ok(text)
}

/// `GET /{gid}/playlist.m3u`。
pub async fn group_playlist(
    State(state): State<Arc<AppState>>,
    Path(gid): Path<String>,
    headers: HeaderMap,
) -> Response {
    let config = state.snapshot();
    let Some(group) = config.group(&gid) else {
        return (StatusCode::NOT_FOUND, "Group not found").into_response();
    };
    let token = token_for(&state, &config);
    match playlist::render_group(&config, group, &base_url(&headers), token.as_deref()) {
        Ok(body) => m3u_response(body, &format!("{gid}.m3u")),
        Err(error) => (StatusCode::NOT_FOUND, error.message()).into_response(),
    }
}

/// Token 仅在校验开启时随订阅下发（S-URL-04）。
fn token_for(state: &AppState, config: &crate::config::Config) -> Option<String> {
    if config.auth.token_enabled {
        Some(state.play_tokens.current())
    } else {
        None
    }
}

/// 由请求推导外部基地址。
fn base_url(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1:19899");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("http");
    format!("{scheme}://{host}")
}

fn m3u_response(body: String, filename: &str) -> Response {
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-mpegurl; charset=utf-8"),
    );
    let disposition = format!("attachment; filename=\"{filename}\"");
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}
