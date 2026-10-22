# 04 · 配置与存储设计

> 设计版本 v1.0 ｜ 2026-10-19 ｜ 状态：**初稿**
> 上游：[`03-http-contract.md`](03-http-contract.md) ｜ 判据：[`../spec/08-target-spec.md`](../spec/08-target-spec.md) §11 ｜ 证据：F-006、F-008、F-015、F-013

## 1. 配置文件

| 项 | 设计 | 依据 |
| --- | --- | --- |
| 路径 | 来自启动参数（`iptv-proxy <config.toml>`），默认 `config.toml` | S-ENV-06 |
| 解析 | 严格 TOML：重复键、类型错都报错并退出 | S-EPG-01（F-008 实测） |
| 默认值 | 缺省键用内置默认补齐；默认值对齐目标 v0.8.2 | S-CFG-04 |
| 未建模键 | 存 `extra`（`toml::Table`），保存时原样写回 | 设计决策（避免丢第三方配置） |
| 写回 | 全量规范化 + 原子替换（`config.toml.tmp` → `rename`） | S-CFG-01（F-006） |
| 并发 | 进程内用一把写锁串行化"改内存 + 写盘" | 设计决策 |

### 1.1 段与职责

| 段 | 关键键 | P0 用途 |
| --- | --- | --- |
| `[server]` | `bind`、`not_found_url`、`error_url`、`home_url`、`m3u_header_lines`、`timezone_offset_hours`、`log_level`、`trust_proxy` | 监听、跳转、订阅头行、日志级别 |
| `[panel]` | `enabled`、`username`、`password_sha256`、`session_ttl` | 面板登录 |
| `[cache]` | `backend`、`disk_path`、`m3u8/segment_max_entries`、`disk_max_mb`、`disguise.*` | 缓存与伪装 |
| `[cache.prefetch]` | `enabled`、`concurrency`、`timeout`、`prefetch_limit`、`skip_tail`、`mpd_readahead` | 媒体清单刷新后按并发与超时预取切片；`skip_tail` 跳过尾部 |
| `[cache.segments]` | `global_max`、`per_channel_max`、`retain_count` | 内存切片索引上限 |
| `[cache.ttl]` | `sub_url_secs`、`segment_secs`、`refresh_ratio`、`fallback_mpd_secs` | 订阅地址与切片缓存时长 |
| `[cache.dash]` | `mpd_refresh_*`、`*_timeout_secs`、`fetch_concurrency`、`ondemand_idle_secs`、`retry_*`、`watchdog_*` 等 | MPD/切片超时、重试退避、会话空闲回收、`playlist_min_secs` 媒体窗口、`start_time_offset_secs`、看门狗阈值与冷却已接入；`fast_start_secs`、`publish_delay_secs` 等仍待目标语义证据 |
| `[cache.remux]` | `target_duration_secs`、`window_segments`、`publish_delay_segments`、`reconnect_backoff_ms` | Remux 切片时长、窗口与重连退避 |
| `[auth]` | `token_enabled`、`token_ttl_hours`、`token_failure_url`、`whitelist` | 播放 Token |
| `[health]`/`[stats]` | `enabled`、`interval_secs`、`online_window_secs` | 健康与指标 |
| `[assets]` | `map`（原始 URL → 本地 HLS 入口） | 占位/错误视频转换 |
| `[epg]` | `enabled`、`cron`、`public_viewer_enabled`、`keep_days`、`future_days`、`keep_yesterday`、`match_by_name`、`allow_scripts`、`python_bin`、`script_timeout_secs`、`record_pad_before_secs`、`record_pad_after_secs` | 节目单采集与浏览 |
| `[[epg.sources]]` | `id`、`name`、`kind`、`enabled`、`url`、`script`、`cron`、`timeout_secs`、`tz_shift_mins` | 节目单源 |
| `[epg.translate]` | `enabled`、`url`/`mtran_url`、`token`/`mtran_token`、`source`、`target(s)`、`mtran_batch` | EPG 机翻 |
| `[subtitle.translate]` | `enabled`、`url`、`token`、`source`、`target(s)`、`batch` | 字幕机翻 |
| `[[groups]]`/`[[groups.channels]]` | 见 `02-architecture.md` §4.1 | 数据面 |

## 2. 热重载矩阵

