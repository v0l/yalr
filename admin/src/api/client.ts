import type {
  Provider,
  Model,
  MetricsResponse,
  HealthResponse,
  ProviderCreateRequest,
  ModelSyncReport,
  ApiKey,
  ApiKeyListItem,
  RouterConfig,
} from '../types'

export const API_BASE_URL = import.meta.env.VITE_API_URL || window.location.origin

async function request<T>(endpoint: string, options?: RequestInit): Promise<T> {
  const response = await fetch(`${API_BASE_URL}${endpoint}`, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...options?.headers,
    },
  })

  if (!response.ok) {
    const error = await response.text().catch(() => 'Unknown error')
    throw new Error(`API Error: ${response.status} - ${error}`)
  }

  return response.json()
}

async function getAuthHeaders(): Promise<HeadersInit> {
  const token = localStorage.getItem('token')
  return token ? { Authorization: `Bearer ${token}` } : {}
}

export const api = {
  async getHealth(): Promise<HealthResponse> {
    return request('/health')
  },

  async checkSetupComplete(): Promise<{ setup_complete: boolean }> {
    return request('/api/setup/status')
  },

  async setupFirstUser(username: string, password: string): Promise<void> {
    return request('/api/auth/setup', {
      method: 'POST',
      body: JSON.stringify({ username, password }),
    })
  },

  async login(credentials: { username: string; password: string }): Promise<{ token: string; username: string; isAdmin: boolean }> {
    const response = await fetch(`${API_BASE_URL}/api/auth/login`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(credentials),
    })
    
    if (!response.ok) {
      const error = await response.text().catch(() => 'Unknown error')
      throw new Error(`Login failed: ${error}`)
    }
    
    return response.json()
  },

  async getProviders(): Promise<{ providers: Provider[] }> {
    const headers = await getAuthHeaders()
    const response = await fetch(`${API_BASE_URL}/api/providers`, { headers })
    
    if (!response.ok) {
      if (response.status === 401) {
        localStorage.removeItem('token')
        window.location.href = '/login'
      }
      throw new Error('Failed to fetch providers')
    }
    
    return response.json()
  },

  async createProvider(data: ProviderCreateRequest): Promise<Provider> {
    const headers = await getAuthHeaders()
    const response = await fetch(`${API_BASE_URL}/api/providers`, {
      method: 'POST',
      headers: { ...headers, 'Content-Type': 'application/json' },
      body: JSON.stringify(data),
    })
    
    if (!response.ok) {
      if (response.status === 401) {
        localStorage.removeItem('token')
        window.location.href = '/login'
      }
      throw new Error('Failed to create provider')
    }
    
    return response.json()
  },

  async deleteProvider(slug: string): Promise<{ deleted: boolean; slug: string }> {
    const headers = await getAuthHeaders()
    const response = await fetch(`${API_BASE_URL}/api/providers/${slug}`, {
      method: 'DELETE',
      headers,
    })
    
    if (!response.ok) {
      if (response.status === 401) {
        localStorage.removeItem('token')
        window.location.href = '/login'
      }
      throw new Error('Failed to delete provider')
    }
    
    return response.json()
  },

  async getModels(): Promise<{ object: string; data: Model[] }> {
    return request('/v1/models')
  },

  async getMetrics(): Promise<MetricsResponse> {
    const headers = await getAuthHeaders()
    const response = await fetch(`${API_BASE_URL}/api/metrics`, { headers })
    
    if (!response.ok) {
      if (response.status === 401) {
        localStorage.removeItem('token')
        window.location.href = '/login'
      }
      throw new Error('Failed to fetch metrics')
    }
    
    return response.json()
  },

  async getApiKeys(): Promise<ApiKeyListItem[]> {
    const headers = await getAuthHeaders()
    const response = await fetch(`${API_BASE_URL}/api/api-keys`, { headers })
    
    if (!response.ok) {
      if (response.status === 401) {
        localStorage.removeItem('token')
        window.location.href = '/login'
      }
      throw new Error('Failed to fetch API keys')
    }
    
    return response.json()
  },

  async createApiKey(name: string, expiresInDays?: number): Promise<ApiKey> {
    const headers = await getAuthHeaders()
    const response = await fetch(`${API_BASE_URL}/api/api-keys`, {
      method: 'POST',
      headers: { ...headers, 'Content-Type': 'application/json' },
      body: JSON.stringify({ name, expires_in_days: expiresInDays }),
    })
    
    if (!response.ok) {
      if (response.status === 401) {
        localStorage.removeItem('token')
        window.location.href = '/login'
      }
      throw new Error('Failed to create API key')
    }
    
    return response.json()
  },

  async deleteApiKey(id: number): Promise<{ deleted: boolean; id: number }> {
    const headers = await getAuthHeaders()
    const response = await fetch(`${API_BASE_URL}/api/api-keys/${id}`, {
      method: 'DELETE',
      headers,
    })
    
    if (!response.ok) {
      if (response.status === 401) {
        localStorage.removeItem('token')
        window.location.href = '/login'
      }
      throw new Error('Failed to delete API key')
    }
    
    return response.json()
  },

  async disableApiKey(id: number): Promise<{ disabled: boolean; id: number }> {
    const headers = await getAuthHeaders()
    const response = await fetch(`${API_BASE_URL}/api/api-keys/${id}/disable`, {
      method: 'POST',
      headers,
    })
    
    if (!response.ok) {
      if (response.status === 401) {
        localStorage.removeItem('token')
        window.location.href = '/login'
      }
      throw new Error('Failed to disable API key')
    }
    
    return response.json()
  },

  async enableApiKey(id: number): Promise<{ enabled: boolean; id: number }> {
    const headers = await getAuthHeaders()
    const response = await fetch(`${API_BASE_URL}/api/api-keys/${id}/enable`, {
      method: 'POST',
      headers,
    })
    
    if (!response.ok) {
      if (response.status === 401) {
        localStorage.removeItem('token')
        window.location.href = '/login'
      }
      throw new Error('Failed to enable API key')
    }
    
    return response.json()
  },

  async syncProviderModels(slug: string): Promise<{
    provider: string
    models: Record<string, unknown>[]
    total_count: number
  }> {
    return request(`/models/sync/${slug}`)
  },

  async detectModelDiscrepancies(models: Record<string, unknown>): Promise<ModelSyncReport[]> {
    return request('/models/discrepancies', {
      method: 'POST',
      body: JSON.stringify({ models }),
    })
  },

  async getRouterConfig(): Promise<RouterConfig> {
    const headers = await getAuthHeaders()
    const response = await fetch(`${API_BASE_URL}/api/config`, { headers })
    
    if (!response.ok) {
      if (response.status === 401) {
        localStorage.removeItem('token')
        window.location.href = '/login'
      }
      throw new Error('Failed to fetch router config')
    }
    
    return response.json()
  },
}
