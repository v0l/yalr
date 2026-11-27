import { useCallback, useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { api } from '@/api/client'
import { tenantApi } from '@/api/tenantApi'
import { currentRole } from '@/lib/role'
import type { Provider } from '@/types'
import type { VerificationDetail } from '@/types/verification'
import { ProbeTable, StatusBadge, VerificationLine } from './VerificationView'

export default function ProviderVerificationPanel({ slug }: { slug: string }) {
  const { isAdmin } = currentRole()
  const [provider, setProvider] = useState<Provider | null>(null)
  const [runs, setRuns] = useState<VerificationDetail[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(async () => {
    try {
      const [list, history] = await Promise.all([api.getProviders(), tenantApi.verifications(slug)])
      setProvider(list.providers.find(p => p.slug === slug) ?? null)
      setRuns(history)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load verification')
    }
  }, [slug])

  useEffect(() => { load() }, [load])

  async function act(work: () => Promise<unknown>) {
    setBusy(true)
    setError(null)
    try { await work(); await load() } catch (e) { setError(e instanceof Error ? e.message : 'Request failed') } finally { setBusy(false) }
  }

  if (!provider) return null
  const status = provider.status ?? 'approved'
  const latestRun = runs[0]
  const latestByModel = provider.verifications ?? []
  const userOwned = provider.owner_user_id != null

  return (
    <div className="panel space-y-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Verification</span>
          <StatusBadge status={status} />
          {provider.is_reference && <span className="font-mono text-[10px] uppercase tracking-wider text-brand">reference</span>}
          {provider.status_locked && <span className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">decided by admin</span>}
        </div>
        <div className="flex gap-1">
          <Button size="xs" variant="outline" disabled={busy} onClick={() => act(() => tenantApi.verifyProvider(slug))} className="border-border font-mono text-[10px] uppercase">
            {busy ? 'Verifying' : 'Verify now'}
          </Button>
          {isAdmin && (
            <>
              <Button size="xs" variant="outline" disabled={busy} onClick={() => act(() => tenantApi.setProviderStatus(slug, 'approve'))} className="border-border font-mono text-[10px] uppercase text-brand">Approve</Button>
              <Button size="xs" variant="outline" disabled={busy} onClick={() => act(() => tenantApi.setProviderStatus(slug, 'suspend', prompt('Reason for suspend') ?? ''))} className="border-border font-mono text-[10px] uppercase">Suspend</Button>
              <Button size="xs" variant="outline" disabled={busy} onClick={() => act(() => tenantApi.setProviderStatus(slug, 'reject', prompt('Reason for reject') ?? ''))} className="border-border font-mono text-[10px] uppercase text-destructive">Reject</Button>
              {!userOwned && (
                <Button size="xs" variant="outline" disabled={busy} onClick={() => act(() => tenantApi.setProviderReference(slug, !provider.is_reference))} className="border-border font-mono text-[10px] uppercase">
                  {provider.is_reference ? 'Unmark reference' : 'Mark reference'}
                </Button>
              )}
            </>
          )}
        </div>
      </div>
      {error && <p className="font-mono text-[12px] text-destructive">{error}</p>}
      {userOwned && status === 'pending' && (
        <p className="font-mono text-[11px] text-muted-foreground">
          Pending: YALR does not call it and it serves no requests yet. Press Verify now (5 checks an hour); it goes live once every model it prices passes verification against a reference provider.
        </p>
      )}
      {status === 'suspended' && <p className="font-mono text-[11px] text-destructive">Suspended and out of the shared pool.</p>}
      {provider.status_reason && <p className="font-mono text-[11px] text-muted-foreground">{provider.status_reason}</p>}
      {latestByModel.length === 0 ? (
        <p className="font-mono text-[11px] text-muted-foreground">
          {userOwned ? 'Not verified yet. Price a model on Payments, then press Verify now.' : 'Platform providers are approved without verification.'}
        </p>
      ) : latestByModel.map(v => <VerificationLine key={v.model} v={v} />)}
      {latestRun && (
        <div className="space-y-1">
          <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Latest probes: {latestRun.model}</div>
          <ProbeTable run={latestRun} />
        </div>
      )}
    </div>
  )
}
