import { useCallback, useEffect, useState } from 'react'
import { ArrowDownLeftIcon, ArrowUpRightIcon, CheckIcon, CopyIcon, ReceiptIcon, SlidersHorizontalIcon } from 'lucide-react'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { accountApi, AccountAuthError, type AccountData, type CreatedInvoice } from '@/api/account'
import AccountNav from './AccountNav'
import AccountShell from './AccountShell'
import AmountPicker from './AmountPicker'
import PayInvoice from './PayInvoice'
import { sats, when } from './format'

const REFRESH_MS = 30_000

function invoiceBadge(status: string) {
  const tone = status === 'paid' ? 'bg-brand/15 text-brand border-brand/30'
    : status === 'pending' ? 'bg-warning/15 text-warning border-warning/30'
    : 'bg-secondary text-muted-foreground border-border'
  return <Badge className={`h-5 px-1.5 py-0 font-mono text-[10px] uppercase tracking-wider ${tone}`}>{status}</Badge>
}

export default function AccountHome({ accountKey: key, onSignOut }: { accountKey: string; onSignOut: () => void }) {
  const [data, setData] = useState<AccountData | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [invoice, setInvoice] = useState<CreatedInvoice | null>(null)
  const [busy, setBusy] = useState(false)
  const [tab, setTab] = useState<'history' | 'invoices'>('history')
  const [copied, setCopied] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)

  const load = useCallback(async () => {
    try {
      setData(await accountApi.load(key))
      setError(null)
    } catch (e) {
      if (e instanceof AccountAuthError) return onSignOut()
      setError(e instanceof Error ? e.message : 'Failed to load account')
    }
  }, [key, onSignOut])

  useEffect(() => {
    load()
    const timer = setInterval(load, REFRESH_MS)
    return () => clearInterval(timer)
  }, [load])

  async function topup(amount: number) {
    setBusy(true)
    setError(null)
    try {
      setInvoice(await accountApi.createInvoice(amount, 'topup', key))
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not create invoice')
    } finally {
      setBusy(false)
    }
  }

  async function copyKey() {
    await navigator.clipboard.writeText(key)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }

  const keyHint = `${key.slice(0, 7)}…${key.slice(-4)}`

  return (
    <AccountShell onSignOut={onSignOut}>
      <AccountNav />
      {error && (
        <Alert className="border-destructive/30 bg-destructive/5 font-mono text-[13px] text-destructive">
          <AlertDescription><span className="text-destructive">!</span> {error}</AlertDescription>
        </Alert>
      )}
      {notice && (
        <Alert className="border-brand/30 bg-brand/5 font-mono text-[13px] text-brand">
          <AlertDescription className="flex items-center justify-between">
            {notice}
            <Button variant="ghost" size="icon-xs" onClick={() => setNotice(null)} className="text-brand">×</Button>
          </AlertDescription>
        </Alert>
      )}

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <div className="panel p-4 sm:col-span-2">
          <p className="mb-1 font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Balance</p>
          {data ? (
            <p className="font-mono text-[32px] font-bold leading-none tabular-nums text-brand">{sats(data.balance_msat)}</p>
          ) : <Skeleton className="h-8 w-48 bg-secondary" />}
        </div>
        <div className="panel p-4">
          <p className="mb-1 font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Total deposited</p>
          {data ? (
            <p className="font-mono text-[20px] tabular-nums text-muted-foreground">{sats(data.lifetime_deposited_msat)}</p>
          ) : <Skeleton className="h-6 w-24 bg-secondary" />}
        </div>
      </div>

      <div className="panel flex items-center justify-between gap-3 p-4">
        <div className="min-w-0">
          <p className="mb-1 font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">API key</p>
          <p className="truncate font-mono text-[13px] text-foreground">{keyHint}</p>
        </div>
        <Button variant="outline" size="sm" onClick={copyKey} className="h-8 border-border font-mono text-[11px] uppercase tracking-wider text-muted-foreground">
          {copied ? <CheckIcon className="size-3 text-brand" /> : <CopyIcon className="size-3" />} Copy key
        </Button>
      </div>

      {invoice ? (
        <PayInvoice
          invoice={invoice}
          onCancel={() => setInvoice(null)}
          onPaid={status => { setInvoice(null); setNotice(`Received ${status.amount_sats.toLocaleString()} sats`); setData(null); load() }}
        />
      ) : (
        <div className="panel space-y-3 p-4">
          <p className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Top up</p>
          <AmountPicker label="Create invoice" busy={busy} onSubmit={topup} />
        </div>
      )}

      <div>
        <div className="flex gap-0 border-b border-border/50">
          {([['history', 'Balance history', <ReceiptIcon className="size-3.5" key="h" />], ['invoices', 'Invoices', <SlidersHorizontalIcon className="size-3.5" key="i" />]] as const).map(([id, label, icon]) => (
            <button
              key={id}
              onClick={() => setTab(id)}
              className={`-mb-px inline-flex items-center gap-1.5 border-b-[1.5px] px-4 py-2.5 font-mono text-[12px] uppercase tracking-wider transition-colors ${
                tab === id ? 'border-brand text-brand' : 'border-transparent text-muted-foreground hover:text-foreground/80'
              }`}
            >
              {icon}{label}
            </button>
          ))}
        </div>

        <div className="panel mt-4 overflow-x-auto">
          {!data ? <Skeleton className="m-4 h-24 bg-secondary" /> : tab === 'history' ? (
            data.history.length === 0 ? (
              <p className="p-6 text-center font-mono text-[12px] text-muted-foreground">No activity yet</p>
            ) : (
              <table className="table-scan w-full">
                <thead>
                  <tr className="border-b border-border/50 text-left">
                    <th className="px-4 py-2.5 font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Time</th>
                    <th className="px-4 py-2.5 font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Activity</th>
                    <th className="px-4 py-2.5 text-right font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Amount</th>
                  </tr>
                </thead>
                <tbody>
                  {data.history.map(h => (
                    <tr key={h.id} className="border-b border-border/50">
                      <td className="whitespace-nowrap px-4 py-2.5 font-mono text-[11px] tabular-nums text-muted-foreground">{when(h.created_at)}</td>
                      <td className="px-4 py-2.5 font-mono text-[12px] text-foreground">
                        <span className="inline-flex items-center gap-1.5">
                          {h.amount_msat >= 0 ? <ArrowDownLeftIcon className="size-3 text-brand" /> : <ArrowUpRightIcon className="size-3 text-muted-foreground" />}
                          <span className="capitalize">{h.kind}</span>
                          {h.detail && <span className="text-muted-foreground">{h.detail}</span>}
                        </span>
                      </td>
                      <td className={`whitespace-nowrap px-4 py-2.5 text-right font-mono text-[12px] tabular-nums ${h.amount_msat >= 0 ? 'text-brand' : 'text-foreground'}`}>
                        {h.amount_msat >= 0 ? '+' : ''}{sats(h.amount_msat)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )
          ) : data.invoices.length === 0 ? (
            <p className="p-6 text-center font-mono text-[12px] text-muted-foreground">No invoices yet</p>
          ) : (
            <table className="table-scan w-full">
              <thead>
                <tr className="border-b border-border/50 text-left">
                  <th className="px-4 py-2.5 font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Created</th>
                  <th className="px-4 py-2.5 font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Type</th>
                  <th className="px-4 py-2.5 text-right font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Amount</th>
                  <th className="px-4 py-2.5 font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Status</th>
                </tr>
              </thead>
              <tbody>
                {data.invoices.map(i => (
                  <tr key={i.invoice_id} className="border-b border-border/50">
                    <td className="whitespace-nowrap px-4 py-2.5 font-mono text-[11px] tabular-nums text-muted-foreground">{when(i.created_at)}</td>
                    <td className="px-4 py-2.5 font-mono text-[12px] capitalize text-foreground">{i.purpose === 'create' ? 'Purchase' : 'Top up'}</td>
                    <td className="px-4 py-2.5 text-right font-mono text-[12px] tabular-nums text-foreground">{i.amount_sats.toLocaleString()} sats</td>
                    <td className="px-4 py-2.5">{invoiceBadge(i.status)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
    </AccountShell>
  )
}
