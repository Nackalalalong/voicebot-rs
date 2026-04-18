-- Add per-tenant concurrent session limit.
-- The default of 10 is appropriate for the 'starter' plan;
-- production tenants can be updated via UPDATE tenants SET max_concurrent_sessions = N.
ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS max_concurrent_sessions INT NOT NULL DEFAULT 10;
