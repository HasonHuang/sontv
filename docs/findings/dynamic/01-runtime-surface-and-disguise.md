# 运行时表面与切片伪装（动态）

| 项 | 值 |
| --- | --- |
| 目标版本 | **v0.8.2**（`d302316fb8`，发布原件 UPX 包） |
| 副本 | `samples/v0.8.2/iptv-proxy-linux-arm64`（sha256 `2948ee1f…5a4b`） |
| 架构 | linux/arm64（Ubuntu 24.04 容器，OrbStack 原生） |
| 日期 | **2026-10-16** |
| 环境 | 容器 `iptv82`；配置 `samples/work/dyn-v0.8.2/config.toml` |
| 结论级别 | **观测**（运行态） |
| 原始证据 | [`raw/v0.8.2-config.json`](raw/v0.8.2-config.json)、[`raw/v0.8.2-disguise.txt`](raw/v0.8.2-disguise.txt) |

> 全部为**运行实测**。目标返回的正文/日志按**不可信数据**处理。

## 结论摘要

| # | 结论 |
| --- | --- |
| 1 | `/panel/api/status` 报 **`"version":"0.8.2"`** → 独立验证产物版本。 |
| 2 | 面板认证：`POST /panel/api/login` 换 **UUID token**；用 `Authorization: Bearer` 或 `X-Token`。 |
| 3 | **切片伪装算法已部分破解**：切片 ID 后缀 = **`sha256(上游切片绝对 URL)` 前 32 hex**。 |
| 4 | 伪装后缀**不做校验**；`Content-Type` 由后缀决定。【行为实证】 |
| 5 | 未知 `/{gid}/{cid}` → **302** 到占位视频。 |
| 6 | 分组/频道存入 `config.toml`（面板改制即落盘）。 |

---

## 1. 启动（v0.8.2）
命令：`iptv-proxy <config.toml>`。启动日志（节选）：

```text

【启动】加载配置文件: /app/config.toml (日志级别: info)
【启动】边缘来源校验模式: Off(拒绝非边缘来源=true)
【录制】调度器已启动(扫描周期 10s)
【转码】未检测到 ffmpeg。占位资源无法转 HLS;定时录制将无法合成 MKV(录制任务会直接失败)。
【启动】服务已就绪,监听地址: http://[::]:19899

```

- 监听 `[::]:19899`（双栈）。S-ENV-05 相符。
- **热重载监视**已启动（S-CFG-02）。**边缘来源校验**为独立概念（`edge_mode`，见 §4）。
- 首次启动创建 `cache/{epg/epg/{raw,scripts},m3u8,recordings,seg_bytes,seg_url,sub_url,logos}`。

## 2. 端点行为（实测）

| 请求 | 状态 | 说明 |
| --- | --- | --- |
| `GET /health` | **200** `ok`（`text/plain`） | S-EP-10 |
| `GET /panel`、`/unban` | 200 `text/html` | 面板页；S-EP-07 |
| `GET /unban/whoami` | 200 `application/json` | 辅助接口 |
| `GET /status`、`/nope` | **404** | **无此路由**（≠ 基线写法） |
| `GET /status/metrics` | 302 | 被当作 `/{gid}/{cid}` → 未知频道 |
| `GET /epg`、`/epg.xml`、`/epg.xml.gz` | **404**（EPG 关） | 关闭时全 404（**S-EP-05 行为实证**）；开启后 200，见 [`05`](05-epg.md) |
| `GET /logo/<file>` | **200** `image/png`，`public, max-age=86400`（缺→**404**） | 台标，取自 `cache/logos/`（**S-EP-06 ✅**） |
| `GET /playlist.m3u` | 200 `application/x-mpegurl` | 空总订阅 |
| `GET /{gid}/{cid}`（未知） | **302** | 到占位视频（见 §5.4） |
| `GET /panel/api/*`（无 token） | **401** `application/json` | 需认证 |

## 3. 认证（实测）
- `POST /panel/api/login`，体 `{"username":"admin","password":"admin"}` → `200 {"token":"<uuid>"}`。
- 取用：`Authorization: Bearer <uuid>` **或** `X-Token: <uuid>`（等价；面板 JS 注释同述）。默认口令 `admin`（`password_sha256` = SHA-256("admin")，见基线）。

