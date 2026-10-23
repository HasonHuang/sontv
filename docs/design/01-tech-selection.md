# 01 · 技术选型（IPTV Proxy 重实现）

> 设计版本 v1.0 ｜ 2026-10-19 ｜ 状态：**已确认（用户决策）**
> 依据：[`../spec/08-target-spec.md`](../spec/08-target-spec.md)、[`../spec/03-environment-and-tooling.md`](../spec/03-environment-and-tooling.md)、[`../../PROGRESS_bak.md`](../../PROGRESS_bak.md) 附录 A

## 1. 目标与约束

| 项 | 值 | 依据 |
| --- | --- | --- |
| 目标 | 重实现 IPTV Proxy v0.8.2 的功能 | `spec/08` |
| 兼容级别 | **完全兼容（drop-in）** | 用户决策 |
| 平台 | linux-arm64（主）、linux-amd64 | `spec/01` §3 |
| 首期范围 | P0（反代核心） | 用户决策 |
| 外部依赖 | `ffmpeg`、`python3` 保持**可选依赖**语义 | 用户决策 / `S-DEP-01`、`S-DEP-02` |

**完全兼容的判据**：

- 沿用 `config.toml` 的键名与默认值（`spec/08` §11）。
- 沿用全部路由与播放地址规则（`spec/08` §2、§3）。
- 沿用响应状态码、关键响应头与缓存窗口（`spec/08` §10，F-006）。
- 现有客户端与订阅地址不改即可播放。

**合规边界**：

- 目标前端（`panel.html` 等）只作**行为与布局参考**，不复制其代码与资源。
- 目标派生内容按不可信数据处理（`spec/05`）。

## 2. 决策总表

| 决策点 | 结论 | 备选 | 理由 |
| --- | --- | --- | --- |
| 后端语言 | **Rust**（edition 2021） | Go、TypeScript、Python | 与目标一致；可对齐 hyper 的 HTTP 语义；静态单二进制；交叉编译简单 |
| 后端框架 | **axum 0.7 + tokio + hyper 1** | actix-web、warp | 与目标同栈（F-004 观测：axum 0.7.9 / hyper 1.9.0）；中间件生态（tower-http）成熟 |
| 出站客户端 | **reqwest 0.12 + rustls** | hyper-util 手写 | 与目标一致（F-004）；需 rustls 以适配 `--no-default-features` 静态构建 |
| 前端 | **Vite + React + TypeScript** | Vue、Svelte、无构建原生 | 用户决策；组件化便于复刻面板的四个页面 |
| 前端嵌入 | 构建产物经 `rust-embed` 嵌入二进制 | 运行时读目录 | 单文件发布；与目标形态一致（F-003） |
| 面板 UI | **复刻目标面板**（布局与交互对齐，实现自研） | 全新 UI | 用户决策；便于逐屏对拍，降低回归风险 |
| 配置 | `toml` + `serde`，**严格模式** | 宽松解析 | 目标对重复键报错（F-008），严格模式对齐该行为 |
| 首发范围 | **P0 反代核心** | P0+P1、全量 | 用户决策；先打通播放链路再扩面 |

## 3. 后端依赖清单

### 3.1 P0（首期）

