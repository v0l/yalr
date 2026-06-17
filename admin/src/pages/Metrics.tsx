import { useEffect, useRef, useState, useCallback, useMemo } from 'react'
import { api, API_BASE_URL } from '../api/client'
import type { WsProviderMetrics, MetricsResponse, MetricsSnapshot, HealthOverviewResponse, CurrencyAmount, MetricsUser, QuotaSnapshot } from '../types'
import { cn, formatBalance } from '@/lib/utils'
import { worstQuota, quotaUsedPct } from '@/lib/quota'
import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer, Legend } from 'recharts'
import { ActivityIcon, GaugeIcon, BarChart3Icon, DollarSignIcon, ShieldAlertIcon,
  UserIcon, KeyIcon
} from 'lucide-react'

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

/* ── Helpers ────────────────────────────────────────────────────── */

function fmtNum(n: number): string {
  return n.toLocaleString('en-US')
}

function pct(v: number[], p: number): number | null {
  if (v.length === 0) return null
  const s = [...v].sort((a, b) => a - b)
  return s[Math.round(p * (s.length - 1))]
}

function has<T extends string>(o: unknown, k: T): o is Record<T, unknown> {
  return typeof o === 'object' && o !== null && k in o
}

type EK = 'ok' | 'warn' | 'err' | 'info'

function fmt(e: WsProviderMetrics['event']): { label: string; value: string; kind: EK } {
  if (e === 'Success') return { label: '✓', value: 'OK', kind: 'ok' }
  if (has(e, 'TTFT')) return { label: 'TTFT', value: `${fmtNum(e.TTFT)}ms`, kind: 'info' }
  if (has(e, 'OutputTokensPerSecond')) return { label: 'O/s', value: (e.OutputTokensPerSecond as number).toFixed(1), kind: 'info' }
  if (has(e, 'InputTokensPerSecond')) return { label: 'I/s', value: (e.InputTokensPerSecond as number).toFixed(1), kind: 'info' }
  if (has(e, 'TotalLatency')) return { label: 'LAT', value: `${fmtNum(e.TotalLatency)}ms`, kind: 'info' }
  if (has(e, 'InputTokens')) return { label: 'IN', value: fmtNum(e.InputTokens), kind: 'info' }
  if (has(e, 'OutputTokens')) return { label: 'OUT', value: fmtNum(e.OutputTokens), kind: 'info' }
  if (has(e, 'Failure')) {
    const f = e.Failure as { error_message: string }
    return { label: 'FAIL', value: f.error_message, kind: 'err' }
  }
  if (has(e, 'ProviderLoad')) {
    const l = e.ProviderLoad as { in_flight: number; max_concurrency: number | null }
    return { label: 'LOAD', value: `${l.in_flight}${l.max_concurrency ? `/${l.max_concurrency}` : ''}`, kind: l.in_flight > 0 ? 'warn' : 'ok' }
  }
  if (has(e, 'Balance')) {
    const b = e.Balance as CurrencyAmount
    return { label: 'BAL', value: formatBalance(b.amount, b.currency), kind: 'info' }
  }
  if (has(e, 'Quota')) {
    const all = (e.Quota ?? []) as QuotaSnapshot[]
    const q = worstQuota(all)
    if (!q) return { label: 'QUOTA', value: '—', kind: 'info' }
    const usage = typeof q.used_pct === 'number'
      ? `${q.used_pct.toFixed(0)}% used`
      : (typeof q.remaining === 'number' ? `${q.remaining.toLocaleString()} left` : (q.status ?? '?'))
    const v = q.window ? `${q.window} · ${usage}` : usage
    const exhausted = q.status === 'rejected' || q.status === 'exceeded' || q.status === 'rate_limited' || (typeof q.used_pct === 'number' && q.used_pct >= 100)
    return { label: 'QUOTA', value: v, kind: exhausted ? 'err' : (q.status === 'allowed_warning' ? 'warn' : 'info') }
  }
  return { label: '?', value: JSON.stringify(e), kind: 'info' }
}

