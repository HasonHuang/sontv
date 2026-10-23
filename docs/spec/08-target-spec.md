# 08 · 目标功能规格（iptv-proxy v0.8.2）
> 规范版本 v1.0 ｜ 2026-10-15 ｜ 目标：**IPTV Proxy v0.8.2**（YanG，闭源 Rust，linux-arm64/amd64）
> **本文是"待验证的功能规格"（Normative Spec），不是逆向发现**：内容由**公开 README** 与**基线文档**（`HasonHuang/sontv/docs/iptv-proxy/`，实测于 2026-10-02/03）整理；每条声明用 `规格 ID` 标注，供 `07-baseline-and-verification.md` 逐条核验。所有条目默认 **【基线观测】**（来自基线实测），状态列默认 `⏳`；核验结果（观测/推断/未知）由逆向产出回填，**不得**在此文件直接改写结论。
> **版本说明**：上游**无 tag/release**，`main` 随构建变动。主分析对象为 **v0.8.2**（提交 `d302316fb8`，与本文基线一致）；`main` 实为 **v0.8.8**，仅作差异对照。锚点与判定见 [`../findings/artifact.md`](../findings/artifact.md) §5 与 [`07`](07-baseline-and-verification.md) §3.1 `D-001`；状态列的 `✅` 只表示**静态存在性**一致，**行为**待动态核验。
> 来源缩写：`RM`=公开 README ｜ `D-README`/`D-iptv`/`D-fields`/`D-panel`/`D-disg`/`D-res`/`D-mode-*`=基线各文档。

## 1. 身份与运行形态

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-ENV-01 | 产品名 IPTV Proxy，作者 YanG，上游 `YanG-1989/rust` 子目录 `IPTV Proxy/`；**源码未公开** | RM || ✅ 见 F-002 / `artifact.md` §2.1 |
| S-ENV-02 | 发布形态：预编译二进制 `iptv-proxy-linux-{amd64,arm64,armv7}` + 安装脚本 | RM | ✅（见 `07` §2.3） |
| S-ENV-03 | 安装目录 `/opt/iptv-proxy/`：`iptv-proxy`、`config.toml`、`iptv-proxy.log`、`cache/`、`iptv-proxy.sh` | RM/D-panel | ⏳ |
| S-ENV-04 | 服务常驻：systemd（Alpine 用 OpenRC），开机自启 | RM | ⏳ |
| S-ENV-05 | 默认面板端口 `19899`，默认监听 `[::]`（双栈） | RM/D-panel | ✅（见 `07` §2.3） |
| S-ENV-06 | 启动命令形如 `./iptv-proxy config.toml`（配置以参数传入） | D-panel | ✅（见 `07` §2.3） |
| S-ENV-07 | 依赖语言/工具链为 **Rust** | RM | ✅【观测】`rustc version 1.98.1`（v0.8.2） |
| S-ENV-08 | 分发产物经 **UPX 4.24** 加壳；脱壳后静态、非 PIE、已 strip | 二进制 | ✅【观测】 |

## 2. HTTP 端点（surface）

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-EP-01 | 面板：`GET /panel` | D-panel || ✅【观测】200 HTML |
| S-EP-02 | 分组订阅：`GET /{gid}/playlist.m3u`；总订阅：`GET /playlist.m3u` | D-panel || ✅【观测】总订阅与分组订阅均 200（F-004/F-009） |
| S-EP-03 | 频道播放：`GET /{gid}/{cid}.{ext}`（v0.5.1 起路由**不带** `/index.m3u8` 路径段） | D-panel || ✅【观测】`/{gid}/{cid}.m3u8`；`/…/index.m3u8` 无独立路径段（F-004） |
| S-EP-04 | 回看：`GET /{gid}/{cid}?playseek=<start>-<end>` | D-panel | ✅【观测】只取 `playseek` 原样追加到上游；`start/end/utc/utcend` 丢弃；本地回看忽略该区间 |
| S-EP-05 | EPG：`/epg.xml`、`/epg.xml.gz`、`/epg`；**EPG 总开关关闭时三者全部 404** | D-panel | ✅【观测】开启后三者 200（`application/xml`/`application/gzip`/`text/html`），均无需登录 |
| S-EP-06 | 台标：`GET /logo/<文件名>` | D-panel | ✅【观测】200 `image/png`，`public, max-age=86400`；缺→404 |
| S-EP-07 | 自助解封：`GET /unban`，**无需登录**，每月限 3 次 | D-panel | ✅（见 `07` §2.3） |
| S-EP-08 | 状态：`/status`（返回 `entry_exts` 等）、`/status/metrics` | D-panel/D-res | ✅【观测】**均非路由**：`/status` 404、`/status/metrics` 302 假路由；实际在 `/panel/api/status` |
| S-EP-09 | 多 VPS 数据分享端点 `GET /api/share/pull?key=`（**仅 GET/HEAD**）；同步 token/封禁 | 二进制 | ✅【观测】key+`trust_ips` 双门禁（否则 403）；载荷 `{token,token_old,bans,unbans}`；master 轮询从机、follower 禁用本地轮换（见 13） |
| S-EP-10 | 健康检查：`GET /health`（安装脚本用于就绪探测） | 安装脚本 | ✅（见 `07` §2.3） |
| S-EP-11 | 运维 API：`/api/reload`、`/api/backup`、`/api/restore`、`/whoami` | 二进制 | 🔶【观测】实际为 `/panel/api/{reload,backup}`；`/whoami`、`/unban/whoami` 存在（路由表见 13 §3） |
| S-EP-12 | 面板接口前缀 `/panel/api/…`（备份/恢复、EPG 全套） | 二进制 | ✅【观测】`/panel/api/epg/{status,refresh,regenerate,mappings,programmes,scripts,match-report,mapping,extra,script,script-delete,cron-preview}` |
| S-EP-13 | EPG 翻译接口 `/epg/translate/{run,status,clear,probe,models,mtran/test}` | 二进制 | 🔶【观测】端点存在（前缀 `/panel/api`）；默认关，未触发 |
| S-EP-14 | DASH 接口 `/dash/…`、`/dash-timeshift/status` | 二进制 | ✅【观测】`/dash/status` 字段 `stream_key/source_url/phase/segment_count/last_error/checked_ms/uptime_secs/active_tracks`；`/dash-timeshift/status` 键 `{gid}:{cid}` |
| S-EP-15 | 解封辅助 `/unban/whoami` | 二进制 | 🔶【观测】路由存在（路由表见 13 §3）；响应未测 |
| S-EP-16 | 完整路由表（静态补录）：公开面含 `/epg/api/{channels,programmes/:channel,search}`、`/logo/api/list`、`/__assets/*path`、`/whoami`、`/{gid}/{cid}/{timeshift,dash-timeshift,segment}…`；私有面新增 `/groups/{import,import/force,:gid/export}`、`/cache/flush`、`/backup`、`/assets/ffmpeg-status`、`/dash/scan-url`、`/dash/tracks/:gid/:cid`、`/livestream/:gid/:cid/{start,stop}`、`/schedule/{from-epg,download/:filename,:idx/stop}` | 二进制 | ✅【观测】见 13 §3 |

