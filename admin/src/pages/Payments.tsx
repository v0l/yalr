import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { BalanceAdjustDialog } from '@/components/BalanceAdjustDialog'
import type { BalanceAdjustKind } from '../types'
import { formatMilli, formatPrice } from '@/lib/money'
import { api } from '../api/client'
import type { UserBalanceEntry, BalanceTransaction, LightningInvoice, ModelPricingEntry, Provider } from '../types'
import { currentRole } from '@/lib/role'
import { TopupBalanceDialog } from '@/components/TopupBalanceDialog'
import ApiKeysPanel from '@/components/ApiKeysPanel'
import { Button } from '@/components/ui/button'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import PricingDialog from '@/components/pricing/PricingDialog'
import CompareDialog from '@/components/pricing/CompareDialog'
import { marketApi, type ListingStatus } from '@/api/market'
import { ScaleIcon, PlusIcon, TrashIcon, PencilIcon, CoinsIcon, ReceiptIcon, ZapIcon, DollarSignIcon } from 'lucide-react'

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
  if (Math.abs(msat) >= 1_000_000_000) return `${(msat / 1_000_000_000).toFixed(2)}M sats`
  if (Math.abs(msat) >= 1_000_000) return `${(msat / 1_000_000).toFixed(1)}k sats`
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
    usage: { c: 'var(--destructive)', t: 'var(--destructive)', b: 'var(--destructive)', l: 'USAGE' },
    provider_earning: { c: 'var(--brand)', t: 'var(--brand)', b: 'var(--brand)', l: 'EARNING' },
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
  const [providers, setProviders] = useState<Provider[]>([])
  const { isAdmin } = currentRole()

  const [adjustKind, setAdjustKind] = useState<BalanceAdjustKind | null>(null)
  const [topupOpen, setTopupOpen] = useState(false)

  const [pricingDialog, setPricingDialog] = useState(false)
  const [editingPrice, setEditingPrice] = useState<ModelPricingEntry | null>(null)
  const [compareTarget, setCompareTarget] = useState<ModelPricingEntry | null>(null)
  const [listing, setListing] = useState<ListingStatus | null>(null)
  const [deletePricingTarget, setDeletePricingTarget] = useState<ModelPricingEntry | null>(null)

  useEffect(() => {
    loadAll()
    if (isAdmin) marketApi.listing().then(setListing).catch(() => undefined)
  }, [])

  async function loadAll() {
    setLoading(true); setError(null)
    try {
      const [b, t, i, m, p] = await Promise.allSettled([api.getAllBalances(), api.getAllTransactions(), api.getAllInvoices(), api.getModelPricing(), api.getProviders()])
      if (b.status === 'fulfilled') setBalances(b.value)
      if (t.status === 'fulfilled') setTransactions(t.value)
      if (i.status === 'fulfilled') setInvoices(i.value)
      if (m.status === 'fulfilled') setModelPricings(m.value)
      if (p.status === 'fulfilled') setProviders(p.value.providers)
    } catch (e) { setError(e instanceof Error ? e.message : 'Failed to load payment data') }
    finally { setLoading(false) }
  }

  function openCreatePricing() {
    setEditingPrice(null); setPricingDialog(true)
  }

  function openEditPricing(entry: ModelPricingEntry) {
    setEditingPrice(entry); setPricingDialog(true)
  }

  async function handleDeletePricing() {
    if (!deletePricingTarget) return
    try { await api.deleteModelPricing(deletePricingTarget.id); setDeletePricingTarget(null); setSuccessMessage('PRICING DELETED'); loadAll() }
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
            <Button size="sm" onClick={() => setTopupOpen(true)} className="font-mono text-[11px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20 h-8">
              <ZapIcon className="size-3" /> Top up
            </Button>
            {isAdmin && (
              <>
                <Button size="sm" variant="outline" onClick={() => setAdjustKind('credit')} className="font-mono text-[11px] tracking-wider uppercase border-brand/40 text-brand h-8">
                  <PlusIcon className="size-3" /> Credit
                </Button>
                <Button size="sm" variant="outline" onClick={() => setAdjustKind('debit')} className="font-mono text-[11px] tracking-wider uppercase border-border text-muted-foreground h-8">
                  <PlusIcon className="size-3" /> Debit
                </Button>
              </>
            )}
          </div>

          {!isAdmin && <ApiKeysPanel onError={setError} />}

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
          <Button size="sm" onClick={openCreatePricing} disabled={!isAdmin && providers.length === 0} className="font-mono text-[11px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20 h-8">
            <PlusIcon className="size-3" /> Add Pricing
          </Button>
          <div className="panel">
            <div className="overflow-x-auto">
              <table className="w-full table-scan">
                <thead>
                  <tr className="border-b border-border/50 text-left">
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Model</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Owner / Provider</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">In/1M</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Out/1M</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Cache Rd/1M</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Cache Wr/1M</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Per Req</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Context</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Max Out</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Flags</th>
                    <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {modelPricings.length === 0 ? (
                    <tr><td colSpan={11} className="px-4 py-16 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO CUSTOM PRICING</td></tr>
                  ) : modelPricings.map(mp => (
                    <tr key={mp.id} className="border-b border-border/50 hover:bg-surface">
                      <td className="px-4 py-3 font-mono text-[13px] font-medium">{mp.model_name}</td>
                      <td className="px-4 py-3 font-mono text-[12px]">
                        {mp.owner_user_id === null ? <span className="text-muted-foreground">platform</span> : isAdmin ? <Link to={`/users/${mp.owner_user_id}`} className="text-brand hover:underline">{mp.owner_username ?? `user ${mp.owner_user_id}`}</Link> : <span>{mp.owner_username ?? 'you'}</span>}
                        {mp.provider_slug && <span className="ml-2 text-muted-foreground">{mp.provider_slug}</span>}
                      </td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{formatPrice(mp.currency === 'sats' ? mp.price_per_1m_input_sats : mp.price_per_1m_input_milli, mp.currency)}</td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{formatPrice(mp.currency === 'sats' ? mp.price_per_1m_output_sats : mp.price_per_1m_output_milli, mp.currency)}</td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{formatPrice(mp.currency === 'sats' ? (mp.price_per_1m_cache_read_sats ?? mp.price_per_1m_input_sats) : mp.price_per_1m_cache_read_milli, mp.currency)}</td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{formatPrice(mp.currency === 'sats' ? (mp.price_per_1m_cache_write_sats ?? mp.price_per_1m_input_sats) : mp.price_per_1m_cache_write_milli, mp.currency)}</td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{formatPrice(mp.currency === 'sats' ? mp.price_per_request_sats : mp.price_per_request_milli, mp.currency)}</td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{mp.context_window?.toLocaleString() ?? '—'}</td>
                      <td className="px-4 py-3 font-mono text-[13px] tabular-nums text-right">{mp.max_output_tokens?.toLocaleString() ?? '—'}</td>
                      <td className="px-4 py-3">
                        <div className="flex gap-1">
                          {mp.is_free && <Badge className="font-mono text-[9px] uppercase bg-brand/15 text-brand border-brand/30">FREE</Badge>}
                          {mp.is_advertised
                            ? <Badge className="font-mono text-[9px] uppercase bg-brand/15 text-brand border-brand/30">FOR SALE</Badge>
                            : <Badge className="font-mono text-[9px] uppercase bg-secondary text-muted-foreground border-border">NOT FOR SALE</Badge>}
                        </div>
                      </td>
                      <td className="px-4 py-3">
                        <div className="flex items-center justify-end gap-1">
                          {isAdmin && <Button variant="ghost" size="icon-xs" title="Compare with other Routstr nodes" onClick={() => setCompareTarget(mp)} className="text-muted-foreground hover:text-foreground"><ScaleIcon className="size-3" /></Button>}
                          <Button variant="ghost" size="icon-xs" onClick={() => openEditPricing(mp)} className="text-muted-foreground hover:text-foreground"><PencilIcon className="size-3" /></Button>
                          <Button variant="ghost" size="icon-xs" onClick={() => setDeletePricingTarget(mp)} className="text-muted-foreground hover:text-destructive"><TrashIcon className="size-3" /></Button>
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
                    <td className={`px-4 py-3 font-mono text-[13px] tabular-nums text-right ${(tx.currency === 'sats' ? tx.amount_msat : tx.amount_milli) >= 0 ? 'text-brand' : 'text-destructive'}`}>
                      {(tx.currency === 'sats' ? tx.amount_msat : tx.amount_milli) >= 0 ? '+' : ''}{tx.currency === 'sats' ? formatMsat(tx.amount_msat) : formatMilli(tx.amount_milli, tx.currency)}
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

      <BalanceAdjustDialog kind={adjustKind} onClose={() => setAdjustKind(null)} onDone={m => { setSuccessMessage(m); loadAll() }} onError={setError} />
      <PricingDialog
        open={pricingDialog}
        onOpenChange={setPricingDialog}
        editing={editingPrice}
        isAdmin={isAdmin}
        providers={providers}
        prices={modelPricings}
        listing={listing}
        onSaved={m => { setSuccessMessage(m); loadAll() }}
        onError={setError}
      />
      <TopupBalanceDialog open={topupOpen} onOpenChange={setTopupOpen} onPaid={() => { setSuccessMessage('TOP UP RECEIVED'); loadAll() }} />
      <CompareDialog open={!!compareTarget} onOpenChange={o => { if (!o) setCompareTarget(null) }} entry={compareTarget} />

      {/* ══ Delete Pricing ══════════════════════════════════════════ */}
      <AlertDialog open={!!deletePricingTarget} onOpenChange={o => { if (!o) setDeletePricingTarget(null) }}>
        <AlertDialogContent className="border-border bg-card">
          <AlertDialogHeader>
            <AlertDialogTitle className="font-display text-xl tracking-[0.04em] text-destructive">DELETE PRICING</AlertDialogTitle>
            <AlertDialogDescription className="font-mono text-[13px] text-muted-foreground">Delete pricing for <span className="text-foreground">{deletePricingTarget?.model_name}</span>{deletePricingTarget?.provider_slug ? ` on ${deletePricingTarget.provider_slug}` : ''}?</AlertDialogDescription>
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
