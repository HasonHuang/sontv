# 03 · HTTP 契约

> 设计版本 v1.0 ｜ 2026-10-19 ｜ 状态：**初稿**
> 上游：[`02-architecture.md`](02-architecture.md) ｜ 判据：[`../spec/08-target-spec.md`](../spec/08-target-spec.md) §2–§4
> 证据：F-004、F-006、F-009、F-010、F-012 §3（路由表）、F-013（导入导出）、F-014

## 1. 范围

| 面 | 本文件覆盖 | 说明 |
| --- | --- | --- |
| 公开面 | 全部路由的**存在性** | P0 的播放与订阅给完整契约 |
| 播放面 | proxy 模式全链路、access、Token、缓存头 | 其余模式给状态码契约 |
| 面板面 | 已实现端点 + M1 端点 | 其余端点登记存在性，后续里程碑补契约 |
| 分享面 | `/api/share/pull` | 登记存在性（P2 实现） |

## 2. 通用约定

| 项 | 约定 | 依据 |
| --- | --- | --- |
| 会话鉴权 | `Authorization: Bearer <uuid>` 或 `X-Token: <uuid>` | S-AUTH-01、F-004 |
| 面板错误体 | `{"error":"<文本>"}` | F-013 §5 |
| 播放失败 | `302` 跳占位视频；**不用错误体** | S-DIS-15、F-004 |
| 面板兜底 | `/panel/api/*` 未匹配 → `404` JSON；其余未匹配 → `302` 占位 | S-EP-08 |
| 播放 Token 参数 | `?token=<48hex>`；`?t=` 无效 | S-URL-06 |
| 前端资源前缀 | `/__assets/*` | S-EP-16 |
| 系统时区 | 回看时间戳按 `server.timezone_offset_hours` 解析 | S-URL-05 |

状态码用量：

| 码 | 用途 |
| --- | --- |
| `200` | 成功；面板成功体多为 `{"ok":true}` |
| `302` | 占位跳转（未知频道、归档组、Token 失效、未匹配路径）、直连模式跳转 |
| `400` | 参数或请求体不合法 |
| `401` | 未登录、口令错误 |
| `403` | 边缘密钥不符、面板关闭 |
| `404` | 分组订阅（归档组）、未知面板接口、`/logo/<缺文件>` |
| `409` | 分组 id 已存在 |
| `422` | 请求体字段缺失（如 `subs/preview` 缺 `source.id`） |
| `500` | 内部错误 |
| `502` | 上游返回非预期内容 |
| `503` | `remux`/`dash` 预热；`Retry-After: 2`（remux） |

## 3. 公开面路由

| 方法 | 路径 | 鉴权 | 响应 | 阶段 | 依据 |
| --- | --- | --- | --- | --- | --- |
| GET | `/playlist.m3u` | 无 | `200` M3U | M1 | S-EP-02 |
| GET | `/subs/status` | 无 | `200` JSON 源列表 | ✅ P3 | S-GAP-07 |
| POST | `/subs/groups` | 无 | `200` JSON 分组；非 http(s) → `400`；抓取失败 → `502` | ✅ P3 | S-GAP-07 |
| POST | `/subs/preview` | 无 | `200` JSON 预检 | ✅ P3 | S-GAP-07 |
| GET | `/{gid}/playlist.m3u` | 无 | `200` M3U / `404`（归档组） | M1 | S-EP-02、F-009 |
| GET | `/{gid}/{cid}.{ext}` | 按 token 开关 | `200` 清单 / `302` 占位 | M2 | S-EP-03 |
| GET | `/{gid}/{cid}/index.m3u8` | 同上 | 同上（兼容旧路径） | M2 | F-012 §3 |
| GET | `/{gid}/{cid}/video/index.txt` | 无 | `200` `text/plain` 媒体清单（仅主清单上游）；否则 `302 error_url` | M2 | F-004、F-018 |
| GET | `/{gid}/{cid}/<P>_<S>.<ext>` | 无 | `200` 切片 | M2 | S-DIS-04 |
| GET | `/{gid}/{cid}/timeshift/{filename}` | 按 token 开关 | `200` 本地回看 | P1 | F-014 |
| GET | `/{gid}/{cid}/dash-timeshift/{track}/{...}` | 按 token 开关 | `200` DASH 回看 | P2 | S-EP-16 |
| GET | `/epg` | 无 | `200` `text/html`；EPG 关 → `404` | ✅ P1 | S-EP-05 |
| GET | `/epg.xml`、`/epg.xml.gz` | 无 | `200` `application/xml` / `application/gzip`；EPG 关 → `404` | ✅ P1 | S-EP-05 |
| GET | `/epg/api/{channels,programmes/:channel,search}` | 无 | `200` JSON | P1 | S-EP-16 |
| GET | `/logo` | 无 | `200` `text/html` | ✅ P1 | S-EP-06 |
| GET | `/logo/<file>` | 无 | `200` 图片 + `public, max-age=86400`；缺文件、越界名、非白名单扩展 → `404` | ✅ P1 | F-011 §6 |
| GET | `/logo/api/list` | 无 | `{"allowed_exts","count","logos":[{"file","name","ext","bytes","mtime","url"}]}` | ✅ P1 | F-011 |
| GET | `/unban` | 无 | `200` `text/html`；封禁期也可访问 | ✅ P3 | S-EP-07 |
| POST | `/unban` | 无 | `200` 解封结果；配额用完 → `429` | ✅ P3 | S-AUTH-07 |
| GET | `/unban/whoami`、`/whoami` | 无 | `200` `{"ip":"<地址>"}`；取 `X-Forwarded-For` 首项、`X-Real-IP`，都没有时取 TCP 对端 | ✅ P1 | S-EP-15 |
| GET | `/health` | 无 | `200` `ok`（`text/plain`） | ✅ M0 | S-EP-10 |
| GET | `/panel`、`/panel/`、`/panel/epg` | 无 | `200` `text/html` | ✅ M0 | S-EP-01 |
| GET | `/__assets/*path` | 无 | `200` 静态资源 / `404` | ✅ M0 | S-EP-16 |

## 4. 播放契约（proxy）

> 状态：**已实现，并经对拍确认**（`src/http/routes/stream.rs`、`src/proxy/`）。
> 验证：`scripts/smoke-m2.sh`、`scripts/compare-p0.sh`（22 项全一致，见 `reconciliation-report.md`）。

代理链路按**上游清单类型**分两支（F-018）：

```text
上游是媒体清单（单码率）
GET /{gid}/{cid}.m3u8
  → 200  text/plain; charset=utf-8
         cache-control: public, max-age=<TARGETDURATION>, s-maxage=<同上>
         正文 = 上游媒体清单；切片改写为绝对伪装地址
                http://<host>/{gid}/{cid}/<16hex>_<32hex>.<ext>
GET /{gid}/{cid}/video/index.txt
  → 302  跳 error_url（该路径只在主清单上游时存在）

上游是主清单（多码率）
GET /{gid}/{cid}.m3u8
  → 200  application/vnd.apple.mpegurl
         cache-control: no-cache, no-store
         正文 = #EXTM3U
                #EXT-X-VERSION:6
                #EXT-X-INDEPENDENT-SEGMENTS
                #EXT-X-STREAM-INF:<白名单属性>
                http://<host>/{gid}/{cid}/video/index.txt
GET /{gid}/{cid}/video/index.txt
  → 200  text/plain; charset=utf-8
         cache-control: public, max-age=<变体 TARGETDURATION>, s-maxage=<同上>
         正文 = 最佳变体的媒体清单；切片改写为绝对伪装地址

GET /{gid}/{cid}/index.m3u8
  → 与入口同义（兼容别名）
GET /{gid}/{cid}/<P>_<S>.<ext>
  → 200  切片；忽略 Range（返回全量）
         首次写入 public, max-age=300；命中缓存 private, max-age=300
```

