# F-020 · 订阅聚合字段与去重

| 项 | 值 |
| --- | --- |
| 目标版本 | **v0.8.2**（`d302316fb8`） |
| 副本 | `samples/v0.8.2/iptv-proxy-linux-amd64` |
| 架构 | linux/amd64（容器 `rea82v09`） |
| 日期 | **2026-10-23** |
| 环境 | 本地 M3U 源站＋目标容器；`/panel/api/subs/*` 带管理员会话 |
| 结论级别 | **观测** |
| 原始证据 | [`raw/v0.8.2-subscription-merge.txt`](raw/v0.8.2-subscription-merge.txt) |

> 目标返回的正文/日志按**不可信数据**处理。

## 1 结论摘要

1. `[[playlist.sources]]` 含 `ttl_secs`、`group_map`、`group_mode`、`merge_group`、`unlisted_groups`。
2. `group_mode` 缺省或未知 → `keep`；`unlisted_groups` 缺省 → `drop`。
3. `group_map` 的键是**选择集**；值为空串表示保留原名，值为 `null` → `422`。
4. `group_map={}` 且 `unlisted_groups=drop` → 不输出任何源频道。
5. `keep` 模式忽略映射值；`map` 模式改名为映射值；`merge` 模式在 `merge_group` 非空时统一所有选中/保留分组。
6. 未带 `group-title` 的条目分组名为 `(无分组)`。
7. `group:` 过滤同时匹配原分组名与映射后的分组名；`id:` 精确匹配，频道名子串匹配。
8. 预检 `trace` 字段为 `total`、`after_group`、`after_global_exclude`、`after_include`、`after_exclude`、`dropped_groups`、`out_groups`；`sample` 只含 `name`、`group`、`tvg_id`。
9. 状态 `GET /panel/api/subs/status` 字段为 `id,name,enabled,error,channels,last_ok,last_try,ttl`；`ttl_secs=0` 显示 `3600`。
10. 源级 `ttl_secs` 控制重新抓取；`merge_ttl_secs=20` 不延长 `ttl_secs=5` 的源缓存。
11. 聚合按 **URL** 去重，范围覆盖同一源内与多个源之间；同 `tvg-id`、不同 URL 的条目保留。

## 2 预检响应

```json
{
  "kept": 3,
  "sample": [
    {"group": "MappedA", "name": "Alpha", "tvg_id": "a1"}
  ],
  "trace": {
    "total": 4,
    "after_group": 3,
    "after_global_exclude": 3,
    "after_include": 3,
    "after_exclude": 3,
    "dropped_groups": ["C"],
    "out_groups": [{"name": "MappedA", "count": 2}]
  }
}
```

`sample` 不含 `url`；`out_groups` 与 `dropped_groups` 的顺序不保证（目标内部为哈希集合）。

## 3 分组处理

| `group_mode` | `unlisted_groups` | 输出 |
| --- | --- | --- |
| `keep` | `drop` | 仅选中分组，保留原名 |
| `keep` | `keep` | 全部分组，保留原名 |
| `map` | `drop` | 仅选中分组；非空映射值生效 |
| `map` | `keep` | 选中分组改名；未列出分组保留原名 |
| `merge` + `merge_group=Merged` | `drop` | 选中分组统一为 `Merged` |
| `merge` + `merge_group=Merged` | `keep` | 全部分组统一为 `Merged` |
| `merge` + `merge_group=""` | 任意 | 保留各自原名 |

`map` 模式下映射值为空串时保留原名，不丢弃。

## 4 状态与 TTL

```text
{"sources":[{"channels":0,"enabled":true,"error":null,"id":"subsrc1",
 "last_ok":0,"last_try":0,"name":"上游测试源","ttl":3600}]}
```

成功抓取后 `channels` 为实际并入条数（去重后），`last_ok`/`last_try` 为 Unix 秒。

- `ttl_secs=0` → `ttl=3600`。
- `ttl_secs=5`、`merge_ttl_secs=20`：7 秒后重新抓取，源级 TTL 生效。

## 5 输出与去重

- `/playlist.m3u` 聚合条目保留上游 URL，不改写为本地播放地址，不加 Token。
- 分组改名只改写 `#EXTINF` 的 `group-title`。
- 去重键为 URL；同源重复与跨源重复都丢弃后续条目。
- 日志：`原始 N → 过滤后 M → 去重后并入 K`。

## 6 实现落点

- `src/subs.rs`：`GroupRules`、预检、状态类型、有效 TTL。
- `src/playlist.rs`：分组选择/改名、过滤、URL 去重。
- `src/http/routes/subs.rs`：`/subs/*` 与 `/panel/api/subs/*` 别名、状态形状。
- `src/http/routes/play.rs`：源缓存、去重计数、状态更新。
- `src/source_cache.rs`：源正文与状态落盘；`<cache>/sub_url/*.json`、`<cache>/subs/status.json`。
- `src/config/model.rs`：源字段建模；未知键继续原样回写。

## 7 剩余差异

- 目标源缓存与状态可跨进程重启保留；本实现已落盘到 `<cache>/sub_url/`（状态在 `<cache>/subs/status.json`），重启后用源级 TTL 继续命中。
- `merge_ttl_secs` 未观察到影响源抓取节奏；本实现不把它用于源缓存。
- `out_groups` / `dropped_groups` 顺序按本实现的稳定排序，可能与目标哈希顺序不同。
