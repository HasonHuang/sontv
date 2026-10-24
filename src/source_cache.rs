//! 订阅源持久缓存。
//!
//! 目标 v0.8.2 的聚合源正文与状态可跨进程重启保留（F-020）。
//! 本模块把正文和状态写入 `<cache>/subs/`：
//!
//! - `sub_url/<sha256(key)>.json`：源正文 + 抓取时间；
//! - `subs/status.json`：每个源最近一次抓取状态。
//!
//! 时间用 Unix 秒而不是 `Instant`，重启后仍可比较。

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::subs::SourceStatus;

/// 一条源正文缓存。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    key: String,
    body: String,
    stored_at: u64,
}

/// 订阅源缓存：正文 + 状态。
pub struct SourceCache {
    dir: PathBuf,
    status_path: PathBuf,
    ttl: RwLock<Duration>,
    entries: Mutex<HashMap<String, Record>>,
    status: Mutex<HashMap<String, SourceStatus>>,
    write_lock: Mutex<()>,
}

impl SourceCache {
    /// 从磁盘加载；损坏条目跳过，不阻塞启动。
    pub fn load(cache_dir: &Path, ttl: Duration) -> Self {
        let dir = cache_dir.join("sub_url");
        let status_path = cache_dir.join("subs").join("status.json");
        let _ = fs::create_dir_all(&dir);

        let mut entries = HashMap::new();
        if let Ok(read) = fs::read_dir(&dir) {
            for entry in read.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let Ok(bytes) = fs::read(&path) else {
                    continue;
                };
                let Ok(record) = serde_json::from_slice::<Record>(&bytes) else {
                    continue;
                };
                if !record.key.is_empty() {
                    entries.insert(record.key.clone(), record);
                }
            }
        }

        let status = fs::read(&status_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<HashMap<String, SourceStatus>>(&bytes).ok())
            .unwrap_or_default();

        Self {
            dir,
            status_path,
            ttl: RwLock::new(ttl),
            entries: Mutex::new(entries),
            status: Mutex::new(status),
            write_lock: Mutex::new(()),
        }
    }

    /// 更新源缓存默认 TTL。
    pub fn set_ttl(&self, ttl: Duration) {
        *self.ttl.write().unwrap_or_else(|error| error.into_inner()) = ttl;
    }

    /// 按源级 TTL 读取；`ttl` 为零表示不按时间失效。
    pub fn get_with_ttl(&self, key: &str, ttl: Duration) -> Option<String> {
        let guard = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let record = guard.get(key)?;
        if ttl.is_zero() || now_secs().saturating_sub(record.stored_at) < ttl.as_secs() {
            Some(record.body.clone())
        } else {
            None
        }
    }

    /// 写入正文；`stored_at` 为当前 Unix 秒。
    pub fn put(&self, key: &str, body: String) {
        let record = Record {
            key: key.to_string(),
            body,
            stored_at: now_secs(),
        };
        {
            let mut guard = self
                .entries
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            guard.insert(key.to_string(), record.clone());
        }
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let path = self.dir.join(format!("{}.json", digest(key)));
        let _ = write_atomic(&path, &serde_json::to_vec(&record).unwrap_or_default());
    }

    /// 当前正文条目数。
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }

    /// 清空正文缓存；状态保留。
    pub fn clear(&self) {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        entries.clear();
        if let Ok(read) = fs::read_dir(&self.dir) {
            for entry in read.flatten() {
                if entry.path().extension().and_then(|ext| ext.to_str()) == Some("json") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    /// 所有源状态快照。
    pub fn statuses(&self) -> HashMap<String, SourceStatus> {
        self.status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// 记录抓取成功；`last_try` 与 `last_ok` 都更新为当前时间。
    pub fn mark_success(&self, id: &str) {
        self.update_status(id, |status| {
            let now = now_secs();
            status.last_try = now;
            status.last_ok = now;
            status.error = None;
        });
    }

    /// 记录抓取失败；保留上次成功时间与频道数。
    pub fn mark_error(&self, id: &str, error: String) {
        self.update_status(id, |status| {
            status.last_try = now_secs();
            status.error = Some(error);
        });
    }

    /// 更新最近一次聚合的频道数。
    pub fn set_channels(&self, id: &str, channels: usize) {
        self.update_status(id, |status| status.channels = channels);
    }

    fn update_status<F>(&self, id: &str, update: F)
    where
        F: FnOnce(&mut SourceStatus),
    {
        let snapshot = {
            let mut guard = self
                .status
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let status = guard.entry(id.to_string()).or_default();
            update(status);
            guard.clone()
        };
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _ = write_atomic(
            &self.status_path,
            &serde_json::to_vec(&snapshot).unwrap_or_default(),
        );
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

fn digest(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_body_and_status_across_reload() {
        let dir = std::env::temp_dir().join(format!("source-cache-{}", uuid::Uuid::new_v4()));
        let key = "sub:s1:http://up/source.m3u";

        {
            let cache = SourceCache::load(&dir, Duration::from_secs(60));
            cache.put(key, "#EXTM3U\n".into());
            cache.mark_success("s1");
            cache.set_channels("s1", 3);
            assert_eq!(
                cache.get_with_ttl(key, Duration::from_secs(60)).as_deref(),
                Some("#EXTM3U\n")
            );
        }

        let cache = SourceCache::load(&dir, Duration::from_secs(60));
        assert_eq!(
            cache.get_with_ttl(key, Duration::from_secs(60)).as_deref(),
            Some("#EXTM3U\n")
        );
        let status = cache.statuses();
        let status = status.get("s1").expect("状态应保留");
        assert_eq!(status.channels, 3);
        assert!(status.last_ok > 0);
        assert!(status.last_try >= status.last_ok);
        assert!(status.error.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_removes_body_but_keeps_status() {
        let dir = std::env::temp_dir().join(format!("source-cache-{}", uuid::Uuid::new_v4()));
        let key = "sub:s1:http://up/source.m3u";
        let cache = SourceCache::load(&dir, Duration::from_secs(60));
        cache.put(key, "body".into());
        cache.mark_success("s1");
        cache.set_channels("s1", 2);
        cache.clear();
        assert!(cache.get_with_ttl(key, Duration::from_secs(60)).is_none());
        assert_eq!(cache.statuses().get("s1").expect("状态").channels, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expired_entry_is_not_returned() {
        let dir = std::env::temp_dir().join(format!("source-cache-{}", uuid::Uuid::new_v4()));
        let key = "sub:s1:http://up/source.m3u";
        let cache = SourceCache::load(&dir, Duration::from_secs(60));
        cache.put(key, "body".into());
        {
            let mut entries = cache
                .entries
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            entries.get_mut(key).expect("条目").stored_at = now_secs().saturating_sub(10);
        }
        assert!(cache.get_with_ttl(key, Duration::from_secs(1)).is_none());
        assert!(cache.get_with_ttl(key, Duration::from_secs(0)).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
