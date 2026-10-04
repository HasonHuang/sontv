package playlist

import (
	"net/url"
	"regexp"
	"strings"
	"unicode/utf8"
)

// Playlist 改写是纯函数：输入正文与选项，输出正文，不碰网络。
// 选项全部显式传入，调用方决定「本站入口」「上游基准」「过滤词」是什么。

// /play 查询参数名。与 server 包的同名常量是同一份契约的两端；
// playlist 是纯函数包，不该反向依赖 server，故此处各自成文。
const (
	tempParam   = "t"   // 临时 token（改写后子链接携带）
	targetParam = "url" // 目标地址（/play 的上游，与 /sub 的 url= 同名）
)

// RewriteOptions 描述一次改写所需的全部上下文。
type RewriteOptions struct {
	// FilterKeywords 为空表示不过滤；非空按条目名子串匹配（OR）。
	FilterKeywords []string
	// TempToken 是本次响应所有子链接共用的临时 token（一条响应签一枚即可）。
	TempToken string
	// BaseRoot 是上游的 scheme://host[:port]。
	BaseRoot string
	// BaseDir 是上游相对资源所在目录（BaseRoot + 目录路径）。
	BaseDir string
	// SelfHost 是本站 host（小写、去端口），用于「本站入口形态」判定。
	SelfHost string
	// SelfRoot 是本站入口根 scheme://host[:port]，由请求头 Host 现取。
	// 子链接据此拼成绝对地址：上游列表里的相对路径与本站入口路径毫无关系，
	// 只有绝对地址才能让播放器无论从哪个入口拿到列表都一路走回本站。
	// 为空时退化为根相对 /play（纯函数测试等无入口场景）。
	SelfRoot string
}

// RewritePlaylist 逐行改写一份播放列表。
//
// 三类链接（ADR-0003）：
//   - 本站形态（自己的入口 /play?url=）→ 剥旧 t、重盖新 t（天然幂等）
//   - 相对路径 → 按 BaseRoot/BaseDir 补全成绝对
//   - 其余按资源行处理：包装进本站 /play
//
// 属性行（URL= / url-tvg= / x-tvg-url= / catchup-source=）只改写「本站形态」
// 的链接；纯第三方直连原样保留，凭据不落别人域名。含 ${...} 的 catchup-source
// 整条不动（模板不能 urlencode）。
//
// 换行符逐字节保留；不改写时输出 == 输入。
func RewritePlaylist(body string, opts RewriteOptions) string {
	if body == "" {
		return body
	}
	r := &rewriter{opts: opts, filtering: len(opts.FilterKeywords) > 0}
	r.run(body)
	return r.out.String()
}

// rewriter 承载逐行改写的跨行状态。抽成类型是为了让「一个条目块」的生命周期
// （# 行先缓冲、遇到资源行才决定留弃）显式化，主循环只做分发。
type rewriter struct {
	opts      RewriteOptions
	filtering bool
	out       strings.Builder

	pending    []string // 已改写、尚未落盘的 # 行（含各自分隔符），随块一起提交
	blockLines []string // 本块参与过滤匹配的原始 # 行
	infSeen    bool     // 本块出现过 #EXTINF
	infMatch   bool     // 本块命中过滤词
}

func (r *rewriter) run(body string) {
	r.out.Grow(len(body) + 256)
	for i := 0; i < len(body); {
		line, delim := splitLine(body, i)
		i += len(line) + len(delim)
		r.feed(line, delim)
	}
	// 收尾：末尾悬空的注释块（只有 # 行、没有 URL）
	if !r.filtering {
		r.flushPending()
	}
}

