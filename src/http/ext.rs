//! 请求提取器：面板会话、客户端地址。

use std::sync::Arc;

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};

use crate::{http::error::ApiError, state::AppState};

/// 已认证的面板请求。携带原始 token，便于注销。
pub struct PanelAuth(pub String);

#[async_trait::async_trait]
impl FromRequestParts<Arc<AppState>> for PanelAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        // 目标同时接受 `Authorization: Bearer <token>` 与 `X-Token: <token>`（S-AUTH-01）。
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(|value| value.trim().to_string())
            .or_else(|| {
                parts
                    .headers
                    .get("x-token")
                    .and_then(|value| value.to_str().ok())
                    .map(|value| value.trim().to_string())
            })
            .filter(|token| !token.is_empty())
            .ok_or_else(ApiError::need_login)?;

        if state.sessions.verify(&token) {
            Ok(PanelAuth(token))
        } else {
            Err(ApiError::need_login())
        }
    }
}

/// 客户端地址：取 `X-Forwarded-For` 末项，其次 `X-Real-IP`，最后 TCP 对端。
///
/// 目标把代理追加到 XFF 末尾，并以末项作为调用方地址（F-010 §3.1）。
/// 都没有时返回 `unknown`。
pub fn client_ip(headers: &axum::http::HeaderMap) -> String {
    client_ip_with_peer(headers, None)
}

/// 同 [`client_ip`]，但对端地址可作为兜底。
pub fn client_ip_with_peer(
    headers: &axum::http::HeaderMap,
    peer: Option<std::net::SocketAddr>,
) -> String {
    if let Some(value) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(last) = value
            .split(',')
            .map(str::trim)
            .rfind(|item| !item.is_empty())
        {
            return last.to_string();
        }
        // 目标对“有 XFF 头但全为空项”的情况直接回退到对端，不再看 X-Real-IP。
        if !value.trim().is_empty() {
            return peer
                .map(|peer| peer.ip().to_string())
                .unwrap_or_else(|| "unknown".to_string());
        }
    }
    if let Some(value) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    match peer {
        Some(peer) => peer.ip().to_string(),
        None => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarded_for_uses_last_item() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "1.2.3.4, 5.6.7.8, 9.10.11.12".parse().expect("头值"),
        );
        assert_eq!(client_ip(&headers), "9.10.11.12");
    }

    #[test]
    fn real_ip_is_the_fallback() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-real-ip", "1.2.3.4".parse().expect("头值"));
        assert_eq!(client_ip(&headers), "1.2.3.4");
    }

    #[test]
    fn empty_forwarded_items_are_skipped() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, ".parse().expect("头值"));
        assert_eq!(client_ip(&headers), "1.2.3.4");
        headers.insert("x-forwarded-for", ", 5.6.7.8".parse().expect("头值"));
        assert_eq!(client_ip(&headers), "5.6.7.8");
    }

    #[test]
    fn invalid_forwarded_list_uses_peer() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-forwarded-for", ",".parse().expect("头值"));
        headers.insert("x-real-ip", "1.2.3.4".parse().expect("头值"));
        let peer: std::net::SocketAddr = "9.9.9.9:1234".parse().expect("地址");
        assert_eq!(client_ip_with_peer(&headers, Some(peer)), "9.9.9.9");
    }
}
