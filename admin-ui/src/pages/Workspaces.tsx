// v1.13: hub-canonical workspace inventory (per-account, global).
// Read-only listing. Columns: account | name | locked_by_agent | last_sync_at | size_bytes.

import { useEffect, useMemo, useState } from 'react';
import { apiClient, type WorkspaceRowDto } from '@/lib/api';
import { formatRelative } from '@/lib/time';

const ALL = '__all__';

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function Workspaces() {
  const [rows, setRows] = useState<WorkspaceRowDto[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [accountFilter, setAccountFilter] = useState<string>(ALL);
  const [lockFilter, setLockFilter] = useState<'all' | 'free' | 'locked'>('all');

  async function reload() {
    setErr(null);
    try {
      const list = await apiClient.workspaces.list();
      setRows(list);
    } catch (e: any) {
      setErr(e?.message ?? 'failed to load workspaces');
    }
  }

  useEffect(() => {
    reload();
  }, []);

  const accounts = useMemo(() => {
    if (!rows) return [];
    return Array.from(new Set(rows.map((r) => r.account))).sort();
  }, [rows]);

  const filtered = useMemo(() => {
    if (!rows) return [];
    return rows.filter((r) => {
      if (accountFilter !== ALL && r.account !== accountFilter) return false;
      if (lockFilter === 'free' && r.locked_by_agent !== null) return false;
      if (lockFilter === 'locked' && r.locked_by_agent === null) return false;
      return true;
    });
  }, [rows, accountFilter, lockFilter]);

  const totals = useMemo(() => {
    if (!rows) return { free: 0, locked: 0 };
    return {
      free: rows.filter((r) => r.locked_by_agent === null).length,
      locked: rows.filter((r) => r.locked_by_agent !== null).length,
    };
  }, [rows]);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-base font-semibold">Workspaces</h2>
        <button
          onClick={reload}
          className="px-3 py-1.5 text-sm rounded border border-zinc-300 dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800"
        >
          Refresh
        </button>
      </div>

      {err && (
        <div className="rounded border-l-2 border-red-500 bg-red-50 dark:bg-red-950/30 px-3 py-2 text-sm text-red-700 dark:text-red-300">
          {err}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-3 text-sm">
        <label className="flex items-center gap-2">
          <span className="text-zinc-500">Account</span>
          <select
            value={accountFilter}
            onChange={(e) => setAccountFilter(e.target.value)}
            className="px-2 py-1 rounded border border-zinc-300 dark:border-zinc-700 bg-transparent"
          >
            <option value={ALL}>all</option>
            {accounts.map((a) => (
              <option key={a} value={a}>
                {a}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-2">
          <span className="text-zinc-500">Lock</span>
          <select
            value={lockFilter}
            onChange={(e) => setLockFilter(e.target.value as 'all' | 'free' | 'locked')}
            className="px-2 py-1 rounded border border-zinc-300 dark:border-zinc-700 bg-transparent"
          >
            <option value="all">all</option>
            <option value="free">free</option>
            <option value="locked">locked</option>
          </select>
        </label>
        <div className="ml-auto text-xs text-zinc-500">
          {filtered.length} shown · {totals.locked} locked · {totals.free} free
        </div>
      </div>

      {rows === null ? (
        <div className="text-sm text-zinc-500">Loading...</div>
      ) : (
        <div className="overflow-x-auto rounded-lg border border-zinc-200 dark:border-zinc-800">
          <table className="w-full text-sm">
            <thead className="bg-zinc-50 dark:bg-zinc-900/50 text-xs uppercase tracking-wide text-zinc-500">
              <tr>
                <th className="px-3 py-2 text-left">Account</th>
                <th className="px-3 py-2 text-left">Workspace</th>
                <th className="px-3 py-2 text-left">Lock status</th>
                <th className="px-3 py-2 text-left">Last sync</th>
                <th className="px-3 py-2 text-right">Size</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-zinc-200 dark:divide-zinc-800 bg-white dark:bg-zinc-900">
              {filtered.length === 0 ? (
                <tr>
                  <td
                    colSpan={5}
                    className="px-3 py-6 text-center text-zinc-500"
                  >
                    No workspaces match the current filter.
                  </td>
                </tr>
              ) : (
                filtered.map((r) => (
                  <tr key={`${r.account}|${r.name}`}>
                    <td className="px-3 py-2 font-mono">{r.account}</td>
                    <td className="px-3 py-2 font-mono">{r.name}</td>
                    <td className="px-3 py-2">
                      <LockBadge lockedBy={r.locked_by_agent} />
                    </td>
                    <td className="px-3 py-2 text-zinc-500">
                      {r.last_sync_at ? formatRelative(r.last_sync_at) : '—'}
                    </td>
                    <td className="px-3 py-2 text-zinc-500 text-right font-mono">
                      {formatBytes(r.size_bytes)}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function LockBadge({ lockedBy }: { lockedBy: string | null }) {
  if (!lockedBy) {
    return <span className="text-xs text-zinc-400">free</span>;
  }
  return (
    <span className="inline-flex items-center gap-1.5 text-xs">
      <span className="text-amber-500">●</span>
      <span className="font-mono text-zinc-700 dark:text-zinc-300">{lockedBy}</span>
    </span>
  );
}
