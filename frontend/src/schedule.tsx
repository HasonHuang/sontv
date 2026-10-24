/** 录制与回看：任务 CRUD、直播开关、回看缓冲状态。 */

import { useCallback, useEffect, useState } from 'react';

import { api, getToken } from './api';
import type { Channel, Group } from './groups';
import { Badge, useToast } from './ui';

type Task = {
  id: string;
  name: string;
  group_id: string;
  channel_id: string;
  start_time: string;
  end_time: string;
  format: string;
  status: string;
  output_file: string;
  note: string;
  error: string;
  file_size: number;
  record_secs: number;
};

type TimeshiftStatus = {
  enabled: boolean;
  ready: boolean;
  channels: { group_id: string; channel_id: string; hours: number; segments: number }[];
};

const STATUS_TONE: Record<string, string> = {
  pending: 'info',
  recording: 'warn',
  finished: 'ok',
  failed: 'err',
  stopped: 'muted',
};

function sizeText(bytes: number): string {
  if (!bytes) return '—';
  if (bytes >= 1 << 30) return `${(bytes / (1 << 30)).toFixed(1)} GB`;
  if (bytes >= 1 << 20) return `${(bytes / (1 << 20)).toFixed(1)} MB`;
  return `${Math.round(bytes / 1024)} KB`;
}

/** 下载任务产物（带 Bearer）。 */
async function download(name: string): Promise<void> {
  const response = await fetch(`/panel/api/schedule/download/${encodeURIComponent(name)}`, {
    headers: { Authorization: `Bearer ${getToken()}` },
  });
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  const blob = await response.blob();
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  URL.revokeObjectURL(url);
}