| 项 | 契约 | 依据 |
| --- | --- | --- |
| 清单缓存窗口 | **VOD**：`max-age` = `#EXT-X-TARGETDURATION`（缺失按 6 秒）；**直播**：`no-cache, no-store` | F-018、F-019 |
| 切片地址 | 绝对地址（`http://<Host>/…`），不是根相对路径 | F-018 |
| 扩展名 | `format=hls` → `.m3u8`；`format=mpd` → `.mpd` | S-URL-01 |
| `<S>` | `sha256(上游切片绝对 URL)` 前 32 hex | S-DIS-04 |
| `<P>` | 16 hex，仅依赖 `(gid,cid)`；错值 → `302` | S-DIS-17、F-017 |
| 扩展名轮换 | 上游扩展名已在 `[cache.disguise].formats` 中时保留；否则按频道序列轮换 | S-DIS-04、F-019 |
| 上游超时 | 单个上游请求总超时 **25 秒**；正文中断后保持 `200` 并发送已收字节 | F-019 |
| 切片类型 | 按伪装后缀给 `Content-Type` | S-DIS-13 |
| 载荷 | 不改字节 | S-DIS-02 |
| Token | 入口按开关校验；**切片与子路径不校验** | F-007、F-018 |
| 未知分组/频道（入口形状） | 无/错 Token → `token_failure_url`；Token 合法 → `not_found_url` | F-018 |
| 未知路径（其它形状） | `302 not_found_url`，不看 Token | F-018 |
| 归档组 | 入口 `302 not_found_url`（先于 Token 判定）；订阅 `404` | F-009、F-018 |
| `Range` | 忽略 | S-DIS-07 |
| 回看 `playseek` | 只把 `playseek` 原样追加到上游 URL；`start`/`end`/`utc`/`utcend` 丢弃 | S-EP-04 |
| 未知单段路径 | `404`（如 `/status`）；两段及以上按播放形状处理 | S-EP-08 |
| remux 预热 | `503` + `Retry-After: 2`，体 `remux warming up` | S-MODE-16 |
| dash 未就绪 | `503 timeout waiting for master playlist` | S-MODE-15 |

### 4.1 实现细节（内部，不是兼容契约）

| 项 | 取值 | 说明 |
| --- | --- | --- |
| `<P>` 算法 | `sha256("<gid>:<cid>")` 前 16 位十六进制 | 目标算法未解（F-017）；校验失败仍按契约 302 |
| 扩展名轮换起点 | `sha256("<gid>:<cid>")` 首字节 + 切片序号 | 目标轮换算法未解；只保证取自配置列表 |
| 入口 Host | 取请求 `Host` 头；有 `X-Forwarded-Proto` 时用其 scheme | 用于拼切片绝对地址 |
| 上游清单复用 | 进程内 TTL 5 秒 | 防突发请求打穿上游 |
| 切片索引 | `cache/seg_url/<sha256(gid/cid)[..16]>.json`（`S` → 上游 URL + 扩展名） | 内部实现 |
| 切片正文 | `cache/seg_bytes/<gid>_<cid>_<S>.<ext>` | 同上 |
| 伪装关闭 | 媒体清单直接给上游绝对地址，不建索引 | 目标行为未采证 |
| `#EXT-X-KEY` 等 | `URI="…"` 绝对化，不改名 | S-DIS-02（不改载荷） |
| 主清单属性 | 只保留 `BANDWIDTH`、`CODECS`、`RESOLUTION`、`FRAME-RATE` | F-019 |
| HLS 清单重试 | 传输错误、正文解码错误、5xx、408、429 最多 4 次；退避 1/2/4 秒；其它 4xx 不重试 | F-019 |
| 切片失败 | 上游 404/连接错误 → `200` 空正文；正文中断 → `200` 部分正文；半包不写缓存 | F-019 |
| 清单失败 | 入口或变体清单失败 → `302 error_url` | F-019 |

### 4.2 直连模式（M3）

| 模式 | 入口行为 | 细节 | 依据 |
| --- | --- | --- | --- |
| `redirect` | `302` 到上游 URL | `redirect_mode` = `解析最佳画质` 时先取主清单挑**最高码率**变体；地址按 `redirect_ttl`（0 → 300 s）缓存；失败不缓存，改跳 `error_url` | S-MODE-08/13、F-018 |
| `passthrough` | `302` 到上游 URL | **校验 Token**（与其它模式一致），不缓存上游清单 | S-MODE-11、F-018 |
| `rewrite` | `200 text/plain; charset=utf-8` + `no-cache, must-revalidate` | 相对地址改绝对，切片客户端直连；清单缓存 ≈5 s | S-MODE-09/14、F-018 |
| `remux` | 预热期 `503` + `Retry-After: 2`，体 `remux warming up`；就绪后 `200` 清单 | 纯 Rust 把连续 TS 切成 HLS；HLS 上游永不就绪（S-MODE-18） | S-MODE-16/18 |
| `dash` | `200 application/vnd.apple.mpegurl` + `no-cache, no-store` | MPD→HLS；主清单指向 `/{gid}/{cid}/{track}/index.txt`；轨道名 `video_{高}p` / `audio-{n}`；init/切片伪装为图片；就绪门 30 s，超时 → `503 timeout waiting for master playlist` | S-MODE-06/15 |
| `proxy` | 代理链路（§4） | — | S-MODE-03 |

> `remux` 已实现（纯 Rust 切片，不调用 ffmpeg）。`dash` 已实现 MPD→HLS：解析 `BaseURL`、`SegmentTemplate`/`SegmentTimeline`、`SegmentList`，按需代理 init 与切片。`proxy` 模式下的 MPD 保留正文，只把相对 `<BaseURL>` 改写到 `/{gid}/{cid}/mpd/` 并代理 init/切片与查询串；绝对 `<BaseURL>` 仍直连上游（目标行为）。`mpd_keep_seconds > 0` 时裁剪时间线，并同步推进 `$Number$` 的 `startNumber`；DASH 媒体清单窗口按 `[cache.dash].playlist_min_secs`（默认 30 秒）取最后 N 秒，分组 `mpd_keep_seconds > 0` 优先。直播媒体清单按 `start_time_offset_secs` 输出 `#EXT-X-START:TIME-OFFSET=-…`。

## 5. 订阅契约

### 5.1 响应头

| 路径 | 头 |
| --- | --- |
| `/playlist.m3u` | `content-type: application/x-mpegurl; charset=utf-8`、`content-disposition: attachment; filename="playlist.m3u"` |
| `/{gid}/playlist.m3u` | 同上，`filename="<gid>.m3u"` |

### 5.2 正文形状

```text
#EXTM3U x-tvg-url="http://<host>/epg.xml.gz"      ← EPG 开启才有
<server.m3u_header_lines 逐行原文>
#EXTINF:-1 tvg-id="<tvg_id||id>" tvg-name="<tvg_name||name>" tvg-logo="<logo>" group-title="<分组显示名>",<频道名>
http://<host>/<gid>/<cid>.<ext>[?token=<48hex>]
```

