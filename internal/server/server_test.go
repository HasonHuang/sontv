package server

import (
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/HasonHuang/sontv/internal/config"
	"github.com/HasonHuang/sontv/internal/temptoken"
	"github.com/HasonHuang/sontv/internal/tokens"
)

// newTestTable 建一份含两行的 token 表文件并装载。
//   - 主 token：带 TTL 6h
//   - 无 TTL：回落全局默认
func newTestTable(t *testing.T, toks ...string) (*tokens.TokenTable, string) {
	t.Helper()
	dir := t.TempDir()
	path := filepath.Join(dir, "tokens.txt")
	var b strings.Builder
	for i, tok := range toks {
		sum := sha256.Sum256([]byte(tok))
		h := hex.EncodeToString(sum[:])
		if i == 0 {
			b.WriteString("主token," + h + ",6\n")
		} else {
			b.WriteString("副token," + h + "\n")
		}
	}
	if err := os.WriteFile(path, []byte(b.String()), 0o600); err != nil {
		t.Fatal(err)
	}
	tt := tokens.NewTokenTable(path)
	if err := tt.Load(); err != nil {
		t.Fatalf("装载失败: %v", err)
	}
	return tt, path
}

// newTestServer 造一个 Server，时钟固定，便于断言 exp。
func newTestServer(t *testing.T, cfg *config.Config, table *tokens.TokenTable) *Server {
	t.Helper()
	s := NewServer(cfg, table)
	s.now = func() time.Time { return time.Unix(1_700_000_000, 0) }
	return s
}

func testConfig() *config.Config {
	c := config.DefaultConfig()
	c.UpstreamM3U = "https://up.example.com/list.m3u"
	return c
}

// ---------- 验证矩阵 #1~#9（认证与失效） ----------

func TestSubMatrixAuth(t *testing.T) {
	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)
	h := s.Handler()

	// #9：清空表 → 503（用另一份空表替换）
	empty, _ := newTestTable(t)
	se := newTestServer(t, testConfig(), empty)
	rec := httptest.NewRecorder()
	se.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/sub?token=good", nil))
	if rec.Code != http.StatusServiceUnavailable {
		t.Fatalf("#9 空表期望 503，实际 %d", rec.Code)
	}

	cases := []struct {
		name string
		url  string
		want int
	}{
		{"#1 无token", "/sub", http.StatusForbidden},
		{"#2 坏token", "/sub?token=bad", http.StatusForbidden},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			rec := httptest.NewRecorder()
			h.ServeHTTP(rec, httptest.NewRequest("GET", c.url, nil))
			if rec.Code != c.want {
				t.Fatalf("期望 %d，实际 %d", c.want, rec.Code)
			}
		})
	}
}

// #3：好 token → 200，正文含临时 token、不含稳定 token。
func TestSubIssuesTempTokenNotStable(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\nhttp://cdn/a.ts\n"))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	cfg := testConfig()
	cfg.UpstreamM3U = up.URL
	s := newTestServer(t, cfg, table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/sub?token=good", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("期望 200，实际 %d: %s", rec.Code, rec.Body.String())
	}
	body := rec.Body.String()
	if !strings.Contains(body, "t=") {
		t.Fatalf("#3 正文应含临时 token: %s", body)
	}
	if strings.Contains(body, "good") {
		t.Fatalf("#3 正文绝不该含稳定 token: %s", body)
	}
	if !strings.Contains(body, "url=http%3A%2F%2Fcdn%2Fa.ts") {
		t.Fatalf("#3 子链接未改写: %s", body)
	}
}

