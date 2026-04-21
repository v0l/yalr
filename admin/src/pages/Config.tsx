import { useState, useEffect } from 'react'
import { api } from '../api/client'
import type { RouterConfig } from '../types'

export default function Config() {
  const [config, setConfig] = useState<RouterConfig | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    async function loadConfig() {
      try {
        const data = await api.getRouterConfig()
        setConfig(data)
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to load config')
      } finally {
        setLoading(false)
      }
    }
    loadConfig()
  }, [])

  if (loading) {
    return (
      <div className="p-6">
        <div className="animate-pulse space-y-4">
          <div className="h-8 bg-layer-2 rounded w-1/4"></div>
          <div className="h-4 bg-layer-2 rounded w-1/2"></div>
          <div className="h-64 bg-layer-2 rounded"></div>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-6">
        <div className="bg-red-500/10 border border-red-500/50 rounded-lg p-4">
          <p className="text-red-500">Error: {error}</p>
        </div>
      </div>
    )
  }

  if (!config || config.routing_configs.length === 0) {
    return (
      <div className="p-6">
        <p className="text-text-secondary">No configuration found</p>
      </div>
    )
  }

  return (
    <div className="p-6 space-y-6">
      <div>
        <h1 className="text-2xl font-bold text-text-primary mb-2">Router Configuration</h1>
        <p className="text-text-secondary">Model aliases and their routing strategies</p>
      </div>

      {config.routing_configs.map((routingConfig, configIndex) => (
        <div key={configIndex} className="bg-layer-1 border border-border rounded-lg p-6">
          <div className="flex items-center justify-between mb-6">
            <div>
              <h2 className="text-xl font-semibold text-text-primary">{routingConfig.name}</h2>
              <p className="text-sm text-text-secondary">Routing Strategy: {routingConfig.strategy}</p>
            </div>
            <span className="px-3 py-1 bg-accent/20 text-accent rounded-full text-sm font-medium">
              {routingConfig.provider_count} Providers
            </span>
          </div>

          {routingConfig.providers.length === 0 ? (
            <div className="text-text-secondary py-8 text-center border border-dashed border-border rounded-lg">
              No providers configured for this routing alias
            </div>
          ) : (
            <div className="space-y-4">
              {routingConfig.providers.map((provider) => (
                <div
                  key={provider.slug || provider.name}
                  className="border border-border rounded-lg p-4 hover:border-accent/50 transition-colors"
                >
                  <div className="flex items-start justify-between mb-3">
                    <div>
                      <h3 className="text-lg font-semibold text-text-primary">{provider.name}</h3>
                      {provider.slug && (
                        <p className="text-sm text-text-secondary font-mono">{provider.slug}</p>
                      )}
                    </div>
                    <span className="px-2 py-1 bg-green-500/20 text-green-500 rounded text-xs font-medium">
                      Active
                    </span>
                  </div>
                  
                  <div className="space-y-2 text-sm">
                    <div className="flex items-center gap-2">
                      <span className="text-text-secondary w-20">Base URL:</span>
                      <code className="text-text-primary bg-layer-2 px-2 py-1 rounded font-mono text-xs break-all">
                        {provider.base_url}
                      </code>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-text-secondary w-20">Models:</span>
                      <code className="text-text-primary bg-layer-2 px-2 py-1 rounded font-mono text-xs">
                        {provider.list_url}
                      </code>
                    </div>
                  </div>

                  {provider.metrics && (
                    <div className="mt-4 pt-4 border-t border-border">
                      <h4 className="text-xs font-semibold text-text-secondary uppercase tracking-wider mb-3">
                        Performance Metrics
                      </h4>
                      <div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
                        <div className="bg-layer-2 rounded p-2">
                          <div className="text-xs text-text-secondary">P90 TTFT</div>
                          <div className="text-sm font-medium text-text-primary">
                            {provider.metrics.p90_ttft_ms !== null 
                              ? `${provider.metrics.p90_ttft_ms}ms` 
                              : '—'}
                          </div>
                        </div>
                        <div className="bg-layer-2 rounded p-2">
                          <div className="text-xs text-text-secondary">P90 Output TPS</div>
                          <div className="text-sm font-medium text-text-primary">
                            {provider.metrics.p90_output_tokens_per_second !== null 
                              ? provider.metrics.p90_output_tokens_per_second.toFixed(1) 
                              : '—'}
                          </div>
                        </div>
                        <div className="bg-layer-2 rounded p-2">
                          <div className="text-xs text-text-secondary">P90 Input TPS</div>
                          <div className="text-sm font-medium text-text-primary">
                            {provider.metrics.p90_input_tokens_per_second !== null 
                              ? provider.metrics.p90_input_tokens_per_second.toFixed(1) 
                              : '—'}
                          </div>
                        </div>
                        <div className="bg-layer-2 rounded p-2">
                          <div className="text-xs text-text-secondary">Avg Latency</div>
                          <div className="text-sm font-medium text-text-primary">
                            {provider.metrics.avg_latency_ms !== null 
                              ? `${provider.metrics.avg_latency_ms.toFixed(1)}ms` 
                              : '—'}
                          </div>
                        </div>
                        <div className="bg-layer-2 rounded p-2 col-span-2 sm:col-span-1">
                          <div className="text-xs text-text-secondary">Success Rate</div>
                          <div className="text-sm font-medium text-text-primary">
                            {provider.metrics.success_rate !== null 
                              ? `${(provider.metrics.success_rate * 100).toFixed(1)}%` 
                              : '—'}
                          </div>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      ))}
    </div>
  )
}
