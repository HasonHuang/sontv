# 阶段报告 01 · IPTV Proxy v0.8.2 逆向

> 目标版本：**v0.8.2**（上游提交 `d302316fb8`）｜架构：linux/arm64 + linux/amd64
> 采集日期：2026-10-15 … 2026-10-18 ｜ 环境：macOS（静态）+ linux/arm64 容器（动态）
> 结论级别：逐条标注【观测】/【推断】/【未知】

## 1. 范围与方法

- 样本：`samples/v0.8.2/`（主）、`samples/v0.8.8/`（对照，= upstream `main`）。
- 静态：脱壳（UPX 4.24）→ `strings`/`readelf`；内嵌面板雕刻；路由与配置键盘点。
- 动态：容器 `iptv82`（原生 linux/arm64）；主实例 `:19899` + 6 个隔离实例（`:19901–19906`）。
- 上游源：本地 `python3 -m http.server 9999`（HLS/MPD/XMLTV/TS）；ffmpeg 6.1.1 + python3 3.12。
- 证据纪律：每条结论附命令/字节/响应；目标派生内容按**不可信数据**处理。

## 2. 覆盖矩阵

| 域 | 结论摘要 | 状态 | 证据 |
| --- | --- | --- | --- |
| 样本/构建 | UPX 4.24；Rust 1.98.1；静态非 PIE；版本常量锚定 | ✅ | F-001/F-002 |
| 运行时表面 | `[::]:19899`；`/health`=200；`/status`=404；未知频道→302 占位 | ✅ | F-004 |
| 六种模式 | proxy/remux/redirect/rewrite/dash/passthrough；模式属分组；**rewrite+MPD 损坏**（D-005） | ✅ | F-005/F-014/F-016 |
| 切片伪装 | `<S>`=`sha256(上游切片URL)[:32]`；`<P>` 仅依赖 `(gid,cid)`、参与校验；扩展名不校验 | 🔶 算法未解 | F-004/F-017 |
| 缓存 | 首写 `public,max-age=300`→命中 `private`；flush 保留 `dash/`；四项上限未生效 | ✅ | F-006/F-015 |
| 入口清单/access | public/hidden/archived/open 四档；`open` 不进总订阅；`m3u_header_lines` | ✅ | F-009 |
| Token/安全 | 轮换 + 旧 token ≈1h 宽限；IP 封禁 30 天；登录 429；指标仅面板私有面 | ✅ | F-010 |
| 面板 API | 80+ 端点全集（公开面 + `/panel/api` 私有面） | ✅ | F-010/F-012 |
| EPG | 采集/裁剪/`lang`/匹配算法（tvg-id→name、规范化、多命中取最后） | ✅ | F-008/F-016 |
| 台标库 | 9 端点；浏览器侧转码；`.orig/` 备份；改名/删除同步引用 | ✅ | F-011 |
| 多 VPS 分享 | `GET /api/share/pull?key=`；key+trust_ips；20s 内同步 token/封禁 | ✅ | F-012 |
| 导入/导出/聚合 | 组导出数组；M3U 导入 JSON 文本；merge 仅新增；聚合输出上游原始地址 | ✅ | F-013 |
| 备份/恢复/边缘 | 三 scope；restore 另存 `.bak`；`X-IPP-Edge-Secret`；CF 段刷新 | ✅ | F-013 |
| Remux/录制/字幕/回看 | remux 仅连续流、**不用 ffmpeg**；MKV + `ffmpeg -c copy`；VTT 透传；`playseek` | ✅ | F-014 |
| DASH/ClearKey | MPD→HLS（需 `SegmentTimeline`）；KID 检测但**不解密** | ✅ | F-016 |
| 健康/URL 池 | `GET`+`Range: bytes=0-1`；池仅 DASH；无多 CDN 切换 | ✅ | F-015 |
| 订阅合并 | `/subs/{status,groups,preview}` 形状与过滤语义 | ✅ | 本报告 §4 |
| 资产端点 | `/assets/ffmpeg-status`→`installed:true`；`/assets/list` | ✅ | 本报告 §4 |

## 3. 关键结论（可判定）

