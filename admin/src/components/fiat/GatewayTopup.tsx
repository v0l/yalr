import { useEffect, useState } from 'react'
import { QRCodeSVG } from 'qrcode.react'
import { CheckIcon, CopyIcon } from 'lucide-react'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { fiatApi, type FiatConfig, type FiatOrder } from '@/api/fiat'
import { formatMilli } from '@/lib/money'

const POLL_MS = 2500
const FIAT_PRESETS = [10, 25, 50, 100]
const SAT_PRESETS = [1000, 5000, 10000, 25000]

interface Props {
  config: FiatConfig
  currency: string
  hasSavedCard: boolean
  onPaid: () => void
  onCancel?: () => void
}

export default function GatewayTopup({ config, currency, hasSavedCard, onPaid, onCancel }: Props) {
  const lightning = currency === 'sats'
  const presets = lightning ? SAT_PRESETS : FIAT_PRESETS
  const [amount, setAmount] = useState(presets[1])
  const [saveCard, setSaveCard] = useState(false)
  const [email, setEmail] = useState('')
  const [order, setOrder] = useState<FiatOrder | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)

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
      const created = await fiatApi.create({ currency, amount, save_card: saveCard, email: email || undefined })
      setOrder(created)
      if (!lightning && created.pay_url) window.open(created.pay_url, '_blank', 'noopener')
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not start the payment')
    } finally {
      setBusy(false)
    }
  }

  async function copy() {
    if (!order?.bolt11) return
    await navigator.clipboard.writeText(order.bolt11)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
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
              <label className="flex items-center gap-2 font-mono text-[12px]">
                <input type="checkbox" checked={saveCard} onChange={e => setSaveCard(e.target.checked)} /> Save this card for auto-reload{hasSavedCard ? ' (adds another card)' : ''}
              </label>
            </>
          )}
          <Button type="submit" disabled={busy} className={primary}>{busy ? '...' : lightning ? 'Create invoice' : 'Continue to payment'}</Button>
          {onCancel && <div className="text-center"><button type="button" onClick={reset} className={cancel}>Cancel</button></div>}
        </form>
      )}
      {order && !done && lightning && order.bolt11 && (
        <div className="space-y-3">
          <div className="flex items-center justify-between font-mono text-[12px]">
            <span className="text-[10px] uppercase tracking-wider text-muted-foreground">{order.status}</span>
            <span className="tabular-nums text-brand">{formatMilli(order.amount_milli, 'sats')}</span>
          </div>
          <a href={`lightning:${order.bolt11}`} className={`mx-auto block w-fit bg-white p-3 ${order.status !== 'pending' ? 'opacity-30' : ''}`}>
            <QRCodeSVG value={order.bolt11.toUpperCase()} size={208} />
          </a>
          <div className="flex gap-2">
            <Input readOnly value={order.bolt11} className="bg-surface font-mono text-[11px] text-muted-foreground" />
            <Button type="button" variant="outline" size="icon" onClick={copy} className="shrink-0 border-border">{copied ? <CheckIcon className="size-3.5" /> : <CopyIcon className="size-3.5" />}</Button>
          </div>
          <div className="text-center"><button type="button" onClick={reset} className={cancel}>Close</button></div>
        </div>
      )}
      {order && !done && !lightning && (
        <div className="space-y-3">
          {config.mode === 'sandbox' && <div className="font-mono text-[11px] text-warning">Sandbox mode: use a Revolut test card</div>}
          <div className="font-mono text-[13px]">Pay {formatMilli(order.amount_milli, order.currency)} on the secure payment page. This page updates when it is paid.</div>
          {order.pay_url && <a href={order.pay_url} target="_blank" rel="noopener noreferrer" className={`${primary} flex items-center justify-center`}>Open payment page</a>}
          <div className="text-center"><button type="button" onClick={reset} className={cancel}>Cancel</button></div>
        </div>
      )}
      {done && <Button size="sm" variant="outline" onClick={reset} className="h-8 font-mono text-[11px] uppercase">{onCancel ? 'Close' : 'Top up again'}</Button>}
    </div>
  )
}
