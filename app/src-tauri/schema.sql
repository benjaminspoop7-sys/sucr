-- Sucr storage schema, version 1 (plan §2.3).
-- Applied inside one transaction by db::open when PRAGMA user_version = 0.

CREATE TABLE categories (
  id            INTEGER PRIMARY KEY,
  name          TEXT    NOT NULL,
  folder        TEXT,
  rule_tier     TEXT    NOT NULL DEFAULT 'URL'
                        CHECK (rule_tier IN ('URL', 'FILE_NAME', 'EXTENSION')),
  patterns_json TEXT    NOT NULL DEFAULT '[]',
  match_all     INTEGER NOT NULL DEFAULT 0 CHECK (match_all IN (0, 1)),
  remember_path INTEGER NOT NULL DEFAULT 0 CHECK (remember_path IN (0, 1)),
  sort_order    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE queues (
  id            INTEGER PRIMARY KEY,
  name          TEXT    NOT NULL,
  files_at_once INTEGER NOT NULL DEFAULT 1,
  order_json    TEXT    NOT NULL DEFAULT '[]'
);

CREATE TABLE schedules (
  id           INTEGER PRIMARY KEY,
  kind         TEXT    NOT NULL CHECK (kind IN ('one-time', 'weekly')),
  at_minute    INTEGER,
  days         INTEGER NOT NULL DEFAULT 0,
  end_date     INTEGER,
  one_time     INTEGER,
  stop_minute  INTEGER,
  retries      INTEGER NOT NULL DEFAULT 0,
  on_done      TEXT    NOT NULL DEFAULT 'none'
);

CREATE TABLE credentials (
  id          INTEGER PRIMARY KEY,
  scheme      TEXT    NOT NULL
                      CHECK (scheme IN ('basic', 'digest', 'ntlm', 'proxy', 'cookie', 'header-secret')),
  host        TEXT    NOT NULL,
  path        TEXT,
  username    TEXT,
  secret_blob BLOB,
  created_at  INTEGER NOT NULL
);

CREATE TABLE networks (
  id                INTEGER PRIMARY KEY,
  mac_hint          TEXT,
  display_name      TEXT    NOT NULL,
  kind              TEXT    NOT NULL,
  color_id          TEXT,
  enabled           INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  speed_limit       INTEGER,
  data_limit        INTEGER,
  data_limit_period TEXT    CHECK (data_limit_period IN ('day', 'week', 'month')),
  custom_name       TEXT
);

CREATE TABLE downloads (
  id               INTEGER PRIMARY KEY,
  url              TEXT    NOT NULL,
  final_url        TEXT,
  method           TEXT    NOT NULL DEFAULT 'GET',
  status           TEXT    NOT NULL DEFAULT 'queued'
                   CHECK (status IN ('queued', 'downloading', 'paused',
                                     'finished', 'verified', 'needs-attention')),
  category_id      INTEGER REFERENCES categories(id) ON DELETE SET NULL,
  folder           TEXT,
  filename         TEXT    NOT NULL,
  content_type     TEXT,
  staging_path     TEXT,
  etag             TEXT,
  last_modified    TEXT,
  total_bytes      INTEGER,
  wanted_bytes     INTEGER,
  verified_bytes   INTEGER NOT NULL DEFAULT 0,
  block_size       INTEGER,
  block_count      INTEGER,
  streams_mode     TEXT,
  streams_fixed    INTEGER,
  streams_final    INTEGER,
  speed_limit      INTEGER,
  proxy_profile_id INTEGER,
  schedule_id      INTEGER REFERENCES schedules(id) ON DELETE SET NULL,
  queue_id         INTEGER REFERENCES queues(id) ON DELETE SET NULL,
  checksum_algo    TEXT    CHECK (checksum_algo IN ('md5', 'sha1', 'sha256', 'sha512')),
  checksum_hex     TEXT,
  created_at       INTEGER NOT NULL,
  started_at       INTEGER,
  finished_at      INTEGER,
  error_code       TEXT,
  error_hint       TEXT,
  resumable        INTEGER CHECK (resumable IN (0, 1)),
  source_browser   TEXT,
  referer          TEXT,
  ua               TEXT,
  headers_json     TEXT    NOT NULL DEFAULT '{}',
  zone_referrer    TEXT,
  zone_host        TEXT,
  -- A DASH pair taken as one job: the sound stream's link, and the picture
  -- already fetched to disk. One row, one file, no second entry in the library.
  audio_url        TEXT,
  part_path        TEXT
);

CREATE TABLE blocks (
  download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
  idx         INTEGER NOT NULL,
  range_start INTEGER NOT NULL,
  range_end   INTEGER NOT NULL,
  state       TEXT    NOT NULL DEFAULT 'pending'
              CHECK (state IN ('pending', 'claimed', 'active', 'stalled',
                               'hedge', 'done', 'failed')),
  bytes_done  INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (download_id, idx)
);

CREATE TABLE block_attempts (
  id          INTEGER PRIMARY KEY,
  download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
  block_idx   INTEGER NOT NULL,
  network_id  INTEGER REFERENCES networks(id) ON DELETE SET NULL,
  started_at  INTEGER NOT NULL,
  ended_at    INTEGER,
  bytes       INTEGER NOT NULL DEFAULT 0,
  state       TEXT    NOT NULL,
  reason      TEXT,
  detail      TEXT
);

CREATE TABLE header_secrets (
  download_id   INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
  name          TEXT    NOT NULL,
  credential_id INTEGER NOT NULL REFERENCES credentials(id) ON DELETE CASCADE,
  PRIMARY KEY (download_id, name)
);

CREATE TABLE dupe_index (
  id          INTEGER PRIMARY KEY,
  url_hash    TEXT NOT NULL,
  filename    TEXT NOT NULL,
  size        INTEGER,
  etag        TEXT,
  download_id INTEGER REFERENCES downloads(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL CHECK (kind IN ('SAME_URL', 'SAME_NAME_AND_SIZE', 'SAME_ETAG'))
);

CREATE TABLE net_usage (
  network_id INTEGER NOT NULL REFERENCES networks(id) ON DELETE CASCADE,
  period_key TEXT    NOT NULL,
  bytes      INTEGER NOT NULL DEFAULT 0,
  day        INTEGER NOT NULL,
  week       INTEGER NOT NULL,
  month      INTEGER NOT NULL,
  PRIMARY KEY (network_id, period_key)
);

-- Bytes that left on no adapter we can name: a proxied link, a localhost target,
-- a route whose device has no card. Kept beside `net_usage` rather than inside it
-- because a row here belongs to no interface, and charging it to one would be the
-- false instrument this ledger exists to avoid. Same day-keyed shape, so the two
-- add up to what Sucr actually moved.
CREATE TABLE unattributed_usage (
  period_key TEXT    NOT NULL PRIMARY KEY,
  bytes      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE media_captures (
  id            INTEGER PRIMARY KEY,
  page_url      TEXT    NOT NULL,
  kind          TEXT    NOT NULL,
  quality       TEXT,
  codec         TEXT,
  size          INTEGER,
  url           TEXT    NOT NULL,
  decided_at    INTEGER,
  site          TEXT,
  manifest_kind TEXT,
  container     TEXT,
  tracks_json   TEXT,
  subtitle_lang TEXT
);

CREATE TABLE media_tracks (
  capture_id    INTEGER NOT NULL REFERENCES media_captures(id) ON DELETE CASCADE,
  idx           INTEGER NOT NULL,
  role          TEXT    NOT NULL,
  codec         TEXT,
  bandwidth     INTEGER,
  width         INTEGER,
  height        INTEGER,
  framerate     REAL,
  language      TEXT,
  segments_json TEXT,
  init_url      TEXT,
  key_ref       TEXT,
  PRIMARY KEY (capture_id, idx)
);

CREATE TABLE history_events (
  id           INTEGER PRIMARY KEY,
  download_id  INTEGER REFERENCES downloads(id) ON DELETE CASCADE,
  ts           INTEGER NOT NULL,
  kind         TEXT    NOT NULL,
  payload_json TEXT    NOT NULL DEFAULT '{}'
);

CREATE TABLE grabber_projects (
  id              INTEGER PRIMARY KEY,
  name            TEXT NOT NULL,
  start_url       TEXT NOT NULL,
  levels          INTEGER NOT NULL DEFAULT 1,
  filters_json    TEXT NOT NULL DEFAULT '{}',
  save_rules_json TEXT NOT NULL DEFAULT '{}',
  last_run        INTEGER
);

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE INDEX idx_downloads_status     ON downloads(status);
CREATE INDEX idx_downloads_category   ON downloads(category_id);
CREATE INDEX idx_blocks_state         ON blocks(download_id, state);
CREATE INDEX idx_attempts_download    ON block_attempts(download_id, started_at);
CREATE INDEX idx_dupe_url             ON dupe_index(url_hash);
CREATE INDEX idx_dupe_name_size       ON dupe_index(filename, size);
CREATE INDEX idx_history_download     ON history_events(download_id, ts);
CREATE INDEX idx_media_tracks_capture ON media_tracks(capture_id);
