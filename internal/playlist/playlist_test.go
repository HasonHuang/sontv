package playlist

import (
	"strings"
	"testing"
	"unicode/utf8"
)

// optsFor 造一份改写选项，便于断言。
//   - SelfHost：本站 host，判定「本站入口形态」
//   - SelfRoot：本站入口根，子链接的绝对前缀
func optsFor(kw []string) RewriteOptions {
	return RewriteOptions{
		FilterKeywords: kw,
		TempToken:      "EXP.UID.SIG",
		BaseRoot:       "https://up.example.com",
		BaseDir:        "https://up.example.com/live/",
		SelfHost:       "self.example.com",
		SelfRoot:       "https://self.example.com",
	}
}

func TestRewriteUnwrapsProxyForm(t *testing.T) {
	// 已是本站形态：剥内层重盖，跑两次结果一致（幂等）。
	body := "#EXTM3U\n#EXTINF:-1,A\n/play?t=OLD&url=http%3A%2F%2Fcdn%2Fa.ts\n"
	once := RewritePlaylist(body, optsFor(nil))
	twice := RewritePlaylist(once, optsFor(nil))
	if once != twice {
		t.Fatalf("非幂等:\n1 %q\n2 %q", once, twice)
	}
	if !strings.Contains(once, "t=EXP.UID.SIG") || !strings.Contains(once, "url=http%3A%2F%2Fcdn%2Fa.ts") {
		t.Fatalf("未重签: %s", once)
	}
}

// 绝对形态的本站链接（host 与 SelfHost 一致）同样要认出，
// 否则它会被当成普通直连再包一层，链式订阅下就套娃了。
func TestRewriteAbsoluteSelfLinkResigned(t *testing.T) {
	body := "#EXTM3U\n#EXTINF:-1,A\nhttps://self.example.com:8443/play?t=OLD&url=http%3A%2F%2Fcdn%2Fa.ts\n"
	got := RewritePlaylist(body, optsFor(nil))
	if strings.Contains(got, "OLD") {
		t.Fatalf("旧凭据未剥掉: %s", got)
	}
	if !strings.Contains(got, "url=http%3A%2F%2Fcdn%2Fa.ts") {
		t.Fatalf("本站绝对链接未重签: %s", got)
	}
}

// 改名前的旧形态 /url?u=… 不再是本站形态：只当普通相对路径处理。
func TestRewriteLegacyProxyFormTreatedAsResource(t *testing.T) {
	body := "#EXTM3U\n#EXTINF:-1,A\n/url?u=http%3A%2F%2Fcdn%2Fa.ts\n"
	got := RewritePlaylist(body, optsFor(nil))
	if !strings.Contains(got, "url=https%3A%2F%2Fup.example.com%2Furl%3Fu%3Dhttp") {
		t.Fatalf("/url 应被当普通相对路径: %s", got)
	}
}

// 别站用同样的 /play 形态也不是本站的：不认它，只当普通绝对地址包一层。
// 刻意不替别家解包——那是替别人的服务承担跳转与故障。
func TestRewriteForeignProxyFormNotUnwrapped(t *testing.T) {
	body := "#EXTM3U\n#EXTINF:-1,A\nhttps://other.example/play?url=http%3A%2F%2Fcdn%2Fa.ts\n"
	got := RewritePlaylist(body, optsFor(nil))
	// 整条别站链接作为目标被包进去（值里能看到它自己的 /play?url=）
	if !strings.Contains(got, "url=https%3A%2F%2Fother.example%2Fplay%3Furl%3Dhttp") {
		t.Fatalf("别站链接应被整体包一层: %s", got)
	}
	// 但凭据照样盖上本站的——经过本就该由本站发临时 token
	if !strings.Contains(got, "t=EXP.UID.SIG") {
		t.Fatalf("别站链接仍应盖本站临时 token: %s", got)
	}
}

func TestAttrThirdPartyDirectUntouched(t *testing.T) {
	// url-tvg 指向纯第三方直连：属性行只改本站形态，第三方原样保留。
	body := "#EXTM3U url-tvg=\"http://epg.third.com/tv.xml\"\n#EXTINF:-1,A\nhttp://cdn/a.ts\n"
	got := RewritePlaylist(body, optsFor(nil))
	if !strings.Contains(got, `url-tvg="http://epg.third.com/tv.xml"`) {
		t.Fatalf("第三方 url-tvg 被改动: %s", got)
	}
}

