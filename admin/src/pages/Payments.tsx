import { useEffect, useState } from 'react'
import { api } from '../api/client'
import type { UserBalanceEntry, BalanceTransaction, LightningInvoice, ModelPricingEntry, ModelPricingCreateRequest } from '../types'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { PlusIcon, TrashIcon, PencilIcon, CoinsIcon, ReceiptIcon, ZapIcon, DollarSignIcon } from 'lucide-react'

/* ── Tab types ─────────────────────────────────────────────────── */

type TabId = 'balances' | 'pricing' | 'transactions' | 'invoices'
const tabs: { id: TabId; label: string; icon: React.ReactNode }[] = [
  { id: 'balances', label: 'Balances', icon: <CoinsIcon className="size-3.5" /> },
  { id: 'pricing', label: 'Pricing', icon: <DollarSignIcon className="size-3.5" /> },
  { id: 'transactions', label: 'Transactions', icon: <ReceiptIcon className="size-3.5" /> },
  { id: 'invoices', label: 'Invoices', icon: <ZapIcon className="size-3.5" /> },
]

/* ── Helpers ──────────────────────────────────────────────────── */

function satsFromMsat(msat: number): string {
  return (msat / 1000).toLocaleString(undefined, { minimumFractionDigits: 3, maximumFractionDigits: 3 })
}
function formatMsat(msat: number): string {
  if (Math.abs(msat) >= 100_000_000) return `${(msat / 100_000_000).toFixed(2)}M sats`
  if (Math.abs(msat) >= 100_000) return `${(msat / 100_000).toFixed(1)}k sats`
  return `${(msat / 1000).toFixed(3)} sats`
}
function formatDate(s: string): string { try { return new Date(s).toLocaleString() } catch { return s } }

function txTypeBadge(type: string) {
  const v: Record<string, { c: string; t: string; b: string; l: string }> = {
    deposit: { c: 'var(--brand)', t: 'var(--brand)', b: 'var(--brand)', l: 'DEPOSIT' },
    refund: { c: 'var(--warning)', t: 'var(--warning)', b: 'var(--warning)', l: 'REFUND' },
    refund_reversal: { c: 'var(--muted-foreground)', t: 'var(--muted-foreground)', b: 'var(--border)', l: 'REVERSAL' },
    reserve: { c: 'var(--muted-foreground)', t: 'var(--muted-foreground)', b: 'var(--border)', l: 'RESERVE' },
    charge: { c: 'var(--destructive)', t: 'var(--destructive)', b: 'var(--destructive)', l: 'CHARGE' },
    admin_credit: { c: 'var(--brand)', t: 'var(--brand)', b: 'var(--brand)', l: 'CREDIT' },
    admin_debit: { c: 'var(--destructive)', t: 'var(--destructive)', b: 'var(--destructive)', l: 'DEBIT' },
  }
  const m = v[type] ?? { c: 'var(--muted-foreground)', t: 'var(--muted-foreground)', b: 'var(--border)', l: type.toUpperCase() }
  return <Badge className="font-mono text-[9px] tracking-wider uppercase px-1.5 py-0" style={{ background: `${m.c}10`, color: m.t, borderColor: `${m.b}30` }}>{m.l}</Badge>
}

function invoiceStatusBadge(status: string) {
  switch (status) {
    case 'pending': return <Badge className="font-mono text-[10px] uppercase tracking-wider bg-warning/15 text-warning border-warning/30">PENDING</Badge>
    case 'paid': return <Badge className="font-mono text-[10px] uppercase tracking-wider bg-brand/15 text-brand border-brand/30">PAID</Badge>
    case 'expired': return <Badge className="font-mono text-[10px] uppercase tracking-wider bg-secondary text-muted-foreground border-border">EXPIRED</Badge>
    case 'cancelled': return <Badge className="font-mono text-[10px] uppercase tracking-wider bg-secondary text-muted-foreground border-border">CANCELLED</Badge>
    default: return <Badge className="font-mono text-[10px] uppercase tracking-wider bg-secondary text-muted-foreground border-border">{status.toUpperCase()}</Badge>
  }
}

/* ═══════════════════════════════════════════════════════════════ */
/*  Payments Page                                                 */
/* ═══════════════════════════════════════════════════════════════ */

