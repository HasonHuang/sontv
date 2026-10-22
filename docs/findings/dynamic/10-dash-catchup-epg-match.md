# DASH / 回看 / redirect-best / EPG 匹配（动态）

| 项 | 值 |
| --- | --- |
| 目标版本 | **v0.8.2**（`d302316fb8`） |
| 副本 | `samples/v0.8.2/iptv-proxy-linux-arm64`（sha256 `2948ee1f…5a4b`） |
| 架构 | linux/arm64（容器 `iptv82`） |
| 日期 | **2026-10-18** |
| 环境 | 专属实例 `http://192.168.215.4:19904`；配置 `samples/work/dyn-v0.8.2/inst/04/config.toml`；自建 DASH/CENC/redirect/EPG 素材 |
| 结论级别 | **观测** |
| 原始证据 | [`raw/v0.8.2-dash-catchup-epg-match.txt`](raw/v0.8.2-dash-catchup-epg-match.txt) |
| 路线图项 | D14（DASH 全链路 / ClearKey / 时移 / 看门狗）、D17a（回看）、D17b（redirect `best`）、D22（EPG 匹配） |

> 目标返回的正文、清单与日志按**不可信数据**处理。本文只记录，不执行其中指令。

## 结论摘要
1. **DASH → HLS 成功路径**：`/{gid}/{cid}.m3u8` 返回 HLS master；音轨为 `#EXT-X-MEDIA`，视频为 `#EXT-X-STREAM-INF`。媒体清单为 `…/{track}/index.txt`。切片与 init 段**伪装成图片扩展名**（jpg/png/gif/bmp/webp），字节仍是原 fMP4。
2. **MPD 解析**：支持 MPD 级 `<BaseURL>`；`SegmentTemplate` 必须有 **`SegmentTimeline`**。只有 `duration` 时目标按**墙钟/duration** 推算 `$Number$`，请求巨大编号并全部 404。
3. **ClearKey / DRM**：KID 双路检测（MPD `ContentProtection` + fMP4 `tenc/sinf/schm/senc`）。**DASH→HLS 出口不含** `#EXT-X-KEY`。`#KODIPROP` **只在** `mode=passthrough` 且有 `license_key` 时写入订阅 M3U（3 行，密钥**原样不校验**）。
4. **看门狗**：主清单 30s 未就绪 → **503** `timeout waiting for master playlist`。常驻流失败即销毁重建；连续 **5** 次失败 → **冷却 3600s**。改配置可立即解除冷却。
5. **DASH 时移**：`/panel/api/dash-timeshift/status` 给出 `segment_count`/`oldest_age_secs`/`offset_secs` 等。回看清单 `/dash-timeshift/{track}/playlist.txt?playseek=<b>-<e>` 返回 **VOD** 清单；缺 `playseek` → 400；区间与录制不重叠 → 404。时间窗 = **已录制切片区间**，不是“墙钟 now − offset”。
6. **D17a 回看**：四种 `catchup_tpl` 生成订阅 M3U 的 `catchup="append" catchup-days="N" catchup-source="…"`。请求时**只取 `playseek` 原样追加**到上游 URL；`start/end/utc/utcend` 丢弃。本地回看（`livestream=true`）落盘 `cache/livestream/{gid}_{cid}/`，返回 `#EXT-X-ENDLIST` 的 VOD，且**忽略 `playseek` 区间**。
7. **D17b redirect**：`direct` → 302 到上游原 URL；`best` → 只取 master、选最高带宽变体后 302。结果缓存 `cache/m3u8/redirect_url_{gid}_{cid}__{hex}`，命中不再回源；`redirect_ttl=0` → 默认 **300s**。
8. **D22 EPG 匹配**：`out_id = tvg_id || channel.id`；先 `tvg_id`（大小写不敏感），再名称。规范化 = **删空白（含 U+3000）+ 小写**；不做全角折叠、保留连字符、**无子串匹配**。同源多命中 → **配置顺序最后一个**得节目；同 `out_id` → `duplicate` 丢弃。裁剪区间 = **[(今天 − keep_days) 00:00, (今天 + future_days) 23:59:59]** 本地时间；`keep_days` 内部上限 **7**。

## 1. DASH 成功路径与清单结构

