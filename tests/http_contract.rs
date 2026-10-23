//! 路由级集成测试：起真实 HTTP 服务，用 `reqwest` 观测对外契约。
//!
//! 覆盖 `docs/design/03-http-contract.md` §3–§4 的关键形状。
//! 伪上游也由 axum 提供，避免外部依赖。

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use axum::{
    extract::{Path, Query},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use iptv_proxy::{config, epg, http, logging::RingBuffer, state::AppState};
use reqwest::{redirect::Policy, Client, Response as HttpResponse};

const LIVE_BODY: &str = "#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXTINF:4.0,\nseg1.ts\n#EXTINF:4.0,\nseg2.ts\n#EXT-X-ENDLIST\n";
const LIVE: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXTINF:4.0,\nseg1.ts\n#EXTINF:4.0,\nseg2.ts\n#EXT-X-ENDLIST\n";
const BADSEG: &str =
    "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXTINF:4.0,\nmissing.ts\n";
const MASTER: &str = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1920x1080\nv0/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nv1/index.m3u8\n";
const VARIANT: &str =
    "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\nv1.ts\n#EXT-X-ENDLIST\n";
const SEGMENT: &[u8] = b"segment-bytes";
/// 合成 TS 流缓存（8 个关键帧）。
static TS_STREAM: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

/// 伪 DASH 清单：两条轨道。
const MPD: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT30S">
  <Period>
    <AdaptationSet contentType="video" maxWidth="1280" maxHeight="720">
      <Representation id="v0" codecs="avc1.4d401f" bandwidth="2500000"/>
    </AdaptationSet>
    <AdaptationSet mimeType="audio/mp4" lang="zh">
      <Representation id="a0" codecs="mp4a.40.2" bandwidth="128000"/>
    </AdaptationSet>
  </Period>
</MPD>"#;

/// 伪 DASH 清单：带相对 `<BaseURL>`。
const DASH_BASE_MPD: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT2S">
  <BaseURL>sub/</BaseURL>
  <Period>
    <AdaptationSet contentType="video" maxWidth="640" maxHeight="360">
      <SegmentTemplate timescale="1000" media="chunk-$Number$.m4s" startNumber="1">
        <SegmentTimeline><S d="2000" r="0"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v0" codecs="avc1.42c01e" bandwidth="1000000" width="640" height="360"/>
    </AdaptationSet>
  </Period>
</MPD>"#;

/// 伪 DASH 清单：带 SegmentTimeline 与相对 init/切片。
const DASH_MPD: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT8S">
  <Period>
    <AdaptationSet contentType="video" maxWidth="1280" maxHeight="720">
      <SegmentTemplate timescale="1000" initialization="init-$RepresentationID$.mp4"
        media="chunk-$RepresentationID$-$Number$.m4s" startNumber="1">
        <SegmentTimeline><S d="2000" r="3"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v0" codecs="avc1.4d401f" bandwidth="2500000" width="1280" height="720"/>
    </AdaptationSet>
  </Period>
</MPD>"#;

/// 伪翻译服务：给每条文本加目标语言前缀。
async fn upstream_translate(axum::Json(body): axum::Json<serde_json::Value>) -> Response {
    let target = body
        .get("target_lang")
        .and_then(|value| value.as_str())
        .unwrap_or("zh-cn");
    let translations: Vec<String> = body
        .get("texts")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(|text| format!("[{target}]译:{text}"))
                .collect()
        })
        .unwrap_or_default();
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "translations": translations }).to_string(),
    )
        .into_response()
}

/// 伪单条翻译：只提供 `/translate`，用于回退测试。
async fn upstream_translate_single(axum::Json(body): axum::Json<serde_json::Value>) -> Response {
    let target = body
        .get("to")
        .or_else(|| body.get("target_lang"))
        .and_then(|value| value.as_str())
        .unwrap_or("zh-cn");
    let text = body
        .get("text")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "result": format!("[{target}]译:{text}") }).to_string(),
    )
        .into_response()
}

/// 伪 MTranServer v5：要求 Bearer Token，返回 `{results}`。
async fn upstream_translate_v5(
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        != Some("Bearer demo-token")
    {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let target = body
        .get("to")
        .or_else(|| body.get("target_lang"))
        .and_then(|value| value.as_str())
        .unwrap_or("zh-cn");
    let results: Vec<String> = body
        .get("texts")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(|text| format!("[{target}]译:{text}"))
                .collect()
        })
        .unwrap_or_default();
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "results": results, "detected_languages": ["en"] }).to_string(),
    )
        .into_response()
}

/// 伪上游：按路径返回清单或切片。
async fn upstream_file(
    Path(path): Path<String>,
    Query(query): Query<std::collections::HashMap<String, String>>,
    incoming: axum::http::HeaderMap,
) -> Response {
    // 回看与自定义头：回显成注释行，便于断言透传。
    if path == "live.m3u8" {
        let seek = query.get("playseek");
        let token = incoming
            .get("x-token")
            .and_then(|value| value.to_str().ok());
        if seek.is_some() || token.is_some() {
            let mut extra = String::new();
            if let Some(seek) = seek {
                extra.push_str(&format!("# playseek={seek}\n"));
            }
            if let Some(token) = token {
                extra.push_str(&format!("# x-token={token}\n"));
            }
            let body = format!("#EXTM3U\n{extra}{LIVE_BODY}");
            return (
                [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")],
                body,
            )
                .into_response();
        }
    }
    let (mime, body): (&str, &[u8]) = match path.as_str() {
        "live.m3u8" => ("application/vnd.apple.mpegurl", LIVE.as_bytes()),
        "badseg.m3u8" => ("application/vnd.apple.mpegurl", BADSEG.as_bytes()),
        "master.m3u8" => ("application/vnd.apple.mpegurl", MASTER.as_bytes()),
        "v0/index.m3u8" => ("application/vnd.apple.mpegurl", VARIANT.as_bytes()),
        "seg1.ts" | "seg2.ts" | "v0/v1.ts" => ("video/mp2t", SEGMENT),
        "extra.m3u8" => ("application/vnd.apple.mpegurl", EXTRA.as_bytes()),
        "manifest.mpd" => ("application/dash+xml", MPD.as_bytes()),
        "dash/live.mpd" => ("application/dash+xml", DASH_MPD.as_bytes()),
        "dash/base.mpd" => ("application/dash+xml", DASH_BASE_MPD.as_bytes()),
        "dash/init-v0.mp4" => ("video/mp4", b"dash-init"),
        "dash/sub/chunk-1.m4s" => ("video/mp4", b"dash-base-segment"),
        "dash/chunk-v0-1.m4s"
        | "dash/chunk-v0-2.m4s"
        | "dash/chunk-v0-3.m4s"
        | "dash/chunk-v0-4.m4s" => ("video/mp4", b"dash-segment"),
        "live.ts" => (
            "video/mp2t",
            TS_STREAM.get_or_init(|| ts_stream(8)).as_slice(),
        ),
        _ => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    ([(header::CONTENT_TYPE, mime)], body.to_vec()).into_response()
}

/// 测试夹具：临时目录、伪上游、本实现服务、播放 Token。
struct Fixture {
    dir: PathBuf,
    base: String,
    upstream: String,
    token: String,
    client: Client,
    state: Arc<AppState>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("绑定端口");
    let addr = listener.local_addr().expect("取端口");
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await;
    });
    format!("http://{addr}")
}

async fn wait_ready(client: &Client, base: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(response) = client.get(format!("{base}/health")).send().await {
            if response.status() == StatusCode::OK {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("服务未就绪：{base}");
}

async fn fixture() -> Fixture {
    fixture_with(false).await
}

/// 聚合源文本：三条频道，供 `[playlist]` 过滤测试。
const EXTRA: &str = "#EXTM3U\n#EXTINF:-1 tvg-id=\"keep\" group-title=\"远端组\",远端频道\nhttp://placeholder.test/remote.m3u8\n#EXTINF:-1 tvg-id=\"skip\" group-title=\"远端组\",跳过频道\nhttp://placeholder.test/skip.m3u8\n#EXTINF:-1 tvg-id=\"excluded\" group-title=\"远端组\",全局排除\nhttp://placeholder.test/excluded.m3u8\n";

/// `epg_enabled` 为真时，伪上游额外提供 `/epg.xml`，配置里打开 EPG。
async fn fixture_with(epg_enabled: bool) -> Fixture {
    fixture_with_all(epg_enabled, "").await
}

/// 带额外 TOML 片段的夹具（用于 `[playlist]` 聚合）。
async fn fixture_with_extra(extra: &str) -> Fixture {
    fixture_with_all(false, extra).await
}

async fn fixture_with_all(epg_enabled: bool, extra_section: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("iptv-proxy-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("建临时目录");

    let mut upstream_app = Router::new()
        .route("/translate/batch", axum::routing::post(upstream_translate))
        .route(
            "/v5/translate/batch",
            axum::routing::post(upstream_translate_v5),
        )
        .route(
            "/single/translate",
            axum::routing::post(upstream_translate_single),
        )
        .route("/*path", get(upstream_file));
    if epg_enabled {
        let body = epg_document();
        upstream_app = upstream_app.route(
            "/epg.xml",
            get(move || {
                let body = body.clone();
                async move { ([(header::CONTENT_TYPE, "application/xml")], body) }
            }),
        );
    }
    let upstream = serve(upstream_app).await;
    let extra_section = extra_section.replace("{upstream}", &upstream);

    let toml = format!(
        r#"
[server]
bind = "127.0.0.1:0"
log_level = "error"
not_found_url = "http://placeholder.test/404.mp4"
error_url = "http://placeholder.test/mistake.mp4"

[auth]
token_enabled = true
token_failure_url = "http://placeholder.test/Failure.mp4"

[cache]
backend = "disk"
disk_path = "{dir}/cache"
[cache.disguise]
enabled = true
formats = ["jpg", "png"]

[[groups]]
id = "pg"
name = "代理组"
mode = "proxy"
access = "public"
[[groups.channels]]
id = "c1"
name = "媒体清单频道"
url = "{upstream}/live.m3u8"
headers = ["X-Token: abc123"]

[[groups]]
id = "pm"
name = "主清单组"
mode = "proxy"
access = "public"
[[groups.channels]]
id = "c1"
name = "主清单频道"
url = "{upstream}/master.m3u8"

[[groups]]
id = "ar"
name = "归档组"
mode = "proxy"
access = "archived"
[[groups.channels]]
id = "c1"
name = "归档频道"
url = "{upstream}/live.m3u8"

[[groups]]
id = "op"
name = "开放组"
mode = "proxy"
access = "open"
[[groups.channels]]
id = "c1"
name = "开放频道"
url = "{upstream}/live.m3u8"

[[groups]]
id = "pt"
name = "透传组"
mode = "passthrough"
access = "public"
[[groups.channels]]
id = "c1"
name = "透传频道"
url = "{upstream}/live.m3u8"

[[groups]]
id = "badlist"
name = "失败清单组"
mode = "proxy"
access = "open"
[[groups.channels]]
id = "c1"
name = "失败清单频道"
url = "{upstream}/missing.m3u8"

[[groups]]
id = "badseg"
name = "失败切片组"
mode = "proxy"
access = "open"
[[groups.channels]]
id = "c1"
name = "失败切片频道"
url = "{upstream}/badseg.m3u8"
{epg_section}
{extra_section}
"#,
        dir = dir.display(),
        extra_section = extra_section,
        epg_section = if epg_enabled {
            format!(
                r#"
[epg]
enabled = true
keep_days = 3
future_days = 1
keep_yesterday = false
match_by_name = true

[[epg.sources]]
id = "s1"
name = "本地源"
kind = "xml"
url = "{upstream}/epg.xml"
"#
            )
        } else {
            String::new()
        }
    );
    let config_path = dir.join("config.toml");
    std::fs::write(&config_path, toml).expect("写配置");
    let cfg = config::load(&config_path).expect("加载配置");

    let state = Arc::new(AppState::new(
        config_path,
        cfg,
        Arc::new(RingBuffer::default()),
    ));
    let base = serve(http::router(state.clone())).await;

    let client = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .expect("建客户端");
    wait_ready(&client, &base).await;

    let body = client
        .get(format!("{base}/playlist.m3u"))
        .send()
        .await
        .expect("取订阅")
        .text()
        .await
        .expect("读订阅");
    let token = body
        .split("token=")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_hexdigit()).next())
        .filter(|value| value.len() == 48)
        .expect("订阅应含 48 位 Token")
        .to_string();

    Fixture {
        dir,
        base,
        upstream,
        token,
        client,
        state,
    }
}

/// 生成带动态时间窗的 XMLTV：一条在窗口内，一条在 1990 年。
fn epg_document() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("取时间")
        .as_secs() as i64;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<tv generator-info-name="src">
  <channel id="cctv1"><display-name lang="zh">媒体清单频道</display-name></channel>
  <channel id="unused"><display-name>未匹配频道</display-name></channel>
  <programme start="{start}" stop="{stop}" channel="cctv1">
    <title lang="zh">新闻</title>
    <desc>早间新闻</desc>
  </programme>
  <programme start="19900101000000 +0000" stop="19900101010000 +0000" channel="cctv1">
    <title>老节目</title>
  </programme>
  <programme start="{start}" stop="{stop}" channel="unused">
    <title>未匹配节目</title>
  </programme>
</tv>"#,
        start = xmltv_time(now - 1800),
        stop = xmltv_time(now + 1800),
    )
}

/// Unix 秒 → `YYYYMMDDHHMMSS +0000`。
fn xmltv_time(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}{:02}{:02}{:02} +0000",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// 1970-01-01 起的天数 → 公历。
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

async fn fetch(fixture: &Fixture, path: &str) -> HttpResponse {
    fixture
        .client
        .get(format!("{}{}", fixture.base, path))
        .send()
        .await
        .expect("发请求")
}

