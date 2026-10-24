# PROGRESS · IPTV Proxy 逆向与重实现

> 本文件是当前进度与待办事项的权威入口。
> 完整历史记录见 [`PROGRESS_bak.md`](PROGRESS_bak.md)。
> 规范见 [`docs/spec/`](docs/spec/)。发现见 [`docs/findings/`](docs/findings/)。设计见 [`docs/design/`](docs/design/)。
> 目标版本：**IPTV Proxy v0.8.2**（上游提交 `d302316fb8`）。

## 1. 当前状态

### 1.1 目标与约束

- 逆向闭源 Linux 二进制 IPTV Proxy v0.8.2。
- 用 Rust 实现 drop-in 替代服务。
- 只支持 `linux-arm64` 与 `linux-amd64`。
- 发布静态 musl 产物。
- 目标派生的代码、前端资源、面板文案按不可信数据处理，不复制。
- 每次提交后 push 到 `git@github.com:HasonHuang/sontv.git` 的 `rust` 分支。
- GitHub Actions 只在推送 `v*` tag 时执行。

### 1.2 总体进度

| 阶段 | 范围 | 状态 |
| --- | --- | --- |
| P0 | 逆向、配置、认证、代理/直连、缓存、播放链路 | ✅ 完成 |
| P1 | EPG、回看、Remux、录制、台标、分享、安全面 | ✅ 完成 |
| P2 | 订阅聚合、面板分区、DASH 回看、EPG/字幕翻译 | ✅ 完成 |
| P3 | 目标剩余端点与兼容细节 | ✅ 完成 |
| P4 | DASH 完整链路与 DASH→HLS | ✅ 完成 |

当前实现已覆盖：

- 六种分组模式：`proxy`、`redirect`、`rewrite`、`passthrough`、`remux`、`dash`。
- HLS、MPD、TS 与真实上游兼容处理。
- 订阅、Token、访问范围、缓存伪装、切片缓存。
- EPG 采集、匹配、手动映射、额外频道、公开页与面板接口。
- EPG/字幕翻译。
- 定时录制、DASH 定时录制、回看、时移。
- 台标库、素材转换、分享、安全管控、备份恢复。
- 内嵌管理面板与交互式安装/管理 CLI。
- `install.sh` 保留已有 `config.toml`，支持自定义安装目录与服务注册。

### 1.3 最近完成

2026-10-24：

- **U-05 `[cache.dash]` 剩余参数完成**：
  - `publish_delay_secs>0` 按 `ceil(delay/切片时长)` 压后直播清单末端；`=0` 使用 `fetch_extra_segments`。
  - 看门狗接入 `watchdog_tick_secs`、`watchdog_stall_secs`、`watchdog_rebuild_stagger_ms`，并复用重建上限与冷却。
  - `fast_start_secs` 在三类目标场景中无可观测差异，保留配置字段，不改变输出。
  - 证据：F-018、`docs/findings/dynamic/18-dash-window-knobs.md`。

2026-10-23：

- **V-09 订阅聚合字段对齐**：
  - 实现 `ttl_secs`、`group_map`、`group_mode`、`merge_group`、`unlisted_groups`。
  - `ttl_secs=0` 按 3600 秒处理。
  - `group_map` 是选择集，不是“未列出保留全部”。
  - 未分组名为 `(无分组)`。
  - `group:` 同时匹配原分组与映射后分组。
  - 聚合按 URL 跨源去重。
  - `/subs/{status,groups,preview}` 与 `/panel/api/subs/*` 已对齐。
  - 证据：F-020、`docs/findings/dynamic/17-subscription-merge.md`。
- 提交：`6911fa8 feat(playlist): align subscription merge semantics with target`。
- 修复 `scripts/smoke-aggregate.sh` 的新分组语义。

2026-10-22 及以前：

- U-01 至 U-04 完成：`x-tvg-url`、EPG 端点、EPG 配置语义、`proxy`+MPD 裁剪。
- V-11 真实 HLS/M3U 验证完成：超时、半包、404 回退、302 错误页、重试、属性白名单、缓存头、扩展名保留。
- DASH 媒体窗口 `playlist_min_secs`、`start_time_offset_secs`、看门狗阈值与冷却已接入。
- 台标扩展名白名单与目标一致。
- 订阅聚合字段对拍完成。

### 1.4 质量门（2026-10-23）

