use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use mikke_core::SearchHit;

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

fn cmd_index(dir: &Path, index_dir: &Path) -> Result<()> {
    let dir = dir
        .canonicalize()
        .with_context(|| format!("dossier introuvable : {}", dir.display()))?;
    let start = Instant::now();
    let stats = mikke_core::build_index(&dir, index_dir)
        .with_context(|| format!("indexation de {} impossible", dir.display()))?;
    println!(
        "{} fichiers indexés ({} chunks), {} ignorés, {} illisibles — {:.1}s",
        stats.files_indexed,
        stats.chunks,
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
    let hits = mikke_core::search(index_dir, query, top).context("recherche impossible")?;

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
            "\x1b[1m{rank:2}. {path}\x1b[0m  \x1b[2m{:.2}\x1b[0m",
            hit.score
        )?;
    } else {
        writeln!(out, "{rank:2}. {path}  {:.2}", hit.score)?;
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
