//! Extraction du texte brut des fichiers. Formats v1 : txt, md, pdf, docx,
//! html. Un fichier corrompu retourne une erreur, jamais un panic — même
//! quand la crate sous-jacente panique (pdf-extract le fait, cf. ADR 0001).

use std::io::Read;
use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("lecture impossible : {0}")]
    Io(#[from] std::io::Error),
    #[error("fichier corrompu : {0}")]
    Corrupt(String),
}

/// Le fichier est-il d'un format que mikke sait lire ?
pub fn supported(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("txt" | "text" | "md" | "markdown" | "pdf" | "docx" | "html" | "htm" | "xhtml")
    )
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

/// Extrait le texte d'un fichier supporté. Un PDF scanné (sans couche
/// texte) retourne une chaîne vide : à compter « ignoré », pas « erreur ».
pub fn extract(path: &Path) -> Result<String, ExtractError> {
    match extension(path).as_deref() {
        Some("pdf") => pdf_text(path),
        Some("docx") => docx_text(path),
        Some("html" | "htm" | "xhtml") => html_text(path),
        _ => {
            // txt/md : les octets non-UTF-8 sont remplacés au lieu d'échouer
            let bytes = std::fs::read(path)?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
    }
}

/// pdf-extract panique sur certains PDF malformés (1/50 sur le corpus du
/// spike) : le panic est converti en `ExtractError::Corrupt`.
fn pdf_text(path: &Path) -> Result<String, ExtractError> {
    let path = path.to_path_buf();
    let result = std::panic::catch_unwind(move || pdf_extract::extract_text(&path));
    match result {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) => Err(ExtractError::Corrupt(e.to_string())),
        Err(_) => Err(ExtractError::Corrupt("panic dans pdf-extract".into())),
    }
}

/// Un .docx est un zip ; le texte vit dans word/document.xml, dans les
/// éléments <w:t>. Fins de paragraphe <w:p> → sauts de ligne.
fn docx_text(path: &Path) -> Result<String, ExtractError> {
    let corrupt = |e: &dyn std::fmt::Display| ExtractError::Corrupt(e.to_string());
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| corrupt(&e))?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|e| corrupt(&e))?
        .read_to_string(&mut xml)
        .map_err(|e| corrupt(&e))?;

    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut out = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) if e.local_name().as_ref() == b"t" => {
                in_text = true;
            }
            Ok(quick_xml::events::Event::End(e)) => match e.local_name().as_ref() {
                b"t" => in_text = false,
                b"p" => out.push('\n'),
                _ => {}
            },
            Ok(quick_xml::events::Event::Empty(e)) if e.local_name().as_ref() == b"tab" => {
                out.push(' ');
            }
            Ok(quick_xml::events::Event::Text(t)) if in_text => {
                out.push_str(&t.decode().map_err(|e| corrupt(&e))?);
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(corrupt(&e)),
        }
    }
    Ok(out)
}

/// html2text ignore correctement <script> et <style>, contrairement à une
/// extraction naïve des nœuds texte.
fn html_text(path: &Path) -> Result<String, ExtractError> {
    let bytes = std::fs::read(path)?;
    html2text::config::plain()
        .string_from_read(&bytes[..], 200)
        .map_err(|e| ExtractError::Corrupt(e.to_string()))
}
