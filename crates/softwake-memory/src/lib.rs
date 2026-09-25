//! Long-term memory boundary.
//!
//! [`Memory`] stores and retrieves text snippets. [`MockMemory`] keeps them
//! on the value and starts disabled. This crate does not open a socket, read
//! credentials, or create a directory.

mod memory;
mod mock;

pub use memory::{MAX_TEXT_BYTES, Memory, MemoryError, MemoryId, Snippet};
pub use mock::MockMemory;

#[cfg(test)]
mod tests {
    use super::{Memory, MemoryId, MockMemory, Snippet};

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
}
