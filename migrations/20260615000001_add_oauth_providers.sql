-- Add OAuth support to providers.
-- OAuth providers (Claude Max, ChatGPT subscription) authenticate with
-- short-lived access tokens + refresh tokens instead of static API keys.
--
-- provider_type values:
--   8 = anthropic-oauth (Claude Pro/Max subscription via claude.ai OAuth)
--   9 = openai-oauth    (ChatGPT Plus/Pro subscription via Codex OAuth)

ALTER TABLE providers ADD COLUMN oauth_access_token TEXT;
ALTER TABLE providers ADD COLUMN oauth_refresh_token TEXT;
-- Unix epoch milliseconds at which the access token expires.
ALTER TABLE providers ADD COLUMN oauth_expires_at INTEGER;
-- Provider-specific account identifier (ChatGPT account id from id_token).
ALTER TABLE providers ADD COLUMN oauth_account_id TEXT;
