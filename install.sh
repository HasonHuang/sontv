#!/bin/sh
# install.sh — sontv 一键安装脚本
#
#   curl -fsSL https://raw.githubusercontent.com/HasonHuang/sontv/main/install.sh | bash
#
# Alpine 默认不带 bash，用等价写法即可（脚本是 POSIX sh，bash 也照跑）：
#
#   curl -fsSL https://raw.githubusercontent.com/HasonHuang/sontv/main/install.sh | sh
#
# 支持 Debian / Ubuntu / Alpine。产物是完全静态链接的二进制（CGO_ENABLED=0），
# 所以 glibc 与 musl 两份在功能上无差别；脚本仍按发行版挑一个符合直觉的名字下载。

set -eu

REPO="${SONTV_REPO:-HasonHuang/sontv}"
BINARY="sontv-go"
INSTALL_DIR="${SONTV_INSTALL_DIR:-/opt/sontv}"
VERSION="${SONTV_VERSION:-}"          # 指定 tag（如 v0.1.0）；留空则取 latest
FORCE_FLAVOR="${SONTV_FLAVOR:-}"     # 强制 glibc / musl
FORCE_CONFIG="${SONTV_REINSTALL_CONFIG:-0}"
ENABLE_SERVICE="${SONTV_SERVICE:-1}"  # 默认装完就跑起来；--no-service 关掉
NO_RESTART="${SONTV_NO_RESTART:-0}"   # 更新时不重启服务（默认重启，好让新代码真的跑起来）

# 下面四个标记「这个值是命令行/环境变量显式给的」，用来决定还要不要提问。
# 显式给过的绝不提问：脚本化调用（CI、ansible、Dockerfile）里冒出一句提示
# 就已经算交互了，静默直接用比让它按默认走更符合调用方的预期。
PORT_GIVEN=0; NAME_GIVEN=0; LOGLEVEL_GIVEN=0; TOKEN_GIVEN=0
case "${SONTV_PORT+x}" in x) PORT_GIVEN=1 ;; esac
case "${SONTV_SERVICE_NAME+x}" in x) NAME_GIVEN=1 ;; esac
case "${SONTV_LOG_LEVEL+x}" in x) LOGLEVEL_GIVEN=1 ;; esac
case "${SONTV_TOKEN+x}" in x) TOKEN_GIVEN=1 ;; esac
AUTO_TOKEN="${SONTV_NO_TOKEN:-0}"    # 1 = 不自动生成 token，保持「只装不管凭据」
TOKEN_LABEL="${SONTV_TOKEN_LABEL:-我的订阅}"
TOKEN_INPUT="${SONTV_TOKEN:-}"       # 首个 token 的明文；留空则随机生成
PORT="${SONTV_PORT:-9900}"           # 写进 config.json 的 listen
LOG_LEVEL="${SONTV_LOG_LEVEL:-info}" # 写进 config.json 的 log_level
SERVICE_NAME="${SONTV_SERVICE_NAME:-sontv}"
SERVICE_USER="${SONTV_USER:-}"       # 缺省跟随服务名
SERVICE_GROUP=""                      # 装服务时按 id -gn 现查，缺省与用户名同名

TMPDIR_=""
cleanup() { [ -n "$TMPDIR_" ] && rm -rf "$TMPDIR_"; return 0; }
trap cleanup 0
trap 'cleanup; exit 130' INT TERM

# ---------- 输出 ----------
if [ -t 1 ]; then
  C_OK=$(printf '\033[32m'); C_WARN=$(printf '\033[33m')
  C_ERR=$(printf '\033[31m'); C_OFF=$(printf '\033[0m')
else
  C_OK=""; C_WARN=""; C_ERR=""; C_OFF=""
fi
info() { printf '%s==>%s %s\n' "$C_OK" "$C_OFF" "$*"; }
warn() { printf '%s警告:%s %s\n' "$C_WARN" "$C_OFF" "$*" >&2; }
die()  { printf '%s错误:%s %s\n' "$C_ERR" "$C_OFF" "$*" >&2; exit 1; }
usage() {
  cat <<'EOF'
sontv 安装脚本

用法：
  curl -fsSL https://raw.githubusercontent.com/HasonHuang/sontv/main/install.sh | bash
  # Alpine 无 bash 时用 | sh（等价）

选项：
  -v, --version TAG   安装指定版本（如 v0.1.0），默认 latest
  -d, --dir DIR       安装目录，默认 /opt/sontv
  -n, --name NAME     服务名，默认 sontv。决定 systemd 单元 / OpenRC 脚本的
                      文件名、服务用户名与 OpenRC 日志目录
  -p, --port PORT     监听端口，默认 9900（写进 config.json 的 listen）。
                      仅首次安装时询问
  -l, --log-level L   日志级别 debug/info/warn/error，默认 info（写进 config.json）。
                      仅首次安装时询问
      --token TOKEN   指定首个 token 的明文，脚本自己算 sha256 存进 tokens.txt
                      （默认随机生成一条）。仅首次安装时询问
      --flavor F      强制使用 glibc 或 musl 产物
      --force-config  覆盖已存在的 config.json（默认保留）
      --no-token      不自动生成 tokens.txt（默认缺凭据时生成一条并打印明文）
      --token-label L 自动生成的那条 token 的标签，默认「我的订阅」
      --service       注册并启动系统服务（systemd / OpenRC 自适应）；缺省就是开的
      --no-service    只装文件，不碰服务（等价 SONTV_SERVICE=0）
      --no-restart    更新时不重启服务，新版本下次重启后生效（缺省会重启）
  -h, --help          显示本帮助

环境变量（等价于上面的选项）：
  SONTV_VERSION SONTV_INSTALL_DIR SONTV_FLAVOR SONTV_REINSTALL_CONFIG
  SONTV_SERVICE SONTV_REPO SONTV_NO_TOKEN SONTV_TOKEN_LABEL SONTV_USER
  SONTV_PORT SONTV_SERVICE_NAME SONTV_LOG_LEVEL SONTV_TOKEN SONTV_NO_RESTART

更新：服务名对应的服务已存在时，脚本走更新流程——只换二进制，不碰 config.json
与 tokens.txt；服务原本在跑就重启它，好让新代码真的生效（否则新文件要等下次重启
才生效，/proc 里的旧进程还在跑老代码）。交互环境下会先问一句是否更新，读不到
/dev/tty 时直接更新、不提示。端口、token、日志级别只在首次安装时询问。

交互：能读 /dev/tty 时，脚本在探测完环境后逐项询问，直接回车取默认值。命令行选项
或环境变量给过的项不再询问。读不到 /dev/tty（CI、Docker build、`curl | bash` 配上
重定向的 stdin）时全部取默认值，不提示、不失败。

缺省行为：装到 /opt/sontv，补齐配置，缺凭据就生成一条 token（明文只在终端打印
一次，文件里只存 sha256），再按 init 系统注册服务并启动——systemd 与 OpenRC（Alpine、
Gentoo）都支持。两个 init 都没有时退化为只装文件并提示手动运行。
已存在的 config.json 与 tokens.txt 不会被覆盖。
EOF
}

