//! Opt-in snippet file.
//!
//! [`FileMemory`] writes one JSON document. It does not open a socket, read
//! credentials, or share a cache with another value. A disabled value does
//! not read or write its path.

use std::collections::HashSet;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

use crate::{MAX_TEXT_BYTES, Memory, MemoryId, Snippet};

/// File name under the Softwake state directory.
pub const MEMORY_FILE_NAME: &str = "memory.json";

/// Largest `memory.json` this backend will read or replace, in bytes.
///
/// Matches the soul-pack cap so a huge file is refused before it is decoded.
pub const MAX_FILE_BYTES: usize = 1024 * 1024;

const DOCUMENT_VERSION: u32 = 1;

/// Snippet file. A new value is disabled and does not touch `path`.
///
/// The file is created on the first successful [`Self::remember`] or
/// [`Self::forget`] that changes the store. [`Self::disable`] drops this
/// handle's cache and leaves the file in place.
#[derive(Debug)]
#[allow(clippy::module_name_repetitions)] // `FileMemory` is the public name of this backend.
pub struct FileMemory {
    path: PathBuf,
    enabled: bool,
    next_id: u64,
    records: Vec<Snippet>,
}

/// Failure from [`FileMemory`].
///
/// Disabled, empty, too long, and missing use the same sentences as
/// [`crate::MemoryError`].
#[derive(Debug, thiserror::Error)]
#[allow(clippy::module_name_repetitions)] // `FileMemoryError` is the public name of this failure.
pub enum FileMemoryError {
    /// The value is off. No snippet was read or written.
    #[error("memory is disabled")]
    Disabled,

    /// Remember was given empty or whitespace-only text.
    #[error("memory text is empty")]
    Empty,

    /// Remember was given text longer than [`MAX_TEXT_BYTES`].
    #[error("memory text is {len} bytes; max is {max}")]
    TooLong {
        /// UTF-8 byte length of the rejected text.
        len: usize,
        /// Cap that was exceeded.
        max: usize,
    },

    /// Forget named an id this value did not hold.
    #[error("memory id {id} is missing")]
    Missing {
        /// Id that was not in the store.
        id: MemoryId,
    },

    /// `XDG_STATE_HOME` and `HOME` were both unset or blank.
    #[error("memory state directory is unset")]
    NoStateDir,

    /// `open_enabled` or `enable` was given an empty path.
    #[error("memory path is empty")]
    EmptyPath,

    /// The file or its parent directory could not be read or written.
    #[error("memory file {} could not be accessed", path.display())]
    Io {
        /// Path the caller asked to use.
        path: PathBuf,
        /// Filesystem error.
        #[source]
        source: Box<io::Error>,
    },

    /// The bytes at `path` are not a JSON document.
    #[error("memory file {} is not valid json", path.display())]
    Invalid {
        /// Path that was read.
        path: PathBuf,
        /// Parse error.
        #[source]
        source: Box<serde_json::Error>,
    },

    /// The document's `version` is not 1.
    #[error("memory file version {found} is unsupported")]
    UnsupportedVersion {
        /// Path that was read.
        path: PathBuf,
        /// Version field from the document.
        found: u32,
    },

    /// The document parsed and then broke a store rule.
    #[error("memory file disagrees with itself")]
    Corrupt {
        /// Path that was read.
        path: PathBuf,
    },

    /// The file, or the bytes that would replace it, exceed [`MAX_FILE_BYTES`].
    #[error("memory file is {len} bytes; max is {max}")]
    TooLarge {
        /// Path that was refused.
        path: PathBuf,
        /// Size that was refused.
        len: u64,
        /// Cap that was exceeded.
        max: u64,
    },

