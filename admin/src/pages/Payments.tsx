import { useEffect, useState } from 'react'
import { api } from '../api/client'
import type {
  UserBalanceEntry,
  BalanceTransaction,
  LightningInvoice,
  ModelPricingEntry,
  ModelPricingCreateRequest,
} from '../types'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { Card, CardContent } from '@/components/ui/card'
import { PlusIcon, TrashIcon, PencilIcon, CoinsIcon, ReceiptIcon, ZapIcon, DollarSignIcon } from 'lucide-react'

type TabId = 'balances' | 'pricing' | 'transactions' | 'invoices'

const tabs: { id: TabId; label: string; icon: React.ReactNode }[] = [
  { id: 'balances', label: 'Balances', icon: <CoinsIcon className="size-3.5" /> },
  { id: 'pricing', label: 'Model Pricing', icon: <DollarSignIcon className="size-3.5" /> },
  { id: 'transactions', label: 'Transactions', icon: <ReceiptIcon className="size-3.5" /> },
  { id: 'invoices', label: 'Invoices', icon: <ZapIcon className="size-3.5" /> },
]

// ── Helpers ────────────────────────────────────────────────────────────────

function satsFromMsat(msat: number): string {
  return (msat / 1000).toLocaleString(undefined, { minimumFractionDigits: 3, maximumFractionDigits: 3 })
}

function formatMsat(msat: number): string {
  if (Math.abs(msat) >= 100_000_000) return `${(msat / 100_000_000).toFixed(2)}M sats`
  if (Math.abs(msat) >= 100_000) return `${(msat / 100_000).toFixed(1)}k sats`
  return `${(msat / 1000).toFixed(3)} sats`
}

function formatDate(s: string): string {
  try { return new Date(s).toLocaleString() } catch { return s }
}

function txTypeBadge(type: string) {
  const v: Record<string, { variant: 'default' | 'secondary' | 'outline' | 'destructive'; label: string }> = {
    deposit: { variant: 'default', label: 'Deposit' },
    refund: { variant: 'outline', label: 'Refund' },
    refund_reversal: { variant: 'secondary', label: 'Reversal' },
    reserve: { variant: 'secondary', label: 'Reserve' },
    charge: { variant: 'destructive', label: 'Charge' },
    admin_credit: { variant: 'default', label: 'Admin Credit' },
    admin_debit: { variant: 'destructive', label: 'Admin Debit' },
  }
  const m = v[type] ?? { variant: 'secondary' as const, label: type }
  return <Badge variant={m.variant}>{m.label}</Badge>
}

function invoiceStatusBadge(status: string) {
  switch (status) {
    case 'pending': return <Badge variant="secondary">Pending</Badge>
    case 'paid': return <Badge>Paid</Badge>
    case 'expired': return <Badge variant="outline">Expired</Badge>
    case 'cancelled': return <Badge variant="outline">Cancelled</Badge>
    default: return <Badge variant="secondary">{status}</Badge>
  }
}

