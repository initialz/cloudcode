import { useEffect, useMemo, useState } from 'react';
import {
  apiClient,
  type DashboardDto,
  type HourlyBucket,
  type LeaderboardRowDto,
  type SessionDurationDto,
  type MessagesPerSessionDto,
  type DailyMessageDto,
  type DailyTokenDto,
} from '@/lib/api';
import { StatCard } from '@/components/StatCard';
import { SessionsChart } from '@/components/SessionsChart';
import { formatDuration, formatCount } from '@/lib/format';

// ── tiny helpers ────────────────────────────────────────────────────────────

type Window = '7d' | '30d';

function WindowToggle({
  value,
  onChange,
}: {
  value: Window;
  onChange: (v: Window) => void;
}) {
  const btn = (v: Window, label: string) => (
    <button
      key={v}
      onClick={() => onChange(v)}
      className={
        'px-2 py-0.5 text-xs rounded transition ' +
        (value === v
          ? 'bg-zinc-800 text-white dark:bg-zinc-200 dark:text-zinc-900'
          : 'text-zinc-500 hover:text-zinc-800 dark:hover:text-zinc-200')
      }
    >
      {label}
    </button>
  );
  return (
    <span className="inline-flex items-center gap-0.5 rounded border border-zinc-200 dark:border-zinc-700 p-0.5">
      {btn('7d', '7d')}
      {btn('30d', '30d')}
    </span>
  );
}

function SectionError({ msg }: { msg: string }) {
  return (
    <div className="rounded border-l-2 border-red-500 bg-red-50 dark:bg-red-950/30 px-3 py-2 text-sm text-red-700 dark:text-red-300">
      {msg}
    </div>
  );
}

function SectionLoading() {
  return <div className="text-sm text-zinc-500">Loading…</div>;
}

// ── niceTicks helper (shared with chart math) ───────────────────────────────

function niceTicks(max: number): number[] {
  if (max === 0) return [0];
  if (max < 4) {
    const out: number[] = [];
    for (let v = 0; v <= Math.max(4, max); v++) out.push(v);
    return out;
  }
  const exp = Math.floor(Math.log10(max));
  const pow = Math.pow(10, exp);
  const ratio = max / pow;
  const step = ratio > 5 ? pow * 2 : ratio > 2 ? pow : pow / 2;
  const top = Math.ceil(max / step) * step;
  const ticks: number[] = [];
  for (let v = 0; v <= top + 1e-9; v += step) ticks.push(Math.round(v));
  return ticks;
}

// ── Leaderboard section ──────────────────────────────────────────────────────

