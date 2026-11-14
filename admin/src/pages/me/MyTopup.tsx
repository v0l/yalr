import { useEffect, useRef, useState } from 'react'
import { QRCodeSVG } from 'qrcode.react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { userApi } from '@/api/userApi'

const PRESETS = [1000, 5000, 10000, 50000]

export default function MyTopup({ onError, onPaid }: { onError: (message: string | null) => void; onPaid: () => void }) {
  const [amount, setAmount] = useState('5000')
  const [invoice, setInvoice] = useState<{ bolt11: string; payment_hash: string; amount_sats: number } | null>(null)
  const [state, setState] = useState('pending')
  const paid = useRef(onPaid)
  paid.current = onPaid

  useEffect(() => {
    if (!invoice) return
    let stopped = false
    async function poll() {
      try {
        const status = await userApi.invoiceStatus(invoice!.payment_hash)
        if (stopped) return
        setState(status.status)
        if (status.status === 'paid') { paid.current(); return }
        if (status.status === 'expired' || status.status === 'cancelled') return
      } catch { /* retry */ }
      setTimeout(poll, 3000)
    }
    poll()
    return () => { stopped = true }
  }, [invoice])

  async function create(e: React.FormEvent) {
    e.preventDefault()
    onError(null)
    try {
      const made = await userApi.createInvoice(Number.parseInt(amount, 10))
      setState('pending')
      setInvoice(made.instruction)
      navigator.clipboard?.writeText(made.instruction.bolt11).catch(() => undefined)
    } catch (err) { onError(err instanceof Error ? err.message : 'Could not create invoice') }
  }

  return (
    <div className="panel space-y-3 p-4">
      <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Top up with Lightning</div>
      {invoice && state !== 'paid' ? (
        <div className="space-y-3">
          <div className="inline-block bg-white p-2"><QRCodeSVG value={invoice.bolt11.toUpperCase()} size={176} /></div>
          <div className="break-all border border-border bg-surface p-2 font-mono text-[11px] text-muted-foreground">{invoice.bolt11}</div>
          <div className="flex items-center justify-between font-mono text-[12px]">
            <span className="text-muted-foreground">{invoice.amount_sats.toLocaleString()} sats, {state}</span>
            <Button size="xs" variant="outline" onClick={() => setInvoice(null)} className="border-border font-mono text-[10px] uppercase">Cancel</Button>
          </div>
        </div>
      ) : (
        <form onSubmit={create} className="space-y-2">
          <div className="flex flex-wrap gap-2">
            {PRESETS.map(p => (
              <Button key={p} type="button" size="xs" variant="outline" onClick={() => setAmount(String(p))} className="border-border font-mono text-[11px]">
                {p.toLocaleString()}
              </Button>
            ))}
          </div>
          <div className="flex gap-2">
            <Input type="number" min={100} value={amount} onChange={e => setAmount(e.target.value)} className="border-border bg-surface font-mono" />
            <Button type="submit" className="border border-brand/40 bg-brand/10 font-mono text-[11px] uppercase tracking-wider text-brand hover:bg-brand/20">Create invoice</Button>
          </div>
          {state === 'paid' && <p className="font-mono text-[12px] text-brand">Payment received.</p>}
        </form>
      )}
    </div>
  )
}