## 3. 播放地址与扩展名规则

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-URL-01 | 播放地址扩展名由**后端**计算（`/status.entry_exts`），前端只查表：HLS→`.m3u8`，MPEG-DASH→`.mpd`，**开启切片伪装→`.txt`** | D-panel | 🔶【观测】入口=按 `format`（`.m3u8`/`.mpd`）；`.txt` 用于**媒体清单** `…/video/index.txt`，非入口 |
| S-URL-02 | **Passthrough 例外**：播放地址直接是上游原始 URL，不走 `/{gid}/{cid}` 路由 | D-panel || ✅【观测】passthrough 302 上游原始 URL（F-005） |
| S-URL-03 | 订阅地址含分组 ID：`http://host:19899/<gid>/playlist.m3u`；改分组 ID 使旧地址失效 | D-fields | ✅【观测】分组订阅 200；filename=`<gid>.m3u`（失效分支未测） |
| S-URL-04 | 开启 Token 后，**导出** M3U 自动附加 token；手工拼地址需自带 token | D-panel | ✅【观测】`?token=<48hex>` 附加于条目（`open` 组例外：不带） |
| S-URL-07 | `server.m3u_header_lines` 自定义行逐行附加于 `#EXTM3U` 后；总/分组订阅均输出；热重载+重启持久 | 二进制 | ✅ |
| S-URL-08 | `channel.url` **不支持**多 URL（换行/`|`/`,`/空格分隔均被当作整串路径 → 上游 404 → 入口 302） | 二进制 | ✅【观测】 |
| S-URL-05 | 回看时间戳 `yyyyMMddHHmmss`，**使用本地时间**（依赖 `timezone_offset_hours`） | D-panel | ✅【观测】按 `timezone_offset_hours` 解析 |
| S-URL-06 | 播放 token 参数名为 `token`（`?t=` 无效）；面板 Bearer token 不适用于播放 | 二进制 | ✅ |

