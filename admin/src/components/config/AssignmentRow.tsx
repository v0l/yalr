import { useState } from 'react'
import { GripVerticalIcon, TrashIcon } from 'lucide-react'
import type { RoutingConfigProvider } from '../../types'
import { Input } from '@/components/ui/input'
import { Checkbox } from '@/components/ui/checkbox'
import { cn } from '@/lib/utils'
import ModelSelect from './ModelSelect'
import type { ProviderModel } from './modelCache'

export interface AssignmentPatch { model?: string | null; weight?: number; is_active?: boolean }

interface AssignmentRowProps {
  assignment: RoutingConfigProvider
  index: number
  isPriority: boolean
  busy: boolean
  models: ProviderModel[]
  modelsLoading: boolean
  ensureModels: () => void
  onPatch: (patch: AssignmentPatch) => void
  onDelete: () => void
  // drag wiring (priority only)
  dragging: boolean
  dragOver: boolean
  onDragStart: () => void
  onDragOver: (e: React.DragEvent) => void
  onDrop: () => void
  onDragEnd: () => void
}

export default function AssignmentRow(props: AssignmentRowProps) {
  const {
    assignment: a, index, isPriority, busy, models, modelsLoading, ensureModels,
    onPatch, onDelete, dragging, dragOver, onDragStart, onDragOver, onDrop, onDragEnd,
  } = props

  const [weight, setWeight] = useState(String(a.weight))
  // Sync local input when the prop changes (e.g. after a reorder) without an effect.
  const [lastWeight, setLastWeight] = useState(a.weight)
  if (a.weight !== lastWeight) { setLastWeight(a.weight); setWeight(String(a.weight)) }

  function commitWeight() {
    const n = Number(weight)
    if (!Number.isFinite(n) || n < 1) { setWeight(String(a.weight)); return }
    if (n !== a.weight) onPatch({ weight: n })
  }

  return (
    <tr
      onDragOver={isPriority ? onDragOver : undefined}
      onDrop={isPriority ? onDrop : undefined}
      className={cn(
        'border-b border-border/40 transition-colors',
        dragging ? 'opacity-40' : 'hover:bg-surface',
        dragOver && 'bg-brand/10',
        !a.is_active && 'opacity-60',
      )}
    >
      <td className="px-2 py-1.5">
        <div className="flex items-center gap-1.5">
          {isPriority && (
            <span
              draggable
              onDragStart={onDragStart}
              onDragEnd={onDragEnd}
              className="cursor-grab text-muted-foreground/60 hover:text-foreground active:cursor-grabbing"
              title="Drag to reorder priority"
            >
              <GripVerticalIcon className="size-3.5" />
            </span>
          )}
          {isPriority && <span className="font-mono text-[11px] tabular-nums text-muted-foreground">#{index + 1}</span>}
          <span className="font-mono text-[13px] font-medium text-foreground">{a.provider_name}</span>
          <span className="font-mono text-[11px] text-muted-foreground">{a.provider_slug}</span>
        </div>
      </td>
      <td className="px-2 py-1.5 w-[34%]">
        <ModelSelect
          value={a.model || ''}
          models={models}
          loading={modelsLoading}
          disabled={busy}
          onOpen={ensureModels}
          onChange={m => onPatch({ model: m || null })}
        />
      </td>
      <td className="px-2 py-1.5 w-20">
        <Input
          type="number"
          min={1}
          value={weight}
          disabled={busy}
          onChange={e => setWeight(e.target.value)}
          onBlur={commitWeight}
          onKeyDown={e => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur() }}
          className="h-7 w-16 text-right font-mono text-[12px] tabular-nums bg-surface border-border text-foreground"
        />
      </td>
      <td className="px-2 py-1.5">
        <button
          type="button"
          disabled={busy}
          onClick={() => onPatch({ is_active: !a.is_active })}
          className={cn(
            'flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-wider transition-colors',
            a.is_active ? 'text-brand' : 'text-muted-foreground hover:text-foreground',
          )}
        >
          <Checkbox checked={a.is_active} className="pointer-events-none size-3.5" />
          {a.is_active ? 'active' : 'off'}
        </button>
      </td>
      <td className="px-2 py-1.5 text-right">
        <button
          type="button"
          disabled={busy}
          onClick={onDelete}
          title="Remove provider"
          className="text-muted-foreground transition-colors hover:text-destructive disabled:opacity-30"
        >
          <TrashIcon className="size-3.5" />
        </button>
      </td>
    </tr>
  )
}
