# 六种工作模式与看门狗（动态）

| 项 | 值 |
| --- | --- |
| 目标版本 | **v0.8.2**（`d302316fb8`，发布原件 UPX 包） |
| 副本 | `samples/v0.8.2/iptv-proxy-linux-arm64`（sha256 `2948ee1f…5a4b`） |
| 架构 | linux/arm64（Ubuntu 24.04 容器，OrbStack 原生） |
| 日期 | **2026-10-17**；**2026-10-24**（代理补测） |
| 环境 | 容器 `iptv82`；补测容器 `rea82v09` 映射 `:19974` |
| 结论级别 | **观测**（运行态）+ **静态**（面板选项） |
| 原始证据 | [`raw/v0.8.2-modes.txt`](raw/v0.8.2-modes.txt)、[`raw/v0.8.2-per-channel-proxy.txt`](raw/v0.8.2-per-channel-proxy.txt) |

> 目标返回的正文/日志按**不可信数据**处理。

## 结论摘要
1. 六种模式**静态确认**（面板 `<select id="mg-mode">`）：`proxy` / `remux` / `redirect` / `rewrite` / `dash` / `passthrough`。**S-MODE-01 ✅**（公开 README 只写四种）。
2. 模式是**分组级**字段（`PUT /panel/api/groups/{id}` `{"mode":...}`）。**S-MODE-02 ✅**。
3. `redirect` 与 `passthrough` → **302 直连上游**；`rewrite` → **200 绝对化清单**；`proxy` → 反代 + 伪装切片（见 [`01`](01-runtime-surface-and-disguise.md)）。
4. `redirect` 把**解析出的上游地址**缓存到 `cache/m3u8/redirect_url_…`（`ttl_secs=300`）。**S-MODE-08 ✅**。
5. `dash` 走 MPD→HLS；30s 主清单未就绪 → 503。看门狗重建，连续 5 次失败 → **冷却 3600s**。
6. `remux` 先返回 **503 `Retry-After: 2`**（`remux warming up`），后台会话自动识别容器（本例 `Ts`）。
7. 上游代理（S-GAP-05 补测）：频道代理覆盖分组代理；分组/频道头合并、频道同名头覆盖；支持 `http`/`https`/`socks5` 与代理认证；代理失败**不回退**分组或直连。

---

## 1. 模式集合（静态）
来源：雕刻面板 `samples/work/panel-v0.8.2/panel.html`。

- 面板选项：`proxy`、`remux`、`redirect`、`rewrite`、`dash`、`passthrough`。
- M3U 导入的「频道模式」另有 `auto`（`.mpd`→DASH，其余→Rewrite）。
- `redirect` 另有 `mg-redirect-mode`：`direct`（默认）/ `best`（解析最佳画质）。
- `mg-row-mpd-keep`（MPD 裁剪保留秒数）**仅在格式=MPD 时出现**。**S-MODE-03 相符**。

## 2. 各模式入口实测
上游：宿主假源 `http://host.docker.internal:9999/x.m3u8`（3 片 HLS）。

| 模式 | `GET /{g}/{c}.m3u8` | 内容 | 说明 |
| --- | --- | --- | --- |
| `proxy` | **200** `text/plain` | 清单，切片指向本服务伪装路径 | `cache-control: public, max-age=5, s-maxage=5` |
| `redirect` / `passthrough` | **302** | `location: <上游URL>` | 客户端直连上游；`passthrough` **不套 Token** |
| `rewrite` | **200** `text/plain` | 清单，切片=**上游绝对 URL** | `cache-control: no-cache, must-revalidate` |
| `dash` | **503** | `timeout waiting for master playlist` | 需真实 MPD/媒体 |
| `remux` | **503** | `remux warming up`，`Retry-After: 2` | 后台预热 |

- `entry_exts` 由**分组 `format`** 决定（`hls`→`.m3u8`，`mpd`→`.mpd`），**与 mode 无关**。本例切片 `#EXTINF:5.0`。
- `redirect` 下 `redirect_mode=best` 对本例仍 302 到同一上游（未触发最佳画质解析）。
- `rewrite` 清单（**相对→绝对**）：