fn header(response: &HttpResponse, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// 取正文里的非标签行。
fn media_lines(body: &str) -> Vec<&str> {
    body.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// 断言伪装切片地址形状：绝对地址 + `<16hex>_<32hex>.<ext>`。
fn assert_disguised(fixture: &Fixture, url: &str) {
    let prefix = format!(
        "{}/{}/c1/",
        fixture.base,
        if url.contains("/pm/") { "pm" } else { "pg" }
    );
    assert!(
        url.starts_with(&prefix),
        "应为 {prefix} 下的绝对地址：{url}"
    );
    let tail = url.rsplit('/').next().expect("取文件名");
    let (hash, ext) = tail.split_once('.').expect("应含扩展名");
    assert!(["jpg", "png"].contains(&ext), "扩展名应取自配置：{ext}");
    let (left, right) = hash.split_once('_').expect("应含下划线");
    assert_eq!(left.len(), 16, "前缀长度：{left}");
    assert!(
        left.chars().all(|c| c.is_ascii_hexdigit()),
        "前缀应为十六进制：{left}"
    );
    assert_eq!(right.len(), 32, "后缀长度：{right}");
    assert!(
        right.chars().all(|c| c.is_ascii_hexdigit()),
        "后缀应为十六进制：{right}"
    );
}

#[tokio::test]
async fn health_and_total_playlist() {
    let fixture = fixture().await;

    let response = fetch(&fixture, "/health").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.text().await.expect("读正文"), "ok");

    let response = fetch(&fixture, "/playlist.m3u").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        "application/x-mpegurl; charset=utf-8"
    );
    assert_eq!(
        header(&response, "content-disposition"),
        "attachment; filename=\"playlist.m3u\""
    );
    let body = response.text().await.expect("读正文");
    assert_eq!(
        body.lines().next(),
        Some("#EXTM3U"),
        "EPG 关闭时不注入 x-tvg-url"
    );
    // 归档组不导出；公开、开放、透传组导出。
    assert!(body.contains("/pg/c1.m3u8?token="));
    // 直通组在订阅里直接给上游地址。
    assert!(body.contains(&format!("{}/live.m3u8", fixture.upstream)));
    assert!(!body.contains("/pt/"));
    assert!(!body.contains("/ar/"));
}

#[tokio::test]
async fn media_upstream_entry_is_rewritten_media_playlist() {
    let fixture = fixture().await;
    let response = fetch(&fixture, &format!("/pg/c1.m3u8?token={}", fixture.token)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        header(&response, "cache-control"),
        "public, max-age=4, s-maxage=4"
    );
    let body = response.text().await.expect("读正文");
    assert!(body.contains("#EXT-X-TARGETDURATION:4"));
    assert!(!body.contains("#EXT-X-STREAM-INF"));
    let segments = media_lines(&body);
    assert_eq!(segments.len(), 2);
    for url in segments {
        assert_disguised(&fixture, url);
    }
}

#[tokio::test]
async fn prefetch_caches_media_playlist_segments_except_tail() {
    let fixture = fixture().await;
    let response = fetch(&fixture, &format!("/pg/c1.m3u8?token={}", fixture.token)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let dir = fixture.dir.join("cache/seg_bytes");
    let mut count = 0;
    for _ in 0..40 {
        count = std::fs::read_dir(&dir)
            .map(|read| {
                read.filter_map(Result::ok)
                    .filter(|item| item.file_type().map(|kind| kind.is_file()).unwrap_or(false))
                    .count()
            })
            .unwrap_or(0);
        if count >= 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(count, 1, "默认 skip_tail=1，应只预取第一条切片");
}

#[tokio::test]
async fn upstream_playlist_failure_redirects_to_error_url() {
    let fixture = fixture().await;
    let response = fetch(&fixture, "/badlist/c1.m3u8").await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        header(&response, "location"),
        "http://placeholder.test/mistake.mp4"
    );
}

#[tokio::test]
async fn missing_upstream_segment_returns_empty_200() {
    let fixture = fixture().await;
    let response = fetch(&fixture, "/badseg/c1.m3u8").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "cache-control"), "no-cache, no-store");
    let body = response.text().await.expect("读正文");
    let url = media_lines(&body)[0].to_string();
    let path = url.strip_prefix(&fixture.base).expect("切片应为绝对地址");
    let response = fetch(&fixture, path).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "cache-control"), "public, max-age=300");
    assert!(response.bytes().await.expect("读字节").is_empty());
}

#[tokio::test]
async fn media_upstream_variant_path_redirects_to_error_url() {
    let fixture = fixture().await;
    let response = fetch(&fixture, "/pg/c1/video/index.txt").await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        header(&response, "location"),
        "http://placeholder.test/mistake.mp4"
    );
}

#[tokio::test]
async fn master_upstream_entry_is_synthetic_master_playlist() {
    let fixture = fixture().await;
    let response = fetch(&fixture, &format!("/pm/c1.m3u8?token={}", fixture.token)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        "application/vnd.apple.mpegurl"
    );
    assert_eq!(header(&response, "cache-control"), "no-cache, no-store");
    let body = response.text().await.expect("读正文");
    assert!(body.contains("#EXT-X-VERSION:6"));
    assert!(body.contains("#EXT-X-INDEPENDENT-SEGMENTS"));
    assert!(body.contains("#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1920x1080"));
    assert!(body.contains(&format!("{}/pm/c1/video/index.txt", fixture.base)));
}

#[tokio::test]
async fn master_upstream_variant_playlist_and_index_alias() {
    let fixture = fixture().await;
    let response = fetch(&fixture, "/pm/c1/video/index.txt").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        header(&response, "cache-control"),
        "public, max-age=6, s-maxage=6"
    );
    let body = response.text().await.expect("读正文");
    let segments = media_lines(&body);
    assert_eq!(segments.len(), 1);
    assert_disguised(&fixture, segments[0]);

    // `index.m3u8` 是入口别名。
    let response = fetch(
        &fixture,
        &format!("/pm/c1/index.m3u8?token={}", fixture.token),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        "application/vnd.apple.mpegurl"
    );
}

#[tokio::test]
async fn segment_served_from_cache_with_hit_headers() {
    let fixture = fixture().await;
    let response = fetch(&fixture, "/pm/c1/video/index.txt").await;
    let body = response.text().await.expect("读正文");
    let url = media_lines(&body)[0].to_string();
    let path = url
        .strip_prefix(&fixture.base)
        .expect("切片应为绝对地址")
        .to_string();

    let response = fetch(&fixture, &path).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "cache-control"), "public, max-age=300");
    assert!(header(&response, "content-type").starts_with("image/"));
    assert_eq!(response.bytes().await.expect("读字节").as_ref(), SEGMENT);

    let response = fetch(&fixture, &path).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "cache-control"), "private, max-age=300");
}

#[tokio::test]
async fn token_and_unknown_path_semantics() {
    let fixture = fixture().await;

    // 未知频道：无 Token → failure；合法 Token → not_found。
    let response = fetch(&fixture, "/pg/ghost.m3u8").await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        header(&response, "location"),
        "http://placeholder.test/Failure.mp4"
    );
    let response = fetch(&fixture, &format!("/pg/ghost.m3u8?token={}", fixture.token)).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        header(&response, "location"),
        "http://placeholder.test/404.mp4"
    );

    // 未知分组同理。
    let response = fetch(&fixture, &format!("/ghost/c1.m3u8?token={}", fixture.token)).await;
    assert_eq!(
        header(&response, "location"),
        "http://placeholder.test/404.mp4"
    );

    // 其它未知路径不看 Token。
    let response = fetch(&fixture, "/nope/nope/nope").await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        header(&response, "location"),
        "http://placeholder.test/404.mp4"
    );

    // 错 Token。
    let response = fetch(&fixture, "/pg/c1.m3u8?token=deadbeef").await;
    assert_eq!(
        header(&response, "location"),
        "http://placeholder.test/Failure.mp4"
    );

    // open 组免 Token。
    let response = fetch(&fixture, "/op/c1.m3u8").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn archived_entry_redirects_to_not_found() {
    let fixture = fixture().await;
    let response = fetch(&fixture, &format!("/ar/c1.m3u8?token={}", fixture.token)).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        header(&response, "location"),
        "http://placeholder.test/404.mp4"
    );

    let response = fetch(&fixture, "/ar/playlist.m3u").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn passthrough_requires_token() {
    let fixture = fixture().await;
    let response = fetch(&fixture, "/pt/c1.m3u8").await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        header(&response, "location"),
        "http://placeholder.test/Failure.mp4"
    );

    let response = fetch(&fixture, &format!("/pt/c1.m3u8?token={}", fixture.token)).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        header(&response, "location"),
        format!("{}/live.m3u8", fixture.upstream)
    );
}

