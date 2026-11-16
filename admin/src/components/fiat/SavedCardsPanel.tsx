import { useEffect, useState } from 'react'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { fiatApi, type AutoReload, type FiatConfig, type SavedCard } from '@/api/fiat'

const UNIT = 100_000

interface Props {
  config: FiatConfig
  cards: SavedCard[]
  onChanged: () => void
}

function AutoReloadForm({ currency, cards, current, config, onSaved }: { currency: string; cards: SavedCard[]; current?: AutoReload; config: FiatConfig; onSaved: () => void }) {
  const [enabled, setEnabled] = useState(current?.enabled ?? false)
  const [threshold, setThreshold] = useState((current?.threshold_milli ?? 5 * UNIT) / UNIT)
  const [amount, setAmount] = useState((current?.amount_milli ?? 25 * UNIT) / UNIT)
  const [cap, setCap] = useState((current?.monthly_cap_milli ?? 100 * UNIT) / UNIT)
  const [card, setCard] = useState(current?.payment_method_id ?? cards.find(c => c.is_default)?.id ?? cards[0]?.id)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => { if (current) setEnabled(current.enabled) }, [current])

  async function save(e: React.FormEvent) {
    e.preventDefault()
    setError(null)
    try {
      await fiatApi.saveAutoReload({ currency, enabled, threshold, amount, monthly_cap: cap, payment_method_id: card! })
      onSaved()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not save')
    }
  }

  const field = (label: string, value: number, set: (n: number) => void) => (
    <div className="space-y-1">
      <Label className="font-mono text-[10px] uppercase tracking-wider">{label}</Label>
      <Input type="number" step="0.01" value={value} onChange={e => set(Number(e.target.value))} className="h-8 w-28 font-mono tabular-nums" />
    </div>
  )

  return (
    <form onSubmit={save} className="space-y-3 border-t border-border/50 pt-3">
      <div className="flex items-center gap-2 font-mono text-[12px] uppercase tracking-wider">
        <span>{currency}</span>
        {current?.disabled_reason && <Badge className="h-5 border-destructive/30 bg-destructive/15 px-1.5 py-0 text-[10px] text-destructive">{current.disabled_reason}</Badge>}
      </div>
      {current?.disabled_reason && !enabled && (
        <Alert className="border-destructive/30 bg-destructive/5"><AlertDescription className="font-mono text-[12px] text-destructive">Auto-reload stopped: {current.disabled_reason}. Check your card and enable it again.</AlertDescription></Alert>
      )}
      {error && <div className="font-mono text-[12px] text-destructive">{error}</div>}
      <label className="flex items-center gap-2 font-mono text-[12px]"><input type="checkbox" checked={enabled} onChange={e => setEnabled(e.target.checked)} /> Reload automatically</label>
      <div className="flex flex-wrap items-end gap-3">
        {field('When below', threshold, setThreshold)}
        {field(`Reload (min ${config.min_amount})`, amount, setAmount)}
        {field('Monthly cap', cap, setCap)}
        <select value={card} onChange={e => setCard(Number(e.target.value))} className="h-8 border border-border bg-card px-2 font-mono text-[12px]">
          {cards.map(c => <option key={c.id} value={c.id}>{c.brand ?? 'Card'} ...{c.last_four}</option>)}
        </select>
        <Button type="submit" size="sm" className="h-8 border border-brand/40 bg-brand/10 font-mono text-[11px] uppercase tracking-wider text-brand hover:bg-brand/20">Save</Button>
      </div>
    </form>
  )
}

export default function SavedCardsPanel({ config, cards, onChanged }: Props) {
  const [reloads, setReloads] = useState<AutoReload[]>([])
  const [error, setError] = useState<string | null>(null)

  const load = () => fiatApi.autoReload().then(setReloads).catch(() => undefined)
  useEffect(() => { load() }, [cards])

  async function act(run: () => Promise<void>) {
    setError(null)
    try { await run(); onChanged() } catch (e) { setError(e instanceof Error ? e.message : 'Failed') }
  }

  return (
    <div className="panel space-y-4 p-4">
      <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Saved cards</div>
      {error && <div className="font-mono text-[12px] text-destructive">{error}</div>}
      {cards.length === 0 && <div className="font-mono text-[13px] text-muted-foreground">{'>'} NO SAVED CARDS. Tick "Save this card" when topping up.</div>}
      {cards.map(c => (
        <div key={c.id} className="flex items-center gap-3 border-b border-border/50 pb-2 font-mono text-[13px]">
          <span className="uppercase">{c.brand ?? 'Card'}</span>
          <span className="tabular-nums">**** {c.last_four}</span>
          <span className="tabular-nums text-muted-foreground">{c.exp_month && c.exp_year ? `${String(c.exp_month).padStart(2, '0')}/${c.exp_year}` : ''}</span>
          {c.is_default ? <Badge className="h-5 border-brand/30 bg-brand/15 px-1.5 py-0 text-[10px] text-brand">default</Badge>
            : <Button size="sm" variant="ghost" onClick={() => act(() => fiatApi.defaultCard(c.id))} className="h-6 font-mono text-[11px] uppercase">Make default</Button>}
          <Button size="sm" variant="ghost" onClick={() => act(() => fiatApi.deleteCard(c.id))} className="ml-auto h-6 font-mono text-[11px] uppercase text-destructive">Remove</Button>
        </div>
      ))}
      {cards.length > 0 && (
        <div className="space-y-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Auto-reload</div>
          {config.currencies.map(currency => (
            <AutoReloadForm key={currency + (reloads.find(r => r.currency === currency)?.enabled ?? '')} currency={currency} cards={cards} config={config} current={reloads.find(r => r.currency === currency)} onSaved={() => { load(); onChanged() }} />
          ))}
        </div>
      )}
    </div>
  )
}
