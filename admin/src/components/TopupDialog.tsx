import { useState, useEffect } from 'react'
import { CheckIcon, WalletIcon, CopyIcon, ExternalLinkIcon, ArrowLeftIcon, ZapIcon, GlobeIcon, FileTextIcon, LinkIcon } from 'lucide-react'
import { api } from '../api/client'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Alert, AlertDescription } from '@/components/ui/alert'
import type { TopupResponse, LightningBolt11Instruction, TopupRequest, PaymentOption, CurrencyAmount } from '../types'

interface TopupDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  providerSlug: string
  providerName: string
  supportedPaymentMethods?: PaymentOption[]
  currentBalance?: CurrencyAmount
}

/* ── Helpers ──────────────────────────────────────────────────── */
const PRESETS: Record<string, { values: number[]; label: (v: number) => string; step: string; min: string }> = {
  usd_micro: { values: [5, 10, 25, 50], label: v => `$${v}`, step: '0.01', min: '0.01' },
  sats: { values: [1000, 5000, 10000, 25000], label: v => `${v.toLocaleString()}`, step: '1', min: '1' },
  msats: { values: [1000000, 5000000, 10000000, 25000000], label: v => `${(v / 1000).toLocaleString()}k`, step: '1', min: '1' },
}
const DEFAULT_PRESET = PRESETS.usd_micro

function formatBalance(amount: number, currency: 'msats' | 'sats' | 'usd_micro'): string {
  switch (currency) {
    case 'msats': return `${Math.floor(amount / 1000)} sats`
    case 'sats': return `${amount.toLocaleString()} sats`
    case 'usd_micro': return `$${(amount / 1_000_000).toFixed(2)}`
    default: return `${amount}`
  }
}

/* ═══════════════════════════════════════════════════════════════ */
/*  TopupDialog — two-step flow                                   */
/* ═══════════════════════════════════════════════════════════════ */

