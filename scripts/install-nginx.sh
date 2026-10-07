#!/bin/sh
# install-nginx.sh — 在 Alpine / Debian 上一键装好一个能用的 nginx
#
#   curl -fsSL https://raw.githubusercontent.com/HasonHuang/sontv/main/scripts/install-nginx.sh | sh
#
# Alpine 默认不带 bash，脚本本身是 POSIX sh（bash 也照跑）：
#
#   curl -fsSL … | bash
#
# 做四件事：
#   1. 装 nginx（Alpine 走 apk，Debian / Ubuntu 走 apt）
#   2. 按需指定工作线程用户——默认不动，沿用发行版包自己那个（Alpine 是 nginx，
#      Debian 是 www-data：两家的包本来就以非 root 跑工作线程，没必要再掺一脚）
#   3. 建工作目录（默认 /var/www/html），作为默认站点的 root，整棵目录归工作线程用户
#   4. 写 nginx.conf 与默认站点（覆盖前先备份），注册服务并启动
#
# 有终端时会问这两项，直接回车取默认值；没有终端（curl | sh、CI、ansible）
# 就静默用默认值；命令行或环境变量显式给过的绝不提问。
#
# 只认 Alpine（OpenRC）与 Debian / Ubuntu（systemd）。发行版认不出来、或者系统
# 没有以对应的 init 在跑，都在动任何文件之前退出——装了一半的 nginx 比没装更难
# 收拾：包管理器的 postinst 已经把服务注册进来了，剩下的只能靠人手动清。
#
# 环境变量（等价于 --user / --root）：
#   NGINX_WEB_USER   工作线程用户；留空或 "不修改" = 不动，沿用发行版默认
#   NGINX_WEB_ROOT   工作目录（默认站点的 root），默认 /var/www/html

set -eu

WEB_USER="${NGINX_WEB_USER:-}"            # 空 = 不修改工作线程用户
WEB_ROOT="${NGINX_WEB_ROOT:-}"            # 空 = 用下面的默认值
DEFAULT_WEB_ROOT="/var/www/html"
NGINX_CONF="/etc/nginx/nginx.conf"
MARK="# 由 install-nginx.sh 生成"

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
nginx 安装脚本

用法：
  curl -fsSL https://raw.githubusercontent.com/HasonHuang/sontv/main/scripts/install-nginx.sh | sh

说明：
  在 Alpine（OpenRC）或 Debian / Ubuntu（systemd）上装好 nginx，默认工作目录
  /var/www/html。nginx.conf 与默认站点会先备份到 /etc/nginx/backup/ 再覆盖。
  有终端时会问「工作线程用户」和「工作目录」两项，回车取默认值。

选项：
  -u, --user NAME  工作线程用户；给 "不修改"（默认）就沿用发行版包自己那个
  -r, --root DIR   工作目录，默认 /var/www/html
  -h, --help       显示本帮助

环境变量：
  NGINX_WEB_USER   同 --user
  NGINX_WEB_ROOT   同 --root
EOF
}

# 工作线程用户：默认不改。留空、或字面写 "不修改"，都表示沿用现有 nginx.conf
# 里那个 user 指令（发行版的包本来就以非 root 跑工作线程）。
if [ -n "$WEB_USER" ] && [ "$WEB_USER" != "不修改" ]; then
  USER_GIVEN=1
else
  USER_GIVEN=0
  WEB_USER=""
fi

if [ -z "$WEB_ROOT" ]; then
  WEB_ROOT="$DEFAULT_WEB_ROOT"
  ROOT_GIVEN=0
else
  ROOT_GIVEN=1
fi

while [ $# -gt 0 ]; do
  case "$1" in
    -u|--user)
      [ $# -ge 2 ] || die "--user 需要参数"
      if [ -n "$2" ] && [ "$2" != "不修改" ]; then USER_GIVEN=1; WEB_USER="$2"; else USER_GIVEN=0; WEB_USER=""; fi
      shift 2 ;;
    -r|--root)
      [ $# -ge 2 ] || die "--root 需要参数"
      WEB_ROOT="$2"; ROOT_GIVEN=1; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; die "未知参数：$1" ;;
  esac
done

# 用户名会进 nginx.conf 的 user 指令、chown 的属主和 adduser 的参数表，限死成
# 字母数字下划线连字符，免得从命令行/环境变量里塞进来的 ";" 或 "-D" 逃到别处去。
valid_user_name() {
  case "$1" in
    ''|-*|*[!A-Za-z0-9_-]*) return 1 ;;
  esac
  return 0
}

