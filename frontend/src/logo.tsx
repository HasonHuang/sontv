/**
 * 台标库页；可独立挂载（`/logo`），也可嵌入管理面板。
 *
 * 公开浏览；登录后可上传、重命名、删除，并把台标应用到频道。
 */

import { useCallback, useEffect, useState } from 'react';
import { api, getToken } from './api';
import './style.css';

type Logo = {
  file: string;
  name: string;
  ext: string;
  bytes: number;
  url: string;
};

type UploadResult = {
  added: Logo[];
  failed: { name: string; error: string }[];
};

type Missing = { gid: string; cid: string; name: string };

type Report = {
  channels: number;
  missing: number;
  broken: number;
  unused: number;
  missing_channels: Missing[];
  unused_logos: string[];
};

type Suggestion = {
  gid: string;
  cid: string;
  name: string;
  suggest: { file: string; score: number }[];
};

function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

function channelKey(channel: Missing): string {
  return `${channel.gid}/${channel.cid}`;
}

export function LogosPage() {
  const [logos, setLogos] = useState<Logo[]>([]);
  const [report, setReport] = useState<Report | null>(null);
  const [suggestions, setSuggestions] = useState<Record<string, string>>({});
  const [picked, setPicked] = useState<Record<string, string>>({});
  const [message, setMessage] = useState('');
  const [authed] = useState(() => Boolean(getToken()));

  const load = useCallback(async () => {
    const response = await fetch('/logo/api/list');
    if (!response.ok) {
      setMessage(`读取台标库失败：HTTP ${response.status}`);
      return;
    }
    const data = (await response.json()) as { logos?: Logo[] };
    setLogos(data.logos ?? []);
    if (!getToken()) {
      return;
    }
    const nextReport = await api<Report>('GET', '/logo/match-report');
    setReport(nextReport);
    const nextSuggest = await api<{ channels: Suggestion[] }>('POST', '/logo/suggest', {
      threshold: 0.4,
    });
    const map: Record<string, string> = {};
    for (const item of nextSuggest.channels) {
      const first = item.suggest[0];
      if (first) {
        map[`${item.gid}/${item.cid}`] = first.file;
      }
    }
    setSuggestions(map);
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function upload(files: FileList | null) {
    if (!files || files.length === 0) {
      return;
    }
    try {
      const items = await Promise.all(
        Array.from(files).map(async (file) => ({
          name: file.name,
          data: await readAsDataUrl(file),
        })),
      );
      const result = await api<UploadResult>('POST', '/logo/upload', { items });
      setMessage(`已上传 ${result.added.length} 个，失败 ${result.failed.length} 个`);
      await load();
    } catch (error) {
      setMessage((error as Error).message);
    }
  }

  async function rename(file: string, current: string) {
    const name = window.prompt('新名称（扩展名不变）', current);
    if (!name) {
      return;
    }
    try {
      await api('POST', '/logo/rename', { file, name });
      setMessage('已重命名');
      await load();
    } catch (error) {
      setMessage((error as Error).message);
    }
  }

  async function remove(file: string) {
    if (!window.confirm(`删除 ${file}？引用该文件的频道会变成坏引用。`)) {
      return;
    }
    try {
      await api('POST', '/logo/delete', { file });
      await load();
      setMessage('已删除');
    } catch (error) {
      setMessage((error as Error).message);
    }
  }

  async function apply(channel: Missing) {
    const key = channelKey(channel);
    const file = picked[key] ?? suggestions[key];
    if (!file) {
      setMessage('请选择台标文件');
      return;
    }
    try {
      await api('POST', '/logo/apply', { gid: channel.gid, cid: channel.cid, file });
      setMessage(`已把 ${file} 应用到 ${channel.name}`);
      await load();
    } catch (error) {
      setMessage((error as Error).message);
    }
  }

  return (
    <>
      <div className="card">
        <div className="row" style={{ justifyContent: 'space-between', alignItems: 'center' }}>
          <h1>台标库</h1>
          {authed && (
            <label className="tag" style={{ cursor: 'pointer' }}>
              上传台标
              <input
                type="file"
                accept="image/*"
                multiple
                style={{ display: 'none' }}
                onChange={(event) => void upload(event.target.files)}
              />
            </label>
          )}
        </div>
        <p className="muted">
          共 {logos.length} 个台标。公开地址形如 <code className="mono">/logo/&lt;文件名&gt;</code>。
        </p>
        {report && (
          <p className="muted">
            频道 {report.channels} 个；缺台标 {report.missing} 个；坏引用 {report.broken} 个；未使用{' '}
            {report.unused} 个。
          </p>
        )}
        {message && <p className="msg">{message}</p>}
      </div>

      <div className="card" style={{ marginTop: 16 }}>
        {logos.length === 0 && <p className="muted">台标库为空。</p>}
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(150px, 1fr))',
            gap: 12,
          }}
        >
          {logos.map((logo) => (
            <div key={logo.file} style={{ textAlign: 'center' }}>
              <img
                src={logo.url}
                alt={logo.name}
                style={{ maxWidth: '100%', maxHeight: 80, objectFit: 'contain' }}
              />
              <div className="mono ellipsis" style={{ fontSize: 12 }}>
                {logo.file}
              </div>
              <div className="muted" style={{ fontSize: 12 }}>
                {formatBytes(logo.bytes)}
              </div>
              {authed && (
                <div className="row" style={{ justifyContent: 'center' }}>
                  <button className="ghost" onClick={() => void rename(logo.file, logo.name)}>
                    重命名
                  </button>
                  <button className="icon-btn" onClick={() => void remove(logo.file)}>
                    删除
                  </button>
                </div>
              )}
            </div>
          ))}
        </div>
      </div>

      {authed && report && report.missing_channels.length > 0 && (
        <div className="card" style={{ marginTop: 16 }}>
          <h2>缺台标的频道</h2>
          <table className="table">
            <thead>
              <tr>
                <th>分组</th>
                <th>频道</th>
                <th>台标</th>
                <th className="right">操作</th>
              </tr>
            </thead>
            <tbody>
              {report.missing_channels.slice(0, 100).map((channel) => {
                const key = channelKey(channel);
                return (
                  <tr key={key}>
                    <td className="mono">{channel.gid}</td>
                    <td>{channel.name}</td>
                    <td>
                      <select
                        value={picked[key] ?? suggestions[key] ?? ''}
                        onChange={(event) =>
                          setPicked({ ...picked, [key]: event.target.value })
                        }
                      >
                        <option value="">（选择文件）</option>
                        {logos.map((logo) => (
                          <option key={logo.file} value={logo.file}>
                            {logo.file}
                          </option>
                        ))}
                      </select>
                    </td>
                    <td className="right">
                      <button className="ghost" onClick={() => void apply(channel)}>
                        应用
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}
