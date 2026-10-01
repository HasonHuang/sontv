package config

import (
	"os"
	"path/filepath"
	"strconv"
	"testing"
)

// TestDefaultConfig 钉住缺省：改任一常量都会在此处被拦下，提醒同步 README。
func TestDefaultConfig(t *testing.T) {
	c := DefaultConfig()
	if c.TokensFile != "/opt/sontv/tokens.txt" || c.DefaultTTLHours != 24 {
		t.Fatalf("凭据缺省不符: %+v", c)
	}
	if c.UpstreamM3U != "https://cdn.qd.je/mytv0.m3u" || c.Listen != "127.0.0.1:9900" {
		t.Fatalf("上游/监听缺省不符: %+v", c)
	}
	if !c.UnwrapRemoteProxy {
		t.Fatalf("缺省应解包远端代理")
	}
}

// TestLoadConfigEmptyPath 空路径不报错、直接给缺省——便于 -config 留空跑测试。
func TestLoadConfigEmptyPath(t *testing.T) {
	c, err := LoadConfig("")
	if err != nil {
		t.Fatalf("空路径应成功: %v", err)
	}
	if c.TokensFile != DefaultConfig().TokensFile {
		t.Fatalf("空路径应返回缺省: %+v", c)
	}
}

// TestLoadConfigPartialKeepsDefaults 关键契约：JSON 是覆盖式合并，
// 只写一个字段绝不能把其余字段清零（否则一份最小 config 会悄悄改掉上游）。
func TestLoadConfigPartialKeepsDefaults(t *testing.T) {
	// 只写 listen：其余字段必须保持缺省（json 覆盖式合并）
	dir := t.TempDir()
	path := filepath.Join(dir, "c.json")
	if err := os.WriteFile(path, []byte(`{"listen":":9000"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	c, err := LoadConfig(path)
	if err != nil {
		t.Fatalf("解析失败: %v", err)
	}
	if c.Listen != ":9000" {
		t.Fatalf("listen 未生效: %q", c.Listen)
	}
	if c.UpstreamM3U != DefaultConfig().UpstreamM3U {
		t.Fatalf("未写的上游被清空: %q", c.UpstreamM3U)
	}
}

// TestLoadConfigSanitizes 病态值（0/负 TTL、空 listen）静默改回缺省：
// 可用性优先——一份手滑的 config 不该让服务起不来。
func TestLoadConfigSanitizes(t *testing.T) {
	// 0/负 TTL 与空 listen 会被静默改写为可用缺省（可用性优先）
	dir := t.TempDir()
	path := filepath.Join(dir, "c.json")
	if err := os.WriteFile(path, []byte(`{"default_ttl_hours":-5,"listen":""}`), 0o600); err != nil {
		t.Fatal(err)
	}
	c, err := LoadConfig(path)
	if err != nil {
		t.Fatalf("解析失败: %v", err)
	}
	if c.DefaultTTLHours != 24 {
		t.Fatalf("负 TTL 应回落 24，实际 %d", c.DefaultTTLHours)
	}
	if c.Listen != "127.0.0.1:9900" {
		t.Fatalf("空 listen 应回落缺省，实际 %q", c.Listen)
	}
}

// ---------- 配置路径解析 ----------

// TestResolveConfigPathEmpty 空串原样返回，调用方据此走全缺省。
func TestResolveConfigPathEmpty(t *testing.T) {
	got, err := ResolveConfigPath("")
	if err != nil || got != "" {
		t.Fatalf("空串应原样返回，实际 %q, %v", got, err)
	}
}

// TestResolveConfigPathAbsolute 绝对路径原样透传，不做任何改写。
func TestResolveConfigPathAbsolute(t *testing.T) {
	abs := filepath.Join(t.TempDir(), "c.json")
	got, err := ResolveConfigPath(abs)
	if err != nil {
		t.Fatal(err)
	}
	if got != abs {
		t.Fatalf("绝对路径应原样返回: %q", got)
	}
}

// TestResolveConfigPathRelativeToBinary 关键契约：相对路径按**二进制同级目录**
// 解析，而不是工作目录——否则 systemd/容器里换个 cwd 就读不到配置了。
// 测试自身是二进制（go test 编译出的 test 程序），所以它的目录就是可预期的基准。
func TestResolveConfigPathRelativeToBinary(t *testing.T) {
	exe, err := os.Executable()
	if err != nil {
		t.Skipf("拿不到可执行文件路径: %v", err)
	}
	if resolved, err := filepath.EvalSymlinks(exe); err == nil {
		exe = resolved
	}
	got, err := ResolveConfigPath(DefaultConfigName)
	if err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(filepath.Dir(exe), DefaultConfigName)
	if got != want {
		t.Fatalf("相对路径应基于二进制同级目录:\n  得到 %q\n  期望 %q", got, want)
	}
}

// TestResolveConfigPathNestedRelative 相对路径里的子目录同样以二进制目录为基准。
func TestResolveConfigPathNestedRelative(t *testing.T) {
	got, err := ResolveConfigPath(filepath.Join("etc", "sontv.json"))
	if err != nil {
		t.Fatal(err)
	}
	if !filepath.IsAbs(got) {
		t.Fatalf("结果应为绝对路径，实际 %q", got)
	}
	if filepath.Base(filepath.Dir(got)) != "etc" {
		t.Fatalf("子目录应保留: %q", got)
	}
}

// TestTokensFileRelative 相对 tokens_file 以配置文件同级目录为基准补全，
// 绝对路径原样透传。
func TestTokensFileRelative(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "c.json")
	if err := os.WriteFile(path, []byte(`{"tokens_file":"tokens.txt"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	c, err := LoadConfig(path)
	if err != nil {
		t.Fatal(err)
	}
	if want := filepath.Join(dir, "tokens.txt"); c.TokensFile != want {
		t.Fatalf("相对 tokens_file 应基于配置文件目录:\n  得到 %q\n  期望 %q", c.TokensFile, want)
	}

	// 绝对路径不动
	abs := filepath.Join(dir, "t.txt")
	if err := os.WriteFile(path, []byte(`{"tokens_file":`+strconv.Quote(abs)+`}`), 0o600); err != nil {
		t.Fatal(err)
	}
	c, err = LoadConfig(path)
	if err != nil {
		t.Fatal(err)
	}
	if c.TokensFile != abs {
		t.Fatalf("绝对 tokens_file 应原样保留: %q", c.TokensFile)
	}
}

// TestTokensFileDefaultIsAbsolute 缺省 tokens_file 本来就是绝对路径，不该被改写。
func TestTokensFileDefaultIsAbsolute(t *testing.T) {
	if !filepath.IsAbs(DefaultConfig().TokensFile) {
		t.Fatalf("缺省 tokens_file 应为绝对路径: %q", DefaultConfig().TokensFile)
	}
}

// TestLoadConfigErrors 与可用性相反的一面：文件缺失/坏 JSON 必须响亮失败，
// 不能悄悄用缺省顶替一份读不到的配置。
func TestLoadConfigErrors(t *testing.T) {
	if _, err := LoadConfig(filepath.Join(t.TempDir(), "nope.json")); err == nil {
		t.Fatalf("文件不存在应报错")
	}
	dir := t.TempDir()
	bad := filepath.Join(dir, "bad.json")
	if err := os.WriteFile(bad, []byte(`{not json`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := LoadConfig(bad); err == nil {
		t.Fatalf("坏 JSON 应报错")
	}
}
