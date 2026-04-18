CREATE TABLE usage_records (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    campaign_id         UUID REFERENCES campaigns(id) ON DELETE SET NULL,
    period_start        TIMESTAMPTZ NOT NULL,
    period_end          TIMESTAMPTZ NOT NULL,
    call_count          BIGINT NOT NULL DEFAULT 0,
    total_duration_secs BIGINT NOT NULL DEFAULT 0,
    asr_seconds         BIGINT NOT NULL DEFAULT 0,
    tts_characters      BIGINT NOT NULL DEFAULT 0,
    llm_tokens          BIGINT NOT NULL DEFAULT 0,
    cost_usd_cents      BIGINT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_usage_tenant ON usage_records(tenant_id);
CREATE INDEX idx_usage_period ON usage_records(tenant_id, period_start, period_end);

ALTER TABLE usage_records ENABLE ROW LEVEL SECURITY;
CREATE POLICY usage_records_tenant_isolation ON usage_records
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
