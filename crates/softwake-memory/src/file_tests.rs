use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    FileMemory, FileMemoryError, MAX_FILE_BYTES, MEMORY_FILE_NAME, MemoryDocument, StoredSnippet,
    resolve_memory_dir_from, resolve_memory_file_from,
};
use crate::{MAX_TEXT_BYTES, MemoryId, Snippet};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("softwake-memory-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).expect("read")
}

#[test]
fn new_does_not_create_or_enable() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let mut memory = FileMemory::new(&path);
    let over = "a".repeat(MAX_TEXT_BYTES + 1);
    assert!(!memory.is_enabled());
    let disabled = memory.remember("hi").expect_err("disabled");
    assert!(matches!(disabled, FileMemoryError::Disabled));
    assert_eq!(disabled.to_string(), "memory is disabled");
    assert!(matches!(
        memory.remember(&over),
        Err(FileMemoryError::Disabled)
    ));
    assert!(matches!(
        memory.remember("  "),
        Err(FileMemoryError::Disabled)
    ));
    assert!(matches!(
        memory.recall("hi"),
        Err(FileMemoryError::Disabled)
    ));
    assert!(matches!(
        memory.forget(MemoryId::from_raw(1)),
        Err(FileMemoryError::Disabled)
    ));
    assert!(!path.exists());
}

#[test]
fn empty_path_is_rejected() {
    let opened = FileMemory::open_enabled("").expect_err("empty");
    assert!(matches!(opened, FileMemoryError::EmptyPath));
    assert_eq!(opened.to_string(), "memory path is empty");
    let mut memory = FileMemory::new("");
    assert!(matches!(memory.enable(), Err(FileMemoryError::EmptyPath)));
    assert!(matches!(
        memory.remember("hi"),
        Err(FileMemoryError::Disabled)
    ));
}

#[test]
fn open_enabled_missing_file_is_empty() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let mut memory = FileMemory::open_enabled(&path).expect("open");
    assert!(memory.is_enabled());
    assert!(memory.recall("hi").expect("recall").is_empty());
    let missing = memory.forget(MemoryId::from_raw(1)).expect_err("missing");
    assert!(matches!(missing, FileMemoryError::Missing { .. }));
    assert_eq!(missing.to_string(), "memory id 1 is missing");
    assert!(!path.exists());

    let mut gated = FileMemory::new(&path);
    gated.enable().expect("enable");
    assert!(gated.is_enabled());
    assert!(!path.exists());
}

#[test]
fn reopen_sees_remembered_text() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let text = "  café  ";
    let id = {
        let mut memory = FileMemory::open_enabled(&path).expect("open");
        memory.remember(text).expect("remember")
    };
    let memory = FileMemory::open_enabled(&path).expect("reopen");
    assert_eq!(
        memory.recall("café").expect("hit"),
        vec![Snippet {
            id,
            text: text.to_owned(),
        }]
    );
    assert!(memory.recall("Café").expect("case").is_empty());
    assert!(memory.recall("").expect("blank query").is_empty());
    assert_eq!(memory.recall("  ").expect("spaces").len(), 1);
}

#[test]
fn forget_persists_and_ids_do_not_reuse() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let (first, second) = {
        let mut memory = FileMemory::open_enabled(&path).expect("open");
        let first = memory.remember("alpha note").expect("first");
        let second = memory.remember("beta note").expect("second");
        memory.forget(first).expect("forget");
        (first, second)
    };
    let mut memory = FileMemory::open_enabled(&path).expect("reopen");
    assert_eq!(
        memory.recall("note").expect("remaining"),
        vec![Snippet {
            id: second,
            text: "beta note".to_owned(),
        }]
    );
    assert!(matches!(
        memory.forget(first),
        Err(FileMemoryError::Missing { .. })
    ));
    assert_eq!(
        memory.remember("gamma note").expect("third"),
        MemoryId::from_raw(3)
    );
    let doc: serde_json::Value = serde_json::from_slice(&read(&path)).expect("json");
    assert_eq!(doc["next_id"], 3);
    let ids: Vec<u64> = doc["snippets"]
        .as_array()
        .expect("snippets")
        .iter()
        .map(|snippet| snippet["id"].as_u64().expect("id"))
        .collect();
    assert_eq!(ids, vec![2, 3]);
}

