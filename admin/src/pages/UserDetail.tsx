import { useEffect, useState } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import { api } from '../api/client'
import type { UserDetailResponse } from '../types'

export default function UserDetail() {
  const { id } = useParams<{ id: string }>()
  const navigate = useNavigate()
  const [data, setData] = useState<UserDetailResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [showEditModal, setShowEditModal] = useState(false)
  const [newKeyName, setNewKeyName] = useState('')
  const [expiresInDays, setExpiresInDays] = useState('')
  const [creating, setCreating] = useState(false)
  const [createdKey, setCreatedKey] = useState<string | null>(null)
  const [editFormData, setEditFormData] = useState<{ username: string; password: string; is_admin: boolean }>({
    username: '',
    password: '',
    is_admin: false,
  })

  useEffect(() => {
    if (id) {
      loadUser()
    }
  }, [id])

  async function loadUser() {
    try {
      setLoading(true)
      const userId = parseInt(id!)
      const result = await api.getUser(userId)
      setData(result)
      setError(null)
      if (result.user) {
        setEditFormData({
          username: result.user.username || '',
          password: '',
          is_admin: result.user.is_admin,
        })
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
    try {
      await api.updateUser(parseInt(id!), editFormData)
      setShowEditModal(false)
      loadUser()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to update user')
    }
  }

  async function handleDisable(keyId: number) {
    if (!confirm('Are you sure you want to disable this API key?')) return
    try {
      await api.disableApiKey(keyId)
      loadUser()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to disable API key')
    }
  }

  async function handleEnable(keyId: number) {
    try {
      await api.enableApiKey(keyId)
      loadUser()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to enable API key')
    }
  }

  async function handleDelete(keyId: number) {
    if (!confirm('Are you sure you want to permanently delete this API key?')) return
    try {
      await api.deleteApiKey(keyId)
      loadUser()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to delete API key')
    }
  }

  function getUserTypeLabel(type: string) {
    switch (type) {
      case 'internal': return 'Internal'
      case 'nostr': return 'Nostr'
      case 'oauth': return 'OAuth'
      default: return type
    }
  }

  if (loading) {
    return (
      <div className="p-8">
        <h1 className="text-2xl font-bold mb-6 text-text-primary">User Details</h1>
        <p className="text-text-secondary">Loading...</p>
      </div>
    )
  }

  if (error || !data) {
    return (
      <div className="p-8">
        <h1 className="text-2xl font-bold mb-6 text-text-primary">User Details</h1>
        <div className="p-4 bg-red-100 border border-red-400 text-red-700 rounded">
          {error || 'User not found'}
        </div>
        <button
          onClick={() => navigate('/users')}
          className="mt-4 px-4 py-2 bg-accent text-white rounded hover:bg-accent-hover"
        >
          Back to Users
        </button>
      </div>
    )
  }

  const { user, api_keys } = data

  return (
    <div className="p-8">
      <div className="flex justify-between items-center mb-6">
        <h1 className="text-2xl font-bold text-text-primary">User Details</h1>
        <div className="flex gap-2">
          <button
            onClick={() => {
              if (data?.user) {
                setEditFormData({
                  username: data.user.username || '',
                  password: '',
                  is_admin: data.user.is_admin,
                })
              }
              setShowEditModal(true)
            }}
            className="px-4 py-2 bg-layer-2 text-text-primary rounded hover:bg-layer-3 border border-border"
          >
            Edit User
          </button>
          <button
            onClick={() => navigate('/users')}
            className="px-4 py-2 bg-layer-2 text-text-primary rounded hover:bg-layer-3 border border-border"
          >
            Back to Users
          </button>
        </div>
      </div>

      {/* User Info Card */}
      <div className="bg-layer-3 rounded-lg border border-border p-6 mb-8">
        <h2 className="text-lg font-semibold mb-4 text-text-primary">User Information</h2>
        <div className="grid grid-cols-2 gap-4">
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">ID</label>
            <p className="text-text-primary">{user.id}</p>
          </div>
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">Username</label>
            <p className="text-text-primary">{user.username || <span className="text-text-secondary">N/A</span>}</p>
          </div>
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">External ID</label>
            <p className="text-text-primary font-mono text-sm truncate" title={user.external_id || ''}>
              {user.external_id || <span className="text-text-secondary">N/A</span>}
            </p>
          </div>
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">User Type</label>
            <span className={`px-2 py-1 rounded text-xs ${
              user.user_type === 'internal' ? 'bg-blue-100 text-blue-800' :
              user.user_type === 'nostr' ? 'bg-purple-100 text-purple-800' :
              'bg-green-100 text-green-800'
            }`}>
              {getUserTypeLabel(user.user_type)}
            </span>
          </div>
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">Admin</label>
            <p className="text-text-primary">
              {user.is_admin ? (
                <span className="px-2 py-1 rounded text-xs bg-yellow-100 text-yellow-800">Yes</span>
              ) : (
                <span className="text-text-secondary">No</span>
              )}
            </p>
          </div>
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">Created</label>
            <p className="text-text-primary">{new Date(user.created_at).toLocaleString()}</p>
          </div>
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">Last Updated</label>
            <p className="text-text-primary">{new Date(user.updated_at).toLocaleString()}</p>
          </div>
        </div>
      </div>

      {/* API Keys Card */}
      <div className="bg-layer-3 rounded-lg border border-border p-6">
        <div className="flex justify-between items-center mb-4">
          <h2 className="text-lg font-semibold text-text-primary">API Keys</h2>
          <button
            onClick={() => {
              setCreatedKey(null)
              setNewKeyName('')
              setExpiresInDays('')
              setShowCreateModal(true)
            }}
            className="px-3 py-1 text-sm bg-accent text-white rounded hover:bg-accent-hover"
          >
            Create Key
          </button>
        </div>

        {api_keys.length === 0 ? (
          <p className="text-text-secondary text-center py-8">No API keys for this user</p>
        ) : (
          <div className="overflow-hidden rounded border border-border">
            <table className="w-full">
              <thead className="bg-layer-2 border-b border-border">
                <tr>
                  <th className="px-4 py-3 text-left text-sm font-semibold text-text-secondary">Name</th>
                  <th className="px-4 py-3 text-left text-sm font-semibold text-text-secondary">Last Four</th>
                  <th className="px-4 py-3 text-left text-sm font-semibold text-text-secondary">Created</th>
                  <th className="px-4 py-3 text-left text-sm font-semibold text-text-secondary">Expires</th>
                  <th className="px-4 py-3 text-left text-sm font-semibold text-text-secondary">Status</th>
                  <th className="px-4 py-3 text-left text-sm font-semibold text-text-secondary">Actions</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {api_keys.map((key) => (
                  <tr key={key.id}>
                    <td className="px-4 py-3 text-sm font-medium text-text-primary">{key.name}</td>
                    <td className="px-4 py-3 text-sm font-mono text-text-secondary">...{key.last_four}</td>
                    <td className="px-4 py-3 text-sm text-text-secondary">
                      {new Date(key.created_at).toLocaleDateString()}
                    </td>
                    <td className="px-4 py-3 text-sm text-text-secondary">
                      {key.expires_at ? new Date(key.expires_at).toLocaleDateString() : 'Never'}
                    </td>
                    <td className="px-4 py-3 text-sm">
                      <span className={`px-2 py-1 rounded text-xs ${
                        key.is_active ? 'bg-green-100 text-green-800' : 'bg-red-100 text-red-800'
                      }`}>
                        {key.is_active ? 'Active' : 'Inactive'}
                      </span>
                    </td>
                    <td className="px-4 py-3 text-sm">
                      <div className="flex gap-2">
                        {key.is_active ? (
                          <>
                            <button
                              onClick={() => handleDisable(key.id)}
                              className="px-2 py-1 text-xs bg-yellow-500 text-white rounded hover:bg-yellow-600"
                            >
                              Disable
                            </button>
                            <button
                              onClick={() => handleDelete(key.id)}
                              className="px-2 py-1 text-xs bg-red-500 text-white rounded hover:bg-red-600"
                            >
                              Delete
                            </button>
                          </>
                        ) : (
                          <>
                            <button
                              onClick={() => handleEnable(key.id)}
                              className="px-2 py-1 text-xs bg-green-500 text-white rounded hover:bg-green-600"
                            >
                              Enable
                            </button>
                            <button
                              onClick={() => handleDelete(key.id)}
                              className="px-2 py-1 text-xs bg-red-500 text-white rounded hover:bg-red-600"
                            >
                              Delete
                            </button>
                          </>
                        )}
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Create API Key Modal */}
      {showCreateModal && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-layer-3 rounded-lg p-6 w-full max-w-md border border-border">
            <h2 className="text-xl font-bold mb-4 text-text-primary">Create API Key</h2>
            
            <form onSubmit={async (e) => {
              e.preventDefault()
              if (!id) return
              setCreating(true)
              try {
                const key = await api.createApiKey(newKeyName, expiresInDays ? parseInt(expiresInDays) : undefined, parseInt(id!))
                setCreatedKey(key.key || null)
                setShowCreateModal(false)
                setNewKeyName('')
                setExpiresInDays('')
                loadUser()
              } catch (err) {
                setError(err instanceof Error ? err.message : 'Failed to create API key')
              } finally {
                setCreating(false)
              }
            }}>
              <div className="space-y-4">
                <div>
                  <label className="block text-sm font-medium text-text-secondary mb-1">Key Name</label>
                  <input
                    type="text"
                    value={newKeyName}
                    onChange={(e) => setNewKeyName(e.target.value)}
                    className="w-full px-3 py-2 bg-layer-1 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                    placeholder="My API Key"
                    required
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium text-text-secondary mb-1">Expires In (days, optional)</label>
                  <input
                    type="number"
                    value={expiresInDays}
                    onChange={(e) => setExpiresInDays(e.target.value)}
                    className="w-full px-3 py-2 bg-layer-1 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                    placeholder="30"
                    min="1"
                  />
                </div>
              </div>

              <div className="flex justify-end gap-3 mt-6">
                <button
                  type="button"
                  onClick={() => {
                    setShowCreateModal(false)
                    setCreatedKey(null)
                    setNewKeyName('')
                    setExpiresInDays('')
                  }}
                  className="px-4 py-2 text-text-secondary hover:text-text-primary"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  disabled={creating}
                  className="px-4 py-2 bg-accent text-white rounded hover:bg-accent-hover disabled:opacity-50"
                >
                  {creating ? 'Creating...' : 'Create Key'}
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Edit User Modal */}
      {showEditModal && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-layer-3 rounded-lg p-6 w-full max-w-md border border-border">
            <h2 className="text-xl font-bold mb-4 text-text-primary">Edit User</h2>
            
            <form onSubmit={handleEditSave}>
              <div className="space-y-4">
                <div>
                  <label className="block text-sm font-medium text-text-secondary mb-1">
                    Username
                  </label>
                  <input
                    type="text"
                    value={editFormData.username}
                    onChange={(e) => setEditFormData({ ...editFormData, username: e.target.value })}
                    className="w-full px-3 py-2 bg-layer-1 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                  />
                </div>

                <div>
                  <label className="block text-sm font-medium text-text-secondary mb-1">
                    New Password (leave empty to keep current)
                  </label>
                  <input
                    type="password"
                    value={editFormData.password}
                    onChange={(e) => setEditFormData({ ...editFormData, password: e.target.value })}
                    className="w-full px-3 py-2 bg-layer-1 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                    placeholder="Enter new password"
                  />
                </div>

                <div className="flex items-center">
                  <input
                    type="checkbox"
                    id="edit_is_admin"
                    checked={editFormData.is_admin}
                    onChange={(e) => setEditFormData({ ...editFormData, is_admin: e.target.checked })}
                    className="mr-2"
                  />
                  <label htmlFor="edit_is_admin" className="text-sm font-medium text-text-secondary">
                    Admin user
                  </label>
                </div>
              </div>

              <div className="flex justify-end gap-3 mt-6">
                <button
                  type="button"
                  onClick={() => setShowEditModal(false)}
                  className="px-4 py-2 text-text-secondary hover:text-text-primary"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="px-4 py-2 bg-accent text-white rounded hover:bg-accent-hover"
                >
                  Update
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Show created key */}
      {createdKey && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-layer-3 rounded-lg p-6 w-full max-w-md border border-border">
            <h2 className="text-xl font-bold mb-4 text-text-primary">API Key Created</h2>
            <div className="mb-4 p-4 bg-green-100 border border-green-400 text-green-700 rounded">
              <p className="font-semibold mb-2">Your new API key:</p>
              <div className="flex items-center gap-2">
                <code className="flex-1 p-2 bg-white rounded font-mono text-sm">{createdKey}</code>
                <button
                  onClick={() => {
                    navigator.clipboard.writeText(createdKey)
                    alert('Copied!')
                  }}
                  className="px-3 py-1 bg-green-600 text-white rounded hover:bg-green-700 text-sm"
                >
                  Copy
                </button>
              </div>
              <p className="text-sm mt-2 text-green-600">
                Copy this key now - it won't be shown again!
              </p>
            </div>
            <div className="flex justify-end">
              <button
                onClick={() => setCreatedKey(null)}
                className="px-4 py-2 bg-accent text-white rounded hover:bg-accent-hover"
              >
                Done
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
