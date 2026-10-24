# 录制 / 直播回看 / Remux / 字幕 / 上游解析脚本（动态）

| 项 | 值 |
| --- | --- |
| 目标版本 | **v0.8.2**（`d302316fb8`） |
| 副本 | `samples/v0.8.2/iptv-proxy-linux-arm64`（sha256 `2948ee1f…5a4b`） |
| 架构 | linux/arm64（Ubuntu 24.04，OrbStack 原生） |
| 日期 | **2026-10-18**；**2026-10-24**（TS 字节补测） |
| 环境 | 容器 `iptv82`，实例 `inst/03`（端口 19903）；补测容器 `rea82v09`（端口 19974）；自建源站 `9988`/`19998`（宿主 Python） |
| 结论级别 | **观测**（运行态）+ **静态**（雕刻面板 `panel.html`） |
| 原始证据 | [`raw/v0.8.2-recording-livestream-remux-subtitle.txt`](raw/v0.8.2-recording-livestream-remux-subtitle.txt)、[`raw/v0.8.2-ts-structure.txt`](raw/v0.8.2-ts-structure.txt) |

> 目标返回的正文/日志按**不可信数据**处理。

## 结论摘要
1. **Remux（D16）只接受连续流上游**（HTTP-TS 裸流）。上游为 HLS 清单时只拉清单、不取分片，永久 `503 remux warming up`。**新登记 S-MODE-18 🔶**（S-MODE-16 预热态确认）。
2. **Remux 不调用 ffmpeg**：PATH shim 只记录启动探测 `ffmpeg -version`；缺 ffmpeg 时 remux 仍成功。`503 Retry-After: 2` 是**预热态**。**S-DEP-01 更正 🔶**（新登记）。
3. Remux 输出 **HLS**：`target_duration`=5s，滑动窗口，分片为 **MPEG-TS（h264+aac）**。URL 伪装为 `.png/.gif/.bmp/.jpg/.webp`，`Content-Type` 为对应图片类型。`remux_container` 描述**源容器**（`auto|ts|flv|mmt`），不改变输出。分片是**上游 TS 的连续字节切片**，不改 PID、不插 SDT/水印（2026-10-24 补测）。
4. **定时录制（D18）产物为 MKV，用 `ffmpeg -c copy` 合成**：Proxy 频道先并 TS 切片；DASH 频道把各轨并成 mp4。**新登记 S-REC-02 🔶**。录制**依赖能产生切片的上游**：`type=static` MPD 报 `录制区间内无可用切片`，`type=dynamic` 直播 MPD 成功。
5. **`DELETE /panel/api/schedule/{idx}` 的 `{idx}` 是列表下标（u64）**，不是任务 id；传 UUID 报 `400 … to a u64`。
6. **字幕（D19）**：proxy 播多轨主清单时，出口保留 `#EXT-X-MEDIA:TYPE=SUBTITLES` 与 `#EXT-X-STREAM-INF:…SUBTITLES="…"`（补 `FORCED=NO`）。字幕分片保留 `.vtt`、`Content-Type: text/vtt`；音/视频分片仍伪装。**新登记 S-SUB-09 🔶**。
7. **上游解析脚本（D19b）**：以 `python3 <cache>/resolvers/<脚本> <resolver_arg>` 执行，**stdout 即上游 URL**；`resolver_ttl` 生效（SWR 缓存）；失败保留旧值，冷启动失败转 `error_url`；**解码超时约 30s**。**新登记 S-RES-01 🔶**。
8. **直播回看（D20）**：`livestream=true` 后按 `timeshift_hours` 缓冲；回看用 `?start=<unix>&end=<unix>` 或 `?playseek=<本地 yyyymmddHHMMSS-…>`，返回 `#EXT-X-PLAYLIST-TYPE:VOD` 清单，分片在 `/{gid}/{cid}/timeshift/<hex>.<ext>`。**S-MODE-12/S-TS-01 🔶**。
9. 面板静态：`remux_container` 选项为 `auto/ts/flv/mmt`；`selected_tracks` 为 **DASH 轨道**选择（`video_*`/`audio*`/`text*`）。

