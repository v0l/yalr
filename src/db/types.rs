#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[repr(u16)]
pub enum UserType {
    Internal = 0,
    Nostr = 1,
    OAuth = 2,
}

impl UserType {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserType::Internal => "internal",
            UserType::Nostr => "nostr",
            UserType::OAuth => "oauth",
        }
    }
    
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "internal" => Some(UserType::Internal),
            "nostr" => Some(UserType::Nostr),
            "oauth" => Some(UserType::OAuth),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[repr(u16)]
pub enum ProviderType {
    OpenAi = 0,
    LlamaCpp = 1,
    Vllm = 2,
    Ollama = 3,
    Routstr = 4,
    OpenRouter = 5,
    Anthropic = 6,
    Ppq = 7,
    AnthropicOauth = 8,
    OpenAiOauth = 9,
}

impl ProviderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderType::OpenAi => "openai",
            ProviderType::LlamaCpp => "llamacpp",
            ProviderType::Vllm => "vllm",
            ProviderType::Ollama => "ollama",
            ProviderType::Routstr => "routstr",
            ProviderType::OpenRouter => "openrouter",
            ProviderType::Anthropic => "anthropic",
            ProviderType::Ppq => "ppq",
            ProviderType::AnthropicOauth => "anthropic-oauth",
            ProviderType::OpenAiOauth => "openai-oauth",
        }
    }

    /// Whether this provider type authenticates via OAuth tokens rather than a static API key.
    pub fn is_oauth(&self) -> bool {
        matches!(self, ProviderType::AnthropicOauth | ProviderType::OpenAiOauth)
    }
    
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "openai" => Some(ProviderType::OpenAi),
            "llamacpp" => Some(ProviderType::LlamaCpp),
            "vllm" => Some(ProviderType::Vllm),
            "ollama" => Some(ProviderType::Ollama),
            "routstr" => Some(ProviderType::Routstr),
            "openrouter" => Some(ProviderType::OpenRouter),
            "anthropic" => Some(ProviderType::Anthropic),
            "ppq" => Some(ProviderType::Ppq),
            "anthropic-oauth" => Some(ProviderType::AnthropicOauth),
            "openai-oauth" => Some(ProviderType::OpenAiOauth),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Provider {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub provider_type: ProviderType,
    #[sqlx(default)]
    pub oauth_access_token: Option<String>,
    #[sqlx(default)]
    pub oauth_refresh_token: Option<String>,
    #[sqlx(default)]
    pub oauth_expires_at: Option<i64>,
    #[sqlx(default)]
    pub oauth_account_id: Option<String>,
    #[sqlx(default)]
    pub client_version: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// OAuth credentials persisted for an OAuth-backed provider.
#[derive(Clone, Debug)]
pub struct OAuthCredentials {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix epoch milliseconds at which the access token expires.
    pub expires_at: i64,
    pub account_id: Option<String>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Model {
    pub id: i64,
    pub name: String,
    pub cost_per_1m_input: f64,
    pub cost_per_1m_output: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct ModelProvider {
    pub id: i64,
    pub model_id: i64,
    pub provider_id: i64,
    pub weight: i32,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct RoutingConfig {
    pub id: i64,
    pub name: String,
    pub strategy: String,
    pub health_check_enabled: bool,
    pub health_check_interval_seconds: i32,
    pub health_check_timeout_seconds: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub input_modalities: Option<String>,
    pub output_modalities: Option<String>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct RoutingConfigProvider {
    pub id: i64,
    pub routing_config_id: i64,
    pub provider_id: i64,
    pub model: Option<String>,
    pub weight: i32,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug)]
pub struct NewProvider<'a> {
    pub name: &'a str,
    pub slug: &'a str,
    pub base_url: &'a str,
    pub api_key: Option<&'a str>,
    pub provider_type: Option<ProviderType>,
}

#[derive(Clone, Debug)]
pub struct NewModel<'a> {
    pub name: &'a str,
    pub cost_per_1m_input: f64,
    pub cost_per_1m_output: f64,
}

#[derive(Clone, Debug)]
pub struct NewModelProvider {
    pub model_id: i64,
    pub provider_id: i64,
    pub weight: i32,
    pub is_active: bool,
}

#[derive(Clone, Debug)]
pub struct NewRoutingConfig {
    pub name: String,
    pub strategy: String,
    pub health_check_enabled: bool,
    pub health_check_interval_seconds: i32,
    pub health_check_timeout_seconds: i32,
}

#[derive(Clone, Debug)]
pub struct NewRoutingConfigProvider {
    pub routing_config_id: i64,
    pub provider_id: i64,
    pub model: Option<String>,
    pub weight: i32,
    pub is_active: bool,
}

#[derive(Clone, Debug)]
pub struct UpdateProvider<'a> {
    pub name: Option<&'a str>,
    pub slug: Option<&'a str>,
    pub base_url: Option<&'a str>,
    pub api_key: Option<Option<&'a str>>,
    pub provider_type: Option<ProviderType>,
}

#[derive(Clone, Debug)]
pub struct UpdateModel<'a> {
    pub name: Option<&'a str>,
    pub cost_per_1m_input: Option<f64>,
    pub cost_per_1m_output: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct UpdateModelProvider {
    pub weight: Option<i32>,
    pub is_active: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct UpdateRoutingConfig {
    pub name: Option<String>,
    pub strategy: Option<String>,
    pub health_check_enabled: Option<bool>,
    pub health_check_interval_seconds: Option<i32>,
    pub health_check_timeout_seconds: Option<i32>,
}

#[derive(Clone, Debug)]
pub struct UpdateRoutingConfigProvider {
    pub model: Option<String>,
    pub weight: Option<i32>,
    pub is_active: Option<bool>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub username: Option<String>,
    pub password_hash: Option<String>,
    pub external_id: Option<String>,
    pub user_type: UserType,
    pub is_admin: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug)]
pub struct NewUser<'a> {
    pub username: Option<&'a str>,
    pub password_hash: Option<&'a str>,
    pub external_id: Option<&'a str>,
    pub user_type: UserType,
    pub is_admin: bool,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct ApiKey {
    pub id: i64,
    pub key_hash: String,
    pub name: String,
    pub user_id: i64,
    pub last_four: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub is_active: bool,
}

#[derive(Clone, Debug)]
pub struct NewApiKey<'a> {
    pub key_hash: &'a str,
    pub name: &'a str,
    pub user_id: i64,
    pub last_four: &'a str,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Copy)]
pub struct Page<'a> {
    pub limit: i64,
    pub before: Option<i64>,
    pub search: Option<&'a str>,
}

