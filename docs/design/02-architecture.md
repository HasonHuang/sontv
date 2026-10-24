# 02 · 架构与模块设计

> 设计版本 v1.0 ｜ 2026-10-19 ｜ 状态：**初稿**
> 上游：[`01-tech-selection.md`](01-tech-selection.md)（Rust + axum；drop-in 兼容；首期 P0）
> 依据：[`../spec/08-target-spec.md`](../spec/08-target-spec.md)、[`reference/README.md`](reference/README.md)

## 1. 架构目标

| 目标 | 说明 | 依据 |
| --- | --- | --- |
| 行为对齐 | 状态码、响应头、清单形状、缓存窗口与目标一致 | `spec/08` §2/§3/§10 |
| 单进程 | 一个进程、一个端口、一个配置文件 | S-ENV-05/06 |
| 单二进制 | 前端产物编译期嵌入 | F-003 |
| 可热重载 | `[server]`/`[auth]` 改文件即生效 | S-CFG-02 |
| 可降级 | 缺 `ffmpeg`/`python3` 时行为与目标一致 | S-DEP-01/02/04 |
| 可对拍 | 每层都有稳定契约，便于与目标逐条比对 | `spec/07` §2.3 |

## 2. 运行时模型

```text
┌──────────────────────────────────────────────────────────────────────┐
│ 进程（tokio 多线程 runtime）                                          │
│                                                                      │
│  main ──► 加载配置 ──► 建缓存目录 ──► 起后台任务 ──► 起 HTTP 监听     │
│                                                                      │
│  HTTP（axum Router）                                                  │
│   ├─ 公开面   /health /unban /logo /epg /unban/whoami /__assets       │
│   ├─ 播放面   /playlist.m3u /{gid}/playlist.m3u /{gid}/{cid}.{ext}    │
│   │            切片 …/<P>_<S>.<ext>、timeshift、dash-timeshift        │
│   └─ 面板面   /panel（静态） + /panel/api/*（Bearer 或 X-Token）      │
│                                                                      │
│  后台任务                                                             │
│   ├─ 配置监视（notify，分段生效）                                     │
│   ├─ 看门狗（dash/remux 就绪与冷却）                                  │
│   ├─ 健康探测（可选，仅展示）                                         │
│   ├─ 缓存清理与日志截断                                               │
│   └─ EPG 采集 / 分享轮询（P1/P2）                                     │
└──────────────────────────────────────────────────────────────────────┘
```

启动顺序（对齐目标行为）：

1. 解析 CLI（`iptv-proxy <config.toml>`；子命令见 `cli`）。
2. 读配置并校验（**严格 TOML**：重复键报错，F-008）。
3. 创建 `cache/` 子目录（仅启动时创建，F-006）。
4. 初始化缓存索引、Token 存储、封禁表。
5. 启动后台任务。
6. 监听 `bind`（默认 `[::]:19899`，S-ENV-05）。

## 3. 分层

| 层 | 职责 | 不做什么 |
| --- | --- | --- |
| `http` | 路由、提取器、中间件、错误映射 | 不实现业务规则 |
| `auth` | 面板会话、播放 Token、封禁、限流 | 不碰清单内容 |
| `proxy` | 模式分发、上游请求、清单改写 | 不管缓存落盘细节 |
| `cache` | 条目读写、TTL、容量、flush | 不改写正文 |
| `disguise` | 切片 ID 生成与校验、扩展名映射 | 不改写载荷字节（S-DIS-02） |
| `playlist` | 订阅生成、聚合、Token 附加 | 不发起上游请求（聚合除外） |
| `epg` / `logo` / `record` / `share` | 独立增值能力 | 不阻塞播放链路 |

## 4. 模块清单

### 4.1 P0（首期）

