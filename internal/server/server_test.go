package server

import (
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/HasonHuang/mytv/go/internal/config"
	"github.com/HasonHuang/mytv/go/internal/temp"
	"github.com/HasonHuang/mytv/go/internal/tokens"
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
	if !strings.Contains(body, "u=http%3A%2F%2Fcdn%2Fa.ts") {
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

// #10：/url?t=<临时>&u=<本站自身> → 400。
func TestURLSelfRef400(t *testing.T) {
	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	// 先拿一枚合法临时 token
	snap := table.Current()
	row := snap.LookupUID(firstUID(t, snap, "good"))
	if row == nil {
		t.Fatal("找不到行")
	}
	temp := temp.IssueTempToken(row, s.now(), time.Hour)

	rec := httptest.NewRecorder()
	req := httptest.NewRequest("GET", "/url?t="+temp+"&u=http%3A%2F%2Fself.example.com%2Fx.ts", nil)
	req.Host = "self.example.com"
	s.Handler().ServeHTTP(rec, req)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("#10 期望 400，实际 %d: %s", rec.Code, rec.Body.String())
	}
}

// #11：/url 代理 .ts + Range → 206，Body 就是上游片段。
func TestURLRange206(t *testing.T) {
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
	req := httptest.NewRequest("GET", "/url?token=good&u="+urlEnc(up.URL+"/seg.ts"), nil)
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

// #12：/url 稳定 token 入口代理 .m3u8 → 子链接盖临时 token（ADR-0001）。
func TestURLStableEntryRewritesPlaylist(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\nhttp://cdn/a.ts\n"))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/url?token=good&u="+urlEnc(up.URL+"/x.m3u8"), nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("#12 期望 200，实际 %d", rec.Code)
	}
	body := rec.Body.String()
	if !strings.Contains(body, "t=") || !strings.Contains(body, "u=http%3A%2F%2Fcdn%2Fa.ts") {
		t.Fatalf("#12 子链接未盖章: %s", body)
	}
}

// #18：上游 302 → 服务端跟跳，最终 200，响应无 Location。
func TestURLFollowsRedirectNoLocation(t *testing.T) {
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
	req := httptest.NewRequest("GET", "/url?token=good&u="+urlEnc(hop.URL+"/start.ts"), nil)
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

// #19：/url 上游返回含相对路径的 m3u8 → 绝对补全 + 盖章。
func TestURLRelativeChildAbsolutized(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\nseg/a.ts\n"))
	}))
	defer up.Close()

	table, _ := newTestTable(t, "good")
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/url?token=good&u="+urlEnc(up.URL+"/live/x.m3u8"), nil))
	body := rec.Body.String()
	if !strings.Contains(body, "u="+urlEnc(up.URL+"/live/seg/a.ts")) {
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

	if !strings.Contains(rec.Body.String(), "http://tv.example.com/url?t=") {
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

	if !strings.Contains(rec.Body.String(), "https://tv.example.com/url?t=") {
		t.Fatalf("子链接 scheme 应取自 X-Forwarded-Proto: %s", rec.Body.String())
	}
}

// ---------- 辅助 ----------

func urlEnc(s string) string {
	// 只编码最关键的字符，保持断言可读
	r := strings.NewReplacer(":", "%3A", "/", "%2F")
	return r.Replace(s)
}

// extractFirstURL 从响应体里取第一条本站 /url 子链接（绝对地址），
// 只取 path+query——httptest.NewRequest 不吃带 host 的绝对地址。
func extractFirstURL(t *testing.T, body string) string {
	t.Helper()
	for _, line := range strings.Split(body, "\n") {
		line = strings.TrimSpace(line)
		if i := strings.Index(line, "/url?"); i >= 0 {
			return line[i:]
		}
	}
	t.Fatalf("正文里没有 /url 子链接:\n%s", body)
	return ""
}

// tamperTemp 把子链接里 t= 值改动一位，破坏签名。
func tamperTemp(child string) string {
	// child 形如 http://host/url?t=XXX&u=YYY 或 /url?t=XXX&u=YYY
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
