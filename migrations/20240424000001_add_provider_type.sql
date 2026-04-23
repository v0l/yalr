-- Add provider_type column to providers table
-- This allows us to store which backend implementation to use for each provider
-- Values: 0 (openai), 1 (llamacpp) - matching ProviderType enum repr(u16)

ALTER TABLE providers ADD COLUMN provider_type INTEGER NOT NULL DEFAULT 0;

-- Update existing providers to use 0 (openai) as the default type
-- This maintains backward compatibility