// feed 处理一行。分行类型：空行、# 行（含头部）、资源行。
func (r *rewriter) feed(line, delim string) {
	// 先剥 UTF-8 BOM 再判类型：带 BOM 的 #EXTM3U 否则会被当资源行，拼到 BaseDir 上。
	trimmed := strings.TrimRight(strings.TrimPrefix(line, "\ufeff"), " \t\r")
	if trimmed == "" {
		// 空行：不过滤时原样保留；过滤时丢弃（被滤掉的块不留下一片空行）
		if !r.filtering {
			r.flushPending()
			r.out.WriteString(line)
			r.out.WriteString(delim)
		}
		r.endBlock()
		return
	}
	if strings.HasPrefix(trimmed, "#") {
		r.feedComment(line, delim, trimmed)
		return
	}
	r.feedResource(line, delim)
}

// feedComment 处理一条 # 行：头行立即落盘，其余先缓冲等资源行定去留。
func (r *rewriter) feedComment(line, delim, trimmed string) {
	if isPlaylistHeader(trimmed) {
		// #EXTM3U 永远保留：它是列表头不是内容
		if !r.filtering {
			r.flushPending()
		}
		r.endBlock()
		r.out.WriteString(rewriteAttrs(line, r.opts))
		r.out.WriteString(delim)
		return
	}
	r.pending = append(r.pending, rewriteAttrs(line, r.opts)+delim)
	r.blockLines = append(r.blockLines, trimmed)
	if strings.HasPrefix(strings.ToUpper(trimmed), "#EXTINF") {
		r.infSeen = true
		if entryMatches(r.blockLines, r.opts.FilterKeywords) {
			r.infMatch = true
		}
	}
}

// feedResource 处理一条资源行：一个条目块到此结束，据命中情况决定留弃。
func (r *rewriter) feedResource(line, delim string) {
	if !r.filtering || (r.infSeen && r.infMatch) {
		r.flushPending()
		r.out.WriteString(rewriteResourceLine(strings.TrimSpace(line), r.opts))
		r.out.WriteString(delim)
	}
	r.endBlock()
}

// flushPending 提交本块缓冲的 # 行。
func (r *rewriter) flushPending() {
	for _, p := range r.pending {
		r.out.WriteString(p)
	}
	r.pending = r.pending[:0]
}

// endBlock 结束当前块，清空全部跨行状态。
func (r *rewriter) endBlock() {
	r.pending = r.pending[:0]
	r.blockLines = r.blockLines[:0]
	r.infSeen, r.infMatch = false, false
}

// splitLine 从 s[i:] 取出一行与它的分隔符（\r\n / \n / \r，原样返回）。
func splitLine(s string, i int) (line, delim string) {
	j := i
	for j < len(s) && s[j] != '\n' && s[j] != '\r' {
		j++
	}
	line = s[i:j]
	if j >= len(s) {
		return line, ""
	}
	if s[j] == '\r' && j+1 < len(s) && s[j+1] == '\n' {
		return line, "\r\n"
	}
	return line, s[j : j+1]
}

// ---------- 只取地址 ----------

// ExtractURLs 从一份播放列表里逐条取出资源行（地址行），丢弃 # 行与空行。
//
// 行类型判定与 rewriter.feed 完全一致（剥 BOM、去首尾空白、# 开头算属性行），
// 因此 url-tvg= / catchup-source= 这些属性里裹着的地址不会被误当成资源行。
//
// 抽出来的是原样地址，既不补全也不包装。要绝对地址，先过一遍 RewritePlaylist：
// 带 TempToken 空值调用时它只做「相对地址补全 + 按 FilterKeywords 丢弃条目块」，
// 两步串起来就是「过滤后仅返回 URL」。
func ExtractURLs(body string) []string {
	if body == "" {
		return nil
	}
	var out []string
	for i := 0; i < len(body); {
		line, delim := splitLine(body, i)
		i += len(line) + len(delim)
		trimmed := strings.TrimRight(strings.TrimPrefix(line, "\ufeff"), " \t\r")
		if trimmed == "" || strings.HasPrefix(trimmed, "#") {
			continue
		}
		out = append(out, strings.TrimSpace(line))
	}
	return out
}