#[tokio::test]
async fn unknown_panel_api_is_404() {
    let fixture = fixture().await;
    let response = fetch(&fixture, "/panel/api/nope").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn epg_disabled_returns_404() {
    let fixture = fixture().await;
    for path in ["/epg.xml", "/epg.xml.gz", "/epg"] {
        let response = fetch(&fixture, path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn epg_enabled_serves_xmltv_and_filters() {
    let fixture = fixture_with(true).await;

    let summary = epg::refresh(&fixture.state).await.expect("采集应成功");
    assert_eq!(summary.channels, 1, "只保留匹配到的频道");
    assert_eq!(summary.kept, 1, "只保留时间窗内的节目");

    let response = fetch(&fixture, "/epg.xml").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        "application/xml; charset=utf-8"
    );
    let body = response.text().await.expect("读正文");
    assert!(body.contains("generator-info-name=\"iptv-proxy\""));
    assert!(body.contains("<channel id=\"c1\">"));
    assert!(body.contains("<programme start="));
    assert!(body.contains("channel=\"c1\""));
    assert!(body.contains("<title lang=\"zh\">新闻</title>"));
    assert!(!body.contains("老节目"));
    assert!(!body.contains("未匹配节目"));

    let response = fetch(&fixture, "/epg.xml.gz").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), "application/gzip");
    let bytes = response.bytes().await.expect("读字节");
    assert_eq!(&bytes[..2], &[0x1f, 0x8b], "应为 gzip 魔数");

    let response = fetch(&fixture, "/epg").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(header(&response, "content-type").starts_with("text/html"));

    // EPG 开启时，订阅首行注入公开节目单地址。
    let response = fetch(&fixture, "/playlist.m3u").await;
    assert_eq!(response.status(), StatusCode::OK);
    let playlist = response.text().await.expect("读订阅");
    let expected = format!("#EXTM3U x-tvg-url=\"{}/epg.xml.gz\"", fixture.base);
    assert_eq!(playlist.lines().next(), Some(expected.as_str()));

    // 面板接口需要会话。
    let response = fetch(&fixture, "/panel/api/epg/status").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn epg_panel_helpers_scripts_programmes_and_sources() {
    let fixture = fixture_with(true).await;
    epg::refresh(&fixture.state).await.expect("采集应成功");
    let session = login(&fixture).await;
    let auth = format!("Bearer {session}");

    // 脚本列表、上传、删除。
    let response = fixture
        .client
        .get(format!("{}/panel/api/epg/scripts", fixture.base))
        .header("authorization", &auth)
        .send()
        .await
        .expect("脚本列表");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert!(body.get("python").is_some());
    assert!(body["scripts"].as_array().is_some());

    let response = fixture
        .client
        .post(format!("{}/panel/api/epg/script", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(r#"{"name":"demo.py","content":"print('demo')"}"#)
        .send()
        .await
        .expect("上传脚本");
    assert_eq!(response.status(), StatusCode::OK);

    let response = fixture
        .client
        .get(format!("{}/panel/api/epg/scripts", fixture.base))
        .header("authorization", &auth)
        .send()
        .await
        .expect("脚本列表");
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert!(body["scripts"]
        .as_array()
        .expect("scripts")
        .iter()
        .any(|item| item["name"] == "demo.py"));

    let response = fixture
        .client
        .post(format!("{}/panel/api/epg/script-delete", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(r#"{"name":"demo.py"}"#)
        .send()
        .await
        .expect("删除脚本");
    assert_eq!(response.status(), StatusCode::OK);

    // Cron 预览：合法与非法。
    let response = fixture
        .client
        .post(format!("{}/panel/api/epg/cron-preview", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(r#"{"expr":"*/30 * * * *"}"#)
        .send()
        .await
        .expect("Cron 预览");
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["ok"], true);
    assert_eq!(body["next"].as_array().expect("next").len(), 3);

    let response = fixture
        .client
        .post(format!("{}/panel/api/epg/cron-preview", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(r#"{"expr":"bad"}"#)
        .send()
        .await
        .expect("Cron 预览");
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["ok"], false);

    // 频道节目表包含录制缓冲。
    let response = fixture
        .client
        .get(format!("{}/panel/api/epg/programmes/c1", fixture.base))
        .header("authorization", &auth)
        .send()
        .await
        .expect("节目表");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["pad_before"], 60);
    assert_eq!(body["pad_after"], 180);
    let programmes = body["programmes"].as_array().expect("programmes");
    assert!(programmes.iter().any(|item| item["title"] == "新闻"));
    assert!(programmes
        .iter()
        .all(|item| item.get("rec_start").is_some() && item.get("stop_epoch").is_some()));

    // 源频道搜索、建议与时区预览。
    let response = fixture
        .client
        .get(format!(
            "{}/panel/api/epg/source-channels/s1?q=未匹配&offset=0&limit=60",
            fixture.base
        ))
        .header("authorization", &auth)
        .send()
        .await
        .expect("源频道");
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["id"], "unused");

    let response = fixture
        .client
        .get(format!("{}/panel/api/epg/suggest/pg/c1", fixture.base))
        .header("authorization", &auth)
        .send()
        .await
        .expect("建议");
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert!(body["suggestions"]
        .as_array()
        .expect("suggestions")
        .iter()
        .any(|item| item["source_channel_id"] == "cctv1"));

    let response = fixture
        .client
        .get(format!(
            "{}/panel/api/epg/tz-preview/s1?shift=60",
            fixture.base
        ))
        .header("authorization", &auth)
        .send()
        .await
        .expect("时区预览");
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["ok"], true);
    assert!(body["items"][0]["current"] != body["items"][0]["shifted"]);

    // raw 缓存可重新生成，不重新下载。
    let response = fixture
        .client
        .post(format!("{}/panel/api/epg/regenerate", fixture.base))
        .header("authorization", &auth)
        .send()
        .await
        .expect("重新生成");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn epg_extra_channels_and_public_viewer_toggle() {
    let fixture = fixture_with(true).await;
    epg::refresh(&fixture.state).await.expect("采集应成功");
    let session = login(&fixture).await;
    let auth = format!("Bearer {session}");

    let response = fixture
        .client
        .post(format!("{}/panel/api/epg/extra", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(r#"{"action":"add","source_id":"s1","source_channel_id":"unused","out_id":"extra1","out_name":"额外频道"}"#)
        .send()
        .await
        .expect("添加额外频道");
    assert_eq!(response.status(), StatusCode::OK);

    // 等后台重新生成。
    tokio::time::sleep(Duration::from_millis(200)).await;

    let response = fixture
        .client
        .get(format!("{}/panel/api/epg/mappings", fixture.base))
        .header("authorization", &auth)
        .send()
        .await
        .expect("映射表");
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["extra"].as_array().expect("extra").len(), 1);

    let xml = fetch(&fixture, "/epg.xml").await;
    assert_eq!(xml.status(), StatusCode::OK);
    let xml = xml.text().await.expect("读 XML");
    assert!(xml.contains("<channel id=\"extra1\">"));
    assert!(xml.contains("未匹配节目"));

    let response = fixture
        .client
        .post(format!("{}/panel/api/epg/extra", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(r#"{"action":"remove","source_id":"s1","source_channel_id":"unused"}"#)
        .send()
        .await
        .expect("移除额外频道");
    assert_eq!(response.status(), StatusCode::OK);

    // 公开浏览页开关。
    let mut config = (*fixture.state.snapshot()).clone();
    config.epg.public_viewer_enabled = false;
    fixture.state.config.store(Arc::new(config));
    let response = fetch(&fixture, "/epg").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn playseek_is_forwarded_to_upstream() {
    let fixture = fixture().await;
    let seek = "20261020080000-20261020090000";

    // 代理：上游收到 playseek，响应带注释行。
    let response = fetch(
        &fixture,
        &format!("/pg/c1.m3u8?token={}&playseek={seek}", fixture.token),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("读正文");
    assert!(body.contains(&format!("# playseek={seek}")), "{body}");

    // 透传：跳转地址带 playseek。
    let response = fetch(
        &fixture,
        &format!("/pt/c1.m3u8?token={}&playseek={seek}", fixture.token),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        header(&response, "location"),
        format!("{}/live.m3u8?playseek={seek}", fixture.upstream)
    );
}

#[tokio::test]
async fn single_segment_unknown_path_is_404() {
    let fixture = fixture().await;
    for path in ["/status", "/foo"] {
        let response = fetch(&fixture, path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn epg_manual_mapping_and_match_report() {
    let fixture = fixture_with(true).await;
    let session = login(&fixture).await;
    let auth = format!("Bearer {session}");
    let post = |path: &str, body: serde_json::Value| {
        fixture
            .client
            .post(format!("{}{path}", fixture.base))
            .header("authorization", auth.clone())
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
    };

    let response = post(
        "/panel/api/epg/mapping",
        serde_json::json!({
            "gid": "pg", "cid": "c1", "source_id": "s1", "source_channel_id": "unused"
        }),
    )
    .await
    .expect("设置映射");
    assert_eq!(response.status(), StatusCode::OK);

    let mappings: serde_json::Value = serde_json::from_str(
        &fixture
            .client
            .get(format!("{}/panel/api/epg/mappings", fixture.base))
            .header("authorization", auth.clone())
            .send()
            .await
            .expect("映射表")
            .text()
            .await
            .expect("正文"),
    )
    .expect("JSON");
    assert_eq!(mappings["mappings"]["pg/c1"]["source_id"], "s1");
    assert_eq!(mappings["mappings"]["pg/c1"]["source_channel_id"], "unused");

    assert_eq!(
        post("/panel/api/epg/refresh", serde_json::json!({}))
            .await
            .expect("刷新")
            .status(),
        StatusCode::OK
    );
    let body = fetch(&fixture, "/epg.xml").await.text().await.expect("EPG");
    assert!(body.contains("<channel id=\"c1\">"));
    assert!(body.contains("未匹配节目"));
    assert!(!body.contains("早间新闻"));

    let mappings: serde_json::Value = serde_json::from_str(
        &fixture
            .client
            .get(format!("{}/panel/api/epg/mappings", fixture.base))
            .header("authorization", auth.clone())
            .send()
            .await
            .expect("映射表")
            .text()
            .await
            .expect("正文"),
    )
    .expect("JSON");
    assert_eq!(
        mappings["mappings"]["pg/c1"]["source_display_name"],
        "未匹配频道"
    );

    let report = fixture
        .client
        .get(format!("{}/panel/api/epg/match-report", fixture.base))
        .header("authorization", auth.clone())
        .send()
        .await
        .expect("报告");
    assert_eq!(report.status(), StatusCode::OK);
    assert!(
        header(&report, "content-type").starts_with("text/plain"),
        "报告应为纯文本"
    );
    let report = report.text().await.expect("正文");
    assert!(report.contains("pg     c1"), "{report}");
    assert!(report.contains("manual"), "{report}");
    assert!(report.contains("s1"), "{report}");

    // 清空映射后回到自动匹配。
    assert_eq!(
        post(
            "/panel/api/epg/mapping",
            serde_json::json!({"gid":"pg","cid":"c1","source_id":"","source_channel_id":""})
        )
        .await
        .expect("清除")
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        post("/panel/api/epg/refresh", serde_json::json!({}))
            .await
            .expect("刷新")
            .status(),
        StatusCode::OK
    );
    let body = fetch(&fixture, "/epg.xml").await.text().await.expect("EPG");
    assert!(body.contains("早间新闻"));
    assert!(!body.contains("未匹配节目"));
}

#[tokio::test]
async fn epg_api_exposes_channels_programmes_and_search() {
    let fixture = fixture_with(true).await;
    epg::refresh(&fixture.state).await.expect("采集应成功");

    let response = fetch(&fixture, "/epg/api/channels").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["count"], 1);
    assert_eq!(body["channels"][0]["id"], "c1");

    let response = fetch(&fixture, "/epg/api/programmes/c1").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["count"], 1);
    assert_eq!(body["programmes"][0]["title"], "新闻");

    let response = fetch(&fixture, "/epg/api/search?q=新闻").await;
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["count"], 1);

    let response = fetch(&fixture, "/epg/api/search?q=不存在").await;
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["count"], 0);

    let disabled = fixture_with(false).await;
    let response = fetch(&disabled, "/epg/api/channels").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// 用默认口令登录，返回会话 Token。
async fn login(fixture: &Fixture) -> String {
    let response = fixture
        .client
        .post(format!("{}/panel/api/login", fixture.base))
        .header("content-type", "application/json")
        .body(r#"{"username":"admin","password":"admin"}"#)
        .send()
        .await
        .expect("登录请求");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    body["token"].as_str().expect("应返回 token").to_string()
}

#[tokio::test]
async fn whoami_reports_forwarded_ip() {
    let fixture = fixture().await;

    let response = fixture
        .client
        .get(format!("{}/whoami", fixture.base))
        .header("x-forwarded-for", "1.2.3.4, 5.6.7.8")
        .send()
        .await
        .expect("请求 whoami");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["ip"], "1.2.3.4");

    let response = fetch(&fixture, "/unban/whoami").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    let ip = body["ip"].as_str().expect("ip");
    assert!(!ip.is_empty() && ip != "unknown", "{body}");
}

#[tokio::test]
async fn group_import_adds_skips_and_overwrites() {
    let fixture = fixture().await;
    let token = login(&fixture).await;
    let body = serde_json::json!([{
        "id": "imp",
        "name": "导入组",
        "mode": "proxy",
        "access": "public",
        "channels": [{
            "id": "c1",
            "name": "导入频道",
            "url": format!("{}/live.m3u8", fixture.upstream)
        }]
    }]);

    let post = |path: &str, body: &serde_json::Value| {
        fixture
            .client
            .post(format!("{}{path}", fixture.base))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(body.to_string())
            .send()
    };

    // 未登录 → 401。
    let response = fixture
        .client
        .post(format!("{}/panel/api/groups/import", fixture.base))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 首次导入。
    let response = post("/panel/api/groups/import", &body).await.expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let result: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(result["imported"], 1);
    assert_eq!(result["skipped"], 0);

    let playlist = fetch(&fixture, "/playlist.m3u")
        .await
        .text()
        .await
        .expect("读正文");
    assert!(playlist.contains("/imp/c1.m3u8?token="));

    // 重复导入 → 跳过。
    let response = post("/panel/api/groups/import", &body).await.expect("请求");
    let result: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(result["imported"], 0);
    assert_eq!(result["skipped"], 1);

    // 强制导入 → 覆盖。
    let mut renamed = body.clone();
    renamed[0]["name"] = serde_json::json!("覆盖组");
    let response = post("/panel/api/groups/import/force", &renamed)
        .await
        .expect("请求");
    let result: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(result["imported"], 1);
    let playlist = fetch(&fixture, "/playlist.m3u")
        .await
        .text()
        .await
        .expect("读正文");
    assert!(playlist.contains("覆盖组"));

    // 非数组 → 400。
    let response = post("/panel/api/groups/import", &serde_json::json!({}))
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn m3u_import_creates_groups_and_channels() {
    let fixture = fixture().await;
    let token = login(&fixture).await;

    let post = |path: &str, body: &serde_json::Value| {
        fixture
            .client
            .post(format!("{}{path}", fixture.base))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(body.to_string())
            .send()
    };

    let text = "#EXTM3U\n\
        #EXTINF:-1 tvg-id=\"m1\" tvg-name=\"频道一\" group-title=\"导入A\",频道一\n\
        http://up/1.m3u8\n\
        #EXTINF:-1 group-title=\"导入A\",频道二\n\
        http://up/2.m3u8\n\
        #EXTINF:-1 tvg-id=\"d1\" group-title=\"导入D\",DASH\n\
        http://up/3.mpd\n";

    let body = serde_json::json!({ "text": text, "mode": "append", "group_mode": "auto" });
    let response = post("/panel/api/groups/import/m3u", &body)
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let result: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(result["parsed_groups"], 2);
    assert_eq!(result["parsed_channels"], 3);
    assert_eq!(result["added_groups"], 2);
    assert_eq!(result["skipped_groups"], 0);

    // 落库检查：auto 按扩展名判定模式，tvg-id 作为频道 id。
    let response = fixture
        .client
        .get(format!("{}/panel/api/config", fixture.base))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("取配置");
    let config: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    let groups = config["groups"].as_array().expect("分组数组");
    let group_a = groups
        .iter()
        .find(|group| group["id"] == "A")
        .expect("应有分组 A");
    assert_eq!(group_a["mode"], "rewrite");
    assert_eq!(group_a["format"], "hls");
    assert_eq!(group_a["channels"].as_array().expect("频道").len(), 2);
    assert_eq!(group_a["channels"][0]["id"], "m1");
    assert_eq!(group_a["channels"][0]["name"], "频道一");
    let group_d = groups
        .iter()
        .find(|group| group["id"] == "D")
        .expect("应有分组 D");
    assert_eq!(group_d["mode"], "dash");
    assert_eq!(group_d["format"], "mpd");

    // append 再导入 → 全部跳过。
    let response = post("/panel/api/groups/import/m3u", &body)
        .await
        .expect("请求");
    let result: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(result["skipped_groups"], 2);

    // merge 补一个新频道。
    let merge = serde_json::json!({
        "text": "#EXTM3U\n#EXTINF:-1 tvg-id=\"m3\" group-title=\"导入A\",频道三\nhttp://up/4.m3u8\n",
        "mode": "merge",
        "group_mode": "auto"
    });
    let response = post("/panel/api/groups/import/m3u", &merge)
        .await
        .expect("请求");
    let result: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(result["merged_channels"], 1);

    // replace 整体替换。
    let response = post("/panel/api/groups/import/m3u", &merge)
        .await
        .expect("请求");
    let _ = response;
    let replace = serde_json::json!({
        "text": "#EXTM3U\n#EXTINF:-1 tvg-id=\"m9\" group-title=\"导入A\",新频道\nhttp://up/9.m3u8\n",
        "mode": "replace",
        "group_mode": "auto"
    });
    let response = post("/panel/api/groups/import/m3u", &replace)
        .await
        .expect("请求");
    let result: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(result["replaced_groups"], 1);

    // 非法 mode → 400。
    let bad = serde_json::json!({ "text": text, "mode": "upsert" });
    let response = post("/panel/api/groups/import/m3u", &bad)
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn logo_library_upload_serve_and_delete() {
    let fixture = fixture().await;
    let token = login(&fixture).await;

    // PNG 魔数的 base64。
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let body = serde_json::json!({
        "items": [{ "name": "cctv1.png", "data": format!("data:image/png;base64,{png}") }]
    });
    let upload = |body: &serde_json::Value| {
        fixture
            .client
            .post(format!("{}/panel/api/logo/upload", fixture.base))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(body.to_string())
            .send()
    };

    // 未登录 → 401。
    let response = fixture
        .client
        .post(format!("{}/panel/api/logo/upload", fixture.base))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = upload(&body).await.expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let result: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(result["added"][0]["file"], "cctv1.png");
    assert_eq!(result["failed"].as_array().expect("失败列表").len(), 0);

    // 公开列表。
    let response = fetch(&fixture, "/logo/api/list").await;
    assert_eq!(response.status(), StatusCode::OK);
    let list: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(list["count"], 1);
    assert_eq!(list["logos"][0]["url"], "/logo/cctv1.png");

    // 公开取图。
    let response = fetch(&fixture, "/logo/cctv1.png").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), "image/png");
    assert_eq!(header(&response, "cache-control"), "public, max-age=86400");
    let bytes = response.bytes().await.expect("读字节");
    assert_eq!(&bytes[..4], &[0x89, 0x50, 0x4e, 0x47], "应为 PNG 魔数");

    // 缺文件与越界名 → 404。
    for path in [
        "/logo/missing.png",
        "/logo/..%2F..%2Fconfig.toml",
        "/logo/evil.exe",
    ] {
        let response = fetch(&fixture, path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }

    // 动效请求校验：quality 必须是字符串。
    let response = fixture
        .client
        .post(format!("{}/panel/api/logo/animate", fixture.base))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(
            serde_json::json!({
                "file": "cctv1.png",
                "effect": "spot",
                "quality": 80
            })
            .to_string(),
        )
        .send()
        .await
        .expect("动效请求");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // 原图还原。
    let original = fixture.dir.join("cache/logos/.orig/cctv1.png");
    std::fs::create_dir_all(original.parent().expect("父目录")).expect("建原图目录");
    std::fs::write(&original, b"static-original").expect("写原图");
    let response = fixture
        .client
        .post(format!("{}/panel/api/logo/restore", fixture.base))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(serde_json::json!({ "file": "cctv1.png" }).to_string())
        .send()
        .await
        .expect("还原请求");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read(fixture.dir.join("cache/logos/cctv1.png")).expect("读还原文件"),
        b"static-original"
    );
    let response = fixture
        .client
        .post(format!("{}/panel/api/logo/restore", fixture.base))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(serde_json::json!({ "file": "missing.png" }).to_string())
        .send()
        .await
        .expect("还原请求");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // 删除。
    let response = fixture
        .client
        .post(format!("{}/panel/api/logo/delete", fixture.base))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(serde_json::json!({ "file": "cctv1.png" }).to_string())
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        fetch(&fixture, "/logo/cctv1.png").await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn assets_map_lists_serves_and_removes_local_hls() {
    let fixture = fixture().await;
    let token = login(&fixture).await;
    let dir = fixture.dir.join("cache/assets/convert/demo");
    std::fs::create_dir_all(&dir).expect("建 HLS 目录");
    std::fs::write(dir.join("index.m3u8"), "#EXTM3U\n#EXT-X-ENDLIST\n").expect("写清单");
    fixture
        .state
        .mutate(|config| {
            let assets = config
                .extra
                .entry("assets".to_string())
                .or_insert_with(|| toml::Value::Table(Default::default()));
            let table = assets.as_table_mut().expect("assets 表");
            let map = table
                .entry("map".to_string())
                .or_insert_with(|| toml::Value::Table(Default::default()));
            map.as_table_mut().expect("map 表").insert(
                "http://placeholder.test/404.mp4".to_string(),
                toml::Value::String("/assets/convert/demo/index.m3u8".to_string()),
            );
            Ok(())
        })
        .expect("写配置");

    let response = fixture
        .client
        .get(format!("{}/panel/api/assets/list", fixture.base))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("素材列表");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("JSON");
    assert!(body["ffmpeg_installed"].is_boolean());
    let item = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["url"] == "http://placeholder.test/404.mp4")
        .expect("应检测到占位 URL");
    assert_eq!(item["converted"], true);
    assert_eq!(item["local_path"], "/assets/convert/demo/index.m3u8");

    let response = fixture
        .client
        .post(format!("{}/panel/api/assets/convert", fixture.base))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "url": "ftp://example/x.mp4" }).to_string())
        .send()
        .await
        .expect("转换请求");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = fetch(&fixture, "/assets/convert/demo/index.m3u8").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        "application/vnd.apple.mpegurl"
    );

    let response = fixture
        .client
        .post(format!("{}/panel/api/assets/remove", fixture.base))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "url": "http://placeholder.test/404.mp4" }).to_string())
        .send()
        .await
        .expect("移除资源");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        fetch(&fixture, "/assets/convert/demo/index.m3u8")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn backup_and_restore_roundtrip() {
    let fixture = fixture().await;
    let token = login(&fixture).await;
    let auth = format!("Bearer {token}");

    // 备份。
    let response = fixture
        .client
        .get(format!("{}/panel/api/backup?include=all", fixture.base))
        .header("authorization", &auth)
        .send()
        .await
        .expect("备份请求");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), "application/gzip");
    assert!(header(&response, "content-disposition").contains("iptv-proxy-backup-"));
    let data = response.bytes().await.expect("读字节");
    assert_eq!(&data[..2], &[0x1f, 0x8b], "应为 gzip 魔数");

    // 改动配置：导入一个分组。
    let import = serde_json::json!([{ "id": "tmp", "name": "临时组", "mode": "proxy" }]);
    let response = fixture
        .client
        .post(format!("{}/panel/api/groups/import", fixture.base))
        .header("content-type", "application/json")
        .header("authorization", &auth)
        .body(import.to_string())
        .send()
        .await
        .expect("导入请求");
    assert_eq!(response.status(), StatusCode::OK);
    let groups = |fixture: &Fixture| {
        let request = fixture
            .client
            .get(format!("{}/panel/api/config", fixture.base))
            .header("authorization", &auth);
        request.send()
    };
    let config: serde_json::Value = serde_json::from_str(
        &groups(&fixture)
            .await
            .expect("取配置")
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析 JSON");
    assert!(
        config["groups"]
            .as_array()
            .expect("分组")
            .iter()
            .any(|group| group["id"] == "tmp"),
        "导入后应有临时组"
    );

    // 恢复。
    let response = fixture
        .client
        .post(format!("{}/panel/api/restore", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/gzip")
        .body(data)
        .send()
        .await
        .expect("恢复请求");
    assert_eq!(response.status(), StatusCode::OK);
    let result: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(result["ok"], true);
    assert_eq!(result["config_reloaded"], true);

    // 恢复后临时组消失，原分组仍在。
    let config: serde_json::Value = serde_json::from_str(
        &groups(&fixture)
            .await
            .expect("取配置")
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析 JSON");
    let ids: Vec<&str> = config["groups"]
        .as_array()
        .expect("分组")
        .iter()
        .filter_map(|group| group["id"].as_str())
        .collect();
    assert!(!ids.contains(&"tmp"), "恢复后不应有临时组：{ids:?}");
    assert!(ids.contains(&"pg"), "恢复后应保留原分组：{ids:?}");

    // 非法内容 → 400。
    let response = fixture
        .client
        .post(format!("{}/panel/api/restore", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/gzip")
        .body("not gzip")
        .send()
        .await
        .expect("恢复请求");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn channel_headers_are_sent_upstream() {
    let fixture = fixture().await;
    let response = fetch(&fixture, &format!("/pg/c1.m3u8?token={}", fixture.token)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("读正文");
    assert!(body.contains("# x-token=abc123"), "{body}");
}

#[tokio::test]
async fn self_service_unban_respects_quota() {
    let fixture = fixture().await;
    let session = login(&fixture).await;
    let auth = format!("Bearer {session}");
    let ban = |ip: &str| {
        fixture
            .client
            .post(format!("{}/panel/api/security/ip/ban", fixture.base))
            .header("content-type", "application/json")
            .header("authorization", &auth)
            .body(format!(r#"{{"ip":"{ip}","seconds":60}}"#))
    };

    // 未封禁时只报告状态，不消耗配额。
    let response = fixture
        .client
        .post(format!("{}/unban", fixture.base))
        .header("x-forwarded-for", "8.8.8.8")
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["ok"], false);
    assert_eq!(body["remaining"], 3);

    for round in 0..3 {
        assert_eq!(
            ban("8.8.8.8").send().await.expect("请求").status(),
            StatusCode::OK
        );

        // 封禁期内仍可打开解封页。
        let response = fixture
            .client
            .get(format!("{}/unban", fixture.base))
            .header("x-forwarded-for", "8.8.8.8")
            .send()
            .await
            .expect("请求");
        assert_eq!(response.status(), StatusCode::OK);

        let response = fixture
            .client
            .post(format!("{}/unban", fixture.base))
            .header("x-forwarded-for", "8.8.8.8")
            .send()
            .await
            .expect("请求");
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
        assert_eq!(body["ok"], true);
        assert_eq!(body["remaining"], 2 - round);
    }

    assert_eq!(
        ban("8.8.8.8").send().await.expect("请求").status(),
        StatusCode::OK
    );
    let response = fixture
        .client
        .post(format!("{}/unban", fixture.base))
        .header("x-forwarded-for", "8.8.8.8")
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let response = fixture
        .client
        .get(format!("{}/playlist.m3u", fixture.base))
        .header("x-forwarded-for", "8.8.8.8")
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn login_locks_after_repeated_failures() {
    let fixture = fixture().await;
    let session = login(&fixture).await;
    for _ in 0..21 {
        let response = fixture
            .client
            .post(format!("{}/panel/api/login", fixture.base))
            .header("content-type", "application/json")
            .body(r#"{"username":"admin","password":"wrong"}"#)
            .send()
            .await
            .expect("请求");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let response = fixture
        .client
        .post(format!("{}/panel/api/login", fixture.base))
        .header("content-type", "application/json")
        .body(r#"{"username":"admin","password":"admin"}"#)
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    let locked = body["locked_for_secs"].as_u64().expect("locked_for_secs");
    assert!(locked >= 1 && locked <= 300, "{body}");

    // 既有会话不受登录锁影响。
    let response = fixture
        .client
        .get(format!("{}/panel/api/status", fixture.base))
        .header("authorization", format!("Bearer {session}"))
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn link_info_matches_target_shape() {
    let fixture = fixture().await;
    let session = login(&fixture).await;
    let response = fixture
        .client
        .get(format!("{}/panel/api/link-info", fixture.base))
        .header("authorization", format!("Bearer {session}"))
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert!(body["aliases"].is_object(), "{body}");
    assert_eq!(body["random_id_enabled"], false);
    assert_eq!(body["token_enabled"], true);
    assert_eq!(body["use_alias"], false);
    assert_eq!(body["token"].as_str().expect("token").len(), 48);

    let response = fetch(&fixture, "/panel/api/link-info").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn security_token_rotation_and_ip_ban() {
    let fixture = fixture().await;
    let session = login(&fixture).await;
    let auth = format!("Bearer {session}");
    let panel = |path: &str| {
        fixture
            .client
            .get(format!("{}{path}", fixture.base))
            .header("authorization", &auth)
            .send()
    };

    // Token 查看。
    let response = panel("/panel/api/security/token").await.expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["enabled"], true);
    let first = body["current"].as_str().expect("token").to_string();
    assert_eq!(first.len(), 48);

    // 轮换：订阅地址改用新 Token。
    let response = fixture
        .client
        .post(format!("{}/panel/api/security/token/rotate", fixture.base))
        .header("authorization", &auth)
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    let second = body["token"].as_str().expect("token").to_string();
    assert_ne!(first, second);
    let playlist = fetch(&fixture, "/playlist.m3u")
        .await
        .text()
        .await
        .expect("读正文");
    assert!(playlist.contains(&second));

    // 统计：带转发头的公开请求会被记账。
    let response = fixture
        .client
        .get(format!("{}/playlist.m3u", fixture.base))
        .header("x-forwarded-for", "7.7.7.7")
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);

    // 封禁后用同一地址请求 → 403。
    let response = fixture
        .client
        .post(format!("{}/panel/api/security/ip/ban", fixture.base))
        .header("content-type", "application/json")
        .header("authorization", &auth)
        .body(r#"{"ip":"7.7.7.7","seconds":60}"#)
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let response = fixture
        .client
        .get(format!("{}/playlist.m3u", fixture.base))
        .header("x-forwarded-for", "7.7.7.7")
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // 面板面不受封禁影响（未登录仍是 401）。
    let response = fixture
        .client
        .get(format!("{}/panel/api/status", fixture.base))
        .header("x-forwarded-for", "7.7.7.7")
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 统计反映封禁状态。
    let response = panel("/panel/api/security/ip/stats").await.expect("请求");
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert!(body["banned"].as_u64().expect("banned") >= 1);
    let entry = body["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["ip"] == "7.7.7.7")
        .expect("应有 7.7.7.7 记录");
    assert_eq!(entry["banned"], true);
    assert!(entry["requests"].as_u64().expect("requests") >= 1);

    // 解封后恢复。
    let response = fixture
        .client
        .post(format!("{}/panel/api/security/ip/unban", fixture.base))
        .header("content-type", "application/json")
        .header("authorization", &auth)
        .body(r#"{"ip":"7.7.7.7"}"#)
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let response = fixture
        .client
        .get(format!("{}/playlist.m3u", fixture.base))
        .header("x-forwarded-for", "7.7.7.7")
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn playlist_aggregates_sources_with_filters() {
    let extra = r#"
[playlist]
merge_ttl_secs = 60
merge_position = "top"
global_exclude = ["id:excluded"]

[[playlist.sources]]
id = "s1"
name = "远端源"
url = "{upstream}/extra.m3u8"
enabled = true
ttl_secs = 60
group_mode = "map"
group_map = { "远端组" = "映射组" }
unlisted_groups = "keep"
exclude = ["id:skip"]
"#;
    let fixture = fixture_with_extra(extra).await;
    let body = fetch(&fixture, "/playlist.m3u")
        .await
        .text()
        .await
        .expect("读正文");
    assert!(body.contains("远端频道"), "{body}");
    assert!(
        body.contains("group-title=\"映射组\",远端频道"),
        "group_map 应改写分组名: {body}"
    );
    assert!(!body.contains("跳过频道"), "exclude 应生效");
    assert!(!body.contains("全局排除"), "global_exclude 应生效");
    assert!(body.contains("媒体清单频道"), "本地频道仍应存在");
    // merge_position = "top"：聚合条目在本地条目之前。
    let remote = body.find("远端频道").expect("远端");
    let local = body.find("媒体清单频道").expect("本地");
    assert!(remote < local, "聚合应位于顶部");
}

#[tokio::test]
async fn playlist_dedups_urls_across_sources() {
    let extra = r#"
[[playlist.sources]]
id = "s1"
name = "源一"
url = "{upstream}/extra.m3u8"
enabled = true
unlisted_groups = "keep"

[[playlist.sources]]
id = "s2"
name = "源二"
url = "{upstream}/extra.m3u8"
enabled = true
unlisted_groups = "keep"
"#;
    let fixture = fixture_with_extra(extra).await;
    let body = fetch(&fixture, "/playlist.m3u")
        .await
        .text()
        .await
        .expect("读正文");
    assert_eq!(body.matches("远端频道").count(), 1, "{body}");
    let status: serde_json::Value = serde_json::from_str(
        &fetch(&fixture, "/subs/status")
            .await
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析 JSON");
    let channels: Vec<u64> = status["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|source| source["channels"].as_u64().unwrap_or(0))
        .collect();
    assert_eq!(channels, vec![3, 0], "{status}");
}

#[tokio::test]
async fn playlist_source_failure_does_not_break_subscription() {
    let extra = r#"
[[playlist.sources]]
id = "bad"
name = "坏源"
url = "http://127.0.0.1:9/none.m3u8"
enabled = true
"#;
    let fixture = fixture_with_extra(extra).await;
    let response = fetch(&fixture, "/playlist.m3u").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("读正文");
    assert!(body.contains("媒体清单频道"));
}

/// 伪 HTTP 转发代理：记录次数，并按绝对 URI 取上游。
async fn proxy_forward(
    axum::extract::State((client, count)): axum::extract::State<(Client, Arc<AtomicUsize>)>,
    request: axum::extract::Request,
) -> Response {
    count.fetch_add(1, Ordering::SeqCst);
    let uri = request.uri().to_string();
    match client.get(uri).send().await {
        Ok(response) => {
            let status = response.status();
            let bytes = response.bytes().await.unwrap_or_default();
            (status, bytes.to_vec()).into_response()
        }
        Err(error) => (StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
    }
}

#[tokio::test]
async fn channel_proxy_is_used_for_upstream() {
    let counter = Arc::new(AtomicUsize::new(0));
    let proxy = serve(
        Router::new()
            .fallback(proxy_forward)
            .with_state((Client::new(), counter.clone())),
    )
    .await;
    let extra = format!(
        r#"
[[groups]]
id = "px"
name = "代理组"
mode = "proxy"
access = "open"

[groups.proxy]
kind = "http"
addr = "{proxy}"

[[groups.channels]]
id = "c1"
name = "代理频道"
url = "{{upstream}}/live.m3u8"
"#
    );
    let fixture = fixture_with_extra(&extra).await;
    let response = fetch(&fixture, "/px/c1.m3u8").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(counter.load(Ordering::SeqCst) > 0, "上游请求应经过频道代理");
}

#[tokio::test]
async fn subs_status_lists_configured_sources() {
    let extra = r#"
[[playlist.sources]]
id = "s1"
name = "远端源"
url = "{upstream}/extra.m3u8"
enabled = true
unlisted_groups = "keep"
include = ["group:远端组"]
"#;
    let fixture = fixture_with_extra(extra).await;
    let response = fetch(&fixture, "/subs/status").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    let sources = body["sources"].as_array().expect("sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["id"], "s1");
    assert_eq!(sources[0]["enabled"], true);
    assert_eq!(sources[0]["ttl"], 3600);
    assert_eq!(sources[0]["channels"], 3);
    assert!(sources[0]["last_ok"].as_u64().unwrap_or(0) > 0);
    assert!(sources[0]["last_try"].as_u64().unwrap_or(0) > 0);
    assert!(sources[0]["error"].is_null());

    fetch(&fixture, "/playlist.m3u").await;
    let body: serde_json::Value = serde_json::from_str(
        &fetch(&fixture, "/subs/status")
            .await
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析 JSON");
    assert_eq!(body["sources"][0]["channels"], 3);
    assert!(body["sources"][0]["last_ok"].as_u64().unwrap_or(0) > 0);
    assert!(body["sources"][0]["last_try"].as_u64().unwrap_or(0) > 0);
}

#[tokio::test]
async fn panel_subs_aliases_require_session_and_share_logic() {
    let fixture = fixture().await;
    let path = format!("{}/panel/api/subs/preview", fixture.base);
    let body = serde_json::json!({
        "source": {
            "url": format!("{}/extra.m3u8", fixture.upstream),
            "group_map": {},
            "unlisted_groups": "keep"
        }
    });
    let response = fixture
        .client
        .post(&path)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let session = login(&fixture).await;
    let response = fixture
        .client
        .post(&path)
        .header("authorization", format!("Bearer {session}"))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let parsed: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(parsed["kept"], 3);
    assert_eq!(parsed["trace"]["after_group"], 3);
}

#[tokio::test]
async fn subs_groups_lists_upstream_groups() {
    let fixture = fixture().await;
    let response = fixture
        .client
        .post(format!("{}/subs/groups", fixture.base))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "url": format!("{}/extra.m3u8", fixture.upstream) }).to_string())
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["total"], 3);
    assert_eq!(body["groups"][0]["name"], "远端组");
    assert_eq!(body["groups"][0]["count"], 3);
}

#[tokio::test]
async fn subs_preview_applies_group_map_and_filters() {
    let extra = r#"
[playlist]
global_exclude = ["id:excluded"]

[[playlist.sources]]
id = "s1"
name = "远端源"
url = "{upstream}/extra.m3u8"
enabled = true
exclude = ["id:skip"]
"#;
    let fixture = fixture_with_extra(extra).await;
    let response = fixture
        .client
        .post(format!("{}/subs/preview", fixture.base))
        .header("content-type", "application/json")
        .body(
            serde_json::json!({
                "source": {
                    "id": "s1",
                    "group_mode": "map",
                    "group_map": { "远端组": "映射组" }
                }
            })
            .to_string(),
        )
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["trace"]["total"], 3);
    assert_eq!(body["trace"]["after_group"], 3);
    assert_eq!(body["trace"]["after_global_exclude"], 2);
    assert_eq!(body["trace"]["after_include"], 2);
    assert_eq!(body["trace"]["after_exclude"], 1);
    assert_eq!(body["kept"], 1);
    assert_eq!(body["trace"]["out_groups"][0]["name"], "映射组");
    assert_eq!(body["trace"]["out_groups"][0]["count"], 1);
    assert_eq!(body["sample"][0]["name"], "远端频道");
    assert!(body["sample"][0].get("url").is_none());
}

#[tokio::test]
async fn subs_preview_drops_mapped_group_and_rejects_bad_url() {
    let fixture = fixture().await;
    let response = fixture
        .client
        .post(format!("{}/subs/preview", fixture.base))
        .header("content-type", "application/json")
        .body(
            serde_json::json!({
                "source": {
                    "url": format!("{}/extra.m3u8", fixture.upstream),
                    "group_map": {}
                }
            })
            .to_string(),
        )
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["kept"], 0);
    assert_eq!(body["trace"]["after_group"], 0);
    assert_eq!(body["trace"]["dropped_groups"][0], "远端组");

    let response = fixture
        .client
        .post(format!("{}/subs/preview", fixture.base))
        .header("content-type", "application/json")
        .body(
            serde_json::json!({
                "source": {
                    "url": format!("{}/extra.m3u8", fixture.upstream),
                    "group_map": { "远端组": null }
                }
            })
            .to_string(),
        )
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = fixture
        .client
        .post(format!("{}/subs/preview", fixture.base))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "source": { "url": "file:///etc/passwd" } }).to_string())
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn placeholder() {}

#[tokio::test]
async fn logo_rename_apply_and_match_report() {
    let fixture = fixture().await;
    let session = login(&fixture).await;
    let auth = format!("Bearer {session}");
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let post = |path: &str, body: serde_json::Value| {
        fixture
            .client
            .post(format!("{}{path}", fixture.base))
            .header("content-type", "application/json")
            .header("authorization", auth.clone())
            .body(body.to_string())
            .send()
    };

    for name in ["cctv1.png", "hunan.png"] {
        let body = serde_json::json!({
            "items": [{ "name": name, "data": format!("data:image/png;base64,{png}") }]
        });
        assert_eq!(
            post("/panel/api/logo/upload", body)
                .await
                .expect("上传")
                .status(),
            StatusCode::OK
        );
    }

    // 应用到频道。
    let response = post(
        "/panel/api/logo/apply",
        serde_json::json!({ "gid": "pg", "cid": "c1", "file": "cctv1.png" }),
    )
    .await
    .expect("应用");
    assert_eq!(response.status(), StatusCode::OK);
    let playlist = fetch(&fixture, "/playlist.m3u")
        .await
        .text()
        .await
        .expect("读正文");
    assert!(
        playlist.contains("tvg-logo=\"/logo/cctv1.png\""),
        "{playlist}"
    );

    // 缺文件 → 404。
    let response = post(
        "/panel/api/logo/apply",
        serde_json::json!({ "gid": "pg", "cid": "c1", "file": "missing.png" }),
    )
    .await
    .expect("应用");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // 匹配报告：hunan.png 未使用；其余频道缺台标。
    let report: serde_json::Value = serde_json::from_str(
        &fixture
            .client
            .get(format!("{}/panel/api/logo/match-report", fixture.base))
            .header("authorization", &auth)
            .send()
            .await
            .expect("报告")
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析 JSON");
    assert_eq!(report["unused"], 1);
    assert_eq!(report["unused_logos"][0], "hunan.png");
    assert!(report["missing"].as_u64().expect("missing") >= 1);

    // 重命名（保留扩展名，支持中文）。
    let response = post(
        "/panel/api/logo/rename",
        serde_json::json!({ "file": "cctv1.png", "name": "央视一套" }),
    )
    .await
    .expect("重命名");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["logo"]["file"], "央视一套.png");
    assert_eq!(
        fetch(&fixture, "/logo/央视一套.png").await.status(),
        StatusCode::OK
    );

    // 推荐：无台标频道返回数组。
    let response = post(
        "/panel/api/logo/suggest",
        serde_json::json!({ "threshold": 0.1 }),
    )
    .await
    .expect("推荐");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert!(body["channels"].is_array());
}

#[tokio::test]
async fn media_assets_subtitles_and_resolver_scripts() {
    let fixture = fixture().await;
    let session = login(&fixture).await;
    let auth = format!("Bearer {session}");
    let post = |path: &str, body: serde_json::Value| {
        fixture
            .client
            .post(format!("{}{path}", fixture.base))
            .header("content-type", "application/json")
            .header("authorization", auth.clone())
            .body(body.to_string())
            .send()
    };
    let get = |path: &str| {
        fixture
            .client
            .get(format!("{}{path}", fixture.base))
            .header("authorization", auth.clone())
            .send()
    };

    // ffmpeg 检测：只要求字段存在。
    let body: serde_json::Value = serde_json::from_str(
        &get("/panel/api/assets/ffmpeg-status")
            .await
            .expect("请求")
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析 JSON");
    assert!(body["installed"].is_boolean());

    // 视频素材：上传、列表、下载、删除。
    let payload = serde_json::json!({
        "items": [{ "name": "花絮.mp4", "data": "data:video/mp4;base64,AAAA" }]
    });
    assert_eq!(
        post("/panel/api/assets/upload", payload)
            .await
            .expect("上传")
            .status(),
        StatusCode::OK
    );
    let list: serde_json::Value = serde_json::from_str(
        &get("/panel/api/assets/list")
            .await
            .expect("列表")
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析 JSON");
    assert_eq!(list["count"], 1);
    assert_eq!(list["files"][0]["file"], "花絮.mp4");

    let response = get("/panel/api/assets/raw/花絮.mp4").await.expect("下载");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await.expect("字节").as_ref(), b"\0\0\0");

    let removed = post(
        "/panel/api/assets/remove",
        serde_json::json!({ "file": "花絮.mp4" }),
    )
    .await
    .expect("删除");
    assert_eq!(removed.status(), StatusCode::OK);
    assert_eq!(
        get("/panel/api/assets/raw/花絮.mp4")
            .await
            .expect("再取")
            .status(),
        StatusCode::NOT_FOUND
    );

    // 字幕素材：扩展名白名单。
    let srt = serde_json::json!({
        "items": [{ "name": "第1集.srt", "data": "data:application/x-subrip;base64,MSAwOjAwOjAw" }]
    });
    assert_eq!(
        post("/panel/api/subtitle/upload", srt)
            .await
            .expect("上传")
            .status(),
        StatusCode::OK
    );
    let list: serde_json::Value = serde_json::from_str(
        &get("/panel/api/subtitle/list")
            .await
            .expect("列表")
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析 JSON");
    assert_eq!(list["count"], 1);

    // 解析脚本：保存、列出、删除。
    let saved = post(
        "/panel/api/resolver/script",
        serde_json::json!({ "name": "demo", "content": "print(1)" }),
    )
    .await
    .expect("保存");
    assert_eq!(saved.status(), StatusCode::OK);
    let scripts: serde_json::Value = serde_json::from_str(
        &get("/panel/api/resolver/scripts")
            .await
            .expect("列表")
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析 JSON");
    assert_eq!(scripts["count"], 1);
    assert_eq!(scripts["scripts"][0]["name"], "demo.py");
    let removed = post(
        "/panel/api/resolver/remove",
        serde_json::json!({ "name": "demo.py" }),
    )
    .await
    .expect("删除");
    assert_eq!(removed.status(), StatusCode::OK);

    // 未知素材类型 → 404。
    assert_eq!(
        get("/panel/api/unknown/list").await.expect("请求").status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn share_pull_checks_key_and_source_ip() {
    let fixture = fixture_with_extra(
        r#"
[share]
enabled = true
role = "master"
key = "REAKEY"
trust_ips = ["127.0.0.1"]
poll_secs = 5
"#,
    )
    .await;

    let pull = |query: &str, ip: Option<&str>| {
        let mut request = fixture
            .client
            .get(format!("{}/api/share/pull{query}", fixture.base));
        if let Some(ip) = ip {
            request = request.header("x-real-ip", ip);
        }
        request.send()
    };

    // 缺 key / 错 key → 403。
    assert_eq!(
        pull("", Some("127.0.0.1")).await.expect("请求").status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        pull("?key=WRONG", Some("127.0.0.1"))
            .await
            .expect("请求")
            .status(),
        StatusCode::FORBIDDEN
    );

    // key 正确但源 IP 不在白名单 → 403。
    assert_eq!(
        pull("?key=REAKEY", Some("10.1.1.1"))
            .await
            .expect("请求")
            .status(),
        StatusCode::FORBIDDEN
    );

    // 双重门禁通过 → 200 且载荷字段齐全。
    let response = pull("?key=REAKEY", Some("127.0.0.1")).await.expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(payload["token"].as_str().map(str::len), Some(48));
    assert!(payload["bans"].is_array());
    assert!(payload["unbans"].is_array());

    // 只允许 GET/HEAD。
    let response = fixture
        .client
        .post(format!("{}/api/share/pull?key=REAKEY", fixture.base))
        .header("x-real-ip", "127.0.0.1")
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn share_disabled_returns_404_and_follower_cannot_rotate() {
    // 默认配置未启用分享 → 404。
    let fixture = fixture().await;
    let response = fixture
        .client
        .get(format!("{}/api/share/pull?key=x", fixture.base))
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // follower 角色禁止本地轮换 → 409。
    let follower = fixture_with_extra(
        r#"
[share]
enabled = true
role = "follower"
key = "REAKEY"
master_url = "http://placeholder.test"
trust_ips = ["127.0.0.1"]
poll_secs = 5
"#,
    )
    .await;
    let session = login(&follower).await;
    let response = follower
        .client
        .post(format!("{}/panel/api/security/token/rotate", follower.base))
        .header("authorization", format!("Bearer {session}"))
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn dash_tracks_scan_url_health_and_timeshift_status() {
    let fixture = fixture_with_extra(
        r#"
[[groups]]
id = "dg"
name = "DASH 组"
mode = "proxy"
access = "public"
[[groups.channels]]
id = "c1"
name = "DASH 频道"
url = "{upstream}/manifest.mpd"
"#,
    )
    .await;
    let session = login(&fixture).await;
    let auth = format!("Bearer {session}");
    let get = |path: &str| {
        fixture
            .client
            .get(format!("{}{path}", fixture.base))
            .header("authorization", auth.clone())
            .send()
    };

    let body: serde_json::Value = serde_json::from_str(
        &get("/panel/api/dash/tracks/dg/c1")
            .await
            .expect("请求")
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析 JSON");
    assert_eq!(body["count"], 2);
    assert_eq!(body["tracks"][0]["kind"], "video");
    assert_eq!(body["tracks"][0]["width"], 1280);
    assert_eq!(body["tracks"][1]["lang"], "zh");

    assert_eq!(
        get("/panel/api/dash/tracks/none/c1")
            .await
            .expect("请求")
            .status(),
        StatusCode::NOT_FOUND
    );

    let scan = serde_json::json!({ "url": format!("{}/manifest.mpd", fixture.upstream) });
    let response = fixture
        .client
        .post(format!("{}/panel/api/dash/scan-url", fixture.base))
        .header("authorization", auth.clone())
        .header("content-type", "application/json")
        .body(scan.to_string())
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("读正文")).expect("解析 JSON");
    assert_eq!(body["kind"], "static");
    assert_eq!(body["count"], 2);

    let response = fixture
        .client
        .post(format!("{}/panel/api/dash/scan-url", fixture.base))
        .header("authorization", auth.clone())
        .header("content-type", "application/json")
        .body(serde_json::json!({ "url": "file:///etc/passwd" }).to_string())
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body: serde_json::Value = serde_json::from_str(
        &get("/panel/api/health")
            .await
            .expect("请求")
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析 JSON");
    assert_eq!(body["ok"], true);
    assert!(body["channels"].as_u64().unwrap_or(0) >= 1);

    let body: serde_json::Value = serde_json::from_str(
        &get("/panel/api/dash-timeshift/status")
            .await
            .expect("请求")
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析 JSON");
    assert_eq!(body["enabled"], false);
}

#[tokio::test]
async fn dash_mode_converts_mpd_to_hls_and_proxies_segments() {
    let fixture = fixture_with_extra(
        r#"
[[groups]]
id = "dg2"
name = "DASH 转封装"
mode = "dash"
access = "open"
mpd_keep_seconds = 4

[[groups.channels]]
id = "c1"
name = "DASH 频道"
url = "{upstream}/dash/live.mpd"
"#,
    )
    .await;

    let master = fetch(&fixture, "/dg2/c1.m3u8").await;
    assert_eq!(master.status(), StatusCode::OK);
    assert_eq!(
        master
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/vnd.apple.mpegurl")
    );
    let body = master.text().await.expect("读主清单");
    assert!(body.contains("#EXT-X-STREAM-INF"), "{body}");
    assert!(body.contains("/dg2/c1/video_720p/index.txt"), "{body}");

    let media = fetch(&fixture, "/dg2/c1/video_720p/index.txt").await;
    assert_eq!(media.status(), StatusCode::OK);
    let body = media.text().await.expect("读媒体清单");
    assert!(body.contains("#EXT-X-MAP:URI="), "{body}");
    assert!(body.contains("#EXT-X-ENDLIST"), "{body}");
    // keep_seconds = 4，仅保留最后两段。
    assert_eq!(body.matches("#EXTINF:").count(), 2, "{body}");

    let session = login(&fixture).await;
    let status: serde_json::Value = serde_json::from_str(
        &fixture
            .client
            .get(format!("{}/panel/api/dash/status", fixture.base))
            .header("authorization", format!("Bearer {session}"))
            .send()
            .await
            .expect("请求")
            .text()
            .await
            .expect("读正文"),
    )
    .expect("解析状态");
    assert!(status.as_array().is_some(), "{status}");
    assert_eq!(status[0]["stream_key"], "dg2_c1");
    assert_eq!(status[0]["phase"], "ready");
    assert_eq!(status[0]["segment_count"], 2);
    assert!(status[0]["active_tracks"]
        .as_array()
        .expect("轨道数组")
        .iter()
        .any(|track| track["name"] == "video_720p" && track["kind"] == "video"));

    let segment_url = body
        .lines()
        .find(|line| {
            line.contains("/dg2/c1/video_720p/") && line.ends_with(".jpg") || line.ends_with(".png")
        })
        .expect("切片地址")
        .to_string();
    let segment = fixture
        .client
        .get(&segment_url)
        .send()
        .await
        .expect("请求切片");
    assert_eq!(segment.status(), StatusCode::OK);
    assert_eq!(
        segment.bytes().await.expect("读切片").as_ref(),
        b"dash-segment"
    );

    let init_url = body
        .lines()
        .find_map(|line| line.strip_prefix("#EXT-X-MAP:URI=\""))
        .and_then(|rest| rest.strip_suffix("\""))
        .expect("init 地址")
        .to_string();
    let init = fixture
        .client
        .get(&init_url)
        .send()
        .await
        .expect("请求 init");
    assert_eq!(init.status(), StatusCode::OK);
    assert_eq!(init.bytes().await.expect("读 init").as_ref(), b"dash-init");
}

#[tokio::test]
async fn proxy_mode_rewrites_mpd_base_url_and_proxies_segments() {
    let fixture = fixture_with_extra(
        r#"
[[groups]]
id = "mpg"
name = "代理 MPD"
mode = "proxy"
access = "open"
mpd_keep_seconds = 4
[[groups.channels]]
id = "c1"
name = "无 BaseURL"
url = "{upstream}/dash/live.mpd"
[[groups.channels]]
id = "c2"
name = "相对 BaseURL"
url = "{upstream}/dash/base.mpd"
"#,
    )
    .await;

    assert_eq!(
        fixture
            .state
            .snapshot()
            .group("mpg")
            .map(|group| group.mpd_keep_seconds),
        Some(4)
    );
    let manifest = fetch(&fixture, "/mpg/c1.mpd").await;
    assert_eq!(manifest.status(), StatusCode::OK);
    assert_eq!(
        manifest
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/dash+xml")
    );
    let body = manifest.text().await.expect("读 MPD");
    assert!(
        body.contains(&format!("<BaseURL>{}/mpg/c1/mpd/</BaseURL>", fixture.base)),
        "{body}"
    );
    assert!(
        body.contains("chunk-$RepresentationID$-$Number$.m4s"),
        "{body}"
    );
    // mpd_keep_seconds = 4：保留最后两段，并把编号从 1 推到 3。
    assert!(body.contains("startNumber=\"3\""), "{body}");
    assert!(body.contains("t=\"4000\""), "{body}");
    assert!(body.contains("r=\"1\""), "{body}");
    assert!(!body.contains("<S d=\"2000\" r=\"3\"/>"), "{body}");

    let init = fetch(&fixture, "/mpg/c1/mpd/init-v0.mp4").await;
    assert_eq!(init.status(), StatusCode::OK);
    assert_eq!(init.bytes().await.expect("读 init").as_ref(), b"dash-init");
    let segment = fetch(&fixture, "/mpg/c1/mpd/chunk-v0-1.m4s").await;
    assert_eq!(segment.status(), StatusCode::OK);
    assert_eq!(
        segment.bytes().await.expect("读切片").as_ref(),
        b"dash-segment"
    );
    // BaseURL 的查询串会随切片请求转发。
    let segment = fetch(&fixture, "/mpg/c1/mpd/chunk-v0-1.m4s?token=abc").await;
    assert_eq!(segment.status(), StatusCode::OK);

    // 已有的相对 BaseURL 改写后仍保留 `sub/`，切片继续经本服务代理。
    let body = fetch(&fixture, "/mpg/c2.mpd")
        .await
        .text()
        .await
        .expect("读 MPD");
    assert!(
        body.contains(&format!(
            "<BaseURL>{}/mpg/c2/mpd/sub/</BaseURL>",
            fixture.base
        )),
        "{body}"
    );
    let segment = fetch(&fixture, "/mpg/c2/mpd/sub/chunk-1.m4s").await;
    assert_eq!(segment.status(), StatusCode::OK);
    assert_eq!(
        segment.bytes().await.expect("读切片").as_ref(),
        b"dash-base-segment"
    );
}

#[tokio::test]
async fn dash_mode_resolver_url_is_used_for_media_playlist() {
    let fixture = fixture_with_extra(
        r#"
[[groups]]
id = "dgr"
name = "DASH 解析组"
mode = "dash"
access = "open"
[[groups.channels]]
id = "c1"
name = "DASH 解析频道"
url = "http://127.0.0.1:9/never"
resolver_script = "dash_probe.py"
resolver_ttl = 60
"#,
    )
    .await;
    let resolvers = fixture.dir.join("cache/resolvers");
    std::fs::create_dir_all(&resolvers).expect("建脚本目录");
    std::fs::write(
        resolvers.join("dash_probe.py"),
        format!("print('{}/dash/live.mpd')\n", fixture.upstream),
    )
    .expect("写脚本");

    let master = fetch(&fixture, "/dgr/c1.m3u8").await;
    assert_eq!(master.status(), StatusCode::OK);
    assert!(master
        .text()
        .await
        .expect("主清单")
        .contains("/dgr/c1/video_720p/index.txt"));

    // 媒体清单必须沿用解析脚本算出的地址，而不是频道原始死地址。
    let media = fetch(&fixture, "/dgr/c1/video_720p/index.txt").await;
    assert_eq!(media.status(), StatusCode::OK);
    assert!(media.text().await.expect("媒体清单").contains("#EXTINF:"));
}

#[tokio::test]
async fn resolver_script_overrides_channel_url_with_ttl_cache() {
    let fixture = fixture_with_extra(
        r#"
[[groups]]
id = "rg"
name = "解析组"
mode = "proxy"
access = "open"
[[groups.channels]]
id = "c1"
name = "脚本频道"
url = "http://127.0.0.1:9/never"
resolver_script = "probe.py"
resolver_arg = "MARKER_PATH"
resolver_ttl = 60
[[groups.channels]]
id = "c2"
name = "坏脚本频道"
url = "http://127.0.0.1:9/never"
resolver_script = "bad.py"
[[groups.channels]]
id = "c3"
name = "缺脚本频道"
url = "http://127.0.0.1:9/never"
resolver_script = "missing.py"
"#,
    )
    .await;

    let resolvers = fixture.dir.join("cache/resolvers");
    std::fs::create_dir_all(&resolvers).expect("建脚本目录");
    let marker = fixture.dir.join("probe.count");
    let toml = format!(
        "import sys\nopen(r'{}', 'a').write('x')\nprint('{}/live.m3u8')\n",
        marker.display(),
        fixture.upstream
    );
    std::fs::write(resolvers.join("probe.py"), toml).expect("写脚本");
    std::fs::write(resolvers.join("bad.py"), "print('not-a-url')\n").expect("写脚本");

    // 首次请求：执行脚本，返回代理清单。
    let response = fixture
        .client
        .get(format!("{}/rg/c1.m3u8", fixture.base))
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("读正文");
    assert_eq!(media_lines(&body).len(), 2);

    // 第二次请求命中 TTL 缓存：脚本只执行一次。
    let response = fixture
        .client
        .get(format!("{}/rg/c1.m3u8", fixture.base))
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(&marker)
            .expect("读计数")
            .lines()
            .count(),
        1
    );

    // 脚本输出非 URL → 302 error_url。
    let response = fixture
        .client
        .get(format!("{}/rg/c2.m3u8", fixture.base))
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("http://placeholder.test/mistake.mp4")
    );

    // 脚本缺失 → 302 error_url。
    let response = fixture
        .client
        .get(format!("{}/rg/c3.m3u8", fixture.base))
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::FOUND);
}

/// 合成 TS 流：PAT/PMT + 每 5 秒一个 IDR 关键帧。
fn ts_stream(keyframes: u64) -> Vec<u8> {
    fn packet(pid: u16, pusi: bool, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![
            0x47,
            if pusi { 0x40 } else { 0x00 } | ((pid >> 8) as u8 & 0x1f),
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

    let mut out = Vec::new();
    let pat = [
        0x00, 0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xE1, 0x00, 0, 0, 0, 0,
    ];
    out.extend(packet(0, true, &pat));
    let pmt = [
        0x00, 0x02, 0xB0, 0x12, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xE1, 0x01, 0xF0, 0x00, 0x1B, 0xE1,
        0x01, 0xF0, 0x00, 0, 0, 0, 0,
    ];
    out.extend(packet(0x100, true, &pmt));

    for index in 0..keyframes {
        let pts = index * 90_000 * 5;
        for frame in 0..5 {
            let key = frame == 0;
            let mut pes = vec![0x00, 0x00, 0x01, 0xE0, 0x00, 0x00, 0x80, 0x80, 0x05];
            pes.extend_from_slice(&pts_bytes(pts + frame * 18_000));
            pes.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, if key { 0x65 } else { 0x41 }]);
            pes.extend(std::iter::repeat(0x88).take(40));
            out.extend(packet(0x101, true, &pes));
        }
    }
    out
}

async fn wait_remux_playlist(client: &Client, url: &str) -> HttpResponse {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = client.get(url).send().await.expect("请求");
        if response.status() == StatusCode::OK || Instant::now() > deadline {
            return response;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn remux_segments_continuous_ts_and_serves_disguised_slices() {
    let fixture = fixture_with_extra(
        r#"
[[groups]]
id = "rx"
name = "Remux 组"
mode = "remux"
access = "open"
[[groups.channels]]
id = "c1"
name = "连续流"
url = "{upstream}/live.ts"
"#,
    )
    .await;

    // 首次请求一定在会话创建前 → 503 预热。
    let first = fixture
        .client
        .get(format!("{}/rx/c1.m3u8", fixture.base))
        .send()
        .await
        .expect("请求");
    assert_eq!(first.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        first
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("2")
    );
    assert_eq!(first.text().await.expect("正文"), "remux warming up");

    let response =
        wait_remux_playlist(&fixture.client, &format!("{}/rx/c1.m3u8", fixture.base)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("正文");
    assert!(body.contains("#EXT-X-TARGETDURATION:5"));
    let segments = media_lines(&body);
    assert!(!segments.is_empty());
    assert!(segments[0].starts_with("/rx/c1/rseg_"));

    let response = fixture
        .client
        .get(format!("{}{}", fixture.base, segments[0]))
        .send()
        .await
        .expect("请求切片");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        header(&response, "content-type").starts_with("image/"),
        "切片应伪装为图片"
    );
    assert_eq!(header(&response, "cache-control"), "public, max-age=30");
}

#[tokio::test]
async fn schedule_crud_and_livestream_toggle() {
    let fixture = fixture().await;
    let session = login(&fixture).await;
    let auth = format!("Bearer {session}");
    let post = |path: &str, body: serde_json::Value| {
        fixture
            .client
            .post(format!("{}{path}", fixture.base))
            .header("authorization", auth.clone())
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
    };

    let task = serde_json::json!({
        "name": "测试录制",
        "group_id": "pg",
        "channel_id": "c1",
        "start_time": "2099-01-01T00:00:00",
        "end_time": "2099-01-01T00:10:00",
        "format": "mkv"
    });
    let response = post("/panel/api/schedule", task).await.expect("新增");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["ok"], true);
    let id = body["id"].as_str().expect("id").to_string();

    let list: serde_json::Value = serde_json::from_str(
        &fixture
            .client
            .get(format!("{}/panel/api/schedule", fixture.base))
            .header("authorization", auth.clone())
            .send()
            .await
            .expect("列表")
            .text()
            .await
            .expect("正文"),
    )
    .expect("JSON");
    assert_eq!(list.as_array().map(Vec::len), Some(1));
    assert_eq!(list[0]["id"], id);
    assert_eq!(list[0]["status"], "pending");

    let response = fixture
        .client
        .put(format!("{}/panel/api/schedule/0", fixture.base))
        .header("authorization", auth.clone())
        .header("content-type", "application/json")
        .body(serde_json::json!({ "name": "改名任务" }).to_string())
        .send()
        .await
        .expect("更新");
    assert_eq!(response.status(), StatusCode::OK);

    let response = post("/panel/api/schedule/0/stop", serde_json::json!({}))
        .await
        .expect("停止");
    assert_eq!(response.status(), StatusCode::OK);

    // 未知频道 → 404。
    let bad = serde_json::json!({
        "name": "x", "group_id": "pg", "channel_id": "none",
        "start_time": "2099-01-01T00:00:00", "end_time": "2099-01-01T00:10:00"
    });
    assert_eq!(
        post("/panel/api/schedule", bad)
            .await
            .expect("新增")
            .status(),
        StatusCode::NOT_FOUND
    );

    let response = fixture
        .client
        .delete(format!("{}/panel/api/schedule/0", fixture.base))
        .header("authorization", auth.clone())
        .send()
        .await
        .expect("删除");
    assert_eq!(response.status(), StatusCode::OK);

    // 直播开关：改写频道 livestream。
    let response = post("/panel/api/livestream/pg/c1/start", serde_json::json!({}))
        .await
        .expect("点亮");
    assert_eq!(response.status(), StatusCode::OK);
    let list: serde_json::Value = serde_json::from_str(
        &fixture
            .client
            .get(format!("{}/panel/api/livestream", fixture.base))
            .header("authorization", auth.clone())
            .send()
            .await
            .expect("列表")
            .text()
            .await
            .expect("正文"),
    )
    .expect("JSON");
    assert_eq!(list["count"], 1);

    let response = post("/panel/api/livestream/pg/c1/stop", serde_json::json!({}))
        .await
        .expect("熄灭");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn epg_script_source_runs_python_and_merges_output() {
    let fixture = fixture_with_all(
        true,
        r#"
[[epg.sources]]
id = "s2"
name = "脚本源"
kind = "script"
script = "gen.py"
timeout_secs = 30
"#,
    )
    .await;

    let scripts = fixture.dir.join("cache/epg/scripts");
    std::fs::create_dir_all(&scripts).expect("建脚本目录");
    std::fs::write(
        scripts.join("gen.py"),
        r#"import datetime
now = datetime.datetime.now(datetime.timezone.utc)
fmt = "%Y%m%d%H%M%S +0000"
start = (now - datetime.timedelta(minutes=5)).strftime(fmt)
end = (now + datetime.timedelta(minutes=30)).strftime(fmt)
print(f'''<?xml version="1.0" encoding="UTF-8"?>
<tv>
<channel id="s2"><display-name>媒体清单频道</display-name></channel>
<programme start="{start}" stop="{end}" channel="s2"><title>脚本节目</title></programme>
</tv>''')
"#,
    )
    .expect("写脚本");

    let summary = epg::refresh(&fixture.state).await.expect("采集应成功");
    assert!(summary.channels >= 1, "两个源都应匹配到本地频道");
    let response = fetch(&fixture, "/epg.xml").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("正文");
    assert!(body.contains("脚本节目"), "脚本源输出应合并进节目单");
}

#[tokio::test]
async fn epg_script_source_reports_missing_script() {
    let fixture = fixture_with_all(
        false,
        r#"
[epg]
enabled = true

[[epg.sources]]
id = "s2"
name = "缺失脚本"
kind = "script"
script = "missing.py"
"#,
    )
    .await;

    let error = epg::refresh(&fixture.state).await.expect_err("应失败");
    assert!(
        format!("{error:#}").contains("脚本不存在"),
        "应报告脚本缺失：{error:#}"
    );
}

#[tokio::test]
async fn dash_schedule_rejects_static_mpd() {
    let fixture = fixture_with_extra(
        r#"
[[groups]]
id = "rgd"
name = "DASH 录制组"
mode = "dash"
access = "open"
format = "mpd"
[[groups.channels]]
id = "c1"
name = "静态 DASH"
url = "{upstream}/dash/base.mpd"
"#,
    )
    .await;
    let session = login(&fixture).await;
    let now = iptv_proxy::schedule::now_epoch();
    let task = serde_json::json!({
        "name": "静态 DASH",
        "group_id": "rgd",
        "channel_id": "c1",
        "start_time": iptv_proxy::schedule::format_local(now - 2, 8),
        "end_time": iptv_proxy::schedule::format_local(now + 8, 8),
        "format": "mkv"
    });
    let response = fixture
        .client
        .post(format!("{}/panel/api/schedule", fixture.base))
        .header("authorization", format!("Bearer {session}"))
        .header("content-type", "application/json")
        .body(task.to_string())
        .send()
        .await
        .expect("新增");
    assert_eq!(response.status(), StatusCode::OK);
    iptv_proxy::schedule::spawn(fixture.state.clone());

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let list: serde_json::Value = serde_json::from_str(
            &fixture
                .client
                .get(format!("{}/panel/api/schedule", fixture.base))
                .header("authorization", format!("Bearer {session}"))
                .send()
                .await
                .expect("列表")
                .text()
                .await
                .expect("正文"),
        )
        .expect("JSON");
        let task = &list[0];
        if task["status"] == "failed" {
            assert_eq!(task["error"], "录制区间内无可用切片");
            return;
        }
        assert_ne!(task["status"], "finished", "静态 MPD 不应录制成功");
        assert!(Instant::now() < deadline, "等待录制失败超时: {task}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn schedule_from_epg_batch_creates_tasks_and_reports_failures() {
    let fixture = fixture().await;
    let session = login(&fixture).await;
    let response = fixture
        .client
        .post(format!("{}/panel/api/schedule/from-epg", fixture.base))
        .header("authorization", format!("Bearer {session}"))
        .header("content-type", "application/json")
        .body(
            serde_json::json!({
                "items": [
                    {
                        "name": "EPG 任务", "group_id": "pg", "channel_id": "c1",
                        "start_time": "2099-01-01T00:00:00", "end_time": "2099-01-01T00:30:00",
                        "format": "mkv"
                    },
                    {
                        "name": "坏任务", "group_id": "pg", "channel_id": "none",
                        "start_time": "2099-01-01T00:00:00", "end_time": "2099-01-01T00:30:00"
                    }
                ]
            })
            .to_string(),
        )
        .send()
        .await
        .expect("批量新增");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["ok"], false);
    assert_eq!(body["created"], 1);
    assert_eq!(body["failed"][0]["error"], "Channel not found");

    let response = fixture
        .client
        .delete(format!("{}/panel/api/schedule/0", fixture.base))
        .header("authorization", format!("Bearer {session}"))
        .send()
        .await
        .expect("清理");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn subtitle_translate_endpoints_report_missing_engine() {
    let fixture = fixture().await;

    let response = fixture
        .client
        .post(format!("{}/subtitle/mtran/test", fixture.base))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "text": "hello" }).to_string())
        .send()
        .await
        .expect("自检");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .contains("MTranServer 地址"));

    let response = fixture
        .client
        .post(format!("{}/subtitle/translate/clear", fixture.base))
        .send()
        .await
        .expect("清空");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["ok"], true);

    let response = fixture
        .client
        .get(format!("{}/epg/translate/status", fixture.base))
        .send()
        .await
        .expect("状态");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["enabled"], false);

    let response = fixture
        .client
        .post(format!("{}/epg/translate/clear-engine", fixture.base))
        .send()
        .await
        .expect("清空引擎");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn subtitle_mtran_reports_connection_failure_when_configured() {
    let fixture = fixture_with_extra(
        r#"
[subtitle.translate]
url = "http://127.0.0.1:9"
"#,
    )
    .await;

    let response = fixture
        .client
        .post(format!("{}/subtitle/mtran/test", fixture.base))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "text": "hello" }).to_string())
        .send()
        .await
        .expect("自检");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["ok"], false);
    assert_eq!(body["targets"][0]["code"], "zh-Hans");
    assert!(body["targets"][0]["error"]
        .as_str()
        .unwrap_or("")
        .starts_with("请求失败"));
}

#[tokio::test]
async fn timeshift_buffers_livestream_and_serves_vod_playlist() {
    let fixture = fixture_with_extra(
        r#"
[[groups]]
id = "ts"
name = "回看组"
mode = "proxy"
access = "open"
[[groups.channels]]
id = "c1"
name = "回看频道"
url = "{upstream}/live.m3u8"
livestream = true
timeshift_hours = 4
"#,
    )
    .await;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("时钟")
        .as_secs() as i64;
    let url = format!(
        "{}/ts/c1.m3u8?start={}&end={}",
        fixture.base,
        now - 60,
        now + 60
    );

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut body;
    loop {
        let response = fixture.client.get(&url).send().await.expect("回看请求");
        assert_eq!(response.status(), StatusCode::OK);
        body = response.text().await.expect("正文");
        if body.contains("#EXT-X-PLAYLIST-TYPE:VOD") || Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        body.contains("#EXT-X-PLAYLIST-TYPE:VOD"),
        "应产出 VOD 清单：{body}"
    );
    assert!(body.contains("#EXT-X-ENDLIST"));
    assert!(body.contains("#EXT-X-TARGETDURATION:4"));
    let segment = body
        .lines()
        .find(|line| line.starts_with("/ts/c1/timeshift/"))
        .expect("回看切片路径")
        .to_string();
    assert!(segment.len() > "/ts/c1/timeshift/".len());

    let response = fixture
        .client
        .get(format!("{}{}", fixture.base, segment))
        .send()
        .await
        .expect("回看切片");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "cache-control"), "private, max-age=3600");
    assert!(header(&response, "content-type").starts_with("image/"));

    // 越界 → 落回实时清单。
    let response = fixture
        .client
        .get(format!("{}/ts/c1.m3u8?start=0&end=60", fixture.base))
        .send()
        .await
        .expect("越界请求");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("正文");
    assert!(
        !body.contains("#EXT-X-PLAYLIST-TYPE:VOD"),
        "越界应落回实时：{body}"
    );
}

#[tokio::test]
async fn playlist_config_roundtrip_and_validation() {
    let fixture = fixture().await;
    let session = login(&fixture).await;
    let auth = format!("Bearer {session}");

    let response = fixture
        .client
        .get(format!("{}/panel/api/playlist", fixture.base))
        .header("authorization", auth.clone())
        .send()
        .await
        .expect("读取");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["merge_position"], "bottom");

    let cfg = serde_json::json!({
        "merge_ttl_secs": 120,
        "merge_position": "top",
        "global_exclude": ["购物"],
        "sources": [{
            "id": "",
            "name": "上游",
            "url": format!("{}/extra.m3u8", fixture.upstream),
            "enabled": true,
            "ttl_secs": 30,
            "group_map": { "远端组": "映射组" },
            "group_mode": "map",
            "merge_group": "",
            "unlisted_groups": "keep",
            "include": [],
            "exclude": []
        }]
    });
    let response = fixture
        .client
        .put(format!("{}/panel/api/playlist", fixture.base))
        .header("authorization", auth.clone())
        .header("content-type", "application/json")
        .body(cfg.to_string())
        .send()
        .await
        .expect("保存");
    assert_eq!(response.status(), StatusCode::OK);

    let response = fixture
        .client
        .get(format!("{}/panel/api/playlist", fixture.base))
        .header("authorization", auth.clone())
        .send()
        .await
        .expect("复核");
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["merge_position"], "top");
    assert_eq!(body["merge_ttl_secs"], 120);
    assert_eq!(body["sources"][0]["name"], "上游");
    assert!(!body["sources"][0]["id"].as_str().unwrap_or("").is_empty());
    assert_eq!(body["sources"][0]["ttl_secs"], 30);
    assert_eq!(body["sources"][0]["group_mode"], "map");
    assert_eq!(body["sources"][0]["group_map"]["远端组"], "映射组");
    assert_eq!(body["sources"][0]["unlisted_groups"], "keep");

    let bad = serde_json::json!({
        "merge_ttl_secs": 60, "merge_position": "bottom",
        "global_exclude": [],
        "sources": [{ "id": "x", "name": "坏", "url": "ftp://x", "enabled": true }]
    });
    let response = fixture
        .client
        .put(format!("{}/panel/api/playlist", fixture.base))
        .header("authorization", auth)
        .header("content-type", "application/json")
        .body(bad.to_string())
        .send()
        .await
        .expect("坏配置");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn subtitle_translation_engine_translates_and_caches() {
    let extra = r#"
[subtitle.translate]
enabled = true
url = "{upstream}"
target = "zh-cn"
"#;
    let fixture = fixture_with_all(false, extra).await;
    let auth = format!("Bearer {}", login(&fixture).await);
    let directory = fixture.dir.join("cache/subtitles");
    std::fs::create_dir_all(&directory).expect("字幕目录");
    std::fs::write(
        directory.join("demo.srt"),
        "1\n00:00:01,000 --> 00:00:03,000\nHello\nWorld\n\n2\n00:00:04,000 --> 00:00:05,000\nHello\n",
    )
    .expect("写字幕");

    let response = fixture
        .client
        .get(format!(
            "{}/panel/api/subtitle/translate/status",
            fixture.base
        ))
        .header("authorization", &auth)
        .send()
        .await
        .expect("状态");
    assert_eq!(response.status(), StatusCode::OK);
    let status: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(status["enabled"], true);
    assert_eq!(status["engine"], "mtran");
    assert_eq!(status["targets"][0], "zh-cn");

    let response = fixture
        .client
        .post(format!("{}/panel/api/subtitle/translate", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "file": "demo.srt" }).to_string())
        .send()
        .await
        .expect("翻译");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["cached"], false);
    assert_eq!(body["format"], "srt");
    assert_eq!(body["lines"], 2);
    let download = body["download"].as_str().expect("下载地址").to_string();

    let text = fixture
        .client
        .get(format!("{}{download}", fixture.base))
        .header("authorization", &auth)
        .send()
        .await
        .expect("下载")
        .text()
        .await
        .expect("正文");
    assert!(
        text.contains(
            "00:00:01,000 --> 00:00:03,000\nHello\n[zh-cn]译:Hello\nWorld\n[zh-cn]译:World\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("2\n00:00:04,000 --> 00:00:05,000\nHello\n[zh-cn]译:Hello\n"),
        "{text}"
    );

    let response = fixture
        .client
        .post(format!("{}/panel/api/subtitle/translate", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "file": "demo.srt" }).to_string())
        .send()
        .await
        .expect("再次翻译");
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["cached"], true);
}

#[tokio::test]
async fn subtitle_translation_accepts_mtran_v5_and_token() {
    let extra = r#"
[subtitle.translate]
enabled = true
url = "{upstream}/v5"
token = "demo-token"
target = "zh-cn"
"#;
    let fixture = fixture_with_all(false, extra).await;
    let auth = format!("Bearer {}", login(&fixture).await);
    let directory = fixture.dir.join("cache/subtitles");
    std::fs::create_dir_all(&directory).expect("字幕目录");
    std::fs::write(
        directory.join("v5.srt"),
        "1\n00:00:01,000 --> 00:00:03,000\nHello\n",
    )
    .expect("写字幕");

    let response = fixture
        .client
        .post(format!("{}/panel/api/subtitle/translate", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "file": "v5.srt" }).to_string())
        .send()
        .await
        .expect("翻译");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    let download = body["download"].as_str().expect("下载地址").to_string();
    let text = fixture
        .client
        .get(format!("{}{download}", fixture.base))
        .header("authorization", &auth)
        .send()
        .await
        .expect("下载")
        .text()
        .await
        .expect("正文");
    assert!(text.contains("[zh-cn]译:Hello"), "{text}");

    let response = fixture
        .client
        .post(format!("{}/subtitle/mtran/test", fixture.base))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "text": "Hello" }).to_string())
        .send()
        .await
        .expect("自检");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["ok"], true);
    assert_eq!(body["result"], "[zh-cn]译:Hello");
}

#[tokio::test]
async fn subtitle_mtran_falls_back_to_single_endpoint() {
    let fixture = fixture_with_extra(
        r#"
[subtitle.translate]
url = "{upstream}/single"
"#,
    )
    .await;

    let response = fixture
        .client
        .post(format!("{}/subtitle/mtran/test", fixture.base))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "text": "Hello" }).to_string())
        .send()
        .await
        .expect("自检");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["ok"], true);
    assert_eq!(body["result"], "[zh-cn]译:Hello");
}

#[tokio::test]
async fn subtitle_translation_converts_ass_to_vtt() {
    let extra = r#"
[subtitle.translate]
enabled = true
url = "{upstream}"
target = "zh-cn"
"#;
    let fixture = fixture_with_all(false, extra).await;
    let auth = format!("Bearer {}", login(&fixture).await);
    let directory = fixture.dir.join("cache/subtitles");
    std::fs::create_dir_all(&directory).expect("字幕目录");
    std::fs::write(
        directory.join("demo.ass"),
        "[Script Info]\nScriptType: v4.00+\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:03.50,Default,,0,0,0,,{\\i1}Hello\\NWorld\n",
    )
    .expect("写字幕");

    let response = fixture
        .client
        .post(format!("{}/panel/api/subtitle/translate", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "file": "demo.ass" }).to_string())
        .send()
        .await
        .expect("翻译");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["format"], "vtt");
    let download = body["download"].as_str().expect("下载地址").to_string();
    let text = fixture
        .client
        .get(format!("{}{download}", fixture.base))
        .header("authorization", &auth)
        .send()
        .await
        .expect("下载")
        .text()
        .await
        .expect("正文");
    assert!(text.starts_with("WEBVTT\n\n"), "{text}");
    assert!(
        text.contains(
            "00:00:01.000 --> 00:00:03.500\nHello\n[zh-cn]译:Hello\nWorld\n[zh-cn]译:World\n"
        ),
        "{text}"
    );
    assert!(!text.contains("{\\i1}"), "{text}");
}

#[tokio::test]
async fn subtitle_translation_requires_engine_config() {
    let fixture = fixture().await;
    let auth = format!("Bearer {}", login(&fixture).await);
    let response = fixture
        .client
        .post(format!("{}/panel/api/subtitle/translate", fixture.base))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "file": "demo.srt" }).to_string())
        .send()
        .await
        .expect("请求");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn epg_translate_endpoints_report_unconfigured_engine() {
    let fixture = fixture().await;
    let auth = login(&fixture).await;

    let response = fixture
        .client
        .get(format!("{}/epg/translate/models", fixture.base))
        .send()
        .await
        .expect("模型");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["models"].as_array().map(Vec::len), Some(0));

    // 面板 `api()` 使用 `/panel/api` 前缀，别名必须同样可用。
    let response = fixture
        .client
        .get(format!("{}/panel/api/epg/translate/models", fixture.base))
        .header("authorization", format!("Bearer {auth}"))
        .send()
        .await
        .expect("面板模型别名");
    assert_eq!(response.status(), StatusCode::OK);

    for path in ["/epg/translate/run", "/epg/translate/probe"] {
        let response = fixture
            .client
            .post(format!("{}{path}", fixture.base))
            .header("authorization", format!("Bearer {auth}"))
            .header("content-type", "application/json")
            .body(serde_json::json!({ "text": "hello" }).to_string())
            .send()
            .await
            .expect("请求");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
    }

    let response = fixture
        .client
        .post(format!("{}/epg/translate/mtran/test", fixture.base))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "text": "hello" }).to_string())
        .send()
        .await
        .expect("自检");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = fixture
        .client
        .post(format!("{}/epg/translate/clear", fixture.base))
        .send()
        .await
        .expect("清空");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn epg_translation_engine_appends_target_texts_and_reuses_cache() {
    let extra = r#"
[epg.translate]
enabled = true
url = "{upstream}"
target = "zh-cn"
source = "auto"
"#;
    let fixture = fixture_with_all(true, extra).await;
    let auth = login(&fixture).await;

    let response = fixture
        .client
        .post(format!("{}/panel/api/epg/refresh", fixture.base))
        .header("authorization", format!("Bearer {auth}"))
        .send()
        .await
        .expect("采集");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert!(
        body["translated"].as_u64().unwrap_or(0) > 0,
        "应写入译文：{body}"
    );

    let xml = fixture
        .client
        .get(format!("{}/epg.xml", fixture.base))
        .send()
        .await
        .expect("取节目单")
        .text()
        .await
        .expect("正文");
    assert!(
        xml.contains(r#"<title lang="zh-cn">[zh-cn]译:新闻</title>"#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<desc lang="zh-cn">[zh-cn]译:早间新闻</desc>"#),
        "{xml}"
    );

    let cache = fixture.dir.join("cache/epg/.translate/cache.json");
    let entries: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cache).expect("缓存文件")).expect("JSON");
    assert_eq!(entries.as_object().map(|map| map.len()), Some(2));

    let response = fixture
        .client
        .post(format!("{}/epg/translate/run", fixture.base))
        .header("authorization", format!("Bearer {auth}"))
        .send()
        .await
        .expect("翻译");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["ok"], true);
    assert!(body["translated"].as_u64().unwrap_or(0) > 0);
}

