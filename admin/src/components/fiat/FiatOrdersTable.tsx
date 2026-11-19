import { Badge } from '@/components/ui/badge'
import type { FiatOrder } from '@/api/fiat'
import { formatMilli } from '@/lib/money'

function tone(status: string) {
  return status === 'completed' ? 'bg-brand/15 text-brand border-brand/30'
    : status === 'pending' ? 'bg-warning/15 text-warning border-warning/30'
    : 'bg-destructive/15 text-destructive border-destructive/30'
}

export default function FiatOrdersTable({ orders, showUser }: { orders: FiatOrder[]; showUser?: boolean }) {
  const head = 'font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium'
  return (
    <div className="panel overflow-x-auto">
      <table className="w-full table-scan">
        <thead>
          <tr className="border-b border-border/50 text-left">
            {showUser && <th className={head}>User</th>}
            <th className={head}>Amount</th>
            <th className={head}>Kind</th>
            <th className={head}>Status</th>
            <th className={head}>Created</th>
            <th className={head}>Completed</th>
          </tr>
        </thead>
        <tbody>
          {orders.length === 0 ? (
            <tr><td colSpan={showUser ? 6 : 5} className="px-4 py-16 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO TOP-UPS</td></tr>
          ) : orders.map(o => (
            <tr key={o.public_id} className="border-b border-border/50 hover:bg-surface">
              {showUser && <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{o.username}</td>}
              <td className="px-4 py-3 font-mono text-[13px] tabular-nums">{formatMilli(o.amount_milli, o.currency)}</td>
              <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{o.kind === 'auto_reload' ? 'auto-reload' : 'top-up'}</td>
              <td className="px-4 py-3"><Badge className={`h-5 px-1.5 py-0 font-mono text-[10px] uppercase ${tone(o.status)}`} title={o.failure_reason ?? undefined}>{o.status}</Badge></td>
              <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{new Date(o.created_at).toLocaleString()}</td>
              <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{o.completed_at ? new Date(o.completed_at).toLocaleString() : '-'}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
