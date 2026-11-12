# NZB FF Tool Pro

**Préparer, publier et suivre ses releases Usenet, de la vidéo brute au dépôt sur les indexers — en un seul logiciel Windows.**

Renommage au format des indexers, rapports MediaInfo / NFO / BDInfo, post avec NgPost, dépôt
direct par API, vérification des doublons, recherche sur tous les indexers à la fois,
téléchargement et repost via SABnzbd / NZBGet, dossiers surveillés, notifications par e-mail.
Simple pour débuter (assistant, mode Simple), complet pour les utilisateurs avancés (mode Expert,
profils d'API, requêtes personnalisées).

Version actuelle : **3.26.0** — Rust, interface native (egui), un seul `.exe`, aucune télémétrie.

---

## Sommaire

- [Fonctionnalités](#fonctionnalités)
- [Confidentialité et sécurité](#confidentialité-et-sécurité)
- [Installation et compilation](#installation-et-compilation)
- [Premiers pas](#premiers-pas)
- [Organisation du code](#organisation-du-code)
- [Tests](#tests)
- [Documentation complète](#documentation-complète)
- [Soutenir le projet](#soutenir-le-projet)
- [Licence](#licence)

---

## Fonctionnalités

### Traitement d'une vidéo, d'une série, d'un lot ou d'un autre contenu

- **Glisser-déposer** ou sélection de fichiers : vidéo, dossier de film, plusieurs vidéos (lot),
  saison de série, archive ZIP / RAR / 7z, image disque ISO, jeu, logiciel, album, ebook.
- **Nom final au format des indexers**, calculé d'après MediaInfo : titre, année, langue
  (FRENCH, MULTi, VFF, VOSTFR…), résolution, source, codecs vidéo et audio, HDR, équipe.
  Modifiable à la main avant le traitement.
- **Séries** : détection des saisons et épisodes, traitement d'une saison entière, nommage
  cohérent de chaque épisode.
- **Disques complets Blu-ray et DVD** (dossiers BDMV / VIDEO_TS, ISO), avec rapport BDInfo.
- **Autres contenus** : jeux, logiciels, musique, ebooks… avec leur catégorie de contenu.
- **Trois niveaux d'action** : renommer seulement, renommer et poster, ou tout automatique
  (post puis dépôt sur les indexers choisis).
- **Choix des indexers juste sous le nom**, catégorie affichée avec son nom chez chaque indexer
  (« défaut (2040 · Movies > HD) »), modifiable par indexer.
- **Choix des groupes Usenet du post** : propositions d'après le contenu, les réglages et les
  groupes réellement indexés par chaque indexer visé, ou saisie libre.
- **Récapitulatif avant de lancer** : nom, destination, rapports, indexers, catégories, groupes,
  doublons, place sur le disque ; chaque problème est expliqué avec sa correction en un clic.

### Rapports : MediaInfo, NFO, BDInfo

- MediaInfo complet (XML recommandé, ou texte, JSON, HTML), NFO lisible, BDInfo pour les Blu-ray.
- **Page « Rapports » dédiée** : rapports seuls de fichiers, dossiers, disques ou NZB (téléchargés
  d'abord), en file, à côté de la source ou dans un dossier choisi — sans renommer, déplacer ni
  poster. Un NFO existant n'est jamais écrasé.
- **Jamais de dépôt sans MediaInfo** : contrôle avant chaque envoi.

### Post Usenet avec NgPost

- Pilotage de NgPost : serveurs, connexions, taille d'articles, obfuscation, fichiers de
  réparation (par2 de NgPost, par2cmdline-turbo ou ParPar).
- **Groupes Usenet** : par type de contenu, par catégorie d'indexer, ou ajout automatique d'un
  groupe indexé quand l'indexer l'exige (liste lue dans ses capacités).
- **Suivi en direct** : progression, débit, courbe de vitesse, temps restant, arrêt propre.
- File d'attente de posts, reprise après coupure.

### Indexers : dépôt, vérification, configuration

- **Dépôt direct par API** : Newznab (`t=nzbadd`), adresse d'envoi à part (ex. `/v1/upload`,
  la recherche restant sur l'API Newznab), commande personnalisée, ou dépôt à la main (la page
  d'upload s'ouvre avec le bon dossier).
- **Configuration automatique d'un indexer** à partir de l'adresse de l'API et de la clé :
  capacités, catégories, groupes, vérification de la clé, recherche de l'adresse d'envoi
  (Newznab, OpenAPI, adresses courantes). Délai réglable entre deux requêtes et nombre maximum de
  requêtes : pas de rafale, pas de risque de bannissement. Annulable.
- **Lecture de la documentation d'API** de l'indexer (texte collé ou fichier) : adresse d'envoi,
  champs du formulaire, clé, catégorie, page de la release — puis application en un clic.
- **« Réglage actuel »** de chaque indexer : recherche, envoi, catégories, groupes, identité
  réseau, en vert quand c'est prêt.
- **Dépôt refusé → diagnostic et correction proposée** (405, 404, 401/403, 413, 422, 429, codes
  Newznab 100–102, 200–203, 600–699…).
- **Aperçu exact de chaque requête avant l'envoi**, clé masquée, puis la réponse reçue.
- **Catégories** : import de la liste de l'indexer, remplissage automatique (films et séries
  SD / 720p / 1080p / UHD, versions FR, autres contenus), export en texte, CSV ou JSON.
- **Profils d'API** : préréglages, explorateur de requêtes, lecture des réponses XML / JSON /
  HTML, import et export de profils.
- **Ma requête** : écrire sa propre requête (adresse, « POST … » ou commande curl de la doc),
  la tester, la garder dans le profil.
- **Identité réseau (User-Agent)** : curl, NZB FF Tool Pro, navigateur courant tiré au sort,
  roulement à chaque requête, ou texte personnalisé — en général ou par indexer.

### Doublons : déjà sur l'indexer ?

- Avant chaque post, la release est cherchée sur chaque indexer visé, **sous son nom renommé et
  sous ses noms d'origine** (fichier, dossier, NZB reçu).
- **« Uploadée à la main sur X »** : la release n'y est plus jamais repostée.
- Politique au choix : ne pas poster pour cet indexer, avertir, ou ignorer.

### Recherche sur tous les indexers (à la Prowlarr, en plus simple)

- Une recherche part sur tous les indexers à la fois, chacun avec sa limite de requêtes.
- Résultats **regroupés par release** : où elle est (✔), où elle manque (✖), taille, âge, type.
- Filtres Films / Séries / Musique / Livres / Autre, et « manque quelque part ».
- **Télécharger**, **reposter seulement là où elle manque**, rapports seuls, enregistrer le NZB.

### Téléchargements et repost (SABnzbd, NZBGet)

- Envoi d'un NZB ou d'une adresse au client, suivi des files, pause / reprise.
- Modes : téléchargement seul, repost vers d'autres indexers (traitement complet), rapports seuls.
- **Dossiers de NZB surveillés**, entièrement réglables (par indexer d'origine, téléchargement
  seul, ailleurs…).

### Automatisation

- **Inbox surveillée** : chaque vidéo déposée suit le circuit choisi sans rien demander.
- Démarrage avec Windows, application dans la zone de notification.
- Intégration à l'Explorateur : clic droit, ouverture des `.nzb`.
- **Mode sans interface** (ligne de commande) pour les scripts.

### Notifications et suivi

- **E-mail de notification en couleur** : rouge en cas d'erreur, jaune pour de simples alertes,
  vert quand tout est bon (texte brut joint).
- Historique des traitements, statistiques, journaux détaillés, quarantaine des fichiers douteux.

### Confort

- **Interface Simple ou Expert**, assistant de configuration au premier lancement.
- Thème clair / sombre, couleur d'accent, recherche dans les réglages.
- Sauvegarde automatique et restauration des réglages.
- Très faible consommation au repos : fenêtre cachée ou réduite, aucune image n'est dessinée.

---

## Confidentialité et sécurité

- **Aucune télémétrie, aucune statistique d'usage envoyée.** L'application ne contacte que les
  services que tu configures : tes indexers, ton client de téléchargement, tes serveurs Usenet et
  ton serveur d'e-mail.
- **Clés API et mots de passe jamais écrits dans les journaux ni sur une ligne de commande** :
  curl les reçoit par un fichier de configuration temporaire (`-K`), ils sont masqués dans tous
  les affichages et aperçus.
- Les réglages restent sur ton PC, à côté de l'application (dossier `dist\`), jamais dans ce
  dépôt.

---

## Installation et compilation

Windows 10 / 11, sans IDE.

1. **Rust**, chaîne GNU (par exemple avec Chocolatey) :
   ```
   choco install rustup.install -y
   rustup default stable-x86_64-pc-windows-gnu
   ```
   et **MinGW-w64** (WinLibs, version UCRT, [winlibs.com](https://winlibs.com)) décompressé dans
   `C:\mingw64`.
2. Double-clic sur **`build.cmd`**.
3. L'application est créée dans **`dist\NZB FF Tool Pro.exe`**.

Outils externes utilisés si présents (chemins réglables dans l'application) : MediaInfo, NgPost,
par2cmdline-turbo ou ParPar, BDInfo, curl (inclus dans Windows 10 / 11).

---

## Premiers pas

1. Au premier lancement, l'**assistant** demande l'essentiel : dossiers, NgPost, indexers.
2. Pour chaque indexer : **Réglages → Indexers → l'indexer → « ✨ Configurer automatiquement »**
   avec l'adresse de l'API et la clé.
3. Glisse une vidéo sur la page **Traitement**, vérifie le nom et les indexers, lance.

---

## Organisation du code

| Fichier | Rôle |
|---|---|
| `src/main.rs` | démarrage, mode sans interface |
| `src/gui.rs` | interface (pages, réglages, fenêtres) |
| `src/prep.rs` | réglages, nommage, MediaInfo, NFO |
| `src/jobs.rs` | traitements, surveillance de l'inbox, rapports seuls |
| `src/ngpost.rs` | post avec NgPost, groupes Usenet, circuit vers les indexers |
| `src/upload.rs` | dépôt sur les indexers, catégories, appels curl sécurisés |
| `src/apidef.rs` | profils d'API, requêtes, lecture des réponses, User-Agent |
| `src/apidoc.rs` | lecture des docs d'API, configuration automatique, diagnostic |
| `src/search.rs` | vérification des doublons, « uploadée à la main » |
| `src/finder.rs` | recherche sur tous les indexers |
| `src/dl.rs` | SABnzbd / NZBGet, NZB surveillés, repost |
| `src/disc.rs` | Blu-ray, DVD, BDInfo |
| `src/series.rs`, `src/content.rs` | séries, autres contenus |
| `src/notify.rs` | e-mails de notification |
| `src/backup.rs` | sauvegarde et restauration des réglages |
| `src/tray.rs` | zone de notification, démarrage avec Windows |

---

## Tests

```
cargo test
```

Environ 170 tests automatiques (nommage, séries, MediaInfo, catégories, dépôts, configuration
automatique contre un faux indexer, recherche, groupes, e-mail, sauvegarde…). Deux tests de
lecture de documentation d'API utilisent des fichiers d'exemple non publiés : sans eux, ils sont
simplement sautés.

---

## Documentation complète

Le mode d'emploi détaillé, réglage par réglage, et l'historique des versions sont dans
**[LISEZMOI.md](LISEZMOI.md)**.

---

## Soutenir le projet

NZB FF Tool Pro est gratuit et open source. S'il te rend service, un don aide à le maintenir à
jour (les API des indexers changent souvent) :

- **Monero (XMR)** : 87mbgGKqNXccHrDasneNmkfKYsPwJu16xfoR4YbzpFo9fQmY4xDxePH4CbPX2Ja8PN7Ld3BjmLtFAB18Y4qfqakiNw4BKi2

Merci !

---

## Licence

Copyright (C) 2026 florent4040

Ce programme est un logiciel libre : tu peux le redistribuer et le modifier selon les termes de
la **GNU General Public License version 3** (ou toute version ultérieure), publiée par la Free
Software Foundation. Toute version modifiée et redistribuée doit rester sous la même licence,
avec son code source.

Ce programme est distribué dans l'espoir qu'il sera utile, mais **sans aucune garantie** ; voir
le fichier [LICENSE](LICENSE) pour les détails.
