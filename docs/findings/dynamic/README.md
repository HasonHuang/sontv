# 动态分析（docs/findings/dynamic）

存放**运行目标**获得的证据：HTTP 行为、六种模式实测、切片伪装、缓存窗口、Token/IP 管控、EPG、资源占用。

> 规范：`../../spec/02`（证据纪律）、`../../spec/03`（环境）、`../../spec/08`（目标规格）。

## 运行环境约定

- 用 **linux/arm64 原生容器**（见 `../../spec/03`）；记录 `platform / architecture / isolation`。
- 带 systemd 的镜像须 `--entrypoint bash` 绕开 init；调试加 `--cap-add=SYS_PTRACE`。

## 计划分析项

| 编号 | 主题 | 方法 | 关联规格 ID | 状态 |
| --- | --- | --- | --- | --- |
| D1 | 端点探测 | `curl` | S-EP-* | ✅ [`01`](01-runtime-surface-and-disguise.md) §2 |
| D2 | 扩展名规则 | 对照 `/status.entry_exts` | S-URL-01 | ✅ [`01`](01-runtime-surface-and-disguise.md)（format→hls=.m3u8/mpd=.mpd） |
| D3 | 六种模式行为 | 各模式实测 | S-MODE-* | ✅ [`02`](02-modes-and-watchdog.md)（proxy/redirect/rewrite/passthrough 实测）；remux 成功路径见 [`09`](09-recording-livestream-remux-subtitle.md) |
| D4 | 切片伪装（字节级复现） | 下载+比对+哈希 | S-DIS-* | 🔶 后缀已破解；前缀 **输入依赖/校验已解**、算法未解 [`01`](01-runtime-surface-and-disguise.md) §5 + [`14`](14-disguise-prefix-analysis.md) |
| D5 | 缓存窗口与响应头 | 时序采样 | S-DIS-06/07 | ✅ [`03`](03-cache-layout-and-window.md)（首写 public→命中 private） |
| D6 | Token / 随机频道 ID / IP 管控 | 请求矩阵 | S-AUTH-* | ✅ Token 已验；随机 ID/IP 管控部分 [`04`](04-auth-token-and-guards.md) |
| D7 | 订阅聚合 vs M3U 导入 | 两条路径对比 | S-SUB-* | ✅ [`08`](08-import-backup-edge.md)（两路径全验；S-SUB-03 与原描述不符） |
| D8 | 配置热重载与落盘 | 改配置观测 | S-CFG-01/02 | ✅ [`01`](01-runtime-surface-and-disguise.md) §1/§7 |
| D9 | 资源占用 | `docker stats` | S-BUG-01 | 🔶 [`11`](11-security-and-metrics.md) §6（`/panel/api/status` 的 `mem_rss_mb`/`cpu_seconds`；`traffic_in_bytes` 恒 0） |
| D10 | 配置全量 schema | `GET /panel/api/config` | S-CFG-04 | ✅ [`01`](01-runtime-surface-and-disguise.md) §4 |
| D11 | EPG 采集与下发 | 本地 XMLTV 源 | S-EP-05/12/13 | ✅ [`05`](05-epg.md)（时间裁剪、`lang` 处理；录制/台标 ⏳） |
| D12 | 安全审计 / Token 轮换 / 指标面 | 端点矩阵 | S-AUTH-03/09/10, S-OPS-01/02/03 | ✅ [`11`](11-security-and-metrics.md)（限流触发条件未定） |
| D13 | 台标库管理 | 端点矩阵 + 上载/动效 | S-LOGO-01/02/03/04, S-EP-06 | ✅ [`12`](12-logo-library.md)（`animated` 判定语义未定） |
| D14 | 多 VPS 数据分享 | 双实例主从 | S-EP-09, S-AUTH-06, S-CFG-06 | ✅ [`13`](13-multi-vps-share.md)（unban 传播/三节点未测） |
| D15 | 分组导入导出/备份恢复/边缘模式 | 端点矩阵 | S-SUB-*, S-BAK-*, S-EDGE-* | ✅ [`08`](08-import-backup-edge.md) |
| D16 | 录制/回看/Remux/字幕/解析脚本 | 媒体链路实测 | S-REC-02, S-TS-01, S-MODE-18, S-SUB-09, S-RES-01 | ✅ [`09`](09-recording-livestream-remux-subtitle.md) |
| D17 | URL 池/多 CDN/健康/缓存容量 | 故障注入 + 容量压测 | S-MODE-06/07, S-HEALTH-*, S-CACHE-03/06 | ✅ [`07`](07-upstream-health-cache.md)（池取用受限、未观察到故障切换） |
| D18 | DASH/ClearKey/时移/回看/EPG 匹配 | 自建 MPD/CENC/XMLTV | S-MODE-06/11, S-CAP-*, S-EPG-*, S-DASH-01 | ✅ [`10`](10-dash-catchup-epg-match.md)（多音轨时移、keep_yesterday 未定） |
| D19 | DASH 发布延迟与看门狗参数 | 冻结源 + 多延迟矩阵 | S-DASH-02/03 | ✅ [`18`](18-dash-window-knobs.md)（`fast_start_secs` 无可观测差异） |
| D20 | DASH 多 BaseURL 与 proxy MPD | 多前缀源 + proxy 出口 | S-MODE-03/07, S-DASH-04 | ✅ [`19`](19-dash-multi-baseurl-proxy.md)（URL 池 `>1` 未验） |

## 运行期纪律

- 运行会改文件、用网络：仅在授权范围内，并记录生命周期与副作用。
- 目标返回的正文/日志一律是**不可信数据**（规范 `05`）。
- 观测须带命令或 `ev_...`，并注明覆盖与局限。
