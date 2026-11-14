import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { userApi } from '@/api/userApi'
import { formatPrice } from '@/lib/money'
import type { MyProvider, OwnedPrice } from '@/types/userProviders'
import MyPriceDialog from './MyPriceDialog'

interface Props {
  providers: MyProvider[]
  prices: OwnedPrice[]
  onChange: () => void
  onError: (message: string | null) => void
  onSaved?: (message: string) => void
}

export default function MyPricesPanel({ providers, prices, onChange, onError, onSaved }: Props) {
  const [dialogOpen, setDialogOpen] = useState(false)
  const [editing, setEditing] = useState<OwnedPrice | null>(null)

  async function run(action: () => Promise<unknown>) {
    onError(null)
    try { await action(); onChange() } catch (e) { onError(e instanceof Error ? e.message : 'Request failed') }
  }

  function openAdd() {
    onError(null)
    setEditing(null)
    setDialogOpen(true)
  }

  function openEdit(price: OwnedPrice) {
    onError(null)
    setEditing(price)
    setDialogOpen(true)
  }

  return (
    <div className="panel p-4 space-y-3">
      <div className="flex items-center justify-between">
        <div className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Pricing</div>
        <Button onClick={openAdd} disabled={providers.length === 0} className="font-mono text-[11px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">Add pricing</Button>
      </div>
      <MyPriceDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        providers={providers}
        prices={prices}
        editing={editing}
        onSaved={m => { onSaved?.(m); onChange() }}
        onError={onError}
      />
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
              <td className="px-2 py-2.5 text-right space-x-1">
                <Button size="xs" variant="outline" onClick={() => openEdit(p)} className="font-mono text-[10px] uppercase border-border">Edit</Button>
                <Button size="xs" variant="outline" onClick={() => run(() => userApi.deletePrice(p.id))} className="font-mono text-[10px] uppercase border-border text-destructive">Delete</Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
