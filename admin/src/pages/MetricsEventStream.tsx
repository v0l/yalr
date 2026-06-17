import { useEffect, useMemo, useRef, useState } from 'react'
import type { WsProviderMetrics } from '../types'
import { cn } from '@/lib/utils'
import { ActivityIcon, SearchIcon, FilterIcon, XIcon, ChevronDownIcon, RefreshCwIcon, Trash2Icon, UserIcon, KeyIcon } from 'lucide-react'
import { fmt, fmtUser, eventKind, MAXL } from './metricsHelpers'

interface Props {
  events: WsProviderMetrics[]
  onClear: () => void
  wsStatus: 'connecting' | 'connected' | 'disconnected'
  skipped: number
}

export default function MetricsEventStream({ events, onClear, wsStatus, skipped }: Props) {
  const [evtFilter, setEvtFilter] = useState<'all' | 'success' | 'failure' | 'load' | 'balance' | 'info'>('all')
  const [evtProvFilter, setEvtProvFilter] = useState('')
  const [evtModelFilter, setEvtModelFilter] = useState('')
  const [evtTail, setEvtTail] = useState(true)
  const [evtShowFilters, setEvtShowFilters] = useState(false)
  const streamRef = useRef<HTMLDivElement>(null)

  const filteredEvents = useMemo(() => {
    let evs = events
    if (evtFilter !== 'all') evs = evs.filter(e => eventKind(e) === evtFilter)
    if (evtProvFilter) evs = evs.filter(e => e.provider.toLowerCase().includes(evtProvFilter.toLowerCase()))
    if (evtModelFilter) evs = evs.filter(e => (e.model ?? '').toLowerCase().includes(evtModelFilter.toLowerCase()))
    return evs
  }, [events, evtFilter, evtProvFilter, evtModelFilter])

  const eventCounts = useMemo(() => {
    const c: Record<string, number> = { success: 0, failure: 0, load: 0, balance: 0, info: 0, other: 0 }
    for (const e of events) c[eventKind(e)]++
    return c
  }, [events])

  const distinctProviders = useMemo(() => {
    const set = new Set<string>()
    for (const e of events) if (e.provider) set.add(e.provider)
    return Array.from(set).sort()
  }, [events])

  /* ── Auto-scroll to bottom when tailing ────────────────────── */
  useEffect(() => {
    if (!evtTail) return
    const el = streamRef.current
    if (!el) return
    requestAnimationFrame(() => { el.scrollTop = el.scrollHeight })
  }, [filteredEvents, evtTail])

  function resetFilters() { setEvtFilter('all'); setEvtProvFilter(''); setEvtModelFilter('') }

  const hasFilter = evtFilter !== 'all' || evtProvFilter || evtModelFilter

  return (
    <div className="lg:col-span-2">
      <div className="panel">
        {/* Header bar */}
        <div className="flex flex-wrap items-center gap-2 p-3 border-b border-border">
          <ActivityIcon className="size-3.5 text-brand shrink-0" />
          <h3 className="font-mono text-[12px] uppercase tracking-[0.1em] text-foreground shrink-0">EVENT STREAM</h3>

          <button
            onClick={() => setEvtShowFilters(!evtShowFilters)}
            className={cn(
              'flex items-center gap-1 font-mono text-[10px] uppercase tracking-wider px-2 py-1 border transition-colors',
              evtShowFilters || hasFilter
                ? 'text-brand border-brand/30 bg-brand/5'
                : 'text-muted-foreground border-border/50 hover:border-border'
            )}
          >
            <FilterIcon className="size-3" />
            FILTER
            {hasFilter && <span className="text-[9px] text-brand ml-0.5">*</span>}
          </button>

          <div className="flex-1" />

          <span className="font-mono text-[11px] text-muted-foreground tabular-nums">
            {filteredEvents.length < events.length ? (
              <>{filteredEvents.length}<span className="text-muted-foreground/50">/{events.length}</span></>
            ) : events.length}
          </span>

          <button
            onClick={() => setEvtTail(!evtTail)}
            className={cn(
              'font-mono text-[10px] uppercase tracking-wider px-2 py-1 border transition-colors',
              evtTail ? 'text-brand border-brand/30 bg-brand/5' : 'text-muted-foreground border-border/50 hover:border-border'
            )}
          >
            <span className="flex items-center gap-1">
              <ChevronDownIcon className={cn('size-3 transition-transform', evtTail && 'animate-pulse')} />
              TAIL
              {evtTail && <span className="text-[9px] text-brand ml-0.5">ON</span>}
            </span>
          </button>

          <button
            onClick={onClear}
            className="font-mono text-[10px] uppercase tracking-wider px-2 py-1 border border-border/50 text-muted-foreground hover:text-destructive hover:border-destructive/30 transition-colors flex items-center gap-1"
            title="Clear events"
          >
            <Trash2Icon className="size-3" />
          </button>
        </div>

        {/* Collapsible filter bar */}
        {evtShowFilters && (
          <div className="px-3 py-2 border-b border-border/50 space-y-2 bg-surface/50">
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="text-[9px] uppercase tracking-wider text-muted-foreground font-mono mr-1">KIND</span>
              {[
                { key: 'all', label: 'ALL', count: events.length },
                { key: 'success', label: '✓ OK', count: eventCounts.success, cls: 'text-brand border-brand/30 bg-brand/5' },
                { key: 'failure', label: 'FAIL', count: eventCounts.failure, cls: 'text-destructive border-destructive/30 bg-destructive/5' },
                { key: 'info', label: 'INFO', count: eventCounts.info, cls: 'text-muted-foreground border-border/50' },
                { key: 'load', label: 'LOAD', count: eventCounts.load, cls: 'text-warning border-warning/30 bg-warning/5' },
                { key: 'balance', label: 'BAL', count: eventCounts.balance, cls: 'text-muted-foreground border-border/50' },
              ].map(f => (
                <button
                  key={f.key}
                  onClick={() => setEvtFilter(f.key as typeof evtFilter)}
                  className={cn(
                    'font-mono text-[9px] uppercase tracking-wider px-2 py-0.5 border transition-colors',
                    evtFilter === f.key
                      ? (f.cls ?? 'text-foreground border-foreground/30 bg-surface')
                      : 'text-muted-foreground border-transparent hover:border-border/30'
                  )}
                >
                  {f.label}
                  <span className="ml-1 opacity-50 tabular-nums">{f.count}</span>
                </button>
              ))}
            </div>

            <div className="flex flex-wrap items-center gap-2">
              <select
                value={evtProvFilter}
                onChange={e => setEvtProvFilter(e.target.value)}
                className="font-mono text-[10px] border border-border bg-surface text-foreground px-2 py-1 outline-none focus:border-brand/50"
              >
                <option value="">ALL PROVIDERS</option>
                {distinctProviders.map(p => (
                  <option key={p} value={p}>{p}</option>
                ))}
              </select>
              <div className="flex items-center gap-1 flex-1 min-w-0">
                <SearchIcon className="size-3 text-muted-foreground shrink-0" />
                <input
                  type="text"
                  value={evtModelFilter}
                  onChange={e => setEvtModelFilter(e.target.value)}
                  placeholder="model..."
                  className="font-mono text-[10px] border-0 border-b border-border bg-transparent text-foreground px-1 py-1 flex-1 min-w-0 outline-none focus:border-brand/50 placeholder:text-muted-foreground/50"
                />
                {evtModelFilter && (
                  <button onClick={() => setEvtModelFilter('')} className="text-muted-foreground hover:text-foreground">
                    <XIcon className="size-3" />
                  </button>
                )}
              </div>
              {hasFilter && (
                <button
                  onClick={resetFilters}
                  className="font-mono text-[9px] uppercase tracking-wider text-muted-foreground hover:text-brand border border-border/50 px-2 py-1"
                >
                  <RefreshCwIcon className="size-3 inline mr-0.5" />
                  RESET
                </button>
              )}
            </div>
          </div>
        )}

        {/* Event rows */}
        <div
          ref={streamRef}
          onScroll={() => {
            if (!streamRef.current) return
            const el = streamRef.current
            if (el.scrollTop + el.clientHeight < el.scrollHeight - 30) setEvtTail(false)
            if (el.scrollTop + el.clientHeight >= el.scrollHeight - 5) setEvtTail(true)
          }}
          className="max-h-[550px] overflow-y-auto font-mono text-[11px]"
        >
          <div className="flex items-center gap-2 px-2 py-1.5 border-b border-border text-[9px] uppercase tracking-wider text-muted-foreground sticky top-0 bg-card">
            <span className="shrink-0 w-[82px] text-right pr-2">TIME</span>
            <span className="shrink-0 w-36">PROVIDER</span>
            <span className="shrink-0 w-28">MODEL</span>
            <span className="shrink-0 w-12">EVENT</span>
            <span className="flex-1 min-w-0">VALUE</span>
            <span className="shrink-0 w-28">USER</span>
          </div>
          {filteredEvents.length === 0 ? (
            <p className="text-muted-foreground py-16 text-center">
              {events.length === 0
                ? (wsStatus === 'connected' ? 'WAITING FOR EVENTS...' : 'NOT CONNECTED')
                : 'NO EVENTS MATCH FILTERS'}
            </p>
          ) : (
            // Reverse so newest is at bottom (terminal tail)
            [...filteredEvents].reverse().map((ev, i) => {
              const { label, value, kind } = fmt(ev.event)
              const d = new Date(ev.timestamp_ms)
              const t = d.toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' })
              const ms = String(d.getMilliseconds()).padStart(3, '0')
              const kc = kind === 'ok' ? 'text-brand' : kind === 'err' ? 'text-destructive' : kind === 'warn' ? 'text-warning' : 'text-muted-foreground'
              const userText = fmtUser(ev.user)
              return (
                <div
                  key={`${ev.timestamp_ms}-${i}`}
                  className={cn(
                    'flex items-center gap-2 px-2 py-0.5 border-b border-border/50 transition-colors',
                    i === filteredEvents.length - 1 && 'bg-brand/5'
                  )}
                >
                  <span className="text-muted-foreground shrink-0 w-[82px] tabular-nums text-right pr-2">{t}.{ms}</span>
                  <span className="font-medium shrink-0 w-36 truncate" title={ev.provider}>{ev.provider}</span>
                  <span className="text-muted-foreground shrink-0 w-28 truncate" title={ev.model ?? ''}>{ev.model ?? ''}</span>
                  <span className={cn('shrink-0 w-12 text-[9px] px-1 py-0 border font-mono uppercase tracking-wider text-center', kc)} style={{ borderColor: 'currentColor' }}>
                    {label}
                  </span>
                  <span
                    className={cn('truncate flex-1 min-w-0', kind === 'err' && 'cursor-copy hover:underline')}
                    title={kind === 'err' ? `Click to copy: ${value}` : value}
                    onClick={() => { if (kind === 'err') navigator.clipboard.writeText(value) }}
                  >{value}</span>
                  <span className="shrink-0 w-32 text-[10px] text-muted-foreground truncate flex items-center gap-1" title={userText}>
                    {userText && userText.includes('🔑') ? <KeyIcon className="size-2.5 shrink-0" /> : userText ? <UserIcon className="size-2.5 shrink-0" /> : null}
                    {userText}
                  </span>
                </div>
              )
            })
          )}
        </div>

        {/* Footer bar */}
        <div className="flex items-center gap-3 px-3 py-1.5 border-t border-border text-[10px] font-mono text-muted-foreground">
          <span className={cn(
            'flex items-center gap-1',
            wsStatus === 'connected' ? 'text-brand' : wsStatus === 'connecting' ? 'text-warning' : 'text-destructive'
          )}>
            <span className={cn('w-1.5 h-1.5 rounded-full',
              wsStatus === 'connected' && 'bg-brand animate-pulse-status',
              wsStatus === 'connecting' && 'bg-warning animate-pulse-status',
              wsStatus === 'disconnected' && 'bg-destructive'
            )} />
            {wsStatus.toUpperCase()}
          </span>
          <span>BUFFER {events.length}/{MAXL}</span>
          {skipped > 0 && <span className="text-warning">SKIPPED {skipped}</span>}
          <span className="flex-1" />
          {evtTail
            ? <span className="text-brand text-[9px] uppercase tracking-wider">TAILING</span>
            : <span className="text-muted-foreground text-[9px] uppercase tracking-wider">PAUSED</span>
          }
        </div>
      </div>
    </div>
  )
}
