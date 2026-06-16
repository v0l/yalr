import { useState } from 'react'
import { Loader2Icon } from 'lucide-react'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import type { ProviderModel } from './modelCache'

interface ModelSelectProps {
  value: string
  models: ProviderModel[]
  loading: boolean
  disabled?: boolean
  onOpen: () => void
  onChange: (model: string) => void
  className?: string
}

/**
 * Model picker with a "no override" option, the provider's advertised models,
 * and a custom free-text fallback. Lazily triggers loading via onOpen.
 */
export default function ModelSelect({ value, models, loading, disabled, onOpen, onChange, className }: ModelSelectProps) {
  // Custom mode if a value is set that isn't in the known model list.
  const known = models.some(m => m.id === value)
  const [custom, setCustom] = useState(!!value && !known && models.length > 0)
  const showCustom = custom || (!!value && !known && !loading)

  function handleSelect(v: string) {
    if (v === '__custom__') { setCustom(true); onChange('') }
    else if (v === 'empty') { setCustom(false); onChange('') }
    else { setCustom(false); onChange(v) }
  }

  return (
    <div className={className}>
      <Select
        value={showCustom ? '__custom__' : value || 'empty'}
        onValueChange={handleSelect}
        onOpenChange={o => { if (o) onOpen() }}
        disabled={disabled}
      >
        <SelectTrigger className="h-7 w-full font-mono text-[12px] bg-surface border-border text-foreground">
          {loading
            ? <span className="flex items-center gap-1.5 text-muted-foreground"><Loader2Icon className="size-3 animate-spin" /> loading…</span>
            : <SelectValue placeholder="no override" />}
        </SelectTrigger>
        <SelectContent className="bg-card border-border">
          <SelectGroup>
            <SelectItem value="empty" className="font-mono text-[12px] text-muted-foreground">no override</SelectItem>
            {models.map(m => <SelectItem key={m.id} value={m.id} className="font-mono text-[12px] text-foreground">{m.id}</SelectItem>)}
            <SelectItem value="__custom__" className="font-mono text-[12px] text-brand">custom…</SelectItem>
          </SelectGroup>
        </SelectContent>
      </Select>
      {showCustom && (
        <Input
          autoFocus
          value={value}
          onChange={e => onChange(e.target.value)}
          placeholder="model name"
          className="mt-1 h-7 font-mono text-[12px] bg-surface border-border text-foreground"
        />
      )}
    </div>
  )
}
