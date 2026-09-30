package server

import (
	"net/http"

	"github.com/HasonHuang/mytv/go/internal/temp"
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
func (s *Server) authStable(w http.ResponseWriter, r *http.Request) *tokens.Row {
	snap := s.snapshot(w)
	if snap == nil {
		return nil
	}
	row := snap.LookupToken(r.URL.Query().Get(stableParam))
	if row == nil {
		writeErr(w, http.StatusForbidden, "凭据无效")
		return nil
	}
	return row
}

// authURL 接受稳定或临时 token（/url）。判定优先级：有 t 按临时校验，
// 否则按稳定校验，都没有则 403（ADR-0002）。
func (s *Server) authURL(w http.ResponseWriter, r *http.Request) *tokens.Row {
	snap := s.snapshot(w)
	if snap == nil {
		return nil
	}
	q := r.URL.Query()
	if tok := q.Get(tempParam); tok != "" {
		row, err := temp.VerifyTempToken(snap, tok, s.now())
		if err != nil {
			writeErr(w, http.StatusUnauthorized, "临时凭据无效")
			return nil
		}
		return row
	}
	row := snap.LookupToken(q.Get(stableParam))
	if row == nil {
		writeErr(w, http.StatusForbidden, "凭据无效")
		return nil
	}
	return row
}