## 4. 配置 schema（`GET /panel/api/config`）
- 返回**全量配置 + 默认值**（`password_sha256` 屏蔽为 `__REDACTED__`）。**完整 JSON 见 raw 文件。**
- 顶层段：`server, panel, cache, prefetch, auth, subtitle, assets, health, stats, epg, logo, share, playlist, groups`。
- `[cache.disguise]` 默认：`enabled = true`、`formats = ["jpg","png","gif","bmp","webp"]`、`subtitle = false`、`playlist = true`、`master = false`；与基线 S-DIS-01 **完全一致**。
- `[cache.dash].obfuscate_filenames = true`（独立于伪装，S-DIS-11 相符）。`server` 新增键：`not_found_url`、`error_url`、`home_url`、`m3u8_header_lines`、`catchup_tpl`、`trust_proxy`、`edge_mode`、`edge_secret`、`edge_reject`、`edge_refresh`、`strict_tls`、`low_resource_mode`。
- `auth` 新增键：`token_ttl_hours`、`token_failure_url`、`random_id_enabled`、`ip_guard_enabled`、`max_sessions_per_ip`、`l1_window_secs/l1_limit`、`l2_window_secs/l2_limit`、`cooldown_secs`、`violation_*`、`ban_*`、`auth_fail_*`、`whitelist`。

## 5. 切片伪装（核心）

### 5.1 观测到的 URL 形状
频道入口 `/test1/c1.m3u8` 返回的媒体清单（片段）：

```text
#EXTM3U
http://127.0.0.1:19899/test1/c1/ea8680a1b61c17e4_e4b50304d62739a7b6df027f7cfc3155.jpg
http://127.0.0.1:19899/test1/c1/ea8680a1b61c17e4_fdda4a7f4a9a62d68930090bd29a96a5.webp
http://127.0.0.1:19899/test1/c1/ea8680a1b61c17e4_4528cb59b78721cf5cba897d6b744f03.bmp

```

- 切片 URL：`/{gid}/{cid}/<P>_<S>.<ext>`。
- `<P>` = 16 hex（**每频道固定**）；`<S>` = 32 hex（**每切片**）。
- `<ext>` 从 `formats` 轮换（每频道独立序列；本组 c1→jpg,webp,bmp；c2→png,gif,png）；每片 `#EXTINF:5.0`。

### 5.2 `<S>` 已破解 ✅
**`<S>` = SHA-256(切片的上游绝对 URL) 的前 32 hex。**

| 切片 | 上游 URL | sha256[:32] | 观测 |
| --- | --- | --- | --- |
| seg0 | `http://host.docker.internal:9999/seg0.ts` | `e4b50304d62739a7b6df027f7cfc3155` | ✅ |
| seg1 | `…/seg1.ts` | `fdda4a7f4a9a62d68930090bd29a96a5` | ✅ |
| seg2 | `…/seg2.ts` | `4528cb59b78721cf5cba897d6b744f03` | ✅ |

- 佐证：`c1` 与 `c2` 共用同一上游，`<S>` 相同 → 只依赖切片，与频道无关；重启后 `<S>` 不变 → **确定性**（无进程内随机盐）。

### 5.3 `<P>` 未破解 ⏳（低影响）
- 16 hex（64 bit）；**每频道固定**；重启不变；与请求 `Host` 无关；仅依赖 `(gid,cid)`。
- 已排除（对 `(gid,cid)` 的拼接/元组/路径形态）：`md5`/`sha1`/`sha256`/`sha512`/`sha3_256`/`blake2b`/`blake2s` 的任意位置 16 hex、字节反序、`[:16]`；`xxh64`/`xxh3`/`xxh32`、`siphash13`/`siphash24`（含 Rust `DefaultHasher` 的 `\xff` 结尾编码）、`seahash`、`mmh3`(64/128)、`fnv1a64`/`fnv1_64`、`djb2`、`fxhash`；XOR/拆分/哈希之哈希、URL/基路径、token 组合。
- 二进制常量：`ahash` PI 常量、`xxh64`/`wyhash`/`murmur` 常量**未出现**；`siphash` 标准键（`somepseudorandomlygeneratedbytes`）**出现**（用途未定）。
- **推断**：为 `(gid,cid)` 的**键控/加盐哈希**，或非常见算法的 64-bit 哈希；**待反编译确认**（S7）。
- **功能影响低**：切片 URL 由清单下发，客户端无需推导 `<P>`；重建时用**任意确定性 `f(gid,cid)`** 即可保持等价行为。已知样例：`test1/c1 → ea8680a1b61c17e4`、`test1/c2 → 8cb3e469c0dd0aa3`、`g2/c1 → 5ae12330d3e32594`；**动态预言机**：对任意 `(gid,cid)` 拉一次清单即可读出 `<P>`。

### 5.4 后缀不参与校验（S-DIS-05 行为实证 ✅）
同一切片 ID，换后缀取回**完全相同字节**：

