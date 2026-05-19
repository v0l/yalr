import { useEffect, useState, useRef } from 'react'
import { api, API_BASE_URL } from '../api/client'
import type { MetricsResponse, Provider, ProviderHealthEntry, HealthOverviewResponse } from '../types'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Button } from '@/components/ui/button'
import { TopupDialog } from '@/components/TopupDialog'
import { cn, formatBalance } from '@/lib/utils'

function HealthBadge({ state }: { state: string }) {
  switch (state) {
    case 'healthy':
      return <Badge className="bg-emerald-100 text-emerald-800 dark:bg-emerald-900/30 dark:text-emerald-400">Healthy</Badge>
    case 'degraded':
      return <Badge className="bg-amber-100 text-amber-800 dark:bg-amber-900/30 dark:text-amber-400">Degraded</Badge>
    case 'unhealthy':
      return <Badge className="bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400">Down</Badge>
    default:
      return <Badge variant="secondary">{state}</Badge>
  }
}

function BalanceDisplay({ health }: { health?: ProviderHealthEntry }) {
  if (!health?.balance) return <span className="text-muted-foreground">—</span>
  const { currency, amount } = health.balance
  return (
    <span className="font-mono text-sm">
      {formatBalance(amount, currency)}
    </span>
  )
}

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

  // WebSocket connection for real-time metrics
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
          // Update providers list when we get ProviderLoad events
          if (d.provider) {
            setProviders(prev => {
              const updated = [...prev]
              const idx = updated.findIndex(p => p.name === d.provider)
              if (idx !== -1 && d.event && typeof d.event === 'object' && 'ProviderLoad' in d.event) {
                const load = (d.event as Record<string, unknown>).ProviderLoad as { in_flight: number; max_concurrency: number | null }
                if (load && updated[idx].health) {
                  updated[idx] = {
                    ...updated[idx],
                    health: {
                      ...updated[idx].health!,
                      in_flight: load.in_flight,
                      max_concurrency: load.max_concurrency,
                    }
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

  // Periodic refresh of health data
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

  if (loading) {
    return (
      <div className="flex flex-col gap-6 p-6">
        <div className="flex flex-col gap-1">
          <Skeleton className="h-7 w-32" />
          <Skeleton className="h-4 w-48" />
        </div>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          <Skeleton className="h-32" />
          <Skeleton className="h-32" />
          <Skeleton className="h-32" />
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-6">
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      </div>
    )
  }

  const totalRequests = metrics?.total_requests ?? 0
  const totalSuccesses = metrics?.total_successes ?? 0
  const totalFailures = metrics?.total_failures ?? 0
  const activeProviders = providers.filter(p => p.health?.available).length
  const avgLatency = metrics?.providers.length
    ? (metrics.providers.reduce((sum, p) => sum + (p.avg_latency_ms || 0), 0) || 0) / metrics.providers.length
    : 0

  return (
    <div className="flex flex-col gap-6 p-6">
      <div className="flex items-center gap-2 mb-2">
        <h1 className="text-2xl font-bold text-foreground">Dashboard</h1>
        <span className={cn('size-2 rounded-full',
          wsStatus==='connected'?'bg-emerald-500':wsStatus==='connecting'?'bg-amber-500':'bg-destructive')}/>
      </div>
      <p className="text-sm text-muted-foreground">Overview of your YALR instance</p>

      <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm text-muted-foreground">Total Requests</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-3xl font-bold">{totalRequests.toLocaleString()}</p>
            <div className="flex gap-3 mt-1 text-sm">
              <span className="text-emerald-600 dark:text-emerald-400">{totalSuccesses.toLocaleString()} ok</span>
              {totalFailures > 0 && <span className="text-destructive">{totalFailures.toLocaleString()} fail</span>}
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm text-muted-foreground">Providers</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-3xl font-bold">{activeProviders}<span className="text-lg text-muted-foreground">/{providers.length}</span></p>
            {health?.unhealthy_count && health.unhealthy_count > 0 && (
              <p className="text-sm text-destructive mt-1">{health.unhealthy_count} down</p>
            )}
            {health?.degraded_count && health.degraded_count > 0 && (
              <p className="text-sm text-amber-500 mt-1">{health.degraded_count} degraded</p>
            )}
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm text-muted-foreground">Avg Latency</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-3xl font-bold">{avgLatency.toFixed(0)}ms</p>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm text-muted-foreground">Success Rate</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-3xl font-bold">
              {totalRequests > 0 ? ((totalSuccesses / totalRequests) * 100).toFixed(1) : '—'}%
            </p>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm text-muted-foreground">Quick Actions</CardTitle>
          </CardHeader>
          <CardContent>
            <Button onClick={() => handleTopup(providers[0])} disabled={providers.length === 0}>
              Top-up Balance
            </Button>
          </CardContent>
        </Card>
      </div>

      {/* Provider Health Table */}
      <div>
        <h2 className="text-lg font-semibold text-foreground mb-3">Provider Health</h2>
        <Card>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Provider</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Balance</TableHead>
                  <TableHead>In-flight</TableHead>
                  <TableHead>Failures</TableHead>
                  <TableHead>Backoff</TableHead>
                  <TableHead>Latency</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {providers.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-center text-muted-foreground py-12">
                      No providers configured.
                    </TableCell>
                  </TableRow>
                ) : (
                  providers.map((provider) => {
                    const h = provider.health
                    const m = metrics?.providers.find(p => p.provider === provider.name)
                    return (
                      <TableRow key={provider.slug}>
                        <TableCell>
                          <div className="font-medium">{provider.name}</div>
                          <div className="font-mono text-xs text-muted-foreground">{provider.slug}</div>
                        </TableCell>
                        <TableCell><HealthBadge state={h?.health_state ?? 'unknown'} /></TableCell>
                        <TableCell>
                          <div className="flex items-center gap-2">
                            <BalanceDisplay health={h} />
                          </div>
                        </TableCell>
                        <TableCell className="font-mono">
                          {h?.in_flight ?? '—'}{h?.max_concurrency ? ` / ${h.max_concurrency}` : ''}
                        </TableCell>
                        <TableCell className="font-mono">{h?.consecutive_failures ?? '—'}</TableCell>
                        <TableCell className="font-mono text-sm">{h?.backoff_ms ? `${h.backoff_ms}ms` : '—'}</TableCell>
                        <TableCell className="font-mono text-sm">{m?.avg_latency_ms ? `${m.avg_latency_ms.toFixed(0)}ms` : '—'}</TableCell>
                      </TableRow>
                    )
                  })
                )}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      </div>

      {selectedProvider && (
        <TopupDialog
          open={topupDialogOpen}
          onOpenChange={setTopupDialogOpen}
          providerSlug={selectedProvider.slug}
          providerName={selectedProvider.name}
          providerType={selectedProvider.provider_type}
        />
      )}
    </div>
  )
}
