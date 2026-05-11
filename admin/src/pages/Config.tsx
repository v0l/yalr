import { useEffect, useState, useCallback } from 'react'
import { PlusIcon, PencilIcon, TrashIcon, Loader2Icon } from 'lucide-react'
import { api } from '../api/client'
import type {
  RoutingConfigFull,
  RoutingConfigCreateRequest,
  RoutingConfigProviderCreateRequest,
  RoutingConfigProviderUpdateRequest,
  ProviderListItem,
} from '../types'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { Card, CardContent } from '@/components/ui/card'

// ── Types ───────────────────────────────────────────────────────────────────

interface ProviderModel {
  id: string
  created: number
  owned_by: string
}

type ProviderFormState = {
  provider_id: number
  provider_slug: string
  model: string
  modelCustom: boolean
  weight: number
  is_active: boolean
}

const emptyProviderForm: ProviderFormState = {
  provider_id: 0,
  provider_slug: '',
  model: '',
  modelCustom: false,
  weight: 1,
  is_active: true,
}

const emptyConfigForm: RoutingConfigCreateRequest = {
  name: '',
  strategy: 'round_robin',
  health_check_enabled: true,
  health_check_interval_seconds: 30,
  health_check_timeout_seconds: 10,
}

// ── Page component ──────────────────────────────────────────────────────────