func TestAttrOwnHostRewritten(t *testing.T) {
	// url-tvg 指向本站 host：本站形态，应被包装敲章（绝对地址带本站入口根）。
	body := "#EXTM3U url-tvg=\"http://self.example.com/tv.xml\"\n#EXTINF:-1,A\nhttp://cdn/a.ts\n"
	got := RewritePlaylist(body, optsFor(nil))
	if !strings.Contains(got, "url-tvg=\"https://self.example.com/play?") {
		t.Fatalf("本站 url-tvg 未被包装: %s", got)
	}
}

func TestResourceLineThirdPartyWrapped(t *testing.T) {
	// 资源行的第三方直连也要包装（凭据落在我们的链接上）。
	body := "#EXTM3U\n#EXTINF:-1,A\nhttp://cdn/a.ts\n"
	got := RewritePlaylist(body, optsFor(nil))
	if !strings.Contains(got, "url=http%3A%2F%2Fcdn%2Fa.ts") {
		t.Fatalf("资源行第三方未包装: %s", got)
	}
}

func TestRewritePlaylistNoTempTokenIsIdentity(t *testing.T) {
	// TempToken 为空时子链接退化为「原样绝对地址」，属性/资源行都不新增内容。
	body := "#EXTM3U\n#EXTINF:-1,频道\nhttp://x/a.ts\n"
	opts := RewriteOptions{BaseRoot: "https://up", BaseDir: "https://up/"}
	got := RewritePlaylist(body, opts)
	want := "#EXTM3U\n#EXTINF:-1,频道\nhttp://x/a.ts\n"
	if got != want {
		t.Fatalf("无 token 时应原样：\n got %q\nwant %q", got, want)
	}
}

func TestRewriteResourceLinesWrapped(t *testing.T) {
	body := "#EXTM3U\n" +
		"#EXTINF:-1,A\nhttp://cdn/a.ts\n" +
		"#EXTINF:-1,B\n/sub/b.ts\n" +
		"#EXTINF:-1,C\nc.ts\n"
	got := RewritePlaylist(body, optsFor(nil))

	// 本站形态永远回本站 /play，绝对地址带本站入口根，凭据带 t= 与 url=（url 被编码）
	if !strings.Contains(got, "https://self.example.com/play?t=EXP.UID.SIG&url=http%3A%2F%2Fcdn%2Fa.ts") {
		t.Fatalf("绝对资源未包装: %s", got)
	}
	if !strings.Contains(got, "url=https%3A%2F%2Fup.example.com%2Fsub%2Fb.ts") {
		t.Fatalf("根相对路径未补全: %s", got)
	}
	if !strings.Contains(got, "url=https%3A%2F%2Fup.example.com%2Flive%2Fc.ts") {
		t.Fatalf("目录相对路径未补全: %s", got)
	}
}

func TestRewritePreservesCRLF(t *testing.T) {
	body := "#EXTM3U\r\n#EXTINF:-1,A\r\nhttp://cdn/a.ts\r\n"
	got := RewritePlaylist(body, optsFor(nil))
	if !strings.Contains(got, "#EXTM3U\r\n") || !strings.Contains(got, "\r\n") {
		t.Fatalf("CRLF 未保留: %q", got)
	}
}

func TestRewriteIdempotentSelfProxy(t *testing.T) {
	// 已是本站 /play 形态：剥旧凭据重盖，跑两次结果一致。
	once := RewritePlaylist("#EXTM3U\n#EXTINF:-1,A\n/play?t=OLD&url=http%3A%2F%2Fcdn%2Fa.ts\n", optsFor(nil))
	twice := RewritePlaylist(once, optsFor(nil))
	if once != twice {
		t.Fatalf("非幂等:\n1 %q\n2 %q", once, twice)
	}
	if !strings.Contains(once, "t=EXP.UID.SIG") {
		t.Fatalf("未重盖新 token: %s", once)
	}
}

func TestRewriteAttrsOnlySelfForms(t *testing.T) {
	body := "#EXTM3U url-tvg=\"http://epg.example.com/tv.xml\"\n" +
		"#EXTINF:-1 tvg-name=\"A\",A\nhttp://cdn/a.ts\n"
	got := RewritePlaylist(body, optsFor(nil))

	// 第三方 EPG 直连：绝不能被包装（凭据不落别人域名）
	if !strings.Contains(got, `url-tvg="http://epg.example.com/tv.xml"`) {
		t.Fatalf("第三方属性被改动了: %s", got)
	}
}

