export interface Provider {
  id: number
  name: string
  slug: string
  base_url: string
  provider_type: string
  created_at: string
  updated_at: string
  health?: ProviderHealthEntry
  payment_options: PaymentOption[]
  is_oauth?: boolean
  oauth?: OAuthStatus
}

export interface OAuthStatus {
  connected: boolean
  expires_at?: number
  expired: boolean
}

/** OAuth subscription provider kinds the server can connect. */
export type OAuthProviderKind = 'anthropic' | 'openai'

export interface OAuthStartResponse {
  authorize_url: string
  state: string
  instructions: string
}

export interface OAuthCompleteResponse {
  id: number
  name: string
  slug: string
  provider_type: string
}

export interface ProviderFormData {
  name: string
  slug: string
  base_url: string
  api_key: string
  provider_type: string
}

export interface PaymentOption {
  currency: 'msats' | 'sats' | 'usd_micro'
  payment_method: 'lightning' | 'redirect' | 'manual' | 'payment_link'
}

export interface TopupRequest {
  amount: number
  currency: 'msats' | 'sats' | 'usd_micro'
  preferred_method?: 'lightning' | 'redirect' | 'manual' | 'payment_link'
}

export interface Model {
  id: string
  object: string
  created: number
  owned_by: string
  /// Pricing per RIP-05 (present when payments enabled)
  pricing?: ModelPricingInfo
}

export interface ModelPricingInfo {
  prompt: number
  completion: number
  request: number
  unit: string
}

export interface ProviderMetricsSummary {
  provider: string
  p90_tokens_per_second: number | null
  p90_input_tokens_per_second: number | null
  p90_ttft_ms: number | null
  avg_latency_ms: number | null
  success_rate: number | null
  health_state: string | null
  consecutive_failures: number | null
  in_flight: number | null
  max_concurrency: number | null
  backoff_ms: number | null
  load_score: number | null
  available: boolean | null
  health?: ProviderHealthEntry
}

export interface ProviderHealthEntry {
  provider: string
  health_state: string
  consecutive_failures: number
  in_flight: number
  max_concurrency: number | null
  load_score: number | null
  backoff_ms: number
  available: boolean
  last_failure_ago_ms: number | null
  rate_limited: boolean
  balance?: CurrencyAmount
  /** Most-consumed quota window ("first to stop"), for compact display. */
  quota?: QuotaSnapshot
  /** All enforced quota windows, for the detail modal. */
  quotas?: QuotaSnapshot[]
}

export interface QuotaSnapshot {
  remaining?: number
  limit?: number
  used_pct?: number
  resets_at?: number
  window?: string
  status?: string
}

export interface CurrencyAmount {
  currency: 'msats' | 'sats' | 'usd_micro'
  amount: number
}

export interface HealthOverviewResponse {
  providers: ProviderHealthEntry[]
  provider_count: number
  unhealthy_count: number
  degraded_count: number
}

export interface MetricsSnapshotEntry {
  provider: string
  model: string
  p50_ttft_ms: number | null
  p90_ttft_ms: number | null
  p50_output_tps: number | null
  p90_output_tps: number | null
  p50_input_tps: number | null
  p90_input_tps: number | null
  avg_latency_ms: number | null
  success_rate: number | null
}

export interface MetricsSnapshot {
  timestamp_ms: number
  providers: MetricsSnapshotEntry[]
}

export interface MetricsResponse {
  providers: ProviderMetricsSummary[]
  recent_events: Record<string, unknown>[]
  total_requests: number
  total_successes: number
  total_failures: number
}

// WebSocket real-time metrics event types (matching Rust backend)

export interface WsProviderMetrics {
  provider: string
  model: string
  timestamp_ms: number
  event: WsMetricsEvent
  user?: MetricsUser | null
}

export interface MetricsUser {
  id?: number | null
  name?: string | null
  api_key_id?: number | null
  api_key_name?: string | null
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
  | { Balance: CurrencyAmount }
  | { Quota: QuotaSnapshot[] }

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

// ── Payments / Balance ────────────────────────────────────────────────

export interface UserBalanceEntry {
  id: number
  user_id: number
  balance_msat: number
  lifetime_deposited_msat: number
  created_at: string
  updated_at: string
  username: string
}

export interface BalanceTransaction {
  id: number
  user_id: number
  amount_msat: number
  transaction_type: string
  reference_id: string | null
  metadata: string | null
  created_at: string
}

export interface UserBalanceDetail {
  user_id: number
  balance_msat: number
  lifetime_deposited_msat: number
  transactions: BalanceTransaction[]
}

// ── Model Access Control ─────────────────────────────────────────────

export interface UserModelPermission {
  id: number
  user_id: number
  model: string
  allow: boolean
  created_at: string
  updated_at: string
}

export interface CreateUserModelPermission {
  user_id: number
  model: string
  allow: boolean
}

export interface DeleteUserModelPermissionResponse {
  message: string
  user_id: number
  model: string
}

export interface LightningInvoice {
  id: number
  user_id: number
  payment_hash: string
  bolt11: string
  amount_msat: number
  amount_sats: number
  status: string
  created_at: string
  expires_at: string | null
  paid_at: string | null
}

// Payment Instruction Types for Top-up
export interface TopupResponse {
  provider: {
    slug: string
    name: string
  }
  instruction: PaymentInstruction
  message?: string
}

export type PaymentInstruction =
  | LightningBolt11Instruction
  | RedirectInstruction
  | ManualInstruction
  | PaymentLinkInstruction

export interface LightningBolt11Instruction {
  type: 'lightning_bolt11'
  bolt11: string
  payment_hash: string
  amount_sats: number
  amount_msat: number
  memo?: string
  expires_at?: number // Unix timestamp
  invoice_id?: number
}

export interface RedirectInstruction {
  type: 'redirect'
  url: string
  amount_usd?: number
  session_token?: string
}

export interface ManualInstruction {
  type: 'manual'
  instructions: string
  amount_usd?: number
  reference_code?: string
}

export interface PaymentLinkInstruction {
  type: 'payment_link'
  url: string
  amount_usd?: number
  label?: string
}

export interface AdminCreditRequest {
  user_id: number
  amount_sats: number
  reason?: string
}

export interface AdminDebitRequest {
  user_id: number
  amount_sats: number
  reason?: string
}

export interface ModelPricingEntry {
  id: number
  model_name: string
  is_advertised: boolean
  is_free: boolean
  price_per_1m_input_sats: number | null
  price_per_1m_output_sats: number | null
  price_per_request_sats: number | null
  context_window: number | null
  max_output_tokens: number | null
  created_at: string
  updated_at: string
}

export interface ModelPricingCreateRequest {
  model_name: string
  is_advertised: boolean
  is_free: boolean
  price_per_1m_input_sats?: number | null
  price_per_1m_output_sats?: number | null
  price_per_request_sats?: number | null
  context_window?: number | null
  max_output_tokens?: number | null
}

export interface ModelPricingUpdateRequest {
  is_advertised?: boolean
  is_free?: boolean
  price_per_1m_input_sats?: number | null
  price_per_1m_output_sats?: number | null
  price_per_request_sats?: number | null
  context_window?: number | null
  max_output_tokens?: number | null
}