function LeaderboardSection({ group }: { group: 'account' | 'agent' }) {
  const title = group === 'account' ? 'Top accounts' : 'Top agents';
  const [win, setWin] = useState<Window>('7d');
  const [data, setData] = useState<LeaderboardRowDto[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setErr(null);
    apiClient.stats
      .leaderboard(win, group)
      .then((d) => {
        if (!cancelled) { setData(d); setLoading(false); }
      })
      .catch((e: any) => {
        if (!cancelled) { setErr(e?.message ?? 'load failed'); setLoading(false); }
      });
    return () => { cancelled = true; };
  }, [win, group]);

  return (
    <section className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 bg-white dark:bg-zinc-900">
      <header className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium">{title}</h3>
        <WindowToggle value={win} onChange={setWin} />
      </header>

      {loading && <SectionLoading />}
      {err && <SectionError msg={err} />}
      {!loading && !err && data && (
        data.length === 0 ? (
          <p className="text-sm text-zinc-400">No data for this period.</p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-xs uppercase tracking-wide text-zinc-500 border-b border-zinc-100 dark:border-zinc-800">
                  <th className="pb-1 text-left w-8">#</th>
                  <th className="pb-1 text-left">Name</th>
                  <th className="pb-1 text-right">Sessions</th>
                  <th className="pb-1 text-right">Duration</th>
                  <th className="pb-1 text-right">Messages</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-zinc-100 dark:divide-zinc-800">
                {data.map((row, i) => (
                  <tr key={row.name}>
                    <td className="py-1.5 pr-2 text-zinc-400 tabular-nums text-xs">
                      #{i + 1}
                    </td>
                    <td className="py-1.5 font-mono truncate max-w-[140px]" title={row.name}>
                      {row.name}
                    </td>
                    <td className="py-1.5 text-right tabular-nums">{row.session_count}</td>
                    <td className="py-1.5 text-right tabular-nums text-zinc-600 dark:text-zinc-400">
                      {formatDuration(row.total_duration_seconds)}
                    </td>
                    <td className="py-1.5 text-right tabular-nums">{formatCount(row.message_count)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )
      )}
    </section>
  );
}

// ── horizontal bar row ───────────────────────────────────────────────────────

function BucketBar({
  label,
  count,
  maxCount,
  countLabel,
}: {
  label: string;
  count: number;
  maxCount: number;
  countLabel: string;
}) {
  const pct = maxCount > 0 ? (count / maxCount) * 100 : 0;
  return (
    <div className="flex items-center gap-2 text-sm">
      <span className="w-14 shrink-0 text-right text-xs text-zinc-500 tabular-nums">
        {label}
      </span>
      <div className="flex-1 bg-zinc-100 dark:bg-zinc-800 rounded-sm h-2 overflow-hidden">
        <div
          className="h-2 bg-blue-500 dark:bg-blue-400 rounded-sm"
          style={{ width: `${pct}%` }}
        />
      </div>
      <span className="w-12 shrink-0 text-xs tabular-nums text-zinc-600 dark:text-zinc-400">
        {countLabel}
      </span>
    </div>
  );
}

// ── stat strip ───────────────────────────────────────────────────────────────

function StatStrip({ items }: { items: { label: string; value: string }[] }) {
  return (
    <div className="flex flex-wrap gap-x-5 gap-y-1 mb-3 text-xs">
      {items.map(({ label, value }) => (
        <span key={label} className="flex items-baseline gap-1">
          <span className="text-zinc-400">{label}</span>
          <span className="font-medium tabular-nums">{value}</span>
        </span>
      ))}
    </div>
  );
}

// ── Session duration distribution ────────────────────────────────────────────

function SessionDurationSection() {
  const [win, setWin] = useState<Window>('7d');
  const [data, setData] = useState<SessionDurationDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setErr(null);
    apiClient.stats
      .sessionDuration(win)
      .then((d) => {
        if (!cancelled) { setData(d); setLoading(false); }
      })
      .catch((e: any) => {
        if (!cancelled) { setErr(e?.message ?? 'load failed'); setLoading(false); }
      });
    return () => { cancelled = true; };
  }, [win]);

  const maxCount = data ? Math.max(0, ...data.buckets.map((b) => b.count)) : 0;

  return (
    <section className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 bg-white dark:bg-zinc-900">
      <header className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium">Session duration distribution</h3>
        <WindowToggle value={win} onChange={setWin} />
      </header>

      {loading && <SectionLoading />}
      {err && <SectionError msg={err} />}
      {!loading && !err && data && (
        <>
          <StatStrip
            items={[
              { label: 'sessions', value: formatCount(data.count) },
              { label: 'mean', value: formatDuration(data.mean_seconds) },
              { label: 'median', value: formatDuration(data.median_seconds) },
              { label: 'p95', value: formatDuration(data.p95_seconds) },
              { label: 'max', value: formatDuration(data.max_seconds) },
            ]}
          />
          <div className="space-y-1.5">
            {data.buckets.map((b) => (
              <BucketBar
                key={b.label}
                label={b.label}
                count={b.count}
                maxCount={maxCount}
                countLabel={formatCount(b.count)}
              />
            ))}
          </div>
        </>
      )}
    </section>
  );
}

// ── Messages per session distribution ───────────────────────────────────────

function MessagesPerSessionSection() {
  const [win, setWin] = useState<Window>('7d');
  const [data, setData] = useState<MessagesPerSessionDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setErr(null);
    apiClient.stats
      .messagesPerSession(win)
      .then((d) => {
        if (!cancelled) { setData(d); setLoading(false); }
      })
      .catch((e: any) => {
        if (!cancelled) { setErr(e?.message ?? 'load failed'); setLoading(false); }
      });
    return () => { cancelled = true; };
  }, [win]);

  const maxCount = data ? Math.max(0, ...data.buckets.map((b) => b.count)) : 0;

  return (
    <section className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 bg-white dark:bg-zinc-900">
      <header className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium">Messages per session distribution</h3>
        <WindowToggle value={win} onChange={setWin} />
      </header>

      {loading && <SectionLoading />}
      {err && <SectionError msg={err} />}
      {!loading && !err && data && (
        <>
          <StatStrip
            items={[
              { label: 'sessions', value: formatCount(data.count) },
              { label: 'mean', value: String(Math.round(data.mean)) },
              { label: 'median', value: String(data.median) },
              { label: 'p95', value: String(data.p95) },
              { label: 'max', value: String(data.max) },
            ]}
          />
          <div className="space-y-1.5">
            {data.buckets.map((b) => (
              <BucketBar
                key={b.label}
                label={b.label}
                count={b.count}
                maxCount={maxCount}
                countLabel={formatCount(b.count)}
              />
            ))}
          </div>
        </>
      )}
    </section>
  );
}

