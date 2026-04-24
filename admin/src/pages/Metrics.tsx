import { useEffect, useRef, useState, useCallback } from 'react'
import { api, API_BASE_URL } from '../api/client'
import type { WsProviderMetrics, WsMetricsEvent, WsFailureDetails, MetricsResponse } from '../types'

interface AggregatedProvider {
  name: string
  models: Map<string, ModelStats>
  totalRequests: number
  successes: number
  failures: number
  ttftValues: number[]
  latencyValues: number[]
  outputTpsValues: number[]
  lastEvent: number
}

interface ModelStats {
  name: string
  requests: number
  successes: number
  failures: number
  ttftValues: number[]
  latencyValues: number[]
  outputTpsValues: number[]
  lastEvent: number
}

function percentile(values: number[], p: number): number | null {
  if (values.length === 0) return null
  const sorted = [...values].sort((a, b) => a - b)
  const idx = Math.round(p * (sorted.length - 1))
  return sorted[idx]
}

function hasKey<T extends string>(obj: unknown, key: T): obj is Record<T, unknown> {
  return typeof obj === 'object' && obj !== null && key in obj
}

function formatEventType(event: WsMetricsEvent): { label: string; value: string; kind: 'ok' | 'warn' | 'err' | 'info' } {
  if (event === 'Success') return { label: 'Success', value: '✓', kind: 'ok' }
  if (hasKey(event, 'TTFT')) return { label: 'TTFT', value: `${event.TTFT}ms`, kind: 'info' }
  if (hasKey(event, 'OutputTokensPerSecond')) return { label: 'Out tok/s', value: (event.OutputTokensPerSecond as number).toFixed(1), kind: 'info' }
  if (hasKey(event, 'InputTokensPerSecond')) return { label: 'In tok/s', value: (event.InputTokensPerSecond as number).toFixed(1), kind: 'info' }
  if (hasKey(event, 'TotalLatency')) return { label: 'Latency', value: `${event.TotalLatency}ms`, kind: 'info' }
  if (hasKey(event, 'InputTokens')) return { label: 'In tokens', value: String(event.InputTokens), kind: 'info' }
  if (hasKey(event, 'OutputTokens')) return { label: 'Out tokens', value: String(event.OutputTokens), kind: 'info' }
  if (hasKey(event, 'Failure')) return { label: 'Failure', value: (event.Failure as WsFailureDetails).error_message, kind: 'err' }
  if (hasKey(event, 'ProviderLoad')) {
    const load = event.ProviderLoad as { in_flight: number; max_concurrency: number | null }
    const cap = load.max_concurrency ? `/${load.max_concurrency}` : ''
    return { label: 'Load', value: `${load.in_flight}${cap}`, kind: load.in_flight > 0 ? 'warn' : 'ok' }
  }
  return { label: 'Unknown', value: JSON.stringify(event), kind: 'info' }
}

const MAX_LIVE_EVENTS = 200
const MAX_AGGREGATION_EVENTS = 500

