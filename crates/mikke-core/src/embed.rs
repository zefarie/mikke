//! Inférence model2vec maison, taillée pour une CLI :
//!
//! - la matrice d'embeddings (~500 Mo) est lue en mmap, jamais chargée
//!   entière en RAM — seules les pages touchées deviennent résidentes ;
//! - le tokenizer est notre Unigram sur FST ([`crate::tok`]), rechargé en
//!   quelques millisecondes là où celui de HuggingFace coûte ~700 ms et
//!   ~550 Mo à chaque lancement.
//!
//! Sémantique répliquée de model2vec : pas de tokens spéciaux, tokens
//! inconnus jetés, troncature à 512 tokens, moyenne des lignes,
//! normalisation L2.

use std::path::Path;

use memmap2::Mmap;
use thiserror::Error;

use crate::tok::{CACHE_FILE, Tok, TokError};

/// Même valeur par défaut que model2vec.
const MAX_TOKENS: usize = 512;

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("unreadable model: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Tokenizer(#[from] TokError),
    #[error("invalid model: {0}")]
    Format(String),
}

pub struct Embedder {
    tok: Tok,
    map: Mmap,
    data_start: usize,
    rows: usize,
    dim: usize,
    normalize: bool,
}

impl Embedder {
    /// Charge un modèle model2vec depuis un dossier contenant
    /// `model.safetensors`, `tokenizer.json` et `config.json`. Au premier
    /// appel, le cache FST du tokenizer est construit (~2 s, une fois).
    pub fn load(model_dir: &Path) -> Result<Self, EmbedError> {
        let cache = model_dir.join(CACHE_FILE);
        if !cache.exists() {
            Tok::build_cache(&model_dir.join("tokenizer.json"), &cache)?;
        }
        let tok = Tok::load(&cache)?;

        let file = std::fs::File::open(model_dir.join("model.safetensors"))?;
        // SAFETY : lecture seule ; le fichier ne doit pas être modifié pendant
        // l'exécution (il vit dans le cache de mikke).
        let map = unsafe { Mmap::map(&file)? };
        let tensors = safetensors::SafeTensors::deserialize(&map)
            .map_err(|e| EmbedError::Format(e.to_string()))?;
        let tensor = tensors
            .tensor("embeddings")
            .map_err(|e| EmbedError::Format(e.to_string()))?;
        if tensor.dtype() != safetensors::Dtype::F32 {
            return Err(EmbedError::Format(format!(
                "unsupported dtype {:?} (expected F32)",
                tensor.dtype()
            )));
        }
        let [rows, dim]: [usize; 2] = tensor
            .shape()
            .try_into()
            .map_err(|_| EmbedError::Format("embeddings tensor is not 2-D".into()))?;
        // offset des données du tenseur dans le fichier mmappé (zéro copie)
        let data_start = tensor.data().as_ptr() as usize - map.as_ptr() as usize;

        let normalize = std::fs::read_to_string(model_dir.join("config.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("normalize").and_then(|b| b.as_bool()))
            .unwrap_or(true);

        Ok(Self {
            tok,
            map,
            data_start,
            rows,
            dim,
            normalize,
        })
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    fn row(&self, ix: usize) -> impl Iterator<Item = f32> + '_ {
        let start = self.data_start + ix * self.dim * 4;
        self.map[start..start + self.dim * 4]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().expect("chunks_exact(4)")))
    }

    /// Embedding L2-normalisé d'un texte. Un texte vide donne un vecteur nul.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let ids = self.tok.tokenize(text);

        let mut sum = vec![0.0_f32; self.dim];
        let mut count = 0usize;
        for &id in ids.iter().take(MAX_TOKENS) {
            let ix = id as usize;
            if ix >= self.rows {
                continue;
            }
            for (s, v) in sum.iter_mut().zip(self.row(ix)) {
                *s += v;
            }
            count += 1;
        }
        let denom = count.max(1) as f32;
        for x in &mut sum {
            *x /= denom;
        }
        if self.normalize {
            let norm = sum.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
            for x in &mut sum {
                *x /= norm;
            }
        }
        Ok(sum)
    }
}
