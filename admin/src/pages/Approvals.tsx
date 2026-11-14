import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { userApi } from '@/api/userApi'
import type { AdminProvider } from '@/types/userProviders'
import { StatusBadge, VerificationHistory, VerificationLine } from '@/components/mine/VerificationView'

export default function Approvals() {
  const [rows, setRows] = useState<AdminProvider[]>([])
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [filter, setFilter] = useState<'all' | 'pending' | 'suspended' | 'approved'>('all')

  async function load() {
    try { setRows((await userApi.listAdminProviders()).providers) } catch (e) { setError(e instanceof Error ? e.message : 'Failed to load') }
  }
  useEffect(() => { load() }, [])

  async function act(slug: string, work: () => Promise<unknown>) {
    setBusy(slug)
    setError(null)
    try { await work(); await load() } catch (e) { setError(e instanceof Error ? e.message : 'Request failed') } finally { setBusy(null) }
  }

  function ask(slug: string, action: 'suspend' | 'reject') {
    const reason = prompt(`Reason for ${action}`) ?? ''
    act(slug, () => userApi.setProviderStatus(slug, action, reason))
  }

  const shown = rows.filter(r => (r.owner_user_id !== null || r.is_reference) && (filter === 'all' || r.status === filter))
  const platform = rows.filter(r => r.owner_user_id === null)

  return (
    <div className="space-y-4 p-4 sm:p-5">
      <div>
        <h1 className="mb-1 font-display text-[28px] leading-none tracking-[0.04em] text-foreground">PROVIDER APPROVALS</h1>
        <p className="font-mono text-[13px] text-muted-foreground">User providers join the shared pool once verified against a reference. Mark platform providers as references first.</p>
      </div>
      {error && <div className="panel p-3 font-mono text-[12px] text-destructive">{error}</div>}

      <div className="panel space-y-2 p-4">
        <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Reference providers</div>
        <div className="flex flex-wrap gap-2">
          {platform.length === 0 && <span className="font-mono text-[12px] text-muted-foreground">No platform providers</span>}
          {platform.map(p => (
            <Button
              key={p.slug}
              size="xs"
              variant="outline"
              disabled={busy === p.slug}
              onClick={() => act(p.slug, () => userApi.setProviderReference(p.slug, !p.is_reference))}
              className={`font-mono text-[11px] ${p.is_reference ? 'border-brand/40 bg-brand/10 text-brand' : 'border-border'}`}
            >
              {p.is_reference ? '[x] ' : '[ ] '}{p.name}
            </Button>
          ))}
        </div>
      </div>

      <div className="flex gap-2 font-mono text-[11px] uppercase tracking-wider">
        {(['all', 'pending', 'approved', 'suspended'] as const).map(f => (
          <button key={f} onClick={() => setFilter(f)} className={filter === f ? 'text-brand' : 'text-muted-foreground hover:text-foreground'}>{f}</button>
        ))}
      </div>

      <div className="space-y-3">
        {shown.length === 0 && <div className="panel p-8 text-center font-mono text-[13px] text-muted-foreground">{'>'} NOTHING TO REVIEW</div>}
        {shown.map(p => (
          <div key={p.slug} className="panel space-y-2 p-4">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex flex-wrap items-center gap-2 font-mono text-[13px]">
                <span className="text-foreground">{p.name}</span>
                <StatusBadge status={p.status} />
                {p.is_reference && <span className="text-[10px] uppercase tracking-wider text-brand">reference</span>}
                {p.status_locked && <span className="text-[10px] uppercase tracking-wider text-muted-foreground">decided by admin</span>}
                <span className="text-[11px] text-muted-foreground">{p.base_url}</span>
              </div>
              <div className="flex gap-1">
                <Button size="xs" variant="outline" disabled={busy === p.slug} onClick={() => act(p.slug, () => userApi.adminVerify(p.slug))} className="border-border font-mono text-[10px] uppercase">Verify</Button>
                <Button size="xs" variant="outline" disabled={busy === p.slug} onClick={() => act(p.slug, () => userApi.setProviderStatus(p.slug, 'approve'))} className="border-border font-mono text-[10px] uppercase text-brand">Approve</Button>
                <Button size="xs" variant="outline" disabled={busy === p.slug} onClick={() => ask(p.slug, 'suspend')} className="border-border font-mono text-[10px] uppercase">Suspend</Button>
                <Button size="xs" variant="outline" disabled={busy === p.slug} onClick={() => ask(p.slug, 'reject')} className="border-border font-mono text-[10px] uppercase text-destructive">Reject</Button>
              </div>
            </div>
            {p.status_reason && <p className="font-mono text-[11px] text-muted-foreground">{p.status_reason}</p>}
            {p.verifications.map(v => <VerificationLine key={v.model} v={v} />)}
            <VerificationHistory load={() => userApi.adminVerifications(p.slug)} />
          </div>
        ))}
      </div>
    </div>
  )
}
