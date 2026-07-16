//! Spike : compare pdf-extract et pdfium-render sur un corpus de vrais PDF.
//!
//! Usage : spike-pdf <dossier-pdf> [timeout-secondes]
//! La lib pdfium est cherchée dans $MIKKE_PDFIUM (défaut : ~/.cache/mikke-spike/libpdfium.so).

use std::panic;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use walkdir::WalkDir;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Outcome {
    Ok,
    Empty,
    Error,
    Panic,
    Timeout,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Empty => "vide",
            Outcome::Error => "erreur",
            Outcome::Panic => "PANIC",
            Outcome::Timeout => "TIMEOUT",
        }
    }
}

struct EngineResult {
    outcome: Outcome,
    chars: usize,
    cid_garbage: usize,
    ms: u128,
}

fn measure<F>(f: F, timeout: Duration) -> EngineResult
where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let start = Instant::now();
    std::thread::spawn(move || {
        let r = panic::catch_unwind(panic::AssertUnwindSafe(f));
        let _ = tx.send(r);
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(Ok(text))) => {
            let chars = text.chars().filter(|c| !c.is_whitespace()).count();
            let cid_garbage = text.matches("(cid:").count();
            EngineResult {
                outcome: if chars < 20 {
                    Outcome::Empty
                } else {
                    Outcome::Ok
                },
                chars,
                cid_garbage,
                ms: start.elapsed().as_millis(),
            }
        }
        Ok(Ok(Err(_))) => EngineResult {
            outcome: Outcome::Error,
            chars: 0,
            cid_garbage: 0,
            ms: start.elapsed().as_millis(),
        },
        Ok(Err(_)) => EngineResult {
            outcome: Outcome::Panic,
            chars: 0,
            cid_garbage: 0,
            ms: start.elapsed().as_millis(),
        },
        Err(_) => EngineResult {
            outcome: Outcome::Timeout,
            chars: 0,
            cid_garbage: 0,
            ms: timeout.as_millis(),
        },
    }
}

fn pdfium_lib_path() -> String {
    std::env::var("MIKKE_PDFIUM").unwrap_or_else(|_| {
        let home = std::env::var("HOME").expect("HOME non défini");
        format!("{home}/.cache/mikke-spike/libpdfium.so")
    })
}

fn extract_pdfium(path: PathBuf) -> Result<String, String> {
    use pdfium_render::prelude::*;
    let bindings = Pdfium::bind_to_library(pdfium_lib_path()).map_err(|e| e.to_string())?;
    let pdfium = Pdfium::new(bindings);
    let doc = pdfium
        .load_pdf_from_file(&path, None)
        .map_err(|e| e.to_string())?;
    let mut out = String::new();
    for page in doc.pages().iter() {
        match page.text() {
            Ok(t) => {
                out.push_str(&t.all());
                out.push('\n');
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(out)
}

/// pdfium ne peut être initialisé qu'UNE fois par processus (FPDF_InitLibrary) :
/// chaque fichier passe donc par un sous-processus jetable, tué au timeout.
/// Le chrono est mesuré dans l'enfant (extraction seule, sans le spawn).
fn measure_pdfium_subprocess(path: &PathBuf, timeout: Duration) -> EngineResult {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe().expect("current_exe");
    let child = Command::new(exe)
        .arg("--pdfium-one")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(_) => {
            return EngineResult {
                outcome: Outcome::Error,
                chars: 0,
                cid_garbage: 0,
                ms: 0,
            };
        }
    };

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut s = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut s);
                }
                if status.success() {
                    let mut it = s.split_whitespace();
                    let chars: usize = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                    let ms: u128 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                    return EngineResult {
                        outcome: if chars < 20 {
                            Outcome::Empty
                        } else {
                            Outcome::Ok
                        },
                        chars,
                        cid_garbage: 0,
                        ms,
                    };
                }
                let outcome = if status.code().is_none() {
                    Outcome::Panic
                } else {
                    Outcome::Error
                };
                return EngineResult {
                    outcome,
                    chars: 0,
                    cid_garbage: 0,
                    ms: 0,
                };
            }
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return EngineResult {
                        outcome: Outcome::Timeout,
                        chars: 0,
                        cid_garbage: 0,
                        ms: timeout.as_millis(),
                    };
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => {
                return EngineResult {
                    outcome: Outcome::Error,
                    chars: 0,
                    cid_garbage: 0,
                    ms: 0,
                };
            }
        }
    }
}

