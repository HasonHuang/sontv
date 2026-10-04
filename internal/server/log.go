package server

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/url"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"time"
	"unicode"
	"unicode/utf8"
)

// 播放链路的日志（/sub 与 /play 共用）。
//
// 分级的意义在于「默认安静、按需全开」：播一路电视每分钟打几十行分片日志，
// 与启动/改配置/凭据失效这些真正稀有的事混在一起，出问题时反而找不到重点。
// 故门槛默认 INFO，逐请求、逐分片的行进细节一律 DEBUG——
// 排查时把 SONTV_LOG_LEVEL 设成 debug 再看日志。
//
// 各端点的分工：
//
//	DEBUG  逐请求、逐分片（开始 / 上游响应 / 直传完成 / 改写完成）
//	INFO   稀有的、值得留痕的状态变化（订阅改写结果、启动、重载、退出）
//	WARN   有人在做无效的事（凭据不对、目标非法、自引用、认证失败）
//	ERROR  链路真的断了（上游抓不到、读不了、超限）
//
// 两条硬规矩，改动时别绕过：
//
//  1. 绝不打印凭据明文。上游播放列表里到处是第三方凭据（?u=admin&p=<hash>），
//     连带 *url.Error 的 Error() 都嵌着完整 URL——目标地址一律走 safeURL 脱敏，
//     错误一律走 safeErr 剥壳，只留「哪个站、哪条路径、哪些参数名、哪种失败」。
//
//  2. 绝不打印响应正文。只留探测块开头若干个可打印字符（preview），
//     够认出「HTML 错误页 / 纯文本提示 / 真二进制流」三类，就够定位播不了的原因。

// level 是全站日志门槛。可用 SONTV_LOG_LEVEL 覆盖（见 ConfigureLogging）。
// 用 slog.LevelVar 而非普通变量：门槛要在 ConfigureLogging 里改，
// LevelVar 内部带原子，读侧无需加锁。
var level = new(slog.LevelVar)

// ConfigureLogging 设定日志门槛。三个入口按优先级取第一个非空者：
// flagVal（-log-level 参数）→ SONTV_LOG_LEVEL 环境变量 → cfgVal（config.json
// 的 log_level）→ info。
//
// 三个入口各有各的位置，不是冗余：
//
//	config.json 的 log_level——正式配置项，常驻部署改这里，不碰服务单元。
//	  它排在最后，是为了不挡住临时覆盖：改完配置想立刻看效果的人，
//	  不必先回去把配置改回 info。
//	-log-level 参数——临时提门槛最省事的一条路，优先级最高。
//	SONTV_LOG_LEVEL 环境变量——手工跑二进制时的第三种顺手写法。
//
// 取值认 slog 的标准写法：debug / info / warn / error（大小写不敏感），
// 也认 INFO+2 这类偏移写法。非法或未设置时回落到 info——
// 日志配置写错不该让服务起不来。
//
// 放在 server 包而非 cmd，是因为 reqLog 持有的也是同一个门槛；
// 两处各设一次必然对不上。
func ConfigureLogging(flagVal, cfgVal string) {
	level.Set(slog.LevelInfo) // 每次调用都从缺省重来，避免重复调用时残留
	// SetDefault 同时把标准 log 包也接到这条 handler 上：
	// 启动期那些还没来得及用 slog 的 log.Printf 也会带上级别与格式。
	slog.SetDefault(slog.New(newHandler(os.Stderr)))

	src, source := pickLogLevel(flagVal, cfgVal)
	if src == "" {
		return
	}
	if err := level.UnmarshalText([]byte(src)); err != nil {
		// 门槛回落到 info 已经在上面做过，这里只提示一下配置被忽略了。
		slog.Warn("日志级别取值非法，已回落 info", "来源", source, "取值", src)
	}
}

// LogLevelEnv 是日志级别的环境变量名，导出供命令行帮助文本引用，
// 免得两处各写一份字符串、改一处漏一处。
const LogLevelEnv = "SONTV_LOG_LEVEL"

// pickLogLevel 按优先级选出级别取值，并返回它来自哪个入口——
// 用户改错入口时，警告行里能一眼看出级别当初是从哪读到的。
func pickLogLevel(flagVal, cfgVal string) (src, source string) {
	if s := strings.TrimSpace(flagVal); s != "" {
		return s, "-log-level 参数"
	}
	if s := strings.TrimSpace(os.Getenv(LogLevelEnv)); s != "" {
		return s, LogLevelEnv + " 环境变量"
	}
	if s := strings.TrimSpace(cfgVal); s != "" {
		return s, "config.json 的 log_level"
	}
	return "", ""
}

