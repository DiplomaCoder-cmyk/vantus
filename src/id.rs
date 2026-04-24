use std::sync::atomic::{AtomicU64, Ordering};

/// Shared service contract for generating application identifiers.
pub trait IdGenerator: Send + Sync {
    /// Returns the next identifier value.
    fn next_id(&self) -> String;
}

/// Cheap monotonic ID generator backed by an atomic counter.
pub struct AtomicIdGenerator {
    prefix: String,
    counter: AtomicU64,
}

impl AtomicIdGenerator {
    /// Creates an atomic generator with the default `id` prefix.
    pub fn new() -> Self {
        Self::with_prefix("id")
    }

    /// Creates an atomic generator that prefixes every generated ID.
    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            counter: AtomicU64::new(0),
        }
    }
}

impl Default for AtomicIdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl IdGenerator for AtomicIdGenerator {
    fn next_id(&self) -> String {
        let sequence = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        format!("{}-{sequence}", self.prefix)
    }
}

/// UUID-backed generator for globally unique identifiers.
#[derive(Default)]
pub struct UuidIdGenerator;

impl IdGenerator for UuidIdGenerator {
    fn next_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}
