package temptoken

import (
	"crypto/sha256"
	"encoding/hex"
	"strings"
	"testing"
	"time"

	"github.com/HasonHuang/sontv/internal/tokens"
)

// hexOf 返回 token 的 sha256 十六进制，用于构造合法 token 表。
// 与 tokens 包测试里的同名辅助各写一份：两包已分家，测试不便共享。
func hexOf(token string) string {
	sum := sha256.Sum256([]byte(token))
	return hex.EncodeToString(sum[:])
}

// testSnapshot 造一份含单行的表，返回快照与行。
func testSnapshot(t *testing.T) (*tokens.Snapshot, *tokens.Row) {
	t.Helper()
	snap, err := tokens.ParseTokenTable([]byte("客厅," + hexOf("stable-token") + "\n"))
	if err != nil {
		t.Fatalf("构造表失败: %v", err)
	}
	return snap, &snap.Rows[0]
}

// 临时 token 的验证顺序（temptoken.go）：先查 uid 还在不在表里、再比签名、最后看 exp。
// 查行是第一步，因为签名 key 就是该行的 hash；行没了就无从验签，直接判吊销。
// 这里几个测试各钉住其中一环：任何一环被绕过都会在此处暴露。

func TestIssueVerifyRoundTrip(t *testing.T) {
	snap, row := testSnapshot(t)
	now := time.Unix(1_700_000_000, 0)
	tok := Issue(row, now, time.Hour)

	// TTL 内验签应通过，且返回的行就是签发它的那一行
	got, err := Verify(snap, tok, now.Add(30*time.Minute))
	if err != nil {
		t.Fatalf("校验应通过: %v", err)
	}
	if got.UID != row.UID {
		t.Fatalf("返回行不符: %+v", got)
	}
}

func TestVerifyExpired(t *testing.T) {
	snap, row := testSnapshot(t)
	now := time.Unix(1_700_000_000, 0)
	tok := Issue(row, now, time.Hour)
	// 恰在 exp 时刻即过期（边界取「不晚于」而非「早于」）
	if _, err := Verify(snap, tok, now.Add(time.Hour)); err != errExpired {
		t.Fatalf("恰到期应过期: %v", err)
	}
	if _, err := Verify(snap, tok, now.Add(2*time.Hour)); err != errExpired {
		t.Fatalf("过时后应过期: %v", err)
	}
}

// TestVerifySignature 是防篡改契约：改动 token 任一可被攻击者
// 触碰的部分，签名就不再成立。此处穷举畸形输入的正确拒绝。
func TestVerifySignature(t *testing.T) {
	snap, row := testSnapshot(t)
	now := time.Unix(1_700_000_000, 0)
	tok := Issue(row, now, time.Hour)

	cases := []struct {
		name string
		tok  string
	}{
		{"篡改签名", tok[:len(tok)-1] + flipLast(tok)},
		{"篡改 exp", "9999999999" + tok[strings.Index(tok, "."):]},
		{"缺段", "a.b"},
		{"空", ""},
		{"uid 长度错", "1234.short.zzz"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if _, err := Verify(snap, c.tok, now); err == nil {
				t.Fatalf("非法 token 应被拒: %q", c.tok)
			}
		})
	}
}

// flipLast 改动末位字符，制造签名不符。
func flipLast(s string) string {
	if s[len(s)-1] == 'a' {
		return "b"
	}
	return "a"
}

// TestVerifyRowGone 钉住「删行即吊销」：签名仍对、未过期，
// 但 uid 已不在表里 → 拒。这是封人的唯一手段，不能被绕过。
func TestVerifyRowGone(t *testing.T) {
	_, row := testSnapshot(t)
	now := time.Unix(1_700_000_000, 0)
	tok := Issue(row, now, time.Hour)

	// 换一张没有该行的表（模拟删行）
	empty, err := tokens.ParseTokenTable([]byte("# 空表\n"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := Verify(empty, tok, now); err != errRowGone {
		t.Fatalf("删行后应 errRowGone，实际 %v", err)
	}
}

// TestSplitTempToken 分段解析的边界：段数对但 uid 长度不对要拒。
func TestSplitTempToken(t *testing.T) {
	if _, _, _, ok := split("a.b.c"); ok {
		t.Fatalf("uid 长度不符应 false")
	}
	uid := strings.Repeat("0", tokens.UIDLen)
	exp, gotUID, sig, ok := split("123." + uid + ".sig")
	if !ok || exp != "123" || gotUID != uid || sig != "sig" {
		t.Fatalf("正常三段应解析: %q %q %q %v", exp, gotUID, sig, ok)
	}
}

// TestTempTTLFallback 优先级：行级 TTL > 全局默认 > 兜底 24h；
// 病态值（<=0）一律回落，绝不签出零寿命的 token。
func TestTempTTLFallback(t *testing.T) {
	cases := []struct {
		row      *tokens.Row
		deflt    int
		expected time.Duration
	}{
		{&tokens.Row{TTLHours: 6}, 24, 6 * time.Hour},
		{&tokens.Row{TTLHours: 0}, 12, 12 * time.Hour},
		{&tokens.Row{TTLHours: 0}, 0, 24 * time.Hour},
		{&tokens.Row{TTLHours: -3}, 8, 8 * time.Hour},
	}
	for i, c := range cases {
		if got := TTL(c.row, c.deflt); got != c.expected {
			t.Fatalf("用例 %d: got %v want %v", i, got, c.expected)
		}
	}
}
