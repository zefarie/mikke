# 0001 — Extraction PDF : pdf-extract plutôt que pdfium-render

Date : 2026-07-16 · Statut : acceptée

## Contexte

Il faut extraire le texte des PDF pour l'indexation. Deux candidates sérieuses :

- `pdf-extract` : pur Rust, aucune dépendance native.
- `pdfium-render` : bindings vers PDFium (le moteur PDF de Chrome), nécessite
  `libpdfium.so` (~7 Mo), soit embarqué en statique (build pénible, multiplateforme
  douloureux), soit téléchargé au premier lancement.

Benchmark reproductible (`spike/spike-pdf`) sur 50 vrais PDF hétérogènes : scans
de 1985/1994 sans couche texte, posters image seule, formulaires Cerfa/IRS à champs,
chinois/polonais/japonais, sourcebooks de 85 Mo, thèses LaTeX, PDF malformés
(collisions SHA-1 « shattered »), datasheets de 650 pages.

## Résultats

|  | pdf-extract 0.12 | pdfium-render 0.9 |
|---|---:|---:|
| texte extrait | 40/50 | 40/50 |
| vide (scan/image, comptés « ignorés ») | 9 | 10 |
| échec (panic) | 1 | 0 |
| caractères totaux | 3 798 100 | 3 866 499 |
| artefacts `(cid:)` | 0 | 0 |
| temps total | 3,0 s | 3,6 s |

Points notables :

- Couverture identique à 96 % : sur ce corpus, les deux moteurs lisent et
  échouent quasiment sur les mêmes fichiers, avec des volumes de texte proches.
- `pdfium` est plus robuste sur les cas limites : il tire 52 162 caractères d'un
  scan OCRisé de 1994 qui fait paniquer `pdf-extract`, et lit mieux certains PDF
  chinois (40 073 vs 29 946 caractères sur le même fichier).
- `pdf-extract` panique au lieu de retourner une erreur sur les PDF très tordus
  (1/50) : il FAUT l'isoler derrière `catch_unwind`.
- Piège pdfium découvert pendant le spike : `FPDF_InitLibrary` ne peut être
  appelé qu'une fois par processus — toute réinitialisation échoue. Le spike
  isole chaque fichier dans un sous-processus.
- Vitesse : équivalente, largement suffisante (50 fichiers dont 248 Mo en ~3 s).

Détail par fichier en annexe ci-dessous.

## Décision

**`pdf-extract`**, encapsulé dans `catch_unwind`, un panic = fichier compté
« ignoré » avec warning, jamais un crash.

La contrainte produit non négociable est « un seul binaire statique, zéro
dépendance runtime ». `pdf-extract` la satisfait gratuitement. `pdfium` ne
l'améliore que sur ~2 % du corpus, au prix d'une lib native de 7 Mo à
distribuer par plateforme et d'un cycle de vie d'init pénible.

## Conséquences

