import { useState } from 'react'
import { GripVerticalIcon, TrashIcon, BracesIcon } from 'lucide-react'
import type { RoutingConfigProvider } from '../../types'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { ExtraBodyField, formatExtraBody, parseExtraBody } from '@/components/ExtraBodyField'
import { Checkbox } from '@/components/ui/checkbox'
import { cn } from '@/lib/utils'
import ModelPicker from '@/components/ModelPicker'
import type { ModelEntry } from '@/components/ModelPicker'

export interface AssignmentPatch { model?: string | null; weight?: number; is_active?: boolean; extra_body?: Record<string, unknown> }

interface AssignmentRowProps {
  assignment: RoutingConfigProvider
  index: number
  isPriority: boolean
  busy: boolean
  models: ModelEntry[]
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

function ConfigModelInput({ value, disabled, onCommit }: { value: string; disabled: boolean; onCommit: (m: string) => void }) {
  const [text, setText] = useState(value)
  return (
    <Input
      value={text}
      disabled={disabled}
      placeholder="child's own model"
      onChange={e => setText(e.target.value)}
      onBlur={() => { if (text !== value) onCommit(text) }}
      onKeyDown={e => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur() }}
      className="h-7 font-mono text-[12px] bg-surface border-border text-foreground"
    />
  )
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

  const isConfig = a.kind === 'config'
  const hasExtraBody = !!a.extra_body && Object.keys(a.extra_body).length > 0
  const [bodyOpen, setBodyOpen] = useState(false)
  const [bodyText, setBodyText] = useState('')
  const parsedBody = parseExtraBody(bodyText)

  function openBody() { setBodyText(formatExtraBody(a.extra_body)); setBodyOpen(true) }
  function saveBody() {
    if (parsedBody.error !== null) return
    onPatch({ extra_body: parsedBody.value })
    setBodyOpen(false)
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
          {isConfig && (
            <span className="border border-brand/30 bg-brand/15 px-1.5 font-mono text-[10px] uppercase tracking-wider text-brand">config</span>
          )}
          <span className="font-mono text-[13px] font-medium text-foreground">{isConfig ? a.config_name : a.provider_name}</span>
          {!isConfig && <span className="font-mono text-[11px] text-muted-foreground">{a.provider_slug}</span>}
        </div>
      </td>
      <td className="px-2 py-1.5 w-[34%]">
        {isConfig ? (
          <ConfigModelInput value={a.model ?? ''} disabled={busy} onCommit={m => onPatch({ model: m })} />
        ) : (
        <ModelPicker
          triggerSize="sm"
          value={a.model || ''}
          models={models}
          loading={modelsLoading}
          disabled={busy}
          onOpen={ensureModels}
          onChange={m => onPatch({ model: m || '' })}
          allowEmpty emptyLabel="no override" allowCustom
        />
        )}
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
        <div className="flex items-center justify-end gap-2">
        <button
          type="button"
          disabled={busy}
          onClick={openBody}
          title="Extra request body fields"
          className={cn('transition-colors disabled:opacity-30', hasExtraBody ? 'text-brand' : 'text-muted-foreground hover:text-foreground')}
        >
          <BracesIcon className="size-3.5" />
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={onDelete}
          title={isConfig ? 'Remove config' : 'Remove provider'}
          className="text-muted-foreground transition-colors hover:text-destructive disabled:opacity-30"
        >
          <TrashIcon className="size-3.5" />
        </button>
        </div>
        <Dialog open={bodyOpen} onOpenChange={setBodyOpen}>
          <DialogContent className="sm:max-w-lg border-border bg-card text-left">
            <DialogHeader>
              <DialogTitle className="font-display text-xl tracking-[0.04em]">EXTRA BODY</DialogTitle>
              <DialogDescription className="font-mono text-[12px] text-muted-foreground">
                Fields merged into requests routed through {isConfig ? a.config_name : a.provider_name}. They override the provider's own and lose to the client's.
              </DialogDescription>
            </DialogHeader>
            <ExtraBodyField id={`rcp-extra-body-${a.id}`} value={bodyText} onChange={setBodyText} rows={8} />
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setBodyOpen(false)} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
              <Button type="button" onClick={saveBody} disabled={parsedBody.error !== null} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">SAVE</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </td>
    </tr>
  )
}
