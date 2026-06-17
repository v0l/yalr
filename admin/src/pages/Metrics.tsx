import { useEffect, useRef, useState, useCallback, useMemo } from 'react'
import { api, API_BASE_URL } from '../api/client'
import type { WsProviderMetrics, MetricsResponse, MetricsSnapshot, HealthOverviewResponse, CurrencyAmount } from '../types'
import { cn } from '@/lib/utils'
import { quotaUsedPct } from '@/lib/quota'
import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer, Legend } from 'recharts'
import { ActivityIcon, GaugeIcon, BarChart3Icon, DollarSignIcon, ShieldAlertIcon } from 'lucide-react'
import MetricsEventStream from './MetricsEventStream'
import StatCard from '@/components/StatCard'
import { has, pct, fmtNum, MAXL, MAXA } from './metricsHelpers'

/* ── Aggregation types ─────────────────────────────────────────── */

interface AggProvider {
  name: string; models: Map<string, ModelStats>
  totalRequests: number; successes: number; failures: number
  ttftVals: number[]; latVals: number[]; outTpsVals: number[]
  lastEvent: number; inFlight: number; maxConcurrency: number | null
}
interface ModelStats {
  name: string; requests: number; successes: number; failures: number
  ttftVals: number[]; latVals: number[]; outTpsVals: number[]; lastEvent: number
}

/* ═══════════════════════════════════════════════════════════════ */
/*  Metrics Page                                                  */
/* ═══════════════════════════════════════════════════════════════ */

