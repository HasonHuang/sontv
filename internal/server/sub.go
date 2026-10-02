package server

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"

	"github.com/HasonHuang/mytv/go/internal/playlist"
)

// maxPlaylistBytes 是上游列表的读取上限，防超大响应撑爆内存（设计 §2.7）。
// 超限即拒（不静默截断）——截断会吐出半份播放列表，比报错更难排查。
const maxPlaylistBytes = 8 << 20 // 8 MiB

// handleSub 处理 /sub：抓取上游 m3u（缺省或 url=），过滤并改写后原样吐回。
//
// 认证只认稳定 token（ADR-0002）：订阅地址由用户主动配置，长期凭据落在这里。
// 改写盖章用一条响应一枚临时 token（子链接只带短命凭据）。
func (s *Server) handleSub(w http.ResponseWriter, r *http.Request) {
	// 与 /url 同一套日志：编号 + 脱敏地址，认证失败也留痕（订阅拉不到内容同样是故障）。
	lg := newReqLog("订阅", "", r.URL.Query().Get(subParam))

	row := s.authStable(w, r, lg)
	if row == nil {
		return
	}
	lg.label = row.Label
	lg.debugf("开始 目标=%s", lg.target)

	upstream, err := s.subUpstream(r)
	if err != nil {
		lg.warnf("上游非法 原因=%s", err)
		writeErr(w, http.StatusBadRequest, err.Error())
		return
	}
	if s.isSelfRef(upstream, r) {
		lg.warnf("拒绝自引用 上游=%s", safeURL(upstream.String()))
		writeErr(w, http.StatusBadRequest, "上游不可指向本站")
		return
	}

	body, err := s.fetchPlaylist(r.Context(), upstream)
	if err != nil {
		lg.errorf("上游抓取失败 原因=%s 耗时=%dms", safeErr(err), lg.ms())
		writeErr(w, http.StatusBadGateway, "上游抓取失败")
		return
	}

	kws := playlist.ParseFilterKeywords(r.URL.Query().Get(filterParam))
	opts := s.rewriteOpts(r, upstream, row, kws)
	out := playlist.RewritePlaylist(body, opts)

	w.Header().Set("Content-Type", "application/vnd.apple.mpegurl; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(http.StatusOK)
	_, _ = io.WriteString(w, out)
	lg.infof("改写完成 上游字节=%d 出=%dB 入口=%s 过滤=%v 耗时=%dms",
		len(body), len(out), safeURL(selfRoot(r)+"/sub"), kws, lg.ms())
}

// subUpstream 解析 /sub 的上游：有 url= 用它，否则用配置的缺省上游。
func (s *Server) subUpstream(r *http.Request) (*url.URL, error) {
	raw := strings.TrimSpace(r.URL.Query().Get(subParam))
	if raw == "" {
		raw = s.cfg.UpstreamM3U
	}
	if raw == "" {
		return nil, errors.New("未配置上游")
	}
	return parseTarget(raw)
}

// fetchPlaylist 拉取上游正文，限制大小与整体超时（设计 §2.7）。
// 错误消息保持笼统——上游细节只进日志语义，不出现在响应正文。
func (s *Server) fetchPlaylist(ctx context.Context, upstream *url.URL) (string, error) {
	ctx, cancel := context.WithTimeout(ctx, fetchTimeout)
	defer cancel()

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, upstream.String(), nil)
	if err != nil {
		return "", err
	}
	req.Header.Set("User-Agent", UserAgent)

	resp, err := s.client.Do(req)
	if err != nil {
		return "", err
	}
	defer func() { _ = resp.Body.Close() }()

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("上游返回 %d", resp.StatusCode)
	}
	// 多读 1 字节以区分「恰好 = 上限」与「超出上限」；超限即无法保证列表完整，宁可报错。
	body, err := io.ReadAll(io.LimitReader(resp.Body, maxPlaylistBytes+1))
	if err != nil {
		return "", err
	}
	if len(body) > maxPlaylistBytes {
		return "", errors.New("上游播放列表过大")
	}
	return string(body), nil
}
