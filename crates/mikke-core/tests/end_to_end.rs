//! Test bout en bout : corpus → index → recherche.

use std::fs;
use std::path::Path;

fn write(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
}

fn corpus() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "facture_veto.md",
        "# Facture\n\nClinique vétérinaire des Carmes.\nConsultation et vaccination du chat \
         Perceval.\nTotal dû : 85,00 euros — janvier 2026.",
    );
    write(
        dir.path(),
        "recette.txt",
        "Recette de la tarte aux pommes : pâte brisée, pommes, sucre, cannelle. \
         Cuire quarante minutes au four à 180 degrés.",
    );
    write(
        dir.path(),
        "cours_pythagore.md",
        "Théorème de Pythagore : dans un triangle rectangle, le carré de l'hypoténuse \
         est égal à la somme des carrés des deux autres côtés.",
    );
    // ne doit jamais être indexé (format non supporté)
    write(dir.path(), "binaire.bin", "\u{0}\u{1}\u{2}");
    dir
}

#[test]
fn index_puis_recherche() {
    let corpus = corpus();
    let index_dir = tempfile::tempdir().unwrap();

    let stats = mikke_core::build_index(corpus.path(), index_dir.path()).unwrap();
    assert_eq!(stats.files_indexed, 3);
    assert_eq!(stats.files_skipped, 1);
    assert_eq!(stats.files_failed, 0);
    assert!(stats.chunks >= 3);

    let hits = mikke_core::search(index_dir.path(), "vaccination chat", 10).unwrap();
    assert!(!hits.is_empty());
    assert!(hits[0].path.ends_with("facture_veto.md"));
    assert!(!hits[0].snippet.is_empty());
    assert!(!hits[0].highlights.is_empty());
}

#[test]
fn les_accents_ne_comptent_pas() {
    let corpus = corpus();
    let index_dir = tempfile::tempdir().unwrap();
    mikke_core::build_index(corpus.path(), index_dir.path()).unwrap();

    // « veterinaire » sans accent doit retrouver « vétérinaire »
    let hits = mikke_core::search(index_dir.path(), "veterinaire", 10).unwrap();
    assert!(!hits.is_empty());
    assert!(hits[0].path.ends_with("facture_veto.md"));

    // et l'inverse : requête accentuée sur texte accentué
    let hits = mikke_core::search(index_dir.path(), "théorème hypoténuse", 10).unwrap();
    assert!(hits[0].path.ends_with("cours_pythagore.md"));
}

#[test]
fn requete_sans_resultat() {
    let corpus = corpus();
    let index_dir = tempfile::tempdir().unwrap();
    mikke_core::build_index(corpus.path(), index_dir.path()).unwrap();

    let hits = mikke_core::search(index_dir.path(), "zygomatique quaternion", 10).unwrap();
    assert!(hits.is_empty());
}

#[test]
fn un_fichier_par_resultat() {
    let corpus = corpus();
    let index_dir = tempfile::tempdir().unwrap();
    mikke_core::build_index(corpus.path(), index_dir.path()).unwrap();

    // « pommes » apparaît plusieurs fois dans recette.txt : un seul hit attendu
    let hits = mikke_core::search(index_dir.path(), "pommes", 10).unwrap();
    assert_eq!(hits.len(), 1);
}