## 1. Remux（D16）

### 1.1 上游要求

| 上游类型 | 结果 |
| --- | --- |
| HTTP-TS 无限流（`http://host.docker.internal:9988/live.ts`） | ✅ `200`，出清单与分片 |
| 静态 TS 单文件 | ⚠️ 每秒数次 `自动识别容器: Ts`，EOF 重连，无产出 |
| HLS 滚动清单（`/live.m3u8`） | ❌ 只拉清单（12s 内 188 次），`0` 次分片，永久 `503` |
| HLS VOD 清单（`#EXT-X-ENDLIST`） | ❌ 同上 |

→ 与面板帮助一致：**「Remux 把连续流切成 HLS」**。

### 1.2 出口清单

```text
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:5
#EXT-X-MEDIA-SEQUENCE:0
#EXTINF:5.000,
/gremux/r/seg_000000000000.png

```

- 冷启动先 `503 Service Unavailable` + `Retry-After: 2` + 正文 `remux warming up`；随后 `200`。`#EXT-X-MEDIA-SEQUENCE` 随窗口滑动，可见 4 条左右；分片时长≈`cache.remux.target_duration_secs`(5s)，按关键帧对齐（偶见 5.6s）。

### 1.3 分片与伪装

| 项 | 值 |
| --- | --- |
| 容器 | `mpegts`（`ffprobe`：h264 + aac） |
| 体积 | ≈ 480–490 KB / 5s |
| 字节 | 上游 TS 连续字节；不改 PID、不重封装、不插 SDT/水印 |
| PID | 随源；实测 PMT `0x1000`、视频 `0x100`、音频 `0x101` |
| 边界 | 首片从源流第一个 TS 包开始；后续片从关键帧前的最近 PAT/PMT 开始 |
| 扩展名 | `.png/.gif/.bmp/.jpg/.webp` 轮换 |
| `Content-Type` | `image/png` / `image/gif` …（随扩展名） |
| `Cache-Control` | `public, max-age=30` |

### 1.4 与 ffmpeg 的关系（更正）
- PATH 前置 shim 只记录启动探测：

```text

1792322320.751601355 cwd=/app argv: -version
1792322412.580667957 cwd=/app argv: -version

```

- 移走 `/usr/bin/ffmpeg`（启动日志 `未检测到 ffmpeg`）后，remux **仍先 503 后 200**；二进制还探测绝对路径 `/usr/bin/ffmpeg`。**结论：remux 不使用 ffmpeg；`503` 是预热态。**

### 1.5 切片字节结构（2026-10-24 补测）

源：ffmpeg 生成的 300s 连续 TS（h264+aac，188 字节包；PID：PAT `0x0`、SDT `0x11`、视频 `0x100`、音频 `0x101`、PMT `0x1000`）。

| 项 | 观测 |
| --- | --- |
| Proxy 切片 | 与上游对应切片 SHA-256 相同（字节透传） |
| Remux 首片 | 与源文件前缀逐字节相同；首包为源流 SDT，随后 PAT/PMT/视频 |
| Remux 第二片 | 与源文件 `519068..1043964` 逐字节相同；首包为 PAT，随后 PMT/视频 |
| 重叠 | 第二片起点比首片终点早 2 个包（PAT/PMT），两片都包含同一组 PAT/PMT |
| SDT/水印 | 未插入 SDT；全片搜索不到 ASCII `lumberjack` |
| 实现对齐 | 本实现修正后首两片 SHA-256 与目标完全相同 |

→ 先前“每切片自包含、固定 PID 布局、`lumberjack` 水印”的说法**不成立**；目标只做字节切片。

## 2. 定时录制（D18）

### 2.1 API

