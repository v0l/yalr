import type { RequestLogSummary, UsageReport } from '@/types'
import { API_BASE_URL } from './client'

export interface AccountHistoryEntry {
  id: number
  kind: 'deposit' | 'usage' | 'withdrawal' | 'adjustment'
  amount_msat: number
  detail: string | null
  created_at: string
}

export interface AccountInvoice {
  invoice_id: string
  purpose: string
  amount_sats: number
  status: string
  created_at: string
  paid_at: string | null
}

export interface AccountData {
  balance_msat: number
  lifetime_deposited_msat: number
  history: AccountHistoryEntry[]
  invoices: AccountInvoice[]
}

export interface CreatedInvoice {
  invoice_id: string
  bolt11: string
  amount_sats: number
  expires_at: number
  payment_hash: string
}

export interface InvoiceStatus {
  status: string
  api_key: string | null
  amount_sats: number
  paid_at: number | null
  created_at: number
  expires_at: number
}

const KEY_STORAGE = 'account_key'

export const accountKey = {
  get: () => localStorage.getItem(KEY_STORAGE),
  set: (key: string) => localStorage.setItem(KEY_STORAGE, key),
  clear: () => localStorage.removeItem(KEY_STORAGE),
}

export class AccountAuthError extends Error {}

async function call<T>(path: string, init?: RequestInit, key?: string | null): Promise<T> {
  const response = await fetch(`${API_BASE_URL}${path}`, {
    ...init,
    headers: {
      'Content-Type': 'application/json',
      ...(key ? { Authorization: `Bearer ${key}` } : {}),
    },
  })
  if (response.status === 401) throw new AccountAuthError('API key not recognised')
  if (!response.ok) {
    const body = await response.json().catch(() => null)
    throw new Error(body?.error ?? `Request failed (${response.status})`)
  }
  return response.json()
}

export const accountApi = {
  usage: (key: string, hours: number) => call<UsageReport>(`/v1/usage?hours=${hours}`, undefined, key),
  requests: (key: string, hours: number, before?: number) =>
    call<RequestLogSummary[]>(`/v1/usage/requests?hours=${hours}&limit=50${before ? `&before=${before}` : ''}`, undefined, key),
  load: (key: string) => call<AccountData>('/v1/account', undefined, key),
  createInvoice: (amount_sats: number, purpose: 'create' | 'topup', key?: string) =>
    call<CreatedInvoice>('/v2/lightning/invoice', { method: 'POST', body: JSON.stringify({ amount_sats, purpose }) }, key),
  invoiceStatus: (id: string) => call<InvoiceStatus>(`/v2/lightning/invoice/${encodeURIComponent(id)}/status`),
}
