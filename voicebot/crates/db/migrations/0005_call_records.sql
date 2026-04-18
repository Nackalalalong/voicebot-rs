CREATE TYPE call_direction AS ENUM ('inbound', 'outbound');
CREATE TYPE call_status AS ENUM ('initiated', 'ringing', 'answered', 'completed', 'failed', 'no_answer', 'busy');

CREATE TABLE call_records (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    campaign_id     UUID REFERENCES campaigns(id) ON DELETE SET NULL,
    contact_id      UUID REFERENCES contacts(id) ON DELETE SET NULL,
    session_id      TEXT NOT NULL UNIQUE,
    direction       TEXT NOT NULL DEFAULT 'inbound',
    phone_number    TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'initiated',
    duration_secs   INT,
    recording_url   TEXT,
    transcript      JSONB,
    sentiment       TEXT,
    custom_metrics  JSONB NOT NULL DEFAULT '{}',
    started_at      TIMESTAMPTZ,
    ended_at        TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_call_records_tenant ON call_records(tenant_id);
CREATE INDEX idx_call_records_campaign ON call_records(tenant_id, campaign_id);
CREATE INDEX idx_call_records_session ON call_records(session_id);
CREATE INDEX idx_call_records_created ON call_records(tenant_id, created_at DESC);

ALTER TABLE call_records ENABLE ROW LEVEL SECURITY;
CREATE POLICY call_records_tenant_isolation ON call_records
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
