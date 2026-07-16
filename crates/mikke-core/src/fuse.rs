//! Fusion de classements par Reciprocal Rank Fusion (RRF).
//!
//! RRF ne compare jamais les scores bruts (BM25 et distance cosinus ne sont
//! pas commensurables) : seul le rang compte. score(d) = Σ 1/(k + rang).

const RRF_K: f32 = 60.0;

/// Fusionne des listes d'ids classées (meilleur en premier). Retourne les ids
/// avec leur score RRF, du meilleur au moins bon, de façon déterministe.
pub fn rrf(lists: &[Vec<u64>]) -> Vec<(u64, f32)> {
    let mut scores: std::collections::HashMap<u64, f32> = std::collections::HashMap::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
    }
    let mut fused: Vec<(u64, f32)> = scores.into_iter().collect();
    fused.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_dans_les_deux_listes_gagne() {
        let bm25 = vec![1, 2, 3];
        let vecteurs = vec![9, 2, 8];
        let fused = rrf(&[bm25, vecteurs]);
        assert_eq!(fused[0].0, 2); // seul id présent des deux côtés
    }

    #[test]
    fn une_seule_liste_conserve_l_ordre() {
        let fused = rrf(&[vec![7, 5, 3]]);
        let ids: Vec<u64> = fused.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![7, 5, 3]);
    }

    #[test]
    fn vide() {
        assert!(rrf(&[]).is_empty());
        assert!(rrf(&[vec![], vec![]]).is_empty());
    }
}
