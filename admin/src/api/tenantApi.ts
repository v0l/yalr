import type { Balances, ProviderStats } from '../types/providerStats'
import { API_BASE_URL } from './client'

async function call<T>(path: string, method = 'GET', body?: unknown): Promise<T> {
  const token = localStorage.getItem('token')
  const response = await fetch(`${API_BASE_URL}/api${path}`, {
    method,
    headers: {
      ...(body !== undefined ? { 'Content-Type': 'application/json' } : {}),
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  })
  if (!response.ok) {
    const text = await response.text().catch(() => '')
    throw new Error(text || `Request failed (${response.status})`)
  }
  return response.json()
}

const slugPath = (slug: string) => `/providers/${encodeURIComponent(slug)}`

export const tenantApi = {
  providerStats: (slug: string, days: number) => call<ProviderStats>(`${slugPath(slug)}/stats?days=${days}`),
  checkProvider: (slug: string) => call<{ healthy: boolean; error: string | null }>(`${slugPath(slug)}/check`, 'POST'),
  myBalances: () => call<Balances>('/my/balances'),
}
