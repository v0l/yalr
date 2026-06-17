import { useState } from 'react'
import { PencilIcon, TrashIcon, PlusIcon, XIcon } from 'lucide-react'
import type { RoutingConfigFull, RoutingConfigProvider, ProviderListItem } from '../../types'
import { api } from '../../api/client'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { Checkbox } from '@/components/ui/checkbox'
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import AssignmentRow, { type AssignmentPatch } from './AssignmentRow'
import ModelPicker from '@/components/ModelPicker'
import { useModelCache } from './modelCache'

interface AssignmentsPanelProps {
  config: RoutingConfigFull
  providers: ProviderListItem[]
  notify: (msg: string) => void
  onError: (msg: string) => void
  onChanged: () => Promise<void> | void
  onEditConfig: () => void
  onDeleteConfig: () => void
  onRequestDeleteAssignment: (a: RoutingConfigProvider) => void
}

type NewAssignment = { provider_id: number; slug: string; model: string; weight: number; is_active: boolean }
const emptyNew: NewAssignment = { provider_id: 0, slug: '', model: '', weight: 1, is_active: true }

export default function AssignmentsPanel(props: AssignmentsPanelProps) {
  const { config, providers, notify, onError, onChanged, onEditConfig, onDeleteConfig, onRequestDeleteAssignment } = props
  const isPriority = config.strategy === 'priority'
  const { ensure, get } = useModelCache()

  const [busy, setBusy] = useState<Set<number>>(new Set())
  const [drag, setDrag] = useState<{ from: number; over: number } | null>(null)
  const [adding, setAdding] = useState(false)
  const [draft, setDraft] = useState<NewAssignment>({ ...emptyNew })
  const [savingNew, setSavingNew] = useState(false)

  const setRowBusy = (id: number, on: boolean) =>
    setBusy(prev => { const n = new Set(prev); if (on) n.add(id); else n.delete(id); return n })

  async function patch(a: RoutingConfigProvider, p: AssignmentPatch) {
    setRowBusy(a.id, true); onError('')
    try {
      await api.updateProviderInConfig(a.id, {
        model: p.model !== undefined ? p.model : a.model,
        weight: p.weight !== undefined ? p.weight : a.weight,
        is_active: p.is_active !== undefined ? p.is_active : a.is_active,
      })
      await onChanged()
    } catch (e) { onError(e instanceof Error ? e.message : 'Failed to update assignment') }
    finally { setRowBusy(a.id, false) }
  }

  /* Drag reorder: write strictly-descending weights (n..1) so the engine's
     weight-DESC ordering matches the visual order. */
  async function commitReorder(from: number, to: number) {
    if (from === to) return
    const list = [...config.providers]
    const [moved] = list.splice(from, 1)
    list.splice(to, 0, moved)
    const n = list.length
    onError('')
    try {
      for (let i = 0; i < list.length; i++) {
        const a = list[i]
        const desired = n - i
        if (a.weight !== desired) {
          await api.updateProviderInConfig(a.id, { model: a.model, weight: desired, is_active: a.is_active })
        }
      }
      await onChanged()
    } catch (e) { onError(e instanceof Error ? e.message : 'Failed to reorder providers') }
  }

  async function addProvider() {
    if (draft.provider_id === 0) { onError('Select a provider first'); return }
    setSavingNew(true); onError('')
    try {
      await api.addProviderToConfig({
        routing_config_id: config.id, provider_id: draft.provider_id,
        model: draft.model || null, weight: draft.weight, is_active: draft.is_active,
      })
      notify('PROVIDER ADDED'); setDraft({ ...emptyNew }); setAdding(false); await onChanged()
    } catch (e) { onError(e instanceof Error ? e.message : 'Failed to add provider') }
    finally { setSavingNew(false) }
  }

  const colLabel = isPriority ? 'Priority / Weight' : 'Weight'

  return (
    <div className="panel flex flex-col">
      {/* Header */}
      <div className="flex items-start justify-between gap-3 border-b border-border/60 px-4 py-3">
        <div className="min-w-0">
          <h2 className="truncate font-mono text-[16px] font-semibold text-foreground">{config.name}</h2>
          <div className="mt-1 flex flex-wrap items-center gap-2 font-mono text-[11px] text-muted-foreground">
            <Badge variant="secondary" className="font-mono text-[10px] uppercase tracking-wider bg-secondary text-muted-foreground border-border">{config.strategy.replace(/_/g, ' ')}</Badge>
            <span className="text-border">·</span>
            <span>{config.providers.length} provider{config.providers.length !== 1 ? 's' : ''}</span>
            <span className="text-border">·</span>
            <span>health {config.health_check_enabled ? `every ${config.health_check_interval_seconds}s` : 'off'}</span>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <Button variant="outline" size="sm" onClick={onEditConfig} className="h-8 font-mono text-[11px] uppercase tracking-wider border-border text-muted-foreground hover:text-foreground">
            <PencilIcon className="size-3" /> Edit
          </Button>
          <Button variant="outline" size="sm" onClick={onDeleteConfig} className="h-8 font-mono text-[11px] uppercase tracking-wider border-border text-muted-foreground hover:text-destructive">
            <TrashIcon className="size-3" /> Delete
          </Button>
        </div>
      </div>

      {isPriority && (
        <p className="border-b border-border/40 bg-surface/40 px-4 py-2 font-mono text-[11px] text-muted-foreground">
          Tried top-to-bottom; failover moves down only when a provider is unavailable. Drag the handle to reorder.
        </p>
      )}

      {/* Assignments */}
      <div className="overflow-x-auto p-2">
        {config.providers.length === 0 && !adding ? (
          <p className="py-10 text-center font-mono text-[13px] text-muted-foreground">No providers assigned.</p>
        ) : (
          <table className="w-full table-scan">
            <thead>
              <tr className="border-b border-border/50 text-left">
                {['Provider', 'Model', colLabel, 'Status', ''].map((h, i) => (
                  <th key={i} className="px-2 py-1.5 font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-foreground">{h}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {config.providers.map((a, idx) => {
                const entry = get(a.provider_slug)
                return (
                  <AssignmentRow
                    key={a.id}
                    assignment={a}
                    index={idx}
                    isPriority={isPriority}
                    busy={busy.has(a.id)}
                    models={entry.models}
                    modelsLoading={entry.loading}
                    ensureModels={() => ensure(a.provider_slug)}
                    onPatch={p => patch(a, p)}
                    onDelete={() => onRequestDeleteAssignment(a)}
                    dragging={drag?.from === idx}
                    dragOver={drag?.over === idx && drag?.from !== idx}
                    onDragStart={() => setDrag({ from: idx, over: idx })}
                    onDragOver={e => { e.preventDefault(); setDrag(d => (d ? { ...d, over: idx } : d)) }}
                    onDrop={() => { if (drag) commitReorder(drag.from, idx); setDrag(null) }}
                    onDragEnd={() => setDrag(null)}
                  />
                )
              })}
            </tbody>
          </table>
        )}
      </div>

      {/* Inline add */}
      <div className="border-t border-border/40 p-2">
        {adding ? (
          <div className="flex flex-wrap items-end gap-2 rounded-sm bg-surface/50 p-2">
            <div className="flex flex-col gap-1">
              <label className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">Provider</label>
              <Select
                value={draft.provider_id === 0 ? '' : String(draft.provider_id)}
                onValueChange={v => { const p = providers.find(x => x.id === Number(v)); setDraft(d => ({ ...d, provider_id: Number(v), slug: p?.slug || '', model: '' })) }}
              >
                <SelectTrigger className="h-7 w-44 font-mono text-[12px] bg-surface border-border text-foreground"><SelectValue placeholder="select…" /></SelectTrigger>
                <SelectContent className="bg-card border-border">
                  <SelectGroup>{providers.map(p => <SelectItem key={p.id} value={String(p.id)} className="font-mono text-[12px] text-foreground">{p.name} ({p.slug})</SelectItem>)}</SelectGroup>
                </SelectContent>
              </Select>
            </div>
            <div className="flex flex-col gap-1">
              <label className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">Model</label>
              <ModelPicker
                className="w-44"
                triggerSize="sm"
                value={draft.model}
                models={get(draft.slug).models}
                loading={get(draft.slug).loading}
                disabled={draft.provider_id === 0}
                onOpen={() => draft.slug && ensure(draft.slug)}
                onChange={m => setDraft(d => ({ ...d, model: m }))}
                allowEmpty emptyLabel="no override" allowCustom
              />
            </div>
            <div className="flex flex-col gap-1">
              <label className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">Weight</label>
              <Input type="number" min={1} value={draft.weight} onChange={e => setDraft(d => ({ ...d, weight: Number(e.target.value) }))} className="h-7 w-16 text-right font-mono text-[12px] tabular-nums bg-surface border-border text-foreground" />
            </div>
            <button type="button" onClick={() => setDraft(d => ({ ...d, is_active: !d.is_active }))} className="flex h-7 items-center gap-1.5 px-1 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
              <Checkbox checked={draft.is_active} className="pointer-events-none size-3.5" /> active
            </button>
            <div className="ml-auto flex items-center gap-1">
              <Button size="sm" disabled={savingNew} onClick={addProvider} className="h-7 font-mono text-[11px] uppercase tracking-wider border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">{savingNew ? 'adding…' : 'Add'}</Button>
              <Button size="icon-xs" variant="ghost" onClick={() => { setAdding(false); setDraft({ ...emptyNew }) }} className="text-muted-foreground hover:text-foreground"><XIcon className="size-3.5" /></Button>
            </div>
          </div>
        ) : (
          <Button variant="secondary" size="sm" onClick={() => setAdding(true)} className="h-8 w-full font-mono text-[11px] uppercase tracking-wider bg-secondary text-muted-foreground hover:text-foreground border border-border">
            <PlusIcon className="size-3" /> Add Provider
          </Button>
        )}
      </div>
    </div>
  )
}
