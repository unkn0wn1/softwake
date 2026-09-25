//! Failures from resolving or reading a soul pack.

use std::path::PathBuf;

/// Which required file a failure belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoulFile {
    /// `soul.md`
    Soul,
    /// `user.md`
    User,
    /// `rules.md`
    Rules,
    /// `glossary.md`
    Glossary,
}

impl SoulFile {
    /// File name inside the soul directory.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Soul => "soul.md",
            Self::User => "user.md",
            Self::Rules => "rules.md",
            Self::Glossary => "glossary.md",
        }
    }
}

impl std::fmt::Display for SoulFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

/// A soul directory could not be chosen, or a required file failed validation.
#[derive(Debug, thiserror::Error)]
pub enum SoulError {
    /// Neither a flag, `SOFTWAKE_SOUL_DIR`, `XDG_CONFIG_HOME`, nor `HOME` was set.
    #[error(
        "soul directory is not set; pass --soul-dir, set SOFTWAKE_SOUL_DIR, or set HOME or XDG_CONFIG_HOME"
    )]
    Unresolved,

    /// The file is not in the directory.
    #[error("missing {file} ({path})")]
    Missing {
        /// Which file was missing.
        file: SoulFile,
        /// Path that was opened.
        path: PathBuf,
    },

    /// The file is empty or only whitespace.
    #[error("{file} is empty ({path})")]
    Empty {
        /// Which file was empty.
        file: SoulFile,
        /// Path that was read.
        path: PathBuf,
    },

    /// The file is not UTF-8.
    #[error("{file} is not valid UTF-8 ({path})")]
    InvalidUtf8 {
        /// Which file was not UTF-8.
        file: SoulFile,
        /// Path that was read.
        path: PathBuf,
    },

    /// The file is larger than [`MAX_FILE_BYTES`](crate::MAX_FILE_BYTES).
    #[error("{file} exceeds {max} bytes ({path})")]
    TooLarge {
        /// Which file was too large.
        file: SoulFile,
        /// Path that was read.
        path: PathBuf,
        /// Cap that was exceeded.
        max: u64,
    },

    /// Opening or reading the file failed for a reason other than absence.
    #[error("cannot read {file} ({path}): {source}")]
    Read {
        /// Which file could not be read.
        file: SoulFile,
        /// Path that was opened.
        path: PathBuf,
        /// Filesystem error.
        #[source]
        source: std::io::Error,
    },

    /// `glossary.md` passed the file checks and failed the map checks.
    #[error("invalid glossary ({path}): {detail}")]
    InvalidGlossary {
        /// Path that was parsed.
        path: PathBuf,
        /// Why the alias map was refused, such as `duplicate alias docs`.
        detail: String,
    },
}
