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
AUTO_TOKEN="${SONTV_NO_TOKEN:-0}"    # 1 = 不自动生成 token，保持「只装不管凭据」
TOKEN_LABEL="${SONTV_TOKEN_LABEL:-我的订阅}"
SERVICE_USER="${SONTV_USER:-sontv}"
SERVICE_GROUP=""                       # 装服务时按 id -gn 现查，缺省与用户名同名

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
      --flavor F      强制使用 glibc 或 musl 产物
      --force-config  覆盖已存在的 config.json（默认保留）
      --no-token      不自动生成 tokens.txt（默认缺凭据时生成一条并打印明文）
      --token-label L 自动生成的那条 token 的标签，默认「我的订阅」
      --service       注册并启动系统服务（systemd / OpenRC 自适应）；缺省就是开的
      --no-service    只装文件，不碰服务（等价 SONTV_SERVICE=0）
  -h, --help          显示本帮助

环境变量（等价于上面的选项）：
  SONTV_VERSION SONTV_INSTALL_DIR SONTV_FLAVOR SONTV_REINSTALL_CONFIG
  SONTV_SERVICE SONTV_REPO SONTV_NO_TOKEN SONTV_TOKEN_LABEL SONTV_USER

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
    --flavor)     [ $# -ge 2 ] || die "--flavor 需要参数";  FORCE_FLAVOR="$2"; shift 2 ;;
    --force-config) FORCE_CONFIG=1; shift ;;
    --no-token)   AUTO_TOKEN=1; shift ;;
    --token-label) [ $# -ge 2 ] || die "--token-label 需要参数"; TOKEN_LABEL="$2"; shift 2 ;;
    --service)    ENABLE_SERVICE=1; shift ;;
    --no-service) ENABLE_SERVICE=0; shift ;;
    -h|--help)    usage; exit 0 ;;
    *)            usage >&2; die "未知参数：$1" ;;
  esac
done

# 标签会原样写进 tokens.txt 的第一列：逗号是分隔符，控制字符（含换行）会截断整行。
printf '%s' "$TOKEN_LABEL" | grep -q '[,[:cntrl:]]' \
  && die "token 标签不能含逗号或控制字符：$TOKEN_LABEL" || true

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
[ "$INIT" = none ] || info "服务管理  $INIT"

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
run_root install -m 0755 "$TMPDIR_/$BINARY" "$INSTALL_DIR/$BINARY"

if [ -f "$INSTALL_DIR/config.json" ] && [ "$FORCE_CONFIG" != 1 ]; then
  info "配置      已存在，保持不变（--force-config 可覆盖）"
else
  # 包里没有 config.json 时给个空文件，让服务以全缺省启动而不是因缺文件退出。
  run_root install -m 0644 "${TMPDIR_}/config.json" "$INSTALL_DIR/config.json" 2>/dev/null \
    || run_root install -m 0644 /dev/null "$INSTALL_DIR/config.json"
  info "配置      已写入 $INSTALL_DIR/config.json"
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
  # 命令替换里可能一条实现都没有，用 || true 兜住 set -e，之后再判空。
  NEW_TOKEN="$(random_hex 24 | tr -d '\n' || true)"
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

# 写 /etc/init.d/sontv。$1 = supervise（supervise-daemon 托管，有崩溃重启与日志）
# 或 plain（start-stop-daemon 后台，老 OpenRC 或容器里 supervise-daemon 起不来时用）。
write_openrc_initd() {
  if [ "$1" = supervise ]; then
    SUPERVISOR="supervisor=supervise-daemon
output_log=\"/var/log/sontv/sontv.log\"
error_log=\"/var/log/sontv/sontv.log\"
respawn_delay=5
respawn_max=0"
    RELOAD_CMD="start-stop-daemon --signal USR1 --name $BINARY"
  else
    SUPERVISOR="command_background=\"yes\"
pidfile=\"/run/sontv.pid\""
    RELOAD_CMD="start-stop-daemon --signal USR1 --pidfile /run/sontv.pid"
  fi
  cat > "$TMPDIR_/sontv.initd" <<INITD
#!/sbin/openrc-run
# sontv — 由 install.sh 生成，重跑脚本会覆盖

name="sontv"
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
	checkpath --directory --owner "$SERVICE_USER:$SERVICE_GROUP" --mode 0755 /var/log/sontv
}

