package curl

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/HasonHuang/mytv/go/internal/config"
)

// 本文件是 E2E 层：真编译、真起进程、真走 TCP。
// 与（单元/集成，httptest 直打 handler）的区别在于——这里把整个
// 二进制当黑盒，验证「配置→进程→上游」这条链路端到端成立，而非函数契约。
//
// 置于 module 内的独立包，只依赖 cmd 那个二进制与 config 的导出面：
// 不碰 server 的任何未导出符号，才配叫黑盒。

// e2eServer 是一台跑起来的被测服务。
type e2eServer struct {
	base string
	cmd  *exec.Cmd
	t    *testing.T
}

// startE2E 编译 cmd 二进制、按给定 config/tokens 起进程，并就绪探测。
func startE2E(t *testing.T, cfg *config.Config, tokenLines ...string) *e2eServer {
	t.Helper()
	dir := t.TempDir()

	bin := filepath.Join(dir, "sontv-go")
	// 用模块导入路径定位 cmd，不受本包所在目录影响。
	build := exec.Command("go", "build", "-o", bin, "github.com/HasonHuang/mytv/go/cmd/sontv-go")
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("编译失败: %v\n%s", err, out)
	}

	tokensPath := filepath.Join(dir, "tokens.txt")
	if err := os.WriteFile(tokensPath, []byte(strings.Join(tokenLines, "\n")+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	cfg.TokensFile = tokensPath
	cfgBytes := []byte(fmt.Sprintf(
		`{"tokens_file":%q,"default_ttl_hours":%d,"upstream_m3u":%q,"listen":%q,"unwrap_remote_proxy":%v}`,
		cfg.TokensFile, cfg.DefaultTTLHours, cfg.UpstreamM3U, cfg.Listen, cfg.UnwrapRemoteProxy))
	cfgPath := filepath.Join(dir, "config.json")
	if err := os.WriteFile(cfgPath, cfgBytes, 0o600); err != nil {
		t.Fatal(err)
	}

	cmd := exec.Command(bin, "-config", cfgPath)
	cmd.Stdout, cmd.Stderr = os.Stdout, os.Stderr
	if err := cmd.Start(); err != nil {
		t.Fatalf("启动失败: %v", err)
	}
	es := &e2eServer{base: "http://" + cfg.Listen, cmd: cmd, t: t}
	t.Cleanup(es.stop)
	es.waitReady()
	return es
}

// waitReady 轮询直到某个端点能建立 TCP 连接（进程已监听）。
func (e *e2eServer) waitReady() {
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		resp, err := http.Get(e.base + "/sub")
		if err == nil {
			resp.Body.Close()
			return
		}
		time.Sleep(50 * time.Millisecond)
	}
	e.t.Fatalf("服务未在超时内就绪: %s", e.base)
}

func (e *e2eServer) stop() {
	if e.cmd.Process != nil {
		_ = e.cmd.Process.Kill()
		_, _ = e.cmd.Process.Wait()
	}
}

// get 发一次 GET，返回状态码与正文。target 可以是本站相对路径，
// 也可以是子链接那样的绝对地址——绝对地址原样用，免得重复拼 base。
func (e *e2eServer) get(target string) (int, string) {
	e.t.Helper()
	if !strings.HasPrefix(target, "http://") && !strings.HasPrefix(target, "https://") {
		target = e.base + target
	}
	resp, err := http.Get(target)
	if err != nil {
		e.t.Fatalf("GET %s 失败: %v", target, err)
	}
	defer resp.Body.Close()
	b, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, string(b)
}

// freePort 拿一个当前空闲的回环端口（先监听再关，E2E 起进程前占位）。
func freePort(t *testing.T) string {
	t.Helper()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := ln.Addr().String()
	ln.Close()
	return addr
}

// hashOf 造一行所需的 sha256hex。
func hashOf(token string) string {
	sum := sha256.Sum256([]byte(token))
	return hex.EncodeToString(sum[:])
}

