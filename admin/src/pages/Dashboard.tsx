import { useEffect, useState } from 'react'
import { api } from '../api/client'
import type { MetricsResponse, Provider } from '../types'

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
      <div className="p-8">
        <h1 className="text-2xl font-bold mb-6 text-text-primary">Dashboard</h1>
        <p className="text-text-secondary">Loading...</p>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-8">
        <h1 className="text-2xl font-bold mb-6 text-text-primary">Dashboard</h1>
        <div className="p-4 bg-red-100 border border-red-400 text-red-700 rounded">
          Error: {error}
        </div>
      </div>
    )
  }

  const totalRequests = metrics?.recent_events.length || 0
  const activeProviders = providers.length
  const avgLatency = metrics?.providers.reduce((sum, p) => sum + (p.avg_latency_ms || 0), 0)
    ? (metrics?.providers.reduce((sum, p) => sum + (p.avg_latency_ms || 0), 0) || 0) /
      (metrics?.providers.length || 1)
    : 0

  return (
    <div className="p-8">
      <h1 className="text-2xl font-bold mb-6 text-text-primary">Dashboard</h1>
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
        <div className="p-6 bg-layer-3 rounded-lg border border-border">
          <h2 className="text-lg font-semibold text-text-secondary">Total Requests</h2>
          <p className="text-4xl font-bold mt-2 text-accent">{totalRequests}</p>
        </div>
        <div className="p-6 bg-layer-3 rounded-lg border border-border">
          <h2 className="text-lg font-semibold text-text-secondary">Active Providers</h2>
          <p className="text-4xl font-bold mt-2 text-accent">{activeProviders}</p>
        </div>
        <div className="p-6 bg-layer-3 rounded-lg border border-border">
          <h2 className="text-lg font-semibold text-text-secondary">Avg Latency</h2>
          <p className="text-4xl font-bold mt-2 text-accent">
            {avgLatency.toFixed(0)}ms
          </p>
        </div>
      </div>
    </div>
  )
}
