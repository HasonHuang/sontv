# 台标库管理（动态）

| 项 | 值 |
| --- | --- |
| 目标版本 | **v0.8.2**（`d302316fb8`） |
| 副本 | `samples/v0.8.2/iptv-proxy-linux-arm64`（sha256 `2948ee1f…5a4b`） |
| 架构 | linux/arm64（Ubuntu 24.04，OrbStack 原生） |
| 日期 | **2026-10-18**（初测）；**2026-10-24**（动画补测） |
| 环境 | 容器 `iptv82` 主实例 `:19899`；配置 `samples/work/dyn-v0.8.2/config.toml` |
| 结论级别 | **观测**（运行态）+ **静态**（面板 JS / 配置段） |
| 原始证据 | [`raw/v0.8.2-logo-library.txt`](raw/v0.8.2-logo-library.txt) |

> 目标返回的正文/日志按**不可信数据**处理。

## 结论摘要
1. 台标库共 **9 个端点**：`GET /logo/{list,match-report}`、`POST /logo/{upload,apply,rename,delete,restore,suggest,animate}`，另有公开取图 `GET /logo/<file>`。
2. 台标文件存 `cache/logos/<name>.<ext>`；**原图备份**在 `cache/logos/.orig/`（生成动效时自动写入）。允许扩展名：`png/jpg/jpeg/webp/bmp/avif`。
3. **上载转码在浏览器侧**（`canvas.toBlob`→WebP）：服务端上载不转码，PNG 仍存 PNG。动效生成时服务端**读取 `max_size` 缩放**，但忽略 `quality` 与 `anim_frame_ms`。
4. **改名会同步刷新频道引用**（`updated_channels`）；**删除可选清引用**（`clear_refs`）。
5. **动效为纯 Rust 无损动画 WebP**（`.orig/` 备份 + 覆盖原文件）。输出固定 10 帧：前 9 帧 80 ms，末帧 1500 ms，`total_ms=2220`。`quality` 必须是**字符串**，但目标不读取该值；`[logo].anim_frame_ms` 也不读取。6 种效果：`diagonal/vertical/bounce/spot/glow/breathe`。
6. 公开取图：`200` + `cache-control: public, max-age=86400`。`content-type` 由**扩展名**决定：`.png` 返回 `image/png`，即使文件内容是动画 WebP；`.webp` 返回 `image/webp`。
7. 模糊匹配打分：精确 `100`、前缀 `80`（`POST /logo/suggest`）。`logo/list` 的 `used_by[].via` 区分 `name`（按 `logo_name` 自动匹配）与 `logo`（显式写入频道字段）。`animated=true` 只用于**扩展名为 `.webp` 且内容含动画块**的文件。

## 1. 端点与数据形态

| 端点 | 请求 | 响应要点 |
| --- | --- | --- |
| `GET /logo/list` | — | `allowed_exts[]`、`logos[]`（`file,name,ext,bytes,mtime,url,animated,has_original,used_by[]`）、`channels_without_logo[]`（`gid,cid,name,logo_name,logo,external`） |
| `GET /logo/match-report` | — | `logos[]`（`file,name,url,animated,match_kind,channel`）、`missing[]`（含 `suggest`） |
| `POST /logo/upload` | `{"items":[{"name","ext","data":"data:image/…;base64,…"}]}` | `{"ok":true,"added":[{file,name,url,bytes,animated,anim_fixed}],"failed":[{name,error}]}` |
| `POST /logo/apply` | `{"file","gid","cid"}` | `{"ok":true,"applied":N,"errors":[]}`；写入频道 `logo` |
| `POST /logo/rename` | `{"file","new_name"}` | `{"ok":true,"file","url","updated_channels":N}` |
| `POST /logo/delete` | `{"file","clear_refs":bool}` | `{"ok":true,"removed":N,"cleared_channels":N,"errors":[]}` |
| `POST /logo/restore` | `{"file"}` | `{"ok":true}`；无备份 → `{"error":"没有找到原图备份"}` |
| `POST /logo/suggest` | `{"hint"}` | `{"suggestions":[{channel,cid,gid,group,has_logo,logo_name,score}]}` |
| `POST /logo/animate` | `{"file","effect","quality":"<1..100 字符串>"}` | `{"ok":true,"frames","width","height","total_ms","bytes_before","bytes_after","dirty_ratio"}` |
| `GET /logo/<file>` | — | `200`、`content-type: image/png`、`cache-control: public, max-age=86400` |

- `upload` 每项**必须**带 `ext`（否则 `missing field 'ext'`）；`data` 为 data URL；`animate` 的 `quality` 是**字符串**（整数报 `invalid type: integer, expected a string`）。

## 2. 与播放清单的衔接

```text

apply C1.png → test1/c1:  #EXTINF:-1 tvg-id="c1" tvg-name="C1" tvg-logo="http://127.0.0.1:19899/logo/C1.png" group-title="test1",C1
rename C1.png → "C1 HD":  tvg-logo="…/logo/C1%20HD.png"（文件名 URL 编码）
delete clear_refs=true:   tvg-logo=""（回到空）

```

台标 URL 是**本服务公开地址**，播放器直接取（免登录）。

## 3. 动效生成

```text

动画前：cache/logos/ZZnomatch.png = 176 B
animate spot quality=80 → frames=10 64×64 total_ms=2220 bytes_before=176 bytes_after=2752 dirty_ratio=20.0
动画后：cache/logos/ZZnomatch.png = 2752 B（覆盖）+ cache/logos/.orig/ZZnomatch.png = 176 B（原图）
list：has_original=true；animated=false（文件扩展名仍为 .png）
再次 animate → 400 {"error":"源图已经是动画,请先用静态图生成"}
restore → {"ok":true}（用 .orig 还原）

```