| 项 | 结果 |
| --- | --- |
| `cargo fmt --check` | ✅ 通过 |
| `cargo test --offline --all-targets` | ✅ 216 项通过（147 单元 + 69 集成） |
| `cargo clippy --offline --all-targets` | ✅ 无 error；仍有历史 warning |
| 冒烟脚本 | ✅ 15 个：assets、panel、resolver、share、remux、schedule、timeshift、record、record-dash、epg、aggregate、backup、logo、security、cli |
| 发布校验 | ✅ 双架构静态产物通过 `verify-release.sh` |
| CI | ✅ 仅 `v*` tag 触发；fmt/clippy/test、CLI 冒烟与双架构校验已配置 |

### 1.5 当前工作区

- 基线提交：`6911fa8`，已推送 `rust` 分支。
- `PROGRESS_bak.md` 保存 2026-10-23 之前的完整 1169 行记录。
- `dist/` 与 `samples/` 不入库。
- 目标测试容器、源站与本实现的临时实例应按需清理。

## 2. 待处理任务

### 2.1 代码缺口

| ID | 优先级 | 任务 | 当前状态 | 下一步 |
| --- | --- | --- | --- | --- |
| U-06 | P1 | 台标动效与解码 | 6 种效果与 APNG 编码已完成；依赖外部 `ffmpeg`/`ffprobe` | 评估 APNG 脏矩形优化；验证 WebP 解码；减少外部依赖 |

### 2.2 待补目标证据

| ID | 项 | 当前结论 | 所需证据 |
| --- | --- | --- | --- |
| V-01 | DASH URL 池正常取用 | 已实现视频取 `pool[0]`，音频/字幕轮询其余地址；目标只拉 MPD 后零切片 | 目标正常多 `<BaseURL>` 样本；记录实际请求分布与回退顺序 |
| V-02 | `proxy`+MPD 绝对 `<BaseURL>` | 相对地址已对拍；绝对地址按目标证据保持直连 | 目标构造绝对 `<BaseURL>` 与多切片 MPD，确认是否代理 |
| V-03 | 公开 `/epg/api/*` 形状 | 面板私有端点已实现；公开端点形状仍为本实现自定 | 目标逐字段抓取；先解决 `spec/08` 的版本冲突 |
| V-05 | 每频道独立代理 | 实现顺序为频道 > 分组 > 直连 | 目标覆盖顺序、认证头、失败回退、`socks5` 行为 |
| V-06 | `/unban` 配额与 `whoami` | 已实现每 IP 30 天 3 次；目标未实测 | 目标连续解封；记录状态码、`remaining`、窗口重置 |
| V-07 | 切片 TS 结构 | 当前只做字节代理；无 PAT/PMT/SDT 证据 | 真实 TS 切片；核对 PID、SDT 与 `lumberjack` 水印 |
| V-08 | 切片前缀 `<P>` 算法 | 自研 `sha256("<gid>:<cid>")[:16]`；目标算法未解 | 更多 `(gid,cid)` 样本或反编译；低影响 |
| V-10 | 面板与录制/EPG 交互 | API 对拍 22/22 差异 0；12 个面板分区未逐屏对拍 | 目标操作录像；只比对行为，不复制前端代码、资源与文案 |

已完成：

- U-05 `[cache.dash]` 发布延迟与看门狗参数 ✅
- V-04 `/subs/{groups,preview}` 字段 ✅
- V-09 `[playlist]` / `[share]` 目标语义 ✅
- V-11 真实 HLS/M3U 验证 ✅

### 2.3 已登记差异与不实现项

| 项 | 结论 | 依据 |
| --- | --- | --- |
| 切片前缀 `<P>` 算法 | 未解；仅依赖 `(gid,cid)`，低影响 | F-017 |
| 缓存容量上限 | 不实现；目标配置存在但未生效 | F-015 |
| 旧数据迁移 `./var/dash` | 不实现；仅 v0.8.8 行为 | F-015 |
| DASH 多 CDN 故障切换 | 未复刻；只做顺序回退 | F-015 |
| CENC/ClearKey 解密 | 不实现；按原字节代理 | F-009 |
| `l1/l2` 限流与每 IP 会话上限 | 不实现；触发条件未复现 | F-010 |
| `random_id_enabled` | 仅回显；行为未观测 | F-018 |
| `cache.backend=memory` | 仅回显；实际仍落盘 | F-015 |
| 播放侧 `compat_mode` 与 `srt_ua_keywords` | 不实现；目标未测出行为 | D19 |
| `socks5` 代理 | 不支持；未知类型直连并告警 | F-018 |
| `match-report` 形状 | 保留目标 raw 抓包的 `text/plain` 表格；面板 JS 期望 JSON，冲突未解 | F-013 |
| 扩展名轮换算法 | 未完全破解；只保证取自 `[cache.disguise].formats` | F-018、F-019 |
| 源缓存跨重启 | 目标可保留；本实现仅内存保存 | F-020 |
| `out_groups`/`dropped_groups` 顺序 | 目标为哈希序；本实现稳定排序 | F-020 |