| crate | 版本 | 用途 | 关联规格 |
| --- | --- | --- | --- |
| `axum` | 0.7 | HTTP 服务与路由 | `S-EP-*` |
| `tokio` | 1.4x | 异步运行时 | `S-ENV-*` |
| `hyper` / `hyper-util` | 1.x | 底层 HTTP、客户端连接 | F-004 |
| `tower` / `tower-http` | 0.5 / 0.6 | 中间件（trace、limit、set-header） | `S-GAP-08` |
| `reqwest` | 0.12（`rustls-tls`） | 上游拉流与 302 解析 | `S-MODE-04` |
| `rustls` + `webpki-roots` | 0.23 | TLS | — |
| `serde` / `serde_json` / `toml` | 1 / 1 / 0.8 | 配置与 API 载荷 | `S-CFG-*` |
| `tracing` / `tracing-subscriber` | 0.1 / 0.3 | 日志（`log_level`） | `S-CACHE-05` |
| `clap` | 4 | 命令行（`config.toml` 参数、`pass`/`port`） | `S-CLI-01` |
| `notify` | 6 | 配置文件热重载 | `S-CFG-02` |
| `sha2` / `hex` | 0.10 / 0.4 | 密码校验、切片指纹 `<S>` | `S-DIS-01`、F-004 |
| `base64` / `uuid` / `rand` | 0.22 / 1 / 0.8 | Token、Cookie、随机前缀 `<P>` | `S-AUTH-*`、F-017 |
| `url` / `percent-encoding` | 2 | URL 改写与清单重写 | `S-MODE-05` |
| `mime_guess` | 2 | 伪装扩展名 → `Content-Type` | `S-DIS-02` |
| `chrono` / `chrono-tz` | 0.4 / 0.10 | `timezone_offset_hours`、时间戳 | `S-URL-05` |
| `ipnet` | 2 | `trust_ips`、Cloudflare 网段 | `S-EP-09`、F-013 |
| `bytes` / `futures` | 1 / 0.3 | 流式转发 | `S-MODE-03` |
| `rust-embed` | 8 | 嵌入面板静态资源 | F-003 |
| `thiserror` / `anyhow` | 1 / 1 | 错误处理 | — |

### 3.2 P1 / P2（延后）

| crate | 用途 | 阶段 | 关联规格 |
| --- | --- | --- | --- |
| `m3u8-rs` | HLS 清单解析与重写 | P1 | `S-MODE-03` |
| `quick-xml` | XMLTV 生成、MPD 解析 | P1 | `S-EP-05`、`S-GAP-01` |
| `flate2`（gzip） | `/epg.xml.gz`、备份包 | P1/P2 | `S-EP-05`、`S-BAK-01` |
| `tar` | 备份与恢复 | P2 | `S-BAK-01` |
| `aes-gcm` / `mp4` | ClearKey KID 检测（不解密） | P1 | `S-GAP-01` |
| `image` | 台标转码与尺寸探测 | P2 | F-011 |

## 4. 前端选型

| 项 | 结论 | 说明 |
| --- | --- | --- |
| 构建 | Vite 6 + TypeScript 5 | 产物 `dist/` 由 `rust-embed` 嵌入 |
| 框架 | React 19 | 组件化复刻四个页面 |
| 路由 | `react-router` | 与目标路径一致（`/panel`、`/panel/epg`、`/panel/logo`、`/unban`） |
| 数据层 | `@tanstack/react-query` | 统一 `/panel/api/*` 的请求、轮询与失效 |
| 样式 | 自写 CSS（CSS Modules + 设计变量） | 从目标面板提取**视觉参数**（配色、间距、字号），不复制其 CSS 文件 |
| 组件库 | 不加重型 UI 库；按需加 `@radix-ui/*` 无样式原语 | 控制体积，保持外观可控 |
| Node 工具链 | pnpm 8 / Node 24（本机已具备） | 仅构建期依赖，运行期无 Node |

**复刻范围**（F-003）：管理面板、节目单、台标库、自助解封。

**契约要求**：前端只调用 `/panel/api/*` 中已登记的端点（`spec/08` §2，F-012 §3）。端点名、请求体、响应字段**不得重命名**。

## 5. 兼容契约（必须逐条对齐）

| 契约 | 来源 | 加载阶段 |
| --- | --- | --- |
| `config.toml` 键名与默认值 | `spec/08` §11 | P0 |
| 路由表与鉴权面 | `spec/08` §2、F-012 §3 | P0 |
| 播放地址与扩展名规则 | `spec/08` §3 | P0 |
| 六种模式语义 | `spec/08` §4、F-005 | P0（proxy/redirect/rewrite/passthrough）、P1（remux/dash） |
| 切片伪装形状与扩展名映射 | `spec/08` §9、F-006 | P0 |
| 缓存窗口与 `Range` 行为 | `spec/08` §10、F-006 | P0 |
| Token 与 IP 封禁 | `spec/08` §7、F-010 | P0（Token）、P2（封禁面） |
| 状态码与响应体 | `spec/07` §2.3、F-004/F-009 | P0 |

