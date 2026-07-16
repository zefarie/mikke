# 0002 — Embeddings, tokenizer maison et fusion hybride

Date : 2026-07-16 · Statut : acceptée

## Modèle d'embeddings : potion-multilingual-128M, lu en mmap

Seul modèle statique multilingue sérieux du moment (model2vec, 101 langues,
dim 256, distillé de bge-m3). Un modèle statique = une matrice de lookup :
l'inférence est une moyenne de lignes, pure CPU, sans runtime ONNX.

Le fichier fait 489 Mo en f32. La crate officielle `model2vec-rs` le charge
entier en RAM — hors budget (< 300 Mo en recherche). Ici : mmap du
safetensors, lecture des seules lignes touchées. Sémantique répliquée de
model2vec : pas de tokens spéciaux, `<unk>` jetés, troncature à 512 tokens,
moyenne, normalisation L2. Le modèle est téléchargé au premier `mikke index`
(±500 Mo, une fois), jamais embarqué dans le binaire.

## Tokenizer : Unigram maison sur FST, pas la crate HuggingFace

Mesure sur le chemin de requête de la CLI :

|  | tokenizers (HF) | tok maison (FST) |
|---|---:|---:|
| chargement | 665 ms | 7 ms |
| RSS après chargement | ~550 Mo | ~14 Mo |
| requête complète (binaire réel) | 1 001 ms | **11 ms** |
| pic RSS du process | 613 Mo | **23 Mo** |

Une CLI sans daemon paie le chargement à CHAQUE requête : la crate HF était
donc intenable, uniquement à cause du vocabulaire bge-m3 (500 353 entrées).
Le `tokenizer.json` est converti une fois en cache compact (~14 Mo) :
FST token→id, scores f64, et le charsmap Precompiled de SentencePiece copié
tel quel (appliqué via `spm_precompiled`, la crate de référence extraite de
HF). La segmentation Viterbi (une descente de FST par position) réplique le
Unigram.

Piège découvert : le normalizer de ce modèle ne se limite pas au charsmap —
il enchaîne 32 règles Replace qui entourent d'espaces toute la ponctuation
ASCII, puis fusionne les blancs et strip. Répliqué en une passe dans
`tok::tokenize`.

Fidélité mesurée (`cargo run -p mikke-core --example tok_check`) sur les
24 documents + 20 requêtes d'éval : **100 % de séquences identiques** à la
référence HF, cosinus des embeddings 1.0000 (min comme moyenne).

## Index vectoriel : hnsw_rs

Pure Rust (usearch = C++ à linker). Pièges rencontrés, encapsulés dans
`vector.rs` :

- le dump exige exactement 16 couches (`NB_MAX_LAYER`), sinon erreur ;
- le rechargement emprunte le `HnswIo` (`'a: 'b`) → `Box::leak`, une fois
  par ouverture d'index ;
- le rechargement panique sur un dump corrompu → `catch_unwind`, on dégrade
  en BM25 seul au lieu de crasher.

## Fusion : Reciprocal Rank Fusion (k = 60)

Les scores BM25 et les distances cosinus ne sont pas commensurables ; RRF ne
compare que les rangs. Sans modèle (pas encore téléchargé, cache absent),
la recherche dégrade proprement en BM25 seul — mikke répond toujours.

## Qualité mesurée (eval/queries.toml, hit@k sur 20 requêtes)

|  | BM25 seul | hybride |
|---|---:|---:|
| hit@10 | 18/20 | **19/20** |
| hit@1 | — | 17/20 |

Les requêtes sémantiques justifient l'hybride : « mon salaire au
restaurant » passe du rang 10 au rang 1, « posologie antibiotique »
d'introuvable au rang 5, « am I covered if I crash my car » du rang 2 au
rang 1. Échec restant (assumé, cas volontairement difficile) : requête
française → document anglais (« facture du plombier fuite cuisine »).
Latence de recherche pure : p95 < 1 ms sur ce corpus ; requête CLI complète
11 ms, pic RSS 23 Mo.
