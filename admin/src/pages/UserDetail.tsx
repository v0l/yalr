import { useEffect, useState } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import { api } from '../api/client'
import type { UserDetailResponse, UserBalanceDetail, UserModelPermission } from '../types'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Checkbox } from '@/components/ui/checkbox'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { ArrowLeftIcon, PencilIcon, PlusIcon, CopyIcon, BanIcon, TrashIcon, Save, X, HelpCircle, RefreshCw } from 'lucide-react'

/* ── Helpers ──────────────────────────────────────────────────── */
function txBadge(type: string) {
  switch (type) {
    case 'deposit': return <Badge className="font-mono text-[9px] uppercase bg-brand/15 text-brand border-brand/30">DEPOSIT</Badge>
    case 'refund': return <Badge className="font-mono text-[9px] uppercase bg-warning/15 text-warning border-warning/30">REFUND</Badge>
    case 'charge': return <Badge className="font-mono text-[9px] uppercase bg-destructive/15 text-destructive border-destructive/30">CHARGE</Badge>
    case 'reserve': return <Badge className="font-mono text-[9px] uppercase bg-secondary text-muted-foreground border-border">RESERVE</Badge>
    case 'admin_credit': return <Badge className="font-mono text-[9px] uppercase bg-brand/15 text-brand border-brand/30">CREDIT</Badge>
    case 'admin_debit': return <Badge className="font-mono text-[9px] uppercase bg-destructive/15 text-destructive border-destructive/30">DEBIT</Badge>
    default: return <Badge className="font-mono text-[9px] uppercase bg-secondary text-muted-foreground border-border">{type.toUpperCase()}</Badge>
  }
}
function fmtSats(msat: number) { return (msat / 1000).toLocaleString(undefined, { minimumFractionDigits: 3, maximumFractionDigits: 3 }) }
function fmtDateShort(d: string) { return new Date(d).toLocaleDateString() }

/* ═══════════════════════════════════════════════════════════════ */
/*  User Detail Page                                              */
/* ═══════════════════════════════════════════════════════════════ */