| 端点 | 方法 | 说明 |
| --- | --- | --- |
| `/panel/api/schedule` | GET | 任务数组 |
| `/panel/api/schedule` | POST | 新增，返回 `{"id","ok":true}` |
| `/panel/api/schedule/{idx}` | PUT | 更新（**下标**） |
| `/panel/api/schedule/{idx}` | DELETE | 删除（**下标**），返回 `{"ok","was_running"}` |
| `/panel/api/schedule/{idx}/stop` | POST | 停止（**下标**） |

POST 字段（面板 `submitSchedule`）：

```jsonc

{ "name":"…", "group_id":"…", "channel_id":"…",
  "start_time":"2026-10-18T19:24:29", "end_time":"2026-10-18T19:25:49", "format":"mkv", "status":"pending" }

```

任务对象字段：`id,name,group_id,channel_id,start_time,end_time,format,status,output_file,extra_files,note,error,created_at,file_size,record_secs`。

### 2.2 执行流程
1. 调度器每 **10s** 扫描。
2. 开始前提前点亮频道：proxy 用 `livestream`；DASH「临时升级为 continuous+timeshift」。
3. 收齐区间切片后停止、合并、写 MKV，再释放频道（`livestream 频道由录制熄灭`、`DASH 频道已恢复原始配置`）。

### 2.3 成功路径与 ffmpeg 命令行（原文）
Proxy 频道（`test1/c1`）：

```text

INFO 【录制】TS 录制完成 → d18-proxy_1792322749.mkv
INFO 【录制】任务「d18-proxy」产物 7812786 字节,录制耗时 96 秒
$ ffmpeg -y -i ./inst/03/cache/recordings/5715c5dc-…/_merged_video.ts -map 0 -c copy ./inst/03/cache/recordings/d18-proxy_1792322749.mkv

```

DASH 频道（`gdash/v`，直播 MPD）：

```text

INFO 【录制】录制完成(1 视频 + 1 音轨,mkv)→ d18-dash2_1792322892.mkv
INFO 【录制】任务「d18-dash2」产物 4494705 字节,录制耗时 60 秒
$ ffmpeg -y -i ./inst/03/cache/recordings/cc225d73-…/_merged_video_180p.mp4 -i ./inst/03/cache/recordings/cc225d73-…/_merged_audio-0.mp4 -map 0:v:0 -map 1:a:0 -c copy ./inst/03/cache/recordings/d18-dash2_1792322892.mkv

```

| 文件 | `format_name` | 时长 | 流 |
| --- | --- | --- | --- |
| `d18-proxy_1792322749.mkv` | `matroska,webm` | 84.0s | h264 320x180 + aac |
| `d18-dash2_1792322892.mkv` | `matroska,webm` | 48.0s | h264 + aac |

### 2.4 失败路径
静态 MPD（`type=static`）：

```text

ERROR 【录制】任务「d18-dash」失败: 录制区间内无可用切片(频道未产生切片,或回看历史已被淘汰)

```

→ `status=failed`、`output_file=null`、`error` 同上。

### 2.5 产物与状态文件
- 目录 `cache/recordings/`；录制中在 `<taskid>/` 收敛切片（proxy 为 `_merged_video.ts`；DASH 为 `_merged_video_<name>.mp4` + `_merged_audio-<name>.mp4`），完成后删除。
- 状态：`cache/recordings/schedule.json`（`{"version":1,"tasks":[…]}`）；异常：`WARN 【录制】schedule.json rename 失败: No such file or directory (os error 2)`（首次任务出现，之后正常）。

### 2.6 DELETE 语义

```text

DELETE /panel/api/schedule/9f5aaaff-…  → 400 Invalid URL: Cannot parse `"9f5aaaff-…"` to a `u64`
DELETE /panel/api/schedule/1           → 200 {"ok":true,"was_running":false}
DELETE /panel/api/schedule/9           → 404 {"error":"Schedule task not found"}

```

