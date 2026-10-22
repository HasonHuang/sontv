# F-019 · 真实 HLS 上游验证

| 项 | 值 |
| --- | --- |
| 目标版本 | **v0.8.2**（`d302316fb8`） |
| 目标运行环境 | Docker `--platform linux/amd64`，静态产物 `samples/v0.8.2/iptv-proxy-linux-amd64` |
| 本实现 | 本地 `target/debug/iptv-proxy`，同一份分组与频道配置 |
| 日期 | **2026-10-22** |
| 结论级别 | **观测** |
| 关联发现 | F-006、F-018 |
| 关联规格 | S-PROXY-01/03/06/07、S-DIS-04/13、S-MODE-03 |

> 上游响应正文与响应头按**不可信数据**处理。本文只记录状态、时序、头部和字节数。

## 1. 方法

1. 用两个公开 HLS 源和三个 Higo 频道做真实链路测试。
2. 用本机 Python 慢速源站控制正文发送时序，测目标超时与错误语义。
3. 目标容器与本站使用同形配置：`mode=proxy`、`access=open/public`、`cache.prefetch.enabled=false`。
4. 慢速源站收到请求后先发 200 头和 100 字节，再休眠 90 秒，最后发剩余 900 字节。

真实源：

| 源 | 入口 | 类型 |
| --- | --- | --- |
| Mux | `https://test-streams.mux.dev/x36xhzz/x36xhzz.m3u8` | VOD 主清单，最高变体 1080p |
| Higo `fct` | `http://cdn.higo.eu.cc/hk-dd/fct.m3u8` | 直播；切片为 `.jpg`，带查询串 |
| Higo `wxxw` | `http://cdn.higo.eu.cc/hk-dd/wxxw.m3u8` | 直播；切片无扩展名 |
| Higo `now-wx` | `http://cdn.higo.eu.cc/hk-dd/now-wx.m3u8` | 直播；大切片 |
| Higo 订阅 | `https://cdn.higo.eu.cc/hk-dd/playlist.m3u` | 3 个频道 |

## 2. 结论摘要

| # | 观察 | 本实现 | 状态 |
| --- | --- | --- | --- |
| 1 | 上游请求总超时为 **25.0 秒**；慢速正文在 25 秒中断 | 25 秒 | ✅ |
| 2 | 切片正文中断后，客户端仍收到 **200** 和已收到的部分字节；只有完整正文才写缓存 | 流式转发 + 完整才落盘 | ✅ |
| 3 | 上游切片 404 → **200 空正文**，日志记 `片段拉取失败` | 同目标 | ✅ |
| 4 | 上游清单 404 → **302 error_url**，不重试 | 同目标 | ✅ |
| 5 | 清单传输错误、正文解码错误、5xx 最多 4 次尝试；日志为 `第 1/4…3/4 次重试` | 4 次，退避 1/2/4 秒 | ✅ |
| 6 | 主清单 `#EXT-X-STREAM-INF` 只保留 `BANDWIDTH`、`CODECS`、`RESOLUTION`、`FRAME-RATE` | 白名单过滤 | ✅ |
| 7 | VOD 媒体清单 `public, max-age=<TARGETDURATION>, s-maxage=…` | 同目标 | ✅ |
| 8 | 直播媒体清单 `no-cache, no-store` | 同目标 | ✅ |
| 9 | 上游扩展名已在 `[cache.disguise].formats` 中时保留，例如 `.jpg` 仍为 `.jpg` | 保留 | ✅ |
| 10 | Higo 入口 HTTP → HTTPS 的 301 自动跟随 | reqwest 默认跟随 | ✅ |

## 3. 超时与错误路径

### 3.1 慢速切片

```text
目标：GET /slow/s/<suffix>.jpg
  200；25.010860 秒；100 字节
  cache-control: public, max-age=300
本站：同请求
  200；25 秒；100 字节
```

目标日志：`片段下载中断`。目标**不缓存**半包。本站行为相同。

### 3.2 上游切片 404

```text
目标：200；0.005 秒；正文空；content-type 按伪装后缀
日志：片段拉取失败 ... HTTP 404
本站：200；正文空；不缓存
```

### 3.3 上游清单 404