1. **伪装链路**：入口 `/{gid}/{cid}.m3u8`（`no-cache`）→ 媒体 `.../video/index.txt`（`text/plain`）→ 切片 `/<P>_<S>.<ext>`。载荷字节不改；`Content-Type` 按扩展名映射；切片不受 token 保护、忽略 `Range`。
2. **`redirect`/`passthrough`** 均 302 上游；`best` 选最高 `BANDWIDTH`；`rewrite` 绝对化后客户端直连。
3. **Remux 与 ffmpeg 无关**：仅接受连续流（HTTP-TS）；HLS 清单永久 503；`503 Retry-After:2` 是预热态。
4. **DASH 转封装**支持 `<BaseURL>`，要求 `SegmentTimeline`；ClearKey 仅识别 KID，不解密。
5. **配置保存**：唯一安全路径是 `PUT /panel/api/config` 全量原子写；文件直改与面板保存有竞态。
6. **分享同步**：20s 周期拉取；follower 禁用本地 token 轮换。
7. **EPG 匹配**：`out_id=tvg_id||id`；名称规范化（删空白+小写）；无子串匹配；多命中取最后。
8. **健康探测**只影响展示，不驱动故障切换（与基线“多 CDN 容错”不符 → `D-004`）。

## 4. 本阶段补充验证（2026-10-18）

| 端点 | 方法/体 | 结果 |
| --- | --- | --- |
| `/panel/api/dash/scan-url` | `POST {url:<MPD>}` | 200 `{tracks:[{bandwidth,codecs,height,width,name,kind,…}]}`；非 MPD→502 |
| `/panel/api/subs/status` | `GET` | 200 `{"sources":[]}` |
| `/panel/api/subs/groups` | `POST {url}` | 200 `{groups:[{name,count}],total}` |
| `/panel/api/subs/preview` | `POST {source:{id,url,group_map}}` | 200 `{kept,sample,trace{total,after_group,after_global_exclude,after_include},dropped_groups,out_groups}`；`unlisted_groups` 默认 `drop` |
| `/panel/api/assets/ffmpeg-status` | `GET` | 200 `{"installed":true,"path":"ffmpeg"}` |
| `/panel/api/assets/list` | `GET` | 200 配置项清单（含 `converted/field/url`） |

## 5. 差异登记（相对基线）

| ID | 差异 | 状态 |
| --- | --- | --- |
| `D-001` | 基线 v0.8.2 vs 上游 `main`=v0.8.8 | ✅ 已用 v0.8.2 产物收敛 |
| `D-002` | 聚合频道 `group-title` = 上游分组名（非本地组 ID） | 🔶 已澄清（以观测为准） |
| `D-003` | remux 503 曾被归因于缺 ffmpeg | ✅ 已更正（预热态） |
| `D-004` | DASH“多 CDN 故障切换”未观测到 | 🔶 已记录 |
| `D-005` | `rewrite` + MPD 正文损坏（逐行绝对化），非基线所述“直连 CDN” | 🔶 已记录 |

## 6. 未解与缺口

| 项 | 影响 | 现状 |
| --- | --- | --- |
| `<P>`（16 hex 频道前缀）算法 | 低：客户端从清单取值，不需自算 | 已排除 12 类哈希/2 万模板（F-017） |
| `S-DIS-08/09` 切片 TS 结构/`lumberjack` 水印 | 低：属上游载荷特征 | 未验（本地伪源无 PAT/SDT） |
| `S-GAP-05` 每频道独立上游代理 | 中：需代理环境 | 字段存在，行为未验 |
| `S-GAP-08` `l1/l2` 限流触发条件 | 低：本环境未触发 | 键面已录 |
| `S-GAP-02` `/epg/api/*` 公开 JSON | 中：v0.8.2 路由存在 | 响应未验 |
| `/unban` 每月 3 次限次 | 低 | 未验 |
| 多音轨 DASH 时移、`keep_yesterday` | 低 | 未验 |

## 7. 复现入口

```bash

python3 -m http.server 9999 --directory samples/work/origin &   # 假源（宿主）
docker start iptv82
docker exec -d iptv82 sh -c 'cd /app && ./iptv-proxy /app/config.toml > /app/run.log 2>&1'
curl -s -X POST http://127.0.0.1:19899/panel/api/login -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"admin"}'
curl -s http://127.0.0.1:19899/test1/c1.m3u8
S=e4b50304d62739a7b6df027f7cfc3155   # F-017：正确前缀 200、错误前缀 302
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:19899/test1/c1/ea8680a1b61c17e4_$S.jpg
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:19899/test1/c1/0000000000000000_$S.jpg

```

## 8. 结论

- 覆盖反代/直连、伪装、缓存、鉴权、EPG、面板、分享、录制/回看、DASH、边缘模式；路线图 P0/P1/P2 项均完成或已定界（`PROGRESS_bak.md` §5）。
- 差异 `D-001…D-005` 已登记；剩余未知均为**低影响**或**环境受限**，不影响重建。可按 `docs/spec/08` 实现规格。