function fmtUser(u?: MetricsUser | null): string {
  if (!u) return ''
  const parts: string[] = []
  if (u.name) parts.push(u.name)
  if (u.api_key_name) parts.push(`🔑 ${u.api_key_name}`)
  if (parts.length === 0 && u.id) parts.push(`ID:${u.id}`)
  return parts.join(' · ')
}

const MAXL = 200, MAXA = 500

/** Normalize a CurrencyAmount to sats for consistent charting. */
function normalizeBalance(b: CurrencyAmount): number {
  switch (b.currency) {
    case 'msats': return b.amount / 1000
    case 'sats': return b.amount
    case 'usd_micro': return b.amount / 1_000_000
    default: return b.amount
  }
}

/** Label for a CurrencyAmount. */
function balanceUnit(b: CurrencyAmount): string {
  switch (b.currency) {
    case 'msats': case 'sats': return 'sats'
    case 'usd_micro': return '$'
    default: return ''
  }
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
  const endRef = useRef<HTMLDivElement>(null)
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

  const chartData = useMemo(() => {
    if (!hist) return []
    return hist.map(snap => {
      const e: Record<string, unknown> = {
        time: new Date(snap.timestamp_ms).toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' })
      }
      if (selP) {
        for (const p of snap.providers) {
          if (p.provider !== selP) continue
          if (selM && p.model !== selM) continue
          const k = `${p.provider}/${p.model}`
          e[`t_${k}`] = p.p90_ttft_ms ?? null
          e[`o_${k}`] = p.p90_output_tps ?? null
          e[`i_${k}`] = p.p90_input_tps ?? null
        }
      } else if (selM) {
        for (const p of snap.providers) {
          if (p.model !== selM) continue
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

      // Balance & quota from provider_health
      const ph = snap.provider_health ?? []
      if (selP) {
        const h = ph.find(p => p.provider === selP)
        if (h?.balance) e[`bal_${selP}`] = normalizeBalance(h.balance)
        if (h?.quota) e[`quota_${selP}`] = quotaUsedPct(h.quota) ?? null
      } else {
        for (const h of ph) {
          if (h.balance) e[`bal_${h.provider}`] = normalizeBalance(h.balance)
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
    const out: { key: string; name: string; color: string }[] = []
    all.forEach((k, i) => {
      const lp = k.slice(2).replace('/', ' / ')
      const lbl = k.startsWith('t_') ? `TTFT ${lp}` : k.startsWith('o_') ? `Out TPS ${lp}` : `In TPS ${lp}`
      out.push({ key: k, name: lbl, color: clr[i % clr.length] })
    })
    return out
  }, [chartData, selP])

  const balanceLines = useMemo(() => {
    const ks = new Set<string>()
    for (const d of chartData) for (const k of Object.keys(d)) { if (k.startsWith('bal_')) ks.add(k) }
    const clr = ['var(--brand)', 'var(--warning)', '#a855f7', '#ec4899', '#84cc16', '#f97316']
    return Array.from(ks).map((k, i) => ({ key: k, name: k.slice(4), color: clr[i % clr.length] }))
  }, [chartData])

  const quotaLines = useMemo(() => {
    const ks = new Set<string>()
    for (const d of chartData) for (const k of Object.keys(d)) { if (k.startsWith('quota_')) ks.add(k) }
    const clr = ['var(--brand)', 'var(--warning)', '#a855f7', '#ec4899', '#84cc16', '#f97316']
    return Array.from(ks).map((k, i) => ({ key: k, name: k.slice(6), color: clr[i % clr.length] }))
  }, [chartData])

  const balUnit = useMemo(() => {
    if (!hist) return 'sats'
    for (const snap of hist) {
      for (const h of (snap.provider_health ?? [])) {
        if (h.balance) return balanceUnit(h.balance)
      }
    }
    return 'sats'
  }, [hist])

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

  const chartSubtitle = selP
    ? (selM ? `${selP} / ${selM}` : selP)
    : selM ? `${selM} (all providers)` : 'ALL PROVIDERS'

  const ttStyle = { background: 'var(--card)', border: '1px solid var(--border)', borderRadius: '4px', fontSize: '11px', fontFamily: '"JetBrains Mono", monospace' }

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
                className={cn(
                  'text-left p-3 border transition-colors cursor-pointer font-mono',
                  is ? 'border-brand/50 bg-brand/5' : 'panel hover:border-brand/20'
                )}
              >
                <div className="flex items-center justify-between mb-2 gap-1.5">
                  <span className="text-[13px] font-semibold truncate min-w-0">{p.name}</span>
                  <div className="flex items-center gap-1.5 shrink-0">
                    {h?.rate_limited && <span className="text-[9px] uppercase text-warning font-medium tracking-wider">RL</span>}
                    {h?.backoff_ms ? h.backoff_ms > 0 && <span className="text-[10px] text-muted-foreground tabular-nums">{h.backoff_ms >= 1000 ? `${(h.backoff_ms / 1000).toFixed(1)}s` : `${h.backoff_ms}ms`}</span> : null}
                    {h && (
                      <span className={cn(
                        'w-1.5 h-1.5 rounded-full shrink-0',
                        h.health_state === 'healthy' && 'bg-brand',
                        h.health_state === 'degraded' && 'bg-warning',
                        h.health_state === 'unhealthy' && 'bg-destructive'
                      )} title={h.health_state} />
                    )}
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-x-2 gap-y-1 text-[11px]">
                  <span className="text-muted-foreground">LOAD</span>
                  <span className="tabular-nums text-right">{p.inFlight}{p.maxConcurrency ? <span className="text-muted-foreground">/{p.maxConcurrency}</span> : ''}</span>
                  <span className="text-muted-foreground">SUCCESS</span>
                  <span className={cn('tabular-nums text-right', sr !== null ? (sr >= 95 ? 'text-brand' : sr >= 80 ? 'text-warning' : 'text-destructive') : 'text-muted-foreground')}>
                    {sr !== null ? `${sr.toFixed(1)}%` : '—'}
                  </span>
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

      {/* Charts */}
      {hist && hist.length > 0 && (
        <div className="grid grid-cols-1 xl:grid-cols-5 gap-4">
          {/* TTFT Chart */}
          <div className="panel p-4">
            <h3 className="font-mono text-[12px] uppercase tracking-[0.1em] text-muted-foreground mb-3 flex items-center gap-2">
              <ActivityIcon className="size-3.5 text-brand" />
              P90 TTFT
              <span className="ml-auto font-mono text-[11px] text-muted-foreground normal-case tracking-normal">{chartSubtitle}</span>
            </h3>
            {loadingHist ? (
              <div className="h-48 flex items-center justify-center font-mono text-[13px] text-muted-foreground">Loading...</div>
            ) : (
              <ResponsiveContainer width="100%" height={200}>
                <LineChart data={chartData} margin={{ top: 5, right: 5, left: 0, bottom: 5 }}>
                  <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                  <XAxis dataKey="time" tick={{ fontSize: 10, fontFamily: '"JetBrains Mono", monospace' }} stroke="currentColor" />
                  <YAxis tick={{ fontSize: 10, fontFamily: '"JetBrains Mono", monospace' }} stroke="currentColor" />
                  <Tooltip contentStyle={ttStyle} />
                  <Legend wrapperStyle={{ fontSize: '10px', fontFamily: '"JetBrains Mono", monospace' }} />
                  {lines.filter(l => l.key.startsWith('t_') || l.key === 'ttft').map(l => (
                    <Line key={l.key} type="monotone" dataKey={l.key} name={l.name} stroke={l.color} strokeWidth={2} dot={false} connectNulls />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            )}
          </div>

          {/* Output TPS Chart */}
          <div className="panel p-4">
            <h3 className="font-mono text-[12px] uppercase tracking-[0.1em] text-muted-foreground mb-3 flex items-center gap-2">
              <GaugeIcon className="size-3.5 text-brand" />
              P90 Output TPS
              <span className="ml-auto font-mono text-[11px] text-muted-foreground normal-case tracking-normal">{chartSubtitle}</span>
            </h3>
            {loadingHist ? (
              <div className="h-48 flex items-center justify-center font-mono text-[13px] text-muted-foreground">Loading...</div>
            ) : (
              <ResponsiveContainer width="100%" height={200}>
                <LineChart data={chartData} margin={{ top: 5, right: 5, left: 0, bottom: 5 }}>
                  <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                  <XAxis dataKey="time" tick={{ fontSize: 10, fontFamily: '"JetBrains Mono", monospace' }} stroke="currentColor" />
                  <YAxis tick={{ fontSize: 10, fontFamily: '"JetBrains Mono", monospace' }} stroke="currentColor" />
                  <Tooltip contentStyle={ttStyle} />
                  <Legend wrapperStyle={{ fontSize: '10px', fontFamily: '"JetBrains Mono", monospace' }} />
                  {lines.filter(l => l.key.startsWith('o_') || l.key === 'out').map(l => (
                    <Line key={l.key} type="monotone" dataKey={l.key} name={l.name.replace(/^(Out TPS |P90 Out TPS )/, '')} stroke={l.color} strokeWidth={2} dot={false} connectNulls />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            )}
          </div>

          {/* Input TPS Chart */}
          <div className="panel p-4">
            <h3 className="font-mono text-[12px] uppercase tracking-[0.1em] text-muted-foreground mb-3 flex items-center gap-2">
              <GaugeIcon className="size-3.5 text-warning" />
              P90 Input TPS
              <span className="ml-auto font-mono text-[11px] text-muted-foreground normal-case tracking-normal">{chartSubtitle}</span>
            </h3>
            {loadingHist ? (
              <div className="h-48 flex items-center justify-center font-mono text-[13px] text-muted-foreground">Loading...</div>
            ) : (
              <ResponsiveContainer width="100%" height={200}>
                <LineChart data={chartData} margin={{ top: 5, right: 5, left: 0, bottom: 5 }}>
                  <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                  <XAxis dataKey="time" tick={{ fontSize: 10, fontFamily: '"JetBrains Mono", monospace' }} stroke="currentColor" />
                  <YAxis tick={{ fontSize: 10, fontFamily: '"JetBrains Mono", monospace' }} stroke="currentColor" />
                  <Tooltip contentStyle={ttStyle} />
                  <Legend wrapperStyle={{ fontSize: '10px', fontFamily: '"JetBrains Mono", monospace' }} />
                  {lines.filter(l => l.key.startsWith('i_') || l.key === 'inp').map(l => (
                    <Line key={l.key} type="monotone" dataKey={l.key} name={l.name.replace(/^(In TPS |P90 In TPS )/, '')} stroke={l.color} strokeWidth={2} dot={false} connectNulls />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            )}
          </div>

          {/* Balance Chart */}
          {balanceLines.length > 0 && (
            <div className="panel p-4">
              <h3 className="font-mono text-[12px] uppercase tracking-[0.1em] text-muted-foreground mb-3 flex items-center gap-2">
                <DollarSignIcon className="size-3.5 text-brand" />
                Balance ({balUnit})
                <span className="ml-auto font-mono text-[11px] text-muted-foreground normal-case tracking-normal">{chartSubtitle}</span>
              </h3>
              <ResponsiveContainer width="100%" height={200}>
                <LineChart data={chartData} margin={{ top: 5, right: 5, left: 0, bottom: 5 }}>
                  <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                  <XAxis dataKey="time" tick={{ fontSize: 10, fontFamily: '"JetBrains Mono", monospace' }} stroke="currentColor" />
                  <YAxis tick={{ fontSize: 10, fontFamily: '"JetBrains Mono", monospace' }} stroke="currentColor" />
                  <Tooltip contentStyle={ttStyle} />
                  <Legend wrapperStyle={{ fontSize: '10px', fontFamily: '"JetBrains Mono", monospace' }} />
                  {balanceLines.map(l => (
                    <Line key={l.key} type="monotone" dataKey={l.key} name={l.name} stroke={l.color} strokeWidth={2} dot={false} connectNulls />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}

          {/* Quota Usage Chart */}
          {quotaLines.length > 0 && (
            <div className="panel p-4">
              <h3 className="font-mono text-[12px] uppercase tracking-[0.1em] text-muted-foreground mb-3 flex items-center gap-2">
                <ShieldAlertIcon className="size-3.5 text-warning" />
                Quota Usage %
                <span className="ml-auto font-mono text-[11px] text-muted-foreground normal-case tracking-normal">{chartSubtitle}</span>
              </h3>
              <ResponsiveContainer width="100%" height={200}>
                <LineChart data={chartData} margin={{ top: 5, right: 5, left: 0, bottom: 5 }}>
                  <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
                  <XAxis dataKey="time" tick={{ fontSize: 10, fontFamily: '"JetBrains Mono", monospace' }} stroke="currentColor" />
                  <YAxis domain={[0, 100]} tick={{ fontSize: 10, fontFamily: '"JetBrains Mono", monospace' }} stroke="currentColor" />
                  <Tooltip contentStyle={ttStyle} />
                  <Legend wrapperStyle={{ fontSize: '10px', fontFamily: '"JetBrains Mono", monospace' }} />
                  {quotaLines.map(l => (
                    <Line key={l.key} type="monotone" dataKey={l.key} name={l.name} stroke={l.color} strokeWidth={2} dot={false} connectNulls />
                  ))}
                </LineChart>
              </ResponsiveContainer>
            </div>
          )}
        </div>
      )}

      {/* Bottom panels: model breakdown + event stream */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        {/* Left: Model Breakdown */}
        <div className="lg:col-span-1 space-y-4">
          {/* Model Breakdown */}
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
                    <button
                      key={m.name}
                      onClick={() => { setSelM(selM === m.name ? null : m.name); setSelP(null) }}
                      className={cn(
                        'w-full text-left p-2 border transition-colors',
                        selM === m.name ? 'border-brand/40 bg-brand/5' : 'border-border/50 hover:border-border bg-surface'
                      )}
                    >
                      <div className="flex items-center justify-between">
                        <span className="font-mono text-[12px] font-medium text-foreground truncate">{m.name}</span>
                        {sr !== null && (
                          <span className={cn('font-mono text-[11px] tabular-nums ml-2 shrink-0', sr >= 95 ? 'text-brand' : sr >= 80 ? 'text-warning' : 'text-destructive')}>
                            {sr.toFixed(1)}%
                          </span>
                        )}
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
                  const pt = pct(m.ttftVals, .9)
                  const po = pct(m.outTpsVals, .9)
                  const is = selP === sdata.name && selM === m.name
                  return (
                    <button
                      key={`${sdata.name}-${m.name}`}
                      onClick={() => { setSelP(sdata.name); setSelM(is ? null : m.name) }}
                      className={cn(
                        'w-full text-left p-2 border transition-colors',
                        is ? 'border-brand/40 bg-brand/5' : 'border-border/50 hover:border-border bg-surface'
                      )}
                    >
                      <div className="font-mono text-[12px] font-medium mb-1 text-foreground">{m.name}</div>
                      <div className="grid grid-cols-2 gap-x-2 gap-y-0.5 text-[11px] font-mono">
                        <span className="text-muted-foreground">P90 TTFT</span>
                        <span className="tabular-nums text-right">{pt !== null ? `${fmtNum(pt)}ms` : '—'}</span>
                        <span className="text-muted-foreground">P90 TPS</span>
                        <span className="tabular-nums text-right">{po !== null ? po.toFixed(1) : '—'}</span>
                        <span className="text-muted-foreground">REQS</span>
                        <span className="tabular-nums text-right">{m.requests}</span>
                      </div>
                    </button>
                  )
                })}
              </>
            )}
          </div>
        </div>

        {/* Right: Event Stream */}
        <div className="lg:col-span-2">
          <div className="panel p-4">
            <h3 className="font-mono text-[12px] uppercase tracking-[0.1em] text-muted-foreground mb-3 flex items-center gap-2">
              <ActivityIcon className="size-3.5 text-brand" /> Live Event Stream
              <span className="ml-auto font-mono text-[11px] text-muted-foreground normal-case tracking-normal">({liveEvents.length})</span>
            </h3>
            <div className="space-y-0 max-h-[450px] overflow-y-auto font-mono text-[11px]">
              <div className="flex items-center gap-2 px-1.5 py-1 border-b border-border text-[10px] uppercase tracking-wider text-muted-foreground sticky top-0 bg-surface">
                <span className="shrink-0 w-[82px] text-right pr-2">TIME</span>
                <span className="shrink-0 w-36">PROVIDER</span>
                <span className="shrink-0 w-28">MODEL</span>
                <span className="shrink-0 w-12">EVENT</span>
                <span className="flex-1 min-w-0">VALUE</span>
                <span className="shrink-0 w-28">USER</span>
              </div>
              {liveEvents.length === 0 ? (
                <p className="text-muted-foreground py-12 text-center">{wsStatus === 'connected' ? 'WAITING FOR EVENTS...' : 'NOT CONNECTED'}</p>
              ) : (
                liveEvents.map((ev, i) => {
                  const { label, value, kind } = fmt(ev.event)
                  const d = new Date(ev.timestamp_ms)
                  const t = d.toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' })
                  const ms = String(d.getMilliseconds()).padStart(3, '0')
                  const kc = kind === 'ok'
                    ? 'text-brand'
                    : kind === 'err'
                      ? 'text-destructive'
                      : kind === 'warn'
                        ? 'text-warning'
                        : 'text-muted-foreground'
                  const userText = fmtUser(ev.user)
                  return (
                    <div
                      key={`${ev.timestamp_ms}-${i}`}
                      className={cn(
                        'flex items-center gap-2 px-1.5 py-0.5 border-b border-border/50',
                        i === 0 && 'bg-brand/5'
                      )}
                    >
                      <span className="text-muted-foreground shrink-0 w-[82px] tabular-nums text-right pr-2">{t}.{ms}</span>
                      <span className="font-medium shrink-0 w-36 truncate" title={ev.provider}>{ev.provider}</span>
                      <span className="text-muted-foreground shrink-0 w-28 truncate" title={ev.model ?? ''}>{ev.model ?? ''}</span>
                      <span className={cn('shrink-0 w-12 text-[10px] px-1 py-0 border font-mono uppercase tracking-wider text-center', kc)} style={{ borderColor: 'currentColor' }}>
                        {label}
                      </span>
                      <span
                        className={cn('truncate flex-1 min-w-0', kind === 'err' && 'cursor-copy hover:underline')}
                        title={kind === 'err' ? `Click to copy: ${value}` : value}
                        onClick={() => { if (kind === 'err') navigator.clipboard.writeText(value) }}
                      >{value}</span>
                      <span className="shrink-0 w-32 text-[10px] text-muted-foreground truncate flex items-center gap-1" title={userText}>
                        {userText && userText.includes('🔑') ? <KeyIcon className="size-2.5 shrink-0" /> : userText ? <UserIcon className="size-2.5 shrink-0" /> : null}
                        {userText}
                      </span>
                    </div>
                  )
                })
              )}
              <div ref={endRef} />
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
