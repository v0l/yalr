import { useEffect, useState } from 'react'
import { PlusIcon, PencilIcon, TrashIcon, WalletIcon, KeyIcon, RefreshCw } from 'lucide-react'
import { api } from '../api/client'
import type { Provider } from '../types'
import { Button } from '@/components/ui/button'
import { TopupDialog } from '@/components/TopupDialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { Card, CardContent } from '@/components/ui/card'

// Common providers with logos and defaults
const COMMON_PROVIDERS = [
  { name: 'ChatGPT', slug: 'chatgpt', type: 'openai', url: 'https://api.openai.com/v1', logo: '🤖' },
  { name: 'Anthropic', slug: 'anthropic', type: 'anthropic', url: 'https://api.anthropic.com', logo: '🎭' },
  { name: 'OpenRouter', slug: 'openrouter', type: 'openrouter', url: 'https://openrouter.ai/api/v1', logo: '🔀' },
  { name: 'PPQ.ai', slug: 'ppq', type: 'ppq', url: 'https://api.ppq.ai', logo: '⚡' },
]

function HealthBadge({ state }: { state?: string }) {
  switch (state) {
    case 'healthy':
      return <Badge className="bg-emerald-100 text-emerald-800 dark:bg-emerald-900/30 dark:text-emerald-400">Healthy</Badge>
    case 'degraded':
      return <Badge className="bg-amber-100 text-amber-800 dark:bg-amber-900/30 dark:text-amber-400">Degraded</Badge>
    case 'unhealthy':
      return <Badge className="bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400">Down</Badge>
    default:
      return <Badge variant="outline">Unknown</Badge>
  }
}

type ProviderFormData = {
  name: string
  slug: string
  base_url: string
  api_key: string
  provider_type: string
}

const DEFAULT_BASE_URLS: Record<string, string> = {
  openai: 'https://api.openai.com/v1',
  anthropic: 'https://api.anthropic.com',
  openrouter: 'https://openrouter.ai/api/v1',
  ppq: 'https://api.ppq.ai',
}