# ---------- 参数 ----------
while [ $# -gt 0 ]; do
  case "$1" in
    -v|--version) [ $# -ge 2 ] || die "--version 需要参数"; VERSION="$2"; shift 2 ;;
    -d|--dir)     [ $# -ge 2 ] || die "--dir 需要参数";     INSTALL_DIR="$2"; shift 2 ;;
    -p|--port)    [ $# -ge 2 ] || die "--port 需要参数";    PORT="$2"; PORT_GIVEN=1; shift 2 ;;
    -n|--name)    [ $# -ge 2 ] || die "--name 需要参数";    SERVICE_NAME="$2"; NAME_GIVEN=1; shift 2 ;;
    -l|--log-level) [ $# -ge 2 ] || die "--log-level 需要参数"; LOG_LEVEL="$2"; LOGLEVEL_GIVEN=1; shift 2 ;;
    --token)      [ $# -ge 2 ] || die "--token 需要参数";   TOKEN_INPUT="$2"; TOKEN_GIVEN=1; shift 2 ;;
    --flavor)     [ $# -ge 2 ] || die "--flavor 需要参数";  FORCE_FLAVOR="$2"; shift 2 ;;
    --force-config) FORCE_CONFIG=1; shift ;;
    --no-token)   AUTO_TOKEN=1; shift ;;
    --token-label) [ $# -ge 2 ] || die "--token-label 需要参数"; TOKEN_LABEL="$2"; shift 2 ;;
    --service)    ENABLE_SERVICE=1; shift ;;
    --no-service) ENABLE_SERVICE=0; shift ;;
    --no-restart) NO_RESTART=1; shift ;;
    -h|--help)    usage; exit 0 ;;
    *)            usage >&2; die "未知参数：$1" ;;
  esac
done

# 标签会原样写进 tokens.txt 的第一列：逗号是分隔符，控制字符（含换行）会截断整行。
printf '%s' "$TOKEN_LABEL" | grep -q '[,[:cntrl:]]' \
  && die "token 标签不能含逗号或控制字符：$TOKEN_LABEL" || true

# ---------- 交互 ----------
# 一律走 /dev/tty，不碰 stdin：`curl … | bash` 时 stdin 是脚本管道本身，
# 读它会把剩下的脚本文本吃掉（下面给二进制做的 </dev/null 是同一个道理）。
#
# 可用性靠「真能打开」判断，不能只看 [ -r/-w ]：docker exec 这类环境里
# /dev/tty 存在且权限正常，打开却会失败（No such device or address）。
# 只看权限位的话，脚本会一路顺利跑到第一次提问才崩。
#
# 放在子 shell 里试：开 /dev/tty 就是让这一行失败，脚本自己有 set -e，
# 让它在本进程里失败会直接带走整个安装（dash 对 if 条件内的重定向失败
# 同样会触发 errexit）。子 shell 里的失败只影响它自己。
TTY=""
if ( : 2>/dev/null >/dev/tty ) 2>/dev/null; then
  TTY="/dev/tty"
fi

# ask 提示 默认值 → 结果放 ASK_REPLY。直接回车或 EOF 都取默认值：
# 不想回答的人一路回车到底，得到的正是一份完整的默认安装。
ask() {
  if [ -z "$TTY" ]; then
    ASK_REPLY="$2"
    return 0
  fi
  printf '%s [%s] ' "$1" "$2" >"$TTY"
  # 读不到行（Ctrl-D）也当回车：交互中途放弃不该把脚本带崩。
  IFS= read -r ASK_REPLY <"$TTY" || ASK_REPLY=""
  [ -n "$ASK_REPLY" ] || ASK_REPLY="$2"
}

# ask_yes_no 问一个是/否。答案收敛成 y 或 n，其余（含回车）取默认值——
# 调用点只判这两个值，不必再写一遍 case。
ask_yes_no() {
  ask "$1" "$2"
  case "$ASK_REPLY" in
    [Yy]*) ASK_REPLY="y" ;;
    [Nn]*) ASK_REPLY="n" ;;
    *)     ASK_REPLY="$2" ;;
  esac
}

# ask_num 提示 默认值 校验函数名：校验不通过就重新问，而不是报错退出。
# 端口填错导致服务起不来，是最难排查的一类问题，值得多问一句。
ask_num() {
  while :; do
    ask "$1" "$2"
    if "$3" "$ASK_REPLY"; then
      return 0
    fi
    printf '%s不是合法取值，请重新输入。\n' "$ASK_REPLY" >"$TTY"
  done
}

valid_port() {
  case "$1" in
    ''|*[!0-9]*) return 1 ;;
  esac
  [ "$1" -ge 1 ] 2>/dev/null && [ "$1" -le 65535 ] 2>/dev/null
}

# 服务名会直接进 systemd 单元文件名、OpenRC 脚本名、用户名与日志目录，
# 放开特殊字符会一路逃到这些地方去。限死成字母数字下划线连字符。
valid_name() {
  case "$1" in
    ''|*[!A-Za-z0-9_-]*) return 1 ;;
  esac
  return 0
}

valid_level() {
  case "$1" in
    debug|info|warn|error) return 0 ;;
  esac
  return 1
}