| 项 | 规则 | 依据 |
| --- | --- | --- |
| `tvg-id` | 缺省用频道 id | S-MODEL-04 |
| `tvg-name` | 缺省用频道名 | S-MODEL-04 |
| `group-title` | 分组**显示名** | F-013 §3 |
| Token 附加 | `token_enabled` 且 `access != open` 时附加 | S-URL-04、F-009 |
| `m3u_header_lines` | 追加在 `#EXTM3U` 后；热重载生效 | S-URL-07 |

### 5.3 `access` 矩阵

| access | 总订阅 | 分组订阅 | 直播入口 | EPG |
| --- | --- | --- | --- | --- |
| `public` | ✅ 含 | ✅ | 按 Token 开关 | ✅ |
| `hidden` | ❌ | ✅ `200` | ✅ | ✅ |
| `archived` | ❌ | ❌ `404` | ❌ `302` 占位 | ✅ 仍含 |
| `open` | ❌ | ✅（条目无 token） | ✅ 免 Token | ✅ |

> `open` 同时**排除出总订阅**——与 `hidden` 同规则（F-009）。

### 5.4 订阅聚合（P1 已实现）

`GET /playlist.m3u` 可合并外部 M3U 源（F-013 §5）。分组订阅不聚合。

| 配置 | 含义 |
| --- | --- |
| `[playlist].merge_ttl_secs` | 单个源的抓取缓存时长（秒，默认 300） |
| `[playlist].merge_position` | `top` 或 `bottom`（默认 `bottom`） |
| `[playlist].global_exclude` | 对所有源生效的过滤词 |
| `[[playlist.sources]].enabled` | `false` 时跳过 |
| `[[playlist.sources]].include` / `.exclude` | 源级过滤词；`include` 为空表示全收 |

过滤词规则：

| 写法 | 匹配对象 |
| --- | --- |
| `group:<文本>` | `group-title` 包含文本 |
| `id:<文本>` | `tvg-id` 完全相等 |
| 其它 | 频道名包含文本 |

规则与限制：

| 项 | 规则 |
| --- | --- |
| 大小写 | 不区分 |
| 优先级 | `exclude` 与 `global_exclude` 优先于 `include` |
| 地址 | 保留上游原始 URL；**不改写**，也不加本服务 Token |
| 失败 | 单个源超时或非 2xx → 跳过并记日志；本地订阅照常返回 `200` |
| 正则 | `/…/` 形式未实现（P2） |

## 6. 面板接口契约

### 6.1 已实现（M0）

| 方法 | 路径 | 请求 | 响应 |
| --- | --- | --- | --- |
| POST | `/panel/api/login` | `{"username","password"}` | `{"token":"<uuid>"}`；错 → `401`；失败过多 → `429`（`locked_for_secs`） |
| POST | `/panel/api/logout` | — | `{"ok":true}` |
| GET | `/panel/api/status` | — | 状态 JSON（含 `version`、`entry_exts`） |
| GET | `/panel/api/config` | — | 全量配置 JSON |
| PUT | `/panel/api/config` | 全量配置 | `{"ok":true}`；非法 → `400` |
| POST | `/panel/api/reload` | — | `{"ok":true}` |

登录锁：窗口内失败数达到 `auth_fail_free_per_window + auth_fail_ban_threshold` 后，后续登录返回 `429`。默认窗口 60 秒、免费 15 次、阈值 6、锁定 300 秒。既有 Bearer 会话不受影响。

### 6.2 M1 落地

| 方法 | 路径 | 请求 | 响应 | 依据 |
| --- | --- | --- | --- | --- |
| POST | `/panel/api/groups` | `{"id":"<gid>", …}` | `{"ok":true}`；重复 → `409` `Group id already exists` | F-013 §5 |
| PUT | `/panel/api/groups/{gid}` | **局部**字段 | `{"ok":true}`；缺组 → `404` `Group not found` | F-005、F-013 §5 |
| DELETE | `/panel/api/groups/{gid}` | — | `{"ok":true}`；缺组 → `404` | F-013 §5 |
| POST | `/panel/api/groups/{gid}/channels` | `{"id","name","url", …}` | `{"ok":true}` | F-004 |
| PUT | `/panel/api/groups/{gid}/channels/{cid}` | 局部字段 | `{"ok":true}` | F-004 |
| DELETE | `/panel/api/groups/{gid}/channels/{cid}` | — | `{"ok":true}`；缺 → `404` `Channel not found` | F-013 §5 |
| GET | `/panel/api/groups/export` | — | 顶层**数组**（完整分组对象） | F-013 §1 |
| POST | `/panel/api/groups/import` | — | 体为分组数组；id 已存在则跳过；`{"ok":true,"imported":n,"skipped":m}` | F-013 |
| POST | `/panel/api/groups/import/force` | — | 同上，但同 id 覆盖 | F-013 |
| POST | `/panel/api/groups/import/m3u` | — | 体 `{text,mode,group_mode}`；解析后落库 | F-013 §2 |
| GET | `/panel/api/logo/list` | — | 同 `/logo/api/list` | F-012 |
| POST | `/panel/api/logo/upload` | — | 体 `{"items":[{"name","ext","data"}]}`（`data` 可带 `data:image/…;base64,` 前缀）；返回 `{"ok":true,"added":[…],"failed":[…]}` | F-012 |
| POST | `/panel/api/logo/delete` | — | 体 `{"file"}`；成功 `{"ok":true,"removed":1}`；缺文件 → `404` | F-012 |
| POST | `/panel/api/logo/rename` | — | 体 `{"file","name"}`；保留扩展名；`{"ok":true,"logo":{…}}`；缺文件 → `404`，目标已存在 → `400` | F-012（形状自定） |
| POST | `/panel/api/logo/restore` | — | 体 `{"file"}`；用 `<cache>/logos/.orig/` 备份覆盖当前文件；`{"ok":true}`；无备份 → `400` | F-012 |
| POST | `/panel/api/logo/animate` | — | 体 `{"file","effect","quality":"<1..100>"}`；`effect` ∈ `diagonal/vertical/bounce/spot/glow/breathe`；返回 `{"ok":true,"frames","width","height","total_ms","bytes_before","bytes_after","dirty_ratio"}`；当前仅 PNG，解码依赖 `ffmpeg`/`ffprobe` | F-012 |
| GET | `/panel/api/logo/match-report` | — | `{"channels","missing","broken","unused","missing_channels":[…],"broken_logos":[…],"unused_logos":[…]}` | F-012（形状自定） |
| POST | `/panel/api/logo/apply` | — | 体 `{"gid","cid","file"}`；把 `/logo/<file>` 写入频道 `logo`；`404` 用于缺文件或未知分组/频道 | F-012（形状自定） |
| POST | `/panel/api/logo/suggest` | — | 体 `{"threshold","limit"}`；按名称相似度推荐，`{"threshold","count","channels":[{"gid","cid","name","suggest":[{"file","score"}]}]}` | F-012（形状自定） |
| GET | `/panel/api/groups/{gid}/export` | — | 单个分组对象 | F-013 §1 |

写入语义：分组与频道改动**立即重写** `config.toml`；`PUT /config` 是**全量替换**（F-013 §6）。

### 6.2.1 M3U 导入语义

