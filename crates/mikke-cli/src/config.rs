//! Config utilisateur : les racines indexées et les chemins exclus.
//!
//! `mikke index <dir>` ajoute une racine ici ; `mikke index` sans argument
//! réindexe toutes les racines. La config est la source de vérité : un
//! fichier de l'index qui ne vit plus sous aucune racine en sort.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Dossiers indexés par `mikke index`.
    pub roots: Vec<String>,
    /// Chemins jamais indexés (préfixes, `~` accepté).
    pub exclude: Vec<String>,
}

pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("MIKKE_CONFIG") {
        return PathBuf::from(p);
    }
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").expect("HOME non défini")).join(".config")
        })
        .join("mikke")
        .join("config.toml")
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME non défini"))
}

/// `~/Documents` → `/home/x/Documents`
fn expand(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(path),
    }
}

/// `/home/x/Documents` → `~/Documents` (plus lisible dans le fichier)
pub fn contract(path: &Path) -> String {
    match path.strip_prefix(home()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("lecture impossible : {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("config invalide : {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = String::from(
            "# mikke — dossiers indexés et exclusions\n\
             # `mikke index <dossier>` ajoute une racine ici.\n\n",
        );
        out.push_str(&toml::to_string_pretty(self)?);
        std::fs::write(&path, out)
            .with_context(|| format!("écriture impossible : {}", path.display()))?;
        Ok(())
    }

    /// Ajoute une racine (dédupliquée). Retourne vrai si la config a changé.
    pub fn add_root(&mut self, dir: &Path) -> bool {
        let entry = contract(dir);
        if self.roots.iter().any(|r| expand(r) == dir) {
            return false;
        }
        self.roots.push(entry);
        true
    }

    pub fn root_paths(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|r| expand(r)).collect()
    }

    pub fn exclude_paths(&self) -> Vec<PathBuf> {
        self.exclude.iter().map(|r| expand(r)).collect()
    }
}
