import { memo, useMemo } from 'react'
import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer, Legend } from 'recharts'
import { ActivityIcon, GaugeIcon, DollarSignIcon, ShieldAlertIcon } from 'lucide-react'
import type { MetricsSnapshot, CurrencyAmount } from '../types'
import { quotaUsedPct } from '@/lib/quota'

const COLORS = ['var(--brand)', 'var(--warning)', '#a855f7', '#ec4899', '#84cc16', '#f97316']
const TICK = { fontSize: 11, fontFamily: '"JetBrains Mono", monospace', fill: 'var(--muted-foreground)' }
const MARGIN = { top: 10, right: 20, left: 10, bottom: 10 }
const LEGEND = { fontSize: '11px', fontFamily: '"JetBrains Mono", monospace', paddingTop: '12px' }
const TOOLTIP = { background: 'var(--card)', border: '1px solid var(--border)', borderRadius: '2px', fontSize: '12px', fontFamily: '"JetBrains Mono", monospace', padding: '8px 12px' }

interface LineDef { key: string; name: string; color: string }
interface BalanceLine extends LineDef { currency: string }

function balanceDisplay(b: CurrencyAmount): number {
  switch (b.currency) {
    case 'msats': return b.amount / 1000
    case 'usd_micro': return b.amount / 1_000_000
    default: return b.amount
  }
}

type Point = Record<string, unknown>

