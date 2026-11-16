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
  currency: string
  hasSavedCard: boolean
  onPaid: () => void
  onCancel?: () => void
}

function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim()
}

function cardFieldStyles() {
  const font = token('--font-mono') || 'monospace'
  const style = (s: Record<string, unknown>) => s as unknown as Partial<CSSStyleDeclaration>
  return {
    default: style({ color: token('--foreground'), fontSize: '15px', fontFamily: font, '::placeholder': { color: token('--muted-foreground') } }),
    focused: style({ color: token('--foreground') }),
    invalid: style({ color: token('--destructive') }),
  }
}

export default function CardTopup({ config, currency, hasSavedCard, onPaid, onCancel }: Props) {
  const [amount, setAmount] = useState(PRESETS[1])
  const [saveCard, setSaveCard] = useState(false)
  const [email, setEmail] = useState('')
  const [holder, setHolder] = useState('')
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
        theme: document.documentElement.classList.contains('dark') ? 'dark' : 'light',
        hidePostcodeField: true,
        styles: cardFieldStyles(),
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

  function pay(e: React.FormEvent) {
    e.preventDefault()
    setBusy(true); setError(null)
    field.current?.submit({ name: holder.trim(), email: email || undefined, ...(saveCard ? { savePaymentMethodFor: 'merchant' as const } : {}) })
  }

  function reset() { setOrder(null); setStatus(null); setError(null); setBusy(false); onCancel?.() }

  const done = status === 'completed'
  const processing = busy || status === 'pending'
  const label = 'font-mono text-[10px] uppercase tracking-wider'
  const primary = 'h-10 w-full border border-brand/40 bg-brand/10 font-mono text-[12px] uppercase tracking-wider text-brand hover:bg-brand/20'
  const cancel = 'font-mono text-[11px] uppercase tracking-wider text-muted-foreground hover:text-foreground'

  return (
    <div className="w-full max-w-md space-y-4">
      {error && <Alert className="border-destructive/30 bg-destructive/5"><AlertDescription className="font-mono text-[13px] text-destructive">{error}</AlertDescription></Alert>}
      {done && <Alert className="border-brand/30 bg-brand/5"><AlertDescription className="font-mono text-[13px] text-brand">TOP UP RECEIVED</AlertDescription></Alert>}
      {!order && (
        <form onSubmit={start} className="space-y-3">
          <div className="space-y-1">
            <Label className={label}>Amount in {currency.toUpperCase()} ({config.min_amount} to {config.max_amount})</Label>
            <Input type="number" step="0.01" min={config.min_amount} max={config.max_amount} value={amount} onChange={e => setAmount(Number(e.target.value))} className="h-9 font-mono tabular-nums" />
          </div>
          <div className="grid grid-cols-4 gap-1">
            {PRESETS.map(p => <Button key={p} type="button" size="sm" variant={p === amount ? 'default' : 'outline'} onClick={() => setAmount(p)} className="h-8 font-mono text-[11px] tabular-nums">{p}</Button>)}
          </div>
          <label className="flex items-center gap-2 font-mono text-[12px]">
            <input type="checkbox" checked={saveCard} onChange={e => setSaveCard(e.target.checked)} /> Save this card for auto-reload{hasSavedCard ? ' (adds another card)' : ''}
          </label>
          <Button type="submit" disabled={busy} className={primary}>Continue</Button>
          {onCancel && <div className="text-center"><button type="button" onClick={reset} className={cancel}>Cancel</button></div>}
        </form>
      )}
      {order && !done && (
        <form onSubmit={pay} className="space-y-3">
          {config.mode === 'sandbox' && <div className="font-mono text-[11px] text-warning">Sandbox mode: use a Revolut test card</div>}
          <div className="space-y-1">
            <Label className={label}>Cardholder name</Label>
            <Input value={holder} onChange={e => setHolder(e.target.value)} placeholder="First Last" className="h-9 font-mono" />
          </div>
          <div className="space-y-1">
            <Label className={label}>Email for the receipt</Label>
            <Input type="email" required value={email} onChange={e => setEmail(e.target.value)} className="h-9 font-mono" />
          </div>
          <div className="space-y-1">
            <Label className={label}>Card details</Label>
            <div ref={target} className="rounded border border-border bg-surface p-3" />
          </div>
          <Button type="submit" disabled={processing} className={primary}>
            {processing ? 'Processing…' : `Pay ${formatMilli(order.amount_milli, order.currency)}`}
          </Button>
          <div className="text-center"><button type="button" onClick={reset} className={cancel}>Cancel</button></div>
        </form>
      )}
      {done && <Button size="sm" variant="outline" onClick={reset} className="h-8 font-mono text-[11px] uppercase">{onCancel ? 'Close' : 'Top up again'}</Button>}
    </div>
  )
}
