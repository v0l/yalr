import { useState, useEffect } from 'react'
import { CheckIcon, WalletIcon } from 'lucide-react'
import { api } from '../api/client'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Alert, AlertDescription } from '@/components/ui/alert'

interface TopupDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  providerSlug: string
  providerName: string
  providerType?: string
}

type TopupCurrency = 'sats' | 'usd'

export function TopupDialog({ open, onOpenChange, providerSlug, providerName, providerType }: TopupDialogProps) {
  const [amount, setAmount] = useState('10')
  const [currency, setCurrency] = useState<TopupCurrency>('usd')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [success, setSuccess] = useState<string | null>(null)

  // Check if this is a PPQ provider
  const isPpq = providerType === 'ppq'
  const isRoutstr = providerType === 'routstr'

  // Reset state when dialog opens/closes
  useEffect(() => {
    if (open) {
      setAmount(isPpq ? '10' : '1000')
      setCurrency('usd')
      setError(null)
      setSuccess(null)
    }
  }, [open, isPpq])

  async function handleCreateTopup() {
    const amountNum = parseFloat(amount)
    if (isNaN(amountNum) || amountNum <= 0) {
      setError('Please enter a valid amount')
      return
    }

    setLoading(true)
    setError(null)
    setSuccess(null)
    try {
      if (isPpq) {
        // For PPQ, call the new topup endpoint with USD amount
        await api.topupProvider(providerSlug, {
          amount_usd: amountNum,
          currency: 'USD'
        })
        setSuccess('Top-up request created! Check your email or PPQ dashboard for payment instructions.')
      } else if (isRoutstr) {
        // For Routstr, create Lightning invoice
        await api.createProviderInvoice(providerSlug, {
          amount_sats: Math.round(amountNum),
          memo: `Topup for ${providerName}`,
          expire_seconds: 1800,
        })
        setSuccess('Invoice created! Check your email for payment instructions.')
      } else {
        setError('Top-up is not supported for this provider type')
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to create top-up')
    } finally {
      setLoading(false)
    }
  }

  function handleClose() {
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <WalletIcon className="size-5" />
            Top-up {providerName}
          </DialogTitle>
          <DialogDescription>
            {isPpq 
              ? 'Add USD credits to your PPQ account. Payments are processed through PPQ\'s payment system.'
              : isRoutstr
                ? 'Create a Lightning invoice to add funds to your Routstr account.'
                : 'Select an amount to top-up your provider account.'}
          </DialogDescription>
        </DialogHeader>

        {error && (
          <Alert variant="destructive">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {success && (
          <Alert className="bg-emerald-50 dark:bg-emerald-950/20 border-emerald-200 dark:border-emerald-900">
            <CheckIcon className="size-4 text-emerald-600 dark:text-emerald-400" />
            <AlertDescription className="text-emerald-800 dark:text-emerald-400">
              {success}
            </AlertDescription>
          </Alert>
        )}

        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="amount">Amount</Label>
            <div className="flex gap-2">
              <Input
                id="amount"
                type="number"
                min="0.01"
                step={isPpq ? '0.01' : '1'}
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                placeholder={isPpq ? '10' : '1000'}
                className="flex-1"
              />
              <select 
                value={currency} 
                onChange={(e) => setCurrency(e.target.value as TopupCurrency)}
                className="flex h-9 w-24 items-center justify-between rounded-md border border-input bg-transparent px-3 py-2 text-sm"
              >
                <option value="usd">USD</option>
                {isRoutstr && <option value="sats">sats</option>}
              </select>
            </div>
          </div>
          
          <div className="flex flex-col gap-1.5">
            <Label>Provider</Label>
            <div className="font-mono text-sm text-muted-foreground bg-muted p-2 rounded">
              {providerName} ({providerSlug})
            </div>
          </div>

          <div className="flex gap-2 pt-2">
            <Button variant="outline" onClick={handleClose} disabled={loading}>
              Cancel
            </Button>
            <Button onClick={handleCreateTopup} disabled={loading}>
              {loading ? 'Creating...' : 'Create Top-up'}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