# 同理：nginx 的 root 指令只吃绝对路径，顺手把 mkdir/chown 的参数注入也堵掉。
valid_web_root() {
  case "$1" in
    /*) return 0 ;;
  esac
  return 1
}

# 显式给过（命令行/环境变量）的值只校验不追问，措辞一次说清怎么改。
if [ "$USER_GIVEN" = 1 ]; then
  valid_user_name "$WEB_USER" \
    || die "工作线程用户名只接受字母、数字、下划线与连字符，且不能以 - 开头，收到：$WEB_USER"
fi
if [ "$ROOT_GIVEN" = 1 ]; then
  valid_web_root "$WEB_ROOT" \
    || die "工作目录必须是绝对路径，收到：$WEB_ROOT"
fi

# ---------- 权限 ----------
if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
elif command -v sudo >/dev/null 2>&1; then
  SUDO="sudo"
else
  die "需要 root 权限：本机既不是 root，也没有 sudo，请换 root 重跑"
fi
run_root() { if [ -n "$SUDO" ]; then sudo "$@"; else "$@"; fi; }

BACKUP_DIR="/etc/nginx/backup"

# 覆盖前留一份原件。备份一律放到 $BACKUP_DIR，绝不放回原目录——nginx 会 include
# conf.d/*.conf、sites-enabled/*、http.d/*.conf，备份躺在那里就会被当成第二份
# 配置加载：Debian 上表现为 default_server 撞车，nginx -t 直接不过。
# 已经是本脚本写的（带 MARK）就直接覆盖，免得重跑一次攒一堆 .bak——反复重跑
# 本来就该是安全操作。
backup_if_users() { # 文件
  _f="$1"
  [ -e "$_f" ] || return 0
  grep -q "$MARK" "$_f" 2>/dev/null && return 0
  run_root mkdir -p "$BACKUP_DIR"
  _b="$BACKUP_DIR/$(basename "$_f").bak.$(date +%Y%m%d%H%M%S)"
  if run_root cp -p "$_f" "$_b"; then
    info "备份      $_f → $_b"
  else
    warn "备份 $_f 失败，为免丢配置就此停下"
    exit 1
  fi
}

# ---------- 交互 ----------
# 一律走 /dev/tty，不碰 stdin：`curl … | sh` 时 stdin 是脚本管道本身，
# 读它会把剩下的脚本文本吃掉。可用性靠「真能打开」判断，不能只看 [ -r/-w ]：
# docker exec 这类环境里 /dev/tty 存在且权限正常，打开却会失败。
# 放子 shell 里试，免得失败的重定向把整个脚本带走（脚本自己有 set -e）。
TTY=""
if ( : 2>/dev/null >/dev/tty ) 2>/dev/null; then
  TTY="/dev/tty"
fi

# ask 提示 默认值 → 结果放 ASK_REPLY。回车或 EOF 都取默认值：不想回答的人
# 一路回车到底，得到的正是一份完整的默认安装。
ask() {
  if [ -z "$TTY" ]; then
    ASK_REPLY="$2"
    return 0
  fi
  printf '%s [%s] ' "$1" "$2" >"$TTY"
  IFS= read -r ASK_REPLY <"$TTY" || ASK_REPLY=""
  [ -n "$ASK_REPLY" ] || ASK_REPLY="$2"
}

# 答错就重新问，而不是报错退出——交互中途被一句「非法取值」打断，
# 比多问一句烦人得多。回车（= 取默认值）永远合法。
ask_worker_user() {
  while :; do
    ask "工作线程用户（回车=不修改，沿用发行版的 $DEFAULT_WORKER_USER）" "不修改"
    if [ "$ASK_REPLY" = "不修改" ]; then
      USER_GIVEN=0
      return 0
    fi
    if valid_user_name "$ASK_REPLY"; then
      WEB_USER="$ASK_REPLY"
      USER_GIVEN=1
      return 0
    fi
    printf '%s不是合法的用户名（只接受字母、数字、下划线与连字符），请重新输入。\n' \
      "$ASK_REPLY" >"$TTY"
  done
}

ask_web_root() {
  while :; do
    ask "工作目录（站点根目录）" "$DEFAULT_WEB_ROOT"
    if valid_web_root "$ASK_REPLY"; then
      WEB_ROOT="$ASK_REPLY"
      ROOT_GIVEN=1
      return 0
    fi
    printf '%s不是绝对路径，请重新输入。\n' "$ASK_REPLY" >"$TTY"
  done
}

# ---------- 发行版与 init：装任何东西之前先确认 ----------
if [ -f /etc/alpine-release ]; then
  OS=alpine
  INIT=openrc
  # 包自己用的工作线程用户，也是「不修改」时的默认值。
  DEFAULT_WORKER_USER="nginx"
elif [ -f /etc/os-release ]; then
  OS="$(. /etc/os-release 2>/dev/null && printf '%s' "${ID:-unknown}")"
  case "$OS" in
    debian|ubuntu) ;;
    *) die "只支持 Alpine / Debian / Ubuntu（当前：$OS）" ;;
  esac
  INIT=systemd
  DEFAULT_WORKER_USER="www-data"
else
  die "认不出这是什么系统：/etc/alpine-release 与 /etc/os-release 都不存在"
fi

case "$INIT" in
  systemd)
    # /run/systemd/system 只在 systemd 真的作为 PID 1 跑着时才存在。纯容器里
    # 装了 systemctl 也过不了这一关——那种环境下 systemctl 一用就报
    # "System has not been booted with systemd"，注册和启动都会失败。
    if ! command -v systemctl >/dev/null 2>&1 || [ ! -d /run/systemd/system ]; then
      die "$OS 要靠 systemd 管理 nginx，但本机没有以 systemd 作为 PID 1 运行（缺 systemctl 或不是 systemd 引导）"
    fi
    ;;
  openrc)
    # 只看 rc-service 在不在。Alpine 里 openrc 是个普通的包，没装的话
    # 后面 nginx 装上了也起不来服务，不如现在就停。
    if ! command -v rc-service >/dev/null 2>&1; then
      die "Alpine 要靠 OpenRC 管理 nginx，但本机没有 rc-service（先 apk add openrc）"
    fi
    ;;
esac
info "系统      $OS（$(uname -m)），服务由 $INIT 管理"

# 环境探明（发行版、init、默认用户是谁）之后再问，答得才有底；反过来先问一串，
# 用户还不知道这机器上有没有 systemd，答也答得没底气。两项都只在没显式给过时问。
if [ -n "$TTY" ]; then
  if [ "$USER_GIVEN" = 0 ] || [ "$ROOT_GIVEN" = 0 ]; then
    printf '\n检测到终端，下面几项直接回车取默认值。\n\n'
    if [ "$USER_GIVEN" = 0 ]; then
      ask_worker_user
    fi
    if [ "$ROOT_GIVEN" = 0 ]; then
      ask_web_root
    fi
    printf '\n'
  fi
fi

# ---------- 装 nginx ----------
if command -v nginx >/dev/null 2>&1; then
  info "nginx     已安装（$(nginx -v 2>&1 | sed 's/^nginx version: //')），继续按本脚本的约定重写配置"
else
  info "nginx     安装中（$OS）"
  # 两个包管理器都自己报错，但 set -e 会把它们的输出留在屏幕上然后无声退出；
  # 包一句「装包失败」比让用户自己猜要省事。
  if [ "$OS" = alpine ]; then
    run_root apk add --no-cache nginx || die "apk add nginx 失败，检查网络与 /etc/apk/repositories 后重跑"
  else
    run_root apt-get update || die "apt-get update 失败，检查网络与软件源后重跑"
    # 非交互：容器里 apt 没有 tzdata 之类的配置输入，会卡在对话框上。
    run_root env DEBIAN_FRONTEND=noninteractive apt-get install -y nginx \
      || die "apt-get install nginx 失败，处理完后重跑"
  fi
fi
command -v nginx >/dev/null 2>&1 || die "装完仍找不到 nginx，请手动装一次再重跑本脚本"

# ---------- 工作线程用户 ----------
# 「不修改」时要从现有 nginx.conf 里把 user 指令原样捞出来。第一遍跑时它是包
# 自带的，重跑时是上一轮本脚本写的——两种都算「现状」，一并沿用。
conf_user_directive() { # → "user nginx;" 这样的整行（不含缩进），没有则空
  [ -f "$NGINX_CONF" ] || return 0
  sed -n 's/^[[:space:]]*\(user[[:space:]][^;]*\);.*/\1;/p' "$NGINX_CONF" | head -n 1
}
conf_user_name() { # → 上面的第一个字段（用户名）
  [ -f "$NGINX_CONF" ] || return 0
  sed -n 's/^[[:space:]]*user[[:space:]][[:space:]]*\([A-Za-z0-9_.-]*\).*/\1/p' "$NGINX_CONF" | head -n 1
}

