use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use mikke_core::{Embedder, SearchHit};

const MODEL_NAME: &str = "potion-multilingual-128M";
const MODEL_BASE_URL: &str =
    "https://huggingface.co/minishlab/potion-multilingual-128M/resolve/main";
const MODEL_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];

#[derive(Parser)]
#[command(
    name = "mikke",
    version,
    about = "Retrouve n'importe quel fichier que tu sais décrire.",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// Requête en langage naturel, ex : mikke "facture vétérinaire janvier"
    query: Vec<String>,

    /// Sortie JSON, pour scripter
    #[arg(long)]
    json: bool,

    /// Nombre de résultats
    #[arg(long, default_value_t = 10)]
    top: usize,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Indexe un dossier (réindexation complète pour l'instant)
    Index { dir: PathBuf },
    /// Recherche interactive plein écran (étape 6 de la roadmap)
    Tui,
}

fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("MIKKE_DATA") {
        return PathBuf::from(dir);
    }
    std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").expect("HOME non défini")).join(".local/share")
        })
        .join("mikke")
}

fn model_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("MIKKE_MODEL_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").expect("HOME non défini")).join(".cache")
        })
        .join("mikke")
        .join(MODEL_NAME)
}

fn model_present(dir: &Path) -> bool {
    MODEL_FILES.iter().all(|f| dir.join(f).exists())
}

/// Télécharge le modèle d'embeddings au premier run (une seule fois).
fn ensure_model(dir: &Path) -> Result<()> {
    if model_present(dir) {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    eprintln!(
        "premier lancement : téléchargement du modèle {MODEL_NAME} (~500 Mo, une seule fois)"
    );
    for file in MODEL_FILES {
        let dest = dir.join(file);
        if dest.exists() {
            continue;
        }
        download(&format!("{MODEL_BASE_URL}/{file}"), &dest)
            .with_context(|| format!("téléchargement de {file} impossible"))?;
    }
    Ok(())
}

fn download(url: &str, dest: &Path) -> Result<()> {
    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    let mut resp = ureq::get(url).call()?;
    let total: Option<u64> = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());
    let mut reader = resp.body_mut().as_reader();

    let tmp = dest.with_extension("part");
    let mut out = std::fs::File::create(&tmp)?;
    let mut buf = vec![0u8; 1 << 16];
    let mut done: u64 = 0;
    let mut last_pct: i64 = -1;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        if let Some(total) = total {
            let pct = (done * 100 / total.max(1)) as i64;
            if pct != last_pct {
                eprint!("\r  {name} : {pct}%");
                last_pct = pct;
            }
        }
    }
    out.flush()?;
    eprintln!("\r  {name} : ok      ");
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            // `mikke … | head` coupe le tube : ce n'est pas une erreur
            if let Some(ioe) = e.downcast_ref::<std::io::Error>()
                && ioe.kind() == std::io::ErrorKind::BrokenPipe
            {
                return std::process::ExitCode::SUCCESS;
            }
            eprintln!("erreur : {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let index_dir = data_dir().join("index");

    match cli.command {
        Some(Command::Index { dir }) => cmd_index(&dir, &index_dir),
        Some(Command::Tui) => bail!("le TUI n'existe pas encore (étape 6 de la roadmap)"),
        None if cli.query.is_empty() => {
            Cli::command().print_help()?;
            Ok(())
        }
        None => cmd_search(&cli.query.join(" "), &index_dir, cli.top, cli.json),
    }
}

