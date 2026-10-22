import { useCallback, useEffect, useMemo, useState } from 'react'
import { RefreshCwIcon } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { marketApi, type MarketResult, type PeerPrice } from '@/api/market'
import type { ModelPricingEntry } from '@/types'

const TH = 'px-3 py-2 font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground'
const TD = 'px-3 py-2 font-mono text-[12px] tabular-nums'

function sats(value: number): string {
  if (!value) return '-'
  return value >= 100 ? Math.round(value).toLocaleString() : value.toLocaleString(undefined, { maximumFractionDigits: 2 })
}

function versus(peer: number, ours: number): string {
  if (!peer || !ours) return '-'
  const pct = Math.round(((peer - ours) / ours) * 100)
  return pct === 0 ? '0%' : `${pct > 0 ? '+' : ''}${pct}%`
}

function median(values: number[]): number {
  if (values.length === 0) return 0
  const sorted = [...values].sort((a, b) => a - b)
  const mid = Math.floor(sorted.length / 2)
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2
}

function ourPrice(entry: ModelPricingEntry | null): PeerPrice | null {
  if (!entry || entry.is_free || entry.price_per_1m_input_sats == null) return null
  const input = entry.price_per_1m_input_sats
  return {
    node: 'You',
    model: entry.model_name,
    input,
    output: entry.price_per_1m_output_sats ?? 0,
    cache_read: entry.price_per_1m_cache_read_sats ?? input,
    cache_write: entry.price_per_1m_cache_write_sats ?? input,
    request: entry.price_per_request_sats ?? 0,
  }
}

export default function CompareDialog({
  open,
  onOpenChange,
  entry,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  entry: ModelPricingEntry | null
}) {
  const [query, setQuery] = useState('')
  const [data, setData] = useState<MarketResult | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(async (q: string, refresh: boolean) => {
    setLoading(true)
    setError(null)
    try {
      setData(await marketApi.market(q, refresh))
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not load the market')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    if (!open || !entry) return
    setQuery(entry.model_name)
    setData(null)
    load(entry.model_name, false)
  }, [open, entry, load])

  const ours = ourPrice(entry)
  const peers = data?.results ?? []
  const rows = useMemo(() => {
    const all = ours && peers.some(p => p.model === ours.model && p.node === ours.node) ? peers : ours ? [...peers, ours] : peers
    return [...all].sort((a, b) => a.input - b.input)
  }, [peers, ours])

  const inputs = peers.map(p => p.input).filter(v => v > 0)
  const outputs = peers.map(p => p.output).filter(v => v > 0)
  const cheaper = ours ? inputs.filter(v => v < ours.input).length : 0

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto border-border bg-card sm:max-w-5xl">
        <DialogHeader>
          <DialogTitle className="font-display text-xl tracking-[0.04em]">COMPARE PRICING</DialogTitle>
          <DialogDescription className="font-mono text-[12px] text-muted-foreground">
            Sats per 1M tokens on other Routstr nodes, read from their public model lists. Matches by model name, so check the model before trusting a number.
          </DialogDescription>
        </DialogHeader>

        <form
          className="flex gap-2"
          onSubmit={e => { e.preventDefault(); load(query, false) }}
        >
          <Input value={query} onChange={e => setQuery(e.target.value)} placeholder="model name" className="border-border bg-surface font-mono text-[12px]" />
          <Button type="submit" variant="outline" disabled={loading} className="border-border font-mono text-[11px] uppercase tracking-wider">Search</Button>
          <Button type="button" variant="outline" disabled={loading} onClick={() => load(query, true)} title="Refetch every node" className="border-border">
            <RefreshCwIcon className={`size-3.5 ${loading ? 'animate-spin' : ''}`} />
          </Button>
        </form>

        {error && <p className="font-mono text-[12px] text-destructive">! {error}</p>}

        {loading && !data ? <Skeleton className="h-40 bg-secondary" /> : data && (
          <>
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              <Stat label="Nodes answered" value={`${data.nodes_ok} of ${data.nodes}`} />
              <Stat label="Matches" value={String(peers.length)} />
              <Stat label="Median input" value={sats(median(inputs))} detail={ours ? versus(median(inputs), ours.input) + ' vs you' : undefined} />
              <Stat label="Median output" value={sats(median(outputs))} detail={ours ? versus(median(outputs), ours.output) + ' vs you' : undefined} />
            </div>
            {ours && inputs.length > 0 && (
              <p className="font-mono text-[12px] text-muted-foreground">
                {cheaper === 0 ? 'You are the cheapest on input.' : `${cheaper} of ${inputs.length} listings are cheaper than you on input.`}
              </p>
            )}
            <div className="overflow-x-auto border border-border">
              <table className="w-full">
                <thead>
                  <tr className="border-b border-border/50 text-left">
                    <th className={TH}>Node</th>
                    <th className={TH}>Model</th>
                    <th className={`${TH} text-right`}>In</th>
                    <th className={`${TH} text-right`}>vs you</th>
                    <th className={`${TH} text-right`}>Out</th>
                    <th className={`${TH} text-right`}>vs you</th>
                    <th className={`${TH} text-right`}>Cache rd</th>
                    <th className={`${TH} text-right`}>Cache wr</th>
                    <th className={`${TH} text-right`}>Req</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.length === 0 ? (
                    <tr><td colSpan={9} className="px-3 py-8 text-center font-mono text-[12px] text-muted-foreground">No listings match</td></tr>
                  ) : rows.map(row => {
                    const mine = row === ours
                    return (
                      <tr key={`${row.node}/${row.model}`} className={`border-b border-border/50 ${mine ? 'bg-brand/10 text-brand' : ''}`}>
                        <td className={`${TD} max-w-[14rem] truncate`}>{row.node}</td>
                        <td className={`${TD} max-w-[16rem] truncate`}>{row.model}</td>
                        <td className={`${TD} text-right`}>{sats(row.input)}</td>
                        <td className={`${TD} text-right text-muted-foreground`}>{mine || !ours ? '' : versus(row.input, ours.input)}</td>
                        <td className={`${TD} text-right`}>{sats(row.output)}</td>
                        <td className={`${TD} text-right text-muted-foreground`}>{mine || !ours ? '' : versus(row.output, ours.output)}</td>
                        <td className={`${TD} text-right`}>{sats(row.cache_read)}</td>
                        <td className={`${TD} text-right`}>{sats(row.cache_write)}</td>
                        <td className={`${TD} text-right`}>{sats(row.request)}</td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}

function Stat({ label, value, detail }: { label: string; value: string; detail?: string }) {
  return (
    <div className="border border-border bg-surface p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">{label}</div>
      <div className="font-mono text-[16px] tabular-nums text-foreground">{value}</div>
      {detail && <div className="font-mono text-[10px] text-muted-foreground">{detail}</div>}
    </div>
  )
}
