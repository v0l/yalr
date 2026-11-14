import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Badge } from '@/components/ui/badge'
import { userApi } from '@/api/userApi'
import type { MyProvider } from '@/types/userProviders'

interface Props {
  providers: MyProvider[]
  onChange: () => void
  onError: (message: string | null) => void
}

const empty = { name: '', slug: '', base_url: '', api_key: '' }

export default function MyProvidersPanel({ providers, onChange, onError }: Props) {
  const [form, setForm] = useState(empty)
  const [status, setStatus] = useState<Record<string, string>>({})

  async function run(action: () => Promise<unknown>) {
    onError(null)
    try { await action(); onChange() } catch (e) { onError(e instanceof Error ? e.message : 'Request failed') }
  }

  async function check(slug: string) {
    setStatus(s => ({ ...s, [slug]: '...' }))
    try {
      const result = await userApi.checkProvider(slug)
      const models = result.healthy ? await userApi.providerModels(slug).catch(() => null) : null
      setStatus(s => ({ ...s, [slug]: result.healthy ? `healthy${models ? `, ${models.models.length} models` : ''}` : (result.error ?? 'unhealthy') }))
    } catch (e) {
      setStatus(s => ({ ...s, [slug]: e instanceof Error ? e.message : 'failed' }))
    }
  }

  return (
    <div className="panel p-4 space-y-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Endpoints</div>
      <form
        className="grid grid-cols-1 md:grid-cols-5 gap-2"
        onSubmit={e => { e.preventDefault(); run(async () => { await userApi.createProvider({ ...form, api_key: form.api_key || undefined }); setForm(empty) }) }}
      >
        <Input placeholder="Name" value={form.name} onChange={e => setForm({ ...form, name: e.target.value })} required className="font-mono bg-surface border-border" />
        <Input placeholder="slug" value={form.slug} onChange={e => setForm({ ...form, slug: e.target.value })} required className="font-mono bg-surface border-border" />
        <Input placeholder="https://host/v1" value={form.base_url} onChange={e => setForm({ ...form, base_url: e.target.value })} required className="font-mono bg-surface border-border" />
        <Input type="password" placeholder="API key (write only)" value={form.api_key} onChange={e => setForm({ ...form, api_key: e.target.value })} className="font-mono bg-surface border-border" />
        <Button type="submit" className="font-mono text-[11px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">Add</Button>
      </form>
      <table className="w-full table-scan">
        <thead>
          <tr className="border-b border-border/50 text-left font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">
            <th className="px-2 py-2 font-medium">Name</th>
            <th className="px-2 py-2 font-medium">Slug</th>
            <th className="px-2 py-2 font-medium">URL</th>
            <th className="px-2 py-2 font-medium">Key</th>
            <th className="px-2 py-2 font-medium">Status</th>
            <th className="px-2 py-2 font-medium text-right">Actions</th>
          </tr>
        </thead>
        <tbody>
          {providers.length === 0 ? (
            <tr><td colSpan={6} className="px-2 py-10 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO PROVIDERS YET</td></tr>
          ) : providers.map(p => (
            <tr key={p.id} className="border-b border-border/50 hover:bg-surface font-mono text-[12px]">
              <td className="px-2 py-2.5">{p.name}</td>
              <td className="px-2 py-2.5 text-muted-foreground">{p.slug}</td>
              <td className="px-2 py-2.5 text-muted-foreground truncate max-w-64">{p.base_url}</td>
              <td className="px-2 py-2.5">{p.has_api_key ? <Badge className="text-[10px] px-1.5 py-0 h-5 bg-brand/15 text-brand border-brand/30">SET</Badge> : <span className="text-muted-foreground">none</span>}</td>
              <td className="px-2 py-2.5 text-muted-foreground">{status[p.slug] ?? '—'}</td>
              <td className="px-2 py-2.5 text-right space-x-1">
                <Button size="xs" variant="outline" onClick={() => check(p.slug)} className="font-mono text-[10px] uppercase border-border">Check</Button>
                <Button size="xs" variant="outline" onClick={() => { if (confirm(`Delete ${p.name}?`)) run(() => userApi.deleteProvider(p.slug)) }} className="font-mono text-[10px] uppercase border-border text-destructive">Delete</Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