export default function Payments() {
  const [activeTab, setActiveTab] = useState<TabId>('balances')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [successMessage, setSuccessMessage] = useState<string | null>(null)

  const [balances, setBalances] = useState<UserBalanceEntry[]>([])
  const [transactions, setTransactions] = useState<BalanceTransaction[]>([])
  const [invoices, setInvoices] = useState<LightningInvoice[]>([])
  const [modelPricings, setModelPricings] = useState<ModelPricingEntry[]>([])

  /* Credit/Debit */
  const [creditDialog, setCreditDialog] = useState(false)
  const [debitDialog, setDebitDialog] = useState(false)
  const [adjustForm, setAdjustForm] = useState({ user_id: 0, amount_sats: 0, reason: '' })
  const [adjustSaving, setAdjustSaving] = useState(false)

  /* Model Pricing */
  const [pricingDialog, setPricingDialog] = useState(false)
  const [editingModel, setEditingModel] = useState<string | null>(null)
  const [pricingForm, setPricingForm] = useState<ModelPricingCreateRequest>({
    model_name: '', is_advertised: true, is_free: false,
    price_per_1m_input_sats: null, price_per_1m_output_sats: null,
    price_per_request_sats: null, context_window: null, max_output_tokens: null,
  })
  const [pricingSaving, setPricingSaving] = useState(false)
  const [deletePricingTarget, setDeletePricingTarget] = useState<string | null>(null)

  useEffect(() => { loadAll() }, [])

  async function loadAll() {
    setLoading(true); setError(null)
    try {
      const [b, t, i, m] = await Promise.allSettled([api.getAllBalances(), api.getAllTransactions(), api.getAllInvoices(), api.getModelPricing()])
      if (b.status === 'fulfilled') setBalances(b.value)
      if (t.status === 'fulfilled') setTransactions(t.value)
      if (i.status === 'fulfilled') setInvoices(i.value)
      if (m.status === 'fulfilled') setModelPricings(m.value)
    } catch (e) { setError(e instanceof Error ? e.message : 'Failed to load payment data') }
    finally { setLoading(false) }
  }

  async function handleCredit() {
    setAdjustSaving(true)
    try { await api.adminCredit({ user_id: adjustForm.user_id, amount_sats: adjustForm.amount_sats, reason: adjustForm.reason || undefined }); setCreditDialog(false); setSuccessMessage('CREDITED'); loadAll() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to credit') }
    finally { setAdjustSaving(false) }
  }

  async function handleDebit() {
    setAdjustSaving(true)
    try { await api.adminDebit({ user_id: adjustForm.user_id, amount_sats: adjustForm.amount_sats, reason: adjustForm.reason || undefined }); setDebitDialog(false); setSuccessMessage('DEBITED'); loadAll() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to debit') }
    finally { setAdjustSaving(false) }
  }

  function openCreatePricing() {
    setPricingForm({ model_name: '', is_advertised: true, is_free: false, price_per_1m_input_sats: null, price_per_1m_output_sats: null, price_per_request_sats: null, context_window: null, max_output_tokens: null })
    setEditingModel(null); setPricingDialog(true)
  }

  function openEditPricing(entry: ModelPricingEntry) {
    setPricingForm({ model_name: entry.model_name, is_advertised: entry.is_advertised, is_free: entry.is_free, price_per_1m_input_sats: entry.price_per_1m_input_sats, price_per_1m_output_sats: entry.price_per_1m_output_sats, price_per_request_sats: entry.price_per_request_sats, context_window: entry.context_window, max_output_tokens: entry.max_output_tokens })
    setEditingModel(entry.model_name); setPricingDialog(true)
  }

  async function handlePricingSave(e: React.FormEvent) {
    e.preventDefault(); setPricingSaving(true)
    try {
      if (editingModel) {
        await api.updateModelPricing(editingModel, { is_advertised: pricingForm.is_advertised, is_free: pricingForm.is_free, price_per_1m_input_sats: pricingForm.price_per_1m_input_sats, price_per_1m_output_sats: pricingForm.price_per_1m_output_sats, price_per_request_sats: pricingForm.price_per_request_sats, context_window: pricingForm.context_window, max_output_tokens: pricingForm.max_output_tokens })
        setSuccessMessage('PRICING UPDATED')
      } else {
        await api.createModelPricing({ model_name: pricingForm.model_name, is_advertised: pricingForm.is_advertised, is_free: pricingForm.is_free, price_per_1m_input_sats: pricingForm.price_per_1m_input_sats ?? 5, price_per_1m_output_sats: pricingForm.price_per_1m_output_sats ?? 15, price_per_request_sats: pricingForm.price_per_request_sats ?? 1, context_window: pricingForm.context_window ?? 8192, max_output_tokens: pricingForm.max_output_tokens ?? 4096 })
        setSuccessMessage('PRICING CREATED')
      }
      setPricingDialog(false); loadAll()
    } catch (e) { setError(e instanceof Error ? e.message : 'Failed to save pricing') }
    finally { setPricingSaving(false) }
  }

  async function handleDeletePricing() {
    if (!deletePricingTarget) return
    try { await api.deleteModelPricing(deletePricingTarget); setDeletePricingTarget(null); setSuccessMessage('PRICING DELETED'); loadAll() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to delete') }
  }

  if (loading) {
    return (
      <div className="space-y-6 p-6">
        <div><Skeleton className="h-8 w-32 bg-secondary" /><Skeleton className="h-4 w-48 bg-secondary mt-1" /></div>
        <div className="flex gap-1"><Skeleton className="h-8 w-20 bg-secondary" /><Skeleton className="h-8 w-20 bg-secondary" /></div>
        <Skeleton className="h-64 bg-secondary" />
      </div>
    )
  }

  return (
    <div className="space-y-6 p-6">
      {/* Header */}
      <div>
        <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground leading-none mb-1">PAYMENTS</h1>
        <p className="font-mono text-[13px] text-muted-foreground">Manage balances, pricing, transactions &amp; Lightning invoices</p>
      </div>

      {/* Messages */}
      {successMessage && (
        <Alert className="border-brand/30 bg-brand/5 text-brand font-mono text-[13px]">
          <AlertDescription className="flex items-center justify-between">{successMessage}<Button variant="ghost" size="icon-xs" onClick={() => setSuccessMessage(null)} className="text-brand">×</Button></AlertDescription>
        </Alert>
      )}
      {error && (
        <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono text-[13px]">
          <AlertDescription className="flex items-center justify-between">{error}<Button variant="ghost" size="icon-xs" onClick={() => setError(null)} className="text-destructive">×</Button></AlertDescription>
        </Alert>
      )}

      {/* Tabs */}
      <div className="flex gap-0 border-b border-border/50">
        {tabs.map(tab => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`inline-flex items-center gap-1.5 px-4 py-2.5 font-mono text-[12px] tracking-wider uppercase border-b-[1.5px] -mb-[1px] transition-colors ${
              activeTab === tab.id ? 'border-brand text-brand' : 'border-transparent text-muted-foreground hover:text-foreground/80'
            }`}
          >
            {tab.icon}
            {tab.label}
          </button>
        ))}
      </div>

      {/* ══ Balances ═══════════════════════════════════════════════ */}
      {activeTab === 'balances' && (
        <div className="space-y-4">
          <div className="flex items-center gap-2">
            <Button size="sm" onClick={() => { setAdjustForm({ user_id: 0, amount_sats: 0, reason: '' }); setCreditDialog(true) }} className="font-mono text-[11px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20 h-8">
              <PlusIcon className="size-3" /> Credit
            </Button>
            <Button size="sm" variant="outline" onClick={() => { setAdjustForm({ user_id: 0, amount_sats: 0, reason: '' }); setDebitDialog(true) }} className="font-mono text-[11px] tracking-wider uppercase border-border text-muted-foreground h-8">
              <PlusIcon className="size-3" /> Debit
            </Button>
          </div>

          <div className="panel">
            <div className="overflow-x-auto">
              <table className="w-full table-scan">
                <thead>
                  <tr className="border-b border-border/50 text-left">
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">User</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Balance</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Lifetime Deposited</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Updated</th>
                  </tr>
                </thead>
                <tbody>
                  {balances.length === 0 ? (
                    <tr><td colSpan={4} className="px-4 py-16 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO BALANCES</td></tr>
                  ) : balances.map(b => (
                    <tr key={b.id} className="border-b border-border/50 hover:bg-surface">
                      <td className="px-4 py-3">
                        <span className="font-mono text-[13px] font-medium">{b.username}</span>
                        <span className="ml-2 font-mono text-[11px] text-muted-foreground">ID:{b.user_id}</span>
                      </td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{satsFromMsat(b.balance_msat)}</td>
                      <td className="px-4 py-3 font-mono text-[12px] tabular-nums text-right text-muted-foreground">{formatMsat(b.lifetime_deposited_msat)}</td>
                      <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{formatDate(b.updated_at)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      )}

      {/* ══ Model Pricing ══════════════════════════════════════════ */}
      {activeTab === 'pricing' && (
        <div className="space-y-4">
          <Button size="sm" onClick={openCreatePricing} className="font-mono text-[11px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20 h-8">
            <PlusIcon className="size-3" /> Add Pricing
          </Button>
          <div className="panel">
            <div className="overflow-x-auto">
              <table className="w-full table-scan">
                <thead>
                  <tr className="border-b border-border/50 text-left">
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Model</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">In/1M</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Out/1M</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Per Req</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Context</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Max Out</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Flags</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {modelPricings.length === 0 ? (
                    <tr><td colSpan={8} className="px-4 py-16 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO CUSTOM PRICING</td></tr>
                  ) : modelPricings.map(mp => (
                    <tr key={mp.id} className="border-b border-border/50 hover:bg-surface">
                      <td className="px-4 py-3 font-mono text-[13px] font-medium">{mp.model_name}</td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{mp.price_per_1m_input_sats ?? '—'} sats</td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{mp.price_per_1m_output_sats ?? '—'} sats</td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{mp.price_per_request_sats ?? '—'} sats</td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{mp.context_window?.toLocaleString() ?? '—'}</td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{mp.max_output_tokens?.toLocaleString() ?? '—'}</td>
                      <td className="px-4 py-3">
                        <div className="flex gap-1">
                          {mp.is_free && <Badge className="font-mono text-[9px] uppercase bg-brand/15 text-brand border-brand/30">FREE</Badge>}
                          {!mp.is_advertised && <Badge className="font-mono text-[9px] uppercase bg-secondary text-muted-foreground border-border">HIDDEN</Badge>}
                        </div>
                      </td>
                      <td className="px-4 py-3">
                        <div className="flex items-center justify-end gap-1">
                          <Button variant="ghost" size="icon-xs" onClick={() => openEditPricing(mp)} className="text-muted-foreground hover:text-foreground"><PencilIcon className="size-3" /></Button>
                          <Button variant="ghost" size="icon-xs" onClick={() => setDeletePricingTarget(mp.model_name)} className="text-muted-foreground hover:text-destructive"><TrashIcon className="size-3" /></Button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      )}

      {/* ══ Transactions ═══════════════════════════════════════════ */}
      {activeTab === 'transactions' && (
        <div className="panel">
          <div className="overflow-x-auto">
            <table className="w-full table-scan">
              <thead>
                <tr className="border-b border-border/50 text-left">
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Time</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">User</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Type</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Amount</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Reference</th>
                </tr>
              </thead>
              <tbody>
                {transactions.length === 0 ? (
                  <tr><td colSpan={5} className="px-4 py-16 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO TRANSACTIONS</td></tr>
                ) : transactions.map(tx => (
                  <tr key={tx.id} className="border-b border-border/50 hover:bg-surface">
                    <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{formatDate(tx.created_at)}</td>
                    <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">ID:{tx.user_id}</td>
                    <td className="px-4 py-3">{txTypeBadge(tx.transaction_type)}</td>
                    <td className={`px-4 py-3 font-mono text-[13px] tabular-nums text-right ${tx.amount_msat >= 0 ? 'text-brand' : 'text-destructive'}`}>
                      {tx.amount_msat >= 0 ? '+' : ''}{formatMsat(tx.amount_msat)}
                    </td>
                    <td className="px-4 py-3 font-mono text-[11px] text-muted-foreground truncate max-w-40" title={tx.reference_id || ''}>{tx.reference_id || '—'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* ══ Invoices ══════════════════════════════════════════════ */}
      {activeTab === 'invoices' && (
        <div className="panel">
          <div className="overflow-x-auto">
            <table className="w-full table-scan">
              <thead>
                <tr className="border-b border-border/50 text-left">
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">User</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Amount</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Status</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Created</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Expires</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Paid At</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Hash</th>
                </tr>
              </thead>
              <tbody>
                {invoices.length === 0 ? (
                  <tr><td colSpan={7} className="px-4 py-16 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO INVOICES</td></tr>
                ) : invoices.map(inv => (
                  <tr key={inv.id} className="border-b border-border/50 hover:bg-surface">
                    <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">ID:{inv.user_id}</td>
                    <td className="px-4 py-3 font-mono text-[13px] tabular-nums">{inv.amount_sats.toLocaleString()} sats</td>
                    <td className="px-4 py-3">{invoiceStatusBadge(inv.status)}</td>
                    <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{formatDate(inv.created_at)}</td>
                    <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{inv.expires_at ? formatDate(inv.expires_at) : '—'}</td>
                    <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{inv.paid_at ? formatDate(inv.paid_at) : '—'}</td>
                    <td className="px-4 py-3 font-mono text-[11px] text-muted-foreground truncate max-w-24" title={inv.payment_hash}>{inv.payment_hash.slice(0, 12)}…</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* ══ Credit Dialog ═══════════════════════════════════════════ */}
      <Dialog open={creditDialog} onOpenChange={setCreditDialog}>
        <DialogContent className="border-border bg-card">
          <DialogHeader>
            <DialogTitle className="font-display text-xl tracking-[0.04em] text-brand">CREDIT BALANCE</DialogTitle>
            <DialogDescription className="font-mono text-[12px] text-muted-foreground">Manually add funds to a user&apos;s balance.</DialogDescription>
          </DialogHeader>
          <form onSubmit={e => { e.preventDefault(); handleCredit() }} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5"><Label htmlFor="cr-user" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">User ID</Label><Input id="cr-user" type="number" min={1} value={adjustForm.user_id || ''} onChange={e => setAdjustForm({ ...adjustForm, user_id: Number(e.target.value) })} required className="font-mono bg-surface border-border text-foreground" /></div>
            <div className="flex flex-col gap-1.5"><Label htmlFor="cr-amount" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Amount (sats)</Label><Input id="cr-amount" type="number" min={1} value={adjustForm.amount_sats || ''} onChange={e => setAdjustForm({ ...adjustForm, amount_sats: Number(e.target.value) })} required className="font-mono bg-surface border-border text-foreground" /></div>
            <div className="flex flex-col gap-1.5"><Label htmlFor="cr-reason" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Reason</Label><Input id="cr-reason" value={adjustForm.reason} onChange={e => setAdjustForm({ ...adjustForm, reason: e.target.value })} placeholder="Optional" className="font-mono bg-surface border-border text-foreground" /></div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setCreditDialog(false)} disabled={adjustSaving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
              <Button type="submit" disabled={adjustSaving} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand">{adjustSaving ? '...' : 'CREDIT'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ══ Debit Dialog ════════════════════════════════════════════ */}
      <Dialog open={debitDialog} onOpenChange={setDebitDialog}>
        <DialogContent className="border-border bg-card">
          <DialogHeader>
            <DialogTitle className="font-display text-xl tracking-[0.04em] text-destructive">DEBIT BALANCE</DialogTitle>
            <DialogDescription className="font-mono text-[12px] text-muted-foreground">Manually deduct funds from a user&apos;s balance.</DialogDescription>
          </DialogHeader>
          <form onSubmit={e => { e.preventDefault(); handleDebit() }} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5"><Label htmlFor="db-user" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">User ID</Label><Input id="db-user" type="number" min={1} value={adjustForm.user_id || ''} onChange={e => setAdjustForm({ ...adjustForm, user_id: Number(e.target.value) })} required className="font-mono bg-surface border-border text-foreground" /></div>
            <div className="flex flex-col gap-1.5"><Label htmlFor="db-amount" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Amount (sats)</Label><Input id="db-amount" type="number" min={1} value={adjustForm.amount_sats || ''} onChange={e => setAdjustForm({ ...adjustForm, amount_sats: Number(e.target.value) })} required className="font-mono bg-surface border-border text-foreground" /></div>
            <div className="flex flex-col gap-1.5"><Label htmlFor="db-reason" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Reason</Label><Input id="db-reason" value={adjustForm.reason} onChange={e => setAdjustForm({ ...adjustForm, reason: e.target.value })} placeholder="Optional" className="font-mono bg-surface border-border text-foreground" /></div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setDebitDialog(false)} disabled={adjustSaving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
              <Button type="submit" disabled={adjustSaving} className="font-mono text-[12px] tracking-wider uppercase border border-destructive/40 bg-destructive/10 text-destructive">{adjustSaving ? '...' : 'DEBIT'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ══ Pricing Dialog ══════════════════════════════════════════ */}
      <Dialog open={pricingDialog} onOpenChange={setPricingDialog}>
        <DialogContent className="sm:max-w-lg border-border bg-card">
          <DialogHeader>
            <DialogTitle className="font-display text-xl tracking-[0.04em]">{editingModel ? 'EDIT PRICING' : 'ADD PRICING'}</DialogTitle>
            <DialogDescription className="font-mono text-[12px] text-muted-foreground">{editingModel ? 'Update pricing for this model.' : 'Define custom pricing for a model.'}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handlePricingSave} className="flex flex-col gap-4">
            {!editingModel && (
              <div className="flex flex-col gap-1.5"><Label htmlFor="mp-name" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Model Name</Label><Input id="mp-name" value={pricingForm.model_name} onChange={e => setPricingForm({ ...pricingForm, model_name: e.target.value })} placeholder="gpt-4" required className="font-mono bg-surface border-border text-foreground" /></div>
            )}
            <div className="flex items-center gap-4">
              <div className="flex items-center gap-2"><Checkbox id="mp-free" checked={pricingForm.is_free} onCheckedChange={c => setPricingForm({ ...pricingForm, is_free: !!c })} /><Label htmlFor="mp-free" className="font-mono text-[12px]">FREE</Label></div>
              <div className="flex items-center gap-2"><Checkbox id="mp-advertised" checked={pricingForm.is_advertised} onCheckedChange={c => setPricingForm({ ...pricingForm, is_advertised: !!c })} /><Label htmlFor="mp-advertised" className="font-mono text-[12px]">ADVERTISED</Label></div>
            </div>
            {!pricingForm.is_free && (
              <>
                <div className="grid grid-cols-2 gap-4">
                  <div className="flex flex-col gap-1.5"><Label htmlFor="mp-input" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">In /1M (sats)</Label><Input id="mp-input" type="number" min={0} value={pricingForm.price_per_1m_input_sats ?? ''} onChange={e => setPricingForm({ ...pricingForm, price_per_1m_input_sats: e.target.value ? Number(e.target.value) : null })} className="font-mono bg-surface border-border text-foreground" /></div>
                  <div className="flex flex-col gap-1.5"><Label htmlFor="mp-output" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Out /1M (sats)</Label><Input id="mp-output" type="number" min={0} value={pricingForm.price_per_1m_output_sats ?? ''} onChange={e => setPricingForm({ ...pricingForm, price_per_1m_output_sats: e.target.value ? Number(e.target.value) : null })} className="font-mono bg-surface border-border text-foreground" /></div>
                </div>
                <div className="flex flex-col gap-1.5"><Label htmlFor="mp-per-req" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Per Request (sats)</Label><Input id="mp-per-req" type="number" min={0} value={pricingForm.price_per_request_sats ?? ''} onChange={e => setPricingForm({ ...pricingForm, price_per_request_sats: e.target.value ? Number(e.target.value) : null })} className="font-mono bg-surface border-border text-foreground" /></div>
                <div className="grid grid-cols-2 gap-4">
                  <div className="flex flex-col gap-1.5"><Label htmlFor="mp-ctx" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Context Window</Label><Input id="mp-ctx" type="number" min={1} value={pricingForm.context_window ?? ''} onChange={e => setPricingForm({ ...pricingForm, context_window: e.target.value ? Number(e.target.value) : null })} className="font-mono bg-surface border-border text-foreground" /></div>
                  <div className="flex flex-col gap-1.5"><Label htmlFor="mp-max-tok" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Max Output Tokens</Label><Input id="mp-max-tok" type="number" min={1} value={pricingForm.max_output_tokens ?? ''} onChange={e => setPricingForm({ ...pricingForm, max_output_tokens: e.target.value ? Number(e.target.value) : null })} className="font-mono bg-surface border-border text-foreground" /></div>
                </div>
              </>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setPricingDialog(false)} disabled={pricingSaving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
              <Button type="submit" disabled={pricingSaving} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand">{pricingSaving ? '...' : editingModel ? 'UPDATE' : 'CREATE'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ══ Delete Pricing ══════════════════════════════════════════ */}
      <AlertDialog open={!!deletePricingTarget} onOpenChange={o => { if (!o) setDeletePricingTarget(null) }}>
        <AlertDialogContent className="border-border bg-card">
          <AlertDialogHeader>
            <AlertDialogTitle className="font-display text-xl tracking-[0.04em] text-destructive">DELETE PRICING</AlertDialogTitle>
            <AlertDialogDescription className="font-mono text-[13px] text-muted-foreground">Delete pricing for <span className="text-foreground">{deletePricingTarget}</span>?</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleDeletePricing} className="font-mono text-[12px] tracking-wider uppercase">DELETE</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
