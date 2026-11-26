-- log.db: the reference store/0 store, schema version 1 (PRAGMA user_version).
--
-- This file runs once, inside the transaction that creates a store
-- (SqliteLog::init) or rebuilds one from an export (import), so a store that
-- failed half way holds no schema. application_id and user_version live in the
-- database header and are written by that same transaction.
--
-- Connection settings are not here, because they cannot be set inside a
-- transaction (journal_mode) or do nothing there (foreign_keys). Every
-- connection runs sqlite::WRITER_PRAGMAS or sqlite::READER_PRAGMAS when it
-- opens:
--   writers  PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;
--            PRAGMA foreign_keys = ON;  PRAGMA busy_timeout = 5000;
--   readers  PRAGMA foreign_keys = ON;  PRAGMA busy_timeout = 5000;
--            PRAGMA query_only = ON;    (opened with SQLITE_OPEN_READ_ONLY)
--
-- The rows of meta are written by code: format = 'store/0', log_id, name,
-- store_signer, store_key and writer_epoch (decimal text).

PRAGMA application_id = 1163281456; -- 0x45564430, 'EVD0'
PRAGMA user_version = 1;

CREATE TABLE meta (k TEXT PRIMARY KEY, v TEXT NOT NULL) STRICT;

-- The log. One row per entry, holding the canonical log-entry/0 text.
CREATE TABLE entries (
  seq INTEGER PRIMARY KEY CHECK (seq >= 1),
  kind TEXT NOT NULL CHECK (kind IN ('object', 'attestation')),
  ref TEXT NOT NULL UNIQUE,
  received_at TEXT NOT NULL,
  epoch INTEGER NOT NULL,
  entry_hash TEXT NOT NULL UNIQUE,
  entry TEXT NOT NULL
) STRICT;

-- Held objects, in canonical form, with the columns heads and lookups need.
CREATE TABLE objects (
  id TEXT PRIMARY KEY,
  seq INTEGER NOT NULL UNIQUE REFERENCES entries(seq),
  at TEXT NOT NULL,
  op TEXT NOT NULL,
  system TEXT NOT NULL,
  type TEXT NOT NULL,
  key TEXT NOT NULL,
  lane TEXT NOT NULL,
  bytes TEXT NOT NULL
) STRICT;
CREATE INDEX objects_subject ON objects(system, type, key, lane, seq);

-- Held attestations, as canonical attestation/0 lines.
CREATE TABLE attestations (
  hash TEXT PRIMARY KEY,
  object TEXT NOT NULL REFERENCES objects(id),
  seq INTEGER NOT NULL UNIQUE REFERENCES entries(seq),
  purpose TEXT NOT NULL,
  signer TEXT NOT NULL,
  key TEXT NOT NULL,
  at TEXT NOT NULL,
  line TEXT NOT NULL
) STRICT;
CREATE INDEX attestations_object ON attestations(object);

-- Projections: written in the same transaction as the entries they follow,
-- and recomputed by verify_store (S_PROJECTION_DRIFT).
CREATE TABLE heads (
  system TEXT NOT NULL,
  type TEXT NOT NULL,
  key TEXT NOT NULL,
  lane TEXT NOT NULL,
  id TEXT NOT NULL REFERENCES objects(id),
  PRIMARY KEY (system, type, key, lane, id)
) STRICT, WITHOUT ROWID;
CREATE TABLE checkpoints (
  seq INTEGER PRIMARY KEY REFERENCES entries(seq),
  id TEXT NOT NULL UNIQUE,
  size INTEGER NOT NULL,
  root TEXT NOT NULL,
  head TEXT NOT NULL
) STRICT;

-- Append-only, enforced inside storage. verify_store compares these six
-- triggers with the ones this file creates (S_TRIGGERS_MISSING).
CREATE TRIGGER entries_no_update BEFORE UPDATE ON entries BEGIN SELECT RAISE(ABORT, 'S_APPEND_ONLY'); END;
CREATE TRIGGER entries_no_delete BEFORE DELETE ON entries BEGIN SELECT RAISE(ABORT, 'S_APPEND_ONLY'); END;
CREATE TRIGGER objects_no_update BEFORE UPDATE ON objects BEGIN SELECT RAISE(ABORT, 'S_APPEND_ONLY'); END;
CREATE TRIGGER objects_no_delete BEFORE DELETE ON objects BEGIN SELECT RAISE(ABORT, 'S_APPEND_ONLY'); END;
CREATE TRIGGER attestations_no_update BEFORE UPDATE ON attestations BEGIN SELECT RAISE(ABORT, 'S_APPEND_ONLY'); END;
CREATE TRIGGER attestations_no_delete BEFORE DELETE ON attestations BEGIN SELECT RAISE(ABORT, 'S_APPEND_ONLY'); END;
