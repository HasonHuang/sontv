package main

import (
	"flag"
	"log"
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
	cfgPath, showOnly := parseFlags()

	cfg, err := config.LoadConfig(*cfgPath)
	if err != nil {
		log.Fatalf("配置加载失败: %v", err)
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

	log.Printf("监听 %s", cfg.Listen)
	if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
		log.Fatalf("服务退出: %v", err)
	}
}

func parseFlags() (cfgPath *string, showOnly *bool) {
	cfgPath = flag.String("config", "/opt/sontv/config.json", "配置文件路径")
	showOnly = flag.Bool("check", false, "只做配置与 token 表校验，不启动服务")
	flag.Parse()
	return
}

// newTokenTable 建表并立即装载。表缺失/格式错时只记日志、照常返回：
// 服务仍可起，但认证必然 503（fail closed）。
func newTokenTable(path string) *tokens.TokenTable {
	table := tokens.NewTokenTable(path)
	if err := table.Load(); err != nil {
		log.Printf("token 表装载失败（服务将 503）: %v", err)
	} else {
		log.Printf("token 表已装载: %s", path)
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
					log.Printf("重载失败，保留旧表: %v", err)
				} else {
					log.Printf("token 表已重载")
				}
				continue
			}
			log.Printf("收到 %s，退出", sig)
			_ = srv.Close()
			return
		}
	}()
}
