package server

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/url"
	"strconv"
	"strings"
	"testing"
)

// safeURL 是日志里唯一会碰目标地址的出口，而目标地址常带第三方凭据
// （?u=admin&p=<hash>）。这组测试的核心断言是：凭据值一个字节都不许进日志。

func TestSafeURL不泄漏凭据值(t *testing.T) {
	tests := []struct {
		name string
		raw  string
		want string
	}{
		{
			name: "参数值全部抹掉，只留参数名",
			raw:  "https://cdn3.indevs.in/stream/tvb/fct4k/stream_0.php?u=admin&p=69fe3eb1f0b037bc2479cb4fbd56551848343f071f939422ef250f04f68b4cee&uid=",
			want: "https://cdn3.indevs.in/stream/tvb/fct4k/stream_0.php?u=***&p=***&uid=***",
		},
		{
			name: "无查询串时原样保留",
			raw:  "https://cdn.qd.je/163189/fhzw",
			want: "https://cdn.qd.je/163189/fhzw",
		},
		{
			name: "同名参数只出现一次",
			raw:  "http://h/p?a=1&b=2&a=3",
			want: "http://h/p?a=***&b=***",
		},
		{
			name: "无值的参数也要显形（裸键）",
			raw:  "http://h/p?uid=",
			want: "http://h/p?uid=***",
		},
		{
			name: "空串",
			raw:  "",
			want: "(空)",
		},
		{
			name: "解析失败时整体不显示——绝不回显原串",
			raw:  "ht tp://坏地址?p=SECRET",
			want: "(不可解析的地址)",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := safeURL(tt.raw); got != tt.want {
				t.Fatalf("safeURL(%q)\n got = %s\nwant = %s", tt.raw, got, tt.want)
			}
		})
	}
}

func TestSafeURL不含任何原值片段(t *testing.T) {
	const secret = "69fe3eb1f0b037bc2479cb4fbd56551848343f071f939422ef250f04f68b4cee"
	got := safeURL("https://h/p?u=admin&p=" + secret + "&uid=")
	for _, frag := range []string{secret, "admin"} {
		if strings.Contains(got, frag) {
			t.Fatalf("日志里出现了凭据值 %q: %s", frag, got)
		}
	}
	if !strings.Contains(got, "p=***") {
		t.Fatalf("参数名应保留以便排障: %s", got)
	}
}

// safeErr 的存在理由和 safeURL 一样：*url.Error 会把完整 URL 拼进 Error()。
func TestSafeErr剥掉URL只留原因(t *testing.T) {
	const secretURL = "https://h/p?p=SECRETVALUE"
	err := &url.Error{Op: "Get", URL: secretURL, Err: errors.New("dial tcp: 超时")}

	got := safeErr(err)
	if strings.Contains(got, "SECRETVALUE") {
		t.Fatalf("错误文本里带出了 URL: %s", got)
	}
	if got != "dial tcp: 超时" {
		t.Fatalf("应只保留内层原因，得到 %q", got)
	}
}

func TestSafeErr普通错误原样(t *testing.T) {
	if got := safeErr(errors.New("上游返回 404")); got != "上游返回 404" {
		t.Fatalf("got %q", got)
	}
}

