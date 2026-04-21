import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { api } from '../api/client'
import type { User, CreateUserRequest } from '../types'

export default function Users() {
  const navigate = useNavigate()
  const [users, setUsers] = useState<User[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [deletingUser, setDeletingUser] = useState<User | null>(null)

  // Form state
  const [formData, setFormData] = useState<CreateUserRequest>({
    username: '',
    password: '',
    external_id: '',
    user_type: 'internal',
    is_admin: false,
  })

  useEffect(() => {
    loadUsers()
  }, [])

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

  function handleCreateClick() {
    setFormData({
      username: '',
      password: '',
      external_id: '',
      user_type: 'internal',
      is_admin: false,
    })
    setShowCreateModal(true)
  }

  function handleDeleteClick(user: User) {
    setDeletingUser(user)
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    
    try {
      await api.createUser(formData)
      setShowCreateModal(false)
      loadUsers()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save user')
    }
  }

  async function handleDeleteConfirm() {
    if (!deletingUser) return
    
    try {
      await api.deleteUser(deletingUser.id)
      setDeletingUser(null)
      loadUsers()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to delete user')
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
        <h1 className="text-2xl font-bold mb-6 text-text-primary">User Management</h1>
        <p className="text-text-secondary">Loading...</p>
      </div>
    )
  }

  return (
    <div className="p-8">
      <div className="flex justify-between items-center mb-6">
        <h1 className="text-2xl font-bold text-text-primary">User Management</h1>
        <button
          onClick={handleCreateClick}
          className="px-4 py-2 bg-accent text-white rounded hover:bg-accent-hover"
        >
          Add User
        </button>
      </div>

      {error && (
        <div className="mb-4 p-4 bg-red-100 border border-red-400 text-red-700 rounded">
          {error}
        </div>
      )}

      <div className="bg-layer-3 rounded-lg border border-border overflow-hidden">
        <table className="w-full">
          <thead className="bg-layer-2 border-b border-border">
            <tr>
              <th className="px-4 py-3 text-left text-sm font-semibold text-text-secondary">ID</th>
              <th className="px-4 py-3 text-left text-sm font-semibold text-text-secondary">Username</th>
              <th className="px-4 py-3 text-left text-sm font-semibold text-text-secondary">External ID</th>
              <th className="px-4 py-3 text-left text-sm font-semibold text-text-secondary">Type</th>
              <th className="px-4 py-3 text-left text-sm font-semibold text-text-secondary">Admin</th>
              <th className="px-4 py-3 text-left text-sm font-semibold text-text-secondary">Created</th>
              <th className="px-4 py-3 text-right text-sm font-semibold text-text-secondary">Actions</th>
            </tr>
          </thead>
          <tbody>
            {users.length === 0 ? (
              <tr>
                <td colSpan={7} className="px-4 py-8 text-center text-text-secondary">
                  No users found
                </td>
              </tr>
            ) : (
              users.map((user) => (
                <tr key={user.id} className="border-b border-border hover:bg-layer-2">
                  <td className="px-4 py-3 text-sm">{user.id}</td>
                  <td className="px-4 py-3 text-sm font-medium text-text-primary">
                    {user.username || <span className="text-text-secondary">N/A</span>}
                  </td>
                  <td className="px-4 py-3 text-sm text-text-secondary">
                    {user.external_id ? (
                      <span className="truncate max-w-xs inline-block" title={user.external_id}>
                        {user.external_id}
                      </span>
                    ) : (
                      <span className="text-text-secondary">N/A</span>
                    )}
                  </td>
                  <td className="px-4 py-3 text-sm">
                    <span className={`px-2 py-1 rounded text-xs ${
                      user.user_type === 'internal' ? 'bg-blue-100 text-blue-800' :
                      user.user_type === 'nostr' ? 'bg-purple-100 text-purple-800' :
                      'bg-green-100 text-green-800'
                    }`}>
                      {getUserTypeLabel(user.user_type)}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-sm">
                    {user.is_admin ? (
                      <span className="px-2 py-1 rounded text-xs bg-yellow-100 text-yellow-800">
                        Admin
                      </span>
                    ) : (
                      <span className="text-text-secondary">Regular</span>
                    )}
                  </td>
                  <td className="px-4 py-3 text-sm text-text-secondary">
                    {new Date(user.created_at).toLocaleDateString()}
                  </td>
                  <td className="px-4 py-3 text-sm">
                    <div className="flex gap-2">
                      <button
                        onClick={() => handleDeleteClick(user)}
                        className="px-3 py-1 text-sm bg-red-500 text-white rounded hover:bg-red-600"
                      >
                        Delete
                      </button>
                      <button
                        onClick={() => navigate(`/users/${user.id}`)}
                        className="px-3 py-1 text-sm bg-layer-2 text-text-primary rounded hover:bg-layer-3 border border-border"
                      >
                        View
                      </button>
                    </div>
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      {/* Create/Edit Modal */}
      {showCreateModal && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-layer-3 rounded-lg p-6 w-full max-w-md border border-border">
            <h2 className="text-xl font-bold mb-4 text-text-primary">
              Create User
            </h2>
            
            <form onSubmit={handleSubmit}>
              <div className="space-y-4">
                <div>
                  <label className="block text-sm font-medium text-text-secondary mb-1">
                    Username
                  </label>
                  <input
                    type="text"
                    value={formData.username || ''}
                    onChange={(e) => setFormData({ ...formData, username: e.target.value })}
                    className="w-full px-3 py-2 bg-layer-1 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                    required
                    disabled={formData.user_type !== 'internal'}
                  />
                </div>
                
                <div>
                  <label className="block text-sm font-medium text-text-secondary mb-1">
                    Password
                  </label>
                  <input
                    type="password"
                    value={formData.password || ''}
                    onChange={(e) => setFormData({ ...formData, password: e.target.value })}
                    className="w-full px-3 py-2 bg-layer-1 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                    required
                    disabled={formData.user_type !== 'internal'}
                  />
                </div>

                <div>
                  <label className="block text-sm font-medium text-text-secondary mb-1">
                    External ID (for Nostr/OAuth)
                  </label>
                  <input
                    type="text"
                    value={formData.external_id || ''}
                    onChange={(e) => setFormData({ ...formData, external_id: e.target.value })}
                    className="w-full px-3 py-2 bg-layer-1 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                    disabled={formData.user_type === 'internal'}
                  />
                </div>

                <div>
                  <label className="block text-sm font-medium text-text-secondary mb-1">
                    User Type
                  </label>
                  <select
                    value={formData.user_type}
                    onChange={(e) => setFormData({ 
                      ...formData, 
                      user_type: e.target.value as 'internal' | 'nostr' | 'oauth' 
                    })}
                    className="w-full px-3 py-2 bg-layer-1 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                  >
                    <option value="internal">Internal</option>
                    <option value="nostr">Nostr</option>
                    <option value="oauth">OAuth</option>
                  </select>
                </div>

                <div className="flex items-center">
                  <input
                    type="checkbox"
                    id="is_admin"
                    checked={formData.is_admin}
                    onChange={(e) => setFormData({ ...formData, is_admin: e.target.checked })}
                    className="mr-2"
                  />
                  <label htmlFor="is_admin" className="text-sm font-medium text-text-secondary">
                    Admin user
                  </label>
                </div>
              </div>

              <div className="flex justify-end gap-3 mt-6">
                <button
                  type="button"
                  onClick={() => setShowCreateModal(false)}
                  className="px-4 py-2 text-text-secondary hover:text-text-primary"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="px-4 py-2 bg-accent text-white rounded hover:bg-accent-hover"
                >
                  Create
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Delete Confirmation Modal */}
      {deletingUser && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-layer-3 rounded-lg p-6 w-full max-w-md border border-border">
            <h2 className="text-xl font-bold mb-4 text-text-primary">Delete User</h2>
            <p className="text-text-secondary mb-6">
              Are you sure you want to delete user "{deletingUser.username}"? This action cannot be undone.
            </p>
            <div className="flex justify-end gap-3">
              <button
                onClick={() => setDeletingUser(null)}
                className="px-4 py-2 text-text-secondary hover:text-text-primary"
              >
                Cancel
              </button>
              <button
                onClick={handleDeleteConfirm}
                className="px-4 py-2 bg-red-600 text-white rounded hover:bg-red-700"
              >
                Delete
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
