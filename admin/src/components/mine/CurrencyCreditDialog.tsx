import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { userApi } from '@/api/userApi'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  onDone: (message: string) => void
  onError: (message: string) => void
}

export default function CurrencyCreditDialog({ open, onOpenChange, onDone, onError }: Props) {
  const [userId, setUserId] = useState('')
  const [currency, setCurrency] = useState('usd')
  const [amount, setAmount] = useState('')
  const [saving, setSaving] = useState(false)

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    setSaving(true)
    try {
      await userApi.creditCurrency({
        user_id: Number(userId),
        currency,
        amount_milli: Math.round(Number(amount) * 100_000),
        reason: 'admin_credit',
      })
      onOpenChange(false)
      onDone(`CREDITED ${currency.toUpperCase()}`)
    } catch (err) {
      onError(err instanceof Error ? err.message : 'Failed to credit')
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="border-border bg-card">
        <DialogHeader>
          <DialogTitle className="font-display text-xl tracking-[0.04em] text-brand">CREDIT FIAT BALANCE</DialogTitle>
          <DialogDescription className="font-mono text-[12px] text-muted-foreground">
            Amount is in whole units of the currency (for example dollars). Stored in thousandths of a cent.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="fc-user" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">User ID</Label>
            <Input id="fc-user" type="number" min={1} value={userId} onChange={e => setUserId(e.target.value)} required className="font-mono bg-surface border-border" />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="fc-cur" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Currency</Label>
            <Input id="fc-cur" value={currency} onChange={e => setCurrency(e.target.value)} required className="font-mono bg-surface border-border" />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="fc-amt" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Amount</Label>
            <Input id="fc-amt" type="number" min={0.00001} step="any" value={amount} onChange={e => setAmount(e.target.value)} required className="font-mono bg-surface border-border" />
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={saving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
            <Button type="submit" disabled={saving} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand">{saving ? '...' : 'CREDIT'}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
