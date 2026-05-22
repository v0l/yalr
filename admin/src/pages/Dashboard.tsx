import { useEffect, useState, useRef } from 'react'
import { api, API_BASE_URL } from '../api/client'
import type { MetricsResponse, Provider, ProviderHealthEntry, HealthOverviewResponse } from '../types'
import { Skeleton } from '@/components/ui/skeleton'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { TopupDialog } from '@/components/TopupDialog'
import { cn, formatBalance } from '@/lib/utils'

/* ── Sub-components ────────────────────────────────────────────── */

function HealthBadge({ state }: { state: string }) {
  switch (state) {
    case 'healthy': return <Badge className="bg-brand/15 text-brand border-brand/30 font-mono text-[10px] tracking-wider uppercase">HEALTHY</Badge>
    case 'degraded': return <Badge className="bg-warning/15 text-warning border-warning/30 font-mono text-[10px] tracking-wider uppercase">DEGRADED</Badge>
    case 'unhealthy': return <Badge className="bg-destructive/15 text-destructive border-destructive/30 font-mono text-[10px] tracking-wider uppercase">DOWN</Badge>
    default: return <Badge variant="secondary" className="font-mono text-[10px]">{state.toUpperCase()}</Badge>
  }
}

function BalanceDisplay({ health }: { health?: ProviderHealthEntry }) {
  if (!health?.balance) return <span className="text-muted-foreground font-mono">—</span>
  return <span className="font-mono text-[13px] tabular-nums">{formatBalance(health.balance.amount, health.balance.currency)}</span>
}

/* ── Stat Card ─────────────────────────────────────────────────── */

function StatCard({
  label, value, sub, subError, subGood, color = 'default'
}: {
  label: string
  value: string
  sub?: string
  subError?: boolean
  subGood?: boolean
  color?: 'green' | 'amber' | 'red' | 'default'
}) {
  const borderColor = color === 'green' ? 'border-brand/20' : color === 'amber' ? 'border-warning/20' : color === 'red' ? 'border-destructive/20' : 'border-border/50'
  const glowColor = color === 'green' ? 'var(--brand)' : color === 'amber' ? 'var(--warning)' : color === 'red' ? 'var(--destructive)' : 'transparent'
  return (
    <div className={cn('panel px-4 py-3.5', borderColor)} style={glowColor !== 'transparent' ? { boxShadow: `inset 0 0 20px ${glowColor}08` } : undefined}>
      <div className="text-[10px] uppercase tracking-[0.1em] text-muted-foreground font-mono mb-1">{label}</div>
      <div className="font-mono text-[28px] font-bold tabular-nums leading-none tracking-tight">{value}</div>
      {sub && (
        <div className={cn(
          'mt-1 font-mono text-[11px]',
          subError ? 'text-destructive' : subGood ? 'text-brand' : 'text-muted-foreground'
        )}>
          {sub}
        </div>
      )}
    </div>
  )
}

/* ═══════════════════════════════════════════════════════════════ */
/*  Dashboard Page                                                */
/* ═══════════════════════════════════════════════════════════════ */

