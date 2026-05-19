import { useEffect, useRef, useState, useCallback, useMemo } from 'react'
import { api, API_BASE_URL } from '../api/client'
import type { WsProviderMetrics, MetricsResponse, MetricsSnapshot, HealthOverviewResponse, CurrencyAmount } from '../types'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { cn, formatBalance } from '@/lib/utils'
import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer, Legend } from 'recharts'
import {
  ActivityIcon, GaugeIcon, ShieldAlertIcon, BarChart3Icon,
  AlertTriangleIcon, CheckCircle2Icon, ClockIcon
} from 'lucide-react'

/* ------------------------------------------------------------------ */
/*  helpers                                                            */
/* ------------------------------------------------------------------ */

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
  if (e === 'Success') return { label: 'OK', value: '\u2713', kind: 'ok' }
  if (has(e, 'TTFT')) return { label: 'TTFT', value: `${e.TTFT}ms`, kind: 'info' }
  if (has(e, 'OutputTokensPerSecond')) return { label: 'tok/s', value: (e.OutputTokensPerSecond as number).toFixed(1), kind: 'info' }
  if (has(e, 'TotalLatency')) return { label: 'Lat', value: `${e.TotalLatency}ms`, kind: 'info' }
  if (has(e, 'InputTokens')) return { label: 'In tok', value: String(e.InputTokens), kind: 'info' }
  if (has(e, 'OutputTokens')) return { label: 'Out tok', value: String(e.OutputTokens), kind: 'info' }
  if (has(e, 'Failure')) {
    const f = e.Failure as { error_message: string }
    return { label: 'FAIL', value: f.error_message, kind: 'err' }
  }
  if (has(e, 'ProviderLoad')) {
    const l = e.ProviderLoad as { in_flight: number; max_concurrency: number | null }
    return {
      label: 'Load', value: `${l.in_flight}${l.max_concurrency ? `/${l.max_concurrency}` : ''}`,
      kind: l.in_flight > 0 ? 'warn' : 'ok'
    }
  }
  if (has(e, 'Balance')) {
    const b = e.Balance as CurrencyAmount
    return { label: 'Balance', value: formatBalance(b.amount, b.currency), kind: 'info' }
  }
  return { label: '?', value: JSON.stringify(e), kind: 'info' }
}

const MAXL = 200, MAXA = 500