# 默认不动：两个发行版的包本来就以非 root 跑工作线程（Alpine 是 nginx，
# Debian 是 www-data），没理由默认再掺一脚。显式指定了才建/改，并写进新配置。
#
# 两条路都得落到一个真实存在的用户名上——后面 chown 站点目录要用它，
# 工作线程读不到自己的工作目录就是一片 403。
if [ "$USER_GIVEN" = 1 ]; then
  # 组要先单独保证：Alpine 的 busybox adduser -S 不会建同名组，而是把用户塞进
  # nogroup；Debian 的 useradd 也未必建同名组。组不对的话，nginx.conf 里就成了
  # "user www nogroup"，chown www:www 也会因为组不存在而失败。
  WORKER_GROUP="$WEB_USER"
  if ! grep -q "^$WORKER_GROUP:" /etc/group 2>/dev/null; then
    # busybox 只有 addgroup，shadow 只有 groupadd，各试一次。
    if command -v groupadd >/dev/null 2>&1; then
      run_root groupadd -r "$WORKER_GROUP" >/dev/null 2>&1 || true
    fi
    if ! grep -q "^$WORKER_GROUP:" /etc/group 2>/dev/null \
       && command -v addgroup >/dev/null 2>&1; then
      run_root addgroup -S "$WORKER_GROUP" >/dev/null 2>&1 || true
    fi
  fi

  # 三种 adduser/useradd 的参数语义完全不同：shadow 的 useradd、busybox 的
  # adduser、Debian 的 perl adduser（只吃长选项）。依次尝试，建出来了就停——
  # 失败信息一律咽掉，最后统一验人。同 install.sh。
  if id "$WEB_USER" >/dev/null 2>&1; then
    info "用户      $WEB_USER 已存在，沿用"
  else
    if command -v useradd >/dev/null 2>&1; then
      run_root useradd -r -M -g "$WORKER_GROUP" -s /usr/sbin/nologin "$WEB_USER" \
        >/dev/null 2>&1 || true
    fi
    if ! id "$WEB_USER" >/dev/null 2>&1; then
      run_root adduser -S -D -H -G "$WORKER_GROUP" -s /sbin/nologin "$WEB_USER" \
        >/dev/null 2>&1 || true
    fi
    if ! id "$WEB_USER" >/dev/null 2>&1; then
      run_root adduser --system --no-create-home --ingroup "$WORKER_GROUP" \
        --shell /usr/sbin/nologin "$WEB_USER" >/dev/null 2>&1 || true
    fi
    id "$WEB_USER" >/dev/null 2>&1 || die "建不出用户 $WEB_USER，请手动建一个系统用户再重跑"
    info "用户      $WEB_USER 已创建（不可登录的系统用户）"
  fi

  WORKER_USER="$WEB_USER"
  # 组名以系统里实际的为准：用户可能早就存在，主组未必是我们刚建的那个。
  WORKER_GROUP="$(id -gn "$WORKER_USER")"
  USER_DIRECTIVE="user $WORKER_USER $WORKER_GROUP;"
