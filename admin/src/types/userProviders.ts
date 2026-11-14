export interface MyProvider {
  id: number
  name: string
  slug: string
  base_url: string
  provider_type: string
  has_api_key: boolean
  owner_user_id: number | null
  created_at: string
  updated_at: string
}

export interface MyProviderCreate {
  name: string
  slug: string
  base_url: string
  api_key?: string
  provider_type?: string
}

export interface MyProviderUpdate {
  name?: string
  base_url?: string
  api_key?: string
  provider_type?: string
}

export interface OwnedPrice {
  id: number
  model_name: string
  is_advertised: boolean
  is_free: boolean
  currency: string
  price_per_1m_input: number | null
  price_per_1m_output: number | null
  price_per_1m_cache_read: number | null
  price_per_1m_cache_write: number | null
  price_per_request: number | null
  owner_user_id: number | null
  owner_username: string | null
  provider_id: number | null
  provider_slug: string | null
  created_at: string
  updated_at: string
}

export interface PriceInput {
  currency: string
  price_per_1m_input: number | null
  price_per_1m_output: number | null
  price_per_1m_cache_read: number | null
  price_per_1m_cache_write: number | null
  price_per_request: number | null
}

export interface MyPriceCreate extends PriceInput {
  provider_slug: string
  model_name: string
  is_advertised: boolean
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

export interface CreditCurrencyRequest {
  user_id: number
  currency: string
  amount_milli: number
  reason?: string
}
