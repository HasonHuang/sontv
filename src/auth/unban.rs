//! 自助解封配额。
//!
//! 证据：S-EP-07、S-AUTH-07。`/unban` 免登录，每个 IP 每月限 3 次。
//! 计数保存在内存；进程重启后清零。

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

/// 每个 IP 的窗口上限。
pub const MONTHLY_LIMIT: u32 = 3;
/// 窗口长度（30 天）。
pub const WINDOW_SECS: u64 = 30 * 24 * 3600;

/// 按 IP 记录自助解封次数。
#[derive(Debug, Default)]
pub struct UnbanQuota {
    inner: Mutex<HashMap<String, Vec<Instant>>>,
}

impl UnbanQuota {
    pub fn new() -> Self {
        Self::default()
    }

    /// 窗口内已用次数。
    pub fn used(&self, ip: &str) -> u32 {
        let mut inner = self.lock();
        Self::prune(inner.entry(ip.to_string()).or_default()) as u32
    }

    /// 剩余次数。
    pub fn remaining(&self, ip: &str) -> u32 {
        MONTHLY_LIMIT.saturating_sub(self.used(ip))
    }

    /// 消耗一次；配额不足时返回 `false`。
    pub fn consume(&self, ip: &str) -> bool {
        let mut inner = self.lock();
        let attempts = inner.entry(ip.to_string()).or_default();
        let used = Self::prune(attempts);
        if used >= MONTHLY_LIMIT as usize {
            return false;
        }
        attempts.push(Instant::now());
        true
    }

    fn prune(attempts: &mut Vec<Instant>) -> usize {
        let window = Duration::from_secs(WINDOW_SECS);
        attempts.retain(|at| at.elapsed() < window);
        attempts.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Vec<Instant>>> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_three_attempts() {
        let quota = UnbanQuota::new();
        assert_eq!(quota.remaining("1.2.3.4"), 3);
        assert!(quota.consume("1.2.3.4"));
        assert!(quota.consume("1.2.3.4"));
        assert!(quota.consume("1.2.3.4"));
        assert!(!quota.consume("1.2.3.4"));
        assert_eq!(quota.remaining("1.2.3.4"), 0);
        assert_eq!(quota.used("1.2.3.4"), 3);
        assert_eq!(quota.used("2.2.2.2"), 0);
    }

    #[test]
    fn quota_is_per_ip() {
        let quota = UnbanQuota::new();
        assert!(quota.consume("1.1.1.1"));
        assert_eq!(quota.remaining("2.2.2.2"), 3);
    }
}
