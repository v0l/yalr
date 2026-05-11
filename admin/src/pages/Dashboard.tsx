import { useEffect, useState } from 'react'
import { api } from '../api/client'
import type { MetricsResponse, Provider } from '../types'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { Alert, AlertDescription } from '@/components/ui/alert'

export default function Dashboard() {
  const [metrics, setMetrics] = useState<MetricsResponse | null>(null)
  const [providers, setProviders] = useState<Provider[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

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
  const activeProviders = providers.length
  const avgLatency = metrics?.providers.length
    ? (metrics.providers.reduce((sum, p) => sum + (p.avg_latency_ms || 0), 0) || 0) / metrics.providers.length
    : 0

  return (
    <div className="flex flex-col gap-6 p-6">
      <div>
        <h1 className="text-2xl font-bold text-foreground">Dashboard</h1>
        <p className="text-sm text-muted-foreground">Overview of your YALR instance</p>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
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
            <CardTitle className="text-sm text-muted-foreground">Active Providers</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-3xl font-bold">{activeProviders}</p>
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
      </div>
    </div>
  )
}