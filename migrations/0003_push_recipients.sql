ALTER TABLE push_notifications ADD COLUMN account_id uuid NOT NULL;
ALTER TABLE outbox ADD COLUMN failed_at timestamptz;
CREATE INDEX push_notifications_account_idx
    ON push_notifications (account_id, created_at DESC);
