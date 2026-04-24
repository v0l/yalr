export interface Provider {
  id: number
  name: string
  slug: string
  base_url: string
  provider_type: string
  created_at: string
  updated_at: string
}

export interface Model {
  id: string
  object: string
  created: number
  owned_by: string
}

export interface ProviderMetricsSummary {
  provider: string
  p90_tokens_per_second: number | null
  p90_ttft_ms: number | null
  avg_latency_ms: number | null
  success_rate: number | null
}

export interface MetricsResponse {
  providers: ProviderMetricsSummary[]
  recent_events: Record<string, unknown>[]
}

// WebSocket real-time metrics event types (matching Rust backend)

export interface WsProviderMetrics {
  provider: string
  model: string
  timestamp_ms: number
  event: WsMetricsEvent
}

export type WsMetricsEvent =
  | { TTFT: number }
  | { OutputTokensPerSecond: number }
  | { InputTokensPerSecond: number }
  | { TotalLatency: number }
  | { InputTokens: number }
  | { OutputTokens: number }
  | 'Success'
  | { Failure: WsFailureDetails }
  | { ProviderLoad: { in_flight: number; max_concurrency: number | null } }

export interface WsFailureDetails {
  error_type: 'RateLimit' | 'ServerError' | 'Timeout' | 'Authentication' | 'NotFound' | 'Other'
  error_code: string | null
  error_message: string
  retry_after_ms: number | null
  status_code: number | null
}

export interface WsLagMessage {
  type: 'lag'
  skipped: number
}

export interface HealthResponse {
  status: string
  timestamp: number
}

export interface ProviderCreateRequest {
  name: string
  slug: string
  base_url: string
  api_key: string
  provider_type: string
}

export interface ProviderUpdateRequest {
  name?: string
  slug?: string
  base_url?: string
  api_key?: string
  provider_type?: string
}

export interface ModelSyncReport {
  model_name: string
  provider_name: string
  discrepancies: ModelDiscrepancy[]
  is_synced: boolean
}

export interface ModelDiscrepancy {
  field: string
  database_value: string | null
  api_value: string | null
  severity: 'info' | 'warning' | 'error'
}

export interface ApiKey {
  id: number
  name: string
  key?: string
  last_four: string
  created_at: string
  expires_at: string | null
  is_active?: boolean
}

export interface ApiKeyListItem {
  id: number
  name: string
  last_four: string
  created_at: string
  expires_at: string | null
  is_active: boolean
}

export interface ProviderMetrics {
  p90_ttft_ms: number | null
  p90_output_tokens_per_second: number | null
  p90_input_tokens_per_second: number | null
  avg_latency_ms: number | null
  success_rate: number | null
}

export interface RouterConfigProvider {
  name: string
  slug: string
  base_url: string
  list_url: string
  metrics: ProviderMetrics
}

export interface RoutingConfig {
  name: string
  strategy: string
  providers: RouterConfigProvider[]
  provider_count: number
}

export interface RouterConfig {
  routing_configs: RoutingConfig[]
}

export interface RoutingConfigProvider {
  id: number
  routing_config_id: number
  provider_id: number
  provider_name: string
  provider_slug: string
  model: string | null
  weight: number
  is_active: boolean
  created_at: string
  updated_at: string
}

export interface RoutingConfigFull {
  id: number
  name: string
  strategy: string
  health_check_enabled: boolean
  health_check_interval_seconds: number
  health_check_timeout_seconds: number
  created_at: string
  updated_at: string
  providers: RoutingConfigProvider[]
}

export interface RoutingConfigCreateRequest {
  name: string
  strategy: string
  health_check_enabled: boolean
  health_check_interval_seconds: number
  health_check_timeout_seconds: number
}

export interface RoutingConfigUpdateRequest {
  name?: string
  strategy?: string
  health_check_enabled?: boolean
  health_check_interval_seconds?: number
  health_check_timeout_seconds?: number
}

export interface RoutingConfigProviderCreateRequest {
  routing_config_id: number
  provider_id: number
  model: string | null
  weight: number
  is_active: boolean
}

export interface RoutingConfigProviderUpdateRequest {
  model?: string | null
  weight?: number
  is_active?: boolean
}

export interface ProviderListItem {
  id: number
  name: string
  slug: string
  base_url: string
}

export interface User {
  id: number
  username: string | null
  external_id: string | null
  user_type: 'internal' | 'nostr' | 'oauth'
  is_admin: boolean
  created_at: string
  updated_at: string
}

export interface CreateUserRequest {
  username?: string
  password?: string
  external_id?: string
  user_type: 'internal' | 'nostr' | 'oauth'
  is_admin: boolean
}

export interface UpdateUserRequest {
  username?: string
  password?: string
  is_admin?: boolean
}

export interface UserApiKeyListItem {
  id: number
  name: string
  last_four: string
  created_at: string
  expires_at: string | null
  is_active: boolean
}

export interface UserDetailResponse {
  user: User
  api_keys: UserApiKeyListItem[]
}
