<div align="center">

# mikke

**Type what you remember. Get the file.**

Local semantic search for the documents on your disk.

<!-- demo.gif -->

</div>

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

## Status

Early days, nothing to install yet. The v1 checklist:

- [ ] `mikke index <dir>` — incremental indexing (mtime + blake3)
- [ ] `mikke "<query>"` — hybrid search, highlighted excerpts, `--json` for scripts
- [ ] `mikke tui` — fzf-style live search, Enter opens the file
- [ ] PDF, DOCX, Markdown, TXT, HTML

## The name

ミッケ! ("mikke!") is what Japanese kids shout when they spot the thing in a
seek-and-find book. Found it.

## License

MIT or Apache-2.0, whichever you prefer.
