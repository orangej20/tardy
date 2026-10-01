CREATE TABLE push_devices (
    id uuid PRIMARY KEY,
    account_id uuid NOT NULL,
    environment text NOT NULL CHECK (environment IN ('sandbox', 'production')),
    topic text NOT NULL,
    token text NOT NULL,
    token_digest bytea NOT NULL,
    active boolean NOT NULL DEFAULT true,
    invalidated_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (environment, topic, token_digest)
);

CREATE INDEX push_devices_account_idx ON push_devices (account_id) WHERE active;

CREATE TABLE notification_preferences (
    account_id uuid NOT NULL,
    category text NOT NULL,
    enabled boolean NOT NULL DEFAULT true,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, category)
);

CREATE TABLE push_notifications (
    id uuid PRIMARY KEY,
    source_event_id uuid UNIQUE,
    category text NOT NULL,
    title text NOT NULL,
    body text NOT NULL,
    deep_link text,
    data jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE push_deliveries (
    id uuid PRIMARY KEY,
    notification_id uuid NOT NULL REFERENCES push_notifications(id) ON DELETE CASCADE,
    device_id uuid NOT NULL REFERENCES push_devices(id) ON DELETE CASCADE,
    status text NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'sending', 'retry', 'delivered', 'permanent_failure')),
    attempts integer NOT NULL DEFAULT 0,
    available_at timestamptz NOT NULL DEFAULT now(),
    lease_owner text,
    lease_until timestamptz,
    apns_id uuid,
    response_status integer,
    response_reason text,
    delivered_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (notification_id, device_id)
);

CREATE INDEX push_deliveries_ready_idx ON push_deliveries (available_at, id)
    WHERE status IN ('queued', 'retry');
