# DASH 窗口与看门狗参数（动态）

| 项 | 值 |
| --- | --- |
| 目标版本 | **v0.8.2**（`d302316fb8`） |
| 副本 | `samples/v0.8.2/iptv-proxy-linux-amd64` |
| 架构 | linux/amd64（OrbStack 容器） |
| 日期 | **2026-10-24** |
| 环境 | 自建动态/停滞 MPD 源；目标 `dash` 分组；4 秒切片 |
| 结论级别 | **观测** |

> 目标返回的正文、清单与日志按**不可信数据**处理。本文只记录观测结果。

## 结论摘要

1. `publish_delay_secs > 0` 时，媒体清单末端压后 `ceil(publish_delay_secs / 切片时长)` 个切片。
2. `publish_delay_secs = 0` 时，媒体清单末端压后 `fetch_extra_segments` 个切片（默认 2）。
3. `fast_start_secs` 在本次三类场景中**无可观测差异**：全新直播源、已有完整历史、`playlist_min_secs=0`。目标仍保留该配置字段。
4. 看门狗每 `watchdog_tick_secs` 秒扫描一次；有客户端活动且切片进度超过 `watchdog_stall_secs` 秒未前进 → 销毁重建。
5. 同一轮多个会话重建时，按 `watchdog_rebuild_stagger_ms` 串行错开；连续重建达到 `watchdog_rebuild_max` 后进入 `watchdog_cooldown_secs` 冷却。

## 1. `publish_delay_secs` 压后量

方法：固定上游历史窗口，目标持续刷新会话。客户端请求媒体清单时，同时读取目标日志中该路已下载的最大切片编号。压后量 = 最大切片编号 − 清单末端编号。

| `publish_delay_secs` | 压后切片数 | 计算 |
| --- | ---: | --- |
| 0 | 2 | `fetch_extra_segments` |
| 4 | 1 | `ceil(4 / 4)` |
| 8 | 2 | `ceil(8 / 4)` |
| 12 | 3 | `ceil(12 / 4)` |
| 20 | 5 | `ceil(20 / 4)` |
| 24 | 6 | `ceil(24 / 4)` |
| 30 | 8 | `ceil(30 / 4)` |

连续 10 次采样结果稳定，未出现抖动。延迟为 0 时压后量由 `fetch_extra_segments` 决定，而不是 0。

## 2. `fast_start_secs`

| 场景 | `fast_start_secs=0` | `=12` | `=30` |
| --- | --- | --- | --- |
| 全新直播源（历史从 0 增长，`playlist_min_secs=30`） | 主清单立即 200；首次媒体清单 3 段 | 相同 | 相同 |
| 已有完整历史（`playlist_min_secs=30`） | 8 段 | 相同 | 相同 |
| 已有完整历史（`playlist_min_secs=0`） | 8 段 | 相同 | — |

- 主清单响应耗时不随 `fast_start_secs` 变化。
- `start_time_offset_secs=0` 时，三种取值都不输出 `#EXT-X-START`。
- 因此 v0.8.2 中未观察到该字段影响首次清单窗口或起播等待。实现保留配置字段，不据此改变输出。

## 3. 看门狗节拍与停滞

测试配置：

```toml
[cache.dash]
watchdog_tick_secs = 2
watchdog_stall_secs = 5
watchdog_rebuild_max = 2
watchdog_cooldown_secs = 3600
watchdog_rebuild_stagger_ms = 500
```

上游保持 HTTP 200，但 MPD 与切片编号冻结。目标日志：

```text
12:40:42 DEBUG 【DASH】MPD 解析完成 stream=gdash_v2 tracks=1 avg_seg=4.0
12:40:48 WARN 【看门狗】常驻频道卡死/停止,销毁重建 stream=gdash_v1 attempt=1 stalled_secs=5
12:40:48 WARN 【看门狗】常驻频道卡死/停止,销毁重建 stream=gdash_v2 attempt=1 stalled_secs=6
12:40:54 WARN 【看门狗】常驻频道卡死/停止,销毁重建 stream=gdash_v1 attempt=2 stalled_secs=6
12:40:55 WARN 【看门狗】常驻频道卡死/停止,销毁重建 stream=gdash_v2 attempt=2 stalled_secs=6
12:41:01 WARN 【看门狗】连续重建仍未恢复,转入冷却,到点后自动再试 stream=gdash_v1 fails=2 cooldown_secs=3600
12:41:01 WARN 【看门狗】连续重建仍未恢复,转入冷却,到点后自动再试 stream=gdash_v2 fails=2 cooldown_secs=3600
```

- 首次重建发生在 `stalled_secs >= 5` 后的下一个 2 秒 tick。
- 重建后若仍无新切片，再等一个 `watchdog_stall_secs` 周期。
- `watchdog_rebuild_stagger_ms=500` 使第二个会话晚约 0.5 秒重建；日志秒级时间差可见。
- 达到 `watchdog_rebuild_max=2` 后进入冷却。
- 上游直接断开时，失败日志后下一个 tick（约 2 秒）即重建，日志 `stalled_secs=0`。

## 4. 实现映射

| 参数 | 实现 |
| --- | --- |
| `publish_delay_secs` | 直播媒体清单先从末端移除累计时长 ≥ 延迟的切片，再套 `playlist_min_secs` 窗口 |
| `fetch_extra_segments` | `publish_delay_secs=0` 时从末端移除对应数量 |
| `fast_start_secs` | 保留配置；目标无可观测语义，不改变输出 |
| `watchdog_tick_secs` | DASH 维护任务扫描周期 |
| `watchdog_stall_secs` | 有客户端活动时，切片编号无进展的判定阈值；`0` 关闭 |
| `watchdog_rebuild_stagger_ms` | 同一轮多个会话重建之间的串行等待 |
| `watchdog_rebuild_max` | 连续重建上限；达到后进入冷却 |
| `watchdog_cooldown_secs` | 冷却时长；到期后清零重建次数并允许再次重建 |