#[tokio::test]
async fn logo_animate_generates_apng_when_ffmpeg_is_available() {
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_err()
        || std::process::Command::new("ffprobe")
            .arg("-version")
            .output()
            .is_err()
    {
        return;
    }
    let fixture = fixture().await;
    let token = login(&fixture).await;
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let response = fixture
        .client
        .post(format!("{}/panel/api/logo/upload", fixture.base))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(
            serde_json::json!({
                "items": [{ "name": "anim.png", "data": format!("data:image/png;base64,{png}") }]
            })
            .to_string(),
        )
        .send()
        .await
        .expect("上传");
    assert_eq!(response.status(), StatusCode::OK);

    let response = fixture
        .client
        .post(format!("{}/panel/api/logo/animate", fixture.base))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(
            serde_json::json!({
                "file": "anim.png",
                "effect": "spot",
                "quality": "80"
            })
            .to_string(),
        )
        .send()
        .await
        .expect("动效");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    assert_eq!(body["ok"], true);
    assert!(body["frames"].as_u64().unwrap_or(0) > 1);
    assert!(body["bytes_after"].as_u64().unwrap_or(0) > 0);

    let bytes = std::fs::read(fixture.dir.join("cache/logos/anim.png")).expect("读动图");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(bytes.windows(4).any(|window| window == b"acTL"));
    let check = fixture.dir.join("check.png");
    std::fs::write(&check, &bytes).expect("写校验图");
    assert!(
        std::process::Command::new("ffprobe")
            .args(["-v", "error"])
            .arg(&check)
            .status()
            .expect("ffprobe")
            .success(),
        "APNG 应可被 ffprobe 解码"
    );
    assert!(fixture.dir.join("cache/logos/.orig/anim.png").is_file());

    let response = fixture
        .client
        .get(format!("{}/panel/api/logo/list", fixture.base))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("列表");
    let list: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("正文")).expect("JSON");
    let item = list["logos"]
        .as_array()
        .expect("logos")
        .iter()
        .find(|item| item["file"] == "anim.png")
        .expect("应列出动图");
    assert_eq!(item["animated"], true);
    assert_eq!(item["has_original"], true);
}
