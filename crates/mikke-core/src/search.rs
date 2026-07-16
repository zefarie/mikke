//! Recherche BM25 avec extraits surlignés.
//!
//! La recherche se fait au niveau des chunks ; on garde le meilleur chunk de
//! chaque fichier pour ne jamais montrer deux fois le même document.

use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;

use serde::Serialize;
use tantivy::TantivyDocument;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::Value;
use tantivy::snippet::SnippetGenerator;

use crate::index::open_index;

/// Combien de chunks on ramène avant dédoublonnage par fichier.
const CHUNK_POOL: usize = 50;
const SNIPPET_MAX_CHARS: usize = 240;

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub score: f32,
    /// Extrait du chunk le mieux classé.
    pub snippet: String,
    /// Plages d'octets de `snippet` correspondant aux termes de la requête.
    pub highlights: Vec<Range<usize>>,
}

pub fn search(index_dir: &Path, query_str: &str, limit: usize) -> tantivy::Result<Vec<SearchHit>> {
    let index = open_index(index_dir)?;
    let schema = index.schema();
    let f_path = schema.get_field("path").expect("champ path");
    let f_body = schema.get_field("body").expect("champ body");

    let searcher = index.reader()?.searcher();
    let parser = QueryParser::for_index(&index, vec![f_body]);
    // lenient : une requête utilisateur n'est jamais une erreur de syntaxe
    let (query, _) = parser.parse_query_lenient(query_str);
    let top_chunks = searcher.search(&query, &TopDocs::with_limit(CHUNK_POOL).order_by_score())?;

    let mut snippets = SnippetGenerator::create(&searcher, &*query, f_body)?;
    snippets.set_max_num_chars(SNIPPET_MAX_CHARS);

    let mut seen = HashSet::new();
    let mut hits = Vec::new();
    for (score, addr) in top_chunks {
        let doc: TantivyDocument = searcher.doc(addr)?;
        let path = doc
            .get_first(f_path)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if !seen.insert(path.clone()) {
            continue;
        }
        let snippet = snippets.snippet_from_doc(&doc);
        let (fragment, highlights) = if snippet.fragment().is_empty() {
            // requête sans position exploitable : début du chunk en secours
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

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}