// rewriteResourceLine 处理一条资源行：第三方直连也要包装，凭据才落在本站链接上。
// （属性行相反，见 rewriteLink 的 onlyOwn。）
func rewriteResourceLine(line string, opts RewriteOptions) string {
	return rewriteLink(line, opts, false)
}

// rewriteLink 把一条链接归一化成「本站单跳代理链接」并盖章。
//
// 本站形态（自己发出的 /play?url=）剥出内层重签，反复改写不套娃；其余链接
// 一律按它本来的样子处理——本站只在自己的子链接形态上盖章。
//
// onlyOwn=true 时只处理本站形态与本站链接，纯第三方直连原样返回——用于
// url-tvg / catchup-source 等属性：既不把凭据送给源站，也不把第三方 EPG 平白
// 拖进本站代理（ADR-0003）。
func rewriteLink(uri string, opts RewriteOptions, onlyOwn bool) string {
	uri = strings.TrimSpace(uri)
	if uri == "" {
		return uri
	}

	if inner, ok := parseProxyInner(uri, opts); ok {
		return buildProxyLink(inner, opts)
	}
	if onlyOwn && !isOwnHost(uri, opts) {
		return uri // 纯第三方直连：不动、不盖章
	}

	var target string
	switch {
	case hasHTTPPrefix(uri):
		target = uri
	case strings.HasPrefix(uri, "//"):
		// 协议相对地址（//host/path）：与以 / 开头不同，必须补 scheme。
		// 若按 BaseRoot+uri 拼接会得到 http://host//host/path——上游 host 被改写坏。
		target = schemeOf(opts.BaseRoot) + ":" + uri
	case strings.HasPrefix(uri, "/"):
		target = opts.BaseRoot + uri
	default:
		target = opts.BaseDir + uri
	}
	return buildProxyLink(target, opts)
}

// isOwnHost 判断链接是否指向本站 host。根相对链接（无 host）不算——
// 属性行里的相对路径归 BaseDir 补全，不是入口。
func isOwnHost(uri string, opts RewriteOptions) bool {
	h := urlHost(uri)
	return h != "" && h == opts.SelfHost
}

// parseProxyInner 判定是否「本站形态」并抽出内层地址。
// 本站自己发出的链接是 /play?t=…&url=…，参数顺序不固定，不能用字面量匹配。
//
// 认出它是为了幂等：改写时剥掉旧 t、重盖新 t。没有这一步，本站的子链接会被
// 自己再包一层——链式订阅（拿一份已改写的列表当上游）下会无限套娃。
//
// 只认本站形态。别家的 /play 约定我们无从得知，也不该由我们替他解包：
// 那等于替别人的服务承担跳转与故障。想让某份订阅走本站单跳，让它的链接
// 指向本站入口即可——那是订阅地址该做的事，不是改写器该做的。
//
// 路径必须整段匹配（/play 或 /play/…）：/playlist 这类别的资源不是入口，
// 误判会把它的 url= 当内层地址抽走。
func parseProxyInner(ref string, opts RewriteOptions) (inner string, ok bool) {
	u, err := url.Parse(ref)
	if err != nil || (u.Path != "/play" && !strings.HasPrefix(u.Path, "/play/")) {
		return "", false
	}
	// 绝对形态还得 host 是本站才算本站的。根相对形态（SelfRoot 为空时发出的）
	// 没有 host，按本站处理——它本就出自本站。
	if h := urlHost(ref); h != "" && h != opts.SelfHost {
		return "", false
	}
	inner = u.Query().Get(targetParam)
	if inner == "" {
		return "", false
	}
	return inner, true
}

// urlHost 取链接 host（小写去端口）；无 host（相对链接）返回空串。
func urlHost(ref string) string {
	u, err := url.Parse(ref)
	if err != nil {
		return ""
	}
	return strings.ToLower(u.Hostname())
}

