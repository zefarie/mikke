//! Index vectoriel HNSW (hnsw_rs), persisté à côté de l'index BM25.
//!
//! Les vecteurs étant L2-normalisés, la distance dot (1 − produit scalaire)
//! ordonne comme la similarité cosinus.

use std::path::Path;

use hnsw_rs::api::AnnT;
use hnsw_rs::hnswio::HnswIo;
use hnsw_rs::prelude::*;
use thiserror::Error;

const BASENAME: &str = "vectors";
const MAX_NB_CONN: usize = 16;
/// hnsw_rs ne sait dumper qu'avec ce nombre exact de couches (NB_MAX_LAYER).
const NB_LAYER: usize = 16;
const EF_CONSTRUCTION: usize = 200;
const EF_SEARCH: usize = 96;

#[derive(Debug, Error)]
#[error("index vectoriel : {0}")]
pub struct VectorError(String);

pub struct VectorIndex {
    hnsw: Hnsw<'static, f32, DistDot>,
}

impl VectorIndex {
    /// Construit l'index à partir de (id de chunk, vecteur) et le persiste.
    pub fn build_and_save(dir: &Path, entries: &[(u64, Vec<f32>)]) -> Result<(), VectorError> {
        // hnsw_rs n'écrase jamais un dump existant (il suffixe les noms) :
        // on nettoie d'abord pour que open() lise toujours la version courante
        for suffix in ["graph", "data"] {
            let _ = std::fs::remove_file(dir.join(format!("{BASENAME}.hnsw.{suffix}")));
        }
        let hnsw = Hnsw::<f32, DistDot>::new(
            MAX_NB_CONN,
            entries.len().max(1),
            NB_LAYER,
            EF_CONSTRUCTION,
            DistDot {},
        );
        let data: Vec<(&Vec<f32>, usize)> =
            entries.iter().map(|(id, v)| (v, *id as usize)).collect();
        hnsw.parallel_insert(&data);
        hnsw.file_dump(dir, BASENAME)
            .map_err(|e| VectorError(e.to_string()))?;
        Ok(())
    }

    /// Ouvre l'index vectoriel s'il existe (None : index BM25 seul).
    pub fn open(dir: &Path) -> Result<Option<Self>, VectorError> {
        if !dir.join(format!("{BASENAME}.hnsw.graph")).exists() {
            return Ok(None);
        }
        // Le Hnsw rechargé emprunte le HnswIo ('a: 'b) : on le leake, une
        // seule fois par ouverture d'index, pour obtenir un 'static propre.
        // hnswio panique sur un dump corrompu : catch_unwind pour dégrader
        // en BM25 seul plutôt que crasher.
        let dir = dir.to_path_buf();
        let loaded = std::panic::catch_unwind(move || {
            let io: &'static mut HnswIo = Box::leak(Box::new(HnswIo::new(&dir, BASENAME)));
            io.load_hnsw::<f32, DistDot>()
        });
        match loaded {
            Ok(Ok(hnsw)) => Ok(Some(Self { hnsw })),
            Ok(Err(e)) => Err(VectorError(e.to_string())),
            Err(_) => Err(VectorError(
                "dump corrompu (réindexe avec `mikke index`)".into(),
            )),
        }
    }

    /// Ids de chunks les plus proches, du meilleur au moins bon.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<u64> {
        self.hnsw
            .search(query, k, EF_SEARCH)
            .into_iter()
            .map(|n| n.d_id as u64)
            .collect()
    }
}