/// Mode enfant : extrait un seul PDF via pdfium et imprime "chars ms".
fn pdfium_child(path: PathBuf) -> ! {
    let start = Instant::now();
    match extract_pdfium(path) {
        Ok(text) => {
            let chars = text.chars().filter(|c| !c.is_whitespace()).count();
            println!("{} {}", chars, start.elapsed().as_millis());
            std::process::exit(0);
        }
        Err(_) => std::process::exit(3),
    }
}

fn extract_pdf_extract(path: PathBuf) -> Result<String, String> {
    pdf_extract::extract_text(&path).map_err(|e| e.to_string())
}

fn main() {
    // pdf-extract panique sur les PDF tordus : on étouffe les backtraces.
    panic::set_hook(Box::new(|_| {}));

    let mut args = std::env::args().skip(1);
    let first = args
        .next()
        .expect("usage : spike-pdf <dossier-pdf> [timeout-s]");
    if first == "--pdfium-one" {
        pdfium_child(PathBuf::from(args.next().expect("--pdfium-one <fichier>")));
    }
    let dir = first;
    let timeout = Duration::from_secs(args.next().and_then(|s| s.parse().ok()).unwrap_or(60));

    let mut files: Vec<PathBuf> = WalkDir::new(&dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .map(|x| x.eq_ignore_ascii_case("pdf"))
                    .unwrap_or(false)
        })
        .map(|e| e.into_path())
        .collect();
    files.sort();

    println!("| fichier | Ko | pdf-extract | chars | (cid:) | ms | pdfium | chars | ms |");
    println!("|---|---:|---|---:|---:|---:|---|---:|---:|");

    let mut totals = Totals::default();
    for path in &files {
        let size_kb = std::fs::metadata(path).map(|m| m.len() / 1024).unwrap_or(0);
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        let p1 = path.clone();
        let pe = measure(move || extract_pdf_extract(p1), timeout);
        let pi = measure_pdfium_subprocess(path, timeout);

        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            name,
            size_kb,
            pe.outcome.label(),
            pe.chars,
            pe.cid_garbage,
            pe.ms,
            pi.outcome.label(),
            pi.chars,
            pi.ms
        );
        totals.add(&pe, &pi);
    }

    totals.print(files.len());
}

#[derive(Default)]
struct Totals {
    pe_ok: usize,
    pe_empty: usize,
    pe_fail: usize,
    pe_ms: u128,
    pe_chars: usize,
    pe_cid: usize,
    pi_ok: usize,
    pi_empty: usize,
    pi_fail: usize,
    pi_ms: u128,
    pi_chars: usize,
}

impl Totals {
    fn add(&mut self, pe: &EngineResult, pi: &EngineResult) {
        match pe.outcome {
            Outcome::Ok => self.pe_ok += 1,
            Outcome::Empty => self.pe_empty += 1,
            _ => self.pe_fail += 1,
        }
        self.pe_ms += pe.ms;
        self.pe_chars += pe.chars;
        self.pe_cid += pe.cid_garbage;
        match pi.outcome {
            Outcome::Ok => self.pi_ok += 1,
            Outcome::Empty => self.pi_empty += 1,
            _ => self.pi_fail += 1,
        }
        self.pi_ms += pi.ms;
        self.pi_chars += pi.chars;
    }

    fn print(&self, n: usize) {
        println!();
        println!("## Bilan sur {n} fichiers");
        println!();
        println!("|  | pdf-extract | pdfium-render |");
        println!("|---|---:|---:|");
        println!("| texte extrait | {} | {} |", self.pe_ok, self.pi_ok);
        println!(
            "| vide (scan/image) | {} | {} |",
            self.pe_empty, self.pi_empty
        );
        println!(
            "| échec (erreur/panic/timeout) | {} | {} |",
            self.pe_fail, self.pi_fail
        );
        println!(
            "| caractères totaux | {} | {} |",
            self.pe_chars, self.pi_chars
        );
        println!("| artefacts (cid:) | {} | 0 |", self.pe_cid);
        println!("| temps total (ms) | {} | {} |", self.pe_ms, self.pi_ms);
    }
}
