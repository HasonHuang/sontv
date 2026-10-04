package server

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/HasonHuang/sontv/internal/tokens"
)

// writeTable 覆盖写 token 表文件（每行 "标签,sha256hex[,ttl]"）。
func writeTable(t *testing.T, path string, lines ...string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(strings.Join(lines, "\n")+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
}

// rowLine 造一行 "标签,hash[,ttl]"。
func rowLine(label, token, ttl string) string {
	sum := sha256.Sum256([]byte(token))
	h := hex.EncodeToString(sum[:])
	if ttl == "" {
		return label + "," + h
	}
	return label + "," + h + "," + ttl
}

// tableWith 建一份表并装载，返回表与路径（便于后续覆盖重载）。
func tableWith(t *testing.T, lines ...string) (*tokens.TokenTable, string) {
	t.Helper()
	path := filepath.Join(t.TempDir(), "tokens.txt")
	writeTable(t, path, lines...)
	tt := tokens.NewTokenTable(path)
	if err := tt.Load(); err != nil {
		t.Fatalf("装载失败: %v", err)
	}
	return tt, path
}

// #13：/sub?url=<编码上游> → 用指定上游，子链接规则同 #3。
func TestMatrixSubExplicitUpstream(t *testing.T) {
	up := httptest.NewServer(cannedM3U("http://cdn/special.ts"))
	defer up.Close()

	table, _ := tableWith(t, rowLine("主", "good", ""))
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec,
		httptest.NewRequest("GET", "/sub?token=good&url="+urlEnc(up.URL+"/list.m3u"), nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("#13 期望 200，实际 %d: %s", rec.Code, rec.Body.String())
	}
	if !strings.Contains(rec.Body.String(), "url=http%3A%2F%2Fcdn%2Fspecial.ts") {
		t.Fatalf("#13 指定上游未生效: %s", rec.Body.String())
	}
}

// #14 / #15：filter 命中与不命中。
func TestMatrixSubFilter(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n" +
			"#EXTINF:-1,翡翠台\nhttp://cdn/jade.ts\n" +
			"#EXTINF:-1,新闻台\nhttp://cdn/news.ts\n"))
	}))
	defer up.Close()

	table, _ := tableWith(t, rowLine("主", "good", ""))
	cfg := testConfig()
	cfg.UpstreamM3U = up.URL
	s := newTestServer(t, cfg, table)

	// #14
	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/sub?token=good&filter="+urlEnc("翡翠"), nil))
	body := rec.Body.String()
	if !strings.Contains(body, "jade.ts") || strings.Contains(body, "news.ts") {
		t.Fatalf("#14 过滤结果不对: %s", body)
	}

	// #15
	rec15 := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec15, httptest.NewRequest("GET", "/sub?token=good&filter="+urlEnc("不存在"), nil))
	if rec15.Code != http.StatusOK {
		t.Fatalf("#15 期望 200，实际 %d", rec15.Code)
	}
	if got := rec15.Body.String(); strings.Contains(got, ".ts") {
		t.Fatalf("#15 应只剩 #EXTM3U: %s", got)
	}
}

// #16：第三列设 TTL=2h → 子链接 2h 后 401（用户级 TTL 生效）。
func TestMatrixPerUserTTL(t *testing.T) {
	up := httptest.NewServer(cannedM3U("http://cdn/a.ts"))
	defer up.Close()

	base := time.Unix(1_700_000_000, 0)
	table, _ := tableWith(t, rowLine("主", "good", "2"))
	cfg := testConfig()
	cfg.UpstreamM3U = up.URL
	s := NewServer(cfg, table)
	s.now = func() time.Time { return base }

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/sub?token=good", nil))
	child := extractFirstURL(t, rec.Body.String())

	// 2h 内：认证通过即可（子链接指向假主机，抓取失败是 502，不该是 401）
	if code := fetchChild(s, child, base.Add(time.Hour)).Code; code == http.StatusUnauthorized {
		t.Fatalf("#16 1h 内不该 401，实际 %d", code)
	}
	// 超过 2h 过期 → 401
	if code := fetchChild(s, child, base.Add(3*time.Hour)).Code; code != http.StatusUnauthorized {
		t.Fatalf("#16 超 TTL 应 401，实际 %d", code)
	}
}