- L'extraction vit derrière le module `extract` de `mikke-core` : si le taux
  d'échec de `pdf-extract` devient un problème réel, un backend pdfium optionnel
  (téléchargé au premier run, comme le modèle d'embeddings) reste branchable
  sans toucher au reste du pipeline.
- Les PDF sans couche texte (scans, posters) sont détectés (< 20 caractères
  non blancs) et comptés « ignorés », pas « erreurs ». L'OCR reste hors scope v1.
- Le spike est rejouable : `cargo run --release -p spike-pdf -- <dossier> [timeout]`
  avec `libpdfium.so` dans `~/.cache/mikke-spike/` (ou `$MIKKE_PDFIUM`).

## Annexe — détail par fichier

Corpus local + PDF publics ; noms personnels anonymisés.

| fichier | Ko | pdf-extract | chars | ms | pdfium | chars | ms |
|---|---:|---|---:|---:|---|---:|---:|
| CP2020_Sourcebook_EN.pdf | 84659 | ok | 683881 | 867 | ok | 683862 | 1086 |
| CP2020_Sourcebook_PL.pdf | 28309 | ok | 774320 | 343 | ok | 787179 | 473 |
| cv-personnel.pdf | 259 | ok | 1985 | 5 | ok | 1985 | 7 |
| Cyberpunk2077_Short-Story.pdf | 4979 | ok | 83170 | 19 | ok | 83170 | 37 |
| Cyberpunk2077_Short-Story_ZHCN.pdf | 1683 | ok | 36663 | 73 | ok | 36663 | 116 |
| End-user license agreement EULA.pdf | 33 | ok | 2103 | 1 | ok | 2103 | 4 |
| F2500008.pdf | 30 | ok | 1361 | 2 | ok | 1361 | 2 |
| InfCont85.PDF | 1943 | vide | 0 | 2 | vide | 0 | 1 |
| LLVM-Passes-all.pdf | 158 | vide | 0 | 4 | vide | 0 | 3 |
| Posters_Arasaka_24inx36in.pdf | 14129 | vide | 0 | 2 | vide | 0 | 5 |
| Posters_Electronic_Murderer_61x91cm.pdf | 15220 | vide | 0 | 2 | vide | 0 | 5 |
| RGSeaOfThievesEBook.pdf | 28947 | ok | 103400 | 78 | ok | 103728 | 91 |
| RTG-CPR-EasyModev1.1.pdf | 6427 | ok | 100831 | 95 | ok | 95662 | 115 |
| RTG-CPR-EasyModev1.1_CN.pdf | 21818 | ok | 29946 | 95 | ok | 40073 | 136 |
| Reference.pdf | 495 | ok | 92979 | 31 | ok | 93428 | 42 |
| SB_Aggro_Font_license.pdf | 112 | ok | 415 | 2 | ok | 415 | 3 |
| Winklerrr94-5.pdf | 250 | PANIC | 0 | 3 | ok | 52162 | 41 |
| arxiv-attention.pdf | 2163 | ok | 33486 | 67 | ok | 33486 | 71 |
| arxiv-rag.pdf | 864 | ok | 58970 | 24 | ok | 59294 | 40 |
| berkshire-2023.pdf | 119 | ok | 38696 | 4 | ok | 38696 | 15 |
| bitcoin-whitepaper.pdf | 179 | ok | 17670 | 8 | ok | 17670 | 12 |
| bzip2-format.pdf | 1068 | ok | 46725 | 36 | ok | 46725 | 62 |
| cerfa-13750.pdf | 526 | ok | 3473 | 59 | ok | 3473 | 11 |
| colm2025_conference.pdf | 119 | ok | 8811 | 6 | ok | 8811 | 6 |
| cours_chapitre_10.pdf | 370 | ok | 5160 | 7 | ok | 5160 | 21 |
| cours_chapitre_9.pdf | 498 | ok | 6043 | 9 | ok | 6043 | 11 |
| example_paper.pdf | 188 | ok | 20132 | 14 | ok | 20362 | 24 |
| grosser-diploma-thesis.pdf | 865 | ok | 146429 | 47 | ok | 146689 | 91 |
| grosser-impact-2011-slides.pdf | 717 | ok | 6986 | 19 | ok | 7045 | 11 |
| grosser-impact-2011.pdf | 315 | ok | 26666 | 16 | ok | 26778 | 17 |
| help.pdf | 1 | vide | 0 | 0 | vide | 0 | 0 |
| hermes-kanban-v1-spec.pdf | 208 | ok | 46134 | 18 | ok | 45962 | 32 |
| iclr2026_conference.pdf | 195 | ok | 10711 | 17 | ok | 10711 | 9 |
| irs-f1040-form.pdf | 215 | ok | 7927 | 9 | ok | 7927 | 12 |
| irs-fw9-form.pdf | 137 | ok | 31411 | 5 | ok | 31411 | 14 |
| japan-soumu.pdf | 458 | ok | 8675 | 14 | ok | 8675 | 33 |
| mapreduce-osdi04.pdf | 186 | ok | 46351 | 8 | ok | 46776 | 24 |
| mozila-content-spoof.pdf | 992 | ok | 69227 | 31 | ok | 69395 | 40 |
| pico-datasheet.pdf | 17750 | ok | 34954 | 240 | ok | 35028 | 85 |
| raghesh-a-masters-thesis.pdf | 497 | ok | 60800 | 38 | ok | 61276 | 51 |
| rp2040-datasheet.pdf | 5176 | ok | 1085378 | 498 | ok | 1085377 | 620 |
| sample-simple.pdf | 18 | ok | 2432 | 0 | ok | 2433 | 2 |
| shattered-1.pdf | 412 | vide | 0 | 0 | vide | 0 | 0 |
| shattered-2.pdf | 412 | vide | 0 | 0 | vide | 0 | 0 |
| subplots.pdf | 1 | vide | 0 | 0 | vide | 0 | 0 |
| test.pdf | 7739 | ok | 1545 | 141 | ok | 1545 | 8 |
| unicode-ch01.pdf | 167 | ok | 11773 | 5 | ok | 7510 | 8 |
| w3c-dummy.pdf | 12 | vide | 12 | 0 | vide | 12 | 0 |
| wikimedia-example.pdf | 17 | ok | 44 | 0 | vide | 13 | 1 |
| xflate-format.pdf | 995 | ok | 50425 | 38 | ok | 50425 | 63 |
