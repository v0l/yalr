import { Badge } from '@/components/ui/badge'
import type { Model } from '@/types'

function tokens(n: number | undefined): string {
  if (!n) return '-'
  return n >= 1_000_000 ? `${(n / 1_000_000).toFixed(n % 1_000_000 ? 1 : 0)}M` : `${Math.round(n / 1000)}k`
}

function sats(n: number): string {
  return n.toLocaleString()
}

export default function ModelPriceTable({ models }: { models: Model[] }) {
  return (
    <table className="table-scan w-full">
      <thead>
        <tr className="border-b border-border/50 text-left">
          <th className="px-4 py-2.5 font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Model</th>
          <th className="px-4 py-2.5 text-right font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Input / 1M</th>
          <th className="px-4 py-2.5 text-right font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Output / 1M</th>
          <th className="px-4 py-2.5 text-right font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Cache read / 1M</th>
          <th className="px-4 py-2.5 text-right font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Cache write / 1M</th>
          <th className="px-4 py-2.5 text-right font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Per request</th>
          <th className="px-4 py-2.5 text-right font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">Context</th>
        </tr>
      </thead>
      <tbody>
        {models.map(m => (
          <tr key={m.id} className="border-b border-border/50">
            <td className="px-4 py-2.5 font-mono text-[12px] text-foreground">
              <div>{m.id}</div>
              {m.architecture?.input_modalities && m.architecture.input_modalities.length > 0 && (
                <div className="mt-1 flex flex-wrap gap-1">
                  {m.architecture.input_modalities.map(i => (
                    <Badge key={i} className="h-5 border-border bg-secondary px-1.5 py-0 font-mono text-[10px] text-muted-foreground">{i}</Badge>
                  ))}
                </div>
              )}
            </td>
            <td className="px-4 py-2.5 text-right font-mono text-[12px] tabular-nums text-foreground">{m.pricing ? `${sats(m.pricing.prompt)} sats` : '-'}</td>
            <td className="px-4 py-2.5 text-right font-mono text-[12px] tabular-nums text-foreground">{m.pricing ? `${sats(m.pricing.completion)} sats` : '-'}</td>
            <td className="px-4 py-2.5 text-right font-mono text-[12px] tabular-nums text-foreground">{m.pricing ? `${sats(m.pricing.cache_read)} sats` : '-'}</td>
            <td className="px-4 py-2.5 text-right font-mono text-[12px] tabular-nums text-foreground">{m.pricing ? `${sats(m.pricing.cache_write)} sats` : '-'}</td>
            <td className="px-4 py-2.5 text-right font-mono text-[12px] tabular-nums text-foreground">{m.pricing ? `${sats(m.pricing.request)} sats` : '-'}</td>
            <td className="px-4 py-2.5 text-right font-mono text-[12px] tabular-nums text-muted-foreground">{tokens(m.context_length)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}
