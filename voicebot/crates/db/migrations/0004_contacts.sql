CREATE TYPE contact_status AS ENUM ('pending', 'claimed', 'calling', 'completed', 'failed', 'do_not_call');

CREATE TABLE contacts (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    campaign_id     UUID NOT NULL REFERENCES campaigns(id) ON DELETE CASCADE,
    phone_number    TEXT NOT NULL,
    first_name      TEXT,
    last_name       TEXT,
    metadata        JSONB NOT NULL DEFAULT '{}',
    status          TEXT NOT NULL DEFAULT 'pending',
    retry_count     INT NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ,
    next_attempt_at TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, campaign_id, phone_number)
);

CREATE INDEX idx_contacts_tenant_campaign ON contacts(tenant_id, campaign_id);
CREATE INDEX idx_contacts_status ON contacts(tenant_id, campaign_id, status);
CREATE INDEX idx_contacts_next_attempt ON contacts(campaign_id, next_attempt_at) WHERE status = 'pending';

ALTER TABLE contacts ENABLE ROW LEVEL SECURITY;
CREATE POLICY contacts_tenant_isolation ON contacts
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
