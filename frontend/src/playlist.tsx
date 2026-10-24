/** 订阅聚合：管理 `[playlist]` 源与过滤规则。 */

import { useCallback, useEffect, useState } from 'react';

import { api, copyText } from './api';
import { useToast } from './ui';

type Source = {
  id: string;
  name: string;
  url: string;
  enabled: boolean;
  ttl_secs: number;
  group_mode: string;
  merge_group: string;
  unlisted_groups: string;
  group_map: Record<string, string>;
  include: string[];
  exclude: string[];
};

function newSource(): Source {
  return {
    id: '',
    name: '',
    url: '',
    enabled: true,
    ttl_secs: 0,
    group_mode: 'keep',
    merge_group: '',
    unlisted_groups: 'drop',
    group_map: {},
    include: [],
    exclude: [],
  };
}

function lines(value: string): string[] {
  return value
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
}

function mapText(map: Record<string, string>): string {
  return Object.entries(map)
    .map(([from, to]) => `${from}=${to}`)
    .join('\n');
}

function parseMap(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of lines(text)) {
    const index = line.indexOf('=');
    if (index <= 0) continue;
    out[line.slice(0, index).trim()] = line.slice(index + 1).trim();
  }
  return out;
}

type PlaylistCfg = {
  merge_ttl_secs: number;
  merge_position: string;
  global_exclude: string[];
  sources: Source[];
};

const EMPTY: PlaylistCfg = {
  merge_ttl_secs: 300,
  merge_position: 'bottom',
  global_exclude: [],
  sources: [],
};

