import type { Balances, ProviderStats } from '../types/providerStats'
import type { ApprovalView, CreatedApiKey, MyApiKey, VerificationDetail } from '../types/verification'
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
  verifyProvider: (slug: string) => call<ApprovalView>(`${slugPath(slug)}/verify`, 'POST'),
  verifications: (slug: string) => call<VerificationDetail[]>(`${slugPath(slug)}/verifications`),
  setProviderStatus: (slug: string, action: string, reason?: string) => call<ApprovalView>(`${slugPath(slug)}/status`, 'PUT', { action, reason }),
  setProviderReference: (slug: string, is_reference: boolean) => call<ApprovalView>(`${slugPath(slug)}/reference`, 'PUT', { is_reference }),
  listApiKeys: () => call<MyApiKey[]>('/api-keys'),
  createApiKey: (name: string) => call<CreatedApiKey>('/api-keys', 'POST', { name }),
  deleteApiKey: (id: number) => call<{ deleted: boolean }>(`/api-keys/${id}`, 'DELETE'),
}