请求体是 JSON（不是 multipart）：`{"text":"#EXTM3U…","mode":"append","group_mode":"auto"}`。

| 项 | 规则 |
| --- | --- |
| `mode` | `append`（默认，同名分组跳过）｜`merge`（同名分组补新频道）｜`replace`（同名分组整体替换）。其它值 → `400` |
| `group_mode` | `auto`（默认）或 `proxy`/`remux`/`dash`/`redirect`/`rewrite`/`passthrough`。其它值 → `400` |
| `auto` 判定 | 分组内有 `.mpd` 地址 → `mode=dash`、`format=mpd`；否则 `mode=rewrite`、`format=hls`（与 F-013 观测一致） |
| 分组 id | `group-title` 去掉非 ASCII 字母数字与 `_`/`-`；结果为空 → `g-<6hex>` |
| 频道 id | 有 `tvg-id` 用 `tvg-id`；否则 `g-<6hex>`（不幂等，与目标一致）。组内重复时追加 `-2`、`-3` |
| 无 `group-title` | 落到分组 `Default` |
| 属性保留 | `tvg_id`/`tvg_name`/`tvg_logo`/`group_title` 存入频道 `extra`；订阅渲染时 `logo` 优先、回退 `tvg_logo`；`#EXTVLCOPT`、`#EXTHTTP` 与行内 `http-user-agent`/`http-referer` 转为 `channel.headers` 字符串 |
| 响应 | `{"ok":true,"parsed_groups","parsed_channels","added_groups","merged_channels","replaced_groups","skipped_groups","group_mode"}` |

> `channel.headers` 在**代理/改写/串流（dash、remux）取上游清单与切片**时附加。302 模式（redirect、passthrough）由客户端直连上游，无法附加。空 `text` 或未解析到频道 → `400`。

### 6.3 后续里程碑（登记存在性）

| 组 | 路径 | 阶段 |
| --- | --- | --- |
| 缓存 | `POST /panel/api/cache/flush`、`GET /panel/api/cache/stats` | ✅ M4 |
| 素材 | `/panel/api/assets/{list,convert,remove,ffmpeg-status}`、`GET /assets/*path`（本地 HLS） | ✅ P2 |
| 录制 | `/panel/api/schedule`、`/schedule/from-epg`、`/schedule/{idx}`、`/schedule/{idx}/stop`、`/schedule/download/{filename}` | P1 |
| 推流 | `/panel/api/livestream`、`/livestream/{gid}/{cid}/{start,stop}` | P1 |
| DASH | `/panel/api/dash/{status,scan-url,tracks/:gid/:cid}`、`/dash-timeshift/status` | P2 |
| EPG | `/panel/api/epg/{status,refresh,regenerate,mappings,mapping,extra,match-report,scripts,script,script-delete,cron-preview,programmes/:channel,suggest/:gid/:cid,search,viewer-channels,source-channels/:id,tz-preview/:id}` | ✅ P1 |
| 安全 | `/panel/api/security/{token,token/rotate,ip/stats,ip/ban,ip/unban,ip/unban-batch}` | ✅ P1（见 §6.7） |
| 指标 | `/panel/api/{stats,logs,health}` | ✅ M4（`health` 在 M0） |
| 备份 | `GET /panel/api/backup?include=all\|epg-scripts\|resolver`、`POST /panel/api/restore` | ✅ P1（见 §6.6） |
| 导入 | `POST /panel/api/groups/import/m3u` | ✅ P1（见 §6.2） |
| 导入 | `/groups/{import,import/force}`（JSON 分组数组） | ✅ P1（`/groups/import/m3u` 相邻） |
| 订阅聚合 | `POST /panel/api/subs/{groups,preview}` | P2 |
| 翻译 | `/panel/api/epg/translate/{run,status,clear,clear-engine,probe,models,mtran/test}`（兼容无前缀别名）、`/panel/api/subtitle/{translate/clear,mtran/test}` | ✅ P2 |
| 分享 | `GET /api/share/pull?key=`（仅 GET/HEAD） | P2 |
| 解析脚本 | `/panel/api/resolver/{scripts,script-delete}` | P2 |

### 6.4 运维接口（M4 已实现）

响应形状为本实现自定；目标只登记了存在性与用途（F-010），未逐字段采证。

| 方法 | 路径 | 响应 |
| --- | --- | --- |
| `GET` | `/panel/api/cache/stats` | `{"backend","disk_path","entries":{"m3u8","seg_url","seg_bytes","sub_url","dash"},"bytes","playlist_cache_entries"}` |
| `POST` | `/panel/api/cache/flush` | `{"ok":true,"removed":<文件数>}`；清 `m3u8`/`seg_url`/`seg_bytes`/`sub_url`，**保留 `dash`**（S-CACHE-06）；同时丢弃进程内索引与清单缓存 |
| `GET` | `/panel/api/stats` | `{"version","uptime_secs","requests","playlists","segments","segment_hits","upstream_failures","online_window_secs","groups":[{"id","requests"}]}` |
| `GET` | `/panel/api/logs?limit=n` | `{"total","lines":[…]}`；`limit` 上限 500，默认 200；来源为进程内环形缓冲（500 行） |
| `POST` | `/panel/api/reload` | 立即重读配置；返回 `{"ok":true}` |

热重载：只监视配置文件本身；解析失败保留旧快照并记 `error` 日志（`design/04` §2）。冷却中的频道在配置重载后立即解除冷却（S-MODE-17）。

### 6.5 EPG（P1 已实现）

证据：F-008、F-010（S-EP-05、S-EP-12、S-EPG-07）。本实现覆盖公开三件、状态/刷新、手工映射与匹配报告。

