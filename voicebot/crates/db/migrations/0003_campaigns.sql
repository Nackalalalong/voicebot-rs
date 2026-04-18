CREATE TYPE campaign_status AS ENUM ('draft', 'active', 'paused', 'completed', 'archived');

CREATE TABLE campaigns (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id               UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name                    TEXT NOT NULL,
    status                  TEXT NOT NULL DEFAULT 'draft',
    system_prompt           TEXT NOT NULL DEFAULT '',
    language                TEXT NOT NULL DEFAULT 'en',
    voice_id                TEXT,
    asr_provider            TEXT NOT NULL DEFAULT 'whisper',
    tts_provider            TEXT NOT NULL DEFAULT 'kokoro',
    llm_provider            TEXT NOT NULL DEFAULT 'openai',
    llm_model               TEXT NOT NULL DEFAULT 'gpt-4o-mini',
    max_call_duration_secs  INT NOT NULL DEFAULT 300,
    recording_enabled       BOOLEAN NOT NULL DEFAULT true,
    tools_config            JSONB NOT NULL DEFAULT '[]',
    custom_metrics          JSONB NOT NULL DEFAULT '{}',
    schedule_config         JSONB NOT NULL DEFAULT '{}',
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_campaigns_tenant_id ON campaigns(tenant_id);
CREATE INDEX idx_campaigns_status ON campaigns(tenant_id, status);

CREATE TABLE prompt_versions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    campaign_id UUID NOT NULL REFERENCES campaigns(id) ON DELETE CASCADE,
    tenant_id   UUID NOT NULL,
    version     INT NOT NULL,
    system_prompt TEXT NOT NULL,
    change_note TEXT,
    created_by  UUID NOT NULL REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (campaign_id, version)
);

CREATE INDEX idx_prompt_versions_campaign ON prompt_versions(campaign_id);

ALTER TABLE campaigns ENABLE ROW LEVEL SECURITY;
CREATE POLICY campaigns_tenant_isolation ON campaigns
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

ALTER TABLE prompt_versions ENABLE ROW LEVEL SECURITY;
CREATE POLICY prompt_versions_tenant_isolation ON prompt_versions
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);