| 模块 | 职责 | 关键类型 | 规格 |
| --- | --- | --- | --- |
| `main` | 启动、日志、信号处理 | `fn main` | S-ENV-06 |
| `config::model` | 全部段与字段、默认值 | `Config`、`ServerCfg`、`PanelCfg`、`CacheCfg`、`AuthCfg`、`Group`、`Channel` | S-CFG-04、S-MODEL-01/02 |
| `config::load` | 严格解析、默认合并、原子写回、规范化 | `load_config`、`save_config` | S-CFG-01 |
| `config::watch` | 文件监视与分段生效 | `ConfigWatcher` | S-CFG-02、S-CFG-03 |
| `state` | 共享状态容器 | `AppState` | — |
| `http::router` | 路由表组装、中间件栈 | `build_router(AppState) -> Router` | S-EP-16 |
| `http::error` | 错误 → 响应（302/400/403/404/422/503） | `ApiError`、`PlayError` | S-DIS-15、S-EP-08 |
| `http::ext` | Bearer/X-Token 提取、`trust_ips`、边缘头 | `PanelAuth`、`ClientIp` | S-AUTH-01、S-EDGE-02 |
| `auth::session` | 面板登录、会话 TTL、失败锁定 | `SessionStore` | S-AUTH-01、S-AUTH-03 |
| `auth::play_token` | 48-hex Token、TTL 24h、轮换与 1h 宽限 | `PlayTokenStore` | S-AUTH-02/03/09 |
| `auth::ban` | 封禁、解封、批量、白名单 | `BanStore` | S-AUTH-10 |
| `proxy::mode` | 六模式分发 | `enum Mode` | S-MODE-01…06 |
| `proxy::hls` | 清单解析与改写、切片转发 | `rewrite_playlist` | S-MODE-03、S-DIS-10/14 |
| `proxy::upstream` | reqwest 客户端、UA/头、超时 | `UpstreamClient` | S-HEALTH-01 |
| `cache::disk` | 落盘、TTL、索引、统计 | `DiskCache` | S-CACHE-01/02 |
| `cache::flush` | 清理（保留 `dash/`） | `flush` | S-CACHE-06 |
| `disguise` | `<P>_<S>.<ext>` 生成与校验、类型映射 | `SegmentId`、`verify` | S-DIS-12/13/17 |
| `playlist::build` | 总/分组订阅、Token、头行、access 过滤 | `render_playlist` | S-URL-03/04/07、S-AUTH-05 |
| `watchdog` | dash/remux 就绪、失败计数、冷却 | `ChannelHealth` | F-005（30s / 5 次 / 3600s） |
| `metrics` | 面板指标（内存、CPU、在线、流量） | `Snapshot` | S-OPS-01 |

### 4.2 P1 / P2

| 模块 | 职责 | 阶段 | 规格 |
| --- | --- | --- | --- |
| `dash` | MPD 摘要与完整演示解析（BaseURL、SegmentTemplate/Timeline、SegmentList） | P4 | S-MODE-06、S-DASH-01 |
| `dash_hls` | MPD→HLS 主/媒体清单、init/切片代理、时间窗与 URL 池 | P4 | S-MODE-06、S-GAP-01 |
| `proxy::remux` | 连续流切 HLS（不用 ffmpeg） | P1 | F-014（D16） |
| `epg::*` | 采集、匹配、XMLTV 生成、脚本源 | P1 | S-EPG-01…07 |
| `record` | 定时录制（ffmpeg `-c copy` → MKV） | P1 | S-REC-01/02 |
| `timeshift` | 本地回看、`cache/livestream/` | P1 | S-TS-01、S-CAP-02 |
| `subtitle` | WebVTT 透传与 `.vtt` 伪装 | P1 | S-SUB-09、S-DIS-01 |
| `logo` | 台标库 9 端点、`.orig/` 备份 | P2 | S-LOGO-01…04 |
| `subs` | 订阅聚合、include/exclude、group_map | P2 | S-GAP-07 |
| `source_cache` | 订阅源正文与状态落盘、跨重启 TTL | P2 | S-GAP-07、F-020 |
| `share` | master/follower 同步 | P2 | S-EP-09、S-AUTH-06 |
| `edge` | `X-IPP-Edge-Secret` 校验、CF 网段 | P2 | S-EDGE-01…03 |
| `backup` | tar.gz 备份与恢复 | P2 | S-BAK-01/02 |
| `assets` | 素材转换、`ffmpeg-status` | P2 | S-DEP-01/04 |
| `cli` | 交互菜单、`install`/`update`/`port`/`pass`/`log`/`url`/`uninstall` | P2 | S-CLI-01/02、S-AUTH-08 |

## 5. 请求流水线

### 5.1 播放请求

```text
请求 /{gid}/{cid}.m3u8
  → [1] 路由匹配
  → [2] 边缘校验（edge_mode != off）
  → [3] 查分组/频道；判定 access（public/hidden/archived/open）
  → [4] 播放 Token 校验（token_enabled 且 access != open）
  → [5] 模式分发
        proxy      → 上游取清单 → 改写切片地址（伪装）→ 缓存 → 响应
        redirect   → 上游取地址 → 302；缓存地址 TTL 300s
        rewrite    → 上游取清单 → 绝对化 → 响应
        passthrough→ 302 上游原始 URL
        dash/remux → 看门狗状态机；未就绪 → 503 + Retry-After: 2
  → [6] 响应头：Content-Type（按伪装后缀）、Cache-Control（窗口）
```

