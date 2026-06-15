import { useEffect, useState, useRef } from 'react'
import { api, API_BASE_URL } from '../api/client'
import type { MetricsResponse, Provider, ProviderFormData, HealthOverviewResponse } from '../types'
import { Skeleton } from '@/components/ui/skeleton'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { PlusIcon } from 'lucide-react'
import { TopupDialog } from '@/components/TopupDialog'
import { ProviderCard } from '@/components/ProviderCard'
import { ProviderCatalogDialog } from '@/components/ProviderQuickAdd'
import { ProviderFormDialog } from '@/components/ProviderFormDialog'
import { OAuthConnectDialog } from '@/components/OAuthConnectDialog'
import { ShieldCheckIcon } from 'lucide-react'
import type { OAuthProviderKind } from '../types'
import { ProviderDeleteDialog } from '@/components/ProviderDeleteDialog'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { cn } from '@/lib/utils'

/* ── Stat Card ─────────────────────────────────────────────────── */

function StatCard({
  label, value, sub, subError, subGood, color = 'default'
}: {
  label: string
  value: string
  sub?: string
  subError?: boolean
  subGood?: boolean
  color?: 'green' | 'amber' | 'red' | 'default'
}) {
  const borderColor = color === 'green' ? 'border-brand/20' : color === 'amber' ? 'border-warning/20' : color === 'red' ? 'border-destructive/20' : 'border-border/50'
  const glowColor = color === 'green' ? 'var(--brand)' : color === 'amber' ? 'var(--warning)' : color === 'red' ? 'var(--destructive)' : 'transparent'
  return (
    <div className={cn('panel px-4 py-3.5', borderColor)} style={glowColor !== 'transparent' ? { boxShadow: `inset 0 0 20px ${glowColor}08` } : undefined}>
      <div className="text-[10px] uppercase tracking-[0.1em] text-muted-foreground font-mono mb-1">{label}</div>
      <div className="font-mono text-[28px] font-bold tabular-nums leading-none tracking-tight">{value}</div>
      {sub && (
        <div className={cn('mt-1 font-mono text-[11px]', subError ? 'text-destructive' : subGood ? 'text-brand' : 'text-muted-foreground')}>
          {sub}
        </div>
      )}
    </div>
  )
}

/* ── Form helpers ──────────────────────────────────────────────── */

const emptyForm: ProviderFormData = { name: '', slug: '', base_url: '', api_key: '', provider_type: 'openai' }

/* ═══════════════════════════════════════════════════════════════ */
/*  Dashboard Page                                                */
/* ═══════════════════════════════════════════════════════════════ */