## 4. 六种工作模式

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-MODE-01 | 面板提供**六种**模式：`proxy`/`remux`/`dash`/`redirect`/`rewrite`/`passthrough`（公开 README 只写四种） | D-README | ✅【观测】面板 `<select id="mg-mode">` 六项 |
| S-MODE-02 | 模式属于**分组**；同组频道共用 | D-README | ✅【观测】 |
| S-MODE-03 | Proxy：反代 + 切片缓存；内容格式 HLS / MPD；`MPD 裁剪保留秒数` 仅在格式=MPD 时出现 | D-mode-proxy | 🔶 反代+伪装切片已验；MPD 分支未验 |
| S-MODE-04 | Proxy + 内容格式=MPD 时，频道多出 **ClearKey 密钥** 字段（`kid_hex:key_hex`，每行一对）；行为为"反代穿透本服务" | D-mode-proxy || ✅【观测】频道 `license_key` 字段；`#KODIPROP` 仅 passthrough（F-016） |
| S-MODE-05 | Remux：连续流（FLV / 裸 TS / MMT-TLV）→ HLS；**不解码**，CPU 低；`源容器类型`= 自动识别 / TS / FLV / MMT-TLV；**无**内容格式字段 | D-mode-remux | 🔶【观测】容器自动识别（`【remux】自动识别容器: Ts`） |
| S-MODE-06 | DASH：`.mpd`→HLS 实时转封装，支持 CENC ClearKey 解密；自动 304 增量刷新 / 多 CDN 故障切换 / 防断流备份 / 片段级容错；**无**内容格式字段 | D-mode-dash | ✅【观测】MPD→HLS 成功；track 名 `video_{高}p`/`audio-{n}`；**需 `SegmentTimeline`**；支持 MPD 级 `<BaseURL>`；**未观测到多 CDN 故障切换**（→ D-004） |
| S-MODE-07 | DASH 频道级 `URL 池`（池大小 2–6，默认 3）：视频固定 `pool[0]`，音频/字幕在 `pool[1..]` 轮询 | D-mode-dash | 🔶【观测】**仅 DASH**；`pool≥2` 本环境只拉 MPD N 次后零切片、入口 503；取用方式受限未验证 |
| S-MODE-08 | Redirect：客户端 302 直连上游，本服务仅调度；`跳转方式`= 直接跳转（默认）/ 解析最佳画质；`上游缓存时长` 默认 300s（缓存的是解析出的上游地址，非切片） | D-mode-redirect | ✅【观测】302 + `redirect_url_…` TTL=300；补充：失败不缓存，成功清单缓存 ≈5s |
| S-MODE-09 | Rewrite：仅取一次清单并把相对地址改绝对；切片客户端直连；**请求头只作用于取清单**（源站要求切片带 Referer 时会失败） | D-mode-rewrite | ✅【观测】绝对化清单；Referer 分支未复现 |
| S-MODE-10 | Rewrite + MPD / Redirect + MPD → **直连 CDN**（不转封装）；Proxy + MPD → 反代穿透本服务 | D-mode-rewrite | ⚠️【观测】redirect+MPD → 302 上游 ✅；**rewrite+MPD → 200 但正文损坏**（逐行被绝对化，`D-005`） |
| S-MODE-11 | Passthrough：原样导出上游 URL，**不套 Token**；有密钥时按 MPD 形式下发（但**不解密**） | D-mode-passthrough | ✅【观测】302 `location=上游`；有 `license_key` 时追加 3 行 `#KODIPROP`（`manifest_type=mpd` 硬编码 / `license_type=clearkey` / 密钥原样不校验） |
| S-MODE-12 | 仅 Proxy / Remux / DASH 支持**本地回看与定时录制**；Rewrite / Redirect / Passthrough 只有**源站回看**（`上游支持回看`+`回看天数`，追加 `playseek`） | D-iptv/D-mode-* | ✅【观测】Proxy 与 DASH 本地回看/录制已验（09/10）；Remux 录制未测 |
| S-MODE-13 | Redirect 上游地址缓存键 `redirect_url_{gid}_{cid短}__{hex}`，TTL=`redirect_ttl`（默认 300s） | 二进制 | ✅ |
| S-MODE-14 | 入口清单缓存键：Proxy=`{gid}_{cid}__{hex}`；Rewrite=`rw_{gid}_{cid}__{hex}` | 二进制 | ✅ |
| S-MODE-15 | DASH 主清单就绪超时 **30s**；未就绪 → `503 timeout waiting for master playlist` | 二进制 | ✅【观测】客户端同时收到该 503 |
| S-MODE-16 | Remux 预热期 → `503` + `Retry-After: 2`，体 `remux warming up` | 二进制 | ✅【观测】预热态**与 ffmpeg 无关**（缺 ffmpeg 仍 `503→200`） |
| S-MODE-17 | 看门狗：常驻频道卡死/停止 → 销毁重建（attempt 递增）；连续 5 次失败 → **冷却 3600s** | 二进制 | ✅【观测】改配置可**立即解除**冷却 |
| S-MODE-18 | Remux **只接受连续流**（HTTP-TS 裸流）；HLS 清单上游只重复拉清单、不取分片 → 永久 `503 remux warming up` | 二进制 | ✅【观测】输出 HLS（`target_duration`=5s、滑动窗、TS 分片、图片后缀伪装）；`remux_container` 描述**源**容器（auto/ts/flv/mmt），不改变输出 |
| S-RES-01 | 上游解析脚本：`python3 <cache>/resolvers/<脚本> <resolver_arg>`，stdout 即上游 URL；`resolver_ttl` SWR；失败保旧值、冷启动/超时转 `error_url`（约 30s） | 二进制 | 🔶【观测】见 09 §5 |

### 4.1 模式 × 字段显隐矩阵

| 字段 | Proxy | Remux | DASH | Redirect | Rewrite | Passthrough |
| --- | :-: | :-: | :-: | :-: | :-: | :-: |
| 内容格式 | ✅(HLS/MPD) | ✖ | ✖ | ✅ | ✅ | ✖ |
| MPD 裁剪保留秒数 | 格式=MPD 时 | ✖ | ✖ | ✖ | 格式=MPD 时 | ✖ |
| 源容器类型 | ✖ | ✅ | ✖ | ✖ | ✖ | ✖ |
| 跳转方式 / 上游缓存时长 | ✖ | ✖ | ✖ | ✅ | ✖ | ✖ |
| ClearKey 密钥 | 格式=MPD 时 | ✖ | ✅ | ✖ | ✖ | ✅(按 MPD 下发) |
| 预载配置 | ✅ | ✖ | ✖ | ✖ | ✖ | ✖ |
| 网络配置(UA/代理/请求头) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

> 面板仅对 **5 个字段**做模式区分（上游缓存时长、跳转方式、源容器类型、内容格式、MPD 裁剪保留秒数）；其余（含网络配置）各模式通用。来源：D-fields。

## 5. 数据模型与字段

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-MODEL-01 | **分组级**字段：分组 ID、显示名称、工作模式、EPG 地址、访问范围、内容格式、网络配置（分组默认，频道可覆盖） | D-fields/D-panel || ✅【观测】`GET /panel/api/config` 全量字段（S-CFG-04） |
| S-MODEL-02 | **频道级**字段：频道 ID、名称、上游 URL、Logo URL、tvg-id、tvg-name、上游解析（静态/脚本 PY）、源站回看、网络配置 | D-fields/D-panel || ✅【观测】同上（含 `resolver_script`/`catchup`/`proxy`） |
| S-MODEL-03 | 工作模式/访问范围/网络配置**属于分组**；频道级网络配置覆盖分组，自定义请求头**合并**、同名以频道级为准 | D-iptv/D-fields || ✅【观测】模式属分组（S-MODE-02）；频道级 `proxy`/头覆盖字段存在 |
| S-MODEL-04 | `tvg-id` 缺省则用频道 ID；`tvg-name` 缺省则用频道名称（EPG 匹配） | D-fields || ✅【观测】EPG 匹配 `out_id=tvg_id||id`（F-016） |
| S-MODEL-05 | 上游 URL 支持**脚本解析（PY）**，每次播放前算地址，需 `python3`；有解析缓存时长（须小于签名有效期） | D-fields || ✅【观测】`python3 <cache>/resolvers/<脚本>`，TTL/SWR（F-014） |