function buildPoints(hist: MetricsSnapshot[], selP: string | null, selM: string | null): Point[] {
  return hist.map(snap => {
    const e: Point = { time: new Date(snap.timestamp_ms).toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' }) }
    if (selP || selM) {
      for (const p of snap.providers) {
        if (selP && p.provider !== selP) continue
        if (selM && p.model !== selM) continue
        const k = `${p.provider}/${p.model}`
        e[`t_${k}`] = p.p90_ttft_ms ?? null
        e[`o_${k}`] = p.p90_output_tps ?? null
        e[`i_${k}`] = p.p90_input_tps ?? null
      }
    } else {
      let tt = 0, tN = 0, ot = 0, oN = 0, it = 0, iN = 0
      for (const p of snap.providers) {
        if (p.p90_ttft_ms != null) { tt += p.p90_ttft_ms; tN++ }
        if (p.p90_output_tps != null) { ot += p.p90_output_tps; oN++ }
        if (p.p90_input_tps != null) { it += p.p90_input_tps; iN++ }
      }
      e.ttft = tN ? tt / tN : null
      e.out = oN ? ot / oN : null
      e.inp = iN ? it / iN : null
    }
    for (const h of snap.provider_health ?? []) {
      if (selP && h.provider !== selP) continue
      if (h.balance) { e[`bal_${h.provider}`] = balanceDisplay(h.balance); e[`bcur_${h.provider}`] = h.balance.currency }
      if (h.quota) e[`quota_${h.provider}`] = quotaUsedPct(h.quota) ?? null
    }
    return e
  })
}

function keysWith(points: Point[], prefixes: string[]): string[] {
  const ks = new Set<string>()
  for (const d of points) for (const k of Object.keys(d)) if (prefixes.some(p => k.startsWith(p))) ks.add(k)
  return Array.from(ks).sort()
}

const SERIES = [
  { key: 'ttft', prefix: 't_', icon: <ActivityIcon className="size-4 text-brand" />, label: 'P90 TTFT', name: 'P90 TTFT' },
  { key: 'out', prefix: 'o_', icon: <GaugeIcon className="size-4 text-brand" />, label: 'P90 Output TPS', name: 'Output TPS' },
  { key: 'inp', prefix: 'i_', icon: <GaugeIcon className="size-4 text-warning" />, label: 'P90 Input TPS', name: 'Input TPS' },
]

function Panel({ icon, title, subtitle, children }: { icon: React.ReactNode; title: string; subtitle: string; children: React.ReactNode }) {
  return (
    <div className="panel p-5">
      <div className="flex items-center gap-2 mb-4">
        {icon}
        <h3 className="font-mono text-[13px] uppercase tracking-[0.1em] text-foreground">{title}</h3>
        <span className="ml-auto font-mono text-[11px] text-muted-foreground">{subtitle}</span>
      </div>
      {children}
    </div>
  )
}

function Chart({ points, lines, domain, children }: { points: Point[]; lines: LineDef[]; domain?: [number, number]; children?: React.ReactNode }) {
  return (
    <ResponsiveContainer width="100%" height={340}>
      <LineChart data={points} margin={MARGIN}>
        <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" strokeOpacity={0.4} />
        <XAxis dataKey="time" tick={TICK} stroke="var(--border)" minTickGap={24} />
        {children ?? <YAxis tick={TICK} stroke="var(--border)" width={60} domain={domain} />}
        {!children && <Tooltip contentStyle={TOOLTIP} />}
        <Legend wrapperStyle={LEGEND} />
        {lines.map(l => (
          <Line key={l.key} type="monotone" dataKey={l.key} name={l.name} stroke={l.color} strokeWidth={2} dot={false} connectNulls isAnimationActive={false} />
        ))}
      </LineChart>
    </ResponsiveContainer>
  )
}

function MetricsCharts({ hist, selP, selM }: { hist: MetricsSnapshot[]; selP: string | null; selM: string | null }) {
  const points = useMemo(() => buildPoints(hist, selP, selM), [hist, selP, selM])
  const subtitle = selP ? (selM ? `${selP} / ${selM}` : selP) : selM ? `${selM} (all providers)` : 'ALL PROVIDERS'

  const perSeries = useMemo(() => SERIES.map(s => {
    const keys = keysWith(points, [s.prefix])
    const lines: LineDef[] = keys.length === 0 && !selP && !selM
      ? [{ key: s.key, name: s.name, color: COLORS[0] }]
      : keys.map((k, i) => ({ key: k, name: k.slice(2).replace('/', ' / '), color: COLORS[i % COLORS.length] }))
    return { ...s, lines }
  }), [points, selP, selM])

  const balanceLines = useMemo<BalanceLine[]>(() => keysWith(points, ['bcur_']).map((k, i) => {
    const provider = k.slice(5)
    const currency = points.find(p => p[k] !== undefined)?.[k] as string
    return { key: `bal_${provider}`, name: provider, color: COLORS[i % COLORS.length], currency }
  }), [points])

  const quotaLines = useMemo<LineDef[]>(
    () => keysWith(points, ['quota_']).map((k, i) => ({ key: k, name: k.slice(6), color: COLORS[i % COLORS.length] })),
    [points],
  )

  const left = balanceLines.filter(l => l.currency !== 'usd_micro')
  const right = balanceLines.filter(l => l.currency === 'usd_micro')

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
      {perSeries.map(s => (
        <Panel key={s.key} icon={s.icon} title={s.label} subtitle={subtitle}>
          <Chart points={points} lines={s.lines} />
        </Panel>
      ))}
      {balanceLines.length > 0 && (
        <Panel icon={<DollarSignIcon className="size-4 text-brand" />} title="Balance" subtitle={subtitle}>
          <ResponsiveContainer width="100%" height={340}>
            <LineChart data={points} margin={MARGIN}>
              <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" strokeOpacity={0.4} />
              <XAxis dataKey="time" tick={TICK} stroke="var(--border)" minTickGap={24} />
              {left.length > 0 && <YAxis yAxisId="left" tick={TICK} stroke="var(--border)" width={60} />}
              {right.length > 0 && <YAxis yAxisId="right" orientation="right" tick={TICK} stroke="var(--border)" width={60} />}
              <Tooltip
                contentStyle={TOOLTIP}
                formatter={(v: unknown, _: unknown, entry: unknown) => {
                  const line = balanceLines.find(l => l.key === (entry as { dataKey?: string })?.dataKey)
                  const unit = line?.currency === 'usd_micro' ? '$' : 'sats'
                  return [`${Number(v ?? 0).toLocaleString('en-US', { maximumFractionDigits: 2 })} ${unit}`, line?.name ?? '']
                }}
              />
              <Legend wrapperStyle={LEGEND} />
              {left.map(l => <Line key={l.key} yAxisId="left" type="monotone" dataKey={l.key} name={l.name} stroke={l.color} strokeWidth={2} dot={false} connectNulls isAnimationActive={false} />)}
              {right.map(l => <Line key={l.key} yAxisId="right" type="monotone" dataKey={l.key} name={l.name} stroke={l.color} strokeWidth={2} dot={false} connectNulls isAnimationActive={false} strokeDasharray="6 3" />)}
            </LineChart>
          </ResponsiveContainer>
        </Panel>
      )}
      {quotaLines.length > 0 && (
        <Panel icon={<ShieldAlertIcon className="size-4 text-warning" />} title="Quota Usage %" subtitle={subtitle}>
          <Chart points={points} lines={quotaLines} domain={[0, 100]} />
        </Panel>
      )}
    </div>
  )
}

export default memo(MetricsCharts)