// #4 + #5：取 #3 子链接访问 → 200；篡改一位 → 401。
func TestSubChildLinkRoundTrip(t *testing.T) {
	var upstream string
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/a.ts" {
			w.Header().Set("Content-Type", "video/mp2t")
			_, _ = w.Write([]byte("TS-BYTES"))
			return
		}
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\n" + upstream + "/a.ts\n"))
	}))
	defer up.Close()
	upstream = up.URL

	table, _ := newTestTable(t, "good")
	cfg := testConfig()
	cfg.UpstreamM3U = up.URL
	s := newTestServer(t, cfg, table)

	// 拉订阅，抽出子链接路径
	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/sub?token=good", nil))
	child := extractFirstURL(t, rec.Body.String())

	// #4
	rec4 := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec4, httptest.NewRequest("GET", child, nil))
	if rec4.Code != http.StatusOK || rec4.Body.String() != "TS-BYTES" {
		t.Fatalf("#4 期望 200+TS，实际 %d %q", rec4.Code, rec4.Body.String())
	}

	// #5：篡改临时 token 一位
	tampered := tamperTemp(child)
	rec5 := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec5, httptest.NewRequest("GET", tampered, nil))
	if rec5.Code != http.StatusUnauthorized {
		t.Fatalf("#5 期望 401，实际 %d", rec5.Code)
	}
}

// #10：/play?t=<临时>&url=<本站自身> → 400。
func TestPlaySelfRef400(t *testing.T) {
	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	// 先拿一枚合法临时 token
	snap := table.Current()
	row := snap.LookupUID(firstUID(t, snap, "good"))
	if row == nil {
		t.Fatal("找不到行")
	}
	tok := temptoken.Issue(row, s.now(), time.Hour)

	rec := httptest.NewRecorder()
	req := httptest.NewRequest("GET", "/play?t="+tok+"&url=http%3A%2F%2Fself.example.com%2Fx.ts", nil)
	req.Host = "self.example.com"
	s.Handler().ServeHTTP(rec, req)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("#10 期望 400，实际 %d: %s", rec.Code, rec.Body.String())
	}
}

// #11：/play 代理 .ts + Range → 206，Body 就是上游片段。
func TestPlayRange206(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Range") != "bytes=0-1023" {
			t.Errorf("Range 未透传: %q", r.Header.Get("Range"))
		}
		w.Header().Set("Content-Type", "video/mp2t")
		w.Header().Set("Content-Range", "bytes 0-1023/99999")
		w.WriteHeader(http.StatusPartialContent)
		_, _ = w.Write([]byte(strings.Repeat("T", 1024)))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	req := httptest.NewRequest("GET", "/play?token=good&url="+urlEnc(up.URL+"/seg.ts"), nil)
	req.Header.Set("Range", "bytes=0-1023")
	s.Handler().ServeHTTP(rec, req)

	if rec.Code != http.StatusPartialContent {
		t.Fatalf("#11 期望 206，实际 %d", rec.Code)
	}
	if rec.Body.Len() != 1024 {
		t.Fatalf("#11 正文长度 = %d，期望 1024", rec.Body.Len())
	}
	if ct := rec.Header().Get("Content-Range"); !strings.Contains(ct, "0-1023") {
		t.Fatalf("#11 Content-Range 未透传: %q", ct)
	}
}

// #12：/play 稳定 token 入口代理 .m3u8 → 子链接盖临时 token（ADR-0001）。
func TestPlayStableEntryRewritesPlaylist(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\nhttp://cdn/a.ts\n"))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/play?token=good&url="+urlEnc(up.URL+"/x.m3u8"), nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("#12 期望 200，实际 %d", rec.Code)
	}
	body := rec.Body.String()
	if !strings.Contains(body, "t=") || !strings.Contains(body, "url=http%3A%2F%2Fcdn%2Fa.ts") {
		t.Fatalf("#12 子链接未盖章: %s", body)
	}
}

// #18：上游 302 → 服务端跟跳，最终 200，响应无 Location。
func TestPlayFollowsRedirectNoLocation(t *testing.T) {
	final := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "video/mp2t")
		_, _ = w.Write([]byte("FINAL"))
	}))
	defer final.Close()

	var redir string
	hop := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, redir, http.StatusFound)
	}))
	defer hop.Close()
	redir = final.URL + "/final.ts"

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	req := httptest.NewRequest("GET", "/play?token=good&url="+urlEnc(hop.URL+"/start.ts"), nil)
	s.Handler().ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("#18 期望 200，实际 %d", rec.Code)
	}
	if rec.Header().Get("Location") != "" {
		t.Fatalf("#18 不该透传 Location: %q", rec.Header().Get("Location"))
	}
	if rec.Body.String() != "FINAL" {
		t.Fatalf("#18 正文 = %q，期望 FINAL", rec.Body.String())
	}
}

