//! Construction de l'index plein-texte (tantivy, BM25).
//!
//! v1 étape 2 : réindexation complète à chaque `mikke index`. L'incrémental
//! (mtime + blake3, étape 5) se branchera ici — le chemin est déjà stocké
//! par chunk, il suffira de supprimer/réémettre les documents d'un fichier.

use std::path::Path;

use tantivy::schema::{IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions};
use tantivy::tokenizer::{
    AsciiFoldingFilter, LowerCaser, RemoveLongFilter, SimpleTokenizer, TextAnalyzer,
};
use tantivy::{Index, doc};
use walkdir::WalkDir;

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

/// (Re)construit l'index complet de `corpus` dans `index_dir`.
pub fn build_index(corpus: &Path, index_dir: &Path) -> tantivy::Result<IndexStats> {
    if index_dir.exists() {
        std::fs::remove_dir_all(index_dir)?;
    }
    std::fs::create_dir_all(index_dir)?;
    let index = Index::create_in_dir(index_dir, build_schema())?;
    register_tokenizer(&index);

    let schema = index.schema();
    let f_path = schema.get_field("path").expect("champ path");
    let f_chunk_ix = schema.get_field("chunk_ix").expect("champ chunk_ix");
    let f_body = schema.get_field("body").expect("champ body");

    let mut writer = index.writer(WRITER_HEAP_BYTES)?;
    let mut stats = IndexStats::default();

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
            writer.add_document(doc!(
                f_path => path_str.clone(),
                f_chunk_ix => ix as u64,
                f_body => piece,
            ))?;
            stats.chunks += 1;
        }
        stats.files_indexed += 1;
    }

    writer.commit()?;
    Ok(stats)
}
