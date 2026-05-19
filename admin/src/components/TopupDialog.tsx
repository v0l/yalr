import { useState, useEffect } from 'react'
import { CopyIcon, CheckIcon, WalletIcon, ZapIcon, ClockIcon, ExternalLinkIcon } from 'lucide-react'
import { api } from '../api/client'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Card, CardContent } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'

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
        const response = await api.post(`/providers/${providerSlug}/topup`, {
          amount_usd: amountNum,
          currency: 'USD'
        })
        setSuccess('Top-up request created! Check your email or PPQ dashboard for payment instructions.')
      } else {
        // For Routstr, create Lightning invoice (existing flow)
        const data = await api.createProviderInvoice(providerSlug, {
          amount_sats: Math.round(amountNum),
          memo: `Topup for ${providerName}`,
          expire_seconds: 1800,
        })
        setSuccess('Invoice created! Check your email for payment instructions.')
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
              : 'Create a Lightning invoice to add funds to your provider account.'}
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
              <Select value={currency} onValueChange={(v) => setCurrency(v as TopupCurrency)}>
                <SelectTrigger className="w-24">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="usd">USD</SelectItem>
                  {isRoutstr && <SelectItem value="sats">sats</SelectItem>}
                </SelectContent>
              </Select>
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

// Simple Select component for currency
function Select({ value, onValueChange, children }: { value: string, onValueChange: (v: string) => void, children: React.ReactNode }) {
  return (
    <select 
      value={value} 
      onChange={(e) => onValueChange(e.target.value)}
      className="flex h-9 w-full items-center justify-between rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm ring-offset-background placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
    >
      {children}
    </select>
  )
}

function SelectTrigger({ children, className }: { children: React.ReactNode, className?: string }) {
  return <div className={className}>{children}</div>
}

function SelectContent({ children }: { children: React.ReactNode }) {
  return <>{children}</>
}

function SelectItem({ value, children }: { value: string, children: React.ReactNode }) {
  return <option value={value}>{children}</option>
}

function SelectValue({}: { children?: React.ReactNode }) {
  return null
}
