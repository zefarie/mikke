//! mikke indexe des documents, pas du code — et le nom du fichier compte.

use std::fs;

#[test]
fn les_projets_de_code_sont_sautes() {
    let corpus = tempfile::tempdir().unwrap();
    fs::write(
        corpus.path().join("notes.md"),
        "Idée cadeau : un livre de cuisine japonaise.",
    )
    .unwrap();
    let projet = corpus.path().join("mon-mod");
    fs::create_dir(&projet).unwrap();
    fs::write(projet.join("build.gradle"), "plugins { id 'java' }").unwrap();
    fs::write(
        projet.join("notes-de-dev.md"),
        "Assert.isTrue loop assign fourbi de tests",
    )
    .unwrap();
    // jar décompressé : META-INF à la racine
    let archive = corpus.path().join("mod-extrait");
    fs::create_dir_all(archive.join("META-INF")).unwrap();
    fs::write(
        archive.join("License.txt"),
        "fourbi de licence GPL verbeuse",
    )
    .unwrap();

    let index_dir = tempfile::tempdir().unwrap();
    let stats = mikke_core::build_index(corpus.path(), index_dir.path(), None, false).unwrap();
    assert_eq!(stats.files_indexed, 1, "seul notes.md doit être indexé");
    assert_eq!(stats.code_dirs_skipped, 2);

    let hits = mikke_core::search(index_dir.path(), "fourbi tests", 10, None).unwrap();
    assert!(
        hits.is_empty(),
        "le contenu du projet de code ne doit pas sortir"
    );
}

#[test]
fn une_miette_ne_suffit_pas_a_sortir() {
    // le scénario réel : des cours, et un rapport technique qui contient
    // un « 9 » perdu dans une regex — il ne doit PAS sortir
    let corpus = tempfile::tempdir().unwrap();
    fs::write(
        corpus.path().join("cours_chapitre_9.md"),
        "Chapitre 9 : changement d'état et transfert thermique. Physique de première.",
    )
    .unwrap();
    fs::write(
        corpus.path().join("cours_chapitre_10.md"),
        "Chapitre 10 : des acides et des bases. Physique et chimie des solutions.",
    )
    .unwrap();
    fs::write(
        corpus.path().join("rapport_deobf.md"),
        "Détection des proxies : getThreadName().matches(\"[a-zA-Z0-9]\") ajoute le \
         thread à la liste des suspects. Analyse du bytecode et des mixins.",
    )
    .unwrap();

    let index_dir = tempfile::tempdir().unwrap();
    mikke_core::build_index(corpus.path(), index_dir.path(), None, false).unwrap();

    let hits =
        mikke_core::search(index_dir.path(), "cours de physique chapitre 9", 10, None).unwrap();
    assert!(
        hits.first()
            .is_some_and(|h| h.path.ends_with("cours_chapitre_9.md")),
        "le chapitre 9 doit être premier"
    );
    assert!(
        !hits.iter().any(|h| h.path.ends_with("rapport_deobf.md")),
        "un match sur le seul « 9 » ne doit pas sortir : {:?}",
        hits.iter().map(|h| &h.path).collect::<Vec<_>>()
    );
}

#[test]
fn le_nom_du_fichier_pese_dans_le_score() {
    let corpus = tempfile::tempdir().unwrap();
    fs::write(
        corpus.path().join("facture_electricite.md"),
        "Montant du mois de mars, prélèvement automatique le cinq.",
    )
    .unwrap();
    fs::write(
        corpus.path().join("cours_generalites.md"),
        "L'électricité est le déplacement de charges dans un conducteur.",
    )
    .unwrap();

    let index_dir = tempfile::tempdir().unwrap();
    mikke_core::build_index(corpus.path(), index_dir.path(), None, false).unwrap();

    // « facture » n'apparaît que dans le NOM du premier fichier
    let hits = mikke_core::search(index_dir.path(), "facture électricité", 10, None).unwrap();
    assert!(
        hits.first()
            .is_some_and(|h| h.path.ends_with("facture_electricite.md")),
        "le nom de fichier doit primer : {:?}",
        hits.iter().map(|h| &h.path).collect::<Vec<_>>()
    );
}