- 效果与文案：`diagonal` 斜向扫光 / `vertical` 竖向扫光 / `bounce` 来回扫光 / `spot` 光点掠过 / `glow` 边缘辉光 / `breathe` 呼吸明暗。生成在服务端，**逐帧编码**故耗时秒级；文件名与扩展名不变。

### 3.1 容器与帧

| 项 | 观测值 |
| --- | --- |
| 容器 | RIFF/WebP：`VP8X` + `ANIM` + 10 × `ANMF` |
| `VP8X` | flags 恒为 `0x12`（alpha + animation），即使图像全不透明 |
| `ANIM` | 背景 `0x00000000`；循环次数 `0` |
| `ANMF` | flags `0x02`（no blend + no dispose）；`x/y` 以偶数存储 |
| 帧载荷 | `VP8L` 无损 WebP；目标链接 `image-webp 0.2.4`、`png 0.17.16` |
| 帧数/时长 | 10 帧；第 0–8 帧 80 ms，第 9 帧 1500 ms；`total_ms=2220` |
| 忽略项 | `[logo].anim_frame_ms` 改成 40/200 并重启，时长仍为 80/…/1500；`quality` 也不影响输出 |

### 3.2 缩放与脏矩形

- 缩放：最长边缩到 `[logo].max_size`，保持宽高比。`800×400` + `max_size=512` 得到 `512×256`。缩放采样接近角对齐双线性；逐像素一致性未证明。
- 第 0 帧为整幅画布。后续帧只编码与前一帧不同的矩形。
- 脏矩形 `x/y` 向下取偶数；宽高可为奇数。无变化帧用 `(0,0,2,2)`。
- `dirty_ratio = round(rect_area_sum × 100 / (帧数 × 画布面积))`，结果为整数。样本：`55.0`、`13.0`、`29.0`、`80.0`。

### 3.3 效果证据

- 效果只改不透明像素；全透明像素保持不变。
- 增亮按 6 级量化：`level × 0.11`，最大 `0.55`。
- `breathe`：全图不透明像素按 `[0,2,3,4,5,5,4,3,2,0]` 增亮；不改 alpha。
- `glow`：从边缘向内 2–3 像素；基础级别 `[0,1,2,3,3,3,3,2,1,0]`；外圈级别约 `max(1, base - distance)`。
- `vertical`：光带中心约 `(frame-1) × height / 7`；第 0/9 帧无效果。
- `diagonal`：光带方向约为 `x + y/4`；`bounce` 是同方向往返。
- `spot`：圆形光斑从左上向右中移动，半径约最长边 15%。
- 精确像素公式尚未完全证明。当前实现已对齐容器、帧时长、脏矩形、缩放、透明像素跳过和级别量化；`breathe` 输出与目标逐字节一致，其余效果为近似。

## 4. 相关配置段

| 段 | 值 |
| --- | --- |
| `[panel]` | `enabled,username,password_sha256,session_ttl=86400` |
| `[logo]` | `public_enabled=true,max_bytes=2097152,anim_frame_ms=80,max_size=512,quality=80,convert_default=true` |
| `[assets]` | `map{}` |
| `[subtitle]` | `compat_mode=auto,srt_ua_keywords[…],style{…},translate{…}` |
| `[share]` | `enabled,role,key,trust_ips[],master_url,follower_urls[],share_token,poll_secs=300` |
| `[playlist]` | `merge_ttl_secs,merge_position,sources[],global_exclude[]` |
| `[stats]` | `enabled,online_window_secs=90` |

## 5. 未决 / 局限
- `[logo].max_bytes=2 MiB` 的**上载上限行为**未测（未做超限样本）；`convert_default`/`max_size`/`quality` 的**浏览器侧**行为属面板实现，未在浏览器中验证。
- `diagonal`/`vertical`/`bounce`/`spot`/`glow` 的逐像素公式只做了小图采样；缩放后的效果路径、超大图耗时、`quality` 边界值未逐项验证。
- `logo/suggest` 的评分算法只观测到 2 个样本（100/80），完整规则未定；未测 `Range` 请求与 `If-Modified-Since`（与切片不同，台标是静态文件）；仅 arm64，未做大批量（数百张）台标库的列表/匹配性能测试。

## 6. 复现

```bash

B=http://127.0.0.1:19899
T=$(curl -s -X POST "$B/panel/api/login" -H 'Content-Type: application/json' -d '{"username":"admin","password":"admin"}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
AUTH="Authorization: Bearer $T"
curl -s -H "$AUTH" "$B/panel/api/logo/list"; curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"items":[{"name":"C1","ext":"png","data":"data:image/png;base64,<…>"}]}' "$B/panel/api/logo/upload"
curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"file":"C1.png","gid":"test1","cid":"c1"}' "$B/panel/api/logo/apply"; curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"file":"C1.png","effect":"spot","quality":"80"}' "$B/panel/api/logo/animate"
curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"file":"C1.png","new_name":"C1 HD"}' "$B/panel/api/logo/rename"; curl -s -X POST -H "$AUTH" -H 'Content-Type: application/json' -d '{"file":"C1 HD.png","clear_refs":true}' "$B/panel/api/logo/delete"
curl -s -D - -o /dev/null "$B/logo/C1.png"

```
