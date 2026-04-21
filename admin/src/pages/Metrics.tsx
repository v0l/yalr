import { useEffect, useState } from 'react'
import { api } from '../api/client'
import type { MetricsResponse } from '../types'

export default function Metrics() {
  const [metrics, setMetrics] = useState<MetricsResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    loadMetrics()
  }, [])

  async function loadMetrics() {
    try {
      const data = await api.getMetrics()
      setMetrics(data)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch metrics')
    } finally {
      setLoading(false)
    }
  }

  if (loading) {
    return (
      <div className="p-8">
        <h1 className="text-2xl font-bold mb-6 text-text-primary">Metrics</h1>
        <p className="text-text-secondary">Loading...</p>
      </div>
    )
  }

  return (
    <div className="p-8">
      <h1 className="text-2xl font-bold mb-6 text-text-primary">Metrics</h1>

      {error && (
        <div className="mb-6 p-4 bg-red-100 border border-red-400 text-red-700 rounded">
          Error: {error}
        </div>
      )}

      <div className="grid grid-cols-1 gap-6">
        <div className="p-6 bg-layer-3 rounded-lg border border-border">
          <h2 className="text-lg font-semibold mb-4 text-text-primary">Provider Performance</h2>
          {metrics?.providers.length === 0 ? (
            <p className="text-text-secondary">No provider data available</p>
          ) : (
            <div className="overflow-x-auto">
              <table className="min-w-full divide-y divide-border">
                <thead className="bg-layer-2">
                  <tr>
                    <th className="px-4 py-2 text-left text-xs font-medium text-text-secondary uppercase">
                      Provider
                    </th>
                    <th className="px-4 py-2 text-left text-xs font-medium text-text-secondary uppercase">
                      Avg Latency (ms)
                    </th>
                    <th className="px-4 py-2 text-left text-xs font-medium text-text-secondary uppercase">
                      P90 TTFT (ms)
                    </th>
                    <th className="px-4 py-2 text-left text-xs font-medium text-text-secondary uppercase">
                      P90 Tokens/s
                    </th>
                    <th className="px-4 py-2 text-left text-xs font-medium text-text-secondary uppercase">
                      Success Rate
                    </th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-border">
                  {metrics?.providers.map((p) => (
                    <tr key={p.provider}>
                      <td className="px-4 py-3 text-text-primary">{p.provider}</td>
                      <td className="px-4 py-3 text-text-secondary">
                        {p.avg_latency_ms?.toFixed(2) ?? '-'}
                      </td>
                      <td className="px-4 py-3 text-text-secondary">
                        {p.p90_ttft_ms ?? '-'}
                      </td>
                      <td className="px-4 py-3 text-text-secondary">
                        {p.p90_tokens_per_second?.toFixed(2) ?? '-'}
                      </td>
                      <td className="px-4 py-3 text-text-secondary">
                        {p.success_rate?.toFixed(1) ?? '-'}%
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>

        <div className="p-6 bg-layer-3 rounded-lg border border-border">
          <h2 className="text-lg font-semibold mb-4 text-text-primary">Recent Events</h2>
          {metrics?.recent_events.length === 0 ? (
            <p className="text-text-secondary">No recent events</p>
          ) : (
            <div className="space-y-2 max-h-96 overflow-y-auto">
              {metrics?.recent_events.slice(0, 20).map((event, i) => (
                <div key={i} className="p-3 bg-layer-2 rounded text-sm text-text-secondary font-mono">
                  {JSON.stringify(event)}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
