import type { QuotaSnapshot } from '../types'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog'
import { quotaExhausted, quotaUsedPct, quotaWindowLabel, formatResetsIn } from '@/lib/quota'

export interface QuotaDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  providerName: string
  quotas: QuotaSnapshot[]
}

function statusLabel(q: QuotaSnapshot): string {
  if (quotaExhausted(q)) return 'EXHAUSTED'
  switch (q.status) {
    case 'allowed_warning': return 'WARNING'
    case 'allowed': return 'OK'
    default: return q.status ? q.status.toUpperCase() : 'OK'
  }
}

function QuotaRow({ q }: { q: QuotaSnapshot }) {
  const pct = quotaUsedPct(q)
  const exhausted = quotaExhausted(q)
  const warn = exhausted || (pct !== null && pct >= 90)
  const caution = q.status === 'allowed_warning' || (pct !== null && pct >= 75)
  const barColor = warn ? 'var(--destructive)' : caution ? 'var(--warning)' : 'var(--brand)'
  const textColor = warn ? 'text-destructive' : caution ? 'text-warning' : 'text-muted-foreground'
  const resetsIn = formatResetsIn(q.resets_at)

  const detail = pct !== null
    ? `${pct.toFixed(0)}% used`
    : (typeof q.remaining === 'number' && typeof q.limit === 'number'
        ? `${q.remaining.toLocaleString()} / ${q.limit.toLocaleString()} left`
        : (typeof q.remaining === 'number' ? `${q.remaining.toLocaleString()} left` : null))

  return (
    <div className="panel p-3 flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono text-[12px] font-bold">{quotaWindowLabel(q.window)}</span>
        <span className={`font-mono text-[9px] uppercase tracking-wider px-1.5 py-0 h-5 flex items-center ${textColor}`}>
          {statusLabel(q)}
        </span>
      </div>
      {pct !== null && (
        <div className="h-1.5 w-full bg-surface overflow-hidden">
          <div className="h-full transition-all" style={{ width: `${pct}%`, backgroundColor: barColor }} />
        </div>
      )}
      <div className="flex items-center justify-between gap-2">
        {detail && <span className={`font-mono text-[10px] tabular-nums ${textColor}`}>{detail}</span>}
        {resetsIn && (
          <span className="font-mono text-[9px] tabular-nums text-muted-foreground">
            {exhausted ? 'resets in' : 'resets'} {resetsIn}
          </span>
        )}
      </div>
    </div>
  )
}

/**
 * Shows every enforced quota window for a provider (e.g. Anthropic's 5h + 7d
 * subscription limits, or token/request limits). The provider card surfaces only
 * the worst one; this modal breaks them all out.
 */
export function QuotaDialog({ open, onOpenChange, providerName, quotas }: QuotaDialogProps) {
  // Most-consumed first so the "first to stop" window is at the top.
  const sorted = [...quotas].sort((a, b) => (quotaUsedPct(b) ?? 0) - (quotaUsedPct(a) ?? 0))

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="font-mono text-sm uppercase tracking-wider">
            {providerName} · Quota
          </DialogTitle>
          <DialogDescription className="font-mono text-[11px]">
            All enforced rate-limit windows. The card shows whichever is closest to its limit.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-2">
          {sorted.length === 0 ? (
            <span className="font-mono text-[11px] text-muted-foreground">No quota data available.</span>
          ) : (
            sorted.map((q, i) => <QuotaRow key={`${q.window ?? 'window'}-${i}`} q={q} />)
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
