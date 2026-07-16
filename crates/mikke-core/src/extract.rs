//! Extraction du texte brut des fichiers. Formats v1 : txt et md pour
//! l'instant ; pdf, docx et html arrivent à l'étape 4 du plan.

use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("lecture impossible : {0}")]
    Io(#[from] std::io::Error),
}

/// Le fichier est-il d'un format que mikke sait lire ?
pub fn supported(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("txt" | "text" | "md" | "markdown")
    )
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

/// Extrait le texte d'un fichier supporté. Les octets non-UTF-8 sont
/// remplacés au lieu de faire échouer l'indexation.
pub fn extract(path: &Path) -> Result<String, ExtractError> {
    let bytes = std::fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