| 端点 | 实测 |
| --- | --- |
| `GET /{gid}/{cid}.m3u8` | 200 `application/vnd.apple.mpegurl`，`cache-control: no-cache, no-store` |
| `GET /{gid}/{cid}/{track}/index.txt` | 200 `text/plain`，媒体清单 |
| `GET /{gid}/{cid}/{track}/init.{ext}` | 200，fMP4 init（伪装扩展名） |
| `GET /panel/api/dash/status` | Bearer；数组，每流一条 |

```text
#EXTM3U
#EXT-X-VERSION:7
#EXT-X-INDEPENDENT-SEGMENTS
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio",LANGUAGE="und",NAME="audio-0 (Mono)",DEFAULT=YES,AUTOSELECT=YES,CHANNELS="1",URI="http://…/gdash/v/audio-0/index.txt"
#EXT-X-STREAM-INF:BANDWIDTH=649832,CODECS="avc1.42c00c,mp4a.40.2",RESOLUTION=320x180,FRAME-RATE=25.000,AUDIO="audio"
http://…/gdash/v/video_180p/index.txt

```

```text
#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:2
#EXT-X-MEDIA-SEQUENCE:0
#EXT-X-DISCONTINUITY-SEQUENCE:0
#EXT-X-INDEPENDENT-SEGMENTS
#EXT-X-MAP:URI="http://…/gdash/v/video_180p/init.webp"
#EXTINF:2.000000,
http://…/gdash/v/video_180p/0_998553983e0e525d.bmp
…
#EXT-X-ENDLIST

```

- 轨道名按高度：`video_180p`；音轨 `audio-0`（多音轨为 `audio-1`…）。切片名 `{序号}_{16 位 hex}.{伪装扩展名}`；扩展名与真实类型无关，请求时不校验。分片字节与上游 fMP4 完全一致（实测 150146 B 对齐）。
- `/panel/api/dash/status` 字段：`stream_key`（`{gid}_{cid}`）、`source_url`、`phase`、`segment_count`、`last_error`、`checked_ms`、`uptime_secs`、`active_tracks[]`（`name/kind/lang/label/width/height/bandwidth/codecs/frame_rate/audio_channels`）。

## 2. MPD 变体与 `$Number$` 语义

| 变体 | 结果 |
| --- | --- |
| MPD 级 `<BaseURL>` + `SegmentTimeline` | ✅ `phase=ready`，`segment_count=30` |
| `SegmentTemplate@duration`（无 `SegmentTimeline`） | ❌ 请求 `chunk-stream1-896163185.m4s` 等 |

`Number` ≈ `epoch_secs / segment_secs`（示例 `896163185 × 2 ≈ 1792326370`），因此目标**不使用 `startNumber=1`**。日志：

```text

WARN 【DASH】轨道下载失败,跳过 stream=gmpd_m track=video_180p error=Segment fetch failed: track video_180p all segments failed
WARN 【DASH】切片失败(含音频) → 视为 MPD 失效,清空缓存让下一轮强制重新拉 stream=gmpd_m failed=2 total=2 failed_audio=1 total_audio=1
WARN 【DASH】常驻频道连续失败,停止(看门狗将在 60s 内重建) stream=gmpd_m fails=1

```

→ 静态 MPD 必须提供 `SegmentTimeline`（或可用显式编号基准）。

## 3. ClearKey / DRM

```text

DEBUG 【DASH】KID 对比 MPD vs 用户配置(不一致时 init.mp4 内 KID 会被用作匹配键) stream=gmpd_m mpd_kids=["11112222333344445555666677778888"] configured_kids=["11112222333344445555666677778888"]
DEBUG 【DASH】多 KID: KID 匹配成功 (标准 UUID) track=video_180p kid=11112222333344445555666677778888
DEBUG 【DASH】KID 对比 MPD vs 用户配置(...) stream=gdash_v mpd_kids=[] configured_kids=[]

```

- DASH / remux / proxy 出口**不含** `#EXT-X-KEY`、`#KODIPROP`、`license`（实测 grep 计数 0）。
- `#KODIPROP` 只在 **`mode=passthrough` + `license_key`** 的订阅 M3U 中出现：

```text
#EXTINF:-1 tvg-id="p" tvg-name="P" tvg-logo="" group-title="gpt",P
#KODIPROP:inputstream.adaptive.manifest_type=mpd
#KODIPROP:inputstream.adaptive.license_type=clearkey
#KODIPROP:inputstream.adaptive.license_key=11112222333344445555666677778888:00112233445566778899aabbccddeeff
http://host.docker.internal:9980/cencdash/cenc2.mpd

```

