//! An init killed at any moment (`docs/m1-store-spec.md` section 11, T7):
//! it leaves a complete store or a directory the next `init` takes.
//!
//! This test starts processes, so it runs in a binary of its own. A process
//! started with `fork` and `exec` holds a copy of every file its parent had
//! open until its `exec`, a writer's `LOCK` included, and a lease held that
//! way lasts past its drop: a test beside this one that drops a writer and
//! opens the store again at once could get `S_LOCKED`. Data: the store
//! key vector in `test-vectors/keys`. Nothing is real.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use evidence_log::VerifyOptions;
use evidence_store::sqlite::lease::LOCK_FILE;
use evidence_store::sqlite::{DB_FILE, INIT_FILE};
use evidence_store::{verify_store, ScriptClock, SqliteLog};

mod common;
use common::{names, store_key};

/// The variable that makes [`killed_init_child`] create a store.
const INIT_CHILD: &str = "EVIDENCE_STORE_TEST_INIT_CHILD";

/// Not a test of its own: [`init_killed_at_any_moment`] runs this test
/// binary with only this test, and `INIT_CHILD` set to a directory, and
/// kills it while it creates a store there. Without the variable it does
/// nothing.
#[test]
fn killed_init_child() {
    let Some(dir) = std::env::var_os(INIT_CHILD) else {
        return;
    };
    let c = ScriptClock::new(&["2026-11-26T00:00:00Z"]);
    drop(SqliteLog::init(Path::new(&dir), "cer-demo", &store_key(), Box::new(c)).unwrap());
}

/// An init killed at any moment (here, from 0 to 20 ms after its process
/// starts) leaves either a complete store, which verifies and opens, or
/// nothing but what the next `init` takes: never a directory that is
/// neither.
#[test]
fn init_killed_at_any_moment() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let leftovers = [
        LOCK_FILE,
        INIT_FILE,
        "log.db.init-journal",
        "log.db.init-wal",
        "log.db.init-shm",
    ];
    let verify = |dir: &Path| verify_store(dir, VerifyOptions::default()).unwrap();
    let (mut stores, mut taken) = (0, 0);
    for run in 0..40_u64 {
        let dir = tmp.path().join(format!("run-{run}")).join("store");
        let mut child = Command::new(&exe)
            .args(["killed_init_child", "--exact", "--test-threads=1", "-q"])
            .env(INIT_CHILD, &dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_micros(run * 500));
        let _ = child.kill();
        child.wait().unwrap();
        if dir.join(DB_FILE).exists() {
            assert!(verify(&dir).ok(), "run {run}: {:?}", names(&dir));
            drop(SqliteLog::open(&dir, Box::new(ScriptClock::default())).unwrap());
            stores += 1;
            continue;
        }
        if dir.exists() {
            let left = names(&dir);
            assert!(
                left.iter().all(|name| leftovers.contains(&name.as_str())),
                "run {run}: {left:?}"
            );
        }
        let c = ScriptClock::new(&["2026-11-26T00:00:00Z"]);
        drop(SqliteLog::init(&dir, "cer-demo", &store_key(), Box::new(c)).unwrap());
        assert!(verify(&dir).ok(), "run {run}");
        taken += 1;
    }
    assert_eq!(stores + taken, 40);
}
