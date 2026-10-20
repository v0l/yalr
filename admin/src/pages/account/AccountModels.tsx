import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Skeleton } from '@/components/ui/skeleton'
import { loadAccountModels } from '@/api/account-models'
import type { Model } from '@/types'
import AccountNav from './AccountNav'
import AccountShell from './AccountShell'

function tokens(n: number | undefined): string {
  if (!n) return '-'
  return n >= 1_000_000 ? `${(n / 1_000_000).toFixed(n % 1_000_000 ? 1 : 0)}M` : `${Math.round(n / 1000)}k`
}

function sats(n: number): string {
  return n.toLocaleString()
}

export default function AccountModels({ accountKey, onSignOut }: { accountKey: string; onSignOut: () => void }) {
  const [models, setModels] = useState<Model[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    loadAccountModels(accountKey).then(setModels).catch(e => setError(e instanceof Error ? e.message : 'Failed to load models'))
  }, [accountKey])

  return (
    <AccountShell onSignOut={onSignOut} wide>
      <AccountNav />
      {error && (
        <Alert className="border-destructive/30 bg-destructive/5 font-mono text-[13px] text-destructive">
          <AlertDescription><span className="text-destructive">!</span> {error}</AlertDescription>
        </Alert>
      )}
      <div className="panel overflow-x-auto">
        {!models ? <Skeleton className="m-4 h-24 bg-secondary" /> : models.length === 0 ? (
          <p className="p-6 text-center font-mono text-[12px] text-muted-foreground">No models are available to this key</p>
        ) : (
          <table className="table-scan w-full">
            <thead>
              <tr className="border-b border-border/50 text-left">
                <th className="px-4 py-2.5 font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Model</th>
                <th className="px-4 py-2.5 text-right font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Input / 1M</th>
                <th className="px-4 py-2.5 text-right font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Output / 1M</th>
                <th className="px-4 py-2.5 text-right font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Per request</th>
                <th className="px-4 py-2.5 text-right font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Context</th>
              </tr>
            </thead>
            <tbody>
              {models.map(m => (
                <tr key={m.id} className="border-b border-border/50">
                  <td className="px-4 py-2.5 font-mono text-[12px] text-foreground">
                    <div>{m.id}</div>
                    {m.architecture?.input_modalities && m.architecture.input_modalities.length > 0 && (
                      <div className="mt-1 flex flex-wrap gap-1">
                        {m.architecture.input_modalities.map(i => (
                          <Badge key={i} className="h-5 border-border bg-secondary px-1.5 py-0 font-mono text-[10px] text-muted-foreground">{i}</Badge>
                        ))}
                      </div>
                    )}
                  </td>
                  <td className="px-4 py-2.5 text-right font-mono text-[12px] tabular-nums text-foreground">{m.pricing ? `${sats(m.pricing.prompt)} sats` : '-'}</td>
                  <td className="px-4 py-2.5 text-right font-mono text-[12px] tabular-nums text-foreground">{m.pricing ? `${sats(m.pricing.completion)} sats` : '-'}</td>
                  <td className="px-4 py-2.5 text-right font-mono text-[12px] tabular-nums text-foreground">{m.pricing ? `${sats(m.pricing.request)} sats` : '-'}</td>
                  <td className="px-4 py-2.5 text-right font-mono text-[12px] tabular-nums text-muted-foreground">{tokens(m.context_length)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      <div className="panel space-y-2 p-4">
        <p className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Use the API</p>
        <p className="break-all font-mono text-[12px] text-foreground">Base URL: <span className="text-brand">{window.location.origin}/v1</span></p>
        <p className="font-mono text-[11px] text-muted-foreground">
          OpenAI compatible. Send your key as a bearer token. <Link to="/account/chat" className="text-brand hover:text-brand/80">Try it in chat</Link>.
        </p>
      </div>
    </AccountShell>
  )
}
