import { useState, useEffect } from 'react'
import { api } from '../api/client'
import type { 
  RoutingConfigFull, 
  RoutingConfigCreateRequest, 
  RoutingConfigUpdateRequest,
  RoutingConfigProviderCreateRequest,
  RoutingConfigProviderUpdateRequest,
  ProviderListItem,
} from '../types'

export default function Config() {
  const [configs, setConfigs] = useState<RoutingConfigFull[]>([])
  const [providers, setProviders] = useState<ProviderListItem[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [successMessage, setSuccessMessage] = useState<string | null>(null)
  
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [editingConfig, setEditingConfig] = useState<RoutingConfigFull | null>(null)
  const [deletingConfig, setDeletingConfig] = useState<RoutingConfigFull | null>(null)
  const [expandingConfig, setExpandingConfig] = useState<number | null>(null)
  
  const [formData, setFormData] = useState<RoutingConfigCreateRequest>({
    name: '',
    strategy: 'round_robin',
    health_check_enabled: true,
    health_check_interval_seconds: 30,
    health_check_timeout_seconds: 10,
  })

  const [providerForm, setProviderForm] = useState<{
    provider_id: number
    model: string
    weight: number
    is_active: boolean
  }>({
    provider_id: 0,
    model: '',
    weight: 1,
    is_active: true,
  })

  const [editingProvider, setEditingProvider] = useState<{
    id: number
    provider_id: number
    model: string | null
    weight: number
    is_active: boolean
    routing_config_id: number
  } | null>(null)

  const [deletingProvider, setDeletingProvider] = useState<{
    id: number
    routing_config_id: number
    provider_name: string
  } | null>(null)

  useEffect(() => {
    loadData()
  }, [])

  async function loadData() {
    try {
      setLoading(true)
      const [configsData, providersData] = await Promise.all([
        api.getRoutingConfigs(),
        api.getProvidersList(),
      ])
      setConfigs(configsData)
      setProviders(providersData)
      setError(null)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load data')
    } finally {
      setLoading(false)
    }
  }

  function handleCreateClick() {
    setFormData({
      name: '',
      strategy: 'round_robin',
      health_check_enabled: true,
      health_check_interval_seconds: 30,
      health_check_timeout_seconds: 10,
    })
    setShowCreateModal(true)
  }

  function handleEditClick(config: RoutingConfigFull) {
    setEditingConfig({
      ...config,
      name: config.name,
      strategy: config.strategy,
      health_check_enabled: config.health_check_enabled,
      health_check_interval_seconds: config.health_check_interval_seconds,
      health_check_timeout_seconds: config.health_check_timeout_seconds,
    })
    setShowCreateModal(true)
  }

  function handleDeleteClick(config: RoutingConfigFull) {
    setDeletingConfig(config)
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    
    try {
      if (editingConfig) {
        const updateData: RoutingConfigUpdateRequest = {
          name: formData.name,
          strategy: formData.strategy,
          health_check_enabled: formData.health_check_enabled,
          health_check_interval_seconds: formData.health_check_interval_seconds,
          health_check_timeout_seconds: formData.health_check_timeout_seconds,
        }
        await api.updateRoutingConfig(editingConfig.id, updateData)
        setSuccessMessage('Routing config updated successfully')
      } else {
        await api.createRoutingConfig(formData)
        setSuccessMessage('Routing config created successfully')
      }
      setShowCreateModal(false)
      loadData()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save routing config')
    }
  }

  async function handleDeleteConfirm() {
    if (!deletingConfig) return
    
    try {
      await api.deleteRoutingConfig(deletingConfig.id)
      setDeletingConfig(null)
      setSuccessMessage('Routing config deleted successfully')
      loadData()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to delete routing config')
    }
  }

  function handleAddProviderClick(config: RoutingConfigFull) {
    setExpandingConfig(config.id)
    setProviderForm({
      provider_id: 0,
      model: '',
      weight: 1,
      is_active: true,
    })
  }

  async function handleAddProviderSubmit(config: RoutingConfigFull) {
    if (providerForm.provider_id === 0) {
      setError('Please select a provider')
      return
    }

    try {
      const data: RoutingConfigProviderCreateRequest = {
        routing_config_id: config.id,
        provider_id: providerForm.provider_id,
        model: providerForm.model || null,
        weight: providerForm.weight,
        is_active: providerForm.is_active,
      }
      await api.addProviderToConfig(data)
      setSuccessMessage('Provider added successfully')
      loadData()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to add provider')
    }
  }

  function handleEditProviderClick(config: RoutingConfigFull, provider: RoutingConfigFull['providers'][0]) {
    setExpandingConfig(config.id)
    setEditingProvider({
      id: provider.id,
      provider_id: provider.provider_id,
      model: provider.model,
      weight: provider.weight,
      is_active: provider.is_active,
      routing_config_id: provider.routing_config_id,
    })
    setProviderForm({
      provider_id: provider.provider_id,
      model: provider.model || '',
      weight: provider.weight,
      is_active: provider.is_active,
    })
  }

  async function handleUpdateProviderSubmit() {
    if (!editingProvider) return

    try {
      const data: RoutingConfigProviderUpdateRequest = {
        model: providerForm.model || null,
        weight: providerForm.weight,
        is_active: providerForm.is_active,
      }
      await api.updateProviderInConfig(editingProvider.id, data)
      setSuccessMessage('Provider updated successfully')
      setEditingProvider(null)
      loadData()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to update provider')
    }
  }

  function handleDeleteProviderClick(config: RoutingConfigFull, provider: RoutingConfigFull['providers'][0]) {
    setDeletingProvider({
      id: provider.id,
      routing_config_id: config.id,
      provider_name: provider.provider_name,
    })
  }

  async function handleDeleteProviderConfirm() {
    if (!deletingProvider) return

    try {
      await api.deleteProviderFromConfig(deletingProvider.id)
      setDeletingProvider(null)
      setSuccessMessage('Provider removed successfully')
      loadData()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to delete provider')
    }
  }

  function handleClearSuccess() {
    setSuccessMessage(null)
  }

  function handleClearError() {
    setError(null)
  }

  if (loading) {
    return (
      <div className="p-6">
        <div className="animate-pulse space-y-4">
          <div className="h-8 bg-layer-2 rounded w-1/4"></div>
          <div className="h-4 bg-layer-2 rounded w-1/2"></div>
          <div className="h-64 bg-layer-2 rounded"></div>
        </div>
      </div>
    )
  }

  return (
    <div className="p-6 space-y-6">
      <div className="flex justify-between items-center">
        <div>
          <h1 className="text-2xl font-bold text-text-primary">Routing Configurations</h1>
          <p className="text-sm text-text-secondary mt-1">
            Manage routing engines and their provider assignments
          </p>
        </div>
        <button
          onClick={handleCreateClick}
          className="px-4 py-2 bg-accent text-white rounded hover:bg-accent-hover transition-colors"
        >
          + Add Routing Config
        </button>
      </div>

      {successMessage && (
        <div className="bg-green-500/10 border border-green-500/50 rounded-lg p-4 flex justify-between items-center">
          <p className="text-green-500">{successMessage}</p>
          <button onClick={handleClearSuccess} className="text-green-500 hover:text-green-400">
            ×
          </button>
        </div>
      )}

      {error && (
        <div className="bg-red-500/10 border border-red-500/50 rounded-lg p-4 flex justify-between items-center">
          <p className="text-red-500">{error}</p>
          <button onClick={handleClearError} className="text-red-500 hover:text-red-400">
            ×
          </button>
        </div>
      )}

      {configs.length === 0 ? (
        <div className="bg-layer-1 border border-border rounded-lg p-12 text-center">
          <p className="text-text-secondary mb-4">No routing configurations found</p>
          <button
            onClick={handleCreateClick}
            className="px-4 py-2 bg-accent text-white rounded hover:bg-accent-hover transition-colors"
          >
            Create Your First Routing Config
          </button>
        </div>
      ) : (
        <div className="space-y-4">
          {configs.map((config) => (
            <div key={config.id} className="bg-layer-1 border border-border rounded-lg overflow-hidden">
              <div className="p-6">
                <div className="flex items-center justify-between mb-4">
                  <div>
                    <h2 className="text-xl font-semibold text-text-primary">{config.name}</h2>
                    <div className="flex items-center gap-3 mt-1">
                      <span className="text-sm text-text-secondary">
                        Strategy: <span className="font-mono text-accent">{config.strategy}</span>
                      </span>
                      <span className="text-sm text-text-secondary">
                        Providers: <span className="font-medium">{config.providers.length}</span>
                      </span>
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <button
                      onClick={() => handleEditClick(config)}
                      className="px-3 py-1 text-sm bg-layer-2 text-text-primary rounded hover:bg-layer-3 border border-border transition-colors"
                    >
                      Edit
                    </button>
                    <button
                      onClick={() => handleDeleteClick(config)}
                      className="px-3 py-1 text-sm bg-red-500/20 text-red-500 rounded hover:bg-red-500/30 transition-colors"
                    >
                      Delete
                    </button>
                  </div>
                </div>

                <div className="flex items-center gap-4 text-sm">
                  <span className="text-text-secondary">
                    Health Check: {config.health_check_enabled ? 'Enabled' : 'Disabled'}
                  </span>
                  <span className="text-text-secondary">
                    Interval: {config.health_check_interval_seconds}s
                  </span>
                  <span className="text-text-secondary">
                    Timeout: {config.health_check_timeout_seconds}s
                  </span>
                </div>
              </div>

              <div className="border-t border-border">
                <div className="p-4 bg-layer-2">
                  <div className="flex justify-between items-center mb-4">
                    <h3 className="text-sm font-semibold text-text-secondary uppercase tracking-wider">
                      Provider Assignments
                    </h3>
                    <button
                      onClick={() => handleAddProviderClick(config)}
                      className="px-3 py-1 text-sm bg-accent/20 text-accent rounded hover:bg-accent/30 transition-colors"
                    >
                      + Add Provider
                    </button>
                  </div>

                  {expandingConfig === config.id && (
                    <div className="mb-4 p-4 bg-layer-1 rounded-lg border border-border">
                      <h4 className="text-sm font-medium text-text-primary mb-3">
                        {editingProvider ? 'Edit Provider' : 'Add New Provider'}
                      </h4>
                      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                        <div>
                          <label className="block text-sm text-text-secondary mb-1">
                            Provider
                          </label>
                          <select
                            value={providerForm.provider_id}
                            onChange={(e) => setProviderForm({ ...providerForm, provider_id: Number(e.target.value) })}
                            className="w-full px-3 py-2 bg-layer-2 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                          >
                            <option value={0}>Select a provider...</option>
                            {providers.map((p) => (
                              <option key={p.id} value={p.id}>
                                {p.name} ({p.slug})
                              </option>
                            ))}
                          </select>
                        </div>
                        <div>
                          <label className="block text-sm text-text-secondary mb-1">
                            Model (optional)
                          </label>
                          <input
                            type="text"
                            value={providerForm.model}
                            onChange={(e) => setProviderForm({ ...providerForm, model: e.target.value })}
                            placeholder="e.g., gpt-4"
                            className="w-full px-3 py-2 bg-layer-2 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                          />
                        </div>
                        <div>
                          <label className="block text-sm text-text-secondary mb-1">
                            Weight
                          </label>
                          <input
                            type="number"
                            min="1"
                            value={providerForm.weight}
                            onChange={(e) => setProviderForm({ ...providerForm, weight: Number(e.target.value) })}
                            className="w-full px-3 py-2 bg-layer-2 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                          />
                        </div>
                        <div className="flex items-center">
                          <input
                            type="checkbox"
                            id={`is_active_${config.id}_${editingProvider ? editingProvider.id : 'new'}`}
                            checked={providerForm.is_active}
                            onChange={(e) => setProviderForm({ ...providerForm, is_active: e.target.checked })}
                            className="mr-2"
                          />
                          <label htmlFor={`is_active_${config.id}_${editingProvider ? editingProvider.id : 'new'}`} className="text-sm text-text-secondary">
                            Active
                          </label>
                        </div>
                      </div>
                      <div className="flex justify-end gap-2 mt-4">
                        <button
                          onClick={() => {
                            setExpandingConfig(null)
                            setEditingProvider(null)
                          }}
                          className="px-3 py-1 text-sm text-text-secondary hover:text-text-primary transition-colors"
                        >
                          Cancel
                        </button>
                          <button
                            onClick={() => editingProvider ? handleUpdateProviderSubmit() : handleAddProviderSubmit(config)}
                            className="px-3 py-1 text-sm bg-accent text-white rounded hover:bg-accent-hover transition-colors"
                          >
                            {editingProvider ? 'Update' : 'Add'}
                          </button>
                      </div>
                    </div>
                  )}

                  {config.providers.length === 0 ? (
                    <div className="text-center py-8 text-text-secondary">
                      No providers assigned to this routing config
                    </div>
                  ) : (
                    <div className="space-y-2">
                      {config.providers.map((provider) => (
                        <div
                          key={provider.id}
                          className="p-3 bg-layer-1 rounded border border-border hover:border-accent/30 transition-colors"
                        >
                          <div className="flex items-center justify-between">
                            <div className="flex items-center gap-4">
                              <div>
                                <div className="font-medium text-text-primary">
                                  {provider.provider_name}
                                </div>
                                <div className="text-sm text-text-secondary font-mono">
                                  {provider.provider_slug}
                                  {provider.model && (
                                    <span className="ml-2 text-accent">→ {provider.model}</span>
                                  )}
                                </div>
                              </div>
                              <div className="flex items-center gap-3 text-sm">
                                <span className="text-text-secondary">
                                  Weight: <span className="font-medium">{provider.weight}</span>
                                </span>
                                <span className={`px-2 py-0.5 rounded text-xs ${
                                  provider.is_active 
                                    ? 'bg-green-500/20 text-green-500' 
                                    : 'bg-gray-500/20 text-gray-500'
                                }`}>
                                  {provider.is_active ? 'Active' : 'Inactive'}
                                </span>
                              </div>
                            </div>
                            <div className="flex items-center gap-2">
                              <button
                                onClick={() => handleEditProviderClick(config, provider)}
                                className="px-2 py-1 text-xs bg-layer-2 text-text-secondary rounded hover:bg-layer-3 border border-border transition-colors"
                              >
                                Edit
                              </button>
                              <button
                                onClick={() => handleDeleteProviderClick(config, provider)}
                                className="px-2 py-1 text-xs bg-red-500/20 text-red-500 rounded hover:bg-red-500/30 transition-colors"
                              >
                                Delete
                              </button>
                            </div>
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Create/Edit Modal */}
      {showCreateModal && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-layer-3 rounded-lg p-6 w-full max-w-md border border-border">
            <h2 className="text-xl font-bold mb-4 text-text-primary">
              {editingConfig ? 'Edit Routing Config' : 'Create Routing Config'}
            </h2>
            
            <form onSubmit={handleSubmit}>
              <div className="space-y-4">
                <div>
                  <label className="block text-sm font-medium text-text-secondary mb-1">
                    Name
                  </label>
                  <input
                    type="text"
                    value={formData.name}
                    onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                    className="w-full px-3 py-2 bg-layer-1 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                    placeholder="e.g., production, staging"
                    required
                  />
                </div>
                
                <div>
                  <label className="block text-sm font-medium text-text-secondary mb-1">
                    Strategy
                  </label>
                  <select
                    value={formData.strategy}
                    onChange={(e) => setFormData({ ...formData, strategy: e.target.value })}
                    className="w-full px-3 py-2 bg-layer-1 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                  >
                    <option value="round_robin">Round Robin</option>
                    <option value="least_loaded">Least Loaded</option>
                    <option value="random">Random</option>
                  </select>
                </div>

                <div className="flex items-center">
                  <input
                    type="checkbox"
                    id="health_check_enabled"
                    checked={formData.health_check_enabled}
                    onChange={(e) => setFormData({ ...formData, health_check_enabled: e.target.checked })}
                    className="mr-2"
                  />
                  <label htmlFor="health_check_enabled" className="text-sm font-medium text-text-secondary">
                    Enable health checks
                  </label>
                </div>

                {formData.health_check_enabled && (
                  <div className="grid grid-cols-2 gap-4">
                    <div>
                      <label className="block text-sm font-medium text-text-secondary mb-1">
                        Interval (seconds)
                      </label>
                      <input
                        type="number"
                        min="1"
                        value={formData.health_check_interval_seconds}
                        onChange={(e) => setFormData({ ...formData, health_check_interval_seconds: Number(e.target.value) })}
                        className="w-full px-3 py-2 bg-layer-1 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                      />
                    </div>
                    <div>
                      <label className="block text-sm font-medium text-text-secondary mb-1">
                        Timeout (seconds)
                      </label>
                      <input
                        type="number"
                        min="1"
                        value={formData.health_check_timeout_seconds}
                        onChange={(e) => setFormData({ ...formData, health_check_timeout_seconds: Number(e.target.value) })}
                        className="w-full px-3 py-2 bg-layer-1 border border-border rounded text-text-primary focus:outline-none focus:ring-2 focus:ring-accent"
                      />
                    </div>
                  </div>
                )}
              </div>

              <div className="flex justify-end gap-3 mt-6">
                <button
                  type="button"
                  onClick={() => {
                    setShowCreateModal(false)
                    setEditingConfig(null)
                  }}
                  className="px-4 py-2 text-text-secondary hover:text-text-primary transition-colors"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="px-4 py-2 bg-accent text-white rounded hover:bg-accent-hover transition-colors"
                >
                  {editingConfig ? 'Update' : 'Create'}
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Delete Config Confirmation Modal */}
      {deletingConfig && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-layer-3 rounded-lg p-6 w-full max-w-md border border-border">
            <h2 className="text-xl font-bold mb-4 text-text-primary">Delete Routing Config</h2>
            <p className="text-text-secondary mb-6">
              Are you sure you want to delete routing config "{deletingConfig.name}"? 
              This will also remove all provider assignments. This action cannot be undone.
            </p>
            <div className="flex justify-end gap-3">
              <button
                onClick={() => setDeletingConfig(null)}
                className="px-4 py-2 text-text-secondary hover:text-text-primary transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleDeleteConfirm}
                className="px-4 py-2 bg-red-600 text-white rounded hover:bg-red-700 transition-colors"
              >
                Delete
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Delete Provider Confirmation Modal */}
      {deletingProvider && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-layer-3 rounded-lg p-6 w-full max-w-md border border-border">
            <h2 className="text-xl font-bold mb-4 text-text-primary">Remove Provider</h2>
            <p className="text-text-secondary mb-6">
              Are you sure you want to remove provider "{deletingProvider.provider_name}" from this routing config?
            </p>
            <div className="flex justify-end gap-3">
              <button
                onClick={() => setDeletingProvider(null)}
                className="px-4 py-2 text-text-secondary hover:text-text-primary transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleDeleteProviderConfirm}
                className="px-4 py-2 bg-red-600 text-white rounded hover:bg-red-700 transition-colors"
              >
                Remove
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