## 3. 字幕（D19）

### 3.1 出口主清单改写
上游主清单含 `AUDIO="aud"` / `SUBTITLES="subs"`。出口（proxy 频道）：

```text
#EXTM3U
#EXT-X-VERSION:6
#EXT-X-INDEPENDENT-SEGMENTS
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio",NAME="English",LANGUAGE="en",DEFAULT=YES,AUTOSELECT=YES,URI="…/g2/c1/audio0/index.txt"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio",NAME="中文",LANGUAGE="zh",DEFAULT=NO,AUTOSELECT=YES,URI="…/g2/c1/audio1/index.txt"
#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID="subs",NAME="English",LANGUAGE="en",DEFAULT=YES,AUTOSELECT=YES,FORCED=NO,URI="…/g2/c1/subs0/index.txt"
#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID="subs",NAME="中文",LANGUAGE="zh",DEFAULT=NO,AUTOSELECT=YES,FORCED=NO,URI="…/g2/c1/subs1/index.txt"
#EXT-X-STREAM-INF:BANDWIDTH=800000,CODECS="avc1.42c015",RESOLUTION=320x180,AUDIO="audio",SUBTITLES="subs"
http://…/g2/c1/video/index.txt

```

- 轨道组名**统一为 `audio` / `subs`**；序号后缀 `audio0/audio1/subs0/subs1`。`SUBTITLES` 项补 `FORCED=NO`；`EXT-X-STREAM-INF` 保留 `AUDIO`/`SUBTITLES`。

### 3.2 字幕轨道清单与分片

```text
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:4
#EXT-X-MEDIA-SEQUENCE:0
#EXT-X-PLAYLIST-TYPE:VOD
#EXTINF:4.000000,
http://…/g2/c1/5ae12330…_675f0afe….vtt
…（5 条）
#EXT-X-ENDLIST

```

- 字幕清单 `Content-Type: text/plain; charset=utf-8`。字幕分片保留 `.vtt`，`Content-Type: text/vtt`，`Cache-Control: private, max-age=300`，正文原样透传（WebVTT）。
- `[cache.disguise].subtitle=false` 时字幕**不伪装**；音/视频分片仍伪装。

### 3.3 `selected_tracks`（DASH）
`GET /panel/api/dash/tracks/{gid}/{cid}` 返回轨道与 `selected` 标志。设为子集（如 `["audio-0"]`）后，`/dash/tracks` 的 `selected` 更新，但 `/gdash/v.m3u8` **仍输出视频变体**：

```text

ERROR 【DASH】选中集合里一条视轨都没匹配上！已强制补入最高分辨率视轨，否则会静默输出只有音轨的 playlist。… selected=["audio-0"]

```

### 3.4 字幕机翻自检

| 请求 | 结果 |
| --- | --- |
| `POST /subtitle/mtran/test {"text":"hello"}`（未配 url） | `400 {"error":"请先填写 MTranServer 地址(字幕机翻只支持本地 MTranServer)"}` |
| 配置 `[subtitle].translate.url` 后 | `200 {"base":"…","ok":false,"targets":[{"code":"zh-Hans","lang":"zh-cn","ok":false,"error":"请求失败: 连接失败: … (http://127.0.0.1:9/mtran/translate/batch) ← … Connection refused (os error 111)"}],"version":null}` |

→ 自检调用 `POST <base>/translate/batch`。

## 4. 上游解析脚本（D19b）

### 4.1 API 与落盘

| 端点 | 方法 |
| --- | --- |
| `/panel/api/resolver/scripts` | GET |
| `/panel/api/resolver/script` | POST `{"name":"x.py","content":"<源码>"}` → `{"ok":true}` |

脚本落盘于 `<cache>/resolvers/<name>.py`（另生成 `__pycache__`）。

### 4.2 调用契约（实测）