## 6. 订阅来源：M3U 导入 vs 订阅聚合

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-SUB-01 | **M3U 导入**：只接受粘贴内容或上传文件（**不填地址**）；频道落地成分组/频道，获得**代理地址**，支持模式/缓存/录制/Token | D-iptv | ✅【观测】`POST /groups/import/m3u` body 为 JSON 文本 `{text,mode,group_mode}`（非 multipart）；落地获本地反代地址（见 08 §2） |
| S-SUB-02 | **订阅聚合**：接受订阅地址；输出**上游原始地址**，**不做反向代理**，不具分组归属，套不上模式/Token/缓存/录制 | D-iptv | ✅【观测】条目 URL 即上游原始地址；`merge_position`(top/bottom) 决定拼接位置 |
| S-SUB-03 | 输出可区分两类：手动/导入频道 `group-title=<显示名>`；聚合频道 `group-title=<分组ID>`（不同代码路径） | D-iptv | ❌【观测】**聚合或为上游分组名**（非本地分组 ID）；两类差异在 URL（本地反代 vs 上游原始）——见 07 差异登记 |
| S-SUB-04 | M3U 导入 `频道模式`= 自动识别（`.mpd`→DASH，其余→Rewrite）/ Redirect / Rewrite / Proxy / DASH / Passthrough | D-panel | ✅【观测】`auto`：`.mpd`→组 `dash/mpd`，其余→`rewrite/hls`；字段名 `group_mode` |
| S-SUB-05 | 导入`策略`：追加（默认，同名跳过）/ 合并 / 覆盖 | D-panel | ✅【观测】`mode`∈append/merge/replace；**merge 仅新增 id 不存在者、已存在不更新**；replace 整组替换 |
| S-SUB-06 | 导入保留 `tvg-id`/`group-title` 等元数据；自动抓取 UA/Referer/Origin（含 `#EXTVLCOPT`/`#KODIPROP`/`#EXTHTTP`）；ClearKey 自动填入 `license_key` | D-panel | ✅【观测】另解析行内 `http-user-agent`/`http-referer`；组 id 去非 ASCII（纯中文→`g-<6hex>`）；无 tvg-id 则频道 id 非幂等 |
| S-SUB-07 | 订阅聚合的 include/exclude 按**频道名称**匹配（不区分大小写、子串、多词 OR）；`group:`/`id:`/`/正则/` 仅用于**全局排除** | D-iptv | 🔶【观测】`group:`/`id:`/`/正则/` **per-source 过滤同样可用**（非仅全局排除） |
| S-SUB-08 | 分组处理：各自保留原名 / 逐个指定 / 全部合并到一个组；**只改输出分组标题，不改代理规则** | D-iptv | ✅【观测】`group_mode`∈keep/merge/map |
| S-SUB-09 | 字幕出口：保留 `TYPE=SUBTITLES`/`SUBTITLES="…"`（补 `FORCED=NO`）；`.vtt` 分片与 `text/vtt` 原样透传，不改写 | 二进制 | ✅【观测】`selected_tracks` 为 **DASH 轨道**选择（见 09 §4） |

## 7. 认证与安全

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-AUTH-01 | 面板密码在配置中存 **SHA-256**（明文不落盘）；改密码后旧会话立即失效 | RM/D-panel | ✅【观测】`password_sha256`；SHA-256("admin") |
| S-AUTH-02 | Token 验证：开启后所有播放地址需带 token；**随机频道 ID**（6 位）替换真实 ID，与 Token 同步轮转 | D-panel | 🔶【观测】入口需 `?token=`；**随机频道 ID 未观测**（入口路径/切片前缀不变） |
| S-AUTH-03 | Token 有效期（小时）可配；失效跳转可指向自定义提示页 | D-panel | ✅【观测】`token_ttl_hours=24`；失效→`token_failure_url`；面板登录失败锁 429（见 11 §4） |
| S-AUTH-04 | IP 管控：每 IP 并发会话数、会话超时、频率限制与封禁、白名单；**仅监控播放列表请求** | D-panel | 🔶 键面已全（l1/l2 双窗口等 20+）；8 次突发（含 token）未触发；仅 token 播放使 `active_sessions/online_viewers`+1；触发条件未定（见 11 §5） |
| S-AUTH-05 | 访问范围：公开 / 隐藏（不进总订阅，独立地址可播）/ 归档（整组停用，EPG+台标照常匹配）/ 免验证（跳过 Token） | D-fields | ✅【观测】四档全验；**open 亦被排除出总订阅**（仅直链/分组订阅可达），与基线措辞有出入 |
| S-AUTH-06 | 多 VPS 数据分享：共享 token 与封禁 IP；可开关"同时分享 token" | D-panel | ✅【观测】token+封禁同步已验证（见 13）；`share_token` 开关行为未测 |
| S-AUTH-07 | 自助解封 `/unban` 无需登录，每月限 3 次 | D-panel | 🔶【观测】`/unban` 无需登录 200；每月 3 次未验 |
| S-AUTH-08 | `iptv-proxy pass <密码> [账号]` 可直接改密码（无需进面板）；改端口/密码均改 `config.toml` 并重启 | RM/D-panel | ⏳ |
| S-AUTH-09 | Token 可**轮换**：`POST /panel/api/security/token/rotate` → 新 48-hex；**旧 token 宽限 ≈1h**；`GET /security/token` 给掩码与剩余秒 | D-panel/二进制 | ✅【观测】轮换即生效于订阅；旧 token 3598s 内仍 200 |
| S-AUTH-10 | IP 封禁面：`POST /security/ip/ban {"ips":[…]}`（默认 30 天）、`unban {"ip":…}`、`unban-batch`、`GET /security/ip/stats` | D-panel/二进制 | ✅【观测】字段不对称（ips/ip）；`unique_ips` 解封后不回落 |
| S-EDGE-01 | 边缘校验 `edge_mode`∈{`off`,`cloudflare`,`proxy`}；探测头 **`X-IPP-Edge-Secret`** | D-panel/二进制 | ✅【观测】见 08 §5 |
| S-EDGE-02 | `edge_reject=true`：**带转发头**（XFF/`X-Real-IP`/`CF-Connecting-IP`）而密钥不符 → **403**；无转发头直连放行；`false`=观察模式（放行+周期日志） | 二进制 | ✅【观测】见 08 §5 |
| S-EDGE-03 | `edge_refresh`：启动拉取 `cloudflare.com/ips-v4|v6` 作为可信边缘段 | 二进制 | ✅【观测】拉取 22 条 |