| 方法 | 路径 | 鉴权 | 行为 |
| --- | --- | --- | --- |
| `GET` | `/epg.xml` | 无 | `200 application/xml; charset=utf-8`；`[epg].enabled = false` 或尚未生成 → `404` |
| `GET` | `/epg.xml.gz` | 无 | `200 application/gzip`（gzip 魔数 `1f 8b`）；同上 `404` |
| `GET` | `/epg` | 无 | `200 text/html`（内嵌浏览页）；`epg.enabled = false` 或 `public_viewer_enabled = false` → `404` |
| `GET` | `/panel/api/epg/status` | Bearer | `{"enabled","running","sources","channels","programmes","kept","keep_days","future_days","last_success","last_error"}` |
| `POST` | `/panel/api/epg/refresh` | Bearer | 同步采集一次；`{"ok":true,"msg":"采集完成","channels","programmes"}`；失败 → `500` + 原因 |
| `POST` | `/panel/api/epg/regenerate` | Bearer | 只读 `<cache>/epg/raw/` 重新匹配并生成，不重新下载；`{"ok":true,"channels","programmes"}` |
| `GET` | `/panel/api/epg/mappings` | Bearer | `{"mappings":{"gid/cid":{"source_id","source_channel_id","source_name","source_display_name"}},"extra":[{"source_id","source_channel_id","out_id","out_name"}]}` |
| `POST` | `/panel/api/epg/mapping` | Bearer | `{"gid","cid","source_id","source_channel_id"}`；两者留空 → 清除 |
| `POST` | `/panel/api/epg/extra` | Bearer | `{"action":"add","source_id","source_channel_id","out_id","out_name"}`；`{"action":"remove","source_id","source_channel_id"}`；批量 `items[]` |
| `GET` | `/panel/api/epg/match-report` | Bearer | `text/plain`：`matched_count=… empty_count=… extra_count=…` + 表格 |
| `GET` | `/panel/api/epg/scripts` | Bearer | `{"python":<版本或 null>,"scripts":[{"name","size"}]}` |
| `POST` | `/panel/api/epg/script` | Bearer | `{"name","content"}`；只接受 `.py`；`{"ok":true,"script":{…}}` |
| `POST` | `/panel/api/epg/script-delete` | Bearer | `{"name"}`；`{"ok":true,"removed":<bool>}` |
| `POST` | `/panel/api/epg/cron-preview` | Bearer | `{"expr"}`；`{"ok":true,"next":[epoch…]}` 或 `{"ok":false,"error"}` |
| `GET` | `/panel/api/epg/programmes/{channel}` | Bearer | `{"now","pad_before","pad_after","programmes":[{"disp_start","disp_end","stop_epoch","rec_start","rec_end","live","title","desc"}]}` |
| `GET` | `/panel/api/epg/suggest/{gid}/{cid}` | Bearer | `{"suggestions":[{"source_id","source_channel_id","source_name","display_name","score"}]}` |
| `GET` | `/panel/api/epg/search?q=` | Bearer | 同公开搜索 |
| `GET` | `/panel/api/epg/viewer-channels` | Bearer | 同公开频道表 |
| `GET` | `/panel/api/epg/source-channels/{id}?q=&offset=&limit=` | Bearer | `{"items":[{"id","display_names","source_id","source_name"}],"total"}`；`id=__all__` 跨源 |
| `GET` | `/panel/api/epg/tz-preview/{id}?shift=` | Bearer | `{"ok":true,"items":[{"raw","current","shifted"}]}`；`shift` 单位为分钟 |
| `GET` | `/epg/api/channels` | 无 | `{"count","channels":[{"id","names":[{"lang","name"}]}]}` |
| `GET` | `/epg/api/programmes/{channel}` | 无 | `{"channel","count","programmes":[{"channel","start","stop","title","desc","categories"}]}` |
| `GET` | `/epg/api/search?q=` | 无 | `{"q","count","programmes":[…]}`；最多 200 条；空 `q` 返回 0 |

采集规则（F-008 观测值）：

| 项 | 规则 |
| --- | --- |
| 源类型 | `kind = "xml"`：按 `url` 拉 XMLTV。`kind = "script"`：执行 `<cache>/epg/scripts/<script>`（`python_bin`，默认 `python3`；cwd 为脚本目录，stdout 即 XMLTV；为空时读 `output_files`）。`allow_scripts = false` 时拒绝脚本源 |
| 源缓存 | 每次抓取/执行后写 `<cache>/epg/raw/<source_id>.xml`；`regenerate` 与单源定时任务只读该缓存 |
| 定时 | `[epg].cron` 与源级 `cron` 均按标准 5 字段解析；源级空表示仅随全局采集。非法全局表达式回退 4 小时 |
| 超时 | 源级 `timeout_secs > 0` 优先，否则用 `[epg].script_timeout_secs`；均限制在 5–3600 秒 |
| 时间偏移 | `source.tz_shift_mins` 在解析后平移 `start`/`stop`，单位分钟，正数向后 |
| 时间窗 | 保留 `stop >= 现在 - keep_days` 且 `start <= 现在 + future_days`；`keep_yesterday = true` 时下界不晚于昨天 0 点 |
| 输出标识 | `out_id = tvg_id \|\| channel.id`；XMLTV 的 `channel/@id` 与节目 `channel` 使用 `out_id` |
| 额外频道 | `extra.json` 持久化；`out_id` 为空时用源频道 ID；与输出 ID 冲突时跳过 |
| 频道匹配 | 手工映射优先；否则 `tvg_id`（大小写不敏感）→ 名称（删全部空白含 U+3000、转小写、完全相等，不做子串）；`match_by_name=false` 时只保留 `tvg_id` |
| 冲突 | 同一源频道被多个本地频道命中 → 配置顺序最后一个获得节目；重复 `out_id` 标为 `duplicate` |
| 输出 | `<cache>/epg/epg.xml` 与 `epg.xml.gz`；根节点 `generator-info-name="iptv-proxy"`；保留 `programme` 的 `start`/`stop`/`channel` |
| 语言 | 文本节点缺 `xml:lang` 时补 `lang="zh"`；显式语言保留 |
| 原样保留 | `icon`、`credits` 等未建模子节点按原片段输出 |
| 手工映射 | 存 `<cache>/epg/mappings.json`；`GET /panel/api/epg/mappings` 读取；刷新后补源频道显示名 |
| 调度 | 启动即采集一次；之后每分钟检查 `[epg].cron` 与源级 `cron`。全局到期刷新全部源，单源到期只刷新该源再用 raw 缓存重生成 |

> `/epg/api/*` 的响应形状为本实现自定（目标只登记了路由存在性）；面板私有 EPG 端点按目标面板调用形状实现。
> 实现：按请求读取 `epg.xml` 并解析；节目单较大时再改为内存快照。
>
| 翻译 | 读 `[epg.translate]`：`enabled`、`url`（或 `engine="mtran"` 时的 `mtran_url`）、`token`/`mtran_token`、`source`/`mtran_source`、`target`/`targets`、`translation_first`、`mtran_batch`。采集后把节目文本批量发到 MTranServer v5 `/translate/batch`（`{from,to,texts}` → `{results}`），兼容旧 `{texts,source_lang,target_lang}`；`/translate/batch` 不可用时回退单条 `/translate`。译文按目标语言追加为 `lang="<target>"` 的文本节点；`translation_first = true` 时排在原文前。结果缓存在 `<cache>/epg/.translate/cache.json`，命中不发请求 |
| 翻译接口 | `GET /epg/translate/{status,models}` 报告引擎与目标语言；`POST /epg/translate/{run,probe,mtran/test,clear,clear-engine}`。面板别名在相同路径前加 `/panel/api`。`run/probe` 需面板会话；未启用或未配地址 → `400`。`clear*` 清空缓存 |

### 6.6 备份与恢复（P1 已实现）

| 项 | 规则 |
| --- | --- |
| 备份请求 | `GET /panel/api/backup?include=all|epg-scripts|resolver`；其它值回退 `all` |
| 备份响应 | `200 application/gzip`；`Content-Disposition: attachment; filename="iptv-proxy-backup-YYYYmmdd-HHMMSS.tar.gz"`（UTC） |
| 包内布局 | `iptv-proxy-backup/config.toml`、`logos/*`、`epg/scripts/*.py`、`epg/epg_map.json`、`resolvers/*.py`、`manifest.json`（`generator`/`version`/`created`/`scope`） |
| 恢复请求 | `POST /panel/api/restore`，`Content-Type: application/gzip`，体为 tar.gz 原文；上限 64 MiB |
| 恢复步骤 | 解包 → 解析并**校验** `config.toml` → 旧文件另存 `config.toml.bak` → 原子写入 → 恢复台标 → 重载快照（立即解除看门狗冷却） |
| 恢复响应 | `{"ok":true,"config_reloaded":true,"report":{"files":[…],"skipped":[…]},"groups":N}` |
| 失败 | 无 `config.toml`、配置不合法、非 gzip → `400`；未登录 → `401` |

> 台标以外的脚本类文件（`epg/scripts`、`resolvers`）只备份，不在恢复时执行。

### 6.7 安全面（P1 已实现）