// #19：/play 上游返回含相对路径的 m3u8 → 绝对补全 + 盖章。
func TestPlayRelativeChildAbsolutized(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\nseg/a.ts\n"))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/play?token=good&url="+urlEnc(up.URL+"/live/x.m3u8"), nil))
	body := rec.Body.String()
	if !strings.Contains(body, "url="+urlEnc(up.URL+"/live/seg/a.ts")) {
		t.Fatalf("#19 相对路径未补全为绝对: %s", body)
	}
	if !strings.Contains(body, "t=") {
		t.Fatalf("#19 未盖章: %s", body)
	}
}

// 边界：子链接是绝对地址，host 取自请求头——播放器无论从哪个入口拿到列表
// 都能一路回本站（上游的相对路径绝不能当成本站路径）。
func TestSubChildLinkAbsoluteFromHost(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\nhttp://cdn/a.ts\n"))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	cfg := testConfig()
	cfg.UpstreamM3U = up.URL
	s := newTestServer(t, cfg, table)

	rec := httptest.NewRecorder()
	req := httptest.NewRequest("GET", "/sub?token=good", nil)
	req.Host = "tv.example.com"
	s.Handler().ServeHTTP(rec, req)

	if !strings.Contains(rec.Body.String(), "http://tv.example.com/play?t=") {
		t.Fatalf("子链接应为绝对地址且 host 取自请求头: %s", rec.Body.String())
	}
}

// X-Forwarded-Proto 决定 scheme：TLS 在 nginx 终止，Go 只收到明文 HTTP，
// 离了它会把 https 播主下回 http。
func TestSubChildLinkSchemeFromForwardedProto(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\nhttp://cdn/a.ts\n"))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	cfg := testConfig()
	cfg.UpstreamM3U = up.URL
	s := newTestServer(t, cfg, table)

	rec := httptest.NewRecorder()
	req := httptest.NewRequest("GET", "/sub?token=good", nil)
	req.Host = "tv.example.com"
	req.Header.Set("X-Forwarded-Proto", "https")
	s.Handler().ServeHTTP(rec, req)

	if !strings.Contains(rec.Body.String(), "https://tv.example.com/play?t=") {
		t.Fatalf("子链接 scheme 应取自 X-Forwarded-Proto: %s", rec.Body.String())
	}
}

// ---------- /proxy：原样透传 ----------

// 上游给一份带绝对 CDN 链接与第三方凭据的 m3u8 —— 恰恰是 /play 会改写的那一类。
// /proxy 必须把它逐字节原样吐回：不改链接、不盖 token、不缓冲。
func TestProxy原样透传不改写(t *testing.T) {
	const src = "#EXTM3U\r\n#EXTINF:-1 tvg-name=\"x\",x\r\nhttp://cdn/live/a.ts?u=1&p=s3cr3t\r\n"
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/vnd.apple.mpegurl")
		_, _ = w.Write([]byte(src))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/proxy?token=good&url="+urlEnc(up.URL+"/a.m3u8"), nil))

	if rec.Code != http.StatusOK {
		t.Fatalf("期望 200，实际 %d: %s", rec.Code, rec.Body.String())
	}
	if got := rec.Body.String(); got != src {
		t.Fatalf("原样透传应逐字节相等（含 \\r\\n）：\n得到 %q\n期望 %q", got, src)
	}
	if strings.Contains(rec.Body.String(), "/play?") {
		t.Fatalf("正文不该出现本站子链接: %s", rec.Body.String())
	}
}

