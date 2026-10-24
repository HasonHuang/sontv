#!/usr/bin/env bash
# Remux 冒烟：连续 TS → HLS；预热 503、清单、伪装切片。
set -euo pipefail

cd "$(dirname "$0")/.."
BIN=${BIN:-target/debug/iptv-proxy}
PORT=${PORT:-19932}
UPORT=${UPORT:-19933}
WORK=$(mktemp -d)

if ! command -v ffmpeg >/dev/null 2>&1; then
  echo "跳过：未安装 ffmpeg（仅用于生成测试流）"
  exit 0
fi

ffmpeg -loglevel error -f lavfi -i testsrc=size=320x240:rate=25 -f lavfi -i sine=frequency=440 \
  -t 20 -c:v libx264 -preset ultrafast -g 125 -c:a aac -f mpegts "$WORK/live.ts"
python3 -m http.server "$UPORT" --bind 127.0.0.1 --directory "$WORK" >"$WORK/upstream.log" 2>&1 &
UPID=$!

cat > "$WORK/config.toml" <<TOML
[server]
bind = "127.0.0.1:$PORT"
log_level = "warn"

[cache]
disk_path = "$WORK/cache"

[[groups]]
id = "rx"
name = "Remux 组"
mode = "remux"
access = "open"
[[groups.channels]]
id = "c1"
name = "连续流"
url = "http://127.0.0.1:$UPORT/live.ts"
TOML

"$BIN" "$WORK/config.toml" >"$WORK/server.log" 2>&1 &
PID=$!
trap 'kill $PID $UPID 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$WORK"' EXIT
B="http://127.0.0.1:$PORT"

for _ in $(seq 1 60); do
  curl -fsS "$B/health" >/dev/null 2>&1 && break
  sleep 0.1
done

fail=0
check() { if [ "$2" = "$3" ]; then printf 'ok   %s\n' "$1"; else printf 'FAIL %s（期望 %s，实际 %s）\n' "$1" "$2" "$3"; fail=1; fi; }

check "预热 503" "503" "$(curl -s -o /dev/null -w '%{http_code}' "$B/rx/c1.m3u8")"

CODE=503
for _ in $(seq 1 60); do
  sleep 0.5
  CODE=$(curl -s -o "$WORK/list.m3u8" -w '%{http_code}' "$B/rx/c1.m3u8")
  [ "$CODE" = "200" ] && break
done
check "清单 200" "200" "$CODE"
check "目标时长 5" "1" "$(grep -c '#EXT-X-TARGETDURATION:5' "$WORK/list.m3u8" || true)"
check "含伪装切片" "yes" "$(grep -q '/rx/c1/seg_.*[.][a-z]*' "$WORK/list.m3u8" && echo yes || echo no)"

SEG=$(grep -o '/rx/c1/seg_[0-9]*\.[a-z]*' "$WORK/list.m3u8" | head -1)
check "切片 200" "200" "$(curl -s -o /dev/null -w '%{http_code}' "$B$SEG")"
check "切片类型为图片" "1" "$(curl -s -o /dev/null -D - "$B$SEG" | grep -ci '^content-type: image/')"
check "未知切片 404" "404" "$(curl -s -o /dev/null -w '%{http_code}' "$B/rx/c1/seg_999999999999.png")"

if [ "$fail" = "0" ]; then echo "Remux 冒烟测试全部通过"; else echo "Remux 冒烟测试有失败项"; exit 1; fi