// buildProxyLink 生成 "<入口根>/play?t=<临时>&url=<编码后的绝对地址>"。
// 入口根 opts.SelfRoot 来自请求头 Host（scheme://host[:port]），与上游地址无关——
// 上游列表里的相对路径绝不能拿来当本站路径。SelfRoot 为空时退化为根相对
// /play，返回体不带 host；临时 token 为空时退化为原样绝对地址。
//
// tempParam/targetParam 是 /play 的查询参数名，与服务端 auth.go 的同名常量
// 是同一份契约的两端；playlist 是纯函数包，不该反向依赖 server，故各写一份。
func buildProxyLink(absURL string, opts RewriteOptions) string {
	if opts.TempToken == "" {
		return absURL
	}
	q := url.Values{}
	q.Set(tempParam, opts.TempToken)
	q.Set(targetParam, absURL)
	return opts.SelfRoot + "/play?" + q.Encode()
}

// attrPattern 匹配 # 行里带 URL 的属性：url-tvg / x-tvg-url / catchup-source /
// URI，值可带引号或不带。
var attrPattern = regexp.MustCompile(`(?i)\b(url-tvg|x-tvg-url|catchup-source|URI)\s*=\s*("([^"]*)"|[^\s]+)`)

// rewriteAttrs 改写 # 行里的 URL 属性（含 #EXTM3U 头部行）。
func rewriteAttrs(line string, opts RewriteOptions) string {
	if !strings.Contains(line, "=") {
		return line
	}
	return attrPattern.ReplaceAllStringFunc(line, func(m string) string {
		sub := attrPattern.FindStringSubmatch(m)
		attr, full := sub[1], sub[2]
		quoted := strings.HasPrefix(full, `"`)
		val := full
		if quoted {
			val = full[1 : len(full)-1]
		}
		if val == "" {
			return m
		}
		// catchup-source 含 ${...} 模板：整条不动，模板不能 urlencode。
		if strings.EqualFold(attr, "catchup-source") && strings.Contains(val, "${") {
			return m
		}
		newVal := rewriteAttrValue(attr, val, opts)
		if newVal == val {
			return m
		}
		if quoted {
			return attr + `="` + newVal + `"`
		}
		return attr + "=" + newVal
	})
}

// rewriteAttrValue 改写一条属性值。
//   - URI=（如 #EXT-X-KEY）承载单个资源，与资源行同规则（可被包装）；
//   - url-tvg / x-tvg-url / catchup-source 可能逗号分隔多值，且只改写本站形态，
//     第三方直连原样保留（凭据不落别人域名，ADR-0003）。
func rewriteAttrValue(attr, val string, opts RewriteOptions) string {
	if strings.EqualFold(attr, "URI") {
		return rewriteLink(val, opts, false)
	}
	parts := strings.Split(val, ",")
	for k, one := range parts {
		trimmed := strings.TrimSpace(one)
		if trimmed == "" {
			continue
		}
		parts[k] = rewriteLink(trimmed, opts, true)
	}
	return strings.Join(parts, ",")
}

// hasHTTPPrefix 判断是否 http(s):// 开头（大小写不敏感）。
func hasHTTPPrefix(s string) bool {
	return len(s) >= 7 && strings.EqualFold(s[:7], "http://") ||
		len(s) >= 8 && strings.EqualFold(s[:8], "https://")
}

// schemeOf 取 "scheme://host" 的 scheme；无分隔符时兜底 "http"。
// 仅供协议相对地址（//host）补 scheme，不改动其它拼接。
func schemeOf(baseRoot string) string {
	if i := strings.Index(baseRoot, "://"); i > 0 {
		return baseRoot[:i]
	}
	return "http"
}

// isPlaylistHeader 判断是否 #EXTM3U 头部行。
func isPlaylistHeader(trimmed string) bool {
	return strings.HasPrefix(strings.ToUpper(trimmed), "#EXTM3U")
}

// entryMatches 判断一个条目块是否命中任一关键字（子串、大小写不敏感）。
func entryMatches(blockLines, kws []string) bool {
	if len(kws) == 0 {
		return true
	}
	for _, line := range blockLines {
		if !strings.HasPrefix(strings.ToUpper(line), "#EXTINF") {
			continue
		}
		for _, cand := range entryNames(line) {
			for _, kw := range kws {
				if containsFold(cand, kw) {
					return true
				}
			}
		}
	}
	return false
}