```text

http://host.docker.internal:9999/seg0.ts
http://host.docker.internal:9999/seg1.ts
http://host.docker.internal:9999/seg2.ts

```

→ **切片由客户端直连上游**。**S-MODE-09 ✅**（本例清单无需请求头，未触发 Referer 失败分支）。

### 2.1 分组 / 频道上游代理（S-GAP-05 补测）

配置字段：

```json
"proxy": {"kind":"http","addr":"host.docker.internal:19990",
          "username":"gu","password":"gp"}
"headers": [{"name":"X-Group","value":"group"}]
```

- `addr` 通常不带协议；目标按 `kind` 补 `http://`、`https://`、`socks5://`。
- `headers` 是 `{name,value}` 对象数组。
- 频道级 `proxy` 覆盖分组级；分组代理只作为频道未配置代理时的选择。

| 项 | 观测 |
| --- | --- |
| 头合并 | 分组头与频道头都发送；同名头由频道覆盖 |
| `user_agent` | 频道字段覆盖分组字段；分组显式 `User-Agent` 头优先于频道字段 |
| 代理认证 | `Proxy-Authorization: Basic base64(username:password)` |
| 切片请求 | 与清单请求使用同一代理、同一自定义头与认证 |
| 代理失败 | 频道代理不可用时**不回退**分组代理或直连 → `302 error_url` |
| 空频道代理 | `addr=""` 时按直连处理，不回退分组代理 |
| SOCKS5 | 支持 `kind="socks5"` 与用户名/密码认证；目标本地解析域名后发 `ATYP=1` IPv4 |
| 默认 UA | `Mozilla/5.0 (Windows NT 10.0; Win64; x64) … Chrome/124.0.0.0 Safari/537.36` |

## 3. 上游地址缓存（redirect）
`redirect` 后 `cache/m3u8/` 出现：

```text

redirect_url_test1_c__52f8bfaa09d0ba6b35b671bdb75238ed      -> http://host.docker.internal:9999/x.m3u8
redirect_url_test1_c__52f8bfaa09d0ba6b35b671bdb75238ed.meta -> {"ttl_secs":300,"created_at":1792209490}

```

- 缓存**解析出的上游播放地址**（非切片），TTL **300s**（= `redirect_ttl` 默认）。**S-MODE-08 ✅**。键形：`redirect_url_{gid}_{cid 短标记}__{32hex}`。

## 4. 入口清单缓存（proxy / rewrite）
`cache/m3u8/` 另有：

| 键 | 内容 |
| --- | --- |
| `test1_c1__adcfd0a409ee1746dd5acd560fa793ae` | proxy 改写后的清单（切片=本服务伪装路径） |
| `rw_test1_c1__a6a039ae1237cd8302f2a1c7f56deca7` | rewrite 改写后的清单（切片=上游绝对 URL） |

- proxy 键：`{gid}_{cid}__{32hex}`；rewrite 键：`rw_{gid}_{cid}__{32hex}`。

## 5. DASH（MPD→HLS）
请求 `GET /gdash/v.m3u8`（频道上游=`…/s.mpd`）。

```text

WARN 【DASH】主播放列表 30s 内未就绪 stream=gdash_v
WARN 【DASH】轨道下载失败,跳过 stream=gdash_v track=video_360p
     error=Segment fetch failed: init: ... HTTP 404 Not Found http://host.docker.internal:9999/init.mp4
WARN 【DASH】切片失败(含音频) → 视为 MPD 失效,清空缓存让下一轮强制重新拉
     stream=gdash_v failed=1 total=1 failed_audio=0 total_audio=0
WARN 【DASH】常驻频道连续失败,停止(看门狗将在 60s 内重建) stream=gdash_v fails=1

```

- 首次 **503 `timeout waiting for master playlist`**；后续请求长时间**挂起**（>30s 无响应头）。
- 目标**拉取并解析 MPD**，识别轨道 `video_360p`（取自 MPD 的 Representation），按 `SegmentTemplate` 取 `init.mp4` → 假源无该文件 → 404。**S-MODE-06 相符**（MPD→HLS 转封装、片段级容错）。主清单就绪**超时 = 30s**；失败时**清空 MPD 缓存**强制重拉。