// #17：行首 # 禁用一行 + reload → 403。
func TestMatrixLineDisabledAfterReload(t *testing.T) {
	table, path := tableWith(t, rowLine("主", "good", ""), rowLine("副", "other", ""))
	s := newTestServer(t, testConfig(), table)

	ok := httptest.NewRecorder()
	s.Handler().ServeHTTP(ok, httptest.NewRequest("GET", "/sub?token=good", nil))
	// 上游默认不可达，认证过应是 502 而非 403——只要不是 403 即认证已通过
	if ok.Code == http.StatusForbidden {
		t.Fatalf("重载前不该 403，实际 %d", ok.Code)
	}

	// 行首加 # 禁用主行；副行仍在，表非空，故应为 403 而非 503
	writeTable(t, path, "#"+rowLine("主", "good", ""), rowLine("副", "other", ""))
	if err := table.Load(); err != nil {
		t.Fatalf("重载失败: %v", err)
	}
	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/sub?token=good", nil))
	if rec.Code != http.StatusForbidden {
		t.Fatalf("#17 禁用后应 403，实际 %d", rec.Code)
	}
}

// #6/#7：临时 token 篡改 ur 或已过期 → 401；删行 + reload 后 → 401。
func TestMatrixTempTokenInvalid(t *testing.T) {
	up := httptest.NewServer(cannedM3U("http://cdn/a.ts"))
	defer up.Close()

	table, path := tableWith(t, rowLine("主", "good", ""))
	cfg := testConfig()
	cfg.UpstreamM3U = up.URL
	s := newTestServer(t, cfg, table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/sub?token=good", nil))
	child := extractFirstURL(t, rec.Body.String())

	// #6：过期（篡改 exp），签名随之破裂
	expired := child
	if i := strings.Index(expired, "t="); i >= 0 {
		expired = expired[:i+2] + "0." + expired[i+2:]
	}
	if code := fetchChild(s, expired, s.now()).Code; code != http.StatusUnauthorized {
		t.Fatalf("#6 期望 401，实际 %d", code)
	}

	// #7：删行 + reload → 该 uid 的临时 token 失效
	writeTable(t, path, rowLine("其它", "other", ""))
	if err := table.Load(); err != nil {
		t.Fatalf("重载失败: %v", err)
	}
	if code := fetchChild(s, child, s.now()).Code; code != http.StatusUnauthorized {
		t.Fatalf("#7 uid 已删应 401，实际 %d", code)
	}
}

// #8：删行 + reload 后用稳定 token 访问 → 403。
func TestMatrixStableDeletedAfterReload(t *testing.T) {
	table, path := tableWith(t, rowLine("主", "good", ""))
	s := newTestServer(t, testConfig(), table)

	writeTable(t, path, rowLine("副", "other", ""))
	if err := table.Load(); err != nil {
		t.Fatalf("重载失败: %v", err)
	}
	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec, httptest.NewRequest("GET", "/sub?token=good", nil))
	if rec.Code != http.StatusForbidden {
		t.Fatalf("#8 期望 403，实际 %d", rec.Code)
	}
}

// 边界：/play 缺 url、非 http(s)、方法非 GET/HEAD。
func TestMatrixPlayErrors(t *testing.T) {
	table, _ := tableWith(t, rowLine("主", "good", ""))
	s := newTestServer(t, testConfig(), table)
	h := s.Handler()

	cases := []struct {
		name, url string
		code      int
	}{
		{"缺url", "/play?token=good", http.StatusBadRequest},
		{"非http", "/play?token=good&url=ftp%3A%2F%2Fx", http.StatusBadRequest},
		{"坏token", "/play?token=bad&url=" + urlEnc("http://x/a.ts"), http.StatusForbidden},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			rec := httptest.NewRecorder()
			h.ServeHTTP(rec, httptest.NewRequest("GET", c.url, nil))
			if rec.Code != c.code {
				t.Fatalf("期望 %d，实际 %d", c.code, rec.Code)
			}
		})
	}

	// 方法不允许
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequest("POST", "/play?token=good&url="+urlEnc("http://x/a.ts"), nil))
	if rec.Code != http.StatusMethodNotAllowed {
		t.Fatalf("POST 期望 405，实际 %d", rec.Code)
	}
}

// 边界：上游 500 转发给客户端；缺 Content-Type 的流照常吐。
func TestMatrixUpstreamStatus(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
		_, _ = w.Write([]byte("boom"))
	}))
	defer up.Close()

	table, _ := tableWith(t, rowLine("主", "good", ""))
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec,
		httptest.NewRequest("GET", "/play?token=good&url="+urlEnc(up.URL+"/a.ts"), nil))
	if rec.Code != http.StatusInternalServerError {
		t.Fatalf("上游 500 应原样透传，实际 %d", rec.Code)
	}
	if rec.Body.String() != "boom" {
		t.Fatalf("正文未透传: %q", rec.Body.String())
	}
}