# token 明文里也不能有逗号和控制字符：前者是 tokens.txt 的分隔符，
# 后者会让终端回显与文件行格式出乱子。
valid_token() {
  case "$1" in
    '') return 1 ;;
  esac
  printf '%s' "$1" | grep -q '[,[:cntrl:]]' && return 1
  return 0
}

if [ -n "$TTY" ]; then
  printf '\n检测到终端，下面几项直接回车取默认值。\n\n'
fi

# 提问本体放在环境探测之后（见「配置询问」一节）：先让用户知道这是什么系统、
# 用什么 init，再问怎么装。反过来先问一串端口服务名，用户还不知道这机器上
# 有没有 systemd，答也答得没底气。

if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
elif command -v sudo >/dev/null 2>&1; then
  SUDO="sudo"
else
  die "需要 root 权限：装到 $INSTALL_DIR 请用 sudo 重跑（本机没有 sudo）"
fi
run_root() { if [ -n "$SUDO" ]; then sudo "$@"; else "$@"; fi; }

command -v tar >/dev/null 2>&1 || die "缺少 tar，请先安装"

# ---------- 检测下载器 ----------
if command -v curl >/dev/null 2>&1; then
  DL="curl"
elif command -v wget >/dev/null 2>&1; then
  DL="wget"
else
  die "需要 curl 或 wget"
fi
download() { # url dest
  if [ "$DL" = curl ]; then
    curl -fL --retry 3 --retry-delay 1 -o "$2" "$1"
  else
    wget -q --tries=3 -O "$2" "$1"
  fi
}
resolve_url() { # 跟随重定向后输出最终 URL
  if [ "$DL" = curl ]; then
    curl -fsSLI -o /dev/null -w '%{url_effective}' "$1" || true
  else
    wget -q --spider -S "$1" 2>&1 | awk '/^[Ll]ocation:/ {print $2}' | tr -d '\r' | tail -n1 || true
  fi
}
sha256_of() { # 文件 → 十六进制；本机没有任何实现时输出空
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$1" | awk '{print $NF}'
  fi
}
sha256_stdin() { # 标准输入 → 十六进制；同上，没有实现时输出空
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 | awk '{print $1}'
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 | awk '{print $NF}'
  fi
}
random_hex() { # n 字节 → 2n 位十六进制；openssl 缺失时退回 /dev/urandom
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex "$1"
  else
    od -An -vtx1 -N "$1" /dev/urandom | tr -d ' \n'
    printf '\n'
  fi
}

# ---------- 改写既有 config.json ----------
# 已有配置默认不碰，这是脚本一贯的承诺。但「填了端口 8080 而配置里还是 9900」
# 是个让人当场白跑一趟的落差，所以留一条窄路：只改 listen，且只在真不同的时候。

# json_field_value 读出某字段当前的字符串值，读不到输出空。
json_field_value() { # 文件 字段名
  sed -n "s|^[[:space:]]*\"$2\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*|\1|p" "$1" | head -n1
}

# json_replace_field 就地替换某字段的字符串值，成功返回 0。
# 只动引号内的值，文件其余部分（含中文注释）原样搬过去。
# 字段不存在时返回 1，调用方据此决定是补一条还是放弃。
json_replace_field() { # 文件 字段名 新值
  grep -q "^[[:space:]]*\"$2\"[[:space:]]*:" "$1" || return 1
  sed "s|\"$2\"[[:space:]]*:[[:space:]]*\"[^\"]*\"|\"$2\": \"$3\"|" "$1" > "$1.tmp" || return 1
  mv "$1.tmp" "$1"
}

# json_set_listen 把已有配置的 listen 改成目标值。
# 目标值与现值相同则返回 1（=「不需要动」，调用方据此宣告保持不变）。
json_set_listen() { # 文件 目标值
  [ "$(json_field_value "$1" listen)" = "$2" ] && return 1
  cp "$1" "$TMPDIR_/config.keep.json"
  json_replace_field "$TMPDIR_/config.keep.json" listen "$2" || return 1
  run_root install -m 0644 "$TMPDIR_/config.keep.json" "$1" || return 1
  return 0
}

# json_set_log_level 写 log_level。字段不存在时补到最后一项，
# 这样老配置（写出这个字段之前装的）不会静默丢掉这次选择。
json_set_log_level() { # 文件 值
  if grep -q "^[[:space:]]*\"log_level\"[[:space:]]*:" "$1"; then
    json_replace_field "$1" log_level "$2"
    return 0
  fi
  # 倒数第一个非空行必须是收尾的 }，它的上一行才是最后一项字段。
  # 两条都对不上就放弃：与其拼出一份坏 JSON 让服务起不来，不如不写。
  awk -v v="$2" '
    { line[NR] = $0; if ($0 ~ /[^ \t]/) last = NR }
    END {
      if (last < 2) exit 1
      if (line[last] !~ /^[ \t]*}[ \t]*$/) exit 1
      prev = last - 1
      if (line[prev] ~ /^[ \t]*$/) exit 1          # 空对象，补出来的逗号无处可挂
      for (i = 1; i <= last; i++) {
        if (i == prev && line[i] !~ /,[ \t]*$/) line[i] = line[i] ","
        print line[i]
      }
      print "  \"log_level\": \"" v "\""
      for (i = last + 1; i <= NR; i++) print line[i]
    }
  ' "$1" > "$1.tmp" || return 1
  mv "$1.tmp" "$1"
}

# ---------- 检测架构 ----------
UNAME_M="$(uname -m)"
case "$UNAME_M" in
  x86_64|amd64)  ARCH="amd64" ;;
  aarch64|arm64) ARCH="arm64" ;;
  *) die "不支持的架构：$UNAME_M（目前只提供 amd64 / arm64）" ;;
esac

# ---------- 检测发行版与 libc ----------
OS="unknown"
if [ -f /etc/alpine-release ]; then
  OS="alpine"
elif [ -f /etc/os-release ]; then
  OS="$(. /etc/os-release 2>/dev/null && echo "${ID:-unknown}")"
fi

if [ -n "$FORCE_FLAVOR" ]; then
  FLAVOR="$FORCE_FLAVOR"
elif [ "$OS" = alpine ] || ldd --version 2>&1 | grep -qi musl; then
  FLAVOR="musl"
else
  FLAVOR="glibc"
