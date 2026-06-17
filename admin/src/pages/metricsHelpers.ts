import type { WsProviderMetrics, CurrencyAmount, MetricsUser, QuotaSnapshot } from '../types'
import { formatBalance } from '@/lib/utils'
import { worstQuota } from '@/lib/quota'

export const MAXL = 500, MAXA = 500

export function fmtNum(n: number): string {
  return n.toLocaleString('en-US')
}

export function has<T extends string>(o: unknown, k: T): o is Record<T, unknown> {
  return typeof o === 'object' && o !== null && k in o
}

type EK = 'ok' | 'warn' | 'err' | 'info'

export function fmt(e: WsProviderMetrics['event']): { label: string; value: string; kind: EK } {
  if (e === 'Success') return { label: '✓', value: 'OK', kind: 'ok' }
  if (has(e, 'TTFT')) return { label: 'TTFT', value: `${fmtNum(e.TTFT)}ms`, kind: 'info' }
  if (has(e, 'OutputTokensPerSecond')) return { label: 'O/s', value: (e.OutputTokensPerSecond as number).toFixed(1), kind: 'info' }
  if (has(e, 'InputTokensPerSecond')) return { label: 'I/s', value: (e.InputTokensPerSecond as number).toFixed(1), kind: 'info' }
  if (has(e, 'TotalLatency')) return { label: 'LAT', value: `${fmtNum(e.TotalLatency)}ms`, kind: 'info' }
  if (has(e, 'InputTokens')) return { label: 'IN', value: fmtNum(e.InputTokens), kind: 'info' }
  if (has(e, 'OutputTokens')) return { label: 'OUT', value: fmtNum(e.OutputTokens), kind: 'info' }
  if (has(e, 'Failure')) {
    const f = e.Failure as { error_message: string }
    return { label: 'FAIL', value: f.error_message, kind: 'err' }
  }
  if (has(e, 'ProviderLoad')) {
    const l = e.ProviderLoad as { in_flight: number; max_concurrency: number | null }
    return { label: 'LOAD', value: `${l.in_flight}${l.max_concurrency ? `/${l.max_concurrency}` : ''}`, kind: l.in_flight > 0 ? 'warn' : 'ok' }
  }
  if (has(e, 'Balance')) {
    const b = e.Balance as CurrencyAmount
    return { label: 'BAL', value: formatBalance(b.amount, b.currency), kind: 'info' }
  }
  if (has(e, 'Quota')) {
    const all = (e.Quota ?? []) as QuotaSnapshot[]
    const q = worstQuota(all)
    if (!q) return { label: 'QUOTA', value: '—', kind: 'info' }
    const usage = typeof q.used_pct === 'number'
      ? `${q.used_pct.toFixed(0)}% used`
      : (typeof q.remaining === 'number' ? `${q.remaining.toLocaleString()} left` : (q.status ?? '?'))
    const v = q.window ? `${q.window} · ${usage}` : usage
    const exhausted = q.status === 'rejected' || q.status === 'exceeded' || q.status === 'rate_limited' || (typeof q.used_pct === 'number' && q.used_pct >= 100)
    return { label: 'QUOTA', value: v, kind: exhausted ? 'err' : (q.status === 'allowed_warning' ? 'warn' : 'info') }
  }
  return { label: '?', value: JSON.stringify(e), kind: 'info' }
}

export function fmtUser(u?: MetricsUser | null): string {
  if (!u) return ''
  const parts: string[] = []
  if (u.name) parts.push(u.name)
  if (u.api_key_name) parts.push(`🔑 ${u.api_key_name}`)
  if (parts.length === 0 && u.id) parts.push(`ID:${u.id}`)
  return parts.join(' · ')
}

export function eventKind(ev: WsProviderMetrics): 'success' | 'failure' | 'load' | 'balance' | 'info' | 'other' {
  if (ev.event === 'Success') return 'success'
  if (has(ev.event, 'Failure')) return 'failure'
  if (has(ev.event, 'ProviderLoad')) return 'load'
  if (has(ev.event, 'Balance') || has(ev.event, 'Quota')) return 'balance'
  if (has(ev.event, 'TTFT') || has(ev.event, 'OutputTokensPerSecond') || has(ev.event, 'InputTokensPerSecond') || has(ev.event, 'TotalLatency') || has(ev.event, 'InputTokens') || has(ev.event, 'OutputTokens')) return 'info'
  return 'other'
}

export type EventKind = ReturnType<typeof eventKind>

export function pct(v: number[], p: number): number | null {
  if (v.length === 0) return null
  const s = [...v].sort((a, b) => a - b)
  return s[Math.round(p * (s.length - 1))]
}
