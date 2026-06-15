import { ExternalLinkIcon, ShieldCheckIcon } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import type { Provider, ProviderFormData } from '../types'

const DEFAULT_BASE_URLS: Record<string, string> = {
  openai: 'https://api.openai.com/v1',
  anthropic: 'https://api.anthropic.com',
  openrouter: 'https://openrouter.ai/api/v1',
  ppq: 'https://api.ppq.ai',
}

const PROVIDER_TYPES = ['openai', 'anthropic', 'llamacpp', 'vllm', 'ollama', 'routstr', 'openrouter', 'ppq']

export interface ProviderFormDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  editingProvider: Provider | null
  form: ProviderFormData
  setForm: (f: ProviderFormData) => void
  onSave: (e: React.FormEvent) => void
  saving: boolean
  error: string | null
  onClearError: () => void
  apiKeysUrl?: string
}

export function ProviderFormDialog({
  open, onOpenChange, editingProvider, form, setForm, onSave, saving, error, onClearError, apiKeysUrl,
}: ProviderFormDialogProps) {
  const isOauth = !!editingProvider?.is_oauth || form.provider_type.endsWith('-oauth')
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg border-border bg-card">
        <DialogHeader>
          <DialogTitle className="font-display text-xl tracking-[0.04em]">
            {editingProvider ? 'EDIT PROVIDER' : 'ADD PROVIDER'}
          </DialogTitle>
          <DialogDescription className="font-mono text-[12px] text-muted-foreground">
            {editingProvider ? 'Update the provider configuration.' : 'Connect a new LLM provider.'}
          </DialogDescription>
        </DialogHeader>
        {error && (
          <div className="font-mono text-[12px] text-destructive bg-destructive/5 border border-destructive/20 p-2 px-3">
            {error}
            <button onClick={onClearError} className="ml-2 text-destructive/60 hover:text-destructive">×</button>
          </div>
        )}
        <form onSubmit={onSave} className="flex flex-col gap-4">
          <div className="grid grid-cols-2 gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="p-name" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Name</Label>
              <Input id="p-name" value={form.name} onChange={e => setForm({ ...form, name: e.target.value })} placeholder="My OpenAI" required className="font-mono bg-surface border-border text-foreground" />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="p-slug" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Slug</Label>
              <Input id="p-slug" value={form.slug} onChange={e => setForm({ ...form, slug: e.target.value })} placeholder="my-openai" className="font-mono bg-surface border-border text-foreground" required />
            </div>
            {!isOauth && (
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="p-type" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Type</Label>
                <Select value={form.provider_type} onValueChange={v => setForm({ ...form, provider_type: v, base_url: DEFAULT_BASE_URLS[v] ?? form.base_url })}>
                  <SelectTrigger id="p-type" className="font-mono bg-surface border-border text-foreground"><SelectValue /></SelectTrigger>
                  <SelectContent className="bg-card border-border">
                    <SelectGroup>
                      {PROVIDER_TYPES.map(t => <SelectItem key={t} value={t} className="font-mono text-foreground">{t}</SelectItem>)}
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </div>
            )}
            {isOauth && (
              <div className="flex flex-col gap-1.5">
                <Label className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Type</Label>
                <div className="flex items-center gap-1.5 h-9 px-3 font-mono text-[12px] bg-surface border border-border text-brand">
                  <ShieldCheckIcon className="size-3.5" />
                  {form.provider_type.toUpperCase()}
                </div>
              </div>
            )}
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="p-url" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Base URL</Label>
              <Input id="p-url" type="url" value={form.base_url} onChange={e => setForm({ ...form, base_url: e.target.value })} placeholder="https://api.openai.com" className="font-mono bg-surface border-border text-foreground" required />
            </div>
          </div>
          {!isOauth && (
            <div className="flex flex-col gap-1.5">
              <div className="flex items-center gap-2">
                <Label htmlFor="p-key" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">
                  API Key{editingProvider && <span className="font-normal text-muted-foreground tracking-normal"> (leave blank to keep current)</span>}
                </Label>
                {apiKeysUrl && (
                  <a
                    href={apiKeysUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center gap-1 font-mono text-[10px] text-muted-foreground hover:text-brand transition-colors ml-auto"
                  >
                    <ExternalLinkIcon className="size-2.5" />
                    Get API key
                  </a>
                )}
              </div>
              <Input id="p-key" type="password" value={form.api_key} onChange={e => setForm({ ...form, api_key: e.target.value })} placeholder={editingProvider ? '••••••••' : 'sk-...'} className="font-mono bg-surface border-border text-foreground" required={!editingProvider} />
            </div>
          )}
          {isOauth && (
            <div className="font-mono text-[11px] text-muted-foreground bg-brand/5 border border-brand/20 p-2.5 leading-relaxed">
              This provider authenticates with an OAuth subscription. Credentials are
              managed automatically — use <span className="text-brand">RE-AUTH</span> on the
              provider card to refresh access.
            </div>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={saving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
            <Button type="submit" disabled={saving} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">
              {saving ? 'SAVING...' : editingProvider ? 'UPDATE' : 'CREATE'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