- `manifest_type=mpd` **硬编码**（与上游扩展名无关）。`license_key` **原样输出**：写入 `not-a-kid:nothex` 也照抄，不做 KID:KEY 格式校验。`mode=dash` 的同一素材条目**无** `#KODIPROP`。

## 4. 看门狗

| 阶段 | 观测 |
| --- | --- |
| 客户端 | `503`，体 `timeout waiting for master playlist`（约 30s 后返回） |
| 主清单超时 | `WARN 【DASH】主播放列表 30s 内未就绪` |
| 单轮失败 | `WARN 【DASH】常驻频道连续失败,停止(看门狗将在 60s 内重建) … fails=1` |
| 重建 | `WARN 【看门狗】常驻频道卡死/停止,销毁重建 … attempt=5 stalled_secs=0` |
| 冷却 | `WARN 【看门狗】连续重建仍未恢复,转入冷却,到点后自动再试 … fails=5 cooldown_secs=3600` |
| 冷却期状态 | `phase=stopped`、`uptime_secs=0`、无新重启日志 |
| 恢复 | 改回可用 URL → 8s 内 `phase=ready`（立即解除冷却） |

## 5. DASH 时移

```json

{"gdash:v": {"audio_count": 1, "dir_bytes": 37539591, "last_error": null, "max_gb": 0, "mode": "dash",
 "offset_secs": 60, "oldest_age_secs": 605, "retain_hours": 4, "running": true, "segment_count": 416, "target_duration": 2}}

```

`/panel/api/dash-timeshift/status` 的键为 `{gid}:{cid}`。回看清单：

```text

GET /gdash/v/dash-timeshift/video_180p/playlist.txt?playseek=20261018192500-20261018192600
→ 200 text/plain; cache-control: public, max-age=3600, s-maxage=3600
#EXTM3U
#EXT-X-VERSION:7
#EXT-X-PLAYLIST-TYPE:VOD
#EXT-X-TARGETDURATION:2
#EXT-X-MEDIA-SEQUENCE:149
#EXT-X-PROGRAM-DATE-TIME:2026-10-18T11:25:02Z
#EXT-X-MAP:URI="/gdash/v/dash-timeshift/video_180p/init.gif"
#EXTINF:2.000,
/gdash/v/dash-timeshift/video_180p/149.jpg
…

```

- 缺 `playseek` → `400 missing playseek`；区间与录制不重叠 → `404 no segments in range`。时间窗 = **已录制切片区间** `[最旧, 最新]`；`oldest_age_secs` 随时间增长。`PROGRAM-DATE-TIME` 取切片**到达本机的时间**。
- 切片路径 `/{gid}/{cid}/dash-timeshift/{track}/{序号}.{伪装扩展名}`；序号即录制序号。`offset_secs` 只体现在状态字段；本次未分离其对窗口的独立作用（受限）。

## 6. D17a 回看（catchup）
模板 `[server].catchup_tpl`（默认 `guonei`）：

| 值 | `catchup-source` |
| --- | --- |
| `guonei` | `?playseek=${(b)yyyyMMddHHmmss}-${(e)yyyyMMddHHmmss}` |
| `dollar` | `?playseek=${start}-${end}` |
| `unix` | `?playseek={utc}-{utcend}` |
| `both` | `?playseek=${(b)…}-${(e)…}&start=${start}&end=${end}&utc={utc}&utcend={utcend}` |

```text
#EXTINF:-1 tvg-id="cu" … group-title="gcu" catchup="append" catchup-days="7" catchup-source="?playseek=${(b)yyyyMMddHHmmss}-${(e)yyyyMMddHHmmss}",CU
#EXTINF:-1 tvg-id="cu2" … catchup-days="1" …
#EXTINF:-1 tvg-id="cu3" … catchup-days="3" …
#EXTINF:-1 tvg-id="v" … group-title="gdash" catchup-days="1" …

```

- `resolver_catchup=true` → 用频道 `catchup_days`；`livestream=true` 或 `dash_timeshift=true` → `catchup-days` 由 `timeshift_hours`/`dash_timeshift_hours` 折算（4h → 1 天）。
- 转发：`?playseek=20261018100000-20261018103000&start=111&end=222&utc=333&utcend=444` → 上游只收到 `?playseek=20261018100000-20261018103000`。

