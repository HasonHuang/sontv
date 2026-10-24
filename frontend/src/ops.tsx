/**
 * 运维分区：安全管控、运行日志、备份与恢复。
 *
 * 接口：`/panel/api/security/*`、`/panel/api/logs`、`/panel/api/{backup,restore}`。
 */

import { useCallback, useEffect, useRef, useState } from 'react';

import { api, getToken } from './api';

type TokenInfo = {
  enabled: boolean;
  ttl_hours: number;
  current: string;
  old: string | null;
};

type IpEntry = {
  ip: string;
  requests: number;
  failures: number;
  banned: boolean;
  remaining_secs: number;
};

type IpStats = {
  count: number;
  banned: number;
  entries: IpEntry[];
};

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** 安全管控：播放 Token 与 IP 封禁。 */
export function SecurityPage() {
  const [token, setToken] = useState<TokenInfo | null>(null);
  const [stats, setStats] = useState<IpStats | null>(null);
  const [ip, setIp] = useState('');
  const [seconds, setSeconds] = useState('3600');
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');

  const load = useCallback(async () => {
    try {
      const [nextToken, nextStats] = await Promise.all([
        api<TokenInfo>('GET', '/security/token'),
        api<IpStats>('GET', '/security/ip/stats'),
      ]);
      setToken(nextToken);
      setStats(nextStats);
      setError('');
    } catch (err) {
      setError(errorText(err));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const rotate = async () => {
    setMessage('');
    setError('');
    try {
      await api('POST', '/security/token/rotate');
      setMessage('Token 已轮换，旧值保留 1 小时宽限');
      await load();
    } catch (err) {
      setError(errorText(err));
    }
  };

  const ban = async () => {
    setMessage('');
    setError('');
    try {
      await api('POST', '/security/ip/ban', {
        ip: ip.trim(),
        seconds: Number(seconds) || 0,
      });
      setMessage(`已封禁 ${ip.trim()}`);
      setIp('');
      await load();
    } catch (err) {
      setError(errorText(err));
    }
  };

  const unban = async (target: string) => {
    setMessage('');
    setError('');
    try {
      await api('POST', '/security/ip/unban', { ip: target });
      setMessage(`已解封 ${target}`);
      await load();
    } catch (err) {
      setError(errorText(err));
    }
  };

  return (
    <>
      <div className="card">
        <div className="row between">
          <div>
            <h1>安全管控</h1>
            <h2>播放 Token</h2>
          </div>
          <div className="row">
            <button className="ghost" onClick={() => void load()}>
              刷新
            </button>
            <button onClick={() => void rotate()}>轮换 Token</button>
          </div>
        </div>
        <dl>
          <dt>校验</dt>
          <dd>{token ? (token.enabled ? '已开启' : '已关闭') : '—'}</dd>
          <dt>有效期</dt>
          <dd>{token ? `${token.ttl_hours} 小时` : '—'}</dd>
          <dt>当前值</dt>
          <dd className="mono ellipsis">{token?.current ?? '—'}</dd>
          <dt>旧值（宽限）</dt>
          <dd className="mono ellipsis">{token?.old ?? '—'}</dd>
        </dl>
        {message && <div className="msg ok">{message}</div>}
        {error && <div className="msg err">{error}</div>}
      </div>

      <div className="card">
        <h2>IP 统计与封禁</h2>
        <p className="muted">
          共 {stats?.count ?? 0} 个地址，其中封禁 {stats?.banned ?? 0} 个。封禁只作用于非面板路径。
        </p>
        <div className="row">
          <input
            placeholder="地址，例如 10.0.0.2"
            value={ip}
            onChange={(event) => setIp(event.target.value)}
            style={{ flex: 2 }}
          />
          <input
            placeholder="秒"
            value={seconds}
            onChange={(event) => setSeconds(event.target.value)}
            style={{ flex: 1 }}
          />
          <button onClick={() => void ban()} disabled={ip.trim() === ''}>
            封禁
          </button>
        </div>
        <table className="table" style={{ marginTop: 12 }}>
          <thead>
            <tr>
              <th>地址</th>
              <th className="right">请求</th>
              <th className="right">失败</th>
              <th>状态</th>
              <th className="right">操作</th>
            </tr>
          </thead>
          <tbody>
            {(stats?.entries ?? []).map((entry) => (
              <tr key={entry.ip}>
                <td className="mono">{entry.ip}</td>
                <td className="right">{entry.requests}</td>
                <td className="right">{entry.failures}</td>
                <td>{entry.banned ? `封禁中（${entry.remaining_secs} 秒）` : '正常'}</td>
                <td className="right">
                  {entry.banned && (
                    <button className="ghost" onClick={() => void unban(entry.ip)}>
                      解封
                    </button>
                  )}
                </td>
              </tr>
            ))}
            {(stats?.entries ?? []).length === 0 && (
              <tr>
                <td colSpan={5} className="muted">
                  暂无记录。
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </>
  );
}

/** 运行日志：轮询 `/panel/api/logs`。 */
export function LogsPage() {
  const [lines, setLines] = useState<string[]>([]);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState('');
  const [limit, setLimit] = useState('200');

  const load = useCallback(async () => {
    try {
      const data = await api<{ total: number; lines: string[] }>(
        'GET',
        `/logs?limit=${Number(limit) || 200}`,
      );
      setLines(data.lines);
      setTotal(data.total);
      setError('');
    } catch (err) {
      setError(errorText(err));
    }
  }, [limit]);

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), 5000);
    return () => window.clearInterval(timer);
  }, [load]);

  return (
    <div className="card">
      <div className="row between">
        <div>
          <h1>运行日志</h1>
          <h2>最近 {lines.length} 行（共 {total} 行）</h2>
        </div>
        <div className="row">
          <input
            value={limit}
            onChange={(event) => setLimit(event.target.value)}
            style={{ width: 80 }}
          />
          <button className="ghost" onClick={() => void load()}>
            刷新
          </button>
        </div>
      </div>
      {error && <div className="msg err">{error}</div>}
      <pre className="log">{lines.join('\n')}</pre>
    </div>
  );
}

/** 备份与恢复。 */
export function BackupPage() {
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  const [scope, setScope] = useState('all');
  const file = useRef<HTMLInputElement>(null);

  const download = async () => {
    setMessage('');
    setError('');
    try {
      const response = await fetch(`/panel/api/backup?include=${scope}`, {
        headers: { Authorization: `Bearer ${getToken()}` },
      });
      if (!response.ok) {
        throw new Error(`HTTP ${response.status}`);
      }
      const blob = await response.blob();
      const disposition = response.headers.get('content-disposition') ?? '';
      const name = /filename="([^"]+)"/.exec(disposition)?.[1] ?? 'iptv-proxy-backup.tar.gz';
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = name;
      anchor.click();
      URL.revokeObjectURL(url);
      setMessage(`已导出 ${name}（${blob.size} 字节）`);
    } catch (err) {
      setError(errorText(err));
    }
  };

  const restore = async (input: HTMLInputElement) => {
    const selected = input.files?.[0];
    if (!selected) {
      return;
    }
    setMessage('');
    setError('');
    try {
      const response = await fetch('/panel/api/restore', {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${getToken()}`,
          'Content-Type': 'application/gzip',
        },
        body: await selected.arrayBuffer(),
      });
      const data = (await response.json()) as { error?: string; report?: { files: string[] } };
      if (!response.ok) {
        throw new Error(data.error ?? `HTTP ${response.status}`);
      }
      setMessage(`已恢复 ${data.report?.files.length ?? 0} 个文件，配置已重载`);
    } catch (err) {
      setError(errorText(err));
    } finally {
      input.value = '';
    }
  };

  return (
    <div className="card">
      <h1>备份与恢复</h1>
      <h2>导出</h2>
      <div className="row">
        <select value={scope} onChange={(event) => setScope(event.target.value)}>
          <option value="all">全部</option>
          <option value="epg-scripts">仅 EPG 脚本</option>
          <option value="resolver">仅解析脚本</option>
        </select>
        <button onClick={() => void download()}>下载备份</button>
      </div>
      <h2 style={{ marginTop: 24 }}>恢复</h2>
      <p className="muted">
        恢复会覆盖 `config.toml`，旧文件另存为 `config.toml.bak`。非法内容不会落盘。
      </p>
      <input
        ref={file}
        type="file"
        accept=".gz,application/gzip"
        onChange={(event) => void restore(event.target)}
      />
      {message && <div className="msg ok">{message}</div>}
      {error && <div className="msg err">{error}</div>}
    </div>
  );
}

type EpgStatus = {
  enabled: boolean;
  running: boolean;
  sources: number;
  channels: number;
  programmes: number;
  kept: number;
  keep_days: number;
  future_days: number;
  last_success: number | null;
  last_error: string | null;
};

type EpgMapping = {
  source_id: string;
  source_channel_id: string;
  source_name: string;
  source_display_name: string;
};

type EpgExtra = {
  source_id: string;
  source_channel_id: string;
  out_id: string;
  out_name: string;
};

type EpgMapData = {
  mappings: Record<string, EpgMapping>;
  extra: EpgExtra[];
};

/** 节目指南：采集状态、手工映射与额外频道。 */
export function EpgPanelPage() {
  const [status, setStatus] = useState<EpgStatus | null>(null);
  const [data, setData] = useState<EpgMapData>({ mappings: {}, extra: [] });
  const [mapping, setMapping] = useState({
    gid: '',
    cid: '',
    source_id: '',
    source_channel_id: '',
  });
  const [extra, setExtra] = useState({
    source_id: '',
    source_channel_id: '',
    out_id: '',
    out_name: '',
  });
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      const [nextStatus, nextData] = await Promise.all([
        api<EpgStatus>('GET', '/epg/status'),
        api<EpgMapData>('GET', '/epg/mappings'),
      ]);
      setStatus(nextStatus);
      setData(nextData);
      setError('');
    } catch (err) {
      setError(errorText(err));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const refresh = async () => {
    setBusy(true);
    setMessage('');
    setError('');
    try {
      const result = await api<{ channels: number; programmes: number }>(
        'POST',
        '/epg/refresh',
      );
      setMessage(`采集完成：${result.channels} 个频道，${result.programmes} 条节目`);
      await load();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  const saveMapping = async () => {
    setMessage('');
    setError('');
    try {
      await api('POST', '/epg/mapping', mapping);
      setMapping({ gid: '', cid: '', source_id: '', source_channel_id: '' });
      setMessage('映射已保存');
      await load();
    } catch (err) {
      setError(errorText(err));
    }
  };

  const removeMapping = async (key: string) => {
    const [gid, cid] = key.split('/');
    try {
      await api('POST', '/epg/mapping', { gid, cid, source_id: '', source_channel_id: '' });
      setMessage('映射已清除');
      await load();
    } catch (err) {
      setError(errorText(err));
    }
  };

  const addExtra = async () => {
    setMessage('');
    setError('');
    try {
      await api('POST', '/epg/extra', { action: 'add', ...extra });
      setExtra({ source_id: '', source_channel_id: '', out_id: '', out_name: '' });
      setMessage('额外频道已保存，正在重新生成');
      await load();
    } catch (err) {
      setError(errorText(err));
    }
  };

  const removeExtra = async (item: EpgExtra) => {
    try {
      await api('POST', '/epg/extra', {
        action: 'remove',
        source_id: item.source_id,
        source_channel_id: item.source_channel_id,
      });
      setMessage('额外频道已删除');
      await load();
    } catch (err) {
      setError(errorText(err));
    }
  };

  const regenerate = async () => {
    setBusy(true);
    setMessage('');
    setError('');
    try {
      const result = await api<{ channels: number; programmes: number }>(
        'POST',
        '/epg/regenerate',
      );
      setMessage(`重新生成：${result.channels} 个频道，${result.programmes} 条节目`);
      await load();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  const last = status?.last_success
    ? new Date(status.last_success * 1000).toLocaleString()
    : '—';

  return (
    <>
      <div className="card">
        <div className="row between">
          <div>
            <h1>节目指南</h1>
            <h2>{status?.enabled ? '已开启' : '已关闭'}</h2>
          </div>
          <div className="row">
            <a className="nav-item" href="/epg" target="_blank" rel="noreferrer">
              浏览页面
            </a>
            <button className="ghost" onClick={() => void load()}>
              刷新状态
            </button>
            <button className="ghost" onClick={() => void regenerate()} disabled={busy}>
              重新生成
            </button>
            <button onClick={() => void refresh()} disabled={busy || !status?.enabled}>
              {busy ? '采集中…' : '立即采集'}
            </button>
          </div>
        </div>
        <dl>
          <dt>源</dt>
          <dd>{status?.sources ?? 0} 个</dd>
          <dt>频道 / 节目</dt>
          <dd>
            {status?.channels ?? 0} / {status?.programmes ?? 0}
          </dd>
          <dt>保留</dt>
          <dd>
            已过 {status?.keep_days ?? 0} 天、未来 {status?.future_days ?? 0} 天
          </dd>
          <dt>最近成功</dt>
          <dd>{last}</dd>
          <dt>最近错误</dt>
          <dd className="mono ellipsis">{status?.last_error ?? '—'}</dd>
        </dl>
        {message && <div className="msg ok">{message}</div>}
        {error && <div className="msg err">{error}</div>}
      </div>
      <div className="card">
        <h2>手工映射</h2>
        <div className="row">
          <input
            placeholder="gid"
            value={mapping.gid}
            onChange={(event) => setMapping({ ...mapping, gid: event.target.value })}
          />
          <input
            placeholder="cid"
            value={mapping.cid}
            onChange={(event) => setMapping({ ...mapping, cid: event.target.value })}
          />
          <input
            placeholder="source_id"
            value={mapping.source_id}
            onChange={(event) => setMapping({ ...mapping, source_id: event.target.value })}
          />
          <input
            placeholder="source_channel_id"
            value={mapping.source_channel_id}
            onChange={(event) =>
              setMapping({ ...mapping, source_channel_id: event.target.value })
            }
          />
          <button onClick={() => void saveMapping()}>保存映射</button>
        </div>
        <table className="table">
          <thead>
            <tr>
              <th>频道</th>
              <th>源</th>
              <th>源频道</th>
              <th>源名称</th>
              <th className="right">操作</th>
            </tr>
          </thead>
          <tbody>
            {Object.entries(data.mappings).map(([key, item]) => (
              <tr key={key}>
                <td className="mono">{key}</td>
                <td className="mono">{item.source_id}</td>
                <td className="mono">{item.source_channel_id}</td>
                <td>{item.source_display_name || item.source_name || '—'}</td>
                <td className="right">
                  <button className="icon-btn" onClick={() => void removeMapping(key)}>
                    清除
                  </button>
                </td>
              </tr>
            ))}
            {Object.keys(data.mappings).length === 0 && (
              <tr>
                <td colSpan={5} className="muted">
                  暂无手工映射。
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <div className="card">
        <h2>额外频道</h2>
        <div className="row">
          <input
            placeholder="source_id"
            value={extra.source_id}
            onChange={(event) => setExtra({ ...extra, source_id: event.target.value })}
          />
          <input
            placeholder="source_channel_id"
            value={extra.source_channel_id}
            onChange={(event) => setExtra({ ...extra, source_channel_id: event.target.value })}
          />
          <input
            placeholder="out_id（可空）"
            value={extra.out_id}
            onChange={(event) => setExtra({ ...extra, out_id: event.target.value })}
          />
          <input
            placeholder="out_name（可空）"
            value={extra.out_name}
            onChange={(event) => setExtra({ ...extra, out_name: event.target.value })}
          />
          <button onClick={() => void addExtra()}>添加额外频道</button>
        </div>
        <table className="table">
          <thead>
            <tr>
              <th>源</th>
              <th>源频道</th>
              <th>输出 ID</th>
              <th>输出名称</th>
              <th className="right">操作</th>
            </tr>
          </thead>
          <tbody>
            {data.extra.map((item) => (
              <tr key={`${item.source_id}/${item.source_channel_id}`}>
                <td className="mono">{item.source_id}</td>
                <td className="mono">{item.source_channel_id}</td>
                <td className="mono">{item.out_id || '—'}</td>
                <td>{item.out_name || '—'}</td>
                <td className="right">
                  <button className="icon-btn" onClick={() => void removeExtra(item)}>
                    删除
                  </button>
                </td>
              </tr>
            ))}
            {data.extra.length === 0 && (
              <tr>
                <td colSpan={5} className="muted">
                  暂无额外频道。
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </>
  );
}
