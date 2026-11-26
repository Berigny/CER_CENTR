//! The queries and inserts over one `log.db` connection, shared by
//! [`SqliteLog`](super::SqliteLog), [`SqliteReader`](super::SqliteReader),
//! the writer's [`IntakeState`] and [`import`](crate::import::import).
//!
//! Every function runs on whatever transaction the connection is in. A
//! writer calls them inside its `BEGIN IMMEDIATE`, a reader inside one read
//! transaction per [`LogRead`](crate::api::LogRead) call or snapshot.
//!
//! Reading never panics on what the file holds. Text is decoded lossily, so
//! a damaged value (invalid UTF-8, which only a raw edit can store) reaches
//! the verifier as a finding instead of failing the read. A value of the
//! wrong SQL type, which the `STRICT` tables refuse, is `S_IO`.

use evidence_core::canon;
use evidence_core::json::Value;
use evidence_core::time::{self, Timestamp};
use evidence_log::{
    lane_of, merkle, parse_line, render_line, subject_of, Body, Digest, Entry, Kind, SubjectKey,
};
use rusqlite::types::ValueRef;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::api::{CheckpointInfo, Head, Receipt, StoreError, StoredAttestation, StoredObject};
use crate::intake::{IntakeState, NewEntry, NewItem, Plan, Tip};

// ---------------------------------------------------------------------------
// Columns

/// A `TEXT` column, lossily decoded.
fn text(row: &Row<'_>, i: usize) -> rusqlite::Result<String> {
    match row.get_ref(i)? {
        ValueRef::Text(bytes) => Ok(String::from_utf8_lossy(bytes).into_owned()),
        other => Err(rusqlite::Error::InvalidColumnType(
            i,
            "a TEXT column".into(),
            other.data_type(),
        )),
    }
}

/// A `TEXT` column of a `LEFT JOIN`, or None where the join found no row.
fn text_opt(row: &Row<'_>, i: usize) -> rusqlite::Result<Option<String>> {
    match row.get_ref(i)? {
        ValueRef::Null => Ok(None),
        _ => text(row, i).map(Some),
    }
}

/// An `INTEGER` column holding a seq, size or epoch: 0 or more.
fn count(row: &Row<'_>, i: usize) -> rusqlite::Result<u64> {
    let n: i64 = row.get(i)?;
    u64::try_from(n).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(i, n))
}

/// A seq, size or epoch as SQLite stores it. Every one a plan or an import
/// holds is at most 2^53 − 1.
fn int(n: u64) -> Result<i64, StoreError> {
    i64::try_from(n).map_err(|_| StoreError::Io(format!("{n} does not fit in an SQLite integer")))
}

