import { useCallback, useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';

import { api, clearToken, getToken, setToken, UNAUTHORIZED_EVENT } from './api';
import { GroupsPage } from './groups';
import { BackupPage, EpgPanelPage, LogsPage, SecurityPage } from './ops';
import { CachePage, StatsPage } from './cache';
import { AssetsPage } from './assets';
import { LogosPage } from './logo';
import { PlaylistPage } from './playlist';
import { LivestreamPage, SchedulePage } from './schedule';
import { SystemPage } from './system';
import './style.css';

/** 侧栏分区；顺序按目标面板。 */
type SectionId =
  | 'dashboard'
  | 'cache'
  | 'groups'
  | 'settings'
  | 'security'
  | 'stats'
  | 'schedule'
  | 'epg'
  | 'logos'
  | 'subs'
  | 'livestream'
  | 'assets'
  | 'logs';

const NAV: { id: SectionId; label: string }[] = [
  { id: 'dashboard', label: '概览' },
  { id: 'cache', label: '缓存状态' },
  { id: 'groups', label: '分组管理' },
  { id: 'settings', label: '系统设置' },
  { id: 'security', label: '安全管控' },
  { id: 'stats', label: '频道热度' },
  { id: 'schedule', label: '定时录制' },
  { id: 'epg', label: '节目指南' },
  { id: 'logos', label: '台标管理' },
  { id: 'subs', label: '订阅聚合' },
  { id: 'livestream', label: '回看管理' },
  { id: 'assets', label: '视频素材' },
  { id: 'logs', label: '运行日志' },
];

type Status = Record<string, unknown>;

function Login({ onDone }: { onDone: () => void }) {
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError('');
    try {
      const data = await api<{ token: string }>('POST', '/login', { username, password });
      setToken(data.token);
      onDone();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="page">
      <div className="card">
        <h1>IPTV Proxy</h1>
        <h2>管理面板</h2>
        <form onSubmit={submit}>
          <label htmlFor="user">用户名</label>
          <input id="user" value={username} onChange={(e) => setUsername(e.target.value)} />
          <label htmlFor="pass">密码</label>
          <input
            id="pass"
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
          <div style={{ marginTop: 16 }}>
            <button type="submit" disabled={busy}>
              登 录
            </button>
          </div>
        </form>
        {error && <div className="msg err">{error}</div>}
      </div>
    </div>
  );
}

function Overview() {
  const [status, setStatus] = useState<Status | null>(null);
  const [error, setError] = useState('');

  const load = useCallback(async () => {
    try {
      setStatus(await api<Status>('GET', '/status'));
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const entries = Object.entries(status ?? {}).filter(([, value]) => typeof value !== 'object');

  return (
    <div className="card">
      <div className="row between">
        <div>
          <h1>IPTV Proxy</h1>
          <h2>概览</h2>
        </div>
        <div className="row">
          <button className="ghost" onClick={() => void load()}>
            刷新
          </button>
        </div>
      </div>
      <dl>
        {entries.map(([key, value]) => (
          <div key={key} style={{ display: 'contents' }}>
            <dt>{key}</dt>
            <dd>{String(value)}</dd>
          </div>
        ))}
      </dl>
      {error && <div className="msg err">{error}</div>}
    </div>
  );
}

/** 系统设置：原始配置编辑、运行状态与备份恢复。 */
function SettingsPage() {
  const [config, setConfig] = useState('');
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');

  const load = useCallback(async () => {
    try {
      const next = await api<unknown>('GET', '/config');
      setConfig(JSON.stringify(next, null, 2));
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const save = async () => {
    setMessage('');
    setError('');
    try {
      await api('PUT', '/config', JSON.parse(config) as unknown);
      setMessage('配置已保存');
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <>
      <div className="card">
        <div className="row between">
          <h1>系统设置</h1>
          <button className="ghost" onClick={() => void load()}>
            重新读取
          </button>
        </div>
        <h2>配置（config.toml 全量）</h2>
        <textarea value={config} onChange={(e) => setConfig(e.target.value)} spellCheck={false} />
        <div style={{ marginTop: 12 }}>
          <button onClick={() => void save()}>保存</button>
        </div>
        {message && <div className="msg ok">{message}</div>}
        {error && <div className="msg err">{error}</div>}
      </div>
      <SystemPage />
      <BackupPage />
    </>
  );
}

function Shell({ onLogout }: { onLogout: () => void }) {
  const [section, setSection] = useState<SectionId>('dashboard');

  const logout = async () => {
    try {
      await api('POST', '/logout');
    } catch {
      // 忽略注销错误
    }
    clearToken();
    onLogout();
  };

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">
          <strong>IPTV Proxy</strong>
          <span className="muted">管理面板</span>
        </div>
        <nav>
          {NAV.map((item) => (
            <button
              key={item.id}
              className={`nav-item${section === item.id ? ' active' : ''}`}
              onClick={() => setSection(item.id)}
            >
              {item.label}
            </button>
          ))}
        </nav>
        <div className="sidebar-foot">
          <button className="ghost" onClick={() => void logout()}>
            退出登录
          </button>
        </div>
      </aside>
      <main className="content">
        {section === 'dashboard' && <Overview />}
        {section === 'cache' && <CachePage />}
        {section === 'groups' && <GroupsPage />}
        {section === 'settings' && <SettingsPage />}
        {section === 'security' && <SecurityPage />}
        {section === 'stats' && <StatsPage />}
        {section === 'schedule' && <SchedulePage view="schedule" />}
        {section === 'epg' && <EpgPanelPage />}
        {section === 'logos' && <LogosPage />}
        {section === 'subs' && <PlaylistPage />}
        {section === 'livestream' && <LivestreamPage />}
        {section === 'assets' && <AssetsPage />}
        {section === 'logs' && <LogsPage />}
      </main>
    </div>
  );
}

function App() {
  const [loggedIn, setLoggedIn] = useState(() => getToken() !== '');

  // 会话失效（401）时回到登录卡。
  useEffect(() => {
    const onUnauthorized = () => setLoggedIn(false);
    window.addEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
  }, []);

  return loggedIn ? (
    <Shell onLogout={() => setLoggedIn(false)} />
  ) : (
    <Login onDone={() => setLoggedIn(true)} />
  );
}

createRoot(document.getElementById('root')!).render(<App />);