    /// The next id would not fit in a `u64`.
    #[error("memory ids are exhausted")]
    Exhausted {
        /// Path whose counter cannot advance.
        path: PathBuf,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryDocument {
    version: u32,
    next_id: u64,
    snippets: Vec<StoredSnippet>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSnippet {
    id: u64,
    text: String,
}

impl FileMemory {
    /// Disabled handle. Does not read or create `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            enabled: false,
            next_id: 0,
            records: Vec::new(),
        }
    }

    /// Enabled handle.
    ///
    /// Reads `path` when the file exists. A missing file is an empty store
    /// whose first successful remember returns id 1. The file is not created
    /// here.
    ///
    /// # Errors
    ///
    /// Returns [`FileMemoryError::EmptyPath`] when `path` is empty.
    /// Returns [`FileMemoryError::Io`] when the file cannot be read.
    /// Returns [`FileMemoryError::Invalid`] when the bytes are not JSON.
    /// Returns [`FileMemoryError::UnsupportedVersion`] when `version` is not 1.
    /// Returns [`FileMemoryError::Corrupt`] when the document breaks a store rule.
    /// Returns [`FileMemoryError::TooLarge`] when the file exceeds [`MAX_FILE_BYTES`].
    pub fn open_enabled(path: impl Into<PathBuf>) -> Result<Self, FileMemoryError> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(FileMemoryError::EmptyPath);
        }
        let (next_id, records) = load(&path)?;
        Ok(Self {
            path,
            enabled: true,
            next_id,
            records,
        })
    }

    /// Load the file and opt this value in.
    ///
    /// Already enabled is a no-op: the cache is not reread. On error this
    /// value stays disabled and the cache stays empty.
    ///
    /// # Errors
    ///
    /// See [`Self::open_enabled`]. A disabled value with an empty path returns
    /// [`FileMemoryError::EmptyPath`].
    pub fn enable(&mut self) -> Result<(), FileMemoryError> {
        if self.enabled {
            return Ok(());
        }
        if self.path.as_os_str().is_empty() {
            return Err(FileMemoryError::EmptyPath);
        }
        let (next_id, records) = load(&self.path)?;
        self.enabled = true;
        self.next_id = next_id;
        self.records = records;
        Ok(())
    }

    /// Turn this value off and drop its cache.
    ///
    /// The file is not opened, truncated, or removed. A later [`Self::enable`]
    /// loads whatever is still on disk. Forget is what removes a snippet.
    pub fn disable(&mut self) {
        self.enabled = false;
        self.next_id = 0;
        self.records.clear();
    }

    /// Whether [`Self::remember`] can store text on this value.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Path this value reads and writes once it is enabled.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Store `text` and return its id.
    ///
    /// Disabled is rejected before the text is inspected and before any read
    /// or write. While enabled, blank text is rejected, then text longer than
    /// [`MAX_TEXT_BYTES`]. Accepted text is stored unchanged, including a
    /// duplicate of an existing snippet. The duplicate gets a new id. The
    /// cache updates only after the replacement file is renamed into place.
    ///
    /// # Errors
    ///
    /// Returns [`FileMemoryError::Disabled`] when this value is off.
    /// Returns [`FileMemoryError::Empty`] when `text` is empty or whitespace.
    /// Returns [`FileMemoryError::TooLong`] when `text` exceeds [`MAX_TEXT_BYTES`].
    /// Returns [`FileMemoryError::Exhausted`] when the id counter cannot advance.
    /// Returns [`FileMemoryError::TooLarge`] when the file would exceed [`MAX_FILE_BYTES`].
    /// Returns [`FileMemoryError::Io`] when the directory or file cannot be written.
    pub fn remember(&mut self, text: &str) -> Result<MemoryId, FileMemoryError> {
        if !self.enabled {
            return Err(FileMemoryError::Disabled);
        }
        if text.trim().is_empty() {
            return Err(FileMemoryError::Empty);
        }
        let len = text.len();
        if len > MAX_TEXT_BYTES {
            return Err(FileMemoryError::TooLong {
                len,
                max: MAX_TEXT_BYTES,
            });
        }
        let Some(next_id) = self.next_id.checked_add(1) else {
            return Err(FileMemoryError::Exhausted {
                path: self.path.clone(),
            });
        };
        let mut records = self.records.clone();
        let id = MemoryId::from_raw(next_id);
        records.push(Snippet {
            id,
            text: text.to_owned(),
        });
        store(&self.path, next_id, &records)?;
        self.next_id = next_id;
        self.records = records;
        Ok(id)
    }

    /// Return snippets whose text contains `query`, in file order.
    ///
    /// The match is case-sensitive and is not a regular expression. An empty
    /// `query` matches nothing.
    ///
    /// # Errors
    ///
    /// Returns [`FileMemoryError::Disabled`] when this value is off.
    pub fn recall(&self, query: &str) -> Result<Vec<Snippet>, FileMemoryError> {
        if !self.enabled {
            return Err(FileMemoryError::Disabled);
        }
        if query.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .records
            .iter()
            .filter(|snippet| snippet.text.contains(query))
            .cloned()
            .collect())
    }

    /// Drop the snippet with `id`.
    ///
    /// The remaining snippets stay in order. The id counter does not move.
    /// A missing id does not write.
    ///
    /// # Errors
    ///
    /// Returns [`FileMemoryError::Disabled`] when this value is off, including
    /// when `id` was never issued. Returns [`FileMemoryError::Missing`] when
    /// this value is on and does not hold `id`.
    /// Returns [`FileMemoryError::Io`] when the replacement cannot be written.
    pub fn forget(&mut self, id: MemoryId) -> Result<(), FileMemoryError> {
        if !self.enabled {
            return Err(FileMemoryError::Disabled);
        }
        let Some(index) = self.records.iter().position(|snippet| snippet.id == id) else {
            return Err(FileMemoryError::Missing { id });
        };
        let mut records = self.records.clone();
        records.remove(index);
        store(&self.path, self.next_id, &records)?;
        self.records = records;
        Ok(())
    }
}

