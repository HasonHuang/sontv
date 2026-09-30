//go:build windows

package main

import (
	"os"
	"syscall"
)

// quitSignals 触发优雅退出的信号。Windows 无 SIGTERM，只认 Ctrl+C/Ctrl+Break。
var quitSignals = []os.Signal{syscall.SIGINT}

// reloadSignal 无对应信号：Windows 不支持 SIGUSR1，热重载只能重启进程。
var reloadSignal os.Signal = nil
