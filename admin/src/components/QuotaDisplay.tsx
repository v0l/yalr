import type { ProviderHealthEntry, QuotaSnapshot } from '../types'
import { quotaExhausted, quotaUsedPct, quotaWindowLabel, formatResetsIn, worstQuota } from '@/lib/quota'

export interface QuotaDisplayProps {
  health?: ProviderHealthEntry
  className?: string
  /** Called when the indicator is clicked (e.g. to open the detail modal). */
  onClick?: () => void
}

/**
 * Compact quota indicator for subscription providers (Claude Max, ChatGPT).
 * Shows the single most-consumed window — the first that will throttle.
 * When multiple windows exist it becomes a button that opens the detail modal.
 */
export function QuotaDisplay({ health, className, onClick }: QuotaDisplayProps) {
  const all: QuotaSnapshot[] = health?.quotas ?? (health?.quota ? [health.quota] : [])
  const q = worstQuota(all) ?? health?.quota
  if (!q) return null

  const pct = quotaUsedPct(q)
  const resetsIn = formatResetsIn(q.resets_at)
  const exhausted = quotaExhausted(q)
  const warn = exhausted || (pct !== null && pct >= 90)
  const caution = q.status === 'allowed_warning' || (pct !== null && pct >= 75)
  const barColor = warn ? 'var(--destructive)' : caution ? 'var(--warning)' : 'var(--brand)'
  const textColor = warn ? 'text-destructive' : caution ? 'text-warning' : 'text-muted-foreground'

  const usage = pct !== null
    ? `${pct.toFixed(0)}% used`
    : (typeof q.remaining === 'number' ? `${q.remaining.toLocaleString()} left` : null)

  const extra = all.length - 1
  const clickable = !!onClick

  const Wrapper = clickable ? 'button' : 'div'

  return (
    <Wrapper
      type={clickable ? 'button' : undefined}
      onClick={onClick}
      className={`flex flex-col gap-1 text-left w-full ${clickable ? 'cursor-pointer hover:opacity-80 transition-opacity' : ''} ${className ?? ''}`}
      title={clickable ? 'View all quota windows' : undefined}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono text-[9px] uppercase tracking-wider text-muted-foreground">
          {exhausted ? 'QUOTA REACHED' : 'QUOTA'}
          {q.window ? ` · ${quotaWindowLabel(q.window)}` : ''}
          {extra > 0 ? ` +${extra}` : ''}
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
    </Wrapper>
  )
}