const emptyForm: ProviderFormData = {
  name: '',
  slug: '',
  base_url: '',
  api_key: '',
  provider_type: 'openai',
}

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
    try {
      setLoading(true)
      setError(null)
      const data = await api.getProviders()
      setProviders(data.providers)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch providers')
    } finally {
      setLoading(false)
    }
  }

  function openCreate(prefill?: Partial<ProviderFormData>) {
    setEditingProvider(null)
    setForm({ ...emptyForm, ...prefill })
    setDialogOpen(true)
  }

  function openEdit(provider: Provider) {
    setEditingProvider(provider)
    setForm({
      name: provider.name,
      slug: provider.slug,
      base_url: provider.base_url,
      api_key: '',
      provider_type: provider.provider_type,
    })
    setDialogOpen(true)
  }

  async function handleSave(e: React.FormEvent) {
    e.preventDefault()
    setSaving(true)
    setError(null)
    try {
      if (editingProvider) {
        const updateData: Record<string, string> = {}
        if (form.name !== editingProvider.name) updateData.name = form.name
        if (form.slug !== editingProvider.slug) updateData.slug = form.slug
        if (form.base_url !== editingProvider.base_url) updateData.base_url = form.base_url
        if (form.api_key) updateData.api_key = form.api_key
        if (form.provider_type !== editingProvider.provider_type) updateData.provider_type = form.provider_type
        await api.updateProvider(editingProvider.slug, updateData)
        setSuccessMessage('Provider updated successfully')
      } else {
        await api.createProvider(form)
        setSuccessMessage('Provider created successfully')
      }
      setDialogOpen(false)
      loadProviders()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to save provider')
    } finally {
      setSaving(false)
    }
  }

  async function handleDelete() {
    if (!deleteTarget) return
    setDeleting(true)
    try {
      await api.deleteProvider(deleteTarget.slug)
      setDeleteTarget(null)
      setSuccessMessage('Provider deleted successfully')
      loadProviders()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to delete provider')
    } finally {
      setDeleting(false)
    }
  }

  function handleTopup(provider: Provider) {
    setTopupProvider(provider)
  }

  async function handleGenerateKey(provider: Provider) {
    setGeneratingKey(provider)
    try {
      const response = await api.generateProviderApiKey(provider.slug)
      setGeneratedApiKey(response.api_key)
      setSuccessMessage('API key generated successfully!')
      await loadProviders()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to generate API key')
    } finally {
      setGeneratingKey(null)
    }
  }

  if (loading) {
    return (
      <div className="flex flex-col gap-6 p-6">
        <div className="flex items-center justify-between">
          <div className="flex flex-col gap-1">
            <Skeleton className="h-7 w-28" />
            <Skeleton className="h-4 w-48" />
          </div>
          <Skeleton className="h-8 w-32" />
        </div>
        <Skeleton className="h-64 w-full" />
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-6 p-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold text-foreground">Providers</h1>
          <p className="text-sm text-muted-foreground">Manage LLM provider connections</p>
        </div>
        <Button onClick={() => openCreate()}>
          <PlusIcon />
          Add Provider
        </Button>
      </div>

      {/* Messages */}
      {successMessage && (
        <Alert>
          <AlertDescription className="flex items-center justify-between">
            {successMessage}
            <Button variant="ghost" size="icon-xs" onClick={() => setSuccessMessage(null)}>×</Button>
          </AlertDescription>
        </Alert>
      )}
      {error && (
        <Alert variant="destructive">
          <AlertDescription className="flex items-center justify-between">
            {error}
            <Button variant="ghost" size="icon-xs" onClick={() => setError(null)}>×</Button>
          </AlertDescription>
        </Alert>
      )}

      {/* Quick Add - Common Providers */}
      <Card>
        <CardContent className="p-4">
          <h3 className="text-sm font-medium mb-3">Quick Add Common Providers</h3>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
            {COMMON_PROVIDERS.map((cp) => (
              <Button
                key={cp.slug}
                variant="outline"
                className="h-auto p-3 flex flex-col gap-2"
                onClick={() => openCreate({
                  name: cp.name,
                  slug: cp.slug,
                  provider_type: cp.type,
                  base_url: cp.url,
                })}
              >
                <span className="text-2xl">{cp.logo}</span>
                <span className="text-sm font-medium">{cp.name}</span>
              </Button>
            ))}
          </div>
        </CardContent>
      </Card>

      {/* Table */}
      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Slug</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Base URL</TableHead>
                <TableHead className="w-32 text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {providers.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center text-muted-foreground py-12">
                    No providers configured. Add one to get started.
                  </TableCell>
                </TableRow>
              ) : (
                providers.map((provider) => (
                  <TableRow key={provider.slug}>
                    <TableCell className="font-medium">{provider.name}</TableCell>
                    <TableCell className="font-mono text-muted-foreground">{provider.slug}</TableCell>
                    <TableCell>
                      <Badge variant="secondary">{provider.provider_type}</Badge>
                    </TableCell>
                    <TableCell><HealthBadge state={provider.health?.health_state} /></TableCell>
                    <TableCell className="font-mono text-muted-foreground">{provider.base_url}</TableCell>
                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        {(provider.provider_type === 'routstr' || provider.provider_type === 'ppq') && (
                          <>
                            <Button variant="ghost" size="icon-xs" onClick={() => handleTopup(provider)} title="Top-up provider balance">
                              <WalletIcon />
                            </Button>
                            {provider.provider_type === 'ppq' && (
                              <Button variant="ghost" size="icon-xs" onClick={() => handleGenerateKey(provider)} disabled={!!generatingKey} title="Generate API key">
                                {generatingKey?.slug === provider.slug ? <RefreshCw className="animate-spin" /> : <KeyIcon />}
                              </Button>
                            )}
                          </>
                        )}
                        <Button variant="ghost" size="icon-xs" onClick={() => openEdit(provider)}>
                          <PencilIcon />
                        </Button>
                        <Button variant="ghost" size="icon-xs" onClick={() => setDeleteTarget(provider)}>
                          <TrashIcon />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {/* Create/Edit Dialog */}
      <Dialog open={dialogOpen} onOpenChange={(open) => { if (!open) setDialogOpen(false) }}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{editingProvider ? 'Edit Provider' : 'Add Provider'}</DialogTitle>
            <DialogDescription>
              {editingProvider ? 'Update the provider configuration.' : 'Connect a new LLM provider.'}
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handleSave} className="flex flex-col gap-4">
            <div className="grid grid-cols-2 gap-4">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="p-name">Name</Label>
                <Input
                  id="p-name"
                  value={form.name}
                  onChange={(e) => setForm({ ...form, name: e.target.value })}
                  placeholder="My OpenAI"
                  required
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="p-slug">Slug</Label>
                <Input
                  id="p-slug"
                  value={form.slug}
                  onChange={(e) => setForm({ ...form, slug: e.target.value })}
                  placeholder="my-openai"
                  className="font-mono"
                  required
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="p-type">Provider Type</Label>
                <Select value={form.provider_type} onValueChange={(v) => {
                  setForm({
                    ...form,
                    provider_type: v,
                    base_url: DEFAULT_BASE_URLS[v] ?? form.base_url,
                  });
                }}>
                  <SelectTrigger id="p-type" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      <SelectItem value="openai">OpenAI</SelectItem>
                      <SelectItem value="anthropic">Anthropic</SelectItem>
                      <SelectItem value="llamacpp">LlamaCpp</SelectItem>
                      <SelectItem value="vllm">vLLM</SelectItem>
                      <SelectItem value="ollama">Ollama</SelectItem>
                      <SelectItem value="routstr">Routstr</SelectItem>
                      <SelectItem value="openrouter">OpenRouter</SelectItem>
                      <SelectItem value="ppq">PPQ.ai</SelectItem>
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="p-url">Base URL</Label>
                <Input
                  id="p-url"
                  type="url"
                  value={form.base_url}
                  onChange={(e) => setForm({ ...form, base_url: e.target.value })}
                  placeholder="https://api.openai.com"
                  required
                />
              </div>
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="p-key">
                API Key{editingProvider && <span className="font-normal text-muted-foreground"> (leave blank to keep current)</span>}
              </Label>
              <Input
                id="p-key"
                type="password"
                value={form.api_key}
                onChange={(e) => setForm({ ...form, api_key: e.target.value })}
                placeholder={editingProvider ? 'Leave blank to keep current API key' : 'sk-...'}
                required={!editingProvider}
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)} disabled={saving}>
                Cancel
              </Button>
              <Button type="submit" disabled={saving}>
                {saving ? 'Saving...' : editingProvider ? 'Update Provider' : 'Create Provider'}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Generated API Key Dialog */}
      <Dialog open={!!generatedApiKey} onOpenChange={(open) => { if (!open) { setGeneratedApiKey(null); setSuccessMessage(null); } }}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>API Key Generated!</DialogTitle>
            <DialogDescription>
              Save this API key securely. You won't be able to see it again.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="p-3 bg-muted rounded-md">
              <code className="text-sm break-all">{generatedApiKey}</code>
            </div>
            <Alert>
              <AlertDescription>
                This key is automatically saved to your provider configuration. You can use it to authenticate API requests.
              </AlertDescription>
            </Alert>
          </div>
          <DialogFooter>
            <Button onClick={() => { setGeneratedApiKey(null); setSuccessMessage(null); }}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Topup Dialog */}
      {topupProvider && (
        <TopupDialog
          open={!!topupProvider}
          onOpenChange={(open) => {
            if (!open) setTopupProvider(null)
          }}
          providerSlug={topupProvider.slug}
          providerName={topupProvider.name}
          supportedPaymentMethods={topupProvider.payment_options}
        />
      )}

      {/* Delete Confirmation */}
      <AlertDialog open={!!deleteTarget} onOpenChange={(open) => { if (!open) setDeleteTarget(null) }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Provider</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to delete <span className="font-medium text-foreground">{deleteTarget?.name}</span>?
              This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={deleting}>Cancel</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleDelete} disabled={deleting}>
              {deleting ? 'Deleting...' : 'Delete'}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
