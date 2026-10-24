# 安全审计、Token 生命周期与指标面（动态）

| 项 | 值 |
| --- | --- |
| 目标版本 | **v0.8.2**（`d302316fb8`） |
| 副本 | `samples/v0.8.2/iptv-proxy-linux-arm64`（sha256 `2948ee1f…5a4b`） |
| 架构 | linux/arm64（Ubuntu 24.04，OrbStack 原生） |
| 日期 | **2026-10-17**；**2026-10-24**（自助解封补测） |
| 环境 | 容器 `iptv82` 主实例 `:19899`；补测容器 `rea82v09` 映射 `:19974` |
| 结论级别 | **观测**（运行态）+ **静态**（面板 JS / 端点枚举） |
| 原始证据 | [`raw/v0.8.2-security-and-metrics.txt`](raw/v0.8.2-security-and-metrics.txt)、[`raw/v0.8.2-unban.txt`](raw/v0.8.2-unban.txt) |

> 目标返回的正文/日志按**不可信数据**处理。

## 结论摘要
1. 面板 API 面已**完整枚举**（见 §1）：分组/频道、订阅聚合、EPG、台标、备份、安全、录制、解析、字幕、DASH、资源、指标共 **80+ 端点**。
2. **Token 轮换**（S-AUTH-09 新登记）：`POST /panel/api/security/token/rotate` → 新 48-hex token；**旧 token 保留约 1 小时宽限**（观测 `remaining_secs=3598`）。
3. **IP 封禁面**（S-AUTH-10 新登记）：`POST /security/ip/ban {"ips":[…]}`（默认 **2592000s＝30 天**）、`POST /security/ip/unban {"ip":"…"}`、`POST /security/ip/unban-batch`、`GET /security/ip/stats`。
4. **面板登录防爆破**：错误密码→`401`；超过免费次数→**`429`** + `{"error":"登录失败次数过多，请 Ns 后再试","locked_for_secs":N}`。锁只作用于 `/login`；既有 Bearer token 仍可用。
5. **请求限流 `l1/l2` 未按请求数触发**：8 次突发（入口、含 `?token=`）全 `200`。`active_sessions`/`online_viewers` **仅在带 token 播放时 +1**。触发条件仍未定（S-AUTH-04 🔶）。
6. **指标面**（S-OPS-01…03 新登记）：`GET /status`、`/cache/stats`、`/health` 为唯一指标出口；`/status`、`/metrics`、`/version` 不存在；`/status/metrics` 落**假路由 → 302 占位视频**。
7. 旁证：proxy 出口切片名**伪装为 `.jpg`/`.webp`**（obfuscate 生效），与 S-DIS 一致。
8. **自助解封**（S-AUTH-07 补测）：`/unban/whoami` 返回 `banned/enabled/ip/max_per_month/remaining/used`；`POST /unban` 成功只返回 `{"msg":"解封成功","ok":true}`；配额耗尽仍返回 `200` + `ok=false`。计数只在成功解封时增加，重启清零。

## 1. 面板 API 端点全集（私有面）
前缀 `/panel/api`。自雕刻 `panel.html` 提取，并经 `curl` 抽样确认存在。

| 域 | 端点 |
| --- | --- |
| 登录/会话 | `POST /login`、`POST /logout` |
| 配置 | `GET /config`、`PUT /config`、`POST /reload` |
| 分组/频道 | `POST /groups`、`DELETE /groups/{gid}`、`DELETE /groups/{gid}/channels/{cid}`、`GET /groups/export`、`POST /groups/import/m3u` |
| 订阅聚合 | `GET /subs/status`、`POST /subs/preview`、`POST /subs/groups` |
| 安全 | `GET /security/token`、`POST /security/token/rotate`、`GET /security/ip/stats`、`POST /security/ip/ban`、`POST /security/ip/unban-batch` |
| 指标 | `GET /status`、`GET /status/metrics`、`GET /stats`、`GET /cache/stats`、`GET /health`、`POST /cache/flush`、`GET /logs` |
| EPG | `GET /epg/{status,mappings,match-report,scripts}`、`GET /epg/programmes/…`、`POST /epg/{refresh,regenerate,mapping,extra,script,script-delete,cron-preview}`、`POST /epg/translate/{run,clear,probe,mtran/test}`、`GET /epg/translate/{models,status}` |
| 台标 | `GET /logo/{list,match-report}`、`POST /logo/{upload,apply,delete,rename,restore,suggest,animate}` |
| 录制 | `GET /schedule`、`POST /schedule`、`PUT /schedule/…`、`DELETE /schedule/…` |
| 解析/字幕 | `GET /resolver/scripts`、`POST /resolver/script`、`POST /subtitle/mtran/test`、`POST /subtitle/translate/clear` |
| 直播/回看 | `GET /livestream`、`GET /dash/status`、`GET /dash-timeshift/status` |
| 资源 | `GET /assets/list`、`POST /assets/{convert,remove}` |
| 其它 | `GET /link-info` |