fi
case "$FLAVOR" in
  glibc|musl) ;;
  *) die "--flavor 只接受 glibc 或 musl" ;;
esac

case "$OS" in
  debian|ubuntu|alpine) ;;
  *) warn "未在 Debian / Ubuntu / Alpine 上测试过（当前：$OS）；静态二进制一般可直接运行" ;;
esac

# ---------- 检测 init 系统 ----------
# /run/systemd/system 只在 systemd 真正作为 PID 1 跑起来时才有，所以纯容器里
# 装了 systemctl 也不会被误判成 systemd（那种环境 systemctl 存在但一用就报错）。
INIT="none"
if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
  INIT="systemd"
elif command -v rc-service >/dev/null 2>&1 || [ -x /sbin/openrc-run ] || [ -x /usr/sbin/openrc-run ]; then
  INIT="openrc"
fi

# service_installed 报出这个服务是否已装：判据是服务单元/脚本本身存在，
# 而不是问 init——问 systemctl/rc-service 在没装过的机器上会拖出一堆误导性输出。
service_installed() {
  case "$INIT" in
    systemd) [ -f "/etc/systemd/system/$SERVICE_NAME.service" ] ;;
    openrc)  [ -f "/etc/init.d/$SERVICE_NAME" ] ;;
    *)       return 1 ;;
  esac
}

# service_is_running 报出服务当前是否在跑。更新时只重启原本在跑的：
# 把一个用户特意停掉的服务顺手拉起来，比不重启更糟。
service_is_running() {
  case "$INIT" in
    systemd) systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null ;;
    openrc)  rc-service "$SERVICE_NAME" status >/dev/null 2>&1 ;;
    *)       return 1 ;;
  esac
}

# ---------- 配置询问 ----------
# 环境已经探明（发行版、架构、init 系统），现在才问怎么装。
# 每项都是「命令行/环境变量给了就用、不给才问」，所以脚本化调用全程无交互。
#
# ask 把答案放进 ASK_REPLY，**不会**自己写回目标变量——调用点必须显式取回来：
#   ask_num "监听端口" "$PORT" valid_port; PORT="$ASK_REPLY"
# 漏掉这一句的话，答案会被问出来、会被回显、然后被原样丢掉，
# 表现得像交互根本没生效。

# 服务名排在最前：后面要靠它去判「这个服务装过没有」，问晚了就没法分支了。
if [ "$NAME_GIVEN" = 1 ]; then
  valid_name "$SERVICE_NAME" \
    || die "服务名只接受字母、数字、下划线与连字符，收到：$SERVICE_NAME"
else
  ask_num "服务名" "$SERVICE_NAME" valid_name
  SERVICE_NAME="$ASK_REPLY"
fi

# 服务用户名默认跟随服务名。SONTV_USER 显式给过时以其为准——
# 用户可能已经有一个专用账号了，不该被我们改名字。
[ -n "$SERVICE_USER" ] || SERVICE_USER="$SERVICE_NAME"

# ---------- 已安装检测 ----------
# 装过了就是升级，没装过才是首次安装。两条路的差别只在「要不要动配置和凭据」，
# 下载、校验、覆盖二进制、注册服务这些是一样的。
IS_UPDATE=0
if service_installed; then
  IS_UPDATE=1
fi

if [ "$IS_UPDATE" = 1 ]; then
  if [ -z "$TTY" ]; then
    # 非交互：直接更新，不提示。脚本化调用（CI、ansible、定时任务）里
    # 「重跑 = 升级」是既有语义，冒出一句提示就已经算交互了。
    info "检测到    $SERVICE_NAME 已安装，将更新到最新版本"
  else
    # 更新只换程序，不改配置也不动凭据——需要改那些是另一件事，
    # 用 --force-config 或直接编辑配置文件。免得回答被问了却不生效。
    ask_yes_no "检测到 $SERVICE_NAME 已安装，是否更新到最新版本？(Y/n)" "Y"
    if [ "$ASK_REPLY" = n ] || [ "$ASK_REPLY" = N ]; then
      info "已取消    未做任何改动"
      exit 0
    fi
  fi
else
  if [ "$PORT_GIVEN" = 1 ]; then
    valid_port "$PORT" || die "端口只接受 1~65535，收到：$PORT"
  else
    ask_num "监听端口" "$PORT" valid_port
    PORT="$ASK_REPLY"
  fi

  if [ "$LOGLEVEL_GIVEN" = 1 ]; then
    valid_level "$(printf '%s' "$LOG_LEVEL" | tr 'A-Z' 'a-z')" \
      || die "日志级别只接受 debug / info / warn / error，收到：$LOG_LEVEL"
  else
    ask_num "日志级别 (debug/info/warn/error)" "$LOG_LEVEL" valid_level
    LOG_LEVEL="$ASK_REPLY"
  fi

  if [ "$TOKEN_GIVEN" = 1 ]; then
    valid_token "$TOKEN_INPUT" || die "token 不能为空，也不能含逗号或控制字符"
  elif [ "$AUTO_TOKEN" != 1 ]; then
    # 填了就用填的（脚本自己算 sha256），留空则随机生成一条。
    ask "首个 token（直接回车则随机生成）" ""
    [ -z "$ASK_REPLY" ] || TOKEN_INPUT="$ASK_REPLY"
  fi

  # 级别统一转小写后使用：用户填 INFO / Debug 都不该被当成非法值拒掉。
  LOG_LEVEL="$(printf '%s' "$LOG_LEVEL" | tr 'A-Z' 'a-z')"
fi

# 非交互且是更新时，上面整段都被跳过了，这些变量得有个能用的值：
# 端口只用于安装结束时打印订阅地址，日志级别只用于打印。
PORT="${PORT:-9900}"
LOG_LEVEL="$(printf '%s' "$LOG_LEVEL" | tr 'A-Z' 'a-z')"
[ -n "$LOG_LEVEL" ] || LOG_LEVEL="info"

# ---------- 解析版本与下载地址 ----------
BASE="https://github.com/$REPO/releases"
ASSET="$BINARY-linux-$ARCH-$FLAVOR.tar.gz"