// handler 是本项目自带的 slog.Handler，输出形如：
//
//	12:49:24.081 INFO  [#8 临时 repro] 直传完成 字节=15325008B 耗时=3764ms
//
// 不用 slog.TextHandler 是有原因的：它对含空格的 msg 一律加引号并转义，
// 「直传完成 字节=… 耗时=…」这种中文行会被裹成一坨 \"…\"，排查时还得
// 去掉转义才能读。这里自己拼：级别定宽对齐、时间压到毫秒、msg 原样输出，
// 属性（若有）以 key=value 追加在后面。
type handler struct {
	mu     *sync.Mutex
	w      io.Writer
	attrs  []slog.Attr // WithAttrs 累积的属性，按序输出
	groups []string    // WithGroup 累积的组名，用于给属性加前缀
}

func newHandler(w io.Writer) *handler {
	return &handler{mu: &sync.Mutex{}, w: w}
}

func (h *handler) Enabled(_ context.Context, l slog.Level) bool { return l >= level.Level() }

func (h *handler) Handle(_ context.Context, r slog.Record) error {
	var b strings.Builder
	ts := r.Time
	if ts.IsZero() {
		ts = time.Now()
	}
	b.WriteString(ts.Format("15:04:05.000"))
	b.WriteByte(' ')
	b.WriteString(levelLabel(r.Level))
	b.WriteByte(' ')
	b.WriteString(r.Message)

	for _, a := range h.attrs {
		h.writeAttr(&b, a)
	}
	r.Attrs(func(a slog.Attr) bool {
		h.writeAttr(&b, a)
		return true
	})
	b.WriteByte('\n')

	h.mu.Lock()
	defer h.mu.Unlock()
	_, err := io.WriteString(h.w, b.String())
	return err
}

// writeAttr 追加一个属性。组名逐层用 '.' 连接（a.b.c=value），
// 与 slog 自身的组语义一致，便于日后加「上游」「客户端」两组。
func (h *handler) writeAttr(b *strings.Builder, a slog.Attr) {
	key := a.Key
	if len(h.groups) > 0 {
		key = strings.Join(h.groups, ".") + "." + key
	}
	if a.Value.Kind() == slog.KindGroup {
		// 组本身不输出，只把子属性摊平到同一行。
		for _, sub := range a.Value.Group() {
			sub.Key = key + "." + sub.Key
			h.writeAttr(b, sub)
		}
		return
	}
	b.WriteByte(' ')
	b.WriteString(key)
	b.WriteByte('=')
	b.WriteString(a.Value.String())
}

// WithAttrs / WithGroup 返回新 handler 并拷贝，不共享可变切片：
// slog 的约定是并发安全下不可变，返回同一个 handler 会串味。
func (h *handler) WithAttrs(as []slog.Attr) slog.Handler {
	n := *h
	n.attrs = make([]slog.Attr, 0, len(h.attrs)+len(as))
	n.attrs = append(n.attrs, h.attrs...)
	n.attrs = append(n.attrs, as...)
	return &n
}

func (h *handler) WithGroup(name string) slog.Handler {
	n := *h
	n.groups = make([]string, 0, len(h.groups)+1)
	n.groups = append(n.groups, h.groups...)
	n.groups = append(n.groups, name)
	return &n
}

// levelLabel 给级别一个定宽标签，竖排对齐后扫一眼就能分流。
func levelLabel(l slog.Level) string {
	switch {
	case l < slog.LevelInfo:
		return "DEBUG"
	case l < slog.LevelWarn:
		return "INFO "
	case l < slog.LevelError:
		return "WARN "
	default:
		return "ERROR"
	}
}

// seq 给每次请求编号，把同一次播放的成对日志串成一条线。
// 播放时并发很高，没有编号的话上下游两行日志根本对不上。
var seq atomic.Uint64

// reqLog 是一次受代理请求的日志上下文。
type reqLog struct {
	id     uint64
	start  time.Time
	kind   string // 端点与凭据种类，如 "临时" / "订阅"
	label  string // token 表里的标签：分辨是哪个用户在播
	target string // 已脱敏的目标地址
}