export default function Dashboard() {
  const [metrics, setMetrics] = useState<MetricsResponse | null>(null)
  const [providers, setProviders] = useState<Provider[]>([])
  const [health, setHealth] = useState<HealthOverviewResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [topupDialogOpen, setTopupDialogOpen] = useState(false)
  const [selectedProvider, setSelectedProvider] = useState<Provider | null>(null)
  const [wsStatus, setWsStatus] = useState<'connecting' | 'connected' | 'disconnected'>('disconnected')
  const wsRef = useRef<WebSocket | null>(null)
  const reconnectT = useRef<ReturnType<typeof setTimeout> | null>(null)

  function handleTopup(provider: Provider) {
    setSelectedProvider(provider)
    setTopupDialogOpen(true)
  }

  useEffect(() => {
    async function fetchData() {
      try {
        const [metricsData, providersData, healthData] = await Promise.all([
          api.getMetrics(),
          api.getProviders(),
          api.getHealthOverview(),
        ])
        setMetrics(metricsData)
        setProviders(providersData.providers)
        setHealth(healthData)
      } catch (e) {
        setError(e instanceof Error ? e.message : 'Failed to fetch data')
      } finally {
        setLoading(false)
      }
    }
    fetchData()
  }, [])

  /* WebSocket */
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
          if (d.provider) {
            setProviders(prev => {
              const updated = [...prev]
              const idx = updated.findIndex(p => p.name === d.provider)
              if (idx !== -1 && d.event && typeof d.event === 'object' && 'ProviderLoad' in d.event) {
                const load = (d.event as Record<string, unknown>).ProviderLoad as { in_flight: number; max_concurrency: number | null }
                if (load && updated[idx].health) {
                  updated[idx] = {
                    ...updated[idx],
                    health: { ...updated[idx].health!, in_flight: load.in_flight, max_concurrency: load.max_concurrency }
                  }
                }
              }
              return updated
            })
          }
        } catch {}
      }
      ws.onclose = () => {
        if (!c) { setWsStatus('disconnected'); wsRef.current = null; reconnectT.current = setTimeout(connect, 3000) }
      }
    }
    connect()
    return () => { c = true; if (reconnectT.current) clearTimeout(reconnectT.current); wsRef.current?.close() }
  }, [])

  /* Periodic refresh */
  useEffect(() => {
    const iv = setInterval(async () => {
      try {
        const [healthData, providersData] = await Promise.all([
          api.getHealthOverview(),
          api.getProviders(),
        ])
        setHealth(healthData)
        setProviders(providersData.providers)
      } catch {}
    }, 10000)
    return () => clearInterval(iv)
  }, [])

  /* ── Loading ─────────────────────────────────────────────── */
  if (loading) {
    return (
      <div className="space-y-6 p-6">
        <div className="flex flex-col gap-1">
          <Skeleton className="h-8 w-44 bg-secondary" />
          <Skeleton className="h-4 w-64 bg-secondary" />
        </div>
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
          {[1,2,3,4].map(i => <Skeleton key={i} className="h-28 bg-secondary" />)}
        </div>
        <Skeleton className="h-80 bg-secondary" />
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-6">
        <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      </div>
    )
  }

  /* ── Derived ─────────────────────────────────────────────── */
  const totalRequests = metrics?.total_requests ?? 0
  const totalSuccesses = metrics?.total_successes ?? 0
  const totalFailures = metrics?.total_failures ?? 0
  const activeProviders = providers.filter(p => p.health?.available).length
  const avgLatency = metrics?.providers.length
    ? (metrics.providers.reduce((sum, p) => sum + (p.avg_latency_ms || 0), 0) || 0) / metrics.providers.length
    : 0
  const successRate = totalRequests > 0 ? ((totalSuccesses / totalRequests) * 100).toFixed(1) : null
  const srNum = successRate ? parseFloat(successRate) : 0

  /* ── Render ──────────────────────────────────────────────── */
  return (
    <div className="space-y-6 p-6">
      {/* Header */}
      <div className="flex items-start justify-between">
        <div>
          <div className="flex items-center gap-3 mb-1">
            <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground leading-none">
              DASHBOARD
            </h1>
            <div className={cn(
              'flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-wider px-2 py-0.5 border',
              wsStatus === 'connected' ? 'text-brand border-brand/30 bg-brand/5' :
              wsStatus === 'connecting' ? 'text-warning border-warning/30 bg-warning/5' :
              'text-destructive border-destructive/30 bg-destructive/5'
            )}>
              <span className={cn(
                'w-1.5 h-1.5 rounded-full',
                wsStatus === 'connected' && 'bg-brand animate-pulse-status',
                wsStatus === 'connecting' && 'bg-warning animate-pulse-status',
                wsStatus === 'disconnected' && 'bg-destructive'
              )} />
              {wsStatus === 'connected' ? 'LIVE' : wsStatus === 'connecting' ? 'CONN' : 'OFF'}
            </div>
          </div>
          <p className="font-mono text-[13px] text-muted-foreground">
            System overview &amp; provider health monitoring
          </p>
        </div>

        {/* Quick health summary */}
        <div className="hidden sm:flex items-center gap-4 font-mono text-[12px]">
          <div className="flex items-center gap-1.5">
            <span className="w-2 h-2 rounded-full bg-brand" />
            <span className="text-brand tabular-nums">{providers.filter(p => p.health?.health_state === 'healthy').length}</span>
            <span className="text-muted-foreground">HEALTHY</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2 h-2 rounded-full bg-warning" />
            <span className="text-warning tabular-nums">{providers.filter(p => p.health?.health_state === 'degraded').length}</span>
            <span className="text-muted-foreground">DEGRADED</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2 h-2 rounded-full bg-destructive" />
            <span className="text-destructive tabular-nums">{providers.filter(p => p.health?.health_state === 'unhealthy').length}</span>
            <span className="text-muted-foreground">DOWN</span>
          </div>
        </div>
      </div>

      {/* Stat Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
        <StatCard
          label="Total Requests"
          value={totalRequests.toLocaleString()}
          sub={totalRequests > 0 ? `${totalSuccesses.toLocaleString()} ok · ${totalFailures > 0 ? totalFailures.toLocaleString() + ' fail' : '0 fail'}` : undefined}
          subGood={totalFailures === 0 && totalRequests > 0}
          subError={totalFailures > 0}
        />
        <StatCard
          label="Providers"
          value={`${activeProviders}/${providers.length}`}
          sub={health?.unhealthy_count && health.unhealthy_count > 0
            ? `${health.unhealthy_count} DOWN`
            : health?.degraded_count && health.degraded_count > 0
              ? `${health.degraded_count} DEGRADED`
              : 'ALL HEALTHY'}
          subError={!!health?.unhealthy_count && health.unhealthy_count > 0}
          subGood={!health?.unhealthy_count || health.unhealthy_count === 0}
          color={health?.unhealthy_count ? 'red' : 'green'}
        />
        <StatCard
          label="Avg Latency"
          value={`${avgLatency.toFixed(0).replace(/\B(?=(\d{3})+(?!\d))/g, ',')}ms`}
          sub={avgLatency > 2000 ? 'ELEVATED' : avgLatency > 1000 ? 'MODERATE' : 'NOMINAL'}
          subError={avgLatency > 2000}
          subGood={avgLatency <= 1000}
          color={avgLatency > 2000 ? 'red' : avgLatency > 1000 ? 'amber' : 'green'}
        />
        <StatCard
          label="Success Rate"
          value={successRate ? `${successRate}%` : '—'}
          sub={srNum >= 99 ? 'EXCELLENT' : srNum >= 95 ? 'GOOD' : srNum > 0 ? 'DEGRADED' : undefined}
          subGood={srNum >= 99}
          subError={srNum > 0 && srNum < 95}
          color={srNum >= 99 ? 'green' : srNum >= 95 ? 'amber' : srNum > 0 ? 'red' : 'default'}
        />
      </div>

      {/* Provider Health Table */}
      <div>
        <h2 className="section-header">Provider Health</h2>
        <div className="panel">
          <div className="overflow-x-auto">
            <table className="w-full table-scan">
              <thead>
                <tr className="border-b border-border/50 text-left">
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Provider</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Status</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Balance</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">In-Flight</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Failures</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Backoff</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Latency</th>
                </tr>
              </thead>
              <tbody>
                {providers.length === 0 ? (
                  <tr>
                    <td colSpan={7} className="px-4 py-16 text-center font-mono text-[13px] text-muted-foreground">
                      {'>'} NO PROVIDERS CONFIGURED
                    </td>
                  </tr>
                ) : (
                  providers.map(provider => {
                    const h = provider.health
                    const m = metrics?.providers.find(p => p.provider === provider.name)
                    return (
                      <tr key={provider.slug} className="border-b border-border/50 hover:bg-surface transition-colors">
                        <td className="px-4 py-3">
                          <div className="font-mono text-[13px] font-medium">{provider.name}</div>
                          <div className="font-mono text-[11px] text-muted-foreground">{provider.slug}</div>
                        </td>
                        <td className="px-4 py-3"><HealthBadge state={h?.health_state ?? 'unknown'} /></td>
                        <td className="px-4 py-3">
                          <div className="flex items-center gap-2">
                            <BalanceDisplay health={h} />
                            {(provider.provider_type === 'routstr' || provider.provider_type === 'ppq') && (
                              <Button
                                variant="outline"
                                size="sm"
                                onClick={() => handleTopup(provider)}
                                className="h-7 font-mono text-[10px] uppercase tracking-wider border-border text-muted-foreground hover:text-brand hover:border-brand/30"
                              >
                                TOP-UP
                              </Button>
                            )}
                          </div>
                        </td>
                        <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">
                          {h?.in_flight ?? '—'}{h?.max_concurrency ? <span className="text-muted-foreground">/{h.max_concurrency}</span> : ''}
                        </td>
                        <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">
                          {h?.consecutive_failures ? (
                            <span className={h.consecutive_failures > 0 ? 'text-destructive' : 'text-brand'}>
                              {h.consecutive_failures}
                            </span>
                          ) : '—'}
                        </td>
                        <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right text-muted-foreground">
                          {h?.backoff_ms ? `${h.backoff_ms.toLocaleString('en-US')}ms` : '—'}
                        </td>
                        <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">
                          {m?.avg_latency_ms ? `${m.avg_latency_ms.toFixed(0).replace(/\B(?=(\d{3})+(?!\d))/g, ',')}ms` : '—'}
                        </td>
                      </tr>
                    )
                  })
                )}
              </tbody>
            </table>
          </div>
        </div>
      </div>

      {/* Health Grid — at a glance status */}
      {health && health.providers.length > 0 && (
        <div>
          <h2 className="section-header">Health Grid</h2>
          <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-6 gap-2">
            {health.providers.map(h => {
              const state = h.health_state
              const stateColor = state === 'healthy' ? 'var(--brand)' : state === 'degraded' ? 'var(--warning)' : 'var(--destructive)'
              return (
                <div
                  key={h.provider}
                  className="panel px-3 py-2.5"
                  style={{ borderLeft: `2px solid ${stateColor}` }}
                >
                  <div className="flex items-center gap-1.5 mb-1">
                    <span className={cn(
                      'w-1.5 h-1.5 rounded-full',
                      state === 'healthy' && 'bg-brand',
                      state === 'degraded' && 'bg-warning',
                      state === 'unhealthy' && 'bg-destructive'
                    )} />
                    <span className="font-mono text-[10px] text-muted-foreground uppercase tracking-wider">
                      {state}
                    </span>
                  </div>
                  <div className="font-mono text-[13px] font-medium truncate">{h.provider}</div>
                  <div className="flex items-center gap-3 mt-1.5 font-mono text-[11px] text-muted-foreground tabular-nums">
                    <span title="In-flight">IF:{h.in_flight}</span>
                    <span title="Consecutive failures" className={h.consecutive_failures > 0 ? 'text-destructive' : 'text-brand'}>
                      F:{h.consecutive_failures}
                    </span>
                    {h.backoff_ms > 0 && <span title="Backoff">BO:{h.backoff_ms}ms</span>}
                  </div>
                </div>
              )
            })}
          </div>
        </div>
      )}

      {/* Top-up Dialog */}
      {selectedProvider && (
        <TopupDialog
          open={topupDialogOpen}
          onOpenChange={setTopupDialogOpen}
          providerSlug={selectedProvider.slug}
          providerName={selectedProvider.name}
          supportedPaymentMethods={selectedProvider.payment_options}
          currentBalance={selectedProvider.health?.balance}
        />
      )}
    </div>
  )
}