export default function Metrics() {
  const [wsStatus, setWsStatus] = useState<'connecting' | 'connected' | 'disconnected'>('disconnected')
  const [liveEvents, setLiveEvents] = useState<WsProviderMetrics[]>([])
  const [providers, setProviders] = useState<Map<string, AggProvider>>(new Map())
  const [skipped, setSkipped] = useState(0)
  const [selP, setSelP] = useState<string | null>(null)
  const [selM, setSelM] = useState<string | null>(null)
  const [hist, setHist] = useState<MetricsSnapshot[] | null>(null)
  const [health, setHealth] = useState<HealthOverviewResponse | null>(null)
  const [loadingHist, setLoadingHist] = useState(false)
  const [metricTotals, setMetricTotals] = useState<{ totalReqs: number; totalOk: number; totalFail: number } | null>(null)
  const reconnectT = useRef<ReturnType<typeof setTimeout> | null>(null)
  const wsRef = useRef<WebSocket | null>(null)

  const processEvent = useCallback((m: WsProviderMetrics) => {
    setLiveEvents(prev => [m, ...prev].slice(0, MAXL))
    setProviders(prev => {
      const n = new Map(prev)
      const ex = n.get(m.provider)
      const isO = m.event === 'Success' || has(m.event, 'Failure')
      const isT = has(m.event, 'TTFT')
      const isL = has(m.event, 'TotalLatency')
      const isOT = has(m.event, 'OutputTokensPerSecond')
      const p: AggProvider = ex
        ? { ...ex, lastEvent: m.timestamp_ms }
        : { name: m.provider, models: new Map(), totalRequests: 0, successes: 0, failures: 0, ttftVals: [], latVals: [], outTpsVals: [], lastEvent: m.timestamp_ms, inFlight: 0, maxConcurrency: null }
      if (isO) { p.totalRequests++; m.event === 'Success' ? p.successes++ : p.failures++ }
      if (isT) p.ttftVals = [...p.ttftVals, (m.event as Record<string, number>).TTFT].slice(-MAXA)
      if (isL) p.latVals = [...p.latVals, (m.event as Record<string, number>).TotalLatency].slice(-MAXA)
      if (isOT) p.outTpsVals = [...p.outTpsVals, (m.event as Record<string, number>).OutputTokensPerSecond].slice(-MAXA)
      if (m.event && has(m.event, 'ProviderLoad')) {
        const l = (m.event as Record<string, unknown>).ProviderLoad as { in_flight: number; max_concurrency: number | null }
        p.inFlight = l.in_flight; p.maxConcurrency = l.max_concurrency
      }
      if (m.model) {
        let ms = p.models.get(m.model)
        const m2: ModelStats = ms ? { ...ms, lastEvent: m.timestamp_ms } : { name: m.model, requests: 0, successes: 0, failures: 0, ttftVals: [], latVals: [], outTpsVals: [], lastEvent: m.timestamp_ms }
        if (isO) { m2.requests++; m.event === 'Success' ? m2.successes++ : m2.failures++ }
        if (isT) m2.ttftVals = [...m2.ttftVals, (m.event as Record<string, number>).TTFT].slice(-MAXA)
        if (isL) m2.latVals = [...m2.latVals, (m.event as Record<string, number>).TotalLatency].slice(-MAXA)
        if (isOT) m2.outTpsVals = [...m2.outTpsVals, (m.event as Record<string, number>).OutputTokensPerSecond].slice(-MAXA)
        p.models = new Map(p.models).set(m.model, m2)
      }
      n.set(m.provider, p); return n
    })
  }, [])

  /* ── Preload REST data ──────────────────────────────────────── */
  useEffect(() => {
    async function preload() {
      try {
        const [d, h] = await Promise.all([api.getMetrics(), api.getHealthOverview().catch(() => null)]) as [MetricsResponse, HealthOverviewResponse | null]
        if (h) setHealth(h)
        setMetricTotals({ totalReqs: d.total_requests, totalOk: d.total_successes, totalFail: d.total_failures })
        const map = new Map<string, AggProvider>()
        for (const pr of d.providers) {
          if (pr.provider) map.set(pr.provider, {
            name: pr.provider, models: new Map(), totalRequests: 0, successes: 0, failures: 0,
            ttftVals: pr.p90_ttft_ms != null ? [pr.p90_ttft_ms] : [], latVals: pr.avg_latency_ms != null ? [pr.avg_latency_ms] : [],
            outTpsVals: pr.p90_tokens_per_second != null ? [pr.p90_tokens_per_second] : [],
            lastEvent: Date.now(), inFlight: pr.in_flight ?? 0, maxConcurrency: pr.max_concurrency,
          })
        }
        const evt = (d.recent_events as unknown as WsProviderMetrics[]).reverse()
        for (const e of evt) processEventSilent(e, map)
        setProviders(map); setLiveEvents(evt.reverse().slice(0, MAXL))
      } catch {}
    }
    preload()
  }, [])

  function processEventSilent(e: WsProviderMetrics, map: Map<string, AggProvider>) {
    const isO = e.event === 'Success' || has(e.event, 'Failure')
    let p = map.get(e.provider)
    if (!p) {
      p = { name: e.provider, models: new Map(), totalRequests: 0, successes: 0, failures: 0, ttftVals: [], latVals: [], outTpsVals: [], lastEvent: e.timestamp_ms, inFlight: 0, maxConcurrency: null }
      map.set(e.provider, p)
    } else p.lastEvent = e.timestamp_ms
    if (isO) { p.totalRequests++; e.event === 'Success' ? p.successes++ : p.failures++ }
    if (has(e.event, 'TTFT')) p.ttftVals.push((e.event as Record<string, number>).TTFT)
    if (has(e.event, 'TotalLatency')) p.latVals.push((e.event as Record<string, number>).TotalLatency)
    if (has(e.event, 'OutputTokensPerSecond')) p.outTpsVals.push((e.event as Record<string, number>).OutputTokensPerSecond)
    if (e.event && has(e.event, 'ProviderLoad')) {
      const l = (e.event as Record<string, unknown>).ProviderLoad as { in_flight: number; max_concurrency: number | null }
      p.inFlight = l.in_flight; p.maxConcurrency = l.max_concurrency
    }
    if (e.model) {
      let ms = p.models.get(e.model)
      if (!ms) { ms = { name: e.model, requests: 0, successes: 0, failures: 0, ttftVals: [], latVals: [], outTpsVals: [], lastEvent: e.timestamp_ms }; p.models.set(e.model, ms) }
      else ms.lastEvent = e.timestamp_ms
      if (isO) { ms.requests++; e.event === 'Success' ? ms.successes++ : ms.failures++ }
      if (has(e.event, 'TTFT')) ms.ttftVals.push((e.event as Record<string, number>).TTFT)
      if (has(e.event, 'TotalLatency')) ms.latVals.push((e.event as Record<string, number>).TotalLatency)
      if (has(e.event, 'OutputTokensPerSecond')) ms.outTpsVals.push((e.event as Record<string, number>).OutputTokensPerSecond)
    }
  }

  /* ── History load (once) + periodic health ──────────────────── */
  useEffect(() => {
    let c = false
    async function load() { setLoadingHist(true); try { const h = await api.getMetricsHistory(); if (!c) setHist(h) } catch {} finally { if (!c) setLoadingHist(false) } }
    load()
    const iv = setInterval(() => { api.getHealthOverview().then(setHealth).catch(() => {}) }, 60000)
    return () => { c = true; clearInterval(iv) }
  }, [])

  /* ── WebSocket ───────────────────────────────────────────────── */
  useEffect(() => {
    let c = false
    function connect() {
      if (c) return
      const tok = localStorage.getItem('token')
      if (!tok) { setWsStatus('disconnected'); return }
      let base: string
      try { const u = new URL(API_BASE_URL); u.protocol = u.protocol === 'https:' ? 'wss:' : 'ws:'; base = u.toString().replace(/\/$/, '') }
      catch { const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'; base = `${proto}//${window.location.host}` }
      setWsStatus('connecting')
      const ws = new WebSocket(`${base}/api/metrics/ws?token=${encodeURIComponent(tok)}`)
      wsRef.current = ws
      ws.onopen = () => { if (!c) setWsStatus('connected') }
      ws.onmessage = (ev) => {
        if (c) return
        try {
          const d = JSON.parse(ev.data as string)
          if (d.type === 'lag' && typeof d.skipped === 'number') { setSkipped(s => s + d.skipped); return }
          processEvent(d as WsProviderMetrics)
        } catch {}
      }
      ws.onclose = () => { if (!c) { setWsStatus('disconnected'); wsRef.current = null; reconnectT.current = setTimeout(connect, 3000) } }
    }
    connect()
    return () => { c = true; if (reconnectT.current) clearTimeout(reconnectT.current); wsRef.current?.close() }
  }, [processEvent])

  /* ── Derived data ────────────────────────────────────────────── */
  const plist = Array.from(providers.values()).sort((a, b) => a.name.localeCompare(b.name))
  const sdata = selP ? providers.get(selP) : null

  /** Display amount in the currency's natural unit. */
  function balanceDisplay(b: CurrencyAmount): number {
    switch (b.currency) { case 'msats': return b.amount / 1000; case 'sats': return b.amount; case 'usd_micro': return b.amount / 1_000_000; default: return b.amount }
  }

  const chartData = useMemo(() => {
    if (!hist) return []
    return hist.map(snap => {
      const e: Record<string, unknown> = { time: new Date(snap.timestamp_ms).toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' }) }
      if (selP) {
        for (const p of snap.providers) {
          if (p.provider !== selP) continue
          if (selM && p.model !== selM) continue
          const k = `${p.provider}/${p.model}`
          e[`t_${k}`] = p.p90_ttft_ms ?? null; e[`o_${k}`] = p.p90_output_tps ?? null; e[`i_${k}`] = p.p90_input_tps ?? null
        }
      } else if (selM) {
        for (const p of snap.providers) {
          if (p.model !== selM) continue
          const k = `${p.provider}/${p.model}`
          e[`t_${k}`] = p.p90_ttft_ms ?? null; e[`o_${k}`] = p.p90_output_tps ?? null; e[`i_${k}`] = p.p90_input_tps ?? null
        }
      } else {
        let tt = 0, tN = 0, ot = 0, oN = 0, it = 0, iN = 0
        for (const p of snap.providers) {
          if (p.p90_ttft_ms != null) { tt += p.p90_ttft_ms; tN++ }
          if (p.p90_output_tps != null) { ot += p.p90_output_tps; oN++ }
          if (p.p90_input_tps != null) { it += p.p90_input_tps; iN++ }
        }
        e.ttft = tN ? tt / tN : null; e.out = oN ? ot / oN : null; e.inp = iN ? it / iN : null
      }
      const ph = snap.provider_health ?? []
      if (selP) {
        const h = ph.find(p => p.provider === selP)
        if (h?.balance) { e[`bal_${selP}`] = balanceDisplay(h.balance); e[`bcur_${selP}`] = h.balance.currency }
        if (h?.quota) e[`quota_${selP}`] = quotaUsedPct(h.quota) ?? null
      } else {
        for (const h of ph) {
          if (h.balance) { e[`bal_${h.provider}`] = balanceDisplay(h.balance); e[`bcur_${h.provider}`] = h.balance.currency }
          if (h.quota) e[`quota_${h.provider}`] = quotaUsedPct(h.quota) ?? null
        }
      }
      return e
    })
  }, [hist, selP, selM])

  const lines = useMemo(() => {
    const ks = new Set<string>()
    for (const d of chartData) for (const k of Object.keys(d)) { if (k.startsWith('t_') || k.startsWith('o_') || k.startsWith('i_')) ks.add(k) }
    const clr = ['var(--brand)', 'var(--warning)', '#a855f7', '#ec4899', '#84cc16', '#f97316']
    const all = Array.from(ks)
    if (all.length === 0 && !selP) return [
      { key: 'ttft', name: 'P90 TTFT', color: clr[0] },
      { key: 'out', name: 'Output TPS', color: clr[1] },
      { key: 'inp', name: 'Input TPS', color: clr[2] },
    ]
    return all.map((k, i) => {
      const lp = k.slice(2).replace('/', ' / ')
      const lbl = k.startsWith('t_') ? `TTFT ${lp}` : k.startsWith('o_') ? `Out TPS ${lp}` : `In TPS ${lp}`
      return { key: k, name: lbl, color: clr[i % clr.length] }
    })
  }, [chartData, selP])

  const balanceLines = useMemo(() => {
    const m = new Map<string, { key: string; name: string; color: string; currency: string }>()
    for (const d of chartData) for (const k of Object.keys(d)) {
      if (k.startsWith('bcur_')) m.set(k, { key: k, name: '', color: '', currency: d[k] as string })
    }
    const clr = ['var(--brand)', 'var(--warning)', '#a855f7', '#ec4899', '#84cc16', '#f97316']
    const sorted = Array.from(m.entries()).sort((a, b) => a[0].localeCompare(b[0]))
    return sorted.map(([k, v], i) => {
      const prov = k.slice(5)
      const balK = `bal_${prov}`
      return { key: balK, name: prov, color: clr[i % clr.length], currency: v.currency }
    })
  }, [chartData])

  /** Which balance lines go on right (USD) vs left (sats) axis. */
  const balanceLeft = useMemo(() => balanceLines.filter(l => l.currency !== 'usd_micro'), [balanceLines])
  const balanceRight = useMemo(() => balanceLines.filter(l => l.currency === 'usd_micro'), [balanceLines])

  const quotaLines = useMemo(() => {
    const ks = new Set<string>()
    for (const d of chartData) for (const k of Object.keys(d)) { if (k.startsWith('quota_')) ks.add(k) }
    const clr = ['var(--brand)', 'var(--warning)', '#a855f7', '#ec4899', '#84cc16', '#f97316']
    return Array.from(ks).map((k, i) => ({ key: k, name: k.slice(6), color: clr[i % clr.length] }))
  }, [chartData])

  const fmodels = useMemo(() => {
    if (!sdata) return []
    return Array.from(sdata.models.values()).sort((a, b) => a.name.localeCompare(b.name))
  }, [sdata])

  const allModels = useMemo(() => {
    const map = new Map<string, { requests: number; successes: number; failures: number }>()
    for (const p of plist) {
      for (const [name, ms] of p.models) {
        const e = map.get(name)
        if (e) { e.requests += ms.requests; e.successes += ms.successes; e.failures += ms.failures }
        else map.set(name, { requests: ms.requests, successes: ms.successes, failures: ms.failures })
      }
    }
    return Array.from(map.entries()).map(([name, s]) => ({ name, ...s })).sort((a, b) => a.name.localeCompare(b.name))
  }, [plist])

  const chartSubtitle = selP ? (selM ? `${selP} / ${selM}` : selP) : selM ? `${selM} (all providers)` : 'ALL PROVIDERS'

  const summaryStats = useMemo(() => {
    const totalReqs = metricTotals?.totalReqs ?? plist.reduce((s, p) => s + p.totalRequests, 0)
    const totalOk = metricTotals?.totalOk ?? plist.reduce((s, p) => s + p.successes, 0)
    const sr = totalReqs > 0 ? (totalOk / totalReqs * 100) : null
    const allTTFT = plist.flatMap(p => p.ttftVals)
    const p90ttft = pct(allTTFT, 0.9)
    const active = plist.filter(p => p.inFlight > 0 || (Date.now() - p.lastEvent < 60000)).length
    return { totalReqs, totalOk, sr, p90ttft, active, total: plist.length }
  }, [plist, metricTotals])

  const ttStyle = { background: 'var(--card)', border: '1px solid var(--border)', borderRadius: '2px', fontSize: '12px', fontFamily: '"JetBrains Mono", monospace', padding: '8px 12px' }

  /* ── Render ──────────────────────────────────────────────────── */
  return (
    <div className="space-y-6 p-6">
      {/* Header */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground leading-none mb-1">METRICS</h1>
          <p className="font-mono text-[13px] text-muted-foreground">Real-time provider performance monitoring</p>
        </div>
        <div className="flex items-center gap-3">
          {skipped > 0 && <span className="font-mono text-[11px] text-warning">{skipped} skipped</span>}
          <div className={cn(
            'flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-wider px-2.5 py-1 border',
            wsStatus === 'connected' ? 'text-brand border-brand/30 bg-brand/5' :
            wsStatus === 'connecting' ? 'text-warning border-warning/30 bg-warning/5' :
            'text-destructive border-destructive/30 bg-destructive/5'
          )}>
            <span className={cn('w-1.5 h-1.5 rounded-full',
              wsStatus === 'connected' && 'bg-brand animate-pulse-status',
              wsStatus === 'connecting' && 'bg-warning animate-pulse-status',
              wsStatus === 'disconnected' && 'bg-destructive'
            )} />
            {wsStatus === 'connected' ? 'LIVE' : wsStatus === 'connecting' ? 'CONN' : 'OFF'}
          </div>
        </div>
      </div>

      {/* Empty state */}
      {plist.length === 0 ? (
        <div className="panel p-12 text-center">
          <p className="font-mono text-[13px] text-muted-foreground">{'>'} WAITING FOR METRICS...</p>
        </div>
      ) : (
        /* Provider Grid */
        <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6 gap-2">
          {plist.map(p => {
            const sr = p.totalRequests > 0 ? (p.successes / p.totalRequests * 100) : null
            const tt = pct(p.ttftVals, .9)
            const ot = pct(p.outTpsVals, .9)
            const is = selP === p.name
            const h = health?.providers.find(hp => hp.provider === p.name)
            return (
              <button
                key={p.name}
                onClick={() => { setSelP(is ? null : p.name); setSelM(null) }}
                className={cn('text-left p-3 border transition-colors cursor-pointer font-mono', is ? 'border-brand/50 bg-brand/5' : 'panel hover:border-brand/20')}
              >
                <div className="flex items-center justify-between mb-2 gap-1.5">
                  <span className="text-[13px] font-semibold truncate min-w-0">{p.name}</span>
                  <div className="flex items-center gap-1.5 shrink-0">
                    {h?.rate_limited && <span className="text-[9px] uppercase text-warning font-medium tracking-wider">RL</span>}
                    {h?.backoff_ms ? h.backoff_ms > 0 && <span className="text-[10px] text-muted-foreground tabular-nums">{h.backoff_ms >= 1000 ? `${(h.backoff_ms / 1000).toFixed(1)}s` : `${h.backoff_ms}ms`}</span> : null}
                    {h && <span className={cn('w-1.5 h-1.5 rounded-full shrink-0', h.health_state === 'healthy' && 'bg-brand', h.health_state === 'degraded' && 'bg-warning', h.health_state === 'unhealthy' && 'bg-destructive')} title={h.health_state} />}
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-x-2 gap-y-1 text-[11px]">
                  <span className="text-muted-foreground">LOAD</span>
                  <span className="tabular-nums text-right">{p.inFlight}{p.maxConcurrency ? <span className="text-muted-foreground">/{p.maxConcurrency}</span> : ''}</span>
                  <span className="text-muted-foreground">SUCCESS</span>
                  <span className={cn('tabular-nums text-right', sr !== null ? (sr >= 95 ? 'text-brand' : sr >= 80 ? 'text-warning' : 'text-destructive') : 'text-muted-foreground')}>{sr !== null ? `${sr.toFixed(1)}%` : '—'}</span>
                  <span className="text-muted-foreground">P90 TTFT</span>
                  <span className="tabular-nums text-right">{tt !== null ? `${fmtNum(tt)}ms` : '—'}</span>
                  <span className="text-muted-foreground">P90 TPS</span>
                  <span className="tabular-nums text-right">{ot !== null ? ot.toFixed(1) : '—'}</span>
                  <span className="text-muted-foreground">FAILURES</span>
                  <span className={cn('tabular-nums text-right', (h?.consecutive_failures ?? 0) === 0 ? 'text-brand' : 'text-destructive')}>{h?.consecutive_failures ?? '—'}</span>
                </div>
              </button>
            )
          })}
        </div>
      )}

      {/* Summary Stats */}
      {plist.length > 0 && (
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
          <StatCard label="Providers" valueJsx value={
            <><span className="text-brand">{summaryStats.active}</span><span className="text-muted-foreground text-[20px]">/{summaryStats.total}</span></>
          } sub="active" color={plist.length === summaryStats.active ? 'green' : 'default'} />
          <StatCard label="Total Requests" value={fmtNum(summaryStats.totalReqs)} sub={`${fmtNum(summaryStats.totalOk)} ok · ${fmtNum(summaryStats.totalReqs - summaryStats.totalOk)} fail`}
            subGood={summaryStats.totalReqs > 0 && (summaryStats.totalReqs - summaryStats.totalOk) === 0}
            subError={(summaryStats.totalReqs - summaryStats.totalOk) > 0} />
          <StatCard label="Success Rate" value={summaryStats.sr !== null ? `${summaryStats.sr.toFixed(1)}%` : '—'}
            sub={summaryStats.sr !== null ? (summaryStats.sr >= 99 ? 'EXCELLENT' : summaryStats.sr >= 95 ? 'GOOD' : 'DEGRADED') : undefined}
            subGood={summaryStats.sr !== null && summaryStats.sr >= 99}
            subError={summaryStats.sr !== null && summaryStats.sr < 95}
            color={summaryStats.sr === null ? 'default' : summaryStats.sr >= 99 ? 'green' : summaryStats.sr >= 95 ? 'amber' : 'red'} />
          <StatCard label="P90 TTFT" value={summaryStats.p90ttft !== null ? `${fmtNum(summaryStats.p90ttft)}ms` : '—'} sub="all providers" />
        </div>
      )}

      {/* Charts */}
      {hist && hist.length > 0 && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
          {[
            { key: 'ttft', icon: <ActivityIcon className="size-4 text-brand" />, label: 'P90 TTFT', filter: (l: typeof lines[0]) => l.key.startsWith('t_') || l.key === 'ttft' },
            { key: 'out', icon: <GaugeIcon className="size-4 text-brand" />, label: 'P90 Output TPS', filter: (l: typeof lines[0]) => l.key.startsWith('o_') || l.key === 'out', rename: (n: string) => n.replace(/^(Out TPS |P90 Out TPS )/, '') },
            { key: 'inp', icon: <GaugeIcon className="size-4 text-warning" />, label: 'P90 Input TPS', filter: (l: typeof lines[0]) => l.key.startsWith('i_') || l.key === 'inp', rename: (n: string) => n.replace(/^(In TPS |P90 In TPS )/, '') },
          ].map(ch => (
            <div key={ch.key} className="panel p-5">
              <div className="flex items-center gap-2 mb-4">
                {ch.icon}
                <h3 className="font-mono text-[13px] uppercase tracking-[0.1em] text-foreground">{ch.label}</h3>
                <span className="ml-auto font-mono text-[11px] text-muted-foreground">{chartSubtitle}</span>
              </div>
              {loadingHist ? (
                <div className="flex items-center justify-center font-mono text-[13px] text-muted-foreground" style={{ height: 340 }}><span className="animate-pulse">LOADING...</span></div>
              ) : (
                <ResponsiveContainer width="100%" height={340}>
                  <LineChart data={chartData} margin={{ top: 10, right: 20, left: 10, bottom: 10 }}>
                    <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" strokeOpacity={0.4} />
                    <XAxis dataKey="time" tick={{ fontSize: 11, fontFamily: '"JetBrains Mono", monospace', fill: 'var(--muted-foreground)' }} stroke="var(--border)" />
                    <YAxis tick={{ fontSize: 11, fontFamily: '"JetBrains Mono", monospace', fill: 'var(--muted-foreground)' }} stroke="var(--border)" width={60} />
                    <Tooltip contentStyle={ttStyle} />
                    <Legend wrapperStyle={{ fontSize: '11px', fontFamily: '"JetBrains Mono", monospace', paddingTop: '12px' }} />
                    {lines.filter(ch.filter).map(l => (
                      <Line key={l.key} type="monotone" dataKey={l.key} name={ch.rename ? ch.rename(l.name) : l.name} stroke={l.color} strokeWidth={2} dot={false} connectNulls animationDuration={800} />
                    ))}
                  </LineChart>
                </ResponsiveContainer>
              )}
            </div>
          ))}
          {balanceLines.length > 0 && (
            <div className="panel p-5">
              <div className="flex items-center gap-2 mb-4">
                <DollarSignIcon className="size-4 text-brand" />
                <h3 className="font-mono text-[13px] uppercase tracking-[0.1em] text-foreground">Balance</h3>
                <span className="ml-auto font-mono text-[11px] text-muted-foreground">{chartSubtitle}</span>
              </div>
              <ResponsiveContainer width="100%" height={340}>
                <LineChart data={chartData} margin={{ top: 10, right: 20, left: 10, bottom: 10 }}>
                  <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" strokeOpacity={0.4} />
                  <XAxis dataKey="time" tick={{ fontSize: 11, fontFamily: '"JetBrains Mono", monospace', fill: 'var(--muted-foreground)' }} stroke="var(--border)" />
                  {balanceLeft.length > 0 && (
                    <YAxis yAxisId="left" tick={{ fontSize: 11, fontFamily: '"JetBrains Mono", monospace', fill: 'var(--muted-foreground)' }} stroke="var(--border)" width={60} label={{ value: 'sats', angle: -90, position: 'insideLeft', offset: -2, style: { fontSize: 10, fontFamily: '"JetBrains Mono", monospace', fill: 'var(--muted-foreground)' } }} />
                  )}
                  {balanceRight.length > 0 && (
                    <YAxis yAxisId="right" orientation="right" tick={{ fontSize: 11, fontFamily: '"JetBrains Mono", monospace', fill: 'var(--muted-foreground)' }} stroke="var(--border)" width={60} label={{ value: '$', angle: 90, position: 'insideRight', offset: -2, style: { fontSize: 10, fontFamily: '"JetBrains Mono", monospace', fill: 'var(--muted-foreground)' } }} />
                  )}
                  <Tooltip
                    contentStyle={ttStyle}
                    labelFormatter={(label: string) => label}
                    formatter={(v: number, _: string, entry: { dataKey?: string | number }) => {
                      const line = balanceLines.find(l => l.key === entry.dataKey)
                      const cur = line?.currency === 'usd_micro' ? '$' : 'sats'
                      const prov = line?.name ?? ''
                      return [`${v.toLocaleString('en-US', { maximumFractionDigits: 2 })} ${cur}`, prov]
                    }}
                  />
                  <Legend wrapperStyle={{ fontSize: '11px', fontFamily: '"JetBrains Mono", monospace', paddingTop: '12px' }} />
                  {balanceLeft.map(l => (
                    <Line key={l.key} yAxisId="left" type="monotone" dataKey={l.key} name={l.name} stroke={l.color} strokeWidth={2} dot={false} connectNulls animationDuration={800} />
                  ))}
                  {balanceRight.map(l => (
                    <Line key={l.key} yAxisId="right" type="monotone" dataKey={l.key} name={l.name} stroke={l.color} strokeWidth={2} dot={false} connectNulls animationDuration={800} strokeDasharray="6 3" />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}
          {quotaLines.length > 0 && (
            <div className="panel p-5">
              <div className="flex items-center gap-2 mb-4">
                <ShieldAlertIcon className="size-4 text-warning" />
                <h3 className="font-mono text-[13px] uppercase tracking-[0.1em] text-foreground">Quota Usage %</h3>
                <span className="ml-auto font-mono text-[11px] text-muted-foreground">{chartSubtitle}</span>
              </div>
              <ResponsiveContainer width="100%" height={340}>
                <LineChart data={chartData} margin={{ top: 10, right: 20, left: 10, bottom: 10 }}>
                  <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" strokeOpacity={0.4} />
                  <XAxis dataKey="time" tick={{ fontSize: 11, fontFamily: '"JetBrains Mono", monospace', fill: 'var(--muted-foreground)' }} stroke="var(--border)" />
                  <YAxis domain={[0, 100]} tick={{ fontSize: 11, fontFamily: '"JetBrains Mono", monospace', fill: 'var(--muted-foreground)' }} stroke="var(--border)" width={60} />
                  <Tooltip contentStyle={ttStyle} />
                  <Legend wrapperStyle={{ fontSize: '11px', fontFamily: '"JetBrains Mono", monospace', paddingTop: '12px' }} />
                  {quotaLines.map(l => (
                    <Line key={l.key} type="monotone" dataKey={l.key} name={l.name} stroke={l.color} strokeWidth={2} dot={false} connectNulls animationDuration={800} />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}
        </div>
      )}

      {/* Bottom: model breakdown + event stream */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        {/* Model Breakdown */}
        <div className="lg:col-span-1">
          <div className="panel p-4">
            <h3 className="font-mono text-[12px] uppercase tracking-[0.1em] text-muted-foreground mb-3 flex items-center gap-2">
              <BarChart3Icon className="size-3.5 text-brand" /> Models
              <span className="ml-auto font-mono text-[10px] text-muted-foreground tabular-nums">{allModels.length}</span>
            </h3>
            {allModels.length === 0 ? (
              <p className="font-mono text-[12px] text-muted-foreground py-4 text-center">No model data yet</p>
            ) : (
              <div className="space-y-1">
                {allModels.map(m => {
                  const sr = m.requests > 0 ? (m.successes / m.requests * 100) : null
                  return (
                    <button key={m.name} onClick={() => { setSelM(selM === m.name ? null : m.name); setSelP(null) }}
                      className={cn('w-full text-left p-2 border transition-colors', selM === m.name ? 'border-brand/40 bg-brand/5' : 'border-border/50 hover:border-border bg-surface')}>
                      <div className="flex items-center justify-between">
                        <span className="font-mono text-[12px] font-medium text-foreground truncate">{m.name}</span>
                        {sr !== null && <span className={cn('font-mono text-[11px] tabular-nums ml-2 shrink-0', sr >= 95 ? 'text-brand' : sr >= 80 ? 'text-warning' : 'text-destructive')}>{sr.toFixed(1)}%</span>}
                      </div>
                      <div className="text-muted-foreground text-[10px] font-mono mt-0.5">{m.requests} reqs</div>
                    </button>
                  )
                })}
              </div>
            )}
            {sdata && fmodels.length > 0 && (
              <>
                <div className="section-header mt-4">{sdata.name} Models</div>
                {fmodels.map(m => {
                  const pt = pct(m.ttftVals, .9); const po = pct(m.outTpsVals, .9)
                  const is = selP === sdata.name && selM === m.name
                  return (
                    <button key={`${sdata.name}-${m.name}`} onClick={() => { setSelP(sdata.name); setSelM(is ? null : m.name) }}
                      className={cn('w-full text-left p-2 border transition-colors', is ? 'border-brand/40 bg-brand/5' : 'border-border/50 hover:border-border bg-surface')}>
                      <div className="font-mono text-[12px] font-medium mb-1 text-foreground">{m.name}</div>
                      <div className="grid grid-cols-2 gap-x-2 gap-y-0.5 text-[11px] font-mono">
                        <span className="text-muted-foreground">P90 TTFT</span><span className="tabular-nums text-right">{pt !== null ? `${fmtNum(pt)}ms` : '—'}</span>
                        <span className="text-muted-foreground">P90 TPS</span><span className="tabular-nums text-right">{po !== null ? po.toFixed(1) : '—'}</span>
                        <span className="text-muted-foreground">REQS</span><span className="tabular-nums text-right">{m.requests}</span>
                      </div>
                    </button>
                  )
                })}
              </>
            )}
          </div>
        </div>

        {/* Event Stream */}
        <MetricsEventStream events={liveEvents} wsStatus={wsStatus} skipped={skipped} onClear={() => setLiveEvents([])} />
      </div>
    </div>
  )
}
