CREATE TABLE feed_events (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    kind text NOT NULL CHECK (kind IN ('post_published','hyper_tardy')),
    reel_id uuid NOT NULL,
    hashtags text[] NOT NULL DEFAULT '{}',
    payload jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (kind, reel_id)
);

CREATE INDEX feed_events_hashtags_idx ON feed_events USING gin (hashtags);

CREATE TABLE feed_subscriptions (
    id uuid PRIMARY KEY,
    account_id uuid NOT NULL,
    kind text NOT NULL CHECK (kind IN ('hashtag','hyper_tardy')),
    hashtag text,
    delivery text NOT NULL CHECK (delivery IN ('poll','webhook')),
    webhook_url text,
    active boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((kind='hashtag') = (hashtag IS NOT NULL)),
    CHECK ((delivery='webhook') = (webhook_url IS NOT NULL))
);

CREATE INDEX feed_subscriptions_account_idx ON feed_subscriptions (account_id) WHERE active;

CREATE TABLE webhook_deliveries (
    id uuid PRIMARY KEY,
    subscription_id uuid NOT NULL REFERENCES feed_subscriptions(id) ON DELETE CASCADE,
    event_id bigint NOT NULL REFERENCES feed_events(id) ON DELETE CASCADE,
    status text NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','sending','retry','delivered','failed')),
    attempts integer NOT NULL DEFAULT 0,
    available_at timestamptz NOT NULL DEFAULT now(),
    lease_owner text,
    lease_until timestamptz,
    response_status integer,
    last_error text,
    delivered_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (subscription_id,event_id)
);

CREATE INDEX webhook_deliveries_ready_idx ON webhook_deliveries (available_at,id)
    WHERE status IN ('queued','retry');
