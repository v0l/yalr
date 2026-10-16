import { useCallback, useEffect, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { api } from '../api/client'
import type { RequestLogSummary } from '../types'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { cacheHitRate, firstTokenMs, formatMs, formatTime, formatTokens, STATUSES, statusTone } from '@/lib/requestLog'
import { RefreshCwIcon, LinkIcon } from 'lucide-react'

const PAGE_SIZE = 100

const th = 'font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-3 py-2.5 font-medium whitespace-nowrap'
const td = 'px-3 py-2 font-mono text-[12px] tabular-nums whitespace-nowrap'

export default function RequestLog() {
  const navigate = useNavigate()
  const [params, setParams] = useSearchParams()
  const model = params.get('model') ?? ''
  const status = params.get('status') ?? ''
  const [modelInput, setModelInput] = useState(model)
  const [rows, setRows] = useState<RequestLogSummary[]>([])
  const [loading, setLoading] = useState(true)
  const [loadingMore, setLoadingMore] = useState(false)
  const [hasMore, setHasMore] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const [reloadKey, setReloadKey] = useState(0)

  const fetchPage = useCallback(
    (before?: number) => api.getRequestLog({ limit: PAGE_SIZE, before, model, status }),
    [model, status],
  )

  useEffect(() => {
    let cancelled = false
    fetchPage()
      .then(page => {
        if (cancelled) return
        setRows(page)
        setHasMore(page.length === PAGE_SIZE)
        setError(null)
      })
      .catch(e => { if (!cancelled) setError(e instanceof Error ? e.message : String(e)) })
      .finally(() => { if (!cancelled) setLoading(false) })
    return () => { cancelled = true }
  }, [fetchPage, reloadKey])

  function refresh() {
    setLoading(true)
    setReloadKey(k => k + 1)
  }

  async function loadMore() {
    const last = rows[rows.length - 1]
    if (!last) return
    setLoadingMore(true)
    try {
      const page = await fetchPage(last.id)
      setRows(prev => [...prev, ...page])
      setHasMore(page.length === PAGE_SIZE)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoadingMore(false)
    }
  }

  function setFilter(key: 'model' | 'status', value: string) {
    setLoading(true)
    const next = new URLSearchParams(params)
    if (value) next.set(key, value)
    else next.delete(key)
    setParams(next, { replace: true })
  }

  return (
    <div className="space-y-4 p-4 sm:p-5">
      <div className="flex items-end justify-between gap-4 flex-wrap">
        <div>
          <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground leading-none mb-1">REQUEST LOG</h1>
          <p className="font-mono text-[13px] text-muted-foreground">Every routed request, newest first</p>
        </div>
        <div className="flex items-center gap-2 flex-wrap">
          <form
            onSubmit={e => { e.preventDefault(); setFilter('model', modelInput.trim()) }}
            className="flex items-center gap-2"
          >
            <Input
              value={modelInput}
              onChange={e => setModelInput(e.target.value)}
              placeholder="model"
              className="h-8 w-44 font-mono text-[12px] bg-surface border-border"
            />
          </form>
          <div className="flex border border-border">
            {['', ...STATUSES].map(s => (
              <button
                key={s || 'all'}
                onClick={() => setFilter('status', s)}
                className={cn(
                  'px-2.5 h-8 font-mono text-[10px] uppercase tracking-wider transition-colors',
                  status === s ? 'bg-brand/10 text-brand' : 'text-muted-foreground hover:text-foreground hover:bg-surface'
                )}
              >
                {s || 'all'}
              </button>
            ))}
          </div>
          <Button variant="outline" size="sm" onClick={refresh} className="h-8 font-mono text-[11px] border-border">
            <RefreshCwIcon className={cn('size-3.5', loading && 'animate-spin')} />
          </Button>
        </div>
      </div>

      {error && (
        <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono text-[13px]">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      <div className="panel">
        <div className="overflow-x-auto">
          <table className="w-full table-scan">
            <thead>
              <tr className="border-b border-border/50 text-left">
                <th className={th}>Time</th>
                <th className={th}>Model</th>
                <th className={th}>Provider</th>
                <th className={th}>Status</th>
                <th className={cn(th, 'text-right')}>Headers</th>
                <th className={cn(th, 'text-right')}>First token</th>
                <th className={cn(th, 'text-right')}>Total</th>
                <th className={cn(th, 'text-right')}>In</th>
                <th className={cn(th, 'text-right')}>Cache</th>
                <th className={cn(th, 'text-right')}>Out</th>
                <th className={cn(th, 'text-right')}>Msgs</th>
                <th className={th}>User</th>
              </tr>
            </thead>
            <tbody>
              {loading && rows.length === 0 ? (
                Array.from({ length: 8 }).map((_, i) => (
                  <tr key={i} className="border-b border-border/50">
                    <td colSpan={12} className="px-3 py-2"><Skeleton className="h-4 bg-secondary" /></td>
                  </tr>
                ))
              ) : rows.length === 0 ? (
                <tr><td colSpan={12} className="px-4 py-16 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO REQUESTS LOGGED</td></tr>
              ) : rows.map(row => {
                const hit = cacheHitRate(row)
                return (
                  <tr
                    key={row.id}
                    onClick={() => navigate(`/request-log/${row.id}`)}
                    className="border-b border-border/50 hover:bg-surface transition-colors cursor-pointer"
                  >
                    <td className={cn(td, 'text-muted-foreground')}>{formatTime(row.started_at_ms)}</td>
                    <td className={td}>
                      <span className="text-foreground">{row.model}</span>
                      {row.stream && <span className="ml-1.5 text-[10px] text-muted-foreground">SSE</span>}
                    </td>
                    <td className={cn(td, 'text-muted-foreground')}>
                      {row.provider ?? '-'}
                      {row.attempts > 1 && <span className="ml-1.5 text-warning">×{row.attempts}</span>}
                    </td>
                    <td className={td} title={row.error ?? undefined}>
                      <Badge className={cn('font-mono text-[10px] px-1.5 py-0 h-5 uppercase', statusTone(row.status))}>{row.status}</Badge>
                    </td>
                    <td className={cn(td, 'text-right text-muted-foreground')}>{formatMs(row.headers_ms)}</td>
                    <td className={cn(td, 'text-right')}>{formatMs(firstTokenMs(row))}</td>
                    <td className={cn(td, 'text-right')}>{formatMs(row.total_ms)}</td>
                    <td className={cn(td, 'text-right')}>{formatTokens(row.prompt_tokens)}</td>
                    <td className={cn(td, 'text-right', hit === null ? 'text-muted-foreground' : hit >= 0.8 ? 'text-brand' : hit >= 0.3 ? 'text-warning' : 'text-destructive')}>
                      {hit === null ? '-' : `${Math.round(hit * 100)}%`}
                    </td>
                    <td className={cn(td, 'text-right')}>{formatTokens(row.completion_tokens)}</td>
                    <td className={cn(td, 'text-right text-muted-foreground')}>
                      {row.message_count ?? '-'}
                      {row.previous_request_id && <LinkIcon className="inline ml-1 size-3 text-brand/60" />}
                    </td>
                    <td className={cn(td, 'text-muted-foreground')}>{row.user_name ?? '-'}</td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
        {hasMore && (
          <div className="border-t border-border/50 p-2 flex justify-center">
            <Button variant="ghost" size="sm" onClick={loadMore} disabled={loadingMore} className="font-mono text-[11px] uppercase tracking-wider text-muted-foreground">
              {loadingMore ? 'Loading...' : 'Load older'}
            </Button>
          </div>
        )}
      </div>
    </div>
  )
}