// preview 只用于分辨「HTML 错误页 / 文本提示 / 二进制流」，不该把控制字符
// 或二进制直接写进日志——那会污染终端甚至伪造日志行。
func TestPreview只留可打印字符(t *testing.T) {
	tests := []struct {
		name string
		in   []byte
		n    int
		want string
	}{
		{"TS 分片首部 0x47", []byte{0x47, 0x40, 0x11, 0x1b}, 32, "G@.."},
		{"HTML 错误页", []byte("<html><body>Error"), 32, "<html><body>Error"},
		{"中文错误提示可读", []byte("该节目已下架"), 32, "该节目已下架"},
		{"非法 UTF-8（二进制）退化为点", []byte{0xff, 0xfe, 0x80}, 32, "..."},
		{"空块", []byte{}, 32, "(空)"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := preview(tt.in, tt.n); got != tt.want {
				t.Fatalf("preview() = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestPreview截断到n字节(t *testing.T) {
	if got := preview([]byte("ABCDEFGH"), 3); got != "ABC" {
		t.Fatalf("got %q", got)
	}
}

// ---- 级别门槛 ----
//
// 分级的全部意义就是「默认安静、按需全开」，所以这里直接盯输出：
// 门槛设在 INFO 时，逐分片的 DEBUG 行一条都不许漏出来。

// captureLogs 把门槛临时设成 lv，执行 fn，返回 handler 写出的文本。
// 恢复靠 defer，测试之间互不污染。
func captureLogs(t *testing.T, lv slog.Level, fn func()) string {
	t.Helper()
	var buf bytes.Buffer
	old := slog.Default()
	slog.SetDefault(slog.New(newHandler(&buf)))
	level.Set(lv)
	defer func() {
		level.Set(slog.LevelInfo)
		slog.SetDefault(old)
	}()
	fn()
	return buf.String()
}

func Test门槛挡掉DEBUG行(t *testing.T) {
	lg := newReqLog("临时", "标签", "https://h/p?a=1")

	out := captureLogs(t, slog.LevelInfo, func() {
		lg.debugf("直传完成 字节=%d", 1024)
		lg.infof("改写完成")
		lg.warnf("认证失败")
		lg.errorf("上游请求失败")
	})

	if strings.Contains(out, "直传完成") {
		t.Fatalf("INFO 门槛下漏出了 DEBUG 行:\n%s", out)
	}
	for _, want := range []string{"改写完成", "认证失败", "上游请求失败"} {
		if !strings.Contains(out, want) {
			t.Fatalf("缺少 %q:\n%s", want, out)
		}
	}
}

func TestDEBUG门槛全放行(t *testing.T) {
	lg := newReqLog("临时", "标签", "https://h/p?a=1")

	out := captureLogs(t, slog.LevelDebug, func() {
		lg.debugf("直传完成 字节=%d", 1024)
	})
	if !strings.Contains(out, "直传完成") || !strings.Contains(out, "1024") {
		t.Fatalf("DEBUG 门槛下应放出该行:\n%s", out)
	}
}

func Test级别门槛只挡更低不挡更高(t *testing.T) {
	lg := newReqLog("临时", "", "https://h/p")

	// 门槛设到 WARN：DEBUG/INFO 被挡，WARN/ERROR 照常。
	out := captureLogs(t, slog.LevelWarn, func() {
		lg.debugf("开始")
		lg.infof("改写完成")
		lg.warnf("认证失败")
		lg.errorf("上游请求失败")
	})
	if strings.Contains(out, "开始") || strings.Contains(out, "改写完成") {
		t.Fatalf("WARN 门槛下漏出了低级别行:\n%s", out)
	}
	if !strings.Contains(out, "认证失败") || !strings.Contains(out, "上游请求失败") {
		t.Fatalf("WARN 门槛应挡住低级别但放出高级别:\n%s", out)
	}
}

// 输出必须带级别标签与请求编号，否则分级与并发关联都无从谈起。
func Test输出含级别标签与请求编号(t *testing.T) {
	lg := newReqLog("临时", "我的订阅", "https://h/p")

	out := captureLogs(t, slog.LevelDebug, func() { lg.warnf("认证失败") })
	for _, want := range []string{"WARN", "认证失败", fmt.Sprintf("[#%d", lg.id), "我的订阅"} {
		if !strings.Contains(out, want) {
			t.Fatalf("输出缺少 %q:\n%s", want, out)
		}
	}
}

// 标签为空时（前认证阶段常见）不留空位，编号与凭据种类仍要在。
func Test空标签不留空位(t *testing.T) {
	lg := newReqLog("临时", "", "https://h/p")

	out := captureLogs(t, slog.LevelWarn, func() { lg.warnf("认证失败") })
	if strings.Contains(out, "临时 ]") {
		t.Fatalf("空标签产生了多余空格:\n%s", out)
	}
	if !strings.Contains(out, "[#"+strconv.FormatUint(lg.id, 10)+" 临时]") {
		t.Fatalf("编号与凭据种类应保留:\n%s", out)
	}
}

// handler 自身的行为：定宽级别、msg 不加引号、属性以 key=value 追加。
func TestHandler输出格式(t *testing.T) {
	var buf bytes.Buffer
	h := newHandler(&buf)
	// 门槛压到最低，四档都要真的写出来才能验格式。
	level.Set(slog.LevelDebug)
	defer level.Set(slog.LevelInfo)

	for _, l := range []slog.Level{slog.LevelDebug, slog.LevelInfo, slog.LevelWarn, slog.LevelError} {
		buf.Reset()
		slog.New(h).Log(nil, l, "中文消息 带空格", "键", "值")
		line := buf.String()
		if !strings.Contains(line, "中文消息 带空格") {
			t.Fatalf("msg 被引号包裹了，排障时反而难读:\n%s", line)
		}
		if !strings.Contains(line, "键=值") {
			t.Fatalf("属性未按 key=value 输出:\n%s", line)
		}
	}
}

func Test级别标签定宽(t *testing.T) {
	for _, tc := range []struct {
		lv   slog.Level
		want string
	}{
		{slog.LevelDebug, "DEBUG"},
		{slog.LevelInfo, "INFO "},
		{slog.LevelWarn, "WARN "},
		{slog.LevelError, "ERROR"},
	} {
		if got := levelLabel(tc.lv); got != tc.want {
			t.Fatalf("levelLabel(%v) = %q, want %q", tc.lv, got, tc.want)
		}
	}
}

// ConfigureLogging 认 slog 的标准写法，且非法取值必须回落到 info 而不是崩掉。
func TestConfigureLogging认标准级别名(t *testing.T) {
	for _, tc := range []struct {
		env  string
		want slog.Level
	}{
		{"debug", slog.LevelDebug},
		{"INFO", slog.LevelInfo},
		{"Warn", slog.LevelWarn},
		{"error", slog.LevelError},
		{"", slog.LevelInfo}, // 未设置 → 缺省
		{"瞎写", slog.LevelInfo},
	} {
		t.Setenv(LogLevelEnv, tc.env)
		ConfigureLogging("")
		if got := level.Level(); got != tc.want {
			t.Fatalf("%s=%q → %v, want %v", LogLevelEnv, tc.env, got, tc.want)
		}
	}
}

// 参数必须压过环境变量：systemd 部署下 -log-level 是唯一可靠入口，
// 若环境变量能盖过它，用户在 unit 里加参数会毫无效果。
func TestConfigureLogging参数压过环境变量(t *testing.T) {
	t.Setenv(LogLevelEnv, "error")
	ConfigureLogging("debug")
	if got := level.Level(); got != slog.LevelDebug {
		t.Fatalf("参数应压过环境变量，得到 %v", got)
	}
}

// 传空参数时才回落环境变量（裸机手工跑的路径）。
func TestConfigureLogging空参数回落环境变量(t *testing.T) {
	t.Setenv(LogLevelEnv, "warn")
	ConfigureLogging("")
	if got := level.Level(); got != slog.LevelWarn {
		t.Fatalf("应回落环境变量，得到 %v", got)
	}
}

// WithAttrs/WithGroup 必须返回新实例而不是共享可变切片，
// 否则并发下两个请求的属性会互相串味。
func TestWithAttrs不共享切片(t *testing.T) {
	h := newHandler(io.Discard)
	a := slog.String("k1", "v1")
	h1 := h.WithAttrs([]slog.Attr{a}).(*handler)
	h2 := h1.WithAttrs([]slog.Attr{slog.String("k2", "v2")}).(*handler)

	if len(h1.attrs) != 1 {
		t.Fatalf("父 handler 的属性被改动了: %d", len(h1.attrs))
	}
	if len(h2.attrs) != 2 {
		t.Fatalf("子 handler 属性数不对: %d", len(h2.attrs))
	}
}

func TestWithGroup属性前缀(t *testing.T) {
	level.Set(slog.LevelDebug)
	defer level.Set(slog.LevelInfo)
	var buf bytes.Buffer
	h := newHandler(&buf).WithGroup("上游").WithAttrs([]slog.Attr{slog.String("主机", "cdn.example.com")})
	slog.New(h).Warn("取列表", "耗时", "12ms")

	out := buf.String()
	if !strings.Contains(out, "上游.主机=cdn.example.com") {
		t.Fatalf("组名未作用于属性:\n%s", out)
	}
}