```text

argv=['/app/inst/03/cache/resolvers/probe.py', 'myarg42']
cwd='/app/inst/03/cache/resolvers'
env=[… 无自定义变量,PATH 继承 …]  __name__='__main__'

```

- 命令：`python3 <脚本路径> <resolver_arg>`；`argv[1]` = `resolver_arg`（可为空/缺省）；工作目录 = 脚本目录。**stdout 内容即上游 URL**（频道 `url` 被忽略）。

### 4.3 TTL 与失败

| 场景 | 行为 |
| --- | --- |
| `resolver_ttl=30`，TTL 内多次请求 | 脚本**不再执行**（SWR 缓存命中） |
| TTL 到期 | 再执行一次 |
| 脚本非零退出（有旧值） | `WARN 【代理】resolver SWR 刷新失败 [resolver:gid:cid]: 退出码 3: <stderr>`；仍返回旧清单 |
| 脚本非零退出（冷启动） | `302` → `server.error_url` |
| 脚本超时（>约 30s） | 约 30s 后 `302` → `error_url`，日志无提示 |
| stdout 非 URL | 立即 `302` → `error_url`，日志无提示 |

### 4.4 `resolver_catchup`
`resolver_catchup=true` 时入口订阅写 `catchup="append" catchup-days="<N>" catchup-source="?playseek=…"`。`proxy` 模式下请求 `?playseek=…` **未**追加到上游（面板说明「仅 redirect/直链场景」）。

## 5. 直播 / 回看（D20）

### 5.1 状态接口

```json

{"test1:c1":{"audio_count":0,"dir_bytes":13401322,"last_error":null,"max_gb":0,
 "mode":"proxy","oldest_age_secs":335,"retain_hours":1,"running":true,
 "segment_count":36,"target_duration":4}}

```

- 键为 `"<gid>:<cid>"`；磁盘 `cache/livestream/<gid>_<cid>/`。`livestream=true` + 热重载 → 日志 `【推流】热重载: 自动启动推流`、`【推流】任务启动: test1:c1`。

### 5.2 回看清单

```text

GET /test1/c1.m3u8?playseek=20261018192808-20261018192908     # 本地 +08 时间
GET /test1/c1.m3u8?start=<unix>&end=<unix>

```

```text
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-PLAYLIST-TYPE:VOD
#EXT-X-TARGETDURATION:4
#EXT-X-ALLOW-CACHE:YES
#EXT-X-MEDIA-SEQUENCE:448080744
#EXT-X-PROGRAM-DATE-TIME:2026-10-18T11:28:39Z
#EXTINF:4.000,
/test1/c1/timeshift/c95daa3cf2f25682.png

```

- `playseek` 用**本地时区**解析；用 UTC 值会落回实时清单。越界（超出 `timeshift_hours`）→ 落回实时清单，无报错。
- 回看分片：`/{gid}/{cid}/timeshift/<16hex>.<ext>`，`Content-Type: image/png`，`Cache-Control: private, max-age=3600`，`mpegts`。

### 5.3 入口订阅

```text
#EXTINF:-1 tvg-id="c1" tvg-name="C1" tvg-logo="" group-title="test1" catchup="append" catchup-days="1" catchup-source="?playseek=${(b)yyyyMMddHHmmss}-${(e)yyyyMMddHHmmss}",C1

```

## 6. 未决 / 局限
- Remux 的 `flv` / `mmt` 源容器**未测**（无素材）；其它路径（如容器转换）是否调用 ffmpeg**未穷尽**。
- 录制**多音轨 MKV**、`extra_files`、`note` **未测**；`POST /schedule/{idx}/stop` **未测**。
- DASH 录制对 `type=static` 失败的判据（窗口 vs 切片数）**未定**；`selected_tracks` 与出口清单的关系**未完全确定**（本例 1 视频轨，设子集后入口仍含视频变体）。
- 字幕 `.ass` / SRT→VTT 转换、`[subtitle].compat_mode` 与 `srt_ua_keywords` 分支**未测**（上游已是 WebVTT）；`[subtitle.style]` 注入**未观测到生效**。
- 解析脚本超时阈值**未定值**（观测约 30s）；返回值格式仅验证为**纯 URL**。`resolver_catchup` 在 `redirect`/`passthrough` 模式**未测**；直播回看的 `timeshift_max_gb` 淘汰行为、`dash_timeshift` **未测**。
- 仅 **arm64 + 单实例 + 自建源** 验证。