#[test]
fn disabled_handle_does_not_read_or_write() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    {
        let mut memory = FileMemory::open_enabled(&path).expect("open");
        memory.remember("kept").expect("remember");
    }
    let bytes = read(&path);
    let mut off = FileMemory::new(&path);
    assert!(matches!(
        off.remember("nope"),
        Err(FileMemoryError::Disabled)
    ));
    assert!(matches!(off.recall("kept"), Err(FileMemoryError::Disabled)));
    assert!(matches!(
        off.forget(MemoryId::from_raw(1)),
        Err(FileMemoryError::Disabled)
    ));
    assert_eq!(read(&path), bytes);
    let on = FileMemory::open_enabled(&path).expect("reopen");
    assert_eq!(on.recall("kept").expect("kept").len(), 1);
}

#[test]
fn disable_keeps_the_file() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let mut memory = FileMemory::open_enabled(&path).expect("open");
    let id = memory.remember("kept").expect("remember");
    let bytes = read(&path);
    memory.disable();
    assert!(!memory.is_enabled());
    assert!(matches!(
        memory.remember("next"),
        Err(FileMemoryError::Disabled)
    ));
    assert!(matches!(
        memory.recall("kept"),
        Err(FileMemoryError::Disabled)
    ));
    assert!(matches!(memory.forget(id), Err(FileMemoryError::Disabled)));
    assert_eq!(read(&path), bytes);
    memory.enable().expect("enable");
    assert_eq!(memory.recall("kept").expect("restored").len(), 1);
    assert_eq!(
        memory.remember("next").expect("next id"),
        MemoryId::from_raw(2)
    );
}

#[test]
fn enable_when_already_on_does_not_reread() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let mut memory = FileMemory::open_enabled(&path).expect("open");
    memory.remember("kept").expect("remember");
    fs::write(&path, b"{\"version\":1,\"next_id\":0,\"snippets\":[]}\n").expect("overwrite");
    memory.enable().expect("noop");
    assert_eq!(memory.recall("kept").expect("cache").len(), 1);
}

#[test]
fn reject_blank_and_overlong_without_writing() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let mut memory = FileMemory::open_enabled(&path).expect("open");
    let empty = memory.remember("").expect_err("empty");
    assert!(matches!(empty, FileMemoryError::Empty));
    assert_eq!(empty.to_string(), "memory text is empty");
    assert!(matches!(
        memory.remember(" \n\t"),
        Err(FileMemoryError::Empty)
    ));
    let over = "a".repeat(MAX_TEXT_BYTES + 1);
    let too_long = memory.remember(&over).expect_err("too long");
    assert!(matches!(
        too_long,
        FileMemoryError::TooLong {
            len,
            max: MAX_TEXT_BYTES,
        } if len == MAX_TEXT_BYTES + 1
    ));
    assert_eq!(
        too_long.to_string(),
        format!(
            "memory text is {} bytes; max is {}",
            MAX_TEXT_BYTES + 1,
            MAX_TEXT_BYTES
        )
    );
    assert!(!path.exists());
    let exact = "a".repeat(MAX_TEXT_BYTES);
    assert_eq!(memory.remember(&exact).expect("cap"), MemoryId::from_raw(1));
    assert_eq!(
        memory.recall(&exact).expect("stored")[0].text.len(),
        MAX_TEXT_BYTES
    );
    assert_eq!(
        memory.remember(&exact).expect("duplicate"),
        MemoryId::from_raw(2)
    );
}

#[test]
fn recall_order_is_file_order() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let body = r#"{"version":1,"next_id":2,"snippets":[{"id":2,"text":"beta note"},{"id":1,"text":"alpha note"}]}"#;
    fs::write(&path, body).expect("seed");
    let memory = FileMemory::open_enabled(&path).expect("open");
    assert_eq!(
        memory.recall("note").expect("order"),
        vec![
            Snippet {
                id: MemoryId::from_raw(2),
                text: "beta note".to_owned(),
            },
            Snippet {
                id: MemoryId::from_raw(1),
                text: "alpha note".to_owned(),
            },
        ]
    );
}

