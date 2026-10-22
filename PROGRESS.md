# PROGRESS · IPTV Proxy 逆向任务

> 本文件记录**任务计划**与**进度**。规则：**每处理完一项，立即更新本文件**。
> 规范见 [`docs/spec/`](docs/spec/)。发现见 [`docs/findings/`](docs/findings/)。实现设计见 [`docs/design/`](docs/design/)。
> 目标版本：**v0.8.2**（上游提交 `d302316fb8`）。

## 0. 总览（权威）

> 本节是当前状态与待办的唯一入口。`§3d` 之后是按时间追加的历史日志，编号不连续。

### 0.1 任务目标与约束

逆向闭源 Linux 二进制 **IPTV Proxy**（`YanG-1989/rust` 子目录 `IPTV Proxy/`，仅发布 arm64/amd64/v7）。
产出：可核验的规格与行为结论（对齐基线 [`sontv/docs/iptv-proxy`](../sontv/docs/iptv-proxy)）。

约束：
- 只用 **linux-arm64 / linux-amd64** 两个产物。
- 静态在 macOS（REA + Hopper）；动态在 **linux/arm64 容器**（OrbStack）。
- 目标派生内容一律按**不可信数据**处理。

### 0.2 进度图

```mermaid
flowchart LR
  P0["P0 逆向与核心<br/>✅ 完成"] --> P1["P1 流处理与增值<br/>✅ 完成"]
  P1 --> P2["P2 运维与管理面<br/>✅ 完成"]
  P2 --> P3["P3 兼容性补全<br/>✅ 完成"]
```

| 阶段 | 范围 | 进度 |
| --- | --- | --- |
| P0 | 逆向、代理/直连、Token、缓存、播放链路、打包 | ██████████ 100% |
| P1 | EPG、回看、remux、录制、台标、分享、安全面 | ██████████ 100% |
| P2 | 订阅聚合、面板分区、DASH 回看、EPG/字幕翻译 | ██████████ 100% |
| P3 | 目标剩余端点与兼容细节（见 §0.6） | ██████████ 100% |

### 0.3 质量门（2026-10-22）

