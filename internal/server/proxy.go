package server

import (
	"bytes"
	"io"
	"net/http"
	"net/url"
	"sync"

	"github.com/HasonHuang/mytv/go/internal/playlist"
	"github.com/HasonHuang/mytv/go/internal/tokens"
)

// 内存纪律（设计 §5.3、ADR-0004）：
//   - 探测块 8KB，sync.Pool 复用，每请求不新分配；
//   - 非 m3u8 一律 io.Copy 流式，峰值 O(32KB)；
//   - m3u8 缓冲硬上限 4MB，超限 502，绝不整段读进内存。
const (
	probeSize           = 8 << 10 // 8KB
	maxBufferedPlaylist = 4 << 20 // 4MB
)

// probePool 复用 8KB 探测缓冲，避免每请求分配（ADR-0004）。
var probePool = sync.Pool{New: func() any {
	b := make([]byte, probeSize)
	return &b
}}

// handleURL 处理 /url：代理目标地址，流式吐回；若返回是 m3u8 则逐行改写盖章。
//
// 认证接受稳定或临时 token（ADR-0002），优先临时。Range 原样透传，
// 上游重定向由客户端自动跟随（≤5 跳，不透传 Location）。
func (s *Server) handleURL(w http.ResponseWriter, r *http.Request) {
	// 日志上下文在认证之前就建：认证失败（401/403/503）恰恰是「播不了」的
	// 头号原因，没有编号就对不上是哪一次播放出的问题。
	lg := newReqLog(credKind(r.URL.Query()), "", r.URL.Query().Get(targetParam))

	row := s.authURL(w, r, lg)
	if row == nil {
		return
	}
	lg.label = row.Label

	if r.Method != http.MethodGet && r.Method != http.MethodHead {
		lg.warnf("方法不支持 方法=%s", r.Method)
		writeErr(w, http.StatusMethodNotAllowed, "只支持 GET/HEAD")
		return
	}
	lg.debugf("开始 方法=%s 目标=%s", r.Method, lg.target)

	target, err := parseTarget(r.URL.Query().Get(targetParam))
	if err != nil {
		lg.warnf("目标非法 原因=%s", err)
		writeErr(w, http.StatusBadRequest, err.Error())
		return
	}
	if s.isSelfRef(target, r) {
		lg.warnf("拒绝自引用 目标=%s", lg.target)
		writeErr(w, http.StatusBadRequest, "目标不可指向本站")
		return
	}

	resp, err := s.doUpstream(r, target)
	if err != nil {
		lg.errorf("上游请求失败 原因=%s 耗时=%dms", safeErr(err), lg.ms())
		writeErr(w, http.StatusBadGateway, "上游请求失败")
		return
	}
	defer func() { _ = resp.Body.Close() }()

	lg.debugf("上游 状态=%d 类型=%q 长度=%d 耗时=%dms 最终=%s%s",
		resp.StatusCode, resp.Header.Get("Content-Type"), resp.ContentLength, lg.ms(),
		safeURL(resp.Request.URL.String()), redirectMark(target, resp))

	// HEAD 无正文，直接透传状态与头；改写无从谈起。
	if r.Method == http.MethodHead {
		copyRespHeaders(w.Header(), resp.Header)
		w.WriteHeader(resp.StatusCode)
		lg.debugf("HEAD 透传完成 耗时=%dms", lg.ms())
		return
	}

	probe, err := readProbe(resp.Body)
	if err != nil {
		lg.errorf("探测块读取失败 原因=%s", safeErr(err))
		writeErr(w, http.StatusBadGateway, "上游读取失败")
		return
	}

	// 只有正文真的以 #EXTM3U 开头才改写（ADR-0004 §后果，比 PHP 保守）：
	// 后缀/Content-Type 命中的坑文件不会被误当 playlist 逐行处理。
	if !startsWithEXTM3U(probe) {
		streamThrough(w, resp, probe, lg)
		return
	}
	s.streamRewritten(w, resp, probe, row, r, lg)
}

// redirectMark 在发生重定向时补一句标记：最终地址与请求地址不同即跳转过。
// 排障时「上游把我甩到哪」是高频问题，日志里要一眼看出来。
func redirectMark(want *url.URL, resp *http.Response) string {
	if resp.Request == nil || resp.Request.URL == nil || want == nil {
		return ""
	}
	if resp.Request.URL.String() == want.String() {
		return ""
	}
	return " (已重定向)"
}

// credKind 从查询参数判定凭据种类，仅用于日志措辞。
func credKind(q url.Values) string {
	if q.Get(tempParam) != "" {
		return "临时"
	}
	return "稳定"
}

