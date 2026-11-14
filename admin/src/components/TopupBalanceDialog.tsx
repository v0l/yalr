import { useEffect, useState } from 'react'
import { QRCodeSVG } from 'qrcode.react'
import { CheckIcon, CopyIcon } from 'lucide-react'
import { api } from '../api/client'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import type { LightningBolt11Instruction } from '../types'

const PRESETS = [1000, 5000, 10000, 25000]
const POLL_MS = 3000

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  onPaid: () => void
}

export function TopupBalanceDialog({ open, onOpenChange, onPaid }: Props) {
  const [amount, setAmount] = useState(PRESETS[1])
  const [invoice, setInvoice] = useState<LightningBolt11Instruction | null>(null)
  const [status, setStatus] = useState('pending')
  const [error, setError] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (!open) return
    setInvoice(null); setStatus('pending'); setError(null); setAmount(PRESETS[1])
  }, [open])

  useEffect(() => {
    if (!invoice || !open) return
    let stopped = false
    async function poll() {
      try {
        const result = await api.getInvoiceStatus(invoice!.payment_hash)
        if (stopped) return
        setStatus(result.status)
        if (result.status === 'paid') { onPaid(); return }
        if (result.status === 'expired' || result.status === 'cancelled') return
      } catch {
        if (stopped) return
      }
      setTimeout(poll, POLL_MS)
    }
    poll()
    return () => { stopped = true }
  }, [invoice, open])

  async function create(e: React.FormEvent) {
    e.preventDefault()
    setCreating(true); setError(null)
    try {
      const response = await api.createLightningInvoice({ amount_sats: amount })
      if (response.instruction.type !== 'lightning_bolt11') throw new Error('Unexpected payment instruction')
      setInvoice(response.instruction)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to create invoice')
    } finally {
      setCreating(false)
    }
  }

  async function copy() {
    if (!invoice) return
    await navigator.clipboard.writeText(invoice.bolt11)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }

  const label = 'font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground'

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md border-border bg-card">
        <DialogHeader>
          <DialogTitle className="font-display text-xl tracking-[0.04em] text-brand">TOP UP BALANCE</DialogTitle>
          <DialogDescription className="font-mono text-[12px] text-muted-foreground">Pay a Lightning invoice to add sats to your balance.</DialogDescription>
        </DialogHeader>
        {error && <div className="font-mono text-[12px] text-destructive bg-destructive/5 border border-destructive/20 p-2 px-3">{error}</div>}
        {!invoice ? (
          <form onSubmit={create} className="flex flex-col gap-4">
            <div className="grid grid-cols-4 gap-2">
              {PRESETS.map(p => (
                <button key={p} type="button" onClick={() => setAmount(p)} className={`font-mono text-[12px] tabular-nums py-2 border ${amount === p ? 'border-brand/40 bg-brand/10 text-brand' : 'border-border text-muted-foreground hover:text-foreground'}`}>
                  {p.toLocaleString()}
                </button>
              ))}
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="topup-amount" className={label}>Amount (sats)</Label>
              <Input id="topup-amount" type="number" min={1} step={1} value={amount} onChange={e => setAmount(Number(e.target.value))} required className="font-mono bg-surface border-border text-foreground" />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
              <Button type="submit" disabled={creating || amount < 1} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand">{creating ? '...' : 'CREATE INVOICE'}</Button>
            </DialogFooter>
          </form>
        ) : (
          <div className="flex flex-col gap-4">
            <div className="flex items-center justify-between font-mono text-[12px]">
              <span className="text-muted-foreground uppercase tracking-wider text-[10px]">{status}</span>
              <span className="tabular-nums text-brand">{invoice.amount_sats.toLocaleString()} sats</span>
            </div>
            {status === 'paid' ? (
              <div className="flex items-center justify-center gap-2 py-10 font-mono text-[13px] text-brand"><CheckIcon className="size-4" /> PAID</div>
            ) : (
              <a href={`lightning:${invoice.bolt11}`} className={`mx-auto bg-white p-3 ${status !== 'pending' ? 'opacity-30' : ''}`}>
                <QRCodeSVG value={invoice.bolt11.toUpperCase()} size={208} />
              </a>
            )}
            <div className="flex gap-2">
              <Input readOnly value={invoice.bolt11} className="font-mono text-[11px] bg-surface border-border text-muted-foreground" />
              <Button type="button" variant="outline" size="icon" onClick={copy} className="border-border shrink-0">{copied ? <CheckIcon className="size-3.5" /> : <CopyIcon className="size-3.5" />}</Button>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)} className="font-mono text-[12px] border-border text-muted-foreground">{status === 'paid' ? 'DONE' : 'CLOSE'}</Button>
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
