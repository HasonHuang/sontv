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
ENABLE_SERVICE="${SONTV_SERVICE:-0}"

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
      --service       注册并启动 systemd 服务（非 systemd 系统会跳过）
  -h, --help          显示本帮助

环境变量（等价于上面的选项）：
  SONTV_VERSION SONTV_INSTALL_DIR SONTV_FLAVOR SONTV_REINSTALL_CONFIG
  SONTV_SERVICE SONTV_REPO

无参数时安装到 /opt/sontv；已存在的 config.json 与 tokens.txt 不会被覆盖。
EOF
}

# ---------- 参数 ----------
while [ $# -gt 0 ]; do
  case "$1" in
    -v|--version) [ $# -ge 2 ] || die "--version 需要参数"; VERSION="$2"; shift 2 ;;
    -d|--dir)     [ $# -ge 2 ] || die "--dir 需要参数";     INSTALL_DIR="$2"; shift 2 ;;
    --flavor)     [ $# -ge 2 ] || die "--flavor 需要参数";  FORCE_FLAVOR="$2"; shift 2 ;;
    --force-config) FORCE_CONFIG=1; shift ;;
    --service)    ENABLE_SERVICE=1; shift ;;
    -h|--help)    usage; exit 0 ;;
    *)            usage >&2; die "未知参数：$1" ;;
  esac
done

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

# 凭据文件由使用者自己创建，脚本不碰；已有则顺手把权限收紧。
if [ -f "$INSTALL_DIR/tokens.txt" ]; then
  run_root chmod 0600 "$INSTALL_DIR/tokens.txt" || true
fi

# 冒烟自检：只有凭据就位时才跑（缺 tokens.txt 会 fail closed 直接报错）。
# config.json 的相对路径按二进制同级目录解析，所以不需要先 cd。
if [ -f "$INSTALL_DIR/tokens.txt" ]; then
  # </dev/null：脚本被 `curl | bash` 喂进来时 stdin 就是管道本身，
  # 不挡住的话二进制一旦读 stdin 就会把脚本文本吃掉。
  if run_root "$INSTALL_DIR/$BINARY" -check </dev/null >/dev/null 2>&1; then
    info "自检      通过"
  else
    warn "自检未通过，请手动执行：$INSTALL_DIR/$BINARY -check"
  fi
fi

# ---------- systemd（可选） ----------
if [ "$ENABLE_SERVICE" = 1 ]; then
  if ! command -v systemctl >/dev/null 2>&1; then
    warn "该系统没有 systemd，跳过服务注册"
  else
    if ! id sontv >/dev/null 2>&1; then
      if command -v useradd >/dev/null 2>&1; then
        run_root useradd -r -s /usr/sbin/nologin sontv || true
      elif command -v adduser >/dev/null 2>&1; then
        run_root adduser -S -D -H -s /sbin/nologin sontv || true
      fi
    fi
    run_root chown -R sontv "$INSTALL_DIR" 2>/dev/null || true

    run_root tee /etc/systemd/system/sontv.service >/dev/null <<UNIT
[Unit]
Description=sontv - IPTV subscription proxy
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=sontv
Group=sontv
WorkingDirectory=$INSTALL_DIR
ExecStart=$INSTALL_DIR/$BINARY
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=full
ProtectHome=true

[Install]
WantedBy=multi-user.target
UNIT

    run_root systemctl daemon-reload
    run_root systemctl enable --now sontv
    info "服务      已注册并启动（systemctl status sontv）"
  fi
fi

# ---------- 完成 ----------
printf '\n'
info "sontv $TAG 安装完成 → $INSTALL_DIR"
cat <<EOF

下一步：
  1. 生成稳定 token（明文自己留好，服务端只认它的 sha256）：
       TOKEN="\$(openssl rand -hex 24)"
       printf '%s' "\$TOKEN" | sha256sum
     把输出的 64 位 hex 按「标签,<hex>[,TTL小时]」一行一条写进 tokens.txt：
       sudo sh -c 'printf "%s\n" "我的订阅,<上面那串hex>" > $INSTALL_DIR/tokens.txt'
       sudo chmod 600 $INSTALL_DIR/tokens.txt

  2. 试跑自检：
       sudo $INSTALL_DIR/$BINARY -check

  3. 前台启动看日志：
       sudo $INSTALL_DIR/$BINARY

     想让它常驻，装的时候加上 --service 即可（已装可直接启用）：
       sudo systemctl enable --now sontv

EOF