//! HTTP 层：路由、提取器、错误映射、静态资源。

pub mod error;
pub mod ext;
pub mod guard;
pub mod routes;
pub mod static_assets;

use std::sync::Arc;

use axum::{
    routing::{any, get, post, put},
    Router,
};

use crate::state::AppState;

/// 组装路由表。
///
/// M0：面板与公开页。M1：分组/频道接口与订阅导出。
/// 播放面（`/{gid}/{cid}.{ext}`、切片）在 M2 接入。
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // 公开面
        .route("/health", get(routes::public::health))
        .route("/", get(routes::public::home))
        .route("/epg.xml", get(routes::epg::xml))
        .route("/epg.xml.gz", get(routes::epg::xml_gz))
        .route("/epg/api/channels", get(routes::epg::api_channels))
        .route(
            "/epg/api/programmes/:channel",
            get(routes::epg::api_programmes),
        )
        .route("/epg/api/search", get(routes::epg::api_search))
        .route("/logo/api/list", get(routes::logo::public_list))
        .route("/logo/:file", get(routes::logo::file))
        .route("/unban/whoami", get(routes::public::whoami))
        .route("/api/share/pull", get(routes::share::pull))
        .route("/subs/status", get(routes::subs::status))
        .route("/subs/groups", post(routes::subs::groups))
        .route("/subs/preview", post(routes::subs::preview))
        .route("/playlist.m3u", get(routes::play::total_playlist))
        .route("/:gid/playlist.m3u", get(routes::play::group_playlist))
        // 面板接口：未知路径固定 404 JSON，先于播放通配路由匹配。
        .route("/panel/api/*rest", any(routes::panel::api_missing))
        // 播放链路（M2）
        .route("/:gid/:file", get(routes::stream::entry))
        .route("/:gid/:cid/*path", get(routes::stream::wildcard))
        // 内嵌页面（免登录）
        .route("/panel", get(static_assets::panel_page))
        .route("/panel/", get(static_assets::panel_page))
        .route("/panel/epg", get(routes::epg::viewer))
        .route("/epg", get(routes::epg::viewer))
        .route("/logo", get(static_assets::logo_page))
        .route(
            "/unban",
            get(static_assets::unban_page).post(routes::public::unban),
        )
        .route("/__assets/*path", get(static_assets::asset))
        .route("/assets/*path", get(routes::media::served))
        // 面板接口（需要会话）
        .route("/panel/api/login", post(routes::panel::login))
        .route("/panel/api/logout", post(routes::panel::logout))
        .route("/panel/api/status", get(routes::panel::status))
        .route("/panel/api/health", get(routes::dash::health))
        .route("/panel/api/subs/status", get(routes::subs::panel_status))
        .route("/panel/api/subs/groups", post(routes::subs::panel_groups))
        .route("/panel/api/subs/preview", post(routes::subs::panel_preview))
        .route(
            "/panel/api/schedule",
            get(routes::schedule::list).post(routes::schedule::create),
        )
        .route(
            "/panel/api/schedule/from-epg",
            post(routes::schedule::from_epg),
        )
        .route(
            "/panel/api/schedule/download/:filename",
            get(routes::schedule::download),
        )
        .route(
            "/panel/api/schedule/:idx",
            put(routes::schedule::update).delete(routes::schedule::remove),
        )
        .route(
            "/panel/api/schedule/:idx/stop",
            post(routes::schedule::stop),
        )
        .route(
            "/panel/api/livestream",
            get(routes::schedule::livestream_list),
        )
        .route(
            "/panel/api/livestream/:gid/:cid/:action",
            post(routes::schedule::livestream_switch),
        )
        .route(
            "/panel/api/playlist",
            get(routes::playlist::get).put(routes::playlist::update),
        )
        .route("/panel/api/dash/status", get(routes::dash::status))
        .route(
            "/panel/api/dash/tracks/:gid/:cid",
            get(routes::dash::tracks),
        )
        .route("/panel/api/dash/scan-url", post(routes::dash::scan_url))
        .route(
            "/panel/api/dash-timeshift/status",
            get(routes::dash::timeshift_status),
        )
        .route(
            "/panel/api/config",
            get(routes::panel::get_config).put(routes::panel::put_config),
        )
        .route("/panel/api/reload", post(routes::panel::reload))
        .route("/panel/api/cache/stats", get(routes::admin::cache_stats))
        .route("/panel/api/cache/flush", post(routes::admin::cache_flush))
        .route("/panel/api/stats", get(routes::admin::stats))
        .route("/panel/api/logs", get(routes::admin::logs))
        .route("/panel/api/security/token", get(routes::security::token))
        .route("/panel/api/link-info", get(routes::security::link_info))
        .route(
            "/panel/api/security/token/rotate",
            post(routes::security::token_rotate),
        )
        .route(
            "/panel/api/security/ip/stats",
            get(routes::security::ip_stats),
        )
        .route("/panel/api/security/ip/ban", post(routes::security::ip_ban))
        .route(
            "/panel/api/security/ip/unban",
            post(routes::security::ip_unban),
        )
        .route(
            "/panel/api/security/ip/unban-batch",
            post(routes::security::ip_unban),
        )
        .route("/panel/api/backup", get(routes::backup::backup))
        .route(
            "/panel/api/restore",
            post(routes::backup::restore)
                .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route(
            "/panel/api/assets/ffmpeg-status",
            get(routes::media::ffmpeg_status),
        )
        .route("/panel/api/assets/list", get(routes::media::list))
        .route("/panel/api/assets/convert", post(routes::media::convert))
        .route("/panel/api/assets/upload", post(routes::media::upload))
        .route("/panel/api/assets/remove", post(routes::media::remove))
        .route("/panel/api/assets/raw/:file", get(routes::media::raw))
        .route("/panel/api/subtitle/list", get(routes::media::list))
        .route("/panel/api/subtitle/upload", post(routes::media::upload))
        .route("/panel/api/subtitle/remove", post(routes::media::remove))
        .route("/panel/api/subtitle/raw/:file", get(routes::media::raw))
        .route(
            "/panel/api/subtitle/translate/status",
            get(routes::media::translate_status),
        )
        .route(
            "/panel/api/subtitle/translate",
            post(routes::media::translate_subtitle),
        )
        .route(
            "/panel/api/subtitle/translate/raw/:name",
            get(routes::media::translate_raw),
        )
        .route("/subtitle/mtran/test", post(routes::media::mtran_test))
        .route(
            "/subtitle/translate/clear",
            post(routes::media::translate_clear),
        )
        .route("/epg/translate/status", get(routes::epg::translate_status))
        .route("/epg/translate/models", get(routes::epg::translate_models))
        .route("/epg/translate/run", post(routes::epg::translate_run))
        .route("/epg/translate/probe", post(routes::epg::translate_probe))
        .route(
            "/epg/translate/mtran/test",
            post(routes::epg::translate_mtran),
        )
        .route(
            "/epg/translate/clear",
            post(routes::epg::translate_clear_results),
        )
        .route(
            "/epg/translate/clear-engine",
            post(routes::epg::translate_clear),
        )
        .route(
            "/panel/api/resolver/scripts",
            get(routes::media::resolver_list),
        )
        .route(
            "/panel/api/resolver/script",
            post(routes::media::resolver_save),
        )
        .route(
            "/panel/api/resolver/remove",
            post(routes::media::resolver_remove),
        )
        .route("/panel/api/logo/list", get(routes::logo::panel_list))
        .route("/panel/api/logo/upload", post(routes::logo::upload))
        .route("/panel/api/logo/delete", post(routes::logo::delete))
        .route("/panel/api/logo/rename", post(routes::logo::rename))
        .route("/panel/api/logo/restore", post(routes::logo::restore))
        .route("/panel/api/logo/animate", post(routes::logo::animate))
        .route(
            "/panel/api/logo/match-report",
            get(routes::logo::match_report),
        )
        .route("/panel/api/logo/apply", post(routes::logo::apply))
        .route("/panel/api/logo/suggest", post(routes::logo::suggest))
        // 面板把 `api()` 的路径拼在 `/panel/api` 后，EPG 翻译端点也要有面板别名。
        .route(
            "/panel/api/epg/translate/status",
            get(routes::epg::translate_status),
        )
        .route(
            "/panel/api/epg/translate/models",
            get(routes::epg::translate_models),
        )
        .route(
            "/panel/api/epg/translate/run",
            post(routes::epg::translate_run),
        )
        .route(
            "/panel/api/epg/translate/probe",
            post(routes::epg::translate_probe),
        )
        .route(
            "/panel/api/epg/translate/mtran/test",
            post(routes::epg::translate_mtran),
        )
        .route(
            "/panel/api/epg/translate/clear",
            post(routes::epg::translate_clear_results),
        )
        .route(
            "/panel/api/epg/translate/clear-engine",
            post(routes::epg::translate_clear),
        )
        .route(
            "/panel/api/subtitle/translate/clear",
            post(routes::media::translate_clear),
        )
        .route(
            "/panel/api/subtitle/mtran/test",
            post(routes::media::mtran_test),
        )
        .route("/panel/api/epg/status", get(routes::epg::status))
        .route("/panel/api/epg/refresh", post(routes::epg::refresh))
        .route("/panel/api/epg/regenerate", post(routes::epg::regenerate))
        .route("/panel/api/epg/mappings", get(routes::epg::mappings))
        .route("/panel/api/epg/mapping", post(routes::epg::mapping))
        .route("/panel/api/epg/extra", post(routes::epg::extra))
        .route("/panel/api/epg/scripts", get(routes::epg::scripts_list))
        .route("/panel/api/epg/script", post(routes::epg::script_save))
        .route(
            "/panel/api/epg/script-delete",
            post(routes::epg::script_delete),
        )
        .route(
            "/panel/api/epg/cron-preview",
            post(routes::epg::cron_preview),
        )
        .route(
            "/panel/api/epg/programmes/:channel",
            get(routes::epg::programmes),
        )
        .route(
            "/panel/api/epg/suggest/:gid/:cid",
            get(routes::epg::suggest),
        )
        .route("/panel/api/epg/search", get(routes::epg::search))
        .route(
            "/panel/api/epg/viewer-channels",
            get(routes::epg::viewer_channels),
        )
        .route(
            "/panel/api/epg/source-channels/:id",
            get(routes::epg::source_channels),
        )
        .route(
            "/panel/api/epg/tz-preview/:id",
            get(routes::epg::tz_preview),
        )
        .route(
            "/panel/api/epg/match-report",
            get(routes::epg::match_report),
        )
        .route("/panel/api/groups", post(routes::groups::create_group))
        .route("/panel/api/groups/export", get(routes::groups::export_all))
        .route(
            "/panel/api/groups/import",
            post(routes::groups::import_groups),
        )
        .route(
            "/panel/api/groups/import/force",
            post(routes::groups::import_groups_force),
        )
        .route(
            "/panel/api/groups/import/m3u",
            post(routes::groups::import_m3u),
        )
        .route(
            "/panel/api/groups/:gid",
            put(routes::groups::update_group).delete(routes::groups::delete_group),
        )
        .route(
            "/panel/api/groups/:gid/export",
            get(routes::groups::export_group),
        )
        .route(
            "/panel/api/groups/:gid/channels",
            post(routes::groups::create_channel),
        )
        .route(
            "/panel/api/groups/:gid/channels/:cid",
            put(routes::groups::update_channel).delete(routes::groups::delete_channel),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            routes::admin::count_request,
        ))
        .fallback(routes::public::fallback)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            guard::ip_guard,
        ))
        .with_state(state)
        .layer(tower_http::cors::CorsLayer::permissive())
}