失败映射：

| 情形 | 响应 | 依据 |
| --- | --- | --- |
| 未知 `gid`/`cid`、`archived` 组 | `302` → `server.not_found_url` | S-DIS-15、S-AUTH-05 |
| 切片 ID 错/短/缺 | `302` 占位 | S-DIS-17 |
| Token 缺失/失效 | `302` → `auth.token_failure_url` | S-AUTH-03 |
| 边缘密钥不符且带转发头 | `403` | S-EDGE-02 |
| remux/dash 预热 | `503` + `Retry-After: 2` | F-005 |
| 参数不合法（如 `playseek` 缺参） | `400` | S-TS-01 |
| 上游 MPD 非 MPD | `502` | S-GAP-03 |

### 5.2 面板请求

```text
/panel/api/*  → Bearer 或 X-Token → 处理 → JSON
/panel、/logo、/epg、/unban → 内嵌静态资源（免登录）
```

## 6. 共享状态

```rust
struct AppState {
    config:      ArcSwap<Config>,        // 读多写少；热重载原子替换
    cache:       Arc<DiskCache>,         // 索引 + 落盘
    sessions:    Arc<SessionStore>,      // 面板会话（内存，重启失效）
    play_tokens: Arc<PlayTokenStore>,    // 当前 + 旧 Token（宽限）
    bans:        Arc<BanStore>,          // IP 封禁与白名单
    limits:      Arc<LimitWindows>,      // l1/l2 窗口、会话计数
    channels:    Arc<ChannelRuntime>,    // 看门狗状态、健康结果
    metrics:     Arc<Metrics>,           // 计数与采样
    upstream:    Arc<UpstreamClient>,    // 连接池复用
}
```

规则：

- 播放路径**只读**配置快照；热重载用 `ArcSwap` 原子替换，避免锁竞争。
- 会话与 Token 存内存（与目标一致：重启失效，F-004）。
- 缓存索引在启动时重建，落盘文件为准。

## 7. 缓存设计

| 项 | 设计 | 依据 |
| --- | --- | --- |
| 布局 | `cache/{m3u8,seg_bytes,seg_url,sub_url,epg,logos,recordings,dash}` + `livestream/` | S-CACHE-02、F-006 |
| 键 | 上游绝对 URL 的 SHA-256 十六进制 | 与切片 `<S>` 同源，便于校验 |
| 窗口 | 首写 `public, max-age=300`；命中 `private, max-age=300`；入口 `public, max-age=5, s-maxage=5` | S-DIS-06、F-006 |
| flush | 清 `m3u8`/`seg_url`/`seg_bytes`/`sub_url`，**保留 `dash/`** | S-CACHE-06 |
| 容量 | 键面保留；P0 **不实现**上限（目标上限未生效，F-015） | S-CACHE-03 |
| `Range` | 切片请求忽略 `Range`，返回 200 全量 | S-DIS-07 |

## 8. 伪装设计

| 项 | 设计 | 依据 |
| --- | --- | --- |
| `<S>` | `sha256(上游切片绝对 URL)[:32]` | S-DIS-04（逐字节一致） |
| `<P>` | `sha256("{gid}:{cid}")[:16]`（自研；仅依赖 `(gid,cid)`） | F-017（算法未解，不追字节一致） |
| 校验 | `<P>` 与 `<S>` 必须匹配；扩展名不校验 | S-DIS-17 |
| 后缀 | 从 `[cache.disguise].formats` 按频道稳定轮换 | S-DIS-04 |
| 类型 | 扩展名 → `Content-Type`（图片类型；其余 `video/mp2t`） | S-DIS-13 |
| 载荷 | **不改字节**，仅改 URL 与响应头 | S-DIS-02 |

## 9. 后台任务与调度

| 任务 | 周期 | 依据 |
| --- | --- | --- |
| 配置监视 | 事件驱动（`notify`） | S-CFG-02 |
| 看门狗 | 就绪超时 30s；失败 5 次 → 冷却 3600s | F-005 |
| 健康探测 | 周期不由 `interval_secs` 单独决定；仅展示 | S-HEALTH-04/05 |
| 缓存清理 | 过期条目；日志截断 5 MB | S-CACHE-05 |
| EPG 采集 | `cron` 表达式（默认每分钟检查） | S-EPG-02 |
| 分享轮询 | 20s 周期拉取（P2） | F-012 |