# 改完 tokens.txt 热重载，不必重启（对应 systemd 的 ExecReload）。
reload() {
	ebegin "重载 \$name"
	$RELOAD_CMD >/dev/null 2>&1
	eend \$?
}
INITD
  run_root install -m 0755 "$TMPDIR_/sontv.initd" /etc/init.d/sontv
}

SERVICE_STARTED=0
SERVICE_ENABLED=0   # 是否已登记开机自启
if [ "$ENABLE_SERVICE" = 1 ]; then
  case "$INIT" in
    systemd)
      ensure_service_user
      cat > "$TMPDIR_/sontv.service" <<UNIT
[Unit]
Description=sontv - IPTV subscription proxy
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
      run_root install -m 0644 "$TMPDIR_/sontv.service" /etc/systemd/system/sontv.service
      run_root systemctl daemon-reload
      if run_root systemctl enable --now sontv; then
        SERVICE_STARTED=1
        SERVICE_ENABLED=1
        info "服务      已注册并启动（systemctl status sontv）"
      else
        warn "服务单元已写入，但启动失败：systemctl status sontv"
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
      if run_root rc-service sontv start; then
        SERVICE_STARTED=1
        info "服务      已启动（rc-service sontv status，$MODE 模式）"
      elif [ "$MODE" = supervise ]; then
        warn "supervise-daemon 起不来，降级为 start-stop-daemon 重试"
        write_openrc_initd plain
        if run_root rc-service sontv start; then
          SERVICE_STARTED=1
          info "服务      已启动（rc-service sontv status，plain 模式）"
        fi
      fi
      [ "$SERVICE_STARTED" = 1 ] || warn "服务脚本已写入，但启动失败：rc-service sontv start"

      # default 运行级 = 开机自启。放在启动之后登记：openrc 从没 boot 过的
      # 容器里，/run/openrc 状态是服务起过一次才齐的，早跑容易失败。
      if run_root rc-update add sontv default >/dev/null 2>&1; then
        SERVICE_ENABLED=1
      else
        warn "rc-update add 失败（当前环境多半没有真实 init），不会开机自启；需要时手动：rc-update add sontv default"
      fi
      ;;
    *)
      warn "没找到 systemd 或 OpenRC，跳过服务注册（可手动跑 $INSTALL_DIR/$BINARY）"
      ;;
  esac
fi

# ---------- 完成 ----------
printf '\n'
info "sontv $TAG 安装完成 → $INSTALL_DIR"

if [ -n "$NEW_TOKEN" ]; then
  cat <<EOF

$C_WARN你的 token 明文（只打印这一次，tokens.txt 里只存了它的 sha256）：$C_OFF
  $NEW_TOKEN

订阅地址（listen 缺省 0.0.0.0:9900，按你的 config.json 为准；下面是本机入口，局域网请换成实际 IP）：
  http://127.0.0.1:9900/sub?token=$NEW_TOKEN

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
  sudo systemctl status sontv     # 状态与日志
  sudo systemctl reload sontv     # 改完 tokens.txt 热重载，不用重启
  sudo journalctl -u sontv -f     # 跟日志
EOF
      ;;
    openrc)
      cat <<EOF

$AUTOSTART_NOTE
  sudo rc-service sontv status    # 状态
  sudo rc-service sontv reload    # 改完 tokens.txt 热重载，不用重启
  sudo tail -f /var/log/sontv/sontv.log
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
EOF

if [ "$SERVICE_STARTED" != 1 ] && [ "$INIT" != none ]; then
  case "$INIT" in
    systemd) ENABLE_HINT="  sudo systemctl enable --now sontv" ;;
    openrc)  ENABLE_HINT="  sudo rc-update add sontv default && sudo rc-service sontv start" ;;
  esac
  cat <<EOF

想让它常驻，装的时候加上 --service 即可（已装可直接启用）：
$ENABLE_HINT
EOF
fi

printf '\n'