## 8. EPG / 台标 / 录制 / 回看

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-EPG-01 | 多源采集→匹配本机频道→生成统一 `/epg.xml.gz`；EPG 关闭时 `/epg*` 全 404 且导出 M3U 不带 `x-tvg-url` | D-panel | ✅【观测】匹配算法已解：`out_id = tvg_id \|\| channel.id`；先 `tvg_id`（大小写不敏感）再名称；规范化=删空白（含 U+3000）+小写 |
| S-EPG-02 | 采集节奏：每 4h / 每 6h / 每日 4 点 / 每日 3、5、12 点 / 每 30 分钟 | D-panel | ✅（见 `07` §2.3） |
| S-EPG-03 | 名称匹配开关（tvg-id 不匹配时按频道名再匹配一次）；脚本源需 python3 | D-panel | ✅【观测】不做全角折叠、保留连字符、**无子串匹配** |
| S-EPG-04 | EPG 标题可翻译（Google 免费 / Gemini(Key) / MTranServer），14 种目标语言 | D-panel | ⏳ |
| S-EPG-05 | 同源多命中 → **配置顺序最后一个**得节目；同 `out_id` → `via=duplicate` 丢弃 | 二进制 | ✅【观测】 |
| S-EPG-06 | 节目裁剪区间 = `[(今天−keep_days) 00:00, (今天+future_days) 23:59:59]`；`keep_days` **内部上限 7**（API 接受 10 但按 7 裁剪） | 二进制 | ✅【观测】`keep_yesterday` 开/关无差异，语义未定 |
| S-EPG-07 | `match-report` 的 `via`∈`tvg-id` / `name` / `manual` / `duplicate` / `none` | 二进制 | ✅【观测】 |
| S-DASH-01 | `SegmentTemplate@duration` 缺 `SegmentTimeline` → 按墙钟/duration 推算 `$Number$`（实测 896163185），切片段全 404 | 二进制 | ✅【观测】 |
| S-LOGO-01 | 台标本地托管，`tvg-logo` 与 EPG `icon` 指向本地；支持上传转码与动效生成 | D-panel | ✅【观测】9 端点全验（见 12）；转码在**浏览器侧**，服务端只存字节 |
| S-LOGO-02 | 台标库端点：`GET /logo/{list,match-report}`、`POST /logo/{upload,apply,rename,delete,restore,suggest,animate}` | D-panel | ✅【观测】`upload` 需 `name/ext/data`；`animate.quality` 为**字符串** |
| S-LOGO-03 | 动效纯 Rust 无损编码，覆盖原文件、原图备份到 `logos/.orig/`；6 种效果 | D-panel/二进制 | ✅【观测】`frames/total_ms/bytes_before/after/dirty_ratio`；`restore` 还原 |
| S-LOGO-04 | 改名同步刷新频道引用（`updated_channels`）；删除可选清引用（`clear_refs`） | 二进制 | ✅【观测】引用随改名更新（URL 编码），清引用后 `tvg-logo=""` |
| S-REC-01 | 定时录制：Proxy / DASH 频道；**必须 ffmpeg**，否则任务失败；录制合成 MKV（多音轨，中断可播）；存 `cache/recordings/` | D-panel || ✅【观测】MKV + `ffmpeg -c copy`；`recordings/`；缺 ffmpeg 失败（F-014） |
| S-REC-02 | 录制成功路径：产物 MKV，`ffmpeg -y -i <merged> -map … -c copy`；proxy 先并 `_merged_video.ts`，DASH 并 `_merged_video_<name>.mp4`+`_merged_audio-<n>.mp4` | 二进制 | ✅【观测】`{idx}` 为列表下标（u64）；静态 MPD 报「录制区间内无可用切片」 |
| S-TS-01 | 本地回看：`?start=<unix>&end=<unix>` 或 `?playseek=<本地 yyyyMMddHHMMSS-…>`；返回 `#EXT-X-PLAYLIST-TYPE:VOD` 清单；分片 `/{gid}/{cid}/timeshift/<hex>.<ext>` | 二进制 | ✅【观测】本地回看**忽略 `playseek` 区间**、返回全部已录片段；DASH 时移 `/dash-timeshift/{track}/playlist.txt?playseek=<b>-<e>`（缺参 400、区间不重叠 404；窗=已录制区间） |
| S-CAP-01 | 回看播放地址随「回看格式」变化：国产 `${start}-${end}` / 国外 `${start}-${end}` / Kodi `{utc}-{utcend}` / both | D-panel | ✅【观测】四模板字符串；`catchup-days` 由 `catchup_days` 或 `timeshift_hours/24` 推导（4h→1） |
| S-CAP-02 | 回看切片存 `cache/livestream/`；时间偏移可调（录进时间戳与播出时间差） | D-panel/D-mode-proxy | ✅【观测】`cache/livestream/{gid}_{cid}/index.json` + `*.ts` |

