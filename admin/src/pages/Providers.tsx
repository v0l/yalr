import { useEffect, useState } from 'react'
import { api } from '../api/client'
import type { Provider } from '../types'

export default function Providers() {
  const [providers, setProviders] = useState<Provider[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [formData, setFormData] = useState({
    name: '',
    slug: '',
    base_url: '',
    api_key: '',
    provider_type: 'openai',
  })
  const [editingProvider, setEditingProvider] = useState<Provider | null>(null)
  const [editFormData, setEditFormData] = useState({
    name: '',
    slug: '',
    base_url: '',
    api_key: '',
    provider_type: 'openai',
  })

  useEffect(() => {
    loadProviders()
  }, [])

  async function loadProviders() {
    try {
      const data = await api.getProviders()
      setProviders(data.providers)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch providers')
    } finally {
      setLoading(false)
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    try {
      await api.createProvider(formData)
      setFormData({ name: '', slug: '', base_url: '', api_key: '', provider_type: 'openai' })
      loadProviders()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to create provider')
    }
  }

  async function handleDelete(slug: string) {
    try {
      await api.deleteProvider(slug)
      loadProviders()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to delete provider')
    }
  }

  function startEdit(provider: Provider) {
    setEditingProvider(provider)
    setEditFormData({
      name: provider.name,
      slug: provider.slug,
      base_url: provider.base_url,
      api_key: '',
      provider_type: provider.provider_type,
    })
  }

  function cancelEdit() {
    setEditingProvider(null)
    setEditFormData({
      name: '',
      slug: '',
      base_url: '',
      api_key: '',
      provider_type: 'openai',
    })
  }

  async function handleUpdate(e: React.FormEvent) {
    e.preventDefault()
    if (!editingProvider) return
    try {
      const updateData: Record<string, string> = {}
      if (editFormData.name !== editingProvider.name) updateData.name = editFormData.name
      if (editFormData.slug !== editingProvider.slug) updateData.slug = editFormData.slug
      if (editFormData.base_url !== editingProvider.base_url) updateData.base_url = editFormData.base_url
      if (editFormData.api_key) updateData.api_key = editFormData.api_key
      if (editFormData.provider_type !== editingProvider.provider_type) updateData.provider_type = editFormData.provider_type

      await api.updateProvider(editingProvider.slug, updateData)
      setEditingProvider(null)
      loadProviders()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to update provider')
    }
  }

  if (loading) {
    return (
      <div className="p-8">
        <h1 className="text-2xl font-bold mb-6 text-text-primary">Providers</h1>
        <p className="text-text-secondary">Loading...</p>
      </div>
    )
  }

  return (
    <div className="p-8">
      <h1 className="text-2xl font-bold mb-6 text-text-primary">Providers</h1>

      {error && (
        <div className="mb-6 p-4 bg-red-100 border border-red-400 text-red-700 rounded">
          Error: {error}
        </div>
      )}

      <form onSubmit={handleSubmit} className="mb-8 p-6 bg-layer-3 rounded-lg border border-border">
        <h2 className="text-lg font-semibold mb-4 text-text-primary">Add Provider</h2>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">Name</label>
            <input
              type="text"
              value={formData.name}
              onChange={(e) => setFormData({ ...formData, name: e.target.value })}
              className="w-full px-3 py-2 bg-layer-4 border border-border rounded text-text-primary"
              required
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">Slug</label>
            <input
              type="text"
              value={formData.slug}
              onChange={(e) => setFormData({ ...formData, slug: e.target.value })}
              className="w-full px-3 py-2 bg-layer-4 border border-border rounded text-text-primary"
              required
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">Provider Type</label>
            <select
              value={formData.provider_type}
              onChange={(e) => setFormData({ ...formData, provider_type: e.target.value })}
              className="w-full px-3 py-2 bg-layer-4 border border-border rounded text-text-primary"
              required
            >
              <option value="openai">OpenAI</option>
              <option value="llamacpp">LlamaCpp</option>
              <option value="vllm">vLLM</option>
              <option value="ollama">Ollama</option>
            </select>
          </div>
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">Base URL</label>
            <input
              type="url"
              value={formData.base_url}
              onChange={(e) => setFormData({ ...formData, base_url: e.target.value })}
              className="w-full px-3 py-2 bg-layer-4 border border-border rounded text-text-primary"
              required
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-text-secondary mb-1">API Key</label>
            <input
              type="password"
              value={formData.api_key}
              onChange={(e) => setFormData({ ...formData, api_key: e.target.value })}
              className="w-full px-3 py-2 bg-layer-4 border border-border rounded text-text-primary"
              required
            />
          </div>
        </div>
        <button
          type="submit"
          className="mt-4 px-4 py-2 bg-accent text-white rounded hover:bg-accent-hover"
        >
          Add Provider
        </button>
      </form>

      {editingProvider && (
        <form onSubmit={handleUpdate} className="mb-8 p-6 bg-layer-3 rounded-lg border border-border border-yellow-500">
          <h2 className="text-lg font-semibold mb-4 text-text-primary">Edit Provider</h2>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div>
              <label className="block text-sm font-medium text-text-secondary mb-1">Name</label>
              <input
                type="text"
                value={editFormData.name}
                onChange={(e) => setEditFormData({ ...editFormData, name: e.target.value })}
                className="w-full px-3 py-2 bg-layer-4 border border-border rounded text-text-primary"
                required
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-text-secondary mb-1">Slug</label>
              <input
                type="text"
                value={editFormData.slug}
                onChange={(e) => setEditFormData({ ...editFormData, slug: e.target.value })}
                className="w-full px-3 py-2 bg-layer-4 border border-border rounded text-text-primary"
                required
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-text-secondary mb-1">Provider Type</label>
              <select
                value={editFormData.provider_type}
                onChange={(e) => setEditFormData({ ...editFormData, provider_type: e.target.value })}
                className="w-full px-3 py-2 bg-layer-4 border border-border rounded text-text-primary"
                required
              >
                <option value="openai">OpenAI</option>
                <option value="llamacpp">LlamaCpp</option>
                <option value="vllm">vLLM</option>
                <option value="ollama">Ollama</option>
              </select>
            </div>
            <div>
              <label className="block text-sm font-medium text-text-secondary mb-1">Base URL</label>
              <input
                type="url"
                value={editFormData.base_url}
                onChange={(e) => setEditFormData({ ...editFormData, base_url: e.target.value })}
                className="w-full px-3 py-2 bg-layer-4 border border-border rounded text-text-primary"
                required
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-text-secondary mb-1">API Key (leave blank to keep current)</label>
              <input
                type="password"
                value={editFormData.api_key}
                onChange={(e) => setEditFormData({ ...editFormData, api_key: e.target.value })}
                className="w-full px-3 py-2 bg-layer-4 border border-border rounded text-text-primary"
                placeholder="Leave blank to keep current API key"
              />
            </div>
          </div>
          <div className="mt-4 flex gap-2">
            <button
              type="submit"
              className="px-4 py-2 bg-accent text-white rounded hover:bg-accent-hover"
            >
              Update Provider
            </button>
            <button
              type="button"
              onClick={cancelEdit}
              className="px-4 py-2 bg-gray-500 text-white rounded hover:bg-gray-600"
            >
              Cancel
            </button>
          </div>
        </form>
      )}

      <div className="bg-layer-3 rounded-lg border border-border overflow-hidden">
        <table className="min-w-full divide-y divide-border">
          <thead className="bg-layer-2">
            <tr>
              <th className="px-6 py-3 text-left text-xs font-medium text-text-secondary uppercase tracking-wider">
                Name
              </th>
              <th className="px-6 py-3 text-left text-xs font-medium text-text-secondary uppercase tracking-wider">
                Slug
              </th>
              <th className="px-6 py-3 text-left text-xs font-medium text-text-secondary uppercase tracking-wider">
                Type
              </th>
              <th className="px-6 py-3 text-left text-xs font-medium text-text-secondary uppercase tracking-wider">
                Base URL
              </th>
              <th className="px-6 py-3 text-left text-xs font-medium text-text-secondary uppercase tracking-wider">
                Actions
              </th>
            </tr>
          </thead>
          <tbody className="bg-layer-3 divide-y divide-border">
            {providers.length === 0 ? (
              <tr>
                <td colSpan={5} className="px-6 py-4 text-center text-text-secondary">
                  No providers configured
                </td>
              </tr>
            ) : (
              providers.map((provider) => (
                <tr key={provider.slug}>
                  <td className="px-6 py-4 text-text-primary">{provider.name}</td>
                  <td className="px-6 py-4 text-text-secondary">{provider.slug}</td>
                  <td className="px-6 py-4 text-text-secondary">{provider.provider_type}</td>
                  <td className="px-6 py-4 text-text-secondary">{provider.base_url}</td>
                  <td className="px-6 py-4">
                    <button
                      onClick={() => startEdit(provider)}
                      className="px-3 py-1 text-sm bg-blue-500 text-white rounded hover:bg-blue-600 mr-2"
                    >
                      Edit
                    </button>
                    <button
                      onClick={() => handleDelete(provider.slug)}
                      className="px-3 py-1 text-sm bg-red-500 text-white rounded hover:bg-red-600"
                    >
                      Delete
                    </button>
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
