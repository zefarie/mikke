mod tui;

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
    /// Indexe un dossier (incrémental : seuls les fichiers modifiés sont relus)
    Index {
        dir: PathBuf,
        /// Reconstruit tout l'index de zéro
        #[arg(long)]
        full: bool,
    },
    /// Recherche interactive plein écran, style fzf
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
        Some(Command::Index { dir, full }) => cmd_index(&dir, &index_dir, full),
        Some(Command::Tui) => {
            if !index_dir.exists() {
                bail!("aucun index — lance d'abord : mikke index ~/Documents");
            }
            tui::run(&index_dir, load_embedder(false))
        }
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

fn cmd_index(dir: &Path, index_dir: &Path, full: bool) -> Result<()> {
    let dir = dir
        .canonicalize()
        .with_context(|| format!("dossier introuvable : {}", dir.display()))?;
    let embedder = load_embedder(true);
    let start = Instant::now();
    let stats = mikke_core::build_index(&dir, index_dir, embedder.as_ref(), full)
        .with_context(|| format!("indexation de {} impossible", dir.display()))?;
    let mut parts = vec![format!(
        "{} fichiers indexés ({} chunks{})",
        stats.files_indexed,
        stats.chunks,
        if stats.vectors {
            ", BM25 + vecteurs"
        } else {
            ", BM25 seul"
        }
    )];
    if stats.files_unchanged > 0 {
        parts.push(format!("{} inchangés", stats.files_unchanged));
    }
    if stats.files_deleted > 0 {
        parts.push(format!("{} retirés", stats.files_deleted));
    }
    parts.push(format!("{} ignorés", stats.files_skipped));
    if stats.files_failed > 0 {
        parts.push(format!("{} illisibles", stats.files_failed));
    }
    println!(
        "{} — {:.1}s",
        parts.join(", "),
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
    let width = crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(100)
        .max(40);
    for (rank, hit) in hits.iter().enumerate() {
        print_hit(&mut out, rank + 1, hit, color, width)?;
    }
    Ok(())
}

/// Deux lignes par résultat : nom de fichier en gras + dossier estompé,
/// puis l'extrait sur une seule ligne, tronqué à la largeur du terminal.
fn print_hit(
    out: &mut impl Write,
    rank: usize,
    hit: &SearchHit,
    color: bool,
    width: usize,
) -> std::io::Result<()> {
    let path = Path::new(&hit.path);
    let file = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| hit.path.clone());
    let dir = path
        .parent()
        .map(|p| shorten_home(&p.to_string_lossy()))
        .unwrap_or_default();
    if rank > 1 {
        writeln!(out)?;
    }
    if color {
        writeln!(
            out,
            "\x1b[2m{rank:2}\x1b[0m \x1b[1m{file}\x1b[0m  \x1b[2m{dir}\x1b[0m"
        )?;
    } else {
        writeln!(out, "{rank:2} {file}  {dir}")?;
    }
    write!(out, "   ")?;
    render_snippet(
        out,
        &hit.snippet,
        &hit.highlights,
        color,
        width.saturating_sub(5),
    )?;
    writeln!(out)
}

/// Rend l'extrait : blancs fusionnés, termes de la requête en vermillon, le
/// reste estompé, coupé proprement à `budget` caractères.
fn render_snippet(
    out: &mut impl Write,
    text: &str,
    ranges: &[std::ops::Range<usize>],
    color: bool,
    budget: usize,
) -> std::io::Result<()> {
    let mut sorted: Vec<_> = ranges.to_vec();
    sorted.sort_by_key(|r| r.start);
    let mut segments: Vec<(&str, bool)> = Vec::new();
    let mut cursor = 0;
    for r in sorted {
        if r.start < cursor || r.end > text.len() {
            continue;
        }
        if r.start > cursor {
            segments.push((&text[cursor..r.start], false));
        }
        segments.push((&text[r.start..r.end], true));
        cursor = r.end;
    }
    segments.push((&text[cursor..], false));

    let mut left = budget;
    let mut last_space = true;
    for (segment, highlighted) in segments {
        if color {
            out.write_all(if highlighted {
                b"\x1b[38;5;208m"
            } else {
                b"\x1b[2m"
            })?;
        }
        for ch in segment.chars() {
            let ch = if ch.is_whitespace() { ' ' } else { ch };
            if ch == ' ' && last_space {
                continue;
            }
            if left == 0 {
                if color {
                    out.write_all(b"\x1b[0m")?;
                }
                return write!(out, "…");
            }
            let mut buf = [0u8; 4];
            out.write_all(ch.encode_utf8(&mut buf).as_bytes())?;
            last_space = ch == ' ';
            left -= 1;
        }
        if color {
            out.write_all(b"\x1b[0m")?;
        }
    }
    Ok(())
}

fn shorten_home(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_string(),
    }
}
