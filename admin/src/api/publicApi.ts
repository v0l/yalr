import { API_BASE_URL } from './client'

export interface PublicPrice {
  currency: string
  input_per_1m: number
  output_per_1m: number
}

export interface PublicPriceRange {
  currency: string
  min_input: number
  max_input: number
  min_output: number
  max_output: number
}

export interface PublicModel {
  id: string
  kind: 'routed' | 'pool'
  input_modalities: string[]
  output_modalities: string[]
  context_length: number | null
  max_output_tokens: number | null
  provider_count: number
  price_ranges: PublicPriceRange[]
  requests_24h: number
  cheapest_provider: string | null
  fastest_provider: string | null
}

export interface PublicProvider {
  name: string
  price: PublicPrice | null
  verification: 'verified' | 'reference' | 'unverified'
  uptime_24h: number | null
  uptime_7d: number | null
  requests_24h: number
  error_rate: number | null
  ttft_p50_ms: number | null
  ttft_p95_ms: number | null
  tokens_per_second_p50: number | null
  last_healthy_at_ms: number | null
  listed_since: string
}

export interface PublicModelDetail extends PublicModel {
  providers: PublicProvider[]
}

async function get<T>(path: string): Promise<T> {
  const response = await fetch(`${API_BASE_URL}${path}`)
  if (!response.ok) throw new Error(response.status === 404 ? 'Model not found' : `Request failed (${response.status})`)
  return response.json()
}

export const publicApi = {
  models: (all = false) => get<{ models: PublicModel[] }>(`/api/public/models${all ? '?all=true' : ''}`).then(r => r.models),
  model: (id: string) => get<PublicModelDetail>(`/api/public/models/${id.split('/').map(encodeURIComponent).join('/')}`),
}
