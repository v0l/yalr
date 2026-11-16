import { useCallback, useEffect, useState } from 'react'
import { tenantApi } from '@/api/tenantApi'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { WithdrawalOverview } from '@/types/payouts'
import { formatPayoutAmount, isLink, statusClass, toMilli } from './format'

const methodLabel: Record<string, string> = {
  lightning: 'Lightning invoice or address',
  revolut_link: 'Revolut payout link',
  revolut_bank: 'Bank transfer',
}

const destinationHint: Record<string, string> = {
  lightning: 'lnbc... invoice for the exact amount, or name@domain',
  revolut_link: 'Your full name, as on your bank or Revolut account',
}

export default function WithdrawPanel() {
  const [overview, setOverview] = useState<WithdrawalOverview | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [currency, setCurrency] = useState('')
  const [method, setMethod] = useState('')
  const [amount, setAmount] = useState('')
  const [destination, setDestination] = useState('')

  const load = useCallback(async () => {
    try {
      const next = await tenantApi.myPayouts()
      setOverview(next)
      setCurrency(c => c || next.currencies[0]?.currency || '')
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load')
    }
  }, [])
  useEffect(() => { load() }, [load])

  const current = overview?.currencies.find(c => c.currency === currency)
  const methods = current?.methods ?? []
  const activeMethod = methods.includes(method as never) ? method : methods[0] ?? ''

  async function submit() {
    if (!current) return
    setBusy(true)
    setError(null)
    try {
      await tenantApi.withdraw({
        currency,
        amount_milli: toMilli(amount, current.milli_per_unit),
        method: activeMethod,
        destination,
      })
      setAmount('')
      setDestination('')
      await load()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Request failed')
    } finally {
      setBusy(false)
    }
  }

  async function cancel(id: number) {
    setError(null)
    try { await tenantApi.cancelMyPayout(id); await load() } catch (e) { setError(e instanceof Error ? e.message : 'Request failed') }
  }

  if (!overview) return <div className="panel p-4 font-mono text-[13px] text-muted-foreground">{error ?? 'Loading...'}</div>
  if (!overview.enabled || overview.currencies.length === 0) {
    return <div className="panel p-8 text-center font-mono text-[13px] text-muted-foreground">{'>'} WITHDRAWALS ARE NOT ENABLED</div>
  }

  return (
    <div className="space-y-4">
      {error && <div className="panel p-3 font-mono text-[12px] text-destructive">{error}</div>}
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-4">
        {overview.currencies.map(c => (
          <button key={c.currency} onClick={() => setCurrency(c.currency)} className={`panel p-4 text-left ${c.currency === currency ? 'border-brand/50' : ''}`}>
            <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Withdrawable {c.currency}</div>
            <div className="font-mono text-[18px] tabular-nums">{formatPayoutAmount(c.withdrawable_milli, c.currency)}</div>
            <div className="font-mono text-[10px] text-muted-foreground">min {formatPayoutAmount(c.min_milli, c.currency)}</div>
          </button>
        ))}
      </div>
      <p className="font-mono text-[11px] text-muted-foreground">
        Only provider earnings can be withdrawn, and new earnings become available after {overview.hold_days} days. The first payout and large ones are reviewed by an admin.
      </p>

      {current && (
        <div className="panel space-y-3 p-4">
          <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Withdraw</div>
          <div className="flex flex-wrap gap-2">
            {methods.map(m => (
              <Button key={m} size="xs" variant="outline" onClick={() => setMethod(m)} className={`font-mono text-[11px] ${m === activeMethod ? 'border-brand/40 bg-brand/10 text-brand' : 'border-border'}`}>
                {methodLabel[m] ?? m}
              </Button>
            ))}
          </div>
          <div className="grid gap-2 sm:grid-cols-[160px_1fr_auto]">
            <Input value={amount} onChange={e => setAmount(e.target.value)} placeholder={current.currency === 'sats' ? 'Amount in sats' : `Amount in ${current.currency.toUpperCase()}`} className="font-mono text-[13px]" inputMode="decimal" />
            <Input value={destination} onChange={e => setDestination(e.target.value)} placeholder={destinationHint[activeMethod] ?? ''} className="font-mono text-[13px]" />
            <Button onClick={submit} disabled={busy || !amount || !destination} className="font-mono text-[11px] uppercase tracking-wider">Withdraw</Button>
          </div>
        </div>
      )}

      <div className="panel overflow-x-auto">
        <table className="w-full table-scan">
          <thead>
            <tr className="border-b border-border/50 text-left font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">
              <th className="px-4 py-2.5 font-medium">Time</th>
              <th className="px-4 py-2.5 font-medium">Method</th>
              <th className="px-4 py-2.5 text-right font-medium">Amount</th>
              <th className="px-4 py-2.5 font-medium">Status</th>
              <th className="px-4 py-2.5 font-medium">Details</th>
            </tr>
          </thead>
          <tbody>
            {overview.payouts.length === 0 && <tr><td colSpan={5} className="px-4 py-10 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO PAYOUTS YET</td></tr>}
            {overview.payouts.map(p => (
              <tr key={p.id} className="border-b border-border/50 hover:bg-surface">
                <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{new Date(p.created_at).toLocaleString()}</td>
                <td className="px-4 py-3 font-mono text-[12px]">{p.method}</td>
                <td className="px-4 py-3 text-right font-mono text-[13px] tabular-nums">{formatPayoutAmount(p.amount, p.currency)}</td>
                <td className="px-4 py-3"><Badge className={`h-5 px-1.5 py-0 font-mono text-[10px] uppercase ${statusClass[p.status]}`}>{p.status}</Badge></td>
                <td className="max-w-64 truncate px-4 py-3 font-mono text-[11px] text-muted-foreground">
                  {p.status === 'sent' && isLink(p.destination) ? <a href={p.destination} target="_blank" rel="noreferrer" className="text-brand">Claim link</a> : p.failure_reason ?? ''}
                  {(p.status === 'requested' || p.status === 'held' || (p.status === 'sent' && p.method === 'revolut_link')) && (
                    <button onClick={() => cancel(p.id)} className="ml-2 text-destructive">cancel</button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
