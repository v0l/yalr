import type { QuotaSnapshot } from '../types'

/** A quota window is exhausted when it is (or the next request would be) blocked. */
export function quotaExhausted(q: QuotaSnapshot): boolean {
  const pct = typeof q.used_pct === 'number' ? q.used_pct : null
  return (
    q.status === 'rejected' ||
    q.status === 'exceeded' ||
    q.status === 'rate_limited' ||
    (pct !== null && pct >= 100) ||
    (typeof q.remaining === 'number' && typeof q.limit === 'number' && q.limit > 0 && q.remaining <= 0)
  )
}

/** Percent consumed for a window, derived from used_pct or remaining/limit. */
export function quotaUsedPct(q: QuotaSnapshot): number | null {
  if (typeof q.used_pct === 'number') return Math.min(100, Math.max(0, q.used_pct))
  if (typeof q.remaining === 'number' && typeof q.limit === 'number' && q.limit > 0) {
    return Math.min(100, Math.max(0, ((q.limit - q.remaining) / q.limit) * 100))
  }
  return null
}

/** Severity ranking — exhausted windows always sort highest. */
function severity(q: QuotaSnapshot): number {
  if (quotaExhausted(q)) return Infinity
  return quotaUsedPct(q) ?? 0
}

/** Pick the most-consumed window — the one most likely to throttle next. */
export function worstQuota(quotas?: QuotaSnapshot[] | null): QuotaSnapshot | null {
  if (!quotas || quotas.length === 0) return null
  return quotas.reduce((worst, q) => (severity(q) > severity(worst) ? q : worst))
}

/** Human-friendly window label. */
export function quotaWindowLabel(window?: string): string {
  if (!window) return 'quota'
  switch (window) {
    case '5h': return '5-hour'
    case '7d': return '7-day'
    case '7d_sonnet': return '7-day (Sonnet)'
    case '7d_opus': return '7-day (Opus)'
    default: return window
  }
}

export function formatResetsIn(resetsAt?: number): string | null {
  if (!resetsAt) return null
  const ms = resetsAt - Date.now()
  if (ms <= 0) return 'now'
  const mins = Math.round(ms / 60000)
  if (mins < 60) return `${mins}m`
  const hrs = Math.floor(mins / 60)
  const rem = mins % 60
  if (hrs < 24) return rem ? `${hrs}h ${rem}m` : `${hrs}h`
  const days = Math.floor(hrs / 24)
  const remH = hrs % 24
  return remH ? `${days}d ${remH}h` : `${days}d`
}