export default function Config() {
  const [configs, setConfigs] = useState<RoutingConfigFull[]>([])
  const [providers, setProviders] = useState<ProviderListItem[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [successMessage, setSuccessMessage] = useState<string | null>(null)

  // Config dialog
  const [configDialog, setConfigDialog] = useState<{ open: boolean; editing: RoutingConfigFull | null }>({ open: false, editing: null })
  const [configForm, setConfigForm] = useState<RoutingConfigCreateRequest>({ ...emptyConfigForm })
  const [configSaving, setConfigSaving] = useState(false)

  // Delete config
  const [deleteConfigTarget, setDeleteConfigTarget] = useState<RoutingConfigFull | null>(null)

  // Provider assignment dialog
  const [providerDialog, setProviderDialog] = useState<{
    open: boolean
    configId: number
    editingId: number | null
  }>({ open: false, configId: 0, editingId: null })
  const [providerForm, setProviderForm] = useState<ProviderFormState>({ ...emptyProviderForm })
  const [providerModels, setProviderModels] = useState<ProviderModel[]>([])
  const [loadingModels, setLoadingModels] = useState(false)
  const [providerSaving, setProviderSaving] = useState(false)

  // Delete assignment
  const [deleteAssignment, setDeleteAssignment] = useState<{
    id: number
    routing_config_id: number
    provider_name: string
  } | null>(null)

  useEffect(() => { loadData() }, [])

  // ── Data ──────────────────────────────────────────────────────────────────

  async function loadData() {
    try {
      setLoading(true)
      setError(null)
      const [configsData, providersData] = await Promise.all([
        api.getRoutingConfigs(),
        api.getProvidersList(),
      ])
      setConfigs(configsData)
      setProviders(providersData)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load data')
    } finally {
      setLoading(false)
    }
  }

  const loadModelsForProvider = useCallback(async (slug: string) => {
    if (!slug) return
    setLoadingModels(true)
    try {
      const data = await api.getProviderModels(slug)
      setProviderModels(data.models)
    } catch {
      setProviderModels([])
      setError('Failed to load models from provider')
    } finally {
      setLoadingModels(false)
    }
  }, [])

  // ── Config CRUD ───────────────────────────────────────────────────────────

  function openCreateConfig() {
    setConfigForm({ ...emptyConfigForm })
    setConfigDialog({ open: true, editing: null })
  }

  function openEditConfig(config: RoutingConfigFull) {
    setConfigForm({
      name: config.name,
      strategy: config.strategy,
      health_check_enabled: config.health_check_enabled,
      health_check_interval_seconds: config.health_check_interval_seconds,
      health_check_timeout_seconds: config.health_check_timeout_seconds,
    })
    setConfigDialog({ open: true, editing: config })
  }

  async function handleConfigSave(e: React.FormEvent) {
    e.preventDefault()
    setConfigSaving(true)
    setError(null)
    try {
      if (configDialog.editing) {
        const updateData = {
          name: configForm.name,
          strategy: configForm.strategy,
          health_check_enabled: configForm.health_check_enabled,
          health_check_interval_seconds: configForm.health_check_interval_seconds,
          health_check_timeout_seconds: configForm.health_check_timeout_seconds,
        }
        await api.updateRoutingConfig(configDialog.editing.id, updateData)
        setSuccessMessage('Routing config updated')
      } else {
        await api.createRoutingConfig(configForm)
        setSuccessMessage('Routing config created')
      }
      setConfigDialog({ open: false, editing: null })
      loadData()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save routing config')
    } finally {
      setConfigSaving(false)
    }
  }

  async function handleDeleteConfig() {
    if (!deleteConfigTarget) return
    try {
      await api.deleteRoutingConfig(deleteConfigTarget.id)
      setDeleteConfigTarget(null)
      setSuccessMessage('Routing config deleted')
      loadData()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to delete routing config')
    }
  }

  // ── Provider assignment CRUD ──────────────────────────────────────────────

  function openAddProvider(configId: number) {
    setProviderForm({ ...emptyProviderForm })
    setProviderModels([])
    setProviderDialog({ open: true, configId, editingId: null })
  }

  function openEditProvider(configId: number, assignment: RoutingConfigFull['providers'][0]) {
    const p = providers.find((x) => x.id === assignment.provider_id)
    setProviderForm({
      provider_id: assignment.provider_id,
      provider_slug: p?.slug || '',
      model: assignment.model || '',
      modelCustom: !!(assignment.model && !p),
      weight: assignment.weight,
      is_active: assignment.is_active,
    })
    setProviderModels([])
    if (p) loadModelsForProvider(p.slug)
    setProviderDialog({ open: true, configId, editingId: assignment.id })
  }

  function handleProviderChange(providerId: number) {
    const p = providers.find((x) => x.id === providerId)
    const slug = p?.slug || ''
    setProviderForm((prev) => ({ ...prev, provider_id: providerId, provider_slug: slug, model: '', modelCustom: false }))
    if (slug) {
      loadModelsForProvider(slug)
    } else {
      setProviderModels([])
    }
  }

  function handleModelSelect(modelId: string) {
    if (modelId === '__custom__') {
      setProviderForm((prev) => ({ ...prev, model: '', modelCustom: true }))
    } else {
      setProviderForm((prev) => ({ ...prev, model: modelId, modelCustom: false }))
    }
  }

  async function handleProviderSave(e: React.FormEvent) {
    e.preventDefault()
    if (providerForm.provider_id === 0) {
      setError('Please select a provider')
      return
    }
    setProviderSaving(true)
    setError(null)
    try {
      const modelValue = providerForm.model || null
      if (providerDialog.editingId) {
        const data: RoutingConfigProviderUpdateRequest = {
          model: modelValue,
          weight: providerForm.weight,
          is_active: providerForm.is_active,
        }
        await api.updateProviderInConfig(providerDialog.editingId, data)
        setSuccessMessage('Provider assignment updated')
      } else {
        const data: RoutingConfigProviderCreateRequest = {
          routing_config_id: providerDialog.configId,
          provider_id: providerForm.provider_id,
          model: modelValue,
          weight: providerForm.weight,
          is_active: providerForm.is_active,
        }
        await api.addProviderToConfig(data)
        setSuccessMessage('Provider added to routing config')
      }
      setProviderDialog({ open: false, configId: 0, editingId: null })
      loadData()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save provider assignment')
    } finally {
      setProviderSaving(false)
    }
  }

  async function handleDeleteAssignment() {
    if (!deleteAssignment) return
    try {
      await api.deleteProviderFromConfig(deleteAssignment.id)
      setDeleteAssignment(null)
      setSuccessMessage('Provider removed from routing config')
      loadData()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to remove provider')
    }
  }

  // ── Render ────────────────────────────────────────────────────────────────

  if (loading) {
    return (
      <div className="flex flex-col gap-6 p-6">
        <div className="flex items-center justify-between">
          <div className="flex flex-col gap-1">
            <Skeleton className="h-7 w-48" />
            <Skeleton className="h-4 w-64" />
          </div>
          <Skeleton className="h-8 w-40" />
        </div>
        <div className="flex flex-col gap-4">
          <Skeleton className="h-32 w-full" />
          <Skeleton className="h-32 w-full" />
        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-6 p-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold text-foreground">Routing Configurations</h1>
          <p className="text-sm text-muted-foreground">Manage routing engines and their provider assignments</p>
        </div>
        <Button onClick={openCreateConfig}>
          <PlusIcon />
          Add Routing Config
        </Button>
      </div>

      {/* Messages */}
      {successMessage && (
        <Alert>
          <AlertDescription className="flex items-center justify-between">
            {successMessage}
            <Button variant="ghost" size="icon-xs" onClick={() => setSuccessMessage(null)}>×</Button>
          </AlertDescription>
        </Alert>
      )}
      {error && (
        <Alert variant="destructive">
          <AlertDescription className="flex items-center justify-between">
            {error}
            <Button variant="ghost" size="icon-xs" onClick={() => setError(null)}>×</Button>
          </AlertDescription>
        </Alert>
      )}

      {/* Empty state */}
      {configs.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-4 py-12">
            <p className="text-muted-foreground">No routing configurations found</p>
            <Button onClick={openCreateConfig}>
              <PlusIcon />
              Create Your First Routing Config
            </Button>
          </CardContent>
        </Card>
      ) : (
        <div className="flex flex-col gap-4">
          {configs.map((config) => (
            <Card key={config.id}>
              <CardContent className="flex flex-col gap-4 pt-4">
                {/* Config header */}
                <div className="flex items-start justify-between">
                  <div className="flex flex-col gap-1">
                    <h2 className="font-heading text-lg font-semibold">{config.name}</h2>
                    <div className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
                      <Badge variant="secondary" className="font-mono">{config.strategy}</Badge>
                      <span>·</span>
                      <span>{config.providers.length} provider{config.providers.length !== 1 ? 's' : ''}</span>
                      <span>·</span>
                      <span>Health: {config.health_check_enabled ? `Every ${config.health_check_interval_seconds}s` : 'Disabled'}</span>
                    </div>
                  </div>
                  <div className="flex items-center gap-1">
                    <Button variant="outline" size="sm" onClick={() => openEditConfig(config)}>
                      <PencilIcon />
                      Edit
                    </Button>
                    <Button variant="outline" size="sm" onClick={() => setDeleteConfigTarget(config)}>
                      <TrashIcon />
                      Delete
                    </Button>
                  </div>
                </div>

                {/* Provider assignments */}
                <div>
                  <div className="flex items-center justify-between mb-3">
                    <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">Provider Assignments</h3>
                    <Button variant="secondary" size="xs" onClick={() => openAddProvider(config.id)}>
                      <PlusIcon />
                      Add Provider
                    </Button>
                  </div>
                  {config.providers.length === 0 ? (
                    <p className="py-8 text-center text-sm text-muted-foreground">No providers assigned.</p>
                  ) : (
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>Provider</TableHead>
                          <TableHead>Model</TableHead>
                          <TableHead className="text-right">Weight</TableHead>
                          <TableHead>Status</TableHead>
                          <TableHead className="w-20 text-right">Actions</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {config.providers.map((p) => (
                          <TableRow key={p.id}>
                            <TableCell>
                              <span className="font-medium">{p.provider_name}</span>
                              <span className="ml-1.5 font-mono text-xs text-muted-foreground">{p.provider_slug}</span>
                            </TableCell>
                            <TableCell>
                              {p.model ? (
                                <code className="text-sm text-foreground">{p.model}</code>
                              ) : (
                                <span className="text-muted-foreground/60">—</span>
                              )}
                            </TableCell>
                            <TableCell className="text-right">{p.weight}</TableCell>
                            <TableCell>
                              {p.is_active ? (
                                <Badge variant="secondary">Active</Badge>
                              ) : (
                                <Badge variant="outline">Inactive</Badge>
                              )}
                            </TableCell>
                            <TableCell className="text-right">
                              <div className="flex items-center justify-end gap-1">
                                <Button variant="ghost" size="icon-xs" onClick={() => openEditProvider(config.id, p)}>
                                  <PencilIcon />
                                </Button>
                                <Button variant="ghost" size="icon-xs" onClick={() => setDeleteAssignment({ id: p.id, routing_config_id: config.id, provider_name: p.provider_name })}>
                                  <TrashIcon />
                                </Button>
                              </div>
                            </TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  )}
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      {/* ── Config Create/Edit Dialog ──────────────────────────────────── */}
      <Dialog open={configDialog.open} onOpenChange={(open) => { if (!open) setConfigDialog({ open: false, editing: null }) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{configDialog.editing ? 'Edit Routing Config' : 'Create Routing Config'}</DialogTitle>
            <DialogDescription>
              {configDialog.editing ? 'Update the routing configuration.' : 'Create a new routing engine.'}
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handleConfigSave} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="c-name">Name</Label>
              <Input
                id="c-name"
                value={configForm.name}
                onChange={(e) => setConfigForm({ ...configForm, name: e.target.value })}
                placeholder="production"
                required
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="c-strategy">Strategy</Label>
              <Select value={configForm.strategy} onValueChange={(v) => setConfigForm({ ...configForm, strategy: v })}>
                <SelectTrigger id="c-strategy" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    <SelectItem value="round_robin">Round Robin</SelectItem>
                    <SelectItem value="least_loaded">Least Loaded</SelectItem>
                    <SelectItem value="random">Random</SelectItem>
                  </SelectGroup>
                </SelectContent>
              </Select>
            </div>
            <div className="flex items-center gap-2">
              <Checkbox
                id="c-health"
                checked={configForm.health_check_enabled}
                onCheckedChange={(checked) => setConfigForm({ ...configForm, health_check_enabled: !!checked })}
              />
              <Label htmlFor="c-health" className="cursor-pointer">Enable health checks</Label>
            </div>
            {configForm.health_check_enabled && (
              <div className="grid grid-cols-2 gap-4">
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="c-interval">Interval (s)</Label>
                  <Input
                    id="c-interval"
                    type="number"
                    min={1}
                    value={configForm.health_check_interval_seconds}
                    onChange={(e) => setConfigForm({ ...configForm, health_check_interval_seconds: Number(e.target.value) })}
                  />
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="c-timeout">Timeout (s)</Label>
                  <Input
                    id="c-timeout"
                    type="number"
                    min={1}
                    value={configForm.health_check_timeout_seconds}
                    onChange={(e) => setConfigForm({ ...configForm, health_check_timeout_seconds: Number(e.target.value) })}
                  />
                </div>
              </div>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setConfigDialog({ open: false, editing: null })} disabled={configSaving}>
                Cancel
              </Button>
              <Button type="submit" disabled={configSaving}>
                {configSaving ? 'Saving...' : configDialog.editing ? 'Update' : 'Create'}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ── Config Delete Confirmation ──────────────────────────────────── */}
      <AlertDialog open={!!deleteConfigTarget} onOpenChange={(open) => { if (!open) setDeleteConfigTarget(null) }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Routing Config</AlertDialogTitle>
            <AlertDialogDescription>
              Delete <span className="font-medium text-foreground">{deleteConfigTarget?.name}</span>?
              All provider assignments will be removed. This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleDeleteConfig}>Delete</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* ── Provider Assignment Dialog ──────────────────────────────────── */}
      <Dialog open={providerDialog.open} onOpenChange={(open) => { if (!open) setProviderDialog({ open: false, configId: 0, editingId: null }) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{providerDialog.editingId ? 'Edit Provider Assignment' : 'Add Provider to Routing Config'}</DialogTitle>
            <DialogDescription>
              {providerDialog.editingId ? 'Update this assignment.' : 'Add a new provider to this routing config.'}
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handleProviderSave} className="flex flex-col gap-4">
            {/* Provider */}
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="pa-provider">Provider</Label>
              <Select
                value={providerForm.provider_id === 0 ? '' : String(providerForm.provider_id)}
                onValueChange={(v) => handleProviderChange(Number(v))}
                disabled={!!providerDialog.editingId}
              >
                <SelectTrigger id="pa-provider" className="w-full">
                  <SelectValue placeholder="Select a provider..." />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {providers.map((p) => (
                      <SelectItem key={p.id} value={String(p.id)}>{p.name} ({p.slug})</SelectItem>
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
            </div>

            {/* Model */}
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="pa-model">Model</Label>
              {providerForm.provider_id === 0 ? (
                <p className="text-sm text-muted-foreground mt-0.5">Select a provider first</p>
              ) : loadingModels ? (
                <div className="flex items-center gap-2 text-sm text-muted-foreground mt-0.5">
                  <Loader2Icon className="animate-spin size-3.5" />
                  Loading models...
                </div>
              ) : (
                <>
                  <Select
                    value={providerForm.modelCustom ? '__custom__' : providerForm.model}
                    onValueChange={handleModelSelect}
                  >
                    <SelectTrigger id="pa-model" className="w-full">
                      <SelectValue placeholder="No model override (use request model)" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value="empty" className="text-muted-foreground">No model override (use request model)</SelectItem>
                        {providerModels.map((m) => (
                          <SelectItem key={m.id} value={m.id}>{m.id}</SelectItem>
                        ))}
                        <SelectItem value="__custom__">Custom model name...</SelectItem>
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                  {providerForm.modelCustom && (
                    <Input
                      value={providerForm.model}
                      onChange={(e) => setProviderForm({ ...providerForm, model: e.target.value })}
                      placeholder="Enter custom model name"
                      className="mt-1"
                    />
                  )}
                </>
              )}
            </div>

            {/* Weight + Active */}
            <div className="grid grid-cols-2 gap-4">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="pa-weight">Weight</Label>
                <Input
                  id="pa-weight"
                  type="number"
                  min={1}
                  value={providerForm.weight}
                  onChange={(e) => setProviderForm({ ...providerForm, weight: Number(e.target.value) })}
                />
              </div>
              <div className="flex items-end pb-1.5">
                <div className="flex items-center gap-2">
                  <Checkbox
                    id="pa-active"
                    checked={providerForm.is_active}
                    onCheckedChange={(checked) => setProviderForm({ ...providerForm, is_active: !!checked })}
                  />
                  <Label htmlFor="pa-active" className="cursor-pointer">Active</Label>
                </div>
              </div>
            </div>

            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setProviderDialog({ open: false, configId: 0, editingId: null })} disabled={providerSaving}>
                Cancel
              </Button>
              <Button type="submit" disabled={providerSaving}>
                {providerSaving ? 'Saving...' : providerDialog.editingId ? 'Update' : 'Add'}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ── Remove Assignment Confirmation ─────────────────────────────── */}
      <AlertDialog open={!!deleteAssignment} onOpenChange={(open) => { if (!open) setDeleteAssignment(null) }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove Provider</AlertDialogTitle>
            <AlertDialogDescription>
              Remove <span className="font-medium text-foreground">{deleteAssignment?.provider_name}</span> from this routing config?
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleDeleteAssignment}>Remove</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}