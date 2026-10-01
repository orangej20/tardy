CREATE TABLE source_channels (
    id text PRIMARY KEY,
    display_name text NOT NULL,
    enabled boolean NOT NULL DEFAULT false,
    transport jsonb NOT NULL,
    rights_policy jsonb NOT NULL,
    transform_plugin text NOT NULL,
    item_limit integer NOT NULL CHECK (item_limit BETWEEN 1 AND 100),
    poll_interval_seconds integer NOT NULL DEFAULT 300 CHECK (poll_interval_seconds BETWEEN 30 AND 86400),
    next_poll_at timestamptz NOT NULL DEFAULT now(),
    etag text,
    last_modified text,
    lease_owner text,
    lease_until timestamptz,
    consecutive_failures integer NOT NULL DEFAULT 0,
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX source_channels_due_idx
    ON source_channels (next_poll_at, id)
    WHERE enabled;

CREATE TABLE source_items (
    id uuid PRIMARY KEY,
    source_id text NOT NULL REFERENCES source_channels(id) ON DELETE RESTRICT,
    external_id text NOT NULL,
    canonical_url text NOT NULL,
    title text NOT NULL,
    author text,
    source_published_at timestamptz,
    summary text,
    facts jsonb NOT NULL DEFAULT '{}'::jsonb,
    content_digest bytea NOT NULL,
    first_seen_at timestamptz NOT NULL,
    last_seen_at timestamptz NOT NULL,
    UNIQUE (source_id, external_id)
);

CREATE INDEX source_items_source_time_idx
    ON source_items (source_id, source_published_at DESC NULLS LAST, id);

CREATE TABLE transformation_runs (
    id uuid PRIMARY KEY,
    source_item_id uuid NOT NULL REFERENCES source_items(id) ON DELETE CASCADE,
    plugin text NOT NULL,
    status text NOT NULL CHECK (status IN ('planned', 'running', 'succeeded', 'failed', 'quarantined')),
    plan jsonb NOT NULL,
    output jsonb,
    attempts integer NOT NULL DEFAULT 0,
    available_at timestamptz NOT NULL,
    started_at timestamptz,
    finished_at timestamptz,
    last_error text,
    created_at timestamptz NOT NULL,
    UNIQUE (source_item_id, plugin)
);

CREATE INDEX transformation_runs_ready_idx
    ON transformation_runs (available_at, id)
    WHERE status IN ('planned', 'failed');

CREATE TABLE outbox (
    id uuid PRIMARY KEY,
    topic text NOT NULL,
    aggregate_type text NOT NULL,
    aggregate_id text NOT NULL,
    payload jsonb NOT NULL,
    available_at timestamptz NOT NULL,
    attempts integer NOT NULL DEFAULT 0,
    lease_owner text,
    lease_until timestamptz,
    delivered_at timestamptz,
    last_error text,
    created_at timestamptz NOT NULL,
    UNIQUE (topic, aggregate_type, aggregate_id)
);

CREATE INDEX outbox_ready_idx
    ON outbox (available_at, id)
    WHERE delivered_at IS NULL;
