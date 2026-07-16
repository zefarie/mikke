//! Index vectoriel HNSW (usearch), persisté à côté de l'index BM25.
//!
//! usearch plutôt que hnsw_rs : son format se recharge en `view()` (mmap,
//! zéro parsing) là où hnsw_rs désérialise le graphe nœud par nœud — ~450 ms
//! par invocation de la CLI sur 50 000 chunks, intenable pour un budget de
//! 100 ms. Métrique cosinus native (les scores négatifs ne posent aucun
//! problème, contrairement au `DistDot` de hnsw_rs qui assert `dot >= 0`).

use std::path::Path;

use thiserror::Error;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

const FILE_NAME: &str = "vectors.usearch";
const CONNECTIVITY: usize = 16;
const EXPANSION_ADD: usize = 200;
const EXPANSION_SEARCH: usize = 96;

#[derive(Debug, Error)]
#[error("vector index: {0}")]
pub struct VectorError(String);

fn err(e: impl std::fmt::Display) -> VectorError {
    VectorError(e.to_string())
}

pub struct VectorIndex {
    index: Index,
}

impl VectorIndex {
    fn options(dim: usize) -> IndexOptions {
        IndexOptions {
            dimensions: dim,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: CONNECTIVITY,
            expansion_add: EXPANSION_ADD,
            expansion_search: EXPANSION_SEARCH,
            ..Default::default()
        }
    }

    /// Construit l'index à partir de (id de chunk, vecteur) et le persiste.
    pub fn build_and_save(dir: &Path, entries: &[(u64, Vec<f32>)]) -> Result<(), VectorError> {
        let Some(dim) = entries.first().map(|(_, v)| v.len()) else {
            return Ok(());
        };
        let index = Index::new(&Self::options(dim)).map_err(err)?;
        index.reserve(entries.len()).map_err(err)?;
        // usearch encaisse les insertions concurrentes
        use rayon::prelude::*;
        entries
            .par_iter()
            .try_for_each(|(id, vector)| index.add(*id, vector))
            .map_err(err)?;
        let path = dir.join(FILE_NAME);
        let tmp = dir.join(format!("{FILE_NAME}.part"));
        index
            .save(tmp.to_str().ok_or_else(|| err("non-UTF-8 path"))?)
            .map_err(err)?;
        std::fs::rename(&tmp, &path).map_err(err)?;
        Ok(())
    }

    /// Un index vectoriel est-il présent dans ce dossier ?
    pub fn exists(dir: &Path) -> bool {
        dir.join(FILE_NAME).exists()
    }

    /// Ouvre l'index vectoriel s'il existe (None : index BM25 seul).
    /// `view` mmappe le fichier : l'ouverture est instantanée.
    pub fn open(dir: &Path) -> Result<Option<Self>, VectorError> {
        let path = dir.join(FILE_NAME);
        if !path.exists() {
            return Ok(None);
        }
        // les options (hors dimensions) sont relues depuis le fichier
        let index = Index::new(&Self::options(0)).map_err(err)?;
        index
            .view(path.to_str().ok_or_else(|| err("non-UTF-8 path"))?)
            .map_err(err)?;
        Ok(Some(Self { index }))
    }

    /// (id de chunk, similarité cosinus), du meilleur au moins bon.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        self.index
            .search(query, k)
            .map(|m| {
                m.keys
                    .into_iter()
                    .zip(m.distances)
                    .map(|(id, d)| (id, 1.0 - d))
                    .collect()
            })
            .unwrap_or_default()
    }
}