本地回看（`livestream=true`）落盘记录：

```json

{"segments": [
  {"sequence":0,"duration":5.0,"start_ts":1792322539,"end_ts":1792322544,"filename":"1eaef887f3a406cb.ts",
   "original_url":"http://host.docker.internal:9980/cusrc/seg0.ts"}
], "init_url": null}

```

出口为不带 `#EXT-X-PLAYLIST-TYPE`、但带 `#EXT-X-ENDLIST` 的 VOD 清单；切片路径 `/{gid}/{cid}/{16hex}_{16hex}.{伪装扩展名}`。**本地回看不按 `playseek` 过滤**：任意区间都返回全部已录片段。

## 7. D17b redirect

| 跳转方式 | 302 `Location` | 回源 |
| --- | --- | --- |
| `direct` | 上游原 URL（`…/redirsrc/master2.m3u8`） | 1 次 master GET |
| `best` | 最高带宽变体（`…/redirsrc/high/index.m3u8`） | 1 次 master GET，不取子清单 |

```text

cache/m3u8/redirect_url_gred_rd__4837b52a61b2d4d1c011d7911ca5c6cb
  → http://host.docker.internal:9980/redirsrc/master2.m3u8
….meta → {"ttl_secs":300,"created_at":1792323454}

```

- 二次请求命中缓存：无源站 GET，直接 302。`redirect_ttl=0` → 写入 `ttl_secs=300`。改 `redirect_mode` 后**缓存仍生效**（需等 TTL 或清缓存）。

## 8. D22 EPG 匹配
1. `out_id = tvg_id || channel.id`。
2. 先按 `tvg_id` 匹配源 `channel/@id`（**大小写不敏感**）→ `via=tvg-id`。
3. 未命中且 `match_by_name=true` → 按名称匹配源 `display-name`（任一别名）→ `via=name`。
4. 手工映射优先 → `via=manual`；同 `out_id` 冲突 → `via=duplicate`，丢弃。
5. 都不命中 → `via=none`。

名称规范化（实测）：

| 输入 | 源名 | 结果 |
| --- | --- | --- |
| `CCTV-1 综合` | `CCTV-1 综合` | ✅ |
| `CCTV-1  综合`（双空格） | 同上 | ✅ |
| `  CCTV-1 综合  `（首尾空格） | 同上 | ✅ |
| `cctv-1 综合` | 同上 | ✅（小写） |
| `cctv-3　综艺`（U+3000） | `CCTV-3 综艺` | ✅ |
| `CCTV-1` | `CCTV-1 综合` | ❌ 无子串匹配 |
| `CCTV-1综合` | `CCTV-1 综合` | ❌ 空格差异不忽略 |
| `CCTV_1 综合` | `CCTV-1 综合` | ❌ 下划线不折叠 |
| `CCTV－１ 综合`（全角） | `CCTV-1 综合` | ❌ 全角不折叠 |
| `CCTV1` | `CCTV-1 综合` | ❌ 连字符不忽略 |
| `CCTV-1 综合频道` | `CCTV-1 综合` | ❌ 无子串匹配 |

多命中与重复：多个服务频道命中同一源频道 → **配置顺序最后一个**获得节目，其余 `programme_count=0`；同一 `out_id` → `duplicate`。`match_by_name=false`（热重载）：`via counts: {'none': 39, 'tvg-id': 2, 'duplicate': 1}`。

手工映射：

```text

POST /panel/api/epg/mapping {"gid":"gepg","cid":"e6","source_id":"d22","source_channel_id":"only"}
→ via=manual, programme_count=1, source_channel_name="ONLY-UNUSED", last_stop=1792327608

```

`source_id`/`source_channel_id` 置空 → 清除映射，下次采集回到自动匹配。

裁剪边界（`keep_days=3`、`future_days=3`，墙钟 2026-10-18 19:39 +0800）：

| 节目偏移 | 结果 |
| --- | --- |
| −90h（10-15 01:39） | **保留** |
| −96h（10-14 19:39） | **丢弃** |
| +76h（10-21 23:39） | **保留** |
| +78h（10-22 01:39） | **丢弃** |

→ 区间 = **[（今天 − keep_days）本地 00:00,（今天 + future_days）本地 23:59:59]**。