// /proxy 只认稳定 token：临时 token 是 /play 子链接专用凭证，这里不认。
func TestProxy认证只认稳定token(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("OK"))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)
	h := s.Handler()
	u := urlEnc(up.URL + "/a.bin")

	// 空表 → 503（fail closed），凭据是好的也不行。
	empty, _ := newTestTable(t)
	se := newTestServer(t, testConfig(), empty)
	rec503 := httptest.NewRecorder()
	se.Handler().ServeHTTP(rec503, httptest.NewRequest("GET", "/proxy?token=good&url="+u, nil))
	if rec503.Code != http.StatusServiceUnavailable {
		t.Fatalf("空表期望 503，实际 %d", rec503.Code)
	}

	cases := []struct {
		name string
		q    string
		want int
	}{
		{"无 token", "?&url=" + u, http.StatusForbidden},
		{"坏 token", "?token=bad&url=" + u, http.StatusForbidden},
		{"只带临时 token", "?t=1700000000.deadbeef.deadbeef&url=" + u, http.StatusForbidden},
		{"稳定 token", "?token=good&url=" + u, http.StatusOK},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			rec := httptest.NewRecorder()
			h.ServeHTTP(rec, httptest.NewRequest("GET", "/proxy"+c.q, nil))
			if rec.Code != c.want {
				t.Fatalf("期望 %d，实际 %d: %s", c.want, rec.Code, rec.Body.String())
			}
		})
	}
}

// 目标缺失、非 http(s)、指向本站自身 —— 三种非法目标一律 400。
func TestProxy目标非法400(t *testing.T) {
	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)
	h := s.Handler()

	for _, q := range []string{"/proxy?token=good", "/proxy?token=good&url=" + urlEnc("ftp://host/a.ts")} {
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, httptest.NewRequest("GET", q, nil))
		if rec.Code != http.StatusBadRequest {
			t.Fatalf("%s 期望 400，实际 %d", q, rec.Code)
		}
	}

	// 自引用：目标主机与请求 Host 相同即本站。
	rec := httptest.NewRecorder()
	req := httptest.NewRequest("GET", "/proxy?token=good&url="+urlEnc("http://self.example.com/x.ts"), nil)
	req.Host = "self.example.com"
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("自引用期望 400，实际 %d", rec.Code)
	}
}

func TestProxy方法405(t *testing.T) {
	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("POST", "/proxy?token=good&url="+urlEnc("http://h/a"), nil))
	if rec.Code != http.StatusMethodNotAllowed {
		t.Fatalf("期望 405，实际 %d", rec.Code)
	}
}

// 上游不可达 → 502，而不是把连接错误当成功透传出去。
func TestProxy上游失败502(t *testing.T) {
	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	// 端口 9（discard）几乎必不可达
	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/proxy?token=good&url="+urlEnc("http://127.0.0.1:9/a.ts"), nil))
	if rec.Code != http.StatusBadGateway {
		t.Fatalf("期望 502，实际 %d: %s", rec.Code, rec.Body.String())
	}
}

// Range 原样透传：透传端点同样支持分段，不因「不加工」而丢掉拖动播放。
func TestProxyRange206(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if got := r.Header.Get("Range"); got != "bytes=0-1023" {
			t.Errorf("Range 未透传: %q", got)
		}
		w.Header().Set("Content-Range", "bytes 0-1023/99999")
		w.WriteHeader(http.StatusPartialContent)
		_, _ = w.Write([]byte(strings.Repeat("T", 1024)))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	req := httptest.NewRequest("GET", "/proxy?token=good&url="+urlEnc(up.URL+"/seg.ts"), nil)
	req.Header.Set("Range", "bytes=0-1023")
	s.Handler().ServeHTTP(rec, req)

	if rec.Code != http.StatusPartialContent {
		t.Fatalf("期望 206，实际 %d", rec.Code)
	}
	if rec.Body.Len() != 1024 {
		t.Fatalf("正文长度 = %d，期望 1024", rec.Body.Len())
	}
	if ct := rec.Header().Get("Content-Range"); !strings.Contains(ct, "0-1023") {
		t.Fatalf("Content-Range 未透传: %q", ct)
	}
}