impl Memory for FileMemory {
    type Error = FileMemoryError;

    fn remember(&mut self, text: &str) -> Result<MemoryId, Self::Error> {
        FileMemory::remember(self, text)
    }

    fn recall(&self, query: &str) -> Result<Vec<Snippet>, Self::Error> {
        FileMemory::recall(self, query)
    }

    fn forget(&mut self, id: MemoryId) -> Result<(), Self::Error> {
        FileMemory::forget(self, id)
    }
}

/// `$XDG_STATE_HOME/softwake` when that value is set and non-blank.
/// Otherwise `{home}/.local/state/softwake`.
///
/// Blank and whitespace-only strings count as unset. This function does not
/// create the directory and does not expand `~`.
///
/// # Errors
///
/// Returns [`FileMemoryError::NoStateDir`] when both sources are unset.
pub fn resolve_memory_dir_from(
    xdg_state_home: Option<&str>,
    home: Option<&str>,
) -> Result<PathBuf, FileMemoryError> {
    resolve_memory_dir_from_paths(trimmed_path(xdg_state_home), trimmed_path(home))
}

/// [`resolve_memory_dir_from`] plus [`MEMORY_FILE_NAME`].
///
/// This function does not create the file.
///
/// # Errors
///
/// Returns [`FileMemoryError::NoStateDir`] when both sources are unset.
pub fn resolve_memory_file_from(
    xdg_state_home: Option<&str>,
    home: Option<&str>,
) -> Result<PathBuf, FileMemoryError> {
    Ok(resolve_memory_dir_from(xdg_state_home, home)?.join(MEMORY_FILE_NAME))
}

/// `memory.json` under the process state directory.
///
/// Reads `XDG_STATE_HOME`, then `HOME`. A Unicode value is trimmed, and a
/// blank value is unset. A non-empty value that is not Unicode counts as set.
/// This function does not create the directory or the file.
///
/// # Errors
///
/// Returns [`FileMemoryError::NoStateDir`] when both variables are unset.
pub fn resolve_memory_file() -> Result<PathBuf, FileMemoryError> {
    Ok(
        resolve_memory_dir_from_paths(path_from_env("XDG_STATE_HOME"), path_from_env("HOME"))?
            .join(MEMORY_FILE_NAME),
    )
}

fn resolve_memory_dir_from_paths(
    xdg_state_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Result<PathBuf, FileMemoryError> {
    if let Some(xdg) = xdg_state_home {
        return Ok(xdg.join("softwake"));
    }
    if let Some(home) = home {
        return Ok(home.join(".local").join("state").join("softwake"));
    }
    Err(FileMemoryError::NoStateDir)
}

fn trimmed_path(value: Option<&str>) -> Option<PathBuf> {
    let text = value?.trim();
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

fn path_from_env(key: &str) -> Option<PathBuf> {
    let value = env::var_os(key)?;
    if value.is_empty() {
        return None;
    }
    match value.to_str() {
        Some(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            }
        }
        // Non-empty and not Unicode still counts as set. Do not lossy-convert it.
        None => Some(PathBuf::from(value)),
    }
}

fn file_cap() -> u64 {
    u64::try_from(MAX_FILE_BYTES).unwrap_or(u64::MAX)
}

fn load(path: &Path) -> Result<(u64, Vec<Snippet>), FileMemoryError> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok((0, Vec::new()));
        }
        Err(source) => return Err(io_err(path, source)),
    };
    let len = meta.len();
    if len > file_cap() {
        return Err(FileMemoryError::TooLarge {
            path: path.to_path_buf(),
            len,
            max: file_cap(),
        });
    }
    let bytes = fs::read(path).map_err(|source| io_err(path, source))?;
    let doc = decode(path, &bytes)?;
    validate(path, doc)
}

