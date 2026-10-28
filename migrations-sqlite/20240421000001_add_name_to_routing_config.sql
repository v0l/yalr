-- Add name column to routing_config table
-- This allows named routing configurations for model aliasing

ALTER TABLE routing_config ADD COLUMN name TEXT NOT NULL DEFAULT 'default';