// doUpstream 向上游发请求：透传 Range/If-Range，带上伪装 UA（对齐 PHP 版）。
// 不转发客户端其它头，避免泄露；重定向由 client 自动跟随（≤5 跳，防环）。
func (s *Server) doUpstream(r *http.Request, target *url.URL) (*http.Response, error) {
	req, err := http.NewRequestWithContext(r.Context(), r.Method, target.String(), nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", UserAgent)
	if rng := r.Header.Get("Range"); rng != "" {
		req.Header.Set("Range", rng)
	}
	if ir := r.Header.Get("If-Range"); ir != "" {
		req.Header.Set("If-Range", ir)
	}
	return s.client.Do(req)
}

// readProbe 读取最多 8KB 探测块；不足视为 EOF，正常返回。
func readProbe(body io.Reader) ([]byte, error) {
	buf := probePool.Get().(*[]byte)
	n, err := io.ReadFull(body, *buf)
	if err == io.EOF || err == io.ErrUnexpectedEOF {
		err = nil
	}
	if err != nil {
		probePool.Put(buf)
		return nil, err
	}
	// 复制出探测内容再归还缓冲：调用方要持有这段字节，池缓冲不能外泄。
	probe := make([]byte, n)
	copy(probe, (*buf)[:n])
	probePool.Put(buf)
	return probe, nil
}

// streamThrough 非 m3u8 响应：透传状态与头，写回探测块后 io.Copy 剩余。
// 全程 O(32KB)，但不设正文上限——本就是流。
func streamThrough(w http.ResponseWriter, resp *http.Response, probe []byte, lg *reqLog) {
	copyRespHeaders(w.Header(), resp.Header)
	w.WriteHeader(resp.StatusCode)
	if _, err := w.Write(probe); err != nil {
		// 客户端中途断开：长流的常态，不必响亮，但要留痕（否则「播到一半断了」无从查）。
		lg.debugf("直传写探测块失败 原因=%s 已写=%d 耗时=%dms", safeErr(err), len(probe), lg.ms())
		return
	}
	flush(w)

	// 播 .ts/.m4s 分片时一次播放会打几十上百次请求，逐条记字节数与耗时，
	// 是「卡在第几片」这类问题唯一能用的线索。
	n, err := io.Copy(w, resp.Body)
	if err != nil {
		lg.debugf("直传中断 原因=%s 已写=%dB 耗时=%dms", safeErr(err), int64(len(probe))+n, lg.ms())
		return
	}
	lg.debugf("直传完成 字节=%dB 探测头=%q 耗时=%dms",
		int64(len(probe))+n, preview(probe, 32), lg.ms())
}

// streamRewritten m3u8 响应：缓冲上限 4MB（超限 502），判定为 playlist 才改写。
// WriteHeader 一经发出不可回退，故超限检测必须在此之前完成（ADR-0004）。
func (s *Server) streamRewritten(w http.ResponseWriter, resp *http.Response, probe []byte,
	row *tokens.Row, r *http.Request, lg *reqLog) {
	full, ok := bufferPlaylist(resp.Body, probe)
	if !ok {
		lg.errorf("列表超限 上限=%dB", maxBufferedPlaylist)
		writeErr(w, http.StatusBadGateway, "playlist 超出上限")
		return
	}

	// 相对地址以最终上游 URL 为基准（重定向后 resp.Request.URL 才是真身）。
	final := resp.Request.URL
	kws := playlist.ParseFilterKeywords(r.URL.Query().Get(filterParam))
	out := playlist.RewritePlaylist(full, s.rewriteOpts(r, final, row, kws))

	w.Header().Set("Content-Type", "application/vnd.apple.mpegurl; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(http.StatusOK)
	_, _ = io.WriteString(w, out)
	lg.debugf("改写完成 入=%dB 出=%dB 基准=%s 过滤=%v 耗时=%dms",
		len(full), len(out), safeURL(final.String()), kws, lg.ms())
}

// bufferPlaylist 把探测块与剩余正文拼成完整列表；超过 4MB 上限返回 false。
// 已判定前缀是 #EXTM3U，故非 playlist 的巨物不会走到这里。
func bufferPlaylist(rest io.Reader, probe []byte) (string, bool) {
	if len(probe) > maxBufferedPlaylist {
		return "", false
	}
	var b bytes.Buffer
	b.Grow(len(probe) + probeSize)
	b.Write(probe)

	remaining := int64(maxBufferedPlaylist - len(probe))
	// 多读 1 字节以区分「恰好等于上限」与「超出上限」。
	n, err := io.Copy(&b, io.LimitReader(rest, remaining+1))
	if err != nil || n > remaining {
		return "", false
	}
	return b.String(), true
}

// startsWithEXTM3U 判断正文（跳过 BOM 与首部空白）是否以 #EXTM3U 开头。
func startsWithEXTM3U(probe []byte) bool {
	s := bytes.TrimPrefix(probe, []byte{0xEF, 0xBB, 0xBF}) // 容忍 UTF-8 BOM
	s = bytes.TrimLeft(s, " \t\r\n")
	if len(s) < len("#EXTM3U") {
		return false
	}
	return bytes.EqualFold(s[:len("#EXTM3U")], []byte("#EXTM3U"))
}

// copyRespHeaders 透传内容相关头（绝不含 Location，避免暴露真实上游）。
func copyRespHeaders(dst, src http.Header) {
	for _, k := range []string{
		"Content-Type", "Content-Length", "Content-Range",
		"Accept-Ranges", "Last-Modified", "ETag",
	} {
		if v := src.Get(k); v != "" {
			dst.Set(k, v)
		}
	}
}

// flush 尽量把已写字节推给客户端，长流不憋在缓冲里。
func flush(w http.ResponseWriter) {
	if f, ok := w.(http.Flusher); ok {
		f.Flush()
	}
}