## 2. Token 生命周期（S-AUTH-03 ✅ / S-AUTH-09 新）

```text

GET /panel/api/security/token → {"current_value":"<48hex>","enabled":true,"status":{"current":{"remaining_secs":86398,"valid":true,"value":"edcc****ab7a"},"old":{"remaining_secs":3598,"valid":true,"value":"135a****189f"},"ttl_hours":24},"ttl_hours":24}
POST /panel/api/security/token/rotate → {"ok":true,"new_token":"<48hex>","expires_in_hours":24}

```

- **轮换即生效**：`/playlist.m3u` 条目立刻改为新 token；**旧 token 宽限 ≈ 1 小时**（观测 3598s），宽限内新旧都 `200`。
- `status.current.value` 为**掩码**（前 4 + `****` + 后 4）；`current_value` 为明文（面板自用）。
- `token_ttl_hours=24`；关闭时 `enabled:false`、`current_value:""`。
- `GET /panel/api/link-info` → `{"aliases":{},"random_id_enabled":false,"token":"","token_enabled":false,"use_alias":false}`（别名与随机 ID 面）。

## 3. IP 封禁 / 解封（S-AUTH-10 新）

| 操作 | 请求 | 响应 |
| --- | --- | --- |
| 封禁 | `POST /security/ip/ban` `{"ips":["203.0.113.7","203.0.113.8"]}` | `{"ok":true,"results":[{"ip":…,"ok":true},…],"secs":2592000}` |
| 状态 | `GET /security/ip/stats` | `{"unique_ips":2,"active_sessions":0,"banned_ips":[{"ip":…,"remaining_secs":2591999},…]}` |
| 解封 | `POST /security/ip/unban` `{"ip":"203.0.113.7"}` | `{"ip":"203.0.113.7","ok":true}` |
| 批量解封 | `POST /security/ip/unban-batch`（面板按钮；体未测） | — |

- 字段名**不对称**：封禁用 `ips`（数组），解封用 `ip`（单数）；用错即 `{"error":"Missing ips array"}` / `{"error":"Missing ip"}`。
- 默认封禁时长 `ban_duration_secs = 2592000`（**30 天**）。
- `unique_ips` 在解封后**不回落**（观测 2 → 解封后仍 2）；即“见过即计数”。
- 与 `[auth]` 的 `ban_url` 配套：被拒请求 → 302 `ban_url`（占位视频）。

### 3.1 自助解封 `/unban`（S-AUTH-07 补测）

`[auth].ip_guard_enabled` 控制本组行为。

```text

GET /unban/whoami
200 {"banned":false,"enabled":true,"ip":"203.0.113.211","max_per_month":3,"remaining":3,"used":0}

POST /unban（未封禁）
200 {"msg":"你的 IP 当前未被封禁","ok":false}

POST /unban（已封禁且配额可用）
200 {"msg":"解封成功","ok":true}

POST /unban（配额耗尽）
200 {"msg":"本月自助解封次数已用完（每月最多3次）","ok":false}

```

- 关闭 IP 管控：`/unban/whoami` → `{"enabled":false,"ip":"…"}`；`POST /unban` → `200 {"msg":"IP 管控未启用","ok":false}`。
- `max_per_month=3`；`used` 只在**成功解封**后增加；未封禁不消耗。
- 配额耗尽仍返回 `200`，IP 保持封禁。
- 封禁状态与 `used` 都在内存；`docker restart` 后回到 `used=0`。
- 地址提取：`X-Forwarded-For` **末项**优先；没有时用 `X-Real-IP`；没有时用 TCP 对端。
- `GET /whoami` 实测 `404`；目标公开面只有 `/unban/whoami`。

## 4. 面板登录防爆破（S-AUTH-04 部分）
`[auth]` 键：`auth_fail_window_secs`(60)、`auth_fail_free_per_window`(15)、`auth_fail_ban_threshold`(6)、`auth_fail_ban_duration_secs`(3600)、`auth_fail_ban_repeat_duration_secs`(2592000)、`auth_fail_ban_escalate_window_secs`(86400)、`whitelist`。

实测（`auth_fail_free_per_window=2`、`auth_fail_ban_threshold=3`）：

```text

登录(错误密码) #1..#5 → 401 401 401 401 401
登录(错误密码) #6     → 429 {"error":"登录失败次数过多，请 299s 后再试","locked_for_secs":299}
登录(正确密码)        → 429（锁仍生效）
带既有 Bearer token 的 /panel/api/status → 200（锁不波及 API）

```

- 锁时长观测 `~300s`（首次），与配置 `auth_fail_ban_duration_secs=60` **不一致** → 首锁疑似固定 5 分钟；`auth_fail_ban_*` 更可能作用于**升级为 IP 封禁**（`ban_url`）的后续阶段。
- `auth_fail_free_per_window` 是**免费次数**，之后立刻 `429`（不是逐次惩罚）。

