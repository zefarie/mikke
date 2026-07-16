# Benchmarks

Mesures du 2026-07-16, binaire release (LTO). Machine : Ryzen 7 7800X3D,
32 Go DDR5, NVMe. Les cibles étant définies « sur un laptop 4 cœurs », les
runs marqués *4c* tournent sous `taskset -c 0-3` avec `RAYON_NUM_THREADS=4`.

## Cibles de la spec

| Cible | Mesuré | |
|---|---|---|
| indexer 10 000 fichiers mixtes en < 60 s (4 cœurs) | **10,1 s** (49 628 chunks, embeddings inclus) *4c* | ✓ |
| requête < 100 ms au p95 sur un index de 50 000 chunks | **21 ms** (process complet, 40 requêtes) *4c* | ✓ |
| RAM en recherche < 300 Mo | **80 Mo** de pic RSS | ✓ |
| binaire statique unique < 40 Mo | **17 Mo** (modèle téléchargé au premier run) | ✓ |

À côté des cibles :

- run incrémental sans modification : **0,1 s** sur les mêmes 10 000 fichiers ;
- recherche hybride in-process (index déjà ouvert) : **7 ms** sur 49 628
  chunks, y compris l'embedding de la requête ;
- pic RSS pendant l'indexation : 444 Mo (la cible mémoire de la spec ne
  concerne que la recherche) ;
- sur disque : index tantivy + 56 Mo de vecteurs usearch + 507 Mo de modèle
  dans le cache (une fois pour toutes).

## Corpus de mesure

10 000 fichiers générés (seed fixe) : 9 400 `.md`/`.txt` de 200 à 3 000 mots
tirés d'un lexique FR/EN de documents personnels, plus 600 copies des
fixtures `.pdf`/`.docx`/`.html` du dépôt — 149 Mo au total, 49 628 chunks.

## Micro-bancs (criterion, machine complète)

`cargo bench -p mikke-core` — index de 1 000 fichiers pour les recherches :

| banc | temps |
|---|---:|
| indexation BM25, 200 fichiers × 500 mots | 12,8 ms |
| recherche BM25 seule | 356 µs |
| recherche hybride (BM25 + vecteurs + RRF) | 389 µs |
| embedding d'une requête | 3,5 µs |

## Qualité

20 requêtes de référence (`eval/queries.toml`, `cargo run --release -p
mikke-core --example eval`) : hybride **19/20** en hit@10 (17/20 au rang 1)
contre 16/20 pour BM25 seul (les stopwords FR/EN coûtent deux requêtes au mode dégradé sans modèle, mais suppriment les faux positifs du mode hybride). L'échec restant est volontairement difficile :
requête française vers un document anglais.

## D'où viennent les millisecondes

Deux goulots ont été éliminés en cours de route, mesures à l'appui :

1. Le tokenizer HuggingFace coûtait ~700 ms et ~550 Mo à charger (vocab
   bge-m3 de 500 353 entrées) à chaque invocation. Remplacé par un Unigram
   maison sur FST compact, 100 % fidèle (ADR 0002) : requête 1 001 → 11 ms
   sur un petit index.
2. hnsw_rs désérialise son graphe nœud par nœud au chargement : ~450 ms sur
   49 628 chunks, soit un p95 mesuré à 457 ms. Remplacé par usearch, rechargé
   en `view()` mmap : p95 21 ms, et l'indexation complète est passée de
   20,5 s à 10,1 s (insertions parallèles plus efficaces).
