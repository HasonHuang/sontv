# sontv

sontv 是一个 IPTV 订阅代理服务，用 Go 编写。sontv 抓取上游 m3u 播放列表，按需过滤条目。sontv 把每条链接改写成本站代理地址，再返回给播放器。播放器从本站取流，本站代理真正的音视频流。

订阅地址只放**稳定 token**。稳定 token 长期有效，可以随时吊销。响应体中的每条子链接只带**短命临时 token**。本站不回传上游凭据，也不把流地址挂在第三方域名下。sontv 只使用 Go 标准库，不引入第三方依赖。

> 本仓库是 PHP 版 [mytv](https://github.com/HasonHuang/mytv) 的 Go 重写。

## 快速开始

```bash
curl -fsSL https://raw.githubusercontent.com/HasonHuang/sontv/main/install.sh | bash
```

脚本把 sontv 装到 `/opt/sontv`。脚本支持 Debian / Ubuntu / Alpine 与 amd64 / arm64。脚本下载产物后按 `checksums.txt` 校验 sha256。Alpine 默认不带 bash，请把 `bash` 换成 `sh`。安装选项与手动安装步骤见[安装](#安装)。

---

## 功能

### 两个端点

| 端点 | 作用 | 认证 |
| --- | --- | --- |
| `GET /sub` | 抓取上游 m3u，过滤并改写后返回播放列表 | 只接受稳定 token（`token=`） |
| `GET` / `HEAD` `/url` | 代理任意 http(s) 目标。m3u8 逐行改写，其余流式透传 | 接受稳定 token（`token=`）或临时 token（`t=`） |

### 播放列表改写

sontv 只改写播放列表中「链接」形态的内容。

- **补全绝对地址**：相对路径按上游基准补全。协议相对地址（`//host/path`）补上 scheme。sontv 把链接统一包装成 `本站入口/url?t=…&u=…`。
- **解包第三方代理**：sontv 识别 `…/url?u=…` 形态的别站代理链接。sontv 默认把它们解包成本站单跳（`unwrap_remote_proxy`）。sontv 因此避免了双跳套娃，也不再依赖别人的服务器。
- **改写有分寸**：`url-tvg` / `x-tvg-url` / `catchup-source` 只改写「本站形态」的链接。纯第三方直连原样保留，本站因此不把凭据送给源站，也不把第三方 EPG 拖进本站代理。含 `${...}` 模板的 `catchup-source` 整条不动，因为模板不能 urlencode。`URI=`（如 `#EXT-X-KEY`）与资源行同规则处理。
- **按关键字过滤**：`filter=` 指定关键字。sontv 匹配条目名（显示名 + `tvg-name`），命中即保留。匹配不区分大小写，按子串匹配，多个词之间是 OR。
- **保留格式**：sontv 逐字节保留换行符（`\r\n` / `\n` / `\r`）。不改写时，输出与输入完全一致。

### 凭据与安全

- **只存哈希**：token 表只存 `sha256(token)`。明文既不入库，也不入日志。校验路径是一次无锁的原子读加一次 map 查找。
- **临时 token**：形态是 `<exp>.<uid>.<sig>`。`sig` 为 `HMAC-SHA256(K, "<exp>.<uid>")`，`K` 取该行完整的 64 位 hash。一次响应只签一枚临时 token，所有子链接共用。
- **删行即吊销**：sontv 先查 `uid` 是否还在表里，再验签，再查过期。删掉一行，它签出的全部临时 token 立即失效。
- **fail closed**：token 表缺失、格式错或为空时，整站返回 `503`。本站绝不裸奔。重载失败时保留旧表。
- **拒绝自引用**：目标指向本站自身（含本机另一监听地址）时返回 `400`。sontv 借此挡掉自引用死循环。
- **不回传上游细节**：sontv 在服务端自动跟随上游重定向，最多 5 跳防环。sontv 不把 `Location` 透传给客户端。除 `Range` / `If-Range` 外，sontv 不转发客户端的其它请求头。

### 资源代理的内存纪律

- 每请求先读 8 KB 探测块，缓冲区由 `sync.Pool` 复用。只有正文真的以 `#EXTM3U` 开头时，sontv 才按播放列表改写。sontv 不按「后缀」或「Content-Type」识别，因此不会误判坑文件。
- 非 m3u8 一律用 `io.Copy` 流式透传，峰值内存 O(32 KB)。`Range` / `If-Range` 原样透传，因此支持拖动播放。
- m3u8 缓冲硬上限 4 MiB。超限返回 `502`，sontv 绝不整段读进内存。上游列表读取上限 8 MiB，超限即拒，不静默截断。
- 出站不设总超时，因为总超时会切断 `.ts` 长流。sontv 由连接、TLS 握手、响应头各自的超时兜底。

### 运维

- **热重载 token 表**：sontv 收到 `SIGUSR1` 后原子换表。sontv 无需重启，也不中断长流。Windows 没有此信号，只能重启进程。
- **优雅退出**：sontv 收到 `SIGTERM`（systemd/容器）或 `SIGINT`（Ctrl+C）时退出。
- **启动自检**：加 `-check` 时，sontv 只做配置与 token 表校验，不启动服务。
- **分级日志**：缺省级别是 `info`。sontv 把逐分片的细节沉到 `debug`。排查时才用 `-log-level debug` 打开，详见[日志](#日志)。
- **跨平台**：sontv 支持 Linux / macOS / Windows。信号处理代码按平台分文件构建。

---

## 安装

### 一键安装

从 [Releases](https://github.com/HasonHuang/sontv/releases) 下载对应架构的产物，装到 `/opt/sontv`：

```bash
curl -fsSL https://raw.githubusercontent.com/HasonHuang/sontv/main/install.sh | bash
```

脚本支持 Debian / Ubuntu / Alpine。脚本自动识别架构（`amd64` / `arm64`）与 libc。脚本按 `checksums.txt` 校验 sha256。下载失败或校验不通过时，脚本立即退出。

**一条命令装完全套**。脚本先探测环境（发行版、架构、init 系统），再逐项询问端口、首个 token、服务名与日志级别——**直接回车取默认值**。然后补齐配置、缺凭据时生成一条 token、建服务用户，按 init 系统注册服务并启动。安装完成后存在下列文件：

```
/opt/sontv/sontv-go        二进制
/opt/sontv/config.json     配置（docs/config.example.json 的内容，按你的选择改写）
/opt/sontv/tokens.txt      凭据（缺失时自动生成一条，明文只在终端打印一次）
/etc/systemd/system/<服务名>.service   或   /etc/init.d/<服务名>（按 init 系统二选一）
```

| 询问项 | 默认 | 落到哪 |
| --- | --- | --- |
| 监听端口 | `9900` | `config.json` 的 `listen` |
| 服务名 | `sontv` | systemd 单元 / OpenRC 脚本的文件名、系统用户名、OpenRC 日志目录 |
| 日志级别 | `info` | `config.json` 的 `log_level` |
| 首个 token | 随机生成 | `tokens.txt` 的一行，文件里只存它的 sha256 |

重跑脚本即升级。脚本直接覆盖二进制，**已存在的 `config.json` 与 `tokens.txt` 不会被覆盖**。自动生成凭据只在文件缺失时发生，脚本不覆盖你手写的 token 表。服务单元每次重写并重启。

已有 `config.json` 时只有一处例外：**填的端口与文件里的 `listen` 不同时才改写该字段**，其它字段原样保留，你手改过的值不会被冲掉。日志级别同理不覆盖，若与文件里的值不同，脚本会提示一次。整份覆盖用 `--force-config`。

### 非交互环境

交互只在能打开 `/dev/tty` 时进行。CI、Docker build、`docker exec`、`curl … | bash` 配上重定向的 stdin 时读不到终端，脚本**全部取默认值，不提示也不失败**，行为与不带交互的旧版一致。命令行选项与环境变量在两种模式下都生效，且**显式给过的项不再询问**——所以自动化调用全程无提示：

```bash
# 装指定版本
curl -fsSL .../install.sh | bash -s -- --version v0.1.0

# 换安装目录
curl -fsSL .../install.sh | bash -s -- --dir /usr/local/sontv

# 换端口、服务名、日志级别
curl -fsSL .../install.sh | bash -s -- --port 8080 --name mytv --log-level debug

# 指定首个 token（明文进，脚本自己算 sha256 存进 tokens.txt）
curl -fsSL .../install.sh | bash -s -- --token 'my-secret-token'

# 只装文件，不注册/启动服务（缺省会装完就起）
curl -fsSL .../install.sh | bash -s -- --no-service

# 覆盖已有配置（缺省保留）
curl -fsSL .../install.sh | bash -s -- --force-config

# 不自动生成 tokens.txt，自己管凭据
curl -fsSL .../install.sh | bash -s -- --no-token

# 指定自动生成的那条 token 的标签
curl -fsSL .../install.sh | bash -s -- --token-label 客厅电视
```

执行 `./install.sh --help` 查看全部选项。每个选项都有同名环境变量（`SONTV_VERSION`、`SONTV_INSTALL_DIR`、`SONTV_SERVICE` …），作用等价。不带参数直接执行 `./install.sh` 也行，适合先下载再执行。

> **自动生成的 token**：明文只在安装结束时打印一次。`tokens.txt` 只存 sha256。丢了只能换发，重装不会再次打印。订阅地址形如 `http://<listen>/sub?token=<明文>`，详见[生成 token](#生成-token)。想自己填表就加 `--no-token`。

注册服务时，脚本建一个与**服务名同名**的系统用户。token 表权限是 0600，非属主读不到，服务会直接返回 503。脚本随后按 init 系统二选一（下面的 `sontv` 换成你填的服务名）：

| init | 写入 | 开机自启 | 热重载 |
| --- | --- | --- | --- |
| systemd | `/etc/systemd/system/sontv.service` | `systemctl enable --now sontv` | `systemctl reload sontv` |
| OpenRC（Alpine/Gentoo） | `/etc/init.d/sontv` | `rc-update add sontv default` + `rc-service sontv start` | `rc-service sontv reload` |

脚本按 `/run/systemd/system` 是否存在来判定，再加 `rc-service` / `openrc-run` 是否可用。`/run/systemd/system` 只在 systemd 真正作为 PID 1 时才有，因此**纯容器里装了 systemctl 也不会被误判成 systemd**。两个 init 都没有时，脚本只装文件并提示手动运行，不留半个坏单元。

OpenRC 服务脚本默认用 `supervise-daemon` 托管。进程崩溃后 5 秒拉起，日志写在 `/var/log/sontv/sontv-go.log`。OpenRC 版本太老而没有 `supervise-daemon`，或容器内核上 `supervise-daemon` 报 `failed to acquire lock` 起不来时，脚本当场降级成 `start-stop-daemon` 后台模式，重写一遍脚本再启动。

> **Alpine 上用 `| sh` 代替 `| bash`**。Alpine 默认不带 bash。脚本本身是 POSIX sh，`sh` 与 `bash` 都能跑：
>
> ```bash
> apk add --no-cache curl   # 或者直接用 busybox 自带的 wget
> wget -qO- https://raw.githubusercontent.com/HasonHuang/sontv/main/install.sh | sh
> ```
>
> Alpine 容器里没有 systemd，服务走 OpenRC 分支。非交互式容器里 `rc-update add` 可能失败，脚本只提示，不影响手动 `rc-service sontv start`。

### 从源码构建

构建需要 **Go 1.24 或更高版本**。项目是纯标准库，无需 `go mod download`。

```bash
git clone <仓库地址> sontv && cd sontv
go build -o bin/sontv-go ./cmd/sontv-go
```

交叉编译到 Linux 服务器：

```bash
GOOS=linux GOARCH=amd64 go build -o bin/sontv-go ./cmd/sontv-go
```

### 安装到 /opt/sontv

不想用一键脚本时，可以手动摆。最终布局与脚本一致。sontv 要求二进制与配置文件同级摆放，放好后直接起即可：

```bash
sudo install -d /opt/sontv
sudo install -m 0755 bin/sontv-go /opt/sontv/sontv-go

# 配置：docs/config.example.json 就是全部缺省值，照抄即零改动启动
sudo install -m 0644 docs/config.example.json /opt/sontv/config.json

# 凭据文件，权限收紧到只有服务账号可读
sudo install -m 0600 tokens.txt /opt/sontv/tokens.txt

cd /opt/sontv && sudo -u sontv ./sontv-go   # 直接起，二进制同级已有 config.json
```

也可以不建配置文件。传 `-config ""`，sontv 直接用内置缺省值启动。

### 生成 token

用一键安装脚本时，`tokens.txt` 缺失会自动生成一条，明文在安装结束时打印一次。加 `--no-token` 可关掉自动生成。要自己生成等价的一组：

```bash
TOKEN="$(openssl rand -hex 24)"                 # 或任何足够随机的字符串
echo "明文 token: $TOKEN"
printf '%s' "$TOKEN" | shasum -a 256            # macOS
printf '%s' "$TOKEN" | sha256sum                # Linux
```

把输出的 64 位十六进制填进 `tokens.txt`，一行一条：`标签,<64位hex>[,TTL小时]`。sontv 对**裸字符串**取 sha256，不要带换行，因此用 `printf '%s'` 而不是 `echo`。完整格式见[配置 → tokens.txt](#tokenstxt)。

---

## 配置

### config.json

sontv 在启动时读取一次配置。改后需重启生效，token 表除外，它可热重载。**只写需要改的字段即可**。未出现的字段保持缺省值，因此「只配 listen」这种最小配置文件不会把上游地址清空。

仓库里的 [`docs/config.example.json`](docs/config.example.json) 列出了全部字段及其缺省值，照抄即零改动启动：

```json
{
  "tokens_file": "/opt/sontv/tokens.txt",
  "default_ttl_hours": 24,
  "upstream_m3u": "https://cdn.qd.je/live.m3u",
  "listen": "0.0.0.0:9900",
  "unwrap_remote_proxy": true,
  "log_level": "info"
}
```

| 字段 | 类型 | 缺省值 | 说明 |
| --- | --- | --- | --- |
| `tokens_file` | string | `/opt/sontv/tokens.txt` | token 表路径 |
| `default_ttl_hours` | int | `24` | 临时 token 的默认有效期（小时）。表里没写第三列的行用它。≤0 时静默回落为 24 |
| `upstream_m3u` | string | `https://cdn.qd.je/live.m3u` | `/sub` 未带 `url=` 时使用的上游播放列表 |
| `listen` | string | `0.0.0.0:9900` | 监听地址。绑 `0.0.0.0` 才能被容器端口转发（`-p 9900:9900` 转发到容器 IP，只听 `127.0.0.1` 会无人应答）。裸机部署因此默认对全网卡开放，鉴权由 token 把关。只让本机可达就显式写 `127.0.0.1:9900` |
| `unwrap_remote_proxy` | bool | `true` | 是否把第三方代理链接解包成本站单跳 |
| `log_level` | string | `info` | 日志级别 `debug` / `info` / `warn` / `error`。详见[日志](#日志) |

`tokens_file` 写相对路径时，同样按二进制同级目录解析。

### tokens.txt

每行一条凭据：

```
标签,sha256(token)[,TTL小时]   [ # 行内注释]
```

- **第一列**：人类可读标签。标签只用于排障，不参与任何判定。
- **第二列**：`sha256(明文 token)` 的 64 位小写十六进制。**不能重复**。
- **第三列**（可选）：该行临时 token 的 TTL 小时数，取值范围 1~876000。留空或为 0 则回落到 `default_ttl_hours`。
- 空行与以 `#` 开头的整行都是注释。行内的「空白 + `#`」之后也算注释。`abc#def` 这种**裸 `#`** 属于标签本身，不是注释。
- 字段两端的空白会被容忍。
- **任何一行出错都会让整张表解析失败**。sontv 宁可返回 `503` 也不接受半张表，因为漏读一行等于误删一个用户。

```
# 标签,sha256(token),可选TTL小时
客厅电视,3f2a…（64位hex）
手机,9c1b…,6             # 这条 6 小时
#主号,aaaa…              # 行首 # → 整行禁用
```

### 命令行参数

| 参数 | 缺省值 | 说明 |
| --- | --- | --- |
| `-config` | `config.json` | 配置文件路径。相对路径以二进制所在目录为基准。传空串（`-config ""`）表示全部使用缺省值 |
| `-log-level` | 空 | 日志级别 `debug` / `info` / `warn` / `error`。未指定时读环境变量 `SONTV_LOG_LEVEL`，再读配置文件的 `log_level`，详见[日志](#日志) |
| `-check` | `false` | 只做配置与 token 表校验，不启动服务 |

缺省值是读二进制同级的 `config.json`，因此把二进制和配置放在一起即可免参数启动。指定了路径但文件不存在或 JSON 非法时，启动失败并退出，退出码非 0。

token 表装载失败**不会**阻止启动，sontv 只在日志里提示。服务照常起，但认证必然返回 `503`（fail closed）。因此 `-check` 也只保证「配置合法」。token 表有问题时，`-check` 会打印错误日志，但退出码仍为 0，以日志内容为准。

### 日志

日志走 stderr，格式为 `时间 级别 [#编号 凭据种类 标签] 正文`：

```
13:20:57.025 ERROR [#7 订阅 repro] 上游抓取失败 原因=EOF 耗时=5054ms
13:28:18.539 INFO  [#1 订阅 repro] 改写完成 上游字节=84 出=196B 入口=http://127.0.0.1:9901/sub 过滤=[] 耗时=1ms
13:28:18.545 DEBUG [#4 临时 repro] 直传完成 字节=4096B 探测头="G@..." 耗时=0ms
```

`[#N]` 是请求编号。sontv 用它把同一次播放的上下游日志串成一条线。`标签` 取自 token 表第一列，用来分辨是哪个用户在播。

分级是**默认安静、按需全开**。播一路电视每分钟要打几十行分片日志。sontv 把它们与启动、改配置、凭据失效这些稀有的事分开，避免出问题时找不到重点。

| 级别 | 内容 | 一次播放的行数 |
| --- | --- | --- |
| `DEBUG` | 逐请求、逐分片：开始 / 上游响应 / 直传完成 / 改写完成 | ~12 行/分钟 |
| `INFO` | 稀有的状态变化：订阅改写结果、启动、重载、退出 | ~1 行/次订阅 |
| `WARN` | 有人在做无效的事：凭据不对、目标非法、自引用 | — |
| `ERROR` | 链路真的断了：上游抓不到、读不了、列表超限 | — |

排查播放问题时，把 `config.json` 的 `log_level` 改成 `debug`，重启服务：

```bash
sudo sed -i 's/"log_level": "info"/"log_level": "debug"/' /opt/sontv/config.json
sudo systemctl restart sontv
```

`log_level` 排在三个入口的最后，是为了让临时覆盖不必先改配置：

| 入口 | 用途 |
| --- | --- |
| `config.json` 的 `log_level` | 正式配置项，常驻部署改这里 |
| `-log-level debug` | 临时提门槛，优先级最高，命令行 > 环境变量 > 配置文件 |
| `SONTV_LOG_LEVEL=debug` | 手工跑二进制时的顺手写法 |

非法取值（如 `"verbose"`）一律回落到 `info` 并打一条 WARN。日志配置写错不该让服务起不来。

**服务单元里不需要、也不建议出现 `-log-level`。** 日志级别属于 `config.json`，改配置对 systemd、OpenRC、容器、手工运行是同一套做法。

日志从不打印凭据明文。目标地址只保留 scheme/host/path，参数名保留，参数值一律抹成 `***`。错误只取内层原因，因为 `*url.Error` 会把完整 URL 拼进 `Error()`。响应正文也从不落盘，日志只留探测块开头的若干字符，用来分辨「HTML 错误页 / 文本提示 / 二进制流」。

---

## 运行

```bash
# 缺省：读二进制同级的 config.json
/opt/sontv/sontv-go

# 显式指定路径，相对路径同样以二进制所在目录为基准
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
| `SIGUSR1` | 重新读取 token 表并原子换表。解析失败时保留旧表（Linux/macOS） |
| `SIGTERM` / `SIGINT` | 优雅退出 |

```bash
kill -USR1 "$(pidof sontv-go)"   # 改完 tokens.txt 后热重载
```

### systemd 单元示例

`install.sh --service` 生成的就是下面这个。脚本额外加了 `Group`、`WorkingDirectory` 与几个加固项。单元里**没有** `-log-level`：日志级别由 `config.json` 决定，`ExecStart` 也不必带 `-config`，二进制会读同级的 `config.json`。

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
output_log="/var/log/sontv/sontv-go.log"
error_log="/var/log/sontv/sontv-go.log"
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

执行 `rc-update add sontv default` 加开机自启。执行 `rc-service sontv start|reload|status` 管日常。

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

sontv **必须**收到 `X-Forwarded-Proto`。本站入口的 scheme 由它现推，缺了它会把 https 的子链接播成 http。

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

`/sub` 的响应里**永远不含稳定 token**，只有短命临时 token。播放器直接抓这条订阅地址即可，子链接会自动走回本站。

### 状态码

| 状态码 | 含义 |
| --- | --- |
| `200` | 成功 |
| `400` | 目标地址缺失/非法/非 http(s)、上游未配置、目标指向本站自身 |
| `401` | 临时 token 无效（过期、签名不符，或对应的行已被删除） |
| `403` | 稳定 token 无效（缺失或不在表里） |
| `405` | `/url` 收到非 GET/HEAD 请求 |
| `502` | 上游抓取/请求/读取失败，或播放列表超出上限（`/sub` 8 MiB、`/url` 4 MiB） |
| `503` | token 表为空或未能装载，服务未就绪，fail closed |

错误响应统一是一行纯文本。sontv 不返回服务器信息，也不返回上游细节。

---

## 开发

```bash
go test ./...        # 全部测试
go test ./... -race  # 并发检查
go vet ./...
gofmt -l .
```

测试分三层。`internal/*` 是单元测试，含 `playlist` 的纯函数改写矩阵与 token 表解析边界。`internal/server` 用 `httptest` 直打 handler 做集成测试，含认证 × 端点的状态码矩阵与时钟注入的过期用例。`internal/server/curl` 真编译、真起进程、真走 TCP 做端到端验证。

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

- **凭据形态分离**：稳定 token 只出现在用户主动配置的订阅地址里。响应体与子链接一律走临时 token。
- **不裸奔**：认证开着却没有可用凭据时拒绝服务，而不是放行。
- **不猜内容**：只有正文真的以 `#EXTM3U` 开头时才按播放列表处理。sontv 不看后缀，也不看 `Content-Type`。
- **不设正文上限的地方就别设**：流式代理不设总超时，不设大小上限。需要缓冲的地方（m3u8 改写）给硬上限并明确报错，绝不静默截断。
