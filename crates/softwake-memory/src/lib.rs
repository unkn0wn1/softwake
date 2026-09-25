//! Long-term memory boundary.
//!
//! [`Memory`] stores and retrieves text snippets. [`MockMemory`] keeps them
//! on the value and starts disabled. [`FileMemory`] writes `memory.json` only
//! after that value is enabled and a remember or forget succeeds.
//! [`recall_for_prompt`] applies the ask/chat snippet budget. This crate
//! does not open a socket or read credentials. [`MockMemory`] does not create
//! a directory.

mod file;
mod memory;
mod mock;
mod recall;

pub use file::{
    FileMemory, FileMemoryError, MAX_FILE_BYTES, MEMORY_FILE_NAME, resolve_memory_dir_from,
    resolve_memory_file, resolve_memory_file_from,
};
pub use memory::{MAX_TEXT_BYTES, Memory, MemoryError, MemoryId, Snippet};
pub use mock::MockMemory;
pub use recall::{
    MAX_RECALL_BYTES, MAX_RECALL_SNIPPETS, RECALL_LEAD, budget_recall, recall_for_prompt,
    render_recall,
};

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{FileMemory, Memory, MemoryId, MockMemory, Snippet};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("softwake-memory-trait-{}-{n}", std::process::id()));
            fs::create_dir_all(&path).expect("temp dir");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn remember<M: Memory>(memory: &mut M, text: &str) -> Result<MemoryId, M::Error> {
        memory.remember(text)
    }

    fn recall<M: Memory>(memory: &M, query: &str) -> Result<Vec<Snippet>, M::Error> {
        memory.recall(query)
    }

    fn forget<M: Memory>(memory: &mut M, id: MemoryId) -> Result<(), M::Error> {
        memory.forget(id)
    }

    #[test]
    fn trait_round_trip_uses_the_trait() {
        let mut memory = MockMemory::enabled();
        let id = remember(&mut memory, "kept fact").expect("remember");
        assert_eq!(
            recall(&memory, "fact").expect("recall"),
            vec![Snippet {
                id,
                text: "kept fact".to_owned(),
            }]
        );
        forget(&mut memory, id).expect("forget");
        assert!(
            recall(&memory, "fact")
                .expect("recall after forget")
                .is_empty()
        );
    }

    #[test]
    fn trait_round_trip_uses_file_memory() {
        let dir = TempDir::new();
        let mut memory = FileMemory::open_enabled(dir.path.join("memory.json")).expect("open");
        let id = remember(&mut memory, "kept fact").expect("remember");
        assert_eq!(
            recall(&memory, "fact").expect("recall"),
            vec![Snippet {
                id,
                text: "kept fact".to_owned(),
            }]
        );
        forget(&mut memory, id).expect("forget");
        assert!(
            recall(&memory, "fact")
                .expect("recall after forget")
                .is_empty()
        );
    }
}
