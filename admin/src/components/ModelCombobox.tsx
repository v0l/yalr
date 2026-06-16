import { useEffect, useMemo, useRef, useState } from 'react'
import { SearchIcon, ChevronsUpDownIcon, CheckIcon } from 'lucide-react'
import { cn } from '@/lib/utils'

interface ModelComboboxProps {
  value: string
  models: string[]
  onChange: (model: string) => void
  disabled?: boolean
  className?: string
  placeholder?: string
}

/**
 * Searchable model picker. Replaces a native <select> so users can filter
 * through large model lists (e.g. OpenRouter's hundreds of models).
 */
export default function ModelCombobox({ value, models, onChange, disabled, className, placeholder = 'select model' }: ModelComboboxProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const ref = useRef<HTMLDivElement>(null)

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return models
    return models.filter(m => m.toLowerCase().includes(q))
  }, [models, query])

  useEffect(() => {
    if (!open) return
    function onDown(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', onDown)
    return () => document.removeEventListener('mousedown', onDown)
  }, [open])

  useEffect(() => { if (!open) setQuery('') }, [open])

  return (
    <div ref={ref} className={cn('relative', className)}>
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen(o => !o)}
        className="w-full flex items-center justify-between gap-2 font-mono text-[13px] bg-card border border-border text-foreground px-3 py-1.5 outline-none focus:border-brand/50 transition-colors disabled:opacity-50"
      >
        <span className={cn('truncate', !value && 'text-muted-foreground')}>{value || placeholder}</span>
        <ChevronsUpDownIcon className="size-3.5 shrink-0 text-muted-foreground" />
      </button>
      {open && (
        <div className="absolute z-50 mt-1 w-full bg-card border border-border shadow-md">
          <div className="p-1 border-b border-border">
            <div className="relative">
              <SearchIcon className="absolute left-2 top-1/2 -translate-y-1/2 size-3 text-muted-foreground pointer-events-none" />
              <input
                autoFocus
                value={query}
                onChange={e => setQuery(e.target.value)}
                placeholder={`filter ${models.length} models…`}
                className="w-full h-7 pl-7 pr-2 font-mono text-[12px] bg-surface border border-border text-foreground outline-none focus:border-brand/50"
              />
            </div>
          </div>
          <div className="max-h-72 overflow-y-auto py-1">
            {filtered.map(m => (
              <button
                key={m}
                type="button"
                onClick={() => { onChange(m); setOpen(false) }}
                className="w-full flex items-center gap-2 px-2 py-1 text-left font-mono text-[12px] text-foreground hover:bg-surface"
              >
                <CheckIcon className={cn('size-3 shrink-0', m === value ? 'opacity-100 text-brand' : 'opacity-0')} />
                <span className="truncate">{m}</span>
              </button>
            ))}
            {filtered.length === 0 && (
              <div className="px-2 py-3 text-center font-mono text-[11px] text-muted-foreground">no matches</div>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