// ── Stacked bar SVG chart ────────────────────────────────────────────────────

const CW_CHART = 720;
const CH_CHART = 180;
const PAD_L = 42;
const PAD_R = 8;
const PAD_T = 10;
const PAD_B = 28;
const INNER_W = CW_CHART - PAD_L - PAD_R;
const INNER_H = CH_CHART - PAD_T - PAD_B;

type BarSeries = { key: string; values: number[]; className: string };

interface StackedBarChartProps {
  labels: string[];           // x-axis date labels
  series: BarSeries[];        // stacked bottom→top order
  yFormatter?: (n: number) => string;
  tooltipLines: (i: number) => string[];
  xTickEvery?: number;
}

function StackedBarChart({
  labels,
  series,
  yFormatter = (n) => String(n),
  tooltipLines,
  xTickEvery = 5,
}: StackedBarChartProps) {
  const n = labels.length;
  if (n === 0) return null;

  // compute stacked totals per index
  const totals = Array.from({ length: n }, (_, i) =>
    series.reduce((s, sr) => s + (sr.values[i] ?? 0), 0),
  );
  const dataMax = Math.max(0, ...totals);
  const yTicks = niceTicks(dataMax);
  const yMax = yTicks[yTicks.length - 1] || 1;

  const barW = Math.max(2, INNER_W / n - 1);
  const xOf = (i: number) => PAD_L + (INNER_W * i) / Math.max(1, n - 1) - barW / 2;
  const yOf = (v: number) => PAD_T + INNER_H - (INNER_H * v) / yMax;
  const hOf = (v: number) => (INNER_H * v) / yMax;

  return (
    <svg
      viewBox={`0 0 ${CW_CHART} ${CH_CHART}`}
      className="w-full h-48 text-zinc-900 dark:text-zinc-100"
      preserveAspectRatio="none"
    >
      {/* y gridlines */}
      {yTicks.map((y, i) => (
        <g key={i}>
          <line
            x1={PAD_L}
            y1={yOf(y)}
            x2={CW_CHART - PAD_R}
            y2={yOf(y)}
            stroke="currentColor"
            strokeOpacity="0.08"
          />
          <text
            x={PAD_L - 5}
            y={yOf(y)}
            dy="0.32em"
            textAnchor="end"
            fontSize="10"
            fill="currentColor"
            fillOpacity="0.5"
          >
            {yFormatter(y)}
          </text>
        </g>
      ))}

      {/* x-axis ticks */}
      {labels.map((lbl, i) => {
        if (i % xTickEvery !== 0 && i !== n - 1) return null;
        return (
          <text
            key={i}
            x={xOf(i) + barW / 2}
            y={CH_CHART - 8}
            textAnchor="middle"
            fontSize="10"
            fill="currentColor"
            fillOpacity="0.5"
          >
            {lbl.slice(5)} {/* MM-DD */}
          </text>
        );
      })}

      {/* stacked bars */}
      {Array.from({ length: n }, (_, i) => {
        let accumulated = 0;
        const tooltip = tooltipLines(i).join('\n');
        return (
          <g key={i}>
            {series.map((sr) => {
              const v = sr.values[i] ?? 0;
              const h = hOf(v);
              const y = yOf(accumulated + v);
              accumulated += v;
              if (h < 0.5) return null;
              return (
                <rect
                  key={sr.key}
                  x={xOf(i)}
                  y={y}
                  width={barW}
                  height={h}
                  className={sr.className}
                />
              );
            })}
            {/* invisible full-height hit area for tooltip */}
            <rect
              x={xOf(i)}
              y={PAD_T}
              width={barW}
              height={INNER_H}
              fill="transparent"
            >
              <title>{tooltip}</title>
            </rect>
          </g>
        );
      })}

      {/* baseline */}
      <line
        x1={PAD_L}
        y1={PAD_T + INNER_H}
        x2={CW_CHART - PAD_R}
        y2={PAD_T + INNER_H}
        stroke="currentColor"
        strokeOpacity="0.2"
      />
    </svg>
  );
}