## 5. 限流 `l1/l2`（S-AUTH-04 🔶 未触发）
`[auth]` 键（**仅监控播放列表请求**）：

```text

ip_guard_enabled=false, max_sessions_per_ip=1, session_ttl_secs=30,
l1_window_secs=30, l1_limit=30, l2_window_secs=30, l2_limit=8,
cooldown_secs=3, violation_window_secs=300, violation_decay_secs=1800,
ban_threshold=3, ban_duration_secs=2592000,
limit_url="…/limit.mp4", ban_url="…/Ban.mp4", whitelist=[]

```

实测（`l1_limit=3`、`l2_limit=2`、`cooldown=0`、`ip_guard=true`）：

```text

连打 8 次 /test1/c1.m3u8        → 200 ×8
连打 8 次 /test1/c1.m3u8?token= → 200 ×8

```

- 仅 `active_sessions`/`online_viewers` 在**带 token 播放**时 +1（`{active_sessions:1, online_viewers:1}`）。
- 限流不按“裸请求数”计数；触发条件仍未定（可能与**并发/带宽**、或**按频道的窗口内不同切片**有关）。`limit_url` 重定向未复现。

## 6. 指标面（S-OPS-01…03 新）

| 端点 | 结果 |
| --- | --- |
| `GET /panel/api/status` | 200 JSON（见下） |
| `GET /panel/api/cache/stats` | 200 JSON（见下） |
| `GET /panel/api/health` | 200 JSON（见 §7） |
| `GET /status` | **404** |
| `GET /status/metrics` | **302** → `not_found_url`（假路由） |
| `GET /metrics`、`GET /version` | **404** |

```text

/panel/api/status 字段：uptime_secs, cache_backend, active_sessions, online_viewers, group_count, channel_count, version, mem_rss_mb, cpu_seconds, disk_cache_mb, recording_mb, low_resource_mode, poll_interval_secs, traffic_in_bytes, traffic_out_bytes, whole_file_committed, whole_file_budget, whole_file_peak, whole_file_hits, whole_file_fallbacks, entry_exts{gid → ".m3u8" | …}
/panel/api/cache/stats 字段：backend, disk_bytes, m3u8_entries, segment_entries, segment_url_entries, sub_url_entries, dash_disk_bytes, timeshift_disk_bytes, recording_disk_bytes, livestream_disk_bytes

```

注：`traffic_in_bytes=0` 恒为 0（与 S-BUG-01 一致），`traffic_out_bytes` 有值。

## 7. 健康探测出口（形）

```text

GET /panel/api/health → {"enabled":true,"entries":{"g2/c1":{"checked_at":1792222076,"consecutive_fails":0,"error":null,"http_status":200,"latency_ms":4,"ok":true}, "gdash/v":{…}}}

```

- 键为 `"<gid>/<cid>"`；`[health]` 键：`enabled`、`interval_secs`(300)、`timeout_secs`(10)、`concurrency`(3)。
- 探测**行为**（失败累积→恢复）见 [`07`](07-upstream-health-cache.md)。

## 8. 未决 / 局限
- 限流 `l1/l2` 的**真实触发条件**未定；`limit_url` 未复现（→ S-AUTH-04 保持 🔶）。
- 登录锁的**首锁时长**（观测 300s）与配置字段的对应关系未定；升级为 IP 封禁的阈值路径未复现。
- `POST /security/ip/unban-batch`、`POST /subs/*`、`POST /assets/*`、`GET /stats`、`GET /logs`、`POST /cache/flush` 的**语义**未逐项验证（部分由其它发现覆盖）。
- 封禁/解锁与 token 的**持久化**未验证（疑似内存态；重启即清）。
- 仅 arm64 + 单实例验证；未做并发压测。

## 9. 复现

```bash

B=http://127.0.0.1:19899
T=$(curl -s -X POST "$B/panel/api/login" -H 'Content-Type: application/json' -d '{"username":"admin","password":"admin"}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
AUTH="Authorization: Bearer $T"
curl -s -H "$AUTH" "$B/panel/api/security/token"; curl -s -X POST -H "$AUTH" "$B/panel/api/security/token/rotate"
curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"ips":["203.0.113.7"]}' "$B/panel/api/security/ip/ban"; curl -s -H "$AUTH" "$B/panel/api/security/ip/stats"
curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"ip":"203.0.113.7"}' "$B/panel/api/security/ip/unban"
for i in $(seq 1 6); do curl -s -o /dev/null -w '%{http_code} ' -X POST "$B/panel/api/login" -H 'Content-Type: application/json' -d '{"username":"admin","password":"wrong"}'; done; echo
curl -s -H "$AUTH" "$B/panel/api/status"; curl -s -H "$AUTH" "$B/panel/api/cache/stats"; curl -s -H "$AUTH" "$B/panel/api/health"
curl -s -o /dev/null -w '%{http_code} %{redirect_url}\n' "$B/status/metrics"

```
