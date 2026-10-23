#!/usr/bin/env bash
# 订阅聚合冒烟：远端源合并、过滤与位置。
set -euo pipefail

cd "$(dirname "$0")/.."
BIN=${BIN:-target/debug/iptv-proxy}
PORT=${PORT:-19923}
UP=${UP:-19924}
WORK=$(mktemp -d)
mkdir -p "$WORK/up"
CFG="$WORK/config.toml"

cat > "$WORK/up/extra.m3u8" <<M3U
#EXTM3U
#EXTINF:-1 tvg-id="keep" group-title="远端组",远端频道
http://placeholder.test/remote.m3u8
#EXTINF:-1 tvg-id="skip" group-title="远端组",跳过频道
http://placeholder.test/skip.m3u8
#EXTINF:-1 tvg-id="excluded" group-title="远端组",全局排除
http://placeholder.test/excluded.m3u8
M3U

python3 -m http.server "$UP" --directory "$WORK/up" >"$WORK/up.log" 2>&1 &
UP_PID=$!
trap 'kill $UP_PID $PID 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$WORK"' EXIT
for _ in $(seq 1 50); do
  curl -fsS "http://127.0.0.1:$UP/extra.m3u8" >/dev/null 2>&1 && break
  sleep 0.1
done

cat > "$CFG" <<TOML
[server]
bind = "127.0.0.1:$PORT"
log_level = "warn"

[cache]
disk_path = "$WORK/cache"

[[groups]]
id = "g1"
name = "本地组"
mode = "proxy"
[[groups.channels]]
id = "c1"
name = "本地频道"
url = "http://127.0.0.1:9/none.m3u8"

[playlist]
merge_ttl_secs = 60
merge_position = "top"
global_exclude = ["id:excluded"]

[[playlist.sources]]
id = "s1"
name = "远端源"
url = "http://127.0.0.1:$UP/extra.m3u8"
enabled = true
unlisted_groups = "keep"
exclude = ["id:skip"]
TOML

"$BIN" "$CFG" >"$WORK/server.log" 2>&1 &
PID=$!
B="http://127.0.0.1:$PORT"
for _ in $(seq 1 60); do
  curl -fsS "$B/health" >/dev/null 2>&1 && break
  sleep 0.1
done

fail=0
check() { if [ "$2" = "$3" ]; then printf 'ok   %s\n' "$1"; else printf 'FAIL %s（期望 %s，实际 %s）\n' "$1" "$2" "$3"; fail=1; fi; }
body=$(curl -s "$B/playlist.m3u")

check "含远端频道" "1" "$(printf '%s' "$body" | grep -c '远端频道' || true)"
check "含本地频道" "1" "$(printf '%s' "$body" | grep -c '本地频道' || true)"
check "排除项不出现" "0" "$(printf '%s' "$body" | grep -c '跳过频道' || true)"
check "全局排除不出现" "0" "$(printf '%s' "$body" | grep -c '全局排除' || true)"
check "聚合在顶部" "1" "$(python3 - "$B/playlist.m3u" <<'PY'
import sys, urllib.request
body = urllib.request.urlopen(sys.argv[1]).read().decode()
print(1 if body.find('远端频道') < body.find('本地频道') else 0)
PY
)"

if [ "$fail" = "0" ]; then echo "订阅聚合冒烟测试全部通过"; else echo "订阅聚合冒烟测试有失败项"; exit 1; fi
