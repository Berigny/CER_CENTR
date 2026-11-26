//! Subject keys, lanes and heads. See `spec/store-0.md`, "Lanes and heads".
//!
//! Every object belongs to one lane of its subject. `heads(S, L)` is the set
//! of objects with subject key S and lane L that no object with subject key S
//! and lane L lists in its `prev`. Every `prev` of an accepted object is held
//! at a lower seq, so heads are kept by a fold in seq order: for each object,
//! remove its `prev` ids from `heads(S, L)` and add its own id.

use std::collections::{BTreeMap, BTreeSet};

use evidence_core::json::Value;

/// The lane of record changes (`create`, `update`, `delete`) and of capture's
/// baseline observations.
pub const RECORD: &str = "record";

/// The `ext` member that puts an `assert` in the record lane. Reserved for
/// capture's baseline observations, so heads stay valid when capture lands.
pub const CAPTURE_EXT: &str = "capture/0";

/// The prefix of every assert lane: `"assert:"` + `schema.ref` up to its first `@`.
pub const ASSERT_PREFIX: &str = "assert:";

/// (`subject.system`, `subject.type`, `subject.key`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubjectKey {
    pub system: String,
    pub r#type: String,
    pub key: String,
}

fn text<'a>(v: Option<&'a Value>, key: &str) -> &'a str {
    v.and_then(|v| v.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// The subject key of an object. Meant for objects that passed
/// `evidence_core::object::check`; on anything else a missing member reads
/// as the empty string.
pub fn subject_of(object: &Value) -> SubjectKey {
    let subject = object.get("subject");
    SubjectKey {
        system: text(subject, "system").to_string(),
        r#type: text(subject, "type").to_string(),
        key: text(subject, "key").to_string(),
    }
}

/// The lane of an object:
///
/// - `"record"` if `op` is `create`, `update` or `delete`;
/// - `"record"` if `op` is `assert` and `ext` has a `"capture/0"` member;
/// - otherwise `"assert:"` + `schema.ref` up to, not including, the first `@`
///   (the whole ref if it has no `@`).
pub fn lane_of(object: &Value) -> String {
    let op = text(Some(object), "op");
    let captured = op == "assert"
        && object
            .get("ext")
            .and_then(|ext| ext.get(CAPTURE_EXT))
            .is_some();
    if matches!(op, "create" | "update" | "delete") || captured {
        return RECORD.to_string();
    }
    let schema_ref = text(object.get("schema"), "ref");
    let name = schema_ref.split('@').next().unwrap_or(schema_ref);
    format!("{ASSERT_PREFIX}{name}")
}

/// The heads of every (subject key, lane), kept by a fold in seq order.
#[derive(Default, Clone, PartialEq, Eq, Debug)]
pub struct Heads {
    by_lane: BTreeMap<(SubjectKey, String), BTreeSet<String>>,
}

impl Heads {
    /// Fold one object in: remove its `prev` ids from the heads of its own
    /// subject key and lane, then add its id. A `prev` that points at another
    /// subject key or lane changes nothing.
    pub fn apply(&mut self, id: &str, subject: &SubjectKey, lane: &str, prev: &[String]) {
        let heads = self
            .by_lane
            .entry((subject.clone(), lane.to_string()))
            .or_default();
        for p in prev {
            heads.remove(p);
        }
        heads.insert(id.to_string());
    }

    /// The heads of one subject key and lane, sorted. Empty if it has none.
    pub fn get(&self, subject: &SubjectKey, lane: &str) -> Vec<String> {
        self.by_lane
            .get(&(subject.clone(), lane.to_string()))
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// The number of subjects whose record lane has more than one head.
    pub fn record_forks(&self) -> u64 {
        self.by_lane
            .iter()
            .filter(|((_, lane), ids)| lane == RECORD && ids.len() > 1)
            .count() as u64
    }

    /// Every head as (subject key, lane, id), sorted by all three: the rows a
    /// store's heads table should hold.
    pub fn rows(&self) -> Vec<(SubjectKey, String, String)> {
        self.by_lane
            .iter()
            .flat_map(|((subject, lane), ids)| {
                ids.iter()
                    .map(move |id| (subject.clone(), lane.clone(), id.clone()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evidence_core::json;

    fn obj(text: &str) -> Value {
        json::parse(text.as_bytes()).unwrap()
    }

    fn subject(key: &str) -> SubjectKey {
        SubjectKey {
            system: "rec-registry".into(),
            r#type: "installation".into(),
            key: key.into(),
        }
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn subject_of_reads_the_triple() {
        let o = obj(r#"{"subject":{"system":"s","type":"t","key":"k"}}"#);
        assert_eq!(
            subject_of(&o),
            SubjectKey {
                system: "s".into(),
                r#type: "t".into(),
                key: "k".into()
            }
        );
        // Not an object that passed the checks: no panic, empty parts.
        assert_eq!(
            subject_of(&obj("[]")),
            SubjectKey {
                system: String::new(),
                r#type: String::new(),
                key: String::new()
            }
        );
        assert_eq!(subject_of(&obj(r#"{"subject":{"system":1}}"#)).system, "");
    }

    #[test]
    fn subject_keys_order_by_system_type_key() {
        let a = SubjectKey {
            system: "a".into(),
            r#type: "z".into(),
            key: "z".into(),
        };
        let b = SubjectKey {
            system: "b".into(),
            r#type: "a".into(),
            key: "a".into(),
        };
        assert!(a < b);
        assert!(subject("1") < subject("2"));
    }

    #[test]
    fn lanes() {
        for op in ["create", "update", "delete"] {
            let o = obj(&format!(r#"{{"op":"{op}","schema":{{"ref":"x@1"}}}}"#));
            assert_eq!(lane_of(&o), RECORD, "{op}");
        }
        // A capture/0 baseline observation is a record change.
        let captured = obj(r#"{"op":"assert","schema":{"ref":"x@1"},"ext":{"capture/0":{}}}"#);
        assert_eq!(lane_of(&captured), RECORD);
        // Other profiles do not move an assert.
        let profiled = obj(r#"{"op":"assert","schema":{"ref":"x@1"},"ext":{"centr/0":{}}}"#);
        assert_eq!(lane_of(&profiled), "assert:x");
        // capture/0 on a record change keeps it in the record lane.
        let both = obj(r#"{"op":"update","schema":{"ref":"x@1"},"ext":{"capture/0":{}}}"#);
        assert_eq!(lane_of(&both), RECORD);

        let cases = [
            ("photo-check/result@1", "assert:photo-check/result"),
            ("evidence-store/genesis@0", "assert:evidence-store/genesis"),
            (
                "evidence-store/checkpoint@0",
                "assert:evidence-store/checkpoint",
            ),
            ("no-version", "assert:no-version"),
            ("a@b@c", "assert:a"),
            ("@1", "assert:"),
        ];
        for (schema_ref, lane) in cases {
            let o = obj(&format!(
                r#"{{"op":"assert","schema":{{"ref":"{schema_ref}"}}}}"#
            ));
            assert_eq!(lane_of(&o), lane, "{schema_ref}");
        }
        assert_eq!(
            lane_of(&obj(&format!(
                r#"{{"op":"assert","schema":{{"ref":"{}"}}}}"#,
                crate::schemas::GENESIS_SCHEMA_REF
            ))),
            "assert:evidence-store/genesis"
        );
        // Not an object that passed the checks: no panic.
        assert_eq!(lane_of(&obj("{}")), "assert:");
    }

    #[test]
    fn update_fork_and_merge() {
        let s = subject("A-1");
        let mut h = Heads::default();
        assert!(h.get(&s, RECORD).is_empty());

        h.apply("c1", &s, RECORD, &[]);
        assert_eq!(h.get(&s, RECORD), ids(&["c1"]));
        assert_eq!(h.record_forks(), 0);

        // An update replaces the head.
        h.apply("u1", &s, RECORD, &ids(&["c1"]));
        assert_eq!(h.get(&s, RECORD), ids(&["u1"]));

        // Two updates of the same head fork the subject.
        h.apply("u3", &s, RECORD, &ids(&["u1"]));
        h.apply("u2", &s, RECORD, &ids(&["u1"]));
        assert_eq!(h.get(&s, RECORD), ids(&["u2", "u3"]), "sorted");
        assert_eq!(h.record_forks(), 1);

        // A merge lists both heads and leaves one.
        h.apply("m", &s, RECORD, &ids(&["u2", "u3"]));
        assert_eq!(h.get(&s, RECORD), ids(&["m"]));
        assert_eq!(h.record_forks(), 0);

        // A second create without prev also forks; a delete of one head
        // still leaves two.
        h.apply("c2", &s, RECORD, &[]);
        h.apply("d", &s, RECORD, &ids(&["c2"]));
        assert_eq!(h.get(&s, RECORD), ids(&["d", "m"]));
        assert_eq!(h.record_forks(), 1);
    }

    #[test]
    fn prev_in_another_subject_or_lane_changes_nothing() {
        let (a, b) = (subject("A"), subject("B"));
        let mut h = Heads::default();
        h.apply("a1", &a, RECORD, &[]);
        h.apply("b1", &b, RECORD, &[]);
        h.apply("r1", &a, "assert:review", &[]);

        // An assert about a's record points at it, but lives in its own lane.
        h.apply("r2", &a, "assert:review", &ids(&["a1", "r1"]));
        assert_eq!(h.get(&a, RECORD), ids(&["a1"]));
        assert_eq!(h.get(&a, "assert:review"), ids(&["r2"]));

        // A record change of b that cites a's record leaves a's heads alone.
        h.apply("b2", &b, RECORD, &ids(&["b1", "a1"]));
        assert_eq!(h.get(&a, RECORD), ids(&["a1"]));
        assert_eq!(h.get(&b, RECORD), ids(&["b2"]));

        // A prev that is not held anywhere is ignored too.
        h.apply("a2", &a, RECORD, &ids(&["a1", "nowhere"]));
        assert_eq!(h.get(&a, RECORD), ids(&["a2"]));
        assert_eq!(h.get(&subject("C"), RECORD), Vec::<String>::new());
    }

    #[test]
    fn forks_count_record_lanes_only() {
        let (a, b) = (subject("A"), subject("B"));
        let mut h = Heads::default();
        // Two heads in an assert lane are not a fork.
        h.apply("x1", &a, "assert:review", &[]);
        h.apply("x2", &a, "assert:review", &[]);
        assert_eq!(h.record_forks(), 0);
        // Two forked subjects count twice; a subject is one fork however
        // many heads its record lane has.
        for (s, n) in [(&a, 3), (&b, 2)] {
            for i in 0..n {
                h.apply(&format!("{}{i}", s.key), s, RECORD, &[]);
            }
        }
        assert_eq!(h.record_forks(), 2);
    }

    #[test]
    fn rows_are_sorted_and_cover_every_head() {
        let (a, b) = (subject("A"), subject("B"));
        let mut h = Heads::default();
        h.apply("z", &b, RECORD, &[]);
        h.apply("y", &a, "assert:x", &[]);
        h.apply("x2", &a, RECORD, &[]);
        h.apply("x1", &a, RECORD, &[]);
        let rows: Vec<(String, String, String)> = h
            .rows()
            .into_iter()
            .map(|(s, lane, id)| (s.key, lane, id))
            .collect();
        let want = [
            ("A", "assert:x", "y"),
            ("A", "record", "x1"),
            ("A", "record", "x2"),
            ("B", "record", "z"),
        ];
        assert_eq!(
            rows,
            want.iter()
                .map(|(k, l, i)| (k.to_string(), l.to_string(), i.to_string()))
                .collect::<Vec<_>>()
        );

        // The same objects folded in another order give the same heads when
        // no prev links them.
        let mut g = Heads::default();
        g.apply("x1", &a, RECORD, &[]);
        g.apply("z", &b, RECORD, &[]);
        g.apply("x2", &a, RECORD, &[]);
        g.apply("y", &a, "assert:x", &[]);
        assert_eq!(g, h);
        assert_eq!(g.rows(), h.rows());
    }
}
