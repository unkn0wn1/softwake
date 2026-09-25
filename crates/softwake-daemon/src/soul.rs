//! Soul directory the daemon has read.
//!
//! Startup reads the pack once. `reload_soul` reads it again and sets the
//! pending flag. A wake phrase applies that read; it does not open the files
//! itself. The pending flag is cleared only after that apply succeeds, so a
//! missing pack cannot enter awake and cannot drop the request.

#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

use softwake_ipc::SoulReport;
use softwake_soul::{SoulDir, SoulPack};

/// Last read of a soul directory, plus the text applied to the current session.
#[derive(Debug)]
pub(crate) struct LoadedSoul {
    dir: SoulDir,
    pack: Option<SoulPack>,
    reason: Option<String>,
    reload_pending: bool,
    applied: Option<String>,
}

impl LoadedSoul {
    /// Read `dir` now. The pending flag stays clear until [`Self::reload`].
    pub(crate) fn open(dir: SoulDir) -> Self {
        let mut loaded = Self {
            dir,
            pack: None,
            reason: None,
            reload_pending: false,
            applied: None,
        };
        loaded.read();
        loaded
    }

    /// Re-read the directory and mark the result for the next awake entry.
    pub(crate) fn reload(&mut self) {
        self.reload_pending = true;
        self.read();
    }

    /// The last read produced a pack.
    pub(crate) fn is_valid(&self) -> bool {
        self.pack.is_some()
    }

    /// `reload_soul` has not been applied on an awake entry yet.
    pub(crate) fn reload_pending(&self) -> bool {
        self.reload_pending
    }

    /// Why the last read failed.
    pub(crate) fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// Wire form of the last read. The pack text is not included.
    pub(crate) fn report(&self) -> SoulReport {
        SoulReport {
            ok: self.is_valid(),
            reason: self.reason.clone(),
        }
    }

    /// Sentence for a `reload_soul` reply.
    pub(crate) fn reload_summary(&self) -> String {
        reload_message(self.is_valid(), self.reason())
    }

    /// Why a wake phrase must not enter awake.
    ///
    /// `None` when the last read is a valid pack.
    pub(crate) fn refusal(&self) -> Option<String> {
        if self.pack.is_some() {
            return None;
        }
        let reason = self.reason.as_deref().unwrap_or("unreadable");
        Some(format!(
            "soul pack is missing or invalid ({reason}); refusing awake"
        ))
    }

    /// Remember the rendered pack and clear the pending flag.
    ///
    /// Call this only after the voice machine has stored awake. A missing
    /// pack leaves the flag set.
    pub(crate) fn commit_awake(&mut self) {
        let Some(pack) = &self.pack else {
            return;
        };
        self.applied = Some(pack.render_instructions());
        self.reload_pending = false;
    }

    /// Instructions applied on the last successful awake entry.
    ///
    /// This stays set after the text session closes. The session keeps its
    /// own copy only while it is open.
    pub(crate) fn applied_instructions(&self) -> Option<&str> {
        self.applied.as_deref()
    }

    fn read(&mut self) {
        match self.dir.load() {
            Ok(pack) => {
                self.pack = Some(pack);
                self.reason = None;
            }
            Err(error) => {
                self.pack = None;
                self.reason = Some(error.to_string());
            }
        }
    }
}

/// Operator-facing reload reply.
pub(crate) fn reload_message(ok: bool, reason: Option<&str>) -> String {
    if ok {
        "reloaded soul pack; applies on next awake. Read soul.md, user.md, rules.md, and glossary.md.".to_owned()
    } else {
        let reason = reason.unwrap_or("unreadable");
        format!("soul pack invalid ({reason}); applies on next awake once the pack is valid")
    }
}

/// Temporary soul directory for daemon tests.
#[cfg(test)]
pub(crate) struct TestSoulDir {
    path: PathBuf,
}

#[cfg(test)]
impl TestSoulDir {
    /// Empty directory. Loading it fails until [`Self::write`].
    pub(crate) fn empty() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("softwake-daemon-soul-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).expect("temp soul dir");
        Self { path }
    }

    /// Directory with a small valid pack.
    pub(crate) fn valid() -> Self {
        let dir = Self::empty();
        dir.write("test soul\n", "test user\n");
        dir
    }

    /// Replace the four required files.
    ///
    /// `rules.md` and `glossary.md` reset to the defaults. A test that needs
    /// a custom or broken file writes it after this call.
    pub(crate) fn write(&self, soul: &str, user: &str) {
        fs::write(self.path.join("soul.md"), soul).expect("write soul.md");
        fs::write(self.path.join("user.md"), user).expect("write user.md");
        fs::write(
            self.path.join("rules.md"),
            "No email send. Hand drafts to the operator.\n",
        )
        .expect("write rules.md");
        fs::write(self.path.join("glossary.md"), "docs → /path/to/docs\n")
            .expect("write glossary.md");
    }

    /// Directory the daemon should open.
    pub(crate) fn soul_dir(&self) -> SoulDir {
        SoulDir::new(self.path.clone())
    }

    /// Filesystem path of this temporary directory.
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[cfg(test)]
impl Drop for TestSoulDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
