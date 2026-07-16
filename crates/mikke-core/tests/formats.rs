//! Extraction et indexation des formats pdf, docx, html.

use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn pdf_texte() {
    let text = mikke_core::extract::extract(&fixture("sample.pdf")).unwrap();
    assert!(
        text.contains("douche italienne"),
        "texte extrait : {text:?}"
    );
    assert!(text.contains("4 250 euros"));
}

#[test]
fn docx_texte() {
    let text = mikke_core::extract::extract(&fixture("sample.docx")).unwrap();
    assert!(text.contains("Attestation de scolarite"));
    // deux runs du même paragraphe recollés, paragraphes séparés
    assert!(text.contains("classe de premiere au lycee."));
    assert!(text.contains('\n'));
}

#[test]
fn html_texte_sans_script_ni_style() {
    let text = mikke_core::extract::extract(&fixture("sample.html")).unwrap();
    assert!(text.contains("casque audio"));
    assert!(text.contains("129,90"));
    assert!(!text.contains("bruit de script"));
    assert!(!text.contains("color: red"));
}

#[test]
fn pdf_corrompu_erreur_sans_panic() {
    let err = mikke_core::extract::extract(&fixture("corrupted.pdf"));
    assert!(err.is_err());
}

#[test]
fn indexation_multi_formats() {
    let corpus = tempfile::tempdir().unwrap();
    for name in ["sample.pdf", "sample.docx", "sample.html", "corrupted.pdf"] {
        std::fs::copy(fixture(name), corpus.path().join(name)).unwrap();
    }
    std::fs::write(
        corpus.path().join("note.md"),
        "Le mot de passe du wifi est dans le tiroir.",
    )
    .unwrap();

    let index_dir = tempfile::tempdir().unwrap();
    let stats = mikke_core::build_index(corpus.path(), index_dir.path(), None).unwrap();
    assert_eq!(stats.files_indexed, 4, "pdf + docx + html + md");
    assert_eq!(stats.files_failed, 1, "le pdf corrompu, sans crash");

    for (query, expect) in [
        ("douche italienne", "sample.pdf"),
        ("attestation scolarite", "sample.docx"),
        ("casque audio", "sample.html"),
        ("mot de passe wifi", "note.md"),
    ] {
        let hits = mikke_core::search(index_dir.path(), query, 10, None).unwrap();
        assert!(
            hits.first().is_some_and(|h| h.path.ends_with(expect)),
            "requête {query:?} devait renvoyer {expect}"
        );
    }
}
