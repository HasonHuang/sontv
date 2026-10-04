# sontv

sontv 是一个 IPTV 订阅代理服务，用 Go 编写，只依赖标准库。

sontv 抓取上游 m3u 播放列表，按需过滤条目，再把每条链接改写成本站代理地址。播放器从本站取流，本站代理真正的音视频流。

凭据分两种。订阅地址只放稳定 token，长期有效，可随时吊销。响应体中的每条子链接只带短命临时 token。sontv 不回传上游凭据，也不把流地址挂在第三方域名下。

灵感来自 PHP 版 [mytv](https://github.com/HasonHuang/mytv)。sontv 已独立演进，端点形态、参数命名与配置项都按自己的取舍来。

## 快速开始

```bash
curl -fsSL https://raw.githubusercontent.com/HasonHuang/sontv/main/install.sh | bash
```

脚本把 sontv 装到 `/opt/sontv`，支持 Debian、Ubuntu、Alpine 与 amd64、arm64。脚本下载产物后按 `checksums.txt` 校验 sha256。Alpine 默认不带 bash，要把 `bash` 换成 `sh`。

安装后订阅地址形如 `http://<域名>/sub?token=<明文 token>`。取流与过滤的用法见[使用](#使用)，安装选项见[安装](#安装)。

## 使用

### 端点

| 端点 | 作用 | 认证 |
| --- | --- | --- |
| `GET /sub` | 抓取上游 m3u，过滤并改写后返回播放列表 | 只接受稳定 token（`token=`） |
| `GET`、`HEAD` `/play` | 代理 http(s) 目标。m3u8 逐行改写，其余流式透传 | 接受稳定 token（`token=`）或临时 token（`t=`） |
| `GET`、`HEAD` `/proxy` | 代理 http(s) 目标，逐字节原样返回；带 `filter=` 时改为过滤后只返回 URL 清单 | 只接受稳定 token（`token=`） |

### 调用示例

```bash
# 订阅：用配置里的缺省上游
curl "http://127.0.0.1:9900/sub?token=<稳定token>"

# 订阅：指定上游 + 过滤关键字（英文或全角逗号分隔，最多 20 个，每个 ≤64 字节）
curl "http://127.0.0.1:9900/sub?token=<稳定token>&url=https%3A%2F%2Fexample.com%2Flist.m3u&filter=翡翠台,TVB"

# 原样代理：上游给什么就是什么，连 m3u8 都不改写
curl "http://127.0.0.1:9900/proxy?token=<稳定token>&url=https%3A%2F%2Fexample.com%2Flist.m3u"

# 列表模式：按关键字过滤后每条一行只回地址（text/plain），供测活脚本或别的播放器取用
curl "http://127.0.0.1:9900/proxy?token=<稳定token>&url=https%3A%2F%2Fexample.com%2Flist.m3u&filter=CCTV"
```

列表模式返回的是原样上游地址，一行一条，没有 `#EXTINF` 等属性行，也不含任何凭据：

```
http://cdn.example.com/live/1.ts
http://cdn.example.com/live/5.ts
```

返回体形如：

```
#EXTM3U
#EXTINF:-1 tvg-name="翡翠台",翡翠台
http://<域名>/play?t=1759257600.3f2a1b0c9d8e7f60.xxxxxxxx&url=https%3A%2F%2Fexample.com%2Flive%2Fa.ts
```

`/sub` 的响应里永远不含稳定 token，只有短命临时 token。播放器直接抓这条订阅地址即可。

### /play 与 /proxy 的分工

| | `/play` | `/proxy` |
| --- | --- | --- |
| 用途 | 播放链路，子链接回到本站，凭据是临时 token | 通用代理，把 `url=` 指定的内容原样挂在本域名下 |
| m3u8 | 逐行改写，子链接盖临时 token | 不动，上游给什么就是什么（带了 `filter=` 则改为过滤，见下） |
| 正文处理 | m3u8 整段缓冲，上限 4 MiB，其余 `io.Copy` 流式 | 不带 `filter=` 时一律 `io.Copy` 流式，无大小上限 |
| `filter=` | 支持，按关键字过滤条目 | 支持。一带上就从原样透传切成列表模式：抓列表、过滤、每条一行只回地址 |

典型场景是把第三方 m3u8 挂在本域名下给外部播放器直取。这种场合改写是负作用，走 `/proxy` 即可。

`/proxy` 只收稳定 token。它不产生子链接，临时 token 没有指代对象。

### 列表模式（`/proxy?filter=`）

带上 `filter=` 时 `/proxy` 不再原样透传，而是抓下上游的播放列表、过滤、每条一行只回地址。什么时候算「能过滤」，判据只有一条：正文是否以 `#EXTM3U` 开头。不看扩展名，也不看 `Content-Type`——那两样在真实源站上并不可靠。

- 目标不是播放列表时返回 `400`，不静默忽略 `filter`。悄悄把一份未经筛选的完整内容吐回去，是这类功能最坏的失败方式：调用方无从分辨自己到底滤没滤。
- `filter=` 带上了却解析不出词（`filter=`、`filter=%20`、`filter=,,`）同样返回 `400`，而不是退回「不过滤」。
- 返回**原样上游地址**，不盖 token。这份清单的用处是喂给测活脚本、导入别的播放器，包一层本站入口反而碍事。代价是这条路径绕过了本站代理，上游地址对请求方可见——但请求方本就把 `url=` 明文写在请求里，可见性没有新增。
- 相对路径按上游基准补成绝对地址；重定向时以最终地址为基准。
- 同一地址出现多次只留首次出现的一条。
- 过滤后一个都没命中时返回 `200` 加空正文，那不是错误。
- `HEAD` 仍然抓取，以便给出准确的 `Content-Length`，但不写正文。

过滤要看到全部条目，因此这条路径整段缓冲上游列表，上限 8 MiB，超限 `502`。不带 `filter=` 时没有这个上限。

### 播放列表改写

sontv 只改写播放列表中「链接」形态的内容。

- 相对路径按上游基准补全，协议相对地址（`//host/path`）补上 scheme。所有链接统一包装成 `本站入口/play?t=…&url=…`。
- sontv 只把「本站入口 `/play?url=…`」当作自己发出的链接，剥掉旧凭据重盖新的。一份列表反复改写不会套娃。别家代理站的链接原样看待，只在它外面盖上本站的临时 token。
- `url-tvg`、`x-tvg-url`、`catchup-source` 只改写本站形态的链接。含 `${...}` 模板的 `catchup-source` 整条不动，因为模板不能 urlencode。`URI=` 与资源行同规则处理。
- `filter=` 按关键字匹配条目名（显示名与 `tvg-name`），不区分大小写，按子串匹配，多个词之间是 OR。
- 逐字节保留换行符（`\r\n` / `\n` / `\r`）。不改写时，输出与输入完全一致。

### 凭据与安全

- token 表只存 `sha256(token)`，明文既不入库，也不入日志。
- 临时 token 形态是 `<exp>.<uid>.<sig>`。`sig` 为 `HMAC-SHA256(K, "<exp>.<uid>")`，`K` 取该行完整的 64 位 hash。一次响应只签一枚临时 token，所有子链接共用。
- 删行即吊销。sontv 先查 `uid` 是否还在表里，再验签，再查过期。
- fail closed。token 表缺失、格式错或为空时，整站返回 `503`。重载失败时保留旧表。
- 目标指向本站自身（含本机另一监听地址）时返回 `400`。
- 服务端自动跟随上游重定向，最多五跳防环，不把 `Location` 透传给客户端。除 `Range` 与 `If-Range` 外，不转发客户端的其它请求头。

### 内存纪律

- 每请求先读 8 KB 探测块，缓冲区由 `sync.Pool` 复用。只有正文真的以 `#EXTM3U` 开头时才按播放列表改写。sontv 不看后缀，也不看 `Content-Type`。
- 非 m3u8 一律 `io.Copy` 流式透传，峰值内存 O(32 KB)。`Range` 与 `If-Range` 原样透传，因此支持拖动播放。
- m3u8 缓冲硬上限 4 MiB，超限返回 `502`，不静默截断。上游列表读取上限 8 MiB。`/proxy` 的列表模式同样整段缓冲，上限沿用 8 MiB。
- 出站不设总超时，因为总超时会切断 `.ts` 长流。连接、TLS 握手与响应头各自设超时。

### 状态码

| 状态码 | 含义 |
| --- | --- |
| `200` | 成功 |
| `400` | 目标地址缺失、非法、非 http(s)，上游未配置，目标指向本站自身，或 `filter=` 指定的目标不是播放列表 |
| `401` | 临时 token 无效（过期、签名不符，或表里已删除对应的行） |
| `403` | 稳定 token 无效（缺失或不在表里） |
| `405` | `/play` 或 `/proxy` 收到非 GET、HEAD 请求 |
| `502` | 上游抓取、请求、读取失败，或播放列表超出上限 |
| `503` | token 表为空或未能装载，服务未就绪 |

错误响应统一是一行纯文本。sontv 不返回服务器信息，也不返回上游细节。

## 安装

### 一键安装

```bash
curl -fsSL https://raw.githubusercontent.com/HasonHuang/sontv/main/install.sh | bash
```

交互环境下，脚本先探测环境（发行版、架构、init 系统），再逐项询问端口、首个 token、服务名与日志级别，直接回车取默认值。随后补齐配置，缺凭据时生成一条 token，并创建服务用户，最后注册服务并启动。

安装后存在下列文件：

```
/opt/sontv/sontv-go        二进制
/opt/sontv/config.json     配置
/opt/sontv/tokens.txt      凭据
/etc/systemd/system/<服务名>.service   或   /etc/init.d/<服务名>
```

### 传参方式

脚本从标准输入读取，`bash` 之后的参数要用 `-s --` 传递。不写 `-s` 时 bash 会把第一个参数当成脚本名。

```bash
# 正确
curl -fsSL <脚本地址> | bash -s -- --port 8080

# 错误，--port 不会被解析
curl -fsSL <脚本地址> | bash --port 8080
```

每个选项都有短选项与同名环境变量，作用等价。执行 `./install.sh --help` 查看全部选项与全部环境变量。

| 选项 | 短选项 | 作用 | 缺省值 |
| --- | --- | --- | --- |
| `--version TAG` | `-v` | 安装指定版本 | latest |
| `--dir DIR` | `-d` | 安装目录 | `/opt/sontv` |
| `--name NAME` | `-n` | 服务名，决定单元文件名、服务用户名与 OpenRC 日志目录 | `sontv` |
| `--port PORT` | `-p` | 监听端口，写进 `config.json` 的 `listen` | `9900` |
| `--log-level L` | `-l` | 日志级别，写进 `config.json` | `info` |
| `--token TOKEN` | 无 | 指定首个 token 的明文，脚本自己算 sha256 | 随机生成 |
| `--token-label L` | 无 | 自动生成那条 token 的标签 | `我的订阅` |
| `--no-token` | 无 | 不自动生成 `tokens.txt`，自己管凭据 | 不生成 |
| `--flavor F` | 无 | 强制使用 `glibc` 或 `musl` 产物 | 自动探测 |
| `--force-config` | 无 | 覆盖已有的 `config.json` | 保留原文件 |
| `--no-service` | 无 | 只装文件，不注册服务 | 注册并启动 |
| `--no-restart` | 无 | 更新时不重启服务，新版本下次重启后生效 | 重启 |

示例：

```bash
# 装指定版本
curl -fsSL <脚本地址> | bash -s -- --version v0.1.0

# 换安装目录
curl -fsSL <脚本地址> | bash -s -- --dir /usr/local/sontv

# 换端口、服务名、日志级别
curl -fsSL <脚本地址> | bash -s -- --port 8080 --name mytv --log-level debug

# 指定首个 token 与它的标签（明文进，脚本自己算 sha256 存进 tokens.txt）
curl -fsSL <脚本地址> | bash -s -- --token 'my-secret-token' --token-label 客厅电视

# 自己管凭据，不生成 tokens.txt
curl -fsSL <脚本地址> | bash -s -- --no-token

# 只装文件，不注册服务
curl -fsSL <脚本地址> | bash -s -- --no-service

# 覆盖已有配置
curl -fsSL <脚本地址> | bash -s -- --force-config

# 更新二进制但不重启服务
curl -fsSL <脚本地址> | bash -s -- --no-restart
```

用环境变量时写法相同，变量名大写并加 `SONTV_` 前缀：

```bash
SONTV_PORT=8080 SONTV_NO_TOKEN=1 ./install.sh
SONTV_PORT=8080 SONTV_NO_TOKEN=1 curl -fsSL <脚本地址> | bash
```

交互只在能打开 `/dev/tty` 时进行。CI、Docker build 与重定向 stdin 时读不到终端，脚本全部取默认值，不提示也不失败。命令行选项或环境变量给过的项不再询问，因此自动化调用全程无提示。

### 升级

重跑脚本即升级。检测到服务已存在时进入更新流程：只换二进制，`config.json` 与 `tokens.txt` 一律不碰。服务原本在运行就重启它，服务本来是停的则不擅自启动。

端口、日志级别与凭据只在首次安装时询问，升级时不问。要改这些得直接编辑配置文件，或者用 `--force-config` 整份覆盖。自动生成凭据只在文件缺失时发生，脚本不覆盖手写的 token 表。

更新走显式 `restart` 是必要的。`systemctl enable --now` 与 `rc-service start` 对已在运行的服务都是空操作，新覆盖的二进制要等下次重启机器才生效。

### 服务注册

注册服务时，脚本建一个与服务名同名的系统用户。token 表权限是 `0600`，非属主读不到时服务返回 `503`。

| init | 写入 | 开机自启 | 热重载 |
| --- | --- | --- | --- |
| systemd | `/etc/systemd/system/sontv.service` | `systemctl enable --now sontv` | `systemctl reload sontv` |
| OpenRC（Alpine、Gentoo） | `/etc/init.d/sontv` | `rc-update add sontv default` | `rc-service sontv reload` |

脚本按 `/run/systemd/system` 是否存在来判定 systemd，再加 `rc-service` 或 `openrc-run` 是否可用。纯容器里装了 systemctl 时不会误判。两个 init 都没有时，脚本只装文件并提示手动运行。

Alpine 上用 `| sh` 代替 `| bash`，脚本本身是 POSIX sh：

```bash
apk add --no-cache curl   # 或者直接用 busybox 自带的 wget
wget -qO- <脚本地址> | sh
```

### 从源码构建

构建需要 Go 1.24 或更高版本。项目是纯标准库，无需 `go mod download`。

```bash
git clone <仓库地址> sontv && cd sontv
go build -o bin/sontv-go ./cmd/sontv-go
GOOS=linux GOARCH=amd64 go build -o bin/sontv-go ./cmd/sontv-go   # 交叉编译
```

### 手动部署

sontv 要求二进制与配置文件同级摆放，放好后直接启动即可：

```bash
sudo install -d /opt/sontv
sudo install -m 0755 bin/sontv-go /opt/sontv/sontv-go
sudo install -m 0644 docs/config.example.json /opt/sontv/config.json
sudo install -m 0600 tokens.txt /opt/sontv/tokens.txt

cd /opt/sontv && sudo -u sontv ./sontv-go
```

也可以不建配置文件。传 `-config ""`，sontv 直接用内置缺省值启动。

### 生成 token

安装脚本会在 `tokens.txt` 缺失时生成一条 token，明文在安装结束时打印一次。`tokens.txt` 只存 sha256，丢了只能换发。加 `--no-token` 可关掉自动生成。

要自己生成等价的一组：

```bash
TOKEN="$(openssl rand -hex 24)"
printf '%s' "$TOKEN" | shasum -a 256   # macOS；Linux 用 sha256sum
```

sontv 对裸字符串取 sha256，因此用 `printf '%s'` 而不是 `echo`。把输出的 64 位十六进制填进 `tokens.txt`，格式见 [tokens.txt](#tokenstxt)。

## 配置

### config.json

sontv 在启动时读取一次配置，改后需重启生效。token 表除外，它可热重载。只写需要改的字段即可，未出现的字段保持缺省值。

[`docs/config.example.json`](docs/config.example.json) 列出了全部字段及其缺省值，照抄即可零改动启动：

```json
{
  "tokens_file": "/opt/sontv/tokens.txt",
  "default_ttl_hours": 24,
  "upstream_m3u": "https://cdn.qd.je/live.m3u",
  "listen": "0.0.0.0:9900",
  "log_level": "info"
}
```

| 字段 | 类型 | 缺省值 | 说明 |
| --- | --- | --- | --- |
| `tokens_file` | string | `/opt/sontv/tokens.txt` | token 表路径 |
| `default_ttl_hours` | int | `24` | 临时 token 的默认有效期（小时）。表里没写第三列的行用它。≤0 时静默回落为 24 |
| `upstream_m3u` | string | `https://cdn.qd.je/live.m3u` | `/sub` 未带 `url=` 时使用的上游播放列表 |
| `listen` | string | `0.0.0.0:9900` | 监听地址。只让本机可达就显式写 `127.0.0.1:9900` |
| `log_level` | string | `info` | 日志级别 `debug` / `info` / `warn` / `error` |

`tokens_file` 写相对路径时，同样按二进制同级目录解析。

### tokens.txt

每行一条凭据：

```
标签,sha256(token)[,TTL小时]   [ # 行内注释]
```

- 第一列是人类可读标签，只用于排障，不参与判定。
- 第二列是 `sha256(明文 token)` 的 64 位小写十六进制，不能重复。
- 第三列是该行临时 token 的 TTL 小时数，取值 1~876000。留空或为零则回落到 `default_ttl_hours`。
- 空行与以 `#` 开头的整行都是注释。行内的「空白 + `#`」之后也算注释。`abc#def` 这种裸 `#` 属于标签本身。
- 字段两端的空白一律忽略。
- 任何一行出错都会让整张表解析失败。sontv 宁可返回 `503`，因为漏读一行等于误删一个用户。

```
# 标签,sha256(token),可选TTL小时
客厅电视,3f2a…（64位hex）
手机,9c1b…,6             # 这条 6 小时
```

### 命令行参数

| 参数 | 缺省值 | 说明 |
| --- | --- | --- |
| `-config` | `config.json` | 配置文件路径，相对路径以二进制所在目录为基准。传空串表示全部使用缺省值 |
| `-log-level` | 空 | 日志级别。未指定时依次读环境变量 `SONTV_LOG_LEVEL`、配置文件的 `log_level`、缺省 `info` |
| `-check` | `false` | 只做配置与 token 表校验，不启动服务 |

指定了路径但文件不存在或 JSON 非法时，启动失败并退出，退出码非零。

token 表装载失败不会阻止启动，sontv 只在日志里提示。服务照常起，但认证必然返回 `503`。

### 日志

日志写入 stderr，格式为 `时间 级别 [#编号 凭据种类 标签] 正文`：

```
13:20:57.025 ERROR [#7 订阅 repro] 上游抓取失败 原因=EOF 耗时=5054ms
13:28:18.539 INFO  [#1 订阅 repro] 改写完成 上游字节=84 出=196B 过滤=[] 耗时=1ms
13:28:18.545 DEBUG [#4 临时 repro] 直传完成 字节=4096B 耗时=0ms
```

`[#N]` 是全局递增的请求编号，用它把同一次播放的上下游日志串成一条线。标签取自 token 表第一列。

| 级别 | 内容 |
| --- | --- |
| `debug` | 逐请求、逐分片，约 12 行/分钟 |
| `info` | 稀有的状态变化，约一行/次订阅 |
| `warn` | 凭据不对、目标非法、自引用 |
| `error` | 上游抓不到、读不了、列表超限 |

排查播放问题时，把 `log_level` 改成 `debug` 再重启：

```bash
sudo sed -i 's/"log_level": "info"/"log_level": "debug"/' /opt/sontv/config.json
sudo systemctl restart sontv
```

非法取值（例如 `"verbose"`）一律回落到 `info` 并打一条 WARN。服务单元里不需要也不建议出现 `-log-level`，日志级别属于 `config.json`。

日志从不打印凭据明文。目标地址只保留 scheme、host、path，参数值一律抹成 `***`。

日志落在哪由 init 决定：

| 系统 | 落在哪 | 怎么看 |
| --- | --- | --- |
| Debian、systemd | journald | `journalctl -u sontv -f` |
| Alpine、OpenRC | `/var/log/sontv/sontv-go.log` | `tail -f /var/log/sontv/sontv-go.log` |
| 手工前台跑 | 终端 | 直接看，或自行 `2>>` 重定向 |

查 journald 时用 `-g` 而不是管道 `grep`。journald 只能看到日志行本身，`-p err` 这类按优先级过滤对 sontv 不可靠，`-g` 直接匹配 MESSAGE 字段：

```bash
journalctl -u sontv -b -g 'ERROR|WARN'   # 本次启动的异常
journalctl -u sontv -f -g '#7 '          # 跟某个编号的整条播放链路
```

用管道时记得加 `--line-buffered`，否则实时性全丢。

journald 的滚动是容量驱动，不按天切，条目默认不过期。日志不会因为放太久被删，只会因为总量超限被删，淘汰规则近似 LRU。对 IPTV 服务来说缺省的 10% 上限偏大，建议显式收紧：

```ini
# /etc/systemd/journald.conf
[Journal]
SystemMaxUse=200M
MaxRetentionSec=1month
```

Alpine 那边的日志文件没有轮转，会持续增长。长期开着 `debug` 时可以用 logrotate 收敛，注意必须用 `copytruncate`：

```sh
# /etc/logrotate.d/sontv
/var/log/sontv/sontv-go.log {
    rotate 7
    daily
    compress
    missingok
    notifempty
    copytruncate           # 必须，否则 supervise-daemon 的 fd 指向旧 inode，日志静默丢失
    create 0640 sontv root
}
```

## 运行

```bash
/opt/sontv/sontv-go                      # 缺省：读二进制同级的 config.json
/opt/sontv/sontv-go -config etc/sontv.json
/opt/sontv/sontv-go -config ""           # 全部缺省值
/opt/sontv/sontv-go -check                # 启动前自检，不监听端口
```

| 信号 | 行为 |
| --- | --- |
| `SIGUSR1` | 重新读取 token 表并原子换表，不中断长流。解析失败时保留旧表。Windows 没有此信号，只能重启进程 |
| `SIGTERM` / `SIGINT` | 优雅退出 |

```bash
kill -USR1 "$(pidof sontv-go)"   # 改完 tokens.txt 后热重载
```

sontv 支持 Linux、macOS 与 Windows，信号处理代码按平台分文件构建。

### 反向代理

sontv 必须收到 `X-Forwarded-Proto`。本站入口的 scheme 由它现推，缺了它会把 https 的子链接播成 http。

```nginx
location / {
    proxy_pass http://127.0.0.1:9900;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_buffering off;
    proxy_read_timeout 3600s;
}
```

## 开发

```bash
go test ./...
go test ./... -race
go vet ./...
gofmt -l .
```

测试分三层。`internal/*` 是单元测试，含 playlist 的纯函数改写矩阵与 token 表解析边界。`internal/server` 用 `httptest` 直打 handler 做集成测试，含认证与端点的状态码矩阵。`internal/server/curl` 真实编译、启动进程并建立 TCP 连接，做端到端验证。

```
.
├── cmd/sontv-go/          入口：命令行参数、信号监督、优雅退出
│   ├── main.go
│   ├── signal_unix.go
│   └── signal_windows.go
├── internal/
│   ├── config/            配置装载（JSON 叠加缺省值）
│   ├── tokens/            token 表解析与原子热重载（只存 sha256）
│   ├── temptoken/         临时 token 的签发与校验
│   ├── playlist/          播放列表改写（纯函数，不碰网络）
│   └── server/            HTTP 路由、认证、/sub、/play 与 /proxy 的实现
│       └── curl/          端到端测试
└── docs/                  配置示例与部署架构文档
```