### 5.1 伪装前缀 `<P>` 的处理

| 项 | 目标行为 | 本实现 |
| --- | --- | --- |
| `<S>`（32 hex） | `sha256(上游切片绝对 URL)[:32]`（F-004） | **逐字节一致** |
| `<P>`（16 hex） | 仅依赖 `(gid,cid)`；参与校验；算法未解（F-017） | 自研稳定算法：`sha256("{gid}:{cid}")[:16]` + 校验 |
| 扩展名 | 不参与校验；后缀映射 `Content-Type` | 一致 |
| 兼容性 | 客户端从清单取值 | **不影响 drop-in**（不要求与目标字节相同） |

> 网络客户端从清单读取切片地址。故 `<P>` 无需与目标一致。此差异不登记为 `D-*`。

## 6. 数据与文件布局

```
/opt/iptv-proxy/
├── iptv-proxy            # 单二进制
├── config.toml           # 与目标键面一致（spec/08 §11）
├── iptv-proxy.log        # 5 MB 轮转（S-CACHE-05）
└── cache/                # cache.disk_path（默认 ./cache）
    ├── epg/  logos/  m3u8/  recordings/
    └── seg_bytes/  seg_url/  sub_url/  dash/
```

- 子目录**仅在启动时创建**（F-006）。`cache/flush` 保留 `dash/`。
- 实现追加自有数据（如 `epg_map.json`、`scripts/`、`resolvers/`），与目标同名同路径。

## 7. 模块划分（初稿，详见 `02-architecture.md`）

| 模块 | 责任 | 关联规格 |
| --- | --- | --- |
| `cli` | `config.toml` 参数、`pass`/`port`/`update` 子命令 | `S-CLI-01` |
| `config` | 加载、默认值、规范化写回、热重载 | `S-CFG-01/02` |
| `http` | 路由、中间件、错误映射、302 占位 | `S-EP-*` |
| `proxy` | 六种模式的分发与转发 | `S-MODE-*` |
| `playlist` | 总/分组订阅生成、Token 附加、头行 | `S-URL-04/07` |
| `disguise` | `<P>_<S>.<ext>` 生成、校验、扩展名映射 | `S-DIS-*` |
| `cache` | 内存与磁盘后端、窗口头、flush、统计 | `S-CACHE-*` |
| `auth` | 面板登录、Token、IP 封禁 | `S-AUTH-*` |
| `panel` | 面板 API、静态资源嵌入 | `S-EP-12` |

## 8. 构建与发布

| 项 | 结论 |
| --- | --- |
| 目标三元组 | `aarch64-unknown-linux-musl`、`x86_64-unknown-linux-musl`（静态） |
| 构建工具 | `cargo` + Docker `rust:1-alpine`（双架构静态构建，见 `design/04` §6） |
| 前端构建 | `pnpm install && pnpm build` → `dist/` 嵌入 |
| 发布物 | `iptv-proxy` 二进制（arm64/amd64）+ 安装脚本 |
| 服务单元 | systemd（Alpine 用 OpenRC），`/opt/iptv-proxy` 布局 | 
| 版本上报 | `/panel/api/status.version` 输出**本实现版本**（首发 `0.1.0`）；字段名不变 |

## 9. 阶段范围

