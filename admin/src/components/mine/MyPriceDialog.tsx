import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import AdvertiseField from '@/components/pricing/AdvertiseField'
import { userApi } from '@/api/userApi'
import type { CurrencyOption } from '@/types'
import type { MyProvider, OwnedPrice } from '@/types/userProviders'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  providers: MyProvider[]
  prices: OwnedPrice[]
  editing: OwnedPrice | null
  defaultProviderSlug?: string
  onSaved: (message: string) => void
  onError: (message: string) => void
}

interface Form {
  provider_slug: string
  model_name: string
  currency: string
  is_advertised: boolean
  input: string
  output: string
  cache_read: string
  cache_write: string
  request: string
}

const FALLBACK: CurrencyOption[] = [{ code: 'sats', milli_per_unit: 1000 }, { code: 'usd', milli_per_unit: 100000 }]

const text = (v: number | null) => (v === null ? '' : String(v))
const toNumber = (v: string) => (v.trim() === '' ? null : Number(v))

function blank(providerSlug: string): Form {
  return { provider_slug: providerSlug, model_name: '', currency: 'usd', is_advertised: true, input: '', output: '', cache_read: '', cache_write: '', request: '' }
}

function fromPrice(p: OwnedPrice): Form {
  return {
    provider_slug: p.provider_slug ?? '',
    model_name: p.model_name,
    currency: p.currency,
    is_advertised: p.is_advertised,
    input: text(p.price_per_1m_input),
    output: text(p.price_per_1m_output),
    cache_read: text(p.price_per_1m_cache_read),
    cache_write: text(p.price_per_1m_cache_write),
    request: text(p.price_per_request),
  }
}

