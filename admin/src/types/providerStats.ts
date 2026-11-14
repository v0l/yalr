export interface ProviderTraffic {
  requests: number
  failures: number
  prompt_tokens: number
  completion_tokens: number
  cached_tokens: number
  avg_ttft_ms: number | null
  output_tokens_per_second: number | null
}

export interface ProviderModelTraffic {
  model: string
  requests: number
  completion_tokens: number
  output_tokens_per_second: number | null
}

export interface ProviderEarning {
  currency: string
  earned_milli: number
  requests: number
}

export interface ProviderStats {
  slug: string
  name: string
  base_url: string
  owner_user_id: number | null
  days: number
  traffic: ProviderTraffic
  models: ProviderModelTraffic[]
  earnings: ProviderEarning[]
}

export interface CurrencyBalance {
  currency: string
  balance_milli: number
  lifetime_deposited_milli: number
}

export interface Balances {
  user_id: number
  balance_msat: number
  lifetime_deposited_msat: number
  currencies: CurrencyBalance[]
}