响应形状为本实现自定（目标只登记了路由存在性，F-010）。

| 方法 | 路径 | 响应 |
| --- | --- | --- |
| `GET` | `/panel/api/link-info` | `{"aliases":{},"random_id_enabled":false,"token":"","token_enabled":false,"use_alias":false}`；形状与目标一致（S-GAP-04）；Token 关闭时 `token` 为空串 |
| `GET` | `/panel/api/security/token` | `{"enabled","ttl_hours","current","old"}`；`old` 为宽限期内的旧值 |
| `POST` | `/panel/api/security/token/rotate` | `{"ok":true,"token":"<48hex>"}`；旧值保留 1 小时宽限（S-AUTH-09） |
| `GET` | `/panel/api/security/ip/stats` | `{"count","banned","entries":[{"ip","requests","failures","banned","remaining_secs"}]}`，按请求数降序 |
| `POST` | `/panel/api/security/ip/ban` | `{"ip","seconds"}`；`seconds = 0` 用默认 3600，上限 30 天；返回实际时长 |
| `POST` | `/panel/api/security/ip/unban`<br>`POST …/ip/unban-batch` | `{"ip"}` 或 `{"ips":[…]}`；`{"ok":true,"removed":n,"requested":n}`；两个路径同义 |

执行规则：

| 项 | 规则 |
| --- | --- |
| 记账 | 非 `/panel` 路径按 `X-Forwarded-For` 首项或 `X-Real-IP` 记账；都没有时记 `unknown` 并忽略 |
| 拦截 | 封禁期内 → `403` 纯文本 `banned`；面板路径不拦截（避免管理员自锁） |
| 容量 | 最多保留 10 000 个地址，超出后忽略新地址 |
| 未实现 | 按 IP 的会话上限、自动限流与 `/unban` 自助解封（P2） |

### 6.8 面板分区（前端）

| 分区 | 数据来源 | 状态 |
| --- | --- | --- |
| 概览 | `GET /panel/api/status`、`GET/PUT /panel/api/config` | ✅ |
| 分组管理 | `/panel/api/{groups,channels}` 与导入 | ✅ |
| 安全管控 | `/panel/api/security/{token,token/rotate,ip/stats,ip/ban,ip/unban}` | ✅ |
| 缓存状态 | `GET /panel/api/cache/stats`、`POST /panel/api/cache/flush`（二次确认） | ✅ |
| 频道热度 | `GET /panel/api/stats`（总请求、切片、上游失败、分组排行） | ✅ |
| 节目指南 | `GET /panel/api/epg/status`、`POST /panel/api/epg/refresh`；链接到 `/epg` 浏览页 | ✅ |
| 运行日志 | `GET /panel/api/logs?limit=`（5 秒轮询） | ✅ |
| 备份恢复 | `GET /panel/api/backup`、`POST /panel/api/restore`（浏览器内下载/上传） | ✅ |
| 素材 | `/panel/api/{assets,subtitle}/*`、`/panel/api/assets/ffmpeg-status` | ✅ |
| 录制与回看 | `/panel/api/schedule*`、`/panel/api/livestream*`、`GET /panel/api/dash-timeshift/status` | ✅ |
| 订阅聚合 | `GET/PUT /panel/api/playlist`（源、过滤、缓存与合并位置）；清单 `GET /playlist.m3u` | ✅ |
| 系统状态 | `GET /panel/api/health`、`/assets/ffmpeg-status`、`/resolver/scripts`、`/dash-timeshift/status` | ✅ |
| 台标 | 独立页 `/logo`（`/logo/api/*`） | ✅ |
| 其它（系统设置） | 概览页已支持 `GET/PUT /panel/api/config` 全量编辑 | ✅ |

### 6.9 素材与脚本（P1 已实现）

