# NZB FF Tool Pro 3.26.0

Prépare tes vidéos pour Usenet : nom au format des indexers, rapport MediaInfo complet,
envoi vers le dossier surveillé par ton logiciel de post (NgPost ou autre), historique.

## Nouveautés de la 3.26

- **Processeur au repos, pour de bon.** La vraie cause des 12-13 % dans la zone de
  notification : Windows ne livre aucune image à une fenêtre cachée, et la boucle d'eframe
  tournait à vide en l'attendant (un cœur à 100 %, soit 1/8 du processeur). Désormais, fenêtre
  cachée ou réduite, **aucune image n'est demandée** ; un battement toutes les 2 s traite les
  événements des tâches de fond (posts, surveillance, dépôts et téléchargements continuent
  comme avant). Fenêtre ouverte : les événements sont regroupés (une rafale de lignes de NgPost
  = une image), les barres de téléchargement **glissent** au lieu de sauter, et le client est
  relevé chaque seconde quand un téléchargement avance (3 s sinon).
- **Configuration automatique complète** : recherche sur l'API Newznab (`/api`) **et** envoi
  sur l'adresse à part (`/v1/upload`) — le mode « adresse d'envoi à part » ne remplace pas la
  recherche, elle reste sur `/api`. Les **numéros de catégorie** (films et séries SD / 720p /
  1080p / UHD, FR, autres contenus) et les **groupes Usenet** sont repris tout seuls ; un groupe
  indexé adapté est ajouté au post s'il le faut. Testé de bout en bout contre un faux indexer :
  capacités, clé, recherche, envoi réel avec catégorie.
- **« Réglage actuel » de chaque indexer**, en tête de sa fiche : recherche, envoi, catégories,
  groupes, User-Agent — vert quand c'est prêt, rouge sinon.
- **Groupes Usenet par catégorie de l'indexer** (fiche de l'indexer) : ex. 2045 → 
  alt.binaries.hdtv.x264 ; 2000 vaut pour toute la famille Movies.
- **Ma requête** (fiche de l'indexer → Recherche et API) : écris une adresse, « POST … » ou
  colle une commande curl ; aperçu exact (clé masquée), envoi, réponse, et « Garder dans le
  profil ». Une requête qui modifie quelque chose demande confirmation.
- **User-Agent** (Réglages → Indexers, et par indexer) : curl, NZB FF Tool Pro, navigateur
  courant tiré au sort au lancement, roulement à chaque requête, ou texte personnalisé.
- **Traitement** : « Que faire, et pour quels indexers ? » juste sous le nom (avant : tout en
  bas) — aussi dans Série et Autre contenu ; la catégorie par défaut affiche son nom chez
  l'indexer (« défaut (2040 · Movies > HD) ») ; **choix des groupes Usenet** du post (puces
  proposées d'après le contenu, tes réglages et les groupes indexés, ou saisie libre), avec ce
  que chaque indexer en pense.
- **ZIP, RAR, 7z, ISO…** visibles d'emblée quand on clique pour choisir un fichier à traiter
  (avant : seulement les vidéos, il fallait passer à « Tous les fichiers »). Plusieurs
  fichiers d'un coup = traitement par lot, comme le glisser-déposer.
- **Nouvelle page « 📝 Rapports »** dans le menu : NFO, MediaInfo (format au choix) et BDInfo
  de fichiers, dossiers, disques ou NZB (téléchargés d'abord), en file, à côté de la source ou
  dans un dossier choisi, avec « Afficher » et « Copier » pour chaque rapport.
- Symboles illisibles (carrés) remplacés par des symboles présents dans toutes les polices.

## Nouveautés de la 3.25

- **Nouvelle page « 🔎 Recherche » (à la Prowlarr, en plus simple).** Une recherche part sur
  tous tes indexers actifs à la fois (chacun garde sa limite de requêtes par minute ; on peut
  en décocher). Les résultats sont **regroupés par release** : pour chacune, sur quels
  indexers elle est (✔), où elle manque (✖), sa taille, son âge et son type (filtres Films,
  Séries, Musique, Livres, Autre, et « Manque quelque part »). Actions : **⬇ Télécharger**
  (NZB récupéré puis confié à SABnzbd/NZBGet), **🔁 Reposter (N)** (NZB récupéré, repost
  préparé sur la page Téléchargements vers les seuls indexers où elle manque, à vérifier puis
  lancer), **…** → rapports seuls, enregistrer le NZB, copier le nom. Clic sur un ✔ :
  télécharger depuis cet indexer ou ouvrir sa page ; clic sur un ✖ : « elle y est (uploadée à
  la main) », elle n'y sera jamais repostée. Un indexer en erreur s'affiche avec la raison et
  un bouton « Régler ».
- **Configuration automatique d'un indexer** (Réglages → Indexers → l'indexer) : adresse de
  l'API et clé suffisent. « ✨ Configurer automatiquement » lit les capacités (catégories,
  groupes), vérifie la clé, cherche comment envoyer un NZB (Newznab `t=nzbadd`, OpenAPI,
  adresses d'envoi courantes comme `/v1/upload`) et applique le résultat, avec « ↶ Annuler ».
  **Pas de rafale** : délai réglable entre deux requêtes et nombre maximum de requêtes.
  « 🔎 Chercher l'adresse d'envoi » ne fait que cette dernière partie.
- **Lecture de la documentation d'API d'un indexer** : colle la page de doc (ou ouvre-la
  depuis un fichier) → « 📖 Lire » comprend l'adresse de base, l'adresse d'envoi (ex.
  `https://lien.indexer/v1/upload` alors que la recherche est sur `/api`), les champs
  (`file`, `nfo`, `mediainfo`…), la clé en paramètre, la catégorie facultative, la page de la
  release, et les points à savoir ; « ✔ Appliquer ces réglages ».
- **Dépôt refusé → solution proposée.** Sur un refus de l'API (405, 404, 401/403, codes
  Newznab 100–102, 200–203, 600–699, 413, 422, 429…), une fenêtre explique la cause probable
  et propose la correction : chercher l'adresse d'envoi, relire la doc, corriger le nom du
  champ, la clé, ou ouvrir les réglages de l'indexer.
- **Requêtes d'API visibles avant l'envoi, et avec le résultat** : l'aperçu exact de la
  requête (clé masquée) dans les réglages de l'indexer, puis « Dernier dépôt réel » avec la
  réponse reçue.
- **Groupes Usenet exigés par l'indexer** : les groupes des capacités (`<groups>`) sont
  gardés ; le récapitulatif avant post dit, indexer par indexer, si le post passe par un
  groupe indexé, et un groupe adapté est ajouté au post s'il le faut (règle réglable).
- **Exporter les catégories** lues par l'API : texte, CSV ou JSON.
- **« Uploadée à la main sur X »** (Historique → détails) : la release n'y sera plus
  jamais repostée. Avant chaque post, la vérification porte sur **le nom d'origine du fichier
  et le nom renommé**.
- **Rapports seuls** : NFO, MediaInfo et/ou BDInfo d'un fichier, d'un dossier ou disque
  (page Traitement → « 🧾 Rapports seuls »), ou d'un NZB (téléchargé d'abord : page
  Téléchargements, mode « Rapports seuls »), sans renommage, post ni dépôt. Un NFO existant
  n'est jamais écrasé.
- **E-mail de notification en couleur** : bandeau **rouge** s'il y a une erreur, **jaune**
  pour de simples alertes sans conséquence, **vert** quand tout est bon, chaque ligne colorée
  selon son résultat ; le texte brut reste joint (Réglages → E-mail → « E-mail en couleur »).
- **Processeur au repos** : l'application dans la zone de notification ne redessine plus rien
  (environ 12 % de processeur avant pendant la surveillance, à peu près 0 maintenant) ; fenêtre
  ouverte, 1 image par seconde au repos. Rappel : **aucune télémétrie, aucun mouchard** —
  l'application ne contacte que tes indexers, ton client de téléchargement, tes serveurs
  Usenet et ton serveur d'e-mail.

## Nouveautés de la 3.24

- **Télécharger un NZB sans le reposter, depuis l'application.** Page Téléchargements, carte
  « Ajouter des NZB » (glisser des .nzb ou coller une adresse) : pour chaque NZB, « ⬇
  Télécharger seulement » ou « 🔁 Télécharger puis reposter ». Téléchargement seul : aucun
  indexer demandé, pas de traitement ni de post (une adresse peut même se passer de nom).
  Dans les deux cas : **catégorie**, **priorité** (celle du client, basse, normale, haute,
  forcée) et **ajout en pause**, NZB par NZB. Choix de départ : Réglages → Téléchargements →
  « NZB ajouté sur la page Téléchargements » (me demander, télécharger seulement, reposter)
  et « Priorité donnée au client par défaut ».