| 项 | 结果 |
| --- | --- |
| `cargo test --offline` | ✅ 205 项（138 单元 + 67 集成） |
| 冒烟脚本 | ✅ 15 个（assets、panel、resolver、share、remux、schedule、timeshift、record、record-dash、epg、aggregate、backup、logo、security、cli）；另有真实翻译联调 1 个（需外网） |
| DASH 回看 | ✅ 容器内 4 项断言 × 3 次 |
| 格式与静态检查 | ✅ `cargo fmt --check`；`cargo clippy` 0 error |
| 发布包 | ✅ amd64 3.6 MB、arm64 3.4 MB；`verify-release.sh amd64` 与 `verify-release.sh arm64` 通过 |
| CI | ✅ [运行 39783303837](https://github.com/HasonHuang/sontv/actions/runs/39783303837)（`5fc2b1b`）；fmt/clippy/test、CLI 冒烟与双架构 `verify-release.sh` 全绿；新增 `x-tvg-url` 用例 |

### 0.4 里程碑

| 里程碑 | 内容 | 状态 |
| --- | --- | --- |
| M0 | 配置、登录、内嵌 4 页前端 | ✅ 2026-10-19 |
| M1 | 分组/频道、订阅、Token | ✅ 2026-10-19 |
| M2 | 代理/切片伪装/缓存 | ✅ 2026-10-19 |
| M3 | redirect/rewrite/passthrough/remux/dash | ✅ 2026-10-19 |
| M4 | 运维、对拍、双架构打包 | ✅ 2026-10-20 |
| P1 | EPG、回看、录制、台标、分享、安全、素材 | ✅ 2026-10-20 |
| P2 | 订阅聚合、面板分区、翻译引擎、DASH 复验 | ✅ 2026-10-21 |

### 0.5 已知差异与不实现

| 项 | 结论 | 原因 |
| --- | --- | --- |
| 切片前缀 `<P>` 算法 | 未解 | F-017；只依赖 `(gid,cid)`，低影响 |
| 缓存容量上限 | 不实现 | 目标配置存在但未生效（F-015） |
| 旧数据迁移 `./var/dash` | 不实现 | 仅 088 行为，v0.8.2 无 |
| DASH URL 池 `pool≥2` | 已实现（目标取用未验） | 多个 `<BaseURL>`；视频用 `pool[0]`，音频/字幕轮询其余；目标正常取用未观测（F-015） |
| DASH `mpd_keep_seconds` | 已实现 | DASH→HLS 媒体清单与 `proxy` 模式 MPD 都按末尾时间窗裁剪；proxy 分支会同步推进 `$Number$` 起始编号 |
| `proxy` 模式 MPD 代理 | 已实现（相对地址） | 相对 `<BaseURL>` 改写到本服务 `mpd/`，init/切片与查询串经本服务代理；绝对 `<BaseURL>` 按目标行为保持直连；`mpd_keep_seconds` 已裁剪 |
| CENC ClearKey 解密 | 不实现 | 目标只检测 KID、不解密；本实现按原字节代理 |
| 多 CDN 故障切换 | 未复刻 | 目标未观测到切换；DASH 多 BaseURL 仅做顺序回退 |
| `/epg/api/*` 响应形状 | 自定 | 目标仅登记路由（S-EP-16） |
| `l1/l2` 限流与每 IP 会话上限 | 不实现 | 触发条件未复现（F-010 §5） |
| `random_id_enabled` 随机 ID | 仅回显 | 目标行为未观测；`link-info` 形状已对齐 |
| `cache.backend=memory` | 仅回显 | 实际缓存仍落盘；目标 memory 统计恒 0（F-015） |
| 播放侧 `compat_mode` 与 `srt_ua_keywords` | 不实现 | 目标未测出行为（D19 §3.5） |
| 定时录制 DASH 频道 | 已实现（轨道并轨） | 增量抓取各轨 init/切片，再 `ffmpeg -c copy` 并轨为 MKV；静态 MPD 与目标同报「录制区间内无可用切片」（F-009 S-REC-02） |
| `socks5` 代理 | 不支持 | 未知类型直连并告警；目标行为未验 |

### 0.6 任务计划（P3）

建议按编号顺序推进。每项先补设计与用例，再实现。

| ID | 交付 | 规模 | 依赖 | 验收 | 状态 |
| --- | --- | --- | --- | --- | --- |
| P3-01 | CI 质量门 | S | GitHub Actions | `fmt`/`clippy`/`test`/双架构构建；失败阻断合并 | ✅ 运行 `39593785498` |
| P3-02 | `/subs/{status,groups,preview}` | M | 无 | 形状按 S-GAP-07；复用 `[playlist]` 聚合；集成测试覆盖保留/丢弃/分组映射 | ✅ |
| P3-03 | `GET /panel/api/link-info` | S | 无 | 返回 `aliases`、`random_id_enabled`、`token`、`token_enabled`、`use_alias`；测试 | ✅ |
| P3-04 | 安全面补全 | M | 无 | 登录失败 429 锁、`/unban` 每月 3 次；`l1/l2` 记为不实现 | ✅ |
| P3-05 | 字幕格式扩展 | M | 无 | 翻译端点接受 ASS/SSA 并转 VTT；`compat_mode` 记为不实现 | ✅ |
| P3-06 | 真实翻译联调 | S | MTranServer（可用试用服务） | EPG 与字幕各跑一次；确认响应形状、分批、缓存、失败回退 | ✅ §3bb |
| P3-07 | 每频道代理覆盖 | M | 本地代理夹具 | 分组/频道 `proxy` 覆盖直连；播放、回看、Remux、录制生效 | ✅ |
| P3-08 | 目标差分回归 | S | v0.8.2 容器与样本 | 重跑 22 项对拍与新增端点；差异 0 或登记 | ✅ 22/22 差异 0 |

顺序：P3-01 → P3-02 → P3-03 → P3-04 → P3-05 → P3-06 → P3-07 → P3-08。P3-01 无外部依赖，先做；P3-08 需容器环境，最后做。

### 0.6.1 P4 计划（DASH 完整链路）

| ID | 交付 | 规模 | 验收 | 状态 |
| --- | --- | --- | --- | --- |
| P4-01 | MPD 完整解析与 DASH→HLS | L | `BaseURL`、`SegmentTimeline`、`SegmentList`、init/切片代理、`/dash/status` | ✅ §3bc |
| P4-02 | `mpd_keep_seconds` 与 `url_pool_size` | M | 媒体清单时间窗；多 `<BaseURL>` 音视频分流与顺序回退 | ✅ §3bc |
| P4-03 | 目标 proxy+MPD 分支 | M | 相对 `<BaseURL>` 改写 + init/切片代理；`mpd_keep_seconds` 裁剪；绝对地址按目标保持直连 | ✅ §0.8 |
| P4-04 | CENC 解密 | L | ClearKey 解密与出口策略 | 不实现（目标不解密） |

### 0.7 待环境

当前无阻塞项。

### 0.8 未完成任务总表（2026-10-22）

> 本表汇总当前代码、§0.5、README 与 `docs/spec/08` 的未完成项。P0–P4 已交付项不重复。
> 类型：**待做** = 代码缺口；**待验** = 需要目标容器或真实上游；**不做** = 已登记差异，不计入待办。
> 优先级：P0 影响播放/订阅兼容；P1 影响配置与运维；P2 影响证据完整度。
>
> §0.2 的百分比只表示原计划 P0–P3 的完成度；本节的残余项不计入该百分比。

#### 0.8.1 待做（代码缺口）

| ID | 优先级 | 项 | 现状 | 待做 | 依据 |
| --- | --- | --- | --- | --- | --- |
| U-05 | P1 | 目标配置段部分接入 | `[cache.prefetch]`、`[cache.segments]`、`[cache.ttl]`、`[cache.dash]`、`[cache.remux]` 已建模；切片索引上限、订阅/切片 TTL、媒体切片预取、DASH 超时/重试/空闲回收、媒体窗口 `playlist_min_secs`、`start_time_offset_secs`、DASH 看门狗阈值与冷却、Remux 时长/窗口/退避已接入 | 继续接入 `[cache.dash]` 的 `fast_start_secs`、`publish_delay_secs`、`watchdog_tick_secs`、`watchdog_stall_secs` 与 `watchdog_rebuild_stagger_ms`；未知键继续原样回写 | 目标配置 JSON、S-CACHE-04、`design/04` |
| U-06 | P1 | 台标动效兼容缺口 | `/panel/api/logo/animate` 已实现 6 种效果与 APNG 编码，`.orig/` 备份、`quality` 字符串校验、`animated` 列表标志已接入；输入扩展名与目标白名单一致（`png/jpg/jpeg/webp/bmp/avif`） | 解码仍依赖 `ffmpeg`/`ffprobe`；尚未实现脏矩形优化 | F-012、`findings/dynamic/12` |

> 本轮已完成（2026-10-22）：
> - **U-01 订阅注入 `x-tvg-url`**：`[epg].enabled = true` 时首行输出 `#EXTM3U x-tvg-url="<host>/epg.xml.gz"`；单元与集成测试覆盖。
> - **U-02 EPG 端点补齐**：注册 12 个 `/panel/api/epg/*` 端点；`regenerate` 改为读 `epg/raw/`；额外频道持久化并写入输出；脚本上传/删除、Cron 预览、节目表、源频道、建议与时间偏移预览已实现；面板翻译端点补 `/panel/api` 别名。
> - **U-03 EPG 配置语义**：`public_viewer_enabled` 控制 `/epg`；`allow_scripts=false` 拒绝脚本源；`python_bin`、`script_timeout_secs`、源级 `timeout_secs`、`tz_shift_mins`、源级 `cron` 生效；`record_pad_*` 进入 `/panel/api/epg/programmes`；全局 Cron 支持标准 5 字段。
> - **U-04 `proxy` + MPD**：`mpd_keep_seconds > 0` 时裁剪 `SegmentTemplate/SegmentTimeline` 与 `SegmentList`，并推进 `$Number$` 的 `startNumber`；绝对 `<BaseURL>` 按目标证据保持直连。
> - **U-05 配置段**：新增 `[cache.{prefetch,segments,ttl,dash,remux}]` 模型；接入切片索引上限、`ttl.sub_url_secs`/`segment_secs`、媒体切片预取、DASH MPD/切片超时、重试退避、会话空闲回收、媒体窗口 `playlist_min_secs`、`start_time_offset_secs`、Remux 目标时长/窗口/重连退避。`[cache.dash]` 的 `watchdog_rebuild_max`/`watchdog_cooldown_secs` 已接入；`fast_start_secs`/`publish_delay_secs` 与其余 watchdog 参数仍待接入。
> - **U-06 资源/台标部分**：`/panel/api/assets/{convert,list,remove}` 支持 `[assets].map` 与本地 HLS，新增公开 `GET /assets/*path`；`/panel/api/logo/{restore,animate}` 已实现，动效覆盖 6 种效果与 APNG 编码，`logo/list` 增加 `has_original`/`animated`。动效输入支持目标白名单 `png/jpg/jpeg/webp/bmp/avif`。
> - **V-11 真实 HLS/M3U 验证**：用 Mux 与 Higo 真实源对拍目标；补齐 25 秒总超时、流式部分正文、切片 404 → 200 空正文、清单失败 → 302 `error_url`、HLS 清单 4 次重试、主清单属性白名单、VOD/直播媒体缓存头、已知扩展名保留。详见 F-019。Mux 不再 502；Higo `fct`/`wxxw`/`now-wx` 首切片均完整下载。

#### 0.8.2 待验（目标证据）

| ID | 项 | 当前结论 | 需要的证据 |
| --- | --- | --- | --- |
| V-01 | DASH URL 池正常取用 | 已按视频 `pool[0]`、音频/字幕轮询其余实现；目标在 `pool≥2` 环境只拉 MPD 后零切片 | 目标正常 MPD + 多 `<BaseURL>` 样本；记录实际请求分布与回退顺序 |
| V-02 | `proxy` + MPD 目标分支 | 相对 `<BaseURL>` 分支已对拍；绝对 `<BaseURL>` 目标行为未验；`mpd_keep_seconds` 时间窗裁剪已实现，缺目标 proxy+MPD 对拍 | 目标容器构造绝对 `<BaseURL>` 与多切片 MPD |
| V-03 | `/epg/api/*` 响应形状 | 面板私有 EPG 端点已按目标面板调用形状实现；公开 `/epg/api/*` 仍为本实现自定；`spec/08` §16 与 `design/03` §8 对 v0.8.2/v0.8.8 路由版本记载冲突 | 目标公开端点逐字段抓取；先解决版本证据冲突 |
| V-04 | `/subs/{groups,preview}` 字段 | 行为与主要字段已实现；目标逐字段形状未完全对拍 | 目标请求/响应样本，重点比较 `trace`、`dropped_groups`、`out_groups` |
| V-05 | 每频道独立代理 | 本实现按频道 > 分组 > 直连，覆盖播放、回看、Remux、录制、DASH 回看 | 目标覆盖顺序、认证头、失败回退与 `socks5` 行为 |
| V-06 | `/unban` 配额与 `whoami` | 本实现已有每个 IP 30 天 3 次与 `whoami`；目标配额未实测 | 目标连续解封；记录状态码、`remaining` 与窗口重置 |
| V-07 | 切片 TS 结构 | 本实现只保证字节代理；无 PAT/PMT/SDT 证据 | 真实上游 TS 切片；核对 PID 布局、SDT 与 `lumberjack` 水印（S-DIS-08/09） |
| V-08 | `<P>` 算法 | 自研 `sha256("<gid>:<cid>")[:16]`；目标算法未解 | 更多 `(gid,cid)` 样本或反编译；低影响，不阻塞发布 |
| V-09 | `[playlist]` / `[share]` 目标语义 | 本实现已覆盖主要路径；目标多 VPS 与聚合边界未完整对拍 | 目标 `merge_ttl_secs`、`merge_position`、`share` 角色切换与信任 IP 样本 |
| V-10 | 面板与录制/EPG 交互差分 | API 对拍 22/22 差异 0；12 个面板分区与录制/EPG 完整流程未逐屏对拍 | 目标容器截图与操作录像；只比对行为，不复制目标前端代码、资源与文案 |

#### 0.8.3 已登记为不做或仅回显

以下项不计入上述待办。若要改变结论，先补目标证据。

| 项 | 结论 | 依据 |
| --- | --- | --- |
| 缓存容量上限 | 不实现 | 目标配置存在但上限未生效（F-015） |
| 旧数据迁移 `./var/dash` | 不实现 | 仅 v0.8.8 行为，v0.8.2 无 |
| CENC/ClearKey 解密 | 不实现 | 目标只检测 KID，不解密；本实现按原字节代理 |
| 多 CDN 故障切换 | 未复刻 | 目标未观测到切换；DASH 多 `<BaseURL>` 只做顺序回退 |
| `l1/l2` 限流与每 IP 会话上限 | 不实现 | 触发条件未复现（F-010 §5） |
| `random_id_enabled` 随机 ID | 仅回显 | 目标行为未观测；`link-info` 形状已对齐 |
| `cache.backend = memory` | 仅回显 | 实际缓存仍落盘；目标 memory 统计恒 0（F-015） |
| 播放侧 `compat_mode` 与 `srt_ua_keywords` | 不实现 | 目标未测出行为（D19 §3.5） |
| `socks5` 代理 | 不支持 | 未知类型直连并告警；目标行为未验 |

#### 0.8.4 真实源验证（2026-10-22）

| 源/项 | 结果 | 证据 |
| --- | --- | --- |
| Mux VOD 主清单 | ✅ 选中 1080p；媒体清单 64 条；首切片 25 秒内部分正文，状态 200，不再 502 | F-019 §5.1 |
| Higo `fct` | ✅ 301 跟随；直播清单 `no-cache, no-store`；`.jpg` 保留；首切片 4,387,168 字节完整；二次命中 `private` | F-019 §5.2 |
| Higo `wxxw` | ✅ 无扩展名上游切片改写后完整下载 4,103,100 字节 | F-019 §5.3 |
| Higo `now-wx` | ✅ 首切片 5,866,540 字节完整下载 | F-019 §5.4 |
| Higo M3U 订阅 | ✅ `/playlist.m3u` 聚合 3 个频道；`x-tvg-url` 由本实现注入 | F-019 §5.5 |
| 错误路径 | ✅ 慢速正文 25 秒中断仍 200；切片 404 → 200 空；清单 404 → 302 `error_url`；清单重试 4 次 | F-019 §3 |
| 清单/伪装 | ✅ 主清单 4 属性白名单；VOD/直播缓存头；已知扩展名保留 | F-019 §4 |
| 扩展名轮换起点 | 🔶 目标算法未解；本实现只保证取自 `formats` | F-018、F-019 §6 |
| `[playlist.sources]` 扩展字段 | 🔶 目标 `group_map`/`group_mode`/`unlisted_groups` 未逐字段对拍（V-09） | F-019 §5.5 |

## 1. 已完成（分阶段）

### Phase 0–1 · 规范、骨架与取得 ✅

| 项 | 结果 | 位置 |
| --- | --- | --- |
| 强制规范 8 篇 + 发现骨架 | ✅ `docs/spec/README,01–08`；`docs/findings/{README,artifact,static,dynamic}` | — |
| 取得产物（主 v0.8.2 / 对照 v0.8.8） | ✅ `samples/`（gitignore，指纹登记于 `artifact.md`） | `artifact.md` §2 |
| 加壳与 ELF | ✅ UPX 4.24、`upx -d` 成功；非 PIE、静态、NX、RELRO | `findings/static/01` §2–§3 |
| 工具链与版本 | ✅ Rust 1.98.1；v0.8.2=`d302316fb8`、main=v0.8.8 | `findings/static/01` §4–§5（F-001/F-002） |

### Phase 2 · 静态分析

| 项 | 状态 | 结果 / 位置 |
| --- | --- | --- |
| S1 指纹与 ELF 结构 | ✅ | `artifact.md` §2–§3 |
| S2 加壳/脱壳 | ✅ | `findings/static/01` §2–§3 |
| S3 工具链/语言（Rust） | ✅ | `findings/static/01` §4（S-ENV-07 升级为观测） |
| S4 版本判定 | ✅ | `findings/static/01` §5（F-002） |
| S5 端点/配置键预扫描 | ✅ | `findings/static/01` §6–§7 |
| S6 内嵌面板前端 | ✅ 4 页（管理面板 622KB / 节目单 / 台标库 / 自助解封） | `findings/static/02`（F-003） |
| S7 关键算法反编译 | ✅ 范围内完成（F-017） | 后缀=`sha256(上游切片URL)[:32]`；缓存窗口（F-006）；前缀 `<P>` 仅依赖 `(gid,cid)`、参与校验、**算法未解**（F-017，低影响） |

### Phase 3 · 动态分析（容器 `iptv82`，v0.8.2）

| 项 | 状态 | 结果 |
| --- | --- | --- |
| D1–D4 启动/端点/认证/版本 | ✅ | `[::]:19899`；`/health`=200、`/panel`=200、`/unban`=200、**`/status`=404**、EPG 关闭时 `/epg*` 404；未知频道→302 占位视频；`POST /panel/api/login`→`{"token":uuid}`（`Bearer` 或 `X-Token`）；`/panel/api/status`→`"version":"0.8.2"` |
| D5 配置 schema | ✅ | `/panel/api/config` 全量；见 `findings/dynamic/raw/v0.8.2-config.json` |
| D6 分组/频道 | ✅ | `POST /panel/api/groups`；`…/groups/{gid}/channels`；`PUT …/channels/{cid}` |
| D7 六模式行为 | ✅ | proxy/redirect/rewrite/passthrough 实测；dash/remux 失败路径+看门狗（F-005） |
| D8 切片伪装字节级 | 🔶 | 后缀=`sha256(上游切片绝对URL)[:32]`；前缀仅依赖 `(gid,cid)`、参与校验、扩展名不校验；算法未解（F-017） |
| D9 缓存窗口 | ✅ | 首写 `public,max-age=300` → 命中 `private,max-age=300`；入口 `public,max-age=5`（F-006） |
| D10 EPG | ✅ | `/epg.xml[.gz]`、`/epg` 免登录 200；根 `iptv-proxy`；时间裁剪与 `lang`（F-008） |
| D10b 录制/台标 | ✅ | 台标 `/logo/<file>`（`public,max-age=86400`）；录制 MKV + `ffmpeg -c copy`；`/assets/ffmpeg-status`→`installed:true`（F-014） |
| D11 播放 Token | ✅ | 入口需 `?token=<48hex>`（TTL 24h）；失效→302 `token_failure_url`；切片无 token 也 200（F-007） |
| D13 入口改写/访问范围 | ✅ | access 四档（open 不进总订阅）；`m3u_header_lines`；`PUT /panel/api/config` 全量原子保存（与文件直改有竞态）（F-009） |

### Phase 4 · 报告与回填

| 项 | 状态 | 结果 / 位置 |
| --- | --- | --- |
| R1 差异登记 | ✅ | `07` §3.1 `D-001…D-005` |
| R2 核对矩阵回填 | ✅ | `07` §2.3 动态项覆盖 F-004…F-017（新增 S-DIS-17） |
| R3 阶段性报告 | ✅ | [`findings/report-stage-01.md`](docs/findings/report-stage-01.md) |

### Phase 5 · 实现设计与交付（已完成）

| 项 | 状态 | 结果 / 位置 |
| --- | --- | --- |
| E1 技术选型 | ✅ | [`design/01-tech-selection.md`](docs/design/01-tech-selection.md)（Rust + axum/tokio；Vite+React 面板嵌入；drop-in 兼容；首期 P0） |
| E1b UI 参考截图 | ✅ | [`design/reference/`](docs/design/reference/)（30 张：13 页 + 13 弹窗 + 4 独立页；含功能↔证据映射） |
| E2 架构与模块 | ✅ | [`design/02-architecture.md`](docs/design/02-architecture.md)（分层、模块清单、流水线、状态、缓存/伪装设计、ADR、里程碑） |
| E3 HTTP 契约 | ✅ | [`design/03-http-contract.md`](docs/design/03-http-contract.md)（公开/私有路由表、播放与订阅契约、access 矩阵、错误串、M1 清单） |
| E4 配置与存储 | ✅ | [`design/04-config-and-storage.md`](docs/design/04-config-and-storage.md)（配置写入与热重载矩阵、存储布局、运行时数据、日志、发布布局） |
| E5 前端设计 | ✅ | [`design/05-frontend.md`](docs/design/05-frontend.md)（技术形态、13 分区与弹窗体系、组件清单、交互约定、分期） |
| E6 实现与交付 | ✅ | M0–M4、P1、P2 全部完成；质量门见 §0.2（判据见 [`design/02-architecture.md`](docs/design/02-architecture.md) §14） |

## 2. 关键结论（逆向阶段）

1. 样本：UPX 4.24 加壳；脱壳后 Rust 1.98.1、静态、非 PIE。
2. 版本：主对象 **v0.8.2**（`d302316fb8`）；`main` = **v0.8.8**；运行时 `/panel/api/status` 报 `0.8.2`。
3. 技术栈：`axum 0.7.9` + `tokio 1.52.1` + `hyper 1.9.0` + `reqwest 0.12.28` + `rustls` + `tracing`。
4. 面板前端**完整内嵌**（4 页）；接口前缀 `/panel/api/…`。
5. 伪装默认开启：`[cache.disguise]` enabled/playlist 开、subtitle/master 关、formats=jpg/png/gif/bmp/webp；`[cache.dash].obfuscate_filenames=true`。与基线 S-DIS-01 一致。
6. **切片伪装**：形状 `/{gid}/{cid}/<16hex>_<32hex>.<ext>`；`<32hex>` = `sha256(上游切片绝对 URL)[:32]`；
   `<P>`（16 hex）仅依赖 `(gid,cid)`、参与校验（错→302）、算法未解（F-017）；扩展名不校验。
7. **清单伪装**：主清单 `/{gid}/{cid}.m3u8`（`no-cache`）；媒体清单 `/{gid}/{cid}/video/index.txt`（`text/plain`）。切片忽略 `Range`。
8. 未知频道 → **302** 占位视频 `material.yang-1989.eu.org/v/{404,mistake,Failure,limit,Ban}.mp4`。
9. 面板认证：登录换取 **UUID token**；`Authorization: Bearer` 或 `X-Token` 二选一。
10. **六种模式**（分组级）：`proxy` 反代+伪装切片；`remux`；`redirect`/`passthrough` → 302 直连上游；`rewrite` 清单绝对化；`dash`。
    `redirect` 缓存上游地址 TTL 300s。`rewrite` + MPD 正文损坏（`D-005`）。
11. 看门狗：`dash` 主清单就绪超时 30s；`remux` 预热 `503 Retry-After:2`；连续 5 次失败 → 冷却 3600s。
12. **切片缓存窗口**：首写 `public, max-age=300` → 命中 `private, max-age=300`；入口 `public, max-age=5, s-maxage=5`。
13. **播放 Token**：入口需 `?token=<48hex>`（全局稳定、TTL 24h）；失效 → 302 `token_failure_url`；**切片不受保护**。`random_id`/`ip_guard` 无可测效果。
14. **EPG**：关闭时 `/epg*` 全 404；开启后 `/epg.xml`、`/epg.xml.gz`、`/epg` 免登录 200。
    输出根 `generator-info-name="iptv-proxy"`；按 `keep_days`/`future_days` 裁剪；缺 `lang` 补 `zh`；配置为严格 TOML。
15. **访问范围**（`access`）：`public` 进总订阅；`hidden` 不进但可直连/分组订阅；`archived` 全停（EPG 照常）；`open` 免 token 且不进总订阅。
16. **配置保存**：面板改动经 `PUT /panel/api/config` 全量原子写并规范化；与文件直改有竞态；`m3u_header_lines` 热重载；EPG 开启注入 `x-tvg-url`。

## 3. 逆向阶段收尾（历史，2026-10-18）

- **路线图已收敛**：§5 的 P0/P1/P2 项均已完成或登记为环境受限/仅 088（见各行状态）。
- 已建：分组 `test1`（proxy）、`g2`、`gdash`（dash）、`gremux`（remux）；频道 `c1`/`c2`（HLS 上游）、`v`（MPD）、`r`（TS）。
- **未解项（已定界）**：`<P>` 生成算法（F-017，低影响）；`S-DIS-08/09`（切片 TS 结构/水印）仅部分核验；
  多 CDN 故障切换未观测到（`D-004`）；`epg/api/*` 仅 088 未测。
- **收尾**：主实例配置已还原默认；停 6 个测试实例与容器；`samples/work/` 下运行产物不纳入版本控制。
- 阶段报告：[`docs/findings/report-stage-01.md`](docs/findings/report-stage-01.md)。

## 3b. 实现阶段启动（历史）

- 阶段目标：按 [`docs/spec/08`](docs/spec/08-target-spec.md) 重实现 IPTV Proxy v0.8.2 的功能，**drop-in 兼容**。
- 节奏：先写设计（`docs/design/`），再写 P0 代码；P0 完成后与目标实例逐条对拍。
- 已完成：rustup 工具链（`rustc 1.99.1`）+ 两个 musl 目标；**M0 骨架**（配置读写、面板登录、内嵌 4 页前端、冒烟测试）；
  **M1 数据面**（分组/频道 CRUD、订阅导出、总/分组订阅与 `access` 过滤、播放 Token、分组管理页）。
- 已完成：**M4c 打包**——双架构 musl 静态包、`/opt/iptv-proxy` 布局、systemd/OpenRC 单元（见 §3g）。

## 3c. M1 数据面（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 分组 CRUD：`POST/PUT/DELETE /panel/api/groups[/{gid}]`（局部更新、`409`/`404` 错误串） | ✅ | `src/http/routes/groups.rs`；`smoke-m1.sh` |
| 频道 CRUD：`POST/PUT/DELETE /panel/api/groups/{gid}/channels[/{cid}]` | ✅ | 同上 |
| 导出：`GET /panel/api/groups/export`（数组）、`/{gid}/export`（对象） | ✅ | 同上 |
| 订阅：`GET /playlist.m3u`、`/{gid}/playlist.m3u`；`access` 过滤；直通组用上游地址；归档组订阅 `404` | ✅ | `src/playlist.rs`（6 个单元测试） |
| 播放 Token：48 位十六进制、内存、轮换宽限 1 h；`open` 组与直通组不带 Token | ✅ | `src/auth/play_token.rs` |
| 保存语义：改动立即原子重写 `config.toml`（以磁盘为事实源） | ✅ | `AppState::mutate` |
| 面板：侧栏 + 分组管理页（分组卡、频道表、弹窗增删改、复制订阅/播放地址） | ✅ | [`reference/ours/m1-groups.png`](docs/design/reference/ours/m1-groups.png) |
| 验证：`scripts/smoke-m0.sh`（M0 15 项）+ `scripts/smoke-m1.sh`（M1 全绿，debug 与 release）+ 无头浏览器端到端 9 项 | ✅ | 日志见 `PROGRESS` §4 |
| 未做（属 M2/M3）：播放链路 `/{gid}/{cid}.{ext}`、切片伪装与缓存、`redirect`/`rewrite`/`passthrough` 302 | ⏳ | `design/02` §14 |

## 3d. M2 播放链路（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 入口清单 `GET /{gid}/{cid}.m3u8`（`.mpd`）：`public, max-age=5, s-maxage=5`，指向 `/{gid}/{cid}/video/index.txt` | ✅ | `src/http/routes/stream.rs` |
| 媒体清单 `GET /{gid}/{cid}/video/index.txt`：抓上游、改写切片地址、`text/plain` | ✅ | `src/proxy/media.rs`（8 个单元测试） |
| 切片后缀 `<S>` = `sha256(上游绝对 URL)[:32]`；前缀 `<P>` = `sha256("<gid>:<cid>")[:16]` | ✅ | `smoke-m2.sh` 断言与独立计算一致 |
| 扩展名轮换：`[cache.disguise].formats`，每频道固定起点 | ✅ | `proxy::media::ext_offset` |
| 切片索引与字节缓存：`cache/seg_url/*.json`、`cache/seg_bytes/*`（重启可命中） | ✅ | `src/proxy/segments.rs` |
| 缓存窗口：首写 `public, max-age=300`；命中 `private, max-age=300` | ✅ | `smoke-m2.sh` |
| 忽略 `Range`（返回全量）；按伪装后缀给 `Content-Type` | ✅ | `smoke-m2.sh` |
| 302 占位：未知频道/分组、归档组、错 `<P>`；`404 Segment not found` | ✅ | `smoke-m2.sh` |
| Token：入口校验（`open` 组免校验），切片与媒体清单不校验 | ✅ | `smoke-m2.sh` |
| 主清单上游（含 `#EXT-X-STREAM-INF`）：变体地址绝对化透传 | ✅ | `smoke-m2.sh` |
| 未做（属 M3/M4）：`redirect` 地址 TTL 缓存、`rewrite` 清单改写、`passthrough` 302、`remux` 预热 `503`、`dash` 就绪判定 | ⏳ | `design/02` §14 |

验证：`cargo test` 18 项；`scripts/smoke-m0.sh`/`m1.sh`/`m2.sh` 在 debug 与 release 均全绿。

## 3e. M3 直连模式（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| `redirect`：入口 302 上游；地址按 `redirect_ttl` 缓存（0 → 300 s）；失败不缓存并跳 `error_url` | ✅ | `smoke-m3.sh`（上游请求计数 = 1） |
| `redirect` + `解析最佳画质`：取主清单挑最高 `BANDWIDTH` 变体 | ✅ | `proxy::modes::best_variant`（单元测试 + 冒烟） |
| `passthrough`：302 上游；**校验 Token**（F-018 修正）；不追加 Token | ✅ | `smoke-m3.sh` |
| `rewrite`：入口 200 `text/plain`，相对地址改绝对，切片客户端直连；清单缓存 ≈5 s | ✅ | `smoke-m3.sh` |
| `remux`：冷启动 `503` + `Retry-After: 2` + `remux warming up`；首批切片后 `200` | ✅ | `smoke-m3.sh`、§3ac |
| `dash`：MPD→HLS；30 s 就绪门；失败 `503 timeout waiting for master playlist` | ✅ | `smoke-m3.sh`、§3bc |
| MPD：`proxy` 正文原样透传；`dash` 解析并转 HLS | ✅ | `stream.rs`、`dash_hls.rs` |
| `remux`/`dash` 转封装、DASH 轨道命名与片段容错 | ✅ | ADR-04、S-MODE-06、§3bc |

> 本节保留 M3 当时的验证记录；`remux` 与 `dash` 的最终状态以 §3ac、§3bc 为准。

验证：`cargo test` 23 项；`smoke-m0.sh`…`smoke-m3.sh` 在 debug 与 release 均全绿。

## 3f. M4a 运维面（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 热重载：只监视配置文件本身（避免日志文件写盘误触发）；解析失败保留旧快照并记 `error` | ✅ | `src/reload.rs`；`smoke-m4.sh` |
| 看门狗：同一频道连续失败 5 次 → 冷却 3600 s；冷却期内不再请求上游；配置重载立即解除（S-MODE-17） | ✅ | `src/watchdog.rs`（单元测试 + 冒烟） |
| 缓存：`GET /panel/api/cache/stats`（按目录计数与字节）、`POST /panel/api/cache/flush`（保留 `dash/`，并丢弃进程内索引与清单缓存） | ✅ | `src/http/routes/admin.rs` |
| 指标：`GET /panel/api/stats`（请求数、分组请求数、清单/切片数、切片命中、上游失败、在线窗口） | ✅ | `src/stats.rs`（单元测试） |
| 日志：`GET /panel/api/logs` 返回环形缓冲（500 行）；同时输出到标准输出与 `<配置目录>/iptv-proxy.log`，超 5 MB 截断 | ✅ | `src/logging.rs`（单元测试） |
| 对拍（M4b）：同一夹具下 22 项用例，差异 0；发现并修正 11 处契约差异 | ✅ | `scripts/compare-p0.sh`、`design/reconciliation-report.md`、F-018 |
| 打包（M4c）：Docker 双架构静态包 + `/opt/iptv-proxy` 布局 + systemd/OpenRC | ✅ | §3g、`design/04` §6 |

验证：`cargo test` 30 项；`smoke-m0.sh`…`smoke-m4.sh` 全绿。

## 3g. M4c 打包（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 静态构建：`aarch64-unknown-linux-musl`、`x86_64-unknown-linux-musl` | ✅ | `scripts/build-release.sh`：Docker `rust:1-alpine`（镜像宿主三元组即 musl），依赖走宿主 `~/.cargo` 缓存 `--offline` |
| 发布布局：`/opt/iptv-proxy`（二进制、配置样例、启停脚本、安装脚本、服务单元） | ✅ | `packaging/`、[`design/04`](docs/design/04-config-and-storage.md) §6 |
| 产物校验：ELF 无 `PT_INTERP`、容器内启动、`/health` = `ok` | ✅ | `scripts/verify-release.sh`：arm64、amd64 均通过 |
| 安装脚本：不覆盖已有配置；按存在性注册 systemd 或 OpenRC | ✅ | alpine:3.20 容器内 `install.sh` + `iptv-proxy.sh start/status/stop` 实测 |
| 产物 | `dist/iptv-proxy-0.1.0-{arm64,amd64}.tar.gz`（2.5 / 2.7 MB，附 `.sha256`） | 不入库（`.gitignore` 已忽略 `/dist/`） |

验证：`scripts/verify-release.sh arm64` 与 `... amd64` 均输出「静态链接 OK」「/health = ok」。

## 3h. 集成测试（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 路由级集成测试：起真实 HTTP 服务 + 伪上游，用 `reqwest`（不跟随跳转）观测契约 | ✅ | `tests/http_contract.rs`（10 项） |
| 覆盖：健康检查与总订阅、媒体清单入口、主清单入口、变体清单与别名、切片首写/命中缓存头、Token 与未知路径顺序、归档组、`passthrough` 校验、未知面板接口 | ✅ | 同上 |

验证：`cargo test` 共 40 项（30 单元 + 10 集成）全绿。

> 说明：本机访问 `github.com` release 资产与 `static.rust-lang.org` 的 TLS 会被中途断开，
> 因此不用 `cross`，改用 Docker Hub 可用的 `rust:1-alpine` 完成双架构构建。

## 3i. P1 · EPG（已完成，首个 P1 功能）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 配置：`[epg]` 全字段 + `[[epg.sources]]`（键面对齐目标） | ✅ | `src/config/model.rs` |
| 采集：拉 XMLTV → 解析 → 时间窗裁剪 → 频道匹配 → 生成 `epg.xml` 与 `epg.xml.gz` | ✅ | `src/epg.rs`（6 项单元测试） |
| 公开面：`/epg.xml`、`/epg.xml.gz`、`/epg`；关闭时三者 `404`（S-EP-05） | ✅ | `src/http/routes/epg.rs` |
| 面板：`/panel/api/epg/status`、`/panel/api/epg/refresh`（Bearer） | ✅ | 同上 |
| 公开 JSON：`/epg/api/{channels,programmes/:channel,search}` | ✅ | `src/http/routes/epg.rs`；集成测试 `epg_api_exposes_channels_programmes_and_search` |
| 浏览页：`/epg` 频道列表 + 节目列表 + 搜索（自研 UI，不复用目标文案） | ✅ | `frontend/src/epg.tsx` |
| 调度：启动即采集；之后按 `0 */N * * *` 或 4 小时 | ✅ | `epg::spawn` |
| 验证 | ✅ | `scripts/smoke-epg.sh` 13 项全绿；`tests/http_contract.rs` EPG 2 项 |
| 未做 | ⏳ | 脚本源、翻译接口、映射/录制联动（P2） |

验证：`cargo test` 共 51 项（36 单元 + 15 集成）；`scripts/smoke-epg.sh` 18 项全绿。

## 3j. 回看参数与兜底（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 回看：入口只把 `playseek` 追加到上游；缓存键带地址标签，回看区间不互相覆盖 | ✅ | `stream.rs::with_playseek`/`url_tag`；集成测试 `playseek_is_forwarded_to_upstream` |
| 兜底：未知单段路径 → `404`（目标 `/status` 行为，S-EP-08） | ✅ | 集成测试 `single_segment_unknown_path_is_404` |

验证：`cargo test` 50 项（36 单元 + 14 集成）全绿。

## 3k. P1 · 客户端地址与分组导入（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| `GET /whoami`、`GET /unban/whoami` → `{"ip":…}`（`X-Forwarded-For` 首项 / `X-Real-IP` / `unknown`） | ✅ | `src/http/routes/public.rs` |
| `POST /panel/api/groups/import`：分组数组导入，重复 id 跳过 | ✅ | `src/http/routes/groups.rs` |
| `POST /panel/api/groups/import/force`：重复 id 覆盖 | ✅ | 同上 |
| 校验：非数组 → `400`；未登录 → `401`；导入后订阅立即可见 | ✅ | 集成测试 `group_import_adds_skips_and_overwrites`、`whoami_reports_forwarded_ip` |

验证：`cargo test` 53 项（36 单元 + 17 集成）全绿。

## 3l. P1 · M3U 导入（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 解析：`#EXTINF` 属性、`#EXTVLCOPT`、`#EXTHTTP`、行内头 | ✅ | `src/m3u.rs`（5 项单元测试） |
| 落库：`append`/`merge`/`replace`；`auto` 按 `.mpd` 选 `dash`/`rewrite` | ✅ | `groups.rs::import_m3u` |
| id 规则：分组去非 ASCII、空则 `g-<6hex>`；频道优先 `tvg-id` | ✅ | `m3u::group_id`/`channel_id` |
| 属性保留：`tvg_*`/`group_title` → `extra`；头 → `channel.headers`（只落库） | ✅ | 同上；转发附加留 P2 |
| 校验：非法 `mode`/`group_mode`、空 `text`、无频道 → `400` | ✅ | 集成测试 `m3u_import_creates_groups_and_channels` |
| 文档 | ✅ | `docs/design/03-http-contract.md` §6.2.1 |

验证：`cargo test` 59 项（41 单元 + 18 集成）全绿。

## 3m. 发布产物重建（P1 批次后）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 修复 `build-release.sh` 无参数时把 `amd64 arm64` 当作单架构的问题 | ✅ | `scripts/build-release.sh` |
| 重建双架构静态包（含 EPG 页面与 P1 接口） | ✅ | `dist/iptv-proxy-0.1.0-{amd64,arm64}.tar.gz`（3.0/2.8 MB） |
| 校验：ELF 无 `PT_INTERP`、能启动、`/health = ok` | ✅ | `scripts/verify-release.sh amd64/arm64` |
| 校验：发布包内 `/epg` 页面引用新资源，`/__assets/assets/epg-*.js` 返回 2866 字节且含页面文案 | ✅ | alpine:3.20 + amd64 包实测 |

> `dist/` 不入库（`.gitignore`）。

## 3n. P1 · 台标库（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 存储：`<cache>/logos/`；白名单扩展名 `png/jpg/jpeg/webp/bmp/avif`；文件名去路径与危险字符 | ✅ | `src/logo.rs`（5 项单元测试） |
| 公开面：`/logo` 页面、`/logo/<file>`（`public, max-age=86400`，缺文件/越界名/非法扩展 → 404）、`/logo/api/list` | ✅ | `src/http/routes/logo.rs` |
| 面板：`/panel/api/logo/{list,upload,delete}`；`data:` URL 与纯 base64 都接受；空内容与非法扩展进 `failed` | ✅ | 同上 |
| 前端：台标库页（网格、上传、删除；上传按钮仅在已登录时显示） | ✅ | `frontend/src/logo.tsx` |
| 验证 | ✅ | `scripts/smoke-logo.sh` 15 项；集成测试 `logo_library_upload_serve_and_delete` |

验证：`cargo test` 64 项（45 单元 + 19 集成）。

## 3o. P1 · 备份与恢复（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| `GET /panel/api/backup`：gzip(tar)，`all`/`epg-scripts`/`resolver` 三种作用域，文件名带 UTC 时间戳 | ✅ | `src/backup.rs`、`routes/backup.rs` |
| 包布局：`config.toml`、`logos/`、`epg/scripts/`、`epg/epg_map.json`、`resolvers/`、`manifest.json` | ✅ | 同上 |
| tar 读写内置（`src/backup/tar.rs`），不引入第三方库；离线交叉编译不受影响 | ✅ | 2 项单元测试 |
| `POST /panel/api/restore`：解包 → 校验配置 → `config.toml.bak` → 原子写 → 恢复台标 → 重载 | ✅ | 同上 |
| 请求体上限 64 MiB；非法内容 → `400` | ✅ | `http.rs` |
| 验证 | ✅ | `scripts/smoke-backup.sh` 10 项；集成测试 `backup_and_restore_roundtrip`；`backup` 3 项单元测试 |

验证：`cargo test` 68 项（48 单元 + 20 集成）。

## 3p. P1 · 频道自定义头转发（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| `channel.headers`（`"名称: 值"`）在取上游清单与切片时附加 | ✅ | `stream.rs::channel_headers` + `fetch`/`fetch_bytes` |
| 覆盖模式：`proxy`、`rewrite`、`dash`、`remux` | ✅ | 同上；302 模式由客户端直连，无法附加 |
| 非法条目（无冒号、空名）忽略，不影响播放 | ✅ | `channel_headers` |
| 验证 | ✅ | 集成测试 `channel_headers_are_sent_upstream`（伪上游回显） |

验证：`cargo test` 69 项（48 单元 + 21 集成）。

## 3q. P1 · 安全面（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| Token：`GET /panel/api/security/token`、`POST …/token/rotate`（旧值 1 小时宽限） | ✅ | `routes/security.rs`；`PlayTokenStore` 复用 |
| IP 统计与封禁：`ip/stats`、`ip/ban`（默认 1 小时、上限 30 天）、`ip/unban`（单个或批量） | ✅ | `src/auth/ip_guard.rs`（4 项单元测试） |
| 执行：非面板路径按转发头记账，封禁期内 `403`；面板路径不拦截 | ✅ | `src/http/guard.rs`（中间件） |
| 容量：最多 10 000 个地址 | ✅ | 同上 |
| 验证 | ✅ | `scripts/smoke-security.sh` 10 项；集成测试 `security_token_rotation_and_ip_ban` |

验证：`cargo test` 74 项（52 单元 + 22 集成）。

## 3r. P1 · 订阅聚合（已完成）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| `[playlist]` 配置：`merge_ttl_secs`、`merge_position`、`global_exclude`、`[[playlist.sources]]` | ✅ | `config/model.rs` |
| 合并：只作用于 `GET /playlist.m3u`；保留上游原始 URL，不改写、不加 Token | ✅ | `playlist::aggregate`、`compose`、`routes/play.rs` |
| 过滤词：`group:`、`id:`、频道名；大小写不敏感；`exclude` 优先 | ✅ | 同上（4 项单元测试） |
| 缓存：键含时间桶，按 `merge_ttl_secs` 生效 | ✅ | `AppState::sources` |
| 失败隔离：单源失败只记日志，本地订阅照常返回 | ✅ | `aggregate_sources` |
| 验证 | ✅ | `scripts/smoke-aggregate.sh` 5 项；集成测试 2 项 |

验证：`cargo test` 81 项（56 单元 + 25 集成）。

## 3s. P1 · 面板分区（安全/日志/备份）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 安全管控页：Token 展示与轮换、IP 统计表、封禁与解封 | ✅ | `frontend/src/ops.tsx` |
| 运行日志页：`/panel/api/logs?limit=`，5 秒轮询、可改行数 | ✅ | 同上 |
| 备份恢复页：作用域选择、浏览器内下载；上传 tar.gz 恢复并提示文件数 | ✅ | 同上 |
| 侧栏：已实现分区可切换，其余置灰「待实现」 | ✅ | `frontend/src/panel.tsx` |
| 验证 | ✅ | `pnpm build` 通过；实测 `/panel` 引用 `panel-*.js` 且内含 `security/token` |

验证：`cargo test` 81 项不变（前端不参与单元测试）。

## 3t. P1 · 缓存与热度分区

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 缓存状态页：子目录文件数、总占用、内存清单项数、刷新与清空（二次确认） | ✅ | `frontend/src/cache.tsx` |
| 频道热度页：运行时长、请求/切片/命中/上游失败、分组排行 | ✅ | 同上 |
| 侧栏：两个分区移出「待实现」 | ✅ | `frontend/src/panel.tsx` |
| 验证 | ✅ | `pnpm build` 通过；`cargo test` 81 项 |

## 3u. P1 · 节目指南分区

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 状态：开关、源数、频道/节目数、保留窗口、最近成功与错误 | ✅ | `frontend/src/ops.tsx` → `EpgPanelPage` |
| 操作：立即采集（同步）、刷新状态、新窗口打开 `/epg` | ✅ | 同上 |
| 验证 | ✅ | `pnpm build` 通过 |

## 3v. P1 · 发布重建与校验增强

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 双架构静态包重建（含 EPG、台标、备份、安全、聚合与全部新面板分区） | ✅ | `dist/iptv-proxy-0.1.0-{amd64,arm64}.tar.gz`（3.16 MB / 2.94 MB） |
| `verify-release.sh` 新增：面板页 200、`panel-*.js` 含 `security/token`、`/logo/api/list` 返回 `count` | ✅ | 两架构均输出 `面板资源 = /__assets/assets/panel-*.js` |
| 离线构建修复：备份 tar 改内置实现，移除 `tar` 依赖（缓存缺 `linux-raw-sys`） | ✅ | `src/backup/tar.rs`；`cargo test --offline` 83 项 |

验证：`cargo test --offline` 83 项；`verify-release.sh arm64|amd64` 全绿；`smoke-backup`、`smoke-security` 复跑通过。

## 3w. P1 · 台标库其余端点

| 项 | 状态 | 证据 |
| --- | --- | --- |
| `POST /panel/api/logo/rename`：保留扩展名，支持中文名，目标已存在报错 | ✅ | `logo::rename`（单元测试） |
| `GET /panel/api/logo/match-report`：缺台标、坏引用、未使用 | ✅ | `routes/logo.rs` |
| `POST /panel/api/logo/apply`：写入频道 `logo`，订阅立即生效 | ✅ | 同上；`playlist` 兼容 `tvg_logo` 回退 |
| `POST /panel/api/logo/suggest`：字符集合 Jaccard 相似度（0–1，阈值与条数可调） | ✅ | `logo::similarity`（单元测试） |
| 前端：匹配报告摘要、缺台标频道表（下拉 + 应用）、卡片重命名 | ✅ | `frontend/src/logo.tsx` |
| 验证 | ✅ | `scripts/smoke-logo.sh` 25 项；集成测试 `logo_rename_apply_and_match_report` |

验证：`cargo test --offline` 86 项（60 单元 + 26 集成）。

## 3x. P1 · 台标端点的发布重建

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 双架构包重建（含台标重命名/应用/报告/推荐页面与 unban-batch 路由） | ✅ | `dist/iptv-proxy-0.1.0-{amd64,arm64}.tar.gz`（3.19 MB / 2.97 MB） |
| `verify-release.sh` 两架构校验：静态链接、`/health`、`/logo/api/list`、面板资源含 `security/token`、台标资源含 `match-report` | ✅ | 两架构均输出 `panel-DuIKO3OL.js` 与 `logo-g_CaRg2R.js` |
| 冒烟复跑 | ✅ | `smoke-logo` 25 项、`smoke-security` 11 项、`smoke-m1` |

## 3y. P1 · 素材与脚本管理

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 视频素材：`assets/{list,upload,remove,raw}`；扩展名白名单 `mp4/mkv/ts/mov/webm/m4v` | ✅ | `src/assets.rs`（2 单元）、`routes/media.rs` |
| 字幕：`subtitle/{list,upload,remove,raw}`；扩展名 `srt/ass/ssa/vtt` | ✅ | 同上 |
| `GET /panel/api/assets/ffmpeg-status`：实时执行 `ffmpeg -version`，返回 `installed/version/remux_ready` | ✅ | `routes/media.rs` |
| 解析脚本：`resolver/{scripts,script,remove}`，写 `<cache>/resolvers/*.py`，不执行 | ✅ | 同上 |
| 前端：视频素材分区（ffmpeg 状态、上传、打开、删除；字幕同页） | ✅ | `frontend/src/assets.tsx` |
| 验证 | ✅ | `scripts/smoke-assets.sh` 13 项；集成测试 `media_assets_subtitles_and_resolver_scripts` |

验证：`cargo test --offline` 89 项（62 单元 + 27 集成）。

## 3z. P1 · 多 VPS 分享

| 项 | 状态 | 证据 |
| --- | --- | --- |
| `GET /api/share/pull`：双重门禁（key + trust_ips），`403` 日志 | ✅ | `routes/share.rs`、`share::ip_allowed` |
| 载荷 `{token, token_old, bans, unbans}` | ✅ | `share::payload` |
| follower 同步：采用主机 Token；本地轮换 → `409` | ✅ | `share::spawn`、`play_tokens.adopt` |
| master 汇总：轮询 `follower_urls`，合并封禁/解封 | ✅ | `share::apply` |
| 源 IP：`X-Forwarded-For` → `X-Real-IP` → TCP 对端 | ✅ | `http::ext::client_ip_with_peer`、`ConnectInfo` |
| 配置 `[share]`：enabled/role/key/trust_ips/master_url/follower_urls/poll_secs | ✅ | `config/model.rs` |
| 验证 | ✅ | `scripts/smoke-share.sh` 7 项；集成测试 2 项；单元 3 项 |

验证：`cargo test --offline` 95 项（66 单元 + 29 集成）。

## 3aa. P1 · 健康与 DASH 探测

| 项 | 状态 | 证据 |
| --- | --- | --- |
| `GET /panel/api/health`：版本、运行时长、分组/频道数、EPG/安全/分享摘要 | ✅ | `routes/dash.rs` |
| `GET /panel/api/dash/tracks/:gid/:cid`：抓 MPD 并列出轨道 | ✅ | `src/dash.rs`、`routes/dash.rs` |
| `POST /panel/api/dash/scan-url`：解析任意 MPD，只读 | ✅ | 同上 |
| `GET /panel/api/dash-timeshift/status`：占位，`enabled=false` | ✅ | 同上 |
| 测试 | ✅ | dash 2 单元 + 1 集成；`scripts/smoke-panel.sh` 7 项 |

验证：`cargo test --offline` 98 项（68 单元 + 30 集成）。

## 3ab. P1 · 上游解析脚本

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 频道键 `resolver_script`/`resolver_arg`/`resolver_ttl`（默认 30 秒） | ✅ | `src/resolver.rs` |
| 执行 `python3 <cache>/resolvers/<脚本> <arg>`，stdout 末行即 URL | ✅ | 同上 |
| 30 秒超时；非 HTTP(S) 输出报错 | ✅ | 同上 |
| SWR 缓存：TTL 内不执行；过期先返回旧值并后台刷新 | ✅ | `resolver::resolve` |
| 冷启动失败 → `302 error_url`；`playseek` 追加到解析结果 | ✅ | `routes/stream.rs` |
| 验证 | ✅ | 3 单元 + 1 集成；`scripts/smoke-resolver.sh` 5 项 |

验证：`cargo test --offline` 102 项（71 单元 + 31 集成）。

发布包：双架构重建（amd64 3.36 MB、arm64 3.12 MB），`verify-release.sh` 双架构通过（静态、`/health`、面板资源、台标、`/panel/api/health=401`、分享默认 404）。

## 3ac. P1 · Remux 转封装

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 连续 TS → HLS：PAT/PMT 解析、PES PTS、关键帧切分（H264/HEVC） | ✅ | `src/remux.rs` |
| 预热 503 + `Retry-After: 2`；就绪后清单（4 条窗口、MEDIA-SEQUENCE） | ✅ | `routes/stream.rs` |
| 伪装切片：`rseg_<序号>.<png|gif|bmp|jpg|webp>`，图片 Content-Type | ✅ | `remux::serve` |
| 会话：`gid/cid` 去重、断流重连、90 秒空闲回收 | ✅ | `remux::Manager` |
| 频道自定义头上游 | ✅ | `channel_headers` |
| 验证 | ✅ | 4 单元 + 1 集成；`scripts/smoke-remux.sh` 7 项（真实 ffmpeg TS） |

## 3ad. P1 · 定时录制与直播开关

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 任务 CRUD（下标语义）、停止、下载 | ✅ | `src/schedule.rs`、`routes/schedule.rs` |
| 调度：10 秒扫描，到期转 `recording` | ✅ | `schedule::spawn` |
| 录制：媒体清单增量切片、主清单选最高码率、合并 | ✅ | `schedule::record` |
| 产物：`<cache>/recordings/`；有 ffmpeg 时 MKV，否则保留 TS | ✅ | 同上 |
| 直播开关：`livestream/{start,stop}` 改写配置并落盘 | ✅ | `routes/schedule.rs` |
| 验证 | ✅ | 2 单元 + 1 集成；`smoke-schedule.sh` 9 项、`smoke-record.sh` 5 项（真实 ffmpeg HLS） |

> 时移（`/timeshift/*`、`/dash-timeshift/*`）仍为 P2。

## 3ae. 发布包（Remux + 录制）

| 项 | 状态 |
| --- | --- |
| amd64 | 3.56 MB |
| arm64 | 3.31 MB |
| `verify-release.sh` 双架构 | ✅ 静态链接、`/health`、面板/台标资源、健康 401、分享默认 404 |

验证：`cargo test --offline` 121 项（80 单元 + 41 集成）。

已知缺口（P2）：见 §3ak 订阅聚合面板、§3ap EPG 翻译引擎、§3aq 字幕翻译引擎（均已补齐）。

## 3af. P2 · EPG 脚本源、批量录制、字幕自检

| 项 | 状态 | 证据 |
| --- | --- | --- |
| EPG `kind="script"`：`python3 <cache>/epg/scripts/<script>`，stdout=XMLTV，超时与 `output_files` 兜底 | ✅ | `epg::run_script` |
| 源全失败时返回具体错误（不再只报「没有可用的 EPG 源」） | ✅ | `epg::collect` |
| `schedule/from-epg`：批量条目建任务，返回 `created/ids/failed` | ✅ | `routes/schedule.rs` |
| `/subtitle/mtran/test`：未配地址 `400`；已配则调用 MTranServer 并返回逐目标结果 | ✅ | `routes/media.rs` |
| `/subtitle/translate/clear`、`/epg/translate/{status,clear-engine}` | ✅ | 同上、`routes/epg.rs` |
| 验证 | ✅ | 4 集成（脚本源 2、批量 1、字幕 3 合 2 项） |

## 3ag. P1 · 直播回看（代理频道）

| 项 | 状态 | 证据 |
| --- | --- | --- |
| `livestream=true` 时持续缓冲：媒体清单增量下载、主清单选最高码率 | ✅ | `src/timeshift.rs` |
| 回看参数：`start/end`、`playseek`（本地时区） | ✅ | `timeshift::range` |
| VOD 清单：`PLAYLIST-TYPE:VOD`、`TARGETDURATION`、`ENDLIST` | ✅ | `Session::playlist` |
| 切片：`/{gid}/{cid}/timeshift/<16hex>.<ext>`，图片类型，`private, max-age=3600` | ✅ | `Manager::serve` |
| 越界 → 实时清单；空闲 15 分钟回收 | ✅ | `routes/stream.rs`、`Manager` |
| 启动预热与点亮即缓冲 | ✅ | `timeshift::warm`、`livestream_switch` |
| 面板状态：`/panel/api/dash-timeshift/status` 报告频道与已缓冲片数 | ✅ | `routes/dash.rs` |
| 验证 | ✅ | 3 单元 + 1 集成；`scripts/smoke-timeshift.sh` 8 项（真实 ffmpeg HLS） |

## 3aj. P2 · DASH 回看

| 项 | 状态 | 证据 |
| --- | --- | --- |
| `dash_timeshift=true` 时用 ffmpeg 转本地 HLS（`-hls_time 4`，滑动窗口 120 片） | ✅ | `timeshift::spawn_ffmpeg` |
| 本地 HLS 增量入缓冲，VOD 清单与伪装切片复用 | ✅ | `spawn_poller` |
| 会话回收时结束 ffmpeg | ✅ | `Session::shutdown` |
| 冒烟 | ✅ | `scripts/smoke-dash-timeshift.sh` 4 项通过（Linux arm64 容器，见 §3ar） |

## 3ah. P1 · 面板「录制与回看」分区

| 项 | 状态 |
| --- | --- |
| 新建任务：分组/频道联动、时间、格式 | ✅ `frontend/src/schedule.tsx` |
| 任务表：状态、产物大小、停止、删除、带 Bearer 下载 | ✅ |
| 直播回看表：逐频道点亮/停止，显示已缓冲片数 | ✅ |
| 回看能力卡片：`dash-timeshift/status` | ✅ |
| 侧栏：移除占位「定时录制」「回看管理」 | ✅ |
| 前端构建 | ✅ `npm run build`（panel 32.4 kB） |

## 3ai. 发布包（EPG 脚本源 + 回看 + 面板）

| 项 | 状态 |
| --- | --- |
| amd64 | 3.56 MB |
| arm64 | 3.31 MB |
| `verify-release.sh` 双架构 | ✅ 面板资源 `panel-CiSNGll5.js`、台标资源、健康 401、分享默认 404 |

验证：`cargo test --offline` 120 项；smoke：assets 13、share 7、panel 7、resolver 5、remux 7、schedule 9、timeshift 8、record 5；DASH 回看按 ffmpeg 能力跳过。

## 3ak. P2 · 订阅聚合配置与面板

| 项 | 状态 | 证据 |
| --- | --- | --- |
| `GET/PUT /panel/api/playlist`：全量替换，校验 URL、合并位置、TTL 夹取 | ✅ | `routes/playlist.rs` |
| 空源 ID 自动生成；非法 URL → `400` | ✅ | 同上 |
| 面板「订阅聚合」：源增删改、启用、全局排除、复制聚合地址 | ✅ | `frontend/src/playlist.tsx` |
| 验证 | ✅ | 1 集成；前端构建 panel 35.6 kB |

## 3al. 最终发布包（订阅聚合 + DASH 回看）

| 项 | 状态 |
| --- | --- |
| amd64 | 3.56 MB |
| arm64 | 3.31 MB |
| `verify-release.sh` 双架构 | ✅ 面板资源 `panel-*.js`、台标资源、健康 401、分享默认 404 |

验证：`cargo test --offline` 121 项；全部冒烟通过（DASH 回看按 ffmpeg 能力跳过）。

## 3am. P2 · 面板「系统状态」分区

| 项 | 状态 |
| --- | --- |
| 健康摘要：版本、运行时长、监听、分组/频道、会话 | ✅ `frontend/src/system.tsx` |
| 能力：ffmpeg、回看缓冲（频道/片数）、解析脚本列表 | ✅ |
| 刷新按钮；数据源四个面板接口 | ✅ |
| 前端构建 | ✅ panel 37.9 kB |

## 3an. P2 · EPG 翻译接口面

> 本节为初版接口面；引擎本体见 §3ap，状态与行为以 §3ap 为准。

| 端点 | 行为 |
| --- | --- |
| `GET /epg/translate/status` | `enabled`（读 `[epg.translate].enabled`）、`engine=none` |
| `GET /epg/translate/models` | 空列表 + 说明 |
| `POST /epg/translate/run` | 未启用 → `400 转换未启用`；已启用但无引擎 → `400` |
| `POST /epg/translate/probe` | 未配地址 → `400`；否则探测 `<url>/translate/batch` |
| `POST /epg/translate/mtran/test` | 未配地址 → `400`；否则逐目标结果 |
| `POST /epg/translate/{clear,clear-engine}` | 清空 `<cache>/epg/.translate/` |

| 验证 | ✅ 1 集成（未配置引擎路径） |

## 3ao. 最终发布包（系统状态 + EPG 翻译接口面）

| 项 | 状态 |
| --- | --- |
| amd64 | 3.56 MB |
| arm64 | 3.31 MB |
| `verify-release.sh` 双架构 | ✅ 面板资源、台标资源、健康 401、分享默认 404 |

验证：`cargo test --offline` 121 项；smoke：assets 13、share 7、panel 7、resolver 5、remux 7、schedule 9、timeshift 8、record 5。

## 3ap. P2 · EPG 翻译引擎本体

| 项 | 行为 |
| --- | --- |
| 配置 | `[epg.translate]`：`enabled`、`url`、`source`（默认 `auto`）、`target` 或 `targets`、`translation_first` |
| 采集钩子 | 频道匹配与时间窗过滤之后、写文件之前翻译；只翻节目文本节点 |
| 请求 | `POST <url>/translate/batch`，正文 `{texts, source_lang, target_lang}`；按 40 条分批 |
| 响应 | 接受顶层数组，或 `translations`/`texts`/`result`/`text`/`data` 数组；元素可为字符串或含 `text`/`translation` 的对象 |
| 输出 | 译文追加为 `lang="<target>"` 的文本节点；`translation_first = true` 时排原文前 |
| 缓存 | `<cache>/epg/.translate/cache.json`，键为 `sha256("<target>\\u{1f}<text>")`；命中不发请求；原子写 |
| 失败 | 翻译失败只记告警，不阻断节目单生成；原文保留 |
| 接口 | `status` 报 `engine=http`、目标语言、缓存条数；`models` 返回目标语言；`run` 同步采集并返回 `translated`；`probe` 探测端点 |
| 门禁 | `POST /epg/translate/{run,probe}` 需面板会话；`status`/`models`/`clear*`/`mtran/test` 公开 |

| 验证 | ✅ 3 单元（配置、目标优先级、响应形状）+ 1 集成（译文写入、缓存复用、`run`） |

验证：`cargo test --offline` 125 项（83 单元 + 42 集成）。

## 3aq. P2 · 字幕翻译引擎本体

| 项 | 行为 |
| --- | --- |
| 配置 | `[subtitle.translate]`：`enabled`、`url`、`source`（默认 `auto`）、`target` 或 `targets`、`translation_first` |
| 输入 | 已上传的 `srt`/`vtt`；UTF-8 文本；其它格式或编码 → `400` |
| 解析 | 只取时间行之后的字幕正文；跳过序号、`NOTE`/`STYLE` 块、WebVTT 设置行与纯符号行 |
| 翻译 | 正文行去重后按 40 行分批；调用 `<url>/translate/batch`；译文为空时回退原文 |
| 输出 | `translation_first=false` → 原文 + 译文；`true` → 译文 + 原文；时间轴原样保留 |
| 缓存 | `<cache>/subtitles/.translate/<hash>.<ext>`；命中直接返回；原子写 |
| 接口 | `GET …/translate/status`、`POST …/translate`、`GET …/translate/raw/<name>`；前两者需面板会话 |
| 面板 | 字幕分区显示目标语言选择与逐文件「翻译」按钮；下载走带会话头的 Blob |

| 验证 | ✅ 4 单元（格式识别、取行、双语顺序、结尾换行）+ 2 集成（翻译/缓存/下载、未配置 → `400`） |

验证：`cargo test --offline` 132 项（88 单元 + 44 集成）；前端 panel 39.44 kB。

## 3ar. P2 · DASH 回看端到端复验

本机 ffmpeg 无 `dash` 解复用器。改用 Linux arm64 容器复验。

| 项 | 做法 |
| --- | --- |
| 二进制 | `target/dash-arm64/debug/iptv-proxy`（Docker `rust:1-alpine` 构建，`--offline`） |
| 运行时 | `charmingcheung000/mpd-hls`（arm64，自带 ffmpeg：`dash` 解复用器 + `libx264` + dash 封装器） |
| 上游 | `ffmpeg -f lavfi -i testsrc -f dash -use_timeline 1` 生成 MPD；`python3 -m http.server` 提供 |
| 配置 | `mode = "dash"`、`dash_timeshift = true`、`dash_timeshift_hours = 4`、`access = "open"` |
| 断言 | 回看清单 `200` 且含 `#EXT-X-PLAYLIST-TYPE:VOD`；切片 `200` 且 `Content-Type: image/*` |
| 脚本 | `scripts/smoke-dash-timeshift-docker.sh` + `scripts/dash-smoke-container.py`；先等上游 MPD 可读，再起本实现 |

| 结果 | ✅ 4/4，连续 3 次通过（清单 200、VOD 类型、切片 200、图片类型） |
| 限制 | 容器内无 `bash`/`curl`，用等价 Python 断言复现 `smoke-dash-timeshift.sh` |

验证：`cargo test --offline` 132 项（88 单元 + 44 集成）。

## 3as. 重新出包（P2 翻译引擎 + DASH 复验）

| 项 | 状态 |
| --- | --- |
| amd64 | 3.4 MB |
| arm64 | 3.2 MB |
| 构建 | `scripts/build-release.sh`：`rust:1-alpine` 双架构，`--offline`，内嵌最新前端 |
| `verify-release.sh` | ✅ 静态链接、`/health`、面板/台标资源、健康 401、分享默认 404 |

验证：`cargo test --offline` 132 项；smoke 13 个脚本全通过（assets、panel、resolver、share、remux、schedule、timeshift、record、epg、aggregate、backup、logo、security）；DASH 容器冒烟 4 项 × 3 次。

P2 已关闭；后续计划见 §0.6。

## 3at. P3 · 订阅源预检端点（已完成）

证据：S-GAP-07。

| 项 | 结果 | 位置 |
| --- | --- | --- |
| `GET /subs/status` | 返回配置源：`id`、`name`、`url`、`enabled`、`include`、`exclude` | `src/http/routes/subs.rs` |
| `POST /subs/groups` | 抓取源，返回分组与条目数 | 同上 |
| `POST /subs/preview` | 分组映射、`global_exclude`、白/黑名单，返回 `trace` 与 `sample` | `src/subs.rs` |
| 错误 | 非 http(s) → `400`；抓取失败或非成功状态 → `502` | `ApiError::BadGateway` |
| 规则 | 无映射保留全部；有映射时未列出分组默认丢弃；顺序固定 | `src/subs.rs` |
| 测试 | 4 单元 + 4 集成 | `src/subs.rs`、`tests/http_contract.rs` |

验证：`cargo test --offline` 140 项（92 单元 + 48 集成）。

## 3au. P3 · 播放链接审计（已完成）

证据：S-GAP-04。目标样本已验形状。

| 项 | 结果 |
| --- | --- |
| 端点 | `GET /panel/api/link-info`（需要面板会话） |
| 形状 | `{"aliases":{},"random_id_enabled":false,"token":"<48hex>","token_enabled":true,"use_alias":false}` |
| `aliases` | 读 `[auth]` 的 `aliases` 表；缺失时为空对象 |
| `token` | `token_enabled = true` 时返回当前 Token；否则为空串 |
| `use_alias` | `aliases` 非空时为 `true` |
| 测试 | `link_info_matches_target_shape`；未登录返回 `401` |

验证：`cargo test --offline` 141 项（92 单元 + 49 集成）。

## 3av. P3 · 登录锁与自助解封（已完成）

证据：F-010 §4、S-EP-07、S-AUTH-07。

| 项 | 结果 | 位置 |
| --- | --- | --- |
| 登录锁 | 窗口 60 秒、免费 15 次、阈值 6；失败数达到两者之和后，后续登录返回 `429`，锁 300 秒 | `src/auth/login_guard.rs` |
| 锁定范围 | 只拦 `/panel/api/login`；既有 Bearer 会话仍可用 | `src/http/routes/panel.rs` |
| 自助解封 | `GET /unban` 与 `POST /unban` 免登录；封禁期也可访问 | `src/http/routes/public.rs`、`src/http/guard.rs` |
| 配额 | 每个 IP 30 天 3 次；成功解封才计数；内存保存 | `src/auth/unban.rs` |
| 前端 | 显示当前地址、剩余次数与结果 | `frontend/src/unban.tsx` |
| 测试 | 3 + 2 个单元；2 个集成 | `src/auth/*`、`tests/http_contract.rs` |

不实现：`l1/l2` 限流与每 IP 会话上限。目标触发条件未复现（F-010 §5）。

验证：`cargo test --offline` 148 项（97 单元 + 51 集成）。

## 3aw. P3 · ASS/SSA 字幕翻译（已完成）

证据：D19 §3.5 登记 ASS/SSA 未测。行为为本实现自定。

| 项 | 结果 | 位置 |
| --- | --- | --- |
| 检测 | `[Script Info]` 或 `[Events]` → ASS/SSA | `src/subtitle.rs` |
| 转换 | 读 `Format:` 与 `Dialogue:`；去 `{\…}`；`\N` 转换行；`\h` 转空格 | 同上 |
| 时间 | `H:MM:SS.cc` → `HH:MM:SS.mmm` | 同上 |
| 翻译 | 转 WebVTT 后走既有逐行流程；缓存后缀 `.vtt` | `src/http/routes/media.rs` |
| 状态 | `/subtitle/translate/status` 增 `formats`、`converts_ass` | 同上 |
| 测试 | 1 个单元 + 1 个集成 | `src/subtitle.rs`、`tests/http_contract.rs` |

不实现：播放侧 `compat_mode` 与 `srt_ua_keywords`。目标行为未观测（D19 §3.5）。

验证：`cargo test --offline` 150 项（98 单元 + 52 集成）。

## 3ax. P3 · 上游代理覆盖（已完成）

证据：S-GAP-05。目标只登记字段，行为未验。

| 项 | 结果 | 位置 |
| --- | --- | --- |
| 配置 | 分组与频道均可写 `proxy{kind,addr,user,pass}` | `src/config/model.rs` |
| 优先级 | 频道级 > 分组级 > 直连 | `Config::proxy_for` |
| 传输 | `http`/`https`；`user` 非空时 basic auth | `AppState::http_for` |
| 范围 | 清单、切片、回看、Remux、录制、DASH 回看 | `stream.rs`、`timeshift.rs`、`remux.rs`、`schedule.rs` |
| 缓存 | 按 `kind|addr|user|pass` 复用客户端 | `AppState::http_for` |
| 测试 | 2 个单元（优先级、TOML）；1 个集成（伪转发代理计数） | 同上、`tests/http_contract.rs` |

不实现：`socks5`。遇到未知类型时直连并记告警。

验证：`cargo test --offline` 153 项（100 单元 + 53 集成）。

## 3ay. P3 · 目标差分复验（已完成）

命令：`scripts/compare-p0.sh`（目标容器 `iptv82` 与本实现）。

| 项 | 结果 |
| --- | --- |
| P0 用例 | 22 项，差异 0 |
| `/subs/status` | 空配置下两侧均为 `{"sources":[]}` |
| `/subs/{groups,preview}` | 目标行为未验；形状自定，已登记 |
| `/panel/api/link-info` | 形状一致；Token 启用时本实现返回当前值，目标样本为空串，登记为有意差异 |
| 未覆盖 | 面板 UI 交互、录制与回放（依赖 `ffmpeg` 与环境） |

详见 [`docs/design/reconciliation-report.md`](docs/design/reconciliation-report.md)。

## 3az. 一键安装脚本（已完成）

| 项 | 结果 | 位置 |
| --- | --- | --- |
| 来源 | 已解包目录、仓库 `dist/` 最新同架构包、`--archive` 本地路径或 URL | `install.sh` |
| 参数 | `--prefix`、`--archive`、`--no-service`、`--start`；`PREFIX`/`ARCHIVE` 环境变量 | 同上 |
| 平台 | 只接受 Linux amd64/arm64；其他架构直接报错 | 同上 |
| 校验 | 归档旁有同名 `.sha256` 时校验；缺失则跳过 | 同上 |
| 安装 | 二进制、`config.toml`、缓存目录、启停脚本；已有配置不覆盖 | 同上 |
| 服务 | systemd 或 OpenRC；按 `PREFIX` 替换单元内路径 | 同上 |
| 打包 | 发布包改为携带根目录 `install.sh`；删除 `packaging/install.sh` | `scripts/build-release.sh` |
| 文档 | README 新增「一键安装」：来源、选项、安装内容、示例 | `README.md` |

验证：
- `sh -n install.sh` 通过。
- Alpine 容器内从仓库自动选包、显式 `--archive`、URL（带/不带 `.sha256`）、解包目录内脚本均安装成功。
- 重复安装保留已有 `config.toml`。
- 双架构发布包重新生成，包内均含新 `install.sh` 与交互菜单 CLI。

## 3ba. CLI 子命令与交互菜单（已完成）

| 项 | 结果 | 位置 |
| --- | --- | --- |
| 入口 | 安装后软链 `/usr/local/bin/iptv-proxy` → `<prefix>/iptv-proxy.sh` | `install.sh` |
| 交互菜单 | 无参数或 `menu` 打开；更新、地址、启停、状态、日志、端口、账号密码、重装、卸载 | `packaging/iptv-proxy.sh` |
| 目标证据 | 公开 `IPTV Proxy/iptv-proxy.sh` v1.1.0 菜单结构；未复制脚本；v0.8.2 菜单未单独取证 | 2026-10-21 观测 |
| 菜单差异 | 文案与编号按本项目调整；安装委托 `install.sh`；非 TTY 不暂停 | 同上 |
| 启停 | `start`、`stop`、`restart`、`status`；systemd/OpenRC 优先，缺失时 PID 文件 | `packaging/iptv-proxy.sh` |
| 日志/地址 | `log [行数]`、`url`；`url` 输出面板地址与账号 | 同上 |
| 端口 | `port <N>` 改 `[server].bind` 并重启 | 同上 |
| 密码 | `pass [密码] [账号]` 写 `password_sha256` / `username` 并重启；无参数交互 | 同上 |
| 更新 | `update` 换二进制、保留配置；来源 `IPTV_PROXY_{BIN,ARCHIVE,URL}` 或本地 `dist/` | 同上 |
| 重装 | `reinstall` 备份 `config.toml.bak`，重置配置，可选换二进制 | 同上 |
| 安装/修复 | 发布包目录执行 `install [参数]`，调用同目录 `install.sh`；菜单安装成功后打开已安装菜单 | 同上 |
| 卸载 | `uninstall` 移除服务与软链；选 `N` 保留数据 | 同上 |
| 非交互 | `IPTV_PROXY_YES=1` 跳过重装/删除确认；`uninstall` 也接受管道 `y` | 同上 |
| 配置写入 | 按段定位，不误改同名字段；密码 SHA-256；无明文 | 同上 |
| 进程 | PID 文件记录真实二进制 PID，停止后不残留 | 同上 |
| 样例配置 | `disk_path` 改为 `./cache`，支持自定义安装目录 | `packaging/config.toml.example` |

验证：
- `sh -n`、`dash -n` 通过。
- `scripts/smoke-cli.sh`：端口、非法端口、短密码、密码/账号、`url`、更新、重装、卸载保留数据均通过；菜单入口、状态、错误恢复与卸载取消均覆盖。
- CI 增加 CLI 冒烟步骤。
- 交互菜单在非 TTY 下可脚本化：不暂停，按行读取选项。
- 发布包 amd64 容器内验证：菜单安装到 `/opt/iptv-proxy`、启动服务并打开已安装菜单。
- 双架构发布包重新生成，`verify-release.sh` 全绿。

## 3bb. MTranServer 翻译联调（已完成）

| 项 | 结果 | 位置 |
| --- | --- | --- |
| 协议 | 优先 MTranServer v5：`{from,to,texts}` → `{results}` | `src/translate.rs` |
| 兼容 | 旧 `{texts,source_lang,target_lang}` 与旧响应形状 | 同上 |
| 回退 | `/translate/batch` 不可用时逐条调 `/translate` | 同上 |
| 地址 | 支持根地址、`/translate`、`/translate/batch` | 同上 |
| Token | `Authorization: Bearer` | 同上 |
| EPG 配置 | `url` 或 `engine="mtran"` + `mtran_url`；`mtran_token`、`mtran_source`、`mtran_batch` | `src/epg.rs` |
| 字幕配置 | `url`、`token`、`batch`/`mtran_batch` | `src/http/routes/media.rs` |
| 自检 | `/subtitle/mtran/test`、`/epg/translate/{probe,mtran/test}` 返回真实译文 | 路由层 |
| 样例 | `config.toml.example` 增加注释配置；README 增加说明 | 文档 |

验证：
- 单元/集成：v5 `{results}`、旧形状、完整 `/translate` 地址、Bearer Token、单条回退均覆盖。
- 真实服务 `https://translate.ipacel.cc/translate`：
  - 自检 `Hello world` → `你好，世界`。
  - EPG 采集：`Hello world` → `你好，世界`，缓存写入 `epg/.translate/cache.json`；重跑 `translated: 1`。
  - EPG 探测 → `ok: true`。
  - 字幕 `live.srt` → 输出包含 `你好，世界`。
- `scripts/smoke-translate-live.sh` 可重复联调；CI 不跑外网。

注意：`translate.ipacel.cc` 是社区试用服务，可用性与额度不保证。生产建议自建 MTranServer。

## 3bc. P4 · DASH→HLS、proxy MPD 与 URL 池（已完成）

证据：F-016、S-MODE-06、S-DASH-01。

| 项 | 结果 | 位置 |
| --- | --- | --- |
| MPD 摘要 | `AdaptationSet`/`Representation` 轨道列表；面板接口不变 | `src/dash.rs` |
| 完整解析 | MPD/Period/AdaptationSet/Representation 逐层 `BaseURL`；`SegmentTemplate` 继承；`SegmentTimeline`（含 `r=-1` 与 `timeShiftBufferDepth` 展开）；`SegmentList`；`$Number%05d$` 等占位符 | 同上 |
| HLS 主清单 | `#EXT-X-MEDIA` 音轨/字幕 + `#EXT-X-STREAM-INF` 视频变体；轨道名 `video_{高}p`/`audio-{n}`/`text-{n}` | `src/dash_hls.rs` |
| HLS 媒体清单 | `#EXT-X-MAP`、`#EXT-X-MEDIA-SEQUENCE`、`#EXTINF`；静态 MPD 加 `#EXT-X-ENDLIST`；动态不加 | 同上 |
| 切片代理 | init/切片落 `cache/dash/`；伪装图片扩展名；命中缓存；多 URL 顺序回退 | 同上 |
| 时间窗 | `[groups].mpd_keep_seconds` 按末尾累计时长裁剪媒体清单 | 同上 |
| URL 池 | 多个 `<BaseURL>`；视频固定首选，音频/字幕在其余地址轮询；`url_pool_size` 控制启用 | 同上 |
| 状态接口 | `GET /panel/api/dash/status` 返回会话、`phase=ready/starting`、`segment_count`、`active_tracks[]`（name/kind/lang/label/width/height/bandwidth/codecs/frame_rate/audio_channels） | `src/http/routes/dash.rs` |
| proxy+MPD | 相对 `<BaseURL>` 改写到 `/{gid}/{cid}/mpd/`；init/切片/查询串经本服务代理；绝对地址直连 | `src/http/routes/stream.rs` |
| 看门狗 | 主清单失败累计；连续 5 次冷却 3600 s；改配置解除 | `src/http/routes/stream.rs` |
| 限制 | CENC 不解密；`proxy`+MPD 仅代理相对 BaseURL，绝对地址直连；目标 URL 池正常取用方式未观测 | §0.5 |

验证：
- 单元：MPD 摘要、Timeline/BaseURL、`r=-1` + `timeShiftBufferDepth`、多 BaseURL、SegmentList、主/媒体清单、时间窗、URL 池轮询、媒体序号、动态清单无 `ENDLIST`、BaseURL 查询串继承。
- 集成：`dash_mode_converts_mpd_to_hls_and_proxies_segments` 覆盖主清单、媒体清单、init、切片、状态与 `keep_seconds=4`；`dash_mode_resolver_url_is_used_for_media_playlist` 覆盖解析脚本地址沿用到媒体清单；`proxy_mode_rewrites_mpd_base_url_and_proxies_segments` 覆盖 proxy+MPD 相对 BaseURL、init/切片与查询串代理。
- `smoke-m3.sh` 新增 proxy+MPD：入口 200、`application/dash+xml`、相对 BaseURL 改写、切片代理。
- `cargo test --offline` 177 项（117 单元 + 60 集成）。

## 3bd. 定时录制 DASH 频道（已实现）

证据：F-009 §2.1–§2.3、S-REC-01/S-REC-02。

| 项 | 结果 | 位置 |
| --- | --- | --- |
| 触发 | 分组 `mode = "dash"` 的任务进入录制后走 DASH 分支 | `src/schedule.rs` |
| 直播判定 | 先抓 MPD 并解析；`type=static` → `failed`，`error=录制区间内无可用切片` | 同上 |
| 取材 | 每 2 秒重读 MPD，增量抓取视频/音频轨的 init 与切片，按段号+时间去重 | 同上 |
| 合流 | `ffmpeg -i <video.mp4> [-i <audio.mp4>] -map … -c copy <recordings>/<name>_<unix>.mkv` | 同上 |
| 上游参数 | 复用频道 HTTP 客户端、代理与自定义头 | 同上 |
| 停止与超时 | 采集时每 2 秒检查停止标志；并轨超时 120 秒；失败清理半成品 | 同上 |
| 产物 | 任务字段 `output_file`、`file_size`、`record_secs`；`note=mkv 录制完成` | 同上 |
| 差异 | 中间文件命名与目标不同；产物仍为 MKV | §0.5 |

验证：
- 集成：`dash_schedule_rejects_static_mpd` 覆盖静态 MPD 失败与目标错误串。
- 冒烟：`scripts/smoke-record-dash.sh` 生成动态 MPD，验证采集、并轨、下载与产物时长；`scripts/smoke-record.sh` 继续覆盖 HLS 成功路径。
- `cargo test --offline` 177 项（117 单元 + 60 集成）。

## 3be. EPG 匹配、手工映射与报告（已实现）

证据：F-008、F-010 §9；目标规则见 `docs/findings/dynamic/10`。

| 项 | 结果 | 位置 |
| --- | --- | --- |
| 输出标识 | `out_id = tvg_id || channel.id`；XMLTV `channel/@id` 与节目 `channel` 均改写为 `out_id` | `src/epg.rs` |
| 匹配优先级 | 手工映射 → `tvg_id`（大小写不敏感）→ 名称（删全部空白含 U+3000、转小写、完全相等） | 同上 |
| 冲突 | 同一源频道被多个本地频道命中 → 配置顺序最后一个获得节目；重复 `out_id` 标为 `duplicate` | 同上 |
| 手工映射 | `GET /panel/api/epg/mappings`；`POST /panel/api/epg/mapping`；空 `source_id`/`source_channel_id` 清除 | `src/http/routes/epg.rs` |
| 持久化 | `<cache>/epg/mappings.json`；启动载入；刷新后补 `source_display_name` | `src/epg.rs` |
| 报告 | `GET /panel/api/epg/match-report` 返回 `text/plain`：计数行 + `gid cid name out_id via src_id pc` | `src/http/routes/epg.rs` |
| 多源 | 同一 `out_id` 只输出一个频道；各源节目按本地频道归并 | `src/epg.rs` |

验证：
- 单元：`channel_targets_use_tvg_id_and_normalized_name`、`match_source_prefers_tvg_then_name_and_last_owner_wins`、`manual_mapping_channel_is_used`。
- 集成：`epg_manual_mapping_and_match_report` 覆盖设置/读取/清除映射、`manual` 报告与刷新回退。\n- 冒烟：`scripts/smoke-epg.sh` 覆盖映射设置、映射表、匹配报告与清除。
- `cargo test --offline` 177 项（117 单元 + 60 集成）。

## 4. 进度日志

| 日期 | 事项 | 结果 |
| --- | --- | --- |
| 2026-10-15 | Phase 0–2：规范骨架、取得产物、加壳与工具链、版本判定 | ✅ `de898fe`/`d2ade72`/`c938fbd`（F-001/F-002） |
| 2026-10-16 | 内嵌面板雕刻 + 接口表面；动态基线（启动/端点/认证/配置 schema/版本） | ✅ `5d94435`（F-003） |
| 2026-10-16 | 建组/建频道；**切片伪装后缀破解**（sha256） | ✅（F-004）`54a5b9f` |
| 2026-10-17 | 动态：六种模式+看门狗（F-005）·缓存窗口与布局（F-006）·播放 Token（F-007）·EPG（F-008）·台标 | ✅ `6311561`/`74ee89c`/`14fbe5e` |
| 2026-10-17 | 路线图 Batch1：**D13** 入口改写/访问范围（F-009）；**D13a+D29** 安全审计/指标面（F-010） | ✅ |
| 2026-10-17 | 基建：容器装 ffmpeg 6.1.1 + python3 3.12；起 4 个独立实例（19901–19904）；派发 4 个子代理（D12/D15/D21/D28、D24/D25/D27、D16/D18/D19/D19b/D20、D14/D17ab/D22） | ✅ 全部回收（F-013…F-016） |
| 2026-10-17 | 盘点二进制功能全集（55k 字符串+面板），产出优先级路线图 | ✅ §5 |
| 2026-10-18 | **D23** 台标库 + 配置段补录（F-011）；**D26** 多 VPS 分享 + 完整路由表（F-012） | ✅ |
| 2026-10-18 | 子代理：D24/D25/D27（F-013）·D16/D18/D19/D19b/D20（F-014）·D12/D15/D21/D28（F-015）·D14/D17ab/D22（F-016） | ✅ |
| 2026-10-18 | 收尾：**S7** 伪装前缀（F-017）；资产端点；一致性回填（spec/07-08 + 两个 README）；新增 `D-005`、`S-BUG-02` | ✅ |
| 2026-10-18 | Phase 4：R3 阶段性报告 | ✅ `findings/report-stage-01.md` |
| 2026-10-19 | 全库文档精简：删中间过程与重复叙述，保留全部编号/数字/状态；空行规范化 | ✅ 正文约 −25%~−30% |
| 2026-10-19 | 实现阶段启动 · **E1 技术选型**（Rust + axum；Vite+React 面板嵌入；drop-in；首期 P0） | ✅ [`design/01-tech-selection.md`](docs/design/01-tech-selection.md) |
| 2026-10-19 | **E1b UI 参考截图**：启动目标主实例造夹具，采集 30 张界面截图并映射功能与证据 | ✅ [`design/reference/`](docs/design/reference/) |
| 2026-10-19 | **E2 架构与模块设计**：分层、模块清单（P0/P1/P2）、请求流水线、共享状态、缓存/伪装设计、后台任务、ADR、里程碑 | ✅ [`design/02-architecture.md`](docs/design/02-architecture.md) |
| 2026-10-19 | 工具链：rustup `1.99.1`（stable）+ `aarch64/x86_64-unknown-linux-musl` 目标；两目标 `cargo check` 通过 | ✅ |
| 2026-10-19 | **E3 HTTP 契约**：公开/私有路由表、播放与订阅契约、`access` 矩阵、错误串对照、已知差异、M1 实施清单 | ✅ [`design/03-http-contract.md`](docs/design/03-http-contract.md)（`b080dec`） |
| 2026-10-19 | **E6 · M0 骨架**：配置模型/严格加载/原子保存、面板登录（UUID 会话）、状态与配置接口、内嵌 4 页前端（Vite+React）、`scripts/smoke-m0.sh` 15 项全绿 | ✅ 代码入仓（`cargo build` 无警告） |
| 2026-10-19 | **E4 配置与存储 + E5 前端设计**：热重载矩阵与存储布局、13 分区与弹窗体系、组件与交互约定 | ✅ [`design/04`](docs/design/04-config-and-storage.md)、[`design/05`](docs/design/05-frontend.md)（`aba1d1e`） |
| 2026-10-19 | **E6 · M1 数据面**：分组/频道 CRUD（局部更新、409/404 错误串）、导出、总/分组订阅与 `access` 过滤、播放 Token；面板分组管理页；`scripts/smoke-m1.sh` 全绿 | ✅ 见 §3c |
| 2026-10-19 | **E6 · M2 播放链路**：入口清单 → 媒体清单 → 伪装切片（`<P>_<S>.<ext>`）、切片索引与字节缓存、缓存窗口（首写 `public` / 命中 `private`）、302 占位、Token 校验；`scripts/smoke-m2.sh` 全绿 | ✅ 见 §3d |
| 2026-10-19 | **E6 · M3 直连模式**：`redirect`（含解析最佳画质、地址 TTL 缓存）、`rewrite`（清单绝对化）、`passthrough`（302 上游，校验 Token）、`remux` 预热 503、`dash` 30 s 就绪门；`scripts/smoke-m3.sh` 全绿 | ✅ 见 §3e |
| 2026-10-19 | **E6 · M4a 运维面**：配置热重载（只监视配置文件）、看门狗（连续 5 次失败 → 冷却 3600 s，重载即解除）、`cache/{stats,flush}`、`stats`、`logs`、日志文件 5 MB 截断；`scripts/smoke-m4.sh` 全绿 | ✅ 见 §3f |
| 2026-10-20 | **M4b 对拍**：同一夹具下 22 项用例差异 0；修正代理分支、缓存窗口、Token 判断顺序、`passthrough` 校验等 11 处契约差异 | ✅ [`reconciliation-report.md`](docs/design/reconciliation-report.md)、F-018 |
| 2026-10-20 | **目录结构**：改用「同名文件 + 目录」；新增 `src/lib.rs`，`main.rs` 只做启动 | ✅ `cargo test` 30 项 |
| 2026-10-20 | **M4c 打包**：Docker `rust:1-alpine` 双架构静态包、`/opt/iptv-proxy` 布局、systemd/OpenRC 单元；`verify-release.sh` 两架构通过（静态链接 + `/health`） | ✅ 见 §3g |
| 2026-10-20 | **P1 批次**：EPG、回看、台标、备份、素材、分享、安全、remux、录制、解析脚本 | ✅ §3i–§3ae |
| 2026-10-21 | **P2 批次**：订阅聚合、面板分区、DASH 回看、EPG/字幕翻译引擎 | ✅ §3ak–§3aq |
| 2026-10-21 | **质量门**：132 项测试、13 个冒烟、DASH 容器复验、双架构重出包 | ✅ §3ar、§3as |
| 2026-10-21 | **文档重排**：总览看板（进度图、质量门、里程碑、差异）、P3 任务计划；附录 A/B 合并到文末 | ✅ 本节 |
| 2026-10-21 | **P3-01 CI**：新增 `.github/workflows/ci.yml`（前端构建 → fmt/clippy/test → 原生 amd64/arm64 musl 包）；首跑全绿 | ✅ 运行 `39593785498` |
| 2026-10-21 | **P3-02 订阅源预检**：`/subs/status`、`/subs/groups`、`/subs/preview`；映射/排除/白黑名单与 `trace` | ✅ §3at |
| 2026-10-21 | **P3-03 链接审计**：`GET /panel/api/link-info` 返回目标形状；Token 关闭时为空串 | ✅ §3au |
| 2026-10-21 | **P3-04 安全面**：登录失败 429 锁、自助解封 30 天 3 次、封禁期可访问 `/unban` | ✅ §3av |
| 2026-10-21 | **P3-05 字幕格式**：ASS/SSA 检测与转 VTT；翻译缓存后缀 `.vtt` | ✅ §3aw |
| 2026-10-21 | **P3-07 上游代理**：分组级与频道级 `proxy`，频道优先；播放/回看/Remux/录制共用 | ✅ §3ax |
| 2026-10-21 | **P3-08 差分复验**：`compare-p0.sh` 22 项差异 0；新增端点差异登记 | ✅ §3ay |
| 2026-10-21 | **一键安装**：根目录 `install.sh` 支持发布包/`dist`/URL，注册服务并保留配置；README 补说明 | ✅ §3az |
| 2026-10-21 | **CLI 子命令**：`iptv-proxy` 支持启停、日志、地址、端口、密码、更新、重装、卸载；安装时创建软链 | ✅ §3ba |
| 2026-10-21 | **交互菜单**：无参数或 `menu` 打开；覆盖更新、地址、启停、状态、日志、端口、账号密码、重装、卸载与安装修复 | ✅ §3ba |
| 2026-10-21 | **P4 DASH→HLS**：MPD 完整解析、HLS 主/媒体清单、init/切片代理、`mpd_keep_seconds`、多 `<BaseURL>` 池、`/dash/status` | ✅ §3bc |
| 2026-10-21 | **翻译服务**：兼容 MTranServer v5、旧接口与单条回退；支持 Token；真实服务联调通过 | ✅ §3bb |
| 2026-10-22 | **DASH 定时录制**：动态 MPD 增量抓轨、`ffmpeg -c copy` 并轨 MKV；静态 MPD 失败语义对齐；新增冒烟；双架构重出包 | ✅ §3bd |
| 2026-10-22 | **EPG 匹配**：`out_id` 改写、tvg-id/名称优先级、最后命中、duplicate；手工映射持久化与 `match-report` | ✅ §3be |

## 附录 A · 目标能力全集（历史盘点，2026-10-17）

> 状态列是逆向阶段的结论，不代表当前实现缺口；当前待办见 §0.6。

> 来源：委派子代理对**二进制产物**的全量盘点（脱壳 strings 55k 条 + 雕刻面板 `panel.html` + 配置 schema）。
> 注：盘点证据为内嵌字符串/面板表单，路由前缀（如 `/panel/api`）以实测为准；「状态」为当前验证覆盖。

### P0 · 反代与直连核心

| 域 | 代表能力（二进制证据） | 结论（含路线图项） |
| --- | --- | --- |
| ① 反代 HLS（proxy/rewrite） | `/{gid}/{cid}.m3u8`、【代理】×44、`badge-proxy` | ✅ F-004/F-005；`rewrite`+MPD 损坏（`D-005`） |
| ② 切片伪装 URL | `<16hex>_<32hex>.<ext>`、`invalid segment hash` | ✅ `<S>`=sha256[:32]；`<P>` 依赖/校验已解、算法未解（F-017，低影响）；**S7 ✅** |
| ③ 切片缓存/预取 | `m3u8_max_entries`、`segment_cache_max_mb`、`prefetch.*`、`/cache/flush`、`/cache/stats` | ✅ 容量上限均未生效；flush 保留 `dash/`；memory 后端统计恒 0（F-015，D12） |
| ④ 302 直连 redirect | `redirect_mode`、`redirect_ttl`、`redirect_url_…` | ✅ direct + best（选最高 `BANDWIDTH`）（F-016，D17a） |
| ⑤ passthrough 直链 | `badge-passthrough`；直链不套 token | ✅ D7 |
| ⑥ Token 鉴权 | `token_enabled/ttl/rotate`、`/security/token{,/rotate}`、`/link-info`、登录 429 锁 | ✅ 轮换+1h 宽限+封禁面（F-010，D13a） |
| ⑦ 清单改写/聚合 | `m3u_header_lines`、`x-tvg-url`、`subscription-merge`、`access` | ✅ access/头行/导入/聚合/订阅合并（F-009/F-013，D13） |
| ⑧ MPD/ClearKey | `mpd_keep_seconds`、`license_key`、`#KODIPROP`、`key_manager.rs`(088) | ✅ MPD→HLS；KID 双路检测；不解密；`#KODIPROP` 仅 passthrough（F-016，D14） |
| ⑨ URL 池 | `url_pool_size`、`pool[0..n]`、`/dash/scan-url`、`final_url` | ✅ 契约已验；仅 DASH；`pool≥2` 本环境 503（环境受限）（F-015，D15） |
| ⑩ 解析器画质 | `/resolver/scripts`、`resolver_script/ttl/catchup` | ✅ `python3 <cache>/resolvers/<脚本> <arg>`；TTL/SWR；超时转 `error_url`（F-014，D19b） |

### P1 · 流处理与增值

| 域 | 代表能力 | 结论（含路线图项） |
| --- | --- | --- |
| Remux（TS/FLV/MMT） | `remux_container(auto/ts/flv/mmt)`、`libx264…aac` | ✅ 仅连续流；输出 HLS/5s/TS/伪装；**不用 ffmpeg**（503=预热态）（F-014，D16） |
| DASH 时移 | `/dash-timeshift/:track/…`、`dash_timeshift_hours/max_gb` | ✅ `playlist.txt?playseek=` VOD；窗=已录区间；缺参 400、不重叠 404（F-016） |
| 回看 catchup | `playseek=`、`catchup_days`、`catchup_tpl` | ✅ 只转发 `playseek`；4 模板；`catchup-days` 由 days 或 `timeshift_hours/24` 推导（F-016，D17ab） |
| 定时录制 | `/schedule{,/from-epg,/:idx/stop}`、`recordings/`、【录制】×49 | ✅ MKV + `ffmpeg -c copy`；`{idx}`=u64 下标（F-014，D18） |
| 字幕 | `WebVTT` 转换、`.vtt` 伪装 | ✅ SUBTITLES 组保留；`.vtt`/`text/vtt` 透传（F-014，D19） |
| 推流 livestream | `/livestream/:gid/:cid/start\|stop`、【推流】×35 | ✅ `?start/end`/`?playseek` 回看 + VOD 清单 + `timeshift/<hex>.<ext>`（F-014，D20） |
| 多 CDN 容错 | `consecutive_fails`、`fallback_pct`(088) | ❌ 未观测到故障切换：按 BANDWIDTH 选变体、失败不重试（F-015，`D-004`，D21） |
| EPG 匹配/译制 | `match-report`、`mappings`、`epg_map`、gemini/mtran | ✅ 匹配算法已解；译制未测（F-016，D22） |
| 台标库 | `/logo/{suggest,match-report,apply,animate}`、`to_webp` | ✅ 9 端点全验；转码在浏览器侧；动效 Rust 编码 + `.orig` 备份；改名/删除同步引用（F-011，D23） |

### P2 · 运维与管理面

| 域 | 代表能力 | 结论（含路线图项） |
| --- | --- | --- |
| 组导入导出 | `/groups/{export,import,/m3u}`、`channels/:cid/purge`(088) | ✅ JSON 文本导入；组 id 去非 ASCII；merge 仅新增；聚合输出上游原始地址（F-013，D24） |
| 备份/恢复 | `/panel/api/backup?include=`、`restore` | ✅ 三 scope；restore 另存 `.bak`（F-013，D25） |
| 多 VPS 分享 | `share.{role,master_url,trust_ips}`、`/api/share/pull` | ✅ GET-only；key+trust_ips 双门禁；token/封禁 20s 同步；follower 禁本地轮换（F-012，D26） |
| 边缘校验 | `edge_mode(off/proxy/cloudflare)`、`x-ipp-edge-secret` | ✅ 头名 `X-IPP-Edge-Secret`；带转发头且密钥不符→403；`edge_refresh` 拉 CF 段（F-013，D27） |
| 健康检查 | `health{interval,timeout,concurrency}` | ✅ `GET`+`Range: bytes=0-1`；失败 +1/成功归零；仅展示（F-015，D28） |
| IP 封禁/解封 | `/security/ip/{ban,stats,unban}`、`/unban` 每月 3 次 | ✅ 面已验；`/unban` 限次未验（F-010，D13a） |
| 指标/资源 | `/metrics`、`requests_total`、`mem_rss_mb` | ✅ `/status/metrics` 为假路由；指标仅面板私有面（F-010，D29） |
| 资产/ffmpeg 检测 | `/assets/{convert,list,ffmpeg-status}` | ✅ `ffmpeg-status`→`{"installed":true,"path":"ffmpeg"}`；`assets/list` 列配置项（F-014，D18） |
| EPG 公开 JSON API | `/epg/api/{channels,programmes,search}` | ⏭ 仅 088；已在 `spec/08` §16 登记，不测 |

### 执行批次（已完成）

- Batch 1：D13 → D13a → D15 → D21 → D24 → D25 → D27 → D29。
- Batch 2（装 ffmpeg）：D18 → D16 → D19 → D20 → D19b。
- Batch 3：D14 → D17ab → D22 → **S7**（后缀/缓存窗口完成；`<P>` 输入依赖+校验已定，算法未解 → F-017）。

### spec 缺口登记

盘点新增的 8 项能力（ClearKey/DRM、`epg/api/*`、`dash/scan-url`、`link-info`、每频道代理、旧数据迁移、`subscription-merge`、`l1/l2` 限流）
已录入 [`spec/08` §16](docs/spec/08-target-spec.md)，状态与证据见该表。

## 附录 B · 原 P1/P2 计划（已关闭）

> 本节是当时的计划。完成情况见 §0；未覆盖项已转入 §0.6。

已完成：P0 全部（M0–M4）、EPG、回看、台标库、备份/恢复、分组导入（JSON 与 M3U）、自定义头转发、发布打包。

| 项 | 规模 | 依赖 | 说明 |
| --- | --- | --- | --- |
| 面板其余分区 UI（缓存、日志、EPG、台标、安全、订阅） | 中 | 无 | 接口大多已就绪；前端按既有页签模式补页 |
| 订阅聚合 `[playlist]`（`sources[]`、`merge_ttl_secs`、`merge_position`、`global_exclude[]`） | 中 | 无 | 需新增配置建模、远端拉取与合并、`include`/`exclude` 前缀过滤（F-009、F-013 §2） |
| 安全面补全：`ip/unban-batch`、按 IP 会话上限、自动限流、`GET /unban` 自助解封 | 中 | 无 | 基础面已在 §3q 实现 |
| `remux` / `dash` 转封装 | 大 | `ffmpeg` | 外部程序；P0 起恒 503 预热 |
| 录制与推流 `/panel/api/schedule`、`/livestream/*`、`/timeshift/*` | 大 | `ffmpeg` | 需任务调度与文件管理 |
| EPG 脚本源与翻译接口 | 大 | `python3` | `kind="script"`、`/epg/translate/*` |
| `/logo/{apply,rename,restore,suggest,animate,match-report}` | 中 | 无 | 库内其余 6 个端点；`animate` 需图像处理 |
| 多 VPS 分享 `/api/share/pull` 与 `edge_mode` | 中 | 无 | 见 F-013 §10、F-016 |
| 面板 `/panel/api/{health,assets,schedule,dash,subtitle,resolver}` | 中 | 视项 | 按 F-010/F-011/F-014 补 |

> 每项开工前先补 `docs/design/` 小节与用例，再按 M0–M4 的节奏提交。