// ── Daily messages trend ─────────────────────────────────────────────────────

function DailyMessagesSection() {
  const [data, setData] = useState<DailyMessageDto[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    apiClient.stats
      .messagesDaily(30)
      .then((d) => {
        if (!cancelled) { setData(d); setLoading(false); }
      })
      .catch((e: any) => {
        if (!cancelled) { setErr(e?.message ?? 'load failed'); setLoading(false); }
      });
    return () => { cancelled = true; };
  }, []);

  const { labels, seriesUser, seriesAssistant, seriesOther } = useMemo(() => {
    if (!data) return { labels: [], seriesUser: [], seriesAssistant: [], seriesOther: [] };
    return {
      labels: data.map((d) => d.date),
      seriesUser: data.map((d) => d.user),
      seriesAssistant: data.map((d) => d.assistant),
      seriesOther: data.map((d) => d.other),
    };
  }, [data]);

  return (
    <section className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 bg-white dark:bg-zinc-900 lg:col-span-2">
      <header className="flex items-center justify-between mb-2">
        <h3 className="text-sm font-medium">Daily messages — last 30 days</h3>
        <span className="flex items-center gap-3 text-xs text-zinc-400">
          <span className="flex items-center gap-1">
            <span className="inline-block w-2.5 h-2 rounded-sm bg-blue-500 dark:bg-blue-400 opacity-90" />
            user
          </span>
          <span className="flex items-center gap-1">
            <span className="inline-block w-2.5 h-2 rounded-sm bg-emerald-500 dark:bg-emerald-400 opacity-90" />
            assistant
          </span>
          <span className="flex items-center gap-1">
            <span className="inline-block w-2.5 h-2 rounded-sm bg-zinc-400 dark:bg-zinc-500 opacity-80" />
            other
          </span>
        </span>
      </header>

      {loading && <SectionLoading />}
      {err && <SectionError msg={err} />}
      {!loading && !err && data && (
        <StackedBarChart
          labels={labels}
          series={[
            { key: 'other', values: seriesOther, className: 'fill-zinc-400 dark:fill-zinc-500 opacity-80' },
            { key: 'assistant', values: seriesAssistant, className: 'fill-emerald-500 dark:fill-emerald-400 opacity-90' },
            { key: 'user', values: seriesUser, className: 'fill-blue-500 dark:fill-blue-400 opacity-90' },
          ]}
          yFormatter={formatCount}
          tooltipLines={(i) => {
            const d = data[i];
            if (!d) return [''];
            return [
              d.date,
              `user: ${d.user}`,
              `assistant: ${d.assistant}`,
              `other: ${d.other}`,
            ];
          }}
          xTickEvery={5}
        />
      )}
    </section>
  );
}

// ── Daily token usage ────────────────────────────────────────────────────────

