//! 上游抓取与切片伪装（M2）。
//!
//! 契约见 [`docs/design/03-http-contract.md`](../docs/design/03-http-contract.md) §4。
//! 证据：F-004、F-006、F-017。

pub mod media;
pub mod modes;
pub mod segments;

pub use media::{parse_segment, prefix, prefix_matches, rewrite_media};
pub use modes::AddressCache;
pub use segments::{save_index, segment_path, SegmentIndex};

use std::{
    collections::HashMap,
    sync::{Mutex, RwLock},
    time::{Duration, Instant},
};

/// 带 TTL 的小缓存。用于缓存在线清单。
pub struct TtlCache {
    ttl: RwLock<Duration>,
    inner: Mutex<HashMap<String, (Instant, String)>>,
}

impl TtlCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl: RwLock::new(ttl),
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// 更新 TTL；已有条目按新 TTL 判定。
    pub fn set_ttl(&self, ttl: Duration) {
        *self.ttl.write().unwrap_or_else(|e| e.into_inner()) = ttl;
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let ttl = *self.ttl.read().unwrap_or_else(|e| e.into_inner());
        self.get_with_ttl(key, ttl)
    }

    /// 按条目自带 TTL 读取；`ttl` 为零表示不按时间失效。
    pub fn get_with_ttl(&self, key: &str, ttl: Duration) -> Option<String> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let (at, value) = guard.get(key)?;
        if ttl.is_zero() || at.elapsed() < ttl {
            Some(value.clone())
        } else {
            None
        }
    }

    pub fn put(&self, key: &str, value: String) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(key.to_string(), (Instant::now(), value));
    }

    /// 清空缓存。
    pub fn clear(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// 当前条目数。
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}