// extractFirstURL 从订阅正文里取出第一条本站 /url 子链接（绝对地址）。
// server 包测试有同名辅助，但那是包内私有；黑盒包不该伸手去要，就地重写。
func extractFirstURL(t *testing.T, body string) string {
	t.Helper()
	for _, line := range strings.Split(body, "\n") {
		line = strings.TrimSpace(line)
		if i := strings.Index(line, "/url?"); i >= 0 {
			return line
		}
	}
	t.Fatalf("正文里没有 /url 子链接:\n%s", body)
	return ""
}

// signalUSR1 返回触发重载的信号值（Linux/macOS 均为 SIGUSR1）。
func signalUSR1() os.Signal {
	return syscall.SIGUSR1
}

// e2eConfig 造一份指向自身端口的配置。
func e2eConfig(t *testing.T, upstream string) *config.Config {
	t.Helper()
	c := config.DefaultConfig()
	c.Listen = freePort(t)
	c.UpstreamM3U = upstream
	c.UnwrapRemoteProxy = true
	return c
}

// TestE2EAuthFailsClosed 起真进程，验证无凭据/坏凭据 → 403。
func TestE2EAuthFailsClosed(t *testing.T) {
	cfg := e2eConfig(t, "http://127.0.0.1:9/none.m3u")
	es := startE2E(t, cfg, "主,"+hashOf("good"))

	if code, _ := es.get("/sub"); code != http.StatusForbidden {
		t.Fatalf("无凭据应 403，实际 %d", code)
	}
	if code, _ := es.get("/sub?token=wrong"); code != http.StatusForbidden {
		t.Fatalf("坏凭据应 403，实际 %d", code)
	}
}

// TestE2EHappyPathAndChild 起真进程 + 模拟上游，验证订阅改写与子链接可用。
func TestE2EHappyPathAndChild(t *testing.T) {
	var upstream string
	up := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/live/a.ts" {
			w.Header().Set("Content-Type", "video/mp2t")
			_, _ = w.Write([]byte("TS-BYTES"))
			return
		}
		_, _ = w.Write([]byte("#EXTM3U\n#EXTINF:-1,翡翠台\n" + upstream + "/live/a.ts\n"))
	}))
	defer up.Close()
	upstream = up.URL

	cfg := e2eConfig(t, up.URL+"/list.m3u")
	es := startE2E(t, cfg, "主,"+hashOf("good"))

	code, body := es.get("/sub?token=good")
	if code != http.StatusOK {
		t.Fatalf("订阅应 200，实际 %d: %s", code, body)
	}
	if strings.Contains(body, "good") {
		t.Fatalf("正文不该含稳定 token: %s", body)
	}
	child := extractFirstURL(t, body)

	// 子链接走真 TCP 回服务本身
	if code, body := es.get(child); code != http.StatusOK || body != "TS-BYTES" {
		t.Fatalf("子链接应 200+TS，实际 %d %q", code, body)
	}
}

// TestE2ESelfRef400 起真进程，验证目标指向本站自身 → 400。
func TestE2ESelfRef400(t *testing.T) {
	cfg := e2eConfig(t, "http://127.0.0.1:9/none.m3u")
	es := startE2E(t, cfg, "主,"+hashOf("good"))

	self := url.QueryEscape("http://" + cfg.Listen + "/x.ts")
	if code, _ := es.get("/url?token=good&u=" + self); code != http.StatusBadRequest {
		t.Fatalf("自引用应 400，实际 %d", code)
	}
}

// TestE2EReloadRevokes USR1 重载后删行即吊销。
func TestE2EReloadRevokes(t *testing.T) {
	cfg := e2eConfig(t, "http://127.0.0.1:9/none.m3u")
	es := startE2E(t, cfg, "主,"+hashOf("good"), "副,"+hashOf("other"))

	// 重写 token 表：主行被行首 # 禁用，副行还在（表非空 → 403 而非 503）
	// cfg.TokensFile 由 startE2E 写定；此处直接覆写该文件。
	if err := os.WriteFile(cfg.TokensFile, []byte("#主,"+hashOf("good")+"\n副,"+hashOf("other")+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := es.cmd.Process.Signal(signalUSR1()); err != nil {
		t.Fatalf("发 USR1 失败: %v", err)
	}
	time.Sleep(200 * time.Millisecond)

	if code, _ := es.get("/sub?token=good"); code != http.StatusForbidden {
		t.Fatalf("重载后主 token 应 403，实际 %d", code)
	}
}
