import { useEffect, useState } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import { api } from '../api/client'
import type { UserDetailResponse } from '../types'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Separator } from '@/components/ui/separator'
import { ArrowLeftIcon, PencilIcon, PlusIcon, CopyIcon, BanIcon, TrashIcon } from 'lucide-react'

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

  useEffect(() => { if (id) loadUser() }, [id])

  async function loadUser() {
    try {
      setLoading(true)
      const result = await api.getUser(parseInt(id!))
      setData(result)
      setError(null)
      if (result.user) {
        setEditForm({ username: result.user.username || '', password: '', is_admin: result.user.is_admin })
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load user')
    } finally {
      setLoading(false)
    }
  }

  async function handleEditSave(e: React.FormEvent) {
    e.preventDefault()
    if (!id) return
    setSaving(true)
    try {
      await api.updateUser(parseInt(id), editForm)
      setEditDialog(false)
      setSuccessMessage('User updated')
      loadUser()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to update user')
    } finally { setSaving(false) }
  }

  async function handleCreateKey(e: React.FormEvent) {
    e.preventDefault()
    if (!id) return
    setSaving(true)
    try {
      const result = await api.createApiKey(keyForm.name, keyForm.expiresInDays ? parseInt(keyForm.expiresInDays) : undefined, parseInt(id))
      setCreatedKey(result.key || null)
      setCreateKeyDialog(false)
      setKeyForm({ name: '', expiresInDays: '' })
      setSuccessMessage('API key created')
      loadUser()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to create API key')
    } finally { setSaving(false) }
  }

  async function handleKeyAction(keyId: number, action: 'disable' | 'enable' | 'delete') {
    try {
      if (action === 'disable') await api.disableApiKey(keyId)
      else if (action === 'enable') await api.enableApiKey(keyId)
      else await api.deleteApiKey(keyId)
      if (action === 'delete') setDeleteKeyTarget(null)
      setSuccessMessage(`API key ${action}d`)
      loadUser()
    } catch (e) {
      setError(e instanceof Error ? e.message : `Failed to ${action} API key`)
    }
  }

  if (loading) {
    return (
      <div className="flex flex-col gap-6 p-6">
        <div className="flex items-center gap-2"><Skeleton className="h-7 w-32" /></div>
        <Skeleton className="h-32 w-full" />
        <Skeleton className="h-48 w-full" />
      </div>
    )
  }

  if (error || !data) {
    return (
      <div className="p-6">
        <Alert variant="destructive"><AlertDescription>{error || 'User not found'}</AlertDescription></Alert>
        <Button variant="outline" className="mt-4" onClick={() => navigate('/users')}><ArrowLeftIcon /> Back to Users</Button>
      </div>
    )
  }

  const { user, api_keys } = data

  return (
    <div className="flex flex-col gap-6 p-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Button variant="outline" size="sm" onClick={() => navigate('/users')}><ArrowLeftIcon /> Back</Button>
          <div>
            <h1 className="text-2xl font-bold text-foreground">{user.username || 'User Details'}</h1>
            <p className="text-sm text-muted-foreground">Manage user and their API keys</p>
          </div>
        </div>
        <Button variant="outline" size="sm" onClick={() => { setEditForm({ username: user.username || '', password: '', is_admin: user.is_admin }); setEditDialog(true) }}>
          <PencilIcon /> Edit User
        </Button>
      </div>

      {successMessage && (
        <Alert><AlertDescription className="flex items-center justify-between">{successMessage}<Button variant="ghost" size="icon-xs" onClick={() => setSuccessMessage(null)}>×</Button></AlertDescription></Alert>
      )}
      {error && (
        <Alert variant="destructive"><AlertDescription className="flex items-center justify-between">{error}<Button variant="ghost" size="icon-xs" onClick={() => setError(null)}>×</Button></AlertDescription></Alert>
      )}

      {/* User Info */}
      <Card>
        <CardHeader><CardTitle>User Information</CardTitle></CardHeader>
        <CardContent>
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-4">
            <div><Label className="text-xs text-muted-foreground">ID</Label><p className="text-sm">{user.id}</p></div>
            <div><Label className="text-xs text-muted-foreground">Username</Label><p className="text-sm">{user.username || '—'}</p></div>
            <div><Label className="text-xs text-muted-foreground">Type</Label><Badge variant="secondary">{user.user_type}</Badge></div>
            <div><Label className="text-xs text-muted-foreground">Admin</Label><p className="text-sm">{user.is_admin ? <Badge>Yes</Badge> : 'No'}</p></div>
            <div><Label className="text-xs text-muted-foreground">External ID</Label><p className="text-sm font-mono truncate max-w-48" title={user.external_id || ''}>{user.external_id || '—'}</p></div>
            <div><Label className="text-xs text-muted-foreground">Created</Label><p className="text-sm">{new Date(user.created_at).toLocaleString()}</p></div>
            <div><Label className="text-xs text-muted-foreground">Updated</Label><p className="text-sm">{new Date(user.updated_at).toLocaleString()}</p></div>
          </div>
        </CardContent>
      </Card>

      <Separator />

      {/* API Keys */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <CardTitle>API Keys</CardTitle>
          <Button size="sm" onClick={() => { setCreateKeyDialog(true); setCreatedKey(null); setKeyForm({ name: '', expiresInDays: '' }) }}>
            <PlusIcon /> Create Key
          </Button>
        </CardHeader>
        <CardContent>
          {api_keys.length === 0 ? (
            <p className="text-muted-foreground text-sm text-center py-8">No API keys for this user</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Last Four</TableHead>
                  <TableHead>Created</TableHead>
                  <TableHead>Expires</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead className="w-28 text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {api_keys.map((k) => (
                  <TableRow key={k.id}>
                    <TableCell className="font-medium">{k.name}</TableCell>
                    <TableCell className="font-mono text-sm text-muted-foreground">...{k.last_four}</TableCell>
                    <TableCell className="text-sm text-muted-foreground">{new Date(k.created_at).toLocaleDateString()}</TableCell>
                    <TableCell className="text-sm text-muted-foreground">{k.expires_at ? new Date(k.expires_at).toLocaleDateString() : 'Never'}</TableCell>
                    <TableCell><Badge variant={k.is_active ? 'default' : 'secondary'}>{k.is_active ? 'Active' : 'Inactive'}</Badge></TableCell>
                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        {k.is_active ? (
                          <>
                            <Button variant="ghost" size="icon-xs" onClick={() => handleKeyAction(k.id, 'disable')} title="Disable"><BanIcon /></Button>
                            <Button variant="ghost" size="icon-xs" onClick={() => { setDeleteKeyTarget({ id: k.id, name: k.name }) }} title="Delete"><TrashIcon /></Button>
                          </>
                        ) : (
                          <>
                            <Button variant="ghost" size="icon-xs" onClick={() => handleKeyAction(k.id, 'enable')} title="Enable"><BanIcon /></Button>
                            <Button variant="ghost" size="icon-xs" onClick={() => { setDeleteKeyTarget({ id: k.id, name: k.name }) }} title="Delete"><TrashIcon /></Button>
                          </>
                        )}
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      {/* Edit User Dialog */}
      <Dialog open={editDialog} onOpenChange={(o) => { if (!o) setEditDialog(false) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Edit User</DialogTitle>
            <DialogDescription>Update user settings.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleEditSave} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5"><Label htmlFor="ed-name">Username</Label><Input id="ed-name" value={editForm.username} onChange={(e) => setEditForm({ ...editForm, username: e.target.value })} /></div>
            <div className="flex flex-col gap-1.5"><Label htmlFor="ed-pass">New Password</Label><Input id="ed-pass" type="password" value={editForm.password} onChange={(e) => setEditForm({ ...editForm, password: e.target.value })} placeholder="Leave empty to keep current" /></div>
            <div className="flex items-center gap-2"><Checkbox id="ed-admin" checked={editForm.is_admin} onCheckedChange={(c) => setEditForm({ ...editForm, is_admin: !!c })} /><Label htmlFor="ed-admin">Admin user</Label></div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setEditDialog(false)} disabled={saving}>Cancel</Button>
              <Button type="submit" disabled={saving}>{saving ? 'Saving...' : 'Update'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Create API Key Dialog */}
      <Dialog open={createKeyDialog} onOpenChange={(o) => { if (!o) setCreateKeyDialog(false) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create API Key</DialogTitle>
            <DialogDescription>Create a new API key for this user.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleCreateKey} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5"><Label htmlFor="k-name">Key Name</Label><Input id="k-name" value={keyForm.name} onChange={(e) => setKeyForm({ ...keyForm, name: e.target.value })} required /></div>
            <div className="flex flex-col gap-1.5"><Label htmlFor="k-exp">Expires In (days)</Label><Input id="k-exp" type="number" min={1} value={keyForm.expiresInDays} onChange={(e) => setKeyForm({ ...keyForm, expiresInDays: e.target.value })} placeholder="Optional" /></div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setCreateKeyDialog(false)} disabled={saving}>Cancel</Button>
              <Button type="submit" disabled={saving}>{saving ? 'Creating...' : 'Create Key'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Key Confirmation */}
      <AlertDialog open={!!deleteKeyTarget} onOpenChange={(o) => { if (!o) setDeleteKeyTarget(null) }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete API Key</AlertDialogTitle>
            <AlertDialogDescription>Permanently delete API key <span className="font-medium text-foreground">{deleteKeyTarget?.name}</span>?</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => handleKeyAction(deleteKeyTarget!.id, 'delete')}>Delete</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Show created key */}
      {createdKey && (
        <Dialog open={!!createdKey} onOpenChange={() => setCreatedKey(null)}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>API Key Created</DialogTitle>
              <DialogDescription>Copy this key now — it won&apos;t be shown again.</DialogDescription>
            </DialogHeader>
            <div className="flex flex-col gap-4">
              <div className="flex items-center gap-2">
                <code className="flex-1 rounded border bg-muted p-2 font-mono text-sm break-all">{createdKey}</code>
                <Button variant="outline" size="icon-sm" onClick={() => { navigator.clipboard.writeText(createdKey); setSuccessMessage('Copied!') }}><CopyIcon /></Button>
              </div>
              <Button onClick={() => setCreatedKey(null)}>Done</Button>
            </div>
          </DialogContent>
        </Dialog>
      )}
    </div>
  )
}