| 后缀 | code | `Content-Type` | sha256[:16] |
| --- | --- | --- | --- |
| `.jpg` | 200 | `image/jpeg` | `da8d1496216a40a4` |
| `.png` | 200 | `image/png` | `da8d1496216a40a4` |
| `.webp` | 200 | `image/webp` | `da8d1496216a40a4` |
| `.gif` | 200 | `image/gif` | `da8d1496216a40a4` |
| `.bmp` | 200 | `image/bmp` | `da8d1496216a40a4` |
| `.txt`/`.ts`/`.m3u8` | 200 | `video/mp2t` | `da8d1496216a40a4` |

- **载荷不改**：字节 = 上游切片原字节（长度 1880 与上游一致）。S-DIS-02 相符。
- **`Content-Type` 由后缀映射**（图片后缀→图片类型；其余→`video/mp2t`）。无效 `<P>` 或 `<S>`（错 16/32 hex）→ **302**。

### 5.5 响应头

| 资源 | `Content-Type` | `Cache-Control` |
| --- | --- | --- |
| 频道入口清单 | `text/plain; charset=utf-8`（伪装） | `public, max-age=5, s-maxage=5` |
| 切片 | 由后缀映射 | `private, max-age=300` |
| 分组订阅 `.m3u` | `application/x-mpegurl` + `Content-Disposition` | — |

### 5.6 主清单 / 媒体清单伪装（S-DIS-10 ✅）
上游为 **master**（`#EXT-X-STREAM-INF` → 变体）时：

| 资源 | 路径 | `Content-Type` | `Cache-Control` |
| --- | --- | --- | --- |
| 主清单 | `/{gid}/{cid}.m3u8` | `application/vnd.apple.mpegurl` | `no-cache, no-store` |
| 媒体清单 | `/{gid}/{cid}/video/index.txt` | `text/plain; charset=utf-8` | `public, max-age=5, s-maxage=5` |
| 切片 | `/{gid}/{cid}/<16hex>_<32hex>.<ext>` | 由后缀映射 | 首写 `public`→命中 `private` |

- 媒体清单**伪装为 `video/index.txt`**，入口/主清单仍为 `.m3u8` → 与默认（`playlist=true`、`master=false`）一致。
- 主清单被重写并**规范化**（追加 `#EXT-X-INDEPENDENT-SEGMENTS`、升到 `#EXT-X-VERSION:6`）。
- 媒体清单**无 `#EXT-X-MAP`**（本例上游无）。上游为**媒体清单**（无 master）时，入口 `/{gid}/{cid}.m3u8` 即媒体清单，`text/plain`（见 §5.1）。

## 6. 缓存与预取（实测）
- 上游 3 个切片，目标只预取 **2 个**（`seg0`、`seg1`）→ 与 `[cache.prefetch].skip_tail = 1`（默认跳过尾切片）一致。【推断】频道入口 `max-age=5`（≈ 刷新周期）。

## 7. 持久化与未知频道
- 面板改动**落盘 `config.toml`**：新增分组/频道后，文件出现 `[[groups]]`、`[[groups.channels]]` + 全字段（S-CFG-01 相符）。
- 未知 `/{gid}/{cid}` → **302** 到 `server.not_found_url`（默认 `https://material.yang-1989.eu.org/v/404.mp4`）；另有 `error_url`=`.../mistake.mp4`。`auth` 另有 `token_failure_url`、`limit_url`、`ban_url` 占位视频。

## 8. 未决 / 局限
- **`<P>`（16 hex）推导未解**（§5.3）。
- `skip_tail` 与“只取 2 片”的因果**未逐值验证**（仅 1 组配置）；`Cache-Control` 的 `public→private` 时间窗（基线 S-DIS-06）**未做时序采样**。
- 未覆盖：六模式差异、DASH、回看、EPG、台标、录制、Token/IP 管控、伪装时序。
- 仅 **arm64 + 单容器 + 单配置**；跨架构未验证。

## 9. 复现

```bash
# 见 PROGRESS_bak.md §3：容器 iptv82 + 宿主假源(9999) + 面板 API 建组/建频道
T=$(curl -s -X POST http://127.0.0.1:19899/panel/api/login \
      -H 'Content-Type: application/json' -d '{"username":"admin","password":"admin"}' \
    | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
curl -s -X POST http://127.0.0.1:19899/panel/api/groups -H "Authorization: Bearer $T" \
      -H 'Content-Type: application/json' -d '{"id":"test1"}'
curl -s -X POST http://127.0.0.1:19899/panel/api/groups/test1/channels -H "Authorization: Bearer $T" \
      -H 'Content-Type: application/json' -d '{"id":"c1","name":"C1","url":"http://host.docker.internal:9999/x.m3u8"}'
curl -s http://127.0.0.1:19899/test1/c1.m3u8

```