export default function Payments() {
  const [activeTab, setActiveTab] = useState<TabId>('balances')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [successMessage, setSuccessMessage] = useState<string | null>(null)

  // Tab data
  const [balances, setBalances] = useState<UserBalanceEntry[]>([])
  const [transactions, setTransactions] = useState<BalanceTransaction[]>([])
  const [invoices, setInvoices] = useState<LightningInvoice[]>([])
  const [modelPricings, setModelPricings] = useState<ModelPricingEntry[]>([])

  // Credit/Debit dialog
  const [creditDialog, setCreditDialog] = useState(false)
  const [debitDialog, setDebitDialog] = useState(false)
  const [adjustForm, setAdjustForm] = useState({ user_id: 0, amount_sats: 0, reason: '' })
  const [adjustSaving, setAdjustSaving] = useState(false)

  // Model Pricing dialog
  const [pricingDialog, setPricingDialog] = useState(false)
  const [editingModel, setEditingModel] = useState<string | null>(null)
  const [pricingForm, setPricingForm] = useState<ModelPricingCreateRequest>({
    model_name: '',
    is_advertised: true,
    is_free: false,
    price_per_1m_input_sats: null,
    price_per_1m_output_sats: null,
    price_per_request_sats: null,
    context_window: null,
    max_output_tokens: null,
  })
  const [pricingSaving, setPricingSaving] = useState(false)
  const [deletePricingTarget, setDeletePricingTarget] = useState<string | null>(null)

  useEffect(() => { loadAll() }, [])

  async function loadAll() {
    setLoading(true)
    setError(null)
    try {
      const [b, t, i, m] = await Promise.allSettled([
        api.getAllBalances(),
        api.getAllTransactions(),
        api.getAllInvoices(),
        api.getModelPricing(),
      ])
      if (b.status === 'fulfilled') setBalances(b.value)
      if (t.status === 'fulfilled') setTransactions(t.value)
      if (i.status === 'fulfilled') setInvoices(i.value)
      if (m.status === 'fulfilled') setModelPricings(m.value)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load payment data')
    } finally {
      setLoading(false)
    }
  }

  // ── Credit / Debit ─────────────────────────────────────────────────────

  async function handleCredit() {
    setAdjustSaving(true)
    try {
      await api.adminCredit({ user_id: adjustForm.user_id, amount_sats: adjustForm.amount_sats, reason: adjustForm.reason || undefined })
      setCreditDialog(false)
      setSuccessMessage(`Credited ${adjustForm.amount_sats} sats to user ${adjustForm.user_id}`)
      loadAll()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to credit')
    } finally { setAdjustSaving(false) }
  }

  async function handleDebit() {
    setAdjustSaving(true)
    try {
      await api.adminDebit({ user_id: adjustForm.user_id, amount_sats: adjustForm.amount_sats, reason: adjustForm.reason || undefined })
      setDebitDialog(false)
      setSuccessMessage(`Debited ${adjustForm.amount_sats} sats from user ${adjustForm.user_id}`)
      loadAll()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to debit')
    } finally { setAdjustSaving(false) }
  }

  // ── Model Pricing CRUD ─────────────────────────────────────────────────

  function openCreatePricing() {
    setPricingForm({ model_name: '', is_advertised: true, is_free: false, price_per_1m_input_sats: null, price_per_1m_output_sats: null, price_per_request_sats: null, context_window: null, max_output_tokens: null })
    setEditingModel(null)
    setPricingDialog(true)
  }

  function openEditPricing(entry: ModelPricingEntry) {
    setPricingForm({
      model_name: entry.model_name,
      is_advertised: entry.is_advertised,
      is_free: entry.is_free,
      price_per_1m_input_sats: entry.price_per_1m_input_sats,
      price_per_1m_output_sats: entry.price_per_1m_output_sats,
      price_per_request_sats: entry.price_per_request_sats,
      context_window: entry.context_window,
      max_output_tokens: entry.max_output_tokens,
    })
    setEditingModel(entry.model_name)
    setPricingDialog(true)
  }

  async function handlePricingSave(e: React.FormEvent) {
    e.preventDefault()
    setPricingSaving(true)
    try {
      if (editingModel) {
        await api.updateModelPricing(editingModel, {
          is_advertised: pricingForm.is_advertised,
          is_free: pricingForm.is_free,
          price_per_1m_input_sats: pricingForm.price_per_1m_input_sats,
          price_per_1m_output_sats: pricingForm.price_per_1m_output_sats,
          price_per_request_sats: pricingForm.price_per_request_sats,
          context_window: pricingForm.context_window,
          max_output_tokens: pricingForm.max_output_tokens,
        })
        setSuccessMessage('Model pricing updated')
      } else {
        await api.createModelPricing({
          model_name: pricingForm.model_name,
          is_advertised: pricingForm.is_advertised,
          is_free: pricingForm.is_free,
          price_per_1m_input_sats: pricingForm.price_per_1m_input_sats ?? 5,
          price_per_1m_output_sats: pricingForm.price_per_1m_output_sats ?? 15,
          price_per_request_sats: pricingForm.price_per_request_sats ?? 1,
          context_window: pricingForm.context_window ?? 8192,
          max_output_tokens: pricingForm.max_output_tokens ?? 4096,
        })
        setSuccessMessage('Model pricing created')
      }
      setPricingDialog(false)
      loadAll()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to save model pricing')
    } finally { setPricingSaving(false) }
  }

  async function handleDeletePricing() {
    if (!deletePricingTarget) return
    try {
      await api.deleteModelPricing(deletePricingTarget)
      setDeletePricingTarget(null)
      setSuccessMessage('Model pricing deleted')
      loadAll()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to delete model pricing')
    }
  }

  // ── Render ──────────────────────────────────────────────────────────────

  if (loading) {
    return (
      <div className="flex flex-col gap-6 p-6">
        <div className="flex flex-col gap-1"><Skeleton className="h-7 w-32" /><Skeleton className="h-4 w-48" /></div>
        <div className="flex gap-2"><Skeleton className="h-8 w-20" /><Skeleton className="h-8 w-20" /><Skeleton className="h-8 w-20" /></div>
        <Skeleton className="h-64 w-full" />
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-6 p-6">
      {/* Header */}
      <div>
        <h1 className="text-2xl font-bold text-foreground">Payments</h1>
        <p className="text-sm text-muted-foreground">Manage user balances, model pricing, transactions, and Lightning invoices</p>
      </div>

      {/* Messages */}
      {successMessage && (
        <Alert><AlertDescription className="flex items-center justify-between">{successMessage}<Button variant="ghost" size="icon-xs" onClick={() => setSuccessMessage(null)}>×</Button></AlertDescription></Alert>
      )}
      {error && (
        <Alert variant="destructive"><AlertDescription className="flex items-center justify-between">{error}<Button variant="ghost" size="icon-xs" onClick={() => setError(null)}>×</Button></AlertDescription></Alert>
      )}

      {/* Tabs */}
      <div className="flex gap-1 border-b border-border pb-0">
        {tabs.map(tab => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`inline-flex items-center gap-1.5 px-3 py-2 text-sm font-medium border-b-2 transition-colors -mb-[1px] ${
              activeTab === tab.id
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            }`}
          >
            {tab.icon}
            {tab.label}
          </button>
        ))}
      </div>

      {/* ── Balances Tab ──────────────────────────────────────────────── */}
      {activeTab === 'balances' && (
        <div className="flex flex-col gap-4">
          <div className="flex items-center gap-2">
            <Button size="sm" onClick={() => { setAdjustForm({ user_id: 0, amount_sats: 0, reason: '' }); setCreditDialog(true) }}><PlusIcon /> Credit User</Button>
            <Button size="sm" variant="outline" onClick={() => { setAdjustForm({ user_id: 0, amount_sats: 0, reason: '' }); setDebitDialog(true) }}><PlusIcon /> Debit User</Button>
          </div>

          <Card>
            <CardContent className="p-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>User</TableHead>
                    <TableHead className="text-right">Balance (sats)</TableHead>
                    <TableHead className="text-right">Lifetime Deposited</TableHead>
                    <TableHead>Updated</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {balances.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={4} className="text-center text-muted-foreground py-12">No balances found</TableCell>
                    </TableRow>
                  ) : (
                    balances.map(b => (
                      <TableRow key={b.id}>
                        <TableCell>
                          <span className="font-medium">{b.username}</span>
                          <span className="ml-2 font-mono text-xs text-muted-foreground">ID:{b.user_id}</span>
                        </TableCell>
                        <TableCell className="text-right font-mono">{satsFromMsat(b.balance_msat)}</TableCell>
                        <TableCell className="text-right font-mono text-muted-foreground">{formatMsat(b.lifetime_deposited_msat)}</TableCell>
                        <TableCell className="text-sm text-muted-foreground">{formatDate(b.updated_at)}</TableCell>
                      </TableRow>
                    ))
                  )}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </div>
      )}

      {/* ── Model Pricing Tab ──────────────────────────────────────────── */}
      {activeTab === 'pricing' && (
        <div className="flex flex-col gap-4">
          <div className="flex items-center">
            <Button size="sm" onClick={openCreatePricing}><PlusIcon /> Add Model Pricing</Button>
          </div>
          <Card>
            <CardContent className="p-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Model</TableHead>
                    <TableHead className="text-right">Input /1M</TableHead>
                    <TableHead className="text-right">Output /1M</TableHead>
                    <TableHead className="text-right">Per Request</TableHead>
                    <TableHead className="text-right">Context</TableHead>
                    <TableHead className="text-right">Max Output</TableHead>
                    <TableHead>Flags</TableHead>
                    <TableHead className="w-20 text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {modelPricings.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={8} className="text-center text-muted-foreground py-12">No custom model pricing</TableCell>
                    </TableRow>
                  ) : (
                    modelPricings.map(mp => (
                      <TableRow key={mp.id}>
                        <TableCell className="font-medium font-mono">{mp.model_name}</TableCell>
                        <TableCell className="text-right font-mono text-sm">{mp.price_per_1m_input_sats ?? '—'} sats</TableCell>
                        <TableCell className="text-right font-mono text-sm">{mp.price_per_1m_output_sats ?? '—'} sats</TableCell>
                        <TableCell className="text-right font-mono text-sm">{mp.price_per_request_sats ?? '—'} sats</TableCell>
                        <TableCell className="text-right font-mono text-sm">{mp.context_window?.toLocaleString() ?? '—'}</TableCell>
                        <TableCell className="text-right font-mono text-sm">{mp.max_output_tokens?.toLocaleString() ?? '—'}</TableCell>
                        <TableCell>
                          <div className="flex gap-1">
                            {mp.is_free && <Badge variant="secondary">Free</Badge>}
                            {!mp.is_advertised && <Badge variant="outline">Hidden</Badge>}
                          </div>
                        </TableCell>
                        <TableCell className="text-right">
                          <div className="flex items-center justify-end gap-1">
                            <Button variant="ghost" size="icon-xs" onClick={() => openEditPricing(mp)}><PencilIcon /></Button>
                            <Button variant="ghost" size="icon-xs" onClick={() => setDeletePricingTarget(mp.model_name)}><TrashIcon /></Button>
                          </div>
                        </TableCell>
                      </TableRow>
                    ))
                  )}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </div>
      )}

      {/* ── Transactions Tab ───────────────────────────────────────────── */}
      {activeTab === 'transactions' && (
        <Card>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Time</TableHead>
                  <TableHead>User</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead className="text-right">Amount</TableHead>
                  <TableHead>Reference</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {transactions.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={5} className="text-center text-muted-foreground py-12">No transactions</TableCell>
                  </TableRow>
                ) : (
                  transactions.map(tx => (
                    <TableRow key={tx.id}>
                      <TableCell className="text-sm text-muted-foreground">{formatDate(tx.created_at)}</TableCell>
                      <TableCell className="font-mono text-xs">ID:{tx.user_id}</TableCell>
                      <TableCell>{txTypeBadge(tx.transaction_type)}</TableCell>
                      <TableCell className={`text-right font-mono text-sm ${tx.amount_msat >= 0 ? 'text-emerald-600 dark:text-emerald-400' : 'text-destructive'}`}>
                        {tx.amount_msat >= 0 ? '+' : ''}{formatMsat(tx.amount_msat)}
                      </TableCell>
                      <TableCell className="font-mono text-xs text-muted-foreground truncate max-w-40" title={tx.reference_id || ''}>{tx.reference_id || '—'}</TableCell>
                    </TableRow>
                  ))
                )}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      {/* ── Invoices Tab ───────────────────────────────────────────────── */}
      {activeTab === 'invoices' && (
        <Card>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>User</TableHead>
                  <TableHead>Amount</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Created</TableHead>
                  <TableHead>Expires</TableHead>
                  <TableHead>Paid At</TableHead>
                  <TableHead className="font-mono text-xs">Payment Hash</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {invoices.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-center text-muted-foreground py-12">No invoices</TableCell>
                  </TableRow>
                ) : (
                  invoices.map(inv => (
                    <TableRow key={inv.id}>
                      <TableCell className="font-mono text-xs">ID:{inv.user_id}</TableCell>
                      <TableCell className="font-mono text-sm">{inv.amount_sats.toLocaleString()} sats</TableCell>
                      <TableCell>{invoiceStatusBadge(inv.status)}</TableCell>
                      <TableCell className="text-sm text-muted-foreground">{formatDate(inv.created_at)}</TableCell>
                      <TableCell className="text-sm text-muted-foreground">{inv.expires_at ? formatDate(inv.expires_at) : '—'}</TableCell>
                      <TableCell className="text-sm text-muted-foreground">{inv.paid_at ? formatDate(inv.paid_at) : '—'}</TableCell>
                      <TableCell className="font-mono text-xs text-muted-foreground truncate max-w-32" title={inv.payment_hash}>{inv.payment_hash.slice(0, 12)}…</TableCell>
                    </TableRow>
                  ))
                )}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      {/* ── Credit Dialog ───────────────────────────────────────────────── */}
      <Dialog open={creditDialog} onOpenChange={setCreditDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Credit User Balance</DialogTitle>
            <DialogDescription>Manually add funds to a user&apos;s balance.</DialogDescription>
          </DialogHeader>
          <form onSubmit={(e) => { e.preventDefault(); handleCredit() }} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="cr-user">User ID</Label>
              <Input id="cr-user" type="number" min={1} value={adjustForm.user_id || ''} onChange={e => setAdjustForm({ ...adjustForm, user_id: Number(e.target.value) })} required />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="cr-amount">Amount (sats)</Label>
              <Input id="cr-amount" type="number" min={1} value={adjustForm.amount_sats || ''} onChange={e => setAdjustForm({ ...adjustForm, amount_sats: Number(e.target.value) })} required />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="cr-reason">Reason</Label>
              <Input id="cr-reason" value={adjustForm.reason} onChange={e => setAdjustForm({ ...adjustForm, reason: e.target.value })} placeholder="Optional" />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setCreditDialog(false)} disabled={adjustSaving}>Cancel</Button>
              <Button type="submit" disabled={adjustSaving}>{adjustSaving ? 'Processing...' : 'Credit'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ── Debit Dialog ────────────────────────────────────────────────── */}
      <Dialog open={debitDialog} onOpenChange={setDebitDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Debit User Balance</DialogTitle>
            <DialogDescription>Manually deduct funds from a user&apos;s balance.</DialogDescription>
          </DialogHeader>
          <form onSubmit={(e) => { e.preventDefault(); handleDebit() }} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="db-user">User ID</Label>
              <Input id="db-user" type="number" min={1} value={adjustForm.user_id || ''} onChange={e => setAdjustForm({ ...adjustForm, user_id: Number(e.target.value) })} required />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="db-amount">Amount (sats)</Label>
              <Input id="db-amount" type="number" min={1} value={adjustForm.amount_sats || ''} onChange={e => setAdjustForm({ ...adjustForm, amount_sats: Number(e.target.value) })} required />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="db-reason">Reason</Label>
              <Input id="db-reason" value={adjustForm.reason} onChange={e => setAdjustForm({ ...adjustForm, reason: e.target.value })} placeholder="Optional" />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setDebitDialog(false)} disabled={adjustSaving}>Cancel</Button>
              <Button type="submit" disabled={adjustSaving}>{adjustSaving ? 'Processing...' : 'Debit'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ── Pricing Dialog ──────────────────────────────────────────────── */}
      <Dialog open={pricingDialog} onOpenChange={setPricingDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{editingModel ? 'Edit Model Pricing' : 'Add Model Pricing'}</DialogTitle>
            <DialogDescription>
              {editingModel ? 'Update pricing for this model.' : 'Define custom pricing for a model.'}
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handlePricingSave} className="flex flex-col gap-4">
            {!editingModel && (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="mp-name">Model Name</Label>
                <Input id="mp-name" value={pricingForm.model_name} onChange={e => setPricingForm({ ...pricingForm, model_name: e.target.value })} placeholder="gpt-4" required />
              </div>
            )}
            <div className="flex items-center gap-4">
              <div className="flex items-center gap-2">
                <Checkbox id="mp-free" checked={pricingForm.is_free} onCheckedChange={c => setPricingForm({ ...pricingForm, is_free: !!c })} />
                <Label htmlFor="mp-free">Free</Label>
              </div>
              <div className="flex items-center gap-2">
                <Checkbox id="mp-advertised" checked={pricingForm.is_advertised} onCheckedChange={c => setPricingForm({ ...pricingForm, is_advertised: !!c })} />
                <Label htmlFor="mp-advertised">Advertised</Label>
              </div>
            </div>
            {!pricingForm.is_free && (
              <>
                <div className="grid grid-cols-2 gap-4">
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor="mp-input">Input /1M (sats)</Label>
                    <Input id="mp-input" type="number" min={0} value={pricingForm.price_per_1m_input_sats ?? ''} onChange={e => setPricingForm({ ...pricingForm, price_per_1m_input_sats: e.target.value ? Number(e.target.value) : null })} placeholder="5" />
                  </div>
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor="mp-output">Output /1M (sats)</Label>
                    <Input id="mp-output" type="number" min={0} value={pricingForm.price_per_1m_output_sats ?? ''} onChange={e => setPricingForm({ ...pricingForm, price_per_1m_output_sats: e.target.value ? Number(e.target.value) : null })} placeholder="15" />
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-4">
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor="mp-per-req">Per Request (sats)</Label>
                    <Input id="mp-per-req" type="number" min={0} value={pricingForm.price_per_request_sats ?? ''} onChange={e => setPricingForm({ ...pricingForm, price_per_request_sats: e.target.value ? Number(e.target.value) : null })} placeholder="1" />
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-4">
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor="mp-ctx">Context Window</Label>
                    <Input id="mp-ctx" type="number" min={1} value={pricingForm.context_window ?? ''} onChange={e => setPricingForm({ ...pricingForm, context_window: e.target.value ? Number(e.target.value) : null })} placeholder="8192" />
                  </div>
                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor="mp-max-tok">Max Output Tokens</Label>
                    <Input id="mp-max-tok" type="number" min={1} value={pricingForm.max_output_tokens ?? ''} onChange={e => setPricingForm({ ...pricingForm, max_output_tokens: e.target.value ? Number(e.target.value) : null })} placeholder="4096" />
                  </div>
                </div>
              </>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setPricingDialog(false)} disabled={pricingSaving}>Cancel</Button>
              <Button type="submit" disabled={pricingSaving}>{pricingSaving ? 'Saving...' : editingModel ? 'Update' : 'Create'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ── Delete Pricing Confirmation ──────────────────────────────────── */}
      <AlertDialog open={!!deletePricingTarget} onOpenChange={(o) => { if (!o) setDeletePricingTarget(null) }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Model Pricing</AlertDialogTitle>
            <AlertDialogDescription>
              Delete pricing for <span className="font-medium font-mono text-foreground">{deletePricingTarget}</span>? The model will fall back to defaults.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleDeletePricing}>Delete</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
