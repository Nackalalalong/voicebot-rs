-- C7: Phone number → campaign routing
ALTER TABLE phone_numbers
    ADD COLUMN IF NOT EXISTS campaign_id UUID REFERENCES campaigns(id) ON DELETE SET NULL;
