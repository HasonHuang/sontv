package tokens

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"strconv"
	"strings"
	"sync/atomic"
)

// Row 是 token 表的一行。服务器只保存 sha256 哈希，不存明文。
type Row struct {
	Label    string // 第一列：人类可读标签，不参与任何判定
	Hash     string // 第二列：sha256(token) 的 64 位小写 hex
	UID      string // Hash 的前 16 位：临时 token 里不泄露明文的那段标识
	TTLHours int    // 第三列：临时 token 的 TTL 小时数，0 = 回落全局默认
}

// Snapshot 是某一时刻的 token 表快照，构造后只读。
// 全字段导出但约定不可变：换表是换整个 Snapshot，不就地改。
type Snapshot struct {
	Rows   []Row
	byHash map[string]*Row
	byUID  map[string]*Row
}

// LookupToken 用明文 token 查行：一次 sha256 + 一次 map 查找。
func (s *Snapshot) LookupToken(token string) *Row {
	if s == nil || token == "" {
		return nil
	}
	sum := sha256.Sum256([]byte(token))
	return s.byHash[hex.EncodeToString(sum[:])]
}

// LookupUID 用临时 token 里的 uid 查行。返回 nil 表示该行已被删除——
// 这正是「删行即吊销全部临时 token」那条判定（设计 §2.3 第 3 条）。
func (s *Snapshot) LookupUID(uid string) *Row {
	if s == nil {
		return nil
	}
	return s.byUID[uid]
}

// Empty 表示这份表一行有效凭据都没有。
func (s *Snapshot) Empty() bool {
	return s == nil || len(s.Rows) == 0
}

// UIDLen 是 UID 段的十六进制长度（sha256 前 16 位）。导出是给 temptoken 包复用，
// 免得两处各写一份 16 的魔数。
const UIDLen = 16

// ParseTokenTable 解析 tokens.txt。任何一行格式错都让整张表失败：
// 宁可 503 也不接受半张表——漏读一行就是误删一个用户，误判一行就是放行一个攻击者。
func ParseTokenTable(data []byte) (*Snapshot, error) {
	var rows []Row
	seen := map[string]bool{}
	for i, raw := range strings.Split(string(data), "\n") {
		row, skip, err := parseRow(raw, i+1, seen)
		if err != nil {
			return nil, err
		}
		if !skip {
			rows = append(rows, row)
		}
	}
	return buildSnapshot(rows), nil
}

// parseRow 解析一行。skip 表示该行是空行/注释、应被忽略；error 让整表失败。
func parseRow(raw string, lineNo int, seen map[string]bool) (row Row, skip bool, err error) {
	line := strings.TrimRight(raw, "\r")
	if t := strings.TrimSpace(line); t == "" || strings.HasPrefix(t, "#") {
		return Row{}, true, nil // 空行与行首注释：整行跳过
	}
	// 行内「空白 + #」之后是注释，截掉再解析。
	// 只认紧邻的 " #"/"\t#"：label 里的裸 #（如 abc#def）不算注释，
	// 否则一个普通标签就会把整行的哈希吃掉（设计 §4.1）。
	if idx := firstCommentMark(line); idx >= 0 {
		line = line[:idx]
	}

	parts := strings.Split(strings.TrimSpace(line), ",")
	if len(parts) < 2 {
		return Row{}, false, fmt.Errorf("第 %d 行：至少需要 标签,sha256hex 两列", lineNo)
	}
	label := strings.TrimSpace(parts[0])
	hash := strings.ToLower(strings.TrimSpace(parts[1]))
	if !isSHA256Hex(hash) {
		return Row{}, false, fmt.Errorf("第 %d 行：第二列不是 64 位十六进制 sha256", lineNo)
	}

	ttl, err := parseTTL(parts, lineNo)
	if err != nil {
		return Row{}, false, err
	}
	if seen[hash] {
		return Row{}, false, fmt.Errorf("第 %d 行：哈希与前面某行重复", lineNo)
	}
	seen[hash] = true
	return Row{Label: label, Hash: hash, UID: hash[:UIDLen], TTLHours: ttl}, false, nil
}

// parseTTL 解析可选的第三列 TTL 小时数；缺省为 0（回落全局默认）。
func parseTTL(parts []string, lineNo int) (int, error) {
	if len(parts) < 3 || strings.TrimSpace(parts[2]) == "" {
		return 0, nil
	}
	v, err := strconv.Atoi(strings.TrimSpace(parts[2]))
	if err != nil || v <= 0 || v > maxTTLHours {
		return 0, fmt.Errorf("第 %d 行：第三列必须是 1~%d 的整数小时数", lineNo, maxTTLHours)
	}
	return v, nil
}

// buildSnapshot 在切片定型之后建索引：元素地址此时才稳定。
// 边 append 边取 &rows[len-1] 会拿到"扩容前"的悬空指针。
func buildSnapshot(rows []Row) *Snapshot {
	snap := &Snapshot{Rows: rows, byHash: make(map[string]*Row, len(rows)), byUID: make(map[string]*Row, len(rows))}
	for i := range snap.Rows {
		r := &snap.Rows[i]
		snap.byHash[r.Hash] = r
		snap.byUID[r.UID] = r
	}
	return snap
}

// maxTTLHours 是临时 token TTL 的上限（约 100 年），用于拒绝病态输入。
const maxTTLHours = 876000

// firstCommentMark 返回行内「空白 + #」的起始位置，没有则 -1。
func firstCommentMark(line string) int {
	best := -1
	for _, mark := range []string{" #", "\t#"} {
		if idx := strings.Index(line, mark); idx >= 0 && (best < 0 || idx < best) {
			best = idx
		}
	}
	return best
}

func isSHA256Hex(s string) bool {
	if len(s) != 64 {
		return false
	}
	for _, c := range s {
		if !((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f')) {
			return false
		}
	}
	return true
}

// ErrTokenTableUnreadable 表示重载失败时沿用旧表；启动时它会变成整站 503。
var ErrTokenTableUnreadable = errors.New("token 表不可读")

// TokenTable 持有当前快照，支持原子换表。
// 验证路径只做 atomic.Load，无锁、零文件访问（设计 §4.3）。
type TokenTable struct {
	path string
	cur  atomic.Pointer[Snapshot]
}

func NewTokenTable(path string) *TokenTable {
	return &TokenTable{path: path}
}

// Current 返回当前快照；可能为 nil（启动即失败，尚未装载任何表）。
func (t *TokenTable) Current() *Snapshot {
	return t.cur.Load()
}

// Load 读文件并原子换表。
// 任何失败都保留旧表原样返回错误——「绝不裸奔半吊子」（设计 §4.3）。
func (t *TokenTable) Load() error {
	data, err := os.ReadFile(t.path)
	if err != nil {
		return fmt.Errorf("%w: %v", ErrTokenTableUnreadable, err)
	}
	snap, err := ParseTokenTable(data)
	if err != nil {
		return fmt.Errorf("解析 %s: %w", t.path, err)
	}
	t.cur.Store(snap)
	return nil
}
