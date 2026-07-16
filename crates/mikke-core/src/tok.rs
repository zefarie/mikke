//! Tokenizer Unigram (SentencePiece) maison, sur FST compact.
//!
//! Le tokenizer HuggingFace met ~700 ms et ~550 Mo de RAM à charger le
//! vocabulaire bge-m3 (500 353 entrées) — intenable pour une CLI qui doit
//! répondre en moins de 100 ms. Ici, le `tokenizer.json` est converti UNE
//! fois en un cache compact : FST (token → id), scores f64, et le charsmap
//! Precompiled de SentencePiece tel quel. Rechargement en ~10 ms.
//!
//! Le pipeline reproduit celui du modèle : normalisation Precompiled
//! (via `spm_precompiled`, l'implémentation de référence extraite de
//! HuggingFace), Metaspace `split=false` (chaque espace → `▁`, un `▁`
//! préfixé), segmentation Viterbi du Unigram. Les caractères hors
//! vocabulaire sont abandonnés, comme model2vec jette les `<unk>`.
//! Fidélité mesurée par l'exemple `tok_check`.

use std::io::Write;
use std::path::Path;

use base64::Engine;
use thiserror::Error;

pub const CACHE_FILE: &str = "tokenizer.mikke";
const MAGIC: &[u8; 8] = b"MIKKETK2";
/// Score d'un caractère inconnu : toujours pire que n'importe quel vrai token.
const UNK_SCORE: f64 = -1e6;
/// Au-delà, un texte ne peut plus produire de nouveaux tokens utiles
/// (l'embedding est de toute façon tronqué à 512 tokens).
const MAX_INPUT_CHARS: usize = 8192;

#[derive(Debug, Error)]
pub enum TokError {
    #[error("cache tokenizer illisible : {0}")]
    Io(#[from] std::io::Error),
    #[error("tokenizer invalide : {0}")]
    Format(String),
}

fn format_err(e: impl std::fmt::Display) -> TokError {
    TokError::Format(e.to_string())
}

#[derive(serde::Deserialize)]
struct TokenizerJson {
    normalizer: serde_json::Value,
    model: ModelJson,
}

#[derive(serde::Deserialize)]
struct ModelJson {
    #[serde(rename = "type")]
    kind: String,
    vocab: Vec<(String, f64)>,
}

/// Cherche récursivement le blob `precompiled_charsmap` dans le normalizer.
fn find_charsmap(v: &serde_json::Value) -> Option<&str> {
    match v {
        serde_json::Value::Object(o) => {
            if let Some(s) = o.get("precompiled_charsmap").and_then(|s| s.as_str()) {
                return Some(s);
            }
            o.values().find_map(find_charsmap)
        }
        serde_json::Value::Array(a) => a.iter().find_map(find_charsmap),
        _ => None,
    }
}

pub struct Tok {
    map: fst::Map<Vec<u8>>,
    scores: Vec<f64>,
    /// None : identité (utilisé par les tests ; spm panique sur un trie vide).
    charsmap: Option<spm_precompiled::Precompiled>,
}

/// La ponctuation ASCII que le normalizer du modèle entoure d'espaces
/// (32 règles Replace dans son tokenizer.json — validé par `tok_check`).
fn is_padded_punct(c: char) -> bool {
    matches!(c, '!'..='/' | ':'..='@' | '['..='`' | '{'..='~')
}

impl Tok {
    /// Convertit un `tokenizer.json` HuggingFace (modèle Unigram) en cache
    /// compact. À ne faire qu'une fois : c'est l'opération lente (~2 s).
    pub fn build_cache(tokenizer_json: &Path, cache: &Path) -> Result<(), TokError> {
        let raw = std::fs::read_to_string(tokenizer_json)?;
        let parsed: TokenizerJson = serde_json::from_str(&raw).map_err(format_err)?;
        if parsed.model.kind != "Unigram" {
            return Err(TokError::Format(format!(
                "modèle {} non géré (Unigram attendu)",
                parsed.model.kind
            )));
        }
        let charsmap_b64 = find_charsmap(&parsed.normalizer)
            .ok_or_else(|| TokError::Format("precompiled_charsmap introuvable".into()))?;
        let charsmap = base64::engine::general_purpose::STANDARD
            .decode(charsmap_b64)
            .map_err(format_err)?;

        let scores: Vec<f64> = parsed.model.vocab.iter().map(|(_, s)| *s).collect();
        let mut entries: Vec<(&str, u64)> = parsed
            .model
            .vocab
            .iter()
            .enumerate()
            .map(|(id, (token, _))| (token.as_str(), id as u64))
            .collect();
        entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        entries.dedup_by(|a, b| a.0 == b.0);

        let mut builder = fst::MapBuilder::memory();
        for (token, id) in entries {
            builder.insert(token, id).map_err(format_err)?;
        }
        let fst_bytes = builder.into_inner().map_err(format_err)?;

        let tmp = cache.with_extension("part");
        let mut out = std::fs::File::create(&tmp)?;
        out.write_all(MAGIC)?;
        out.write_all(&(fst_bytes.len() as u64).to_le_bytes())?;
        out.write_all(&fst_bytes)?;
        out.write_all(&(scores.len() as u64).to_le_bytes())?;
        for s in &scores {
            out.write_all(&s.to_le_bytes())?;
        }
        out.write_all(&(charsmap.len() as u64).to_le_bytes())?;
        out.write_all(&charsmap)?;
        out.flush()?;
        std::fs::rename(&tmp, cache)?;
        Ok(())
    }

