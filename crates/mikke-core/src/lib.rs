//! Cœur de mikke : extraction, découpage, embeddings, indexation et recherche.
//!
//! Cette crate est une lib interne sans I/O terminal côté recherche : la CLI
//! d'aujourd'hui et le serveur MCP prévu en v1.1 consomment la même API.

pub mod chunk;
pub mod embed;
pub mod extract;
pub mod fuse;
pub mod index;
pub mod search;
pub mod state;
pub mod tok;
pub mod vector;

pub use embed::Embedder;
pub use index::{IndexStats, build_index};
pub use search::{SearchHit, search};