fn decode(path: &Path, bytes: &[u8]) -> Result<MemoryDocument, FileMemoryError> {
    match serde_json::from_slice::<MemoryDocument>(bytes) {
        Ok(doc) => Ok(doc),
        Err(source) if source.is_syntax() || source.is_eof() => Err(FileMemoryError::Invalid {
            path: path.to_path_buf(),
            source: Box::new(source),
        }),
        // A JSON value that is not this document, including an unknown field.
        Err(_) => Err(FileMemoryError::Corrupt {
            path: path.to_path_buf(),
        }),
    }
}

fn validate(path: &Path, doc: MemoryDocument) -> Result<(u64, Vec<Snippet>), FileMemoryError> {
    if doc.version != DOCUMENT_VERSION {
        return Err(FileMemoryError::UnsupportedVersion {
            path: path.to_path_buf(),
            found: doc.version,
        });
    }
    let mut seen = HashSet::with_capacity(doc.snippets.len());
    let mut max_id = 0u64;
    let mut records = Vec::with_capacity(doc.snippets.len());
    for snippet in doc.snippets {
        if snippet.id == 0 || !seen.insert(snippet.id) || !text_acceptable(&snippet.text) {
            return Err(FileMemoryError::Corrupt {
                path: path.to_path_buf(),
            });
        }
        max_id = max_id.max(snippet.id);
        records.push(Snippet {
            id: MemoryId::from_raw(snippet.id),
            text: snippet.text,
        });
    }
    if doc.next_id < max_id {
        return Err(FileMemoryError::Corrupt {
            path: path.to_path_buf(),
        });
    }
    Ok((doc.next_id, records))
}

fn text_acceptable(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= MAX_TEXT_BYTES
}

fn store(path: &Path, next_id: u64, records: &[Snippet]) -> Result<(), FileMemoryError> {
    let doc = MemoryDocument {
        version: DOCUMENT_VERSION,
        next_id,
        snippets: records
            .iter()
            .map(|snippet| StoredSnippet {
                id: snippet.id.get(),
                text: snippet.text.clone(),
            })
            .collect(),
    };
    let mut bytes = serde_json::to_vec_pretty(&doc).map_err(|source| FileMemoryError::Invalid {
        path: path.to_path_buf(),
        source: Box::new(source),
    })?;
    bytes.push(b'\n');
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if len > file_cap() {
        return Err(FileMemoryError::TooLarge {
            path: path.to_path_buf(),
            len,
            max: file_cap(),
        });
    }
    ensure_parent(path)?;
    let tmp = temp_path(path)?;
    write_temp(&tmp, path, &bytes)?;
    fs::rename(&tmp, path).map_err(|source| {
        let _ = fs::remove_file(&tmp);
        io_err(path, source)
    })?;
    Ok(())
}

fn ensure_parent(path: &Path) -> Result<(), FileMemoryError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    builder.mode(0o700);
    builder
        .create(parent)
        .map_err(|source| io_err(path, source))?;
    Ok(())
}

fn temp_path(path: &Path) -> Result<PathBuf, FileMemoryError> {
    let Some(name) = path.file_name() else {
        return Err(io_err(
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "memory path has no file name"),
        ));
    };
    let mut tmp_name = std::ffi::OsString::from(name);
    tmp_name.push(".tmp");
    Ok(match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(tmp_name),
        _ => PathBuf::from(tmp_name),
    })
}

fn write_temp(tmp: &Path, dest: &Path, bytes: &[u8]) -> Result<(), FileMemoryError> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    options.mode(0o600);
    let mut file = match options.open(tmp) {
        Ok(file) => file,
        Err(source) => return Err(io_err(dest, source)),
    };
    let wrote = write_and_sync(&mut file, bytes);
    drop(file);
    if let Err(source) = wrote {
        let _ = fs::remove_file(tmp);
        return Err(io_err(dest, source));
    }
    Ok(())
}

fn write_and_sync(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn io_err(path: &Path, source: io::Error) -> FileMemoryError {
    FileMemoryError::Io {
        path: path.to_path_buf(),
        source: Box::new(source),
    }
}

#[cfg(test)]
#[path = "file_tests.rs"]
mod tests;
