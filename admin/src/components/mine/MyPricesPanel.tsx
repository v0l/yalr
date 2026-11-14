import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { userApi } from '@/api/userApi'
import { formatPrice } from '@/lib/money'
import type { MyProvider, OwnedPrice } from '@/types/userProviders'

interface Props {
  providers: MyProvider[]
  prices: OwnedPrice[]
  onChange: () => void
  onError: (message: string | null) => void
}

const blank = { provider_slug: '', model_name: '', currency: 'usd', input: '', output: '', request: '' }

const toNumber = (v: string) => (v.trim() === '' ? null : Number(v))

export default function MyPricesPanel({ providers, prices, onChange, onError }: Props) {
  const [form, setForm] = useState(blank)

  async function run(action: () => Promise<unknown>) {
    onError(null)
    try { await action(); onChange() } catch (e) { onError(e instanceof Error ? e.message : 'Request failed') }
  }

  return (
    <div className="panel p-4 space-y-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">
        Prices (sats, or thousandths of a minor unit: usd 1 = 1 millicent)
      </div>
      <form
        className="grid grid-cols-2 md:grid-cols-7 gap-2"
        onSubmit={e => {
          e.preventDefault()
          run(async () => {
            await userApi.createPrice({
              provider_slug: form.provider_slug || providers[0]?.slug,
              model_name: form.model_name,
              is_advertised: true,
              currency: form.currency,
              price_per_1m_input: toNumber(form.input),
              price_per_1m_output: toNumber(form.output),
              price_per_1m_cache_read: null,
              price_per_1m_cache_write: null,
              price_per_request: toNumber(form.request),
            })
            setForm(blank)
          })
        }}
      >
        <select value={form.provider_slug} onChange={e => setForm({ ...form, provider_slug: e.target.value })} className="font-mono text-[12px] bg-surface border border-border h-9 px-2">
          {providers.map(p => <option key={p.id} value={p.slug}>{p.slug}</option>)}
        </select>
        <Input placeholder="org/model-name" value={form.model_name} onChange={e => setForm({ ...form, model_name: e.target.value })} required className="font-mono bg-surface border-border md:col-span-2" />
        <Input placeholder="currency" value={form.currency} onChange={e => setForm({ ...form, currency: e.target.value })} required className="font-mono bg-surface border-border" />
        <Input type="number" min={0} placeholder="in / 1M" value={form.input} onChange={e => setForm({ ...form, input: e.target.value })} className="font-mono bg-surface border-border" />
        <Input type="number" min={0} placeholder="out / 1M" value={form.output} onChange={e => setForm({ ...form, output: e.target.value })} className="font-mono bg-surface border-border" />
        <Button type="submit" disabled={providers.length === 0} className="font-mono text-[11px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">Set price</Button>
      </form>
      <table className="w-full table-scan">
        <thead>
          <tr className="border-b border-border/50 text-left font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">
            <th className="px-2 py-2 font-medium">Model</th>
            <th className="px-2 py-2 font-medium">Provider</th>
            <th className="px-2 py-2 font-medium">Currency</th>
            <th className="px-2 py-2 font-medium text-right">In/1M</th>
            <th className="px-2 py-2 font-medium text-right">Out/1M</th>
            <th className="px-2 py-2 font-medium text-right">Per req</th>
            <th className="px-2 py-2 font-medium text-right">Actions</th>
          </tr>
        </thead>
        <tbody>
          {prices.length === 0 ? (
            <tr><td colSpan={7} className="px-2 py-10 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO PRICES YET</td></tr>
          ) : prices.map(p => (
            <tr key={p.id} className="border-b border-border/50 hover:bg-surface font-mono text-[12px] tabular-nums">
              <td className="px-2 py-2.5">{p.model_name}</td>
              <td className="px-2 py-2.5 text-muted-foreground">{p.provider_slug}</td>
              <td className="px-2 py-2.5 uppercase">{p.currency}</td>
              <td className="px-2 py-2.5 text-right">{formatPrice(p.price_per_1m_input, p.currency)}</td>
              <td className="px-2 py-2.5 text-right">{formatPrice(p.price_per_1m_output, p.currency)}</td>
              <td className="px-2 py-2.5 text-right">{formatPrice(p.price_per_request, p.currency)}</td>
              <td className="px-2 py-2.5 text-right">
                <Button size="xs" variant="outline" onClick={() => run(() => userApi.deletePrice(p.id))} className="font-mono text-[10px] uppercase border-border text-destructive">Delete</Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
