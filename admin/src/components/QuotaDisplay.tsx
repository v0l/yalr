import type { ProviderHealthEntry } from '../types'

export interface QuotaDisplayProps {
  health?: ProviderHealthEntry
  className?: string
}

function formatResetsIn(resetsAt?: number): string | null {
  if (!resetsAt) return null
  const ms = resetsAt - Date.now()
  if (ms <= 0) return 'now'
  const mins = Math.round(ms / 60000)
  if (mins < 60) return `${mins}m`
  const hrs = Math.floor(mins / 60)
  const rem = mins % 60
  if (hrs < 24) return rem ? `${hrs}h ${rem}m` : `${hrs}h`
  const days = Math.floor(hrs / 24)
  return `${days}d`
}

/**
 * Compact quota indicator for subscription providers (Claude Max, ChatGPT).
 * Shows a used-% bar, the consumption figure, and when the window resets.
 */
export function QuotaDisplay({ health, className }: QuotaDisplayProps) {
  const q = health?.quota
  if (!q) return null

  const pct = typeof q.used_pct === 'number' ? Math.min(100, Math.max(0, q.used_pct)) : null
  const resetsIn = formatResetsIn(q.resets_at)
  const exhausted = q.status === 'rejected' || (pct !== null && pct >= 100) || (typeof q.remaining === 'number' && q.remaining <= 0)
  const warn = exhausted || (pct !== null && pct >= 90)
  const caution = q.status === 'allowed_warning' || (pct !== null && pct >= 75)
  const barColor = warn ? 'var(--destructive)' : caution ? 'var(--warning)' : 'var(--brand)'
  const textColor = warn ? 'text-destructive' : caution ? 'text-warning' : 'text-muted-foreground'

  const usage = pct !== null
    ? `${pct.toFixed(0)}% used`
    : (typeof q.remaining === 'number' ? `${q.remaining.toLocaleString()} left` : null)

  return (
    <div className={`flex flex-col gap-1 ${className ?? ''}`}>
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
          {exhausted ? 'QUOTA REACHED' : 'QUOTA'}{q.window ? ` · ${q.window}` : ''}
        </span>
        {usage && <span className={`font-mono text-[10px] tabular-nums ${textColor}`}>{usage}</span>}
      </div>
      {pct !== null && (
        <div className="h-1 w-full bg-surface overflow-hidden">
          <div className="h-full transition-all" style={{ width: `${pct}%`, backgroundColor: barColor }} />
        </div>
      )}
      {resetsIn && (
        <span className="font-mono text-[9px] tabular-nums text-muted-foreground">
          {exhausted ? 'resets in' : 'resets'} {resetsIn}
        </span>
      )}
    </div>
  )
}