else
  WORKER_USER="$(conf_user_name)"
  if [ -z "$WORKER_USER" ]; then
    WORKER_USER="$DEFAULT_WORKER_USER"
  fi
  id "$WORKER_USER" >/dev/null 2>&1 \
    || die "要沿用的工作线程用户 $WORKER_USER 不存在，请用 --user 指定一个"
  WORKER_GROUP="$(id -gn "$WORKER_USER")"

  # 连组名都照抄，做到字面意义上的「不修改」；原配置里没有 user 行才补一条。
  USER_DIRECTIVE="$(conf_user_directive)"
  if [ -z "$USER_DIRECTIVE" ]; then
    USER_DIRECTIVE="user $WORKER_USER;"
  fi
  info "用户      $WORKER_USER:$WORKER_GROUP（未改动，沿用发行版默认）"
fi

# ---------- 发行版的路径差异 ----------
# pid 与临时目录都跟包自己的约定对齐，别让配置文件和服务脚本各说各话：
# Alpine 的 /etc/init.d/nginx 用 /run/nginx/nginx.pid，Debian 的 nginx.service
# 里写着 PIDFile=/run/nginx.pid，pid 对不上时服务启停会认不出自己的进程。
if [ "$OS" = alpine ]; then
  PID_FILE="/run/nginx/nginx.pid"
  # 包编译进去的默认值：临时目录在 tmp/ 下，且 body 那个叫 client_body。
  TMP_ROOT="/var/lib/nginx/tmp"
  TMP_BODY="$TMP_ROOT/client_body"
  # Alpine 的包约定：modules/ 与 conf.d/ 装的是 root 上下文片段，站点放 http.d/。
  ROOT_INCLUDE="    include /etc/nginx/modules/*.conf;
    include /etc/nginx/conf.d/*.conf;"
  SITE_INCLUDE="    include /etc/nginx/http.d/*.conf;"
  SITE_DIR="/etc/nginx/http.d"
else
  PID_FILE="/run/nginx.pid"
  # Debian 用 /var/lib/nginx/{body,proxy,…}
  TMP_ROOT="/var/lib/nginx"
  TMP_BODY="$TMP_ROOT/body"
  # Debian 的包约定：modules-enabled/ 是动态模块，conf.d/ 与 sites-enabled/ 同属 http 上下文。
  ROOT_INCLUDE="    include /etc/nginx/modules-enabled/*.conf;"
  SITE_INCLUDE="    include /etc/nginx/conf.d/*.conf;
    include /etc/nginx/sites-enabled/*;"
  SITE_DIR="/etc/nginx/conf.d"
fi
TMP_PROXY="$TMP_ROOT/proxy"
TMP_FASTCGI="$TMP_ROOT/fastcgi"
TMP_UWSGI="$TMP_ROOT/uwsgi"
TMP_SCGI="$TMP_ROOT/scgi"
SITE_FILE="$SITE_DIR/default.conf"

# ---------- 目录与属主 ----------
run_root mkdir -p "$WEB_ROOT" /var/log/nginx "$SITE_DIR"
if [ "$OS" != alpine ]; then
  run_root mkdir -p /etc/nginx/sites-enabled
fi
for _d in "$(dirname "$PID_FILE")" "$TMP_BODY" "$TMP_PROXY" "$TMP_FASTCGI" "$TMP_UWSGI" "$TMP_SCGI"; do
  run_root mkdir -p "$_d"
done

# 工作线程是 $WORKER_USER，这些目录得归它：客户端上传的 body、反代攒的响应都落在
# 临时目录里，属主不对就是一动就 500（error.log 里只写 permission denied，
# 不会告诉你是哪个目录）。-R 覆盖工作目录下已有的站点文件，否则工作线程读不到
# 就是一片 403。默认（不修改用户）时这些 chown 通常本来就是对的，属于空操作。
run_root chown -R "$WORKER_USER:$WORKER_GROUP" "$WEB_ROOT" /var/lib/nginx /var/log/nginx
run_root chmod 0755 "$WEB_ROOT"

# ---------- 生成配置 ----------
TMPDIR_="$(mktemp -d)"

# IPv6：容器和精简系统上常常没有，硬写 listen [::]:80 会让 nginx 直接起不来
# （socket() [::]:80 failed (97: Address family not supported by protocol)）。
if [ -f /proc/net/if_inet6 ]; then
  IPV6_LISTEN="    listen       [::]:80 default_server;"
else
  IPV6_LISTEN=""
fi

cat > "$TMPDIR_/nginx.conf" <<CONF
$MARK —— 重跑脚本会覆盖这个文件。
# 自己的站点别写在这里：Alpine 放 /etc/nginx/http.d/，Debian 放
# /etc/nginx/conf.d/ 或 /etc/nginx/sites-enabled/，本文件已经 include 了它们。

# 工作线程跑在 $WORKER_USER 下；master 仍是 root，靠它去 bind 80 端口、开日志、
# 以该用户的身份 fork worker。「不修改」时这一行就是原配置那一行的原文照抄。
$USER_DIRECTIVE
worker_processes  auto;
pid  $PID_FILE;
error_log  /var/log/nginx/error.log warn;

# 发行版自己的模块/片段入口。动态模块（Debian 的 geoip、image-filter 之类）
# 靠它加载，漏掉的话别人已有的站点一升级就起不来。
$ROOT_INCLUDE

events {
    worker_connections  1024;
}

http {
    include       /etc/nginx/mime.types;
    default_type  application/octet-stream;

    log_format  main  '\$remote_addr - \$remote_user [\$time_local] "\$request" '
                      '\$status \$body_bytes_sent "\$http_referer" '
                      '"\$http_user_agent" "\$http_x_forwarded_for"';

    access_log  /var/log/nginx/access.log  main;

    sendfile          on;
    tcp_nopush        on;
    keepalive_timeout 65;
    types_hash_max_size 2048;
    server_tokens     off;

    # 工作线程是 $WORKER_USER，这几个目录必须归它所有，否则一上传就 500。
    client_body_temp_path  $TMP_BODY;
    proxy_temp_path        $TMP_PROXY;
    fastcgi_temp_path      $TMP_FASTCGI;
    uwsgi_temp_path        $TMP_UWSGI;
    scgi_temp_path         $TMP_SCGI;

$SITE_INCLUDE
}
CONF

cat > "$TMPDIR_/default.conf" <<SITE
$MARK —— 重跑脚本会覆盖这个文件。
server {
    listen       80 default_server;
$IPV6_LISTEN
    server_name  _;
    root         $WEB_ROOT;
    index        index.html index.htm;

    location / {
        try_files \$uri \$uri/ =404;
    }

    # 反代示例：把 /api/ 交给本机 9900 上的 sontv。
    # 直播、SSE 这类长连接务必 proxy_buffering off——开着 nginx 会先攒够缓冲
    # 再发，延迟从秒级涨到十几秒。
    #location /api/ {
    #    proxy_pass http://127.0.0.1:9900/;
    #    proxy_http_version 1.1;
    #    proxy_set_header Host \$host;
    #    proxy_set_header X-Real-IP \$remote_addr;
    #    proxy_buffering off;
    #}
}
SITE

# Debian / Ubuntu 自带的 default 站点占着 80 的 default_server，不摘掉就和我们的站点
# 撞车，nginx -t 直接报 "duplicate default server for 0.0.0.0:80"。只删 sites-enabled
# 里的入口，sites-available/default 原文件留着。
if [ "$OS" != alpine ] && [ -e /etc/nginx/sites-enabled/default ]; then
  if [ -L /etc/nginx/sites-enabled/default ]; then
    # 发行版装出来的是软链，本体在 sites-available/default：删链不删文件，
    # 想恢复只要把链接补回去。
    run_root rm -f /etc/nginx/sites-enabled/default
    info "站点      已停用发行版自带 default 站点（本体 /etc/nginx/sites-available/default 保留）"
  else
    backup_if_users /etc/nginx/sites-enabled/default
    run_root rm -f /etc/nginx/sites-enabled/default
    info "站点      已停用发行版自带 default 站点（原文件已备份）"
  fi
fi

backup_if_users "$SITE_FILE"
backup_if_users "$NGINX_CONF"
run_root install -m 0644 "$TMPDIR_/default.conf" "$SITE_FILE"
run_root install -m 0644 "$TMPDIR_/nginx.conf" "$NGINX_CONF"

if [ ! -f "$WEB_ROOT/index.html" ]; then
  cat > "$TMPDIR_/index.html" <<HTML
<!doctype html>
<html lang="zh-CN">
<head><meta charset="utf-8"><title>nginx 已就绪</title></head>
<body>
<h1>nginx 已就绪</h1>
<p>工作目录：$WEB_ROOT</p>
</body>
</html>
HTML
  run_root cp "$TMPDIR_/index.html" "$WEB_ROOT/index.html"
  run_root chown "$WORKER_USER:$WORKER_GROUP" "$WEB_ROOT/index.html"
  run_root chmod 0644 "$WEB_ROOT/index.html"
fi

# 交付给服务之前先自己验一遍。服务的 ExecStartPre / start_pre 里也各有一道
# -t，但那是服务的输出，进了 journal 或 error.log；在这里验，报错是直接打在
# 用户脸上的，不用再去翻日志。失败也不动服务：正在跑的 nginx 还是旧配置，
# 站点不会因为一次改配置而中断。
if ! run_root nginx -t -c "$NGINX_CONF" >"$TMPDIR_/nginx-t.log" 2>&1; then
  cat "$TMPDIR_/nginx-t.log" >&2
  die "nginx -t 不通过（配置已落盘但没重载，正在跑的 nginx 仍是旧配置）：改好 $NGINX_CONF 后重跑，或从同目录的 *.bak.* 还原"
fi

# ---------- 注册并启动 ----------
case "$INIT" in
  systemd)
    run_root systemctl enable nginx >/dev/null 2>&1 || true
    # 用 restart 而不是 start：apt 装完 nginx 已经按旧配置起来了，对已在跑的
    # 单元 start 是 no-op，新配置永远不会生效。restart 对没在跑的单元等价于 start。
    if run_root systemctl restart nginx; then
      info "服务      已启动（systemctl status nginx）"
    else
      warn "服务      启动失败：systemctl status nginx / journalctl -u nginx -n 50"
    fi
    ;;
  openrc)
    # OpenRC 没真正 boot 过的环境（典型：docker run alpine 后 apk add openrc）
    # 里，rc-service 常常一个服务都起不来。补 softlevel 标记是通行的第一招，
    # openrc 自己的诊断也指向它；正常开过机的系统上它已存在，这里是空操作。
    # 它治不了全部情况——缺 /run/openrc 里的状态目录时仍会被拒（见下面的失败分支）。
    if [ ! -e /run/openrc/softlevel ]; then
      run_root mkdir -p /run/openrc
      run_root touch /run/openrc/softlevel
      info "OpenRC    补建 /run/openrc/softlevel（本机 openrc 未 boot）"
    fi

    run_root rc-update add nginx default >/dev/null 2>&1 || true
    # OpenRC 的 restart 撞上没在跑的服务会先报一句 "not running"，所以先看状态。
    if run_root rc-service nginx status >/dev/null 2>&1; then
      ACTION=restart
      LABEL=重启
    else
      ACTION=start
      LABEL=启动
    fi
    if run_root rc-service nginx "$ACTION" >"$TMPDIR_/rc-service.log" 2>&1; then
      info "服务      已$LABEL（rc-service nginx status）"
    else
      # 把 openrc 自己的话原样放出来，再去解释。
      cat "$TMPDIR_/rc-service.log" >&2
      warn "服务      $ACTION 失败：rc-service nginx status / tail -n 50 /var/log/nginx/error.log"
      # "already starting" 是 OpenRC 从没 boot 过的症状：/run/openrc 下的状态目录
      # 是 boot 阶段建的，缺了它 rc-service 起任何服务都是这一句（连它自带的
      # hostname 都起不来）。跟 nginx 配置无关，点破一下，省得人去翻 error.log。
      if grep -q "already starting" "$TMPDIR_/rc-service.log" 2>/dev/null; then
        warn "本机 OpenRC 从未 boot 过，这种环境下 rc-service 起不了任何服务（不是配置问题）"
        warn "容器里可以直接跑 nginx -c $NGINX_CONF；想让它受 OpenRC 管，PID 1 得是 openrc-init/init"
      fi
    fi
    ;;
esac

# ---------- 收尾 ----------
if [ "$INIT" = systemd ]; then
  if systemctl is-active --quiet nginx 2>/dev/null; then ACTIVE=1; else ACTIVE=0; fi
  MANAGER="systemctl restart nginx ／ systemctl status nginx"
  RESTART_CMD="systemctl restart nginx"
  LOGHINT="journalctl -u nginx -n 50"
else
  if rc-service nginx status >/dev/null 2>&1; then ACTIVE=1; else ACTIVE=0; fi
  MANAGER="rc-service nginx restart ／ rc-service nginx status"
  RESTART_CMD="rc-service nginx restart"
  LOGHINT="tail -n 50 /var/log/nginx/error.log"
fi

if [ "$ACTIVE" = 1 ]; then
  info "完成      nginx 已就绪：http://<本机 IP>/"
else
  warn "完成      nginx 没在运行，先看：$LOGHINT"
fi

if [ "$USER_GIVEN" = 1 ]; then
  WORKER_NOTE="按指定"
else
  WORKER_NOTE="沿用发行版默认，未改动"
fi

cat <<EOF

  版本       $(nginx -v 2>&1 | sed 's/^nginx version: //')
  工作线程   $WORKER_USER:$WORKER_GROUP（master 仍是 root；$WORKER_NOTE）
  工作目录   $WEB_ROOT        ← 站点文件放这里
  主配置     $NGINX_CONF
  站点配置   $SITE_FILE
  备份目录   $BACKUP_DIR
  加站点     $SITE_DIR/ 下新建 *.conf，然后 $RESTART_CMD
  服务       $MANAGER
  出错看     $LOGHINT
EOF