/// A bound for a query: `n` as an SQLite integer, or `i64::MAX` above it.
fn bound(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

// ---------------------------------------------------------------------------
// Reading the log (LogRead)

/// The last entry. `S_IO` if there is none.
pub(crate) fn head(conn: &Connection) -> Result<Head, StoreError> {
    let head = conn
        .prepare("SELECT seq, entry_hash FROM entries ORDER BY seq DESC LIMIT 1")?
        .query_row([], |r| {
            Ok(Head {
                seq: count(r, 0)?,
                entry_hash: text(r, 1)?,
            })
        })
        .optional()?;
    head.ok_or_else(|| StoreError::Io("the log has no entries".into()))
}

/// `(seq, line)` for the entries from `from_seq` on, at most `limit`. A line
/// is rendered from the stored canonical texts; an entry whose object or
/// attestation row is missing gives `{"entry":E}`.
pub(crate) fn lines(
    conn: &Connection,
    from_seq: u64,
    limit: usize,
) -> Result<Vec<(u64, String)>, StoreError> {
    if limit == 0 || from_seq > i64::MAX as u64 {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT e.seq, e.kind, e.entry, o.bytes, a.line FROM entries AS e \
         LEFT JOIN objects AS o ON o.seq = e.seq \
         LEFT JOIN attestations AS a ON a.seq = e.seq \
         WHERE e.seq >= ?1 ORDER BY e.seq LIMIT ?2",
    )?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = stmt.query_map(params![bound(from_seq), limit], |r| {
        let seq = count(r, 0)?;
        let kind = text(r, 1)?;
        let entry = text(r, 2)?;
        let body = match kind.as_str() {
            "object" => text_opt(r, 3)?.map(|o| (Kind::Object, o)),
            "attestation" => text_opt(r, 4)?.map(|a| (Kind::Attestation, a)),
            _ => None,
        };
        let line = match body {
            Some((kind, body)) => render_line(&entry, kind, &body),
            None => format!("{{\"entry\":{entry}}}"),
        };
        Ok((seq, line))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The held object with this id and its entry's `received_at`.
pub(crate) fn object(conn: &Connection, id: &str) -> Result<Option<StoredObject>, StoreError> {
    let object = conn
        .prepare(
            "SELECT o.id, o.seq, e.received_at, o.bytes FROM objects AS o \
             JOIN entries AS e ON e.seq = o.seq WHERE o.id = ?1",
        )?
        .query_row([id], |r| {
            Ok(StoredObject {
                id: text(r, 0)?,
                seq: count(r, 1)?,
                received_at: text(r, 2)?,
                bytes: text(r, 3)?,
            })
        })
        .optional()?;
    Ok(object)
}

/// The attestations of an object, in seq order.
pub(crate) fn attestations(
    conn: &Connection,
    object_id: &str,
) -> Result<Vec<StoredAttestation>, StoreError> {
    let mut stmt =
        conn.prepare("SELECT hash, seq, line FROM attestations WHERE object = ?1 ORDER BY seq")?;
    let rows = stmt.query_map([object_id], |r| {
        Ok(StoredAttestation {
            hash: text(r, 0)?,
            seq: count(r, 1)?,
            line: text(r, 2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// `heads(subject, lane)`, in byte order of the ids (SQLite's `BINARY`
/// collation compares bytes).
pub(crate) fn heads(
    conn: &Connection,
    subject: &SubjectKey,
    lane: &str,
) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id FROM heads WHERE system = ?1 AND type = ?2 AND key = ?3 AND lane = ?4 \
         ORDER BY id",
    )?;
    let rows = stmt.query_map(
        params![subject.system, subject.r#type, subject.key, lane],
        |r| text(r, 0),
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every heads row, sorted as `evidence_log::Heads::rows` sorts them.
pub(crate) fn head_rows(
    conn: &Connection,
) -> Result<Vec<(SubjectKey, String, String)>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT system, type, key, lane, id FROM heads ORDER BY system, type, key, lane, id",
    )?;
    let rows = stmt.query_map([], |r| {
        let subject = SubjectKey {
            system: text(r, 0)?,
            r#type: text(r, 1)?,
            key: text(r, 2)?,
        };
        Ok((subject, text(r, 3)?, text(r, 4)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn checkpoint_row(r: &Row<'_>) -> rusqlite::Result<CheckpointInfo> {
    Ok(CheckpointInfo {
        seq: count(r, 0)?,
        id: text(r, 1)?,
        size: count(r, 2)?,
        root: text(r, 3)?,
        head: text(r, 4)?,
    })
}

/// Every checkpoints row, in seq order.
pub(crate) fn checkpoints(conn: &Connection) -> Result<Vec<CheckpointInfo>, StoreError> {
    let mut stmt =
        conn.prepare("SELECT seq, id, size, root, head FROM checkpoints ORDER BY seq")?;
    let rows = stmt.query_map([], checkpoint_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The checkpoints row with the highest seq.
pub(crate) fn latest_checkpoint(conn: &Connection) -> Result<Option<CheckpointInfo>, StoreError> {
    let row = conn
        .prepare("SELECT seq, id, size, root, head FROM checkpoints ORDER BY seq DESC LIMIT 1")?
        .query_row([], checkpoint_row)
        .optional()?;
    Ok(row)
}

/// The leaf hashes of the entries with seq 1 to `upto`, over the stored
/// canonical entry bytes. `E_USAGE` above the highest seq; `S_IO` if a seq
/// below it is missing (a damaged store).
pub(crate) fn leaf_hashes(conn: &Connection, upto: u64) -> Result<Vec<Digest>, StoreError> {
    let top: Option<i64> = conn.query_row("SELECT max(seq) FROM entries", [], |r| r.get(0))?;
    let top = top.map_or(0, |n| u64::try_from(n).unwrap_or(0));
    if upto > top {
        return Err(StoreError::Usage(format!(
            "seq {upto} is above the head, seq {top}"
        )));
    }
    let mut stmt = conn.prepare("SELECT seq, entry FROM entries WHERE seq <= ?1 ORDER BY seq")?;
    let mut rows = stmt.query([bound(upto)])?;
    let mut leaves = Vec::new();
    let mut expected = 1;
    while let Some(r) = rows.next()? {
        let seq = count(r, 0)?;
        if seq != expected {
            return Err(StoreError::Io(format!(
                "seq {expected} is missing from the log (a damaged store)"
            )));
        }
        let leaf = match r.get_ref(1)? {
            ValueRef::Text(bytes) => merkle::leaf_hash(bytes),
            other => {
                return Err(rusqlite::Error::InvalidColumnType(
                    1,
                    "entry".into(),
                    other.data_type(),
                )
                .into())
            }
        };
        leaves.push(leaf);
        expected += 1;
    }
    if expected != upto + 1 {
        return Err(StoreError::Io(format!(
            "seq {expected} is missing from the log (a damaged store)"
        )));
    }
    Ok(leaves)
}

/// The `at` of a held object, parsed. `S_IO` if the stored `at` does not
/// parse (a damaged store).
pub(crate) fn object_at(conn: &Connection, id: &str) -> Result<Option<Timestamp>, StoreError> {
    let at: Option<String> = conn
        .prepare("SELECT at FROM objects WHERE id = ?1")?
        .query_row([id], |r| text(r, 0))
        .optional()?;
    at.map(|at| time::parse(&at).map_err(|_| StoreError::Io(format!("object {id} has at {at:?}"))))
        .transpose()
}

/// One `meta` value, or None.
pub(crate) fn meta(conn: &Connection, key: &str) -> Result<Option<String>, StoreError> {
    let v = conn
        .prepare("SELECT v FROM meta WHERE k = ?1")?
        .query_row([key], |r| text(r, 0))
        .optional()?;
    Ok(v)
}

/// One row of `sqlite_master`, without its root page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SchemaRow {
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) table: String,
    pub(crate) sql: Option<String>,
}

/// The `sqlite_master` rows that are triggers (`triggers` true) or that are
/// not (tables, indexes and views), sorted by type and name. Reading them
/// reads no table and runs no view.
pub(crate) fn schema_rows(conn: &Connection, triggers: bool) -> rusqlite::Result<Vec<SchemaRow>> {
    let mut stmt = conn.prepare(
        "SELECT type, name, tbl_name, sql FROM sqlite_master \
         WHERE (type = 'trigger') = ?1 ORDER BY type, name",
    )?;
    let rows = stmt.query_map([triggers], |r| {
        Ok(SchemaRow {
            kind: text(r, 0)?,
            name: text(r, 1)?,
            table: text(r, 2)?,
            sql: text_opt(r, 3)?,
        })
    })?;
    rows.collect()
}

/// The epoch of the last entry, or None if there is none.
pub(crate) fn last_epoch(conn: &Connection) -> Result<Option<u64>, StoreError> {
    let epoch = conn
        .prepare("SELECT epoch FROM entries ORDER BY seq DESC LIMIT 1")?
        .query_row([], |r| count(r, 0))
        .optional()?;
    Ok(epoch)
}

/// What [`derived`] finds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Derived {
    /// Whether anything the store keeps beside the canonical texts differs
    /// from what the lines give (see [`derived`]).
    pub(crate) drift: bool,
    /// The highest `epoch` of the lines that parse, if any does.
    pub(crate) max_epoch: Option<u64>,
}

/// The columns of one entry row and the object and attestation rows at its
/// seq, as [`derived`] reads them.
struct Rows {
    seq: i64,
    kind: String,
    reference: String,
    received_at: String,
    epoch: i64,
    entry_hash: String,
    entry: String,
    /// id, at, op, system, type, key, lane, bytes.
    object: Option<[String; 8]>,
    /// hash, object, purpose, signer, key, at, line.
    attestation: Option<[String; 7]>,
}

/// What `verify_store` checks in the tables beside the lines and the
/// projections. Everything the store reads to answer a query or to plan a
/// write, other than the canonical texts the lines are rendered from, must
/// be what the lines give, or a store that verifies could still answer
/// wrongly, or lead its writer to append lines that never verify. `drift`
/// is set when:
///
/// 1. SQLite's `PRAGMA integrity_check` finds a fault (an index that
///    differs from its table, say, so that a lookup by id or ref misses a
///    row a scan finds). The rows are then not compared.
/// 2. For a line that parses (one that does not is a finding of its own):
///    a column of its `entries` row (`seq`, `kind`, `ref`, `received_at`,
///    `epoch`, `entry_hash`, and `entry`, which must be the line's
///    canonical entry, not merely render to the same line) differs from
///    the line; or a column of its `objects` row (`id`, `at`, `op`,
///    `system`, `type`, `key`, `lane`, `bytes`) or `attestations` row
///    (`hash`, `object`, `purpose`, `signer`, `key`, `at`, `line`) differs
///    from its body.
/// 3. An `objects` or `attestations` row is not at the seq of an entry of
///    its kind: a row that no line shows, which a lookup would still find.
pub(crate) fn derived(conn: &Connection) -> Result<Derived, StoreError> {
    let integrity: String = conn.query_row("PRAGMA integrity_check(1)", [], |r| text(r, 0))?;
    if integrity != "ok" {
        return Ok(Derived {
            drift: true,
            max_epoch: None,
        });
    }
    let mut found = Derived::default();
    let mut stmt = conn.prepare(
        "SELECT e.seq, e.kind, e.ref, e.received_at, e.epoch, e.entry_hash, e.entry, \
         o.id, o.at, o.op, o.system, o.type, o.key, o.lane, o.bytes, \
         a.hash, a.object, a.purpose, a.signer, a.key, a.at, a.line \
         FROM entries AS e LEFT JOIN objects AS o ON o.seq = e.seq \
         LEFT JOIN attestations AS a ON a.seq = e.seq ORDER BY e.seq",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let columns = |from: usize, n: usize| -> rusqlite::Result<Option<Vec<String>>> {
            if matches!(r.get_ref(from)?, ValueRef::Null) {
                return Ok(None);
            }
            (from..from + n)
                .map(|i| text(r, i))
                .collect::<rusqlite::Result<_>>()
                .map(Some)
        };
        let found_rows = Rows {
            seq: r.get(0)?,
            kind: text(r, 1)?,
            reference: text(r, 2)?,
            received_at: text(r, 3)?,
            epoch: r.get(4)?,
            entry_hash: text(r, 5)?,
            entry: text(r, 6)?,
            object: columns(7, 8)?.and_then(|v| v.try_into().ok()),
            attestation: columns(15, 7)?.and_then(|v| v.try_into().ok()),
        };
        if !rows_match(&found_rows, &mut found.max_epoch) {
            found.drift = true;
        }
    }
    let strays: i64 = conn.query_row(
        "SELECT (SELECT count(*) FROM objects WHERE seq NOT IN \
           (SELECT seq FROM entries WHERE kind = 'object')) \
         + (SELECT count(*) FROM attestations WHERE seq NOT IN \
           (SELECT seq FROM entries WHERE kind = 'attestation'))",
        [],
        |r| r.get(0),
    )?;
    found.drift |= strays > 0;
    Ok(found)
}

/// Step 2 of [`derived`] for one entry: false if a column differs from the
/// line it renders. A line that does not parse is left to the verifier.
/// Raises `max_epoch` to the line's epoch.
fn rows_match(rows: &Rows, max_epoch: &mut Option<u64>) -> bool {
    let (kind, body) = match (rows.kind.as_str(), &rows.object, &rows.attestation) {
        ("object", Some(o), None) => (Kind::Object, &o[7]),
        ("attestation", None, Some(a)) => (Kind::Attestation, &a[6]),
        // An item row of the other kind at this seq belongs to no line.
        ("object", _, Some(_)) | ("attestation", Some(_), _) => return false,
        // No item row: the line is {"entry":E}, which the verifier finds.
        _ => return true,
    };
    let Ok(line) = parse_line(render_line(&rows.entry, kind, body).as_bytes()) else {
        return true;
    };
    let e = &line.entry;
    *max_epoch = Some(max_epoch.map_or(e.epoch, |m| m.max(e.epoch)));
    let entry_ok = u64::try_from(rows.seq) == Ok(e.seq)
        && rows.kind == e.kind.as_str()
        && rows.reference == e.reference
        && rows.received_at == e.received_at
        && u64::try_from(rows.epoch) == Ok(e.epoch)
        && rows.entry_hash == canon::sha256_hex(line.entry_canonical.as_bytes())
        && rows.entry == line.entry_canonical;
    let member = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    let item_ok = match (&line.body, &rows.object, &rows.attestation) {
        (Body::Object(v), Some([id, at, op, system, r#type, key, lane, bytes]), _) => {
            let subject = subject_of(v);
            *id == e.reference
                && Some(at) == member(v, "at").as_ref()
                && Some(op) == member(v, "op").as_ref()
                && (system, r#type, key) == (&subject.system, &subject.r#type, &subject.key)
                && *lane == lane_of(v)
                && *bytes == canon::canonicalize(v)
        }
        (Body::Attestation(v), _, Some([hash, object, purpose, signer, key, at, text])) => {
            *hash == e.reference
                && Some(object) == member(v, "object").as_ref()
                && Some(purpose) == member(v, "purpose").as_ref()
                && Some(signer) == member(v, "signer").as_ref()
                && Some(key) == member(v, "key").as_ref()
                && Some(at) == member(v, "at").as_ref()
                && *text == canon::canonicalize(v)
        }
        _ => false,
    };
    entry_ok && item_ok
}

// ---------------------------------------------------------------------------
// The writer's view (IntakeState)

/// A planner's view of a store, over the writer's open `BEGIN IMMEDIATE`
/// transaction. Every read error is returned, never read as "not held".
pub(crate) struct State<'a>(pub(crate) &'a Connection);

impl IntakeState for State<'_> {
    /// The last entry's `seq`, `entry_hash` and `received_at`, which the
    /// next entry chains to and dates from. `S_IO` unless they are those of
    /// the entry text stored beside them (its hash, and its own `seq` and
    /// `received_at`): a damaged or edited row would otherwise be carried
    /// into every later entry.
    fn tip(&self) -> Result<Tip, StoreError> {
        let tip = self
            .0
            .prepare(
                "SELECT seq, entry_hash, received_at, entry FROM entries \
                 ORDER BY seq DESC LIMIT 1",
            )?
            .query_row([], |r| {
                Ok((count(r, 0)?, text(r, 1)?, text(r, 2)?, text(r, 3)?))
            })
            .optional()?;
        let (seq, entry_hash, received_at, entry) =
            tip.ok_or_else(|| StoreError::Io("the log has no entries".into()))?;
        let matches = Entry::parse(entry.as_bytes()).is_ok_and(|e| {
            e.seq == seq
                && e.received_at == received_at
                && canon::sha256_hex(entry.as_bytes()) == entry_hash
        });
        if !matches {
            return Err(crate::api::damaged(format!(
                "the entry row of seq {seq} does not match its entry text"
            )));
        }
        let parsed = time::parse(&received_at)
            .map_err(|_| StoreError::Io(format!("seq {seq} has received_at {received_at:?}")))?;
        Ok(Tip {
            seq,
            entry_hash,
            received_at: parsed,
        })
    }

    fn receipt(&self, reference: &str) -> Result<Option<Receipt>, StoreError> {
        let row = self
            .0
            .prepare("SELECT kind, seq, entry_hash, received_at FROM entries WHERE ref = ?1")?
            .query_row([reference], |r| {
                Ok((text(r, 0)?, count(r, 1)?, text(r, 2)?, text(r, 3)?))
            })
            .optional()?;
        let Some((kind, seq, entry_hash, received_at)) = row else {
            return Ok(None);
        };
        let kind = Kind::parse(&kind)
            .ok_or_else(|| StoreError::Io(format!("seq {seq} has kind {kind:?}")))?;
        Ok(Some(Receipt {
            kind,
            reference: reference.to_string(),
            seq,
            entry_hash,
            received_at,
            duplicate: false,
        }))
    }

    fn object_at(&self, id: &str) -> Result<Option<Timestamp>, StoreError> {
        object_at(self.0, id)
    }

    fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError> {
        heads(self.0, subject, lane)
    }

    fn latest_checkpoint(&self) -> Result<Option<CheckpointInfo>, StoreError> {
        latest_checkpoint(self.0)
    }

    fn root(&self, size: u64) -> Result<Digest, StoreError> {
        Ok(merkle::root(&leaf_hashes(self.0, size)?))
    }
}

// ---------------------------------------------------------------------------
// Writing

/// Insert one `meta` row.
pub(crate) fn insert_meta(conn: &Connection, key: &str, value: &str) -> Result<(), StoreError> {
    conn.execute("INSERT INTO meta (k, v) VALUES (?1, ?2)", [key, value])?;
    Ok(())
}

/// The statements that write a plan's rows, prepared once per transaction.
pub(crate) struct Writer<'a> {
    entry: rusqlite::Statement<'a>,
    object: rusqlite::Statement<'a>,
    attestation: rusqlite::Statement<'a>,
    unhead: rusqlite::Statement<'a>,
    head: rusqlite::Statement<'a>,
    checkpoint: rusqlite::Statement<'a>,
}

impl<'a> Writer<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Result<Writer<'a>, StoreError> {
        Ok(Writer {
            entry: conn.prepare(
                "INSERT INTO entries (seq, kind, ref, received_at, epoch, entry_hash, entry) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?,
            object: conn.prepare(
                "INSERT INTO objects (id, seq, at, op, system, type, key, lane, bytes) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?,
            attestation: conn.prepare(
                "INSERT INTO attestations (hash, object, seq, purpose, signer, key, at, line) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?,
            unhead: conn.prepare(
                "DELETE FROM heads WHERE system = ?1 AND type = ?2 AND key = ?3 AND lane = ?4 \
                 AND id = ?5",
            )?,
            head: conn.prepare(
                "INSERT INTO heads (system, type, key, lane, id) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?,
            checkpoint: conn.prepare(
                "INSERT INTO checkpoints (seq, id, size, root, head) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?,
        })
    }

    /// A plan, in order: each entry, its object or attestation row and an
    /// object's heads change; then the checkpoints row.
    pub(crate) fn apply(&mut self, plan: &Plan) -> Result<(), StoreError> {
        for new in &plan.entries {
            self.insert(new)?;
            if let NewItem::Object(o) = &new.item {
                for p in &o.prev {
                    let s = &o.subject;
                    self.unhead
                        .execute(params![s.system, s.r#type, s.key, o.lane, p])?;
                }
                self.insert_head(&o.subject, &o.lane, &o.id)?;
            }
        }
        if let Some(c) = &plan.checkpoint {
            self.insert_checkpoint(c)?;
        }
        Ok(())
    }

    /// One entry and its object or attestation row, without the heads
    /// change.
    pub(crate) fn insert(&mut self, new: &NewEntry) -> Result<(), StoreError> {
        let e = &new.entry;
        let seq = int(e.seq)?;
        self.entry.execute(params![
            seq,
            e.kind.as_str(),
            e.reference,
            e.received_at,
            int(e.epoch)?,
            new.hash,
            new.canonical,
        ])?;
        match &new.item {
            NewItem::Object(o) => {
                let s = &o.subject;
                self.object.execute(params![
                    o.id, seq, o.at, o.op, s.system, s.r#type, s.key, o.lane, o.bytes,
                ])?;
            }
            NewItem::Attestation(a) => {
                self.attestation.execute(params![
                    a.hash, a.object, seq, a.purpose, a.signer, a.key, a.at, a.line,
                ])?;
            }
        }
        Ok(())
    }

    /// One heads row.
    pub(crate) fn insert_head(
        &mut self,
        subject: &SubjectKey,
        lane: &str,
        id: &str,
    ) -> Result<(), StoreError> {
        let s = subject;
        self.head
            .execute(params![s.system, s.r#type, s.key, lane, id])?;
        Ok(())
    }

    /// One checkpoints row.
    pub(crate) fn insert_checkpoint(&mut self, c: &CheckpointInfo) -> Result<(), StoreError> {
        self.checkpoint
            .execute(params![int(c.seq)?, c.id, int(c.size)?, c.root, c.head])?;
        Ok(())
    }
}
