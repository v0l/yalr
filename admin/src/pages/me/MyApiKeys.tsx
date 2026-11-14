import { useEffect, useState } from 'react'
import { CheckIcon, CopyIcon } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { userApi } from '@/api/userApi'
import type { MyApiKey } from '@/types/userProviders'

export default function MyApiKeys({ onError }: { onError: (message: string | null) => void }) {
  const [keys, setKeys] = useState<MyApiKey[]>([])
  const [name, setName] = useState('')
  const [fresh, setFresh] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)

  async function load() {
    try { setKeys(await userApi.listApiKeys()) } catch (e) { onError(e instanceof Error ? e.message : 'Failed to load keys') }
  }
  useEffect(() => { load() }, [])

  async function create(e: React.FormEvent) {
    e.preventDefault()
    onError(null)
    try {
      const made = await userApi.createApiKey(name.trim() || 'default')
      setFresh(made.key)
      setName('')
      await load()
    } catch (err) { onError(err instanceof Error ? err.message : 'Could not create key') }
  }

  async function remove(id: number) {
    if (!confirm('Delete this key? Anything using it stops working.')) return
    try { await userApi.deleteApiKey(id); await load() } catch (err) { onError(err instanceof Error ? err.message : 'Could not delete key') }
  }

  async function copy() {
    if (!fresh) return
    await navigator.clipboard.writeText(fresh)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }

  return (
    <div className="panel space-y-3 p-4">
      <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">API keys</div>
      <form onSubmit={create} className="flex gap-2">
        <Input value={name} onChange={e => setName(e.target.value)} placeholder="Key name, for example laptop" className="border-border bg-surface font-mono" />
        <Button type="submit" className="border border-brand/40 bg-brand/10 font-mono text-[11px] uppercase tracking-wider text-brand hover:bg-brand/20">Create key</Button>
      </form>
      {fresh && (
        <div className="space-y-1">
          <div className="relative">
            <div className="break-all border border-brand/40 bg-surface p-3 pr-12 font-mono text-[12px] text-brand">{fresh}</div>
            <Button variant="ghost" size="icon" onClick={copy} className="absolute right-1 top-1/2 size-8 -translate-y-1/2" title="Copy key">
              {copied ? <CheckIcon className="size-3.5 text-brand" /> : <CopyIcon className="size-3.5" />}
            </Button>
          </div>
          <p className="font-mono text-[11px] text-muted-foreground">Shown once. Save it now.</p>
        </div>
      )}
      <table className="w-full">
        <tbody>
          {keys.length === 0 ? (
            <tr><td className="px-2 py-6 text-center font-mono text-[12px] text-muted-foreground">{'>'} NO KEYS YET</td></tr>
          ) : keys.map(k => (
            <tr key={k.id} className="border-b border-border/50 font-mono text-[12px] hover:bg-surface">
              <td className="px-2 py-2">{k.name}</td>
              <td className="px-2 py-2 text-muted-foreground">...{k.last_four}</td>
              <td className="px-2 py-2 text-muted-foreground">{k.is_active ? 'active' : 'disabled'}</td>
              <td className="px-2 py-2 text-right">
                <Button size="xs" variant="outline" onClick={() => remove(k.id)} className="border-border font-mono text-[10px] uppercase text-destructive">Delete</Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
