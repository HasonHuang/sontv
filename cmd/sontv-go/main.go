package main

import (
	"flag"
	"fmt"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"time"

	"github.com/HasonHuang/mytv/go/internal/config"
	"github.com/HasonHuang/mytv/go/internal/server"
	"github.com/HasonHuang/mytv/go/internal/tokens"
)

// main 入口：解析参数、装载 token 表、起服务、等信号。
func main() {
	// 先解析参数、再配日志：门槛就写在 -log-level 里，不解析就无从知道。
	// 代价是 flag 自身的用法报错走的是默认格式——那几行只在参数写错时出现，
	// 且内容直白，不必也套上我们的级别前缀。
	cfgArg, logLevel, showOnly := parseFlags()
	server.ConfigureLogging(*logLevel)

	cfgPath, err := config.ResolveConfigPath(*cfgArg)
	if err != nil {
		fatalf("解析配置路径 %s: %v", *cfgArg, err)
	}
	cfg, err := config.LoadConfig(cfgPath)
	if err != nil {
		fatalf("配置加载失败: %v", err)
	}
	table := newTokenTable(cfg.TokensFile)
	if *showOnly {
		return
	}

	srv := &http.Server{
		Addr:              cfg.Listen,
		Handler:           server.NewServer(cfg, table).Handler(),
		ReadHeaderTimeout: 10 * time.Second,
	}
	watchSignals(srv, table) // 重载由 SIGUSR1 触发（设计 §4.3）

	slog.Info("监听", "地址", cfg.Listen)
	if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
		fatalf("服务退出: %v", err)
	}
}

func parseFlags() (cfgArg, logLevel *string, showOnly *bool) {
	// 缺省 config.json：相对路径按二进制同级目录解析（见 config.ResolveConfigPath），
	// 所以「把二进制和 config.json 放一起」就是免配置的默认部署方式。
	cfgArg = flag.String("config", config.DefaultConfigName, "配置文件路径（相对路径按二进制同级目录解析；传空串用全缺省）")
	logLevel = flag.String("log-level", "", "日志级别 debug/info/warn/error，缺省 info；未指定时读环境变量 "+server.LogLevelEnv+"。systemd 下请用这个参数，环境变量传不进服务进程")
	showOnly = flag.Bool("check", false, "只做配置与 token 表校验，不启动服务")
	flag.Parse()
	return
}

// newTokenTable 建表并立即装载。表缺失/格式错时只记日志、照常返回：
// 服务仍可起，但认证必然 503（fail closed）。
func newTokenTable(path string) *tokens.TokenTable {
	table := tokens.NewTokenTable(path)
	if err := table.Load(); err != nil {
		slog.Error("token 表装载失败，服务将以 503 应答", "文件", path, "原因", err)
	} else {
		slog.Info("token 表已装载", "文件", path)
	}
	return table
}

// watchSignals 起监督协程：reloadSignal 原子换表（失败保留旧表），
// quitSignals 优雅退出。reloadSignal 为 nil 的平台（Windows）无热重载。
func watchSignals(srv *http.Server, table *tokens.TokenTable) {
	sigCh := make(chan os.Signal, 1)

	var watch []os.Signal
	if reloadSignal != nil {
		watch = append(watch, reloadSignal)
	}
	watch = append(watch, quitSignals...)
	signal.Notify(sigCh, watch...)

	go func() {
		for sig := range sigCh {
			if reloadSignal != nil && sig == reloadSignal {
				if err := table.Load(); err != nil {
					slog.Error("重载失败，保留旧表", "原因", err)
				} else {
					slog.Info("token 表已重载")
				}
				continue
			}
			slog.Info("收到退出信号", "信号", sig.String())
			_ = srv.Close()
			return
		}
	}()
}

// fatalf 记一条 ERROR 再退出。
// 不用 log.Fatalf：那样走标准 log 包，日志级别与格式都绕过刚配好的门槛，
// 启动期失败反而成了唯一一条看不出级别的日志。
func fatalf(format string, args ...any) {
	slog.Error(fmt.Sprintf(format, args...))
	os.Exit(1)
}
