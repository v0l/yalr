import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import type { ListingStatus } from '@/api/market'
import { api } from '@/api/client'
import type { CurrencyOption, ModelPricingEntry, Provider } from '@/types'
import AdvertiseField from './AdvertiseField'

const PLATFORM = '__platform__'
const FALLBACK: CurrencyOption[] = [{ code: 'sats', milli_per_unit: 1000 }, { code: 'usd', milli_per_unit: 100000 }]
const PRICE_KEYS = ['input', 'output', 'cache_read', 'cache_write', 'request'] as const
type PriceKey = typeof PRICE_KEYS[number]

interface Form {
  provider_slug: string
  model_name: string
  currency: string
  is_advertised: boolean
  is_free: boolean
  prices: Record<PriceKey, string>
  context_window: string
  max_output_tokens: string
}

interface ModelOption {
  id: string
  context_length?: number | null
  max_output_tokens?: number | null
}

const blankPrices = (): Record<PriceKey, string> => ({ input: '', output: '', cache_read: '', cache_write: '', request: '' })

function storedPrice(entry: ModelPricingEntry, key: PriceKey): number | null {
  const sats = entry.currency === 'sats'
  const pick = {
    input: sats ? entry.price_per_1m_input_sats : entry.price_per_1m_input_milli,
    output: sats ? entry.price_per_1m_output_sats : entry.price_per_1m_output_milli,
    cache_read: sats ? entry.price_per_1m_cache_read_sats : entry.price_per_1m_cache_read_milli,
    cache_write: sats ? entry.price_per_1m_cache_write_sats : entry.price_per_1m_cache_write_milli,
    request: sats ? entry.price_per_request_sats : entry.price_per_request_milli,
  }
  return pick[key]
}

function unitsPerStored(currency: string, options: CurrencyOption[]): number {
  if (currency === 'sats') return 1
  return options.find(c => c.code === currency)?.milli_per_unit ?? 100000
}

