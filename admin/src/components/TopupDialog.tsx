import { useState, useEffect } from 'react'
import { CopyIcon, CheckIcon, WalletIcon, ZapIcon, ClockIcon } from 'lucide-react'
import { QRCodeSVG } from 'qrcode.react'
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
}

type InvoiceStatus = 'pending' | 'paid' | 'expired' | 'cancelled'

export function TopupDialog({ open, onOpenChange, providerSlug, providerName }: TopupDialogProps) {
  const [step, setStep] = useState<'form' | 'invoice'>('form')
  const [amount, setAmount] = useState('1000')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [invoice, setInvoice] = useState<{
    invoice_id?: number
    bolt11: string
    payment_hash: string
    amount_sats: number
    expires_at?: string
  } | null>(null)
  const [status, setStatus] = useState<InvoiceStatus>('pending')
  const [copied, setCopied] = useState(false)
  const [pollInterval, setPollInterval] = useState<number | null>(null)

  // Clean up interval on unmount
  useEffect(() => {
    return () => {
      if (pollInterval) clearInterval(pollInterval)
    }
  }, [pollInterval])

  // Reset state when dialog opens/closes
  useEffect(() => {
    if (!open) {
      setStep('form')
      setInvoice(null)
      setStatus('pending')
      setError(null)
      setCopied(false)
      if (pollInterval) clearInterval(pollInterval)
      setPollInterval(null)
    }
  }, [open])

  async function handleCreateInvoice() {
    const amountNum = parseInt(amount, 10)
    if (isNaN(amountNum) || amountNum <= 0) {
      setError('Please enter a valid amount')
      return
    }

    setLoading(true)
    setError(null)
    try {
      const data = await api.createProviderInvoice(providerSlug, {
        amount_sats: amountNum,
        memo: `Topup for ${providerName}`,
        expire_seconds: 1800, // 30 minutes
      })
      setInvoice(data)
      setStep('invoice')
      
      // Start polling for payment
      const interval = window.setInterval(async () => {
        try {
          const statusData = await api.getInvoiceStatus(data.payment_hash)
          setStatus(statusData.status as InvoiceStatus)
          
          if (statusData.status === 'paid') {
            if (pollInterval) clearInterval(pollInterval)
            setPollInterval(null)
          }
        } catch (e) {
          // Ignore polling errors, keep trying
          console.error('Failed to poll invoice status:', e)
        }
      }, 3000) // Poll every 3 seconds
      
      setPollInterval(interval)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to create invoice')
    } finally {
      setLoading(false)
    }
  }

  async function handleCopy() {
    if (!invoice) return
    try {
      await navigator.clipboard.writeText(invoice.bolt11)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch (e) {
      setError('Failed to copy to clipboard')
    }
  }

  function handleClose() {
    if (pollInterval) clearInterval(pollInterval)
    onOpenChange(false)
  }

  function getStatusBadge(status: InvoiceStatus) {
    switch (status) {
      case 'paid':
        return <Badge className="bg-emerald-100 text-emerald-800 dark:bg-emerald-900/30 dark:text-emerald-400">Paid</Badge>
      case 'pending':
        return <Badge variant="secondary">Pending</Badge>
      case 'expired':
        return <Badge variant="outline">Expired</Badge>
      case 'cancelled':
        return <Badge variant="outline">Cancelled</Badge>
      default:
        return <Badge variant="secondary">{status}</Badge>
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <WalletIcon className="size-5" />
            Top-up Provider Balance
          </DialogTitle>
          <DialogDescription>
            Create a Lightning invoice to add funds to your provider account.
          </DialogDescription>
        </DialogHeader>

        {error && (
          <Alert variant="destructive">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {step === 'form' && (
          <div className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="amount">Amount (sats)</Label>
              <Input
                id="amount"
                type="number"
                min="1"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                placeholder="1000"
              />
            </div>
            
              <div className="flex flex-col gap-1.5">
                <Label>Provider</Label>
                <div className="font-mono text-sm text-muted-foreground bg-muted p-2 rounded">
                  {providerName} ({providerSlug})
                </div>
              </div>

            <div className="flex gap-2 pt-2">
              <Button variant="outline" onClick={() => handleClose()} disabled={loading}>
                Cancel
              </Button>
              <Button onClick={handleCreateInvoice} disabled={loading}>
                {loading ? 'Creating...' : 'Generate Invoice'}
              </Button>
            </div>
          </div>
        )}

        {step === 'invoice' && invoice && (
          <div className="flex flex-col gap-4">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <ZapIcon className="size-4 text-muted-foreground" />
                <span className="text-sm font-medium">Amount:</span>
              </div>
              <div className="font-mono text-sm">
                {invoice.amount_sats.toLocaleString()} sats
              </div>
            </div>

            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <ClockIcon className="size-4 text-muted-foreground" />
                <span className="text-sm font-medium">Status:</span>
              </div>
              {getStatusBadge(status)}
            </div>

            {status === 'paid' && (
              <Alert className="bg-emerald-50 dark:bg-emerald-950/20 border-emerald-200 dark:border-emerald-900">
                <CheckIcon className="size-4 text-emerald-600 dark:text-emerald-400" />
                <AlertDescription className="text-emerald-800 dark:text-emerald-400">
                  Payment received! Your balance has been updated.
                </AlertDescription>
              </Alert>
            )}

            <Card>
              <CardContent className="p-4 flex flex-col items-center gap-4">
                <div className="bg-white p-4 rounded-lg">
                  <QRCodeSVG 
                    value={invoice.bolt11}
                    size={180}
                    level="H"
                    includeMargin={true}
                  />
                </div>
                <div className="text-xs text-muted-foreground text-center">
                  Scan with your Lightning wallet to pay
                </div>
              </CardContent>
            </Card>

            <div className="flex flex-col gap-1.5">
              <Label htmlFor="bolt11">Bolt11 Invoice</Label>
              <div className="flex gap-2">
                <Input
                  id="bolt11"
                  value={invoice.bolt11}
                  readOnly
                  className="font-mono text-xs break-all pr-8"
                />
                <Button 
                  variant="outline" 
                  size="icon"
                  onClick={handleCopy}
                  disabled={copied}
                >
                  {copied ? <CheckIcon className="size-4" /> : <CopyIcon className="size-4" />}
                </Button>
              </div>
              {copied && (
                <div className="text-xs text-emerald-600 dark:text-emerald-400">
                  Copied to clipboard!
                </div>
              )}
            </div>

            <div className="flex gap-2 pt-2">
              <Button variant="outline" onClick={() => handleClose()}>
                Done
              </Button>
              {status === 'pending' && (
                <Button variant="outline" onClick={() => {
                  if (pollInterval) clearInterval(pollInterval)
                  setStep('form')
                  setInvoice(null)
                }}>
                  Cancel Invoice
                </Button>
              )}
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
