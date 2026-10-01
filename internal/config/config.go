package config

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

// 缺省监听地址。抽成常量：它同时出现在 DefaultConfig、LoadConfig 的兜底与测试里，
// 改一处漏一处就会让「空 listen 回落」和缺省对不上。
const DefaultListen = "127.0.0.1:9900"

// DefaultConfigName 是缺省配置文件名，与二进制同级放置。
const DefaultConfigName = "config.json"

// Config 是启动时读一次的服务配置，改后重启服务生效。
// 与 token 表（可热重载）分开：那些是凭据，这些是进程的监听与上游地址。
type Config struct {
	TokensFile        string `json:"tokens_file"`
	DefaultTTLHours   int    `json:"default_ttl_hours"`
	UpstreamM3U       string `json:"upstream_m3u"`
	Listen            string `json:"listen"`
	UnwrapRemoteProxy bool   `json:"unwrap_remote_proxy"`
}

// DefaultConfig 返回设计文档 §4.2 的全部缺省值。
// 单独一个函数：测试与「配置文件不存在时照常启动」都依赖它。
func DefaultConfig() *Config {
	return &Config{
		TokensFile:        "/opt/sontv/tokens.txt",
		DefaultTTLHours:   24,
		UpstreamM3U:       "https://cdn.qd.je/mytv0.m3u",
		Listen:            DefaultListen,
		UnwrapRemoteProxy: true,
	}
}

// LoadConfig 读配置文件。path 为空表示只要缺省值。
// 注意 json.Unmarshal 只覆盖 JSON 里出现过的字段，所以未写的字段保持缺省——
// 「只配 listen」这种最小配置文件不会把上游地址清空。
func LoadConfig(path string) (*Config, error) {
	cfg := DefaultConfig()
	if path == "" {
		resolveTokensFile(cfg, "")
		return cfg, nil
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("读配置文件 %s: %w", path, err)
	}
	if err := json.Unmarshal(data, cfg); err != nil {
		return nil, fmt.Errorf("解析配置文件 %s: %w", path, err)
	}
	if cfg.DefaultTTLHours <= 0 {
		// 0/负数会让临时 token 当场过期，静默改成缺省而不是拒绝启动：
		// 服务可用性优先，启动日志里会体现实际生效值。
		cfg.DefaultTTLHours = 24
	}
	if cfg.Listen == "" {
		cfg.Listen = DefaultListen
	}
	resolveTokensFile(cfg, path)
	return cfg, nil
}

// resolveTokensFile 把相对的 tokens_file 补成绝对路径。
// 基准是**配置文件同级目录**（配置文件通常就在二进制旁边）；
// 没有配置文件时退回二进制目录。
func resolveTokensFile(cfg *Config, cfgPath string) {
	if cfg.TokensFile == "" || filepath.IsAbs(cfg.TokensFile) {
		return
	}
	base := ""
	if cfgPath != "" {
		if abs, err := filepath.Abs(cfgPath); err == nil {
			cfgPath = abs
		}
		base = filepath.Dir(cfgPath)
	}
	if base == "" {
		dir, err := executableDir()
		if err != nil {
			// 拿不到二进制目录：让 tokens_file 保持原样，装载时会响亮报错，
			// 好过悄悄指向一个看似成功的错误路径。
			return
		}
		base = dir
	}
	cfg.TokensFile = filepath.Join(base, cfg.TokensFile)
}

// executableDir 返回二进制所在目录（已解析软链）。
func executableDir() (string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", err
	}
	// 解析软链：/opt/sontv/sontv-go 常常是指向版本目录的链接，
	// 跟链接会让配置看起来「放在 /opt/sontv」，实际在别处。
	if resolved, err := filepath.EvalSymlinks(exe); err == nil {
		exe = resolved
	}
	return filepath.Dir(exe), nil
}

// ResolveConfigPath 把 -config 的取值解析成一个可以直接打开的路径。
// 空串原样返回（调用方据此走全缺省）。
//
// 相对路径按**二进制所在目录**解析，而不是进程的工作目录：systemd 与
// 容器里工作目录五花八门，只有二进制同级是稳定的「安装目录」。
func ResolveConfigPath(path string) (string, error) {
	if path == "" {
		return "", nil
	}
	if filepath.IsAbs(path) {
		return path, nil
	}
	dir, err := executableDir()
	if err != nil {
		// 拿不到可执行文件路径就退回工作目录：读不到配置会响亮失败，
		// 总好过在错误的目录下悄悄读到另一份配置。
		return filepath.Abs(path)
	}
	return filepath.Join(dir, path), nil
}
