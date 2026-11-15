import { useEffect, useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { Input } from '@/components/ui/input'
import { publicApi, type PublicModel } from '@/api/publicApi'
import PublicLayout from '@/layout/PublicLayout'
import { formatTokens, rangeLines } from '@/lib/publicFormat'

type SortKey = 'id' | 'provider_count' | 'price' | 'context_length' | 'requests_24h'

function lowestInput(m: PublicModel): number {
  return m.price_ranges.length ? Math.min(...m.price_ranges.map(r => r.min_input)) : Number.POSITIVE_INFINITY
}

const columns: { key: SortKey; label: string; className?: string }[] = [
  { key: 'id', label: 'Model' },
  { key: 'provider_count', label: 'Providers', className: 'text-right' },
  { key: 'price', label: 'Price per 1M tokens' },
  { key: 'context_length', label: 'Context', className: 'text-right' },
  { key: 'requests_24h', label: '24h requests', className: 'hidden text-right md:table-cell' },
]

export default function Models() {
  const [models, setModels] = useState<PublicModel[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>({ key: 'id', desc: false })
  const [showAll, setShowAll] = useState(false)

  useEffect(() => {
    setModels(null)
    publicApi.models(showAll).then(setModels).catch(e => setError(e instanceof Error ? e.message : 'Failed to load'))
  }, [showAll])

  const rows = useMemo(() => {
    const needle = query.trim().toLowerCase()
    const found = (models ?? []).filter(m => !needle || m.id.toLowerCase().includes(needle) || m.input_modalities.some(x => x.includes(needle)))
    const value = (m: PublicModel): number | string => {
      if (sort.key === 'id') return m.id
      if (sort.key === 'price') return lowestInput(m)
      return m[sort.key] ?? 0
    }
    return [...found].sort((a, b) => {
      const x = value(a)
      const y = value(b)
      const order = typeof x === 'string' ? x.localeCompare(y as string) : (x as number) - (y as number)
      return sort.desc ? -order : order
    })
  }, [models, query, sort])

  function toggle(key: SortKey) {
    setSort(s => ({ key, desc: s.key === key ? !s.desc : false }))
  }

  return (
    <PublicLayout>
      <div className="space-y-4 p-4 sm:p-5">
        <div>
          <h1 className="mb-1 font-display text-[28px] leading-none tracking-[0.04em] text-foreground">MODELS</h1>
          <p className="font-mono text-[13px] text-muted-foreground">
            Every model on the router, the providers selling it, what they charge and how they have been performing.
          </p>
        </div>
        <Input
          value={query}
          onChange={e => setQuery(e.target.value)}
          placeholder="Search models or modalities"
          className="h-9 max-w-sm border-border bg-surface font-mono text-[13px]"
        />
        <label className="flex items-center gap-2 font-mono text-[12px] text-muted-foreground">
          <input type="checkbox" checked={showAll} onChange={e => setShowAll(e.target.checked)} />
          Show all pass-through models, including unpriced ones with no traffic
        </label>
        {error && <div className="panel p-3 font-mono text-[12px] text-destructive">{error}</div>}
        <div className="panel overflow-x-auto">
          <table className="w-full font-mono text-[12px] tabular-nums">
            <thead>
              <tr className="border-b border-border text-left text-[10px] uppercase tracking-wider text-muted-foreground">
                {columns.map(c => (
                  <th key={c.key} className={`px-3 py-2 font-normal ${c.className ?? ''}`}>
                    <button onClick={() => toggle(c.key)} className={sort.key === c.key ? 'text-brand' : 'hover:text-foreground'}>
                      {c.label}{sort.key === c.key ? (sort.desc ? ' v' : ' ^') : ''}
                    </button>
                  </th>
                ))}
                <th className="hidden px-3 py-2 font-normal lg:table-cell">Modalities</th>
              </tr>
            </thead>
            <tbody>
              {models === null && !error && <tr><td colSpan={6} className="p-6 text-center text-muted-foreground">LOADING...</td></tr>}
              {models !== null && rows.length === 0 && <tr><td colSpan={6} className="p-6 text-center text-muted-foreground">{'>'} NO MODELS</td></tr>}
              {rows.map(m => (
                <tr key={m.id} className="border-b border-border/50 hover:bg-surface">
                  <td className="px-3 py-2"><Link to={`/models/${m.id}`} className="text-foreground hover:text-brand">{m.id}</Link></td>
                  <td className="px-3 py-2 text-right">{m.provider_count}</td>
                  <td className="px-3 py-2">
                    {rangeLines(m.price_ranges).map(l => (
                      <div key={l.currency} className="whitespace-nowrap">
                        <span className="text-muted-foreground">in</span> {l.input} <span className="text-muted-foreground">out</span> {l.output}
                      </div>
                    ))}
                    {m.price_ranges.length === 0 && '-'}
                  </td>
                  <td className="px-3 py-2 text-right">{formatTokens(m.context_length)}</td>
                  <td className="hidden px-3 py-2 text-right md:table-cell">{m.requests_24h.toLocaleString()}</td>
                  <td className="hidden px-3 py-2 lg:table-cell">
                    <div className="flex flex-wrap gap-1">
                      {m.input_modalities.map(x => (
                        <span key={x} className="inline-flex h-5 items-center border border-border px-1.5 text-[10px] uppercase text-muted-foreground">{x}</span>
                      ))}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </PublicLayout>
  )
}
