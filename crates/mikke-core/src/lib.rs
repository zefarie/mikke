//! Cœur de mikke : extraction, découpage, indexation et recherche.
//!
//! Cette crate est une lib interne sans I/O terminal côté recherche : la CLI
//! d'aujourd'hui et le serveur MCP prévu en v1.1 consomment la même API.

pub mod chunk;
pub mod extract;
pub mod index;
pub mod search;

pub use index::{IndexStats, build_index};
pub use search::{SearchHit, search};