#[test]
fn two_paths_do_not_share() {
    let dir = TempDir::new();
    let mut first = FileMemory::open_enabled(dir.join("a.json")).expect("first");
    let mut second = FileMemory::open_enabled(dir.join("b.json")).expect("second");
    assert_eq!(
        first.remember("local").expect("write"),
        MemoryId::from_raw(1)
    );
    assert!(second.recall("local").expect("other").is_empty());
    assert_eq!(
        second.remember("other").expect("own id"),
        MemoryId::from_raw(1)
    );
}

#[test]
fn two_handles_do_not_alias() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let mut first = FileMemory::open_enabled(&path).expect("first");
    let second = FileMemory::open_enabled(&path).expect("second");
    first.remember("local").expect("write");
    assert!(second.recall("local").expect("stale").is_empty());
    drop(second);
    let third = FileMemory::open_enabled(&path).expect("third");
    assert_eq!(third.recall("local").expect("reopen").len(), 1);
}

#[test]
fn corrupt_file_fails_closed() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    fs::write(&path, b"not json").expect("seed");
    let err = FileMemory::open_enabled(&path).expect_err("invalid");
    assert!(matches!(err, FileMemoryError::Invalid { .. }));
    assert!(err.to_string().contains("not valid json"));
    assert_eq!(read(&path), b"not json");
}

#[test]
fn unsupported_version_fails_closed() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let body = br#"{"version":2,"next_id":0,"snippets":[]}"#;
    fs::write(&path, body).expect("seed");
    let err = FileMemory::open_enabled(&path).expect_err("version");
    assert!(matches!(
        err,
        FileMemoryError::UnsupportedVersion { found: 2, .. }
    ));
    assert_eq!(err.to_string(), "memory file version 2 is unsupported");
    assert_eq!(read(&path), body);
}

#[test]
fn inconsistent_document_fails_closed() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let long = "a".repeat(MAX_TEXT_BYTES + 1);
    let bodies = [
        r#"{"version":1,"next_id":0,"snippets":[{"id":1,"text":"a"}]}"#.to_owned(),
        r#"{"version":1,"next_id":1,"snippets":[{"id":0,"text":"a"}]}"#.to_owned(),
        r#"{"version":1,"next_id":2,"snippets":[{"id":1,"text":"a"},{"id":1,"text":"b"}]}"#
            .to_owned(),
        r#"{"version":1,"next_id":1,"snippets":[{"id":2,"text":"a"}]}"#.to_owned(),
        r#"{"version":1,"next_id":1,"snippets":[{"id":1,"text":"   "}]}"#.to_owned(),
        format!(r#"{{"version":1,"next_id":1,"snippets":[{{"id":1,"text":"{long}"}}]}}"#),
        r#"{"version":1,"next_id":0,"snippets":[],"tags":[]}"#.to_owned(),
    ];
    for body in bodies {
        fs::write(&path, &body).expect("seed");
        let err = FileMemory::open_enabled(&path).expect_err("corrupt");
        assert!(
            matches!(err, FileMemoryError::Corrupt { .. }),
            "{body}: {err}"
        );
        assert_eq!(err.to_string(), "memory file disagrees with itself");
        assert_eq!(read(&path), body.as_bytes());
    }
}

#[test]
fn ids_do_not_wrap() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let body = format!(r#"{{"version":1,"next_id":{},"snippets":[]}}"#, u64::MAX);
    fs::write(&path, &body).expect("seed");
    let mut memory = FileMemory::open_enabled(&path).expect("open");
    let err = memory.remember("hi").expect_err("exhausted");
    assert!(matches!(err, FileMemoryError::Exhausted { .. }));
    assert_eq!(err.to_string(), "memory ids are exhausted");
    assert_eq!(read(&path), body.as_bytes());
}

#[test]
fn oversized_file_is_not_parsed() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let bytes = vec![b'x'; MAX_FILE_BYTES + 1];
    fs::write(&path, &bytes).expect("seed");
    let err = FileMemory::open_enabled(&path).expect_err("too large");
    assert!(matches!(
        err,
        FileMemoryError::TooLarge { len, max, .. }
            if len == u64::try_from(MAX_FILE_BYTES + 1).expect("len")
                && max == u64::try_from(MAX_FILE_BYTES).expect("max")
    ));
    assert_eq!(read(&path), bytes);
}

