import type {
  Balances,
  CreditCurrencyRequest,
  MyPriceCreate,
  MyProvider,
  MyProviderCreate,
  MyProviderUpdate,
  OwnedPrice,
  PriceInput,
} from '../types/userProviders'
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

export const userApi = {
  listProviders: () => call<MyProvider[]>('/my/providers'),
  createProvider: (data: MyProviderCreate) => call<MyProvider>('/my/providers', 'POST', data),
  updateProvider: (slug: string, data: MyProviderUpdate) => call<MyProvider>(`/my/providers/${encodeURIComponent(slug)}`, 'PUT', data),
  deleteProvider: (slug: string) => call<boolean>(`/my/providers/${encodeURIComponent(slug)}`, 'DELETE'),
  checkProvider: (slug: string) => call<{ healthy: boolean; error: string | null }>(`/my/providers/${encodeURIComponent(slug)}/health`, 'POST'),
  providerModels: (slug: string) => call<{ models: string[] }>(`/my/providers/${encodeURIComponent(slug)}/models`),
  listPrices: () => call<OwnedPrice[]>('/my/pricing'),
  createPrice: (data: MyPriceCreate) => call<OwnedPrice>('/my/pricing', 'POST', data),
  updatePrice: (id: number, data: PriceInput) => call<OwnedPrice>(`/my/pricing/${id}`, 'PUT', data),
  deletePrice: (id: number) => call<boolean>(`/my/pricing/${id}`, 'DELETE'),
  myBalances: () => call<Balances>('/my/balances'),
  creditCurrency: (data: CreditCurrencyRequest) =>
    call<{ user_id: number; currency: string; new_balance_milli: number }>('/payments/credit-currency', 'POST', data),
}