func TestRewriteCatchupTemplateUntouched(t *testing.T) {
	body := "#EXTM3U\n" +
		"#EXTINF:-1 catchup-source=\"http://cdn/timeshift?start=${start}\",A\n" +
		"/play?url=http%3A%2F%2Fcdn%2Fx.ts\n"
	got := RewritePlaylist(body, optsFor(nil))
	if !strings.Contains(got, "${start}") || !strings.Contains(got, "catchup-source=\"http://cdn/timeshift?start=${start}\"") {
		t.Fatalf("模板被改动: %s", got)
	}
}

func TestRewriteURIAttrWrapped(t *testing.T) {
	// URL= / URI= 承载单个资源（如 EXT-X-KEY），与资源行同规则。
	body := "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"http://cdn/k\"\n#EXTINF:-1,A\nhttp://cdn/a.ts\n"
	got := RewritePlaylist(body, optsFor(nil))
	if !strings.Contains(got, "URI=\"https://self.example.com/play?") {
		t.Fatalf("URI 未包装: %s", got)
	}
}

// SelfRoot 为空时（纯函数/无入口场景）子链接退化为根相对，不带 host。
func TestBuildProxyLinkNoSelfRootIsRelative(t *testing.T) {
	opts := RewriteOptions{TempToken: "EXP.UID.SIG"}
	if got := buildProxyLink("http://cdn/a.ts", opts); got != "/play?t=EXP.UID.SIG&url=http%3A%2F%2Fcdn%2Fa.ts" {
		t.Fatalf("无 SelfRoot 应根相对: %q", got)
	}
}

// SelfRoot 非空时子链接是绝对地址，带本站入口根（含端口）。
func TestBuildProxyLinkWithSelfRootIsAbsolute(t *testing.T) {
	opts := RewriteOptions{TempToken: "EXP.UID.SIG", SelfRoot: "https://self.example.com:8443"}
	want := "https://self.example.com:8443/play?t=EXP.UID.SIG&url=http%3A%2F%2Fcdn%2Fa.ts"
	if got := buildProxyLink("http://cdn/a.ts", opts); got != want {
		t.Fatalf("带 SelfRoot 应为绝对地址:\n got %q\nwant %q", got, want)
	}
}

func TestFilterDropsNonMatchingBlocks(t *testing.T) {
	body := "#EXTM3U\n" +
		"#EXTINF:-1,翡翠台\nhttp://cdn/a.ts\n" +
		"#EXTINF:-1,新闻台\nhttp://cdn/b.ts\n"
	got := RewritePlaylist(body, optsFor([]string{"翡翠"}))

	if !strings.Contains(got, "a.ts") {
		t.Fatalf("命中块应保留: %s", got)
	}
	if strings.Contains(got, "b.ts") {
		t.Fatalf("未命中块应删除: %s", got)
	}
	// 过滤时头尾部注释即使无 URL 也不该留下未匹配块
	if !strings.Contains(got, "#EXTM3U") {
		t.Fatalf("表头必须保留: %s", got)
	}
}

func TestFilterMatchesTvgName(t *testing.T) {
	body := "#EXTM3U\n" +
		"#EXTINF:-1 tvg-name=\"CCTV1\" other=\"x\",央视一套\nhttp://cdn/a.ts\n"
	got := RewritePlaylist(body, optsFor([]string{"cctv1"}))
	if !strings.Contains(got, "a.ts") {
		t.Fatalf("应命中 tvg-name（大小写不敏感）: %s", got)
	}
}

func TestFilterDisplayNameAfterLastQuote(t *testing.T) {
	// 显示名含引号/逗号：取最后一个引号后的首个逗号之后
	body := "#EXTM3U\n" +
		"#EXTINF:-1 tvg-id=\"x\" tvg-name=\"\",翡翠台\nhttp://cdn/a.ts\n"
	got := RewritePlaylist(body, optsFor([]string{"翡翠"}))
	if !strings.Contains(got, "a.ts") {
		t.Fatalf("显示名匹配失败: %s", got)
	}
}

