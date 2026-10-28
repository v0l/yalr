import { useCallback, useEffect, useRef, useState, useMemo } from 'react'
import { api, API_BASE_URL } from '../api/client'
import type { WsProviderMetrics, MetricsResponse, MetricsSnapshot, HealthOverviewResponse } from '../types'
import { cn } from '@/lib/utils'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle,
} from '@/components/ui/dialog'
import { BarChart3Icon, XIcon } from 'lucide-react'
import MetricsEventStream from './MetricsEventStream'
import MetricsCharts from './MetricsCharts'
import StatCard from '@/components/StatCard'
import { fmtNum, MAXL } from './metricsHelpers'
import { applyEvent, overallP90Ttft, p90, seed, type AggProvider } from './metricsAggregate'

const FLUSH_MS = 250

/* ═══════════════════════════════════════════════════════════════ */
/*  Metrics Page                                                  */
/* ═══════════════════════════════════════════════════════════════ */

export default function Metrics() {
  const [wsStatus, setWsStatus] = useState<'connecting' | 'connected' | 'disconnected'>('disconnected')
  const [liveEvents, setLiveEvents] = useState<WsProviderMetrics[]>([])
  const providersRef = useRef<Map<string, AggProvider>>(new Map())
  const [plist, setPlist] = useState<AggProvider[]>([])
  const [now, setNow] = useState(() => Date.now())
  const [skipped, setSkipped] = useState(0)
  const [selP, setSelP] = useState<string | null>(null)
  const [selM, setSelM] = useState<string | null>(null)
  const [modelDialogOpen, setModelDialogOpen] = useState(false)
  const [hist, setHist] = useState<MetricsSnapshot[] | null>(null)
  const [health, setHealth] = useState<HealthOverviewResponse | null>(null)
  const [loadingHist, setLoadingHist] = useState(false)
  const [metricTotals, setMetricTotals] = useState<{ totalReqs: number; totalOk: number; totalFail: number } | null>(null)
  const pending = useRef<WsProviderMetrics[]>([])
  const reconnectT = useRef<ReturnType<typeof setTimeout> | null>(null)
  const wsRef = useRef<WebSocket | null>(null)

  const publish = useCallback(() => {
    setPlist(Array.from(providersRef.current.values()).sort((a, b) => a.name.localeCompare(b.name)))
    setNow(Date.now())
  }, [])

  /* ── Batched flush of WebSocket events ──────────────────────── */
  useEffect(() => {
    const iv = setInterval(() => {
      const batch = pending.current
      if (batch.length === 0) return
      pending.current = []
      for (const e of batch) applyEvent(providersRef.current, e)
      setLiveEvents(prev => {
        const next = batch.length >= MAXL ? batch.slice(-MAXL) : batch
        return [...next.reverse(), ...prev].slice(0, MAXL)
      })
      publish()
    }, FLUSH_MS)
    return () => clearInterval(iv)
  }, [publish])

  /* ── Preload REST data ──────────────────────────────────────── */
  useEffect(() => {
    async function preload() {
      try {
        const [d, h] = await Promise.all([api.getMetrics(), api.getHealthOverview().catch(() => null)]) as [MetricsResponse, HealthOverviewResponse | null]
        if (h) setHealth(h)
        setMetricTotals({ totalReqs: d.total_requests, totalOk: d.total_successes, totalFail: d.total_failures })
        const map = seed(d)
        const evt = (d.recent_events as unknown as WsProviderMetrics[]).slice().reverse()
        for (const e of evt) applyEvent(map, e)
        for (const [name, p] of providersRef.current) if (!map.has(name)) map.set(name, p)
        providersRef.current = map
        setLiveEvents(prev => [...prev, ...evt.reverse()].slice(0, MAXL))
        publish()
      } catch { setPlist([]) }
    }
    preload()
  }, [publish])

  /* ── History load (once) + periodic health ──────────────────── */
  useEffect(() => {
    let c = false
    async function load() { setLoadingHist(true); try { const h = await api.getMetricsHistory(); if (!c) setHist(h) } catch { if (!c) setHist(null) } finally { if (!c) setLoadingHist(false) } }
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
          pending.current.push(d as WsProviderMetrics)
        } catch { setSkipped(n => n + 1) }
      }
      ws.onclose = () => { if (!c) { setWsStatus('disconnected'); wsRef.current = null; reconnectT.current = setTimeout(connect, 3000) } }
    }
    connect()
    return () => { c = true; if (reconnectT.current) clearTimeout(reconnectT.current); wsRef.current?.close() }
  }, [])

  /* ── Derived data ────────────────────────────────────────────── */
  const sdata = selP ? plist.find(p => p.name === selP) ?? null : null

  const fmodels = useMemo(() => {
    if (!sdata || !modelDialogOpen) return []
    return Array.from(sdata.models.values()).sort((a, b) => a.name.localeCompare(b.name))
  }, [sdata, modelDialogOpen])

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

  const summaryStats = useMemo(() => {
    const totalReqs = metricTotals?.totalReqs ?? plist.reduce((s, p) => s + p.requests, 0)
    const totalOk = metricTotals?.totalOk ?? plist.reduce((s, p) => s + p.successes, 0)
    const sr = totalReqs > 0 ? (totalOk / totalReqs * 100) : null
    const p90ttft = overallP90Ttft(plist)
    const active = plist.filter(p => p.inFlight > 0 || (now - p.lastEvent < 60000)).length
    return { totalReqs, totalOk, sr, p90ttft, active, total: plist.length }
  }, [plist, metricTotals, now])

  const clearEvents = useCallback(() => setLiveEvents([]), [])

  const healthByProvider = useMemo(() => new Map((health?.providers ?? []).map(h => [h.provider, h])), [health])

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
            const sr = p.requests > 0 ? (p.successes / p.requests * 100) : null
            const tt = p90(p.ttft)
            const ot = p90(p.outTps)
            const is = selP === p.name
            const h = healthByProvider.get(p.name)
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
          <StatCard label="Providers" value={
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
      {loadingHist && !hist && (
        <div className="panel flex items-center justify-center font-mono text-[13px] text-muted-foreground" style={{ height: 340 }}><span className="animate-pulse">LOADING...</span></div>
      )}
      {hist && hist.length > 0 && <MetricsCharts hist={hist} selP={selP} selM={selM} />}

      {/* Event Stream */}
      <div className="space-y-4">
        {/* Model filter bar */}
        <div className="flex items-center gap-2">
          <button
            onClick={() => setModelDialogOpen(true)}
            className={cn(
              'flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-wider px-2.5 py-1.5 border transition-colors',
              selM ? 'text-brand border-brand/30 bg-brand/5' : 'text-muted-foreground border-border/50 hover:border-border'
            )}
          >
            <BarChart3Icon className="size-3" />
            MODELS
            <span className="text-[9px] opacity-50">{allModels.length}</span>
            {selM && <span className="text-[9px] text-brand ml-0.5">: {selM}</span>}
          </button>
          {selM && (
            <button onClick={() => setSelM(null)} className="font-mono text-[10px] text-muted-foreground hover:text-foreground">
              <XIcon className="size-3 inline mr-0.5" />
              CLEAR
            </button>
          )}
        </div>
        <MetricsEventStream events={liveEvents} wsStatus={wsStatus} skipped={skipped} onClear={clearEvents} />
      </div>

      {/* Model Selector Dialog */}
      <Dialog open={modelDialogOpen} onOpenChange={setModelDialogOpen}>
        <DialogContent className="max-w-lg max-h-[80vh] overflow-y-auto p-0 gap-0">
          <DialogHeader className="px-4 pt-4 pb-2 border-b border-border">
            <DialogTitle className="font-display text-base tracking-wider text-foreground">SELECT MODEL</DialogTitle>
          </DialogHeader>
          <div className="px-4 py-3 space-y-1">
            {allModels.length === 0 ? (
              <p className="font-mono text-[12px] text-muted-foreground py-4 text-center">No model data yet</p>
            ) : (
              allModels.map(m => {
                const sr = m.requests > 0 ? (m.successes / m.requests * 100) : null
                return (
                  <button
                    key={m.name}
                    onClick={() => { setSelM(selM === m.name ? null : m.name); setSelP(null); setModelDialogOpen(false) }}
                    className={cn(
                      'w-full text-left p-3 border transition-colors',
                      selM === m.name ? 'border-brand/40 bg-brand/5' : 'border-border/50 hover:border-border bg-surface'
                    )}
                  >
                    <div className="flex items-center justify-between mb-0.5">
                      <span className="font-mono text-[13px] font-semibold text-foreground">{m.name}</span>
                      {sr !== null && (
                        <span className={cn('font-mono text-[11px] tabular-nums ml-2 shrink-0', sr >= 95 ? 'text-brand' : sr >= 80 ? 'text-warning' : 'text-destructive')}>{sr.toFixed(1)}%</span>
                      )}
                    </div>
                    <div className="text-muted-foreground text-[10px] font-mono">{m.requests} reqs · {m.successes} ok · {m.failures} fail</div>
                  </button>
                )
              })
            )}
            {/* Provider-specific models when a provider is selected */}
            {sdata && fmodels.length > 0 && (
              <>
                <div className="section-header mt-4">{sdata.name} MODELS</div>
                {fmodels.map(m => {
                  const pt = p90(m.ttft); const po = p90(m.outTps)
                  const is = selP === sdata.name && selM === m.name
                  return (
                    <button
                      key={`${sdata.name}-${m.name}`}
                      onClick={() => { setSelP(sdata.name); setSelM(is ? null : m.name); setModelDialogOpen(false) }}
                      className={cn('w-full text-left p-3 border transition-colors', is ? 'border-brand/40 bg-brand/5' : 'border-border/50 hover:border-border bg-surface')}
                    >
                      <div className="font-mono text-[13px] font-semibold mb-0.5 text-foreground">{m.name}</div>
                      <div className="flex gap-4 text-[11px] font-mono text-muted-foreground">
                        <span>P90 TTFT <span className="text-foreground tabular-nums ml-1">{pt !== null ? `${fmtNum(pt)}ms` : '—'}</span></span>
                        <span>P90 TPS <span className="text-foreground tabular-nums ml-1">{po !== null ? po.toFixed(1) : '—'}</span></span>
                        <span>REQS <span className="text-foreground tabular-nums ml-1">{m.requests}</span></span>
                      </div>
                    </button>
                  )
                })}
              </>
            )}
          </div>
          <div className="px-4 py-2 border-t border-border flex justify-end">
            <button onClick={() => setModelDialogOpen(false)} className="font-mono text-[11px] uppercase tracking-wider text-muted-foreground hover:text-foreground px-2 py-1">
              CLOSE
            </button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
