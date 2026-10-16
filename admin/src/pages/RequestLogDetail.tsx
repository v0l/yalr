import { useEffect, useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { api } from '../api/client'
import type { HeaderPairs, RequestLogBodies, RequestLogDetail as Detail } from '../types'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { cacheHitRate, formatBytes, formatMs, formatTokens, statusTone } from '@/lib/requestLog'
import { ArrowLeftIcon, CopyIcon } from 'lucide-react'

const label = 'font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground'

function Field({ name, children }: { name: string; children: React.ReactNode }) {
  return (
    <div className="flex justify-between gap-4 py-1 border-b border-border/30 last:border-0">
      <span className={label}>{name}</span>
      <span className="font-mono text-[12px] tabular-nums text-right break-all">{children ?? '-'}</span>
    </div>
  )
}

function Panel({ title, children, className }: { title: string; children: React.ReactNode; className?: string }) {
  return (
    <div className={cn('panel p-4', className)}>
      <div className={cn(label, 'mb-2')}>{title}</div>
      {children}
    </div>
  )
}

function Timeline({ detail }: { detail: Detail }) {
  const total = detail.total_ms ?? 0
  const marks = [
    { name: 'headers', ms: detail.headers_ms, tone: 'bg-muted-foreground' },
    { name: 'first event', ms: detail.first_event_ms, tone: 'bg-muted-foreground' },
    { name: 'first thinking', ms: detail.first_thinking_ms, tone: 'bg-warning' },
    { name: 'first text', ms: detail.first_text_ms, tone: 'bg-brand' },
  ].filter((m): m is { name: string; ms: number; tone: string } => m.ms !== null)
  if (!total) return <div className="font-mono text-[12px] text-muted-foreground">No timing recorded</div>
  return (
    <div className="space-y-2">
      <div className="relative h-6 border border-border bg-surface">
        {marks.map(m => (
          <div
            key={m.name}
            title={`${m.name}: ${formatMs(m.ms)}`}
            className={cn('absolute top-0 bottom-0 w-0.5', m.tone)}
            style={{ left: `${Math.min(100, (m.ms / total) * 100)}%` }}
          />
        ))}
      </div>
      <div className="grid grid-cols-2 sm:grid-cols-5 gap-2">
        {[...marks, { name: 'total', ms: total, tone: 'bg-foreground' }].map(m => (
          <div key={m.name} className="flex items-center gap-1.5">
            <span className={cn('size-2 shrink-0', m.tone)} />
            <span className={label}>{m.name}</span>
            <span className="font-mono text-[12px] tabular-nums ml-auto">{formatMs(m.ms)}</span>
          </div>
        ))}
      </div>
    </div>
  )
}

function Headers({ pairs }: { pairs: HeaderPairs | null }) {
  if (!pairs?.length) return <div className="font-mono text-[12px] text-muted-foreground">None</div>
  return (
    <div className="space-y-0.5">
      {pairs.map(([k, v], i) => (
        <div key={`${k}-${i}`} className="font-mono text-[11px] break-all">
          <span className="text-muted-foreground">{k}:</span> {v}
        </div>
      ))}
    </div>
  )
}

function Json({ value }: { value: unknown }) {
  const text = typeof value === 'string' ? value : JSON.stringify(value, null, 2)
  return (
    <div className="relative">
      <Button
        variant="ghost"
        size="icon-xs"
        onClick={() => navigator.clipboard.writeText(text)}
        className="absolute right-1 top-1 text-muted-foreground hover:text-brand"
      >
        <CopyIcon className="size-3.5" />
      </Button>
      <pre className="max-h-[32rem] overflow-auto bg-surface border border-border p-3 font-mono text-[11px] leading-relaxed whitespace-pre-wrap break-all">
        {value === null ? 'not stored' : text}
      </pre>
    </div>
  )
}

const BODY_TABS = [
  { key: 'client_request', name: 'Client request' },
  { key: 'upstream_request', name: 'Upstream request' },
  { key: 'response', name: 'Response' },
] as const

function Bodies({ id }: { id: number }) {
  const [bodies, setBodies] = useState<RequestLogBodies | null>(null)
  const [tab, setTab] = useState<(typeof BODY_TABS)[number]['key']>('client_request')
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  async function load() {
    setLoading(true)
    try {
      setBodies(await api.getRequestLogBodies(id))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }

  if (!bodies) {
    return (
      <div className="flex items-center gap-3">
        <Button variant="outline" size="sm" onClick={load} disabled={loading} className="font-mono text-[11px] uppercase tracking-wider border-border">
          {loading ? 'Loading...' : 'Load bodies'}
        </Button>
        {error && <span className="font-mono text-[12px] text-destructive">{error}</span>}
      </div>
    )
  }
  return (
    <div className="space-y-2">
      <div className="flex border border-border w-fit">
        {BODY_TABS.map(t => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            className={cn(
              'px-2.5 h-7 font-mono text-[10px] uppercase tracking-wider transition-colors',
              tab === t.key ? 'bg-brand/10 text-brand' : 'text-muted-foreground hover:text-foreground hover:bg-surface'
            )}
          >
            {t.name}
          </button>
        ))}
      </div>
      <Json value={bodies[tab]} />
    </div>
  )
}

export default function RequestLogDetail() {
  const { id } = useParams()
  const logId = Number(id)
  const [detail, setDetail] = useState<Detail | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    api.getRequestLogDetail(logId)
      .then(d => { if (!cancelled) { setDetail(d); setError(null) } })
      .catch(e => { if (!cancelled) setError(e instanceof Error ? e.message : String(e)) })
    return () => { cancelled = true }
  }, [logId])

  const back = (
    <Link to="/request-log" className="inline-flex items-center gap-1.5 font-mono text-[12px] text-muted-foreground hover:text-brand">
      <ArrowLeftIcon className="size-3.5" /> Request log
    </Link>
  )

  if (error) {
    return (
      <div className="space-y-4 p-4 sm:p-5">
        {back}
        <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono text-[13px]">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      </div>
    )
  }
  if (!detail || detail.id !== logId) {
    return (
      <div className="space-y-4 p-4 sm:p-5">
        {back}
        <Skeleton className="h-10 w-80 bg-secondary" />
        <Skeleton className="h-64 bg-secondary" />
      </div>
    )
  }

  const hit = cacheHitRate(detail)
  const uncached = detail.prompt_tokens === null
    ? null
    : detail.prompt_tokens - (detail.cached_tokens ?? 0) - (detail.cache_write_tokens ?? 0)

  return (
    <div className="space-y-4 p-4 sm:p-5">
      {back}
      <div className="flex items-end justify-between gap-4 flex-wrap">
        <div>
          <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground leading-none mb-1">
            {detail.model}
          </h1>
          <p className="font-mono text-[12px] text-muted-foreground">
            {new Date(detail.started_at_ms).toLocaleString()} · {detail.endpoint} · {detail.request_id}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {detail.stream && <Badge className="font-mono text-[10px] px-1.5 py-0 h-5 bg-secondary text-muted-foreground border-border">SSE</Badge>}
          {detail.finish_reason && <Badge className="font-mono text-[10px] px-1.5 py-0 h-5 bg-secondary text-muted-foreground border-border uppercase">{detail.finish_reason}</Badge>}
          <Badge className={cn('font-mono text-[10px] px-1.5 py-0 h-5 uppercase', statusTone(detail.status))}>{detail.status}</Badge>
        </div>
      </div>

      {detail.error && (
        <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono text-[12px]">
          <AlertDescription className="break-all">{detail.error}</AlertDescription>
        </Alert>
      )}

      <Panel title="Timing">
        <Timeline detail={detail} />
      </Panel>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        <Panel title="Tokens">
          <Field name="Prompt">{formatTokens(detail.prompt_tokens)}</Field>
          <Field name="Cache read">{formatTokens(detail.cached_tokens)}</Field>
          <Field name="Cache write">{formatTokens(detail.cache_write_tokens)}</Field>
          <Field name="Uncached">{formatTokens(uncached)}</Field>
          <Field name="Hit rate">{hit === null ? '-' : `${(hit * 100).toFixed(1)}%`}</Field>
          <Field name="Weighted input">{formatTokens(detail.weighted_input_tokens)}</Field>
          <Field name="Completion">{formatTokens(detail.completion_tokens)}</Field>
        </Panel>

        <Panel title="Conversation">
          <Field name="Messages">{detail.message_count}</Field>
          <Field name="Diverges at">{detail.first_diverging_message}</Field>
          <Field name="Previous">
            {detail.previous_log_id
              ? <Link to={`/request-log/${detail.previous_log_id}`} className="text-brand hover:underline">#{detail.previous_log_id}</Link>
              : detail.previous_request_id ?? '-'}
          </Field>
          <Field name="System hash">{detail.system_hash?.slice(0, 12)}</Field>
          <Field name="Tools hash">{detail.tools_hash?.slice(0, 12)}</Field>
          <Field name="Request size">{formatBytes(detail.request_bytes)}</Field>
          <Field name="Response size">{formatBytes(detail.response_bytes)}</Field>
        </Panel>

        <Panel title="Routing">
          <Field name="Provider">{detail.provider}</Field>
          <Field name="Upstream status">{detail.upstream_status}</Field>
          <Field name="Attempts">{detail.attempts}</Field>
          <Field name="User">{detail.user_name}</Field>
          <Field name="API key">{detail.api_key_name}</Field>
          <Field name="Client IP">{detail.client_ip}</Field>
          <Field name="Upstream URL">{detail.upstream_url}</Field>
        </Panel>
      </div>

      {(detail.attempt_log_parsed?.length ?? 0) > 1 && (
        <Panel title="Attempts">
          <table className="w-full">
            <tbody>
              {detail.attempt_log_parsed!.map((a, i) => (
                <tr key={i} className="border-b border-border/50 last:border-0">
                  <td className="py-1.5 pr-3 font-mono text-[12px] tabular-nums text-muted-foreground">{formatMs(a.at_ms)}</td>
                  <td className="py-1.5 pr-3 font-mono text-[12px]">{a.provider}</td>
                  <td className="py-1.5 pr-3 font-mono text-[12px] tabular-nums">{a.upstream_status ?? '-'}</td>
                  <td className="py-1.5 font-mono text-[12px] text-destructive break-all">{a.error ?? ''}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Panel>
      )}

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        <Panel title="Client headers"><Headers pairs={detail.client_headers_parsed} /></Panel>
        <Panel title="Upstream request headers"><Headers pairs={detail.upstream_request_headers_parsed} /></Panel>
        <Panel title="Upstream response headers"><Headers pairs={detail.upstream_response_headers_parsed} /></Panel>
      </div>

      <Panel title="Bodies">
        {detail.has_bodies
          ? <Bodies key={detail.id} id={detail.id} />
          : <div className="font-mono text-[12px] text-muted-foreground">Not stored (expired or body capture disabled)</div>}
      </Panel>
    </div>
  )
}