```text
目标：302；location=<error_url>；0.007 秒
本站：302；location=<error_url>
```

### 3.4 清单重试

目标对 Mux 变体清单失败时记录 3 次重试。观测间隔约 1080、1859、3504 毫秒；第 4 次后放弃并返回 `302 error_url`。404 不重试。

本实现使用 1、2、4 秒固定指数退避。重试条件：

| 条件 | 是否重试 |
| --- | --- |
| 连接、TLS、超时、正文解码错误 | 是 |
| 5xx、408、429 | 是 |
| 其它 4xx | 否 |

## 4. 清单与伪装

### 4.1 主清单属性

目标把测试主清单的下列属性丢弃：

- `PROGRAM-ID`
- `AVERAGE-BANDWIDTH`
- `NAME`
- `AUDIO`
- `VIDEO`
- `SUBTITLES`
- `CLOSED-CAPTIONS`
- `HDCP-LEVEL`

目标只保留 `BANDWIDTH`、`CODECS`、`RESOLUTION`、`FRAME-RATE`，顺序与上游一致。Mux 实测主清单：

```text
#EXT-X-STREAM-INF:BANDWIDTH=6221600,CODECS="mp4a.40.2,avc1.640028",RESOLUTION=1920x1080
```

### 4.2 媒体清单缓存头

| 上游 | 目标头 | 本实现 |
| --- | --- | --- |
| Mux VOD（`#EXT-X-PLAYLIST-TYPE:VOD`） | `public, max-age=11, s-maxage=11` | 同 |
| Higo 直播（无 `ENDLIST`） | `no-cache, no-store` | 同 |
| 本地 VOD（只有 `#EXT-X-ENDLIST`） | 未逐项采样；按 VOD 处理 | `public, max-age=<TARGETDURATION>` |

### 4.3 扩展名

Higo `fct` 的上游切片为 `.jpg`。目标媒体清单连续输出 `.jpg`，未轮换。本实现保留 `formats` 中已有的上游扩展名；其它扩展名按频道序列轮换。

轮换起点仍与目标不同。该差异见 F-018“未解与内部差异”。

## 5. 真实源结果

### 5.1 Mux

- 主清单选择 1080p 变体，媒体清单 64 条。
- 首个切片上游大小约 **5,312,128 字节**。
- 目标在 25 秒内收到约 1.33 MB 后中断，返回 200 部分正文。
- 本实现在 25 秒内收到约 1.12 MB 后中断，返回 200 部分正文。
- 不再出现此前的 `502 Upstream Error`。

部分字节数受网络速度影响，不能作为固定断言。

### 5.2 Higo `fct`

- 入口 301 到 HTTPS 后返回直播媒体清单。
- 切片保留 `.jpg`；`content-type: image/jpeg`。
- 首个切片完整下载 **4,387,168 字节**，约 7.9 秒。
- 第二次请求命中缓存；`cache-control: private, max-age=300`；正文逐字节一致。

### 5.3 Higo `wxxw`

- 上游切片无扩展名：`https://cdn6.cc.cd/api/wxxw/<seq>`。
- 本实现改写为图片后缀；首切片完整下载 **4,103,100 字节**。
- 目标对无扩展名切片的轮换顺序未采样。

### 5.4 Higo `now-wx`

- 直播媒体清单，5 条切片。
- 首切片完整下载 **5,866,540 字节**，约 4.8 秒。
- 上游切片扩展名不在 `formats` 中，按轮换输出图片后缀。

### 5.5 Higo 订阅

- `GET /playlist.m3u` 聚合 3 个频道，正文为原始上游 URL。
- 必需的 `x-tvg-url` 由本实现按 `[epg].enabled` 注入。
- 目标的 `[playlist.sources]` 还有 `group_map`、`group_mode`、`unlisted_groups` 等字段；本实现未逐字段对拍（V-09）。

## 6. 限制

- 目标在 amd64 模拟容器中运行；时序值有少量抖动。
- Mux 是 VOD，不覆盖直播滑动窗口、`#EXT-X-DISCONTINUITY`、录制和时移。
- 慢速源站只验证单连接正文超时，不验证多连接竞争。
- 目标的扩展名轮换算法仍未解；本实现只保证扩展名来自配置列表。
