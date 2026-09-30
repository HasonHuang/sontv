package config

import (
	"encoding/json"
	"fmt"
	"os"
)

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
		Listen:            "127.0.0.1:19900",
		UnwrapRemoteProxy: true,
	}
}

// LoadConfig 读配置文件。path 为空表示只要缺省值。
// 注意 json.Unmarshal 只覆盖 JSON 里出现过的字段，所以未写的字段保持缺省——
// 「只配 listen」这种最小配置文件不会把上游地址清空。
func LoadConfig(path string) (*Config, error) {
	cfg := DefaultConfig()
	if path == "" {
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
		cfg.Listen = "127.0.0.1:19900"
	}
	return cfg, nil
}
