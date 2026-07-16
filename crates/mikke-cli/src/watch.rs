//! `mikke watch` : l'index reste frais tout seul. Surveille les racines de
//! la config (inotify via notify), fusionne les rafales d'événements, puis
//! relance une indexation incrémentale — 0,1 s quand rien n'a changé.
//!
//! Tourne en avant-plan, pensé pour un service systemd user
//! (voir contrib/mikke-watch.service).

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use mikke_core::Embedder;
use notify::{RecursiveMode, Watcher};

/// Fenêtre de fusion : une rafale (sauvegarde, rsync…) = une réindexation.
const DEBOUNCE: Duration = Duration::from_secs(2);

pub fn run(
    roots: &[PathBuf],
    excludes: &[PathBuf],
    index_dir: &Path,
    embedder: Option<Embedder>,
) -> Result<()> {
    if roots.is_empty() {
        bail!("no roots configured — run: mikke index ~/Documents");
    }

    reindex(roots, excludes, index_dir, embedder.as_ref(), true)?;

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .context("failed to create the watcher")?;
    for root in roots {
        watcher
            .watch(root, RecursiveMode::Recursive)
            .with_context(|| format!("cannot watch {}", root.display()))?;
    }
    eprintln!("mikke is watching {} root(s) — Ctrl-C to stop", roots.len());

    loop {
        // on bloque jusqu'au premier événement intéressant…
        let event = rx.recv().context("watcher stopped")?;
        if !interesting(&event) {
            continue;
        }
        // …puis on laisse passer la rafale avant de réindexer
        loop {
            match rx.recv_timeout(DEBOUNCE) {
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => bail!("watcher stopped"),
            }
        }
        reindex(roots, excludes, index_dir, embedder.as_ref(), false)?;
    }
}

/// Les fichiers cachés (verrous d'éditeurs, .part…) ne déclenchent rien.
fn interesting(event: &Result<notify::Event, notify::Error>) -> bool {
    match event {
        Err(_) => true, // erreur du watcher : mieux vaut re-regarder
        Ok(ev) => ev.paths.iter().any(|p| {
            p.file_name()
                .map(|n| !n.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
        }),
    }
}

fn reindex(
    roots: &[PathBuf],
    excludes: &[PathBuf],
    index_dir: &Path,
    embedder: Option<&Embedder>,
    first: bool,
) -> Result<()> {
    let start = std::time::Instant::now();
    let stats = mikke_core::build_index(roots, excludes, index_dir, embedder, false)
        .context("reindexing failed")?;
    if first || stats.files_indexed > 0 || stats.files_deleted > 0 {
        eprintln!(
            "reindexed: {} files, {} removed, {} chunks total — {:.1}s",
            stats.files_indexed,
            stats.files_deleted,
            stats.chunks,
            start.elapsed().as_secs_f32()
        );
    }
    Ok(())
}
