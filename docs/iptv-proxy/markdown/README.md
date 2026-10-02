# 📺 IPTV Proxy 使用文档

IPTV Proxy 是一个直播流代理服务。它把直播源中转一遍，播放器只跟你的服务器通信。

文档基于 **v0.8.2** 实机采集写成。采集日期：2026-10-02。

---

## 三分钟跑起来

在服务器上执行这一条命令：

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/YanG-1989/rust/main/IPTV%20Proxy/iptv-proxy.sh)
```

选择 `1` 开始安装。安装程序询问四个问题。**直接按回车键接受默认值。**

| 问题 | 默认值 |
| --- | --- |
| 端口 | `19899` |
| 账号 | `admin` |
| 密码 | `admin` |

然后在浏览器中打开面板，导入订阅，复制订阅地址。

> 详细步骤见 [订阅接入入门](iptv.md)。执行时间约 15 分钟。

> ⚠️ 安装完成后必须改密码。默认密码是公开的。

---

## 我想做什么

| 你的任务 | 阅读 |
| --- | --- |
| 第一次使用。你手上有订阅地址 | [订阅接入入门](iptv.md) |
| 选择工作模式 | 本文的工作模式速查表 |
| 查某个字段的含义 | [通用字段详解](fields-common.md) |
| 源是 FLV / TS / MMT | [Remux](mode-remux.md) |
| 源是 `.mpd` 文件或带 ClearKey | [DASH](mode-dash.md) |
| 降低带宽费用 | [Redirect](mode-redirect.md) 或 [Rewrite](mode-rewrite.md) |
| 完全信任源地址 | [Passthrough](mode-passthrough.md) |
| 设置 Token 或 IP 封禁 | [面板与运维参考](panel.md) |
| 设置缓存、伪装或备份 | [面板与运维参考](panel.md) |
| 选择服务器规格 | [资源占用实测](resource.md) |
| 查询配置项名称 | [面板与运维参考](panel.md) |
| 更改端口或重置密码 | [面板与运维参考](panel.md) |
| 查看图解版 | [可视化上手页](iptv-proxy-handbook.html) |

---

## 六种工作模式

模式设置在 **分组管理 → 编辑分组 → 工作模式**。同一分组内所有频道共用该模式。

> 项目的 README 文件写有四种模式。面板实际提供六个选项。

| 模式 | 英文名 | 功能 | 使用场景 |
| --- | --- | --- | --- |
| **反向代理** | Proxy | 中转流量并缓存切片 | **通用模式**。不确定时选它 |
| **DASH 转封装** | DASH | 把 MPD 转成 HLS，支持 ClearKey 解密 | 源是 `.mpd` 文件 |
| **Remux** | Remux | 把连续流转封装成 HLS | 源是 FLV / TS / MMT |
| **302 跳转** | Redirect | 返回 302 跳转。流量不经过服务器 | 源稳定。需要降低带宽费用 |
| **URL 改写** | Rewrite | 把清单中的相对地址改成绝对地址 | M3U 导入的默认模式 |
| **直链** | Passthrough | 原样传递地址。不添加 Token | 完全信任源地址 |

**快速判断**：

| 源的类型 | 选择 |
| --- | --- |
| `.mpd` 文件 | DASH |
| `.flv` 或 `.ts` 文件 | Remux |
| 其他 | Proxy |

**带宽费用高时**：选择 Rewrite 或 Redirect。这两种模式的出网流量接近零。

各模式的完整说明：

| 模式 | 文档 |
| --- | --- |
| 反向代理 | [mode-proxy.md](mode-proxy.md) |
| Remux | [mode-remux.md](mode-remux.md) |
| DASH | [mode-dash.md](mode-dash.md) |
| 302 跳转 | [mode-redirect.md](mode-redirect.md) |
| URL 改写 | [mode-rewrite.md](mode-rewrite.md) |
| 直链 | [mode-passthrough.md](mode-passthrough.md) |

---

## 两个常见错误

### 订阅聚合不做反向代理

订阅聚合的频道在总订阅中保留**上游原始地址**。播放器直接连接源站。这些频道无法使用 Token、缓存和录制。

要使用反向代理，请用 **分组管理 → M3U 导入**。

### M3U 导入不接受订阅地址

M3U 导入的窗口只接受粘贴内容和上传文件。窗口内没有填写地址的输入框。

能填写地址的是订阅聚合页。但该页面不做反向代理。

详细说明见 [订阅接入入门](iptv.md)。

---

## 上线前安全清单

逐项确认后再开放公网访问。

- [ ] **更改默认密码**。执行 `iptv-proxy pass <新密码>`，或在面板中修改
- [ ] **防火墙只放行必要的端口**。不需要公网访问时，不要放行 19899
- [ ] **开启 Token 验证和随机频道 ID**。在 `config.toml` 中设置 `token_enabled = true`
- [ ] **开启 IP 管控**。设置并发上限和白名单。将自己的地址加入白名单
- [ ] **多 VPS 同步使用强随机密钥**。使用 IP 白名单限制对端
- [ ] **定期下载完整备份**。在面板的系统设置页下载
- [ ] **设置内网监听地址**。不需要公网访问时

---

## 文档结构

```
docs/iptv-proxy/
├── README.md                    ← 本文件。导航页
├── iptv.md                      ← 订阅接入入门。新用户从这篇开始
├── fields-common.md             ← 六种模式共用的字段
├── mode-proxy.md                ┐
├── mode-remux.md                │
├── mode-dash.md                 ├ 六种工作模式各一篇
├── mode-redirect.md             │
├── mode-rewrite.md              │
├── mode-passthrough.md          ┘
├── resource.md                  ← 资源占用实测
├── panel.md                     ← 面板 13 个页面逐页说明、配置、运维命令
├── iptv-proxy-handbook.html     ← 可视化上手页
└── ui/                          ← 界面截图
```

---

## 已知问题

以下问题在 v0.8.2 中存在。

### 回源流量统计失效

面板概览页的累计回源流量始终显示 0。CPU 累计秒数和内存数值准确。

使用 `docker stats` 查看真实的回源流量。

### 无法调整分组顺序

面板未提供分组排序功能。订阅源的优先级和分组内的频道顺序可以调整。

---

## 项目信息

| 项 | 值 |
| --- | --- |
| 版本 | 0.8.2 |
| 面板端口 | 19899。默认监听所有网卡 |
| 作者 | YanG |
| 上游项目 | https://github.com/YanG-1989/rust/tree/main/IPTV%20Proxy |
| 文档采集日期 | 2026-10-02 |