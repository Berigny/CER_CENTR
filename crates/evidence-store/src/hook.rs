//! Named points inside file and store operations where the crate's own
//! unit tests run code of their own: to fail a step that a real system
//! fails only rarely (a directory that cannot be synced), or to change the
//! store directory at the moment another process could (a `LOCK` removed
//! between its open and its lock).
//!
//! Outside `cfg(test)`, [`at`] does nothing and returns `Ok`. In tests a
//! hook is set for one thread with `set` and runs at every point that
//! thread reaches, until the guard `set` returns is dropped. While the hook
//! runs it is taken out, so what it does itself (taking a lease, say) runs
//! without it.
//!
//! The points, each with the path it is given:
//!
//! | Point | Where | Path |
//! |---|---|---|
//! | `lease:open` | `Lease::acquire`, before it opens `LOCK` | `LOCK` |
//! | `lease:opened` | `Lease::acquire`, after the open, before `try_lock` | `LOCK` |
//! | `sync` | `files::sync_dir`, before the directory is opened | the directory |
//! | `read:open` | `files::read_bounded`, after the type check, before the open | the file |
//! | `init:placed` | `SqliteLog::init`, right after `log.db.init` is renamed to `log.db` | the store directory |
//! | `open:checked` | `SqliteLog::open`, after the open checks, before the lease | the store directory |

/// Run the hook set for this thread, if any, at `point`.
#[cfg(not(test))]
#[inline(always)]
pub(crate) fn at(_point: &str, _path: &std::path::Path) -> Result<(), crate::api::StoreError> {
    Ok(())
}

#[cfg(test)]
pub(crate) use imp::{at, set};

#[cfg(test)]
mod imp {
    use std::cell::RefCell;
    use std::path::Path;

    use crate::api::StoreError;

    type Hook = Box<dyn FnMut(&str, &Path) -> Result<(), StoreError>>;

    thread_local! {
        static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    /// Clears this thread's hook when dropped.
    pub(crate) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            HOOK.with(|h| h.borrow_mut().take());
        }
    }

    /// Run `hook` at every point this thread reaches, until the guard drops.
    #[must_use]
    pub(crate) fn set(hook: impl FnMut(&str, &Path) -> Result<(), StoreError> + 'static) -> Guard {
        HOOK.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
        Guard
    }

    /// Run this thread's hook, if any, at `point`; its error is returned.
    pub(crate) fn at(point: &str, path: &Path) -> Result<(), StoreError> {
        let Some(mut hook) = HOOK.with(|h| h.borrow_mut().take()) else {
            return Ok(());
        };
        let out = hook(point, path);
        HOOK.with(|h| {
            let mut slot = h.borrow_mut();
            if slot.is_none() {
                *slot = Some(hook);
            }
        });
        out
    }
}
