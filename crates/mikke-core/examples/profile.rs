//! Profil grossier du chemin de requête : temps et RSS à chaque étape.
//!
//! Usage : MIKKE_DATA=<dossier> cargo run --release -p mikke-core --example profile

use std::time::Instant;

fn rss_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for l in s.lines() {
        if let Some(v) = l.strip_prefix("VmRSS:") {
            return v
                .trim()
                .trim_end_matches(" kB")
                .trim()
                .parse::<f64>()
                .unwrap_or(0.0)
                / 1024.0;
        }
    }
    0.0
}

fn main() {
    let home = std::env::var("HOME").unwrap();
    let model_dir = std::path::PathBuf::from(&home).join(".cache/mikke/potion-multilingual-128M");
    let index_dir =
        std::path::PathBuf::from(std::env::var("MIKKE_DATA").expect("MIKKE_DATA requis"))
            .join("index");

    println!("départ : RSS {:.0} Mo", rss_mb());

    let t = Instant::now();
    let emb = mikke_core::Embedder::load(&model_dir).unwrap();
    println!(
        "Embedder::load complet : {:?} — RSS {:.0} Mo",
        t.elapsed(),
        rss_mb()
    );

    let t = Instant::now();
    let v = emb.embed("comment je note mes idées de projets").unwrap();
    println!(
        "embed requête (dim {}) : {:?} — RSS {:.0} Mo",
        v.len(),
        t.elapsed(),
        rss_mb()
    );

    let t = Instant::now();
    let hits = mikke_core::search(
        &index_dir,
        "comment je note mes idées de projets",
        10,
        Some(&emb),
    )
    .unwrap();
    println!(
        "search hybride ({} hits) : {:?} — RSS {:.0} Mo",
        hits.len(),
        t.elapsed(),
        rss_mb()
    );
}
