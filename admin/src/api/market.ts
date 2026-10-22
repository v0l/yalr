import { API_BASE_URL } from './client'

export interface PeerPrice {
  node: string
  model: string
  input: number
  output: number
  cache_read: number
  cache_write: number
  request: number
}

export interface MarketResult {
  fetched_at: number
  nodes: number
  nodes_ok: number
  results: PeerPrice[]
}

export interface ListingStatus {
  discovery_enabled: boolean
  provider_id: string | null
  npub: string | null
  relays: string[]
  http_url: string | null
}

async function get<T>(path: string): Promise<T> {
  const token = localStorage.getItem('token')
  const response = await fetch(`${API_BASE_URL}${path}`, { headers: token ? { Authorization: `Bearer ${token}` } : {} })
  if (!response.ok) throw new Error(`Request failed (${response.status})`)
  return response.json()
}

export const marketApi = {
  market: (query: string, refresh = false) =>
    get<MarketResult>(`/api/model-pricing/market?q=${encodeURIComponent(query)}${refresh ? '&refresh=true' : ''}`),
  listing: () => get<ListingStatus>('/api/model-pricing/listing'),
}
