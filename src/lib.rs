//! IPTV Proxy 库根：与目标 v0.8.2 兼容的重实现。
//!
//! 模块划分见 [`docs/design/02-architecture.md`](../docs/design/02-architecture.md)。
//! 可执行入口在 `src/main.rs`，只负责启动。

pub mod assets;
pub mod auth;
pub mod backup;
pub mod config;
pub mod dash;
pub mod dash_hls;
pub mod epg;
pub mod http;
pub mod logging;
pub mod logo;
pub mod logo_anim;
pub mod m3u;
pub mod playlist;
pub mod proxy;
pub mod reload;
pub mod remux;
pub mod resolver;
pub mod schedule;
pub mod share;
pub mod source_cache;
pub mod state;
pub mod stats;
pub mod subs;
pub mod subtitle;
pub mod timeshift;
pub mod translate;
pub mod watchdog;