export function PlaylistPage() {
  const [cfg, setCfg] = useState<PlaylistCfg>(EMPTY);
  const [excludeText, setExcludeText] = useState('');
  const [preview, setPreview] = useState('');
  const { node, ok, err } = useToast();

  const load = useCallback(async () => {
    try {
      const next = await api<PlaylistCfg>('GET', '/playlist');
      setCfg({
        ...EMPTY,
        ...next,
        sources: (next.sources ?? []).map((source) => ({ ...newSource(), ...source })),
      });
      setExcludeText((next.global_exclude ?? []).join('\n'));
    } catch (error) {
      err(error instanceof Error ? error.message : String(error));
    }
  }, [err]);

  useEffect(() => {
    void load();
  }, [load]);

  const save = async () => {
    try {
      const body: PlaylistCfg = {
        ...cfg,
        global_exclude: excludeText
          .split('\n')
          .map((line) => line.trim())
          .filter(Boolean),
      };
      await api('PUT', '/playlist', body);
      ok('聚合配置已保存');
      await load();
    } catch (error) {
      err(error instanceof Error ? error.message : String(error));
    }
  };

  const updateSource = (index: number, patch: Partial<Source>) => {
    const sources = cfg.sources.map((source, current) =>
      current === index ? { ...source, ...patch } : source,
    );
    setCfg({ ...cfg, sources });
  };

  const runPreview = async (source: Source) => {
    setPreview('预检中…');
    try {
      const result = await api<unknown>('POST', '/subs/preview', {
        url: source.url,
        group_map: source.group_map,
        group_mode: source.group_mode,
        merge_group: source.merge_group,
        unlisted_groups: source.unlisted_groups,
        include: source.include,
        exclude: source.exclude,
      });
      setPreview(JSON.stringify(result, null, 2));
    } catch (error) {
      setPreview(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <>
      <div className="card">
        <div className="row between">
          <div>
            <h1>订阅聚合</h1>
            <p className="muted">
              聚合清单：<code>{location.origin}/playlist.m3u</code>
            </p>
          </div>
          <div className="row">
            <button
              className="ghost"
              onClick={() =>
                void copyText(`${location.origin}/playlist.m3u`).then((done) =>
                  done ? ok('已复制聚合地址') : err('复制失败'),
                )
              }
            >
              复制地址
            </button>
            <button onClick={() => void save()}>保存</button>
          </div>
        </div>
        <div className="row">
          <label>
            缓存秒数
            <input
              type="number"
              value={cfg.merge_ttl_secs}
              onChange={(event) =>
                setCfg({ ...cfg, merge_ttl_secs: Number(event.target.value) || 300 })
              }
            />
          </label>
          <label>
            合并位置
            <select
              value={cfg.merge_position}
              onChange={(event) => setCfg({ ...cfg, merge_position: event.target.value })}
            >
              <option value="bottom">底部</option>
              <option value="top">顶部</option>
            </select>
          </label>
        </div>
      </div>

      <div className="card">
        <div className="row between">
          <h2>聚合源</h2>
          <button
            className="ghost"
            onClick={() =>
              setCfg({ ...cfg, sources: [...cfg.sources, newSource()] })
            }
          >
            添加源
          </button>
        </div>
        <table>
          <thead>
            <tr>
              <th>启用</th>
              <th>名称</th>
              <th>URL</th>
              <th>TTL（秒）</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {cfg.sources.map((source, index) => (
              <tr key={source.id || index}>
                <td>
                  <input
                    type="checkbox"
                    checked={source.enabled}
                    onChange={(event) => updateSource(index, { enabled: event.target.checked })}
                  />
                </td>
                <td>
                  <input
                    value={source.name}
                    onChange={(event) => updateSource(index, { name: event.target.value })}
                  />
                </td>
                <td>
                  <input
                    value={source.url}
                    placeholder="http(s)://…/playlist.m3u"
                    onChange={(event) => updateSource(index, { url: event.target.value })}
                  />
                </td>
                <td>
                  <input
                    type="number"
                    min={0}
                    value={source.ttl_secs}
                    onChange={(event) =>
                      updateSource(index, { ttl_secs: Number(event.target.value) || 0 })
                    }
                  />
                </td>
                <td>
                  <button
                    className="ghost"
                    onClick={() => void runPreview(source)}
                  >
                    预检
                  </button>
                  <button
                    className="icon-btn"
                    onClick={() =>
                      setCfg({
                        ...cfg,
                        sources: cfg.sources.filter((_, current) => current !== index),
                      })
                    }
                  >
                    删除
                  </button>
                </td>
              </tr>
            ))}
            {cfg.sources.length === 0 && (
              <tr>
                <td colSpan={5} className="muted">
                  暂无聚合源。
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <div className="card">
        <h2>源规则</h2>
        <p className="muted">
          分组映射每行 `原分组=目标分组`；include/exclude 每行一条，支持 `group:` 与 `id:` 前缀。
        </p>
        {cfg.sources.map((source, index) => (
          <details key={source.id || index} style={{ marginTop: 12 }}>
            <summary>{source.name || source.id || `源 ${index + 1}`}</summary>
            <div className="row">
              <label>
                模式
                <select
                  value={source.group_mode}
                  onChange={(event) =>
                    updateSource(index, { group_mode: event.target.value })
                  }
                >
                  <option value="keep">keep</option>
                  <option value="map">map</option>
                  <option value="merge">merge</option>
                </select>
              </label>
              <label>
                合并分组
                <input
                  value={source.merge_group}
                  onChange={(event) =>
                    updateSource(index, { merge_group: event.target.value })
                  }
                />
              </label>
              <label>
                未列出分组
                <select
                  value={source.unlisted_groups}
                  onChange={(event) =>
                    updateSource(index, { unlisted_groups: event.target.value })
                  }
                >
                  <option value="drop">drop</option>
                  <option value="keep">keep</option>
                </select>
              </label>
            </div>
            <label>
              分组映射
              <textarea
                value={mapText(source.group_map)}
                onChange={(event) =>
                  updateSource(index, { group_map: parseMap(event.target.value) })
                }
              />
            </label>
            <div className="row">
              <label>
                include
                <textarea
                  value={source.include.join('\n')}
                  onChange={(event) =>
                    updateSource(index, { include: lines(event.target.value) })
                  }
                />
              </label>
              <label>
                exclude
                <textarea
                  value={source.exclude.join('\n')}
                  onChange={(event) =>
                    updateSource(index, { exclude: lines(event.target.value) })
                  }
                />
              </label>
            </div>
          </details>
        ))}
        {cfg.sources.length === 0 && <p className="muted">暂无聚合源。</p>}
      </div>

      <div className="card">
        <h2>全局排除</h2>
        <p className="muted">每行一个关键词，匹配频道名。</p>
        <textarea
          value={excludeText}
          onChange={(event) => setExcludeText(event.target.value)}
          spellCheck={false}
        />
      </div>

      {preview && (
        <div className="card">
          <h2>预检结果</h2>
          <pre className="log">{preview}</pre>
        </div>
      )}

      {node}
    </>
  );
}
