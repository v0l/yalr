import { useEffect, useState } from 'react'
import { PlusIcon, PencilIcon, TrashIcon, WalletIcon, KeyIcon } from 'lucide-react'
import { api } from '../api/client'
import type { Provider } from '../types'
import { Button } from '@/components/ui/button'
import { TopupDialog } from '@/components/TopupDialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
/* ── Constants ────────────────────────────────────────────────── */

const COMMON_PROVIDERS = [
  { name: 'ChatGPT', slug: 'chatgpt', type: 'openai', url: 'https://api.openai.com/v1', logo: '◈' },
  { name: 'Anthropic', slug: 'anthropic', type: 'anthropic', url: 'https://api.anthropic.com', logo: '◆' },
  { name: 'OpenRouter', slug: 'openrouter', type: 'openrouter', url: 'https://openrouter.ai/api/v1', logo: '◎' },
  { name: 'PPQ.ai', slug: 'ppq', type: 'ppq', url: 'https://api.ppq.ai', logo: '⚡' },
]

const DEFAULT_BASE_URLS: Record<string, string> = {
  openai: 'https://api.openai.com/v1',
  anthropic: 'https://api.anthropic.com',
  openrouter: 'https://openrouter.ai/api/v1',
  ppq: 'https://api.ppq.ai',
}

/* ── Sub-components ───────────────────────────────────────────── */

function HealthBadge({ state }: { state?: string }) {
  switch (state) {
    case 'healthy': return <Badge className="bg-brand/15 text-brand border-brand/30 font-mono text-[10px] tracking-wider uppercase">HEALTHY</Badge>
    case 'degraded': return <Badge className="bg-warning/15 text-warning border-warning/30 font-mono text-[10px] tracking-wider uppercase">DEGRADED</Badge>
    case 'unhealthy': return <Badge className="bg-destructive/15 text-destructive border-destructive/30 font-mono text-[10px] tracking-wider uppercase">DOWN</Badge>
    default: return <Badge variant="outline" className="font-mono text-[10px] text-muted-foreground border-border">UNKNOWN</Badge>
  }
}

type ProviderFormData = {
  name: string; slug: string; base_url: string; api_key: string; provider_type: string
}

const emptyForm: ProviderFormData = { name: '', slug: '', base_url: '', api_key: '', provider_type: 'openai' }

/* ═══════════════════════════════════════════════════════════════ */
/*  Providers Page                                                */
/* ═══════════════════════════════════════════════════════════════ */

