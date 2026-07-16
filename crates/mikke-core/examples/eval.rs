//! Éval de la qualité de recherche : hit@k sur les requêtes de référence.
//!
//! Usage : cargo run --release -p mikke-core --example eval
//! Le modèle doit être présent dans ~/.cache/mikke/potion-multilingual-128M
//! (ou $MIKKE_MODEL_DIR) — lancé une fois, `mikke index` le télécharge.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use mikke_core::Embedder;
use serde::Deserialize;

#[derive(Deserialize)]
struct QueryFile {
    query: Vec<RefQuery>,
}

#[derive(Deserialize)]
struct RefQuery {
    q: String,
    expect: String,
}

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

fn rank_of(hits: &[mikke_core::SearchHit], expect: &str) -> Option<usize> {
    hits.iter()
        .position(|h| Path::new(&h.path).file_name().is_some_and(|f| f == expect))
        .map(|p| p + 1)
}

fn main() -> Result<()> {
    let eval_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../eval");
    let corpus = eval_dir.join("corpus");
    let queries: QueryFile = toml::from_str(
        &std::fs::read_to_string(eval_dir.join("queries.toml"))
            .context("queries.toml illisible")?,
    )?;

    let model_dir = model_dir();
    if !model_dir.join("model.safetensors").exists() {
        bail!(
            "modèle absent de {} — lance `mikke index <dossier>` une fois pour le télécharger",
            model_dir.display()
        );
    }
    let embedder = Embedder::load(&model_dir)?;

    let index_dir = tempfile::tempdir()?;
    let t = Instant::now();
    let stats = mikke_core::build_index(corpus.as_path(), index_dir.path(), Some(&embedder))?;
    eprintln!(
        "index : {} fichiers, {} chunks, vecteurs={} — {:.2}s",
        stats.files_indexed,
        stats.chunks,
        stats.vectors,
        t.elapsed().as_secs_f32()
    );

    let mut hybrid_at_1 = 0;
    let mut hybrid_at_3 = 0;
    let mut hybrid_at_10 = 0;
    let mut bm25_at_10 = 0;
    let mut latencies_us: Vec<u128> = Vec::new();

    println!("| requête | BM25 seul | hybride |");
    println!("|---|---|---|");
    for rq in &queries.query {
        let bm25 = mikke_core::search(index_dir.path(), &rq.q, 10, None)?;
        let t = Instant::now();
        let hybrid = mikke_core::search(index_dir.path(), &rq.q, 10, Some(&embedder))?;
        latencies_us.push(t.elapsed().as_micros());

        let rb = rank_of(&bm25, &rq.expect);
        let rh = rank_of(&hybrid, &rq.expect);
        if rb.is_some() {
            bm25_at_10 += 1;
        }
        if let Some(r) = rh {
            hybrid_at_10 += 1;
            if r <= 3 {
                hybrid_at_3 += 1;
            }
            if r == 1 {
                hybrid_at_1 += 1;
            }
        }
        let show = |r: Option<usize>| match r {
            Some(r) => format!("#{r}"),
            None => "—".to_string(),
        };
        println!("| {} | {} | {} |", rq.q, show(rb), show(rh));
    }

    let n = queries.query.len();
    latencies_us.sort_unstable();
    let p50 = latencies_us[n / 2] as f64 / 1000.0;
    let p95 = latencies_us[(n * 95 / 100).min(n - 1)] as f64 / 1000.0;

    println!();
    println!(
        "hit@10 hybride : {hybrid_at_10}/{n} — hit@3 : {hybrid_at_3}/{n} — hit@1 : {hybrid_at_1}/{n}"
    );
    println!("hit@10 BM25 seul : {bm25_at_10}/{n}");
    println!("latence requête hybride : p50 {p50:.1} ms, p95 {p95:.1} ms");
    Ok(())
}