// entryNames 取条目名：显示名 + tvg-name。
// 显示名在「最后一个引号之后的首个逗号」之后；整行无引号则取首个逗号；
// 连逗号都没有（畸形行）则退回整行——过滤词仍可能命中属性区（如 tvg-id）。
// 不能直接取最后一个逗号——显示名自身可能含逗号（如「翡翠台,高清」）。
func entryNames(extinf string) []string {
	names := make([]string, 0, 2)
	q := strings.LastIndex(extinf, `"`)
	comma := -1
	if q < 0 {
		comma = strings.Index(extinf, ",")
	} else if c := strings.Index(extinf[q:], ","); c >= 0 {
		comma = q + c
	}
	if comma >= 0 {
		names = append(names, strings.TrimSpace(extinf[comma+1:]))
	} else {
		names = append(names, strings.TrimSpace(extinf))
	}
	names = append(names, extractTvgNames(extinf)...)
	return names
}

// tvgNamePattern 匹配 tvg-name：优先带引号（去掉引号），否则取无空白无逗号的裸值。
var tvgNamePattern = regexp.MustCompile(`(?i)tvg-name\s*=\s*"([^"]*)"`)

// tvgNameBarePattern 是 tvg-name 无引号时的兜底：值不含引号/空白/逗号。
var tvgNameBarePattern = regexp.MustCompile(`(?i)tvg-name\s*=\s*([^"\s,]+)`)

// extractTvgNames 抽出 tvg-name 属性值；带引号的整行优先，且不重复匹配裸值。
func extractTvgNames(line string) []string {
	if ms := tvgNamePattern.FindAllStringSubmatch(line, -1); len(ms) > 0 {
		return collectSubmatches(ms)
	}
	if line == "" {
		return nil
	}
	return collectSubmatches(tvgNameBarePattern.FindAllStringSubmatch(line, -1))
}

// collectSubmatches 取每组捕获的第 1 子串（即属性值本身）。
func collectSubmatches(ms [][]string) []string {
	out := make([]string, 0, len(ms))
	for _, m := range ms {
		out = append(out, m[1])
	}
	return out
}

// truncUTF8 把字符串截到不超过 n 字节，且不切断最后一个多字节字符。
// 过滤词多为中文，按字节硬切会留下半个 UTF-8 字符——既匹配不上，也可能台名乱码。
func truncUTF8(s string, n int) string {
	if len(s) <= n {
		return s
	}
	b := []byte(s[:n])
	// 逐字节回退，直到剩下的是合法 UTF-8（且不留悬空的序列首字节）。
	for len(b) > 0 && !utf8.Valid(b) {
		b = b[:len(b)-1]
	}
	return string(b)
}

// containsFold 大小写不敏感的子串判断（ASCII 折叠；CJK 字节不受影响）。
func containsFold(s, substr string) bool {
	if substr == "" {
		return true
	}
	return strings.Contains(strings.ToLower(s), strings.ToLower(substr))
}

// ParseFilterKeywords 解析 filter= 的值：英文/全角逗号分隔，
// 最多 20 个、每个至多 64 字节，去重且保持插入顺序（设计 §2.6）。
func ParseFilterKeywords(raw string) []string {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return nil
	}
	raw = strings.ReplaceAll(raw, "，", ",")
	seen := map[string]bool{}
	var out []string
	for _, kw := range strings.Split(raw, ",") {
		kw = strings.TrimSpace(kw)
		if kw == "" {
			continue
		}
		if len(kw) > 64 {
			kw = truncUTF8(kw, 64)
		}
		if seen[kw] {
			continue
		}
		seen[kw] = true
		out = append(out, kw)
		if len(out) >= 20 {
			break
		}
	}
	return out
}