## 7. 复现

```bash
# 前置 + 源站 + 实例
docker exec iptv82 sh -c 'DEBIAN_FRONTEND=noninteractive apt-get install -y -qq ffmpeg'
docker exec iptv82 ffmpeg -version | head -1
python3 samples/work/dyn-v0.8.2/d16/live-src.py 9988 samples/work/dyn-v0.8.2/d16 /tmp/d16live.log &
docker exec -d iptv82 sh -c 'cd /app && ./iptv-proxy /app/inst/03/config.toml > /app/inst/03/run.log 2>&1'
python3 samples/work/dyn-v0.8.2/cfg03.py tok
B=http://192.168.215.4:19903
# D16 remux：上游必须是连续流
python3 samples/work/dyn-v0.8.2/cfg03.py apply /tmp/p16d.py && python3 samples/work/dyn-v0.8.2/cfg03.py reload
curl -s "$B/gremux/r.m3u8"                # 冷启动 503 "remux warming up"，随后 200
curl -s "$B/gremux/r/seg_000000000000.png" -o /tmp/s.bin   # ffprobe → mpegts/h264+aac
docker exec iptv82 ps aux | grep -c '[f]fmpeg'             # 0（remux 不用 ffmpeg）
# D18 录制（{idx} 是下标）
T=$(python3 samples/work/dyn-v0.8.2/cfg03.py tok)
curl -s -X POST "$B/panel/api/schedule" -H "Authorization: Bearer $T" -H 'Content-Type: application/json' \
  -d '{"name":"d18","group_id":"test1","channel_id":"c1","start_time":"2026-10-18T19:24:29","end_time":"2026-10-18T19:25:49","format":"mkv","status":"pending"}'
curl -s "$B/panel/api/schedule" -H "Authorization: Bearer $T"
ls -la samples/work/dyn-v0.8.2/inst/03/cache/recordings/
curl -s -X DELETE "$B/panel/api/schedule/0" -H "Authorization: Bearer $T"
# D19 字幕：上游多轨主清单
python3 samples/work/dyn-v0.8.2/cfg03.py apply /tmp/pd19.py && python3 samples/work/dyn-v0.8.2/cfg03.py reload
curl -s "$B/g2/c1.m3u8" | grep -E 'SUBTITLES|AUDIO'
curl -s "$B/g2/c1/subs0/index.txt"
# D19b 解析脚本
curl -s -X POST "$B/panel/api/resolver/script" -H "Authorization: Bearer $T" -H 'Content-Type: application/json' \
  --data-binary @<(python3 -c 'import json;print(json.dumps({"name":"probe.py","content":open("samples/work/dyn-v0.8.2/d16/resolver/probe.py").read()}))')
python3 samples/work/dyn-v0.8.2/cfg03.py apply /tmp/pd19b2.py && python3 samples/work/dyn-v0.8.2/cfg03.py reload
curl -s "$B/g2/c1.m3u8" >/dev/null
docker exec iptv82 cat /app/d16/resolver/calls.log
# D20 直播回看
python3 samples/work/dyn-v0.8.2/cfg03.py apply /tmp/pd20.py && python3 samples/work/dyn-v0.8.2/cfg03.py reload
curl -s "$B/panel/api/livestream" -H "Authorization: Bearer $T"
curl -s "$B/test1/c1.m3u8?start=$(( $(date +%s)-120 ))&end=$(( $(date +%s)-60 ))"

```