func TestParseFilterKeywords(t *testing.T) {
	got := ParseFilterKeywords(" 翡翠 ，翡翠, 新闻 ,")
	if len(got) != 2 || got[0] != "翡翠" || got[1] != "新闻" {
		t.Fatalf("去重/全角逗号/trim 失败: %#v", got)
	}
	if ParseFilterKeywords("   ") != nil {
		t.Fatalf("空白应返回 nil")
	}
	// 上限 20 个
	var many []string
	for i := 0; i < 30; i++ {
		many = append(many, string(rune('a'+i)))
	}
	if n := len(ParseFilterKeywords(strings.Join(many, ","))); n != 20 {
		t.Fatalf("关键词上限应为 20，实际 %d", n)
	}
	// 每个至多 64 字节，且不切断多字节字符（中文 3 字节 → 落到 63）
	long := ParseFilterKeywords(strings.Repeat("长", 100))
	if len(long) != 1 || len(long[0]) > 64 || !utf8.ValidString(long[0]) {
		t.Fatalf("单关键词应截断到 ≤64 字节且合法 UTF-8: %d %q", len(long[0]), long[0])
	}
}

func TestRewriteEntryFixtureInOut(t *testing.T) {
	// 端到端：一份含注释/属性/相对/绝对/空行的列表，逐条核对关键输出。
	body := "#EXTM3U\n" +
		"#EXTINF:-1 tvg-name=\"A\",甲\n" +
		"http://cdn/a.ts\n" +
		"\n" +
		"#EXTINF:-1,乙\n" +
		"/live/b.m3u8\n"
	got := RewritePlaylist(body, optsFor(nil))

	wants := []string{
		"#EXTM3U\n",
		`tvg-name="A"`, // 属性不因资源行而消失
		"url=http%3A%2F%2Fcdn%2Fa.ts",
		"url=https%3A%2F%2Fup.example.com%2Flive%2Fb.m3u8",
		"\n\n", // 空行保留
	}
	for _, w := range wants {
		if !strings.Contains(got, w) {
			t.Fatalf("缺少 %q\n实际: %q", w, got)
		}
	}
}

// ---------- 复审修复的回归 ----------

func TestRewriteBOMHeaderNotMangled(t *testing.T) {
	// 带 BOM 的 #EXTM3U：必须先剥 BOM 再判类型，否则整行被当资源拼到 BaseDir 上。
	body := "\ufeff#EXTM3U\n#EXTINF:-1,A\nhttp://cdn/a.ts\n"
	got := RewritePlaylist(body, optsFor(nil))
	if !strings.Contains(got, "\ufeff#EXTM3U") {
		t.Fatalf("BOM 头行被破坏: %q", got)
	}
	if strings.Contains(got, "url=https%3A%2F%2Fup.example.com%2F%EF%BB%BF") {
		t.Fatalf("BOM 头行被当资源行改写: %q", got)
	}
}

func TestRewriteProtocolRelative(t *testing.T) {
	// 协议相对地址 //host/path 必须补 scheme，不能拼成 http://up//host/path。
	body := "#EXTM3U\n#EXTINF:-1,A\n//other.example/a.ts\n"
	got := RewritePlaylist(body, optsFor(nil))
	if strings.Contains(got, "%2Fup%2F%2Fother") {
		t.Fatalf("host 被改写坏（应补 scheme 而非拼接）: %s", got)
	}
	if !strings.Contains(got, "url=https%3A%2F%2Fother.example%2Fa.ts") {
		t.Fatalf("协议相对未补 scheme: %s", got)
	}
}

func TestRewriteProxyPathBoundary(t *testing.T) {
	// /playlist 不是代理端点：不能被当作 /play 抽 url=。它应按普通资源行包装。
	body := "#EXTM3U\n#EXTINF:-1,A\n/playlist?url=http%3A%2F%2Fcdn%2Fa.ts\n"
	got := RewritePlaylist(body, optsFor(nil))
	if !strings.Contains(got, "url=https%3A%2F%2Fup.example.com%2Fplaylist%3Furl%3Dhttp") {
		t.Fatalf("/playlist 应被当普通相对路径: %s", got)
	}
}

func TestRewriteDisplayNameWithQuote(t *testing.T) {
	// 显示名含引号：取「最后一个引号后的首个逗号」之后。
	body := "#EXTM3U\n" +
		"#EXTINF:-1 tvg-name=\"X\" grp=\"G\",频道\"高清\"台\nhttp://cdn/a.ts\n"
	got := RewritePlaylist(body, optsFor([]string{"高清"}))
	if !strings.Contains(got, "a.ts") {
		t.Fatalf("含引号显示名过滤失败: %s", got)
	}
}

