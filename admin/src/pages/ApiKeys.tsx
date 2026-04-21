import { useEffect, useState } from 'react'
import { api } from '../api/client'

interface ApiKey {
  id: number
  name: string
  key?: string
  last_four: string
  created_at: string
  expires_at: string | null
  is_active?: boolean
}

export default function ApiKeys() {
  const [keys, setKeys] = useState<ApiKey[]>([])
  const [newKeyName, setNewKeyName] = useState('')
  const [expiresInDays, setExpiresInDays] = useState('')
  const [loading, setLoading] = useState(false)
  const [showKey, setShowKey] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    loadKeys()
  }, [])

  async function loadKeys() {
    try {
      const data = await api.getApiKeys()
      setKeys(data)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load API keys')
    }
  }

  async function handleCreate(e: React.FormEvent) {
    e.preventDefault()
    setLoading(true)
    setError(null)

    try {
      const key = await api.createApiKey(newKeyName, expiresInDays ? parseInt(expiresInDays) : undefined)
      setShowKey(key.key || null)
      setNewKeyName('')
      setExpiresInDays('')
      loadKeys()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to create API key')
    } finally {
      setLoading(false)
    }
  }

  async function handleDelete(id: number) {
    if (!confirm('Are you sure you want to permanently delete this API key?')) return
    
    try {
      await api.deleteApiKey(id)
      loadKeys()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to delete API key')
    }
  }

  async function handleDisable(id: number) {
    if (!confirm('Are you sure you want to disable this API key? It can still be deleted later.')) return
    
    try {
      await api.disableApiKey(id)
      loadKeys()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to disable API key')
    }
  }

  async function handleEnable(id: number) {
    try {
      await api.enableApiKey(id)
      loadKeys()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to enable API key')
    }
  }

  function copyToClipboard(text: string) {
    navigator.clipboard.writeText(text)
    alert('Copied to clipboard!')
  }

  return (
    <div className="p-8">
      <h1 className="text-2xl font-bold mb-6 text-text-primary">API Keys</h1>

      {error && (
        <div className="mb-6 p-4 bg-red-100 border border-red-400 text-red-700 rounded">
          Error: {error}
        </div>
      )}

      <form onSubmit={handleCreate} className="mb-8 p-6 bg-layer-3 rounded-lg border border-border">
        <h2 className="text-lg font-semibold mb-4 text-text-primary">Create New API Key</h2>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">Key Name</label>
            <input
              type="text"
              value={newKeyName}
              onChange={(e) => setNewKeyName(e.target.value)}
              className="w-full px-3 py-2 bg-layer-4 border border-border rounded text-text-primary"
              placeholder="My API Key"
              required
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">Expires In (days)</label>
            <input
              type="number"
              value={expiresInDays}
              onChange={(e) => setExpiresInDays(e.target.value)}
              className="w-full px-3 py-2 bg-layer-4 border border-border rounded text-text-primary"
              placeholder="30"
              min="1"
            />
          </div>
          <div className="flex items-end">
            <button
              type="submit"
              disabled={loading}
              className="px-4 py-2 bg-accent text-white rounded hover:bg-accent-hover disabled:opacity-50"
            >
              {loading ? 'Creating...' : 'Create Key'}
            </button>
          </div>
        </div>
      </form>

      {showKey && (
        <div className="mb-8 p-4 bg-green-100 border border-green-400 text-green-700 rounded">
          <p className="font-semibold mb-2">Your new API key:</p>
          <div className="flex items-center gap-2">
            <code className="flex-1 p-2 bg-white rounded font-mono text-sm">{showKey}</code>
            <button
              onClick={() => copyToClipboard(showKey)}
              className="px-3 py-1 bg-green-600 text-white rounded hover:bg-green-700 text-sm"
            >
              Copy
            </button>
          </div>
          <p className="text-sm mt-2 text-green-600">
            ⚠️ Copy this key now - it won't be shown again!
          </p>
        </div>
      )}

      <div className="bg-layer-3 rounded-lg border border-border overflow-hidden">
        <table className="min-w-full divide-y divide-border">
          <thead className="bg-layer-2">
            <tr>
              <th className="px-6 py-3 text-left text-xs font-medium text-text-secondary uppercase tracking-wider">
                Name
              </th>
              <th className="px-6 py-3 text-left text-xs font-medium text-text-secondary uppercase tracking-wider">
                Last Four
              </th>
              <th className="px-6 py-3 text-left text-xs font-medium text-text-secondary uppercase tracking-wider">
                Created
              </th>
              <th className="px-6 py-3 text-left text-xs font-medium text-text-secondary uppercase tracking-wider">
                Expires
              </th>
              <th className="px-6 py-3 text-left text-xs font-medium text-text-secondary uppercase tracking-wider">
                Status
              </th>
              <th className="px-6 py-3 text-left text-xs font-medium text-text-secondary uppercase tracking-wider">
                Actions
              </th>
            </tr>
          </thead>
          <tbody className="bg-layer-3 divide-y divide-border">
            {keys.length === 0 ? (
              <tr>
                <td colSpan={6} className="px-6 py-4 text-center text-text-secondary">
                  No API keys created yet
                </td>
              </tr>
            ) : (
              keys.map((key) => (
                <tr key={key.id}>
                  <td className="px-6 py-4 text-text-primary font-medium">{key.name}</td>
                  <td className="px-6 py-4 text-text-secondary font-mono">...{key.last_four}</td>
                  <td className="px-6 py-4 text-text-secondary">
                    {new Date(key.created_at).toLocaleDateString()}
                  </td>
                  <td className="px-6 py-4 text-text-secondary">
                    {key.expires_at ? new Date(key.expires_at).toLocaleDateString() : 'Never'}
                  </td>
                  <td className="px-6 py-4">
                    <span className={`px-2 py-1 rounded text-xs ${key.is_active ? 'bg-green-100 text-green-800' : 'bg-red-100 text-red-800'}`}>
                      {key.is_active ? 'Active' : 'Inactive'}
                    </span>
                  </td>
                  <td className="px-6 py-4">
                    <div className="flex gap-2">
                      {!key.is_active ? (
                        <div className="flex gap-2">
                          <button
                            onClick={() => handleEnable(key.id)}
                            className="px-3 py-1 text-sm bg-green-500 text-white rounded hover:bg-green-600"
                          >
                            Enable
                          </button>
                          <button
                            onClick={() => handleDelete(key.id)}
                            className="px-3 py-1 text-sm bg-red-500 text-white rounded hover:bg-red-600"
                          >
                            Delete
                          </button>
                        </div>
                      ) : (
                        <div className="flex gap-2">
                          <button
                            onClick={() => handleDisable(key.id)}
                            className="px-3 py-1 text-sm bg-yellow-500 text-white rounded hover:bg-yellow-600"
                          >
                            Disable
                          </button>
                          <button
                            onClick={() => handleDelete(key.id)}
                            className="px-3 py-1 text-sm bg-red-500 text-white rounded hover:bg-red-600"
                          >
                            Delete
                          </button>
                        </div>
                      )}
                    </div>
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </div>
  )
}
