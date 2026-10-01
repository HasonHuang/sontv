#!/usr/bin/env bash
# 从 Conventional Commits 生成中文更新日志。
#
# 用法: gen-changelog.sh <当前tag>      # markdown 打到 stdout
#
# 环境变量:
#   GITHUB_REPOSITORY   owner/repo，用于拼 compare 链接；不设则不输出该链接
#   SHOW_MISC           =1 时显示杂项组（chore/style/无前缀提交），默认隐藏
#
# 只依赖 git + bash，不引第三方工具——与项目「纯标准库、无第三方依赖」的取向一致。
# 刻意兼容 bash 3.2（macOS 自带），方便本地试跑。

set -euo pipefail

TAG="${1:?用法: gen-changelog.sh <当前tag>}"

REPO_URL=""
if [ -n "${GITHUB_REPOSITORY:-}" ]; then
  REPO_URL="https://github.com/${GITHUB_REPOSITORY}"
fi

# 上一个 tag。取不到（首个版本）就退回全量历史。
PREV_TAG="$(git describe --tags --abbrev=0 "${TAG}^" 2>/dev/null || true)"
if [ -n "$PREV_TAG" ]; then
  RANGE="${PREV_TAG}..${TAG}"
else
  RANGE="$TAG"
fi

# 分组渲染顺序；杂项默认不在其中。
# 变量名别叫 GROUPS——那是 bash 内建的「当前用户所属组」数组，赋值会被静默忽略。
GROUP_ORDER="breaking feat fix perf refactor docs test build revert"
if [ "${SHOW_MISC:-0}" = "1" ]; then
  GROUP_ORDER="${GROUP_ORDER} misc"
fi

group_title() {
  case "$1" in
    breaking) echo "⚠️ 破坏性变更" ;;
    feat)     echo "✨ 特性" ;;
    fix)      echo "🐛 修复" ;;
    perf)     echo "⚡ 性能" ;;
    refactor) echo "♻️ 重构" ;;
    docs)     echo "📚 文档" ;;
    test)     echo "🧪 测试" ;;
    build)    echo "📦 构建" ;;
    revert)   echo "⏪ 回滚" ;;
    misc)     echo "🧹 杂项" ;;
    *)        echo "📋 其它" ;;
  esac
}

# type → 分组。命中不了的（含无前缀的 Initial commit）一律进 misc。
group_of() {
  case "$1" in
    feat)     echo "feat" ;;
    fix)      echo "fix" ;;
    perf)     echo "perf" ;;
    refactor) echo "refactor" ;;
    docs)     echo "docs" ;;
    test)     echo "test" ;;
    build|ci) echo "build" ;;
    revert)   echo "revert" ;;
    *)        echo "misc" ;;
  esac
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

add_item() { # add_item <组> <整行>
  printf '%s\n' "$2" >>"$WORK/$1"
}

# 记录分隔符 \x1e、字段分隔符 \x1f：body 里有换行也不会串行。
SEP_REC=$'\x1e'
SEP_FLD=$'\x1f'
# 放进变量再匹配，省得在 [[ =~ ]] 里转义空格。
RE_CONV='^([a-zA-Z]+)(\(([^)]*)\))?(!)?:[[:space:]](.*)$'

while IFS= read -r -d "$SEP_REC" rec; do
  rec="${rec%$'\n'}"                       # 去掉 %b 自带的尾换行
  rec="${rec#$'\n'}"                       # git 在记录分隔符之后还补了个换行
  if [ -z "${rec//[[:space:]]/}" ]; then
    continue
  fi

  hash="${rec%%"$SEP_FLD"*}"
  rest="${rec#*"$SEP_FLD"}"
  subject="${rest%%"$SEP_FLD"*}"
  body="${rest#*"$SEP_FLD"}"

  if [ -z "$subject" ]; then
    continue
  fi

  if [[ "$subject" =~ $RE_CONV ]]; then
    type="$(printf '%s' "${BASH_REMATCH[1]}" | tr '[:upper:]' '[:lower:]')"
    scope="${BASH_REMATCH[3]}"
    bang="${BASH_REMATCH[4]}"
    msg="${BASH_REMATCH[5]}"
  else
    type="" scope="" bang="" msg="$subject"
  fi

  # type! 或正文里写了 BREAKING CHANGE 都算破坏性。
  note=""
  if printf '%s' "$body" | grep -qE '^BREAKING[ -]CHANGE:'; then
    # 用 sed 自己的 q 收尾而不是 | head -1：后者会把 sed 打断在 SIGPIPE 上，
    # 在 pipefail + set -e 下直接掀翻整个脚本。
    note="$(printf '%s\n' "$body" \
      | sed -nE '/^BREAKING[ -]CHANGE:/{s/^BREAKING[ -]CHANGE:[[:space:]]*//;p;q;}')"
  fi
  if [ -n "$bang" ] || [ -n "$note" ]; then
    if [ -z "$note" ]; then
      note="$msg"
    fi
    add_item breaking "- ${note} (\`${hash}\`)"
    continue                                # 破坏性条目不再重复出现在原分组
  fi

  if [ -n "$scope" ]; then
    add_item "$(group_of "$type")" "- **${scope}**：${msg} (\`${hash}\`)"
  else
    add_item "$(group_of "$type")" "- ${msg} (\`${hash}\`)"
  fi
done < <(git log "$RANGE" --no-merges --pretty=format:"%h${SEP_FLD}%s${SEP_FLD}%b${SEP_REC}")

# ---- 渲染 ----
echo "## 更新内容"
echo

any=0
for g in $GROUP_ORDER; do
  if [ ! -s "$WORK/$g" ]; then
    continue
  fi
  any=1
  echo "### $(group_title "$g")"
  echo
  cat "$WORK/$g"
  echo
done

if [ "$any" -eq 0 ]; then
  echo "本版本无用户可见变更。"
  echo
fi

if [ -n "$REPO_URL" ]; then
  if [ -n "$PREV_TAG" ]; then
    echo "**完整提交**: ${REPO_URL}/compare/${PREV_TAG}...${TAG}"
  else
    echo "**完整提交**: ${REPO_URL}/commits/${TAG}"
  fi
fi