export default function Dashboard() {
  /* ── Data ─────────────────────────────────────────────── */
  const [metrics, setMetrics] = useState<MetricsResponse | null>(null)
  const [providers, setProviders] = useState<Provider[]>([])
  const [health, setHealth] = useState<HealthOverviewResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [fetchError, setFetchError] = useState<string | null>(null)
  const [successMessage, setSuccessMessage] = useState<string | null>(null)

  /* ── WebSocket ────────────────────────────────────────── */
  const [wsStatus, setWsStatus] = useState<'connecting' | 'connected' | 'disconnected'>('disconnected')
  const wsRef = useRef<WebSocket | null>(null)
  const reconnectT = useRef<ReturnType<typeof setTimeout> | null>(null)

  /* ── Dialogs ──────────────────────────────────────────── */
  const [formDialogOpen, setFormDialogOpen] = useState(false)
  const [editingProvider, setEditingProvider] = useState<Provider | null>(null)
  const [form, setForm] = useState<ProviderFormData>({ ...emptyForm })
  const [saving, setSaving] = useState(false)
  const [formError, setFormError] = useState<string | null>(null)

  const [deleteTarget, setDeleteTarget] = useState<Provider | null>(null)
  const [deleting, setDeleting] = useState(false)

  const [topupProvider, setTopupProvider] = useState<Provider | null>(null)

  const [generatingKey, setGeneratingKey] = useState<Provider | null>(null)
  const [confirmGenKey, setConfirmGenKey] = useState<Provider | null>(null)  // confirmation step before actual generation
  const [generatedApiKey, setGeneratedApiKey] = useState<string | null>(null)
  const [catalogOpen, setCatalogOpen] = useState(false)
  const [oauthOpen, setOauthOpen] = useState(false)
  const [oauthReauth, setOauthReauth] = useState<{ slug: string; kind: OAuthProviderKind; name: string } | undefined>(undefined)

  /* ── Effects ──────────────────────────────────────────── */

  useEffect(() => {
    async function fetchData() {
      try {
        const [metricsData, providersData, healthData] = await Promise.all([
          api.getMetrics(), api.getProviders(), api.getHealthOverview(),
        ])
        setMetrics(metricsData)
        setProviders(providersData.providers)
        setHealth(healthData)
      } catch (e) {
        setFetchError(e instanceof Error ? e.message : 'Failed to fetch data')
      } finally {
        setLoading(false)
      }
    }
    fetchData()
  }, [])

  /* WebSocket for live in-flight counts */
  useEffect(() => {
    let c = false
    function connect() {
      if (c) return
      const tok = localStorage.getItem('token')
      if (!tok) { setWsStatus('disconnected'); return }
      let base: string
      try { const u = new URL(API_BASE_URL); u.protocol = u.protocol === 'https:' ? 'wss:' : 'ws:'; base = u.toString().replace(/\/$/, '') }
      catch { const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'; base = `${proto}//${window.location.host}` }
      setWsStatus('connecting')
      const ws = new WebSocket(`${base}/api/metrics/ws?token=${encodeURIComponent(tok)}`)
      wsRef.current = ws
      ws.onopen = () => { if (!c) setWsStatus('connected') }
      ws.onmessage = (ev) => {
        if (c) return
        try {
          const d = JSON.parse(ev.data as string)
          if (d.provider && d.event && typeof d.event === 'object' && 'ProviderLoad' in d.event) {
            const load = (d.event as Record<string, unknown>).ProviderLoad as { in_flight: number; max_concurrency: number | null }
            if (load) {
              setProviders(prev => {
                const updated = [...prev]
                const idx = updated.findIndex(p => p.name === d.provider)
                if (idx !== -1 && updated[idx].health) {
                  updated[idx] = {
                    ...updated[idx],
                    health: { ...updated[idx].health!, in_flight: load.in_flight, max_concurrency: load.max_concurrency }
                  }
                }
                return updated
              })
            }
          }
        } catch {}
      }
      ws.onclose = () => {
        if (!c) { setWsStatus('disconnected'); wsRef.current = null; reconnectT.current = setTimeout(connect, 3000) }
      }
    }
    connect()
    return () => { c = true; if (reconnectT.current) clearTimeout(reconnectT.current); wsRef.current?.close() }
  }, [])

  /* Periodic refresh */
  useEffect(() => {
    const iv = setInterval(async () => {
      try {
        const [healthData, providersData] = await Promise.all([api.getHealthOverview(), api.getProviders()])
        setHealth(healthData)
        setProviders(providersData.providers)
      } catch {}
    }, 10000)
    return () => clearInterval(iv)
  }, [])

  /* ── Provider CRUD ────────────────────────────────────── */

  const [apiKeysUrl, setApiKeysUrl] = useState<string | undefined>(undefined)

  function openCreateFromCatalog(prefill: Partial<ProviderFormData>, keysUrl?: string) {
    setEditingProvider(null)
    setForm({ ...emptyForm, ...prefill })
    setFormError(null)
    setApiKeysUrl(keysUrl)
    setFormDialogOpen(true)
  }

  function openCustomAdd() {
    setEditingProvider(null)
    setForm({ ...emptyForm })
    setFormError(null)
    setApiKeysUrl(undefined)
    setFormDialogOpen(true)
  }

  function openEdit(provider: Provider) {
    setEditingProvider(provider)
    setForm({ name: provider.name, slug: provider.slug, base_url: provider.base_url, api_key: '', provider_type: provider.provider_type })
    setFormError(null)
    setFormDialogOpen(true)
  }

  async function handleSave(e: React.FormEvent) {
    e.preventDefault(); setSaving(true); setFormError(null)
    try {
      if (editingProvider) {
        const d: Record<string, string> = {}
        if (form.name !== editingProvider.name) d.name = form.name
        if (form.slug !== editingProvider.slug) d.slug = form.slug
        if (form.base_url !== editingProvider.base_url) d.base_url = form.base_url
        if (form.api_key) d.api_key = form.api_key
        if (form.provider_type !== editingProvider.provider_type) d.provider_type = form.provider_type
        await api.updateProvider(editingProvider.slug, d)
        setSuccessMessage('PROVIDER UPDATED')
      } else {
        await api.createProvider(form)
        setSuccessMessage('PROVIDER CREATED')
      }
      setFormDialogOpen(false)
      await loadProviders()
    } catch (e) {
      setFormError(e instanceof Error ? e.message : 'Failed to save provider')
    } finally {
      setSaving(false)
    }
  }

  async function handleDelete() {
    if (!deleteTarget) return; setDeleting(true)
    try { await api.deleteProvider(deleteTarget.slug); setDeleteTarget(null); setSuccessMessage('PROVIDER DELETED'); await loadProviders() }
    catch (e) { setFetchError(e instanceof Error ? e.message : 'Failed to delete provider') }
    finally { setDeleting(false) }
  }

  async function handleGenerateKey(provider: Provider) {
    setConfirmGenKey(null)
    setGeneratingKey(provider)
    try { const r = await api.generateProviderApiKey(provider.slug); setGeneratedApiKey(r.api_key); setSuccessMessage('API KEY GENERATED'); await loadProviders() }
    catch (e) { setFetchError(e instanceof Error ? e.message : 'Failed to generate API key') }
    finally { setGeneratingKey(null) }
  }

  async function loadProviders() {
    try { const data = await api.getProviders(); setProviders(data.providers) } catch {}
  }

  /* ── Derived ─────────────────────────────────────────────── */
  const totalRequests = metrics?.total_requests ?? 0
  const totalSuccesses = metrics?.total_successes ?? 0
  const totalFailures = metrics?.total_failures ?? 0
  const activeProviders = providers.filter(p => p.health?.available).length
  const avgLatency = metrics?.providers.length
    ? (metrics.providers.reduce((sum, p) => sum + (p.avg_latency_ms || 0), 0) || 0) / metrics.providers.length
    : 0
  const successRate = totalRequests > 0 ? ((totalSuccesses / totalRequests) * 100).toFixed(1) : null
  const srNum = successRate ? parseFloat(successRate) : 0

  /* ── Loading ─────────────────────────────────────────────── */
  if (loading) {
    return (
      <div className="space-y-6 p-4 sm:p-5">
        <div className="flex flex-col gap-1"><Skeleton className="h-8 w-44 bg-secondary" /><Skeleton className="h-4 w-64 bg-secondary" /></div>
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
          {[1, 2, 3, 4].map(i => <Skeleton key={i} className="h-28 bg-secondary" />)}
        </div>
        <Skeleton className="h-80 bg-secondary" />
      </div>
    )
  }

  if (fetchError && providers.length === 0) {
    return (
      <div className="p-4 sm:p-5">
        <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono">
          <AlertDescription>{fetchError}</AlertDescription>
        </Alert>
      </div>
    )
  }

  /* ── Render ──────────────────────────────────────────────── */
  return (
    <div className="space-y-4 p-4 sm:p-5">
      {/* Header */}
      <div className="flex items-start justify-between">
        <div>
          <div className="flex items-center gap-3 mb-1">
            <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground leading-none">DASHBOARD</h1>
            <div className={cn(
              'flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-wider px-2 py-0.5 border',
              wsStatus === 'connected' ? 'text-brand border-brand/30 bg-brand/5' :
              wsStatus === 'connecting' ? 'text-warning border-warning/30 bg-warning/5' :
              'text-destructive border-destructive/30 bg-destructive/5'
            )}>
              <span className={cn('w-1.5 h-1.5 rounded-full', wsStatus === 'connected' && 'bg-brand animate-pulse-status', wsStatus === 'connecting' && 'bg-warning animate-pulse-status', wsStatus === 'disconnected' && 'bg-destructive')} />
              {wsStatus === 'connected' ? 'LIVE' : wsStatus === 'connecting' ? 'CONN' : 'OFF'}
            </div>
          </div>
          <p className="font-mono text-[13px] text-muted-foreground">System overview &amp; provider management</p>
        </div>

        {/* Quick health summary */}
        <div className="hidden sm:flex items-center gap-4 font-mono text-[12px]">
          <div className="flex items-center gap-1.5"><span className="w-2 h-2 rounded-full bg-brand" /><span className="text-brand tabular-nums">{providers.filter(p => p.health?.health_state === 'healthy').length}</span><span className="text-muted-foreground">HEALTHY</span></div>
          <div className="flex items-center gap-1.5"><span className="w-2 h-2 rounded-full bg-warning" /><span className="text-warning tabular-nums">{providers.filter(p => p.health?.health_state === 'degraded').length}</span><span className="text-muted-foreground">DEGRADED</span></div>
          <div className="flex items-center gap-1.5"><span className="w-2 h-2 rounded-full bg-destructive" /><span className="text-destructive tabular-nums">{providers.filter(p => p.health?.health_state === 'unhealthy').length}</span><span className="text-muted-foreground">DOWN</span></div>
        </div>
      </div>

      {/* Messages */}
      {successMessage && (
        <Alert className="border-brand/30 bg-brand/5 text-brand font-mono text-[13px]">
          <AlertDescription className="flex items-center justify-between">
            {successMessage}
            <Button variant="ghost" size="icon-xs" onClick={() => setSuccessMessage(null)} className="text-brand hover:text-brand">×</Button>
          </AlertDescription>
        </Alert>
      )}

      {/* Stat Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
        <StatCard label="Total Requests" value={totalRequests.toLocaleString()} sub={totalRequests > 0 ? `${totalSuccesses.toLocaleString()} ok · ${totalFailures > 0 ? totalFailures.toLocaleString() + ' fail' : '0 fail'}` : undefined} subGood={totalFailures === 0 && totalRequests > 0} subError={totalFailures > 0} />
        <StatCard label="Providers" value={`${activeProviders}/${providers.length}`} sub={health?.unhealthy_count && health.unhealthy_count > 0 ? `${health.unhealthy_count} DOWN` : health?.degraded_count && health.degraded_count > 0 ? `${health.degraded_count} DEGRADED` : 'ALL HEALTHY'} subError={!!health?.unhealthy_count && health.unhealthy_count > 0} subGood={!health?.unhealthy_count || health.unhealthy_count === 0} color={health?.unhealthy_count ? 'red' : 'green'} />
        <StatCard label="Avg Latency" value={`${avgLatency.toFixed(0).replace(/\B(?=(\d{3})+(?!\d))/g, ',')}ms`} sub={avgLatency > 2000 ? 'ELEVATED' : avgLatency > 1000 ? 'MODERATE' : 'NOMINAL'} subError={avgLatency > 2000} subGood={avgLatency <= 1000} color={avgLatency > 2000 ? 'red' : avgLatency > 1000 ? 'amber' : 'green'} />
        <StatCard label="Success Rate" value={successRate ? `${successRate}%` : '—'} sub={srNum >= 99 ? 'EXCELLENT' : srNum >= 95 ? 'GOOD' : srNum > 0 ? 'DEGRADED' : undefined} subGood={srNum >= 99} subError={srNum > 0 && srNum < 95} color={srNum >= 99 ? 'green' : srNum >= 95 ? 'amber' : srNum > 0 ? 'red' : 'default'} />
      </div>

      {/* Provider Management */}
      <div>
        <div className="flex items-center justify-between mb-3">
          <h2 className="section-header mb-0">Providers</h2>
          <div className="flex items-center gap-2">
            <Button onClick={() => { setOauthReauth(undefined); setOauthOpen(true) }} className="font-mono text-[12px] tracking-wider uppercase border border-border bg-transparent text-muted-foreground hover:text-brand hover:border-brand/40">
              <ShieldCheckIcon className="size-3.5" /> Connect Subscription
            </Button>
            <Button onClick={() => setCatalogOpen(true)} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">
              <PlusIcon className="size-3.5" /> Add Provider
            </Button>
          </div>
        </div>

        {/* Provider Cards */}
        {providers.length === 0 ? (
          <div className="panel flex items-center justify-center py-16">
            <span className="font-mono text-[13px] text-muted-foreground">{'>'} NO PROVIDERS CONFIGURED</span>
          </div>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
            {providers.map(p => (
              <ProviderCard
                key={p.slug}
                provider={p}
                onEdit={openEdit}
                onDelete={setDeleteTarget}
                onTopup={setTopupProvider}
                onGenerateKey={setConfirmGenKey}
                onReauth={pr => {
                  const kind: OAuthProviderKind = pr.provider_type === 'openai-oauth' ? 'openai' : 'anthropic'
                  setOauthReauth({ slug: pr.slug, kind, name: pr.name })
                  setOauthOpen(true)
                }}
              />
            ))}
          </div>
        )}
      </div>

      {/* Dialogs */}
      <ProviderCatalogDialog
        open={catalogOpen}
        onOpenChange={setCatalogOpen}
        onSelect={openCreateFromCatalog}
        onCustomAdd={openCustomAdd}
      />

      <OAuthConnectDialog
        open={oauthOpen}
        onOpenChange={o => { setOauthOpen(o); if (!o) setOauthReauth(undefined) }}
        reauth={oauthReauth}
        onConnected={async (msg) => { setSuccessMessage(msg); await loadProviders() }}
      />

      <ProviderFormDialog
        open={formDialogOpen}
        onOpenChange={setFormDialogOpen}
        editingProvider={editingProvider}
        form={form}
        setForm={setForm}
        onSave={handleSave}
        saving={saving}
        error={formError}
        onClearError={() => setFormError(null)}
        apiKeysUrl={apiKeysUrl}
      />

      <ProviderDeleteDialog
        open={!!deleteTarget}
        onOpenChange={o => { if (!o) setDeleteTarget(null) }}
        target={deleteTarget}
        onDelete={handleDelete}
        deleting={deleting}
      />

      {topupProvider && (
        <TopupDialog
          open={!!topupProvider}
          onOpenChange={o => { if (!o) setTopupProvider(null) }}
          providerSlug={topupProvider.slug}
          providerName={topupProvider.name}
          supportedPaymentMethods={topupProvider.payment_options}
          currentBalance={topupProvider.health?.balance}
        />
      )}

      {/* API Key Generation Confirmation Dialog */}
      <AlertDialog open={!!confirmGenKey} onOpenChange={o => { if (!o) setConfirmGenKey(null) }}>
        <AlertDialogContent className="border-border bg-card">
          <AlertDialogHeader>
            <AlertDialogTitle className="font-display text-xl tracking-[0.04em] text-warning">GENERATE NEW API KEY</AlertDialogTitle>
            <AlertDialogDescription className="font-mono text-[13px] text-muted-foreground space-y-3">
              <span>You are about to generate a new API key for <span className="text-foreground font-bold">{confirmGenKey?.name}</span>.</span>
              <span className="block p-3 bg-warning/5 border border-warning/20 text-warning font-mono text-[12px] leading-relaxed">
                ⚠ WARNING: Existing balance and credit may be lost. The old API key will be replaced and cannot be recovered. Verify your balance before proceeding.
              </span>
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel className="font-mono text-[12px] border-border text-muted-foreground" disabled={!!generatingKey}>CANCEL</AlertDialogCancel>
            <AlertDialogAction
              onClick={async () => {
                if (confirmGenKey) await handleGenerateKey(confirmGenKey)
              }}
              disabled={!!generatingKey}
              className="font-mono text-[12px] tracking-wider uppercase bg-destructive hover:bg-destructive/90 text-destructive-foreground border-0"
            >
              {generatingKey ? 'GENERATING...' : 'GENERATE'}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Generated Key Dialog */}
      <Dialog open={!!generatedApiKey} onOpenChange={o => { if (!o) { setGeneratedApiKey(null); setSuccessMessage(null) } }}>
        <DialogContent className="sm:max-w-lg border-border bg-card">
          <DialogHeader>
            <DialogTitle className="font-display text-xl tracking-[0.04em] text-brand">API KEY GENERATED</DialogTitle>
            <DialogDescription className="font-mono text-[12px] text-muted-foreground">Save this key securely. You won&apos;t see it again.</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="p-3 bg-surface border border-border font-mono text-[13px] text-brand break-all">{generatedApiKey}</div>
            <Alert className="border-brand/20 bg-brand/5 text-muted-foreground font-mono text-[12px]">
              <AlertDescription>This key is automatically saved to your provider configuration.</AlertDescription>
            </Alert>
          </div>
          <DialogFooter>
            <Button onClick={() => { setGeneratedApiKey(null); setSuccessMessage(null) }} className="font-mono text-[12px] border border-border bg-transparent text-foreground">CLOSE</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
