//! Recherche hybride : BM25 et voisins vectoriels, fusionnés par RRF.
//!
//! La recherche se fait au niveau des chunks ; on garde le meilleur chunk de
//! chaque fichier pour ne jamais montrer deux fois le même document. Sans
//! modèle d'embeddings (ou sans index vectoriel), on retombe en BM25 seul.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;

use serde::Serialize;
use tantivy::collector::TopDocs;
use tantivy::query::{QueryParser, TermQuery};
use tantivy::schema::{IndexRecordOption, Value};
use tantivy::snippet::SnippetGenerator;
use tantivy::{TantivyDocument, Term};

use crate::embed::Embedder;
use crate::fuse::rrf;
use crate::index::open_index;
use crate::vector::VectorIndex;

/// Combien de chunks chaque côté ramène avant fusion.
const CHUNK_POOL: usize = 50;
const SNIPPET_MAX_CHARS: usize = 240;

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub path: String,
    /// Score RRF (comparable entre requêtes, pas entre index).
    pub score: f32,
    /// Extrait du chunk le mieux classé.
    pub snippet: String,
    /// Plages d'octets de `snippet` correspondant aux termes de la requête.
    pub highlights: Vec<Range<usize>>,
}

/// Index ouvert, prêt à répondre à plusieurs requêtes. C'est la forme à
/// utiliser pour tout usage interactif (TUI, futur serveur MCP) : ouvrir
/// l'index — et surtout recharger le graphe HNSW — à chaque requête serait
/// du gaspillage.
pub struct Searcher {
    index: tantivy::Index,
    reader: tantivy::IndexReader,
    vectors: Option<VectorIndex>,
}

impl Searcher {
    pub fn open(index_dir: &Path) -> tantivy::Result<Self> {
        let index = open_index(index_dir)?;
        let reader = index.reader()?;
        let vectors = match VectorIndex::open(index_dir) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("warn: {e}");
                None
            }
        };
        Ok(Self {
            index,
            reader,
            vectors,
        })
    }

    pub fn search(
        &self,
        query_str: &str,
        limit: usize,
        embedder: Option<&Embedder>,
    ) -> tantivy::Result<Vec<SearchHit>> {
        search_open(self, query_str, limit, embedder)
    }
}

/// Recherche « one-shot » : ouvre l'index, cherche, referme.
pub fn search(
    index_dir: &Path,
    query_str: &str,
    limit: usize,
    embedder: Option<&Embedder>,
) -> tantivy::Result<Vec<SearchHit>> {
    Searcher::open(index_dir)?.search(query_str, limit, embedder)
}

fn search_open(
    open: &Searcher,
    query_str: &str,
    limit: usize,
    embedder: Option<&Embedder>,
) -> tantivy::Result<Vec<SearchHit>> {
    let index = &open.index;
    let schema = index.schema();
    let f_path = schema.get_field("path").expect("champ path");
    let f_chunk_id = schema.get_field("chunk_id").expect("champ chunk_id");
    let f_body = schema.get_field("body").expect("champ body");

    let searcher = open.reader.searcher();
    let parser = QueryParser::for_index(index, vec![f_body]);
    // lenient : une requête utilisateur n'est jamais une erreur de syntaxe
    let (query, _) = parser.parse_query_lenient(query_str);

    // côté BM25 : chunks classés + documents déjà récupérés
    let mut docs_by_id: HashMap<u64, TantivyDocument> = HashMap::new();
    let mut bm25_ranked: Vec<u64> = Vec::new();
    for (_score, addr) in
        searcher.search(&query, &TopDocs::with_limit(CHUNK_POOL).order_by_score())?
    {
        let doc: TantivyDocument = searcher.doc(addr)?;
        if let Some(id) = doc.get_first(f_chunk_id).and_then(|v| v.as_u64()) {
            bm25_ranked.push(id);
            docs_by_id.insert(id, doc);
        }
    }

    // côté vecteurs : voisins du même embedding que l'index
    let mut lists = vec![bm25_ranked];
    if let (Some(embedder), Some(vectors)) = (embedder, open.vectors.as_ref()) {
        match embedder.embed(query_str) {
            Ok(qvec) => lists.push(vectors.search(&qvec, CHUNK_POOL)),
            Err(e) => eprintln!("warn: embedding de la requête impossible ({e})"),
        }
    }

    let fused = rrf(&lists);

    let mut snippets = SnippetGenerator::create(&searcher, &*query, f_body)?;
    snippets.set_max_num_chars(SNIPPET_MAX_CHARS);

    let mut seen_paths = HashSet::new();
    let mut hits = Vec::new();
    for (chunk_id, score) in fused {
        let doc = match docs_by_id.remove(&chunk_id) {
            Some(d) => d,
            None => match fetch_by_chunk_id(&searcher, f_chunk_id, chunk_id)? {
                Some(d) => d,
                None => continue,
            },
        };
        let path = doc
            .get_first(f_path)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if !seen_paths.insert(path.clone()) {
            continue;
        }
        let snippet = snippets.snippet_from_doc(&doc);
        let (fragment, highlights) = if snippet.fragment().is_empty() {
            // hit purement vectoriel : pas de terme à surligner, début du chunk
            let body = doc
                .get_first(f_body)
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            (truncate_chars(body, SNIPPET_MAX_CHARS), Vec::new())
        } else {
            (
                snippet.fragment().to_string(),
                snippet.highlighted().to_vec(),
            )
        };
        hits.push(SearchHit {
            path,
            score,
            snippet: fragment,
            highlights,
        });
        if hits.len() == limit {
            break;
        }
    }
    Ok(hits)
}

/// Récupère un chunk par son id (hits venus du seul index vectoriel).
fn fetch_by_chunk_id(
    searcher: &tantivy::Searcher,
    f_chunk_id: tantivy::schema::Field,
    chunk_id: u64,
) -> tantivy::Result<Option<TantivyDocument>> {
    let term = Term::from_field_u64(f_chunk_id, chunk_id);
    let query = TermQuery::new(term, IndexRecordOption::Basic);
    let top = searcher.search(&query, &TopDocs::with_limit(1).order_by_score())?;
    match top.first() {
        Some((_, addr)) => Ok(Some(searcher.doc(*addr)?)),
        None => Ok(None),
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}