export default function PricingDialog({
  open,
  onOpenChange,
  editing,
  isAdmin,
  providers,
  prices,
  listing,
  onSaved,
  onError,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  editing: ModelPricingEntry | null
  isAdmin: boolean
  providers: Provider[]
  prices: ModelPricingEntry[]
  listing: ListingStatus | null
  onSaved: (message: string) => void
  onError: (message: string) => void
}) {
  const [currencies, setCurrencies] = useState<CurrencyOption[]>(FALLBACK)
  const [form, setForm] = useState<Form>({ provider_slug: '', model_name: '', currency: 'sats', is_advertised: true, is_free: false, prices: blankPrices(), context_window: '', max_output_tokens: '' })
  const [models, setModels] = useState<ModelOption[] | null>(null)
  const [modelsFailed, setModelsFailed] = useState(false)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    if (!open) return
    api.listCurrencies().then(setCurrencies).catch(() => setCurrencies(FALLBACK))
    if (editing) {
      const scale = unitsPerStored(editing.currency, currencies)
      const shown = (key: PriceKey) => {
        const v = storedPrice(editing, key)
        return v === null ? '' : String(v / scale)
      }
      setForm({
        provider_slug: editing.provider_slug ?? PLATFORM,
        model_name: editing.model_name,
        currency: editing.currency,
        is_advertised: editing.is_advertised,
        is_free: editing.is_free,
        prices: { input: shown('input'), output: shown('output'), cache_read: shown('cache_read'), cache_write: shown('cache_write'), request: shown('request') },
        context_window: editing.context_window?.toString() ?? '',
        max_output_tokens: editing.max_output_tokens?.toString() ?? '',
      })
    } else {
      setForm({ provider_slug: isAdmin ? PLATFORM : providers[0]?.slug ?? '', model_name: '', currency: isAdmin ? 'sats' : 'usd', is_advertised: true, is_free: false, prices: blankPrices(), context_window: '', max_output_tokens: '' })
    }
  }, [open, editing])

  useEffect(() => {
    if (!open || editing || !form.provider_slug) return
    let stale = false
    setModels(null)
    setModelsFailed(false)
    const load: Promise<ModelOption[]> = form.provider_slug === PLATFORM
      ? api.getModels().then(r => r.data.map(m => ({ id: m.id, context_length: m.context_length, max_output_tokens: m.max_output_tokens })))
      : api.getProviderModels(form.provider_slug).then(r => r.models.map(m => ({ id: m.id })))
    load
      .then(list => { if (!stale) setModels(list) })
      .catch(() => { if (!stale) { setModels([]); setModelsFailed(true) } })
    return () => { stale = true }
  }, [open, editing, form.provider_slug])

  const providerSlug = form.provider_slug === PLATFORM ? null : form.provider_slug
  const priced = new Set(prices.filter(p => (p.provider_slug ?? null) === providerSlug).map(p => p.model_name))
  const unpriced = (models ?? []).filter(m => !priced.has(m.id))
  const unit = form.currency === 'sats' ? 'sats' : form.currency.toUpperCase()

  function pickModel(id: string) {
    const model = models?.find(m => m.id === id)
    setForm(f => ({
      ...f,
      model_name: id,
      context_window: f.context_window || (model?.context_length?.toString() ?? ''),
      max_output_tokens: f.max_output_tokens || (model?.max_output_tokens?.toString() ?? ''),
    }))
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    setSaving(true)
    const scale = unitsPerStored(form.currency, currencies)
    const value = (key: PriceKey) => (form.prices[key].trim() === '' ? null : Math.round(Number(form.prices[key]) * scale))
    const sats = form.currency === 'sats'
    const amounts = sats
      ? { price_per_1m_input_sats: value('input'), price_per_1m_output_sats: value('output'), price_per_1m_cache_read_sats: value('cache_read'), price_per_1m_cache_write_sats: value('cache_write'), price_per_request_sats: value('request') }
      : { price_per_1m_input_milli: value('input'), price_per_1m_output_milli: value('output'), price_per_1m_cache_read_milli: value('cache_read'), price_per_1m_cache_write_milli: value('cache_write'), price_per_request_milli: value('request') }
    const limits = {
      context_window: form.context_window ? Number(form.context_window) : null,
      max_output_tokens: form.max_output_tokens ? Number(form.max_output_tokens) : null,
    }
    try {
      if (editing) {
        await api.updateModelPricing(editing.id, { currency: form.currency, is_advertised: form.is_advertised, is_free: form.is_free, ...amounts, ...limits })
      } else {
        await api.createModelPricing({ model_name: form.model_name, provider_slug: providerSlug, currency: form.currency, is_advertised: form.is_advertised, is_free: form.is_free, ...amounts, ...limits })
      }
      onOpenChange(false)
      onSaved(editing ? 'PRICING UPDATED' : 'PRICING CREATED')
    } catch (err) {
      onError(err instanceof Error ? err.message : 'Failed to save pricing')
    } finally {
      setSaving(false)
    }
  }

  const label = 'font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground'
  const input = 'font-mono bg-surface border-border text-foreground'
  const priceField = (key: PriceKey, title: string, placeholder?: string) => (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={`mp-${key}`} className={label}>{title} ({unit})</Label>
      <Input id={`mp-${key}`} type="number" min={0} step="any" placeholder={placeholder} value={form.prices[key]} onChange={e => setForm({ ...form, prices: { ...form.prices, [key]: e.target.value } })} className={input} />
    </div>
  )

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg border-border bg-card max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="font-display text-xl tracking-[0.04em]">{editing ? 'EDIT PRICING' : 'ADD PRICING'}</DialogTitle>
          <DialogDescription className="font-mono text-[12px] text-muted-foreground">
            {editing ? `${editing.model_name}${editing.provider_slug ? ` on ${editing.provider_slug}` : ''}` : 'Set what consumers pay to use a model.'}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="flex flex-col gap-4">
          {!editing && (
            <>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="mp-provider" className={label}>Provider</Label>
                <Select value={form.provider_slug} onValueChange={v => setForm({ ...form, provider_slug: v, model_name: '' })}>
                  <SelectTrigger id="mp-provider" className={input}><SelectValue placeholder="Select a provider" /></SelectTrigger>
                  <SelectContent className="bg-card border-border max-h-72">
                    <SelectGroup>
                      {isAdmin && <SelectItem value={PLATFORM} className="font-mono text-foreground">Platform (any provider)</SelectItem>}
                      {providers.map(p => <SelectItem key={p.slug} value={p.slug} className="font-mono text-foreground">{p.name}</SelectItem>)}
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="mp-name" className={label}>Model Name</Label>
                {modelsFailed ? (
                  <Input id="mp-name" placeholder="Could not load models, type a name" value={form.model_name} onChange={e => setForm({ ...form, model_name: e.target.value })} required className={input} />
                ) : (
                  <Select value={form.model_name} onValueChange={pickModel}>
                    <SelectTrigger id="mp-name" className={input}><SelectValue placeholder={models === null ? 'Loading models...' : 'Select a model'} /></SelectTrigger>
                    <SelectContent className="bg-card border-border max-h-72">
                      <SelectGroup>
                        {unpriced.length === 0
                          ? <div className="px-2 py-1.5 font-mono text-[12px] text-muted-foreground">{models === null ? 'Loading models...' : 'No unpriced models'}</div>
                          : unpriced.map(m => <SelectItem key={m.id} value={m.id} className="font-mono text-foreground">{m.id}</SelectItem>)}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                )}
              </div>
            </>
          )}
          <AdvertiseField value={form.is_advertised} onChange={v => setForm({ ...form, is_advertised: v })} listing={listing} />
          {isAdmin && (
            <div className="flex items-center gap-2">
              <Checkbox id="mp-free" checked={form.is_free} onCheckedChange={c => setForm({ ...form, is_free: !!c })} />
              <Label htmlFor="mp-free" className="font-mono text-[12px]">FREE</Label>
            </div>
          )}
          {!form.is_free && (
            <>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="mp-currency" className={label}>Currency</Label>
                <Select value={form.currency} onValueChange={v => setForm({ ...form, currency: v })}>
                  <SelectTrigger id="mp-currency" className={input}><SelectValue /></SelectTrigger>
                  <SelectContent className="bg-card border-border">
                    <SelectGroup>
                      {currencies.map(c => <SelectItem key={c.code} value={c.code} className="font-mono text-foreground">{c.code.toUpperCase()}</SelectItem>)}
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </div>
              <div className="grid grid-cols-2 gap-4">
                {priceField('input', 'In /1M')}
                {priceField('output', 'Out /1M')}
              </div>
              <div className="grid grid-cols-2 gap-4">
                {priceField('cache_read', 'Cache Read /1M', 'same as input')}
                {priceField('cache_write', 'Cache Write /1M', 'same as input')}
              </div>
              {priceField('request', 'Per Request')}
            </>
          )}
          <div className="grid grid-cols-2 gap-4">
            <div className="flex flex-col gap-1.5"><Label htmlFor="mp-ctx" className={label}>Context Window</Label><Input id="mp-ctx" type="number" min={1} value={form.context_window} onChange={e => setForm({ ...form, context_window: e.target.value })} className={input} /></div>
            <div className="flex flex-col gap-1.5"><Label htmlFor="mp-max-tok" className={label}>Max Output Tokens</Label><Input id="mp-max-tok" type="number" min={1} value={form.max_output_tokens} onChange={e => setForm({ ...form, max_output_tokens: e.target.value })} className={input} /></div>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={saving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
            <Button type="submit" disabled={saving || (!editing && (!form.model_name || !form.provider_slug))} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand">{saving ? '...' : editing ? 'UPDATE' : 'CREATE'}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
