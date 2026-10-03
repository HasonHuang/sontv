# 切片伪装的实现方式

本文记录 IPTV Proxy v0.8.2 的实测结果。测试在 `localhost:19899` 上进行。样本是面板自带的 `test` 分组。

每条结论都附验证方法。你可以逐条复现。

> ⚠️ 本文分析的是面板自带的测试源。第三方直播源的实现可能不同。

采集日期：2026-10-03。

---

## 结论

**切片伪装不修改切片内容。** 程序只改写 URL 后缀和 Content-Type。切片的每一个字节都保持原样。

伪装发生在四层。四层都不改变载荷。

| 层 | 值 | 是否改变字节 |
| --- | --- | --- |
| URL 后缀 | `png` `jpg` `gif` `bmp` `webp` | 否 |
| Content-Type | `image/png` `image/jpeg` `image/webp` `image/bmp` | 否 |
| 路径形状 | 清单写成 `.txt`，切片平铺在频道目录下 | 否 |
| 切片载荷 | 原始 MPEG-TS，188 字节一个包 | 否 |

**切片不是图片。程序也没有把切片编码进图片。** 这一点与多数人的预期相反。

---

## 一、地址链路

从订阅地址到切片，一共经过三层 HTTP 请求。

```text
http://localhost:19899/playlist.m3u8
  └─ 第 1 条 T1
     └─ http://localhost:19899/test/t1.m3u8?token=29c6b95a...
        │  Content-Type: application/vnd.apple.mpegurl
        │  这是一份主清单（master）
        └─ http://localhost:19899/test/t1/video/index.txt
           │  Content-Type: text/plain; charset=utf-8
           │  这是一份媒体清单（media），伪装成了 .txt
           └─ http://localhost:19899/test/t1/0d9ec19b3b78ba7b_18e34bfbb515537d2bc7054e33d8ecfc.png
              Content-Type: image/png
              这是一段原始 MPEG-TS
```

注意主清单的地址没有被伪装。`/test/t1.m3u8` 仍以 `.m3u8` 结尾。

`panel.md` 的「主列表伪装」开关控制的不是这一层。

### 媒体清单的字段

| 字段 | 值 |
| --- | --- |
| `#EXT-X-VERSION` | `3` |
| `#EXT-X-PLAYLIST-TYPE` | `VOD` |
| `#EXT-X-TARGETDURATION` | `11` |
| 切片数量 | `64` |
| 单片时长 | `10.000`，末片 `4.567` |
| `#EXT-X-MAP` | 无 |

清单中没有 `#EXT-X-MAP`。这个字段缺省时，切片必须自带初始化数据。

因此切片只能是 MPEG-TS，不能是 fMP4。

### 扩展名分布

64 个切片的扩展名分布如下。

| 扩展名 | 数量 |
| --- | --- |
| `.webp` | 18 |
| `.gif` | 13 |
| `.jpg` | 13 |
| `.png` | 12 |
| `.bmp` | 8 |

---

## 二、验证：切片不是图片

下载四种后缀的同一切片。逐字节比较文件头。

```bash
BASE="http://localhost:19899/test/t1/0d9ec19b3b78ba7b_"
curl -s -o s.png  "${BASE}18e34bfbb515537d2bc7054e33d8ecfc.png"
curl -s -o s.jpg  "${BASE}fbce5e1dfefa33e411404eb2761dfa46.jpg"
curl -s -o s.webp "${BASE}1f765d71057f557680705002541565e6.webp"
curl -s -o s.bmp  "${BASE}2f2a80f17383508119e1ba933f79174d.bmp"

file s.png s.jpg s.webp s.bmp
for f in s.png s.jpg s.webp s.bmp; do xxd -l 32 "$f"; done
```

`file` 对四个文件的判定结果相同。

```text
s.png:  data
s.jpg:  data
s.webp: data
s.bmp:  data
```

四个文件的前 32 字节逐字节相同。

```text
47401110 0042f02a 0001c100 000001ff 0001fc80 19481701 0a6c756d6265726a
```

首字节是 `0x47`。这是 MPEG-TS 的同步字节。

### 三项旁证