/* ------------------------------------------------------------------ */
/*  component                                                          */
/* ------------------------------------------------------------------ */

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
        : { name: m.provider, models: new Map(), totalRequests: 0, successes: 0, failures: 0,
            ttftVals: [], latVals: [], outTpsVals: [], lastEvent: m.timestamp_ms,
            inFlight: 0, maxConcurrency: null }
      if (isO) { p.totalRequests++; m.event === 'Success' ? p.successes++ : p.failures++ }
      if (isT)  p.ttftVals    = [...p.ttftVals,    (m.event as Record<string, number>).TTFT].slice(-MAXA)
      if (isL)  p.latVals     = [...p.latVals,     (m.event as Record<string, number>).TotalLatency].slice(-MAXA)
      if (isOT) p.outTpsVals  = [...p.outTpsVals,  (m.event as Record<string, number>).OutputTokensPerSecond].slice(-MAXA)
      if (m.event && has(m.event, 'ProviderLoad')) {
        const l = (m.event as Record<string, unknown>).ProviderLoad as { in_flight: number; max_concurrency: number | null }
        p.inFlight = l.in_flight; p.maxConcurrency = l.max_concurrency
      }
      if (m.model) {
        let ms = p.models.get(m.model)
        const m2: ModelStats = ms
          ? { ...ms, lastEvent: m.timestamp_ms }
          : { name: m.model, requests: 0, successes: 0, failures: 0,
              ttftVals: [], latVals: [], outTpsVals: [], lastEvent: m.timestamp_ms }
        if (isO)  { m2.requests++; m.event === 'Success' ? m2.successes++ : m2.failures++ }
        if (isT)  m2.ttftVals   = [...m2.ttftVals,   (m.event as Record<string, number>).TTFT].slice(-MAXA)
        if (isL)  m2.latVals    = [...m2.latVals,    (m.event as Record<string, number>).TotalLatency].slice(-MAXA)
        if (isOT) m2.outTpsVals = [...m2.outTpsVals, (m.event as Record<string, number>).OutputTokensPerSecond].slice(-MAXA)
        p.models = new Map(p.models).set(m.model, m2)
      }
      n.set(m.provider, p); return n
    })
  }, [])

  /* ---- preload REST data ---- */
  useEffect(() => {
    async function preload() {
      try {
        const [d, h] = await Promise.all([
          api.getMetrics(),
          api.getHealthOverview().catch(() => null),
        ]) as [MetricsResponse, HealthOverviewResponse | null]
        if (h) setHealth(h)
        const map = new Map<string, AggProvider>()
        for (const p of d.providers) {
          if (p.provider) map.set(p.provider, {
            name: p.provider, models: new Map(),
            totalRequests: 0, successes: 0, failures: 0,
            ttftVals: p.p90_ttft_ms != null ? [p.p90_ttft_ms] : [],
            latVals: p.avg_latency_ms != null ? [p.avg_latency_ms] : [],
            outTpsVals: p.p90_tokens_per_second != null ? [p.p90_tokens_per_second] : [],
            lastEvent: Date.now(), inFlight: p.in_flight ?? 0,
            maxConcurrency: p.max_concurrency,
          })
        }
        const evt = (d.recent_events as unknown as WsProviderMetrics[]).reverse()
        for (const e of evt) {
          const isO = e.event === 'Success' || has(e.event, 'Failure')
          let p = map.get(e.provider)
          if (!p) {
            p = { name: e.provider, models: new Map(), totalRequests: 0, successes: 0, failures: 0,
                  ttftVals: [], latVals: [], outTpsVals: [], lastEvent: e.timestamp_ms,
                  inFlight: 0, maxConcurrency: null }
            map.set(e.provider, p)
          } else p.lastEvent = e.timestamp_ms
          if (isO) { p.totalRequests++; e.event === 'Success' ? p.successes++ : p.failures++ }
          if (has(e.event, 'TTFT'))                p.ttftVals.push((e.event as Record<string, number>).TTFT)
          if (has(e.event, 'TotalLatency'))         p.latVals.push((e.event as Record<string, number>).TotalLatency)
          if (has(e.event, 'OutputTokensPerSecond')) p.outTpsVals.push((e.event as Record<string, number>).OutputTokensPerSecond)
          if (e.event && has(e.event, 'ProviderLoad')) {
            const l = (e.event as Record<string, unknown>).ProviderLoad as { in_flight: number; max_concurrency: number | null }
            p.inFlight = l.in_flight; p.maxConcurrency = l.max_concurrency
          }
          if (e.model) {
            let ms = p.models.get(e.model)
            if (!ms) {
              ms = { name: e.model, requests: 0, successes: 0, failures: 0,
                     ttftVals: [], latVals: [], outTpsVals: [], lastEvent: e.timestamp_ms }
              p.models.set(e.model, ms)
            } else ms.lastEvent = e.timestamp_ms
            if (isO) { ms.requests++; e.event === 'Success' ? ms.successes++ : ms.failures++ }
            if (has(e.event, 'TTFT'))                ms.ttftVals.push((e.event as Record<string, number>).TTFT)
            if (has(e.event, 'TotalLatency'))         ms.latVals.push((e.event as Record<string, number>).TotalLatency)
            if (has(e.event, 'OutputTokensPerSecond')) ms.outTpsVals.push((e.event as Record<string, number>).OutputTokensPerSecond)
          }
        }
        setProviders(map); setLiveEvents(evt.reverse().slice(0, MAXL))
      } catch { /* non-critical */ }
    }
    preload()
  }, [])

  /* ---- load history + periodic health ---- */
  useEffect(() => {
    let c = false
    async function load() {
      setLoadingHist(true)
      try { const h = await api.getMetricsHistory(); if (!c) setHist(h) } catch {}
      finally { if (!c) setLoadingHist(false) }
    }
    load()
    const iv = setInterval(() => {
      load()
      api.getHealthOverview().then(setHealth).catch(() => {})
    }, 60000)
    return () => { c = true; clearInterval(iv) }
  }, [])

  /* ---- websocket ---- */
  useEffect(() => {
    let c = false
    function connect() {
      if (c) return
      const tok = localStorage.getItem('token')
      if (!tok) { setWsStatus('disconnected'); return }
      const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
      let base: string
      try { const u = new URL(API_BASE_URL); u.protocol = proto; base = u.toString().replace(/\/$/, '') }
      catch { base = `${proto}//${window.location.host}` }
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
      ws.onclose = () => {
        if (!c) { setWsStatus('disconnected'); wsRef.current = null; reconnectT.current = setTimeout(connect, 3000) }
      }
    }
    connect()
    return () => { c = true; if (reconnectT.current) clearTimeout(reconnectT.current); wsRef.current?.close() }
  }, [processEvent])

  useEffect(() => {
    const c = endRef.current?.parentElement
    if (c) c.scrollTo({ top: 0, behavior: 'smooth' })
  }, [liveEvents.length])

  /* ---- derived data ---- */
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
      return e
    })
  }, [hist, selP, selM])

  const lines = useMemo(() => {
    const ks = new Set<string>()
    for (const d of chartData) for (const k of Object.keys(d)) {
      if (k.startsWith('t_') || k.startsWith('o_') || k.startsWith('i_')) ks.add(k)
    }
    const clr = ['#f59e0b', '#06b6d4', '#a855f7', '#ec4899', '#84cc16', '#f97316']
    const all = Array.from(ks)
    const out: { key: string; name: string; color: string }[] = []
    if (all.length === 0 && !selP) return [
      { key: 'ttft', name: 'P90 TTFT (ms)', color: clr[0] },
      { key: 'out', name: 'P90 Out tok/s', color: clr[1] },
      { key: 'inp', name: 'P90 In tok/s', color: clr[2] },
    ]
    all.forEach((k, i) => {
      const lp = k.slice(2).replace('/', ' / ')
      const lbl = k.startsWith('t_') ? `TTFT ${lp}` : k.startsWith('o_') ? `Out TPS ${lp}` : `In TPS ${lp}`
      out.push({ key: k, name: lbl, color: clr[i % clr.length] })
    })
    return out
  }, [chartData, selP])

  const fmodels = useMemo(() => {
    if (!sdata) return []
    return Array.from(sdata.models.values()).sort((a, b) => b.lastEvent - a.lastEvent)
  }, [sdata])

  const ttStyle = {
    background: 'var(--card)', border: '1px solid var(--border)',
    borderRadius: '8px', fontSize: '12px'
  }

  /* ================================================================ */
  /*  render                                                           */
  /* ================================================================ */
  return (
    <div className="flex flex-col gap-6 p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold text-foreground">Metrics</h1>
          <p className="text-sm text-muted-foreground">Real-time provider performance &amp; historical trends</p>
        </div>
        <div className="flex items-center gap-3">
          {skipped > 0 && <span className="text-xs text-amber-500 font-mono">{skipped} skipped</span>}
          <Badge variant="secondary" className="gap-1.5">
            <span className={cn('size-2 rounded-full',
              wsStatus==='connected'?'bg-emerald-500':wsStatus==='connecting'?'bg-amber-500':'bg-destructive')}/>
            {wsStatus==='connected'?'Live':wsStatus==='connecting'?'Connecting':'Disconnected'}
          </Badge>
        </div>
      </div>

      {plist.length===0 ? <Card><CardContent className="py-12 text-center text-muted-foreground">Waiting for metrics…</CardContent></Card> :
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5 gap-3">
        {plist.map(p=>{const sr=p.totalRequests>0?(p.successes/p.totalRequests*100):null
          const tt=pct(p.ttftVals,.9);const ot=pct(p.outTpsVals,.9);const is=selP===p.name
          return <button key={p.name} onClick={()=>{setSelP(is?null:p.name);setSelM(null)}}
            className={cn('text-left p-3 bg-card rounded-xl border transition-colors cursor-pointer',
              is?'border-primary ring-1 ring-primary/30':'border-border hover:border-primary/40')}>
            <div className="flex items-center justify-between mb-1.5">
              <span className="text-sm font-semibold">{p.name}</span>
              <span className="text-[10px] text-muted-foreground">{p.models.size}m</span>
            </div>
            <div className="grid grid-cols-2 gap-x-2 gap-y-1 text-xs">
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground">Load</span>
                <span className="font-mono">{p.inFlight}{p.maxConcurrency?<span className="text-muted-foreground">/{p.maxConcurrency}</span>:''}</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground">Success</span>
                <span className={cn('font-mono',sr!==null?(sr>=95?'text-emerald-500':sr>=80?'text-amber-500':'text-destructive'):'text-muted-foreground')}>
                  {sr!==null?`${sr.toFixed(1)}%`:'\u2014'}</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground">P90 TTFT</span>
                <span className="font-mono">{tt!==null?`${tt}ms`:'\u2014'}</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground">P90 tok/s</span>
                <span className="font-mono">{ot!==null?ot.toFixed(1):'\u2014'}</span>
              </div>
            </div></button>})} 
      </div>}

      {hist&&hist.length>0&&<div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
        <Card><CardHeader className="pb-2"><CardTitle className="text-base flex items-center gap-2">
          <ActivityIcon className="size-4 text-primary"/>
          P90 TTFT<span className="text-xs font-normal text-muted-foreground ml-auto">{selP||'All providers'}{selM?` / ${selM}`:''}</span></CardTitle></CardHeader>
          <CardContent>{loadingHist?<div className="h-48 flex items-center justify-center text-sm text-muted-foreground">Loading...</div>:
          <ResponsiveContainer width="100%" height={200}><LineChart data={chartData} margin={{top:5,right:5,left:0,bottom:5}}>
            <CartesianGrid strokeDasharray="3 3" stroke="var(--border)"/><XAxis dataKey="time" tick={{fontSize:11}} stroke="var(--muted-foreground)"/>
            <YAxis tick={{fontSize:11}} stroke="var(--muted-foreground)"/><Tooltip contentStyle={ttStyle}/><Legend wrapperStyle={{fontSize:'11px'}}/>
            {lines.filter(l=>l.key.startsWith('t_')).map(l=><Line key={l.key} type="monotone" dataKey={l.key} name={l.name} stroke={l.color} strokeWidth={2} dot={false} connectNulls/>)}
          </LineChart></ResponsiveContainer>}</CardContent></Card>
        <Card><CardHeader className="pb-2"><CardTitle className="text-base flex items-center gap-2">
          <GaugeIcon className="size-4 text-primary"/>
          P90 Tokens/Second<span className="text-xs font-normal text-muted-foreground ml-auto">{selP||'All providers'}</span></CardTitle></CardHeader>
          <CardContent>{loadingHist?<div className="h-48 flex items-center justify-center text-sm text-muted-foreground">Loading...</div>:
          <ResponsiveContainer width="100%" height={200}><LineChart data={chartData} margin={{top:5,right:5,left:0,bottom:5}}>
            <CartesianGrid strokeDasharray="3 3" stroke="var(--border)"/><XAxis dataKey="time" tick={{fontSize:11}} stroke="var(--muted-foreground)"/>
            <YAxis tick={{fontSize:11}} stroke="var(--muted-foreground)"/><Tooltip contentStyle={ttStyle}/><Legend wrapperStyle={{fontSize:'11px'}}/>
            {lines.filter(l=>l.key.startsWith('o_')||l.key.startsWith('i_')).map(l=><Line key={l.key} type="monotone" dataKey={l.key} name={l.name} stroke={l.color} strokeWidth={2} dot={false} connectNulls/>)}
          </LineChart></ResponsiveContainer>}</CardContent></Card>
      </div>}

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        <div className="lg:col-span-1 space-y-4">
          <Card><CardHeader className="pb-2"><CardTitle className="text-sm flex items-center gap-2"><BarChart3Icon className="size-4 text-primary"/>Model Breakdown</CardTitle></CardHeader>
            <CardContent>{!sdata?<p className="text-sm text-muted-foreground">Select a provider above</p>:fmodels.length===0?<p className="text-sm text-muted-foreground">No model-level data yet</p>:
            <div className="space-y-2">{fmodels.map(m=>{const sr=m.requests>0?(m.successes/m.requests*100):null
              const mm=(v:number[],pp:number):number|null=>pct(v,pp)
              const avg=(v:number[]):number|null=>v.length>0?v.reduce((a,b)=>a+b,0)/v.length:null
              const pt=mm(m.ttftVals,.9);const po=mm(m.outTpsVals,.9);const al=avg(m.latVals)
              return <button key={m.name} onClick={()=>setSelM(selM===m.name?null:m.name)}
                className={cn('w-full text-left p-3 bg-muted/50 rounded-lg transition-colors',selM===m.name&&'ring-1 ring-primary/40')}>
                <div className="font-mono text-sm font-medium mb-2">{m.name}</div>
                <div className="grid grid-cols-2 gap-1.5 text-xs">
                  <span className="text-muted-foreground">Success</span>
                  <span className={cn('font-mono text-right',sr!==null?(sr>=95?'text-emerald-500':sr>=80?'text-amber-500':'text-destructive'):'text-muted-foreground')}>{sr!==null?`${sr.toFixed(1)}%`:'\u2014'}</span>
                  <span className="text-muted-foreground">P90 TTFT</span><span className="font-mono text-right">{pt!==null?`${pt}ms`:'\u2014'}</span>
                  <span className="text-muted-foreground">P90 tok/s</span><span className="font-mono text-right">{po!==null?po.toFixed(1):'\u2014'}</span>
                  <span className="text-muted-foreground">Avg Lat</span><span className="font-mono text-right">{al!==null?`${al.toFixed(0)}ms`:'\u2014'}</span>
                </div></button>})}
            </div>}</CardContent></Card>

          <Card><CardHeader className="pb-2"><CardTitle className="text-sm flex items-center gap-2"><ShieldAlertIcon className="size-4 text-primary"/>Provider Health
            {health&&<span className="ml-auto flex gap-2 text-xs font-normal">
              {health.unhealthy_count>0&&<Badge variant="destructive" className="text-[10px] px-1.5 py-0">{health.unhealthy_count} down</Badge>}
              {health.degraded_count>0&&<Badge className="text-[10px] px-1.5 py-0 bg-amber-500/20 text-amber-500 border-0">{health.degraded_count} degraded</Badge>}
            </span>}
          </CardTitle></CardHeader>
          <CardContent>{!health||health.providers.length===0?<p className="text-sm text-muted-foreground">No providers</p>:
            <div className="space-y-1.5">{health.providers.map(h=>{const icon=h.health_state==='healthy'
              ?<CheckCircle2Icon className="size-3.5 text-emerald-500"/>
              :h.health_state==='degraded'?<AlertTriangleIcon className="size-3.5 text-amber-500"/>
              :<ShieldAlertIcon className="size-3.5 text-destructive"/>
              const lf=h.last_failure_ago_ms!=null?(h.last_failure_ago_ms<60000?`${Math.round(h.last_failure_ago_ms/1000)}s`:`${Math.round(h.last_failure_ago_ms/60000)}m`):null
              return <div key={h.provider} className="flex items-center justify-between text-xs p-2 bg-muted/30 rounded-lg gap-2">
                <div className="flex items-center gap-2 min-w-0">{icon}<span className="font-medium truncate">{h.provider}</span>
                  {h.rate_limited&&<Badge variant="outline" className="text-[9px] px-1 py-0 border-amber-500/30 text-amber-500">RL</Badge>}</div>
                <div className="flex items-center gap-2 shrink-0">
                  {h.backoff_ms>0&&<span className="flex items-center gap-1 text-muted-foreground" title="Backoff"><ClockIcon className="size-3"/>{h.backoff_ms>=1000?`${(h.backoff_ms/1000).toFixed(1)}s`:`${h.backoff_ms}ms`}</span>}
                  {lf&&<span className="text-muted-foreground">{lf}</span>}
                  <span className={cn('font-mono',h.consecutive_failures===0?'text-emerald-500':'text-destructive')}>{h.consecutive_failures}f</span>
                </div></div>})}
            </div>}</CardContent></Card>
        </div>

        <div className="lg:col-span-2"><Card><CardHeader className="pb-2"><CardTitle className="text-base flex items-center gap-2">
          <ActivityIcon className="size-4 text-primary"/>Live Event Stream<span className="ml-2 text-sm font-normal text-muted-foreground">({liveEvents.length})</span></CardTitle></CardHeader>
          <CardContent><div className="space-y-0.5 max-h-[450px] overflow-y-auto font-mono text-xs">
            {liveEvents.length===0?<p className="text-muted-foreground py-8 text-center">{wsStatus==='connected'?'Waiting...':'Not connected'}</p>:
            liveEvents.map((ev,i)=>{const{label,value,kind}=fmt(ev.event)
              const d=new Date(ev.timestamp_ms);const t=d.toLocaleTimeString('en-US',{hour12:false,hour:'2-digit',minute:'2-digit',second:'2-digit'})
              const ms=String(d.getMilliseconds()).padStart(3,'0')
              const kc=kind==='ok'?'bg-emerald-500/10 text-emerald-500':kind==='err'?'bg-destructive/15 text-destructive':kind==='warn'?'bg-amber-500/10 text-amber-500':'bg-muted text-muted-foreground'
              return <div key={`${ev.timestamp_ms}-${i}`} className={cn('flex items-center gap-2 px-1.5 py-0.5 rounded',i===0&&'bg-accent/10')}>
                <span className="text-muted-foreground shrink-0">{t}.{ms}</span><span className="font-medium shrink-0 w-24 truncate" title={ev.provider}>{ev.provider}</span>
                {ev.model&&<span className="text-muted-foreground shrink-0 w-28 truncate" title={ev.model}>{ev.model}</span>}
                <Badge variant="outline" className={cn('shrink-0 text-[10px] px-1 py-0 border-0',kc)}>{label}</Badge>
                <span className="truncate" title={value}>{value}</span></div>})}
            <div ref={endRef}/></div></CardContent></Card></div>
      </div>
    </div>
  )
}
