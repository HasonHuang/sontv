# 公开 EPG API 形状（动态）

| 项 | 值 |
| --- | --- |
| 编号 | F-023 |
| 目标版本 | **v0.8.2**（`d302316fb8`） |
| 副本 | `samples/v0.8.2/iptv-proxy-linux-amd64` |
| 架构 | linux/amd64（OrbStack 容器） |
| 日期 | **2026-10-24** |
| 环境 | 本地 XMLTV 源；目标 `rea82v09`（127.0.0.1:19974） |
| 结论级别 | **观测** |
| 原始证据 | [`raw/v0.8.2-public-epg-api.txt`](raw/v0.8.2-public-epg-api.txt) |

> 目标返回的正文按**不可信数据**处理。本文只记录字段与行为，不复制目标前端代码或文案。

## 结论摘要

1. `/epg/api/channels` 返回本地已匹配频道。字段为 `icon/id/kind/last_stop/name/programme_count/source_id/source_name`。
2. `/epg/api/programmes/{channel}` 只返回 `desc/start/stop/title`。`start`/`stop` 为 Unix 秒。
3. `/epg/api/search?q=` 同时返回频道与节目。节目对象含 `channel/channel_name/start/stop/title`。
4. 搜索大小写不敏感；范围为频道 `id`/`name` 与节目标题；**不含简介**；按 `start` 升序；最多 200 条。
5. 频道 ID 统一小写。节目路径大小写敏感。
6. 关闭 EPG 或公开查看器时，三个公开 API 均返回 `404` JSON 错误。
7. 响应带宽松 CORS 头；三个端点分别使用 `max-age=300`、`120`、`30`。

## 1. `/epg/api/channels`

```json
{
  "channels": [
    {
      "icon": "http://host.docker.internal:19981/logo-c1.png",
      "id": "c1",
      "kind": "local",
      "last_stop": 1792836480,
      "name": "C1",
      "programme_count": 3,
      "source_id": "src_local",
      "source_name": "Local XMLTV"
    }
  ],
  "generated_at": 1792818512,
  "total": 1,
  "tz_offset_hours": 8
}
```

字段规则：

| 字段 | 规则 |
| --- | --- |
| `id` | 本地输出 ID 的小写形式 |
| `name` | 本地频道名；不是 XMLTV `display-name` |
| `kind` | 本地频道为 `local` |
| `icon` | 源 XMLTV 图标 URL；无图标为 `null` |
| `last_stop` | 该频道节目最大 `stop` 的 Unix 秒；无节目为 `0` |
| `programme_count` | 保留节目数；可为 `0` |
| `source_id` / `source_name` | 命中的 EPG 源 |
| `generated_at` | 最近一次生成时间（Unix 秒） |
| `total` | 频道数 |
| `tz_offset_hours` | `[server].timezone_offset_hours` |

已匹配但无节目的频道仍出现。额外频道不出现在公开频道表。

面板 `GET /panel/api/epg/viewer-channels` 只返回 `{"channels":[…]}`，不带上表的 `generated_at`、`total`、`tz_offset_hours`。

## 2. `/epg/api/programmes/{channel}`

```json
{
  "channel": "c1",
  "programmes": [
    {
      "desc": "C1 新闻描述 & 测试",
      "start": 1792814880,
      "stop": 1792822080,
      "title": "C1 早间新闻"
    }
  ],
  "tz_offset_hours": 8
}
```

- `channel` 回显请求路径原文。
- 路径大小写敏感：`C1` 不命中 `c1`，返回空数组，但 `channel` 仍为 `C1`。
- 没有 `count`、`categories`、逐条 `channel`。
- 查询参数无效。

## 3. `/epg/api/search?q=`

```json
{
  "channels": [
    {
      "icon": null,
      "id": "c2",
      "kind": "local",
      "last_stop": 1792823940,
      "name": "C2",
      "programme_count": 1,
      "source_id": "src_local",
      "source_name": "Local XMLTV"
    }
  ],
  "programmes": [
    {
      "channel": "c2",
      "channel_name": "C2",
      "start": 1792816680,
      "stop": 1792823880,
      "title": "C2 Show"
    }
  ],
  "tz_offset_hours": 8
}
```

规则：

| 项 | 规则 |
| --- | --- |
| 空 `q` / 缺 `q` | 两个数组为空 |
| 大小写 | 不敏感 |
| 频道匹配 | `id` 或 `name` 包含查询串 |
| 节目匹配 | 标题包含查询串；**不匹配简介** |
| 排序 | 按 `start` 升序 |
| 上限 | 节目最多 200 条；频道不截断 |
| 字段 | 无 `q`、无 `count`、无节目 `desc` |

有一项证据缺口：目标为何按 `start` 排序、是否稳定排序未反编译确认；只确认输出顺序。

## 4. 关闭态与错误

| 条件 | 状态 | 正文 |
| --- | --- | --- |
| `[epg].enabled=false` | `404` | `{"error":"EPG 功能已关闭"}` |
| `public_viewer_enabled=false` | `404` | `{"error":"公开节目单查看器已关闭"}` |

三个公开 API 行为一致。`/epg` 页面同时为 `404`，但正文为 HTML。

## 5. 响应头

| 端点 | `cache-control` |
| --- | --- |
| `channels` | `public, max-age=300` |
| `programmes/{channel}` | `public, max-age=120` |
| `search` | `public, max-age=30` |

共同头：

```http
vary: origin, access-control-request-method, access-control-request-headers
vary: accept-encoding
access-control-allow-origin: *
access-control-expose-headers: *
```

预检返回 `200`，并带 `access-control-allow-methods: *`、`access-control-allow-headers: *`。

## 6. 实现映射

| 目标行为 | 本实现 |
| --- | --- |
| 公开频道表含源信息 | 生成时写 `<cache>/epg/viewer.json`；API 读取该快照 |
| 重启后仍可查表 | 快照落盘；缺失时从 `epg.xml` 反推最小字段 |
| ID 小写 | `channel_targets`、额外频道与公开表统一小写 |
| 节目/搜索 Unix 秒 | `epg::parse_time` 转秒 |
| 搜索不含简介、最多 200、按开始时间 | `api_search` 只匹配标题并排序截断 |
| 关闭态 JSON 错误 | `public_ready` 返回对应 JSON |
| 宽松 CORS | 全局 `CorsLayer::permissive()` |

实现文件：

- `src/epg.rs`：`ViewerData`、`ViewerChannel`、`viewer.json`、ID 小写。
- `src/http/routes/epg.rs`：三个公开端点与错误形状。
- `src/http.rs`：全局 CORS。
- `tests/http_contract.rs`：公开端点契约测试。

## 7. 局限

- 未验证多 VPS 分享场景下公开表是否合并远端节目。
- 未验证 `kind` 是否存在 `local` 之外的值。
- 未验证分页；当前证据显示频道不截断、节目固定 200。
- 未反编译确认 `last_stop` 是否含已过滤节目。