## 10. 前端嵌入

```text
frontend/            # Vite + React + TS（独立目录）
  └─ pnpm build → dist/
        └─ rust-embed（编译期）→ /panel、/logo、/epg、/unban、/__assets/*
```

| 项 | 设计 | 依据 |
| --- | --- | --- |
| 页面 | 4 页：管理面板、节目单、台标库、自助解封 | F-003 |
| 路由 | 与目标一致（`/panel`、`/epg`、`/logo`、`/unban`） | S-EP-01/05/06/07 |
| API | 只调 `/panel/api/*`；端点与字段不重命名 | S-EP-12、F-012 §3 |
| 基准 | 逐屏对照 [`reference/img/`](reference/) | E1b |

## 11. 错误与日志

- `tracing` 结构化日志；级别由 `[server].log_level` 控制（S-CACHE-05）。
- 错误一律转成 JSON（面板面）或重定向/状态码（播放面），不泄漏内部路径。
- 日志文件按 5 MB 截断（与目标一致）。

## 12. 决策记录

| 编号 | 决策 | 备选 | 理由 |
| --- | --- | --- | --- |
| ADR-01 | axum 0.7 | actix-web、warp | 与目标同栈；tower 中间件可组合 |
| ADR-02 | 配置用 `arc-swap` | `RwLock`、消息传递 | 播放路径读多写少，避免锁开销 |
| ADR-03 | 缓存 disk-first | memory-first | 目标默认 `disk`；memory 仅保留键 |
| ADR-04 | `<P>` 自研算法 | 复刻目标算法 | 算法未解且不影响客户端（F-017） |
| ADR-05 | 前端编译期嵌入 | 运行时读目录 | 单文件发布（F-003） |
| ADR-06 | 配置规范化 + 原子写（临时文件 + rename） | 直接覆盖 | 目标全量原子写（S-CFG-01） |
| ADR-07 | 上游单例客户端 | 每请求新建 | 复用连接池，降首字节延迟 |
| ADR-08 | 严格 TOML 解析 | 宽松忽略 | 目标对重复键报错（F-008） |

## 13. 测试策略

| 层 | 方法 | 判据 |
| --- | --- | --- |
| 单元 | 伪装 ID、清单改写、配置默认值 | 与 `spec/08` 表逐条对上 |
| 契约 | 路由存在性检查 | S-EP-16 路由表 |
| 集成对拍 | 同一伪源，对比目标与本实现的响应 | 状态码、关键头、清单形状一致 |
| UI | 逐屏对照 `reference/img/` | 布局与交互一致 |
| 降级 | 移除 `ffmpeg`/`python3` 跑一遍 | 行为与目标一致（S-DEP-01/02） |

对拍环境复用 F-004…F-017 的容器与伪源；差异登记 `spec/07` §3.1。

## 14. 实施里程碑

| 里程碑 | 内容 | 出口判据 |
| --- | --- | --- |
| M0 骨架 | 配置加载/写回、面板登录、静态页嵌入 | 能登录、能保存配置 |
| M1 数据面 | 分组/频道 CRUD、总/分组订阅、access 过滤 | 订阅可导出且形状一致 |
| M2 播放链路 | proxy 模式、切片伪装、缓存窗口、302 占位 | ✅ 已实现（`smoke-m2.sh`；入口/媒体清单/切片/占位/缓存窗口/Token 全绿） |
| M3 直连模式 | redirect/rewrite/passthrough、redirect 缓存 | ✅ 已实现（`smoke-m3.sh`；含 remux/dash 就绪语义与 Token 矩阵） |
| M4 P0 收尾 | 看门狗、热重载、运维接口、对拍报告、打包 | ✅ 看门狗/热重载/运维接口（`smoke-m4.sh`）；对拍 22 项差异 0（`reconciliation-report.md`）；双架构静态包与 `/opt/iptv-proxy` 布局（`build-release.sh`、`verify-release.sh`） |

## 15. 未决问题

| 问题 | 处理 |
| --- | --- |
| 缓存索引用内存表还是小型嵌入式库 | 先用内存表 + 启动重建；条目量大再评估 |
| 面板 API 是否需要 OpenAPI 导出 | 在 `03-http-contract.md` 决定 |
| `[epg]` 热重载例外如何表达 | 在 `04-config-and-storage.md` 定 |
| 前端目录是否与 Cargo 同仓 | 建议同仓 `frontend/`，构建脚本调用 |
