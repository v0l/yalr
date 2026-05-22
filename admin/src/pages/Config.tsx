import { useEffect, useState, useCallback } from 'react'
import { PlusIcon, PencilIcon, TrashIcon, Loader2Icon } from 'lucide-react'
import { api } from '../api/client'
import type { RoutingConfigFull, RoutingConfigCreateRequest, ProviderListItem } from '../types'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'

/* ── Types ──────────────────────────────────────────────────────── */

interface ProviderModel { id: string; created: number; owned_by: string }

type ProviderFormState = {
  provider_id: number; provider_slug: string; model: string; modelCustom: boolean
  weight: number; is_active: boolean
}

const emptyProviderForm: ProviderFormState = { provider_id: 0, provider_slug: '', model: '', modelCustom: false, weight: 1, is_active: true }

const emptyConfigForm: RoutingConfigCreateRequest = {
  name: '', strategy: 'round_robin', health_check_enabled: true,
  health_check_interval_seconds: 30, health_check_timeout_seconds: 10,
}

/* ═══════════════════════════════════════════════════════════════ */
/*  Config Page                                                   */
/* ═══════════════════════════════════════════════════════════════ */

export default function Config() {
  const [configs, setConfigs] = useState<RoutingConfigFull[]>([])
  const [providers, setProviders] = useState<ProviderListItem[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [successMessage, setSuccessMessage] = useState<string | null>(null)

  const [configDialog, setConfigDialog] = useState<{ open: boolean; editing: RoutingConfigFull | null }>({ open: false, editing: null })
  const [configForm, setConfigForm] = useState<RoutingConfigCreateRequest>({ ...emptyConfigForm })
  const [configSaving, setConfigSaving] = useState(false)
  const [deleteConfigTarget, setDeleteConfigTarget] = useState<RoutingConfigFull | null>(null)

  const [providerDialog, setProviderDialog] = useState<{ open: boolean; configId: number; editingId: number | null }>({ open: false, configId: 0, editingId: null })
  const [providerForm, setProviderForm] = useState<ProviderFormState>({ ...emptyProviderForm })
  const [providerModels, setProviderModels] = useState<ProviderModel[]>([])
  const [loadingModels, setLoadingModels] = useState(false)
  const [providerSaving, setProviderSaving] = useState(false)
  const [deleteAssignment, setDeleteAssignment] = useState<{ id: number; routing_config_id: number; provider_name: string } | null>(null)

  useEffect(() => { loadData() }, [])

  async function loadData() {
    try {
      setLoading(true); setError(null)
      const [c, p] = await Promise.all([api.getRoutingConfigs(), api.getProvidersList()])
      setConfigs(c); setProviders(p)
    } catch (e) { setError(e instanceof Error ? e.message : 'Failed to load data') }
    finally { setLoading(false) }
  }

  const loadModelsForProvider = useCallback(async (slug: string) => {
    if (!slug) return; setLoadingModels(true)
    try { const d = await api.getProviderModels(slug); setProviderModels(d.models) }
    catch { setProviderModels([]); setError('Failed to load models') }
    finally { setLoadingModels(false) }
  }, [])

  /* ── Config CRUD ──────────────────────────────────────────────── */

  function openCreateConfig() { setConfigForm({ ...emptyConfigForm }); setConfigDialog({ open: true, editing: null }) }
  function openEditConfig(config: RoutingConfigFull) {
    setConfigForm({ name: config.name, strategy: config.strategy, health_check_enabled: config.health_check_enabled, health_check_interval_seconds: config.health_check_interval_seconds, health_check_timeout_seconds: config.health_check_timeout_seconds })
    setConfigDialog({ open: true, editing: config })
  }

  async function handleConfigSave(e: React.FormEvent) {
    e.preventDefault(); setConfigSaving(true); setError(null)
    try {
      if (configDialog.editing) {
        await api.updateRoutingConfig(configDialog.editing.id, configForm)
        setSuccessMessage('CONFIG UPDATED')
      } else {
        await api.createRoutingConfig(configForm)
        setSuccessMessage('CONFIG CREATED')
      }
      setConfigDialog({ open: false, editing: null }); loadData()
    } catch (e) { setError(e instanceof Error ? e.message : 'Failed to save config') }
    finally { setConfigSaving(false) }
  }

  async function handleDeleteConfig() {
    if (!deleteConfigTarget) return
    try { await api.deleteRoutingConfig(deleteConfigTarget.id); setDeleteConfigTarget(null); setSuccessMessage('CONFIG DELETED'); loadData() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to delete config') }
  }

  /* ── Provider assignment CRUD ─────────────────────────────────── */

  function openAddProvider(configId: number) { setProviderForm({ ...emptyProviderForm }); setProviderModels([]); setProviderDialog({ open: true, configId, editingId: null }) }
  function openEditProvider(configId: number, a: RoutingConfigFull['providers'][0]) {
    const p = providers.find(x => x.id === a.provider_id)
    setProviderForm({ provider_id: a.provider_id, provider_slug: p?.slug || '', model: a.model || '', modelCustom: !!(a.model && !p), weight: a.weight, is_active: a.is_active })
    setProviderModels([]); if (p) loadModelsForProvider(p.slug)
    setProviderDialog({ open: true, configId, editingId: a.id })
  }

  function handleProviderChange(id: number) {
    const p = providers.find(x => x.id === id)
    setProviderForm(prev => ({ ...prev, provider_id: id, provider_slug: p?.slug || '', model: '', modelCustom: false }))
    if (p?.slug) loadModelsForProvider(p.slug); else setProviderModels([])
  }

  function handleModelSelect(modelId: string) {
    if (modelId === '__custom__') setProviderForm(prev => ({ ...prev, model: '', modelCustom: true }))
    else setProviderForm(prev => ({ ...prev, model: modelId, modelCustom: false }))
  }

  async function handleProviderSave(e: React.FormEvent) {
    e.preventDefault()
    if (providerForm.provider_id === 0) { setError('Please select a provider'); return }
    setProviderSaving(true); setError(null)
    try {
      if (providerDialog.editingId) {
        await api.updateProviderInConfig(providerDialog.editingId, { model: providerForm.model || null, weight: providerForm.weight, is_active: providerForm.is_active })
        setSuccessMessage('ASSIGNMENT UPDATED')
      } else {
        await api.addProviderToConfig({ routing_config_id: providerDialog.configId, provider_id: providerForm.provider_id, model: providerForm.model || null, weight: providerForm.weight, is_active: providerForm.is_active })
        setSuccessMessage('PROVIDER ADDED')
      }
      setProviderDialog({ open: false, configId: 0, editingId: null }); loadData()
    } catch (e) { setError(e instanceof Error ? e.message : 'Failed to save assignment') }
    finally { setProviderSaving(false) }
  }

  async function handleDeleteAssignment() {
    if (!deleteAssignment) return
    try { await api.deleteProviderFromConfig(deleteAssignment.id); setDeleteAssignment(null); setSuccessMessage('PROVIDER REMOVED'); loadData() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to remove') }
  }

  /* ── Loading ─────────────────────────────────────────────────── */
  if (loading) {
    return (
      <div className="space-y-6 p-6">
        <div className="flex items-center justify-between">
          <div><Skeleton className="h-8 w-48 bg-[#1c1c1e]" /><Skeleton className="h-4 w-64 bg-[#1c1c1e] mt-1" /></div>
          <Skeleton className="h-9 w-40 bg-[#1c1c1e]" />
        </div>
        <Skeleton className="h-32 bg-[#1c1c1e]" />
        <Skeleton className="h-32 bg-[#1c1c1e]" />
      </div>
    )
  }

  /* ── Render ──────────────────────────────────────────────────── */
  return (
    <div className="space-y-6 p-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground leading-none mb-1">ROUTING CONFIG</h1>
          <p className="font-mono text-[13px] text-muted-foreground">Manage routing engines and their provider assignments</p>
        </div>
        <Button onClick={openCreateConfig} className="font-mono text-[12px] tracking-wider uppercase border border-[#4ce04c]/40 bg-[#4ce04c]/10 text-[#4ce04c] hover:bg-[#4ce04c]/20">
          <PlusIcon className="size-3.5" /> Add Config
        </Button>
      </div>

      {/* Messages */}
      {successMessage && (
        <Alert className="border-[#4ce04c]/30 bg-[#4ce04c]/5 text-[#4ce04c] font-mono text-[13px]">
          <AlertDescription className="flex items-center justify-between">{successMessage}<Button variant="ghost" size="icon-xs" onClick={() => setSuccessMessage(null)} className="text-[#4ce04c]">×</Button></AlertDescription>
        </Alert>
      )}
      {error && (
        <Alert className="border-[#ff3333]/30 bg-[#ff3333]/5 text-[#ff3333] font-mono text-[13px]">
          <AlertDescription className="flex items-center justify-between">{error}<Button variant="ghost" size="icon-xs" onClick={() => setError(null)} className="text-[#ff3333]">×</Button></AlertDescription>
        </Alert>
      )}

      {/* Empty state */}
      {configs.length === 0 ? (
        <div className="panel p-12 flex flex-col items-center gap-4">
          <p className="font-mono text-[13px] text-[#716d66]">{'>'} NO ROUTING CONFIGURATIONS FOUND</p>
          <Button onClick={openCreateConfig} className="font-mono text-[12px] tracking-wider uppercase border border-[#4ce04c]/40 bg-[#4ce04c]/10 text-[#4ce04c] hover:bg-[#4ce04c]/20">
            <PlusIcon className="size-3.5" /> Create First Config
          </Button>
        </div>
      ) : (
        <div className="space-y-4">
          {configs.map(config => (
            <div key={config.id} className="panel p-5 space-y-4">
              {/* Config header */}
              <div className="flex items-start justify-between">
                <div>
                  <h2 className="font-mono text-[16px] font-semibold mb-1">{config.name}</h2>
                  <div className="flex flex-wrap items-center gap-2 font-mono text-[12px] text-[#716d66]">
                    <Badge variant="secondary" className="font-mono text-[10px] uppercase tracking-wider bg-[#1c1c1e] text-[#716d66] border-[#2a2a2e]">{config.strategy.replace(/_/g, ' ')}</Badge>
                    <span className="text-[#3a3a3e]">|</span>
                    <span>{config.providers.length} provider{config.providers.length !== 1 ? 's' : ''}</span>
                    <span className="text-[#3a3a3e]">|</span>
                    <span>Health: {config.health_check_enabled ? `EVERY ${config.health_check_interval_seconds}s` : 'OFF'}</span>
                  </div>
                </div>
                <div className="flex items-center gap-1">
                  <Button variant="outline" size="sm" onClick={() => openEditConfig(config)} className="font-mono text-[11px] uppercase tracking-wider border-[#2a2a2e] text-[#716d66] hover:text-[#d4d0c8] h-8">
                    <PencilIcon className="size-3" /> Edit
                  </Button>
                  <Button variant="outline" size="sm" onClick={() => setDeleteConfigTarget(config)} className="font-mono text-[11px] uppercase tracking-wider border-[#2a2a2e] text-[#716d66] hover:text-[#ff3333] h-8">
                    <TrashIcon className="size-3" /> Delete
                  </Button>
                </div>
              </div>

              {/* Assignments */}
              <div>
                <div className="flex items-center justify-between mb-3">
                  <h3 className="font-mono text-[10px] uppercase tracking-[0.12em] text-[#716d66]">Provider Assignments</h3>
                  <Button variant="secondary" size="xs" onClick={() => openAddProvider(config.id)} className="font-mono text-[10px] uppercase tracking-wider bg-[#1c1c1e] text-[#716d66] hover:text-[#d4d0c8] border border-[#2a2a2e] h-7">
                    <PlusIcon className="size-3" /> Add
                  </Button>
                </div>
                {config.providers.length === 0 ? (
                  <p className="py-8 text-center font-mono text-[13px] text-[#716d66]">No providers assigned.</p>
                ) : (
                  <div className="overflow-x-auto">
                    <table className="w-full table-scan">
                      <thead>
                        <tr className="border-b border-[#1a1a1e] text-left">
                          <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66] px-3 py-2 font-medium">Provider</th>
                          <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66] px-3 py-2 font-medium">Model</th>
                          <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66] px-3 py-2 font-medium text-right">Weight</th>
                          <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66] px-3 py-2 font-medium">Status</th>
                          <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66] px-3 py-2 font-medium text-right">Actions</th>
                        </tr>
                      </thead>
                      <tbody>
                        {config.providers.map(a => (
                          <tr key={a.id} className="border-b border-[#1a1a1e] hover:bg-[#0d0d0f]">
                            <td className="px-3 py-2">
                              <span className="font-mono text-[13px] font-medium">{a.provider_name}</span>
                              <span className="ml-1.5 font-mono text-[11px] text-[#716d66]">{a.provider_slug}</span>
                            </td>
                            <td className="px-3 py-2">
                              {a.model ? <code className="font-mono text-[13px]">{a.model}</code> : <span className="font-mono text-[12px] text-[#716d66]">—</span>}
                            </td>
                            <td className="px-3 py-2 text-right font-mono text-[13px] tabular-nums">{a.weight}</td>
                            <td className="px-3 py-2">
                              {a.is_active
                                ? <Badge className="bg-[#4ce04c]/15 text-[#4ce04c] border-[#4ce04c]/30 font-mono text-[10px] tracking-wider uppercase">ACTIVE</Badge>
                                : <Badge className="bg-[#1c1c1e] text-[#716d66] border-[#2a2a2e] font-mono text-[10px] tracking-wider uppercase">INACTIVE</Badge>}
                            </td>
                            <td className="px-3 py-2">
                              <div className="flex items-center justify-end gap-1">
                                <Button variant="ghost" size="icon-xs" onClick={() => openEditProvider(config.id, a)} className="text-[#716d66] hover:text-[#d4d0c8]"><PencilIcon className="size-3" /></Button>
                                <Button variant="ghost" size="icon-xs" onClick={() => setDeleteAssignment({ id: a.id, routing_config_id: config.id, provider_name: a.provider_name })} className="text-[#716d66] hover:text-[#ff3333]"><TrashIcon className="size-3" /></Button>
                              </div>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      {/* ── Config Create/Edit Dialog ──────────────────────────────── */}
      <Dialog open={configDialog.open} onOpenChange={o => { if (!o) setConfigDialog({ open: false, editing: null }) }}>
        <DialogContent className="sm:max-w-lg border-[#2a2a2e] bg-[#111113]">
          <DialogHeader>
            <DialogTitle className="font-display text-xl tracking-[0.04em]">{configDialog.editing ? 'EDIT CONFIG' : 'CREATE CONFIG'}</DialogTitle>
            <DialogDescription className="font-mono text-[12px] text-[#716d66]">{configDialog.editing ? 'Update routing configuration.' : 'Create a new routing engine.'}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleConfigSave} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="c-name" className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66]">Name</Label>
              <Input id="c-name" value={configForm.name} onChange={e => setConfigForm({ ...configForm, name: e.target.value })} placeholder="production" required className="font-mono bg-[#0d0d0f] border-[#2a2a2e] text-[#d4d0c8]" />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="c-strategy" className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66]">Strategy</Label>
              <Select value={configForm.strategy} onValueChange={v => setConfigForm({ ...configForm, strategy: v })}>
                <SelectTrigger id="c-strategy" className="font-mono bg-[#0d0d0f] border-[#2a2a2e] text-[#d4d0c8]"><SelectValue /></SelectTrigger>
                <SelectContent className="bg-[#111113] border-[#2a2a2e]">
                  <SelectGroup>
                    {['round_robin','least_loaded','random'].map(s => <SelectItem key={s} value={s} className="font-mono text-[#d4d0c8]">{s.replace(/_/g, ' ')}</SelectItem>)}
                  </SelectGroup>
                </SelectContent>
              </Select>
            </div>
            <div className="flex items-center gap-2">
              <Checkbox id="c-health" checked={configForm.health_check_enabled} onCheckedChange={c => setConfigForm({ ...configForm, health_check_enabled: !!c })} />
              <Label htmlFor="c-health" className="font-mono text-[12px] cursor-pointer">ENABLE HEALTH CHECKS</Label>
            </div>
            {configForm.health_check_enabled && (
              <div className="grid grid-cols-2 gap-4">
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="c-interval" className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66]">Interval (s)</Label>
                  <Input id="c-interval" type="number" min={1} value={configForm.health_check_interval_seconds} onChange={e => setConfigForm({ ...configForm, health_check_interval_seconds: Number(e.target.value) })} className="font-mono bg-[#0d0d0f] border-[#2a2a2e] text-[#d4d0c8]" />
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="c-timeout" className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66]">Timeout (s)</Label>
                  <Input id="c-timeout" type="number" min={1} value={configForm.health_check_timeout_seconds} onChange={e => setConfigForm({ ...configForm, health_check_timeout_seconds: Number(e.target.value) })} className="font-mono bg-[#0d0d0f] border-[#2a2a2e] text-[#d4d0c8]" />
                </div>
              </div>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setConfigDialog({ open: false, editing: null })} disabled={configSaving} className="font-mono text-[12px] border-[#2a2a2e] text-[#716d66]">CANCEL</Button>
              <Button type="submit" disabled={configSaving} className="font-mono text-[12px] tracking-wider uppercase border border-[#4ce04c]/40 bg-[#4ce04c]/10 text-[#4ce04c] hover:bg-[#4ce04c]/20">{configSaving ? 'SAVING...' : configDialog.editing ? 'UPDATE' : 'CREATE'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ── Config Delete ──────────────────────────────────────────── */}
      <AlertDialog open={!!deleteConfigTarget} onOpenChange={o => { if (!o) setDeleteConfigTarget(null) }}>
        <AlertDialogContent className="border-[#2a2a2e] bg-[#111113]">
          <AlertDialogHeader>
            <AlertDialogTitle className="font-display text-xl tracking-[0.04em] text-[#ff3333]">DELETE CONFIG</AlertDialogTitle>
            <AlertDialogDescription className="font-mono text-[13px] text-[#716d66]">Delete <span className="text-[#d4d0c8]">{deleteConfigTarget?.name}</span>? All provider assignments will be removed.</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel className="font-mono text-[12px] border-[#2a2a2e] text-[#716d66]">CANCEL</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleDeleteConfig} className="font-mono text-[12px] tracking-wider uppercase">DELETE</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* ── Provider Assignment Dialog ─────────────────────────────── */}
      <Dialog open={providerDialog.open} onOpenChange={o => { if (!o) setProviderDialog({ open: false, configId: 0, editingId: null }) }}>
        <DialogContent className="sm:max-w-lg border-[#2a2a2e] bg-[#111113]">
          <DialogHeader>
            <DialogTitle className="font-display text-xl tracking-[0.04em]">{providerDialog.editingId ? 'EDIT ASSIGNMENT' : 'ADD PROVIDER'}</DialogTitle>
            <DialogDescription className="font-mono text-[12px] text-[#716d66]">{providerDialog.editingId ? 'Update this assignment.' : 'Add a provider to this routing config.'}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleProviderSave} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="pa-provider" className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66]">Provider</Label>
              <Select value={providerForm.provider_id === 0 ? '' : String(providerForm.provider_id)} onValueChange={v => handleProviderChange(Number(v))} disabled={!!providerDialog.editingId}>
                <SelectTrigger id="pa-provider" className="font-mono bg-[#0d0d0f] border-[#2a2a2e] text-[#d4d0c8]"><SelectValue placeholder="Select a provider..." /></SelectTrigger>
                <SelectContent className="bg-[#111113] border-[#2a2a2e]">
                  <SelectGroup>{providers.map(p => <SelectItem key={p.id} value={String(p.id)} className="font-mono text-[#d4d0c8]">{p.name} ({p.slug})</SelectItem>)}</SelectGroup>
                </SelectContent>
              </Select>
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="pa-model" className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66]">Model</Label>
              {providerForm.provider_id === 0 ? (
                <p className="font-mono text-[12px] text-[#716d66]">Select a provider first</p>
              ) : loadingModels ? (
                <div className="flex items-center gap-2 font-mono text-[12px] text-[#716d66]"><Loader2Icon className="animate-spin size-3.5" /> Loading models...</div>
              ) : (
                <>
                  <Select value={providerForm.modelCustom ? '__custom__' : (providerForm.model || 'empty')} onValueChange={handleModelSelect}>
                    <SelectTrigger id="pa-model" className="font-mono bg-[#0d0d0f] border-[#2a2a2e] text-[#d4d0c8]"><SelectValue placeholder="No model override" /></SelectTrigger>
                    <SelectContent className="bg-[#111113] border-[#2a2a2e]">
                      <SelectGroup>
                        <SelectItem value="empty" className="font-mono text-[#716d66]">No model override</SelectItem>
                        {providerModels.map(m => <SelectItem key={m.id} value={m.id} className="font-mono text-[#d4d0c8]">{m.id}</SelectItem>)}
                        <SelectItem value="__custom__" className="font-mono text-[#4ce04c]">Custom model name...</SelectItem>
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                  {providerForm.modelCustom && <Input value={providerForm.model} onChange={e => setProviderForm({ ...providerForm, model: e.target.value })} placeholder="Enter custom model name" className="mt-1 font-mono bg-[#0d0d0f] border-[#2a2a2e] text-[#d4d0c8]" />}
                </>
              )}
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="pa-weight" className="font-mono text-[10px] uppercase tracking-[0.1em] text-[#716d66]">Weight</Label>
                <Input id="pa-weight" type="number" min={1} value={providerForm.weight} onChange={e => setProviderForm({ ...providerForm, weight: Number(e.target.value) })} className="font-mono bg-[#0d0d0f] border-[#2a2a2e] text-[#d4d0c8]" />
              </div>
              <div className="flex items-end pb-1.5">
                <div className="flex items-center gap-2">
                  <Checkbox id="pa-active" checked={providerForm.is_active} onCheckedChange={c => setProviderForm({ ...providerForm, is_active: !!c })} />
                  <Label htmlFor="pa-active" className="font-mono text-[12px] cursor-pointer">ACTIVE</Label>
                </div>
              </div>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setProviderDialog({ open: false, configId: 0, editingId: null })} disabled={providerSaving} className="font-mono text-[12px] border-[#2a2a2e] text-[#716d66]">CANCEL</Button>
              <Button type="submit" disabled={providerSaving} className="font-mono text-[12px] tracking-wider uppercase border border-[#4ce04c]/40 bg-[#4ce04c]/10 text-[#4ce04c] hover:bg-[#4ce04c]/20">{providerSaving ? 'SAVING...' : providerDialog.editingId ? 'UPDATE' : 'ADD'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ── Remove Assignment Confirmation ──────────────────────────── */}
      <AlertDialog open={!!deleteAssignment} onOpenChange={o => { if (!o) setDeleteAssignment(null) }}>
        <AlertDialogContent className="border-[#2a2a2e] bg-[#111113]">
          <AlertDialogHeader>
            <AlertDialogTitle className="font-display text-xl tracking-[0.04em] text-[#ff3333]">REMOVE PROVIDER</AlertDialogTitle>
            <AlertDialogDescription className="font-mono text-[13px] text-[#716d66]">Remove <span className="text-[#d4d0c8]">{deleteAssignment?.provider_name}</span> from this routing config?</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel className="font-mono text-[12px] border-[#2a2a2e] text-[#716d66]">CANCEL</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleDeleteAssignment} className="font-mono text-[12px] tracking-wider uppercase">REMOVE</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