    pub fn load(cache: &Path) -> Result<Self, TokError> {
        let bytes = std::fs::read(cache)?;
        let corrupt =
            || TokError::Format("cache corrompu (supprime-le, il sera reconstruit)".into());
        if bytes.len() < 16 || &bytes[..8] != MAGIC {
            return Err(corrupt());
        }
        fn read_len(bytes: &[u8], pos: &mut usize) -> Result<usize, TokError> {
            let raw = bytes
                .get(*pos..*pos + 8)
                .ok_or_else(|| TokError::Format("cache tronqué".into()))?;
            *pos += 8;
            Ok(u64::from_le_bytes(raw.try_into().expect("8 octets")) as usize)
        }
        let mut pos = 8;

        let fst_len = read_len(&bytes, &mut pos)?;
        let fst_bytes = bytes.get(pos..pos + fst_len).ok_or_else(corrupt)?.to_vec();
        pos += fst_len;
        let map = fst::Map::new(fst_bytes).map_err(format_err)?;

        let n_scores = read_len(&bytes, &mut pos)?;
        let raw = bytes.get(pos..pos + n_scores * 8).ok_or_else(corrupt)?;
        pos += n_scores * 8;
        let scores: Vec<f64> = raw
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().expect("8 octets")))
            .collect();

        let cm_len = read_len(&bytes, &mut pos)?;
        let cm = bytes.get(pos..pos + cm_len).ok_or_else(corrupt)?;
        let charsmap = if cm_len > 4 {
            Some(spm_precompiled::Precompiled::from(cm).map_err(format_err)?)
        } else {
            None
        };

