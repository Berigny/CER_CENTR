//! The verification pipeline. Stops at the first failure and reports one code.
//!
//! 1. parse            E_PARSE, E_DUPLICATE_KEY, E_NUMBER_RANGE
//! 2. shape            E_SHAPE
//! 3. timestamp        E_TIMESTAMP
//! 4. id               E_ID_MISMATCH
//! 5. inline content   E_CONTENT_HASH_MISMATCH
//! 6. prev (if store)  E_PREV_MISSING, E_PREV_ORDER
//! 7. attestations     E_ATTESTATION_SHAPE, E_SIGNATURE_INVALID
//! 8. required purposes E_ATTESTATION_MISSING

use std::collections::{BTreeMap, HashMap};

use crate::attest;
use crate::error::{Code, Error};
use crate::json::Value;
use crate::object;
use crate::time::{self, Timestamp};

/// Objects that `prev` hashes may point at, indexed by verified id.
#[derive(Debug, Default)]
pub struct Store {
    by_id: HashMap<String, Timestamp>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an object after checking it (steps 1-5). Returns its id.
    pub fn insert(&mut self, bytes: &[u8]) -> Result<String, Error> {
        let checked = object::check(bytes)?;
        self.by_id.insert(checked.id.clone(), checked.at);
        Ok(checked.id)
    }

    pub fn at(&self, id: &str) -> Option<Timestamp> {
        self.by_id.get(id).copied()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// Where step 6 looks up the claimed time of a `prev` object.
///
/// [`Store`] is one lookup; a persistent store of verified objects can be
/// another, so step 6 runs the same way whatever holds the objects.
pub trait PrevLookup {
    /// `at` of a verified object with this id, or None if it is not held.
    fn prev_at(&self, id: &str) -> Option<Timestamp>;
}

impl PrevLookup for Store {
    fn prev_at(&self, id: &str) -> Option<Timestamp> {
        self.at(id)
    }
}

/// Step 6 on its own: every prev resolves, and none is dated after `checked.at`.
/// Returns E_PREV_MISSING or E_PREV_ORDER exactly as `verify` does.
pub fn check_prev(checked: &object::Checked, lookup: &dyn PrevLookup) -> Result<(), Error> {
    for p in &checked.prev {
        let at = lookup.prev_at(p).ok_or_else(|| {
            Error::new(Code::PrevMissing, format!("prev {p} is not in the store"))
        })?;
        if at > checked.at {
            return Err(Error::new(
                Code::PrevOrder,
                format!(
                    "prev {p} is dated {} but this object is dated {}",
                    time::format_secs(at.secs),
                    time::format_secs(checked.at.secs)
                ),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct Options<'a> {
    /// Attestation lines (JSON Lines). Lines for other objects are ignored.
    pub attestations: Option<&'a [u8]>,
    /// When given, every `prev` hash must resolve in the store.
    pub store: Option<&'a Store>,
    /// Attestation purposes that must be present with a valid signature.
    pub require: &'a [String],
}

/// [`Options`] with step 6 run against any [`PrevLookup`], not only a [`Store`].
#[derive(Default)]
pub struct LookupOptions<'a> {
    /// Attestation lines (JSON Lines). Lines for other objects are ignored.
    pub attestations: Option<&'a [u8]>,
    /// When given, every `prev` hash must resolve through this lookup.
    pub prev: Option<&'a dyn PrevLookup>,
    /// Attestation purposes that must be present with a valid signature.
    pub require: &'a [String],
}

#[derive(Debug, Default)]
pub struct Report {
    pub id: Option<String>,
    pub error: Option<Error>,
    pub content_checked: bool,
    /// Valid attestations for this object, by purpose.
    pub attestations: BTreeMap<String, usize>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }

    pub fn to_value(&self) -> Value {
        let mut m = vec![("ok".to_string(), Value::Bool(self.ok()))];
        if let Some(id) = &self.id {
            m.push(("id".into(), Value::String(id.clone())));
        }
        if let Some(e) = &self.error {
            m.push(("code".into(), Value::String(e.code.as_str().into())));
            m.push(("detail".into(), Value::String(e.detail.clone())));
        }
        m.push(("content_checked".into(), Value::Bool(self.content_checked)));
        m.push((
            "attestations".into(),
            Value::Object(
                self.attestations
                    .iter()
                    .map(|(k, n)| (k.clone(), Value::Number(*n as f64)))
                    .collect(),
            ),
        ));
        Value::Object(m)
    }
}

pub fn verify(bytes: &[u8], opts: &Options) -> Report {
    verify_lookup(
        bytes,
        &LookupOptions {
            attestations: opts.attestations,
            prev: opts.store.map(|s| s as &dyn PrevLookup),
            require: opts.require,
        },
    )
}

/// The same pipeline as `verify`, with step 6 run against any lookup.
pub fn verify_lookup(bytes: &[u8], opts: &LookupOptions) -> Report {
    let mut report = Report::default();
    if let Err(e) = run(bytes, opts, &mut report) {
        report.error = Some(e);
    }
    report
}

fn run(bytes: &[u8], opts: &LookupOptions, report: &mut Report) -> Result<(), Error> {
    let parsed = crate::json::parse(bytes)?;
    if let Some(id) = parsed.get("id").and_then(Value::as_str) {
        report.id = Some(id.to_string());
    }
    let checked = object::check_value(parsed)?;
    report.content_checked = checked.content_checked;

    if let Some(lookup) = opts.prev {
        check_prev(&checked, lookup)?;
    }

    if let Some(lines) = opts.attestations {
        for (n, line) in lines.split(|b| *b == b'\n').enumerate() {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let att = attest::parse(line).map_err(|e| {
                Error::new(
                    Code::AttestationShape,
                    format!("attestation line {}: {}", n + 1, e.detail),
                )
            })?;
            if att.object != checked.id {
                continue;
            }
            att.verify_signature()?;
            *report.attestations.entry(att.purpose.clone()).or_default() += 1;
        }
    }

    for purpose in opts.require {
        if !report.attestations.contains_key(purpose) {
            return Err(Error::new(
                Code::AttestationMissing,
                format!("no valid {purpose:?} attestation for {}", checked.id),
            ));
        }
    }
    Ok(())
}