func TestTvgNameQuotesStripped(t *testing.T) {
	// tvg-name 捕获值不带引号，否则关键字会带引号匹配不上。
	names := extractTvgNames(`#EXTINF:-1 tvg-name="CCTV1",x`)
	if len(names) != 1 || names[0] != "CCTV1" {
		t.Fatalf("tvg-name 引号未剥: %#v", names)
	}
}

func TestEntryNamesMalformedFallsBackToWholeLine(t *testing.T) {
	// 无逗号的畸形行：退回整行，过滤词仍可命中属性区。
	names := entryNames(`#EXTINF:-1 tvg-id=foo`)
	if len(names) == 0 || !strings.Contains(names[0], "tvg-id=foo") {
		t.Fatalf("畸形行应退回整行: %#v", names)
	}
}

func TestTruncUTF8NeverSplitsRune(t *testing.T) {
	// 100 个中文（300 字节）截到 64：必须是合法 UTF-8，且长度 ≤64。
	got := ParseFilterKeywords(strings.Repeat("长", 100))
	if len(got) != 1 {
		t.Fatalf("应得 1 个关键词: %#v", got)
	}
	if len(got[0]) > 64 || !utf8.ValidString(got[0]) {
		t.Fatalf("截断破坏了 UTF-8: %d %q", len(got[0]), got[0])
	}
}

// ---------- ExtractURLs ----------

// 属性行里裹着的地址不是资源行：url-tvg / catchup-source 指向的是 EPG 与回看，
// 混进「频道地址清单」就是错数据。URI= 同理（它承载分片，属 #EXT-X-KEY 行内）。
func TestExtractURLsSkipsAttrAddresses(t *testing.T) {
	body := "#EXTM3U\n" +
		"#EXTINF:-1 tvg-name=\"A\",A\n" +
		"http://cdn/a.ts\n" +
		"#EXTINF:-1 url-tvg=\"http://epg.example.com/a.xml\" tvg-name=\"B\",B\n" +
		"http://cdn/b.ts\n" +
		"#EXTINF:-1 catchup-source=http://cdn/catchup?x={YYYY},C\n" +
		"http://cdn/c.ts\n"
	got := ExtractURLs(body)
	want := []string{"http://cdn/a.ts", "http://cdn/b.ts", "http://cdn/c.ts"}
	if len(got) != len(want) {
		t.Fatalf("得到 %d 条，期望 %d: %#v", len(got), len(want), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("第 %d 条 = %q，期望 %q", i, got[i], want[i])
		}
	}
}

// 混合换行、末尾无分隔符、空行与 BOM 都不能让地址行漏掉或多出空串。
func TestExtractURLsMixedDelimsAndBOM(t *testing.T) {
	body := "\ufeff" + "#EXTM3U\r\n#EXTINF:-1,A\r\na.ts\n\n#EXTINF:-1,B\n/b.ts" // BOM 不能直接写在源码里
	got := ExtractURLs(body)
	if len(got) != 2 || got[0] != "a.ts" || got[1] != "/b.ts" {
		t.Fatalf("得到 %#v，期望 [a.ts /b.ts]", got)
	}
}

// 过滤后仅返回 URL：TempToken 留空时 RewritePlaylist 只补全地址、不包装，
// 串上 ExtractURLs 就是两步。
func TestFilterThenExtractGivesPlainURLs(t *testing.T) {
	body := "#EXTM3U\n" +
		"#EXTINF:-1 tvg-name=\"翡翠台\",翡翠台\n" +
		"翡翠.ts\n" +
		"#EXTINF:-1 tvg-name=\"其它\",其它\n" +
		"http://cdn/other.ts\n"
	opts := RewriteOptions{
		FilterKeywords: []string{"翡翠"},
		BaseRoot:       "https://up.example.com",
		BaseDir:        "https://up.example.com/live/",
	}
	got := ExtractURLs(RewritePlaylist(body, opts))
	if len(got) != 1 {
		t.Fatalf("得到 %#v，期望只剩 1 条", got)
	}
	if got[0] != "https://up.example.com/live/翡翠.ts" {
		t.Fatalf("相对地址应按 BaseDir 补成绝对: %q", got[0])
	}
	if strings.Contains(got[0], "/play?") {
		t.Fatalf("TempToken 为空时不该包装成本站链接: %q", got[0])
	}
}

func TestExtractURLsEmptyBody(t *testing.T) {
	if got := ExtractURLs(""); got != nil {
		t.Fatalf("空正文应返回 nil，得到 %#v", got)
	}
}
