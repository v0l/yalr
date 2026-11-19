import { API_BASE_URL } from './client'

export interface FiatConfig {
  mode: 'sandbox' | 'prod'
  currencies: string[]
  min_amount: number
  max_amount: number
}

export interface FiatOrder {
  id: number
  public_id: string
  user_id: number
  kind: string
  currency: string
  amount_milli: number
  status: 'pending' | 'completed' | 'failed' | 'cancelled' | 'expired'
  pay_url: string | null
  bolt11: string | null
  expires_at: string | null
  failure_reason: string | null
  created_at: string
  completed_at: string | null
  username?: string
}

export interface SavedCard {
  id: number
  brand: string | null
  last_four: string | null
  exp_month: number | null
  exp_year: number | null
}

export interface AutoReload {
  currency: string
  enabled: boolean
  threshold_milli: number
  amount_milli: number
  monthly_cap_milli: number
  payment_method_id: number
  consecutive_failures: number
  disabled_reason: string | null
}

export interface AutoReloadInput {
  currency: string
  enabled: boolean
  threshold: number
  amount: number
  monthly_cap: number
  payment_method_id: number
}

async function call<T>(path: string, init: RequestInit = {}): Promise<T> {
  const token = localStorage.getItem('token')
  const response = await fetch(`${API_BASE_URL}/api${path}`, {
    ...init,
    headers: {
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...(init.body ? { 'Content-Type': 'application/json' } : {}),
    },
  })
  if (!response.ok) throw new Error((await response.text().catch(() => '')) || `Request failed (${response.status})`)
  return response.status === 204 ? (undefined as T) : response.json()
}

export const fiatApi = {
  config: () => call<FiatConfig>('/account/topup/config'),
  create: (body: { currency: string; amount: number; save_card: boolean; email?: string }) =>
    call<FiatOrder>('/account/topup', { method: 'POST', body: JSON.stringify(body) }),
  status: (id: string) => call<FiatOrder>(`/account/topup/${id}`),
  history: () => call<FiatOrder[]>('/account/topup'),
  adminOrders: () => call<FiatOrder[]>('/payments/topups'),
  cards: () => call<SavedCard[]>('/account/payment-methods'),
  deleteCard: (id: number) => call<void>(`/account/payment-methods/${id}`, { method: 'DELETE' }),
  autoReload: () => call<AutoReload[]>('/account/auto-reload'),
  saveAutoReload: (body: AutoReloadInput) => call<AutoReload>('/account/auto-reload', { method: 'PUT', body: JSON.stringify(body) }),
}
