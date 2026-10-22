# F-018 · P0 契约对拍修正

- 日期：2026-10-20
- 方法：同一份夹具（伪上游 + 同一键面配置），容器内目标 v0.8.2 与本实现各跑一份。
  `scripts/compare-p0.py` 比对状态码、响应头、正文形状；`scripts/compare-p0.sh` 负责启动两侧。
- 结果：`docs/design/reconciliation-report.md`。

## 观察与结论

| 编号 | 目标行为 | 影响 | 处理 |
| --- | --- | --- | --- |
| R-01 | 代理入口按**上游清单类型**分支：上游是媒体清单 → 直接下发媒体清单；上游是主清单 → 下发自造主清单 | 原实现恒返回自造主清单 | 已改（`proxy_entry`） |
| R-02 | 上游是媒体清单时，`/{gid}/{cid}/video/index.txt` → `302 error_url` | 原实现返回 200 | 已改（`variant_playlist`） |
| R-03 | 主清单入口正文固定为 `#EXT-X-VERSION:6` + `#EXT-X-INDEPENDENT-SEGMENTS` + 最佳变体的 `#EXT-X-STREAM-INF` 白名单属性 + 绝对 `index.txt` | 原实现用 `BANDWIDTH=2000000` 与根相对路径 | 已改（`master_entry`） |
| R-04 | 清单缓存窗口 = 清单的 `#EXT-X-TARGETDURATION`；改 `[cache.ttl]`、`[cache.dash]` 不影响 | 原实现固定 5 秒 | 已改（`media_response`） |
| R-05 | 切片地址是**绝对地址**（含 `Host`） | 原实现给根相对路径 | 已改（`rewrite_media` 增加 `self_base`） |
| R-06 | 未知分组/频道（入口形状）：无 Token 或错 Token → `token_failure_url`；Token 合法 → `not_found_url` | 原实现恒 `not_found_url` | 已改（`unknown_endpoint`） |
| R-07 | 未知路径（其它形状，如 `/nope/nope/nope`）→ `not_found_url`，不看 Token | 原实现返回 `404` | 已改（通配路由回退） |
| R-08 | 归档组入口 → `not_found_url`，先于 Token 判定 | 与实现一致 | 不变 |
| R-09 | `passthrough` **校验 Token** | 原实现免 Token（S-MODE-11 有误） | 已改 |
| R-10 | `rewrite` 响应头 = `no-cache, must-revalidate` | 原实现用入口缓存值 | 已改 |
| R-11 | `/{gid}/{cid}/index.m3u8` 与入口同义 | 原实现无此路径 | 已改（别名） |
| R-12 | 未知面板接口（如 `/panel/api/nope`）→ `404` | 面板路径被播放通配路由吞掉 | 已改（显式路由先匹配） |
| R-13 | 切片首写 `public, max-age=300`；命中 `private, max-age=300` | 与 F-006 一致；开 `[cache.prefetch]` 时客户端只会看到命中值 | 不变 |
| R-14 | 伪上游晚于本服务就绪时，首个请求会 502 | 冒烟脚本存在竞态 | 已改（冒烟先等上游就绪） |
| R-15 | 上游请求总超时 **25 秒**；正文中断后仍回 200 和已收部分字节 | 原实现 20 秒且缓冲整包，超时回 502 | 已改（F-019；`stream_segment`） |
| R-16 | 上游切片 404 → **200 空正文** | 原实现 502 | 已改（F-019） |
| R-17 | 上游清单 404 → **302 error_url**；传输错误/5xx 最多 4 次尝试 | 原实现一次请求后 502 | 已改（`fetch_playlist`） |
| R-18 | 主清单 `#EXT-X-STREAM-INF` 只保留 `BANDWIDTH`、`CODECS`、`RESOLUTION`、`FRAME-RATE` | 原实现输出上游全部属性 | 已改（`normalize_variant_attrs`） |
| R-19 | 媒体清单：VOD 用 `public, max-age=<TARGETDURATION>`；直播用 `no-cache, no-store` | 原实现直播也输出 `public` | 已改（`media_cache_control`） |
| R-20 | 上游扩展名已在 `formats` 中时保留（如 `.jpg`） | 原实现一律轮换 | 已改（`pick_ext`） |

## 未解与内部差异

| 项 | 说明 |
| --- | --- |
| 伪装前缀 `<P>` | 目标算法未解（F-017）。本实现用 `sha256("<gid>:<cid>")[:16]`。 |
| 伪装扩展名轮换 | 目标轮换算法未解。两侧取值不同，但都取自 `[cache.disguise].formats`。 |
| 清单缺 `TARGETDURATION` | 目标行为未采证。本实现按 6 秒。 |
| 缓存文件名 | 目标用 `m3u8/`、`seg_url/`、`seg_bytes/`、`sub_url/`；本实现同名目录、不同文件命名。内部值。 |

## 影响文档

- `docs/design/03-http-contract.md` §4、§4.1、§4.2：按实测重写。
- `scripts/smoke-m2.sh`、`scripts/smoke-m3.sh`、`scripts/smoke-m4.sh`：断言更新。
- `scripts/compare-p0.sh`、`scripts/compare_p0.py`：对拍基线。
