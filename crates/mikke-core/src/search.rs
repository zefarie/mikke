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
/// En dessous de cette fraction du meilleur score RRF, un résultat est
/// écarté. Effet : quand BM25 et les vecteurs s'accordent sur une tête de
/// liste, la queue qui n'a été vue que par un seul des deux (miettes BM25
/// sur un stopword, voisin vectoriel lointain) disparaît. Quand il n'y a
/// pas de consensus, tous les scores se tiennent et rien n'est coupé.
const MIN_SCORE_RATIO: f32 = 0.5;

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

    // le nom de fichier participe au score (index d'avant : champ absent)
    let mut fields = vec![f_body];
    let f_filename = schema.get_field("filename").ok();
    if let Some(f) = f_filename {
        fields.push(f);
    }
    let boost_filename = |parser: &mut QueryParser| {
        if let Some(f) = f_filename {
            parser.set_field_boost(f, 2.0);
        }
    };

    let searcher = open.reader.searcher();
    let mut parser = QueryParser::for_index(index, fields.clone());
    boost_filename(&mut parser);
    // lenient : une requête utilisateur n'est jamais une erreur de syntaxe
    let (query, _) = parser.parse_query_lenient(query_str);

    // même requête en conjonction : ne remonte que les documents qui
    // contiennent TOUS les termes — dans la fusion, ils écrasent ceux qui
    // n'ont accroché qu'une miette (« 9 » dans un log, par exemple)
    let mut and_parser = QueryParser::for_index(index, fields);
    boost_filename(&mut and_parser);
    and_parser.set_conjunction_by_default();
    let (and_query, _) = and_parser.parse_query_lenient(query_str);

    let mut docs_by_id: HashMap<u64, TantivyDocument> = HashMap::new();
    let mut lists = vec![
        collect_ranked(&searcher, &*and_query, f_chunk_id, &mut docs_by_id)?,
        collect_ranked(&searcher, &*query, f_chunk_id, &mut docs_by_id)?,
    ];

    // côté vecteurs : voisins du même embedding que l'index
    if let (Some(embedder), Some(vectors)) = (embedder, open.vectors.as_ref()) {
        match embedder.embed(query_str) {
            Ok(qvec) => lists.push(vectors.search(&qvec, CHUNK_POOL)),
            Err(e) => eprintln!("warn: embedding de la requête impossible ({e})"),
        }
    }

    let fused = rrf(&lists);
    let cutoff = fused.first().map(|(_, top)| top * MIN_SCORE_RATIO);

    let mut snippets = SnippetGenerator::create(&searcher, &*query, f_body)?;
    snippets.set_max_num_chars(SNIPPET_MAX_CHARS);

    let mut seen_paths = HashSet::new();
    let mut hits = Vec::new();
    for (chunk_id, score) in fused {
        if cutoff.is_some_and(|c| score < c) {
            break; // la liste est triée : tout ce qui suit est en dessous
        }
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

/// Chunks classés pour une requête tantivy, documents mémorisés au passage.
fn collect_ranked(
    searcher: &tantivy::Searcher,
    query: &dyn tantivy::query::Query,
    f_chunk_id: tantivy::schema::Field,
    docs_by_id: &mut HashMap<u64, TantivyDocument>,
) -> tantivy::Result<Vec<u64>> {
    let mut ranked = Vec::new();
    for (_score, addr) in
        searcher.search(query, &TopDocs::with_limit(CHUNK_POOL).order_by_score())?
    {
        let doc: TantivyDocument = searcher.doc(addr)?;
        if let Some(id) = doc.get_first(f_chunk_id).and_then(|v| v.as_u64()) {
            ranked.push(id);
            docs_by_id.entry(id).or_insert(doc);
        }
    }
    Ok(ranked)
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