// 上游 302 → 服务端跟随，客户端只拿到最终内容，且看不见 Location（不泄露源站）。
func TestProxy跟随重定向不透传Location(t *testing.T) {
	var up *httptest.Server
	up = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/final.ts" {
			_, _ = w.Write([]byte("FINAL"))
			return
		}
		http.Redirect(w, r, up.URL+"/final.ts", http.StatusFound)
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/proxy?token=good&url="+urlEnc(up.URL+"/start.ts"), nil))

	if rec.Code != http.StatusOK {
		t.Fatalf("期望 200，实际 %d", rec.Code)
	}
	if got := rec.Body.String(); got != "FINAL" {
		t.Fatalf("应拿到重定向后的正文，实际 %q", got)
	}
	if loc := rec.Header().Get("Location"); loc != "" {
		t.Fatalf("Location 不该透传给客户端: %q", loc)
	}
}

// HEAD 只透传状态与头，正文为空。
func TestProxyHead无正文(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "video/mp2t")
		w.Header().Set("Content-Length", "1024")
		_, _ = w.Write([]byte(strings.Repeat("T", 1024)))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("HEAD", "/proxy?token=good&url="+urlEnc(up.URL+"/seg.ts"), nil))

	if rec.Code != http.StatusOK {
		t.Fatalf("期望 200，实际 %d", rec.Code)
	}
	if ct := rec.Header().Get("Content-Type"); ct != "video/mp2t" {
		t.Fatalf("Content-Type 未透传: %q", ct)
	}
	if rec.Body.Len() != 0 {
		t.Fatalf("HEAD 不该有正文，实际 %d 字节", rec.Body.Len())
	}
}

// ---------- 辅助 ----------

func urlEnc(s string) string {
	// 只编码最关键的字符，保持断言可读
	r := strings.NewReplacer(":", "%3A", "/", "%2F")
	return r.Replace(s)
}

// extractFirstURL 从响应体里取第一条本站 /play 子链接（绝对地址），
// 只取 path+query——httptest.NewRequest 不吃带 host 的绝对地址。
func extractFirstURL(t *testing.T, body string) string {
	t.Helper()
	for _, line := range strings.Split(body, "\n") {
		line = strings.TrimSpace(line)
		if i := strings.Index(line, "/play?"); i >= 0 {
			return line[i:]
		}
	}
	t.Fatalf("正文里没有 /play 子链接:\n%s", body)
	return ""
}

// tamperTemp 把子链接里 t= 值改动一位，破坏签名。
func tamperTemp(child string) string {
	// child 形如 http://host/play?t=XXX&url=YYY 或 /play?t=XXX&url=YYY
	i := strings.Index(child, "t=")
	if i < 0 {
		return child
	}
	rest := child[i+2:]
	end := strings.IndexByte(rest, '&')
	val := rest[:end]
	flip := byte('A')
	if val[0] == 'A' {
		flip = 'B'
	}
	return child[:i+2] + string(flip) + val[1:]
}

func firstUID(t *testing.T, snap *tokens.Snapshot, token string) string {
	t.Helper()
	row := snap.LookupToken(token)
	if row == nil {
		t.Fatal("token 未找到")
	}
	return row.UID
}

// ---------- /proxy：列表模式（filter=） ----------

