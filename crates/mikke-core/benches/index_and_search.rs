//! Bancs criterion : indexation BM25 et recherche (BM25 seul, et hybride si
//! le modèle est présent dans le cache).
//!
//! cargo bench -p mikke-core

use std::path::PathBuf;

use criterion::{Criterion, criterion_group, criterion_main};

/// Générateur déterministe (LCG) : pas de dépendance rand, résultats stables.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

const WORDS: &[&str] = &[
    "facture",
    "contrat",
    "garantie",
    "assurance",
    "vétérinaire",
    "consultation",
    "loyer",
    "caution",
    "préavis",
    "ordonnance",
    "vaccin",
    "rappel",
    "impôts",
    "revenu",
    "attestation",
    "scolarité",
    "bulletin",
    "trimestre",
    "recette",
    "cuisson",
    "datasheet",
    "invoice",
    "lease",
    "deposit",
    "salary",
    "warranty",
    "coverage",
    "deductible",
    "booking",
    "baggage",
    "meeting",
    "milestone",
    "janvier",
    "avril",
    "septembre",
    "montant",
    "total",
    "euros",
    "dossier",
    "référence",
    "monsieur",
    "madame",
    "cordialement",
    "signature",
    "article",
    "chapitre",
];

fn synth_corpus(dir: &std::path::Path, files: usize, words_per_file: usize, seed: u64) {
    let mut rng = Lcg(seed);
    for i in 0..files {
        let mut text = String::with_capacity(words_per_file * 8);
        for w in 0..words_per_file {
            text.push_str(WORDS[(rng.next() as usize) % WORDS.len()]);
            text.push(if w % 12 == 11 { '\n' } else { ' ' });
        }
        std::fs::write(dir.join(format!("doc{i:05}.md")), text).unwrap();
    }
}

fn model_dir() -> PathBuf {
    std::env::var("MIKKE_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").expect("HOME"))
                .join(".cache/mikke/potion-multilingual-128M")
        })
}

fn bench_index(c: &mut Criterion) {
    let corpus = tempfile::tempdir().unwrap();
    synth_corpus(corpus.path(), 200, 500, 42);
    let mut group = c.benchmark_group("index");
    group.sample_size(10);
    group.bench_function("bm25_200_fichiers_500_mots", |b| {
        b.iter(|| {
            let index_dir = tempfile::tempdir().unwrap();
            mikke_core::build_index(
                &[corpus.path().to_path_buf()],
                &[],
                index_dir.path(),
                None,
                true,
            )
            .unwrap()
        });
    });
    group.finish();
}

fn bench_search(c: &mut Criterion) {
    let corpus = tempfile::tempdir().unwrap();
    synth_corpus(corpus.path(), 1000, 800, 7);
    let index_dir = tempfile::tempdir().unwrap();

    let embedder = mikke_core::Embedder::load(&model_dir()).ok();
    mikke_core::build_index(
        &[corpus.path().to_path_buf()],
        &[],
        index_dir.path(),
        embedder.as_ref(),
        true,
    )
    .unwrap();
    let searcher = mikke_core::Searcher::open(index_dir.path()).unwrap();

    let mut group = c.benchmark_group("search");
    group.bench_function("bm25_seul", |b| {
        b.iter(|| {
            searcher
                .search("facture vétérinaire janvier", 10, None)
                .unwrap()
        });
    });
    if let Some(emb) = &embedder {
        group.bench_function("hybride", |b| {
            b.iter(|| {
                searcher
                    .search("facture vétérinaire janvier", 10, Some(emb))
                    .unwrap()
            });
        });
        group.bench_function("embed_requete", |b| {
            b.iter(|| emb.embed("facture vétérinaire janvier").unwrap());
        });
    } else {
        eprintln!("note : modèle absent, bancs hybrides sautés");
    }
    group.finish();
}

criterion_group!(benches, bench_index, bench_search);
criterion_main!(benches);
