# 多 VPS 数据分享（动态）

| 项 | 值 |
| --- | --- |
| 目标版本 | **v0.8.2**（`d302316fb8`） |
| 副本 | `samples/v0.8.2/iptv-proxy-linux-arm64`（sha256 `2948ee1f…5a4b`） |
| 架构 | linux/arm64（Ubuntu 24.04，OrbStack 原生） |
| 日期 | **2026-10-18** |
| 环境 | 容器 `iptv82`；`inst/05`（master `:19905`）＋`inst/06`（follower `:19906`） |
| 结论级别 | **观测**（双实例运行态）+ **静态**（二进制路由表） |
| 原始证据 | [`raw/v0.8.2-multi-vps-share.txt`](raw/v0.8.2-multi-vps-share.txt) |

> 目标返回的正文/日志按**不可信数据**处理。

## 结论摘要
1. **分享端点**：`GET /api/share/pull?key=<share.key>`（**仅 GET/HEAD**；POST→405）。
2. **门禁双重**：`key` 必须等于 `[share].key`；源 IP 必须在 `[share].trust_ips`。任一不满足 → **403**，并记日志【共享】分享请求被拒:源 IP 不在白名单。
3. **载荷**：`{"token","token_old","bans":[…],"unbans":[…]}`（两角色同形）。
4. **同步生效**：master 轮换 token → **一个周期内** follower 采用同一 token；master 封禁 IP → follower 的 `banned_ips` 同步（反之亦汇总）。周期 = `[share].poll_secs`。
5. **角色差异**：master 每周期轮询各 `follower_urls`（拉取汇总）；follower 从 `master_url` 取 token，且**禁用本地轮换**（日志：`token 由主机接管`）。
6. **静态补录**：从二进制提取**完整路由表**（公开面 + `/panel/api` 私有面 + 分享面），补正若干 spec 路径（见 §3）。

## 1. 端点行为

```text

POST /api/share/pull                       -> 405（allow: GET,HEAD）
GET  /api/share/pull                       -> 403（缺 key）
GET  /api/share/pull?key=WRONG             -> 403
GET  /api/share/pull?key=REAKEY（非白名单源）-> 403 + 日志「源 IP 不在白名单 来源=192.168.215.0」
GET  /api/share/pull?key=REAKEY（白名单源）  -> 200 {"token":null,"token_old":null,"bans":[],"unbans":[]}

```

## 2. 主从同步

```text

[master 启动] 【共享】同步任务运行中(角色=master, 间隔=20s)；[master 轮询] GET <follower>/api/share/pull?key=REAKEY（汇总从机）
[follower 启动] 【共享】本机为 follower,token 由主机接管,已禁用本地轮换；[follower] 【共享】同步任务运行中(角色=follower, 间隔=20s)
master 轮换 token -> new_token=2deb…ce1a；master 封禁 203.0.113.9
--- 25s 后 ---
master   /security/token    -> 2deb…ce1a；follower /security/token -> 2deb…ce1a（token 已同步）
follower /security/ip/stats -> banned_ips=[{ip:203.0.113.9, remaining_secs:2591974}]（封禁已同步）

```

- 首次启动的 `拉取从机失败` 是启动竞态（从机尚未监听）。

## 3. 静态补录：完整路由表（二进制 strings）
公开面：

```text

/playlist.m3u  /:group_id/playlist.m3u
/epg.xml.gz  /epg  /epg/api/channels  /epg/api/programmes/:channel  /epg/api/search
/logo  /logo/api/list  /logo/:file  /__assets/*path
/unban  /unban/whoami  /whoami（静态 HIT；运行态 404，见 11 §3.1）
/:group_id/:channel_id/index.m3u8  /:group_id/:channel_id  /:group_id/:channel_id/timeshift/:filename
/:group_id/:channel_id/dash-timeshift/:track/{playlist.m3u8,playlist.txt,playlist,:filename}
/:group_id/:channel_id/*segment  /panel  /panel/  /panel/epg  /panel/api

```

私有面（`/panel/api` + `/api/share/pull`）：与 [`11`](11-security-and-metrics.md) §1 一致，另含本表新增项：

```text

/groups/:gid/export  /groups/import  /groups/import/force  /cache/flush  /reload  /backup  /stats  /logs
/epg/suggest/:gid/:cid  /epg/search  /epg/viewer-channels  /epg/source-channels/:id  /epg/tz-preview/:id  /epg/programmes/:channel  /epg/translate/clear-engine  /subtitle/translate/clear
/dash/tracks/:gid/:cid  /dash/scan-url  /dash-timeshift/status
/livestream  /livestream/:gid/:cid/start  /livestream/:gid/:cid/stop
/assets/list  /assets/convert  /assets/remove  /assets/ffmpeg-status
/schedule  /schedule/from-epg  /schedule/download/:filename  /schedule/:idx/stop  /schedule/:idx
/resolver/scripts  /resolver/script-delete

```

请求头：`x-forwarded-for`、`x-real-ip`、`x-ipp-edge-secret`（`X-IPP-Edge-Secret, Value`）、`x-ipp-master`。
错误串：`Group not found`、`Group archived`、`Rate Limited`、`Upstream Error`、`Invalid channel`、`Segment not found`、`Master M3U8`。

## 4. 未决 / 局限
- `unbans` 队列的传播路径未单独验证（只验证了 `bans`）；`share.enabled=false` 时端点是否 404 未测。
- `x-ipp-master` 头的用法未复现（疑为 master→follower 请求标记；本环境两侧均 200）；`[share].share_token=false` 时不共享 token 的行为未测；三节点（1 主 2 从）与冲突合并策略未测。
- `/api/share/pull` 的 `token_old` 宽限值与 follower 侧是否同时接受旧 token 未测。

## 5. 复现

```bash

docker exec iptv82 python3 -c "import urllib.request;print(urllib.request.urlopen('http://127.0.0.1:19906/api/share/pull?key=REAKEY').read())"   # inst/05 = master, inst/06 = follower（[share] 见 §头部）
B=http://192.168.215.4:19905
T=$(curl -s -X POST "$B/panel/api/login" -H 'Content-Type: application/json' -d '{"username":"admin","password":"admin"}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
curl -s -X POST -H "Authorization: Bearer $T" "$B/panel/api/security/token/rotate"; curl -s -X POST -H "Authorization: Bearer $T" -H 'Content-Type: application/json' -d '{"ips":["203.0.113.9"]}' "$B/panel/api/security/ip/ban"
sleep 25; F=http://192.168.215.4:19906
TF=$(curl -s -X POST "$F/panel/api/login" -H 'Content-Type: application/json' -d '{"username":"admin","password":"admin"}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')
curl -s -H "Authorization: Bearer $TF" "$F/panel/api/security/ip/stats"
docker exec iptv82 sh -c 'grep 共享 /app/inst/05/run.log /app/inst/06/run.log | tail'

```