// 边界：上游列表超 8MiB → 502，不静默截断（截断会吐出半份列表）。
func TestMatrixSubOverflow(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n"))
		_, _ = w.Write(bytes.Repeat([]byte("# padding\n"), 900_000)) // >8MiB
	}))
	defer up.Close()

	table, _ := tableWith(t, rowLine("主", "good", ""))
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec,
		httptest.NewRequest("GET", "/sub?token=good&url="+urlEnc(up.URL+"/big.m3u"), nil))
	if rec.Code != http.StatusBadGateway {
		t.Fatalf("超限期望 502，实际 %d", rec.Code)
	}
}

// 边界：HEAD 无正文，透传状态与头。
func TestMatrixHeadPassthrough(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "video/mp2t")
		w.Header().Set("Accept-Ranges", "bytes")
		w.WriteHeader(http.StatusOK)
	}))
	defer up.Close()

	table, _ := tableWith(t, rowLine("主", "good", ""))
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec,
		httptest.NewRequest("HEAD", "/play?token=good&url="+urlEnc(up.URL+"/a.ts"), nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("HEAD 期望 200，实际 %d", rec.Code)
	}
	if rec.Header().Get("Accept-Ranges") != "bytes" {
		t.Fatalf("HEAD 未透传头: %v", rec.Header())
	}
}

// 边界：m3u8 超过 4MB 上限 → 502（ADR-0004）。
func TestMatrixPlaylistOverflow(t *testing.T) {
	big := "#EXTM3U\n" + strings.Repeat("# comment padding line\n", 200_000) // >4MB
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(big))
	}))
	defer up.Close()

	table, _ := tableWith(t, rowLine("主", "good", ""))
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec,
		httptest.NewRequest("GET", "/play?token=good&url="+urlEnc(up.URL+"/big.m3u8"), nil))
	if rec.Code != http.StatusBadGateway {
		t.Fatalf("超限期望 502，实际 %d", rec.Code)
	}
}

// 边界：带 BOM 的 #EXTM3U 仍被识别为 playlist。
func TestMatrixBOMRecognized(t *testing.T) {
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("\xEF\xBB\xBF#EXTM3U\n#EXTINF:-1,A\nhttp://cdn/a.ts\n"))
	}))
	defer up.Close()

	table, _ := tableWith(t, rowLine("主", "good", ""))
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec,
		httptest.NewRequest("GET", "/play?token=good&url="+urlEnc(up.URL+"/x.m3u8"), nil))
	if !strings.Contains(rec.Body.String(), "t=") {
		t.Fatalf("BOM 列表未改写: %s", rec.Body.String())
	}
}

// 边界：正文不以 #EXTM3U 开头 → 原样流式，不盖 token（ADR-0004）。
func TestMatrixNonPlaylistStreamed(t *testing.T) {
	payload := bytes.Repeat([]byte("TSPKT"), 3000) // >8KB 跨探测块
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/octet-stream")
		_, _ = w.Write(payload)
	}))
	defer up.Close()

	table, _ := tableWith(t, rowLine("主", "good", ""))
	s := newTestServer(t, testConfig(), table)

	rec := httptest.NewRecorder()
	s.Handler().ServeHTTP(rec,
		httptest.NewRequest("GET", "/play?token=good&url="+urlEnc(up.URL+"/x.m3u8"), nil))
	if !bytes.Equal(rec.Body.Bytes(), payload) {
		t.Fatalf("正文被改动，长度 %d 期望 %d", rec.Body.Len(), len(payload))
	}
	if strings.Contains(rec.Body.String(), "/play?") {
		t.Fatalf("非列表不该改写: %s", rec.Body.String())
	}
}

// ---------- 辅助 ----------

func cannedM3U(target string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,A\n" + target + "\n"))
	}
}

// fetchChild 在指定时刻访问子链接。
func fetchChild(s *Server, child string, at time.Time) *httptest.ResponseRecorder {
	rec := httptest.NewRecorder()
	s.now = func() time.Time { return at }
	req := httptest.NewRequest("GET", child, nil)
	s.Handler().ServeHTTP(rec, req)
	return rec
}