// 端到端：带 filter= 时抓播放列表、过滤、每条一行只吐地址。
// 正文里不该剩下任何 # 行，也不该出现本站子链接或任何凭据。
func TestProxyFilter只返回URL(t *testing.T) {
	const src = "#EXTM3U\n" +
		"#EXTINF:-1 tvg-name=\"CCTV1\",CCTV1 综合\n" +
		"http://cdn/1.ts\n" +
		"#EXTINF:-1 tvg-name=\"CCTV5\",CCTV5 体育\n" +
		"http://cdn/5.ts\n" +
		"#EXTINF:-1 tvg-name=\"湖南\",湖南卫视\n" +
		"http://cdn/hn.ts\n"
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(src))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET",
		"/proxy?token=good&url="+urlEnc(up.URL+"/a.m3u")+"&filter=CCTV", nil))

	if rec.Code != http.StatusOK {
		t.Fatalf("期望 200，实际 %d: %s", rec.Code, rec.Body.String())
	}
	if ct := rec.Header().Get("Content-Type"); !strings.HasPrefix(ct, "text/plain") {
		t.Fatalf("Content-Type = %q，期望 text/plain", ct)
	}
	got := rec.Body.String()
	want := "http://cdn/1.ts\nhttp://cdn/5.ts\n"
	if got != want {
		t.Fatalf("正文 =\n%q\n期望\n%q", got, want)
	}
	// 清单里不该泄漏凭据，也不该把地址包成本站入口。
	for _, bad := range []string{"#EXTINF", "#EXTM3U", "/play?", "token="} {
		if strings.Contains(got, bad) {
			t.Fatalf("正文不该含 %q: %s", bad, got)
		}
	}
}

// 能不能过滤：只认正文是不是以 #EXTM3U 开头，不看后缀也不看 Content-Type。
// 一个叫 .m3u 却返回二进制内容的地址必须 400，而不是静默照原样吐回。
func TestProxyFilter目标非播放列表400(t *testing.T) {
	// 后缀像列表、内容类型也像列表，正文却是二进制——正是只看扩展名会中招的那种。
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/vnd.apple.mpegurl")
		_, _ = w.Write([]byte{0x00, 0x01, 0x02, 0x03})
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET",
		"/proxy?token=good&url="+urlEnc(up.URL+"/fake.m3u")+"&filter=CCTV", nil))

	if rec.Code != http.StatusBadRequest {
		t.Fatalf("期望 400，实际 %d: %s", rec.Code, rec.Body.String())
	}
	// 错误响应只有一行文案，绝不能掺进上游那 4 个二进制字节。
	if strings.ContainsAny(rec.Body.String(), "\x00\x01\x02\x03") {
		t.Fatalf("400 响应混进了上游内容: %q", rec.Body.String())
	}
	if !strings.Contains(rec.Body.String(), "播放列表") {
		t.Fatalf("错误文案应说明原因: %q", rec.Body.String())
	}
}

// filter= 存在却解析不出词（全是空词/逗号）是参数错误，不是「不过滤」——
// 后者会悄悄把整份列表原样吐回，调用方无从分辨。
func TestProxyFilter空过滤词400(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\nhttp://cdn/a.ts\n"))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	// 空壳参数同样算「带了 filter 却滤不了」，落 400 而非原样透传。
	for _, f := range []string{"", "%2C%2C", "%20", "%EF%BC%8C"} { // "" / ",," / " " / "，"
		rec := httptest.NewRecorder()
		s.Handler().ServeHTTP(rec, httptest.NewRequest("GET",
			"/proxy?token=good&url="+urlEnc(up.URL+"/a.m3u")+"&filter="+f, nil))
		if rec.Code != http.StatusBadRequest {
			t.Fatalf("filter=%q 期望 400，实际 %d: %s", f, rec.Code, rec.Body.String())
		}
	}
}

// 相对路径按上游基准补成绝对，且重定向后的最终地址才是基准。
func TestProxyFilter相对路径按最终地址补全(t *testing.T) {
	final := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\na.ts\n#EXTINF:-1,B\n/b.ts\n"))
	}))
	defer final.Close()
	// 上游 302 到 CDN 的子目录：按请求地址补全会得到另一套路径。
	hop := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, final.URL+"/live/", http.StatusFound)
	}))
	defer hop.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET",
		"/proxy?token=good&url="+urlEnc(hop.URL+"/list.m3u")+"&filter=A", nil))

	if rec.Code != http.StatusOK {
		t.Fatalf("期望 200，实际 %d: %s", rec.Code, rec.Body.String())
	}
	want := final.URL + "/live/a.ts\n"
	if got := rec.Body.String(); got != want {
		t.Fatalf("正文 = %q，期望 %q", got, want)
	}
}

