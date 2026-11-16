import { useEffect, useRef, useState } from 'react'
import RevolutCheckout, { type RevolutCheckoutCardField } from '@revolut/checkout'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { fiatApi, type CreatedFiatTopup, type FiatConfig } from '@/api/fiat'
import { formatMilli } from '@/lib/money'

const POLL_MS = 2500
const PRESETS = [10, 25, 50, 100]

interface Props {
  config: FiatConfig
  hasSavedCard: boolean
  onPaid: () => void
}

export default function CardTopup({ config, hasSavedCard, onPaid }: Props) {
  const [currency, setCurrency] = useState(config.currencies[0])
  const [amount, setAmount] = useState(PRESETS[1])
  const [saveCard, setSaveCard] = useState(false)
  const [email, setEmail] = useState('')
  const [order, setOrder] = useState<CreatedFiatTopup | null>(null)
  const [busy, setBusy] = useState(false)
  const [status, setStatus] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const target = useRef<HTMLDivElement | null>(null)
  const field = useRef<RevolutCheckoutCardField | null>(null)

  useEffect(() => {
    if (!order || !target.current) return
    let destroyed = false
    RevolutCheckout(order.token, order.mode).then(instance => {
      if (destroyed) return instance.destroy()
      field.current = instance.createCardField({
        target: target.current!,
        hidePostcodeField: false,
        onSuccess: () => { setBusy(false); setStatus('pending') },
        onError: err => { setBusy(false); setError(err.message || 'Payment failed') },
        onCancel: () => setBusy(false),
      })
    }).catch(e => setError(e instanceof Error ? e.message : 'Could not load the card form'))
    return () => { destroyed = true; field.current?.destroy(); field.current = null }
  }, [order])

  useEffect(() => {
    if (!order || status !== 'pending') return
    const timer = setInterval(async () => {
      try {
        const current = await fiatApi.status(order.id)
        if (current.status === 'pending') return
        clearInterval(timer)
        setStatus(current.status)
        if (current.status === 'completed') onPaid()
        else setError(current.failure_reason ?? `Payment ${current.status}`)
      } catch { /* keep polling */ }
    }, POLL_MS)
    return () => clearInterval(timer)
  }, [order, status, onPaid])

  async function start(e: React.FormEvent) {
    e.preventDefault()
    setBusy(true); setError(null); setStatus(null)
    try {
      setOrder(await fiatApi.create({ currency, amount, save_card: saveCard, email: email || undefined }))
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not start the payment')
    } finally {
      setBusy(false)
    }
  }

  function pay() {
    setBusy(true); setError(null)
    field.current?.submit({ email: email || undefined, ...(saveCard ? { savePaymentMethodFor: 'merchant' as const } : {}) })
  }

  function reset() { setOrder(null); setStatus(null); setError(null); setBusy(false) }

  const done = status === 'completed'
  return (
    <div className="panel space-y-4 p-4">
      <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Top up with card</div>
      {error && <Alert className="border-destructive/30 bg-destructive/5"><AlertDescription className="font-mono text-[13px] text-destructive">{error}</AlertDescription></Alert>}
      {done && <Alert className="border-brand/30 bg-brand/5"><AlertDescription className="font-mono text-[13px] text-brand">TOP UP RECEIVED</AlertDescription></Alert>}
      {!order && (
        <form onSubmit={start} className="space-y-3">
          <div className="flex flex-wrap items-end gap-3">
            <div className="space-y-1">
              <Label className="font-mono text-[10px] uppercase tracking-wider">Currency</Label>
              <div className="flex gap-1">
                {config.currencies.map(c => (
                  <Button key={c} type="button" size="sm" variant={c === currency ? 'default' : 'outline'} onClick={() => setCurrency(c)} className="h-8 font-mono text-[11px] uppercase">{c}</Button>
                ))}
              </div>
            </div>
            <div className="space-y-1">
              <Label className="font-mono text-[10px] uppercase tracking-wider">Amount ({config.min_amount} to {config.max_amount})</Label>
              <Input type="number" step="0.01" min={config.min_amount} max={config.max_amount} value={amount} onChange={e => setAmount(Number(e.target.value))} className="h-8 w-32 font-mono tabular-nums" />
            </div>
            <div className="flex gap-1">
              {PRESETS.map(p => <Button key={p} type="button" size="sm" variant="outline" onClick={() => setAmount(p)} className="h-8 font-mono text-[11px] tabular-nums">{p}</Button>)}
            </div>
          </div>
          <label className="flex items-center gap-2 font-mono text-[12px]">
            <input type="checkbox" checked={saveCard} onChange={e => setSaveCard(e.target.checked)} /> Save this card for auto-reload{hasSavedCard ? ' (adds another card)' : ''}
          </label>
          {saveCard && (
            <div className="space-y-1">
              <Label className="font-mono text-[10px] uppercase tracking-wider">Email for the card receipt</Label>
              <Input type="email" value={email} onChange={e => setEmail(e.target.value)} className="h-8 max-w-xs font-mono" />
            </div>
          )}
          <Button type="submit" disabled={busy} size="sm" className="h-8 border border-brand/40 bg-brand/10 font-mono text-[11px] uppercase tracking-wider text-brand hover:bg-brand/20">Continue</Button>
        </form>
      )}
      {order && !done && (
        <div className="space-y-3">
          <div className="font-mono text-[13px] tabular-nums">Pay {formatMilli(order.amount_milli, order.currency)}</div>
          {config.mode === 'sandbox' && <div className="font-mono text-[11px] text-warning">Sandbox mode: use a Revolut test card</div>}
          <div ref={target} className="min-h-24" />
          <div className="flex gap-2">
            <Button size="sm" disabled={busy || status === 'pending'} onClick={pay} className="h-8 border border-brand/40 bg-brand/10 font-mono text-[11px] uppercase tracking-wider text-brand hover:bg-brand/20">
              {status === 'pending' ? 'Confirming...' : 'Pay'}
            </Button>
            <Button size="sm" variant="outline" onClick={reset} className="h-8 font-mono text-[11px] uppercase">Cancel</Button>
          </div>
        </div>
      )}
      {done && <Button size="sm" variant="outline" onClick={reset} className="h-8 font-mono text-[11px] uppercase">Top up again</Button>}
    </div>
  )
}
