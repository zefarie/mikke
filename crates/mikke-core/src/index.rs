//! Construction de l'index : plein-texte (tantivy, BM25) + vectoriel (HNSW),
//! incrémentale.
//!
//! Un fichier dont mtime + taille n'ont pas bougé n'est jamais relu. S'ils
//! ont bougé, le hash blake3 tranche : contenu identique → rien à faire.
//! Sinon le fichier est ré-extrait, ré-embeddé et réémis dans tantivy
//! (delete_term sur son chemin puis réinsertion). Les embeddings sont cachés
//! en SQLite ([`crate::state`]) : l'index HNSW est reconstruit à chaque run
//! à partir du cache (quelques secondes), mais on ne ré-embedde jamais un
//! fichier inchangé — c'est là que partent les minutes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tantivy::schema::{
    INDEXED, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
};
use tantivy::tokenizer::{
    AsciiFoldingFilter, Language, LowerCaser, RemoveLongFilter, SimpleTokenizer, StopWordFilter,
    TextAnalyzer,
};
use tantivy::{Index, Term, doc};
use thiserror::Error;
use walkdir::WalkDir;

use crate::embed::Embedder;
use crate::state::{DB_FILE, FileRecord, State};
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

#[derive(Debug, Error)]
pub enum IndexError {
    #[error(transparent)]
    Tantivy(#[from] tantivy::TantivyError),
    #[error("état d'index : {0}")]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Vector(#[from] crate::vector::VectorError),
}

#[derive(Debug, Default)]
pub struct IndexStats {
    /// Fichiers (ré)indexés pendant ce run.
    pub files_indexed: usize,
    /// Fichiers déjà à jour (mtime/taille ou hash identiques).
    pub files_unchanged: usize,
    /// Format non supporté, fichier trop gros ou sans texte (scans).
    pub files_skipped: usize,
    /// Fichiers illisibles (signalés en warning, jamais bloquants).
    pub files_failed: usize,
    /// Fichiers disparus du disque, retirés de l'index.
    pub files_deleted: usize,
    /// Total de chunks vivants dans l'index.
    pub chunks: usize,
    /// Faux si l'index ne contient aucun vecteur.
    pub vectors: bool,
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
    // stopwords AVANT le folding : les listes contiennent les formes
    // accentuées (« à », « où »…) en minuscules
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(StopWordFilter::new(Language::French).expect("stopwords fr"))
        .filter(StopWordFilter::new(Language::English).expect("stopwords en"))
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

struct FileEntry {
    path: PathBuf,
    mtime_ns: i64,
    size: i64,
}

/// Parcourt `corpus` et liste les fichiers supportés avec leurs métadonnées.
fn collect_files(corpus: &Path, stats: &mut IndexStats) -> Vec<FileEntry> {
    let mut files = Vec::new();
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
        if !extract::supported(entry.path()) {
            stats.files_skipped += 1;
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            stats.files_failed += 1;
            continue;
        };
        if meta.len() > MAX_FILE_BYTES {
            stats.files_skipped += 1;
            continue;
        }
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        files.push(FileEntry {
            path: entry.into_path(),
            mtime_ns,
            size: meta.len() as i64,
        });
    }
    files
}

/// Met à jour (ou construit) l'index de `corpus` dans `index_dir`.
///
/// Avec un `Embedder`, les nouveaux chunks sont embeddés et l'index HNSW
/// reconstruit ; sans (modèle absent), l'index reste utilisable en BM25.
/// `full` force une reconstruction complète.
pub fn build_index(
    corpus: &Path,
    index_dir: &Path,
    embedder: Option<&Embedder>,
    full: bool,
) -> Result<IndexStats, IndexError> {
    let fresh = full || !index_dir.join(DB_FILE).exists() || !index_dir.join("meta.json").exists();
    if fresh && index_dir.exists() {
        std::fs::remove_dir_all(index_dir)?;
    }
    std::fs::create_dir_all(index_dir)?;
    let index = if fresh {
        Index::create_in_dir(index_dir, build_schema())?
    } else {
        Index::open_in_dir(index_dir)?
    };
    register_tokenizer(&index);
    let state = State::open(index_dir)?;

    let schema = index.schema();
    let f_path = schema.get_field("path").expect("champ path");
    let f_chunk_id = schema.get_field("chunk_id").expect("champ chunk_id");
    let f_chunk_ix = schema.get_field("chunk_ix").expect("champ chunk_ix");
    let f_body = schema.get_field("body").expect("champ body");

    let mut writer = index.writer(WRITER_HEAP_BYTES)?;
    let mut stats = IndexStats::default();
    let files = collect_files(corpus, &mut stats);

    let mut seen: HashSet<String> = HashSet::with_capacity(files.len());

    // phase 1 (séquentielle, peu coûteuse) : écarter les fichiers dont
    // mtime + taille n'ont pas bougé
    let mut candidates: Vec<(FileEntry, String, Option<FileRecord>)> = Vec::new();
    for entry in files {
        let path_str = entry.path.to_string_lossy().into_owned();
        seen.insert(path_str.clone());
        let previous = state.file(&path_str)?;
        if let Some(prev) = &previous
            && prev.mtime_ns == entry.mtime_ns
            && prev.size == entry.size
        {
            stats.files_unchanged += 1;
            continue;
        }
        candidates.push((entry, path_str, previous));
    }

    // phase 2 (parallèle) : lecture, hash, extraction, chunking, embeddings —
    // tout ce qui coûte des minutes part sur tous les cœurs
    enum Outcome {
        Unchanged(FileRecord),
        Failed,
        Empty(FileRecord),
        Content(FileRecord, Vec<(String, Vec<f32>)>),
    }
    use rayon::prelude::*;
    let processed: Vec<(String, Outcome)> = candidates
        .into_par_iter()
        .map(|(entry, path_str, previous)| {
            let bytes = match std::fs::read(&entry.path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("warn: {} illisible, ignoré ({e})", entry.path.display());
                    return (path_str, Outcome::Failed);
                }
            };
            let record = FileRecord {
                mtime_ns: entry.mtime_ns,
                size: entry.size,
                hash: blake3::hash(&bytes).into(),
            };
            if let Some(prev) = &previous
                && prev.hash == record.hash
            {
                // mtime a bougé, pas le contenu (copie, touch…)
                return (path_str, Outcome::Unchanged(record));
            }
            drop(bytes);
            let text = match extract::extract(&entry.path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("warn: {} illisible, ignoré ({e})", entry.path.display());
                    return (path_str, Outcome::Failed);
                }
            };
            let pieces = chunk::chunk(&text, CHUNK_WORDS, CHUNK_OVERLAP);
            if pieces.is_empty() {
                return (path_str, Outcome::Empty(record));
            }
            let content = pieces
                .into_iter()
                .map(|piece| {
                    let vector = match embedder {
                        Some(e) => e.embed(&piece).unwrap_or_default(),
                        None => Vec::new(),
                    };
                    (piece, vector)
                })
                .collect();
            (path_str, Outcome::Content(record, content))
        })
        .collect();

    // phase 3 (séquentielle) : SQLite et tantivy
    let mut content_changed = false;
    state.begin()?;
    for (path_str, outcome) in processed {
        match outcome {
            Outcome::Failed => stats.files_failed += 1,
            Outcome::Unchanged(record) => {
                state.upsert_file(&path_str, &record)?;
                stats.files_unchanged += 1;
            }
            Outcome::Empty(record) => {
                writer.delete_term(Term::from_field_text(f_path, &path_str));
                state.delete_chunks(&path_str)?;
                state.upsert_file(&path_str, &record)?;
                stats.files_skipped += 1;
                content_changed = true;
            }
            Outcome::Content(record, content) => {
                writer.delete_term(Term::from_field_text(f_path, &path_str));
                state.delete_chunks(&path_str)?;
                state.upsert_file(&path_str, &record)?;
                for (ix, (piece, vector)) in content.into_iter().enumerate() {
                    let chunk_id = state.insert_chunk(&path_str, ix as u64, &vector)?;
                    writer.add_document(doc!(
                        f_path => path_str.clone(),
                        f_chunk_id => chunk_id,
                        f_chunk_ix => ix as u64,
                        f_body => piece,
                    ))?;
                }
                stats.files_indexed += 1;
            }
        }
    }

    // fichiers disparus du disque
    for path in state.all_paths()? {
        if !seen.contains(&path) {
            writer.delete_term(Term::from_field_text(f_path, &path));
            state.delete_file(&path)?;
            stats.files_deleted += 1;
        }
    }
    state.commit()?;
    writer.commit()?;

    stats.chunks = state.chunk_count()? as usize;
    // le graphe HNSW n'est reconstruit que si le contenu a bougé : un run
    // « rien à faire » sur 50 000 chunks passerait sinon de ~1 s à ~15 s
    let dirty = fresh || content_changed || stats.files_indexed > 0 || stats.files_deleted > 0;
    if dirty {
        let vectors = state.all_vectors()?;
        if !vectors.is_empty() {
            VectorIndex::build_and_save(index_dir, &vectors)?;
            stats.vectors = true;
        }
    } else {
        stats.vectors = VectorIndex::exists(index_dir);
    }
    Ok(stats)
}