## 9. 切片伪装（disguise）

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-DIS-01 | 配置段 `[cache.disguise]`：`enabled`/`playlist`/`subtitle`/`master`/`formats`；**出厂默认 `enabled=true` + `playlist=true`** | D-panel/D-disg | ✅【观测】`GET /panel/api/config` 默认值一致 |
| S-DIS-02 | **不改切片载荷字节**：只改 URL 后缀与 Content-Type；四种后缀同一切片前 32 字节相同，首字节 `0x47` | D-disg | ✅【观测】切片与上游 `seg0.ts` MD5 相同（F-004） |
| S-DIS-03 | 切片为原始 MPEG-TS：每 188 字节 `0x47`，文件大小 188 倍数；无 FLV/图片/fMP4 特征 | D-disg | ✅【观测】1880 B、`188×10`、全部 `0x47`（伪源 10 包，PID `0x747`） |
| S-DIS-04 | 后缀**轮换稳定且按频道独立**：同一频道重复拉清单序列不变；同一切片哈希在不同频道后缀不同（由 频道ID+切片哈希 推导） | D-disg | ✅【观测】切片后缀=`sha256(上游切片绝对URL)[:32]`；16-hex 前缀 `<P>` **仅依赖 `(gid,cid)`**（与 `name`/`url`/`Host`/缓存/重启无关），算法未解（F-017） |
| S-DIS-05 | 后缀**不参与校验**：清单写 `.png`，请求 `.jpg` 得同字节（MD5 相同） | D-disg | ✅【观测】 |
| S-DIS-06 | `Cache-Control` 切换：切片刚写入短暂 `public, max-age=300`，之后 `private, max-age=300` | D-disg | ✅【观测】3 次复现 |
| S-DIS-07 | 切片不支持 `Range`（忽略，返回 200 全量）；无 `Accept-Ranges`/`ETag`/`Last-Modified` | D-disg | ✅【观测】 |
| S-DIS-08 | 每切片自包含 MPEG-TS（PAT/SDT/PMT + ~10s）；PID 布局非 ffmpeg 默认（PMT `0x100`、视频 `0x102`、音频 `0x101`） | D-disg | ⚠️ 未验：本地伪源仅 10 个 PID `0x747` 填充包，无 PAT/SDT/PMT；需真实上游 TS |
| S-DIS-09 | 每切片第 0 个包为 SDT，描述符载荷 ASCII `lumberjack`（两次）——来源水印，与伪装无关 | D-disg | ⚠️ 未验：伪源无 SDT、无 `lumberjack`（依赖真实上游切片） |
| S-DIS-10 | 媒体清单伪装成 `.txt`（`.../video/index.txt`），主清单入口仍为 `.m3u8`；媒体清单无 `#EXT-X-MAP` | D-disg | ✅【观测】 |
| S-DIS-11 | 文件名混淆由独立的 `cache.dash.obfuscate_filenames` 控制，**与伪装无关** | D-panel | ✅【观测】位于 `[cache.dash]`，与 `[cache.disguise]` 分区不同 |
| S-DIS-12 | 伪装切片 URL 形状 `/{gid}/{cid}/<16hex>_<32hex>.<ext>` | 二进制 | ✅ |
| S-DIS-13 | 切片 `Content-Type` 由后缀映射（图片→图片类型；其余→`video/mp2t`） | 二进制 | ✅ |
| S-DIS-14 | 频道入口清单以 `text/plain` 下发（`[cache.disguise].playlist=true`） | 二进制 | ✅ |
| S-DIS-15 | 未知/无效切片 ID → 302 到 `server.not_found_url` | 二进制 | ✅ |
| S-DIS-16 | 切片（段）**不受** `token_enabled` 保护；安全性依赖不可猜测的切片 ID | 二进制 | ✅ |
| S-DIS-17 | 切片 ID **参与校验**：`<P>` 或 `<S>` 错误/截短/缺失 → 302 占位；**扩展名不校验**（`.png` 与 `.jpg` 同字节） | D-disg | ✅【观测】F-017 |

## 10. 缓存

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-CACHE-01 | 后端 `disk`（持久，重启不丢）/ `memory` | D-panel | 🔶【观测】memory 后端 `disk_bytes:null`、各 `*_entries` 恒 0；播放仍 200；磁盘旧文件不清理 |
| S-CACHE-02 | 缓存目录 `./cache`，结构：`epg/` `logos/` `m3u8/` `recordings/` `seg_bytes/` `seg_url/` `sub_url/`（+ `livestream/`） | D-panel || ✅【观测】目录结构与启动时创建（F-006） |
| S-CACHE-03 | 条目上限：`m3u8_max_entries`(默认 500)、`segment_max_entries`(默认 2000)；`disk_max_mb`(0=不限) | D-panel | 🔶【观测】**四项上限均未观测到生效**：`m3u8_max_entries=2`→6–7 条；`segment_max_entries=2`→17 条；`disk_max_mb=2`→2.79MB；`segment_cache_max_mb=1`→2.7MB |
| S-CACHE-04 | **预载仅 Proxy 生效**：预载开关 / 拉取并发 / 拉取超时 / HLS 预载 / MPD 预载；无人观看仍回源 | D-mode-proxy | ⏳ |
| S-CACHE-05 | 日志 `iptv-proxy.log` 最大 **5MB** | D-panel | ⏳ |
| S-CACHE-06 | `POST /panel/api/cache/flush` 清 `m3u8/seg_url/seg_bytes/sub_url`，**保留 `dash/`** | 二进制 | ✅【观测】 |
| S-CACHE-07 | `whole_file_budget` 回读异常：配置 0 → 回读 536870912；非 0 → 回读 0 | 二进制 | 🔶【观测】 |

