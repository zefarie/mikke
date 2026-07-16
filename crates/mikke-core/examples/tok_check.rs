//! Valide le tokenizer FST maison contre l'implémentation HuggingFace de
//! référence : pourcentage de séquences identiques et de tokens communs sur
//! le corpus d'éval + les requêtes.
//!
//! Usage : cargo run --release -p mikke-core --example tok_check

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use mikke_core::tok::{CACHE_FILE, Tok};

fn model_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("MIKKE_MODEL_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").expect("HOME non défini")).join(".cache")
        })
        .join("mikke")
        .join("potion-multilingual-128M")
}

fn lcs_len(a: &[u32], b: &[u32]) -> usize {
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    for &x in a {
        for (j, &y) in b.iter().enumerate() {
            cur[j + 1] = if x == y {
                prev[j] + 1
            } else {
                cur[j].max(prev[j + 1])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn main() -> Result<()> {
    let dir = model_dir();
    if !dir.join("tokenizer.json").exists() {
        bail!("tokenizer.json absent de {}", dir.display());
    }
    let cache = dir.join(CACHE_FILE);
    if !cache.exists() {
        Tok::build_cache(&dir.join("tokenizer.json"), &cache)?;
    }
    let ours = Tok::load(&cache)?;
    let reference = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
        .map_err(|e| anyhow::anyhow!("tokenizer HF : {e}"))?;

    let eval_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../eval");
    let mut texts: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(eval_dir.join("corpus")).context("eval/corpus absent")? {
        texts.push(std::fs::read_to_string(entry?.path())?);
    }
    let queries = std::fs::read_to_string(eval_dir.join("queries.toml"))?;
    for line in queries.lines() {
        if let Some(q) = line.strip_prefix("q = ") {
            texts.push(q.trim_matches('"').to_string());
        }
    }

    // matrice d'embeddings en mmap, pour comparer ce qui compte vraiment :
    // le vecteur final produit par chaque tokenisation
    let file = std::fs::File::open(dir.join("model.safetensors"))?;
    let map = unsafe { memmap2::Mmap::map(&file)? };
    let tensors = safetensors::SafeTensors::deserialize(&map)?;
    let tensor = tensors.tensor("embeddings")?;
    let [_, dim]: [usize; 2] = tensor.shape().try_into().unwrap();
    let data = tensor.data();
    let pool = |ids: &[u32]| -> Vec<f32> {
        let mut sum = vec![0.0f32; dim];
        for &id in ids.iter().filter(|&&id| id != 1).take(512) {
            let off = id as usize * dim * 4;
            for (s, b) in sum.iter_mut().zip(data[off..off + dim * 4].chunks_exact(4)) {
                *s += f32::from_le_bytes(b.try_into().unwrap());
            }
        }
        let n = ids.len().max(1) as f32;
        let mut norm = 0.0f32;
        for x in sum.iter_mut() {
            *x /= n;
            norm += *x * *x;
        }
        let norm = norm.sqrt().max(1e-12);
        sum.iter().map(|x| x / norm).collect()
    };

    let mut identical = 0usize;
    let mut total_ref_tokens = 0usize;
    let mut total_common = 0usize;
    let mut min_cos = f32::MAX;
    let mut sum_cos = 0.0f32;
    for text in &texts {
        let a = ours.tokenize(text);
        let enc = reference
            .encode(text.as_str(), false)
            .map_err(|e| anyhow::anyhow!("encode HF : {e}"))?;
        let b: Vec<u32> = enc.get_ids().to_vec();
        if a == b {
            identical += 1;
        }
        total_ref_tokens += b.len();
        total_common += lcs_len(&a, &b);
        let (va, vb) = (pool(&a), pool(&b));
        let cos: f32 = va.iter().zip(&vb).map(|(x, y)| x * y).sum();
        min_cos = min_cos.min(cos);
        sum_cos += cos;
    }

    println!(
        "{} textes — séquences strictement identiques : {} ({:.1} %)",
        texts.len(),
        identical,
        100.0 * identical as f64 / texts.len() as f64
    );
    println!(
        "tokens communs (LCS) : {:.2} % des tokens de référence",
        100.0 * total_common as f64 / total_ref_tokens as f64
    );
    println!(
        "cosinus embedding (nous vs référence) : moyen {:.4}, min {:.4}",
        sum_cos / texts.len() as f32,
        min_cos
    );
    Ok(())
}