## 3. 建议推进顺序

1. **V-01、V-02**：补 DASH 多 `<BaseURL>` 与 `proxy`+MPD 绝对 `<BaseURL>` 证据。
2. **V-03**：补公开 EPG API 响应形状。
3. **U-06**：评估 APNG 脏矩形优化；确认 WebP 解码路径。
4. **V-05 至 V-10**：按影响排序补证据；低影响项保持登记。
5. 每项完成后更新本文件、对应 `docs/` 文档与测试。
6. 提交使用 Conventional Commits，并立即 push `rust` 分支。

## 4. 关键设计结论

- `[epg]` 的 `out_id = tvg_id || channel.id`。
- EPG 匹配顺序：手工映射 → `tvg_id` → 名称。
- 名称规范化：删除全部空白并转小写。
- 手工映射保存到 `<cache>/epg/mappings.json`；额外频道保存到 `<cache>/epg/extra.json`。
- EPG 源正文缓存到 `<cache>/epg/raw/<source_id>.xml`；`regenerate` 只读 raw 缓存。
- `public_viewer_enabled=false` 时公开 `/epg` 返回 404。
- `allow_scripts=false` 时拒绝脚本源。
- DASH 轨道命名：`video_{高}p`、`audio-{n}`、`text-{n}`。
- DASH 媒体清单路径：`/{gid}/{cid}/{track}/index.txt`。
- DASH 媒体窗口取 `[cache.dash].playlist_min_secs`；分组 `mpd_keep_seconds > 0` 优先。
- 直播 DASH 清单按 `start_time_offset_secs` 输出 `#EXT-X-START:TIME-OFFSET=-…`。
- CENC/ClearKey 不解密，按原字节代理。
- 台标扩展名白名单：`png/jpg/jpeg/webp/bmp/avif`。
- 订阅源 `group_map` 是选择集；`group_mode` 未知值按 `keep`。
- `map` 模式空映射值保留原名；`merge` 模式仅在 `merge_group` 非空时统一分组。
- 聚合按 URL 去重，范围覆盖同源与跨源。
- 源 `ttl_secs=0` 按 3600 秒；源级 TTL 优先于 `merge_ttl_secs`。

## 5. 主要文档入口

| 主题 | 路径 |
| --- | --- |
| 目标规范 | [`docs/spec/08-target-spec.md`](docs/spec/08-target-spec.md) |
| 验证矩阵 | [`docs/spec/07-baseline-and-verification.md`](docs/spec/07-baseline-and-verification.md) |
| HTTP 契约 | [`docs/design/03-http-contract.md`](docs/design/03-http-contract.md) |
| 配置与存储 | [`docs/design/04-config-and-storage.md`](docs/design/04-config-and-storage.md) |
| 架构 | [`docs/design/02-architecture.md`](docs/design/02-architecture.md) |
| 发现索引 | [`docs/findings/README.md`](docs/findings/README.md) |
| 订阅聚合证据 | [`docs/findings/dynamic/17-subscription-merge.md`](docs/findings/dynamic/17-subscription-merge.md) |
| 真实 HLS 验证 | [`docs/findings/dynamic/16-real-upstream-hls.md`](docs/findings/dynamic/16-real-upstream-hls.md) |
| 历史进度 | [`PROGRESS_bak.md`](PROGRESS_bak.md) |

## 6. 历史归档

原 `PROGRESS.md` 的完整内容已迁移到 [`PROGRESS_bak.md`](PROGRESS_bak.md)。

归档包含：

- P0 至 P4 的阶段记录。
- 每轮实现、对拍、测试与发布细节。
- 旧的任务编号与验证命令。
- 2026-10-23 之前的逐项完成记录。

本文件只保留当前状态、残余缺口、证据需求与推进顺序。
