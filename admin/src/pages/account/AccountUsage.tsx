import { useCallback, useEffect, useState } from 'react'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import UsagePanel from '@/components/UsagePanel'
import { accountApi } from '@/api/account'
import { cacheHitRate, firstTokenMs, formatMs, formatTime, formatTokens, statusTone } from '@/lib/requestLog'
import { cn } from '@/lib/utils'
import type { RequestLogSummary } from '@/types'
import AccountNav from './AccountNav'
import AccountShell from './AccountShell'

const PAGE_SIZE = 50
const REQUEST_HOURS = 24 * 7

const th = 'px-3 py-2.5 font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground whitespace-nowrap'
const td = 'px-3 py-2 font-mono text-[12px] tabular-nums whitespace-nowrap'

export default function AccountUsage({ accountKey, onSignOut }: { accountKey: string; onSignOut: () => void }) {
  const [rows, setRows] = useState<RequestLogSummary[] | null>(null)
  const [hasMore, setHasMore] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    accountApi.requests(accountKey, REQUEST_HOURS)
      .then(page => { setRows(page); setHasMore(page.length === PAGE_SIZE) })
      .catch(e => setError(e instanceof Error ? e.message : 'Failed to load requests'))
  }, [accountKey])

  const loadUsage = useCallback((hours: number) => accountApi.usage(accountKey, hours), [accountKey])

  async function loadMore() {
    const last = rows?.[rows.length - 1]
    if (!last) return
    setLoadingMore(true)
    try {
      const page = await accountApi.requests(accountKey, REQUEST_HOURS, last.id)
      setRows(prev => [...(prev ?? []), ...page])
      setHasMore(page.length === PAGE_SIZE)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load requests')
    } finally {
      setLoadingMore(false)
    }
  }

  return (
    <AccountShell onSignOut={onSignOut} wide>
      <AccountNav />
      {error && (
        <Alert className="border-destructive/30 bg-destructive/5 font-mono text-[13px] text-destructive">
          <AlertDescription><span className="text-destructive">!</span> {error}</AlertDescription>
        </Alert>
      )}
      <UsagePanel title="Usage per hour" scopeKey={accountKey} load={loadUsage} />

      <div className="panel">
        <div className="border-b border-border/50 px-4 py-3">
          <h2 className="font-mono text-[10px] uppercase tracking-[0.12em] text-muted-foreground">Requests, last 7 days</h2>
        </div>
        <div className="overflow-x-auto">
          {!rows ? <Skeleton className="m-4 h-24 bg-secondary" /> : rows.length === 0 ? (
            <p className="p-6 text-center font-mono text-[12px] text-muted-foreground">No requests yet</p>
          ) : (
            <table className="table-scan w-full">
              <thead>
                <tr className="border-b border-border/50 text-left">
                  <th className={th}>Time</th>
                  <th className={th}>Model</th>
                  <th className={th}>Status</th>
                  <th className={cn(th, 'text-right')}>First token</th>
                  <th className={cn(th, 'text-right')}>Total</th>
                  <th className={cn(th, 'text-right')}>In</th>
                  <th className={cn(th, 'text-right')}>Cache</th>
                  <th className={cn(th, 'text-right')}>Out</th>
                </tr>
              </thead>
              <tbody>
                {rows.map(row => {
                  const hit = cacheHitRate(row)
                  return (
                    <tr key={row.id} className="border-b border-border/50" title={row.error ?? undefined}>
                      <td className={cn(td, 'text-muted-foreground')}>{formatTime(row.started_at_ms)}</td>
                      <td className={cn(td, 'text-foreground')}>
                        {row.model}
                        {row.stream && <span className="ml-1.5 text-[10px] text-muted-foreground">SSE</span>}
                      </td>
                      <td className={td}>
                        <Badge className={cn('h-5 px-1.5 py-0 font-mono text-[10px] uppercase', statusTone(row.status))}>{row.status}</Badge>
                      </td>
                      <td className={cn(td, 'text-right')}>{formatMs(firstTokenMs(row))}</td>
                      <td className={cn(td, 'text-right')}>{formatMs(row.total_ms)}</td>
                      <td className={cn(td, 'text-right')}>{formatTokens(row.prompt_tokens)}</td>
                      <td className={cn(td, 'text-right text-muted-foreground')}>{hit === null ? '-' : `${Math.round(hit * 100)}%`}</td>
                      <td className={cn(td, 'text-right')}>{formatTokens(row.completion_tokens)}</td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          )}
        </div>
        {hasMore && (
          <div className="flex justify-center border-t border-border/50 p-2">
            <Button variant="ghost" size="sm" onClick={loadMore} disabled={loadingMore} className="font-mono text-[11px] uppercase tracking-wider text-muted-foreground">
              {loadingMore ? 'Loading...' : 'Load older'}
            </Button>
          </div>
        )}
      </div>
    </AccountShell>
  )
}