| 段 | 生效方式 | 依据 |
| --- | --- | --- |
| `[server]`（含 `m3u_header_lines`） | 立即 | S-CFG-02（F-006） |
| `[auth]` | 立即 | S-CFG-02 |
| `[[groups]]`/`[[groups.channels]]` | 立即 | 面板改动的实测行为 |
| `[cache]`（`disk_path`） | 需重启 | 目标同 |
| `[cache.segments]`/`[cache.ttl]` | 立即（索引上限与 TTL 随快照更新） | 本实现 |
| `[cache.prefetch]`/`[cache.dash]` | 下一次媒体清单刷新/抓取生效 | 本实现 |
| `[cache.remux]` | 新会话生效；已有会话保留旧参数 | 本实现 |
| `[epg]` | **需重启**（`enabled` 与源列表在启动时读取） | F-008 |
| `[epg.translate]`/`[subtitle.translate]` | 立即（下次翻译生效） | 设计决策 |
| `[panel]` | 立即（新会话按新口令） | 设计决策 |

实现：`notify` 监视配置文件。事件到达后重读；解析失败只记日志、保留旧快照。自身写盘时先登记内容摘要，忽略自己触发的下一次事件（避免写回循环）。

> 竞态：目标存在"文件直改 vs 面板保存"的竞态（面板以内存态回写）。本实现以**文件为唯一事实源**：任何保存都从当前文件重读合并，再写回。这会与目标行为不同（更安全），登记为设计差异。

## 3. 存储布局

```text
<disk_path>/                     （默认 ./cache）
├─ m3u8/          入口与媒体清单缓存
├─ seg_url/       上游切片地址缓存
├─ seg_bytes/     切片正文缓存
├─ sub_url/       订阅源缓存
├─ dash/          DASH 会话（flush 保留）
├─ livestream/    本地回看
├─ recordings/    录制产物 + schedule.json
├─ logos/         台标文件（含 .orig/ 原图备份）
├─ assets/        素材；convert/<hash>/ 存本地 HLS
├─ resolvers/     解析脚本缓存
├─ epg/scripts/   EPG Python 脚本
├─ epg/raw/       每个源的原始 XMLTV（regenerate 用）
├─ epg/.translate/ EPG 翻译缓存
└─ subtitles/.translate/ 字幕翻译缓存
```

| 项 | 设计 | 依据 |
| --- | --- | --- |
| 目录创建 | 仅启动时创建 | F-006 |
| 缓存键 | 清单：`{gid}_{cid}__{sha256(url)[..8]}`；切片：`sha256(上游绝对 URL)` | 内部实现，非兼容契约 |
| 切片伪装 | `<S>` = 同一 sha256 前 32 hex | S-DIS-04 |
| 切片索引（M2 已实现） | `seg_url/<sha256("{gid}/{cid}")[..16]>.json`：`<S>` → `{url, ext}`；内存 + 磁盘两级 | `src/proxy/segments.rs` |
| 切片正文（M2 已实现） | `seg_bytes/<gid>_<cid>_<S>.<ext>`；命中复用，不设上限 | 同上 |
| DASH 缓存（P4 已实现） | `dash/<gid>/<cid>/<track>/{index.txt,init.<ext>,<序号>_<hash>.<ext>}`；媒体清单按时重取，init/切片命中复用 | `src/dash_hls.rs` |
| DASH 时间窗 | `[groups].mpd_keep_seconds > 0` 时媒体清单只保留末尾累计时长；`0` 保留全部。`proxy` 模式 MPD 同样裁剪时间线并推进编号 | `src/dash_hls.rs`、`src/http/routes/stream.rs` |
| DASH URL 池 | 多个 `<BaseURL>` 时视频用首选，音频/字幕在其余地址轮询；`url_pool_size<=1` 不轮询 | `src/dash_hls.rs` |
| 清单复用（M2 已实现） | 进程内 TTL 5 秒（`TtlCache`） | `src/proxy/mod.rs` |
| flush | 清 `m3u8`/`seg_url`/`seg_bytes`/`sub_url`，**保留 `dash/`** | S-CACHE-06 |
| 容量上限 | P0 **不实现**（目标上限未生效） | F-015 |
| 索引 | 启动扫描目录重建内存索引 | 设计决策 |

## 4. 运行时数据

| 数据 | 存放 | 生命周期 | 依据 |
| --- | --- | --- | --- |
| 面板会话 | 内存 | 重启失效、TTL `session_ttl` | F-004 |
| 播放 Token | 内存 | 重启失效、TTL `token_ttl_hours`、旧值宽限 ≈1h | S-AUTH-09 |
| IP 封禁 | 内存 | 重启失效、默认 30 天 | S-AUTH-10 |
| 限流窗口 | 内存 | 滑动窗口 | S-GAP-08 |
| 录制任务 | `cache/recordings/schedule.json` | 持久 | F-014 |
| 看门狗状态 | 内存 | 重启重置 | F-005 |