## 11. 配置项（config.toml）规格

```toml

[server]
bind = "[::]:19899"
timezone_offset_hours = 8
log_level = "info"
[panel]
enabled = true
username = "admin"
password_sha256 = "..."
session_ttl = 86400
[cache]
backend = "disk"
disk_path = "./cache"
m3u8_max_entries = 500
segment_max_entries = 2000
disk_max_mb = 0
[auth]
token_enabled = false
[cache.disguise]
enabled = true
playlist = true
subtitle = false
master = false
formats = ["jpg","png","gif","bmp","webp"]

```

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-CFG-01 | 面板改动最终写入 `/opt/iptv-proxy/config.toml`；文件只写最小必要项，其余走默认；保存后回写完整配置 | D-panel | 🔶【观测】面板改动经 `PUT /panel/api/config` 全量保存、规范化重写（数组多行）；容器内路径映射差异 |
| S-CFG-02 | 配置有**热重载监视**（可直接改文件生效，前提是热重载未被关闭） | D-panel | ✅【观测】`[server]`/`[auth]` 即时生效；`[epg]` 例外（见 05 §6）；文件直改与面板保存有竞态 |
| S-CFG-03 | 下列开关**启动时读取，改后须重启**：DASH 会话预启动、配置文件热重载监视、登录失败计数清理 | D-iptv | ⏳ |
| S-CFG-04 | 顶层键与语义如上表（须由二进制/实测确认键名与默认值） | D-panel | 🔶【观测】顶层段：`server,panel,cache,prefetch,auth,subtitle,assets,health,stats,epg,logo,share,playlist,groups`；已录 `panel/logo/assets/subtitle/share/playlist/stats`（见 12 §4） |
| S-CFG-05 | `[playlist]` 段：`merge_ttl_secs,merge_position,sources[],global_exclude[]`（订阅聚合输出面） | 二进制 | 🔶【观测】键面已录；行为未验（→ D24） |
| S-CFG-06 | `[share]` 段：`enabled,role,key,trust_ips[],master_url,follower_urls[],share_token,poll_secs=300`（多 VPS 分享） | 二进制 | 🔶【观测】键面已录；行为未验（→ D26） |
| S-CFG-07 | `[stats]` 段：`enabled,online_window_secs=90`（在线统计窗口） | 二进制 | ✅【观测】键面已录 |

## 12. 命令行与运维

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-CLI-01 | 安装后脚本软链到 `PATH`，命令 `iptv-proxy`：`update`/`reinstall`/`port <N>`/`pass <密码> [账号]`/`restart`/`log`/`url`/`uninstall` | RM/D-panel | ⏳ |
| S-CLI-02 | `uninstall` 询问是否删 `/opt/iptv-proxy`，选 N 仅停服务保留数据 | RM | ⏳ |
| S-OPS-01 | 指标出口仅面板私有面：`GET /panel/api/status`（uptime/会话/在线/流量/whole_file_*）/`cache/stats`/`health` | 二进制 | ✅【观测】字段已列（见 11 §6）；`traffic_in_bytes` 恒 0（S-BUG-01） |
| S-OPS-02 | **无**公开指标端点：`/status`、`/metrics`、`/version` → 404；`/status/metrics` → 假路由 302 占位 | 二进制 | ✅【观测】 |
| S-OPS-03 | 面板 API 全集 80+ 端点（分组/EPG/台标/录制/订阅/解析/字幕/资源/安全） | D-panel | ✅【观测】枚举见 11 §1 |
| S-HEALTH-01 | 健康探测请求 = `GET <channel url>` + `Range: bytes=0-1`（UA 为内置 Chrome 串） | 二进制 | ✅【观测】 |
| S-HEALTH-02 | 结果字段：成功 `{http_status,latency_ms,ok:true}`；HTTP 失败 `error:"HTTP 404"`+`http_status`；超时 `error:"超时"`+`http_status/latency_ms:null` | 二进制 | ✅【观测】 |
| S-HEALTH-03 | `consecutive_fails` 每轮失败 +1；一次成功立即归零 | 二进制 | ✅【观测】 |
| S-HEALTH-04 | 探测周期**不由 `interval_secs` 单独决定**（配置 5s → 实测轮次间隔 12s） | 二进制 | ✅【观测】 |
| S-HEALTH-05 | 健康状态**仅展示**：不影响导出清单、入口播放、缓存、订阅；`/panel/api/status` 无健康字段 | 二进制 | ✅【观测】 |
| S-HEALTH-06 | `concurrency` 控制并行度：6 → 6 条 3ms 内并发；1 → ≈0.4s 串行 | 二进制 | ✅【观测】 |
| S-BAK-01 | 备份：`GET /panel/api/backup?include=all|epg-scripts|resolver` → `application/gzip` + 时间戳文件名；**仅识别三个 scope，其它回退 `all`** | D-panel/二进制 | ✅【观测】`all` 包 = `config.toml`+`epg/epg_map.json`+`epg/scripts/*.py`+`resolvers/*.py`+`logos/*`+`manifest.json`（生成物 `epg.xml*` 不入包） |
| S-BAK-02 | 恢复：`POST /panel/api/restore`（raw `application/gzip`）→ `{ok,config_reloaded,report}`；原配置另存 `config.toml.bak`；非 gzip→400 | D-panel/二进制 | ✅【观测】已实测回滚 |

