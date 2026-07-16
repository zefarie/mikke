# Changelog

## v1.0.1 — 2026-07-16

- Build portability: dropped usearch's optional SIMD kernels (numkong),
  whose C sources don't compile under cross toolchains (zig cc, old gcc)
  and break distro packaging flags. The portable distance code keeps every
  performance target with wide margin. v1.0.0 never shipped binaries.

## v1.0.0 — 2026-07-16

First release.

### Search
- Hybrid retrieval: BM25 (tantivy, accent-insensitive, French and English
  stopwords) fused with multilingual embeddings (potion-multilingual-128M,
  101 languages) through reciprocal rank fusion.
- Custom Unigram tokenizer on a compact FST cache: the whole query path runs
  in ~10 ms where the reference tokenizer alone needs ~700 ms per invocation.
- Vector index on usearch, reloaded with a zero-parse mmap view.
- Noise control: per-list score floors, cross-signal fusion cutoff, and an
  honest "weak match" notice when nothing really fits.
- Filenames weigh in the score; code projects and extracted archives are
  recognised and skipped — mikke indexes documents, not repositories.

### Formats
- PDF (scanned pages without a text layer are detected and skipped, never
  errors), DOCX, Markdown, plain text, HTML.
- A corrupt file can never abort indexing.

### Indexing
- Incremental by design: mtime + size, then blake3. Unchanged files are
  never re-read, embeddings are cached in SQLite.
- Multiple roots and exclusions via `~/.config/mikke/config.toml`; adding a
  root never erases the others.
- `mikke watch` keeps the index fresh (inotify, debounced), with a systemd
  user unit in contrib/.

### Interface
- `mikke "<query>"` with highlighted excerpts fitted to the terminal,
  `--json` for scripts, `mikke tui` for fzf-style live search.
- Shell completions: `mikke completions <shell>`.

### Trust
- 100% offline after the first run; the embedding model download is pinned
  by SHA-256.
- Measured against its own targets in BENCHMARKS.md: 10 000 mixed files
  indexed in 10.1 s on four cores, 21 ms p95 per query on a 50 000-chunk
  index, 80 MB peak RSS while searching, 17 MB binary.
