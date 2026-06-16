import { useEffect, useState } from 'react'
import { PlusIcon } from 'lucide-react'
import { api } from '../api/client'
import type { RoutingConfigFull, RoutingConfigCreateRequest, RoutingConfigProvider, ProviderListItem } from '../types'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import ConfigRail from '../components/config/ConfigRail'
import AssignmentsPanel from '../components/config/AssignmentsPanel'

const emptyConfigForm: RoutingConfigCreateRequest = {
  name: '', strategy: 'round_robin', health_check_enabled: true,
  health_check_interval_seconds: 30, health_check_timeout_seconds: 10,
}

export default function Config() {
  const [configs, setConfigs] = useState<RoutingConfigFull[]>([])
  const [providers, setProviders] = useState<ProviderListItem[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [success, setSuccess] = useState<string | null>(null)
  const [selectedId, setSelectedId] = useState<number | null>(null)

  const [configDialog, setConfigDialog] = useState<{ open: boolean; editing: RoutingConfigFull | null }>({ open: false, editing: null })
  const [configForm, setConfigForm] = useState<RoutingConfigCreateRequest>({ ...emptyConfigForm })
  const [configSaving, setConfigSaving] = useState(false)
  const [deleteConfigTarget, setDeleteConfigTarget] = useState<RoutingConfigFull | null>(null)
  const [deleteAssignment, setDeleteAssignment] = useState<RoutingConfigProvider | null>(null)

  useEffect(() => { loadData() }, [])

  async function loadData() {
    try {
      setLoading(true); setError(null)
      const [c, p] = await Promise.all([api.getRoutingConfigs(), api.getProvidersList()])
      setConfigs(c); setProviders(p)
      setSelectedId(prev => (prev != null && c.some(x => x.id === prev)) ? prev : (c[0]?.id ?? null))
    } catch (e) { setError(e instanceof Error ? e.message : 'Failed to load data') }
    finally { setLoading(false) }
  }

  const selected = configs.find(c => c.id === selectedId) ?? null

  /* ── Config CRUD ──────────────────────────────────────────────── */
  function openCreateConfig() { setConfigForm({ ...emptyConfigForm }); setConfigDialog({ open: true, editing: null }) }
  function openEditConfig(config: RoutingConfigFull) {
    setConfigForm({ name: config.name, strategy: config.strategy, health_check_enabled: config.health_check_enabled, health_check_interval_seconds: config.health_check_interval_seconds, health_check_timeout_seconds: config.health_check_timeout_seconds })
    setConfigDialog({ open: true, editing: config })
  }

  async function handleConfigSave(e: React.FormEvent) {
    e.preventDefault(); setConfigSaving(true); setError(null)
    try {
      if (configDialog.editing) { await api.updateRoutingConfig(configDialog.editing.id, configForm); setSuccess('CONFIG UPDATED') }
      else { const created = await api.createRoutingConfig(configForm); setSuccess('CONFIG CREATED'); setSelectedId(created.id) }
      setConfigDialog({ open: false, editing: null }); await loadData()
    } catch (e) { setError(e instanceof Error ? e.message : 'Failed to save config') }
    finally { setConfigSaving(false) }
  }

  async function handleDeleteConfig() {
    if (!deleteConfigTarget) return
    try { await api.deleteRoutingConfig(deleteConfigTarget.id); setDeleteConfigTarget(null); setSuccess('CONFIG DELETED'); await loadData() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to delete config') }
  }

  async function handleDeleteAssignment() {
    if (!deleteAssignment) return
    try { await api.deleteProviderFromConfig(deleteAssignment.id); setDeleteAssignment(null); setSuccess('PROVIDER REMOVED'); await loadData() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to remove') }
  }

  if (loading) {
    return (
      <div className="space-y-4 p-4 sm:p-5">
        <Skeleton className="h-9 w-56 bg-secondary" />
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-[260px_1fr]">
          <Skeleton className="h-80 bg-secondary" />
          <Skeleton className="h-80 bg-secondary" />
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-4 p-4 sm:p-5">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="mb-1 font-display text-[28px] leading-none tracking-[0.04em] text-foreground">ROUTING CONFIG</h1>
          <p className="font-mono text-[13px] text-muted-foreground">Routing engines and their provider assignments</p>
        </div>
        <Button onClick={openCreateConfig} className="font-mono text-[12px] uppercase tracking-wider border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">
          <PlusIcon className="size-3.5" /> Add Config
        </Button>
      </div>

      {/* Messages */}
      {success && (
        <Alert className="border-brand/30 bg-brand/5 text-brand font-mono text-[13px]">
          <AlertDescription className="flex items-center justify-between">{success}<Button variant="ghost" size="icon-xs" onClick={() => setSuccess(null)} className="text-brand">×</Button></AlertDescription>
        </Alert>
      )}
      {error && (
        <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono text-[13px]">
          <AlertDescription className="flex items-center justify-between">{error}<Button variant="ghost" size="icon-xs" onClick={() => setError(null)} className="text-destructive">×</Button></AlertDescription>
        </Alert>
      )}

      {configs.length === 0 ? (
        <div className="panel flex flex-col items-center gap-4 p-12">
          <p className="font-mono text-[13px] text-muted-foreground">{'>'} NO ROUTING CONFIGURATIONS FOUND</p>
          <Button onClick={openCreateConfig} className="font-mono text-[12px] uppercase tracking-wider border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">
            <PlusIcon className="size-3.5" /> Create First Config
          </Button>
        </div>
      ) : (
        <div className="grid grid-cols-1 items-start gap-4 lg:grid-cols-[260px_1fr]">
          <ConfigRail configs={configs} selectedId={selectedId} onSelect={setSelectedId} onCreate={openCreateConfig} />
          {selected ? (
            <AssignmentsPanel
              key={selected.id}
              config={selected}
              providers={providers}
              notify={setSuccess}
              onError={msg => setError(msg || null)}
              onChanged={loadData}
              onEditConfig={() => openEditConfig(selected)}
              onDeleteConfig={() => setDeleteConfigTarget(selected)}
              onRequestDeleteAssignment={setDeleteAssignment}
            />
          ) : (
            <div className="panel flex items-center justify-center p-12 font-mono text-[13px] text-muted-foreground">Select a config</div>
          )}
        </div>
      )}

      {/* ── Config Create/Edit Dialog ──────────────────────────────── */}
      <Dialog open={configDialog.open} onOpenChange={o => { if (!o) setConfigDialog({ open: false, editing: null }) }}>
        <DialogContent className="sm:max-w-lg border-border bg-card">
          <DialogHeader>
            <DialogTitle className="font-display text-xl tracking-[0.04em]">{configDialog.editing ? 'EDIT CONFIG' : 'CREATE CONFIG'}</DialogTitle>
            <DialogDescription className="font-mono text-[12px] text-muted-foreground">{configDialog.editing ? 'Update routing configuration.' : 'Create a new routing engine.'}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleConfigSave} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="c-name" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Name</Label>
              <Input id="c-name" value={configForm.name} onChange={e => setConfigForm({ ...configForm, name: e.target.value })} placeholder="production" required className="font-mono bg-surface border-border text-foreground" />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="c-strategy" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Strategy</Label>
              <Select value={configForm.strategy} onValueChange={v => setConfigForm({ ...configForm, strategy: v })}>
                <SelectTrigger id="c-strategy" className="font-mono bg-surface border-border text-foreground"><SelectValue /></SelectTrigger>
                <SelectContent className="bg-card border-border">
                  <SelectGroup>{['round_robin','priority','least_loaded','random'].map(s => <SelectItem key={s} value={s} className="font-mono text-foreground">{s.replace(/_/g, ' ')}</SelectItem>)}</SelectGroup>
                </SelectContent>
              </Select>
              {configForm.strategy === 'priority' && (
                <p className="font-mono text-[11px] text-muted-foreground">Providers are tried top-to-bottom; failover only moves on when one is unavailable. Reorder by dragging in the assignments list.</p>
              )}
            </div>
            <div className="flex items-center gap-2">
              <Checkbox id="c-health" checked={configForm.health_check_enabled} onCheckedChange={c => setConfigForm({ ...configForm, health_check_enabled: !!c })} />
              <Label htmlFor="c-health" className="font-mono text-[12px] cursor-pointer">ENABLE HEALTH CHECKS</Label>
            </div>
            {configForm.health_check_enabled && (
              <div className="grid grid-cols-2 gap-4">
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="c-interval" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Interval (s)</Label>
                  <Input id="c-interval" type="number" min={1} value={configForm.health_check_interval_seconds} onChange={e => setConfigForm({ ...configForm, health_check_interval_seconds: Math.max(1, Number(e.target.value) || 1) })} className="font-mono bg-surface border-border text-foreground" />
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="c-timeout" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Timeout (s)</Label>
                  <Input id="c-timeout" type="number" min={1} value={configForm.health_check_timeout_seconds} onChange={e => setConfigForm({ ...configForm, health_check_timeout_seconds: Math.max(1, Number(e.target.value) || 1) })} className="font-mono bg-surface border-border text-foreground" />
                </div>
              </div>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setConfigDialog({ open: false, editing: null })} disabled={configSaving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
              <Button type="submit" disabled={configSaving} className="font-mono text-[12px] uppercase tracking-wider border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">{configSaving ? 'SAVING...' : configDialog.editing ? 'UPDATE' : 'CREATE'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ── Config Delete ──────────────────────────────────────────── */}
      <AlertDialog open={!!deleteConfigTarget} onOpenChange={o => { if (!o) setDeleteConfigTarget(null) }}>
        <AlertDialogContent className="border-border bg-card">
          <AlertDialogHeader>
            <AlertDialogTitle className="font-display text-xl tracking-[0.04em] text-destructive">DELETE CONFIG</AlertDialogTitle>
            <AlertDialogDescription className="font-mono text-[13px] text-muted-foreground">Delete <span className="text-foreground">{deleteConfigTarget?.name}</span>? All provider assignments will be removed.</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleDeleteConfig} className="font-mono text-[12px] uppercase tracking-wider">DELETE</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* ── Remove Assignment ──────────────────────────────────────── */}
      <AlertDialog open={!!deleteAssignment} onOpenChange={o => { if (!o) setDeleteAssignment(null) }}>
        <AlertDialogContent className="border-border bg-card">
          <AlertDialogHeader>
            <AlertDialogTitle className="font-display text-xl tracking-[0.04em] text-destructive">REMOVE PROVIDER</AlertDialogTitle>
            <AlertDialogDescription className="font-mono text-[13px] text-muted-foreground">Remove <span className="text-foreground">{deleteAssignment?.provider_name}</span> from this routing config?</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleDeleteAssignment} className="font-mono text-[12px] uppercase tracking-wider">REMOVE</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
