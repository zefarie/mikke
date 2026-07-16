<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg">
    <img src="assets/logo.svg" alt="a magnifying glass finding the one orange dot in a grid" width="150">
  </picture>
</p>

<h1 align="center">mikke</h1>

<p align="center">
  <b>Type what you remember. Get the file.</b><br>
  Local semantic search for the documents on your disk.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/made%20with-rust-B7410E?style=flat-square&logo=rust&logoColor=white" alt="made with rust">
  <img src="https://img.shields.io/badge/status-early%20days-EB5E28?style=flat-square" alt="status: early days">
  <img src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-6B655E?style=flat-square" alt="license: MIT or Apache-2.0">
</p>

<p align="center">
  <img src="assets/demo.gif" alt="mikke indexing a folder, then finding files from natural-language descriptions" width="820">
</p>

---

Somewhere in `~/Documents` there is a vet invoice from January. You know it
exists, you could describe it to a friend. But it was scanned as
`IMG_20260114_scan.pdf`, so no filename search will ever surface it again.

```console
$ mikke "vet invoice january"

1  ~/Documents/scans/IMG_20260114_scan.pdf
   …Clinique Vétérinaire des Carmes · consultation + vaccin,
   total dû : 85,00 €…
```

That's the whole tool. Describe the file, get the file.

## How it works

`mikke index ~/Documents` walks the tree, pulls text out of PDF, DOCX,
Markdown, HTML and plain text, and builds two indexes side by side: a
classic full-text index (BM25) and a vector index over multilingual
embeddings. Queries run against both and the scores get fused, which is why
"facture véto" can rank an invoice that never contains the word "facture".

Design rules, in order:

1. One static binary. No Docker, no Python, no Ollama, no daemon.
2. Offline. The embedding model lands in `~/.cache/mikke` on first run;
   after that nothing ever touches the network.
3. Fast enough to feel instant: answers in under 100 ms, even on an index
   of tens of thousands of chunks.
4. Multilingual from day one, starting with French and English.

## vs. the others

|              | mikke          | File Brain     | Open Semantic Search | mgrep       |
|--------------|:--------------:|:--------------:|:--------------------:|:-----------:|
| Install      | single binary  | Docker         | Solr + VM            | account     |
| Offline      | ✓              | ✓              | ✓                    | ✗ (cloud)   |
| Runtime deps | none           | Docker         | Java stack           | n/a         |
| Made for     | your documents | your documents | enterprise           | code & docs |

## Install

Not packaged yet — from source it's one line (Rust 1.85+):

```console
$ cargo install --locked --git https://github.com/zefarie/mikke mikke-cli
```

The first `mikke index` fetches the multilingual embedding model (~500 MB,
one time). Everything after that is offline.

## Configuration

`mikke index <dir>` adds the folder as a root in
`~/.config/mikke/config.toml` and refreshes everything; `mikke index` alone
re-syncs all configured roots. Indexing a new folder never erases the
previous ones.

```toml
roots = ["~/Documents", "~/Downloads"]
exclude = ["~/Documents/private"]
```

To keep the index fresh without thinking about it, `mikke watch` follows
the roots with inotify — or install it as a user service:

```console
$ cp contrib/mikke-watch.service ~/.config/systemd/user/
$ systemctl --user enable --now mikke-watch
```

## Status

The v1 feature set is in:

- [x] `mikke index <dir>` — incremental indexing (mtime + blake3), only
      changed files are re-read
- [x] `mikke "<query>"` — hybrid search (BM25 + embeddings, rank fusion),
      highlighted excerpts, `--json` for scripts
- [x] `mikke tui` — fzf-style live search, Enter opens the file
- [x] PDF, DOCX, Markdown, TXT, HTML

Measured numbers live in [BENCHMARKS.md](BENCHMARKS.md); search quality is
tracked by 20 reference queries in [eval/](eval/) (hit@10: 19/20 hybrid
vs 16/20 BM25 alone).

## The name

ミッケ! ("mikke!") is what Japanese kids shout when they spot the thing in a
seek-and-find book. Found it.

## License

MIT or Apache-2.0, whichever you prefer.