- **Dossier de NZB surveillé entièrement réglable** (Réglages → Téléchargements) :
  - **une règle par emplacement** : racine, un sous-dossier par indexer actif (désactivable),
    `_Ailleurs`, `_Télécharger seulement`, et **tes propres sous-dossiers** (« ➕ Ajouter une
    règle », ex. `Séries 4K\`) ;
  - pour chaque règle : **action** (télécharger seulement, télécharger puis reposter, me
    demander, déposer tel quel sur les indexers, comme un double clic — racine —, ignorer),
    indexer d'origine et **indexers visés**, repost **tout seul ou d'un clic**, **catégorie
    du client**, **priorité**, **en pause** ; la phrase sous chaque règle dit ce qui sera
    fait ; « ⟲ Par défaut » remet une règle de base ;
  - **rythme** : passage toutes les N s, .nzb pris quand sa taille n'a pas bougé depuis N s ;
  - rangement (« _Pris en charge » ou supprimé ; refus toujours gardés dans « _Erreurs »
    avec la raison), doublons refusés ou non, **notification** à chaque .nzb pris ou non
    (erreurs toujours notifiées) ;
  - les sous-dossiers des règles sont créés tout seuls, et « À LIRE.txt » reprend tes règles.

## Nouveautés de la 3.23

- **Post en échec : rien n'est jamais supprimé.** La vidéo n'est mise à la corbeille, supprimée
  ou rangée dans « _Postés » qu'après un post dont le NZB est produit et vérifié. Après un échec,
  le message dit pourquoi (par2, compression, identification au serveur…) et **où est la vidéo**
  (« Rien n'a été supprimé ni déplacé : la vidéo est toujours dans … ») ; si elle a disparu
  pendant le post, il le dit aussi (un autre programme l'a prise : NgPost ouvert en surveillance
  du même dossier, par exemple — le récapitulatif signale ce cas).
- **Cause de l'échec « par2 code 3 » corrigée** (« Ignoring non-existent source file :
  D:\xxx.7z.001 ») : NgPost ajoute « /archive » au dossier temporaire, et par2cmdline(-turbo)
  coupait le chemin mélangé « D:\tmp/archive/… » au mauvais endroit. Le dossier temporaire (des
  Options du post ou de TMP_DIR de ngPost.conf) est maintenant toujours passé à NgPost en « / ».
  Dossier temporaire réseau (\\serveur\partage) avec par2cmdline : signalé avant le post.
- **Historique : le post est suivi.** Colonne « POST » : auto / manuel, en file, en cours, ✔, ✖
  (statut « Post en échec » en rouge, filtre « Posts en échec », bouton « ▶ Reposter » dans le
  détail). L'ancienne colonne « MODE » (« Manuel ») disait d'où le traitement avait été lancé,
  pas comment le post se faisait : elle devient « LANCÉ DEPUIS » (Surveillance, Traitement, Lot).
- **Mots de passe et clés jamais affichés ni écrits** : un registre de tous les secrets
  (serveurs Usenet des réglages et de ngPost.conf, clés API des indexers, SABnzbd / NZBGet,
  e-mail) les masque (••••••) dans le récapitulatif, les journaux (écran et fichiers), les
  notifications et l'e-mail ; toute valeur après « pass= », « password: », « apikey= » aussi
  (NgPost en mode debug détaille ses serveurs). L'identifiant d'un serveur n'est montré qu'à
  moitié (« mon•••• »).
- **Débit moyen des téléchargements juste** : moyenne dans le temps du débit annoncé par le
  client pendant qu'il télécharge ce fichier (pauses exclues). Avant : baisse du « reste »
  divisée par le temps, gonflée quand le client retire d'un coup les par2 inutiles ou les
  fichiers déjà complets.
- **Journaux** : le journal de l'application est aussi écrit sur disque, jour par jour
  (`logs\app-AAAA-MM-JJ.log`, gardés 90 jours). Page Journal : « 💾 Exporter (.zip) » (30
  derniers jours : posts, dépôts, application, plus la session en cours ; secrets masqués, à
  joindre à une demande d'aide) et « ↗ Dossier des journaux ». Carte du post : « ↗ Journaux »,
  aussi après un échec.

## Interface Simple ou Expert

Bascule en bas de la barre latérale (et Réglages → Apparence) :
- **🌱 Simple** : l'essentiel. Les réglages avancés (Nommage, Options du post, Serveurs Usenet,
  e-mail, fichiers de l'application, catégories de contenu) sont masqués et gardent leurs
  valeurs ; la recherche des réglages les trouve quand même. Pages de traitement : le format et
  l'emplacement du rapport MediaInfo sont résumés en une ligne ; journaux sans barre de filtres.
- **🛠 Expert** : tout. Une configuration d'une version précédente s'ouvre en Expert ; une
  première installation en Simple.

## Assistant de configuration

Au premier lancement, un assistant pas à pas : ① profil **Simple** ou **Expert** ②
dossiers (un clic crée Inbox, Sortie, Indexers et Quarantaine dans un dossier de travail)
③ logiciels de post (NgPost avec détection, MediaInfo, outil par2 : celui de NgPost,
par2cmdline-turbo ou ParPar, ce qui est trouvé, et les liens officiels de téléchargement)
④ indexers (ajout rapide : nom, adresse de l'API, clé) ⑤ téléchargement (SABnzbd / NZBGet,
avec test) ⑥ Windows (clic droit, ouverture des .nzb) ⑦ récapitulatif (✔ ou ⚠ pour chaque
point). Chaque étape peut être passée. À relancer à tout moment : **Réglages**, en haut →
« Tout configurer », ou « Configurer les logiciels » (NgPost, MediaInfo, par2, SABnzbd)
seulement. Une configuration d'une version précédente n'impose pas l'assistant.

## Récapitulatif et contrôles avant un traitement manuel

Traitement, lot (vidéos, séries, autres contenus), série, autre contenu : « Envoyer » ouvre
**un seul récapitulatif** pour tout le lancement (pas un par fichier), en blocs numérotés :
① ce qui est traité (chaque élément → son nom de release, nombre, taille totale) ② ce qui
sera fait (action, rapport MediaInfo et où il va, NFO, anti-doublon) ③ fichiers et dossiers
(copiés ou déplacés, destination, après le post, dossier temporaire et place libre, où vont
NZB et rapports) ④ post Usenet (logiciel, chaque serveur et ses connexions, serveur le plus
rapide ou non, taille des articles, fils, essais, obfuscation, compression, par2 et son
outil, vérification, groupes) ⑤ indexers (API ou à la main, catégorie envoyée, MediaInfo et
NFO joints). Puis « Lancer » ou « Modifier ».

Depuis la 3.22, le récapitulatif détaille aussi :

- **Serveurs.** Chaque serveur est listé avec son fournisseur, adresse:port, SSL, ses
  connexions, son identifiant, le serveur de vérification des articles (nzbCheck) et les
  serveurs désactivés. La liste vient de Réglages → Serveurs Usenet, ou, si les serveurs ne
  sont pas gérés ici, **de ngPost.conf lui-même** (chemin indiqué). Le total des
  connexions est donné, ou la règle « le plus rapide seul » avec la plage de connexions.
- **Compression et par2.** La compression réellement utilisée tient compte de celle
  imposée par la catégorie de contenu, et les contrôles aussi.
- **Groupes Usenet.** Pour chaque type présent (film, série, chaque catégorie de contenu),
  le récapitulatif dit d'où viennent les groupes. Un groupe de films pour un jeu est
  signalé.
- **Indexers.** Pour chacun : comment le NZB arrivera (API, ou site avec la page d'upload
  ouverte toute seule en « Tout automatique »), et la raison quand le dépôt par API est
  impossible (clé manquante…). Viennent ensuite la catégorie de chaque release (celle
  choisie pour ce traitement est indiquée) et les fichiers joints (MediaInfo, NFO
  d'origine ou créé).
- **NFO d'origine** trouvé ou non, et ce qui en sera fait.
- **Points bloquants** (en tête, en rouge ; « Lancer » désactivé) : ngPost.exe non réglé ou
  introuvable, aucun indexer choisi pour poster, dossier de sortie absent, nom vide, deux
  éléments donnant la même release.
- **Points d'attention** (en orange) : 7-Zip / RAR introuvable, pas de dossier temporaire,
  outil par2 introuvable, release déjà envoyée (avec « Voir l'historique »), dépôt par API
  sans clé, etc. Chacun a son lien « Ouvrir le réglage ».
- « Ne plus afficher le récapitulatif » (ou Réglages → Comportement) : seuls les points
  bloquants ou douteux s'affichent encore.

## Déjà sur les indexers ? (vérification avant le post)

Avant d'envoyer quoi que ce soit sur Usenet, la release est **cherchée sur chaque indexer
visé** :

- sous son **nom de release** (renommé ou non) ;
- et sous ses **noms d'origine** : nom du fichier vidéo, du dossier, du NZB reçu (gardés
  avec la release au moment du traitement).

Ensuite, selon le réglage (Réglages → Indexers → « Avant de poster : déjà sur l'indexer ? ») :

| Réglage | Déjà présente sur un indexer | Présente partout |
|---|---|---|
| **Ne pas la poster pour cet indexer** (défaut) | postée seulement pour les autres | pas postée : fenêtre « Déjà présente », avec « ⬆ Poster quand même » |
| Signaler seulement | postée quand même, signalée | postée quand même, signalée |
| Ne pas vérifier | — | — |

Une fois l'absence confirmée, la release est postée **sous son nom final** (renommé ou
non). Tout est écrit :

- dans le **récapitulatif** : bloc « Déjà sur les indexers ? ». La recherche se lance à
  l'ouverture et le bloc se remplit tout seul : noms cherchés, résultat par indexer, nom
  posté ;
- dans le **journal NgPost** : noms cherchés, résultat de chaque indexer
  (⚠ déjà présente, ℹ versions proches, ✔ absente, ? impossible), nom posté.

La recherche est refaite juste avant chaque post, car la file peut attendre. Si une
vérification échoue (réseau, clé refusée), la release est postée et la ligne « ? » le signale.

**Envoyer des NZB** fait la même vérification avant le dépôt. Les releases déjà présentes ne
sont pas déposées, et « 📤 Déposer quand même » est proposé.

## Profils d'API des indexers (Réglages → Indexers → « Recherche et API »)

Chaque indexer a un **profil d'API** : comment l'interroger, quel qu'il soit, aujourd'hui ou
plus tard. La démarche est celle des projets *arr (Prowlarr, Sonarr, Radarr) et de NZBHydra2 :

- **Préréglages.** Les indexers Usenet connus de ces projets : abNZB, altHUB, DOGnzb,
  DrunkenSlug, NZBgeek, NZBFinder, NZBCat, NzbPlanet, Tabula Rasa, Miatrix, NinjaCentral,
  SimplyNZBs, Usenet Crawler, Spotweb…, avec leur adresse et leur chemin d'API (relevés
  dans leur code en novembre 2026).
  - S'y ajoutent les moteurs publics NZBIndex (API JSON) et BinSearch (page HTML).
  - Trois modèles servent pour tout indexer à venir : **Newznab générique** (nZEDb, NNTmux,
    Spotweb, newznab+…), **API REST / JSON**, et **page de recherche du site**.
  - Dans la liste des indexers, « 📚 Depuis un préréglage… » crée l'indexer tout réglé :
    il ne reste qu'à coller la clé.
- **Capacités** (`t=caps`). Pour un indexer Newznab, « 🔄 Lire les capacités » récupère :
  types de recherche, paramètres acceptés (q, season, ep, imdbid, tvdbid…), catégories et
  limites. Les requêtes en sont tirées automatiquement, comme dans Prowlarr.
- **Profil automatique** (par défaut). C'est l'API Newznab, avec l'adresse et la clé déjà
  réglées pour le dépôt : rien à faire pour un indexer Newznab classique.

L'onglet a quatre parties.

### 🛡 Vérification avant le post

- Choix de la requête de recherche utilisée, ou « Ne pas vérifier sur cet indexer ».
- **Nom envoyé** : tel quel puis avec des espaces si rien n'est revenu (par défaut), tel
  quel, ou avec des espaces.
- **🔍 Tester la recherche** montre les requêtes parties (clé masquée), les résultats
  (● = même nom) et le verdict.

### ▶ Explorateur

Pour interroger l'indexer sans connaître son API :

1. Choisis une **action** (« Rechercher une release », « Rechercher une série »,
   « Capacités et catégories », « Détails d'une release », « Dernières releases »… ou
   celles importées).
2. Remplis ses champs en clair. Les listes de valeurs (catégories…) sont proposées.
3. La **requête qui partira** s'affiche en direct, clé masquée.
4. Clique **▶ Envoyer**.

La réponse montre le code HTTP, le temps, un résumé, et le **tableau des résultats lus**
(titre, taille, lien). La réponse brute est à part, avec « 📋 Copier ». Une requête qui
modifie quelque chose chez l'indexer (POST d'envoi, suppression…) demande une confirmation.

### 🔧 Requêtes et lecture

Chaque requête se règle entièrement.

**La requête**

- **Méthode** : GET, POST, PUT, PATCH ou DELETE.
- **Lecture seule** : une recherche faite en POST reste une lecture ; cochée, elle peut
  servir à la vérification.
- **Adresse modèle**, par exemple :
  - `{base}/search/{release}` : le nom dans le chemin, comme `https://indexer/search/test` ;
  - `{base}/api/search` ;
  - `{api}?t=search`.

**Les paramètres**, chacun avec :

- son nom ;
- sa valeur : texte fixe, `{release}` pour le nom cherché, `{key}`, ou vide pour la demander
  dans l'explorateur ;
- son libellé ;
- **où** il va : adresse (`?q=…`), chemin, formulaire POST, JSON, en-tête ;
- son **encodage** : `%20` (`mon%20ami`), `+` (`mon+ami`) ou brut ;
- s'il est obligatoire.

**Les en-têtes** et le **corps** (JSON ou texte modèle, par exemple
`{"query": "{release}"}`).

**Lecture de la réponse**

| Format | Où sont les résultats | Champs |
|---|---|---|
| XML / RSS | `item`, `channel/item` | `title`, `enclosure@length`, `newznab:attr[name=size]@value` |
| JSON | `data.content` (trouvé tout seul si vide) | `name`, `files.0.name`, `title\|name` |
| Page HTML | motif d'un résultat : `<a class="title">{title}</a>*<td>{size}</td>` | dans le motif |

- Le **nettoyage du titre** (`"{title}"`) garde ce qui est entre guillemets, comme dans les
  sujets Usenet. L'extension (.mkv, .rar…) est retirée.
- L'**erreur** renvoyée par l'API est reconnue : Newznab (codes 100 à 910 traduits), JSON
  `error`, `message`, `errors`.
- **🪄 Régler la lecture d'après cette réponse** (dans l'explorateur) trouve tout seul où
  sont les résultats d'une API inconnue.

### 🔌 Connexion et capacités

- Type de profil.
- Adresse de base et chemin de l'API.
- Clé (vide : celle du dépôt).
- **Transmission de la clé** : paramètre d'adresse (`apikey`, `key`…), en-tête
  (`X-Api-Key`…), `Authorization: Bearer`, identifiant + mot de passe (Basic), cookie, ou
  aucune.
- User-Agent.
- **Limite de requêtes par minute** : l'application attend s'il le faut.
- Délai maximal et notes.

### Importer et exporter

**📥 Importer** lit :

