package tokens

import (
	"crypto/sha256"
	"encoding/hex"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// hexOf 返回 token 的 sha256 十六进制，供构造合法表用。
func hexOf(token string) string {
	sum := sha256.Sum256([]byte(token))
	return hex.EncodeToString(sum[:])
}

// TestParseTokenTableValid 一次过钉住解析的三个承诺：注释行被跳过、
// 字段两端空白被容忍、第三列 TTL 生效，并且明文 token 能经 sha256 反查到行。
func TestParseTokenTableValid(t *testing.T) {
	h1, h2 := hexOf("tok1"), hexOf("tok2")
	data := strings.Join([]string{
		"# 这是表头注释",
		"客厅电视," + h1,
		" 手机 , " + h2 + " , 6", // 带空格与 TTL
		"",
	}, "\n")

	snap, err := ParseTokenTable([]byte(data))
	if err != nil {
		t.Fatalf("解析失败: %v", err)
	}
	if len(snap.Rows) != 2 {
		t.Fatalf("行数 = %d，期望 2", len(snap.Rows))
	}
	if r := snap.LookupToken("tok1"); r == nil || r.Label != "客厅电视" {
		t.Fatalf("tok1 查不到或标签错: %+v", r)
	}
	if r := snap.LookupToken("tok2"); r == nil || r.TTLHours != 6 {
		t.Fatalf("tok2 TTL 应为 6: %+v", r)
	}
	if r := snap.LookupUID(hexOf("tok1")[:UIDLen]); r == nil || r.Label != "客厅电视" {
		t.Fatalf("按 uid 查不到")
	}
	if snap.Empty() {
		t.Fatalf("非空表 Empty 应为 false")
	}
}

// TestParseTokenTableInlineComment 钉住注释边界这条易踩的线：
// 只有「空白 + #」才是行内注释；label 里的裸 #（如 a#b）属于标签本身，
// 若误判就会把整行的哈希当注释吃掉——误删一个用户。
func TestParseTokenTableInlineComment(t *testing.T) {
	h := hexOf("tok")
	// 行内注释：label 后「空白 + #」起，到行尾都是注释，哈希必须在 # 之前
	if _, err := ParseTokenTable([]byte("标签 #注释," + h + "\n")); err == nil {
		t.Fatalf("哈希落在注释里应报错")
	}
	snap, err := ParseTokenTable([]byte("标签," + h + " # 往后是注释\n"))
	if err != nil {
		t.Fatalf("解析失败: %v", err)
	}
	if len(snap.Rows) != 1 || snap.Rows[0].Label != "标签" {
		t.Fatalf("行内注释未截断: %+v", snap.Rows)
	}
	// label 里的裸 # 不算注释，整行照常解析
	snap2, err := ParseTokenTable([]byte("a#b," + h + "\n"))
	if err != nil {
		t.Fatalf("label 里的裸 # 不该被当注释: %v", err)
	}
	if len(snap2.Rows) != 1 {
		t.Fatalf("裸 # 行解析错误: %+v", snap2.Rows)
	}
}

// TestParseTokenTableErrors 是「整表拒绝」契约：任一行格式错都要让整张表
// 失败——宁可 503，也绝不接受半张表（漏读一行 = 误删一个用户）。
func TestParseTokenTableErrors(t *testing.T) {
	valid := hexOf("a")
	dup := valid
	cases := []struct {
		name string
		body string
	}{
		{"缺列", "客厅电视\n"},
		{"哈希非64位", "客厅电视,abc123\n"},
		{"哈希含非法字符", "客厅电视," + strings.Repeat("g", 64) + "\n"},
		{"TTL为零", "客厅电视," + valid + ",0\n"},
		{"TTL超上限", "客厅电视," + valid + ",999999999\n"},
		{"TTL非数字", "客厅电视," + valid + ",x\n"},
		{"哈希重复", "客厅电视," + dup + "\n卧室," + valid + "\n"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if _, err := ParseTokenTable([]byte(c.body)); err == nil {
				t.Fatalf("期望报错，实际通过")
			}
		})
	}
}

// TestParseTokenTableEmpty 与「解析出错」区分开：只有注释和空行的表是
// 合法的，只是有效行数为零——它导致 503，而非解析失败。
func TestParseTokenTableEmpty(t *testing.T) {
	snap, err := ParseTokenTable([]byte("# 只有注释\n\n"))
	if err != nil {
		t.Fatalf("解析失败: %v", err)
	}
	if !snap.Empty() {
		t.Fatalf("空表 Empty 应为 true")
	}
	if snap.LookupToken("x") != nil {
		t.Fatalf("空表不该查到")
	}
}

// TestNilSnapshotSafe 覆盖「启动即失败」的路径：尚未装载任何表时，
// 快照为 nil 也必须安全地查不到、判为空——而不是 panic。
func TestNilSnapshotSafe(t *testing.T) {
	var s *Snapshot
	if s.LookupToken("a") != nil || s.LookupUID("a") != nil || !s.Empty() {
		t.Fatalf("nil 快照应安全返回空")
	}
}

// TestTokenTableReloadKeepsOldOnFailure 钉住热重载最危险的一刻：
// 坏表重载必须失败且保留旧表——绝不能把一张读不懂的表换成「人人放行」。
func TestTokenTableReloadKeepsOldOnFailure(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "tokens.txt")
	if err := os.WriteFile(path, []byte("客厅,"+hexOf("tok")+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	tt := NewTokenTable(path)
	if err := tt.Load(); err != nil {
		t.Fatalf("首次装载失败: %v", err)
	}
	if tt.Current().LookupToken("tok") == nil {
		t.Fatalf("应能查到 tok")
	}

	// 写坏文件再重载：失败且旧表仍在
	if err := os.WriteFile(path, []byte("坏行没有哈希\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := tt.Load(); err == nil {
		t.Fatalf("坏表重载应报错")
	}
	if tt.Current().LookupToken("tok") == nil {
		t.Fatalf("重载失败后旧表必须保留")
	}
}

// TestTokenTableLoadMissing 覆盖最凶的一路：文件根本读不到时，
// Load 报错且 Current 保持 nil——认证侧据此 503（fail closed），而非放行。
func TestTokenTableLoadMissing(t *testing.T) {
	tt := NewTokenTable(filepath.Join(t.TempDir(), "nope.txt"))
	if err := tt.Load(); err == nil {
		t.Fatalf("文件不存在应报错")
	}
	if tt.Current() != nil {
		t.Fatalf("未装载前 Current 应为 nil")
	}
}
