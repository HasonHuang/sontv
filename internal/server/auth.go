package server

import (
	"net/http"

	"github.com/HasonHuang/mytv/go/internal/temptoken"
	"github.com/HasonHuang/mytv/go/internal/tokens"
)

// 查询参数名。凭据形态分离（ADR-0002）：
//   - token：稳定 token（永久、可吊销），/sub 与 /url 都可
//   - t：临时 token（短命），只出现在响应体的子链接里
//   - u：/url 的目标地址；url：/sub 的上游地址；filter：订阅过滤词
const (
	stableParam = "token"
	tempParam   = "t"
	targetParam = "u"
	subParam    = "url"
	filterParam = "filter"
)

// ---------- 认证（fail closed） ----------

// snapshot 取当前 token 表；nil 或空表整站 503。
// 认证开了却没有可用凭据时，宁可拒绝也不裸奔（设计 §2.7）。
func (s *Server) snapshot(w http.ResponseWriter) *tokens.Snapshot {
	snap := s.tokens.Current()
	if snap.Empty() {
		writeErr(w, http.StatusServiceUnavailable, "服务未就绪")
		return nil
	}
	return snap
}

// authStable 只接受稳定 token（/sub）。返回 nil 表示已写出错误响应。
// lg 可为 nil（测试直接调 handler 时）；认证失败一律留痕——
// 「拉不到内容」十有八九是凭据过期或打错，日志里必须有。
func (s *Server) authStable(w http.ResponseWriter, r *http.Request, lg *reqLog) *tokens.Row {
	snap := s.snapshot(w)
	if snap == nil {
		lgAuth(lg, "token 表为空")
		return nil
	}
	row := snap.LookupToken(r.URL.Query().Get(stableParam))
	if row == nil {
		lgAuth(lg, "稳定 token 不匹配")
		writeErr(w, http.StatusForbidden, "凭据无效")
		return nil
	}
	return row
}

// authURL 接受稳定或临时 token（/url）。判定优先级：有 t 按临时校验，
// 否则按稳定校验，都没有则 403（ADR-0002）。
func (s *Server) authURL(w http.ResponseWriter, r *http.Request, lg *reqLog) *tokens.Row {
	snap := s.snapshot(w)
	if snap == nil {
		lgAuth(lg, "token 表为空")
		return nil
	}
	q := r.URL.Query()
	if tok := q.Get(tempParam); tok != "" {
		row, err := temptoken.Verify(snap, tok, s.now())
		if err != nil {
			// err 已区分过期/签名错/行已删，是「播不了」最常见的三种原因之一，
			// 原样落日志即可，不必再包装。
			lgAuth(lg, "临时凭据无效: %v", err)
			writeErr(w, http.StatusUnauthorized, "临时凭据无效")
			return nil
		}
		return row
	}
	row := snap.LookupToken(q.Get(stableParam))
	if row == nil {
		lgAuth(lg, "缺少 t= 且稳定 token 不匹配")
		writeErr(w, http.StatusForbidden, "凭据无效")
		return nil
	}
	return row
}

// lgAuth 记一条认证失败；lg 为 nil 时静默（测试直调 handler 的路径）。
func lgAuth(lg *reqLog, format string, args ...any) {
	if lg == nil {
		return
	}
	lg.warnf("认证失败 "+format, args...)
}
