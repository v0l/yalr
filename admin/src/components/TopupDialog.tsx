import { useState, useEffect } from 'react'
import { CheckIcon, WalletIcon, CopyIcon, ExternalLinkIcon } from 'lucide-react'
import { api } from '../api/client'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Card, CardContent } from '@/components/ui/card'
import type { TopupResponse, PaymentInstruction, LightningBolt11Instruction, TopupRequest, PaymentOption } from '../types'

interface TopupDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  providerSlug: string
  providerName: string
  supportedPaymentMethods?: PaymentOption[]
}

export function TopupDialog({ open, onOpenChange, providerSlug, providerName, supportedPaymentMethods = [] }: TopupDialogProps) {
  const [amount, setAmount] = useState('10')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [success, setSuccess] = useState<string | null>(null)
  const [topupResult, setTopupResult] = useState<TopupResponse | null>(null)
  
  // Get the first payment option for this provider (or default to usd_micro/lightning)
  const paymentOption = supportedPaymentMethods?.[0] || {
    currency: 'usd_micro' as const,
    payment_method: 'lightning' as const
  }
  const currencyLabel = paymentOption.currency === 'sats' ? 'Sats' : 
                        paymentOption.currency === 'msats' ? 'Millisatoshis' : 
                        paymentOption.currency === 'usd_micro' ? 'USD (micro)' : 'Amount'

  // Reset state when dialog opens/closes
  useEffect(() => {
    if (open) {
      setAmount('10')
      setError(null)
      setSuccess(null)
      setTopupResult(null)
    }
  }, [open])

  async function handleCreateTopup() {
    const amountNum = parseFloat(amount)
    if (isNaN(amountNum) || amountNum <= 0) {
      setError('Please enter a valid amount')
      return
    }

    setLoading(true)
    setError(null)
    setSuccess(null)
    setTopupResult(null)
    
    try {
      const data: TopupRequest = {
        amount: paymentOption.currency === 'usd_micro' ? Math.round(amountNum * 1_000_000) : amountNum,
        currency: paymentOption.currency,
      }
      
      const result = await api.topupProvider(providerSlug, data)
      setTopupResult(result)
      setSuccess('Top-up request created! Follow the instructions below to complete payment.')
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to create top-up')
    } finally {
      setLoading(false)
    }
  }

  function handleClose() {
    onOpenChange(false)
  }

  function copyToClipboard(text: string) {
    navigator.clipboard.writeText(text)
    setSuccess('Copied to clipboard!')
    setTimeout(() => setSuccess(null), 3000)
  }

  function renderInstruction(instruction: PaymentInstruction) {
    switch (instruction.type) {
      case 'lightning_bolt11': {
        const inv = instruction as LightningBolt11Instruction
        return (
          <Card className="mt-4">
            <CardContent className="pt-6">
              <div className="space-y-4">
                <div>
                  <Label className="text-sm font-medium">Lightning Invoice (Bolt11)</Label>
                  <div className="flex gap-2 mt-2">
                    <Input
                      value={inv.bolt11}
                      readOnly
                      className="font-mono text-xs flex-1"
                      style={{ wordBreak: 'break-all' }}
                    />
                    <Button
                      variant="outline"
                      size="icon"
                      onClick={() => copyToClipboard(inv.bolt11)}
                      title="Copy invoice"
                    >
                      <CopyIcon className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
                
                <div className="text-sm text-muted-foreground">
                  <div className="flex justify-between">
                    <span>Amount:</span>
                    <span className="font-medium">{inv.amount_sats} sats</span>
                  </div>
                  {inv.memo && (
                    <div className="flex justify-between mt-1">
                      <span>Memo:</span>
                      <span>{inv.memo}</span>
                    </div>
                  )}
                  {inv.expires_at && (
                    <div className="flex justify-between mt-1">
                      <span>Expires:</span>
                      <span>{new Date(inv.expires_at * 1000).toLocaleString()}</span>
                    </div>
                  )}
                </div>
                
                <Alert className="bg-blue-50 dark:bg-blue-950/20 border-blue-200 dark:border-blue-900">
                  <AlertDescription className="text-blue-800 dark:text-blue-400">
                    Scan the QR code with your Lightning wallet or copy the invoice to pay.
                  </AlertDescription>
                </Alert>
              </div>
            </CardContent>
          </Card>
        )
      }
      
      case 'redirect': {
        const redir = instruction as { type: 'redirect'; url: string; amount_usd?: number }
        return (
          <Card className="mt-4">
            <CardContent className="pt-6">
              <div className="space-y-4">
                <div className="text-sm text-muted-foreground">
                  You will be redirected to an external payment page to complete your top-up.
                  {redir.amount_usd && (
                    <div className="mt-2 font-medium">Amount: ${redir.amount_usd.toFixed(2)}</div>
                  )}
                </div>
                <Button onClick={() => window.open(redir.url, '_blank')} className="w-full">
                  <ExternalLinkIcon className="mr-2 h-4 w-4" />
                  Go to Payment Page
                </Button>
              </div>
            </CardContent>
          </Card>
        )
      }
      
      case 'payment_link': {
        const link = instruction as { type: 'payment_link'; url: string; amount_usd?: number; label?: string }
        return (
          <Card className="mt-4">
            <CardContent className="pt-6">
              <div className="space-y-4">
                <div className="text-sm text-muted-foreground">
                  Click the button below to open the payment page.
                  {link.amount_usd && (
                    <div className="mt-2 font-medium">Amount: ${link.amount_usd.toFixed(2)}</div>
                  )}
                </div>
                <Button onClick={() => window.open(link.url, '_blank')} className="w-full">
                  <ExternalLinkIcon className="mr-2 h-4 w-4" />
                  {link.label || 'Pay Now'}
                </Button>
              </div>
            </CardContent>
          </Card>
        )
      }
      
      case 'manual': {
        const manual = instruction as { type: 'manual'; instructions: string; amount_usd?: number; reference_code?: string }
        return (
          <Card className="mt-4">
            <CardContent className="pt-6">
              <div className="space-y-4">
                {manual.amount_usd && (
                  <div className="font-medium">Amount: ${manual.amount_usd.toFixed(2)}</div>
                )}
                {manual.reference_code && (
                  <div className="text-sm">
                    <Label>Reference Code</Label>
                    <div className="font-mono bg-muted p-2 rounded mt-1">
                      {manual.reference_code}
                    </div>
                  </div>
                )}
                <div>
                  <Label className="text-sm font-medium">Payment Instructions</Label>
                  <div className="mt-2 text-sm whitespace-pre-wrap">
                    {manual.instructions}
                  </div>
                </div>
              </div>
            </CardContent>
          </Card>
        )
      }
      
      default:
        return null
    }
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
            Add funds to your {providerName} account. Select an amount and follow the payment instructions.
          </DialogDescription>
        </DialogHeader>

        {error && (
          <Alert variant="destructive">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {success && !topupResult && (
          <Alert className="bg-emerald-50 dark:bg-emerald-950/20 border-emerald-200 dark:border-emerald-900">
            <CheckIcon className="size-4 text-emerald-600 dark:text-emerald-400" />
            <AlertDescription className="text-emerald-800 dark:text-emerald-400">
              {success}
            </AlertDescription>
          </Alert>
        )}

        {!topupResult ? (
          <div className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="amount">Amount ({currencyLabel})</Label>
              <Input
                id="amount"
                type="number"
                min="0.01"
                step={paymentOption.currency === 'usd_micro' ? "0.01" : "1"}
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                placeholder={paymentOption.currency === 'usd_micro' ? "10" : "1000"}
              />
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
        ) : (
          <div className="space-y-4">
            {topupResult.message && (
              <Alert className="bg-blue-50 dark:bg-blue-950/20 border-blue-200 dark:border-blue-900">
                <AlertDescription className="text-blue-800 dark:text-blue-400">
                  {topupResult.message}
                </AlertDescription>
              </Alert>
            )}
            
            {topupResult.instruction && renderInstruction(topupResult.instruction)}
            
            <div className="flex gap-2 pt-2">
              <Button variant="outline" onClick={handleClose}>
                Close
              </Button>
              <Button variant="secondary" onClick={() => setTopupResult(null)}>
                Try Different Amount
              </Button>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