        Ok(Self {
            map,
            scores,
            charsmap,
        })
    }

    /// Ids de tokens du texte, comme le ferait le pipeline de référence :
    /// charsmap Precompiled, ponctuation ASCII entourée d'espaces, blancs
    /// fusionnés, strip, Metaspace (split=false), puis Viterbi Unigram.
    pub fn tokenize(&self, text: &str) -> Vec<u32> {
        let truncated: String = if text.chars().count() > MAX_INPUT_CHARS {
            text.chars().take(MAX_INPUT_CHARS).collect()
        } else {
            text.to_string()
        };
        let normalized = match &self.charsmap {
            Some(cm) => cm.normalize_string(&truncated),
            None => truncated,
        };
        // Replace ponctuation + \s+ → " " + Strip + Metaspace, en une passe
        let mut s = String::with_capacity(normalized.len() + 16);
        let mut sep = false;
        for c in normalized.chars() {
            if c.is_whitespace() {
                sep = true;
                continue;
            }
            let pad = is_padded_punct(c);
            if !s.is_empty() && (sep || pad) {
                s.push('▁');
            }
            s.push(c);
            sep = pad;
        }
        if s.is_empty() {
            return Vec::new();
        }
        s.insert(0, '▁');
        self.viterbi(&s)
    }

    fn viterbi(&self, s: &str) -> Vec<u32> {
        let bytes = s.as_bytes();
        let n = bytes.len();
        if n == 0 {
            return Vec::new();
        }
        let fst = self.map.as_fst();
        let mut best = vec![f64::NEG_INFINITY; n + 1];
        let mut back: Vec<(usize, Option<u32>)> = vec![(0, None); n + 1];
        best[0] = 0.0;

        for i in 0..n {
            if best[i] == f64::NEG_INFINITY || !s.is_char_boundary(i) {
                continue;
            }
            // repli : caractère inconnu abandonné (équivalent <unk> jeté)
            let mut next = i + 1;
            while next < n && !s.is_char_boundary(next) {
                next += 1;
            }
            let fallback = best[i] + UNK_SCORE;
            if fallback > best[next] {
                best[next] = fallback;
                back[next] = (i, None);
            }
            // toutes les entrées du vocabulaire préfixes de bytes[i..],
            // en une seule descente du FST
            let mut node = fst.root();
            let mut out = fst::raw::Output::zero();
            for (k, &b) in bytes.iter().enumerate().skip(i) {
                let Some(t_ix) = node.find_input(b) else {
                    break;
                };
                let t = node.transition(t_ix);
                out = out.cat(t.out);
                node = fst.node(t.addr);
                let end = k + 1;
                if node.is_final() && s.is_char_boundary(end) {
                    let id = out.cat(node.final_output()).value() as u32;
                    let score = self.scores.get(id as usize).copied().unwrap_or(UNK_SCORE);
                    let cand = best[i] + score;
                    if cand > best[end] {
                        best[end] = cand;
                        back[end] = (i, Some(id));
                    }
                }
            }
        }

        let mut ids = Vec::new();
        let mut pos = n;
        while pos > 0 {
            let (prev, id) = back[pos];
            if let Some(id) = id {
                ids.push(id);
            }
            pos = prev;
        }
        ids.reverse();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mini_tok() -> Tok {
        // mini vocab : construit le FST directement en mémoire ; charsmap
        // vide (aucune règle → identité)
        let vocab: &[(&str, f64, u64)] = &[
            ("s", -8.0, 5),
            ("▁chat", -4.0, 1),
            ("▁le", -3.0, 0),
            ("▁les", -3.5, 4),
            ("cha", -6.0, 2),
            ("t", -7.0, 3),
        ];
        let mut scores = vec![UNK_SCORE; 6];
        for (_, score, id) in vocab {
            scores[*id as usize] = *score;
        }
        let mut sorted: Vec<_> = vocab.to_vec();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        let mut b = fst::MapBuilder::memory();
        for (tok, _, id) in sorted {
            b.insert(tok, id).unwrap();
        }
        let map = fst::Map::new(b.into_inner().unwrap()).unwrap();
        Tok {
            map,
            scores,
            charsmap: None,
        }
    }

    #[test]
    fn segmentation_choisit_le_meilleur_score() {
        let tok = mini_tok();
        assert_eq!(tok.tokenize("le chat"), vec![0, 1]);
    }

    #[test]
    fn pluriel_prefere_le_token_long() {
        let tok = mini_tok();
        // "les" : ▁les (-3.5) bat ▁le + s (-3 - 8 = -11)
        assert_eq!(tok.tokenize("les"), vec![4]);
    }

    #[test]
    fn caractere_inconnu_est_abandonne() {
        let tok = mini_tok();
        // Ω n'est pas dans le vocab : jeté ; "chat" sans ▁ passe par cha+t
        assert_eq!(tok.tokenize("le Ωchat"), vec![0, 2, 3]);
    }

    #[test]
    fn texte_vide() {
        let tok = mini_tok();
        assert!(tok.tokenize("").is_empty());
        assert!(tok.tokenize("   ").is_empty());
    }
}