| 检查项 | 结果 |
| --- | --- |
| 每 188 字节是否都是 `0x47` | 是。28256 个包，命中率 100% |
| 文件大小是否为 188 的倍数 | 是。5312128 = 188 × 28256 |
| 是否含图片或 fMP4 特征 | 否 |

如果文件是图片或 fMP4，你会看到下列特征。四个文件一个都没有。

| 格式 | 特征字节 | 是否存在 |
| --- | --- | --- |
| JPEG | `FF D8 FF` | 否 |
| PNG | `89 50 4E 47` | 否 |
| WebP | `RIFF` | 否 |
| BMP | `42 4D` | 否 |
| fMP4 | `ftyp` `moov` `moof` | 否 |

### 解码器可以直接播放

把 `.png` 改名为 `.ts`。ffprobe 不需要任何提示。

```bash
cp s.png ref.ts
ffprobe -v error -show_streams -show_format ref.ts
```

```text
codec_name=h264   codec_type=video   width=1920   height=1080
codec_name=aac    codec_type=audio
nb_streams=2      format_name=mpegts
duration=10.000011
```

解码器识别出 MPEG-TS。它没有识别出任何图片格式。

---

## 三、后缀轮换的规律

轮换结果**稳定**且**按频道独立**。

重复拉取同一份清单，扩展名序列不变。

```bash
for i in 1 2; do
  curl -s "http://localhost:19899/test/t1/video/index.txt" \
    | grep -oE '\.[a-z]+$' | head -8 | tr '\n' ' '; echo
done
```

```text
.png .jpg .webp .jpg .bmp .webp .webp .jpg
.png .jpg .webp .jpg .bmp .webp .webp .jpg
```

同一个切片哈希在不同频道下拿到不同后缀。

| 频道 | 切片哈希 | 扩展名 |
| --- | --- | --- |
| `t1` | `18e34bfbb515537d2bc7054e33d8ecfc` | `.png` |
| `t2` | `18e34bfbb515537d2bc7054e33d8ecfc` | `.webp` |

因此后缀由「频道 ID + 切片哈希」推导。推导结果是确定的。

---

## 四、后缀不参与校验

后缀只是一个标签。服务端不校验。

`/test/t1/` 的清单中，该切片写作 `.png`。你请求 `.jpg` 也能拿到同一份内容。

```bash
H=18e34bfbb515537d2bc7054e33d8ecfc
curl -s -o /dev/null -w "%{http_code} %{size_download} %{content_type}\n" \
  "http://localhost:19899/test/t1/0d9ec19b3b78ba7b_${H}.png"
curl -s -o /dev/null -w "%{http_code} %{size_download} %{content_type}\n" \
  "http://localhost:19899/test/t1/0d9ec19b3b78ba7b_${H}.jpg"
```

```text
200 5312128 image/png
200 5312128 image/jpeg
```

两次请求的字节数相同。MD5 也相同。

```text
2aeaaec995b0b757e003751021303a14
```

跨频道的同一切片哈希，MD5 同样是这个值。

**推论：想核对某个切片，直接去掉扩展名再换任意后缀即可。**

---

## 五、缓存窗口

这是伪装的真正目的。

`Cache-Control` 不是固定值。

| 时机 | 响应头 |
| --- | --- |
| 切片刚写入缓存 | `cache-control: public, max-age=300` |
| 之后 | `cache-control: private, max-age=300` |

实测记录。

```text
public, private, private, private, private, private   ← 全新切片连打 6 次
```

窗口很短，且随时间变化。它不是固定的请求次数。

公开窗口让 CDN 按图片规则收下这个对象。随后的 `private` 让 CDN 不再存储它。

这样 CDN 命中一次真实用户的取片，之后的匿名重复请求不计入 CDN 的滥用统计。

> 没有 CDN 时，这层收益不存在。详见 [`panel.md`](panel.md) 的「前置条件」。

### 其他响应头特征

| 项 | 实测值 |
| --- | --- |
| `Accept-Ranges` | 无 |
| `ETag` | 无 |
| `Last-Modified` | 无 |
| `Range` 请求 | 忽略。返回 `200` 和完整内容 |

不支持 Range 会影响按需起播的部分播放器。

---

## 六、切片内部结构

每个切片是一份**自包含**的 MPEG-TS。它自带节目专用信息表。

以 `.png` 切片为例。