- un fichier, une adresse (`https://…/openapi.json`, page wiki de l'API) ou un texte collé ;
- **OpenAPI 3 / Swagger 2** en JSON ou en YAML : adresse, chemins, paramètres, corps, et la
  clé (`securitySchemes`) ;
- une **collection Postman** ;
- une **page de documentation** : adresses d'exemple (`…/search?q=mon+ami&apikey=…` donne
  la requête, ses paramètres et l'encodage `+`), lignes `GET /chemin`, et tableaux de
  paramètres (descriptions reprises comme libellés) ;
- une **réponse `t=caps`** ;
- un **profil exporté**.

**📤 Exporter** écrit le profil dans un fichier `.nzbff-api.json`, **sans la clé**, pour le
garder ou le partager.

Les réglages de recherche de la 3.20 (adresse personnalisée, actions importées) sont
convertis automatiquement en profil.

## Démarrage avec Windows

- **Lancer avec Windows** : au démarrage du PC, l'application démarre **directement dans la
  zone de notification, sans fenêtre**. Il n'y a rien d'autre à cocher.
- **Même quand je l'ouvre moi-même : sans fenêtre** est désactivé par défaut : un double-clic
  sur l'application ouvre la fenêtre normalement.
- Une ancienne inscription au démarrage (sans `--tray`, ou exécutable déplacé) est corrigée
  automatiquement au lancement.

## Compiler (Windows, chaîne GNU / MinGW)

Double-clique sur **build.cmd** → `dist\NZB FF Tool Pro.exe` (avec son icône).
Si tu avais `ngprep.conf` / `ngprep-history.tsv` dans `dist`, ils sont repris automatiquement.

Prérequis : Rust (`stable-x86_64-pc-windows-gnu`) et WinLibs dans `C:\mingw64`.
`.cargo\config.toml` indique à Rust où est `dlltool.exe` ; copie-le dans
`%USERPROFILE%\.cargo\config.toml` pour tous tes projets Rust.

## Nommage

1. Un nom de release existant est repris (nom du fichier, titre MKV, dossier parent),
   avec la langue ajoutée s'il n'en a pas. Reconnu aussi **sans année ni résolution** (DVD,
   vieux films) dès qu'il a un tag `-GROUPE` et au moins trois mots de release dont une langue
   ou une source : `film.FRENCH.VOF.DVDRemux.AC3.2.0.MPEG2-LiHDL` → titre `film`, langue
   `FRENCH.VOF` (VOF = version originale française), source `DVDRemux`, audio `AC3.2.0`,
   codec `MPEG2`, tag `LiHDL`. Plusieurs langues restent ensemble (`MULTi.VFF`), l'audio
   (`DTS-HD.MA.5.1`, `TrueHD.7.1.Atmos`, `DDP5.1`…) et les autres mots (HDR, DV, 10bit,
   REPACK…) sont gardés quand tu modifies un élément.
2. Sinon : `Titre.Année.FRENCH.1080p.Source.Codec-TAG` (modèle modifiable).
   - Source : seulement si elle est écrite quelque part (MediaInfo, nom du fichier, dossier).
     Jamais inventée : introuvable → omise.
   - Codec : x264 / x265 / H264 / H265 / AV1, et MPEG2 / VC-1 / XviD pour les DVD et vieux
     encodages. Format inconnu → omis.
   - Audio (repère `{audio}`, à ajouter au modèle si tu le veux) : lu par MediaInfo sur la
     première piste (`AC3.2.0`, `DTS-HD.MA.5.1`, `TrueHD.7.1.Atmos`, `DDP5.1.Atmos`…).
   - Catégorie : DVD, DVDRemux, DVDR, PAL, NTSC, MPEG2, XviD comptent comme SD ; sans
     résolution dans le nom, celle du rapport MediaInfo est utilisée.
   - Langue lue piste par piste (langue + titre « VFF », « English »…) : FRENCH, MULTi, VOSTFR.
     Une piste sans langue compte comme française (réglable).
3. Dans Traitement, chaque élément (titre, année, langue, résolution, source, audio, codec,
   tag, et « En plus » s'il y en a) est modifiable, ainsi que le nom final.

## Séries

Page **Série** (dossier glissé dans la fenêtre, bouton « 📺 Dossier de série… » de la page
Traitement), section **Séries** de la page Lot, ou inbox :
- Structures reconnues : `Série\Saison 1\…`, une saison seule, ou épisodes à plat.
  Numéros : `S01E02`, `s1e2`, `S01E01E02`, `S01E01-E03`, `1x02`, « Saison 1 Épisode 2 »,
  `Ep02`, ou simple numéro dans un dossier `Saison N` / `Season N` / `S01`. Échantillons ignorés,
  sous-titres et NFO emportés avec leur épisode.
- Par saison : nombre d'épisodes, épisodes manquants, doublons. Une saison est **complète**
  de E01 au dernier épisode sans trou ; plusieurs saisons doivent se suivre depuis la saison 1.
  Complète → mention `COMPLETE` (ou `INTEGRALE`, au choix) dans le nom.
- Nom de release comme pour les films :
  `Titre.Année.S01-S05.COMPLETE.FRENCH.1080p.WEB.x264-TAG` (modèles dans Réglages → Séries,
  repères `{saison}`, `{episode}`, `{complete}` en plus) ; titre, langue, résolution, codec,
  source et tag détectés, tous modifiables.
- **Épisodes : jamais renommés par défaut.** Seuls le nom de la release et, s'il y a plusieurs
  saisons, les dossiers `Saison N` sont créés ; chaque épisode garde exactement son nom.
  « Renommer aussi les épisodes » (décoché) les renomme `Titre.S01E02.FRENCH.1080p…`.
  Une ancienne configuration où c'était coché le reste : décoche-le dans Réglages → Séries.
- **Releases à créer (plusieurs tours, postés dans cet ordre)** — page Série et Réglages → Séries
  (inbox, Lot) ; au moins un coché :
  1. **toute la série** en une release (un NZB), avec la mention complète si elle l'est ;
  2. **une release par saison** (un NZB par saison) ;
  3. **une release par épisode** (chaque épisode posté seul, comme un film, avec son MediaInfo).
  Ex. intégrale, puis saison par saison : cocher 1 et 2. Les noms de chaque tour sont affichés
  avant de lancer.
- **Copies et suppressions** : seul le premier tour copie ou déplace la source. Les tours
  suivants sont faits à partir de la première release par **liens physiques** (même fichier sur
  le disque, aucune place en plus, instantané) ; une fois un tour posté et rangé ou supprimé,
  les autres releases restent entières. Si le disque ne permet pas les liens (certains NAS,
  FAT32/exFAT), copie à la place.
- **Mention complète : `COMPLETE` ou `INTEGRALE`** au choix sur la page Série (ou un mot à toi),
  `COMPLETE` par défaut, modifiable dans Réglages → Séries. Elle ne concerne que la release de
  toute la série : sans ce tour, elle est décochée et grisée (une saison seule n'est pas une
  intégrale).
- **Résolution vérifiée saison par saison** (MediaInfo du premier épisode de chaque saison ; de
  chaque épisode pour le tour « par épisode ») : si une saison diffère (ex. une saison en 720p
  dans une intégrale 1080p), la page Série **prévient avant de lancer**, la release de toute la
  série prend la résolution de la majorité des épisodes (modifiable) et chaque release de saison
  ou d'épisode porte **sa propre résolution** (`Ma.Serie.S03.FRENCH.720p…`). En automatique
  (inbox, Lot), c'est fait tout seul, et l'écart est noté dans l'historique, le Journal et
  l'e-mail.
- Arborescence livrée (dossier de la release = ce que reçoit celui qui télécharge) :
  - une saison : `Ma.Serie.S01…\épisodes` ;
  - plusieurs saisons (complète) : `Ma.Serie.S01-S05.COMPLETE…\Saison 1\épisodes`, `Saison 2\…`.
  Le rapport MediaInfo du premier épisode est au nom de la release. NgPost reçoit le dossier de
  la release : avec la compression, l'archive garde l'arborescence ; sans compression, les
  épisodes sont postés à plat (avertissement affiché).
- **Mention COMPLETE automatique** seulement pour une série entière livrée d'un bloc : dossier
  de la série, au moins deux saisons depuis S01, sans épisode manquant. Une saison seule (dossier
  « Saison 1 » glissé, ou une release par saison) n'est jamais marquée d'office : rien ne dit
  qu'il ne manque pas d'autres saisons. La case reste disponible pour la cocher toi-même.
  **Exception : un dossier qui se dit complet** (« Intégrale », « Integrale », « L'intégrale »,
  « Complete ») est marqué même s'il n'a qu'une saison (ex. 500 épisodes en S01), tant qu'il ne
  manque aucun épisode ; s'il en manque, la page Série dit lesquels. Le mot ajouté au nom est
  celui de Réglages → Séries (`COMPLETE` par défaut, `INTEGRALE` si tu préfères).
- **Séries de plusieurs centaines d'épisodes** : Windows limite la ligne de commande à
  32 767 caractères, chemins longs activés ou non (« os error 206 »). Quand la liste des
  fichiers ne tient pas, NgPost reçoit le dossier de la release (ou de chaque saison) au lieu
  de chaque épisode et en poste tous les fichiers, **avec leurs noms** : celui qui télécharge
  reçoit toujours `Release\épisodes` (une saison) ou `Release\Saison N\épisodes`. Post
  compressé : le dossier est compressé avec `--rar_no_root_folder`, pour que l'archive ne
  contienne pas un dossier `Release` de plus (option de NgPost récent, avec RAR).
- **Une seule saison de centaines d'épisodes** : `S01E001` à `S01E9999`, ou numérotation
  continue sans saison (`Ma Serie 001`, `Ma Serie - 0512`, `Episode 513`) dans un dossier
  « Intégrale ».
- **Aperçu en arborescence** de ce que reçoit celui qui télécharge (pages Série et Autre
  contenu) : dossiers repliables, nombre de fichiers et tailles, à plat si le post est sans
  compression.
- **Rapport MediaInfo affiché avant de lancer** (onglets Standard / Complet, filtre, copie, choix
  de l'épisode lu), juste avant « Que faire de cette série ? ».
- Anti-doublon : une saison envoyée seule après la série complète n'est **pas** un doublon (nom
  différent). Un nom n'est refusé que s'il a déjà été **posté**, ou préparé et encore en attente
  sur le disque ; le bandeau dit lequel, avec sa date, et propose « Envoyer quand même ».
- Indexers : catégories TV à part (5030 / 5040 / 5045 par défaut, Réglages → Indexers → Dépôt).
- Inbox : un dossier de série déposé est traité d'un bloc, une fois entièrement copié, avec les
  réglages de Réglages → Séries.
- **Lot** : les dossiers de série du dossier analysé (ex. `Ma Serie Saison 1`, `The Office (US)`)
  sont repérés tout seuls et affichés à part (« 📺 Séries · N ») avec le nom de chaque release ;
  leurs épisodes ne sont plus comptés comme des films. « Préparer N série(s) » les traite d'un bloc
  comme l'inbox (réglages des séries), avec l'action, les indexers et la destination choisis pour
  le lot. Un sous-dossier qui n'est qu'un numéro de saison (`Saison 2`, `S03`) appartient à la
  série du dossier parent.
- **Type et catégories affichés en grand avant le post** : « 📺 SÉRIE » / « 🎬 FILM » / catégorie
  de contenu, résolution, français ou non, puis pour chaque indexer la catégorie exacte qui sera
  envoyée (ex. `→ altHUB catégorie 5342 (TV - FR > 1080p) · dépôt par API`). Pages Traitement,
  Série et Autre contenu ; au survol de chaque ligne dans le Lot.

## Autres contenus (jeux, logiciels, musique, ebooks, XXX…)

**Groupes Usenet (3.22).** Une catégorie de contenu sans groupes à elle reçoit ceux
conseillés pour son type, jamais ceux des films réglés par défaut :

- jeux consoles : alt.binaries.games, cd.image, boneless, plus le groupe de la plateforme
  quand il est connu (PS3, Xbox 360, DS) ;
- musique : lossless ou mp3 selon le format.

La page et le récapitulatif affichent les groupes et leur origine.

Page **Autre contenu** (ou fichier / dossier sans vidéo glissé dans la fenêtre) : n'importe quel
fichier ou dossier, posté avec sa **catégorie**.
- Catégories proposées : XXX / Adulte, Jeux PC, Jeux consoles, Logiciels PC, Logiciels Mac,
  Musique, Ebooks, Mangas / BD, Animation, Éducation / Formation, Autre. **Toutes modifiables**
  (Réglages → « Catégories de contenu ») : nom, icône, ordre, suppression, et « + Catégorie »
  pour en créer autant que tu veux.
- Pour chaque catégorie :
  - **catégorie Newznab** envoyée par API (ex. 4050 Jeux PC), et **une catégorie propre à
    chaque indexer** si ses numéros diffèrent (« Par indexer » ; vide = celle par défaut) ;
  - **modèle de nom** avec des champs : `{titre}` `{artiste}` `{auteur}` `{annee}` `{version}`
    `{tome}` `{plateforme}` `{langue}` `{format}` `{resolution}` `{source}` `{codec}` `{tag}`
    (aperçu immédiat ; les champs vides disparaissent avec leur séparateur) ;
  - **MediaInfo** (rapport du premier fichier vidéo ou audio, aucun s'il n'y en a pas) ;
  - **compression** : comme les options du post, toujours (garde les sous-dossiers), ou jamais
    (fichiers à plat) ;
  - **dossier de l'inbox** : ce qui est déposé dans `inbox\@<Catégorie>\` est traité tout seul
    par la surveillance, avec le modèle de nom de la catégorie.
- La catégorie est devinée d'après les fichiers (FLAC/MP3 → Musique, EPUB/PDF → Ebooks,
  CBZ/CBR → Mangas, DMG → Mac, NSP/XCI ou PS5/Switch dans le nom → Consoles, groupe de jeux connu
  → Jeux PC, EXE/ISO → Logiciels PC), sinon celle du dossier `@Catégorie`, sinon la dernière.
- Nom : un nom déjà au format release (`Titre.v1.0-GROUPE`) est gardé ; sinon il est construit
  avec le modèle (titre, version, langue, plateforme, format, groupe détectés), tout est
  modifiable, et l'anti-doublon s'applique comme pour les films.
- **Lot** : les éléments d'un dossier qui ne sont pas des vidéos (jeux, logiciels, albums, ebooks…)
  apparaissent dans « Autres contenus », catégorie devinée et nom tiré du modèle (modifiables),
  traités avec les choix du lot. Traitement : un fichier qui n'est pas une vidéo ouvre Autre contenu.
- Livraison : `<sortie>\<release>\` avec le fichier ou l'arborescence d'origine (fichiers
  cachés et Thumbs.db ignorés). Un `.nfo` présent est copié à côté du NZB pour les indexers.
- La catégorie suit la release jusqu'aux dossiers des indexers : elle est envoyée par API
  (Newznab, REST, `{cat}` des commandes) et affichée sur la page Indexer (« catégorie 4051 »)
  pour un dépôt à la main.

## MediaInfo

- Rapport complet (`--Full`, XML) par défaut ; format modifiable (JSON, texte, HTML…).
- Généré sur la vidéo renommée : il porte le nom de release, pas ton chemin local.
- Emplacement : avec la vidéo, ou dans un dossier dédié aux rapports ; format au choix.
  Réglage par défaut dans Réglages → MediaInfo, modifiable à chaque traitement (Traitement, Lot).
- Traitement affiche le rapport en direct (standard ou complet), filtrable, copiable,
  dans une zone redimensionnable ou une grande fenêtre (« Agrandir »).

## Disques complets : Blu-ray et DVD

Reconnu **tout seul** (glissé dans la fenêtre, page Autre contenu, Lot, inbox) :
- dossier de release rangé comme les releases FBR : `Film\Sample`, `Film\Proof`,
  `Film\film.iso`, `Film\film.nfo` (l'ISO est lue pour y trouver BDMV / VIDEO_TS ; à défaut,
  son nom et sa taille : BLURAY, BD25, BD50, UHD, COMPLETE… ou DVD, DVD9, PAL…) ;
- dossier du disque (`BDMV`, `VIDEO_TS`) ou dossier qui le contient (`Film\DISC\BDMV`) ;
- image ISO seule.

Les échantillons (`Sample\`, `*-sample.mkv`) et les preuves (`Proof\`) ne sont jamais pris pour
des films, nulle part.

- **Catégories** « Film Blu-ray complet » (2050) et « Film DVD complet » (2070), modifiables
  (numéro par indexer si le tien diffère). Le nom d'origine du dossier est gardé s'il a la forme
  d'une release ; sinon `Titre.Année.MULTi.1080p.COMPLETE.BLURAY-TAG` / `…PAL.DVDR-TAG`.
  Compression imposée : l'arborescence et l'ISO arrivent intactes. Tout le dossier est posté
  (ISO, NFO, Sample, Proof).
- **MediaInfo obligatoire** : rapport du film principal (plus grosse playlist `.mpls`, ou titre
  `VTS_xx` le plus lourd ; à défaut son plus gros `.m2ts` / `.VOB`). ISO montée par Windows en
  lecture seule le temps de la lecture. Sans rapport, pas de dépôt.
- **BDInfo** : comme à la main, sur la **plus grosse playlist** (BDInfo demande laquelle
  analyser) : lue sur le disque, ou sur l'ISO montée, ou d'après la liste des playlists de
  BDInfo. Lancé dès l'ouverture du Blu-ray (aperçu), arrêtable, gardé en cache et réutilisé au
  traitement (pas de second scan). « ⬇ Installer bdinfo-rs » (page Autre contenu ou Réglages →
  MediaInfo) télécharge la dernière version sur github.com/agentjp/bdinfo-rs, vérifie son
  empreinte SHA-256 et la range dans le dossier `bdinfo` de l'application ; BDInfoCLI marche
  aussi. Disque chiffré (AACS) : refusé par BDInfo.
- **Rapport BDInfo propre** (modèle dans Réglages → MediaInfo → « Modèle du rapport BDInfo ») :
  parties gardées au choix (infos du disque, ligne « forum », playlist, vidéo, audio,
  sous-titres, fichiers, chapitres, diagnostic, résumé rapide ; par défaut disque, playlist,
  vidéo, audio, sous-titres), **jamais** la publicité de BDInfo (« BDInfo Home »,
  « Includes forums report for »), les notes, les balises `[code]` ni les bandeaux. Texte du
  modèle avec `{bdinfo}`, `{release}`, `{disque}`, `{playlist}`, `{video}`, `{audio}`,
  `{soustitres}`, `{resume}` ; aperçu en direct sur un rapport d'exemple. Appliqué à chaque
  Blu-ray, en manuel comme en automatique. Onglet BDInfo : « Livré (propre) » ou « Brut ».
- **Joint comme le MediaInfo** : « Joindre le rapport BDInfo » (page Autre contenu, pour ce
  traitement). Chaque indexer peut avoir son **champ BDInfo** (Réglages → Indexers → « Champ du
  rapport BDInfo », ex. `bdinfo`, selon la documentation de son API ; `{bdinfo}` en mode
  commande). Sans champ : le BDInfo sert de NFO, ou, si le disque a déjà son NFO, au choix :
  NFO seul, **NFO + BDInfo dessous**, ou BDInfo à la place (Réglages → MediaInfo, et pour ce
  traitement).
- **Aperçu avant dépôt**, onglets **MediaInfo · BDInfo · NFO**, filtrables et copiables.
- **NFO** : celui d'origine (dans le dossier ou à côté de la vidéo) est **toujours gardé et
  envoyé** à l'indexer, même si « créer le NFO » n'est pas coché. Sans NFO d'origine, et
  si l'indexer a « créer le NFO » coché : rapport BDInfo pour un Blu-ray, MediaInfo en texte
  sinon.

## NFO

**Réglages → MediaInfo et options de chaque indexer = valeurs par défaut**, appliquées telles
quelles en automatique (inbox, surveillance). En manuel (pages Traitement, Série, Lot, Autre
contenu), le bloc **« Joint au NZB · pour ce traitement »** les propose toujours, même décochées
par défaut, et les change pour ce traitement seulement (les réglages ne bougent pas) :
- **« Joindre le rapport MediaInfo aux indexers »** (le rapport est créé dans tous les cas) ;
- NFO, comme ci-dessous, avec **« 👁 Aperçu du NFO qui sera créé »**.

- **NFO d'origine** (même nom que la vidéo, seul NFO de son dossier, NFO à la racine de la
  série ou du disque) : visible avant l'envoi (« 👁 Voir »), avec deux choix :
  - **« Garder le NFO d'origine dans la release postée »** (séries, disques, contenus ; un film
    seul ne poste que la vidéo) : décoché, il n'est pas copié dans la release et reste à sa place
    d'origine. Le NFO à la racine d'une série est placé à la racine de chaque release ;
  - **« Envoyer le NFO d'origine aux indexers »** : décoché, aucun NFO n'est joint au NZB, et
    aucun n'est créé à sa place ; le journal des dépôts l'indique.
- Sinon, **« Créer un NFO »** : MediaInfo en texte (rapport BDInfo pour un Blu-ray), envoyé de
  la même façon.

Ce qui décide la création d'un NFO, depuis la 3.22 :

- **Seul le réglage « Créer un NFO quand la release n'en a pas »** (ou le choix du
  traitement) décide qu'un NFO est créé. Décoché, seul le NFO d'origine part, et aucun
  n'est créé. Un traitement manuel peut quand même en demander un.
- La case **« nfo » d'un indexer** ne force plus jamais la création. Elle dit seulement si
  cet indexer reçoit aussi un NFO **créé** ; le NFO d'origine part toujours.
- **Autres contenus** : le NFO d'origine posé **à côté** d'un fichier seul est maintenant
  trouvé. C'est soit le même nom (« Jeu.iso » / « Jeu.nfo »), soit le seul NFO du dossier.

**Modifier avant le traitement** (« ✎ MediaInfo », « ✎ NFO », « ✎ BDInfo » dans le même bloc) :
le texte qui serait livré s'ouvre dans un éditeur (rapport MediaInfo au format choisi, NFO
d'origine ou celui qui serait créé, BDInfo propre) ; « Utiliser ce texte » le livre tel quel,
pour ce traitement seulement ; « annuler » revient à l'original. Série : le NFO modifié vaut
pour chaque release de la série (pas de MediaInfo modifiable : un rapport par release).

## Catégorie par indexer, avant le traitement

Le bandeau du type de release (pages Traitement, Série, Autre contenu) donne la catégorie qui
partira chez chaque indexer, et **« changer »** permet d'en choisir une autre dans la liste de
l'indexer (ou de saisir son numéro) quand celle calculée ne convient pas : pour ce traitement
seulement, elle suit la release jusqu'au dépôt ; « ↺ Auto » revient au calcul.

- La catégorie se change **pour chaque indexer**, qu'il soit servi par API ou sur le site.
- **Plateformes (3.22).** Une plateforme écrite dans le nom (PS2, PS3, Switch, Xbox 360,
  Wii U, GameCube, NDS…) choisit la catégorie de cette plateforme chez chaque indexer,
  d'après **sa liste de catégories**. Par exemple « Gran.Turismo.4.PAL.PS2 » part en
  « Console > PS2 » là où elle existe, et en catégorie Consoles ailleurs.
- Le champ **Plateforme** propose les plateformes courantes, plus celles des catégories
  des indexers visés.
- Une **image de jeu** (PlayStation…) n'est plus jamais prise pour un DVD vidéo, même avec
  « PAL » dans le nom.

**Importer la liste de catégories** (Réglages → Indexers → Catégories) lit :

- le XML `t=caps` ;
- le JSON ;
- la **liste HTML copiée de la page d'upload** (`<optgroup>` / `<option>`) ;
- le texte sous toutes ses formes : « 1080 Console > PS2 », « 1080 - PS2 »,
  « 1080;Console;PS2 », « PS2 (1080) », tableau « | 1080 | Console | PS2 | », et
  sous-catégories indentées sous leur famille.

Toutes les catégories sont gardées. Les lignes sans numéro sont **listées** après l'import,
pour voir ce qui n'a pas été compris.

## Journaux

- **Groupes Usenet** où chaque release a réellement été postée (lus dans le NZB) : journal
  NgPost, Journal de l'application, résumé de chaque dépôt (« Groupes Usenet (d'après le NZB) »)
  et e-mail de fin.
- Journal NgPost, dépôts sur les indexers et Journal de l'application :
  - **agrandir / réduire** en tirant la poignée sous le journal (hauteur gardée ; double-clic :
    taille d'origine) ;
  - **« 🗖 Détacher »** : le journal s'ouvre dans sa propre fenêtre, redimensionnable par les
    bords et les coins, agrandissable, déplaçable sur un autre écran (sa taille est gardée pendant
    la session) ; « 📌 Premier plan » la garde au-dessus des autres ; la fermer ou « ⬅ Rattacher »
    la remet dans la page ;
  - **barre de réglage** (mode Expert, et toujours dans une fenêtre détachée) : 🔍 filtre,
    « ⚠ Problèmes seuls », « ⬇ Suivre » (défilement automatique), « ↵ Retour à la ligne »
    (décoché : défilement horizontal), taille du texte A− / A+ (gardée) ; texte sélectionnable.
- **Niveau de détail** : « 🔎 Normal / Détaillé » sur chaque journal, ou Réglages →
  Comportement → Journaux. Détaillé ajoute, pour les prochains posts et dépôts :
  - choix des serveurs Usenet : mode (tous ensemble ou le plus rapide seul) et pourquoi, serveurs
    désactivés ignorés, mesures complètes de chaque test (connexion, TLS, réponse médiane et
    meilleure, débit et articles lus, identifiants, accueil) et valeur de classement ;
  - réglages du post : taille des articles, fils, essais, obfuscation, compression (volumes,
    noms, mot de passe aléatoire / fixe / aucun — jamais sa valeur ici), par2, vérification ;
  - lignes techniques des connexions NgPost (sinon gardées dans le fichier journal) ;
  - réponses entières des indexers (sinon 40 lignes).
  Mots de passe et clés API jamais écrits, quel que soit le niveau.

## Inbox : tout en automatique

Dans l'inbox, sans rien choisir : vidéo seule, **dossier de film** (`Film\film.mkv` + NFO +
Sample + Proof : la vidéo est traitée avec le nom du dossier, son NFO repris, le reste du
dossier part ensuite à la corbeille), **dossier de série**, **disque complet**, et **autres
contenus** reconnus (jeu, logiciel, album, ebook… d'après leurs fichiers). Ce qui n'est pas
reconnu reste dans l'inbox ; les dossiers « @Catégorie » imposent la catégorie.

## Destination et transfert

- Destination : dossier de sortie, sur place (renommage dans le dossier d'origine), ou autre dossier.
- Lecteur réseau → copie (l'original reste) ; disque local → déplacement. « Conserver
  l'original » décoché : le fichier est **déplacé**, même vers un autre disque (D: → C: :
  copié puis l'original supprimé). Un original en lecture seule est rendu modifiable puis
  supprimé ; encore ouvert ailleurs (antivirus, aperçu), il est réessayé quelques secondes. S'il
  reste malgré tout, c'est dit (avertissement, historique, e-mail) au lieu d'être passé sous
  silence.
- L'interface ne lit jamais elle-même un disque réseau occupé : dossiers (inbox, quarantaine,
  `_Postés`), place libre et emplacements sont relus en arrière-plan, pour qu'une copie depuis
  le NAS ne la ralentisse pas.
- Copie suivie en direct : pourcentage, débit, temps restant.
- La vidéo passe par un dossier temporaire puis arrive d'un coup : le logiciel de post ne
  voit jamais un fichier incomplet.

## Circuit NgPost → indexers

Réglages → « NgPost et indexer » : « Détecter ngPost.exe » (ou Parcourir).
ngPost.conf est alors lu automatiquement (quel que soit son encodage) :
- dossier surveillé par NgPost (`inputDir`, ou un dossier de surveillance s'il y en a un)
  → dossier de sortie ;
- `nzbPath` → dossier des NZB.
Les champs vides sont remplis tout seuls ; si un réglage diffère de NgPost, un avertissement
propose « Utiliser ». Puis : dossier racine des indexers (ex. `Indexers NZB`) et
« Activer le circuit ». Réglages → Indexers : « + Ajouter et créer son dossier ».

    Indexers NZB\
      _En attente du NZB\<release>\          rapport MediaInfo, en attente du NZB
      <Indexer A>\_À uploader\<release>\     NZB + MediaInfo à déposer sur A
      <Indexer A>\_Déjà uploadés\<release>\
      <Indexer A>\_Ignorés\<release>\       pas pour A
      <Indexer B>\...

## Envoyer des NZB (déjà postés, ou déjà en ta possession)

Page « 🔁 Envoyer des NZB » : déposer sur d'autres indexers un NZB qui existe déjà — **rien
n'est reposté sur Usenet**. Le haut de la page montre où tu en es :
① Releases → ② Indexers → ③ Ce qui part → ④ Dépôt.
1. **Quelles releases ?** Deux onglets :
   - **📦 Déjà postées** : retrouvées automatiquement dans les dossiers des indexers
     (`_Déjà uploadés`, `_À uploader`, `_Ignorés`), **y compris ceux d'indexers désactivés,
     renommés ou retirés** (badge « dossier « … » ») et les NZB importés. Chaque release
     affiche où elle est déjà : ✔ déposée, ⏳ à déposer, – ignorée. Filtre, « Seulement celles
     absentes des indexers choisis », tout cocher / décocher.
   - **💻 NZB de ton PC** : glisse des `.nzb` (n'importe où dans la fenêtre, hors page
     Téléchargements) ; MediaInfo et NFO à côté repris. Nom modifiable, case « Série ».
2. **Sur quels indexers ?** Chaque indexer indique son mode : « API » (dépôt tout de suite) ou
   « à la main » (mis dans « à uploader », page Indexer). Jamais deux fois au même endroit,
   sauf « Renvoyer aussi là où elle est déjà déposée ».
3. **Ce qui part**, release par release : résumé du NZB (fichiers, articles, taille, groupes),
   rapport MediaInfo, NFO, BDInfo, chacun **remplaçable ou à joindre** ; puis, pour chaque
   indexer, **la catégorie qui sera envoyée**, modifiable (liste des catégories de
   l'indexer), « ↺ auto » pour revenir au calcul, et ce qui sera ignoré (déjà là).
4. **Déposer** : le nombre d'envois par API, à la main et ignorés, puis un bouton.
**Suivi des envois** : chaque envoi (release → indexer) avec son état, mis à jour d'après les
dossiers. **Nouvel indexer** : un bandeau propose d'un clic toutes les releases déjà postées.
Raccourci : « 🔁 Déposer sur d'autres indexers » dans le détail d'une release de l'Historique.

## Téléchargements (SABnzbd, NZBGet) et repost

Page « 📥 Téléchargements » : ton client Usenet par son API (rien n'est installé ni modifié
dans le client). **SABnzbd** (adresse + clé API) ou **NZBGet** (adresse + identifiant + mot de
passe) ; clé et mot de passe passent par un fichier de configuration curl temporaire, jamais
par la ligne de commande ni dans les journaux. Le chemin complet est affiché en haut :
① Trouvé (sur un indexer, jamais reposté là) → ② Téléchargé → ③ Traité (nom, MediaInfo, NFO,
catégorie) → ④ Posté sur Usenet par NgPost → ⑤ Déposé sur tes autres indexers.
- **État du client** : débit et sa courbe, file, reste et fin estimée, volume sur 24 h et
  7 jours, place libre, **limite de débit** (appliquer / sans limite), pause / reprise.
- **Nouveau repost** : glisse des `.nzb` **sur cette page** ou colle l'adresse d'un NZB. Tout
  se règle **avant** le téléchargement, en étapes numérotées :
  1. nom de la release ; 2. **trouvé sur** (un de tes indexers, ou « ailleurs ») : **jamais
  reposté là** ; 3. **reposter sur**, avec **la catégorie de chaque indexer** (modifiable) et
  son mode (API ou à la main) ; 4. **traitement** : NFO (celui du téléchargement sinon créé
  depuis MediaInfo / BDInfo, celui du téléchargement seulement, ou aucun), joindre ou non le
  MediaInfo, téléchargement **déplacé ou copié** ensuite ; 5. **quand** : tout seul à la fin
  ou sur « Reposter ». Une phrase résume tout le chemin avant de lancer.
- **Reposts** : chaque NZB suivi sur les 5 étapes (pourcentage du téléchargement, traitement,
  post, dépôt par indexer avec ce qui reste). Échec : raison affichée et « ↻ Réessayer ».
- **File d'attente** (pause, retrait ; « puis repost sur … ») et **historique du client**
  (filtre sur la catégorie, « ⟳ Réessayer » un échec, retirer de l'historique,
  « ↻ Reposter… » un téléchargement déjà fait : mêmes questions).
- **Client sur un NAS ou dans Docker** : « Dossiers du client vus d'ici », ex.
  `/data/complete=\\NAS\usenet\complete` (plusieurs séparées par `;`).
- **Tableau de bord** : pendant un téléchargement, carte « Téléchargement en cours » avec les
  mêmes informations qu'un upload : barre et temps restant, débit actuel et moyen, téléchargé
  sur total, écoulé, file restante, ce qui suit (repost vers… ou aucun), place libre, courbe du
  débit en direct (1 min → tout, glisser pour remonter le temps). Aussi en haut de la page
  Téléchargements. Puis la carte de synthèse (volume sur 24 h et 7 jours, terminés et
  échecs, reposts en cours et faits).
- **Pourquoi rien ne bouge** : quand un téléchargement est en pause ou ne progresse pas, la
  place de la courbe explique pourquoi, d'après le client : marqué DOUBLON, ajouté en pause
  (priorité « Pause » de sa catégorie), mot de passe, trop gros, propagation, client en
  pause (minuterie, quota, manque de place, bouton pause), et propose « Reprendre tout le
  client », « ⚡ Forcer ce téléchargement » (démarre même un doublon) et « Ouvrir SABnzbd ».
  Si une reprise demandée ne prend pas, une fenêtre l'explique au bout de quelques secondes.
  « ▶ Reprendre » sur un téléchargement reprend aussi le client s'il est en pause.
- **Toujours visible** : carte « Téléchargement » dans la barre de gauche, bandeau en haut
  des autres pages, et sur grand écran (1640 px et plus) un panneau **Activité** à droite
  (téléchargement et sa raison de pause, reposts et leur étape, dossier surveillé, post,
  surveillance de l'inbox, derniers événements).
- **Fin de téléchargement** : bulle dans la zone de notification (fenêtre cachée) et message
  dans l'application, suivi ou non d'un repost. Un téléchargement n'est considéré terminé
  qu'après le post-traitement du client (vérification, réparation, extraction) : avant,
  un repost aurait pu être marqué « en échec » à tort.
- **.nzb ouvert d'un double clic** : message « reçu → envoyé à SABnzbd » et page
  Téléchargements (bulle si la fenêtre est cachée). L'application vérifie au lancement quel
  programme Windows lance vraiment pour un .nzb ; si ce n'est pas elle (SABnzbd par
  exemple, qui ajoute alors le NZB lui-même, sans repost), une fenêtre le dit et propose
  « Choisir dans Windows… ».
- **Arrêter un téléchargement** : « ■ Arrêter » (deux clics : le second confirme) sur la carte
  en cours, dans la file et dans les reposts : retiré du client (fichiers reçus compris), son
  repost est annulé. « ⏸ Pause » / « ▶ Reprendre » pour lui seul ; la pause de tout le
  client reste en haut de la page. Un téléchargement retiré directement dans le client n'est
  plus suivi (signalé au bout de 3 min).
- **Fenêtre fermée (zone de notification)** : l'infobulle de l'icône montre le téléchargement
  en cours (release, %, débit, temps restant, repost prévu, file) et les reposts en
  traitement ; le menu propose « Mettre les téléchargements en pause » / « Reprendre » ;
  bulles à chaque étape : téléchargement lancé, terminé (puis traitement et repost), en
  échec, repost préparé ou en échec, NZB à régler.
- **Réglages → Téléchargements** : le client, et le **repost par défaut** utilisé quand rien
  n'est demandé (double clic, clic droit, dossier surveillé) :
  - où les NZB que tu ouvres ont été trouvés : « me demander » (conseillé), « ailleurs », ou
    un indexer (jamais reposté là) ;
  - reposter sur : tous mes autres indexers (y compris ceux ajoutés plus tard), ou seulement
    ceux cochés ;
  - traitement : NFO (celui du téléchargement sinon créé, celui du téléchargement seulement,
    aucun), MediaInfo joint ou non ; déplacé ou copié ensuite, reposté tout seul ou non.
- **Dossier de NZB surveillé** (même carte) : par défaut, un sous-dossier par indexer actif,
  plus `_Ailleurs` et `_Télécharger seulement`, créés tout seuls (avec un « À LIRE.txt ») ;
  chaque emplacement a une règle modifiable, et tu peux ajouter tes sous-dossiers (voir
  Nouveautés de la 3.24).
  - `altHUB\Film.nzb` : trouvé sur altHUB → téléchargé, traité, posté, déposé sur tes
    **autres** indexers ;
  - `_Ailleurs\` : reposté sur tous ; `_Télécharger seulement\` : pas de repost ;
  - à la racine : comme un double clic sur un .nzb (réglage ci-dessous), ou sa propre règle.
  Un .nzb est pris quand il a fini d'être écrit (taille stable depuis le délai réglé, 3 s par
  défaut), relu au rythme réglé (3 s par défaut), application réduite comprise ; ensuite rangé dans `_Pris en charge\` (ou supprimé, au
  choix), ou dans `_Erreurs\` avec un `.erreur.txt` qui dit pourquoi. **Doublons refusés**
  (réglable) : même release déjà en téléchargement, téléchargée (historique du client) ou
  repostée. Client injoignable : les .nzb restent dans le dossier et sont repris quand il
  répond.
  - **Page Téléchargements, carte « Dossier de NZB surveillé »** : état (surveillé, arrêté,
    dossier introuvable, client injoignable), dernier passage, ce que devient chaque
    sous-dossier (avec les indexers visés), chiffres des 7 jours, .nzb en cours d'arrivée,
    et le **parcours de chaque NZB pris** : ① trouvé ② téléchargé (%) ③ traité ④ posté
    ⑤ déposé, ou « téléchargé seulement » avec son état chez le client, ou la raison d'un
    refus avec « ↻ Réessayer » (le .nzb retourne dans son sous-dossier). Arrêter / reprendre
    la surveillance, ouvrir le dossier, réglages : d'un clic. Journal gardé
    (`nzbff-nzb-surveilles.tsv`, 500 lignes).
  - **Tableau de bord** : une ligne (surveillé ou non, NZB pris sur 24 h, refus, dernier).

## Intégration à Windows (clic droit, ouverture des .nzb)

Réglages → Intégration à Windows. Tout est écrit dans ton profil Windows (HKEY_CURRENT_USER),
sans droit administrateur ; décocher retire tout. L'application déplacée dans un autre
dossier : menus et .nzb repointés tout seuls au lancement suivant.
- **Menu du clic droit « NZB FF Tool »** :
  - fichiers et dossiers : **Traiter automatiquement** (réglages par défaut de la
    surveillance : action, indexers, MediaInfo, NFO, post, dépôt ; un élément à la fois)
    ou **Ouvrir dans NZB FF Tool** (la bonne page, pour choisir les réglages) ;
  - .nzb : **Télécharger et reposter** (origine par défaut, sinon demandée), **Télécharger
    seulement**, **Déposer sur mes indexers** (sans télécharger), **Ouvrir**.
  Plusieurs fichiers sélectionnés : tous transmis. Windows 11 : ces menus sont dans
  « Afficher plus d'options » (Maj + F10) ; le nouveau menu de Windows 11 n'est ouvert
  qu'aux applications empaquetées et signées.
- **Ouvrir les .nzb avec NZB FF Tool** (double clic). Ce que fait le double clic : ouvrir la
  page Téléchargements pour choisir, télécharger et reposter tout seul, télécharger
  seulement, ou déposer sur les indexers. Si tu as déjà coché « Toujours utiliser cette
  application » pour SABnzbd ou un autre programme, Windows garde ce choix (aucune
  application ne peut le changer à ta place) : le réglage le signale et « Choisir dans
  Windows… » ouvre la fenêtre « Ouvrir avec » où cocher NZB FF Tool et « Toujours ».
- L'application déjà ouverte reçoit la demande (sans se montrer si rien n'est à choisir) ;
  fermée, elle démarre (réduite si rien n'est à choisir). Ligne de commande :
  `"NZB FF Tool Pro.exe" --envoyer <auto|ouvrir|nzb|reposter|telecharger|deposer> <chemin>…`


## Que faire de chaque vidéo

Un **dossier qui ne contient qu'un film** (vidéo + NFO, Sample, Proof…), glissé ou choisi avec
« 📁 Dossier… », s'ouvre dans Traitement comme ce film (NFO repris) ; plusieurs vidéos → lot,
série → page Série, disque ou autre contenu → Autre contenu.

Avant chaque traitement (Traitement, Lot) on choisit :
- **Renommer seulement** : nom de release + rapport MediaInfo, rien n'est posté ;
- **Renommer et poster** : + post Usenet avec NgPost, NZB + MediaInfo rangés pour les
  indexers cochés ;
- **Tout automatique** : + dès que tout est prêt, la page d'upload de chaque indexer coché
  s'ouvre dans le navigateur, avec le dossier du NZB, et le nom de release est copié.

Et **pour quels indexers**. Les autres reçoivent la release dans `_Ignorés` (un clic la
remet « à déposer »). La surveillance de l'inbox se règle entièrement à l'avance dans Réglages →
« Surveillance automatique » (action, indexers par défaut, devenir de la vidéo postée,
MediaInfo, démarrage, rythme), avec le résumé de ce qui arrivera à chaque vidéo.
Les indexers par défaut se cochent aussi dans Réglages → Indexers (colonne « Par défaut »).

## Post avec NgPost

1. La vidéo, déjà renommée, arrive seule dans le dossier de sortie.
2. L'application lance NgPost en ligne de commande, une vidéo à la fois, avec
   `-c ngPost.conf` (serveurs, comptes, chemins de 7-Zip / par2) et les options de la carte
   Réglages → « Options du post », reprises de ngPost.conf au premier lancement
   (`PACK = COMPRESS, GEN_NAME, GEN_PAR2` → compression, nom aléatoire, par2 ;
   `obfuscate = article` → `-x`). La commande exacte est affichée sous les options.
3. Progression par étape (Compression, Par2, Envoi) dans la barre latérale et la page
   Indexer : débit actuel et moyen, articles envoyés, volume envoyé, temps écoulé et
   restant. Journal de NgPost à l'écran (lignes de connexion masquées) et complet
   dans `logs\`.
4. Le NZB est écrit dans un dossier temporaire et n'est rangé que s'il est complet
   (`</nzb>`, fichiers, articles) ; vérification désactivable dans « Options du post ».
   Un NZB complet vaut post réussi, même si NgPost quitte avec un code d'erreur
   (simple avertissement). Un NZB produit n'est jamais supprimé : s'il est refusé,
   il est gardé dans `logs\\`.
5. Le NZB rejoint le rapport MediaInfo dans `_En attente du NZB\<release>\`, puis la paire
   est copiée chez les indexers choisis ; la vidéo est rangée dans `_Postés`.
6. Page « Indexer », pour chaque indexer d'une release : « 🌐 Uploader » ouvre sa page
   d'upload et le dossier du NZB ; « ✔ Uploadé » range la release dans `_Déjà uploadés`
   une fois déposée ; ✖ la range dans `_Ignorés` ; ↩ la remet dans `_À uploader`.
   Chaque changement peut être annulé, et le clic qui sert à revenir dans la fenêtre
   (après le navigateur) ne change jamais rien.

Une fois postée (NZB vérifié), la vidéo du dossier de sortie part à la corbeille par
défaut ; au choix : suppression définitive, rangement dans `_Postés`, ou rien.
Un original situé ailleurs (NAS…) n'est jamais touché.

Statistiques (Tableau de bord et page Indexer) sur 24 h, 7 jours ou 30 jours : volume posté,
débit moyen, pointe, connexions, temps de post (`nzbff-posts.tsv`, `nzbff-speed.tsv`).
Le **débit moyen est celui de l'envoi**, comme NgPost l'annonce : volume ÷ temps d'envoi, sans
la compression RAR, le par2 ni la vérification du NZB (avant la 3.14, il divisait par la durée
totale du post et sortait deux fois trop bas). Les posts déjà enregistrés sont recalculés
(ceux dont NgPost avait annoncé le débit).

ngPost.exe est cherché tout seul au démarrage, sous-dossiers compris
(ex. `C:\ngPost\v4.16_x64\ngPost.exe`).

### Fichiers de réparation : par2 de NgPost, par2cmdline-turbo ou ParPar

Réglages → Options du post → Fichiers de réparation → **Outil** :
- **par2 de NgPost** : le par2.exe livré avec NgPost (par2cmdline), ou celui de ngPost.conf.
- **par2cmdline-turbo** : par2cmdline avec le moteur de calcul de ParPar, mêmes commandes
  (remplacement direct). NgPost le reçoit par `--par2_path`, sans autre réglage ni copie de
  ngPost.conf. Garde-le dans son propre dossier (ex. `C:\Program Files\par2cmdline-turbo\par2.exe`,
  ou `par2-turbo.exe` à côté de ngPost.exe) sans remplacer le par2.exe de NgPost.
- **ParPar** : crée seulement (ne vérifie ni ne répare). NgPost ne le reconnaît que si son
  chemin contient « parpar », et ses arguments ne passent que par ngPost.conf : une copie de
  ngPost.conf est écrite dans le dossier temporaire du post (supprimée ensuite) avec
  `PAR2_ARGS = -s5M -r<redondance>% -m2048M -p1l --progress stdout -q` (arguments conseillés
  par NgPost, redondance réglée ici).
- L'application reconnaît ce qu'est vraiment chaque exécutable (son nom et ce qu'il répond à
  `--version`) : ParPar mis dans le réglage de turbo (ou l'inverse) est signalé, avec
  « Le régler comme … », et utilisé comme ce qu'il est (au post comme à la comparaison) ;
  le par2cmdline classique mis à la place de turbo est signalé (aucun gain).
- **Conseil** : **par2cmdline-turbo**. Même moteur que ParPar, remplacement direct du par2
  de NgPost (aucune copie de ngPost.conf), sait aussi vérifier et réparer ; mesure à faire
  avec « Comparer sur ce PC ».
- Chemin vide : cherché (dossier de NgPost, Program Files, dossier des programmes, PATH).
  Introuvable : le par2 de NgPost est utilisé, et c'est écrit dans le journal du post.
- **⏱ Comparer sur ce PC** : chronomètre chaque outil trouvé sur le même fichier de 512 Mo
  (10 % de redondance, blocs de 1 Mo, même mémoire) dans le dossier temporaire du post, puis
  supprime tout ; « Utiliser … » choisit le plus rapide. Turbo et ParPar partagent le même
  moteur : les écarts entre eux sont en général faibles, l'écart avec par2cmdline classique
  dépend du processeur.

### Groupes Usenet

Réglages → Options du post : groupes par défaut, **groupes des films** et **groupes des séries** ;
Réglages → Catégories de contenu : groupes de chaque catégorie. Au post, les groupes suivent le
type de la release (le journal du post l'indique). Des groupes courants sont proposés pour chaque
type (un clic les ajoute) : a.b.movies, a.b.teevee, a.b.games, a.b.sounds.lossless, a.b.e-book…

**Répartition** quand plusieurs groupes sont donnés (`GROUP_POLICY` de NgPost) :
- celle de ngPost.conf (inchangée) ;
- tous les groupes à chaque post (`ALL`) ;
- un groupe au hasard par post : tiré par l'application, écrit dans le journal du post ;
- un groupe au hasard par fichier (`EACH_FILE`) ;
- automatique : 1 ou 2 groupes → tous ; 3 ou plus → un groupe par fichier.

ngPost.conf n'a qu'une liste `GROUPS` : les groupes des films, des séries et des catégories ne
peuvent pas y être rangés. L'application les donne à NgPost à chaque post (`-g`). « Écrire dans
ngPost.conf » y range les groupes par défaut (`GROUPS`) et la répartition (`GROUP_POLICY`).

### Serveurs Usenet

Réglages → « Serveurs Usenet » : fournisseurs et serveurs, comme les blocs `[server]` de
ngPost.conf (serveur, port, SSL, identifiant, mot de passe, connexions, vérification des
articles `nzbCheck`). Plusieurs serveurs par fournisseur (régions, ports 563 / 443 / 119…).
- « Importer depuis ngPost.conf » reprend les serveurs existants (fournisseur déduit du nom).
- « Utiliser les serveurs configurés ici » : au moment du post, NgPost reçoit une **copie** de
  ngPost.conf (mêmes options) dont les serveurs sont remplacés par ceux-ci ; ngPost.conf n'est
  jamais modifié, la copie est effacée à la fin du post. Mode « ligne de commande » seulement.
- « ⏱ Tester » / « Tester tous les serveurs » : **test complet** de chaque serveur — connexion,
  chiffrement TLS, **identifiants** (refusés : signalé en rouge), temps de réponse à 8 commandes,
  puis 5 s de lecture d'articles (`alt.binaries.boneless`) pour le débit. ★ = identifiants acceptés
  et meilleur temps de réponse. Sous Windows, le SSL passe par PowerShell (fourni avec Windows,
  rien à installer) ; mot de passe par l'environnement, jamais sur une ligne de commande.
- « 💾 Écrire dans ngPost.conf » remplace les serveurs de ngPost.conf par ceux d'ici (autres
  options gardées, copie de l'ancien fichier `ngPost.conf.nzbff-date.bak`) ; « Garder ngPost.conf
  à jour » le refait à chaque modification. Ferme la fenêtre de NgPost avant (elle réécrit son
  fichier en quittant). Les serveurs sont écrits **à la place des anciens** (après le titre
  « servers section » de NgPost), sous un seul titre. Un fichier abîmé par les versions
  précédentes (titres « Serveurs Usenet (écrits par NZB FF Tool Pro) » et « servers section »
  répétés, lignes vides en série) est réparé à la première écriture ; réécrire ne change plus
  rien.
- « contrôle après post » = option `nzbCheck` de NgPost : ce serveur sert à relire les articles
  après l'envoi (pas à poster). À ne pas confondre avec « Vérifier le NZB avant de le ranger »
  de l'application, qui contrôle seulement le fichier NZB.
- **Deux nombres de connexions par serveur** : « connexions » (simultané, quand il sert avec
  les autres) et « seul » (quand il est choisi comme le plus rapide). Ex. 3 serveurs d'un même
  fournisseur limité à 100 connexions : 33 / 33 / 34 en simultané, 100 seul.
- « Choisir automatiquement le serveur le plus rapide » : avant chaque post, tous les serveurs
  activés sont testés et seul le plus rapide est utilisé, avec ses connexions « seul » (le
  serveur de vérification reste). Désactivé : tous servent ensemble avec leurs connexions
  « simultané ».
- **Journal du post** : comment les serveurs sont choisis, chaque test (temps de réponse,
  débit, identifiants), le serveur retenu, puis le détail de chaque serveur utilisé —
  fournisseur, serveur:port, SSL, connexions, identifiant (jamais le mot de passe), vérification
  des articles — et le total des connexions du post. Serveurs non gérés ici : ceux de
  ngPost.conf sont détaillés de la même façon. Encore plus de détail : voir « Journaux ».

### Jamais de dépôt sans MediaInfo

Un film ou une série n'est jamais déposé sur un indexer sans son rapport MediaInfo :
- le rapport suit toujours le NZB, même s'il est rangé « avec la vidéo » ou dans un dossier dédié
  (une copie va dans le dossier d'attente de la release) ;
- s'il manque au moment de la distribution, il est refait depuis la vidéo postée (dossier de
  sortie, « _Postés », dossiers de releases déplacées sur un autre disque) ;
- s'il reste introuvable, le dépôt par API est **bloqué** (message dans le journal) et la page
  Indexer affiche « MediaInfo manquant : dépôt bloqué » avec un bouton « 🛠 Générer le MediaInfo » ;
  « 🌐 Uploader » refuse aussi tant que le rapport manque.
Les autres contenus (jeux, logiciels…) n'ont de rapport que si leur catégorie le demande.

**Jamais deux fois sur l'indexer** :
- un seul envoi à la fois par release et par indexer, même avec deux fenêtres ouvertes ou le
  mode `--watch` en même temps (verrou dans le dossier de la release), et une seule
  distribution à la fois ;
- nouvel essai automatique seulement quand rien n'a pu arriver à l'indexer (connexion
  impossible, 429 trop de requêtes, 503) ;
- réponse perdue ou erreur pendant le traitement (délai dépassé, coupure, 500, 502, 504,
  520–524 — fréquent avec un gros NZB) : **pas de renvoi**. Une recherche sur l'indexer
  (Newznab) vérifie si la release est arrivée : oui → « Déjà uploadés » ; non ou impossible à
  vérifier → message dans le journal, à toi de regarder puis « ⇪ Déposer (API) » ou « ✔ Uploadé ».
- La clé API ne passe jamais sur la ligne de commande de curl (fichier `-K` temporaire).

**Voir ce qui est envoyé** :
- Réglages → Indexers (dépôt par API) : « Requête envoyée », la requête réelle (clé masquée)
  pour un film et une série d'exemple — adresse, paramètres et chaque champ du formulaire, avec
  la catégorie.
- Chaque dépôt est détaillé dans le **Journal** et dans `logs\api-AAAA-MM-JJ.log` : requête,
  code HTTP, réponse complète de l'indexer, et **catégorie qu'il a attribuée** (avertissement si
  elle diffère de celle envoyée — souvent : numéros de catégorie propres à cet indexer, à
  vérifier avec « Tester la connexion »).
- **Doublon** (`dupecheck`) : si l'indexer a déjà la release, il refuse le nouveau dépôt et garde
  la sienne, **catégorie comprise** — signalé en avertissement. Pour la recatégoriser : la
  supprimer sur le site, puis « ↩ » et « ⇪ Déposer (API) ».
- « Aucune » catégorie : avec un NZB obfusqué, l'indexer ne voit que le nom et peut se tromper ;
  « Selon la résolution (films / séries) » est conseillé.
- La catégorie part en paramètre de l'adresse **et** en champ du formulaire (certains indexers ne
  lisent que l'un des deux).

**Film ou série ?** Une release préparée depuis la page Série (ou un dossier de série de
l'inbox) est marquée « série » jusqu'à l'indexer : catégorie et groupes des séries, quel que soit
son nom. Sinon, le nom décide : `S01`, `S01E02`, `S01-S05`, `1x02`, `Saison.1`, `Season 2`, code
collé au titre (`TitreS01`) ; en dernier recours, le rapport MediaInfo joint (nom de l'épisode
analysé, `…S01E01…`). Une catégorie précisée par l'indexer dans la même famille (ex. 5040 → 5340
« TV - FR / HD ») est normale et n'est pas signalée comme une erreur.

**Catégorie envoyée** (Réglages → Indexers) : « Aucune » (rien n'est envoyé, l'indexer classe
lui-même), « l'indexer la détecte » (REST, -1), « selon la résolution » (films 2030/2040/2045,
séries 5030/5040/5045), ou « fixe » avec un numéro pour les films **et un pour les séries** (une
série ne part jamais dans la catégorie des films). La page Indexer affiche la catégorie qui sera
envoyée pour chaque release et chaque indexer, avant le dépôt.

**Envoyer quand même** (anti-doublon) : le nouveau post remplace aussi la release déjà uploadée
chez l'indexer. Un nouveau post gardé à part peut être déposé plus tard d'un clic (« ⇪ Déposer le
nouveau post » sur la page Indexer).

**Release repostée sous le même nom** (test refait, saison renvoyée…) : c'est toujours le
nouveau post (NZB + rapport) qui est déposé. Une ancienne copie restée chez un indexer est mise
de côté dans `<Indexer>\_Remplacés\<release> (ancien, date)`. Si la release était déjà
uploadée sur cet indexer, pas de second dépôt : le nouveau NZB est gardé dans
`_Remplacés\<release> (nouveau post, date)`. Le journal le signale.

## Dépôt direct sur l'indexer (API)

Réglages → Indexers → « ⚙ Dépôt » sur chaque indexer :
- **À la main** : 🌐 Uploader ouvre la page d'upload et le dossier du NZB ;
- **API Newznab** (`t=nzbadd`, la plupart des indexers) : adresse de l'API, clé API,
  catégorie (voir « Catégories » ci-dessous), champs `nzbname`, `mediainfo` (XML), `nfo` (MediaInfo en
  texte, créé au traitement), `includemeta`, `dupecheck`, paramètres et champs en plus ;
- **API REST** (`POST <adresse de base>/nzb`, multipart : `file`, `mediainfo`, `nfo`, `cat`,
  `nzbname`) : adresse et clé propres (gardées à part de celles de Newznab ; clé vide = celle
  de Newznab), chemin d'envoi réglable, clé transmise par en-tête nommé (ex. `X-API-Key`),
  `Authorization: Bearer` ou paramètre d'adresse ; catégorie « l'indexer la détecte »
  (`cat=-1`), selon la résolution, ou fixe ; « Tester la connexion » vérifie l'adresse et la
  clé sans rien déposer ;
- **Commande personnalisée** pour toute autre API, avec {nzb} {mediainfo} {nfo}
  {release} {apikey} {cat} {api}.

Réponses comprises : succès (XML Newznab `<success>`, JSON `release` / `success` / HTTP 2xx),
doublon (Newznab 601, HTTP 409 : compté comme déposé), clé refusée (401), droits
insuffisants (403), requête invalide (400, 405, 422) avec le message de l'indexer.
Erreurs passagères — trop de requêtes (429, délai `retry_after` respecté), serveur en
erreur ou indisponible (500, 502, 503, 504), coupure réseau — : nouvel essai automatique,
5 fois au plus.

**Détail de chaque dépôt** sur les pages Traitement, Lot, Série et Autre contenu (carte
« Dépôts sur les indexers »), sur la page Indexer, dans le Journal, `logs\api-date.log` et
l'e-mail :
- **ce qui est déposé** : NZB (et ce qu'il décrit : nombre de fichiers postés, poids),
  rapport MediaInfo (joint ou pourquoi pas), NFO (d'origine, ou créé depuis BDInfo /
  MediaInfo ; joint ou pourquoi pas), rapport BDInfo ;
- **contenu posté** de la release : chaque fichier et sa taille (ISO, NFO, Sample, Proof…) ;
- requête complète (clé masquée), code HTTP et sa signification, réponse de l'indexer ligne
  par ligne, catégorie attribuée, durée. Bouton « Copier ».

L'étape « ⇪ Déposer directement sur … (API) » se règle dans la surveillance automatique,
et à chaque traitement (Traitement, Lot). Dès que le NZB est prêt, il est envoyé avec le
rapport MediaInfo ; en cas de succès la release passe dans `_Déjà uploadés` (réponse de
l'indexer gardée dans `.nzbff-api`), sinon elle reste dans `_À uploader` avec l'erreur.
Bouton « ⇪ Déposer (API) » sur la page Indexer pour un envoi à la demande.
Les requêtes passent par curl (fourni avec Windows 10 et 11) ; la clé n'apparaît
jamais dans le journal. En ligne de commande : `--no-api` pour ne pas déposer.

Désactive « Surveiller dossier » dans NgPost pour éviter un double post.
Renommer un indexer renomme son dossier ; le retirer garde son dossier.

### Catégories

Trois modes par indexer : **Aucune** (l'indexer classe lui-même), **Selon la résolution**, ou
**Fixe** (une valeur pour les films, une pour les séries).

Selon la résolution : tableau Films / Séries × SD / 720p / 1080p / UHD, chaque case au choix
dans la liste de l'indexer (grandes familles en tête, puis les sous-catégories). Avec
« **Catégories FR pour les releases en français** », deux lignes de plus : Films FR / Séries FR
(FRENCH, TRUEFRENCH, VFF, VFQ, MULTi…). Exemple avec la liste d'un indexer :

| | SD | 720p | 1080p | UHD |
|---|---|---|---|---|
| Séries | 5030 | 5041 | 5042 | 5045 |
| Séries FR | 5330 | 5340 | 5342 | 5345 |
| Films | 2030 | 2041 | 2042 | 2045 |
| Films FR | 2330 | 2340 | 2342 | 2345 |

- **Import : tout est rempli.** « 📄 Importer une liste de catégories… » (fichier XML
  `<categories>` / `<caps>`, ou texte « 5042 TV > 1080p » une par ligne), le premier « Tester la
  connexion » qui renvoie la liste, ou « ⬇ Tout remplir depuis la liste de l'indexer » rangent
  chaque catégorie à sa place : films et séries par résolution (« TV > 1080p », « Movies - FR >
  720p »…), et **chaque autre contenu** : XXX, jeux PC (PC > Games), consoles, logiciels PC
  (PC > 0day) et Mac, musique (Audio), ebooks et BD (Books > Ebook / Comics, et Books - FR en
  français), animation (TV > Anime, TV - FR > Anime), éducation (PC > Tutorials), autre (Other >
  Misc), Blu-ray complet (Movies > BluRay, Movies - FR > BluRay), DVD complet (Movies > DVD, sinon
  SD). Les catégories FR sont activées si l'indexer en a. Une case que la liste ne couvre pas
  garde ton choix. La liste complète est gardée et visible (« Voir les N catégories »).
- **Autres contenus par indexer** : tableau sous celui des films et séries (catégorie, et
  catégorie en français), quel que soit le mode. Une liste déjà importée avec une version
  précédente remplit ces cases une fois, à la première ouverture de la 3.14 (cases déjà
  choisies gardées).
- Sans tableau rempli, une série 1080p part en 5040 (TV HD, catégorie générale) : c'est ce
  qu'envoyaient les versions précédentes. Remplis la case 1080p (5042) pour l'envoyer directement
  en sous-catégorie.
- Réponse de l'indexer : la catégorie qu'il a réellement attribuée est lue (`categoryID`) et
  écrite dans le Journal, `logs\api-date.log` et l'e-mail (« envoyée en 5042, classée par
  l'indexer en 5340 (TV - FR > 720p) »). Une autre famille (film ↔ série) est signalée ⚠.

## Fenêtre et zone de notification

Réglages → « Fenêtre et zone de notification » : en fermant la fenêtre, l'application
demande, se réduit à côté de l'horloge, ou quitte (confirmation si un post ou un dépôt est
en cours). Réduite, tout continue : surveillance de l'inbox, posts, dépôts sur les
indexers, statistiques ; des bulles signalent les posts terminés, les dépôts acceptés ou
refusés et les quarantaines. Clic sur l'icône : rouvrir ; clic droit : état, démarrer ou
arrêter la surveillance, quitter. Options : démarrer réduit, lancer avec Windows
(clé « Exécuter » du compte, lancement avec `--tray`).

Pendant une copie, l'infobulle de l'icône affiche le pourcentage, le volume et le temps restant.
Dans la barre de gauche, un clic sur la copie ou le post en cours ouvre la page qui le suit.

**Une seule instance** : relancer l'application alors qu'elle tourne déjà (même réduite)
rouvre simplement sa fenêtre ; il n'y a jamais deux icônes près de l'horloge.

## Apparence

Réglages → Apparence : 11 thèmes, appliqués d'un clic, avec un aperçu de chacun :
- sombres : **Nuit** (bleuté, indigo), **Graphite** (gris neutre, orange, angles vifs),
  **Minuit** (bleu marine, cyan), **Forêt** (vert sombre, menthe), **Prune** (violet, rose,
  très arrondi), **Solaire** (pétrole et sable) ;
- écran : **Terminal** (phosphore vert) et **Ambre**, en police à chasse fixe, angles droits ;
- clairs : **Papier** (crème, terre cuite) et **Clair** (blanc, bleu), avec ombres douces ;
- **Contraste** : noir, blanc et jaune, bordures épaisses, lisibilité maximale.

Chaque thème a sa couleur d'accent (pastille « Thème ») ; on peut en choisir une autre.
Les réglages s'enregistrent tout seuls : « ✔ Enregistré » s'affiche un instant après
chaque modification, puis s'efface.

## Suivi en direct, arrêt, place sur les disques

- En haut de chaque page : copie (pourcentage, débit, temps restant), post (étape, %, débit,
  temps restant), dépôts API, avec un bouton **⏹ Arrêter**. Résumé dans la barre de gauche.
- Arrêter une copie : fichiers partiels retirés, originaux laissés (ou remis) en place, rien
  en quarantaine. Arrêter un post : NgPost et ses compresseurs (rar, 7z, par2) arrêtés, archives
  temporaires de ce post retirées, la release reste dans « À poster ».
- Pages Série et Traitement : **destination au choix** (dossier de sortie par défaut, sur place,
  autre dossier / autre disque) et place affichée avant de lancer : copie vers la destination
  (ou déplacement instantané sur le même disque), archives + par2 de NgPost dans son dossier
  temporaire (bouton « 💽 Autre disque… »).
- Avant de lancer NgPost, la place est vérifiée : s'il en manque, NgPost n'est pas lancé et des
  solutions s'affichent en haut de la page :
  1. **déplacer la release sur un autre disque** (place libre de chaque disque affichée ;
     dossier `NZB FF Tool Pro - releases` à sa racine, archives sur ce disque si besoin) puis
     poster avec compression — conseillé ;
  2. garder la release et mettre seulement les archives sur un autre disque ;
  3. série : compresser en plusieurs lots qui tiennent dans la place libre, **un seul NZB**
     fusionné à la fin, même mot de passe ; chaque archive garde `Série/Saison N/…` (option
     « Place insuffisante : compresser une série en plusieurs lots » pour le faire sans demander) ;
  4. série : saison par saison sans compression (un NZB par saison, chacune dans son dossier) ;
  5. sans compression (un NZB ; une série arrive alors à plat : Usenet ne transmet pas de
     dossiers hors archive) ;
  6. réessayer après avoir fait de la place.
- Chaque étape (compression, par2, envoi) affiche son pourcentage et son temps restant, et
  les étapes ne reviennent jamais en arrière (Compression → Par2 → Envoi → Finalisation).
  Compression avec 7-Zip (qui n'affiche rien) : avancement mesuré sur l'archive qui grossit dans
  le dossier temporaire. Par2 : calcul (90 %) puis écriture des données de réparation (10 %).
- Journal de chaque post (`logs\`) : chaque ligne porte l'heure à la milliseconde, et
  l'avancement est noté au moins une fois par minute avec le débit et le temps restant.
- Courbe du débit en direct : 1 min, 5 min, 15 min, 1 h ou tout le post ; glisser la courbe
  pour remonter le temps, Ctrl + molette pour zoomer, « ⏵ Direct » pour revenir au temps réel.
- Fenêtre fermée : survoler l'icône près de l'horloge affiche l'étape, le pourcentage, le débit
  et le temps restant.
- Tailles en unités de 1024, comme l'Explorateur de Windows et les indexers (« 144 Go » ici =
  « 144 Go » dans l'Explorateur). Taille postée = vidéo (ou archives) + par2 (environ 10 %).
- La compression suit `PACK` de ngPost.conf (reprise au premier lancement, bouton « Reprendre
  les options de ngPost.conf »).

## Notification par e-mail

Réglages → « Notification par e-mail » : **un seul** e-mail quand tous les traitements sont
terminés (copies, posts, dépôts sur les indexers, reposts) depuis 45 secondes — il attend aussi
les posts et dépôts encore en file et, avec NgPost en surveillance de dossier, les releases dont
le NZB n'est pas encore là (au plus 6 h) ; jamais d'e-mail vide, jamais deux fois le même
résumé (même avec deux fenêtres ouvertes), une adresse écrite deux fois n'en reçoit qu'un, avec le résumé : releases
préparées, postées (taille, durée, débit), déposées, et les échecs. Fonctionne fenêtre fermée
et en mode `--watch`. Fournisseurs préremplis (Gmail, Proton Mail, Yahoo, iCloud, Orange, Free,
GMX) ou n'importe quel serveur SMTP, SSL (465) ou STARTTLS (587). Envoi par curl (fourni avec
Windows 10 et 11), mot de passe jamais sur la ligne de commande. Gmail, Yahoo, iCloud : il faut un
mot de passe d'application. Bouton « Envoyer un e-mail de test ».

- **Ton fournisseur** : saisis serveur, port et sécurité, puis « Ajouter aux fournisseurs »
  (nom au choix) ; il rejoint la liste (★), × pour le retirer.
- **Objet et texte personnalisables**, avec des champs remplacés à l'envoi : `{bilan}`,
  `{reussis}`, `{echecs}`, `{total}`, `{resume}` (le détail), `{reussites}`, `{erreurs}`,
  `{date}`, `{heure}`, `{machine}`, `{app}`, `{version}` (un clic les insère au curseur).
  Vides = modèles par défaut ; « Aperçu avec un exemple » ; l'e-mail de test utilise ton modèle.
- **Dépôts sur les indexers** : chaque dépôt réussi est écrit clairement (« ✔ Déposée sur
  altHUB : … — envoyée en 5042, classée par l'indexer en 5340 ») ; une erreur d'API est suivie
  de la réponse de l'indexer telle quelle.

## Enregistrement, sauvegarde et restauration

**Recherche** en haut des Réglages, **toujours visible** (elle reste en place quand la page
défile) ; Ctrl+F l'ouvre depuis n'importe quelle page. Tape un mot (« latence », « mot de
passe », « par2 », « 1080p », « groupes »…), seules les cartes qui en parlent restent
affichées ; accents et majuscules ignorés, Échap pour tout réafficher.

**Effacer les historiques** (page Historique, « 🗑 Effacer les historiques… ») : au choix,
l'historique des traitements (c'est aussi la mémoire des doublons), les statistiques des posts,
et les releases rangées chez les indexers (`_Déjà uploadés`, `_Ignorés`, `_Remplacés`, envoyées
à la corbeille). Sauvegarde automatique faite avant, copie `.bak` de chaque fichier vidé.

Les réglages s'enregistrent à chaque modification : une pastille « Réglages enregistrés »
apparaît en bas de la fenêtre, quelle que soit la page. Enregistrement immédiat : disquette 💾
en bas de la barre latérale, bouton « 💾 Enregistrer » des Réglages, ou Ctrl+S.

Réglages → « Sauvegarde et restauration » :
- **Sauvegarde automatique** (activée) de la **configuration complète** (dossiers, NgPost,
  indexers et clés API, serveurs Usenet, e-mail, séries, catégories, thème…), de l'historique et
  des statistiques, copiés dans `%APPDATA%\NZB FF Tool Pro\sauvegarde`, avec la date et l'heure de
  la dernière sauvegarde. **Quand** : au choix, en cochant — à chaque modification des réglages,
  à intervalle régulier (10 min, 30 min, 1 h, 2 h, 3 h, 4 h, 6 h, 8 h, 12 h ou 24 h, fenêtre cachée
  comprise ; prochaine sauvegarde affichée), à la fermeture de l'application (y compris « Quitter »
  depuis l'icône), au démarrage. Nombre de copies datées des réglages gardées (une par jour) :
  réglable. « Sauvegarder maintenant » sauvegarde tout de suite, quels que soient ces choix.
  Une nouvelle version lancée depuis un autre dossier, sans `nzbff.conf` à côté d'elle, reprend
  tout toute seule au démarrage.
- **Exporter** un fichier `.nzbff` (réglages, historique et statistiques, chacun au choix), chiffré
  ou non par mot de passe : ChaCha20-Poly1305, clé tirée du mot de passe par PBKDF2-SHA256
  (600 000 itérations, sel aléatoire). Conseillé : le fichier contient tes clés API.
- **Restaurer** : mot de passe demandé s'il est chiffré, contenu affiché, puis tout est rechargé
  sans redémarrer ; les fichiers remplacés sont gardés en `.avant-restauration`.

Les réglages sont à côté de l'exe lancé (`target\release\` si tu lances `nzbff.exe` depuis là,
`dist\` pour « NZB FF Tool Pro.exe »). Passer d'une version 3.3 ou plus ancienne : copie
`nzbff.conf`, `nzbff-history.tsv`, `nzbff-posts.tsv` et `nzbff-speed.tsv` à côté du nouvel exe
(une seule fois : ensuite la sauvegarde automatique s'en charge). Les nouvelles options prennent leur valeur par défaut.

## Sans interface

    "NZB FF Tool Pro.exe" --tray        (interface réduite dans la zone de notification)
    "NZB FF Tool Pro.exe" --watch
    "NZB FF Tool Pro.exe" --file "film.mkv" [--dest D:\Sortie] [--copy|--move] [--dry-run]
    "NZB FF Tool Pro.exe" --file "D:\Séries\Breaking Bad"      (série : livrée d'un bloc)
    options : --action rename|post|auto   --indexers "A,B"
