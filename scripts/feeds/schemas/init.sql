-- DuckDB schema for the feed pipeline
-- Run once to initialize: duckdb <data_home>/feeds/feeds.duckdb < schemas/init.sql

CREATE SEQUENCE IF NOT EXISTS feeds_id_seq START 1;
CREATE SEQUENCE IF NOT EXISTS items_id_seq START 1;
CREATE SEQUENCE IF NOT EXISTS alerts_id_seq START 1;
CREATE SEQUENCE IF NOT EXISTS alert_log_id_seq START 1;

CREATE TABLE IF NOT EXISTS feeds (
    id INTEGER DEFAULT nextval('feeds_id_seq') PRIMARY KEY,
    name VARCHAR NOT NULL,
    source_type VARCHAR NOT NULL,
    url VARCHAR,
    driver VARCHAR,
    channel_id VARCHAR,
    variant VARCHAR,
    config_json VARCHAR,
    trust VARCHAR DEFAULT 'medium',
    enabled BOOLEAN DEFAULT true,
    check_interval VARCHAR DEFAULT '1h',
    created_at TIMESTAMP DEFAULT current_timestamp,
    removed_at TIMESTAMP
);

ALTER TABLE feeds ADD COLUMN IF NOT EXISTS source_id VARCHAR;
ALTER TABLE feeds ADD COLUMN IF NOT EXISTS scope VARCHAR DEFAULT 'global';
ALTER TABLE feeds ADD COLUMN IF NOT EXISTS workspace_id VARCHAR DEFAULT '';
ALTER TABLE feeds ADD COLUMN IF NOT EXISTS security_status VARCHAR DEFAULT 'accepted';
ALTER TABLE feeds ADD COLUMN IF NOT EXISTS last_started_at TIMESTAMP;
ALTER TABLE feeds ADD COLUMN IF NOT EXISTS next_due_at TIMESTAMP;
UPDATE feeds SET source_id = lower(replace(name, ' ', '-')) WHERE source_id IS NULL;

-- Existing databases created before removed_at existed need it added
-- explicitly; CREATE TABLE IF NOT EXISTS above is a no-op for them.
ALTER TABLE feeds ADD COLUMN IF NOT EXISTS removed_at TIMESTAMP;

CREATE TABLE IF NOT EXISTS items (
    id INTEGER DEFAULT nextval('items_id_seq') PRIMARY KEY,
    feed_id INTEGER REFERENCES feeds(id),
    external_id VARCHAR,
    title VARCHAR,
    url VARCHAR,
    author VARCHAR,
    summary TEXT,
    content TEXT,
    published_at TIMESTAMP,
    ingested_at TIMESTAMP DEFAULT current_timestamp,
    tags VARCHAR[],
    content_hash VARCHAR,
    word_count INTEGER DEFAULT 0,
    language VARCHAR DEFAULT 'en',
    key_phrases VARCHAR[],
    source_trust VARCHAR DEFAULT 'medium',
    view_count INTEGER DEFAULT 0,
    search_match_count INTEGER DEFAULT 0,
    last_referenced_at TIMESTAMP
);

ALTER TABLE items ADD COLUMN IF NOT EXISTS scope VARCHAR DEFAULT 'global';
ALTER TABLE items ADD COLUMN IF NOT EXISTS workspace_id VARCHAR DEFAULT '';
ALTER TABLE items ADD COLUMN IF NOT EXISTS security_status VARCHAR DEFAULT 'accepted';
ALTER TABLE items ADD COLUMN IF NOT EXISTS security_detail VARCHAR;

CREATE TABLE IF NOT EXISTS search_index (
    item_id INTEGER REFERENCES items(id),
    chunk_index INTEGER,
    chunk_text TEXT,
    word_offsets JSON,
    PRIMARY KEY (item_id, chunk_index)
);

CREATE TABLE IF NOT EXISTS seen (
    feed_id INTEGER,
    dedup_hash VARCHAR,
    first_seen TIMESTAMP DEFAULT current_timestamp,
    PRIMARY KEY (feed_id, dedup_hash)
);

CREATE TABLE IF NOT EXISTS alerts (
    id INTEGER DEFAULT nextval('alerts_id_seq') PRIMARY KEY,
    name VARCHAR NOT NULL,
    match_config JSON,
    action VARCHAR DEFAULT 'deliver',
    deliver_to VARCHAR,
    hook_command VARCHAR,
    cooldown_minutes INTEGER DEFAULT 30,
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMP DEFAULT current_timestamp
);

ALTER TABLE alerts ADD COLUMN IF NOT EXISTS scope VARCHAR DEFAULT 'global';
ALTER TABLE alerts ADD COLUMN IF NOT EXISTS workspace_id VARCHAR DEFAULT '';

CREATE TABLE IF NOT EXISTS alert_log (
    id INTEGER DEFAULT nextval('alert_log_id_seq') PRIMARY KEY,
    alert_id INTEGER REFERENCES alerts(id),
    item_id INTEGER REFERENCES items(id),
    match_score FLOAT,
    match_reasons JSON,
    action VARCHAR,
    delivered_at TIMESTAMP DEFAULT current_timestamp,
    success BOOLEAN DEFAULT false,
    detail VARCHAR
);

CREATE INDEX IF NOT EXISTS idx_items_published ON items(published_at);
CREATE INDEX IF NOT EXISTS idx_items_feed ON items(feed_id);
CREATE INDEX IF NOT EXISTS idx_items_hash ON items(content_hash);
CREATE INDEX IF NOT EXISTS idx_items_trust ON items(source_trust);
CREATE INDEX IF NOT EXISTS idx_seen_hash ON seen(dedup_hash);
CREATE INDEX IF NOT EXISTS idx_seen_feed ON seen(feed_id);
CREATE INDEX IF NOT EXISTS idx_alert_log_alert ON alert_log(alert_id);
CREATE INDEX IF NOT EXISTS idx_alert_log_time ON alert_log(delivered_at);
CREATE INDEX IF NOT EXISTS idx_search_item ON search_index(item_id);
