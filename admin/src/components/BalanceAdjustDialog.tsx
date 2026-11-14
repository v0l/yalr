import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { api } from '../api/client'
import type { BalanceAdjustKind, CurrencyOption } from '../types'

export interface BalanceAdjustDialogProps {
  kind: BalanceAdjustKind | null
  onClose: () => void
  onDone: (message: string) => void
  onError: (message: string) => void
}

const FALLBACK: CurrencyOption[] = [{ code: 'sats', milli_per_unit: 1000 }, { code: 'usd', milli_per_unit: 100000 }]

export function BalanceAdjustDialog({ kind, onClose, onDone, onError }: BalanceAdjustDialogProps) {
  const [currencies, setCurrencies] = useState<CurrencyOption[]>(FALLBACK)
  const [userId, setUserId] = useState('')
  const [currency, setCurrency] = useState('sats')
  const [amount, setAmount] = useState('')
  const [reason, setReason] = useState('')
  const [saving, setSaving] = useState(false)
  const open = kind !== null

  useEffect(() => {
    if (!open) return
    setUserId(''); setCurrency('sats'); setAmount(''); setReason('')
    api.listCurrencies().then(setCurrencies).catch(() => setCurrencies(FALLBACK))
  }, [open])

  const selected = currencies.find(c => c.code === currency) ?? FALLBACK[0]
  const unit = currency === 'sats' ? 'sats' : `whole ${currency.toUpperCase()} units`
  const credit = kind === 'credit'

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    if (!kind) return
    setSaving(true)
    try {
      await api.adjustBalance(kind, {
        user_id: Number(userId),
        currency,
        amount_milli: Math.round(Number(amount) * selected.milli_per_unit),
        reason: reason || undefined,
      })
      onClose()
      onDone(`${credit ? 'CREDITED' : 'DEBITED'} ${currency.toUpperCase()}`)
    } catch (err) {
      onError(err instanceof Error ? err.message : `Failed to ${kind}`)
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={o => { if (!o) onClose() }}>
      <DialogContent className="border-border bg-card">
        <DialogHeader>
          <DialogTitle className={`font-display text-xl tracking-[0.04em] ${credit ? 'text-brand' : 'text-destructive'}`}>
            {credit ? 'CREDIT BALANCE' : 'DEBIT BALANCE'}
          </DialogTitle>
          <DialogDescription className="font-mono text-[12px] text-muted-foreground">
            Manually {credit ? 'add funds to' : 'deduct funds from'} a user&apos;s balance.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="adj-user" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">User ID</Label>
            <Input id="adj-user" type="number" min={1} value={userId} onChange={e => setUserId(e.target.value)} required className="font-mono bg-surface border-border text-foreground" />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="adj-currency" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Currency</Label>
            <Select value={currency} onValueChange={setCurrency}>
              <SelectTrigger id="adj-currency" className="font-mono bg-surface border-border text-foreground"><SelectValue /></SelectTrigger>
              <SelectContent className="bg-card border-border">
                <SelectGroup>
                  {currencies.map(c => <SelectItem key={c.code} value={c.code} className="font-mono text-foreground">{c.code.toUpperCase()}</SelectItem>)}
                </SelectGroup>
              </SelectContent>
            </Select>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="adj-amount" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Amount ({unit})</Label>
            <Input id="adj-amount" type="number" min={0.00001} step="any" value={amount} onChange={e => setAmount(e.target.value)} required className="font-mono bg-surface border-border text-foreground" />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="adj-reason" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Reason</Label>
            <Input id="adj-reason" value={reason} onChange={e => setReason(e.target.value)} placeholder="Optional" className="font-mono bg-surface border-border text-foreground" />
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose} disabled={saving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
            <Button type="submit" disabled={saving} className={`font-mono text-[12px] tracking-wider uppercase border ${credit ? 'border-brand/40 bg-brand/10 text-brand' : 'border-destructive/40 bg-destructive/10 text-destructive'}`}>
              {saving ? '...' : credit ? 'CREDIT' : 'DEBIT'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
