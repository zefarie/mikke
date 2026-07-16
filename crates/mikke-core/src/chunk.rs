//! Découpage des documents en chunks de taille bornée pour l'indexation.

/// Découpe `text` en morceaux d'environ `target` mots, avec `overlap` mots
/// de recouvrement entre morceaux consécutifs. Les tranches sont prises dans
/// le texte original : ponctuation et sauts de ligne des extraits sont
/// préservés.
pub fn chunk(text: &str, target: usize, overlap: usize) -> Vec<String> {
    assert!(target > 0 && overlap < target);
    let words = word_offsets(text);
    if words.is_empty() {
        return Vec::new();
    }
    let step = target - overlap;
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let end = (i + target).min(words.len());
        let (start_byte, _) = words[i];
        let (_, end_byte) = words[end - 1];
        out.push(text[start_byte..end_byte].to_string());
        if end == words.len() {
            break;
        }
        i += step;
    }
    out
}

/// Offsets (début, fin) en octets de chaque mot (séquence non blanche).
fn word_offsets(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                out.push((s, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(n: usize) -> String {
        (0..n)
            .map(|i| format!("mot{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn texte_court_un_seul_chunk() {
        let text = words(50);
        let chunks = chunk(&text, 400, 80);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn overlap_entre_chunks_consecutifs() {
        let text = words(1000);
        let chunks = chunk(&text, 400, 80);
        // 1000 mots, pas de 320 : chunks à 0, 320, 640 → 3 chunks
        assert_eq!(chunks.len(), 3);
        // le 2e chunk commence 80 mots avant la fin du 1er
        assert!(chunks[1].starts_with("mot320"));
        assert!(chunks[0].ends_with("mot399"));
    }

    #[test]
    fn texte_vide_ou_blanc() {
        assert!(chunk("", 400, 80).is_empty());
        assert!(chunk("   \n\t  ", 400, 80).is_empty());
    }

    #[test]
    fn preserve_la_ponctuation_et_les_sauts_de_ligne() {
        let text = "Facture n° 2026-01.\nTotal : 85,00 €";
        let chunks = chunk(text, 400, 80);
        assert_eq!(chunks[0], text);
    }
}
