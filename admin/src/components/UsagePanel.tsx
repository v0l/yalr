import { useEffect, useMemo, useState } from 'react'
import { Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import StatCard from '@/components/StatCard'
import { cn } from '@/lib/utils'
import { formatTokens } from '@/lib/requestLog'
import type { UsageReport } from '@/types'

const RANGES = [
  { label: '24h', hours: 24 },
  { label: '3d', hours: 72 },
  { label: '7d', hours: 168 },
] as const

const METRICS = [
  { id: 'tokens', label: 'Tokens' },
  { id: 'cost', label: 'Cost' },
  { id: 'requests', label: 'Requests' },
] as const

type Metric = (typeof METRICS)[number]['id']

const HOUR_MS = 3_600_000

export function formatSats(msat: number): string {
  const value = msat / 1000
  const digits = value !== 0 && Math.abs(value) < 10 ? 3 : 0
  return `${value.toLocaleString(undefined, { minimumFractionDigits: digits, maximumFractionDigits: digits })} sats`
}

function hourStart(ms: number): number {
  return Math.floor(ms / HOUR_MS) * HOUR_MS
}

interface Bucket {
  at: number
  label: string
  input: number
  output: number
  cost: number
  requests: number
}

function buckets(report: UsageReport): Bucket[] {
  const byHour = new Map(report.by_hour.map(h => [Date.parse(h.hour), h]))
  const end = hourStart(Date.now())
  const out: Bucket[] = []
  for (let i = report.hours - 1; i >= 0; i--) {
    const at = end - i * HOUR_MS
    const row = byHour.get(at)
    const d = new Date(at)
    out.push({
      at,
      label: report.hours <= 24
        ? d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
        : `${d.toLocaleDateString([], { month: 'short', day: 'numeric' })} ${d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`,
      input: row?.prompt_tokens ?? 0,
      output: row?.completion_tokens ?? 0,
      cost: (row?.cost_msat ?? 0) / 1000,
      requests: row?.requests ?? 0,
    })
  }
  return out
}

const th = 'px-3 py-2 font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground'
const td = 'px-3 py-2 font-mono text-[12px] tabular-nums'

export default function UsagePanel({
  load,
  scopeKey,
  title = 'Usage',
}: {
  load: (hours: number) => Promise<UsageReport>
  scopeKey: string | number
  title?: string
}) {
  const [hours, setHours] = useState<number>(24)
  const [metric, setMetric] = useState<Metric>('tokens')
  const [report, setReport] = useState<UsageReport | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    setReport(null)
    load(hours)
      .then(r => { if (!cancelled) { setReport(r); setError(null) } })
      .catch(e => { if (!cancelled) setError(e instanceof Error ? e.message : String(e)) })
    return () => { cancelled = true }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hours, scopeKey])

  const data = useMemo(() => (report ? buckets(report) : []), [report])
  const total = report?.total

  return (
    <div className="panel space-y-4 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="font-mono text-[10px] uppercase tracking-[0.12em] text-muted-foreground">{title}</h2>
        <div className="flex flex-wrap items-center gap-2">
          <div className="flex border border-border">
            {METRICS.map(m => (
              <button
                key={m.id}
                onClick={() => setMetric(m.id)}
                className={cn('h-7 px-2.5 font-mono text-[10px] uppercase tracking-wider transition-colors', metric === m.id ? 'bg-brand/10 text-brand' : 'text-muted-foreground hover:bg-surface hover:text-foreground')}
              >
                {m.label}
              </button>
            ))}
          </div>
          <div className="flex border border-border">
            {RANGES.map(r => (
              <button
                key={r.hours}
                onClick={() => setHours(r.hours)}
                className={cn('h-7 px-2.5 font-mono text-[10px] uppercase tracking-wider transition-colors', hours === r.hours ? 'bg-brand/10 text-brand' : 'text-muted-foreground hover:bg-surface hover:text-foreground')}
              >
                {r.label}
              </button>
            ))}
          </div>
        </div>
      </div>

      {error && (
        <Alert className="border-destructive/30 bg-destructive/5 font-mono text-[13px] text-destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      <div className="grid grid-cols-2 gap-2 lg:grid-cols-4">
        <StatCard label="Requests" value={total ? total.requests.toLocaleString() : '-'} />
        <StatCard label="Input tokens" value={total ? formatTokens(total.prompt_tokens) : '-'} sub={total && total.cached_tokens ? `${formatTokens(total.cached_tokens)} cached` : undefined} />
        <StatCard label="Output tokens" value={total ? formatTokens(total.completion_tokens) : '-'} />
        <StatCard label="Cost" value={total ? formatSats(total.cost_msat) : '-'} color="green" />
      </div>

      <div className="h-56">
        {!report ? <Skeleton className="h-full w-full bg-secondary" /> : (
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={data} margin={{ top: 4, right: 4, left: 0, bottom: 0 }}>
              <CartesianGrid stroke="var(--border)" strokeDasharray="2 4" vertical={false} />
              <XAxis dataKey="label" tick={{ fill: 'var(--muted-foreground)', fontSize: 10 }} tickLine={false} axisLine={{ stroke: 'var(--border)' }} interval="preserveStartEnd" minTickGap={32} />
              <YAxis tick={{ fill: 'var(--muted-foreground)', fontSize: 10 }} tickLine={false} axisLine={false} width={44} tickFormatter={v => metric === 'tokens' ? formatTokens(v) : String(v)} />
              <Tooltip
                cursor={{ fill: 'var(--surface)' }}
                contentStyle={{ background: 'var(--card)', border: '1px solid var(--border)', fontFamily: 'var(--font-mono)', fontSize: 11 }}
                formatter={(v, name) => [metric === 'cost' ? `${Number(v).toLocaleString(undefined, { maximumFractionDigits: 3 })} sats` : Number(v).toLocaleString(), String(name)]}
              />
              {metric === 'tokens' && <Bar dataKey="input" name="Input" stackId="t" fill="var(--brand)" />}
              {metric === 'tokens' && <Bar dataKey="output" name="Output" stackId="t" fill="var(--warning)" />}
              {metric === 'cost' && <Bar dataKey="cost" name="Cost" fill="var(--brand)" />}
              {metric === 'requests' && <Bar dataKey="requests" name="Requests" fill="var(--brand)" />}
            </BarChart>
          </ResponsiveContainer>
        )}
      </div>

      {report && report.by_model.length > 0 && (
        <div className="overflow-x-auto">
          <table className="table-scan w-full">
            <thead>
              <tr className="border-b border-border/50 text-left">
                <th className={th}>Model</th>
                <th className={cn(th, 'text-right')}>Requests</th>
                <th className={cn(th, 'text-right')}>In</th>
                <th className={cn(th, 'text-right')}>Cached</th>
                <th className={cn(th, 'text-right')}>Out</th>
                <th className={cn(th, 'text-right')}>Cost</th>
              </tr>
            </thead>
            <tbody>
              {report.by_model.map(m => (
                <tr key={m.model} className="border-b border-border/50">
                  <td className={cn(td, 'text-foreground')}>{m.model}</td>
                  <td className={cn(td, 'text-right')}>{m.requests.toLocaleString()}</td>
                  <td className={cn(td, 'text-right')}>{formatTokens(m.prompt_tokens)}</td>
                  <td className={cn(td, 'text-right text-muted-foreground')}>{formatTokens(m.cached_tokens)}</td>
                  <td className={cn(td, 'text-right')}>{formatTokens(m.completion_tokens)}</td>
                  <td className={cn(td, 'text-right')}>{formatSats(m.cost_msat)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {report && report.by_model.length === 0 && (
        <p className="py-4 text-center font-mono text-[12px] text-muted-foreground">No usage in this period</p>
      )}
    </div>
  )
}
