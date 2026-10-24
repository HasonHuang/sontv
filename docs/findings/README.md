# 逆向发现（docs/findings）

本目录存放 **IPTV Proxy 逆向工程的分析产出**。所有内容遵循 [`../spec/`](../spec/) 的规范。

> 目标版本：**v0.8.2**（主，`d302316fb8`）｜ 对照 **v0.8.8**（= `main`）
> ｜ 目标指纹：见 [`artifact.md`](artifact.md)（**已登记**）

---

## 目录结构

```

docs/findings/
├── README.md      ← 本文件。发现索引
├── artifact.md    ← 目标指纹与取得记录（先填这个）
├── static/        ← 静态分析（段/符号/字符串/加固/反编译）
└── dynamic/       ← 动态分析（运行/行为/网络/缓存/伪装）

```

## 发现索引

| 编号 | 标题 | 域 | 结论级别 | 关联规格 ID | 日期 | 文件 |
| --- | --- | --- | --- | --- | --- | --- |
| F-001 | 样本加壳与工具链判定（UPX 4.24 / Rust 1.98.1，v0.8.2） | 静态 | 观测 | S-ENV-02, S-ENV-07, S-ENV-08, S-EP-09, S-CFG-04 | 2026-10-15 | [`static/01-sample-packing-and-toolchain.md`](static/01-sample-packing-and-toolchain.md) |
| F-002 | 版本判定：v0.8.2 = `d302316fb8`；main = v0.8.8 | 静态 | 观测 | S-ENV-01 | 2026-10-15 | [`static/01-sample-packing-and-toolchain.md`](static/01-sample-packing-and-toolchain.md) §5 |
| F-003 | 内嵌面板前端与 HTTP 接口表面 | 静态 | 观测 | S-MODE-01, S-DIS-01, S-DIS-11, S-EP-07/08, S-AUTH-01, S-CFG-04 | 2026-10-16 | [`static/02-embedded-panel-frontend.md`](static/02-embedded-panel-frontend.md) |
| F-004 | 运行时表面与切片伪装（后缀=sha256(上游切片URL)[:32]） | 动态 | 观测 | S-DIS-02/04/05, S-EP-05/07/10, S-CFG-01/02/04, S-ENV-05 | 2026-10-16 | [`dynamic/01-runtime-surface-and-disguise.md`](dynamic/01-runtime-surface-and-disguise.md) |
| F-005 | 六种工作模式与看门狗（redirect/passthrough=302，rewrite=绝对化，dash/remux 失败路径；**rewrite+MPD 损坏 → D-005**） | 动态 | 观测 | S-MODE-01/02/03/08/09/10 | 2026-10-17 | [`dynamic/02-modes-and-watchdog.md`](dynamic/02-modes-and-watchdog.md) |
| F-006 | 缓存布局与切片缓存窗口（首写 public→命中 private，max-age=300） | 动态 | 观测 | S-DIS-06/07, S-MODE-13/14 | 2026-10-17 | [`dynamic/03-cache-layout-and-window.md`](dynamic/03-cache-layout-and-window.md) |
| F-007 | 播放 Token 与安全护栏（入口需 `?token=`，切片不受保护） | 动态 | 观测 | S-AUTH-01/02/03, S-DIS-16, S-URL-06 | 2026-10-17 | [`dynamic/04-auth-token-and-guards.md`](dynamic/04-auth-token-and-guards.md) |
| F-008 | EPG 采集与下发（`/epg.xml[.gz]`、`/epg`；时间裁剪、`lang` 处理） | 动态 | 观测 | S-EP-05/06/12/13 | 2026-10-17 | [`dynamic/05-epg.md`](dynamic/05-epg.md) |
| F-009 | 入口清单改写与分组访问范围（access 四档、m3u_header_lines、配置原子保存） | 动态 | 观测 | S-AUTH-05, S-URL-03/04/07, S-CFG-01/02, S-EPG-01 | 2026-10-17 | [`dynamic/06-entry-playlist-and-access.md`](dynamic/06-entry-playlist-and-access.md) |
| F-010 | 安全审计面、Token 轮换、IP 封禁、指标与健康出口（含面板 API 全集枚举） | 动态 | 观测 | S-AUTH-03/04/09/10, S-OPS-01/02/03 | 2026-10-17 | [`dynamic/11-security-and-metrics.md`](dynamic/11-security-and-metrics.md) |
| F-011 | 台标库管理（9 端点、动效、改名/删除同步引用、配置段） | 动态 | 观测 | S-LOGO-01..04, S-CFG-04/05/06/07 | 2026-10-18 | [`dynamic/12-logo-library.md`](dynamic/12-logo-library.md) |
| F-012 | 多 VPS 数据分享（主从、token/封禁同步、门禁）+ 完整路由表静态补录 | 动态 | 观测 | S-EP-09/11/14/15/16, S-AUTH-06, S-CFG-06 | 2026-10-18 | [`dynamic/13-multi-vps-share.md`](dynamic/13-multi-vps-share.md) |
| F-013 | 分组导入/导出、订阅聚合、备份恢复、边缘模式 | 动态 | 观测 | S-SUB-01..08, S-BAK-01/02, S-EDGE-01..03 | 2026-10-18 | [`dynamic/08-import-backup-edge.md`](dynamic/08-import-backup-edge.md) |
| F-014 | 录制 / 直播回看 / Remux / 字幕 / 上游解析脚本（含 remux 与 ffmpeg 无关的更正） | 动态 | 观测 | S-MODE-12/16/18, S-REC-02, S-TS-01, S-SUB-09, S-RES-01, S-DEP-01 | 2026-10-18 | [`dynamic/09-recording-livestream-remux-subtitle.md`](dynamic/09-recording-livestream-remux-subtitle.md) |
| F-015 | URL 池 / 多 CDN / 健康探测 / 缓存容量与淘汰 | 动态 | 观测 | S-MODE-06/07/08, S-CACHE-01/03/06/07, S-HEALTH-01..06, S-URL-08 | 2026-10-18 | [`dynamic/07-upstream-health-cache.md`](dynamic/07-upstream-health-cache.md) |
| F-016 | DASH 全链路 / ClearKey / 时移 / 回看 / redirect best / EPG 匹配算法 | 动态 | 观测 | S-MODE-06/11/12/15/17, S-EP-04/14, S-EPG-01/03/05/06/07, S-CAP-01/02, S-TS-01, S-DASH-01, S-GAP-01 | 2026-10-18 | [`dynamic/10-dash-catchup-epg-match.md`](dynamic/10-dash-catchup-epg-match.md) |
| F-017 | 伪装前缀 `<P>`：输入依赖、校验行为与哈希族排除（算法未解，低影响） | 动态+静态 | 观测/推断 | S-DIS-04/12/17 | 2026-10-18 | [`dynamic/14-disguise-prefix-analysis.md`](dynamic/14-disguise-prefix-analysis.md) |
| F-018 | P0 对拍：代理入口按上游清单类型分支、缓存窗口取 `TARGETDURATION`、切片绝对地址、Token 与未知路径的判定顺序、`passthrough` 需 Token | 动态 | 对拍 | S-DIS-04/07/13/17, S-MODE-09/11, S-URL-03/04 | 2026-10-20 | [`dynamic/15-contract-reconciliation.md`](dynamic/15-contract-reconciliation.md) |
| F-019 | 真实 HLS 上游：25 秒总超时、流式部分正文、切片/清单失败语义、清单重试、主清单属性白名单、VOD/直播缓存头 | 动态 | 观测 | S-PROXY-01/03/06/07, S-DIS-04/13, S-MODE-03 | 2026-10-22 | [`dynamic/16-real-upstream-hls.md`](dynamic/16-real-upstream-hls.md) |
| F-020 | 订阅聚合字段：`group_mode`/`group_map`/`merge_group`/`unlisted_groups`、状态字段、源级 TTL、URL 去重 | 动态 | 观测 | S-GAP-07, S-SUB-02/03/08 | 2026-10-23 | [`dynamic/17-subscription-merge.md`](dynamic/17-subscription-merge.md) |
| F-021 | DASH `publish_delay_secs` 压后公式、`fast_start_secs` 无可观测差异、看门狗 tick/stall/stagger | 动态 | 观测 | S-DASH-02/03 | 2026-10-24 | [`dynamic/18-dash-window-knobs.md`](dynamic/18-dash-window-knobs.md) |
| F-022 | DASH 多 `<BaseURL>` 首选分布；proxy MPD 绝对/相对/缺失 BaseURL 的本地令牌反代 | 动态 | 观测 | S-MODE-03/07, S-DASH-04 | 2026-10-24 | [`dynamic/19-dash-multi-baseurl-proxy.md`](dynamic/19-dash-multi-baseurl-proxy.md) |
| F-023 | 面板 13 个一级分区、台标/回看/系统设置归属、订阅源规则与 EPG 映射交互对齐 | 动态+静态 | 观测/对拍 | S-EP-07, S-SUB-02/03, S-GAP-07 | 2026-10-24 | [`dynamic/21-panel-sections.md`](dynamic/21-panel-sections.md) |

> 新发现按 `F-0NN` 追加一行；`关联规格 ID` 指向 [`../spec/08-target-spec.md`](../spec/08-target-spec.md) 的 `S-*` 条目。

## 核验状态

核对矩阵见 [`../spec/07-baseline-and-verification.md`](../spec/07-baseline-and-verification.md)。发现的结论用于更新 `07` 中 `⏳` 的条目。

## 写作要求

- 顶部模板（版本/SHA-256/架构/日期/环境/结论级别）见规范 `04`。
- 每条结论标注【观测】/【推断】/【未知】，并附证据（命令或 `ev_...`）。
- 静态 ≠ 运行；否定须有界。目标派生内容按不可信数据处理（规范 `05`）。