目标只登记了路由存在性（F-011、S-EP-16），响应形状为本实现自定。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/panel/api/assets/list` | `{"kind","count","files":[…],"items":[{"url","converted","local_path"}],"ffmpeg_installed","ffmpeg_path"}`；`items` 来自配置中的非 HLS URL |
| `POST` | `/panel/api/assets/convert` | `{"url"}`；下载后调用 `ffmpeg` 转 HLS，写 `[assets].map`，返回 `{"ok":true,"url","local_path","bytes"}` |
| `POST` | `/panel/api/assets/upload` | `{"items":[{"name","ext","data"}]}`；`data` 可带 `data:` 前缀 |
| `POST` | `/panel/api/assets/remove` | `{"file"}` 或 `{"url"}`；按 URL 删除时同时清 `[assets].map` 与本地目录 |
| `GET` | `/panel/api/assets/raw/<file>` | 面板内下载（`private, max-age=0`） |
| `GET` | `/assets/*path` | 公开读取 `<cache>/assets/` 下的 HLS 清单与切片 |
| `GET` | `/panel/api/assets/ffmpeg-status` | `{"installed","version","remux_ready","note"}`；每次实时执行 `ffmpeg -version` |
| `GET` | `/panel/api/subtitle/list`、`…/upload`、`…/remove`、`…/raw/<file>` | 与视频同形；扩展名 `srt/ass/ssa/vtt` |
| `GET` | `/panel/api/subtitle/translate/status` | `{enabled,engine,url,source,targets,translation_first,formats,converts_ass,cached}` |
| `POST` | `/panel/api/subtitle/translate` | `{"file","target"}`：翻译 SRT/VTT/ASS/SSA，返回 `{ok,file,target,format,cached,lines?,bytes,download}`；ASS/SSA 转 VTT 后翻译；未启用/无地址/非 UTF-8 → `400` |
| `GET` | `/panel/api/subtitle/translate/raw/<name>` | 下载译文；名称须为 32 位十六进制 + `.srt`/`.vtt` |
| `GET` | `/panel/api/resolver/scripts` | `{"count","scripts":[{"name","bytes"}]}`（`<cache>/resolvers/*.py`） |
| `POST` | `/panel/api/resolver/script` | `{"name","content"}`；写 `<name>.py`；空内容 → `400` |
| `POST` | `/panel/api/resolver/remove` | `{"name"}`；缺文件 → `404` |

规则：

| 项 | 规则 |
| --- | --- |
| 目录 | `<cache>/assets/`、`<cache>/subtitles/`、`<cache>/resolvers/` |
| 文件名 | 与台标同规则（去路径、允许中文、拒 `..`） |
| 视频扩展 | `mp4/mkv/ts/mov/webm/m4v`；其它进 `failed`，不落盘 |
| 上传上限 | 每个请求 64 MiB（与恢复一致） |
| 脚本执行 | 只保存与列出；**不执行**（P2） |
| 字幕翻译 | 只翻字幕块正文；时间行、序号、`NOTE`/`STYLE` 块与 WebVTT 设置行原样保留 |
| ASS/SSA | 读 `[Events]` 的 `Format:` 与 `Dialogue:`；去 `{\…}` 标签，`\N` 转换行；输出 WebVTT |
| 字幕译法 | 每个正文行单独翻译并按 `batch`/`mtran_batch` 分批（默认 40）；按 `translation_first` 输出「原文 + 译文」或「译文 + 原文」 |
| 字幕缓存 | `<cache>/subtitles/.translate/<sha256(file,target,order,format)[..32]>.<ext>`；ASS/SSA 输出 `.vtt`；命中不发请求；`clear` 清空 |

### 6.10 多 VPS 分享（P1 已实现）

证据：F-013。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/api/share/pull?key=<share.key>` | 返回 `{"token","token_old","bans":[],"unbans":[]}` |
| `POST` | `/api/share/pull` | `405`，`Allow: GET,HEAD` |

门禁与规则：

| 项 | 规则 |
| --- | --- |
| 未启用 | `[share].enabled = false` → `404`（形状自定） |
| 密钥 | `key` 必须等于 `[share].key`；空 key 拒绝 |
| 源 IP | 必须在 `[share].trust_ips`；支持精确地址、CIDR 与 `*`；不匹配 → `403` + 日志 |
| 源 IP 取值 | `X-Forwarded-For` 首项 → `X-Real-IP` → TCP 对端 |
| 载荷 | `token` 为当前播放 Token；`token_old` 为宽限期旧值；`bans` 为封禁中地址；`unbans` 为最近两个周期内解封的地址 |
| 角色 | `master` 每周期拉取 `follower_urls` 汇总；`follower` 从 `master_url` 采用 Token |
| 周期 | `[share].poll_secs`，最小 5 秒 |
| follower 轮换 | `POST /panel/api/security/token/rotate` → `409`（`token 由主机接管`） |
| 封禁同步 | 对端封禁按 30 天写入本机；解封立即生效 |

> 差异：目标启动后立即同步；本实现延迟 5 秒，避开启动竞态。

### 6.11 健康与 DASH 探测（P1 已实现）

目标只登记了路由存在性（F-011、F-013），响应形状为本实现自定。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/panel/api/health` | `{ok, version, uptime_secs, time, bind, groups, channels, sessions, cache, epg, security, share}` |
| `GET` | `/panel/api/dash/status` | 运行中的 DASH 会话数组；`stream_key`、`source_url`、`phase`、`segment_count`、`active_tracks` 等 |
| `GET` | `/panel/api/dash/tracks/:gid/:cid` | 抓频道 MPD 并列出轨道；未知频道 → `404 Channel not found` |
| `POST` | `/panel/api/dash/scan-url` | `{"url"}`；只解析，不改配置；非 HTTP(S) → `400` |
| `GET` | `/panel/api/dash-timeshift/status` | `{ok,enabled,ready,channels[{group_id,channel_id,hours,segments,mode}],note}`；代理频道直接缓冲，DASH 频道经 ffmpeg 转 HLS 后缓冲 |

轨道对象：`{id, kind, codecs, bandwidth, lang, width, height}`。
`kind` 取 `video`/`audio`/`text`/`unknown`。
`tracks`/`scan-url` 只读取 `AdaptationSet` 与 `Representation` 属性，不下载媒体；播放链路的 `dash` 模式按需解析完整 MPD 并代理 init/切片。

### 6.12 上游解析脚本（P1 已实现）

证据：F-009 §5（S-RES-01）。

频道在 `[groups.channels]` 中用三个键开启脚本解析：

| 键 | 默认 | 说明 |
| --- | --- | --- |
| `resolver_script` | 空 | 脚本文件名，位于 `<cache>/resolvers/<name>.py`；空表示关闭 |
| `resolver_arg` | 空 | 作为 `argv[1]` 传入 |
| `resolver_ttl` | `30` | 结果缓存秒数 |

执行规则：

| 项 | 规则 |
| --- | --- |
| 命令 | `python3 <cache>/resolvers/<脚本> <arg>` |
| 工作目录 | 脚本所在目录 |
| 输出 | stdout 最后一个非空行；必须是 `http://` 或 `https://` |
| 超时 | 30 秒 |
| 覆盖 | 解析成功时忽略频道 `url` |
| 缓存 | TTL 内直接返回；过期先返回旧值并后台刷新（SWR） |
| 失败 | 冷启动失败、超时或输出非法 → `302 error_url` |
| 回看 | `playseek` 追加到解析后的地址 |

### 6.13 Remux（P1 已实现）

证据：F-009 §1。

| 项 | 值 |
| --- | --- |
| 输入 | 连续 TS（HTTP 裸流）；HLS 上游不会产出切片 |
| 输出 | `#EXT-X-VERSION:3`、`#EXT-X-TARGETDURATION:5`、`#EXT-X-MEDIA-SEQUENCE`，窗口 4 条 |
| 切片 | 按视频关键帧切分（PTS 计时，5 秒）；扩展名轮换 `png/gif/bmp/jpg/webp` |
| 路径 | `/{gid}/{cid}/rseg_<12 位序号>.<ext>` |
| Content-Type | 与扩展名一致的图片类型 |
| Cache-Control | 切片 `public, max-age=30`；清单 `no-cache, must-revalidate` |
| 预热 | 未产出首片 → `503` + `Retry-After: 2` + `remux warming up` |
| 会话 | 键 `gid/cid`；上游断开后每 500 ms 重连；空闲 90 秒回收并删目录 |
| 上游头 | 附加 `channel.headers` |

> 目标按关键帧对齐；本实现用 PES PTS 与 H264/HEVC 关键帧 NAL 判定。

### 6.14 定时录制与直播开关（P1 已实现）

证据：F-009 §2。`{idx}` 是列表下标，不是任务 ID。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/panel/api/schedule` | 任务数组 |
| `POST` | `/panel/api/schedule` | 新增，返回 `{"ok":true,"id"}`；未知分组/频道 → `404` |
| `PUT` | `/panel/api/schedule/:idx` | 更新字段 |
| `DELETE` | `/panel/api/schedule/:idx` | `{"ok":true,"was_running"}` |
| `POST` | `/panel/api/schedule/:idx/stop` | 停止录制 |
| `GET` | `/panel/api/schedule/download/:filename` | 下载产物 |
| `POST` | `/panel/api/schedule/from-epg` | 批量建任务：`{"items":[…]}` → `{ok,created,ids,failed}` |
| `GET` | `/panel/api/livestream` | 已点亮频道 |
| `POST` | `/panel/api/livestream/:gid/:cid/{start,stop}` | 改写频道 `livestream` 并落盘 |
| `POST` | `/subtitle/mtran/test` | 机翻自检：未配 `[subtitle.translate].url` → `400`；否则调 MTranServer，返回 `{ok,result,...}`。地址可写根地址或 `/translate`；`token` 以 Bearer 发送 |
| `POST` | `/subtitle/translate/clear` | 清空 `<cache>/subtitles/.translate/`，返回 `{ok,cleared}` |

任务字段：`id,name,group_id,channel_id,start_time,end_time,format,status,output_file,extra_files,note,error,created_at,file_size,record_secs`。
时间格式 `YYYY-MM-DDTHH:MM:SS`（按 `[server].timezone_offset_hours` 解释）。

录制规则：

| 项 | 规则 |
| --- | --- |
| 调度 | 每 10 秒扫描；开始时间已到 → `recording` |
| 取材（HLS） | 抓媒体清单，增量下载切片并合并；主清单取最高码率 |
| 取材（DASH） | 确认 `type=dynamic` 后，增量抓取各轨 init/切片；静态 MPD 直接失败 |
| 产物（HLS） | `<cache>/recordings/<name>_<unix>.ts`；`format=mkv` 且有 ffmpeg 时转封装为 MKV |
| 产物（DASH） | `<cache>/recordings/<name>_<unix>.mkv`；`ffmpeg -c copy` 并轨视频与音频 |
| 状态 | `pending` / `recording` / `finished` / `failed` |
| 失败 | 无切片（如静态 MPD）→ `failed`，`error` 记录原因；DASH 缺 ffmpeg → `failed` |
| 时移 | 未实现（P2） |

### 6.15 直播回看（P1 已实现，代理频道）

证据：F-009 §1.8。

| 项 | 规则 |
| --- | --- |
| 启用 | 频道 `livestream = true`（代理）或 `dash_timeshift = true`（DASH）；启动时或点亮后开始缓冲 |
| 缓冲 | 抓上游媒体清单，增量下载切片；主清单取最高码率；窗口 `timeshift_hours`，容量 `timeshift_max_gb`（0 = 不限） |
| 参数 | `?start=<unix>&end=<unix>` 或 `?playseek=<本地 yyyymmddHHMMSS-yyyymmddHHMMSS>` |
| 清单 | `#EXT-X-PLAYLIST-TYPE:VOD`、`#EXT-X-TARGETDURATION`、`#EXT-X-ENDLIST`；`Cache-Control: no-cache, must-revalidate` |
| 切片 | `/{gid}/{cid}/timeshift/<16hex>.<png|gif|bmp|jpg|webp>`；图片 Content-Type；`Cache-Control: private, max-age=3600` |
| 越界 | 缓冲区无交集 → 落回实时清单，不报错 |
| 会话 | 键 `gid/cid`；15 分钟无访问回收；索引在内存，重启后从空开始 |
| 范围 | 代理频道；DASH 频道设 `dash_timeshift = true` 时，用 ffmpeg 转本地 HLS 后缓冲（需 ffmpeg 含 DASH 解复用器） |

## 6.16 订阅源预检（P3 已实现；F-020 对拍）

证据：S-GAP-07、F-020。

| 方法 | 路径 | 请求 | 响应 |
| --- | --- | --- | --- |
| GET | `/subs/status`、`/panel/api/subs/status` | — | `{"sources":[{"id","name","enabled","error","channels","last_ok","last_try","ttl"}]}` |
| POST | `/subs/groups`、`/panel/api/subs/groups` | `{"url":"<m3u>"}` | `{"groups":[{"name","count"}],"total":<n>}` |
| POST | `/subs/preview`、`/panel/api/subs/preview` | `{"source":{"id"?,"url"?,"group_map":{…},"group_mode"?,"merge_group"?,"unlisted_groups"?,"include"?,"exclude"?}}` | `{"kept","sample":[{"name","group","tvg_id"}]×≤5,"trace":{"total","after_group","after_global_exclude","after_include","after_exclude","dropped_groups","out_groups":[{"name","count"}]}}` |

规则：

- `source.id` 命中 `[playlist].sources` 时，未提供的字段用配置补齐。
- `group_map` 是选择集：键命中即选中；未列出的分组按 `unlisted_groups` 处理：`drop`（默认）或 `keep`。空映射 + `drop` → 零频道。
- `group_mode`：`keep`（默认/未知值）保留原名；`map` 使用映射值，空值保留原名；`merge` 在 `merge_group` 非空时统一分组。
- 未带 `group-title` 的条目分组名为 `(无分组)`。
- 顺序固定：分组选择/改名 → `global_exclude` → 源 `include` → 源 `exclude`。
- `trace` 的 `after_group` 只计分组阶段；`after_global_exclude`、`after_include`、`after_exclude` 均在分组与全局排除之后，按各自阶段累计。
- 过滤词：频道名包含；`group:<文本>` 同时匹配原分组与映射分组；`id:<tvg-id>` 精确匹配。
- 源抓取超时 20 秒。`sample` 最多 5 条。
- `ttl_secs=0` 按 3600 秒；状态 `channels` 为去重后并入条数。
- 聚合按 URL 去重，范围覆盖同一源与多个源。

### 6.17 自助解封（P3 已实现）

证据：S-EP-07、S-AUTH-07。

| 项 | 规则 |
| --- | --- |
| 入口 | `GET /unban` 免登录；封禁期也可访问 |
| 提交 | `POST /unban` 免登录；只解封调用方地址 |
| 配额 | 每个 IP 30 天最多 3 次；成功解封才消耗 |
| 未封禁 | `200` `{"ok":false,"ip","reason":"not_banned","remaining","window_secs"}` |
| 成功 | `200` `{"ok":true,"ip","remaining","window_secs":2592000}` |
| 配额用完 | `429` `{"error":"本月解封次数已用完","remaining":0,"window_secs"}` |
| 计数 | 保存在内存；进程重启后清零 |

### 6.18 上游代理覆盖（P3 已实现）

证据：S-GAP-05。目标只登记字段，行为未验。

| 项 | 规则 |
| --- | --- |
| 字段 | `[[groups]]` 或 `[[groups.channels]]` 的 `proxy{kind,addr,user,pass}` |
| 优先级 | 频道级 > 分组级 > 直连 |
| 支持 | `http` 与 `https`；`user` 非空时用 basic auth |
| 生效范围 | 清单、切片、回看、Remux、录制、DASH 回看 |
| 不支持 | `socks5` 等类型；遇到时不代理并记告警 |
| 客户端缓存 | 键为 `kind\|addr\|user\|pass`，配置热重载后按新键重建 |

## 7. 错误串对照

面板错误体保留目标原文，便于前端与外部脚本匹配（F-012 §3、F-013 §5）：

| 串 | 场景 |
| --- | --- |
| `Group not found` | 分组不存在 |
| `Group id already exists` | 建组 id 重复 |
| `Group archived` | 归档组播放 |
| `Channel not found` | 频道不存在 |
| `Invalid channel` | 频道 id 不合法 |
| `Segment not found` | 切片未知 |
| `Rate Limited` | 限流 |
| `Upstream Error` | 上游失败 |
| `Master M3U8` | 主清单相关 |

> 面板提示文案（登录失败、保存成功）与目标**不逐字对齐**：目标面板是闭源前端，我们自研 UI（`spec/05` 纪律：不复制目标前端代码与文案）。

## 8. 已知差异（登记在案，不重复实现）

| 差异 | 处理 | 依据 |
| --- | --- | --- |
| `<P>` 生成算法未解 | 自研 `sha256("{gid}:{cid}")[:16]` + 校验；客户端从清单取值 | F-017 |
| `rewrite` + MPD 正文损坏 | 不复刻该缺陷 | D-005 |
| `/epg/api/*` 仅 v0.8.8 | P0 不实现 | S-GAP-02 |
| 缓存上限未生效（目标缺陷） | P0 不实现上限 | F-015 |
| `/status`、`/status/metrics` 非路由 | 保持 `302` 假路由 | S-EP-08 |

## 9. M1 实施清单

> 状态：**已实现**（`src/http/routes/groups.rs`、`src/playlist.rs`、`src/auth/play_token.rs`、`frontend/src/groups.tsx`；验证见 `scripts/smoke-m1.sh` 与 `PROGRESS_bak.md` §3c）。`x-tvg-url` 在 EPG 开启时输出（见 `src/playlist.rs` `header()`）。

1. 分组 CRUD + 频道 CRUD（§6.2），改后原子写盘并替换内存快照。
2. 订阅渲染（§5）：总订阅与分组订阅、`access` 过滤、Token 附加、`m3u_header_lines`、`x-tvg-url`（EPG 开启时输出）。
3. 分组导出（§6.2 两个 `export`）。
4. 冒烟脚本扩展：分组/频道增删改、订阅形状断言、`access` 矩阵断言。
