import { useState, useMemo, useEffect } from 'react'
import { SearchIcon, ChevronsUpDownIcon } from 'lucide-react'
import { cn } from '@/lib/utils'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle,
} from '@/components/ui/dialog'

interface ModelPickerProps {
  value: string
  models: string[]
  onChange: (model: string) => void
  disabled?: boolean
  className?: string
  placeholder?: string
}

export default function ModelPicker({ value, models, onChange, disabled, className, placeholder = 'select model' }: ModelPickerProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return models
    return models.filter(m => m.toLowerCase().includes(q))
  }, [models, query])

  useEffect(() => { if (!open) setQuery('') }, [open])

  return (
    <>
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen(true)}
        className={cn(
          'w-full flex items-center justify-between gap-2 font-mono text-[13px] bg-card border border-border text-foreground px-3 py-1.5 outline-none focus:border-brand/50 transition-colors disabled:opacity-50',
          className
        )}
      >
        <span className={cn('truncate', !value && 'text-muted-foreground')}>{value || placeholder}</span>
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
                placeholder={`filter ${models.length} models…`}
                className="w-full h-8 pl-8 pr-3 font-mono text-[13px] bg-surface border border-border text-foreground outline-none focus:border-brand/50"
              />
            </div>
          </div>

          <div className="max-h-[60vh] overflow-y-auto py-1">
            {filtered.map(m => (
              <button
                key={m}
                type="button"
                onClick={() => { onChange(m); setOpen(false) }}
                className={cn(
                  'w-full text-left px-4 py-2.5 font-mono text-[13px] text-foreground hover:bg-surface border-b border-border/30 transition-colors flex items-center justify-between',
                  m === value && 'bg-brand/5 border-l-2 border-l-brand'
                )}
              >
                <span className="truncate">{m}</span>
                {m === value && <span className="text-[10px] text-brand font-mono uppercase tracking-wider shrink-0 ml-2">ACTIVE</span>}
              </button>
            ))}
            {filtered.length === 0 && (
              <div className="px-4 py-8 text-center font-mono text-[12px] text-muted-foreground">no models match &ldquo;{query}&rdquo;</div>
            )}
          </div>

          <div className="px-4 py-2 border-t border-border flex items-center justify-between text-[10px] font-mono text-muted-foreground">
            <span>{filtered.length} of {models.length} models</span>
            <button onClick={() => setOpen(false)} className="uppercase tracking-wider hover:text-foreground px-2 py-1">
              CLOSE
            </button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  )
}
