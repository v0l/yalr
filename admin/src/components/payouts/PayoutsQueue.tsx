import { useCallback, useEffect, useState } from 'react'
import { tenantApi } from '@/api/tenantApi'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import type { Payout, PayoutStatus } from '@/types/payouts'
import { formatPayoutAmount, statusClass } from './format'

const filters: (PayoutStatus | 'all')[] = ['requested', 'held', 'failed', 'sent', 'completed', 'all']

export default function PayoutsQueue() {
  const [rows, setRows] = useState<Payout[]>([])
  const [filter, setFilter] = useState<PayoutStatus | 'all'>('requested')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState<number | null>(null)

  const load = useCallback(async () => {
    try { setRows(await tenantApi.listPayouts(filter === 'all' ? undefined : filter)) } catch (e) { setError(e instanceof Error ? e.message : 'Failed to load') }
  }, [filter])
  useEffect(() => { load() }, [load])

  async function act(id: number, work: () => Promise<unknown>) {
    setBusy(id)
    setError(null)
    try { await work(); await load() } catch (e) { setError(e instanceof Error ? e.message : 'Request failed') } finally { setBusy(null) }
  }

  function reject(id: number) {
    const reason = prompt('Reason for rejecting this payout') ?? ''
    act(id, () => tenantApi.rejectPayout(id, reason))
  }

  function settle(id: number) {
    const reference = prompt('Gateway withdrawal id or payment hash') ?? ''
    if (reference.trim()) act(id, () => tenantApi.settlePayout(id, reference.trim()))
  }

  return (
    <div className="space-y-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Payouts</div>
      {error && <div className="panel p-3 font-mono text-[12px] text-destructive">{error}</div>}
      <div className="flex gap-2 font-mono text-[11px] uppercase tracking-wider">
        {filters.map(f => (
          <button key={f} onClick={() => setFilter(f)} className={filter === f ? 'text-brand' : 'text-muted-foreground hover:text-foreground'}>{f}</button>
        ))}
      </div>
      {rows.length === 0 && <div className="panel p-6 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO PAYOUTS</div>}
      {rows.map(p => (
        <div key={p.id} className="panel flex flex-wrap items-center justify-between gap-2 p-4">
          <div className="flex flex-wrap items-center gap-3 font-mono text-[13px]">
            <span className="text-muted-foreground">#{p.id}</span>
            <span>user {p.user_id}</span>
            <span className="tabular-nums">{formatPayoutAmount(p.amount, p.currency)}</span>
            <span className="text-[11px] text-muted-foreground">fee {formatPayoutAmount(p.fee, p.currency)}</span>
            <span className="text-[11px] uppercase">{p.method}</span>
            <Badge className={`h-5 px-1.5 py-0 font-mono text-[10px] uppercase ${statusClass[p.status]}`}>{p.status}</Badge>
            <span className="max-w-72 truncate text-[11px] text-muted-foreground" title={p.destination}>{p.destination}</span>
            {p.failure_reason && <span className="text-[11px] text-destructive">{p.failure_reason}</span>}
          </div>
          <div className="flex gap-1">
            {p.status === 'requested' && (
              <>
                <Button size="xs" variant="outline" disabled={busy === p.id} onClick={() => act(p.id, () => tenantApi.approvePayout(p.id))} className="border-border font-mono text-[10px] uppercase text-brand">Approve</Button>
                <Button size="xs" variant="outline" disabled={busy === p.id} onClick={() => reject(p.id)} className="border-border font-mono text-[10px] uppercase text-destructive">Reject</Button>
              </>
            )}
            {p.status === 'failed' && (
              <Button size="xs" variant="outline" disabled={busy === p.id} onClick={() => act(p.id, () => tenantApi.retryPayout(p.id))} className="border-border font-mono text-[10px] uppercase">Retry</Button>
            )}
            {p.status === 'held' && (
              <Button size="xs" variant="outline" disabled={busy === p.id} onClick={() => settle(p.id)} className="border-border font-mono text-[10px] uppercase text-brand">Mark sent</Button>
            )}
            {(p.status === 'held' || p.status === 'sent') && (
              <Button size="xs" variant="outline" disabled={busy === p.id} onClick={() => act(p.id, () => tenantApi.cancelPayout(p.id))} className="border-border font-mono text-[10px] uppercase text-destructive">Cancel</Button>
            )}
          </div>
        </div>
      ))}
    </div>
  )
}