| 阶段 | 内容 | 规格 |
| --- | --- | --- |
| **P0** | 配置、面板登录、分组/频道模型、proxy/redirect/rewrite/passthrough、切片伪装、缓存窗口、总/分组订阅、302 占位 | `S-CFG`、`S-EP-01/02/03/06/10`、`S-MODE-01/03/04/05/06`、`S-DIS`、`S-CACHE`、`S-URL` |
| P1 | remux、DASH 转封装、时移/回看、录制、字幕、EPG、台标 | `S-MODE-02/08`、`S-EP-04/05`、`S-DEP-01/02` |
| P2 | 多 VPS 分享、边缘校验、备份恢复、指标、CLI/安装脚本、EPG 公开 JSON API | `S-EP-09/16`、`S-BAK`、`S-OPS`、`S-CLI`、`S-GAP-02/07` |

## 10. 前置条件

| 项 | 现状 | 动作 |
| --- | --- | --- |
| Rust 工具链 | 本机 `rustc 1.68.2`（Homebrew，无 rustup） | 安装 rustup，提升到稳定版（≥ 1.80；目标为 1.98.1） |
| Node / pnpm | Node 24.18、pnpm 8.10 | 无 |
| Docker | 29.4（OrbStack） | 无 |
| 交叉编译目标 | 未安装 | `rustup target add aarch64-unknown-linux-musl x86_64-unknown-linux-musl` |
| 对拍环境 | 样本 `samples/`（不入库）+ 伪源 | 复用 F-004…F-017 的容器与伪源 |
| 面板参考 | `samples/work/panel-v0.8.2/panel.html`（不入库） | 只作视觉与交互参考 |

## 11. P0 验收判据

| 判据 | 方法 | 规格 |
| --- | --- | --- |
| 配置加载与规范化写回 | 与目标 `config.toml` 逐键对比 | `S-CFG-01` |
| 热重载 | 改 `[server]`/`[auth]` 后立即生效 | `S-CFG-02` |
| 总/分组订阅形状 | 与目标清单逐行对比（条目、token、头行） | `S-EP-02`、`S-URL-04/07` |
| 入口地址与 302 规则 | 未知频道、`access` 四档、`archived` | `S-URL-01/02/03` |
| 反代与清单重写 | proxy 模式切片改写；rewrite 绝对化 | `S-MODE-03/05` |
| 切片伪装 | `<S>` 与目标逐字节一致；扩展名→`Content-Type` | `S-DIS-01/02` |
| 缓存窗口 | 首写 `public, max-age=300`；命中 `private, max-age=300` | `S-CACHE-01` |
| Token 鉴权 | `?token=<48hex>`；失效 302 `token_failure_url` | `S-AUTH-01` |
| 面板登录 | `POST /panel/api/login` 返回 UUID token；`Bearer`/`X-Token` 均可用 | `S-AUTH-02` |

判据通过条件：与目标实例**逐条对拍**一致；差异须登记 `D-*`。

## 12. 风险与缓解

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| `<P>` 算法未解 | 无（客户端从清单取值） | 用自研稳定算法；不追字节一致 |
| 目标行为只对齐 v0.8.2 | 不支持 v0.8.8 新行为 | 明确版本边界；差异登记 `D-001` |
| 目标前端不可复制 | UI 需重写 | 只复刻布局与交互；逐屏对拍 |
| 本机 Rust 过旧 | 无法编译新版依赖 | 装 rustup；CI/容器内固定工具链 |
| 上游源不可控 | 无法稳定对拍 | 用本地伪源（`samples/work/origin`） |
| 静态 musl 与 TLS 兼容 | 证书链问题 | 用 `rustls` + `webpki-roots`，不用 OpenSSL |

## 13. 未决问题

| 问题 | 处理 |
| --- | --- |
| 是否导出 OpenAPI | 在 `03-http-contract.md` 决定；可用 `utoipa` 生成 |
| 内存缓存后端（`backend="memory"`）是否进 P0 | 建议 P0 只实现 `disk`，保留键并返回相同默认值 |
| 面板 API 的鉴权粒度 | 在 `03-http-contract.md` 逐端点标注 |
| 安装脚本与 CLI 的子命令全集 | P2 处理；先对齐 `pass`/`port` |