if [ -n "$VERSION" ]; then
  case "$VERSION" in
    v*) TAG="$VERSION" ;;
    *)  TAG="v$VERSION" ;;
  esac
  ASSET_URL="$BASE/download/$TAG/$ASSET"
else
  TAG="latest"
  ASSET_URL="$BASE/latest/download/$ASSET"
fi

TMPDIR_="$(mktemp -d)"

info "仓库      $REPO"
info "系统      $OS / $ARCH / $FLAVOR"
info "版本      $TAG"
info "安装目录  $INSTALL_DIR"
if [ "$IS_UPDATE" = 1 ]; then
  info "模式      更新已安装的 $SERVICE_NAME（配置与凭据保持不变）"
else
  info "监听      0.0.0.0:$PORT"
  info "日志级别  $LOG_LEVEL（写进 config.json，改完重启生效）"
fi
if [ "$INIT" != none ]; then
  info "服务      $SERVICE_NAME（$INIT）"
fi

# 顺带把 latest 解析成真实 tag，只为打印用，失败无妨。
if [ "$TAG" = latest ]; then
  REAL_URL="$(resolve_url "$BASE/latest")"
  case "$REAL_URL" in
    */tag/v*) TAG="${REAL_URL##*/tag/}" ;;
  esac
  info "实际版本  $TAG"
fi

# ---------- 下载并校验 ----------
info "下载 $ASSET"
if ! download "$ASSET_URL" "$TMPDIR_/$ASSET"; then
  die "下载失败：$ASSET_URL（检查网络，或用 --version 指定一个已发布的 tag）"
fi

CHECKSUMS="$TMPDIR_/checksums.txt"
if ! download "$BASE/$TAG/download/checksums.txt" "$CHECKSUMS" 2>/dev/null; then
  download "$BASE/latest/download/checksums.txt" "$CHECKSUMS" 2>/dev/null || rm -f "$CHECKSUMS"
fi

EXPECTED=""
if [ -f "$CHECKSUMS" ]; then
  EXPECTED="$(grep -F "  $ASSET" "$CHECKSUMS" | awk '{print $1}' | head -n1 || true)"
fi

if [ -z "$EXPECTED" ]; then
  warn "未找到 $ASSET 的校验和，跳过完整性校验"
else
  ACTUAL="$(sha256_of "$TMPDIR_/$ASSET")"
  [ -n "$ACTUAL" ] || die "本机没有 sha256sum/shasum/openssl，无法校验下载结果"
  [ "$(printf '%s' "$ACTUAL" | tr 'A-F' 'a-f')" = "$(printf '%s' "$EXPECTED" | tr 'A-F' 'a-f')" ] \
    || die "校验和不匹配：下载可能损坏或被篡改，请重试"
  info "校验和    OK"
fi

# ---------- 解压 ----------
tar -xzf "$TMPDIR_/$ASSET" -C "$TMPDIR_"
[ -f "$TMPDIR_/$BINARY" ] || die "压缩包结构异常：找不到 $BINARY"

# ---------- 安装 ----------
run_root install -d -m 0755 "$INSTALL_DIR"

# 覆盖二进制前先记住服务在不在跑：更新完只重启原本在跑的那个。
# 用户特意停掉的服务不该被顺手拉起来——那比不重启更糟。
SERVICE_WAS_RUNNING=0
if [ "$IS_UPDATE" = 1 ] && [ "$NO_RESTART" != 1 ] && service_is_running; then
  SERVICE_WAS_RUNNING=1
fi

run_root install -m 0755 "$TMPDIR_/$BINARY" "$INSTALL_DIR/$BINARY"

# 配置：以包里的 config.example.json 为底，按填写的端口与日志级别改写。
# listen 只在真的与现值不同时才动——重跑一次安装不该让文件无谓地变一次，
# 更不该把用户手改过的其它字段一起冲掉。
NEW_LISTEN="0.0.0.0:$PORT"

if [ "$IS_UPDATE" = 1 ] && [ "$FORCE_CONFIG" != 1 ]; then
  # 更新只换程序。端口与日志级别是另一件事：让用户用 --force-config 或直接
  # 编辑配置文件去改，这里不碰，免得脚本替他做了没问过的决定。
  info "配置      保持不变（--force-config 可覆盖）"
elif [ "$FORCE_CONFIG" != 1 ] && [ -f "$INSTALL_DIR/config.json" ]; then
  if json_set_listen "$INSTALL_DIR/config.json" "$NEW_LISTEN"; then
    info "配置      已存在，只把 listen 更新为 $NEW_LISTEN（--force-config 可整份覆盖）"
  else
    info "配置      已存在，保持不变（--force-config 可覆盖）"
  fi
  # 已有配置一律不碰 log_level，但用户刚回答过级别：静默丢掉会让人以为
  # 没生效。级别对不上就在这里说一声，而不是等他去翻日志才发现。
  CUR_LEVEL="$(json_field_value "$INSTALL_DIR/config.json" log_level)"
  if [ -n "$CUR_LEVEL" ] && [ "$CUR_LEVEL" != "$LOG_LEVEL" ]; then
    warn "配置里的 log_level 是 $CUR_LEVEL，与你填的 $LOG_LEVEL 不同，未改动；要用新级别请改 $INSTALL_DIR/config.json 后重启"
  fi