// ── Payments Row Types ────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserBalanceRow {
    pub id: i64,
    pub user_id: i64,
    pub balance_msat: i64,
    pub lifetime_deposited_msat: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Balance row joined with username for admin listing.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct UserBalanceWithUsername {
    pub id: i64,
    pub user_id: i64,
    pub balance_msat: i64,
    pub lifetime_deposited_msat: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub username: String,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct BalanceTransactionRow {
    pub id: i64,
    pub user_id: i64,
    pub amount_msat: i64,
    pub transaction_type: String,
    pub reference_id: Option<String>,
    pub metadata: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct NewLightningInvoice<'a> {
    pub user_id: i64,
    pub payment_hash: &'a str,
    pub bolt11: &'a str,
    pub amount_msat: i64,
    pub amount_sats: i64,
    pub expire_seconds: Option<u32>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct LightningInvoiceRow {
    pub id: i64,
    pub user_id: i64,
    pub payment_hash: String,
    pub bolt11: String,
    pub amount_msat: i64,
    pub amount_sats: i64,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub paid_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ModelPricingRow {
    pub id: i64,
    pub model_name: String,
    pub is_advertised: bool,
    pub is_free: bool,
    pub price_per_1m_input_sats: Option<i64>,
    pub price_per_1m_output_sats: Option<i64>,
    pub price_per_1m_cache_read_sats: Option<i64>,
    pub price_per_1m_cache_write_sats: Option<i64>,
    pub price_per_request_sats: Option<i64>,
    pub context_window: Option<i32>,
    pub max_output_tokens: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct NewModelPricing<'a> {
    pub model_name: &'a str,
    pub is_advertised: bool,
    pub is_free: bool,
    pub price_per_1m_input_sats: Option<i64>,
    pub price_per_1m_output_sats: Option<i64>,
    pub price_per_1m_cache_read_sats: Option<i64>,
    pub price_per_1m_cache_write_sats: Option<i64>,
    pub price_per_request_sats: Option<i64>,
    pub context_window: Option<i32>,
    pub max_output_tokens: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct UpdateModelPricing {
    pub is_advertised: Option<bool>,
    pub is_free: Option<bool>,
    pub price_per_1m_input_sats: Option<Option<i64>>,
    pub price_per_1m_output_sats: Option<Option<i64>>,
    pub price_per_1m_cache_read_sats: Option<Option<i64>>,
    pub price_per_1m_cache_write_sats: Option<Option<i64>>,
    pub price_per_request_sats: Option<Option<i64>>,
    pub context_window: Option<Option<i32>>,
    pub max_output_tokens: Option<Option<i32>>,
}

// ── Model Access Control ───────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserModelPermission {
    pub id: i64,
    pub user_id: i64,
    pub model: String,
    pub allow: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct NewUserModelPermission<'a> {
    pub user_id: i64,
    pub model: &'a str,
    pub allow: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateUserModelPermission {
    pub allow: Option<bool>,
}

/// Result of a model access check for a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelAccess {
    Allow,
    Deny,
}

