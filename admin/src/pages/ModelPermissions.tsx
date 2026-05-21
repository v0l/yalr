import { useState, useEffect } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Checkbox } from '@/components/ui/checkbox'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Badge } from '@/components/ui/badge'
import { ArrowLeft, Plus, Trash2, Save, X } from 'lucide-react'
import { api } from '@/api/client'
import type { UserModelPermission, CreateUserModelPermission } from '@/types'

export default function ModelPermissions() {
  const { userId } = useParams()
  const navigate = useNavigate()
  const [permissions, setPermissions] = useState<UserModelPermission[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [newModel, setNewModel] = useState('')
  const [newAllow, setNewAllow] = useState(true)
  const [editingId, setEditingId] = useState<number | null>(null)
  const [editModel, setEditModel] = useState('')
  const [editAllow, setEditAllow] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const fetchPermissions = async () => {
    if (!userId) return
    try {
      setLoading(true)
      const data = await api.listUserModelPermissions(parseInt(userId))
      setPermissions(data)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load permissions')
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    fetchPermissions()
  }, [userId])

  const handleAdd = async () => {
    if (!newModel.trim() || !userId) return
    
    try {
      setSaving(true)
      const data: CreateUserModelPermission = {
        user_id: parseInt(userId),
        model: newModel.trim(),
        allow: newAllow,
      }
      await api.createUserModelPermission(data)
      setNewModel('')
      setNewAllow(true)
      await fetchPermissions()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to add permission')
    } finally {
      setSaving(false)
    }
  }

  const handleDelete = async (permission: UserModelPermission) => {
    if (!confirm(`Delete permission for model "${permission.model}"?`)) return
    
    try {
      setSaving(true)
      await api.deleteUserModelPermission(permission.user_id, permission.model)
      await fetchPermissions()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to delete permission')
    } finally {
      setSaving(false)
    }
  }

  const handleEditStart = (permission: UserModelPermission) => {
    setEditingId(permission.id)
    setEditModel(permission.model)
    setEditAllow(permission.allow)
  }

  const handleEditCancel = () => {
    setEditingId(null)
    setEditModel('')
    setEditAllow(true)
  }

  const handleEditSave = async (permission: UserModelPermission) => {
    if (!editModel.trim()) return
    
    try {
      setSaving(true)
      const data: CreateUserModelPermission = {
        user_id: permission.user_id,
        model: editModel.trim(),
        allow: editAllow,
      }
      await api.createUserModelPermission(data)
      setEditingId(null)
      await fetchPermissions()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to update permission')
    } finally {
      setSaving(false)
    }
  }

  const formatModel = (model: string) => {
    if (model === '*') {
      return (
        <span className="flex items-center gap-2">
          <Badge variant="outline" className="bg-purple-100 text-purple-700">Wildcard</Badge>
          <span>All models</span>
        </span>
      )
    }
    return model
  }

  const formatAllow = (allow: boolean, isEditing = false, permission?: UserModelPermission) => {
    if (isEditing && editingId === permission?.id) {
      return (
        <div className="flex items-center gap-2">
          <Checkbox checked={editAllow} onCheckedChange={(c) => setEditAllow(!!c)} />
          <span>{editAllow ? 'Allow' : 'Deny'}</span>
        </div>
      )
    }
    return (
      <Badge variant={allow ? 'default' : 'destructive'}>
        {allow ? 'Allow' : 'Deny'}
      </Badge>
    )
  }

  return (
    <div className="container mx-auto py-6">
      <div className="mb-6">
        <Button variant="ghost" onClick={() => navigate('/users')} className="mb-4">
          <ArrowLeft className="mr-2 h-4 w-4" />
          Back to Users
        </Button>
        
        <Card>
          <CardHeader>
            <CardTitle>Model Access Permissions</CardTitle>
            <CardDescription>
              Control which models this user can access. Leave empty to allow all models by default.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {error && (
              <div className="mb-4 p-3 bg-red-50 border border-red-200 rounded-md text-red-700">
                {error}
              </div>
            )}

            {/* Add New Permission */}
            <div className="mb-6 space-y-4">
              <h3 className="font-semibold text-lg">Add New Permission</h3>
              <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
                <div className="md:col-span-2">
                  <Label htmlFor="model">Model Name</Label>
                  <Input
                    id="model"
                    placeholder="e.g., gpt-4, claude-3, or * for all"
                    value={newModel}
                    onChange={(e) => setNewModel(e.target.value)}
                  />
                </div>
                <div>
                  <Label>Permission</Label>
                  <Select value={newAllow ? 'allow' : 'deny'} onValueChange={(v) => setNewAllow(v === 'allow')}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="allow">Allow</SelectItem>
                      <SelectItem value="deny">Deny</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="flex items-end">
                  <Button onClick={handleAdd} disabled={!newModel.trim() || saving} className="w-full">
                    <Plus className="mr-2 h-4 w-4" />
                    Add
                  </Button>
                </div>
              </div>
            </div>

            {/* Permissions Table */}
            <div>
              <h3 className="font-semibold text-lg mb-4">Current Permissions</h3>
              {loading ? (
                <div className="text-center py-8 text-muted-foreground">Loading...</div>
              ) : permissions.length === 0 ? (
                <div className="text-center py-8 text-muted-foreground">
                  No permissions set. User can access all models by default.
                </div>
              ) : (
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Model</TableHead>
                      <TableHead>Permission</TableHead>
                      <TableHead className="w-32">Actions</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {permissions.map((permission) => (
                      <TableRow key={permission.id}>
                        <TableCell>
                          {editingId === permission.id ? (
                            <Input
                              value={editModel}
                              onChange={(e) => setEditModel(e.target.value)}
                              placeholder="Model name"
                              className="h-8"
                            />
                          ) : (
                            formatModel(permission.model)
                          )}
                        </TableCell>
                        <TableCell>
                          {editingId === permission.id ? (
                            <div className="flex items-center gap-2">
                              <Checkbox checked={editAllow} onCheckedChange={(c) => setEditAllow(!!c)} />
                              <span>{editAllow ? 'Allow' : 'Deny'}</span>
                            </div>
                          ) : (
                            formatAllow(permission.allow)
                          )}
                        </TableCell>
                        <TableCell>
                          {editingId === permission.id ? (
                            <div className="flex gap-2">
                              <Button
                                size="sm"
                                variant="default"
                                onClick={() => handleEditSave(permission)}
                                disabled={saving || !editModel.trim()}
                              >
                                <Save className="h-3 w-3" />
                              </Button>
                              <Button
                                size="sm"
                                variant="outline"
                                onClick={handleEditCancel}
                                disabled={saving}
                              >
                                <X className="h-3 w-3" />
                              </Button>
                            </div>
                          ) : (
                            <div className="flex gap-2">
                              <Button
                                size="sm"
                                variant="outline"
                                onClick={() => handleEditStart(permission)}
                              >
                                Edit
                              </Button>
                              <Button
                                size="sm"
                                variant="destructive"
                                onClick={() => handleDelete(permission)}
                                disabled={saving}
                              >
                                <Trash2 className="h-3 w-3" />
                              </Button>
                            </div>
                          )}
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              )}
            </div>

            {/* Info Box */}
            <div className="mt-6 p-4 bg-blue-50 border border-blue-200 rounded-md">
              <h4 className="font-semibold text-sm mb-2">How it works</h4>
              <ul className="text-sm space-y-1 text-muted-foreground">
                <li>• By default, users can access all models (no restrictions)</li>
                <li>• Add specific model rules to allow or deny access</li>
                <li>• Use <code className="bg-blue-100 px-1 rounded">*</code> (wildcard) to apply rules to all models</li>
                <li>• Exact model matches take precedence over wildcard rules</li>
                <li>• Example: Deny all with <code className="bg-blue-100 px-1 rounded">*</code>, then allow specific models</li>
              </ul>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
