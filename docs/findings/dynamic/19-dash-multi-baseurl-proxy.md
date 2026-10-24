# DASH 多 `<BaseURL>` 与 proxy MPD 反代（动态）

| 项 | 值 |
| --- | --- |
| 目标版本 | **v0.8.2**（`d302316fb8`） |
| 副本 | `samples/v0.8.2/iptv-proxy-linux-amd64` |
| 架构 | linux/amd64（OrbStack 容器） |
| 日期 | **2026-10-24** |
| 环境 | 自建多 BaseURL MPD；目标 `dash` 与 `proxy` 分组 |
| 结论级别 | **观测** |

> 目标返回的正文、清单与日志按**不可信数据**处理。本文只记录观测结果。

## 结论摘要

1. `dash` 模式、频道 `url_pool_size=1` 时，多个 `<BaseURL>` 全部请求走**第一个**地址；音视频都如此。
2. `proxy` 模式会把 **绝对、相对、缺失** 三种 `<BaseURL>` 都改写成本服务的 MPD 反代地址；绝对地址不会直连。
3. 本地反代路径形如：

```text
/{gid}/{cid}/mpd/{32位十六进制}/{Base64URL(原始BaseURL)}/
```

4. 本地切片路径由 Base64URL 解码还原上游 BaseURL，再拼接相对切片路径；错误令牌返回 **302 到 `server.error_url`**。
5. proxy MPD 会把已有 `minimumUpdatePeriod` 改为 `PT1S`，把已有 `publishTime` 改为当前 UTC 时间；缺失的属性不插入。
6. proxy MPD 响应头：`content-type: application/dash+xml; charset=utf-8`，`cache-control: public, max-age=1, s-maxage=1`。

上述 4–6 已在本实现对齐。

## 1. DASH 多 `<BaseURL>`

MPD：

```xml
<BaseURL>http://host.docker.internal:9995/a/</BaseURL>
<BaseURL>http://host.docker.internal:9995/b/</BaseURL>
```

频道：`dash` 模式，`url_pool_size=1`。目标请求统计：

| 轨道 | `/a/` | `/b/` |
| --- | ---: | ---: |
| init | 1 | 0 |
| video 切片 | 12 | 0 |
| audio 切片 | 12 | 0 |

- 目标先请求 MPD 多次，再一次性下载 init 与 12 个切片。
- 没有观察到第二个 BaseURL 的请求。
- `url_pool_size>1` 的实验因目标 URL 池自身失败，未得到有效样本；面板自述的音频轮询规则仍未独立验证。

## 2. proxy 模式 BaseURL 改写

### 2.1 绝对 BaseURL

上游：

```xml
<BaseURL>http://host.docker.internal:9995/a/</BaseURL>
```

目标 `proxy` 出口：

```xml
<BaseURL>http://127.0.0.1:19968/gproxy/v/mpd/b8e6b8c9fa52ce2faba99adb5c692507/aHR0cDovL2hvc3QuZG9ja2VyLmludGVybmFsOjk5OTUvYS8/</BaseURL>
```

- 绝对 BaseURL 被替换，没有任何直连上游的 BaseURL 保留。
- Base64URL 解码：`http://host.docker.internal:9995/a/`。
- 32 位十六进制令牌在同一进程内稳定；换端口/进程后变化，推测为内部缓存键或令牌。

### 2.2 相对 BaseURL

上游：

```xml
<BaseURL>a/</BaseURL>
```

MPD URL：`http://host.docker.internal:9995/rel.mpd`。

目标出口解码后：

```text
http://host.docker.internal:9995/a/
```

即先按 MPD URL 解析相对地址，再本地反代。

### 2.3 缺失 BaseURL

上游没有 `<BaseURL>`。目标在 `<MPD>` 后插入：

```xml
<BaseURL>http://127.0.0.1:19969/gproxy/vno/mpd/d12e0a68ee923ecd06ddd2bc6533b4ac/aHR0cDovL2hvc3QuZG9ja2VyLmludGVybmFsOjk5OTUv/</BaseURL>
```

解码后为 MPD 所在目录：`http://host.docker.internal:9995/`。

### 2.4 多个 BaseURL

`proxy` 模式下两个绝对 BaseURL 都被改写成各自带令牌的本地地址：

```xml
<BaseURL>…/vpool/mpd/28b4c81f…/aHR0…L2Ev/</BaseURL>
<BaseURL>…/vpool/mpd/214d31e7…/aHR0…L2Iv/</BaseURL>
```

### 2.5 本地切片验证

取目标出口 BaseURL，追加 `chunk-0-1.m4s?name=v02`：

```text
GET /gproxy/v/mpd/{token}/{base64}/chunk-0-1.m4s?name=v02
→ 200 150146 字节
→ 上游收到 http://host.docker.internal:9995/a/chunk-0-1.m4s?name=v02
```

错误令牌：

```text
GET /gproxy/v/mpd/00000000000000000000000000000000/{base64}/chunk-0-1.m4s
→ 302
location: https://material.yang-1989.eu.org/v/mistake.mp4   （server.error_url）
```

### 2.6 MPD 响应头与动态属性

同一请求的响应头：

```text
content-type: application/dash+xml; charset=utf-8
cache-control: public, max-age=1, s-maxage=1
```

MPD 根属性变化：

```text
上游：minimumUpdatePeriod="PT2.0S" publishTime="2026-10-24T00:00:00Z"
目标：minimumUpdatePeriod="PT1S"   publishTime="2026-10-24T07:33:26Z"
```

- 连续两次请求（间隔 1.1 秒）的 `publishTime` 各为请求时刻，确认每次刷新。
- 去掉上游 `publishTime` 后，目标输出也不含该属性，确认只改已有值。

## 3. 实现映射

| 项 | 实现 |
| --- | --- |
| 绝对 BaseURL | 改为本地反代，不再直连 |
| 相对 BaseURL | 按 MPD URL 解析后本地反代 |
| 缺失 BaseURL | 用 MPD 所在目录插入本地反代 |
| 本地令牌 | `sha256("rea-iptv/mpd-v1\0gid\0cid\0base")` 前 16 字节，输出 32 位十六进制 |
| 本地路径 | `/{gid}/{cid}/mpd/{token}/{Base64URL(base)}/` |
| 切片还原 | 路径首段校验令牌，Base64URL 解码出 BaseURL，再拼相对路径 |
| 多 BaseURL | 每个节点独立改写 |
| 错误令牌 | 302 到 `server.error_url` |
| `minimumUpdatePeriod` | 已有属性统一改为 `PT1S` |
| `publishTime` | 已有属性改为当前 UTC；缺失不插入 |
| MPD 响应头 | `application/dash+xml; charset=utf-8`；`max-age=1, s-maxage=1` |