else
  SRC="${TMPDIR_}/config.json"
  # 老版本的发布包里没有 config.json，给个底文件而不是空文件。
  [ -f "$SRC" ] || cat > "$SRC" <<'JSON'
{
  "tokens_file": "/opt/sontv/tokens.txt",
  "default_ttl_hours": 24,
  "upstream_m3u": "https://cdn.qd.je/live.m3u",
  "listen": "0.0.0.0:9900",
  "unwrap_remote_proxy": true,
  "log_level": "info"
}
JSON
  # 一律读 SRC、写另一个文件：sed 边读边写同一个路径会把它清空
  # （重定向先截断，sed 才去读），写出来的是 0 字节的坏配置。
  # json_replace_field 同样只往 "$1.tmp" 写，最后才 mv 回去。
  sed "s|\"listen\"[[:space:]]*:[[:space:]]*\"[^\"]*\"|\"listen\": \"$NEW_LISTEN\"|" "$SRC" > "$TMPDIR_/cfg.new"
  # tokens_file 要跟着安装目录走：用户 --dir 换了目录而配置还指向
  # /opt/sontv 的话，服务起来会去读一个根本没被写入的路径。
  sed "s|\"tokens_file\"[[:space:]]*:[[:space:]]*\"[^\"]*\"|\"tokens_file\": \"$INSTALL_DIR/tokens.txt\"|" "$TMPDIR_/cfg.new" > "$TMPDIR_/cfg.json"
  json_set_log_level "$TMPDIR_/cfg.json" "$LOG_LEVEL" || true
  # 落到安装目录之前先确认它还是份合法 JSON：坏配置会让服务直接起不来，
  # 而那正是这个脚本跑完之后最让人意外的结果。
  if ! "$INSTALL_DIR/$BINARY" -config "$TMPDIR_/cfg.json" -check >/dev/null 2>&1 \
     || ! grep -q '"log_level"' "$TMPDIR_/cfg.json"; then
    die "生成的 config.json 不合法，请检查端口与安装目录的取值"
  fi
  run_root install -m 0644 "$TMPDIR_/cfg.json" "$INSTALL_DIR/config.json"
  info "配置      已写入 $INSTALL_DIR/config.json（listen=$NEW_LISTEN log_level=$LOG_LEVEL）"
fi

# ---------- 凭据 ----------
# 已有 tokens.txt 一律不碰内容，只把权限收紧。
# 缺失时默认自动生成一条（明文只在终端打印这一次，文件里只存 sha256），
# 这样「一条命令装完就能用」；不想要就 --no-token。
NEW_TOKEN=""
if [ -f "$INSTALL_DIR/tokens.txt" ]; then
  run_root chmod 0600 "$INSTALL_DIR/tokens.txt" || true
  info "凭据      已存在，保持不变"
elif [ "$AUTO_TOKEN" = 1 ]; then
  info "凭据      未创建（--no-token），请自行写 $INSTALL_DIR/tokens.txt"
else
  # 用户填了明文就用填的，否则随机生成一条。两种情况下的最终形态一致：
  # 明文只在终端打印这一次，文件里只存 sha256。
  # 命令替换里可能一条 sha256 实现都没有，用 || true 兜住 set -e，之后再判空。
  NEW_TOKEN="$TOKEN_INPUT"
  [ -n "$NEW_TOKEN" ] || NEW_TOKEN="$(random_hex 24 | tr -d '\n' || true)"
  NEW_HASH="$(printf '%s' "$NEW_TOKEN" | sha256_stdin || true)"
  if [ -z "$NEW_TOKEN" ] || [ -z "$NEW_HASH" ]; then
    NEW_TOKEN=""
    warn "无法生成 token（本机缺 openssl / sha256sum / shasum），请手动创建 tokens.txt"
  else
    # 每行都得是「行首 #」的注释或合法凭据行：少一个 # 就会让整表解析失败。
    printf '# %s\n# 格式：标签,sha256(token)[,TTL小时] —— 明文只在安装时打印一次，丢失只能换发\n%s,%s\n' \
      "$INSTALL_DIR/tokens.txt" "$TOKEN_LABEL" "$NEW_HASH" > "$TMPDIR_/tokens.txt"
    run_root install -m 0600 "$TMPDIR_/tokens.txt" "$INSTALL_DIR/tokens.txt"
    info "凭据      已生成 $INSTALL_DIR/tokens.txt（标签：$TOKEN_LABEL）"
  fi
fi

# 冒烟自检：只有凭据就位时才跑（缺 tokens.txt 会 fail closed 直接报错）。
# config.json 的相对路径按二进制同级目录解析，所以不需要先 cd。
# 注意 -check 对「token 表装载失败」仍返回 0（设计如此：服务照常起、认证 503），
# 所以这里额外扫一遍输出里的错误行，否则一个坏 token 表会被「自检通过」盖过去。
if [ -f "$INSTALL_DIR/tokens.txt" ]; then
  # </dev/null：脚本被 `curl | bash` 喂进来时 stdin 就是管道本身，
  # 不挡住的话二进制一旦读 stdin 就会把脚本文本吃掉。
  CHECK_LOG="$TMPDIR_/check.log"
  if run_root "$INSTALL_DIR/$BINARY" -check </dev/null >"$CHECK_LOG" 2>&1; then
    if grep -q '失败\|error\|ERROR' "$CHECK_LOG"; then
      warn "自检有报错（服务能起，但认证会 503）："
      sed -n '1,10p' "$CHECK_LOG" >&2
    else
      info "自检      通过"
    fi
  else
    warn "自检未通过，请手动执行：$INSTALL_DIR/$BINARY -check"
    sed -n '1,10p' "$CHECK_LOG" >&2
  fi
fi

# ---------- 服务（可选，systemd / OpenRC 自适应） ----------
ensure_service_user() {
  if ! id "$SERVICE_USER" >/dev/null 2>&1; then
    # 三种 adduser/useradd 的参数语义完全不同：shadow 的 useradd、
    # busybox 的 adduser、Debian 的 perl adduser（只吃长选项）。
    # 依次尝试，建出来了就停——失败信息一律咽掉，最后统一验人。
    if command -v useradd >/dev/null 2>&1; then
      run_root useradd -r -M -s /usr/sbin/nologin "$SERVICE_USER" >/dev/null 2>&1 || true
    fi
    if ! id "$SERVICE_USER" >/dev/null 2>&1; then
      run_root adduser -S -D -H -s /sbin/nologin "$SERVICE_USER" >/dev/null 2>&1 || true
    fi
    if ! id "$SERVICE_USER" >/dev/null 2>&1; then
      run_root adduser --system --no-create-home --shell /usr/sbin/nologin \
        "$SERVICE_USER" >/dev/null 2>&1 || true
    fi
    id "$SERVICE_USER" >/dev/null 2>&1 \
      || warn "建不出系统用户 $SERVICE_USER，服务可能起不来（需要时请手动建）"
  fi
  # 组名现查：Debian 的 useradd 未必建同名组，Alpine 的 busybox adduser 一定会。
  SERVICE_GROUP="$(id -gn "$SERVICE_USER" 2>/dev/null || echo "$SERVICE_USER")"
  # tokens.txt 是 0600，服务以非 root 跑就必须属主对得上，否则读不到直接 503。
  run_root chown -R "$SERVICE_USER:$SERVICE_GROUP" "$INSTALL_DIR" 2>/dev/null || true
}

