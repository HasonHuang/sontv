# sontv

一个用 Go 写的 IPTV 订阅代理服务：把上游 m3u 播放列表抓下来，按需过滤、改写成本站代理链接后吐回给播放器，再由本站统一代理真正的音视频流。

订阅地址里只放**稳定 token**（长期、可吊销），响应体里的每一条子链接都改用**短命临时 token**——上游地址、凭据都不落到第三方域名上。服务只依赖 Go 标准库，无第三方依赖。

> 本仓库是 PHP 版 [mytv](https://github.com/HasonHuang/mytv) 的 Go 重写。

## 快速开始

```bash
curl -fsSL https://raw.githubusercontent.com/HasonHuang/sontv/main/install.sh | bash
```

装到 `/opt/sontv`，支持 Debian / Ubuntu / Alpine 与 amd64 / arm64，下载后按 `checksums.txt` 校验 sha256。Alpine 默认不带 bash，把 `bash` 换成 `sh` 即可。选项与手动安装见[安装](#安装)。

---

## 功能

### 两个端点

| 端点 | 作用 | 认证 |
| --- | --- | --- |
| `GET /sub` | 抓取上游 m3u，过滤 + 改写后返回播放列表 | 只认稳定 token（`token=`） |
| `GET` / `HEAD` `/url` | 代理任意 http(s) 目标；m3u8 会逐行改写，其余流式透传 | 稳定 token（`token=`）或临时 token（`t=`） |

### 播放列表改写

- **链接归一化**：相对路径按上游基准补全成绝对地址，协议相对地址（`//host/path`）补上 scheme，再统一包装成 `本站入口/url?t=…&u=…`。
- **解包第三方代理**：识别 `…/url?u=…` 形态的别站代理链接，默认解包成本站单跳（`unwrap_remote_proxy`），避免双跳套娃，也不再依赖别人的服务器。
- **属性的改写有分寸**：`url-tvg` / `x-tvg-url` / `catchup-source` 只改写「本站形态」的链接，纯第三方直连原样保留——不把凭据送给源站，也不把第三方 EPG 平白拖进本站代理；含 `${...}` 模板的 `catchup-source` 整条不动（模板不能 urlencode）。`URI=`（如 `#EXT-X-KEY`）与资源行同规则处理。
- **过滤**：`filter=` 指定关键字，命中条目名（显示名 + `tvg-name`）即保留，大小写不敏感、子串匹配、多词 OR。
- **格式保真**：换行符（`\r\n` / `\n` / `\r`）逐字节保留；不改写时输出与输入完全一致。

### 凭据与安全

- **只存哈希**：token 表里只有 `sha256(token)`，明文不入库、不入日志。校验路径是无锁的原子读 + 一次 map 查找。
- **临时 token**：形态 `<exp>.<uid>.<sig>`，`sig` 是 `HMAC-SHA256(K, "<exp>.<uid>")`，`K` 取该行完整的 64 位 hash。一条响应只签一枚临时 token，所有子链接共用。
- **删行即吊销**：临时 token 先查 `uid` 是否还在表里，再验签、再查过期——删掉一行，它签出的全部临时 token 立即失效。
- **fail closed**：token 表缺失、格式错或为空时，整站返回 `503`，绝不裸奔；重载失败保留旧表。
- **防呆**：目标指向本站自身（含本机另一监听地址）→ `400`，挡掉自引用死循环。
- **不外泄**：上游重定向由服务端自动跟随（≤5 跳防环），`Location` 不透传给客户端；除 `Range` / `If-Range` 外不转发客户端的其它请求头。

### 资源代理的内存纪律

- 每请求先读 8 KB 探测块（`sync.Pool` 复用），只有正文真的以 `#EXTM3U` 开头才当播放列表改写——不吃「后缀/Content-Type 命中」的坑文件。
- 非 m3u8 一律 `io.Copy` 流式透传，峰值内存 O(32 KB)；`Range` / `If-Range` 原样透传，支持拖动播放。
- m3u8 缓冲硬上限 4 MiB，超限返回 `502`，绝不整段读进内存；上游列表读取上限 8 MiB，超限即拒（不静默截断）。
- 出站不设总超时（总超时会切断 `.ts` 长流），由连接、TLS 握手、响应头各自的超时兜底。

### 运维

- **热重载 token 表**：`SIGUSR1` 信号原子换表，无需重启、不中断长流（Windows 无此信号，只能重启进程）。
- **优雅退出**：`SIGTERM`（systemd/容器）与 `SIGINT`（Ctrl+C）。
- **启动自检**：`-check` 只做配置与 token 表校验，不启动服务。
- **分级日志**：默认 `info`，逐分片的细节沉到 `debug`，排查时才用 `-log-level debug` 打开，详见[日志](#日志)。
- 支持 Linux / macOS / Windows（信号处理按平台分文件构建）。

---

## 安装

### 一键安装

下载 [Releases](https://github.com/HasonHuang/sontv/releases) 里对应架构的产物，装到 `/opt/sontv`：

```bash
curl -fsSL https://raw.githubusercontent.com/HasonHuang/sontv/main/install.sh | bash
```

支持 Debian / Ubuntu / Alpine，会自动认架构（`amd64` / `arm64`）与 libc、按 `checksums.txt` 校验 sha256，下载失败或校验不过就直接退出。

**一条命令装完全套**：补齐配置、缺凭据就生成一条 token、建服务用户、按 init 系统注册服务并启动。装完的东西：

```
/opt/sontv/sontv-go        二进制
/opt/sontv/config.json     配置（docs/config.example.json 的内容）
/opt/sontv/tokens.txt      凭据（缺失时自动生成一条，明文只在终端打印一次）
/etc/systemd/system/sontv.service   或   /etc/init.d/sontv（按 init 系统二选一）
```

重跑脚本是升级：二进制直接覆盖，**已存在的 `config.json` 与 `tokens.txt` 保持不变**——自动生成凭据只在缺失时发生，不会覆盖你手写的 token 表；服务单元每次重写并重启。

常用选项（通过管道传参要放在 `-s --` 之后）：

```bash
# 装指定版本
curl -fsSL .../install.sh | bash -s -- --version v0.1.0

# 换安装目录
curl -fsSL .../install.sh | bash -s -- --dir /usr/local/sontv

# 只装文件，不注册/启动服务（默认是装完就起）
curl -fsSL .../install.sh | bash -s -- --no-service

# 覆盖已有配置（默认保留）
curl -fsSL .../install.sh | bash -s -- --force-config

# 不自动生成 tokens.txt，自己管凭据
curl -fsSL .../install.sh | bash -s -- --no-token

# 自动生成的那条 token 用什么标签
curl -fsSL .../install.sh | bash -s -- --token-label 客厅电视
```

`./install.sh --help` 可看全部选项；同名环境变量（`SONTV_VERSION`、`SONTV_INSTALL_DIR`、`SONTV_SERVICE` …）等价。不带参数直接跑 `./install.sh` 也行，适合先下载再执行。

> **自动生成的 token**：明文只在安装结束时打印一次，`tokens.txt` 里只存 sha256；丢了只能换发，重装不会再次打印。订阅地址形如 `http://<listen>/sub?token=<明文>`，详见[生成 token](#生成-token)。想自己填表就加 `--no-token`。

服务注册会建一个 `sontv` 系统用户（token 表 0600，非属主读不到会直接 503），再按 init 系统二选一：

| init | 写入 | 开机自启 | 热重载 |
| --- | --- | --- | --- |
| systemd | `/etc/systemd/system/sontv.service` | `systemctl enable --now sontv` | `systemctl reload sontv` |
| OpenRC（Alpine/Gentoo） | `/etc/init.d/sontv` | `rc-update add sontv default` + `rc-service sontv start` | `rc-service sontv reload` |

判定依据是 `/run/systemd/system` 是否存在（只在 systemd 真正作为 PID 1 时才有）加 `rc-service` / `openrc-run` 是否可用，所以**纯容器里装了 systemctl 也不会被误判成 systemd**。两个都没有时只装文件并提示手动运行，不留半个坏单元。

OpenRC 服务脚本默认用 `supervise-daemon` 托管（崩溃 5 秒后拉起，日志在 `/var/log/sontv/sontv.log`）；OpenRC 版本太老没有 `supervise-daemon`，或容器内核上 `supervise-daemon` 报 `failed to acquire lock` 起不来时，脚本会当场降级成 `start-stop-daemon` 后台模式重写一遍再启动。

> **Alpine 上用 `| sh` 代替 `| bash`**：Alpine 默认不带 bash。脚本本身是 POSIX sh，`sh` 与 `bash` 都能跑：
>
> ```bash
> apk add --no-cache curl   # 或者直接用 busybox 自带的 wget
> wget -qO- https://raw.githubusercontent.com/HasonHuang/sontv/main/install.sh | sh
> ```
>
> Alpine 容器里没有 systemd，服务走 OpenRC 分支；`rc-update add` 在非交互式容器里可能失败（只提示，不影响手动 `rc-service sontv start`）。

### 从源码构建

需要 **Go 1.24 或更高版本**（纯标准库，无需 `go mod download`）：

```bash
git clone <仓库地址> sontv && cd sontv
go build -o bin/sontv-go ./cmd/sontv-go
```

交叉编译（例如目标为 Linux 服务器）：

```bash
GOOS=linux GOARCH=amd64 go build -o bin/sontv-go ./cmd/sontv-go
```

### 安装到 /opt/sontv

不想用一键脚本时，可以手动摆（最终布局与脚本一致）：
二进制与配置文件同级摆放，放好后直接起即可：

```bash
sudo install -d /opt/sontv
sudo install -m 0755 bin/sontv-go /opt/sontv/sontv-go

# 配置：docs/config.example.json 就是全部缺省值，照抄即零改动启动
sudo install -m 0644 docs/config.example.json /opt/sontv/config.json

# 凭据文件，权限收紧到只有服务账号可读
sudo install -m 0600 tokens.txt /opt/sontv/tokens.txt

cd /opt/sontv && sudo -u sontv ./sontv-go   # 直接起，二进制同级已有 config.json
```

也可以不建配置文件——`-config ""` 直接用内置缺省值启动。

### 生成 token

用一键安装脚本时，`tokens.txt` 缺失会自动生成一条，明文在安装结束时打印一次（`--no-token` 可关掉）。要自己生成等价的一组：

```bash
TOKEN="$(openssl rand -hex 24)"                 # 或任何足够随机的字符串
echo "明文 token: $TOKEN"
printf '%s' "$TOKEN" | shasum -a 256            # macOS
printf '%s' "$TOKEN" | sha256sum                # Linux
```

把输出的 64 位十六进制填进 `tokens.txt`，一行一条：`标签,<64位hex>[,TTL小时]`。注意是**裸字符串的 sha256**，不要带换行——`printf '%s'` 而不是 `echo`。完整格式见[配置 → tokens.txt](#tokenstxt)。

---

## 配置

### config.json

启动时读取一次，改后需重启生效（token 表除外，它可热重载）。**只写需要改的字段即可**——未出现的字段保持缺省值，「只配 listen」这种最小配置文件不会把上游地址清空。

仓库里的 [`docs/config.example.json`](docs/config.example.json) 列出了全部字段及其缺省值，照抄即零改动启动：

```json
{
  "tokens_file": "/opt/sontv/tokens.txt",
  "default_ttl_hours": 24,
  "upstream_m3u": "https://cdn.qd.je/live.m3u",
  "listen": "0.0.0.0:9900",
  "unwrap_remote_proxy": true
}
```

| 字段 | 类型 | 缺省值 | 说明 |
| --- | --- | --- | --- |
| `tokens_file` | string | `/opt/sontv/tokens.txt` | token 表路径 |
| `default_ttl_hours` | int | `24` | 临时 token 的默认有效期（小时）；表里没写第三列的行用它。≤0 时静默回落为 24 |
| `upstream_m3u` | string | `https://cdn.qd.je/live.m3u` | `/sub` 未带 `url=` 时使用的上游播放列表 |
| `listen` | string | `0.0.0.0:9900` | 监听地址。绑 `0.0.0.0` 才能被容器端口转发（`-p 9900:9900` 转发到容器 IP，只听 `127.0.0.1` 会无人应答）；裸机部署因此默认对全网卡开放，鉴权由 token 把关，只想本机可达就显式写 `127.0.0.1:9900` |
| `unwrap_remote_proxy` | bool | `true` | 是否把第三方代理链接解包成本站单跳 |

`tokens_file` 写相对路径时，同样按二进制同级目录解析。

### tokens.txt

每行一条凭据：

```
标签,sha256(token)[,TTL小时]   [ # 行内注释]
```

- **第一列**：人类可读标签，仅用于排障，不参与任何判定。
- **第二列**：`sha256(明文 token)` 的 64 位小写十六进制，**不能重复**。
- **第三列**（可选）：该行临时 token 的 TTL 小时数，取值范围 1~876000；留空或为 0 则回落到 `default_ttl_hours`。
- 空行、以 `#` 开头的整行都是注释；行内的「空白 + `#`」之后也算注释——但 `abc#def` 这种**裸 `#`** 属于标签本身，不是注释。
- 字段两端的空白会被容忍。
- **任何一行出错都会让整张表解析失败**（宁可 `503` 也不接受半张表：漏读一行等于误删一个用户）。

```
# 标签,sha256(token),可选TTL小时
客厅电视,3f2a…（64位hex）
手机,9c1b…,6             # 这条 6 小时
#主号,aaaa…              # 行首 # → 整行禁用
```

### 命令行参数

| 参数 | 缺省值 | 说明 |
| --- | --- | --- |
| `-config` | `config.json` | 配置文件路径；相对路径以二进制所在目录为基准。传空串（`-config ""`）表示全部使用缺省值 |
| `-log-level` | `info` | 日志级别 `debug` / `info` / `warn` / `error`。未指定时读环境变量 `SONTV_LOG_LEVEL`，详见[日志](#日志) |
| `-check` | `false` | 只做配置与 token 表校验，不启动服务 |

缺省就是读二进制同级的 `config.json`，所以把二进制和配置放在一起即可免参数启动。指定了路径但文件不存在或 JSON 非法时，启动失败并退出（退出码非 0）。

token 表装载失败**不会**阻止启动，只在日志里提示——服务照常起，但认证必然 `503`（fail closed）。因此 `-check` 也只保证「配置合法」：token 表有问题时它会打印错误日志，但退出码仍为 0，请以日志内容为准。

### 日志

日志走 stderr，格式为 `时间 级别 [#编号 凭据种类 标签] 正文`：

```
13:28:18.545 DEBUG [#4 临时 repro] 直传完成 字节=4096B 探测头="G@..." 耗时=0ms
13:28:18.539 INFO  [#1 订阅 repro] 改写完成 上游字节=84 出=196B 入口=http://127.0.0.1:9901/sub 过滤=[] 耗时=1ms
13:20:57.025 ERROR [#7 订阅 repro] 上游抓取失败 原因=EOF 耗时=5054ms
```

`[#N]` 是请求编号，把同一次播放的上下游日志串成一条线；`标签` 取自 token 表第一列，分辨是哪个用户在播。

分级是**默认安静、按需全开**——播一路电视每分钟要打几十行分片日志，与启动、改配置、凭据失效这些真正稀有的事混在一起，出问题时反而找不到重点：

| 级别 | 内容 | 一次播放的行数 |
| --- | --- | --- |
| `DEBUG` | 逐请求、逐分片：开始 / 上游响应 / 直传完成 / 改写完成 | ~12 行/分钟 |
| `INFO` | 稀有的状态变化：订阅改写结果、启动、重载、退出 | ~1 行/次订阅 |
| `WARN` | 有人在做无效的事：凭据不对、目标非法、自引用 | — |
| `ERROR` | 链路真的断了：上游抓不到、读不了、列表超限 | — |

排查播放问题时把级别调到 `debug`：

```bash
# systemd：改 ExecStart 加参数，或用 drop-in
systemctl edit sontv          # 写入 [Service] Environment= 或 ExecStart 追加
# 裸机手工跑
./sontv-go -log-level debug
SONTV_LOG_LEVEL=debug ./sontv-go
```

**systemd 部署下请用 `-log-level` 参数。** systemd 给服务进程的是它自己的干净环境，容器里 `docker run -e SONTV_LOG_LEVEL=debug` 传进去的变量**不会**传给 unit 起的进程（实测：PID 1 有，`sontv-go` 没有）。环境变量只在手工跑二进制时可靠。参数优先于环境变量。

日志从不打印凭据明文：目标地址只保留 scheme/host/path 与参数名、参数值一律抹成 `***`，错误只取内层原因（`*url.Error` 会把完整 URL 拼进 `Error()`）。响应正文也从不落盘，只留探测块开头的若干字符用于分辨「HTML 错误页 / 文本提示 / 二进制流」。

---

## 运行

```bash
# 缺省：读二进制同级的 config.json
/opt/sontv/sontv-go

# 也可以显式指定，相对路径同样以二进制所在目录为基准
/opt/sontv/sontv-go -config config.json
/opt/sontv/sontv-go -config etc/sontv.json

# 绝对路径照旧
/opt/sontv/sontv-go -config /etc/sontv/config.json

# 全部缺省值（监听 0.0.0.0:9900）
/opt/sontv/sontv-go -config ""

# 启动前自检，不监听端口
/opt/sontv/sontv-go -check
```

### 信号

| 信号 | 行为 |
| --- | --- |
| `SIGUSR1` | 重新读取 token 表并原子换表；解析失败保留旧表（Linux/macOS） |
| `SIGTERM` / `SIGINT` | 优雅退出 |

```bash
kill -USR1 "$(pidof sontv-go)"   # 改完 tokens.txt 后热重载
```

### systemd 单元示例

`install.sh --service` 生成的就是下面这个（多了 `Group`、`WorkingDirectory` 与几个加固项）：

```ini
[Unit]
Description=sontv-go IPTV subscription proxy
After=network-online.target

[Service]
Type=simple
ExecStart=/opt/sontv/sontv-go -config config.json
ExecReload=/bin/kill -USR1 $MAINPID
Restart=on-failure
RestartSec=3
User=sontv
# 凭据文件权限收紧
UMask=0077
NoNewPrivileges=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

### OpenRC 服务脚本示例

`install.sh --service` 在 Alpine/Gentoo 上写的是 `/etc/init.d/sontv`：

```sh
#!/sbin/openrc-run
name="sontv"
description="sontv - IPTV subscription proxy"

command="/opt/sontv/sontv-go"
directory="/opt/sontv"
command_user="sontv:sontv"
retry="SIGTERM/5"

supervisor=supervise-daemon
output_log="/var/log/sontv/sontv.log"
error_log="/var/log/sontv/sontv.log"
respawn_delay=5
respawn_max=0

depend() {
	need net
	after firewall
}

start_pre() {
	checkpath --directory --owner "sontv:sontv" --mode 0755 /var/log/sontv
}

reload() {
	ebegin "重载 $name"
	start-stop-daemon --signal USR1 --name sontv-go
	eend $?
}
```

`rc-update add sontv default` 加开机自启，`rc-service sontv start|reload|status` 管日常。

### Nginx 反向代理

```nginx
location / {
    proxy_pass http://127.0.0.1:9900;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-Proto $scheme;   # 必须：否则 https 入口下的子链接会写回 http
    proxy_buffering off;                          # 长流不憋在缓冲里
    proxy_read_timeout 3600s;
}
```

`X-Forwarded-Proto` 是**必须**转发的：本站入口的 scheme 由它现推，缺了它会把 https 的子链接播成 http。

---

## 使用

```bash
# 订阅：用配置里的缺省上游
curl "http://127.0.0.1:9900/sub?token=<稳定token>"

# 订阅：指定上游 + 过滤关键字（英文或全角逗号分隔，最多 20 个，每个 ≤64 字节）
curl "http://127.0.0.1:9900/sub?token=<稳定token>&url=https%3A%2F%2Fexample.com%2Flist.m3u&filter=翡翠台,TVB"
```

返回体形如：

```
#EXTM3U
#EXTINF:-1 tvg-name="翡翠台",翡翠台
http://你的域名/url?t=1759257600.3f2a1b0c9d8e7f60.xxxxxxxx&u=https%3A%2F%2Fexample.com%2Flive%2Fa.ts
```

`/sub` 的响应里**永远不含稳定 token**——只有短命临时 token。播放器直接抓这条订阅地址即可，子链接会自动走回本站。

### 状态码

| 状态码 | 含义 |
| --- | --- |
| `200` | 成功 |
| `400` | 目标地址缺失/非法/非 http(s)、上游未配置、目标指向本站自身 |
| `401` | 临时 token 无效（过期、签名不符，或对应的行已被删除） |
| `403` | 稳定 token 无效（缺失或不在表里） |
| `405` | `/url` 收到非 GET/HEAD 请求 |
| `502` | 上游抓取/请求/读取失败，或播放列表超出上限（`/sub` 8 MiB、`/url` 4 MiB） |
| `503` | token 表为空或未能装载——服务未就绪，fail closed |

错误响应统一是一行纯文本，不含服务器信息，也不含上游细节。

---

## 开发

```bash
go test ./...        # 全部测试
go test ./... -race  # 并发检查
go vet ./...
gofmt -l .
```

测试分三层：`internal/*` 是单元测试（含 `playlist` 的纯函数改写矩阵、token 表解析边界），`internal/server` 用 `httptest` 直打 handler 做集成测试（含认证 × 端点的状态码矩阵、时钟注入的过期用例），`internal/server/curl` 真编译、真起进程、真走 TCP 做端到端验证。

### 项目结构

```
cmd/sontv-go/          入口：命令行参数、信号监督、优雅退出
internal/config/       配置装载（JSON 叠加缺省值）
internal/tokens/       token 表解析与原子热重载（只存 sha256）
internal/temptoken/    临时 token 的签发与校验
internal/playlist/     播放列表改写（纯函数，不碰网络）
internal/server/       HTTP 路由、认证、/sub 与 /url 的实现
  └── curl/             端到端测试
docs/                  配置示例（config.example.json）
```

### 设计约束

- **凭据形态分离**：稳定 token 只出现在用户主动配置的订阅地址里；响应体与子链接一律走临时 token。
- **不裸奔**：认证开着却没有可用凭据时拒绝服务，而不是放行。
- **不猜内容**：只有正文真的以 `#EXTM3U` 开头才按播放列表处理，不看后缀、不看 `Content-Type`。
- **不设正文上限的地方就别设**：流式代理不设总超时、不设大小上限；需要缓冲的地方（m3u8 改写）给硬上限并明确报错，绝不静默截断。