export function TopupDialog({ open, onOpenChange, providerSlug, providerName, supportedPaymentMethods = [], currentBalance }: TopupDialogProps) {
  /* state */
  const [step, setStep] = useState<'amount' | 'invoice'>('amount')
  const [amount, setAmount] = useState('25')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [success, setSuccess] = useState<string | null>(null)
  const [topupResult, setTopupResult] = useState<TopupResponse | null>(null)
  const [selectedMethod, setSelectedMethod] = useState<string>('')
  const [copiedInvoice, setCopiedInvoice] = useState(false)
  const [copiedTimeout, setCopiedTimeout] = useState<ReturnType<typeof setTimeout> | null>(null)

  const paymentMethods = supportedPaymentMethods.length ? supportedPaymentMethods : supportedPaymentMethods

  /* derive primary payment option */
  const paymentOption = selectedMethod
    ? paymentMethods.find(m => m.payment_method === selectedMethod)
    : paymentMethods[0]

  const currencyLabel = paymentOption?.currency === 'sats' ? 'SATS'
    : paymentOption?.currency === 'msats' ? 'MSATS'
    : paymentOption?.currency === 'usd_micro' ? 'USD'
    : 'AMOUNT'

  const presets = PRESETS[paymentOption?.currency ?? 'usd_micro'] || DEFAULT_PRESET
  const isUsd = paymentOption?.currency === 'usd_micro'

  /* reset on open */
  useEffect(() => {
    if (open) {
      setStep('amount')
      const cur = paymentMethods[0]?.currency ?? 'usd_micro'
      const p = PRESETS[cur] || DEFAULT_PRESET
      setAmount(String(p.values[0]))
      setError(null)
      setSuccess(null)
      setTopupResult(null)
      setCopiedInvoice(false)
      if (paymentMethods.length > 0) setSelectedMethod(paymentMethods[0].payment_method)
    }
  }, [open])

  /* ── handlers ────────────────────────────────────────────── */
  function selectPreset(v: number) {
    setAmount(String(v))
  }

  async function handleCreateTopup() {
    const amountNum = parseFloat(amount)
    if (isNaN(amountNum) || amountNum <= 0) { setError('Please enter a valid amount'); return }
    setLoading(true); setError(null); setSuccess(null)
    try {
      const data: TopupRequest = {
        amount: isUsd ? Math.round(amountNum * 1_000_000) : amountNum,
        currency: paymentOption?.currency || 'usd_micro',
        preferred_method: paymentOption?.payment_method,
      }
      const result = await api.topupProvider(providerSlug, data)
      setTopupResult(result)
      setStep('invoice')
      /* auto-copy bolt11 invoice */
      if (result.instruction.type === 'lightning_bolt11') {
        const inv = result.instruction as LightningBolt11Instruction
        navigator.clipboard.writeText(inv.bolt11)
        setCopiedInvoice(true)
        const t = setTimeout(() => setCopiedInvoice(false), 5000)
        if (copiedTimeout) clearTimeout(copiedTimeout)
        setCopiedTimeout(t)
      }
    } catch (e) { setError(e instanceof Error ? e.message : 'Failed to create top-up') }
    finally { setLoading(false) }
  }

  function copyToClipboard(text: string) {
    navigator.clipboard.writeText(text)
    setCopiedInvoice(true)
    const t = setTimeout(() => setCopiedInvoice(false), 5000)
    if (copiedTimeout) clearTimeout(copiedTimeout)
    setCopiedTimeout(t)
  }

  function goToAmount() { setStep('amount'); setTopupResult(null); setError(null) }

  /* ── step 1: amount selection ────────────────────────────── */
  function renderAmountStep() {
    return (
      <div className="flex flex-col gap-5">
        {/* Presets */}
        <div>
          <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground mb-2.5">
            Quick Amount ({currencyLabel})
          </div>
          <div className="grid grid-cols-4 gap-2">
            {presets.values.map(v => (
              <button
                key={v}
                onClick={() => selectPreset(v)}
                className={`px-3 py-2.5 font-mono text-[14px] font-bold tabular-nums text-center border border-border transition-colors
                  ${amount === String(v) ? 'bg-brand/10 border-brand/40 text-brand' : 'bg-surface text-foreground hover:border-brand/20'}`}
              >
                {presets.label(v)}
              </button>
            ))}
          </div>
        </div>

        {/* Custom */}
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="amount-custom" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Custom Amount</Label>
          <div className="relative">
            {isUsd && <span className="absolute left-3 top-1/2 -translate-y-1/2 font-mono text-muted-foreground text-[13px]">$</span>}
            <Input
              id="amount-custom"
              type="number"
              min={presets.min}
              step={presets.step}
              value={amount}
              onChange={e => setAmount(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter') handleCreateTopup() }}
              className={`font-mono text-[24px] font-bold bg-surface border-border text-foreground h-14 focus:border-brand/50 ${presets.step === '0.01' ? 'pl-7' : 'pl-3'}`}
            />
          </div>
        </div>

        {/* Payment method */}
        {paymentMethods.length > 1 && (
          <div>
            <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground mb-2">Payment Method</div>
            <div className="grid grid-cols-4 gap-1.5">
              {paymentMethods.map(m => {
                const icon = m.payment_method === 'lightning' ? <ZapIcon className="size-3" />
                  : m.payment_method === 'redirect' ? <GlobeIcon className="size-3" />
                  : m.payment_method === 'payment_link' ? <LinkIcon className="size-3" />
                  : <FileTextIcon className="size-3" />
                return (
                  <button
                    key={m.payment_method}
                    onClick={() => { setSelectedMethod(m.payment_method); const p = PRESETS[m.currency] || DEFAULT_PRESET; setAmount(String(p.values[0])) }}
                    className={`flex flex-col items-center gap-1 py-2.5 text-center border transition-colors
                      ${selectedMethod === m.payment_method ? 'border-brand/40 bg-brand/10 text-brand' : 'border-border bg-surface text-muted-foreground hover:border-brand/20'}`}
                  >
                    {icon}
                    <span className="font-mono text-[9px] uppercase tracking-wider">{m.payment_method}</span>
                  </button>
                )
              })}
            </div>
          </div>
        )}

        {/* Actions */}
        <div className="flex gap-2 pt-1">
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={loading} className="font-mono text-[12px] border-border text-muted-foreground w-1/2">
            CANCEL
          </Button>
          <Button onClick={handleCreateTopup} disabled={loading} className="font-mono text-[12px] uppercase tracking-wider border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20 w-1/2">
            {loading ? 'CREATING…' : 'TOP UP'}
          </Button>
        </div>
      </div>
    )
  }

  /* ── step 2: payment instruction ──────────────────────────── */
  function renderInvoiceStep() {
    const instruction = topupResult?.instruction
    if (!instruction) return null

    return (
      <div className="flex flex-col gap-4">
        {/* Back link */}
        <button onClick={goToAmount} className="inline-flex items-center gap-1.5 font-mono text-[11px] text-muted-foreground hover:text-foreground transition-colors w-fit">
          <ArrowLeftIcon className="size-3" /> CHANGE AMOUNT
        </button>

        {topupResult?.message && (
          <Alert className="border-brand/20 bg-brand/5 text-muted-foreground font-mono text-[12px]">
            <AlertDescription>{topupResult.message}</AlertDescription>
          </Alert>
        )}

        {instruction.type === 'lightning_bolt11' && (
          <div className="space-y-4">
            <div>
              <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground mb-2">Lightning Invoice</div>
              <div className="relative">
                <div className="font-mono text-[11px] break-all bg-surface border border-border p-3 text-brand pr-12 leading-relaxed">
                  {(instruction as LightningBolt11Instruction).bolt11}
                </div>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => copyToClipboard((instruction as LightningBolt11Instruction).bolt11)}
                  className={`absolute right-1 top-1/2 -translate-y-1/2 size-8 ${copiedInvoice ? 'text-brand' : 'text-muted-foreground hover:text-foreground'}`}
                  title="Copy invoice"
                >
                  {copiedInvoice ? <CheckIcon className="size-3.5" /> : <CopyIcon className="size-3.5" />}
                </Button>
              </div>
              {copiedInvoice && <p className="font-mono text-[10px] text-brand mt-1.5 uppercase tracking-wider">COPIED TO CLIPBOARD</p>}
            </div>
            <div className="border border-border/50 bg-surface divide-y divide-border/50">
              <div className="flex justify-between px-3 py-2 font-mono text-[12px]">
                <span className="text-muted-foreground">Amount</span>
                <span className="text-brand tabular-nums font-bold">{(instruction as LightningBolt11Instruction).amount_sats} sats</span>
              </div>
              {(instruction as LightningBolt11Instruction).memo && (
                <div className="flex justify-between px-3 py-2 font-mono text-[12px]">
                  <span className="text-muted-foreground">Memo</span>
                  <span className="text-foreground">{(instruction as LightningBolt11Instruction).memo}</span>
                </div>
              )}
              {(instruction as LightningBolt11Instruction).expires_at && (
                <div className="flex justify-between px-3 py-2 font-mono text-[12px]">
                  <span className="text-muted-foreground">Expires</span>
                  <span className="text-foreground">{new Date((instruction as LightningBolt11Instruction).expires_at! * 1000).toLocaleString()}</span>
                </div>
              )}
            </div>
          </div>
        )}

        {instruction.type === 'redirect' && (
          <div className="space-y-4">
            <div className="panel p-4 space-y-3">
              <p className="font-mono text-[12px] text-muted-foreground">You will be redirected to an external payment page.</p>
              {instruction.amount_usd && <p className="font-mono text-[18px] font-bold text-brand tabular-nums">${instruction.amount_usd.toFixed(2)}</p>}
              <Button onClick={() => window.open(instruction.url, '_blank')} className="w-full font-mono text-[12px] uppercase tracking-wider border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">
                <ExternalLinkIcon className="size-3.5 mr-2" /> GO TO PAYMENT
              </Button>
            </div>
          </div>
        )}

        {instruction.type === 'payment_link' && (
          <div className="space-y-4">
            <div className="panel p-4 space-y-3">
              <p className="font-mono text-[12px] text-muted-foreground">Click below to open the payment page.</p>
              {instruction.amount_usd && <p className="font-mono text-[18px] font-bold text-brand tabular-nums">${instruction.amount_usd.toFixed(2)}</p>}
              <Button onClick={() => window.open(instruction.url, '_blank')} className="w-full font-mono text-[12px] uppercase tracking-wider border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">
                <ExternalLinkIcon className="size-3.5 mr-2" /> {instruction.label || 'PAY NOW'}
              </Button>
            </div>
          </div>
        )}

        {instruction.type === 'manual' && (
          <div className="space-y-4">
            {instruction.amount_usd && (
              <p className="font-mono text-[22px] font-bold text-brand tabular-nums">${instruction.amount_usd.toFixed(2)}</p>
            )}
            {instruction.reference_code && (
              <div>
                <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground mb-1.5">Reference</div>
                <div className="font-mono text-[13px] bg-surface border border-border p-2.5 tabular-nums">{instruction.reference_code}</div>
              </div>
            )}
            <div>
              <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground mb-1.5">Instructions</div>
              <div className="font-mono text-[12px] text-foreground whitespace-pre-wrap leading-relaxed">{instruction.instructions}</div>
            </div>
          </div>
        )}

        {/* Actions */}
        <div className="flex gap-2 pt-1">
          <Button variant="outline" onClick={() => onOpenChange(false)} className="font-mono text-[12px] border-border text-muted-foreground w-1/2">
            CLOSE
          </Button>
          <Button onClick={() => { window.location.reload() }} className="font-mono text-[12px] uppercase tracking-wider border border-border bg-transparent text-foreground w-1/2">
            I'VE PAID
          </Button>
        </div>
      </div>
    )
  }

  /* ═══════════════════════════════════════════════════════════ */
  /*  RENDER                                                      */
  /* ═══════════════════════════════════════════════════════════ */
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md border-border bg-card">
        <DialogHeader className="space-y-3">
          <DialogTitle className="font-display text-xl tracking-[0.04em] flex items-center gap-2">
            <WalletIcon className="size-4 text-brand" />
            TOP-UP
          </DialogTitle>
          <DialogDescription className="font-mono text-[12px] text-muted-foreground">
            {step === 'amount' ? (
              <>Add funds to <span className="text-foreground">{providerName}</span></>
            ) : (
              <>Complete payment for <span className="text-foreground">{providerName}</span></>
            )}
            {currentBalance && (
              <span className="ml-2 font-mono text-[11px]">
                · Balance: <span className="text-brand">
                  {formatBalance(currentBalance.amount, currentBalance.currency)}
                </span>
              </span>
            )}
          </DialogDescription>
        </DialogHeader>

        {/* Error */}
        {error && (
          <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono text-[12px]">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {/* Success flash */}
        {success && step === 'amount' && (
          <Alert className="border-brand/20 bg-brand/5 text-brand font-mono text-[12px]">
            <AlertDescription><CheckIcon className="size-3 inline mr-1" />{success}</AlertDescription>
          </Alert>
        )}

        {/* Step content */}
        {step === 'amount' ? renderAmountStep() : renderInvoiceStep()}
      </DialogContent>
    </Dialog>
  )
}
