package server

import (
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/HasonHuang/sontv/internal/config"
	"github.com/HasonHuang/sontv/internal/playlist"
	"github.com/HasonHuang/sontv/internal/temptoken"
	"github.com/HasonHuang/sontv/internal/tokens"
)

// UserAgent 伪装浏览器，部分源站对默认 Go UA 拒绝服务。
// 导出是给项目外的真实客户端测试复用，免得各写一份。
const UserAgent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)"

// fetchTimeout 是抓上游 playlist 的整体超时；资源流式代理不设总超时。
const fetchTimeout = 30 * time.Second

// maxRedirects 是上游重定向上限，防环（设计 §2.5.1）。
const maxRedirects = 5

// Server 持有配置、token 表与 HTTP 客户端，全部只读，可并发使用。
type Server struct {
	cfg    *config.Config
	tokens *tokens.TokenTable
	client *http.Client
	now    func() time.Time // 注入时钟，测试无需等真实时间
}

// NewServer 组装一个 Server；now 为 nil 时用 time.Now。
func NewServer(cfg *config.Config, tokens *tokens.TokenTable) *Server {
	return &Server{cfg: cfg, tokens: tokens, client: newHTTPClient(), now: time.Now}
}

// NewServerWithHTTP 供项目外（如 cmd 之上的真实客户端测试）注入自定义
// *http.Client——没有它，外部包只能靠环境变量间接影响出站。now 为 nil 时用 time.Now。
func NewServerWithHTTP(cfg *config.Config, tokens *tokens.TokenTable, client *http.Client, now func() time.Time) *Server {
	if client == nil {
		client = newHTTPClient()
	}
	if now == nil {
		now = time.Now
	}
	return &Server{cfg: cfg, tokens: tokens, client: client, now: now}
}

// newHTTPClient 构建跟随重定向（≤maxRedirects）、不设总超时的客户端——
// 总超时会切断 .ts 长流；上游响应头等待由 ResponseHeaderTimeout 兜底。
func newHTTPClient() *http.Client {
	return &http.Client{
		Transport: &http.Transport{
			Proxy:                 http.ProxyFromEnvironment,
			DialContext:           (&net.Dialer{Timeout: 5 * time.Second, KeepAlive: 30 * time.Second}).DialContext,
			TLSHandshakeTimeout:   5 * time.Second,
			ResponseHeaderTimeout: 15 * time.Second,
		},
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if len(via) >= maxRedirects {
				return fmt.Errorf("重定向超过 %d 跳", maxRedirects)
			}
			return nil
		},
	}
}

// Handler 返回路由。两个端点显式注册，各自认证。
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/sub", s.handleSub)
	mux.HandleFunc("/play", s.handlePlay)
	return mux
}

// ---------- 目标地址 ----------

// parseTarget 校验目标地址：必须是 http(s) 绝对地址。
func parseTarget(raw string) (*url.URL, error) {
	if strings.TrimSpace(raw) == "" {
		return nil, errors.New("缺少目标地址")
	}
	u, err := url.Parse(raw)
	if err != nil {
		return nil, errors.New("目标地址非法")
	}
	if !strings.EqualFold(u.Scheme, "http") && !strings.EqualFold(u.Scheme, "https") {
		return nil, errors.New("只支持 http(s) 目标")
	}
	if u.Host == "" {
		return nil, errors.New("目标地址缺少主机")
	}
	return u, nil
}

// isSelfRef 判断目标是否指向本站自身（防呆，非安全边界，ADR-0005）。
//
// 主机名相等之外还要端口能对上：只比主机名会把「指向本机另一端口」的合法
// 上游也误伤（常见于本机 CDN 端口 ≠ 监听端口）。
func (s *Server) isSelfRef(target *url.URL, r *http.Request) bool {
	th := strings.ToLower(target.Hostname())
	if th == "" {
		return false
	}
	tp := targetPort(target)
	return sameEndpoint(r.Host, th, tp) || sameEndpoint(s.cfg.Listen, th, tp)
}

// sameEndpoint 判断 host:port 是否即 thost:tport。任一方无端口时只比主机名。
func sameEndpoint(hostport, thost, tport string) bool {
	if hostport == "" {
		return false
	}
	h, p, err := net.SplitHostPort(hostport)
	if err != nil { // 无端口：整个串就是主机名
		h, p = hostport, ""
	}
	if !strings.EqualFold(h, thost) {
		return false
	}
	return p == "" || tport == "" || p == tport
}

// targetPort 取目标显式端口；缺省时按 scheme 补默认端口。
func targetPort(u *url.URL) string {
	if p := u.Port(); p != "" {
		return p
	}
	if strings.EqualFold(u.Scheme, "https") {
		return "443"
	}
	return "80"
}

// hostOnly 去掉端口并小写；net.SplitHostPort 失败时原样小写。
func hostOnly(h string) string {
	if host, _, err := net.SplitHostPort(h); err == nil {
		return strings.ToLower(host)
	}
	return strings.ToLower(h)
}

// selfRoot 从请求现推本站入口 scheme://host[:port]，作为子链接的绝对前缀。
//
// scheme 取自 X-Forwarded-Proto（前置代理 TLS 终止在 nginx，Go 只收明文
// HTTP，离了它会把 https 播主下回 http 丢裸奔）；无该头时按 r.TLS 判断。
// host 原样保留端口——子链接要精确指回这个入口，去掉端口在自定义端口上就错了。
func selfRoot(r *http.Request) string {
	host := r.Host
	if host == "" {
		return ""
	}
	return schemeOf(r) + "://" + host
}

// schemeOf 取请求的实际 scheme：优先前置代理的 X-Forwarded-Proto，否则看 TLS。
func schemeOf(r *http.Request) string {
	if p := r.Header.Get("X-Forwarded-Proto"); p != "" {
		// 该头可能是逗号分隔的代理链（"https, http"），取第一跳即客户端所见。
		if i := strings.IndexByte(p, ','); i >= 0 {
			p = p[:i]
		}
		if proto := strings.ToLower(strings.TrimSpace(p)); proto == "http" || proto == "https" {
			return proto
		}
	}
	if r.TLS != nil {
		return "https"
	}
	return "http"
}

// ---------- 改写上下文 ----------

// rewriteOpts 依据最终上游地址与签发者生成一次改写的全部上下文。
func (s *Server) rewriteOpts(r *http.Request, upstream *url.URL, row *tokens.Row, kws []string) playlist.RewriteOptions {
	root, dir := baseOf(upstream)
	return playlist.RewriteOptions{
		FilterKeywords: kws,
		TempToken:      temptoken.Issue(row, s.now(), temptoken.TTL(row, s.cfg.DefaultTTLHours)),
		BaseRoot:       root,
		BaseDir:        dir,
		SelfHost:       hostOnly(r.Host),
		SelfRoot:       selfRoot(r),
	}
}

// baseOf 从上游地址推出相对路径基准与目录基准。
func baseOf(u *url.URL) (root, dir string) {
	root = u.Scheme + "://" + u.Host
	if i := strings.LastIndex(u.Path, "/"); i >= 0 {
		dir = root + u.Path[:i+1]
	} else {
		dir = root + "/"
	}
	return root, dir
}

// writeErr 写一行纯文本错误，不回服务器信息、不回上游细节。
func writeErr(w http.ResponseWriter, code int, msg string) {
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	w.WriteHeader(code)
	_, _ = w.Write([]byte(msg + "\n"))
}