/// Charge le modèle, en le téléchargeant d'abord si `download_if_missing`.
/// En cas d'échec on continue en BM25 seul : mikke doit toujours répondre.
fn load_embedder(download_if_missing: bool) -> Option<Embedder> {
    let dir = model_dir();
    if !model_present(&dir) {
        if !download_if_missing {
            eprintln!(
                "note : modèle d'embeddings absent, recherche BM25 seule (lance `mikke index` pour le télécharger)"
            );
            return None;
        }
        if let Err(e) = ensure_model(&dir) {
            eprintln!("warn: {e:#} — indexation en BM25 seul");
            return None;
        }
    }
    match Embedder::load(&dir) {
        Ok(e) => Some(e),
        Err(e) => {
            eprintln!("warn: modèle inutilisable ({e}) — BM25 seul");
            None
        }
    }
}

fn cmd_index(dir: &Path, index_dir: &Path) -> Result<()> {
    let dir = dir
        .canonicalize()
        .with_context(|| format!("dossier introuvable : {}", dir.display()))?;
    let embedder = load_embedder(true);
    let start = Instant::now();
    // pdf-extract panique sur les PDF tordus : le panic est déjà converti en
    // erreur comptée « illisible », inutile d'imprimer une backtrace par fichier
    std::panic::set_hook(Box::new(|_| {}));
    let stats = mikke_core::build_index(&dir, index_dir, embedder.as_ref())
        .with_context(|| format!("indexation de {} impossible", dir.display()))?;
    let _ = std::panic::take_hook();
    println!(
        "{} fichiers indexés ({} chunks{}), {} ignorés, {} illisibles — {:.1}s",
        stats.files_indexed,
        stats.chunks,
        if stats.vectors {
            ", BM25 + vecteurs"
        } else {
            ", BM25 seul"
        },
        stats.files_skipped,
        stats.files_failed,
        start.elapsed().as_secs_f32()
    );
    Ok(())
}

fn cmd_search(query: &str, index_dir: &Path, top: usize, json: bool) -> Result<()> {
    if !index_dir.exists() {
        bail!("aucun index — lance d'abord : mikke index ~/Documents");
    }
    let embedder = load_embedder(false);
    let hits = mikke_core::search(index_dir, query, top, embedder.as_ref())
        .context("recherche impossible")?;

    let mut out = std::io::stdout().lock();
    if json {
        writeln!(
            out,
            "{}",
            serde_json::json!({ "query": query, "results": hits })
        )?;
        return Ok(());
    }
    if hits.is_empty() {
        eprintln!("rien trouvé pour « {query} »");
        return Ok(());
    }

    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    for (rank, hit) in hits.iter().enumerate() {
        print_hit(&mut out, rank + 1, hit, color)?;
    }
    Ok(())
}

fn print_hit(
    out: &mut impl Write,
    rank: usize,
    hit: &SearchHit,
    color: bool,
) -> std::io::Result<()> {
    let path = shorten_home(&hit.path);
    if color {
        writeln!(
            out,
            "\x1b[1m{rank:2}. {path}\x1b[0m  \x1b[2m{:.3}\x1b[0m",
            hit.score
        )?;
    } else {
        writeln!(out, "{rank:2}. {path}  {:.3}", hit.score)?;
    }
    let snippet = highlight(&hit.snippet, &hit.highlights, color);
    let one_line = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
    writeln!(out, "    {one_line}")
}

/// Insère les codes ANSI autour des plages surlignées (offsets en octets).
fn highlight(text: &str, ranges: &[std::ops::Range<usize>], color: bool) -> String {
    if !color || ranges.is_empty() {
        return text.to_string();
    }
    let mut sorted: Vec<_> = ranges.to_vec();
    sorted.sort_by_key(|r| r.start);
    let mut out = String::with_capacity(text.len() + ranges.len() * 16);
    let mut cursor = 0;
    for r in sorted {
        if r.start < cursor || r.end > text.len() {
            continue;
        }
        out.push_str(&text[cursor..r.start]);
        out.push_str("\x1b[38;5;208m");
        out.push_str(&text[r.start..r.end]);
        out.push_str("\x1b[0m");
        cursor = r.end;
    }
    out.push_str(&text[cursor..]);
    out
}

fn shorten_home(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_string(),
    }
}