func newReqLog(kind, label, targetRaw string) *reqLog {
	return &reqLog{
		id:     seq.Add(1),
		start:  time.Now(),
		kind:   kind,
		label:  label,
		target: safeURL(targetRaw),
	}
}

// logf 按级别打一行带请求编号的日志。级别低于门槛时直接返回，
// 连 fmt.Sprintf 都不做——DEBUG 是热路径，省下的分配是真的。
func (l *reqLog) logf(lv slog.Level, format string, args ...any) {
	if lv < level.Level() {
		return
	}
	slog.Log(nil, lv, l.prefix()+fmt.Sprintf(format, args...))
}

// prefix 拼出 "[#编号 凭据种类 标签] "，标签为空时不留空位。
func (l *reqLog) prefix() string {
	p := fmt.Sprintf("[#%d %s", l.id, l.kind)
	if l.label != "" {
		p += " " + l.label
	}
	return p + "] "
}

func (l *reqLog) debugf(format string, args ...any) { l.logf(slog.LevelDebug, format, args...) }
func (l *reqLog) infof(format string, args ...any)  { l.logf(slog.LevelInfo, format, args...) }
func (l *reqLog) warnf(format string, args ...any)  { l.logf(slog.LevelWarn, format, args...) }
func (l *reqLog) errorf(format string, args ...any) { l.logf(slog.LevelError, format, args...) }

// ms 返回距请求开始的毫秒数。
func (l *reqLog) ms() int64 { return time.Since(l.start).Milliseconds() }

// safeURL 脱敏一个地址：保留 scheme/host/path 与参数名，参数值一律抹成 ***。
//
// url= 是客户端可控的，可能根本不是合法 URL——解析失败就整体不显示，
// 绝不把原串打进日志。URL 的 path 段不含查询凭据（凭据一律在 query 或
// path 后段的 userinfo 里，这里都不取），所以 path 可以原样留。
func safeURL(raw string) string {
	if strings.TrimSpace(raw) == "" {
		return "(空)"
	}
	u, err := url.Parse(raw)
	if err != nil {
		return "(不可解析的地址)"
	}
	s := u.Scheme + "://" + u.Host + u.Path
	if u.RawQuery == "" {
		return s
	}
	pairs := make([]string, 0, 8)
	for _, k := range queryKeys(u.RawQuery) {
		pairs = append(pairs, k+"=***")
	}
	return s + "?" + strings.Join(pairs, "&")
}

// queryKeys 取查询串里的参数名，按出现顺序去重。
func queryKeys(raw string) []string {
	var keys []string
	seen := map[string]bool{}
	for _, kv := range strings.Split(raw, "&") {
		k := kv
		if i := strings.IndexByte(kv, '='); i >= 0 {
			k = kv[:i]
		}
		if k == "" || seen[k] {
			continue
		}
		seen[k] = true
		keys = append(keys, k)
	}
	return keys
}

// safeErr 把 error 压成一行可安全落日志的文本。
// *url.Error 的 Error() 形如 Get "<完整URL含凭据>": dial tcp ...，只取内层原因。
func safeErr(err error) string {
	var ue *url.Error
	if errors.As(err, &ue) && ue.Err != nil {
		return ue.Err.Error()
	}
	return err.Error()
}

// preview 取探测块开头的可打印摘要（至多 n 字节）。用途是分辨上游回了什么
// 类型的内容——「HTML 错误页 / 文本提示 / 真二进制流」——不是给日志看正文。
//
// 逐 rune 判可打印性，而不是逐字节判 ASCII：上游的错误提示常是中文
// （「该节目已下架」），按 ASCII 判会退化成一串点，等于没记。非法 UTF-8
// 序列要显式挡掉：DecodeRune 会给出 (U+FFFD, 1)，而 unicode.IsPrint(U+FFFD)
// 为真，直接用 range/IsPrint 会把二进制原样印进日志。
func preview(b []byte, n int) string {
	if len(b) == 0 {
		return "(空)"
	}
	if len(b) > n {
		b = b[:n]
	}
	var sb strings.Builder
	sb.Grow(len(b))
	for len(b) > 0 {
		r, size := utf8.DecodeRune(b)
		invalid := r == utf8.RuneError && size == 1
		if !invalid && unicode.IsPrint(r) {
			sb.WriteRune(r)
		} else {
			sb.WriteByte('.')
		}
		b = b[size:]
	}
	return sb.String()
}
