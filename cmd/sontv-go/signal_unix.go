//go:build !windows

package main

import (
	"os"
	"syscall"
)

// quitSignals 触发优雅退出的信号。SIGTERM 供 systemd/容器停止，SIGINT 供 Ctrl+C。
var quitSignals = []os.Signal{syscall.SIGTERM, syscall.SIGINT}

// reloadSignal 触发 token 表热重载的信号。Windows 无此信号，见 signal_windows.go。
var reloadSignal os.Signal = syscall.SIGUSR1
