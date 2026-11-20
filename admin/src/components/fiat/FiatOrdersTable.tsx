import { Badge } from '@/components/ui/badge'
import type { FiatOrder } from '@/api/fiat'
import { formatMilli } from '@/lib/money'
import { paymentKind, paymentMethod } from '@/lib/gatewayPayment'

function tone(status: string) {
  return status === 'completed' ? 'bg-brand/15 text-brand border-brand/30'
    : status === 'pending' ? 'bg-warning/15 text-warning border-warning/30'
    : status === 'failed' ? 'bg-destructive/15 text-destructive border-destructive/30'
    : 'bg-secondary text-muted-foreground border-border'
}

function when(value: string | null) {
  if (!value) return '-'
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
}

export default function FiatOrdersTable({ orders, showUser, empty = 'NO TOP-UPS' }: { orders: FiatOrder[]; showUser?: boolean; empty?: string }) {
  const head = 'font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium'
  const cols = showUser ? 9 : 8
  return (
    <div className="panel overflow-x-auto">
      <table className="w-full table-scan">
        <thead>
          <tr className="border-b border-border/50 text-left">
            {showUser && <th className={head}>User</th>}
            <th className={head}>Amount</th>
            <th className={head}>Kind</th>
            <th className={head}>Method</th>
            <th className={head}>Status</th>
            <th className={head}>Created</th>
            <th className={head}>Expires</th>
            <th className={head}>Completed</th>
            <th className={head}>Reference</th>
          </tr>
        </thead>
        <tbody>
          {orders.length === 0 ? (
            <tr><td colSpan={cols} className="px-4 py-16 text-center font-mono text-[13px] text-muted-foreground">{'>'} {empty}</td></tr>
          ) : orders.map(o => (
            <tr key={o.public_id} className="border-b border-border/50 hover:bg-surface">
              {showUser && <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{o.username ?? `ID:${o.user_id}`}</td>}
              <td className="px-4 py-3 font-mono text-[13px] tabular-nums">{formatMilli(o.amount_milli, o.currency)}</td>
              <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{paymentKind(o)}</td>
              <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{paymentMethod(o)}</td>
              <td className="px-4 py-3">
                <Badge className={`h-5 px-1.5 py-0 font-mono text-[10px] uppercase ${tone(o.status)}`} title={o.failure_reason ?? undefined}>{o.status}</Badge>
                {o.status === 'failed' && o.failure_reason && <div className="mt-1 max-w-48 truncate font-mono text-[10px] text-destructive" title={o.failure_reason}>{o.failure_reason}</div>}
              </td>
              <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{when(o.created_at)}</td>
              <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{when(o.expires_at)}</td>
              <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{when(o.completed_at)}</td>
              <td className="max-w-40 truncate px-4 py-3 font-mono text-[11px] text-muted-foreground" title={o.payment_hash ?? o.public_id}>
                {o.public_id}{o.payment_hash ? ` ${o.payment_hash.slice(0, 12)}…` : ''}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
