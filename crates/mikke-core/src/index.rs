//! Construction de l'index : plein-texte (tantivy, BM25) + vectoriel (HNSW).
//!
//! v1 étape 3 : réindexation complète à chaque `mikke index`. L'incrémental
//! (mtime + blake3, étape 5) se branchera ici — chaque chunk porte déjà son
//! chemin et un id stable dans l'index.

use std::path::Path;

use tantivy::schema::{
    INDEXED, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
};
use tantivy::tokenizer::{
    AsciiFoldingFilter, LowerCaser, RemoveLongFilter, SimpleTokenizer, TextAnalyzer,
};
use tantivy::{Index, doc};
use walkdir::WalkDir;

use crate::embed::Embedder;
use crate::vector::VectorIndex;
use crate::{chunk, extract};

/// Tokenizer maison : minuscules + suppression des accents, pour que
/// « veterinaire » retrouve « vétérinaire » (et inversement).
pub const TOKENIZER_NAME: &str = "mikke_text";

const CHUNK_WORDS: usize = 400;
const CHUNK_OVERLAP: usize = 80;
const WRITER_HEAP_BYTES: usize = 128 * 1024 * 1024;
/// Au-delà, ce n'est probablement pas un document personnel lisible.
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// Dossiers jamais indexés (en plus des dossiers cachés).
const SKIPPED_DIRS: &[&str] = &["node_modules", "target", "__pycache__", "venv"];

#[derive(Debug, Default)]
pub struct IndexStats {
    pub files_indexed: usize,
    /// Format non supporté, fichier trop gros ou sans texte.
    pub files_skipped: usize,
    /// Fichiers illisibles (signalés en warning, jamais bloquants).
    pub files_failed: usize,
    pub chunks: usize,
    /// Faux si l'index a été construit sans modèle d'embeddings.
    pub vectors: bool,
}

struct ChunkRecord {
    path: String,
    chunk_ix: u64,
    text: String,
}

fn build_schema() -> Schema {
    let mut builder = Schema::builder();
    let body_indexing = TextFieldIndexing::default()
        .set_tokenizer(TOKENIZER_NAME)
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let body_options = TextOptions::default()
        .set_indexing_options(body_indexing)
        .set_stored();
    builder.add_text_field("path", STRING | STORED);
    builder.add_u64_field("chunk_id", INDEXED | STORED);
    builder.add_u64_field("chunk_ix", STORED);
    builder.add_text_field("body", body_options);
    builder.build()
}

fn register_tokenizer(index: &Index) {
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .build();
    index.tokenizers().register(TOKENIZER_NAME, analyzer);
}

/// Ouvre un index existant (avec le tokenizer maison enregistré).
pub fn open_index(index_dir: &Path) -> tantivy::Result<Index> {
    let index = Index::open_in_dir(index_dir)?;
    register_tokenizer(&index);
    Ok(index)
}

/// Parcourt `corpus` et collecte les chunks de tous les fichiers lisibles.
fn collect_chunks(corpus: &Path, stats: &mut IndexStats) -> Vec<ChunkRecord> {
    let mut records = Vec::new();
    let walker = WalkDir::new(corpus).into_iter().filter_entry(|e| {
        if e.depth() == 0 || !e.file_type().is_dir() {
            return true;
        }
        let name = e.file_name().to_string_lossy();
        !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_ref())
    });

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("warn: entrée illisible, ignorée ({e})");
                stats.files_failed += 1;
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !extract::supported(path) {
            stats.files_skipped += 1;
            continue;
        }
        if entry
            .metadata()
            .map(|m| m.len() > MAX_FILE_BYTES)
            .unwrap_or(true)
        {
            stats.files_skipped += 1;
            continue;
        }
        let text = match extract::extract(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("warn: {} illisible, ignoré ({e})", path.display());
                stats.files_failed += 1;
                continue;
            }
        };
        let chunks = chunk::chunk(&text, CHUNK_WORDS, CHUNK_OVERLAP);
        if chunks.is_empty() {
            stats.files_skipped += 1;
            continue;
        }
        let path_str = path.to_string_lossy().into_owned();
        for (ix, piece) in chunks.into_iter().enumerate() {
            records.push(ChunkRecord {
                path: path_str.clone(),
                chunk_ix: ix as u64,
                text: piece,
            });
        }
        stats.files_indexed += 1;
    }
    records
}

/// (Re)construit l'index complet de `corpus` dans `index_dir`.
///
/// Avec un `Embedder`, l'index vectoriel HNSW est construit à côté de l'index
/// BM25 ; sans (modèle absent), l'index reste utilisable en BM25 seul.
pub fn build_index(
    corpus: &Path,
    index_dir: &Path,
    embedder: Option<&Embedder>,
) -> tantivy::Result<IndexStats> {
    if index_dir.exists() {
        std::fs::remove_dir_all(index_dir)?;
    }
    std::fs::create_dir_all(index_dir)?;
    let index = Index::create_in_dir(index_dir, build_schema())?;
    register_tokenizer(&index);

    let schema = index.schema();
    let f_path = schema.get_field("path").expect("champ path");
    let f_chunk_id = schema.get_field("chunk_id").expect("champ chunk_id");
    let f_chunk_ix = schema.get_field("chunk_ix").expect("champ chunk_ix");
    let f_body = schema.get_field("body").expect("champ body");

    let mut stats = IndexStats::default();
    let records = collect_chunks(corpus, &mut stats);
    stats.chunks = records.len();

    let mut writer = index.writer(WRITER_HEAP_BYTES)?;
    for (chunk_id, rec) in records.iter().enumerate() {
        writer.add_document(doc!(
            f_path => rec.path.clone(),
            f_chunk_id => chunk_id as u64,
            f_chunk_ix => rec.chunk_ix,
            f_body => rec.text.clone(),
        ))?;
    }
    writer.commit()?;

    if let Some(embedder) = embedder {
        let mut entries: Vec<(u64, Vec<f32>)> = Vec::with_capacity(records.len());
        for (chunk_id, rec) in records.iter().enumerate() {
            match embedder.embed(&rec.text) {
                Ok(v) => entries.push((chunk_id as u64, v)),
                Err(e) => eprintln!("warn: embedding impossible pour {} ({e})", rec.path),
            }
        }
        if let Err(e) = VectorIndex::build_and_save(index_dir, &entries) {
            eprintln!("warn: {e} — l'index restera en BM25 seul");
        } else {
            stats.vectors = true;
        }
    }

    Ok(stats)
}
