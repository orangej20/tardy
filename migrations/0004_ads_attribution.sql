CREATE TABLE ad_campaigns (
    id uuid PRIMARY KEY,
    advertiser_profile_id uuid NOT NULL,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 160),
    currency varchar(3) NOT NULL CHECK (currency = upper(currency)),
    budget_micros bigint NOT NULL CHECK (budget_micros > 0),
    attribution_window_seconds bigint NOT NULL CHECK (attribution_window_seconds BETWEEN 60 AND 2592000),
    attribution_model text NOT NULL CHECK (attribution_model IN ('first_touch','last_touch')),
    boosted_reel_id uuid,
    status text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','awaiting_payment','active','paused','complete','rejected')),
    funded_micros bigint NOT NULL DEFAULT 0 CHECK (funded_micros >= 0),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE ad_creatives (
    id uuid PRIMARY KEY,
    campaign_id uuid NOT NULL REFERENCES ad_campaigns(id),
    reel_id uuid NOT NULL,
    destination_url text NOT NULL CHECK (length(destination_url) <= 2048),
    disclosure text NOT NULL CHECK (length(disclosure) > 0),
    status text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','approved','rejected')),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE creator_partnerships (
    id uuid PRIMARY KEY,
    campaign_id uuid NOT NULL REFERENCES ad_campaigns(id),
    creator_profile_id uuid NOT NULL,
    revenue_share_bps integer NOT NULL CHECK (revenue_share_bps BETWEEN 0 AND 10000),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (campaign_id, creator_profile_id)
);

CREATE TABLE ad_events (
    id uuid PRIMARY KEY,
    campaign_id uuid NOT NULL REFERENCES ad_campaigns(id),
    creative_id uuid REFERENCES ad_creatives(id),
    creator_profile_id uuid,
    kind text NOT NULL CHECK (kind IN ('impression','qualified_view','click','conversion','refund','spend')),
    idempotency_key varchar(200) NOT NULL,
    occurred_at timestamptz NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    revenue_micros bigint NOT NULL DEFAULT 0,
    spend_micros bigint NOT NULL DEFAULT 0 CHECK (spend_micros >= 0),
    attributed_touch_event_id uuid REFERENCES ad_events(id),
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(metadata) = 'object'),
    UNIQUE (campaign_id, idempotency_key),
    CHECK (kind <> 'conversion' OR revenue_micros >= 0),
    CHECK (kind <> 'refund' OR revenue_micros <= 0),
    CHECK (kind <> 'spend' OR spend_micros > 0),
    CHECK (kind NOT IN ('impression','qualified_view','click') OR (revenue_micros = 0 AND spend_micros = 0)),
    CHECK (kind NOT IN ('conversion','refund') OR spend_micros = 0)
);

CREATE INDEX ad_events_campaign_time_idx ON ad_events (campaign_id, occurred_at);
CREATE INDEX ad_events_creator_time_idx ON ad_events (creator_profile_id, occurred_at)
    WHERE creator_profile_id IS NOT NULL;

-- Attribution facts are an append-only audit log. Corrections are compensating events.
CREATE OR REPLACE FUNCTION reject_ad_event_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'ad_events are immutable; append a compensating event';
END;
$$;

CREATE TRIGGER ad_events_no_update BEFORE UPDATE OR DELETE ON ad_events
FOR EACH ROW EXECUTE FUNCTION reject_ad_event_mutation();

CREATE TABLE ad_funding_intents (
    id uuid PRIMARY KEY,
    campaign_id uuid NOT NULL REFERENCES ad_campaigns(id),
    scheme text NOT NULL,
    network text NOT NULL,
    amount text NOT NULL CHECK (amount ~ '^[0-9]+$'),
    asset text NOT NULL,
    pay_to text NOT NULL,
    budget_credit_micros bigint NOT NULL CHECK (budget_credit_micros > 0),
    status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','settled')),
    created_at timestamptz NOT NULL DEFAULT now(),
    settled_at timestamptz
);

CREATE TABLE ad_payment_receipts (
    id uuid PRIMARY KEY,
    funding_intent_id uuid NOT NULL UNIQUE REFERENCES ad_funding_intents(id),
    transaction_id text NOT NULL,
    network text NOT NULL,
    payer text NOT NULL,
    amount text NOT NULL,
    asset text NOT NULL,
    pay_to text NOT NULL,
    settled_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (network, transaction_id)
);
