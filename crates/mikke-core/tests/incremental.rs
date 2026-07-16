//! Indexation incrémentale : inchangé / modifié / touché / supprimé.

use std::fs;
use std::path::Path;

fn write(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
}

fn top_path(index_dir: &Path, query: &str) -> Option<String> {
    mikke_core::search(index_dir, query, 10, None)
        .unwrap()
        .into_iter()
        .next()
        .map(|h| h.path)
}

#[test]
fn cycle_incremental_complet() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    write(
        corpus.path(),
        "velo.md",
        "Réparation du vélo : changer la chambre à air.",
    );
    write(
        corpus.path(),
        "jardin.md",
        "Semer les tomates en avril, arroser le soir.",
    );
    write(
        corpus.path(),
        "chat.md",
        "Le chat Perceval dort sur le radiateur.",
    );

    // premier run : tout est indexé
    let stats = mikke_core::build_index(corpus.path(), index_dir.path(), None, false).unwrap();
    assert_eq!(stats.files_indexed, 3);
    assert_eq!(stats.files_unchanged, 0);

    // deuxième run sans modification : rien n'est relu
    let stats = mikke_core::build_index(corpus.path(), index_dir.path(), None, false).unwrap();
    assert_eq!(stats.files_indexed, 0);
    assert_eq!(stats.files_unchanged, 3);
    assert_eq!(stats.chunks, 3);

    // modification de contenu : un seul fichier réindexé, la recherche suit
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(
        corpus.path(),
        "velo.md",
        "Réparation du vélo : régler les freins à disque.",
    );
    let stats = mikke_core::build_index(corpus.path(), index_dir.path(), None, false).unwrap();
    assert_eq!(stats.files_indexed, 1);
    assert_eq!(stats.files_unchanged, 2);
    assert!(top_path(index_dir.path(), "freins disque").is_some_and(|p| p.ends_with("velo.md")));
    assert!(
        top_path(index_dir.path(), "chambre").is_none(),
        "l'ancien contenu doit avoir disparu"
    );

    // réécriture à l'identique (mtime bouge, hash identique) : inchangé
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(
        corpus.path(),
        "jardin.md",
        "Semer les tomates en avril, arroser le soir.",
    );
    let stats = mikke_core::build_index(corpus.path(), index_dir.path(), None, false).unwrap();
    assert_eq!(stats.files_indexed, 0);
    assert_eq!(stats.files_unchanged, 3);

    // suppression : le fichier sort de l'index
    fs::remove_file(corpus.path().join("chat.md")).unwrap();
    let stats = mikke_core::build_index(corpus.path(), index_dir.path(), None, false).unwrap();
    assert_eq!(stats.files_deleted, 1);
    assert_eq!(stats.chunks, 2);
    assert!(top_path(index_dir.path(), "Perceval radiateur").is_none());

    // --full : reconstruction complète, mêmes résultats
    let stats = mikke_core::build_index(corpus.path(), index_dir.path(), None, true).unwrap();
    assert_eq!(stats.files_indexed, 2);
    assert!(top_path(index_dir.path(), "tomates avril").is_some_and(|p| p.ends_with("jardin.md")));
}
