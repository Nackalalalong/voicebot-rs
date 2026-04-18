CREATE TABLE phone_numbers (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id               UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    number                  TEXT NOT NULL UNIQUE,
    provider                TEXT NOT NULL,
    provider_number_id      TEXT,
    status                  TEXT NOT NULL DEFAULT 'active',
    capabilities            JSONB NOT NULL DEFAULT '{"voice": true, "sms": false}',
    monthly_cost_usd_cents  BIGINT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_phone_numbers_tenant ON phone_numbers(tenant_id);

ALTER TABLE phone_numbers ENABLE ROW LEVEL SECURITY;
CREATE POLICY phone_numbers_tenant_isolation ON phone_numbers
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