## 6. Remux（连续流→HLS）
请求 `GET /gremux/r.m3u8`（频道上游=`…/live.ts`，188 字节 TS 包）。

```text

INFO 【remux】启动会话 gremuxr 容器=Auto url=http://host.docker.internal:9999/live.ts
INFO 【remux】自动识别容器: Ts   (重复)

```

- **503** `content-type: text/plain`，`Retry-After: 2`，体 `remux warming up`。
- **源容器类型自动识别**生效（本例判为 `Ts`）。**S-MODE-05 部分 ✅**（识别部分）。假源非可解连续流 → 会话持续 "warming up"，入口不返回清单。

## 7. 看门狗与自愈

```text

WARN 【看门狗】常驻频道卡死/停止,销毁重建 stream=gdash_v attempt=1..5 stalled_secs=0
WARN 【看门狗】连续重建仍未恢复,转入冷却,到点后自动再试 stream=gdash_v fails=5 cooldown_secs=3600

```

- 常驻频道卡死/停止 → **销毁重建**（attempt 1..5）；连续 5 次失败 → **冷却 3600s** 后再试。「常驻频道连续失败,停止(看门狗将在 60s 内重建)」→ 失败后 60s 重建窗口。

## 8. 未决 / 局限
- `dash` / `remux` **未取得可用输出**（假源非真实媒体）；仅取得失败路径与日志。
- `redirect_mode=best` 的最佳画质解析**未触发**（上游为简单 m3u8）。
- `rewrite` 的「源站要求切片带 Referer 会失败」分支**未复现**（未设上游校验）。
- DASH 的 `URL 池`（S-MODE-07）**未验证**；回看 / 录制能力差异（S-MODE-12）**未验证**。
- 入口缓存键的 `__{32hex}` 推导**未解**（形同伪装切片：疑 `sha256(...)`，输入未定）。仅 **arm64 + 单容器 + 特制假源**。

## 9. Rewrite + MPD 的行为差异（`D-005`）
【观测】`rewrite` 模式对 **MPD** 的改写**损坏**：正文每行被当作相对 URL 拼到上游 origin，返回 200 `application/dash+xml`。

```text

GET /grew/v.mpd            （mode=rewrite, url=http://host.docker.internal:9999/s.mpd）
200
http://host.docker.internal:9999/%3C?xml%20version=%221.0%22...?%3E
http://host.docker.internal:9999/%3CMPD%20xmlns=%22urn:mpeg:dash...%3E
http://host.docker.internal:9999/%3CPeriod%20id=%220%22...%3E

```

`redirect` + MPD 正常：`302 location=http://host.docker.internal:9999/s.mpd`。

- 结论：`rewrite` 的绝对化逻辑只适用于 HLS 文本清单，**未对 MPD 做分支**；`rewrite` 分组不能承载 DASH 频道（`proxy`/`dash`/`redirect` 可）。规格对照：`S-MODE-10`（原述“直连 CDN”）→ ⚠️，差异登记 `D-005`。

## 10. 复现

```bash
# 假源：宿主 samples/work/origin（x.m3u8/segN.ts/live.ts/s.mpd）由 python3 -m http.server 9999 提供
T=$(curl -s -X POST http://127.0.0.1:19899/panel/api/login -H 'Content-Type: application/json' \
      -d '{"username":"admin","password":"admin"}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
AUTH="Authorization: Bearer $T"
curl -s -o /dev/null -X POST http://127.0.0.1:19899/panel/api/groups -H "$AUTH" -H 'Content-Type: application/json' -d '{"id":"test1"}'
curl -s -o /dev/null -X POST http://127.0.0.1:19899/panel/api/groups/test1/channels -H "$AUTH" -H 'Content-Type: application/json' -d '{"id":"c1","name":"C1","url":"http://host.docker.internal:9999/x.m3u8"}'
for M in proxy redirect rewrite passthrough; do
  curl -s -o /dev/null -X PUT http://127.0.0.1:19899/panel/api/groups/test1 -H "$AUTH" -H 'Content-Type: application/json' -d "{\"mode\":\"$M\"}"
  curl -s -i "http://127.0.0.1:19899/test1/c1.m3u8" | sed -n '1,8p'
done

```