# 写 /etc/init.d/$SERVICE_NAME。$1 = supervise（supervise-daemon 托管，有崩溃重启与日志）
# 或 plain（start-stop-daemon 后台，老 OpenRC 或容器里 supervise-daemon 起不来时用）。
write_openrc_initd() {
  if [ "$1" = supervise ]; then
    SUPERVISOR="supervisor=supervise-daemon
output_log=\"/var/log/$SERVICE_NAME/$BINARY.log\"
error_log=\"/var/log/$SERVICE_NAME/$BINARY.log\"
respawn_delay=5
respawn_max=0"
    RELOAD_CMD="start-stop-daemon --signal USR1 --name $BINARY"
  else
    SUPERVISOR="command_background=\"yes\"
pidfile=\"/run/$SERVICE_NAME.pid\""
    RELOAD_CMD="start-stop-daemon --signal USR1 --pidfile /run/$SERVICE_NAME.pid"
  fi
  cat > "$TMPDIR_/sontv.initd" <<INITD
#!/sbin/openrc-run
# $SERVICE_NAME — 由 install.sh 生成，重跑脚本会覆盖

name="$SERVICE_NAME"
description="sontv - IPTV subscription proxy"

command="$INSTALL_DIR/$BINARY"
directory="$INSTALL_DIR"
command_user="$SERVICE_USER:$SERVICE_GROUP"
retry="SIGTERM/5"

$SUPERVISOR

depend() {
	need net
	after firewall
}

start_pre() {
	checkpath --directory --owner "$SERVICE_USER:$SERVICE_GROUP" --mode 0755 /var/log/$SERVICE_NAME
}

# 改完 tokens.txt 热重载，不必重启（对应 systemd 的 ExecReload）。
#
# extra_commands 这两行不是可选的：openrc-run.sh 的命令分派只遍历一组固定的内置
# 函数（describe/start/stop/status）与这里显式声明的额外命令，自定义 reload()
# 不声明就永远不会被调用——rc-service 会直接报 "unknown function \`reload'"。
# 这不是本脚本的疏忽，是 openrc 的扩展点约定。
extra_commands="reload"
extra_started_commands="reload"

reload() {
	ebegin "重载 \$name"
	$RELOAD_CMD >/dev/null 2>&1
	eend \$?
}
INITD
  run_root install -m 0755 "$TMPDIR_/sontv.initd" "/etc/init.d/$SERVICE_NAME"
}

SERVICE_STARTED=0
SERVICE_ENABLED=0   # 是否已登记开机自启
if [ "$ENABLE_SERVICE" = 1 ]; then
  case "$INIT" in
    systemd)
      ensure_service_user
      cat > "$TMPDIR_/sontv.service" <<UNIT
[Unit]
Description=sontv ($SERVICE_NAME) - IPTV subscription proxy
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$SERVICE_USER
Group=$SERVICE_GROUP
WorkingDirectory=$INSTALL_DIR
ExecStart=$INSTALL_DIR/$BINARY
ExecReload=/bin/kill -USR1 \$MAINPID
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=full
ProtectHome=true

[Install]
WantedBy=multi-user.target
UNIT
      run_root install -d -m 0755 /etc/systemd/system
      run_root install -m 0644 "$TMPDIR_/sontv.service" "/etc/systemd/system/$SERVICE_NAME.service"
      run_root systemctl daemon-reload

      if [ "$IS_UPDATE" = 1 ]; then
        # enable --now 对已在运行的服务是 no-op：systemd 认为无需启动，
        # 新覆盖的二进制就永远不会被执行（/proc/<pid>/exe 指向 (deleted) 的旧 inode）。
        # 所以更新走显式 restart，且只在服务原本在跑时才动它。
        run_root systemctl enable "$SERVICE_NAME" >/dev/null 2>&1 || true
        if [ "$SERVICE_WAS_RUNNING" = 1 ]; then
          if run_root systemctl restart "$SERVICE_NAME"; then
            SERVICE_STARTED=1
            info "服务      已重启到新版本（systemctl status $SERVICE_NAME）"
          else
            warn "二进制已更新，但重启失败：systemctl status $SERVICE_NAME（旧进程仍在跑）"
          fi
        elif [ "$NO_RESTART" = 1 ]; then
          info "服务      未重启（--no-restart），新版本下次重启后生效"
        else
          # 本来就没在跑：不擅自启动，只把开机自启补上。
          info "服务      本未运行，未启动；已登记开机自启（systemctl start $SERVICE_NAME 可手动起）"
        fi
        SERVICE_ENABLED=1
      elif run_root systemctl enable --now "$SERVICE_NAME"; then
        SERVICE_STARTED=1
        SERVICE_ENABLED=1
        info "服务      已注册并启动（systemctl status $SERVICE_NAME）"
      else
        warn "服务单元已写入，但启动失败：systemctl status $SERVICE_NAME"
      fi
      ;;
    openrc)
      ensure_service_user
      # OpenRC 没真正 boot 过的环境（典型：docker run alpine 后 apk add openrc）
      # 里，rc-service 对任何服务都报 "failed to acquire lock: Bad file
      # descriptor" —— 连 sleep 都起不来。openrc 自己的诊断给的就是这条：
      # 补一个 softlevel 标记。正常开过机的系统上它已存在，这里是空操作。
      if [ ! -e /run/openrc/softlevel ]; then
        if run_root mkdir -p /run/openrc && run_root touch /run/openrc/softlevel; then
          info "OpenRC     补建 /run/openrc/softlevel（本机 openrc 未 boot）"
        fi
      fi

      # supervise-daemon 给崩溃重启和日志，但部分容器内核上会
      # "failed to acquire lock" 起不来——所以先试它，失败降级再来一遍。
      MODE="plain"
      command -v supervise-daemon >/dev/null 2>&1 && MODE="supervise"
      write_openrc_initd "$MODE"

      if [ "$IS_UPDATE" = 1 ]; then
        # start 对已在跑的服务同样是 no-op，新二进制不会被执行。显式 restart。
        if [ "$SERVICE_WAS_RUNNING" = 1 ]; then
          if run_root rc-service "$SERVICE_NAME" restart; then
            SERVICE_STARTED=1
            info "服务      已重启到新版本（rc-service $SERVICE_NAME status，$MODE 模式）"
          else
            warn "二进制已更新，但重启失败：rc-service $SERVICE_NAME restart（旧进程仍在跑）"
          fi
        elif [ "$NO_RESTART" = 1 ]; then
          info "服务      未重启（--no-restart），新版本下次重启后生效"
        else
          info "服务      本未运行，未启动；手动起：rc-service $SERVICE_NAME start"
        fi
      elif run_root rc-service "$SERVICE_NAME" start; then
        SERVICE_STARTED=1
        info "服务      已启动（rc-service $SERVICE_NAME status，$MODE 模式）"
      elif [ "$MODE" = supervise ]; then
        warn "supervise-daemon 起不来，降级为 start-stop-daemon 重试"
        write_openrc_initd plain
        if run_root rc-service "$SERVICE_NAME" start; then
          SERVICE_STARTED=1
          info "服务      已启动（rc-service $SERVICE_NAME status，plain 模式）"
        fi
      fi
      if [ "$IS_UPDATE" != 1 ]; then
        [ "$SERVICE_STARTED" = 1 ] || warn "服务脚本已写入，但启动失败：rc-service $SERVICE_NAME start"
      fi

      # default 运行级 = 开机自启。放在启动之后登记：openrc 从没 boot 过的
      # 容器里，/run/openrc 状态是服务起过一次才齐的，早跑容易失败。
      if run_root rc-update add "$SERVICE_NAME" default >/dev/null 2>&1; then
        SERVICE_ENABLED=1
      else
        warn "rc-update add 失败（当前环境多半没有真实 init），不会开机自启；需要时手动：rc-update add $SERVICE_NAME default"
      fi
      ;;
    *)
      warn "没找到 systemd 或 OpenRC，跳过服务注册（可手动跑 $INSTALL_DIR/$BINARY）"
      ;;
  esac