export function SchedulePage({
  view = 'all',
}: {
  view?: 'all' | 'schedule' | 'livestream';
} = {}) {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [groups, setGroups] = useState<Group[]>([]);
  const [timeshift, setTimeshift] = useState<TimeshiftStatus | null>(null);
  const [form, setForm] = useState({
    group_id: '',
    channel_id: '',
    name: '',
    start_time: '',
    end_time: '',
    format: 'mkv',
  });
  const { node, ok, err } = useToast();

  const load = useCallback(async () => {
    try {
      const [nextTasks, nextGroups, nextTimeshift] = await Promise.all([
        api<Task[]>('GET', '/schedule'),
        api<Group[]>('GET', '/groups/export'),
        api<TimeshiftStatus>('GET', '/dash-timeshift/status').catch(() => null),
      ]);
      setTasks(nextTasks);
      setGroups(nextGroups);
      setTimeshift(nextTimeshift);
    } catch (error) {
      err(error instanceof Error ? error.message : String(error));
    }
  }, [err]);

  useEffect(() => {
    void load();
  }, [load]);

  const channels = groups.find((group) => group.id === form.group_id)?.channels ?? [];

  const create = async () => {
    if (!form.group_id || !form.channel_id) {
      err('请选择分组与频道');
      return;
    }
    try {
      const payload = {
        ...form,
        name:
          form.name ||
          `${groups.find((group) => group.id === form.group_id)?.name ?? ''}-${form.channel_id}`,
      };
      await api('POST', '/schedule', payload);
      ok('任务已创建');
      await load();
    } catch (error) {
      err(error instanceof Error ? error.message : String(error));
    }
  };

  const stop = async (index: number) => {
    try {
      await api('POST', `/schedule/${index}/stop`);
      ok('已发送停止');
      await load();
    } catch (error) {
      err(error instanceof Error ? error.message : String(error));
    }
  };

  const remove = async (index: number) => {
    try {
      await api('DELETE', `/schedule/${index}`);
      ok('任务已删除');
      await load();
    } catch (error) {
      err(error instanceof Error ? error.message : String(error));
    }
  };

  const toggle = async (group: Group, channel: Channel) => {
    const action = channel.livestream ? 'stop' : 'start';
    try {
      await api('POST', `/livestream/${group.id}/${channel.id}/${action}`);
      ok(action === 'start' ? '已点亮并开始缓冲' : '已停止回看');
      await load();
    } catch (error) {
      err(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <>
      {view !== 'livestream' && (
        <>
          <div className="card">
            <h1>新建录制任务</h1>
            <div className="row">
              <select
                value={form.group_id}
                onChange={(event) =>
                  setForm({ ...form, group_id: event.target.value, channel_id: '' })
                }
              >
                <option value="">选择分组</option>
                {groups.map((group) => (
                  <option key={group.id} value={group.id}>
                    {group.name}（{group.id}）
                  </option>
                ))}
              </select>
              <select
                value={form.channel_id}
                onChange={(event) => setForm({ ...form, channel_id: event.target.value })}
              >
                <option value="">选择频道</option>
                {channels.map((channel) => (
                  <option key={channel.id} value={channel.id}>
                    {channel.name}
                  </option>
                ))}
              </select>
              <input
                placeholder="任务名（可空）"
                value={form.name}
                onChange={(event) => setForm({ ...form, name: event.target.value })}
              />
            </div>
            <div className="row">
              <input
                type="datetime-local"
                value={form.start_time}
                onChange={(event) => setForm({ ...form, start_time: event.target.value })}
              />
              <input
                type="datetime-local"
                value={form.end_time}
                onChange={(event) => setForm({ ...form, end_time: event.target.value })}
              />
              <select
                value={form.format}
                onChange={(event) => setForm({ ...form, format: event.target.value })}
              >
                <option value="mkv">MKV（需 ffmpeg）</option>
                <option value="ts">TS</option>
              </select>
              <button onClick={() => void create()}>创建任务</button>
            </div>
          </div>

          <div className="card">
            <h1>录制任务</h1>
            <table>
              <thead>
                <tr>
                  <th>名称</th>
                  <th>频道</th>
                  <th>时间</th>
                  <th>状态</th>
                  <th>产物</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {tasks.map((task, index) => (
                  <tr key={task.id}>
                    <td>{task.name}</td>
                    <td>
                      {task.group_id}/{task.channel_id}
                    </td>
                    <td className="muted">
                      {task.start_time} → {task.end_time}
                    </td>
                    <td>
                      <Badge tone={STATUS_TONE[task.status] ?? 'muted'}>{task.status}</Badge>
                      {(task.error || task.note) && (
                        <div className="muted">{task.error || task.note}</div>
                      )}
                    </td>
                    <td>
                      {task.output_file ? (
                        <button
                          className="ghost"
                          onClick={() => void download(task.output_file).catch(() => err('下载失败'))}
                        >
                          {sizeText(task.file_size)}
                        </button>
                      ) : (
                        '—'
                      )}
                    </td>
                    <td>
                      {task.status === 'recording' && (
                        <button className="ghost" onClick={() => void stop(index)}>
                          停止
                        </button>
                      )}
                      <button className="icon-btn" onClick={() => void remove(index)}>
                        删除
                      </button>
                    </td>
                  </tr>
                ))}
                {tasks.length === 0 && (
                  <tr>
                    <td colSpan={6} className="muted">
                      暂无任务。
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </>
      )}

      {view !== 'schedule' && (
        <div className="card">
          <h1>直播回看</h1>
          <p className="muted">
            点亮后按频道 `timeshift_hours` 缓冲；回看清单用 `?start&end` 或 `?playseek` 请求入口。
            {timeshift &&
              ` 已点亮 ${timeshift.channels.length} 个频道，已缓冲切片 ${timeshift.channels.reduce(
                (sum, channel) => sum + channel.segments,
                0,
              )} 条。`}
          </p>
          <table>
            <thead>
              <tr>
                <th>分组</th>
                <th>频道</th>
                <th>缓冲</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              {groups.flatMap((group) =>
                (group.channels ?? []).map((channel) => {
                  const state = timeshift?.channels.find(
                    (item) => item.group_id === group.id && item.channel_id === channel.id,
                  );
                  return (
                    <tr key={`${group.id}/${channel.id}`}>
                      <td>{group.name}</td>
                      <td>{channel.name}</td>
                      <td>
                        {channel.livestream ? (
                          <Badge tone="ok">{state ? `${state.segments} 片` : '缓冲中'}</Badge>
                        ) : (
                          <Badge tone="muted">未点亮</Badge>
                        )}
                      </td>
                      <td>
                        <button className="ghost" onClick={() => void toggle(group, channel)}>
                          {channel.livestream ? '停止' : '点亮'}
                        </button>
                      </td>
                    </tr>
                  );
                }),
              )}
            </tbody>
          </table>
        </div>
      )}

      {node}
    </>
  );
}

/** 独立回看管理分区。 */
export function LivestreamPage() {
  return <SchedulePage view="livestream" />;
}
