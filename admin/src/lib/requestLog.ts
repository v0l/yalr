import type { RequestLogSummary } from '../types'

export function formatMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined) return '-'
  if (ms < 1000) return `${ms}ms`
  return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)}s`
}

export function formatTokens(n: number | null | undefined): string {
  if (n === null || n === undefined) return '-'
  if (n < 1000) return String(n)
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`
  return `${(n / 1_000_000).toFixed(2)}M`
}

export function formatBytes(n: number | null | undefined): string {
  if (n === null || n === undefined) return '-'
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / 1024 / 1024).toFixed(2)} MB`
}

export function formatCost(msat: number | null | undefined): string {
  if (msat === null || msat === undefined) return '-'
  if (msat === 0) return '0'
  const sats = msat / 1000
  return sats < 10 ? sats.toFixed(sats < 1 ? 3 : 2) : Math.round(sats).toLocaleString()
}

export const ENDPOINTS = [
  '/v1/chat/completions',
  '/v1/responses',
  '/v1/systemone',
  '/v1/audio/transcriptions',
  '/v1/audio/translations',
  '/v1/audio/speech',
  '/v1/images/generations',
  '/v1/images/edits',
  '/v1/images/variations',
] as const

export function endpointLabel(endpoint: string): string {
  return endpoint.replace(/^\/v1\//, '')
}

export function formatTime(ms: number): string {
  const d = new Date(ms)
  const pad = (v: number, w = 2) => String(v).padStart(w, '0')
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

export function cacheHitRate(row: Pick<RequestLogSummary, 'prompt_tokens' | 'cached_tokens'>): number | null {
  if (!row.prompt_tokens || row.cached_tokens === null) return null
  return row.cached_tokens / row.prompt_tokens
}

export function firstTokenMs(row: Pick<RequestLogSummary, 'first_thinking_ms' | 'first_text_ms'>): number | null {
  const values = [row.first_thinking_ms, row.first_text_ms].filter((v): v is number => v !== null)
  return values.length ? Math.min(...values) : null
}

export const STATUSES = ['ok', 'error', 'rejected', 'aborted'] as const

export function statusTone(status: string): string {
  if (status === 'ok') return 'bg-brand/15 text-brand border-brand/30'
  if (status === 'error') return 'bg-destructive/15 text-destructive border-destructive/30'
  return 'bg-warning/15 text-warning border-warning/30'
}