export default function MyPriceDialog({ open, onOpenChange, providers, prices, editing, defaultProviderSlug, onSaved, onError }: Props) {
  const [form, setForm] = useState<Form>(blank(''))
  const [currencies, setCurrencies] = useState<CurrencyOption[]>(FALLBACK)
  const [models, setModels] = useState<string[] | null>(null)
  const [modelsFailed, setModelsFailed] = useState(false)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    if (!open) return
    setForm(editing ? fromPrice(editing) : blank(defaultProviderSlug ?? providers[0]?.slug ?? ''))
    userApi.currencies().then(setCurrencies).catch(() => setCurrencies(FALLBACK))
  }, [open, editing, defaultProviderSlug, providers])

  useEffect(() => {
    if (!open || editing || !form.provider_slug) return
    let stale = false
    setModels(null)
    setModelsFailed(false)
    userApi.providerModels(form.provider_slug)
      .then(r => { if (!stale) setModels(r.models) })
      .catch(() => { if (!stale) { setModels([]); setModelsFailed(true) } })
    return () => { stale = true }
  }, [open, editing, form.provider_slug])

  const priced = new Set(prices.filter(p => p.provider_slug === form.provider_slug).map(p => p.model_name))
  const unpriced = (models ?? []).filter(m => !priced.has(m))
  const unit = form.currency === 'sats' ? 'sats' : `${form.currency.toUpperCase()} millis`

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    setSaving(true)
    const price = {
      currency: form.currency,
      price_per_1m_input: toNumber(form.input),
      price_per_1m_output: toNumber(form.output),
      price_per_1m_cache_read: toNumber(form.cache_read),
      price_per_1m_cache_write: toNumber(form.cache_write),
      price_per_request: toNumber(form.request),
    }
    try {
      if (editing) {
        await userApi.updatePrice(editing.id, price)
      } else {
        await userApi.createPrice({ ...price, provider_slug: form.provider_slug, model_name: form.model_name, is_advertised: form.is_advertised })
      }
      onOpenChange(false)
      onSaved(editing ? 'PRICE UPDATED' : 'PRICE CREATED')
    } catch (err) {
      onError(err instanceof Error ? err.message : 'Failed to save price')
    } finally {
      setSaving(false)
    }
  }

  const label = 'font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground'
  const input = 'font-mono bg-surface border-border text-foreground'
  const field = (id: string, title: string, key: 'input' | 'output' | 'cache_read' | 'cache_write' | 'request', placeholder?: string) => (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id} className={label}>{title} ({unit})</Label>
      <Input id={id} type="number" min={0} placeholder={placeholder} value={form[key]} onChange={e => setForm({ ...form, [key]: e.target.value })} className={input} />
    </div>
  )

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg border-border bg-card">
        <DialogHeader>
          <DialogTitle className="font-display text-xl tracking-[0.04em]">{editing ? 'EDIT PRICING' : 'ADD PRICING'}</DialogTitle>
          <DialogDescription className="font-mono text-[12px] text-muted-foreground">
            {editing ? 'Update the price for this model.' : 'Set what consumers pay to use one of your providers.'}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="flex flex-col gap-4">
          {!editing && (
            <>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="mpp-provider" className={label}>Provider</Label>
                <Select value={form.provider_slug} onValueChange={v => setForm({ ...form, provider_slug: v, model_name: '' })}>
                  <SelectTrigger id="mpp-provider" className={input}><SelectValue placeholder="Select a provider" /></SelectTrigger>
                  <SelectContent className="bg-card border-border">
                    <SelectGroup>
                      {providers.map(p => <SelectItem key={p.id} value={p.slug} className="font-mono text-foreground">{p.name}</SelectItem>)}
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="mpp-model" className={label}>Model Name</Label>
                {modelsFailed ? (
                  <Input id="mpp-model" placeholder="Could not load models, type a name" value={form.model_name} onChange={e => setForm({ ...form, model_name: e.target.value })} required className={input} />
                ) : (
                  <Select value={form.model_name} onValueChange={v => setForm({ ...form, model_name: v })}>
                    <SelectTrigger id="mpp-model" className={input}><SelectValue placeholder={models === null ? 'Loading models...' : 'Select a model'} /></SelectTrigger>
                    <SelectContent className="bg-card border-border max-h-72">
                      <SelectGroup>
                        {unpriced.length === 0
                          ? <div className="px-2 py-1.5 font-mono text-[12px] text-muted-foreground">{models === null ? 'Loading models...' : 'No unpriced models'}</div>
                          : unpriced.map(m => <SelectItem key={m} value={m} className="font-mono text-foreground">{m}</SelectItem>)}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                )}
              </div>
              <AdvertiseField value={form.is_advertised} onChange={v => setForm({ ...form, is_advertised: v })} listing={null} />
            </>
          )}
          {editing && (
            <div className="font-mono text-[12px] text-muted-foreground">{editing.model_name} on {editing.provider_slug}</div>
          )}
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="mpp-currency" className={label}>Currency</Label>
            <Select value={form.currency} onValueChange={v => setForm({ ...form, currency: v })}>
              <SelectTrigger id="mpp-currency" className={input}><SelectValue /></SelectTrigger>
              <SelectContent className="bg-card border-border">
                <SelectGroup>
                  {currencies.map(c => <SelectItem key={c.code} value={c.code} className="font-mono text-foreground">{c.code.toUpperCase()}</SelectItem>)}
                </SelectGroup>
              </SelectContent>
            </Select>
          </div>
          <div className="grid grid-cols-2 gap-4">
            {field('mpp-in', 'In /1M', 'input')}
            {field('mpp-out', 'Out /1M', 'output')}
          </div>
          <div className="grid grid-cols-2 gap-4">
            {field('mpp-cr', 'Cache Read /1M', 'cache_read', 'same as input')}
            {field('mpp-cw', 'Cache Write /1M', 'cache_write', 'same as input')}
          </div>
          {field('mpp-req', 'Per Request', 'request')}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={saving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
            <Button type="submit" disabled={saving || (!editing && (!form.model_name || !form.provider_slug))} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand">
              {saving ? '...' : editing ? 'UPDATE' : 'CREATE'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
