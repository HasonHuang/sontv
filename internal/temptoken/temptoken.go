package temptoken

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"errors"
	"math"
	"strconv"
	"strings"
	"time"

	"github.com/HasonHuang/mytv/go/internal/tokens"
)

// 临时 token 形态：<exp>.<uid>.<sig_b64url>（设计 §2.3）。
//
//	exp — 十进制 Unix 秒，明文便于排障
//	uid — 稳定 token 的短哈希（sha256 前 16 位 hex），绑定签发者、不泄露明文
//	sig — HMAC-SHA256(K, "<exp>.<uid>") 的 base64url，K = 该行完整的 64 位 hash
//
// 签 K 用行 hash 而不是独立 secret：能拿到 hash 的人本来就能直接用稳定 token，
// 用 hash 当 key 不降低安全性，却省掉一个密钥文件的运维。

var (
	// errExpired 覆盖「exp 已过」与「exp 格式坏」两件事：对外都是 401。
	errExpired = errors.New("临时 token 已过期")
	// errSig 表示签名不匹配（伪造或篡改）。
	errSig = errors.New("临时 token 签名错误")
	// errRowGone 表示 uid 已不在表中——稳定 token 被删后旧临时 token 即刻失效。
	errRowGone = errors.New("临时 token 对应的行已不存在")
)

// Issue 为某一行签发临时 token。
// now 与 ttl 显式传入，测试里无需等待真实时钟。
func Issue(row *tokens.Row, now time.Time, ttl time.Duration) string {
	exp := strconv.FormatInt(now.Add(ttl).Unix(), 10)
	payload := exp + "." + row.UID
	return payload + "." + sign(row.Hash, payload)
}

// Verify 校验一条临时 token，成功时返回它对应的行。
// 三条全过才放行（设计 §2.3）：签名正确、未过期、uid 仍在表。
//
// 写成自由函数而非 Snapshot 的方法：Go 不许在别的包里给外部类型挂方法。
func Verify(snap *tokens.Snapshot, tok string, now time.Time) (*tokens.Row, error) {
	expPart, uid, sig, ok := split(tok)
	if !ok {
		return nil, errSig
	}
	row := snap.LookupUID(uid)
	if row == nil {
		// 先查行再验签：行已删的行需要先用当前表判断，
		// 否则「删行后旧 token 仍生效」那条要求落空。
		return nil, errRowGone
	}
	payload := expPart + "." + uid
	if !hmac.Equal([]byte(sig), []byte(sign(row.Hash, payload))) {
		return nil, errSig
	}
	exp, err := strconv.ParseInt(expPart, 10, 64)
	if err != nil {
		return nil, errExpired
	}
	if now.Unix() >= exp {
		return nil, errExpired
	}
	return row, nil
}

// sign 计算 base64url(raw, 无 padding) 的签名。
func sign(key, payload string) string {
	mac := hmac.New(sha256.New, []byte(key))
	mac.Write([]byte(payload))
	return base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
}

// split 拆成 exp / uid / sig 三段，任一段缺失或形态不对都返回 false。
func split(tok string) (exp, uid, sig string, ok bool) {
	parts := strings.Split(tok, ".")
	if len(parts) != 3 {
		return "", "", "", false
	}
	exp, uid, sig = parts[0], parts[1], parts[2]
	if exp == "" || uid == "" || sig == "" || len(uid) != tokens.UIDLen {
		return "", "", "", false
	}
	return exp, uid, sig, true
}

// TTL 取该行的临时 token 有效期，0 表示回落全局默认（设计 §2.4）。
func TTL(row *tokens.Row, defaultHours int) time.Duration {
	hours := row.TTLHours
	if hours <= 0 {
		hours = defaultHours
	}
	if hours <= 0 {
		hours = 24
	}
	// 上限保护：hours*time.Hour 不会溢出 int64，但仍拒绝荒谬值以便排障。
	if hours > math.MaxInt32/3600 {
		hours = math.MaxInt32 / 3600
	}
	return time.Duration(hours) * time.Hour
}
