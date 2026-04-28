//! Concrete `GraphBackend` implementations.
//!
//! Each backend lives in its own submodule. Native-only backends are
//! feature-gated off the wasm target.

pub mod in_memory;
pub use in_memory::InMemoryGraph;

#[cfg(not(target_arch = "wasm32"))]
pub mod sqlite;
#[cfg(not(target_arch = "wasm32"))]
pub use sqlite::SqliteGraph;

#[cfg(not(target_arch = "wasm32"))]
pub mod postgres;
#[cfg(not(target_arch = "wasm32"))]
pub use postgres::PostgresGraph;

#[cfg(not(target_arch = "wasm32"))]
pub mod qdrant;
#[cfg(not(target_arch = "wasm32"))]
pub use qdrant::QdrantGraph;

#[cfg(not(target_arch = "wasm32"))]
pub mod falkor;
#[cfg(not(target_arch = "wasm32"))]
pub use falkor::GraphClient;