## 13. 可选依赖

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-DEP-01 | `ffmpeg`（可选）：视频素材 MP4→HLS、定时录制合成 MKV；**不装则录制失败** | D-panel | 🔶【观测】仅**定时录制**需要（`-c copy`）；**remux 不使用 ffmpeg**（缺 ffmpeg 仍成功） |
| S-DEP-02 | `python3`（可选）：EPG 脚本源、频道脚本解析 | D-panel || ✅【观测】解析脚本依赖 `python3`（F-014） |
| S-DEP-03 | `MTranServer`（可选）：字幕机翻（离线自部署） | D-panel | ⏳ |
| S-DEP-04 | 装依赖后需**重启** iptv-proxy 才被检测 | D-panel || ✅【观测】装 ffmpeg 后重启容器，`/assets/ffmpeg-status` 变为 `installed:true` |

## 14. 已知缺陷 / 局限（v0.8.2）

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-BUG-01 | 面板累计**回源流量统计失效**（恒为 0）；CPU 累计秒数与内存准确 | D-README/D-res || ✅【观测】`traffic_in_bytes` 恒 0（F-010 §6） |
| S-BUG-02 | 面板未提供**分组排序**功能 | D-README | ✅【观测】面板无排序/拖拽控件；路由表无 `move`/`order` 端点 |
| S-BUG-03 | 伪装切片不支持 `Range`，影响部分按需起播播放器 | D-disg || ✅【观测】切片忽略 `Range`（S-DIS-07） |
| S-BUG-04 | 本环境容器**无挂载卷**，重建即丢配置与缓存 | D-panel | ⏳ |

## 15. 使用方式
- **本规格是逆向的"判据"**：`docs/findings/` 中的每条结论应能对应到这里的 `规格 ID`（一致 → ✅；不一致 → ⚠️ 并给证据；未覆盖 → ⏳）；状态回填与差异登记规则见 `07-baseline-and-verification.md`。
- 本文件**不承载**逆向结论：发现与本文冲突时，以**带证据的发现**为准，并在 `07` 登记差异；版本变化（非 v0.8.2）时，另建 `08b-target-spec-vX.Y.Z.md`，不复用本文件状态列。

## 16. 盘点补录（v0.8.2 新增能力，已登记；部分深化验证）

| ID | 规格声明 | 来源 | 状态 |
| --- | --- | --- | --- |
| S-GAP-01 | **ClearKey/DRM 子系统**：频道 `license_key` + KODI `#KODIPROP` 注入；MPD `ContentProtection` 处理（`key_manager`） | 二进制/字段 | ✅【观测】KID 双路检测（MPD `ContentProtection` + fMP4 `tenc/sinf/schm/senc`）；转封装**不解密**；`#KODIPROP` 仅 passthrough |
| S-GAP-02 | **公开 JSON API**：`/epg/api/channels`、`/epg/api/programmes/:channel`、`/epg/api/search`（v0.8.2 路由表已含） | 二进制 | 🔶 路由存在；响应未验 |
| S-GAP-03 | `POST /panel/api/dash/scan-url`：手动 URL 池扫描 | 二进制/面板 | ✅【观测】体 `{url}`（缺→422）；MPD→200 `{tracks:[{bandwidth,codecs,height,width,name,kind,label,lang,selected}]}`；非 MPD→502 `MPD parse error` |
| S-GAP-04 | `GET /panel/api/link-info`：审计播放链接/token 剩余期；返回 `{aliases,random_id_enabled,token,token_enabled,use_alias}` | 二进制 | ✅【观测】形状已验（见 11 §2）；语义细节未验 |
| S-GAP-05 | **每频道独立上游代理**：频道/分组 `proxy{kind,addr,user,pass}`（覆盖全局代理） | 字段 | 🔶 字段存在（默认 `null`）；行为未验 |
| S-GAP-06 | **旧数据迁移**：清理遗留 `./var/dash`（**v0.8.8 变更**；v0.8.2 无此行为） | 088 diff | ⏳ 仅 088 |
| S-GAP-07 | **订阅合并** `subscription-merge`：`[playlist]{merge_ttl_secs,merge_position,sources[],global_exclude[]}` + `/subs/{preview,groups,status}` | 字段/端点 | ✅【观测】源字段 `ttl_secs/group_map/group_mode/merge_group/unlisted_groups`；`group_mode` 默认 `keep`，`unlisted_groups` 默认 `drop`；`group_map` 空且 `drop` → 零频道；`map` 空值保留原名；`merge` 用 `merge_group`；未分组为 `(无分组)`；`POST /subs/groups {url}`→`{groups:[{name,count}],total}`；`POST /subs/preview {source:{id,…}}`→`{kept,sample[{name,group,tvg_id}],trace{total,after_group,after_global_exclude,after_include,after_exclude,dropped_groups,out_groups}}`；`GET /subs/status`→`{sources:[{id,name,enabled,error,channels,last_ok,last_try,ttl}]}`，`ttl_secs=0`→`ttl=3600`；聚合按 URL 全局去重（F-020） |
| S-GAP-08 | **双层限流窗口** `l1/l2` 与会话计数（`active_sessions`/`online_viewers`）；缓存分层（`m3u8`/`sub_url`/`seg_url`/`seg_bytes`） | 字段/端点 | 🔶 键面已录；`l1/l2` 触发条件未定（见 11 §5） |

> 来源：二进制 strings/字段与面板 JS 的**全量盘点**（见 `PROGRESS.md` 附录 A）。这些能力在基线文档中缺失，先按"存在"登记；行为验证见对应 `D-*` 路线图项。