export default function Providers() {
  const [providers, setProviders] = useState<Provider[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [successMessage, setSuccessMessage] = useState<string | null>(null)

  const [dialogOpen, setDialogOpen] = useState(false)
  const [editingProvider, setEditingProvider] = useState<Provider | null>(null)
  const [form, setForm] = useState<ProviderFormData>({ ...emptyForm })
  const [saving, setSaving] = useState(false)

  const [deleteTarget, setDeleteTarget] = useState<Provider | null>(null)
  const [deleting, setDeleting] = useState(false)
  const [topupProvider, setTopupProvider] = useState<Provider | null>(null)
  const [generatingKey, setGeneratingKey] = useState<Provider | null>(null)
  const [generatedApiKey, setGeneratedApiKey] = useState<string | null>(null)

  useEffect(() => { loadProviders() }, [])

  async function loadProviders() {
    try { setLoading(true); setError(null); const data = await api.getProviders(); setProviders(data.providers) }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to fetch providers') }
    finally { setLoading(false) }
  }

  function openCreate(prefill?: Partial<ProviderFormData>) {
    setEditingProvider(null); setForm({ ...emptyForm, ...prefill }); setDialogOpen(true)
  }

  function openEdit(provider: Provider) {
    setEditingProvider(provider)
    setForm({ name: provider.name, slug: provider.slug, base_url: provider.base_url, api_key: '', provider_type: provider.provider_type })
    setDialogOpen(true)
  }

  async function handleSave(e: React.FormEvent) {
    e.preventDefault(); setSaving(true); setError(null)
    try {
      if (editingProvider) {
        const d: Record<string, string> = {}
        if (form.name !== editingProvider.name) d.name = form.name
        if (form.slug !== editingProvider.slug) d.slug = form.slug
        if (form.base_url !== editingProvider.base_url) d.base_url = form.base_url
        if (form.api_key) d.api_key = form.api_key
        if (form.provider_type !== editingProvider.provider_type) d.provider_type = form.provider_type
        await api.updateProvider(editingProvider.slug, d)
        setSuccessMessage('PROVIDER UPDATED')
      } else {
        await api.createProvider(form)
        setSuccessMessage('PROVIDER CREATED')
      }
      setDialogOpen(false); loadProviders()
    } catch (e) { setError(e instanceof Error ? e.message : 'Failed to save provider') }
    finally { setSaving(false) }
  }

  async function handleDelete() {
    if (!deleteTarget) return; setDeleting(true)
    try { await api.deleteProvider(deleteTarget.slug); setDeleteTarget(null); setSuccessMessage('PROVIDER DELETED'); loadProviders() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to delete provider') }
    finally { setDeleting(false) }
  }

  async function handleGenerateKey(provider: Provider) {
    setGeneratingKey(provider)
    try { const r = await api.generateProviderApiKey(provider.slug); setGeneratedApiKey(r.api_key); setSuccessMessage('API KEY GENERATED'); await loadProviders() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to generate API key') }
    finally { setGeneratingKey(null) }
  }

  /* ── Loading ─────────────────────────────────────────────── */
  if (loading) {
    return (
      <div className="space-y-6 p-6">
        <div className="flex items-center justify-between">
          <div><Skeleton className="h-8 w-44 bg-secondary" /><Skeleton className="h-4 w-52 bg-secondary mt-1" /></div>
          <Skeleton className="h-9 w-32 bg-secondary" />
        </div>
        <Skeleton className="h-64 bg-secondary" />
      </div>
    )
  }

  /* ── Render ──────────────────────────────────────────────── */
  return (
    <div className="space-y-6 p-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground leading-none mb-1">PROVIDERS</h1>
          <p className="font-mono text-[13px] text-muted-foreground">Manage LLM provider connections</p>
        </div>
        <Button onClick={() => openCreate()} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">
          <PlusIcon className="size-3.5" /> Add Provider
        </Button>
      </div>

      {/* Messages */}
      {successMessage && (
        <Alert className="border-brand/30 bg-brand/5 text-brand font-mono text-[13px]">
          <AlertDescription className="flex items-center justify-between">{successMessage}<Button variant="ghost" size="icon-xs" onClick={() => setSuccessMessage(null)} className="text-brand hover:text-brand">×</Button></AlertDescription>
        </Alert>
      )}
      {error && (
        <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono text-[13px]">
          <AlertDescription className="flex items-center justify-between">{error}<Button variant="ghost" size="icon-xs" onClick={() => setError(null)} className="text-destructive hover:text-destructive">×</Button></AlertDescription>
        </Alert>
      )}

      {/* Quick Add */}
      <div className="panel p-4">
        <div className="section-header mb-3">Quick Add</div>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
          {COMMON_PROVIDERS.map(cp => (
            <button
              key={cp.slug}
              onClick={() => openCreate({ name: cp.name, slug: cp.slug, provider_type: cp.type, base_url: cp.url })}
              className="panel p-3 hover:border-brand/30 transition-colors cursor-pointer text-left group"
            >
              <div className="text-2xl mb-2 text-muted-foreground group-hover:text-brand transition-colors">{cp.logo}</div>
              <div className="font-mono text-[13px] font-medium">{cp.name}</div>
              <div className="font-mono text-[10px] text-muted-foreground uppercase mt-0.5">{cp.type}</div>
            </button>
          ))}
        </div>
      </div>

      {/* Provider Table */}
      <div>
        <h2 className="section-header">Configured Providers</h2>
        <div className="panel">
          <div className="overflow-x-auto">
            <table className="w-full table-scan">
              <thead>
                <tr className="border-b border-border/50 text-left">
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Name</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Slug</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Type</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Status</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Base URL</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Actions</th>
                </tr>
              </thead>
              <tbody>
                {providers.length === 0 ? (
                  <tr><td colSpan={6} className="px-4 py-16 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO PROVIDERS CONFIGURED</td></tr>
                ) : (
                  providers.map(p => (
                    <tr key={p.slug} className="border-b border-border/50 hover:bg-surface transition-colors">
                      <td className="px-4 py-3 font-mono text-[13px] font-medium">{p.name}</td>
                      <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{p.slug}</td>
                      <td className="px-4 py-3"><Badge variant="secondary" className="font-mono text-[10px] uppercase tracking-wider bg-secondary text-muted-foreground border-border">{p.provider_type}</Badge></td>
                      <td className="px-4 py-3"><HealthBadge state={p.health?.health_state} /></td>
                      <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground truncate max-w-48" title={p.base_url}>{p.base_url}</td>
                      <td className="px-4 py-3">
                        <div className="flex items-center justify-end gap-1">
                          {(p.provider_type === 'routstr' || p.provider_type === 'ppq') && (
                            <>
                              <Button variant="ghost" size="icon-xs" onClick={() => setTopupProvider(p)} title="Top-up" className="text-muted-foreground hover:text-brand"><WalletIcon className="size-3.5" /></Button>
                              {p.provider_type === 'ppq' && (
                                <Button variant="ghost" size="icon-xs" onClick={() => handleGenerateKey(p)} disabled={!!generatingKey} title="Generate API key" className="text-muted-foreground hover:text-brand"><KeyIcon className="size-3.5" /></Button>
                              )}
                            </>
                          )}
                          <Button variant="ghost" size="icon-xs" onClick={() => openEdit(p)} className="text-muted-foreground hover:text-foreground"><PencilIcon className="size-3.5" /></Button>
                          <Button variant="ghost" size="icon-xs" onClick={() => setDeleteTarget(p)} className="text-muted-foreground hover:text-destructive"><TrashIcon className="size-3.5" /></Button>
                        </div>
                      </td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </div>
        </div>
      </div>

      {/* Create/Edit Dialog */}
      <Dialog open={dialogOpen} onOpenChange={(o) => { if (!o) setDialogOpen(false) }}>
        <DialogContent className="sm:max-w-lg border-border bg-card">
          <DialogHeader>
            <DialogTitle className="font-display text-xl tracking-[0.04em]">{editingProvider ? 'EDIT PROVIDER' : 'ADD PROVIDER'}</DialogTitle>
            <DialogDescription className="font-mono text-[12px] text-muted-foreground">
              {editingProvider ? 'Update the provider configuration.' : 'Connect a new LLM provider.'}
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSave} className="flex flex-col gap-4">
            <div className="grid grid-cols-2 gap-4">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="p-name" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Name</Label>
                <Input id="p-name" value={form.name} onChange={e => setForm({ ...form, name: e.target.value })} placeholder="My OpenAI" required className="font-mono bg-surface border-border text-foreground" />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="p-slug" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Slug</Label>
                <Input id="p-slug" value={form.slug} onChange={e => setForm({ ...form, slug: e.target.value })} placeholder="my-openai" className="font-mono bg-surface border-border text-foreground" required />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="p-type" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Type</Label>
                <Select value={form.provider_type} onValueChange={v => setForm({ ...form, provider_type: v, base_url: DEFAULT_BASE_URLS[v] ?? form.base_url })}>
                  <SelectTrigger id="p-type" className="font-mono bg-surface border-border text-foreground"><SelectValue /></SelectTrigger>
                  <SelectContent className="bg-card border-border">
                    <SelectGroup>
                      {['openai','anthropic','llamacpp','vllm','ollama','routstr','openrouter','ppq'].map(t => <SelectItem key={t} value={t} className="font-mono text-foreground">{t}</SelectItem>)}
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="p-url" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Base URL</Label>
                <Input id="p-url" type="url" value={form.base_url} onChange={e => setForm({ ...form, base_url: e.target.value })} placeholder="https://api.openai.com" className="font-mono bg-surface border-border text-foreground" required />
              </div>
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="p-key" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">
                API Key{editingProvider && <span className="font-normal text-muted-foreground tracking-normal"> (leave blank to keep current)</span>}
              </Label>
              <Input id="p-key" type="password" value={form.api_key} onChange={e => setForm({ ...form, api_key: e.target.value })} placeholder={editingProvider ? '••••••••' : 'sk-...'} className="font-mono bg-surface border-border text-foreground" required={!editingProvider} />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)} disabled={saving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
              <Button type="submit" disabled={saving} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">
                {saving ? 'SAVING...' : editingProvider ? 'UPDATE' : 'CREATE'}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Generated Key Dialog */}
      <Dialog open={!!generatedApiKey} onOpenChange={(o) => { if (!o) { setGeneratedApiKey(null); setSuccessMessage(null) } }}>
        <DialogContent className="sm:max-w-lg border-border bg-card">
          <DialogHeader>
            <DialogTitle className="font-display text-xl tracking-[0.04em] text-brand">API KEY GENERATED</DialogTitle>
            <DialogDescription className="font-mono text-[12px] text-muted-foreground">Save this key securely. You won&apos;t see it again.</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="p-3 bg-surface border border-border font-mono text-[13px] text-brand break-all">{generatedApiKey}</div>
            <Alert className="border-brand/20 bg-brand/5 text-muted-foreground font-mono text-[12px]">
              <AlertDescription>This key is automatically saved to your provider configuration.</AlertDescription>
            </Alert>
          </div>
          <DialogFooter>
            <Button onClick={() => { setGeneratedApiKey(null); setSuccessMessage(null) }} className="font-mono text-[12px] border border-border bg-transparent text-foreground">CLOSE</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Top-up */}
      {topupProvider && (
        <TopupDialog open={!!topupProvider} onOpenChange={o => { if (!o) setTopupProvider(null) }} providerSlug={topupProvider.slug} providerName={topupProvider.name} supportedPaymentMethods={topupProvider.payment_options} currentBalance={topupProvider.health?.balance} />
      )}

      {/* Delete Confirmation */}
      <AlertDialog open={!!deleteTarget} onOpenChange={o => { if (!o) setDeleteTarget(null) }}>
        <AlertDialogContent className="border-border bg-card">
          <AlertDialogHeader>
            <AlertDialogTitle className="font-display text-xl tracking-[0.04em] text-destructive">DELETE PROVIDER</AlertDialogTitle>
            <AlertDialogDescription className="font-mono text-[13px] text-muted-foreground">
              Delete <span className="text-foreground">{deleteTarget?.name}</span>? This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleDelete} disabled={deleting} className="font-mono text-[12px] tracking-wider uppercase">{deleting ? 'DELETING...' : 'DELETE'}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