#[test]
fn remember_near_the_cap_does_not_clobber() {
    let dir = TempDir::new();
    let path = dir.join(MEMORY_FILE_NAME);
    let bytes = document_one_snippet_under_cap();
    assert!(bytes.len() <= MAX_FILE_BYTES);
    fs::write(&path, &bytes).expect("seed");
    let mut memory = FileMemory::open_enabled(&path).expect("open");
    let extra = "b".repeat(MAX_TEXT_BYTES);
    let err = memory.remember(&extra).expect_err("cap");
    assert!(matches!(err, FileMemoryError::TooLarge { .. }), "{err}");
    assert_eq!(read(&path), bytes);
}

fn document_one_snippet_under_cap() -> Vec<u8> {
    let text = "a".repeat(MAX_TEXT_BYTES);
    let mut low = 1usize;
    let mut high = 200usize;
    while low <= high {
        let count = usize::midpoint(low, high);
        let bytes = render_snippets(&text, count);
        if bytes.len() <= MAX_FILE_BYTES {
            let bigger = render_snippets(&text, count + 1);
            if bigger.len() > MAX_FILE_BYTES {
                return bytes;
            }
            low = count + 1;
        } else if count == 0 {
            break;
        } else {
            high = count - 1;
        }
    }
    panic!("no document sits just under the file cap");
}

fn render_snippets(text: &str, count: usize) -> Vec<u8> {
    let snippets = (1..=count)
        .map(|id| StoredSnippet {
            id: u64::try_from(id).expect("id"),
            text: text.to_owned(),
        })
        .collect();
    let doc = MemoryDocument {
        version: 1,
        next_id: u64::try_from(count).expect("count"),
        snippets,
    };
    let mut bytes = serde_json::to_vec_pretty(&doc).expect("encode");
    bytes.push(b'\n');
    bytes
}

#[test]
fn resolver_does_not_create() {
    assert_eq!(
        resolve_memory_file_from(Some("from-xdg"), Some("from-home")).expect("xdg"),
        PathBuf::from("from-xdg/softwake/memory.json")
    );
    assert_eq!(
        resolve_memory_file_from(Some("   "), Some("from-home")).expect("blank xdg"),
        PathBuf::from("from-home/.local/state/softwake/memory.json")
    );
    assert_eq!(
        resolve_memory_dir_from(None, Some("from-home")).expect("home"),
        PathBuf::from("from-home/.local/state/softwake")
    );
    let unset = resolve_memory_file_from(None, None).expect_err("unset");
    assert!(matches!(unset, FileMemoryError::NoStateDir));
    assert_eq!(unset.to_string(), "memory state directory is unset");
    assert!(matches!(
        resolve_memory_file_from(Some(""), Some(" \t")),
        Err(FileMemoryError::NoStateDir)
    ));

    let dir = TempDir::new();
    let root = dir.join("xdg-root");
    let text = root.to_str().expect("utf8 temp");
    let path = resolve_memory_file_from(Some(text), None).expect("temp xdg");
    assert_eq!(path, root.join("softwake").join(MEMORY_FILE_NAME));
    assert!(!root.exists());
}

#[test]
fn permissions_on_created_files() {
    let dir = TempDir::new();
    let state = dir.join("state");
    let path = state.join(MEMORY_FILE_NAME);
    let mut memory = FileMemory::open_enabled(&path).expect("open");
    memory.remember("hi").expect("remember");
    let dir_mode = fs::metadata(&state).expect("dir").permissions().mode() & 0o777;
    let file_mode = fs::metadata(&path).expect("file").permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700);
    assert_eq!(file_mode, 0o600);
}

#[test]
fn failed_write_does_not_consume_an_id() {
    let dir = TempDir::new();
    let blocker = dir.join("blocker");
    fs::write(&blocker, b"keep").expect("blocker");
    let nested = blocker.join(MEMORY_FILE_NAME);
    match FileMemory::open_enabled(&nested) {
        Err(FileMemoryError::Io { .. }) => {}
        Ok(mut memory) => {
            let err = memory.remember("hi").expect_err("write");
            assert!(matches!(err, FileMemoryError::Io { .. }), "{err}");
        }
        Err(other) => panic!("unexpected {other}"),
    }
    assert_eq!(read(&blocker), b"keep");
    assert!(!dir.join(MEMORY_FILE_NAME).exists());

    let mut memory = FileMemory::open_enabled(dir.join(MEMORY_FILE_NAME)).expect("good");
    assert_eq!(memory.remember("hi").expect("first"), MemoryId::from_raw(1));
}
