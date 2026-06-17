import { useState, useMemo, useEffect } from 'react'
import { SearchIcon, ChevronsUpDownIcon, Loader2Icon } from 'lucide-react'
import { cn } from '@/lib/utils'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle,
} from '@/components/ui/dialog'

export interface ModelEntry {
  id: string
  created?: number
  owned_by?: string
}

interface ModelPickerProps {
  value: string
  /** Models as objects or plain strings. Strings are coerced to { id } internally. */
  models: (string | ModelEntry)[]
  onChange: (model: string) => void
  disabled?: boolean
  loading?: boolean
  /** Called when the dialog opens — lazy-load hook. */
  onOpen?: () => void
  className?: string
  placeholder?: string
  /** Show a "no override" / clear option at the top. */
  allowEmpty?: boolean
  emptyLabel?: string
  /** Allow typing a custom model name. */
  allowCustom?: boolean
  triggerSize?: 'sm' | 'default'
}

function toEntry(m: string | ModelEntry): ModelEntry {
  return typeof m === 'string' ? { id: m } : m
}

export default function ModelPicker({
  value, models, onChange, disabled, loading, onOpen, className,
  placeholder = 'select model', allowEmpty, emptyLabel = 'no override', allowCustom, triggerSize = 'default',
}: ModelPickerProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [customOpen, setCustomOpen] = useState(false)
  const [customText, setCustomText] = useState('')

  const entries = useMemo(() =>
    models.map(toEntry).sort((a, b) => a.id.localeCompare(b.id)),
  [models])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return entries
    return entries.filter(m => m.id.toLowerCase().includes(q))
  }, [entries, query])

  // Detect custom mode: value is set but not in the model list
  const isCustom = !!(value && !entries.some(e => e.id === value))

  useEffect(() => {
    if (open && onOpen) onOpen()
  }, [open, onOpen])

  useEffect(() => { if (!open) { setQuery(''); setCustomOpen(false); setCustomText('') } }, [open])

  function select(id: string) { onChange(id); setOpen(false) }
  function commitCustom() {
    const v = customText.trim()
    if (v) onChange(v)
    setCustomOpen(false)
    setOpen(false)
  }

  const height = triggerSize === 'sm' ? 'h-7' : 'h-8'
  const fontSize = triggerSize === 'sm' ? 'text-[12px]' : 'text-[13px]'

  return (
    <>
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen(true)}
        className={cn(
          'w-full flex items-center justify-between gap-2 font-mono bg-card border border-border text-foreground px-2.5 outline-none focus:border-brand/50 transition-colors disabled:opacity-50',
          height, fontSize, className
        )}
      >
        {loading
          ? <span className="flex items-center gap-1.5 text-muted-foreground"><Loader2Icon className="size-3 animate-spin" /> loading…</span>
          : <span className={cn('truncate', !value && 'text-muted-foreground')}>{value || placeholder}</span>
        }
        <ChevronsUpDownIcon className="size-3.5 shrink-0 text-muted-foreground" />
      </button>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-w-lg max-h-[80vh] p-0 gap-0">
          <DialogHeader className="px-4 pt-4 pb-2 border-b border-border">
            <DialogTitle className="font-display text-base tracking-wider text-foreground">SELECT MODEL</DialogTitle>
          </DialogHeader>

          <div className="px-4 py-2 border-b border-border">
            <div className="relative">
              <SearchIcon className="absolute left-2.5 top-1/2 -translate-y-1/2 size-3.5 text-muted-foreground pointer-events-none" />
              <input
                autoFocus
                value={query}
                onChange={e => setQuery(e.target.value)}
                placeholder={`filter models…`}
                className="w-full h-8 pl-8 pr-3 font-mono text-[13px] bg-surface border border-border text-foreground outline-none focus:border-brand/50"
              />
            </div>
          </div>

          <div className="max-h-[60vh] overflow-y-auto py-1">
            {allowEmpty && (
              <button
                type="button"
                onClick={() => select('')}
                className={cn(
                  'w-full text-left px-4 py-2.5 font-mono text-[13px] hover:bg-surface border-b border-border/30 transition-colors flex items-center justify-between',
                  !value && 'bg-brand/5 border-l-2 border-l-brand'
                )}
              >
                <span className="text-muted-foreground">{emptyLabel}</span>
                {!value && <span className="text-[10px] text-brand font-mono uppercase tracking-wider shrink-0 ml-2">ACTIVE</span>}
              </button>
            )}
            {filtered.map(m => (
              <button
                key={m.id}
                type="button"
                onClick={() => select(m.id)}
                className={cn(
                  'w-full text-left px-4 py-2.5 font-mono text-[13px] text-foreground hover:bg-surface border-b border-border/30 transition-colors flex items-center justify-between',
                  m.id === value && 'bg-brand/5 border-l-2 border-l-brand'
                )}
              >
                <span className="truncate">{m.id}</span>
                {m.id === value && <span className="text-[10px] text-brand font-mono uppercase tracking-wider shrink-0 ml-2">ACTIVE</span>}
              </button>
            ))}
            {/* Custom model entry */}
            {allowCustom && (
              <button
                type="button"
                onClick={() => setCustomOpen(!customOpen)}
                className={cn(
                  'w-full text-left px-4 py-2.5 font-mono text-[13px] hover:bg-surface border-b border-border/30 transition-colors flex items-center justify-between',
                  (isCustom || customOpen) && 'bg-brand/5 border-l-2 border-l-brand'
                )}
              >
                <span className="text-muted-foreground">custom&hellip;</span>
                {isCustom && <span className="text-[10px] text-brand font-mono uppercase tracking-wider shrink-0 ml-2">{value}</span>}
              </button>
            )}
            {customOpen && (
              <div className="px-4 py-2 border-b border-border/30 space-y-2">
                <input
                  autoFocus
                  value={customText}
                  onChange={e => setCustomText(e.target.value)}
                  onKeyDown={e => { if (e.key === 'Enter') commitCustom() }}
                  placeholder="model name…"
                  className="w-full h-8 px-2 font-mono text-[13px] bg-surface border border-border text-foreground outline-none focus:border-brand/50"
                />
                <div className="flex gap-2 justify-end">
                  <button onClick={() => setCustomOpen(false)} className="font-mono text-[11px] text-muted-foreground hover:text-foreground uppercase tracking-wider px-2 py-0.5">CANCEL</button>
                  <button onClick={commitCustom} className="font-mono text-[11px] text-brand hover:underline uppercase tracking-wider px-2 py-0.5">USE</button>
                </div>
              </div>
            )}
            {filtered.length === 0 && !query.trim() && (
              <div className="px-4 py-8 text-center font-mono text-[12px] text-muted-foreground">no models available</div>
            )}
          </div>

          <div className="px-4 py-2 border-t border-border flex items-center justify-between text-[10px] font-mono text-muted-foreground">
            <span>{entries.length} models</span>
            <button onClick={() => setOpen(false)} className="uppercase tracking-wider hover:text-foreground px-2 py-1">
              CLOSE
            </button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  )
}