export default function Metrics() {
  const [wsStatus, setWsStatus] = useState<'connecting' | 'connected' | 'disconnected'>('disconnected')
  const [liveEvents, setLiveEvents] = useState<WsProviderMetrics[]>([])
  const [providers, setProviders] = useState<Map<string, AggregatedProvider>>(new Map())
  const [skipped, setSkipped] = useState(0)
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null)
  const eventsEndRef = useRef<HTMLDivElement>(null)
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const wsRef = useRef<WebSocket | null>(null)

  const processEvent = useCallback((m: WsProviderMetrics) => {
    setLiveEvents(prev => {
      const next = [m, ...prev].slice(0, MAX_LIVE_EVENTS)
      return next
    })

    setProviders(prev => {
      const next = new Map(prev)
      const existing = next.get(m.provider)
      const isOutcome = m.event === 'Success' || hasKey(m.event, 'Failure')
      const isTtft = hasKey(m.event, 'TTFT')
      const isLatency = hasKey(m.event, 'TotalLatency')
      const isOutTps = hasKey(m.event, 'OutputTokensPerSecond')

      const p: AggregatedProvider = existing
        ? { ...existing, lastEvent: m.timestamp_ms }
        : { name: m.provider, models: new Map(), totalRequests: 0, successes: 0, failures: 0, ttftValues: [], latencyValues: [], outputTpsValues: [], lastEvent: m.timestamp_ms }

      if (isOutcome) {
        p.totalRequests++
        if (m.event === 'Success') p.successes++
        if (hasKey(m.event, 'Failure')) p.failures++
      }
      if (isTtft) p.ttftValues = [...p.ttftValues, (m.event as Record<string, number>).TTFT].slice(-MAX_AGGREGATION_EVENTS)
      if (isLatency) p.latencyValues = [...p.latencyValues, (m.event as Record<string, number>).TotalLatency].slice(-MAX_AGGREGATION_EVENTS)
      if (isOutTps) p.outputTpsValues = [...p.outputTpsValues, (m.event as Record<string, number>).OutputTokensPerSecond].slice(-MAX_AGGREGATION_EVENTS)

      // Model-level aggregation
      if (m.model) {
        const modelStats = p.models.get(m.model)
        const ms: ModelStats = modelStats
          ? { ...modelStats, lastEvent: m.timestamp_ms }
          : { name: m.model, requests: 0, successes: 0, failures: 0, ttftValues: [], latencyValues: [], outputTpsValues: [], lastEvent: m.timestamp_ms }

        if (isOutcome) {
          ms.requests++
          if (m.event === 'Success') ms.successes++
          if (hasKey(m.event, 'Failure')) ms.failures++
        }
        if (isTtft) ms.ttftValues = [...ms.ttftValues, (m.event as Record<string, number>).TTFT].slice(-MAX_AGGREGATION_EVENTS)
        if (isLatency) ms.latencyValues = [...ms.latencyValues, (m.event as Record<string, number>).TotalLatency].slice(-MAX_AGGREGATION_EVENTS)
        if (isOutTps) ms.outputTpsValues = [...ms.outputTpsValues, (m.event as Record<string, number>).OutputTokensPerSecond].slice(-MAX_AGGREGATION_EVENTS)

        p.models = new Map(p.models).set(m.model, ms)
      }

      next.set(m.provider, p)
      return next
    })
  }, [])

  // Preload historical data from REST endpoint
  useEffect(() => {
    async function preload() {
      try {
        const data: MetricsResponse = await api.getMetrics()

        // Seed providers from REST summary (always present, even with no events)
        const map = new Map<string, AggregatedProvider>()
        for (const p of data.providers) {
          if (p.provider) {
            map.set(p.provider, {
              name: p.provider,
              models: new Map(),
              totalRequests: 0,
              successes: 0,
              failures: 0,
              ttftValues: p.p90_ttft_ms != null ? [p.p90_ttft_ms] : [],
              latencyValues: p.avg_latency_ms != null ? [p.avg_latency_ms] : [],
              outputTpsValues: p.p90_tokens_per_second != null ? [p.p90_tokens_per_second] : [],
              lastEvent: Date.now(),
            })
          }
        }

        // Process recent events to enrich with model-level data and accurate counts
        // REST returns newest-first; process oldest-first so counters accumulate correctly
        const events: WsProviderMetrics[] = data.recent_events
          .filter((e): e is Record<string, unknown> => typeof e === 'object' && e !== null)
          .map(e => e as unknown as WsProviderMetrics)
          .reverse() // now oldest-first

        for (const ev of events) {
          const isOutcome = ev.event === 'Success' || hasKey(ev.event, 'Failure')
          const isTtft = hasKey(ev.event, 'TTFT')
          const isLatency = hasKey(ev.event, 'TotalLatency')
          const isOutTps = hasKey(ev.event, 'OutputTokensPerSecond')

          let p = map.get(ev.provider)
          if (!p) {
            p = { name: ev.provider, models: new Map(), totalRequests: 0, successes: 0, failures: 0, ttftValues: [], latencyValues: [], outputTpsValues: [], lastEvent: ev.timestamp_ms }
            map.set(ev.provider, p)
          } else {
            p.lastEvent = ev.timestamp_ms
          }

          if (isOutcome) {
            p.totalRequests++
            if (ev.event === 'Success') p.successes++
            if (hasKey(ev.event, 'Failure')) p.failures++
          }
          if (isTtft) p.ttftValues.push((ev.event as Record<string, number>).TTFT)
          if (isLatency) p.latencyValues.push((ev.event as Record<string, number>).TotalLatency)
          if (isOutTps) p.outputTpsValues.push((ev.event as Record<string, number>).OutputTokensPerSecond)

          // Model-level aggregation
          if (ev.model) {
            let ms = p.models.get(ev.model)
            if (!ms) {
              ms = { name: ev.model, requests: 0, successes: 0, failures: 0, ttftValues: [], latencyValues: [], outputTpsValues: [], lastEvent: ev.timestamp_ms }
              p.models.set(ev.model, ms)
            } else {
              ms.lastEvent = ev.timestamp_ms
            }

            if (isOutcome) {
              ms.requests++
              if (ev.event === 'Success') ms.successes++
              if (hasKey(ev.event, 'Failure')) ms.failures++
            }
            if (isTtft) ms.ttftValues.push((ev.event as Record<string, number>).TTFT)
            if (isLatency) ms.latencyValues.push((ev.event as Record<string, number>).TotalLatency)
            if (isOutTps) ms.outputTpsValues.push((ev.event as Record<string, number>).OutputTokensPerSecond)
          }
        }

        setProviders(map)

        // Seed recent events list (display newest-first)
        setLiveEvents(events.reverse().slice(0, MAX_LIVE_EVENTS))
      } catch {
        // Non-critical — WS will still provide live data
      }
    }
    preload()
  }, [])

  useEffect(() => {
    let cancelled = false

    function connect() {
      if (cancelled) return

      const token = localStorage.getItem('token')
      if (!token) {
        setWsStatus('disconnected')
        return
      }

      const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
      // API_BASE_URL may be relative or absolute; derive ws URL from it
      let wsBase: string
      try {
        const url = new URL(API_BASE_URL)
        url.protocol = proto
        wsBase = url.toString().replace(/\/$/, '')
      } catch {
        // relative path — build from window.location
        wsBase = `${proto}//${window.location.host}`
      }

      const wsUrl = `${wsBase}/api/metrics/ws?token=${encodeURIComponent(token)}`
      setWsStatus('connecting')

      const ws = new WebSocket(wsUrl)
      wsRef.current = ws

      ws.onopen = () => {
        if (!cancelled) setWsStatus('connected')
      }

      ws.onmessage = (ev) => {
        if (cancelled) return
        try {
          const data = JSON.parse(ev.data as string)
          // Lag notification
          if (data.type === 'lag' && typeof data.skipped === 'number') {
            setSkipped(s => s + data.skipped)
            return
          }
          // ProviderMetrics event
          processEvent(data as WsProviderMetrics)
        } catch {
          // ignore malformed
        }
      }

      ws.onclose = () => {
        if (!cancelled) {
          setWsStatus('disconnected')
          wsRef.current = null
          // Reconnect after 3s
          reconnectTimer.current = setTimeout(connect, 3000)
        }
      }

      ws.onerror = () => {
        // onclose will fire after this
      }
    }

    connect()

    return () => {
      cancelled = true
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current)
      wsRef.current?.close()
    }
  }, [processEvent])

  // Auto-scroll live events
  useEffect(() => {
    eventsEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [liveEvents.length])

  const providerList = Array.from(providers.values()).sort((a, b) => b.lastEvent - a.lastEvent)
  const selectedData = selectedProvider ? providers.get(selectedProvider) : null

  return (
    <div className="p-8">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold text-text-primary">Metrics</h1>
        <div className="flex items-center gap-3">
          {skipped > 0 && (
            <span className="text-xs text-yellow-600 dark:text-yellow-400">
              {skipped} events skipped (lag)
            </span>
          )}
          <span className={`inline-flex items-center gap-1.5 text-sm px-2.5 py-0.5 rounded-full ${
            wsStatus === 'connected'
              ? 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400'
              : wsStatus === 'connecting'
                ? 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900/30 dark:text-yellow-400'
                : 'bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400'
          }`}>
            <span className={`w-2 h-2 rounded-full ${
              wsStatus === 'connected'
                ? 'bg-green-500 animate-pulse'
                : wsStatus === 'connecting'
                  ? 'bg-yellow-500 animate-pulse'
                  : 'bg-red-500'
            }`} />
            {wsStatus === 'connected' ? 'Live' : wsStatus === 'connecting' ? 'Connecting' : 'Disconnected'}
          </span>
        </div>
      </div>

      {/* Provider summary cards */}
      {providerList.length === 0 ? (
        <div className="p-6 bg-layer-3 rounded-lg border border-border mb-6">
          <p className="text-text-secondary text-center">
            {wsStatus === 'connected'
              ? 'Waiting for metrics events…'
              : 'Connect to see real-time metrics'}
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4 mb-6">
          {providerList.map(p => {
            const successRate = p.totalRequests > 0 ? (p.successes / p.totalRequests * 100) : null
            const p90Ttft = percentile(p.ttftValues, 0.9)
            const p90OutTps = percentile(p.outputTpsValues, 0.9)
            const avgLatency = p.latencyValues.length > 0
              ? p.latencyValues.reduce((a, b) => a + b, 0) / p.latencyValues.length
              : null
            const isSelected = selectedProvider === p.name

            return (
              <button
                key={p.name}
                onClick={() => setSelectedProvider(isSelected ? null : p.name)}
                className={`text-left p-5 bg-layer-3 rounded-lg border transition-colors ${
                  isSelected ? 'border-accent ring-1 ring-accent/30' : 'border-border hover:border-accent/50'
                }`}
              >
                <div className="flex items-center justify-between mb-3">
                  <h3 className="text-base font-semibold text-text-primary">{p.name}</h3>
                  <span className="text-xs text-text-secondary">{p.models.size} model{p.models.size !== 1 ? 's' : ''}</span>
                </div>
                <div className="grid grid-cols-2 gap-3 text-sm">
                  <div>
                    <div className="text-text-secondary text-xs">Success Rate</div>
                    <div className={`font-mono font-medium ${successRate !== null ? (successRate >= 95 ? 'text-green-600 dark:text-green-400' : successRate >= 80 ? 'text-yellow-600 dark:text-yellow-400' : 'text-red-600 dark:text-red-400') : 'text-text-secondary'}`}>
                      {successRate !== null ? `${successRate.toFixed(1)}%` : '-'}
                    </div>
                  </div>
                  <div>
                    <div className="text-text-secondary text-xs">P90 TTFT</div>
                    <div className="font-mono font-medium text-text-primary">
                      {p90Ttft !== null ? `${p90Ttft}ms` : '-'}
                    </div>
                  </div>
                  <div>
                    <div className="text-text-secondary text-xs">P90 Out tok/s</div>
                    <div className="font-mono font-medium text-text-primary">
                      {p90OutTps !== null ? p90OutTps.toFixed(1) : '-'}
                    </div>
                  </div>
                  <div>
                    <div className="text-text-secondary text-xs">Avg Latency</div>
                    <div className="font-mono font-medium text-text-primary">
                      {avgLatency !== null ? `${avgLatency.toFixed(0)}ms` : '-'}
                    </div>
                  </div>
                </div>
              </button>
            )
          })}
        </div>
      )}

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Model breakdown for selected provider */}
        <div className="lg:col-span-1">
          <div className="p-6 bg-layer-3 rounded-lg border border-border h-full">
            <h2 className="text-lg font-semibold mb-4 text-text-primary">
              {selectedData ? `Models — ${selectedData.name}` : 'Model Breakdown'}
            </h2>
            {!selectedData ? (
              <p className="text-text-secondary text-sm">Select a provider above to see per-model stats</p>
            ) : selectedData.models.size === 0 ? (
              <p className="text-text-secondary text-sm">No model-level data yet</p>
            ) : (
              <div className="space-y-3">
                {Array.from(selectedData.models.values())
                  .sort((a, b) => b.lastEvent - a.lastEvent)
                  .map(m => {
                    const sr = m.requests > 0 ? (m.successes / m.requests * 100) : null
                    const p90ttft = percentile(m.ttftValues, 0.9)
                    const p90otps = percentile(m.outputTpsValues, 0.9)
                    const avgLat = m.latencyValues.length > 0
                      ? m.latencyValues.reduce((a, b) => a + b, 0) / m.latencyValues.length
                      : null

                    return (
                      <div key={m.name} className="p-3 bg-layer-2 rounded-lg">
                        <div className="font-mono text-sm font-medium text-text-primary mb-2">{m.name}</div>
                        <div className="grid grid-cols-2 gap-2 text-xs">
                          <div>
                            <span className="text-text-secondary">Success</span>{' '}
                            <span className={`font-mono ${sr !== null ? (sr >= 95 ? 'text-green-600 dark:text-green-400' : sr >= 80 ? 'text-yellow-600 dark:text-yellow-400' : 'text-red-600 dark:text-red-400') : 'text-text-secondary'}`}>
                              {sr !== null ? `${sr.toFixed(1)}%` : '-'}
                            </span>
                          </div>
                          <div>
                            <span className="text-text-secondary">P90 TTFT</span>{' '}
                            <span className="font-mono text-text-primary">{p90ttft !== null ? `${p90ttft}ms` : '-'}</span>
                          </div>
                          <div>
                            <span className="text-text-secondary">P90 tok/s</span>{' '}
                            <span className="font-mono text-text-primary">{p90otps !== null ? p90otps.toFixed(1) : '-'}</span>
                          </div>
                          <div>
                            <span className="text-text-secondary">Avg Lat</span>{' '}
                            <span className="font-mono text-text-primary">{avgLat !== null ? `${avgLat.toFixed(0)}ms` : '-'}</span>
                          </div>
                        </div>
                      </div>
                    )
                  })}
              </div>
            )}
          </div>
        </div>

        {/* Live event stream */}
        <div className="lg:col-span-2">
          <div className="p-6 bg-layer-3 rounded-lg border border-border">
            <h2 className="text-lg font-semibold mb-4 text-text-primary">
              Live Event Stream
              <span className="ml-2 text-sm font-normal text-text-secondary">
                ({liveEvents.length} event{liveEvents.length !== 1 ? 's' : ''})
              </span>
            </h2>
            <div className="space-y-1 max-h-[500px] overflow-y-auto font-mono text-xs">
              {liveEvents.length === 0 ? (
                <p className="text-text-secondary py-4 text-center">
                  {wsStatus === 'connected' ? 'Waiting for events…' : 'Not connected'}
                </p>
              ) : (
                liveEvents.map((ev, i) => {
                  const { label, value, kind } = formatEventType(ev.event)
                  const time = new Date(ev.timestamp_ms)
                  const timeStr = time.toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' })
                  const msStr = String(time.getMilliseconds()).padStart(3, '0')

                  return (
                    <div
                      key={`${ev.timestamp_ms}-${i}`}
                      className={`flex items-center gap-2 px-2 py-1 rounded ${
                        i === 0 ? 'bg-accent/10' : ''
                      }`}
                    >
                      <span className="text-text-secondary shrink-0">{timeStr}.{msStr}</span>
                      <span className="text-text-primary font-medium shrink-0 w-28 truncate" title={ev.provider}>{ev.provider}</span>
                      {ev.model && <span className="text-text-secondary shrink-0 w-32 truncate" title={ev.model}>{ev.model}</span>}
                      <span className={`shrink-0 px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase tracking-wide ${
                        kind === 'ok' ? 'bg-green-100 text-green-800 dark:bg-green-900/40 dark:text-green-300'
                          : kind === 'err' ? 'bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300'
                          : kind === 'warn' ? 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900/40 dark:text-yellow-300'
                          : 'bg-layer-2 text-text-secondary'
                      }`}>
                        {label}
                      </span>
                      <span className="text-text-primary truncate" title={value}>{value}</span>
                    </div>
                  )
                })
              )}
              <div ref={eventsEndRef} />
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