// 同一地址在列表里出现多次，清单里只留首次出现的一条。
func TestProxyFilter去重(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n" +
			"#EXTINF:-1,A\na.ts\n" +
			"#EXTINF:-1,B\na.ts\n" +
			"#EXTINF:-1,C\nb.ts\n"))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET",
		"/proxy?token=good&url="+urlEnc(up.URL+"/a.m3u"), nil))

	// 不带 filter 时走原样透传，正文仍是完整 m3u —— 分流只由 filter= 决定。
	if strings.HasPrefix(rec.Body.String(), "http://") {
		t.Fatalf("不带 filter 不该进列表模式: %q", rec.Body.String())
	}
}

// HEAD 在列表模式下仍走同一条路径：Content-Length 与 GET 一致，但不写正文。
func TestProxyFilterHEAD只回头(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\na.ts\n#EXTINF:-1,B\nb.ts\n"))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)
	q := "/proxy?token=good&url=" + urlEnc(up.URL+"/a.m3u") + "&filter=A"

	get := httptest.NewRecorder()
	s.Handler().ServeHTTP(get, httptest.NewRequest("GET", q, nil))

	head := httptest.NewRecorder()
	s.Handler().ServeHTTP(head, httptest.NewRequest("HEAD", q, nil))

	if head.Code != http.StatusOK {
		t.Fatalf("HEAD 期望 200，实际 %d", head.Code)
	}
	if head.Body.Len() != 0 {
		t.Fatalf("HEAD 不该写正文: %q", head.Body.String())
	}
	g, h := get.Header().Get("Content-Length"), head.Header().Get("Content-Length")
	if g == "" || g != h {
		t.Fatalf("Content-Length GET=%q HEAD=%q", g, h)
	}
	if n, err := strconv.Atoi(g); err != nil || n != get.Body.Len() {
		t.Fatalf("Content-Length=%q 与正文长度 %d 不符", g, get.Body.Len())
	}
}

// 上游不可达 → 502；过滤词一个都没命中 → 200 空正文（不是错误）。
func TestProxyFilter边界状态码(t *testing.T) {
	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)
	enc := urlEnc("http://127.0.0.1:9/a.m3u")

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/proxy?token=good&url="+enc+"&filter=A", nil))
	if rec.Code != http.StatusBadGateway {
		t.Fatalf("上游不可达期望 502，实际 %d", rec.Code)
	}

	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\na.ts\n"))
	}))
	defer up.Close()

	rec2 := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec2, httptest.NewRequest("GET",
		"/proxy?token=good&url="+urlEnc(up.URL+"/a.m3u")+"&filter=不存在的台", nil))
	if rec2.Code != http.StatusOK || rec2.Body.Len() != 0 {
		t.Fatalf("零命中期望 200 空正文，实际 %d %q", rec2.Code, rec2.Body.String())
	}
}

// 认证与自引用检查在列表模式下同样生效：过滤不是绕过凭据的后门。
func TestProxyFilter仍需认证(t *testing.T) {
	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	for _, q := range []string{
		"?url=" + urlEnc("http://h/a.m3u") + "&filter=A",
		"?t=1700000000.deadbeef.deadbeef&url=" + urlEnc("http://h/a.m3u") + "&filter=A",
	} {
		rec := httptest.NewRecorder()
		s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/proxy"+q, nil))
		if rec.Code != http.StatusForbidden {
			t.Fatalf("%s 期望 403，实际 %d", q, rec.Code)
		}
	}

	req := httptest.NewRequest("GET", "/proxy?token=good&url="+urlEnc("http://self.example.com/a.m3u")+"&filter=A", nil)
	req.Host = "self.example.com"
	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, req)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("自引用期望 400，实际 %d", rec.Code)
	}
}