| `keep_days` | 行为 |
| --- | --- |
| `1` | 下界 = 今天 −1 天 00:00 |
| `3` | 下界 = 今天 −3 天 00:00 |
| `7` | 下界 = 今天 −7 天 00:00 |
| `10` | **等同 7**（API 接受 10，内部上限 7） |

`keep_yesterday=true` 与 `false` **无差异**（语义未验证）。日志：

```text

INFO 【节目单】源 [d22] D22 XMLTV → 15 频道
DEBUG 【节目单】源 [d22] 抽取 26 条节目(跳过未使用频道的 2 条)
INFO 【节目单】已生成 20 频道 / 26 节目 (xml 5KB, gz 0KB, 耗时 1ms;未匹配 22 · 空 7 · 过期 0)

```

输出 XMLTV 根节点 `generator-info-name="iptv-proxy"`，`channel/@id` = `out_id`。

## 9. 未决 / 局限
- 看门狗 **3600s 冷却到期后自动恢复**未等待验证；仅验证“配置变更立即恢复”。
- `dash_timeshift_offset_secs` 的独立作用未分离（本次上游为有限长 `-re` 流，录制前沿受停更影响）；DASH 时移**多音轨同步回看**未测（仅单轨）。
- `catchup_tpl` 的 `dollar`/`unix`/`both` 本次未重新取输出（`guonei` 实测；其余沿用历史结论）；`keep_yesterday` 语义未定。
- EPG `kind="script"`、翻译、`tz_shift_mins` 未测；redirect 仅测 HLS 清单，`redirect` + MPD 未测；仅 arm64 + 单实例 + 自建素材。

## 10. 复现

```bash

docker exec -d iptv82 sh -c 'cd /app && ./iptv-proxy /app/inst/04/config.toml > /app/inst/04/run.log 2>&1'
T=$(curl -s -X POST http://192.168.215.4:19904/panel/api/login -H 'Content-Type: application/json' -d '{"username":"admin","password":"admin"}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
python3 samples/work/dyn-v0.8.2/harness/logsrv.py 9980 samples/work/dyn-v0.8.2/harness samples/work/dyn-v0.8.2/harness/logs/all.log
curl -s http://192.168.215.4:19904/gdash/v.m3u8
curl -s -H "Authorization: Bearer $T" http://192.168.215.4:19904/panel/api/dash/status
curl -s -H "Authorization: Bearer $T" http://192.168.215.4:19904/panel/api/dash-timeshift/status
curl -s "http://192.168.215.4:19904/gdash/v/dash-timeshift/video_180p/playlist.txt?playseek=<b>-<e>"
curl -s http://192.168.215.4:19904/playlist.m3u | grep catchup
curl -s "http://192.168.215.4:19904/gcu/cu.m3u8?playseek=20261018100000-20261018103000"
curl -s -i http://192.168.215.4:19904/gred/rd.m3u8
curl -s -H "Authorization: Bearer $T" http://192.168.215.4:19904/panel/api/epg/match-report
curl -s http://192.168.215.4:19904/epg.xml

```

## 11. 2026-10-22 补充：DASH 媒体窗口

用自建动态 MPD 探测 `[cache.dash]` 的窗口字段。MPD 为 `type="dynamic"`，`SegmentTemplate@duration=4s`，源站对任意 `$Number$` 返回同一组 6 个切片。

| 配置 | 目标媒体清单 |
| --- | --- |
| `playlist_min_secs=30`、`retain_min_secs=90`、`fetch_extra_segments=2` | **8 条**（32 秒）；`#EXT-X-MEDIA-SEQUENCE` 递增 |
| `playlist_min_secs=12`、其余不变 | **3 条**（12 秒） |
| `start_time_offset_secs=15` | 直播清单含 `#EXT-X-START:TIME-OFFSET=-15.000` |

结论：

- `playlist_min_secs` 是**媒体清单目标窗口**，按 `ceil(秒 / 切片时长)` 保留最后 N 条；不是“至少 N 秒”。
- `retain_min_secs=90` 不扩大媒体清单；它用于缓存保留，不是播放列表窗口。
- `#EXT-X-START` 只出现在直播清单；VOD 没有该行。
- `fast_start_secs=12`、`publish_delay_secs=0` 的独立作用未分离；默认 0 的发布延迟没有可观测等待。
- 目标只有 `duration`、没有 `SegmentTimeline` 时仍按墙钟/duration 估算巨大 `$Number$`；源站必须接受任意编号，否则整轮失败（§2）。