function DailyTokensSection() {
  const [data, setData] = useState<DailyTokenDto[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    apiClient.stats
      .tokensDaily(30)
      .then((d) => {
        if (!cancelled) { setData(d); setLoading(false); }
      })
      .catch((e: any) => {
        if (!cancelled) { setErr(e?.message ?? 'load failed'); setLoading(false); }
      });
    return () => { cancelled = true; };
  }, []);

  const { labels, seriesInput, seriesOutput } = useMemo(() => {
    if (!data) return { labels: [], seriesInput: [], seriesOutput: [] };
    return {
      labels: data.map((d) => d.date),
      seriesInput: data.map((d) => d.input_tokens),
      seriesOutput: data.map((d) => d.output_tokens),
    };
  }, [data]);

  return (
    <section className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 bg-white dark:bg-zinc-900 lg:col-span-2">
      <header className="flex items-center justify-between mb-1">
        <h3 className="text-sm font-medium">Daily token usage — last 30 days</h3>
        <span className="flex items-center gap-3 text-xs text-zinc-400">
          <span className="flex items-center gap-1">
            <span className="inline-block w-2.5 h-2 rounded-sm bg-sky-400 dark:bg-sky-300 opacity-90" />
            input
          </span>
          <span className="flex items-center gap-1">
            <span className="inline-block w-2.5 h-2 rounded-sm bg-orange-500 dark:bg-orange-400 opacity-90" />
            output
          </span>
        </span>
      </header>
      <p className="text-xs text-zinc-400 mb-2">
        input / output tokens by day (assistant turns only) — hover for cache detail
      </p>

      {loading && <SectionLoading />}
      {err && <SectionError msg={err} />}
      {!loading && !err && data && (
        <StackedBarChart
          labels={labels}
          series={[
            { key: 'input', values: seriesInput, className: 'fill-sky-400 dark:fill-sky-300 opacity-90' },
            { key: 'output', values: seriesOutput, className: 'fill-orange-500 dark:fill-orange-400 opacity-90' },
          ]}
          yFormatter={formatCount}
          tooltipLines={(i) => {
            const d = data[i];
            if (!d) return [''];
            return [
              d.date,
              `input: ${formatCount(d.input_tokens)}`,
              `output: ${formatCount(d.output_tokens)}`,
              `cache creation: ${formatCount(d.cache_creation_tokens)}`,
              `cache read: ${formatCount(d.cache_read_tokens)}`,
            ];
          }}
          xTickEvery={5}
        />
      )}
    </section>
  );
}

// ── Dashboard ────────────────────────────────────────────────────────────────

export function Dashboard() {
  const [dash, setDash] = useState<DashboardDto | null>(null);
  const [hourly, setHourly] = useState<HourlyBucket[] | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([apiClient.dashboard(), apiClient.sessionsHourly(24)])
      .then(([d, h]) => {
        if (cancelled) return;
        setDash(d);
        setHourly(h);
      })
      .catch((e: any) => {
        if (cancelled) return;
        setErr(e?.message ?? 'load failed');
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (err) {
    return (
      <div className="rounded border-l-2 border-red-500 bg-red-50 dark:bg-red-950/30 px-3 py-2 text-sm text-red-700 dark:text-red-300">
        {err}
      </div>
    );
  }
  if (!dash || !hourly) {
    return <div className="text-sm text-zinc-500">Loading…</div>;
  }

  const agentSub =
    dash.online_agents.length === 0
      ? <span className="text-zinc-400">(none connected)</span>
      : dash.online_agents.join(', ');

  return (
    <div className="space-y-6">
      {/* top stat cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        <StatCard label="Accounts" value={dash.accounts} link="/accounts" />
        <StatCard
          label="Active sessions"
          value={dash.active_sessions}
          sub="live now"
          link="/sessions?active=1"
        />
        <StatCard
          label="Sessions (24h)"
          value={dash.sessions_24h}
          link="/sessions"
        />
        <StatCard
          label="Online agents"
          value={dash.online_agents.length}
          sub={agentSub}
        />
      </div>

      {/* hourly sessions chart */}
      <section className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 bg-white dark:bg-zinc-900">
        <header className="flex items-baseline justify-between mb-2">
          <h3 className="text-sm font-medium">Sessions started — last 24 hours</h3>
          <span className="text-xs text-zinc-500">UTC</span>
        </header>
        <SessionsChart data={hourly} hours={24} />
      </section>

      {/* stats sections grid */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <LeaderboardSection group="account" />
        <LeaderboardSection group="agent" />
        <SessionDurationSection />
        <MessagesPerSessionSection />
        <DailyMessagesSection />
        <DailyTokensSection />
      </div>
    </div>
  );
}