`schedule.json` 形状（F-014）：

```json
{ "version": 1, "tasks": [ { "id": "<uuid>", "name": "…", "group_id": "…", "channel_id": "…",
  "start_time": "2026-10-20T19:00:00", "end_time": "…", "format": "mkv", "status": "pending" } ] }
```

## 5. 日志

| 项 | 设计 | 依据 |
| --- | --- | --- |
| 出口 | 标准输出 + `<安装目录>/iptv-proxy.log` | S-ENV-03 |
| 级别 | `[server].log_level`（`trace`/`debug`/`info`/`warn`/`error`） | S-CACHE-05 |
| 截断 | 文件超过 5 MB 截断 | S-CACHE-05 |
| 格式 | `tracing` 文本，含时间、级别、目标 | 设计决策 |

## 6. 发布布局

```text
/opt/iptv-proxy/
├─ iptv-proxy          静态 musl 二进制
├─ config.toml         首次启动生成默认配置
├─ iptv-proxy.log
├─ cache/
└─ iptv-proxy.sh       管理 CLI（无参数打开菜单）
```

| 项 | 实现 | 依据 |
| --- | --- | --- |
| 目标平台 | `aarch64-unknown-linux-musl`、`x86_64-unknown-linux-musl` | 需求 |
| 交叉编译 | Docker `rust:1-alpine`：镜像宿主三元组本身就是 `*-linux-musl`，直接 `cargo build --release` 得静态二进制 | `scripts/build-release.sh` |
| 依赖来源 | 宿主 `~/.cargo/registry` 与 `~/.cargo/git` 挂进容器，`--offline` 构建 | 容器无外网 |
| 目标目录 | `CARGO_TARGET_DIR=target/pack-<arch>`，不污染本机构建 | — |
| 产物校验 | `scripts/verify-release.sh`：`ldd` 无动态依赖、能启动、`/health` 返回 `ok` | `scripts/verify-release.sh` |
| 服务 | systemd 单元；Alpine 用 OpenRC | S-ENV-04 |

发布包内容（`dist/iptv-proxy-<version>-<arch>.tar.gz`）：

```text
iptv-proxy-<version>-<arch>/
├─ iptv-proxy            静态 musl 二进制
├─ config.toml.example   首次安装复制为 config.toml
├─ iptv-proxy.sh         管理 CLI（菜单、启停、端口、密码、更新、卸载）
├─ install.sh            安装到 /opt/iptv-proxy，注册服务
├─ iptv-proxy.service    systemd 单元
└─ iptv-proxy.openrc     Alpine OpenRC 服务
```

> 说明：本机访问 `github.com` 的 release 资产与 `static.rust-lang.org` 会被中间设备断开 TLS，
> 因此不用 `cross`，改用 Docker Hub 可用的 `rust:1-alpine` 完成双架构静态构建。
| 备份（P2） | `tar.gz` 含 `config.toml` + `cache/logos` + `schedule.json`；恢复时原配置另存 `config.toml.bak` | S-BAK-01/02、F-013 §9 |

## 7. 未决问题

| 问题 | 处理 |
| --- | --- |
| 是否复刻目标的"面板保存"规范化格式 | 用 `toml::to_string_pretty`；不追求与目标逐字节相同 |
| CLI `update` 的下载源 | 用 `IPTV_PROXY_{BIN,ARCHIVE,URL}` 覆盖；本仓库未发布 release 资产 |
| `disk_path` 变更是否允许热重载 | 当前要求重启（与目标一致） |

## 9. `[share]`

| 键 | 默认 | 说明 |
| --- | --- | --- |
| `enabled` | `false` | 启用分享端点与同步任务 |
| `role` | `master` | `master` 汇总从机；`follower` 采用主机 Token |
| `key` | 空 | 分享密钥；对端用 `?key=` 传入 |
| `trust_ips` | `[]` | 允许拉取的源地址；支持精确地址、CIDR 与 `*` |
| `master_url` | 空 | follower 拉取的基地址 |
| `follower_urls` | `[]` | master 汇总的基地址 |
| `poll_secs` | `300` | 同步周期（秒），最小 5 |

## 10. 频道级解析脚本

`[groups.channels]` 支持 `resolver_script`、`resolver_arg` 与 `resolver_ttl`。
脚本位于 `<cache>/resolvers/`，通过 `python3` 执行。
详见 `03-http-contract.md` §6.12。
