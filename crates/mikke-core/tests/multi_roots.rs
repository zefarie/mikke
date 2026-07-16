//! Plusieurs racines d'indexation : s'ajouter, ne pas s'effacer.

use std::fs;
use std::path::Path;

fn top_path(index_dir: &Path, query: &str) -> Option<String> {
    mikke_core::search(index_dir, query, 10, None)
        .unwrap()
        .into_iter()
        .next()
        .map(|h| h.path)
}

#[test]
fn deux_racines_coexistent() {
    let docs = tempfile::tempdir().unwrap();
    let dl = tempfile::tempdir().unwrap();
    fs::write(
        docs.path().join("bail.md"),
        "Contrat de location : loyer mensuel de 520 euros.",
    )
    .unwrap();
    fs::write(
        dl.path().join("billet.md"),
        "Confirmation e-billet TGV Toulouse Paris, voiture 12.",
    )
    .unwrap();
    let index_dir = tempfile::tempdir().unwrap();

    // première racine seule
    let stats = mikke_core::build_index(
        &[docs.path().to_path_buf()],
        &[],
        index_dir.path(),
        None,
        false,
    )
    .unwrap();
    assert_eq!(stats.files_indexed, 1);

    // deuxième racine AJOUTÉE : la première ne doit pas être effacée
    let stats = mikke_core::build_index(
        &[docs.path().to_path_buf(), dl.path().to_path_buf()],
        &[],
        index_dir.path(),
        None,
        false,
    )
    .unwrap();
    assert_eq!(stats.files_indexed, 1, "seul billet.md est nouveau");
    assert_eq!(stats.files_deleted, 0, "bail.md ne doit PAS sortir");
    assert!(top_path(index_dir.path(), "loyer mensuel").is_some_and(|p| p.ends_with("bail.md")));
    assert!(top_path(index_dir.path(), "billet TGV").is_some_and(|p| p.ends_with("billet.md")));

    // racine retirée : ses fichiers sortent de l'index
    let stats = mikke_core::build_index(
        &[dl.path().to_path_buf()],
        &[],
        index_dir.path(),
        None,
        false,
    )
    .unwrap();
    assert_eq!(stats.files_deleted, 1);
    assert!(top_path(index_dir.path(), "loyer mensuel").is_none());
}

#[test]
fn exclusions_et_racines_imbriquees() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("visible.md"),
        "Attestation de recensement.",
    )
    .unwrap();
    let prive = root.path().join("prive");
    fs::create_dir(&prive).unwrap();
    fs::write(prive.join("secret.md"), "Journal intime, ne pas indexer.").unwrap();

    let index_dir = tempfile::tempdir().unwrap();
    // racine + la même racine imbriquée : pas de doublons ; `prive` exclu
    let stats = mikke_core::build_index(
        &[root.path().to_path_buf(), root.path().to_path_buf()],
        std::slice::from_ref(&prive),
        index_dir.path(),
        None,
        false,
    )
    .unwrap();
    assert_eq!(stats.files_indexed, 1, "pas de doublon, pas d'exclu");
    assert!(top_path(index_dir.path(), "journal intime").is_none());
    assert!(top_path(index_dir.path(), "recensement").is_some());
}