export default function UserDetail() {
  const { id } = useParams<{ id: string }>()
  const navigate = useNavigate()
  const [data, setData] = useState<UserDetailResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [successMessage, setSuccessMessage] = useState<string | null>(null)

  const [editDialog, setEditDialog] = useState(false)
  const [createKeyDialog, setCreateKeyDialog] = useState(false)
  const [deleteKeyTarget, setDeleteKeyTarget] = useState<{ id: number; name: string } | null>(null)

  const [editForm, setEditForm] = useState({ username: '', password: '', is_admin: false })
  const [keyForm, setKeyForm] = useState({ name: '', expiresInDays: '' })
  const [createdKey, setCreatedKey] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const [balanceData, setBalanceData] = useState<UserBalanceDetail | null>(null)
  const [balanceLoading, setBalanceLoading] = useState(false)

  const [permissions, setPermissions] = useState<UserModelPermission[]>([])
  const [permsLoading, setPermsLoading] = useState(false)
  const [showInfo, setShowInfo] = useState(false)
  const [newModel, setNewModel] = useState('')
  const [newAllow, setNewAllow] = useState(true)
  const [editingPermId, setEditingPermId] = useState<number | null>(null)
  const [editModel, setEditModel] = useState('')
  const [editAllow, setEditAllow] = useState(true)

  useEffect(() => { if (id) loadUser() }, [id])

  async function loadUser() {
    try { setLoading(true); const r = await api.getUser(parseInt(id!)); setData(r); setError(null); if (r.user) setEditForm({ username: r.user.username || '', password: '', is_admin: r.user.is_admin }); loadPermissions(); loadBalance() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to load user') }
    finally { setLoading(false) }
  }

  async function loadPermissions() { if (!id) return; setPermsLoading(true); try { setPermissions(await api.listUserModelPermissions(parseInt(id))) } catch {} finally { setPermsLoading(false) } }
  async function loadBalance() { if (!id) return; setBalanceLoading(true); try { setBalanceData(await api.getUserBalanceDetail(parseInt(id))) } catch {} finally { setBalanceLoading(false) } }

  async function handleEditSave(e: React.FormEvent) {
    e.preventDefault(); if (!id) return; setSaving(true)
    try { await api.updateUser(parseInt(id), editForm); setEditDialog(false); setSuccessMessage('USER UPDATED'); loadUser() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to update user') }
    finally { setSaving(false) }
  }

  async function handleCreateKey(e: React.FormEvent) {
    e.preventDefault(); if (!id) return; setSaving(true)
    try { const r = await api.createApiKey(keyForm.name, keyForm.expiresInDays ? parseInt(keyForm.expiresInDays) : undefined, parseInt(id)); setCreatedKey(r.key || null); setCreateKeyDialog(false); setKeyForm({ name: '', expiresInDays: '' }); setSuccessMessage('API KEY CREATED'); loadUser() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to create API key') }
    finally { setSaving(false) }
  }

  async function handleKeyAction(keyId: number, action: 'disable' | 'enable' | 'delete') {
    try { if (action === 'disable') await api.disableApiKey(keyId); else if (action === 'enable') await api.enableApiKey(keyId); else await api.deleteApiKey(keyId); if (action === 'delete') setDeleteKeyTarget(null); setSuccessMessage(`API KEY ${action.toUpperCase()}D`); loadUser() }
    catch (e) { setError(e instanceof Error ? e.message : `Failed to ${action} API key`) }
  }

  async function handleAddPermission() { if (!newModel.trim() || !id) return; setSaving(true); try { await api.createUserModelPermission({ user_id: parseInt(id), model: newModel.trim(), allow: newAllow }); setNewModel(''); setNewAllow(true); await loadPermissions() } catch (e) { setError(e instanceof Error ? e.message : 'Failed to add permission') } finally { setSaving(false) } }
  async function handleDeletePermission(perm: UserModelPermission) { if (!confirm(`Delete permission for ${perm.model}?`)) return; setSaving(true); try { await api.deleteUserModelPermission(perm.user_id, perm.model); await loadPermissions() } catch (e) { setError(e instanceof Error ? e.message : 'Failed to delete permission') } finally { setSaving(false) } }
  function startEditPerm(perm: UserModelPermission) { setEditingPermId(perm.id); setEditModel(perm.model); setEditAllow(perm.allow) }
  function cancelEditPerm() { setEditingPermId(null); setEditModel(''); setEditAllow(true) }
  async function saveEditPerm(perm: UserModelPermission) { if (!editModel.trim()) return; setSaving(true); try { await api.createUserModelPermission({ user_id: perm.user_id, model: editModel.trim(), allow: editAllow }); setEditingPermId(null); await loadPermissions() } catch (e) { setError(e instanceof Error ? e.message : 'Failed to update permission') } finally { setSaving(false) } }

  if (loading) {
    return (
      <div className="space-y-4 p-6">
        <Skeleton className="h-6 w-32 bg-secondary" />
        <Skeleton className="h-14 w-full bg-secondary" />
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
          <Skeleton className="h-48 w-full bg-secondary" />
          <Skeleton className="h-48 w-full bg-secondary" />
        </div>
      </div>
    )
  }

  if (error || !data) {
    return (
      <div className="p-6 space-y-4">
        <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono"><AlertDescription>{error || 'User not found'}</AlertDescription></Alert>
        <Button variant="outline" onClick={() => navigate('/users')} className="font-mono text-[12px] border-border text-muted-foreground"><ArrowLeftIcon className="size-3.5" /> Back</Button>
      </div>
    )
  }

  const { user, api_keys } = data

  return (
    <div className="space-y-4 p-6">
      {/* Banner */}
      <div className="flex items-center justify-between flex-wrap gap-2">
        <div className="flex items-center gap-3 min-w-0">
          <Button variant="ghost" size="sm" className="shrink-0 text-muted-foreground hover:text-foreground" onClick={() => navigate('/users')}><ArrowLeftIcon className="size-4" /></Button>
          <div className="min-w-0">
            <div className="flex items-center gap-2 flex-wrap">
              <h1 className="font-display text-[20px] tracking-[0.04em] truncate">{user.username || 'USER DETAILS'}</h1>
              <Badge className="font-mono text-[10px] uppercase tracking-wider bg-secondary text-muted-foreground border-border">{user.user_type}</Badge>
              {user.is_admin && <Badge className="font-mono text-[10px] uppercase tracking-wider bg-warning/15 text-warning border-warning/30">ADMIN</Badge>}
            </div>
            <p className="font-mono text-[12px] text-muted-foreground mt-0.5">
              #{user.id} · {fmtDateShort(user.created_at)}
              {user.external_id && <span className="ml-2 font-mono text-muted-foreground/60 truncate">{user.external_id}</span>}
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <Button size="sm" variant="outline" onClick={() => loadBalance()} disabled={balanceLoading} className="font-mono text-[11px] uppercase tracking-wider border-border text-muted-foreground h-8">
            <RefreshCw className={`size-3 mr-1.5 ${balanceLoading ? 'animate-spin' : ''}`} /> REFRESH
          </Button>
          <Button size="sm" variant="outline" onClick={() => { setEditForm({ username: user.username || '', password: '', is_admin: user.is_admin }); setEditDialog(true) }} className="font-mono text-[11px] uppercase tracking-wider border-border text-muted-foreground hover:text-foreground h-8">
            <PencilIcon className="size-3 mr-1.5" /> EDIT
          </Button>
        </div>
      </div>

      {/* Messages */}
      {successMessage && <Alert className="border-brand/30 bg-brand/5 text-brand font-mono text-[13px]"><AlertDescription className="flex items-center justify-between">{successMessage}<Button variant="ghost" size="icon-xs" onClick={() => setSuccessMessage(null)} className="text-brand">×</Button></AlertDescription></Alert>}
      {error && <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono text-[13px]"><AlertDescription className="flex items-center justify-between">{error}<Button variant="ghost" size="icon-xs" onClick={() => setError(null)} className="text-destructive">×</Button></AlertDescription></Alert>}

      {/* Balance + Permissions */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        {/* Balance */}
        <div className="panel p-4">
          <h2 className="font-mono text-[10px] uppercase tracking-[0.12em] text-muted-foreground mb-3">Balance</h2>
          {balanceData ? (
            <>
              <div className="grid grid-cols-2 gap-3 mb-3">
                <div>
                  <p className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground mb-0.5">Balance</p>
                  <p className="font-mono text-[20px] font-bold tabular-nums text-brand">{fmtSats(balanceData.balance_msat)} <span className="text-[13px] font-normal text-muted-foreground">sats</span></p>
                </div>
                <div>
                  <p className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground mb-0.5">Lifetime Deposits</p>
                  <p className="font-mono text-[20px] tabular-nums text-muted-foreground">{fmtSats(balanceData.lifetime_deposited_msat)}</p>
                </div>
              </div>
              {balanceData.transactions.length > 0 ? (
                <div>
                  <p className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground mb-1.5">Recent Activity</p>
                  {balanceData.transactions.slice(0, 6).map(tx => (
                    <div key={tx.id} className="flex items-center justify-between py-1 border-b border-border/50 last:border-0 font-mono text-[12px]">
                      <div className="flex items-center gap-2 min-w-0">{txBadge(tx.transaction_type)}<span className="text-muted-foreground truncate max-w-[140px]" title={tx.reference_id || ''}>{tx.reference_id || '—'}</span></div>
                      <div className="flex items-center gap-2 shrink-0">
                        <span className="text-[10px] text-muted-foreground/60 hidden sm:inline">{fmtDateShort(tx.created_at)}</span>
                        <span className={`tabular-nums ${tx.amount_msat >= 0 ? 'text-brand' : 'text-destructive'}`}>{tx.amount_msat >= 0 ? '+' : ''}{fmtSats(tx.amount_msat)}</span>
                      </div>
                    </div>
                  ))}
                </div>
              ) : <p className="font-mono text-[12px] text-muted-foreground text-center py-3">No transactions yet</p>}
            </>
          ) : <p className="font-mono text-[12px] text-muted-foreground text-center py-4">Balance data unavailable</p>}
        </div>

        {/* Model Permissions */}
        <div className="panel p-4">
          <div className="flex items-center justify-between mb-3">
            <div className="flex items-center gap-1.5">
              <h2 className="font-mono text-[10px] uppercase tracking-[0.12em] text-muted-foreground">Model Permissions</h2>
              <button onClick={() => setShowInfo(!showInfo)} className="text-muted-foreground hover:text-foreground transition-colors"><HelpCircle className="size-3.5" /></button>
            </div>
          </div>
          {showInfo && (
            <div className="border border-border/50 bg-surface p-3 mb-3 font-mono text-[11px] text-muted-foreground space-y-1">
              <p><span className="text-foreground">No rules</span> — user sees all models</p>
              <p><span className="text-foreground">Any rules exist</span> — only explicit Allows are visible</p>
              <p><span className="text-foreground">Wildcard</span> <code className="bg-secondary px-1 text-brand">*</code> — allow/deny everything</p>
              <p><span className="text-foreground">Deny wins</span> — specific deny overrides wildcard allow</p>
            </div>
          )}
          <div className="flex items-end gap-2 mb-2">
            <Input placeholder="model or *" value={newModel} onChange={e => setNewModel(e.target.value)} onKeyDown={e => { if (e.key === 'Enter') handleAddPermission() }}
              className="flex-1 h-8 font-mono text-[12px] bg-surface border-border text-foreground" />
            <Select value={newAllow ? 'allow' : 'deny'} onValueChange={v => setNewAllow(v === 'allow')}>
              <SelectTrigger className="h-8 w-[88px] font-mono text-[11px] bg-surface border-border text-foreground"><SelectValue /></SelectTrigger>
              <SelectContent className="bg-card border-border">
                <SelectItem value="allow" className="font-mono text-brand">ALLOW</SelectItem>
                <SelectItem value="deny" className="font-mono text-destructive">DENY</SelectItem>
              </SelectContent>
            </Select>
            <Button size="sm" className="h-8 font-mono text-[11px] uppercase tracking-wider border border-brand/40 bg-brand/10 text-brand" onClick={handleAddPermission} disabled={!newModel.trim() || saving}><PlusIcon className="size-3" /></Button>
          </div>
          {permsLoading ? <p className="font-mono text-[12px] text-muted-foreground text-center py-3">Loading...</p> :
            permissions.length === 0 ? <p className="font-mono text-[12px] text-muted-foreground text-center py-3">No rules — unrestricted</p> : (
              <div className="space-y-0.5 max-h-[300px] overflow-y-auto">
                {permissions.map(perm => (
                  <div key={perm.id} className="flex items-center justify-between py-1.5 px-2 border border-border/50 hover:bg-surface transition-colors font-mono text-[12px]">
                    {editingPermId === perm.id ? (
                      <div className="flex items-center gap-2 flex-1 min-w-0">
                        <Input value={editModel} onChange={e => setEditModel(e.target.value)} className="h-7 text-[12px] flex-1 min-w-0 font-mono bg-surface border-border text-foreground" />
                        <Checkbox checked={editAllow} onCheckedChange={c => setEditAllow(!!c)} />
                        <span className="text-[11px] w-10">{editAllow ? 'ALLOW' : 'DENY'}</span>
                        <Button size="sm" variant="ghost" className="h-7 w-7 p-0 text-brand hover:text-brand" onClick={() => saveEditPerm(perm)} disabled={saving}><Save className="size-3" /></Button>
                        <Button size="sm" variant="ghost" className="h-7 w-7 p-0 text-muted-foreground hover:text-destructive" onClick={cancelEditPerm} disabled={saving}><X className="size-3" /></Button>
                      </div>
                    ) : (
                      <>
                        <div className="flex items-center gap-2 min-w-0 flex-1">
                          {perm.model === '*' ? <Badge className="font-mono text-[9px] uppercase bg-[#a855f7]/15 text-[#a855f7] border-[#a855f7]/30">* ALL</Badge> : <code className="text-foreground truncate">{perm.model}</code>}
                        </div>
                        <div className="flex items-center gap-1.5 shrink-0">
                          <Badge className={perm.allow ? 'font-mono text-[9px] uppercase bg-brand/15 text-brand border-brand/30' : 'font-mono text-[9px] uppercase bg-destructive/15 text-destructive border-destructive/30'}>{perm.allow ? 'ALLOW' : 'DENY'}</Badge>
                          <Button size="sm" variant="ghost" className="h-7 w-7 p-0 text-muted-foreground hover:text-foreground" onClick={() => startEditPerm(perm)}><PencilIcon className="size-3" /></Button>
                          <Button size="sm" variant="ghost" className="h-7 w-7 p-0 text-muted-foreground hover:text-destructive" onClick={() => handleDeletePermission(perm)} disabled={saving}><TrashIcon className="size-3" /></Button>
                        </div>
                      </>
                    )}
                  </div>
                ))}
              </div>
            )}
        </div>
      </div>

      {/* API Keys */}
      <div className="panel p-4">
        <div className="flex items-center justify-between mb-3">
          <h2 className="font-mono text-[10px] uppercase tracking-[0.12em] text-muted-foreground">API Keys</h2>
          <Button size="sm" className="h-8 font-mono text-[11px] uppercase tracking-wider border border-brand/40 bg-brand/10 text-brand" onClick={() => { setCreateKeyDialog(true); setCreatedKey(null); setKeyForm({ name: '', expiresInDays: '' }) }}><PlusIcon className="size-3 mr-1.5" /> Create Key</Button>
        </div>
        {api_keys.length === 0 ? <p className="font-mono text-[12px] text-muted-foreground text-center py-6">No API keys for this user</p> : (
          <div className="overflow-x-auto">
            <table className="w-full table-scan">
              <thead>
                <tr className="border-b border-border/50 text-left">
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-3 py-2 font-medium">Name</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-3 py-2 font-medium hidden sm:table-cell">Key</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-3 py-2 font-medium hidden md:table-cell">Created</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-3 py-2 font-medium hidden md:table-cell">Expires</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-3 py-2 font-medium">Status</th>
                  <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-3 py-2 font-medium text-right">Actions</th>
                </tr>
              </thead>
              <tbody>
                {api_keys.map(k => (
                  <tr key={k.id} className="border-b border-border/50 hover:bg-surface">
                    <td className="px-3 py-2 font-mono text-[12px] font-medium">{k.name}</td>
                    <td className="px-3 py-2 font-mono text-[12px] text-muted-foreground hidden sm:table-cell">...{k.last_four}</td>
                    <td className="px-3 py-2 font-mono text-[12px] text-muted-foreground hidden md:table-cell">{fmtDateShort(k.created_at)}</td>
                    <td className="px-3 py-2 font-mono text-[12px] text-muted-foreground hidden md:table-cell">{k.expires_at ? fmtDateShort(k.expires_at) : '—'}</td>
                    <td className="px-3 py-2">
                      <Badge className={k.is_active ? 'font-mono text-[9px] uppercase bg-brand/15 text-brand border-brand/30' : 'font-mono text-[9px] uppercase bg-secondary text-muted-foreground border-border'}>
                        {k.is_active ? 'ACTIVE' : 'INACTIVE'}
                      </Badge>
                    </td>
                    <td className="px-3 py-2">
                      <div className="flex items-center justify-end gap-0.5">
                        {k.is_active ? (
                          <Button variant="ghost" size="icon-xs" onClick={() => handleKeyAction(k.id, 'disable')} className="text-muted-foreground hover:text-warning"><BanIcon className="size-3" /></Button>
                        ) : (
                          <Button variant="ghost" size="icon-xs" onClick={() => handleKeyAction(k.id, 'enable')} className="text-muted-foreground hover:text-brand"><BanIcon className="size-3" /></Button>
                        )}
                        <Button variant="ghost" size="icon-xs" onClick={() => setDeleteKeyTarget({ id: k.id, name: k.name })} className="text-muted-foreground hover:text-destructive"><TrashIcon className="size-3" /></Button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* ── Dialogs ────────────────────────────────────────────── */}
      <Dialog open={editDialog} onOpenChange={o => { if (!o) setEditDialog(false) }}>
        <DialogContent className="border-border bg-card">
          <DialogHeader>
            <DialogTitle className="font-display text-xl tracking-[0.04em]">EDIT USER</DialogTitle>
            <DialogDescription className="font-mono text-[12px] text-muted-foreground">Update user settings.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleEditSave} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5"><Label htmlFor="ed-name" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Username</Label><Input id="ed-name" value={editForm.username} onChange={e => setEditForm({ ...editForm, username: e.target.value })} className="font-mono bg-surface border-border text-foreground" /></div>
            <div className="flex flex-col gap-1.5"><Label htmlFor="ed-pass" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">New Password</Label><Input id="ed-pass" type="password" value={editForm.password} onChange={e => setEditForm({ ...editForm, password: e.target.value })} placeholder="Leave empty to keep current" className="font-mono bg-surface border-border text-foreground" /></div>
            <div className="flex items-center gap-2"><Checkbox id="ed-admin" checked={editForm.is_admin} onCheckedChange={c => setEditForm({ ...editForm, is_admin: !!c })} /><Label htmlFor="ed-admin" className="font-mono text-[12px] cursor-pointer">ADMIN USER</Label></div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setEditDialog(false)} disabled={saving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
              <Button type="submit" disabled={saving} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand">{saving ? 'SAVING...' : 'UPDATE'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={createKeyDialog} onOpenChange={o => { if (!o) setCreateKeyDialog(false) }}>
        <DialogContent className="border-border bg-card">
          <DialogHeader>
            <DialogTitle className="font-display text-xl tracking-[0.04em]">CREATE API KEY</DialogTitle>
            <DialogDescription className="font-mono text-[12px] text-muted-foreground">Create a new API key for this user.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleCreateKey} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5"><Label htmlFor="k-name" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Name</Label><Input id="k-name" value={keyForm.name} onChange={e => setKeyForm({ ...keyForm, name: e.target.value })} required className="font-mono bg-surface border-border text-foreground" /></div>
            <div className="flex flex-col gap-1.5"><Label htmlFor="k-exp" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Expires In (days)</Label><Input id="k-exp" type="number" min={1} value={keyForm.expiresInDays} onChange={e => setKeyForm({ ...keyForm, expiresInDays: e.target.value })} placeholder="Optional" className="font-mono bg-surface border-border text-foreground" /></div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setCreateKeyDialog(false)} disabled={saving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
              <Button type="submit" disabled={saving} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand">{saving ? 'CREATING...' : 'CREATE'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <AlertDialog open={!!deleteKeyTarget} onOpenChange={o => { if (!o) setDeleteKeyTarget(null) }}>
        <AlertDialogContent className="border-border bg-card">
          <AlertDialogHeader>
            <AlertDialogTitle className="font-display text-xl tracking-[0.04em] text-destructive">DELETE API KEY</AlertDialogTitle>
            <AlertDialogDescription className="font-mono text-[13px] text-muted-foreground">Permanently delete <span className="text-foreground">{deleteKeyTarget?.name}</span>?</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => handleKeyAction(deleteKeyTarget!.id, 'delete')} className="font-mono text-[12px] tracking-wider uppercase">DELETE</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {createdKey && (
        <Dialog open={!!createdKey} onOpenChange={() => setCreatedKey(null)}>
          <DialogContent className="border-border bg-card">
            <DialogHeader>
              <DialogTitle className="font-display text-xl tracking-[0.04em] text-brand">API KEY CREATED</DialogTitle>
              <DialogDescription className="font-mono text-[12px] text-muted-foreground">Copy this key now — it won&apos;t be shown again.</DialogDescription>
            </DialogHeader>
            <div className="flex flex-col gap-4">
              <div className="flex items-center gap-2">
                <code className="flex-1 border border-border bg-surface p-2 font-mono text-[13px] text-brand break-all">{createdKey}</code>
                <Button variant="outline" size="icon-sm" onClick={() => { navigator.clipboard.writeText(createdKey); setSuccessMessage('COPIED') }} className="text-muted-foreground border-border"><CopyIcon className="size-3.5" /></Button>
              </div>
              <Button onClick={() => setCreatedKey(null)} className="font-mono text-[12px] border border-border bg-transparent text-foreground">DONE</Button>
            </div>
          </DialogContent>
        </Dialog>
      )}
    </div>
  )
}
