//! T13: `verify_lookup` is the same pipeline as `verify`. For every object
//! vector, running step 6 through a [`PrevLookup`] gives a report byte for byte
//! identical to the one `verify` gives with a [`Store`], whether the lookup is
//! the `Store` itself or some other holder of the same objects.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use evidence_core::canon::canonicalize;
use evidence_core::json::{self, Value};
use evidence_core::object;
use evidence_core::time::Timestamp;
use evidence_core::verify::{
    check_prev, verify, verify_lookup, LookupOptions, Options, PrevLookup, Report, Store,
};
use evidence_core::Code;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors")
}

fn load(path: &Path) -> Value {
    json::parse(&fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A lookup that is not a `Store`: verified objects kept in a sorted map.
#[derive(Default)]
struct MapLookup {
    by_id: BTreeMap<String, Timestamp>,
}

impl MapLookup {
    fn insert(&mut self, bytes: &[u8]) {
        let checked = object::check(bytes).expect("store objects are valid");
        self.by_id.insert(checked.id, checked.at);
    }
}

impl PrevLookup for MapLookup {
    fn prev_at(&self, id: &str) -> Option<Timestamp> {
        self.by_id.get(id).copied()
    }
}

/// Everything a report carries: its canonical JSON (which includes the
/// detail text) and its debug form.
fn bytes_of(report: &Report) -> String {
    format!("{}\n{report:?}", canonicalize(&report.to_value()))
}

struct Case {
    name: String,
    object: Vec<u8>,
    attestations: Option<Vec<u8>>,
    store: Option<Vec<Vec<u8>>>,
    require: Vec<String>,
}

fn cases() -> Vec<Case> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root().join("objects"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    dirs.sort();
    dirs.iter()
        .map(|dir| {
            let case = load(&dir.join("case.json"));
            let store = (case.get("store") == Some(&Value::Bool(true))).then(|| {
                let mut files: Vec<PathBuf> = fs::read_dir(dir.join("store"))
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .collect();
                files.sort();
                files.iter().map(|f| fs::read(f).unwrap()).collect()
            });
            Case {
                name: dir.file_name().unwrap().to_string_lossy().to_string(),
                object: fs::read(dir.join("object.json")).unwrap(),
                attestations: fs::read(dir.join("attestations.jsonl")).ok(),
                store,
                require: case
                    .get("require")
                    .and_then(Value::as_array)
                    .unwrap_or(&[])
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect(),
            }
        })
        .collect()
}

#[test]
fn verify_lookup_matches_verify_on_every_object_vector() {
    let cases = cases();
    assert!(!cases.is_empty(), "no object vectors found");
    let mut failures = Vec::new();
    let mut with_store = 0;
    for c in &cases {
        let store = c.store.as_ref().map(|objects| {
            let mut store = Store::new();
            for bytes in objects {
                store.insert(bytes).unwrap();
            }
            store
        });
        let map = c.store.as_ref().map(|objects| {
            let mut map = MapLookup::default();
            for bytes in objects {
                map.insert(bytes);
            }
            map
        });
        with_store += usize::from(store.is_some());

        let expected = bytes_of(&verify(
            &c.object,
            &Options {
                attestations: c.attestations.as_deref(),
                store: store.as_ref(),
                require: &c.require,
            },
        ));
        let through_store = bytes_of(&verify_lookup(
            &c.object,
            &LookupOptions {
                attestations: c.attestations.as_deref(),
                prev: store.as_ref().map(|s| s as &dyn PrevLookup),
                require: &c.require,
            },
        ));
        let through_map = bytes_of(&verify_lookup(
            &c.object,
            &LookupOptions {
                attestations: c.attestations.as_deref(),
                prev: map.as_ref().map(|m| m as &dyn PrevLookup),
                require: &c.require,
            },
        ));
        if through_store != expected {
            failures.push(format!(
                "{}: Store lookup\n    {through_store}\n  want\n    {expected}",
                c.name
            ));
        }
        if through_map != expected {
            failures.push(format!(
                "{}: map lookup\n    {through_map}\n  want\n    {expected}",
                c.name
            ));
        }
    }
    assert!(with_store > 0, "no object vector uses a store");
    assert!(
        failures.is_empty(),
        "{} report(s) differ:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

#[test]
fn check_prev_alone_gives_the_step_6_codes() {
    let cases = cases();
    let by_name = |n: &str| {
        cases
            .iter()
            .find(|c| c.name.starts_with(n))
            .unwrap_or_else(|| panic!("vector {n} not found"))
    };
    let held = |c: &Case| {
        let mut store = Store::new();
        for bytes in c.store.as_ref().unwrap() {
            store.insert(bytes).unwrap();
        }
        store
    };

    // 002 lists a prev that its store holds, dated before it.
    let ok = by_name("002-");
    let checked = object::check(&ok.object).unwrap();
    assert!(!checked.prev.is_empty());
    assert_eq!(check_prev(&checked, &held(ok)), Ok(()));
    // The same object against an empty lookup: its prev is missing.
    let err = check_prev(&checked, &Store::new()).unwrap_err();
    assert_eq!(err.code, Code::PrevMissing);

    for (name, code) in [("110-", Code::PrevMissing), ("111-", Code::PrevOrder)] {
        let c = by_name(name);
        let checked = object::check(&c.object).unwrap();
        let err = check_prev(&checked, &held(c)).unwrap_err();
        assert_eq!(err.code, code, "{}", c.name);
        // The error is exactly the one the full pipeline reports.
        let report = verify(
            &c.object,
            &Options {
                store: Some(&held(c)),
                ..Default::default()
            },
        );
        assert_eq!(report.error, Some(err), "{}", c.name);
    }

    // An object with no prev passes against any lookup.
    let first = by_name("001-");
    let checked = object::check(&first.object).unwrap();
    assert!(checked.prev.is_empty());
    assert_eq!(check_prev(&checked, &MapLookup::default()), Ok(()));
}
