import { useEffect, useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { publicApi, type PublicModelDetail, type PublicProvider } from '@/api/publicApi'
import PublicLayout from '@/layout/PublicLayout'
import { badgesFor, formatMs, formatPercent, formatTokens, priceLine } from '@/lib/publicFormat'

function VerifiedBadge({ status }: { status: PublicProvider['verification'] }) {
  const style = status === 'unverified'
    ? 'border-border text-muted-foreground'
    : 'border-brand/30 bg-brand/15 text-brand'
  return <span className={`inline-flex h-5 items-center border px-1.5 text-[10px] uppercase ${style}`}>{status}</span>
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="panel p-3">
      <div className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">{label}</div>
      <div className="font-mono text-[15px] tabular-nums text-foreground">{value}</div>
    </div>
  )
}

export default function ModelDetail() {
  const { '*': name = '' } = useParams()
  const [model, setModel] = useState<PublicModelDetail | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    setModel(null)
    setError(null)
    publicApi.model(name).then(setModel).catch(e => setError(e instanceof Error ? e.message : 'Failed to load'))
  }, [name])

  const signedIn = !!localStorage.getItem('token')

  return (
    <PublicLayout>
      <div className="space-y-4 p-4 sm:p-5">
        <Link to="/models" className="font-mono text-[11px] uppercase tracking-wider text-muted-foreground hover:text-foreground">{'<'} All models</Link>
        {error && <div className="panel p-3 font-mono text-[12px] text-destructive">{error}</div>}
        {!model && !error && <div className="font-mono text-[13px] text-muted-foreground">LOADING...</div>}
        {model && (
          <>
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div>
                <h1 className="mb-1 font-display text-[28px] leading-none tracking-[0.04em] text-foreground">{model.id}</h1>
                <div className="flex flex-wrap gap-1">
                  {[...model.input_modalities.map(m => `in ${m}`), ...model.output_modalities.map(m => `out ${m}`)].map(x => (
                    <span key={x} className="inline-flex h-5 items-center border border-border px-1.5 font-mono text-[10px] uppercase text-muted-foreground">{x}</span>
                  ))}
                </div>
              </div>
              <Button asChild className="border border-brand/40 bg-brand/10 font-mono text-[12px] uppercase tracking-wider text-brand hover:bg-brand/20">
                <Link to={signedIn ? '/' : '/register'}>Become a provider for this model</Link>
              </Button>
            </div>

            <div className="grid grid-cols-2 gap-2 md:grid-cols-4">
              <Stat label="Context window" value={formatTokens(model.context_length)} />
              <Stat label="Max output" value={formatTokens(model.max_output_tokens)} />
              <Stat label="Providers" value={String(model.provider_count)} />
              <Stat label="Requests 24h" value={model.requests_24h.toLocaleString()} />
            </div>

            <div className="panel overflow-x-auto">
              <table className="w-full font-mono text-[12px] tabular-nums">
                <thead>
                  <tr className="border-b border-border text-left text-[10px] uppercase tracking-wider text-muted-foreground">
                    {['Provider', 'Price per 1M tokens', 'Verified', 'Uptime 24h', 'Uptime 7d', 'TTFT p50', 'TTFT p95', 'Tok/s p50', 'Errors', 'Requests 24h'].map(h => (
                      <th key={h} className="px-3 py-2 font-normal">{h}</th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {model.providers.map(p => (
                    <tr key={p.key} className="border-b border-border/50 hover:bg-surface">
                      <td className="px-3 py-2 text-foreground">
                        {p.provider}
                        {badgesFor(p, model).cheapest && <span className="ml-2 text-[10px] uppercase text-brand">cheapest</span>}
                        {badgesFor(p, model).fastest && <span className="ml-2 text-[10px] uppercase text-brand">fastest</span>}
                        {p.owner && <div className="text-[10px] text-muted-foreground">by {p.owner}</div>}
                        <div className="text-[10px] text-muted-foreground">listed {new Date(p.listed_since).toLocaleDateString()}</div>
                      </td>
                      <td className="whitespace-nowrap px-3 py-2">{priceLine(p.price)}</td>
                      <td className="px-3 py-2"><VerifiedBadge status={p.verification} /></td>
                      <td className="px-3 py-2">{formatPercent(p.uptime_24h)}</td>
                      <td className="px-3 py-2">{formatPercent(p.uptime_7d)}</td>
                      <td className="px-3 py-2">{formatMs(p.ttft_p50_ms)}</td>
                      <td className="px-3 py-2">{formatMs(p.ttft_p95_ms)}</td>
                      <td className="px-3 py-2">{p.tokens_per_second_p50 == null ? '-' : p.tokens_per_second_p50.toFixed(1)}</td>
                      <td className="px-3 py-2">{formatPercent(p.error_rate)}</td>
                      <td className="px-3 py-2">{p.requests_24h.toLocaleString()}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </>
        )}
      </div>
    </PublicLayout>
  )
}
