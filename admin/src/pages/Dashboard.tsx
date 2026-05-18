import { useEffect, useState } from 'react'
import { api } from '../api/client'
import type { MetricsResponse, Provider, ProviderHealthEntry } from '../types'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Button } from '@/components/ui/button'
import { TopupDialog } from '@/components/TopupDialog'

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
  // Convert msats to sats for display
  const amountInSats = currency === 'msats' ? Math.floor(amount / 1000) : amount
  const label = currency === 'msats' || currency === 'sats' ? 'sats' : 'µ$'
  const display = currency === 'usd_micro' ? `$${(amount / 1_000_000).toFixed(4)}` : amountInSats.toLocaleString()
  return (
    <span className="font-mono text-sm">
      {display}<span className="text-muted-foreground ml-0.5">{label}</span>
    </span>
  )
}

export default function Dashboard() {
  const [metrics, setMetrics] = useState<MetricsResponse | null>(null)
  const [providers, setProviders] = useState<Provider[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [topupDialogOpen, setTopupDialogOpen] = useState(false)
  const [selectedProvider, setSelectedProvider] = useState<Provider | null>(null)

  useEffect(() => {
    async function fetchData() {
      try {
        const [metricsData, providersData] = await Promise.all([
          api.getMetrics(),
          api.getProviders(),
        ])
        setMetrics(metricsData)
        setProviders(providersData.providers)
      } catch (e) {
        setError(e instanceof Error ? e.message : 'Failed to fetch data')
      } finally {
        setLoading(false)
      }
    }
    fetchData()
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
  const downCount = providers.filter(p => p.health?.health_state === 'unhealthy').length
  const avgLatency = metrics?.providers.length
    ? (metrics.providers.reduce((sum, p) => sum + (p.avg_latency_ms || 0), 0) || 0) / metrics.providers.length
    : 0

  function handleTopup(provider: Provider) {
    setSelectedProvider(provider)
    setTopupDialogOpen(true)
  }

  return (
    <div className="flex flex-col gap-6 p-6">
      <div>
        <h1 className="text-2xl font-bold text-foreground">Dashboard</h1>
        <p className="text-sm text-muted-foreground">Overview of your YALR instance</p>
      </div>

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
            {downCount > 0 && (
              <p className="text-sm text-destructive mt-1">{downCount} down</p>
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
                            <Button variant="outline" size="sm" onClick={() => handleTopup(provider)}>
                              Top-up
                            </Button>
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
        />
      )}
    </div>
  )
}
