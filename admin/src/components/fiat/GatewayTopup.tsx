import { useEffect, useState } from 'react'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { fiatApi, type FiatConfig, type FiatOrder } from '@/api/fiat'

const POLL_MS = 2500
const FIAT_PRESETS = [10, 25, 50, 100]
const SAT_PRESETS = [1000, 5000, 10000, 25000]

function embedUrl(url: string) {
  const u = new URL(url)
  u.searchParams.set('embed', '1')
  return u.toString()
}

interface Props {
  config: FiatConfig
  currency: string
  onPaid: () => void
  onCancel?: () => void
}

export default function GatewayTopup({ config, currency, onPaid, onCancel }: Props) {
  const lightning = currency === 'sats'
  const presets = lightning ? SAT_PRESETS : FIAT_PRESETS
  const [amount, setAmount] = useState(presets[1])
  const [email, setEmail] = useState('')
  const [order, setOrder] = useState<FiatOrder | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!order || order.status !== 'pending') return
    const timer = setInterval(async () => {
      try {
        const current = await fiatApi.status(order.public_id)
        if (current.status === 'pending') return
        setOrder(current)
        if (current.status === 'completed') onPaid()
        else setError(current.failure_reason ?? `Payment ${current.status}`)
      } catch { /* keep polling */ }
    }, POLL_MS)
    return () => clearInterval(timer)
  }, [order, onPaid])

  async function start(e: React.FormEvent) {
    e.preventDefault()
    setBusy(true); setError(null)
    try {
      const created = await fiatApi.create({ currency, amount, email: email || undefined })
      setOrder(created)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not start the payment')
    } finally {
      setBusy(false)
    }
  }

  function reset() { setOrder(null); setError(null); setBusy(false); onCancel?.() }

  const done = order?.status === 'completed'
  const label = 'font-mono text-[10px] uppercase tracking-wider'
  const primary = 'h-10 w-full border border-brand/40 bg-brand/10 font-mono text-[12px] uppercase tracking-wider text-brand hover:bg-brand/20'
  const cancel = 'font-mono text-[11px] uppercase tracking-wider text-muted-foreground hover:text-foreground'
  const min = lightning ? 1 : config.min_amount
  const max = lightning ? 10_000_000 : config.max_amount

  return (
    <div className="w-full max-w-md space-y-4">
      {error && <Alert className="border-destructive/30 bg-destructive/5"><AlertDescription className="font-mono text-[13px] text-destructive">{error}</AlertDescription></Alert>}
      {done && <Alert className="border-brand/30 bg-brand/5"><AlertDescription className="font-mono text-[13px] text-brand">TOP UP RECEIVED</AlertDescription></Alert>}
      {!order && (
        <form onSubmit={start} className="space-y-3">
          <div className="space-y-1">
            <Label className={label}>Amount in {currency.toUpperCase()} ({min} to {max})</Label>
            <Input type="number" step={lightning ? 1 : 0.01} min={min} max={max} value={amount} onChange={e => setAmount(Number(e.target.value))} className="h-9 font-mono tabular-nums" />
          </div>
          <div className="grid grid-cols-4 gap-1">
            {presets.map(p => <Button key={p} type="button" size="sm" variant={p === amount ? 'default' : 'outline'} onClick={() => setAmount(p)} className="h-8 font-mono text-[11px] tabular-nums">{p.toLocaleString()}</Button>)}
          </div>
          {!lightning && (
            <>
              <div className="space-y-1">
                <Label className={label}>Email for the receipt</Label>
                <Input type="email" value={email} onChange={e => setEmail(e.target.value)} className="h-9 font-mono" />
              </div>
            </>
          )}
          <Button type="submit" disabled={busy} className={primary}>{busy ? '...' : 'Continue to payment'}</Button>
          {onCancel && <div className="text-center"><button type="button" onClick={reset} className={cancel}>Cancel</button></div>}
        </form>
      )}
      {order && !done && (
        <div className="space-y-3">
          {config.mode === 'sandbox' && <div className="font-mono text-[11px] text-warning">Sandbox mode: payments are test payments</div>}
          {order.pay_url && <iframe title="Payment" src={embedUrl(order.pay_url)} className="h-[560px] w-full border border-border bg-card" allow="payment" />}
          <div className="font-mono text-[11px] text-muted-foreground">This page updates when it is paid.</div>
          {order.pay_url && <a href={order.pay_url} target="_blank" rel="noopener noreferrer" className={`${cancel} block text-center`}>Open in a new tab</a>}
          <div className="text-center"><button type="button" onClick={reset} className={cancel}>Cancel</button></div>
        </div>
      )}
      {done && <Button size="sm" variant="outline" onClick={reset} className="h-8 font-mono text-[11px] uppercase">{onCancel ? 'Close' : 'Top up again'}</Button>}
    </div>
  )
}