fi

# ---------- 完成 ----------
printf '\n'
if [ "$IS_UPDATE" = 1 ]; then
  info "sontv $TAG 更新完成 → $INSTALL_DIR（配置与凭据未改动）"
else
  info "sontv $TAG 安装完成 → $INSTALL_DIR"
fi

if [ -n "$NEW_TOKEN" ]; then
  cat <<EOF

$C_WARN你的 token 明文（只打印这一次，tokens.txt 里只存了它的 sha256）：$C_OFF
  $NEW_TOKEN

订阅地址（监听 $NEW_LISTEN，以你的 config.json 为准；下面是本机入口，局域网请换成实际 IP）：
  http://127.0.0.1:$PORT/sub?token=$NEW_TOKEN

明文丢了只能换发：重装脚本不会再次打印它。
EOF
fi

if [ "$SERVICE_STARTED" = 1 ]; then
  if [ "$SERVICE_ENABLED" = 1 ]; then
    AUTOSTART_NOTE="服务已注册、已启动、开机自启："
  else
    AUTOSTART_NOTE="服务已启动，但没登记开机自启（上面有原因）："
  fi
  case "$INIT" in
    systemd)
      cat <<EOF

$AUTOSTART_NOTE
  sudo systemctl status $SERVICE_NAME     # 状态与日志
  sudo systemctl reload $SERVICE_NAME     # 改完 tokens.txt 热重载，不用重启
  sudo journalctl -u $SERVICE_NAME -f     # 跟日志
EOF
      ;;
    openrc)
      cat <<EOF

$AUTOSTART_NOTE
  sudo rc-service $SERVICE_NAME status    # 状态
  sudo rc-service $SERVICE_NAME reload    # 改完 tokens.txt 热重载，不用重启
  sudo tail -f /var/log/$SERVICE_NAME/$BINARY.log
EOF
      ;;
  esac
elif [ "$INIT" = none ]; then
  warn "没有 systemd / OpenRC，只能手动跑：sudo $INSTALL_DIR/$BINARY"
fi

if [ ! -f "$INSTALL_DIR/tokens.txt" ]; then
  cat <<EOF

$C_WARN还没有 tokens.txt，服务会以 503 fail closed。$C_OFF 建一份（标签,64位hex[,TTL小时]）：
  TOKEN="\$(openssl rand -hex 24)"
  HASH="\$(printf '%s' "\$TOKEN" | sha256sum | awk '{print \$1}')"   # 明文 \$TOKEN 自己留好
  sudo sh -c "printf '%s\n' '$TOKEN_LABEL,\$HASH' > $INSTALL_DIR/tokens.txt"
  sudo chmod 600 $INSTALL_DIR/tokens.txt
EOF
fi

cat <<EOF

其它：
  自检      sudo $INSTALL_DIR/$BINARY -check
  前台跑    sudo $INSTALL_DIR/$BINARY
  调日志    改 $INSTALL_DIR/config.json 的 log_level（debug/info/warn/error），改完重启服务
EOF

if [ "$IS_UPDATE" = 1 ] && [ "$NO_RESTART" = 1 ]; then
  case "$INIT" in
    systemd) RESTART_HINT="  sudo systemctl restart $SERVICE_NAME" ;;
    openrc)  RESTART_HINT="  sudo rc-service $SERVICE_NAME restart" ;;
    *)       RESTART_HINT="" ;;
  esac
  if [ -n "$RESTART_HINT" ]; then
    cat <<EOF

新版本已就位但没有重启（--no-restart），手动重启后生效：
$RESTART_HINT
EOF
  fi
elif [ "$IS_UPDATE" != 1 ] && [ "$SERVICE_STARTED" != 1 ] && [ "$INIT" != none ]; then
  case "$INIT" in
    systemd) ENABLE_HINT="  sudo systemctl enable --now $SERVICE_NAME" ;;
    openrc)  ENABLE_HINT="  sudo rc-update add $SERVICE_NAME default && sudo rc-service $SERVICE_NAME start" ;;
  esac
  cat <<EOF

想让它常驻，装的时候加上 --service 即可（已装可直接启用）：
$ENABLE_HINT
EOF
fi

printf '\n'