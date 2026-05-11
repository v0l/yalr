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
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { Card, CardContent } from '@/components/ui/card'
import { PlusIcon, EyeIcon, TrashIcon } from 'lucide-react'

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
    username: '',
    password: '',
    external_id: '',
    user_type: 'internal',
    is_admin: false,
  })

  useEffect(() => { loadUsers() }, [])

  async function loadUsers() {
    try {
      setLoading(true)
      const data = await api.getUsers()
      setUsers(data)
      setError(null)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load users')
    } finally {
      setLoading(false)
    }
  }

  function openCreate() {
    setFormData({ username: '', password: '', external_id: '', user_type: 'internal', is_admin: false })
    setDialogOpen(true)
  }

  async function handleCreate(e: React.FormEvent) {
    e.preventDefault()
    setSaving(true)
    try {
      await api.createUser(formData)
      setDialogOpen(false)
      setSuccessMessage('User created successfully')
      loadUsers()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to create user')
    } finally {
      setSaving(false)
    }
  }

  async function handleDelete() {
    if (!deleteTarget) return
    try {
      await api.deleteUser(deleteTarget.id)
      setDeleteTarget(null)
      setSuccessMessage('User deleted')
      loadUsers()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to delete user')
    }
  }

  const typeBadgeVariant = (t: string) => t === 'internal' ? 'default' : t === 'nostr' ? 'secondary' : 'outline'

  if (loading) {
    return (
      <div className="flex flex-col gap-6 p-6">
        <div className="flex items-center justify-between">
          <div className="flex flex-col gap-1"><Skeleton className="h-7 w-40" /><Skeleton className="h-4 w-52" /></div>
          <Skeleton className="h-8 w-28" />
        </div>
        <Skeleton className="h-64 w-full" />
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-6 p-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold text-foreground">User Management</h1>
          <p className="text-sm text-muted-foreground">Manage users and permissions</p>
        </div>
        <Button onClick={openCreate}>
          <PlusIcon /> Add User
        </Button>
      </div>

      {successMessage && (
        <Alert><AlertDescription className="flex items-center justify-between">{successMessage}<Button variant="ghost" size="icon-xs" onClick={() => setSuccessMessage(null)}>×</Button></AlertDescription></Alert>
      )}
      {error && (
        <Alert variant="destructive"><AlertDescription className="flex items-center justify-between">{error}<Button variant="ghost" size="icon-xs" onClick={() => setError(null)}>×</Button></AlertDescription></Alert>
      )}

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Username</TableHead>
                <TableHead>External ID</TableHead>
                <TableHead>Type</TableHead>
                <TableHead>Admin</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="w-24 text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {users.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center text-muted-foreground py-12">No users found</TableCell>
                </TableRow>
              ) : (
                users.map((user) => (
                  <TableRow key={user.id}>
                    <TableCell className="font-medium">{user.username || <span className="text-muted-foreground">N/A</span>}</TableCell>
                    <TableCell className="font-mono text-sm text-muted-foreground truncate max-w-48" title={user.external_id || ''}>
                      {user.external_id || <span className="text-muted-foreground">N/A</span>}
                    </TableCell>
                    <TableCell><Badge variant={typeBadgeVariant(user.user_type)}>{user.user_type}</Badge></TableCell>
                    <TableCell>{user.is_admin ? <Badge variant="secondary">Admin</Badge> : <span className="text-muted-foreground text-sm">Regular</span>}</TableCell>
                    <TableCell className="text-sm text-muted-foreground">{new Date(user.created_at).toLocaleDateString()}</TableCell>
                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Button variant="ghost" size="icon-xs" onClick={() => navigate(`/users/${user.id}`)}><EyeIcon /></Button>
                        <Button variant="ghost" size="icon-xs" onClick={() => setDeleteTarget(user)}><TrashIcon /></Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {/* Create Dialog */}
      <Dialog open={dialogOpen} onOpenChange={(o) => { if (!o) setDialogOpen(false) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create User</DialogTitle>
            <DialogDescription>Add a new user to the system.</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleCreate} className="flex flex-col gap-4">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="u-type">User Type</Label>
              <Select value={formData.user_type} onValueChange={(v) => setFormData({ ...formData, user_type: v as 'internal' | 'nostr' | 'oauth' })}>
                <SelectTrigger id="u-type" className="w-full"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    <SelectItem value="internal">Internal</SelectItem>
                    <SelectItem value="nostr">Nostr</SelectItem>
                    <SelectItem value="oauth">OAuth</SelectItem>
                  </SelectGroup>
                </SelectContent>
              </Select>
            </div>
            <div className={cn(formData.user_type !== 'internal' && 'opacity-50 pointer-events-none')}>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="u-name">Username</Label>
                <Input id="u-name" value={formData.username || ''} onChange={(e) => setFormData({ ...formData, username: e.target.value })} required disabled={formData.user_type !== 'internal'} />
              </div>
              <div className="flex flex-col gap-1.5 mt-4">
                <Label htmlFor="u-pass">Password</Label>
                <Input id="u-pass" type="password" value={formData.password || ''} onChange={(e) => setFormData({ ...formData, password: e.target.value })} required disabled={formData.user_type !== 'internal'} />
              </div>
            </div>
            <div className={cn(formData.user_type === 'internal' && 'opacity-50 pointer-events-none')}>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="u-ext">External ID</Label>
                <Input id="u-ext" value={formData.external_id || ''} onChange={(e) => setFormData({ ...formData, external_id: e.target.value })} required={formData.user_type !== 'internal'} disabled={formData.user_type === 'internal'} />
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Checkbox id="u-admin" checked={formData.is_admin} onCheckedChange={(c) => setFormData({ ...formData, is_admin: !!c })} />
              <Label htmlFor="u-admin" className="cursor-pointer">Admin user</Label>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setDialogOpen(false)} disabled={saving}>Cancel</Button>
              <Button type="submit" disabled={saving}>{saving ? 'Creating...' : 'Create'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation */}
      <AlertDialog open={!!deleteTarget} onOpenChange={(o) => { if (!o) setDeleteTarget(null) }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete User</AlertDialogTitle>
            <AlertDialogDescription>
              Delete <span className="font-medium text-foreground">{deleteTarget?.username}</span>? This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={handleDelete}>Delete</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function cn(...classes: (string | false | undefined | null)[]) { return classes.filter(Boolean).join(' ') }