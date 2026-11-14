import { useEffect, useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { ArrowLeftIcon } from 'lucide-react'
import { userApi } from '@/api/userApi'
import type { MyProviderStats } from '@/types/userProviders'
import { formatMilli } from '@/lib/money'

const RANGES = [1, 7, 30, 90]

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="panel p-4">
      <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground mb-1">{label}</div>
      <div className="font-mono text-[18px] tabular-nums text-foreground">{value}</div>
    </div>
  )
}

function rate(value: number | null): string {
  return value === null ? '—' : `${value.toFixed(1)} tok/s`
}

export default function MyProviderDetail() {
  const { slug = '' } = useParams()
  const [days, setDays] = useState(30)
  const [stats, setStats] = useState<MyProviderStats | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    setError(null)
    userApi.providerStats(slug, days).then(setStats).catch(e => setError(e instanceof Error ? e.message : 'Failed to load'))
  }, [slug, days])

  const traffic = stats?.traffic
  const successRate = traffic && traffic.requests > 0 ? `${(((traffic.requests - traffic.failures) / traffic.requests) * 100).toFixed(1)}%` : '—'

  return (
    <div className="space-y-4 p-4 sm:p-5">
      <div>
        <Link to="/my-providers" className="inline-flex items-center gap-1 font-mono text-[11px] uppercase tracking-wider text-muted-foreground hover:text-foreground mb-2">
          <ArrowLeftIcon className="size-3" /> My providers
        </Link>
        <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground leading-none mb-1">{stats?.provider.name ?? slug}</h1>
        {stats && <p className="font-mono text-[13px] text-muted-foreground">{stats.provider.base_url}</p>}
      </div>
      {error && <div className="panel p-3 font-mono text-[12px] text-destructive">{error}</div>}
      <div className="flex gap-1">
        {RANGES.map(r => (
          <button
            key={r}
            onClick={() => setDays(r)}
            className={`font-mono text-[11px] uppercase tracking-wider px-2 py-1 border ${days === r ? 'border-brand/40 bg-brand/10 text-brand' : 'border-border text-muted-foreground hover:text-foreground'}`}
          >
            {r === 1 ? '24h' : `${r}d`}
          </button>
        ))}
      </div>
      {stats && traffic && (
        <>
          <div className="grid grid-cols-2 lg:grid-cols-4 gap-2">
            <Stat label="Requests" value={traffic.requests.toLocaleString()} />
            <Stat label="Success rate" value={successRate} />
            <Stat label="Output throughput" value={rate(traffic.output_tokens_per_second)} />
            <Stat label="Avg time to first token" value={traffic.avg_ttft_ms === null ? '—' : `${Math.round(traffic.avg_ttft_ms)} ms`} />
            <Stat label="Input tokens" value={traffic.prompt_tokens.toLocaleString()} />
            <Stat label="Cached tokens" value={traffic.cached_tokens.toLocaleString()} />
            <Stat label="Output tokens" value={traffic.completion_tokens.toLocaleString()} />
          </div>
          <div className="panel p-4">
            <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground mb-2">Earnings</div>
            {stats.earnings.length === 0 ? (
              <div className="font-mono text-[13px] text-muted-foreground">{'>'} NO EARNINGS IN THIS PERIOD</div>
            ) : (
              <div className="flex flex-wrap gap-x-8 gap-y-2 font-mono text-[13px] tabular-nums">
                {stats.earnings.map(e => (
                  <span key={e.currency}>{formatMilli(e.earned_milli, e.currency)} <span className="text-muted-foreground">({e.requests.toLocaleString()} requests)</span></span>
                ))}
              </div>
            )}
          </div>
          <div className="panel p-4">
            <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground mb-2">Models</div>
            <table className="w-full table-scan">
              <thead>
                <tr className="border-b border-border/50 text-left font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">
                  <th className="px-2 py-2 font-medium">Model</th>
                  <th className="px-2 py-2 font-medium text-right">Requests</th>
                  <th className="px-2 py-2 font-medium text-right">Output tokens</th>
                  <th className="px-2 py-2 font-medium text-right">Throughput</th>
                </tr>
              </thead>
              <tbody>
                {stats.models.length === 0 ? (
                  <tr><td colSpan={4} className="px-2 py-10 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO TRAFFIC IN THIS PERIOD</td></tr>
                ) : stats.models.map(m => (
                  <tr key={m.model} className="border-b border-border/50 hover:bg-surface font-mono text-[12px] tabular-nums">
                    <td className="px-2 py-2.5">{m.model}</td>
                    <td className="px-2 py-2.5 text-right">{m.requests.toLocaleString()}</td>
                    <td className="px-2 py-2.5 text-right">{m.completion_tokens.toLocaleString()}</td>
                    <td className="px-2 py-2.5 text-right">{rate(m.output_tokens_per_second)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </div>
  )
}