| PID | 用途 | 包数 |
| --- | --- | --- |
| `0x0000` | PAT | 1 |
| `0x0011` | SDT | 1 |
| `0x0100` | PMT | 1 |
| `0x0101` | AAC 音频 | 1068 |
| `0x0102` | H.264 视频（含 PCR） | 27185 |

### 表结构

| 表 | 内容 |
| --- | --- |
| PAT | `transport_stream_id = 1`，节目 1 映射到 PMT `0x0100` |
| PMT | PCR 在 `0x0102`，节目信息长度 0 |
| PMT 流 1 | `stream_type = 0x0F`（AAC），PID `0x0101` |
| PMT 流 2 | `stream_type = 0x1B`（H.264），PID `0x0102` |

### PID 重排

本流的 PID 布局与 ffmpeg 默认值不同。

| 项目 | ffmpeg 默认 | 本流 |
| --- | --- | --- |
| PMT | `0x1000` | `0x0100` |
| 视频 | `0x0100` | `0x0102` |
| 音频 | `0x0101` | `0x0101` |
| PCR | 视频 | 视频 |

重排 PID 会改变流的指纹。指纹相同的流容易被聚类统计。

---

## 七、lumberjack 水印

每个切片的第 0 个包是一个结构合法的 SDT。

| 字段 | 值 |
| --- | --- |
| PID | `0x0011` |
| `table_id` | `0x42` |
| `section_length` | `42` |
| `transport_stream_id` | `1` |
| `service_id` | `1` |
| `running_status` | `4` |
| 描述符标签 | `0x48`，长度 23 |

描述符载荷是 ASCII 字符串 `lumberjack`，出现两次。

```text
0a 6c 75 6d 62 65 72 6a 61 63 6b 0a 6c 75 6d 62 65 72 6a 61 63 6b
     l   u   m   b   e   r   j   a   c   k   l   u   m   b   e   r   j   a   c   k
```

前面各有一个字节 `0x0a`，长度为 10。其后是 CRC `b537dca2`。包尾用 `0xFF` 填充。

### 分布

| 文件 | 偏移 25 | 偏移 36 |
| --- | --- | --- |
| `t1` 的 `.png` 切片 | 有 | 有 |
| `t1` 的 `.jpg` 切片 | 有 | 有 |
| `t1` 的 `.webp` 切片 | 有 | 有 |
| `t1` 的 `.bmp` 切片 | 有 | 有 |
| `t2` 的 `.webp` 切片 | 有 | 有 |
| ffmpeg 生成的对照流 | 无 | 无 |

**这个水印与图片伪装无关。** 它标记流的来源，不影响播放。

---

## 八、术语表

本文统一使用下列术语。

| 术语 | 含义 | 英文 |
| --- | --- | --- |
| 切片 | 一段约 10 秒的媒体数据 | segment |
| 主清单 | 指向多条码率流的清单 | master playlist |
| 媒体清单 | 列出全部切片的清单 | media playlist |
| 伪装 | 改写 URL 后缀与响应头 | disguise |
| 上游 | 提供数据的源站 | upstream |

---

## 九、排错

### 播放器报「格式不支持」

抓包看到的是 `.jpg`。实际内容是 MPEG-TS。

先关闭「切片伪装」和「列表伪装」。再复测。

关闭后切片恢复为 `.ts`。若仍失败，问题在源本身。

### 确认某个 `.jpg` 是不是切片

```bash
curl -s -o seg.bin "https://你的域名/.../xxxx.jpg"
xxd -l 8 seg.bin
file seg.bin
```

首字节是 `47` 且 `file` 判定为 MPEG-TS，则它是切片。

### 验证 CDN 是否收下了切片

在 CDN 后台查缓存命中率。伪装开启时，命中率应明显高于关闭时。

---

## 相关文档

| 文档 | 内容 |
| --- | --- |
| [`README.md`](README.md) | 文档导航 · 模式速查 · 安全清单 |
| [`panel.md`](panel.md) | 切片伪装的配置项与操作步骤 |
| [`mode-proxy.md`](mode-proxy.md) | 反向代理模式的缓存与回看 |
| [`mode-dash.md`](mode-dash.md) | DASH 转封装 |
| [`resource.md`](resource.md) | 资源占用实测 |
