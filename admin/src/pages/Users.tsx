import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { api } from '../api/client'
import type { User, CreateUserRequest } from '../types'
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
import { PlusIcon, EyeIcon, TrashIcon } from 'lucide-react'
import { cn } from '@/lib/utils'

export default function Users() {
  const navigate = useNavigate()
  const [users, setUsers] = useState<User[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [successMessage, setSuccessMessage] = useState<string | null>(null)

  const [dialogOpen, setDialogOpen] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<User | null>(null)
  const [saving, setSaving] = useState(false)

  const [formData, setFormData] = useState<CreateUserRequest>({
    username: '', password: '', external_id: '', user_type: 'internal', is_admin: false,
  })

  useEffect(() => { loadUsers() }, [])

  async function loadUsers() {
    try { setLoading(true); const d = await api.getUsers(); setUsers(d); setError(null) }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to load users') }
    finally { setLoading(false) }
  }

  function openCreate() { setFormData({ username: '', password: '', external_id: '', user_type: 'internal', is_admin: false }); setDialogOpen(true) }

  async function handleCreate(e: React.FormEvent) {
    e.preventDefault(); setSaving(true)
    try { await api.createUser(formData); setDialogOpen(false); setSuccessMessage('USER CREATED'); loadUsers() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to create user') }
    finally { setSaving(false) }
  }

  async function handleDelete() {
    if (!deleteTarget) return
    try { await api.deleteUser(deleteTarget.id); setDeleteTarget(null); setSuccessMessage('USER DELETED'); loadUsers() }
    catch (e) { setError(e instanceof Error ? e.message : 'Failed to delete user') }
  }

  if (loading) {
    return (
      <div className="space-y-6 p-6">
        <div className="flex items-center justify-between">
          <div><Skeleton className="h-8 w-44 bg-secondary" /><Skeleton className="h-4 w-56 bg-secondary mt-1" /></div>
          <Skeleton className="h-9 w-28 bg-secondary" />
        </div>
        <Skeleton className="h-64 bg-secondary" />
      </div>
    )
  }

  return (
    <div className="space-y-6 p-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="font-display text-[28px] tracking-[0.04em] text-foreground leading-none mb-1">USERS</h1>
          <p className="font-mono text-[13px] text-muted-foreground">Manage users and permissions</p>
        </div>
        <Button onClick={openCreate} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">
          <PlusIcon className="size-3.5" /> Add User
        </Button>
      </div>

      {/* Messages */}
      {successMessage && (
        <Alert className="border-brand/30 bg-brand/5 text-brand font-mono text-[13px]">
          <AlertDescription className="flex items-center justify-between">{successMessage}<Button variant="ghost" size="icon-xs" onClick={() => setSuccessMessage(null)} className="text-brand">×</Button></AlertDescription>
        </Alert>
      )}
      {error && (
        <Alert className="border-destructive/30 bg-destructive/5 text-destructive font-mono text-[13px]">
          <AlertDescription className="flex items-center justify-between">{error}<Button variant="ghost" size="icon-xs" onClick={() => setError(null)} className="text-destructive">×</Button></AlertDescription>
        </Alert>
      )}

      <div className="panel">
        <div className="overflow-x-auto">
          <table className="w-full table-scan">
            <thead>
              <tr className="border-b border-border/50 text-left">
                <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Username</th>
                <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">External ID</th>
                <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Type</th>
                <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Admin</th>
                <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium">Created</th>
                <th className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground px-4 py-2.5 font-medium text-right">Actions</th>
              </tr>
            </thead>
            <tbody>
              {users.length === 0 ? (
                <tr><td colSpan={6} className="px-4 py-16 text-center font-mono text-[13px] text-muted-foreground">{'>'} NO USERS FOUND</td></tr>
              ) : (
                users.map(user => (
                  <tr key={user.id} className="border-b border-border/50 hover:bg-surface transition-colors">
                    <td className="px-4 py-3 font-mono text-[13px] font-medium">
                      {user.username || <span className="text-muted-foreground">N/A</span>}
                    </td>
                    <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground truncate max-w-48" title={user.external_id || ''}>
                      {user.external_id || <span className="text-muted-foreground">N/A</span>}
                    </td>
                    <td className="px-4 py-3">
                      <Badge className={cn(
                        'font-mono text-[10px] uppercase tracking-wider',
                        user.user_type === 'internal' ? 'bg-brand/15 text-brand border-brand/30' :
                        user.user_type === 'nostr' ? 'bg-warning/15 text-warning border-warning/30' :
                        'bg-secondary text-muted-foreground border-border'
                      )}>
                        {user.user_type}
                      </Badge>
                    </td>
                    <td className="px-4 py-3">
                      {user.is_admin
                        ? <Badge className="font-mono text-[10px] uppercase tracking-wider bg-warning/15 text-warning border-warning/30">ADMIN</Badge>
                        : <span className="font-mono text-[12px] text-muted-foreground">REGULAR</span>}
                    </td>
                    <td className="px-4 py-3 font-mono text-[12px] text-muted-foreground">{new Date(user.created_at).toLocaleDateString()}</td>
                    <td className="px-4 py-3">
                      <div className="flex items-center justify-end gap-1">
                        <Button variant="ghost" size="icon-xs" onClick={() => navigate(`/users/${user.id}`)} className="text-muted-foreground hover:text-brand"><EyeIcon className="size-3.5" /></Button>
                        <Button variant="ghost" size="icon-xs" onClick={() => setDeleteTarget(user)} className="text-muted-foreground hover:text-destructive"><TrashIcon className="size-3.5" /></Button>
                      </div>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </div>

      {/* Create Dialog */}
      <Dialog open={dialogOpen} onOpenChange={o => { if (!o) setDialogOpen(false) }}>
        <DialogContent className="sm:max-w-lg border-border bg-card">
          <DialogHeader>
            <DialogTitle className="font-display text-xl tracking-[0.04em]">CREATE USER</DialogTitle>
            <DialogDescription className="font-mono text-[12px] text-muted-foreground">Add a new user to the system.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleCreate} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="u-type" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Type</Label>
              <Select value={formData.user_type} onValueChange={v => setFormData({ ...formData, user_type: v as 'internal' | 'nostr' | 'oauth' })}>
                <SelectTrigger id="u-type" className="font-mono bg-surface border-border text-foreground"><SelectValue /></SelectTrigger>
                <SelectContent className="bg-card border-border">
                  <SelectGroup>
                    {['internal','nostr','oauth'].map(t => <SelectItem key={t} value={t} className="font-mono text-foreground">{t}</SelectItem>)}
                  </SelectGroup>
                </SelectContent>
              </Select>
            </div>
            <div className={cn(formData.user_type !== 'internal' && 'opacity-40 pointer-events-none')}>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="u-name" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Username</Label>
                <Input id="u-name" value={formData.username || ''} onChange={e => setFormData({ ...formData, username: e.target.value })} required disabled={formData.user_type !== 'internal'} className="font-mono bg-surface border-border text-foreground" />
              </div>
              <div className="flex flex-col gap-1.5 mt-4">
                <Label htmlFor="u-pass" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">Password</Label>
                <Input id="u-pass" type="password" value={formData.password || ''} onChange={e => setFormData({ ...formData, password: e.target.value })} required disabled={formData.user_type !== 'internal'} className="font-mono bg-surface border-border text-foreground" />
              </div>
            </div>
            <div className={cn(formData.user_type === 'internal' && 'opacity-40 pointer-events-none')}>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="u-ext" className="font-mono text-[10px] uppercase tracking-[0.1em] text-muted-foreground">External ID</Label>
                <Input id="u-ext" value={formData.external_id || ''} onChange={e => setFormData({ ...formData, external_id: e.target.value })} required={formData.user_type !== 'internal'} disabled={formData.user_type === 'internal'} className="font-mono bg-surface border-border text-foreground" />
              </div>
            </div>
            <div className="flex items-center gap-2 pt-2">
              <Checkbox id="u-admin" checked={formData.is_admin} onCheckedChange={c => setFormData({ ...formData, is_admin: !!c })} />
              <Label htmlFor="u-admin" className="font-mono text-[12px] cursor-pointer">ADMIN USER</Label>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)} disabled={saving} className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</Button>
              <Button type="submit" disabled={saving} className="font-mono text-[12px] tracking-wider uppercase border border-brand/40 bg-brand/10 text-brand hover:bg-brand/20">{saving ? 'CREATING...' : 'CREATE'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation */}
      <AlertDialog open={!!deleteTarget} onOpenChange={o => { if (!o) setDeleteTarget(null) }}>
        <AlertDialogContent className="border-border bg-card">
          <AlertDialogHeader>
            <AlertDialogTitle className="font-display text-xl tracking-[0.04em] text-destructive">DELETE USER</AlertDialogTitle>
            <AlertDialogDescription className="font-mono text-[13px] text-muted-foreground">Delete <span className="text-foreground">{deleteTarget?.username}</span>? This cannot be undone.</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel className="font-mono text-[12px] border-border text-muted-foreground">CANCEL</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleDelete} className="font-mono text-[12px] tracking-wider uppercase">DELETE</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
