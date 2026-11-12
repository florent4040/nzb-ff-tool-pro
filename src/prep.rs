//! Cœur du programme, sans aucune dépendance externe :
//! configuration, lecture MediaInfo, calcul du nom de release, transferts sûrs.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const VIDEO_EXT: &[&str] = &["mkv", "mp4", "avi", "m2ts", "ts", "mov", "wmv"];
pub const SOURCES: &[&str] = &["BluRay", "REMUX", "BDRip", "BRRip", "WEB-DL", "WEBRip", "WEB", "HDTV", "HDLight", "DVDRip", "DVDRemux", "DVD", "COMPLETE.BLURAY", "COMPLETE.UHD.BLURAY", "DVDR"];
pub const LANGS: &[&str] = &["FRENCH", "MULTi", "TRUEFRENCH", "VFF", "VFQ", "VFI", "VOF", "FRENCH.VOF", "MULTi.VFF", "MULTi.VOF", "VOSTFR"];
pub const RESOLUTIONS: &[&str] = &["2160p", "1080p", "720p", "576p", "480p"];
pub const CODECS: &[&str] = &["x264", "x265", "H264", "H265", "AV1", "MPEG2", "VC-1", "XviD"];
/// Pistes audio courantes (repère {audio} du modèle).
pub const AUDIOS: &[&str] = &["AC3.2.0", "AC3.5.1", "EAC3.5.1", "DDP5.1", "DTS.5.1", "DTS-HD.MA.5.1", "DTS-HD.MA.7.1", "TrueHD.7.1.Atmos", "AAC.2.0", "FLAC.2.0", "LPCM.2.0"];
/// Couleurs d'accent ; « Thème » reprend celle du thème choisi.
pub const ACCENTS: &[&str] = &["Thème", "Indigo", "Bleu", "Cyan", "Violet", "Émeraude", "Ambre", "Orange", "Rouge", "Rose", "Ardoise"];
/// Format standard des indexers (ex. Monfilm.1975.FRENCH.1080p.HDTV.x264-UKDHD).
pub const DEFAULT_TEMPLATE: &str = "{titre}.{annee}.{langue}.{resolution}.{source}.{codec}-{tag}";
/// (clé, libellé) des formats d'export MediaInfo.
pub const MI_FORMATS: &[(&str, &str)] = &[
    ("xml-full", "XML complet (recommandé)"),
    ("xml", "XML standard"),
    ("json-full", "JSON complet"),
    ("text-full", "Texte complet"),
    ("text", "Texte standard"),
    ("html", "HTML complet"),
    ("oldxml", "Ancien XML (OLDXML)"),
    ("ebucore", "EBUCore"),
];
/// (clé, libellé) des modes de transfert.
pub const TRANSFERS: &[(&str, &str)] = &[
    ("auto", "Automatique : lecteur réseau → copie, disque local → déplacement"),
    ("copy", "Toujours copier (l'original reste en place)"),
    ("move", "Toujours déplacer"),
];

// ---------------------------------------------------------------- Config

/// Un indexer où déposer les NZB.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IndexerCfg {
    pub name: String,
    /// Page d'upload (ouverte d'un clic), facultative.
    pub url: String,
    pub active: bool,
    /// Dépôt du NZB : "web" (à la main, page d'upload), "newznab" (API t=nzbadd),
    /// "cmd" (commande personnalisée, ex. curl vers une API REST).
    pub mode: String,
    /// Adresse de l'API (ex. https://indexer.example/api) et clé API.
    pub api_url: String,
    pub api_key: String,
    /// Catégorie : "auto" (selon la résolution : SD / HD / UHD) ou un numéro fixe.
    pub cat: String,
    /// Catégories utilisées en automatique (Newznab : 2030 SD, 2040 HD, 2045 UHD).
    pub cat_sd: String,
    pub cat_hd: String,
    pub cat_uhd: String,
    /// Joindre le rapport MediaInfo à l'envoi (champ « mediainfo », XML).
    pub send_mi: bool,
    /// Champs et paramètres Newznab facultatifs.
    pub send_name: bool,
    /// Ancienne case « nfo » (3.21 et avant : créer un NFO pour cet indexer). Lue et gardée
    /// pour la compatibilité, sans effet : la création ne dépend plus que du réglage NFO et du
    /// choix du traitement.
    pub send_nfo: bool,
    /// Joindre à cet indexer un NFO créé par l'application (le NFO d'origine part toujours).
    pub nfo_created: bool,
    pub includemeta: bool,
    pub dupecheck: bool,
    /// Paramètres d'URL en plus (ex. « private=no&foo=1 ») et champs POST en plus (« clé=valeur » par ligne ou séparés par « ; »).
    pub extra_query: String,
    pub extra_fields: String,
    /// Commande personnalisée : {nzb} {mediainfo} {release} {apikey} {cat} {api}.
    pub cmd: String,
    /// API REST : chemin d'envoi (ajouté à l'adresse de base), ex. « /nzb ».
    pub rest_path: String,
    /// Transmission de la clé : "header" (en-tête nommé `auth_name`), "bearer"
    /// (Authorization: Bearer), "query" (paramètre d'URL nommé `auth_name`).
    pub auth: String,
    pub auth_name: String,
    /// API REST : adresse de base et clé propres (souvent différentes de celles de Newznab).
    /// Clé vide : celle de Newznab.
    pub rest_url: String,
    pub rest_key: String,
    /// Catégories des séries en automatique (Newznab : 5030 TV SD, 5040 TV HD, 5045 TV UHD).
    pub cat_tv_sd: String,
    pub cat_tv_hd: String,
    pub cat_tv_uhd: String,
    /// Catégorie propre à cet indexer pour chaque catégorie de contenu : (clé, numéro).
    pub cat_map: Vec<(String, String)>,
    /// Catégories de cet indexer (id, nom), lues par « Tester la connexion » ou importées.
    pub cat_list: Vec<(String, String)>,
    /// Releases en français (FRENCH, TRUEFRENCH, MULTi…) : catégories « FR » de l'indexer.
    pub cat_lang_fr: bool,
    /// Champ de l'API où joindre le rapport BDInfo d'un Blu-ray (vide : il sert de NFO).
    pub bdinfo_field: String,
    /// Profil d'API : recherche avant le post, explorateur, requêtes (voir `apidef`).
    pub api: crate::apidef::Profile,
    /// Noms des champs du formulaire d'envoi (vides : « file », « nfo », « mediainfo »).
    pub f_nzb: String,
    pub f_nfo: String,
    pub f_mi: String,
    /// Groupes Usenet indexés par l'indexer (lus dans ses capacités `t=caps`).
    pub groups: Vec<String>,
    /// Que faire quand le post ne passe par aucun de ces groupes : "" avertir, "add" ajouter un
    /// de ses groupes adapté au contenu, "off" ignorer.
    pub groups_rule: String,
    /// Page d'une release après envoi, ex. « https://indexer.example/account/releases/{guid} ».
    pub release_page: String,
    /// Exigences et remarques comprises dans la documentation de l'indexer (une par ligne).
    pub doc_notes: String,
    /// Groupes Usenet choisis à la main par catégorie de l'indexer : (numéro, « a.b.x,a.b.y »).
    /// Une release envoyée dans cette catégorie est postée aussi dans ces groupes. Un numéro de
    /// famille (« 2000 ») vaut pour toutes ses sous-catégories (2040, 2045…).
    pub group_rules: Vec<(String, String)>,
}

/// Profil d'API tiré des réglages de la 3.20 (mode de recherche, adresse, actions).
fn migrate_320(i: &IndexerCfg, mode: &str, url: &str, actions: &[(String, String, String, String)]) -> crate::apidef::Profile {
    use crate::apidef::{Param, Profile, Req, RespMap};
    let mut p = Profile::default();
    if mode == "off" {
        p.check = "off".into();
    }
    let rest = i.rest_url.trim().trim_end_matches('/').to_string();
    let conv = |t: &str| t.replace("{q}", "{release}").replace("{rest}", &rest).replace("{apikey}", "{key}");
    let params_of = |t: &str| -> Vec<Param> {
        let mut v: Vec<Param> = Vec::new();
        let mut s = t;
        while let Some(a) = s.find('{') {
            let after = &s[a + 1..];
            let Some(b) = after.find('}') else { break };
            let n = &after[..b];
            if !["key", "api", "base", "release", "user"].contains(&n) && !n.is_empty() && !v.iter().any(|x| x.name == n) {
                v.push(Param::new(n, "", n));
            }
            s = &after[b..];
        }
        v
    };
    if mode == "url" && !url.trim().is_empty() {
        p.kind = "json".into();
        p.source = "Adresse de recherche (3.20)".into();
        p.auth = if url.contains("{key}") || url.contains("{apikey}") { "none".into() } else if ["bearer", "header"].contains(&i.auth.as_str()) { i.auth.clone() } else { "none".into() };
        p.auth_name = i.auth_name.clone();
        let u = conv(url.trim());
        p.requests.push(Req { id: "search".into(), label: "Rechercher".into(), method: "GET".into(), params: params_of(&u), url: u, read: true, origin: "manuel".into(), resp: RespMap { format: "auto".into(), ..Default::default() }, ..Default::default() });
    }
    for (n, (label, method, template, help)) in actions.iter().enumerate() {
        if p.is_auto() {
            p.kind = "newznab".into();
            p.requests = crate::apidef::newznab_requests(&p.caps);
        }
        let u = conv(template.trim()).replace("{api}?", "{api}?");
        p.requests.push(Req { id: format!("doc_{}", n + 1), label: label.clone(), help: help.clone(), method: method.clone(), params: params_of(&u), url: u, read: method.eq_ignore_ascii_case("GET"), origin: "import".into(), ..Default::default() });
    }
    p
}

impl IndexerCfg {
    /// Nom d'une catégorie de cet indexer (« TV - FR > 1080p »), si sa liste est connue.
    pub fn cat_name(&self, id: &str) -> Option<&str> {
        self.cat_list.iter().find(|(i, _)| i == id).map(|(_, n)| n.as_str())
    }
    /// « 5342 (TV - FR > 1080p) », ou le numéro seul.
    pub fn cat_label(&self, id: &str) -> String {
        match self.cat_name(id) {
            Some(n) => format!("{id} ({n})"),
            None => id.to_string(),
        }
    }
    /// Catégorie choisie pour une case du tableau (« tv.1080 », « fr.film.sd »…), si réglée.
    pub fn cat_rule(&self, key: &str) -> Option<&str> {
        self.content_cat(key)
    }

    /// Catégorie de cet indexer pour un contenu (vide : celle par défaut du contenu).
    pub fn content_cat(&self, key: &str) -> Option<&str> {
        self.cat_map.iter().find(|(k, v)| k == key && !v.trim().is_empty()).map(|(_, v)| v.trim())
    }

    pub fn new(name: &str) -> IndexerCfg {
        IndexerCfg {
            name: name.to_string(),
            active: true,
            mode: "web".into(),
            cat: "auto".into(),
            cat_sd: "2030".into(),
            cat_hd: "2040".into(),
            cat_uhd: "2045".into(),
            cat_tv_sd: "5030".into(),
            cat_tv_hd: "5040".into(),
            cat_tv_uhd: "5045".into(),
            send_mi: true,
            send_name: true,
            send_nfo: false,
            nfo_created: true,
            includemeta: true,
            dupecheck: true,
            rest_path: "/nzb".into(),
            auth: "header".into(),
            auth_name: "X-API-Key".into(),
            ..Default::default()
        }
    }
    /// Dépôt automatique possible (API ou commande réglée) ?
    pub fn has_api(&self) -> bool {
        match self.mode.as_str() {
            "newznab" => !self.api_url.trim().is_empty() && !self.api_key.trim().is_empty(),
            // adresse d'envoi : base REST + chemin, ou adresse complète dans le chemin
            "rest" => (!self.rest_url.trim().is_empty() || self.rest_path.trim().starts_with("http")) && !self.rest_key_eff().is_empty(),
            "cmd" => !self.cmd.trim().is_empty(),
            _ => false,
        }
    }
    /// Clé de l'API REST (la sienne, sinon celle de Newznab).
    pub fn rest_key_eff(&self) -> &str {
        if self.rest_key.trim().is_empty() {
            self.api_key.trim()
        } else {
            self.rest_key.trim()
        }
    }

    /// Nom du champ du NZB, du NFO, du MediaInfo dans le formulaire d'envoi.
    pub fn field_nzb(&self) -> &str {
        if self.f_nzb.trim().is_empty() { "file" } else { self.f_nzb.trim() }
    }
    pub fn field_nfo(&self) -> &str {
        if self.f_nfo.trim().is_empty() { "nfo" } else { self.f_nfo.trim() }
    }
    pub fn field_mi(&self) -> &str {
        if self.f_mi.trim().is_empty() { "mediainfo" } else { self.f_mi.trim() }
    }

    /// Ligne des réglages d'envoi ajoutés en 3.25 (champs, groupes, page de release, doc).
    fn up_line(&self) -> String {
        let c = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        format!(
            "indexer_up={}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            c(self.name.trim()),
            c(self.f_nzb.trim()),
            c(self.f_nfo.trim()),
            c(self.f_mi.trim()),
            c(self.groups_rule.trim()),
            c(&self.groups.join(",")),
            c(self.release_page.trim()),
            self.doc_notes.lines().map(|l| c(l.trim())).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ¶ ")
        )
    }

    /// Ces réglages sont-ils tous à leur valeur de départ ?
    fn up_default(&self) -> bool {
        self.f_nzb.trim().is_empty() && self.f_nfo.trim().is_empty() && self.f_mi.trim().is_empty() && self.groups.is_empty() && self.groups_rule.is_empty() && self.release_page.trim().is_empty() && self.doc_notes.trim().is_empty()
    }

    /// Copie dont l'adresse et la clé sont celles de l'API REST (pour les fonctions d'envoi).
    pub fn rest_view(&self) -> IndexerCfg {
        IndexerCfg { api_url: self.rest_url.clone(), api_key: self.rest_key_eff().to_string(), ..self.clone() }
    }

    /// Clé utilisée par le mode de dépôt en cours (pour la masquer dans les messages).
    pub fn key_for_mode(&self) -> String {
        if self.mode == "rest" {
            self.rest_key_eff().to_string()
        } else {
            self.api_key.trim().to_string()
        }
    }

    fn api_line(&self) -> String {
        let c = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        format!(
            "indexer_api={}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\tfin\n",
            c(self.name.trim()),
            c(&self.mode),
            c(self.api_url.trim()),
            c(self.api_key.trim()),
            c(&self.cat),
            c(&self.cat_sd),
            c(&self.cat_hd),
            c(&self.cat_uhd),
            self.send_mi,
            self.send_name,
            self.send_nfo,
            self.includemeta,
            self.dupecheck,
            c(self.extra_query.trim()),
            c(self.extra_fields.trim()),
            c(&self.cmd),
            c(self.rest_path.trim()),
            c(&self.auth),
            c(self.auth_name.trim()),
            c(self.rest_url.trim()),
            c(self.rest_key.trim()),
            c(&self.cat_tv_sd),
            c(&self.cat_tv_hd),
            c(&self.cat_tv_uhd)
        )
    }
}

// ---------------------------------------------------------------- secrets

/// Mots de passe et clés connus (serveurs Usenet, ngPost.conf, indexers, client de
/// téléchargement, e-mail) : jamais affichés ni écrits dans un journal, quel que soit
/// l'endroit où ils apparaîtraient (récapitulatif, journal de NgPost, notification, e-mail).
static SECRETS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

/// Texte affiché à la place d'un secret.
pub const MASK: &str = "••••••";

/// Ajoute un secret au registre (moins de 4 caractères : masqué seulement après « pass= »,
/// pour ne pas abîmer le reste du texte).
pub fn add_secret(s: &str) {
    let s = s.trim();
    if s.chars().count() < 4 {
        return;
    }
    if let Ok(mut v) = SECRETS.write() {
        if !v.iter().any(|x| x == s) {
            v.push(s.to_string());
            // le plus long d'abord : un secret qui en contient un autre est masqué en entier
            v.sort_by_key(|x| std::cmp::Reverse(x.len()));
        }
    }
}

/// Secrets des réglages (appelé au démarrage et à chaque enregistrement).
pub fn register_secrets(c: &Config) {
    for s in &c.usenet_servers {
        add_secret(&s.pass);
    }
    for i in &c.indexers {
        add_secret(&i.api_key);
        add_secret(&i.rest_key);
        add_secret(&i.api.key);
    }
    for s in [&c.dl_key, &c.dl_pass, &c.mail_pass] {
        add_secret(s);
    }
}

/// Masque les secrets connus, et toute valeur après « pass= », « password: », « apikey= »…
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    if let Ok(v) = SECRETS.read() {
        for s in v.iter() {
            if out.contains(s.as_str()) {
                out = out.replace(s.as_str(), MASK);
            }
        }
    }
    redact_pairs(&out)
}

/// « pass = abc », « password: abc », « apikey=abc » → valeur masquée (mot isolé, pas
/// « rar_pass » : le mot de passe des archives est montré exprès).
fn redact_pairs(text: &str) -> String {
    const KEYS: [&str; 6] = ["password", "passwd", "pass", "apikey", "api_key", "secret"];
    let low = text.to_ascii_lowercase();
    let b = low.as_bytes();
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let mut hit = None;
        for k in KEYS {
            if b[i..].starts_with(k.as_bytes()) {
                let before_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b'-');
                let after = i + k.len();
                let after_ok = after >= b.len() || !(b[after].is_ascii_alphanumeric() || b[after] == b'_');
                if before_ok && after_ok {
                    hit = Some(after);
                    break;
                }
            }
        }
        let Some(mut j) = hit else {
            i += 1;
            continue;
        };
        while j < b.len() && b[j] == b' ' {
            j += 1;
        }
        if j < b.len() && (b[j] == b'=' || b[j] == b':') {
            j += 1;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            let start = j;
            while j < b.len() && !b" \t,;&\"'<>|)".contains(&b[j]) {
                j += 1;
            }
            if j > start {
                cuts.push((start, j));
            }
        }
        i = j.max(i + 1);
    }
    if cuts.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (a, z) in cuts {
        // positions d'octets ASCII identiques dans `text` (to_ascii_lowercase garde les octets)
        if !text.is_char_boundary(a) || !text.is_char_boundary(z) {
            continue;
        }
        out.push_str(&text[last..a]);
        out.push_str(MASK);
        last = z;
    }
    out.push_str(&text[last..]);
    out
}

/// Niveau de détail des journaux, lu par les fils de travail (post, dépôts) sans la config.
static LOG_DETAIL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_detail(on: bool) {
    LOG_DETAIL.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Journaux détaillés demandés (Réglages → Journaux).
pub fn detail() -> bool {
    LOG_DETAIL.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub inbox: String,
    pub ngpost_dir: String,
    pub xml_dir: String,
    /// Emplacement du rapport MediaInfo : "video" (à côté de la vidéo) ou "dir" (dossier `xml_dir`).
    pub mi_dest: String,
    pub quarantine: String,
    pub group: String,
    pub template: String,
    /// Langue à mettre quand les pistes audio n'ont aucune langue renseignée (vide = ne pas deviner).
    pub undefined_lang: String,
    pub mediainfo: String,
    pub mi_format: String,
    /// BDInfo (bdinfo-rs ou BDInfoCLI) : vide = recherche automatique.
    pub bdinfo: String,
    /// Rapport BDInfo des Blu-ray complets (scan du titre principal), utilisé comme NFO.
    pub bdinfo_scan: bool,
    /// Créer un NFO (MediaInfo en texte, BDInfo pour un Blu-ray) quand la release n'en a pas.
    pub nfo_create: bool,
    /// NFO d'origine gardé dans la release postée (séries, disques, contenus).
    pub nfo_keep: bool,
    /// Journaux détaillés : test de chaque serveur, réglages du post, lignes techniques de
    /// NgPost, réponses complètes des indexers.
    pub log_detail: bool,
    /// Hauteur des journaux NgPost et des dépôts (pixels), réglée en tirant leur poignée.
    pub log_h_post: u32,
    pub log_h_api: u32,
    /// Taille du texte des journaux (points) et retour à la ligne des lignes longues.
    pub log_font: f32,
    pub log_wrap: bool,
    /// Interface Expert (tous les réglages et options) ; sinon Simple : l'essentiel, les
    /// valeurs par défaut pour le reste.
    pub ui_expert: bool,
    /// Catégorie choisie à la main par indexer pour le traitement en cours (jamais enregistrée).
    pub cat_force: Vec<(String, String)>,
    /// Groupes Usenet choisis à la main pour le traitement en cours (jamais enregistrés).
    pub groups_force: String,
    /// Modèle du rapport BDInfo : parties gardées (clés de disc::BD_SECTIONS) et texte.
    pub bd_sections: String,
    pub bd_template: String,
    /// Ce que devient le NFO d'un Blu-ray qui a déjà le sien quand son BDInfo est fait :
    /// « orig » (gardé seul), « bdinfo » (remplacé par le BDInfo), « both » (BDInfo ajouté dessous).
    pub nfo_bd: String,
    /// Client de téléchargement : « sabnzbd », « nzbget » (vide : aucun), adresse, clé API
    /// (SABnzbd), identifiant et mot de passe (NZBGet), catégorie des téléchargements, dossiers
    /// du client vus d'ici (« /data/complete=D:\Usenet »), suite (« move » / « copy »),
    /// repost automatique à la fin du téléchargement.
    pub dl_client: String,
    pub dl_url: String,
    pub dl_key: String,
    pub dl_user: String,
    pub dl_pass: String,
    pub dl_cat: String,
    pub dl_path_map: String,
    pub dl_after: String,
    pub dl_auto: bool,
    /// Indexer où les NZB ouverts (double clic, clic droit) ont été trouvés : vide = demander,
    /// « * » = ailleurs (aucun de mes indexers), sinon le nom de l'indexer.
    pub dl_origin: String,
    /// Traitement par défaut d'un repost : NFO (auto, keep, none) et MediaInfo joint.
    pub dl_nfo: String,
    pub dl_mi: bool,
    /// Dossier de NZB surveillé (sous-dossier par indexer d'origine).
    pub dl_watch: String,
    pub dl_watch_on: bool,
    /// Récapitulatif (et contrôles) avant chaque traitement manuel.
    pub confirm_recap: bool,
    /// Release déjà sur un indexer visé (recherche avant le post) : « skip » (pas postée
    /// pour lui ; pas de post du tout si elle est partout), « warn » (signalée), « off ».
    pub dupe_check: String,
    /// Assistant de configuration fait (premier lancement) ; profil choisi : simple ou expert.
    pub setup_done: bool,
    /// NZB pris en charge : « keep » (rangé dans « _Pris en charge ») ou « delete ».
    pub dl_watch_after: String,
    /// Refuser un NZB déjà téléchargé, en cours ou reposté (même nom de release).
    pub dl_watch_dupes: bool,
    /// Dossier de NZB surveillé : règles modifiées ou ajoutées (racine, sous-dossiers), relu
    /// toutes les `dl_watch_interval` s, .nzb pris quand sa taille ne bouge plus depuis
    /// `dl_watch_stable` s, un sous-dossier par indexer actif, notification à chaque .nzb pris.
    pub dl_routes: Vec<crate::dl::WatchRoute>,
    /// Auto-configuration et recherche de l'adresse d'envoi d'un indexer : secondes entre deux
    /// requêtes (jamais en rafale : risque de bannissement) et nombre de requêtes au plus.
    pub api_probe_delay: u64,
    pub api_probe_max: usize,
    pub dl_watch_interval: u64,
    pub dl_watch_stable: u64,
    pub dl_watch_subdirs: bool,
    pub dl_watch_notify: bool,
    /// Priorité donnée au client par défaut (« default » : la sienne ; low, normal, high, force).
    pub dl_prio: String,
    /// NZB ajouté sur la page Téléchargements : « dl » (télécharger seulement), « repost », ou
    /// « » (repost s'il y a un autre indexer, sinon téléchargement seul).
    pub dl_new_mode: String,
    /// Indexers où reposter par défaut (vide : tous les autres indexers actifs).
    pub dl_targets: Vec<String>,
    /// Ouverture d'un .nzb (double clic) : ask (page Téléchargements), auto (télécharger et
    /// reposter), dl (télécharger seulement), send (déposer sur les indexers).
    pub nzb_open: String,
    /// Intégration à Windows installée : menu du clic droit, ouverture des .nzb.
    pub shell_menu: bool,
    pub nzb_assoc: bool,
    /// Fichiers de réparation : « » = par2 de NgPost, « turbo » = par2cmdline-turbo
    /// (chemin ng_turbo), « parpar » = ParPar (chemin ng_parpar). Chemin vide : cherché.
    pub ng_par2_tool: String,
    pub ng_turbo: String,
    pub ng_parpar: String,
    /// Traitement manuel : textes modifiés avant le traitement (rapport MediaInfo livré, NFO
    /// envoyé, BDInfo). Jamais enregistrés.
    pub mi_override: Option<String>,
    pub nfo_override: Option<String>,
    pub bd_override: Option<String>,
    /// NFO d'origine envoyé aux indexers avec le NZB.
    pub nfo_send: bool,
    pub interval: u64,
    pub stable_secs: u64,
    /// "auto", "copy" ou "move" (traitement manuel et lot ; la surveillance déplace toujours).
    pub transfer: String,
    /// Refuser un nom de release déjà envoyé avec succès (d'après l'historique).
    pub skip_duplicates: bool,
    pub autostart: bool,
    /// Exécutable de NgPost (ngPost.exe) et arguments de lancement facultatifs.
    pub ngpost_exe: String,
    pub ngpost_args: String,
    /// Lancer NgPost s'il est fermé quand une vidéo arrive dans le dossier de sortie.
    pub ngpost_autostart: bool,
    /// Dossier où NgPost écrit ses NZB.
    pub nzb_dir: String,
    /// Dossier « prêt pour l'indexer » : un sous-dossier par release (NZB + rapport MediaInfo).
    pub indexer_dir: String,
    pub indexers: Vec<IndexerCfg>,
    /// "cli" : l'application poste chaque vidéo avec NgPost en ligne de commande ;
    /// "watch" : la vidéo est déposée dans le dossier surveillé par NgPost (interface).
    pub post_mode: String,
    /// Arguments NgPost en ligne de commande ({input}, {nzb}) ; vide = standard.
    pub post_args: String,
    pub post_obfuscate: bool,
    /// Ranger la vidéo postée dans « _Postés » (dossier de sortie).
    pub post_move_done: bool,
    /// Que faire de la vidéo une fois postée (NZB vérifié) : "trash" (corbeille),
    /// "delete" (suppression définitive), "posted" (dossier « _Postés »), "keep" (laisser).
    pub post_after: String,
    // ---- options de post NgPost (traduites en ligne de commande)
    /// Options déjà reprises de ngPost.conf (une fois, à la première lecture).
    pub ng_imported: bool,
    pub ng_compress: bool,
    pub ng_gen_name: bool,
    pub ng_gen_pass: bool,
    /// Mot de passe fixe des archives (si pas de mot de passe aléatoire).
    pub ng_rar_pass: String,
    /// Taille des volumes en Mo, nombre maximum de volumes (0 = valeur de ngPost.conf).
    pub ng_rar_size: u32,
    pub ng_rar_max: u32,
    pub ng_par2: bool,
    /// Redondance par2 en % (0 = valeur de ngPost.conf).
    pub ng_par2_pct: u32,
    /// Groupes (vide = ngPost.conf).
    pub ng_groups: String,
    /// Expéditeur (vide = ngPost.conf / aléatoire).
    pub ng_from: String,
    /// Connexions réparties sur N threads, taille d'article, tentatives (0 = ngPost.conf).
    pub ng_threads: u32,
    pub ng_article_size: u32,
    pub ng_retry: u32,
    /// Dossier temporaire et exécutable 7-Zip / RAR (vide = ngPost.conf).
    pub ng_tmp_dir: String,
    pub ng_rar_path: String,
    /// Journal détaillé de NgPost (-d).
    pub ng_debug: bool,
    /// Vérifier que le NZB est complet avant de le ranger et de le distribuer.
    pub post_verify: bool,
    // ---- que faire de chaque vidéo
    /// "rename" (renommer + MediaInfo), "post" (+ post Usenet et NZB pour les indexers),
    /// "auto" (+ ouverture des pages d'upload quand tout est prêt).
    pub action: String,
    /// Indexers visés par défaut (vide = tous les indexers actifs).
    pub targets: Vec<String>,
    /// Déposer automatiquement le NZB (+ MediaInfo) sur les indexers qui ont une API réglée.
    pub api_upload: bool,
    /// À la fermeture de la fenêtre : "ask" (demander), "tray" (zone de notification), "quit".
    pub close_action: String,
    /// Démarrer réduit dans la zone de notification.
    pub start_in_tray: bool,
    /// Bulles d'information quand l'application est dans la zone de notification.
    pub tray_notify: bool,
    pub accent: String,
    /// Thème de l'interface (clé, ex. « nuit », « papier »).
    pub theme: String,
    /// Copie automatique des réglages, de l'historique et des statistiques dans le profil
    /// Windows, reprise par une nouvelle version installée ailleurs.
    pub auto_backup: bool,
    /// Séries : modèles du nom de release (pack) et des épisodes, mot ajouté aux packs complets.
    pub series_template: String,
    pub episode_template: String,
    pub complete_word: String,
    /// Séries : renommer aussi les épisodes (sinon noms d'origine gardés), une release par
    /// saison (sinon une pour toute la série), mention « COMPLETE » mise d'office si complète.
    pub series_rename: bool,
    pub series_split: bool,
    /// Tours de post d'une série : toute la série, une release par saison, une par épisode.
    pub series_round_whole: bool,
    pub series_round_seasons: bool,
    pub series_round_episodes: bool,
    pub series_auto_complete: bool,
    /// Place insuffisante pour compresser un dossier d'un coup : compression en plusieurs lots,
    /// un seul NZB fusionné.
    pub ng_lots: bool,
    /// E-mail quand tous les traitements sont terminés (SMTP, envoi par curl).
    pub mail_enabled: bool,
    pub mail_server: String,
    pub mail_port: u16,
    /// "ssl" (port 465) ou "starttls" (port 587).
    pub mail_security: String,
    pub mail_user: String,
    pub mail_pass: String,
    /// Expéditeur (vide : l'identifiant) et destinataires (séparés par des virgules).
    pub mail_from: String,
    pub mail_to: String,
    /// Objet et texte de l'e-mail (vides : modèles par défaut), avec des {champs}.
    pub mail_subject: String,
    pub mail_body: String,
    /// Fournisseurs SMTP ajoutés par l'utilisateur, en plus de ceux proposés.
    pub mail_providers: Vec<MailProvider>,
    /// E-mail en couleur (HTML, avec le texte brut à côté) : rouge s'il y a une erreur, jaune
    /// pour de simples alertes, vert quand tout est bon.
    pub mail_html: bool,
    /// Identité des requêtes vers les indexers (User-Agent) : "" curl, "app", "browser"
    /// (navigateur tiré au sort au lancement), "rotate" (change à chaque requête), ou le texte
    /// exact à envoyer.
    pub net_ua: String,
    /// Serveurs Usenet gérés ici (remplacent les [server] de ngPost.conf au moment du post).
    pub usenet_managed: bool,
    /// Avant chaque post : test de latence, seul le serveur le plus rapide est utilisé.
    pub usenet_auto_best: bool,
    pub usenet_servers: Vec<crate::usenet::Server>,
    /// Catégories des autres contenus (jeux, logiciels, musique…), toutes configurables.
    pub content_cats: Vec<crate::content::Category>,
    /// Groupes Usenet des films et des séries (vides : ceux des options du post).
    pub groups_film: String,
    pub groups_tv: String,
    /// Politique de groupes de ngPost : "" (ngPost.conf), all, each_post, each_file, auto.
    pub group_policy: String,
    /// Serveurs Usenet d'ici recopiés dans ngPost.conf à chaque modification.
    pub usenet_sync_conf: bool,
    /// Sauvegarde automatique : à chaque modification des réglages, toutes les N minutes
    /// (0 : jamais), à la fermeture, au démarrage ; nombre de copies datées gardées.
    pub backup_on_change: bool,
    pub backup_every_min: u32,
    pub backup_on_close: bool,
    pub backup_on_start: bool,
    pub backup_keep: u32,
}

/// Fournisseur SMTP enregistré par l'utilisateur.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MailProvider {
    pub name: String,
    pub server: String,
    pub port: u16,
    /// "ssl" ou "starttls".
    pub security: String,
}

/// Texte sur plusieurs lignes gardé sur une seule ligne de nzbff.conf.
pub fn esc_lines(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\r', "").replace('\n', "\\n")
}

/// Valeur sur une ligne (sans saut de ligne) pour le fichier de réglages.
fn one(s: &str) -> String {
    s.replace(['\r', '\n'], "")
}

pub fn unesc_lines(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some(x) => out.push(x),
            None => out.push('\\'),
        }
    }
    out
}

/// (clé, libellé court, explication) des actions possibles sur une vidéo.
pub const ACTIONS: &[(&str, &str, &str)] = &[
    ("rename", "Renommer seulement", "nom de release + rapport MediaInfo, rien n'est posté"),
    ("post", "Renommer et poster", "+ post Usenet avec NgPost, NZB + MediaInfo rangés pour les indexers choisis"),
    ("auto", "Tout automatique", "+ ouverture de la page d'upload de chaque indexer choisi dès que tout est prêt"),
];

/// (clé, libellé) : que faire de la vidéo une fois postée.
pub const POST_AFTER: &[(&str, &str)] = &[
    ("trash", "La supprimer (corbeille de Windows)"),
    ("delete", "La supprimer définitivement"),
    ("posted", "La ranger dans « _Postés »"),
    ("keep", "La laisser dans le dossier de sortie"),
];

pub fn action_label(k: &str) -> &'static str {
    ACTIONS.iter().find(|a| a.0 == k).map(|a| a.1).unwrap_or("Renommer et poster")
}

impl Default for Config {
    fn default() -> Self {
        Config {
            inbox: String::new(),
            ngpost_dir: String::new(),
            xml_dir: String::new(),
            mi_dest: "video".into(),
            quarantine: String::new(),
            group: String::new(),
            template: DEFAULT_TEMPLATE.into(),
            undefined_lang: "FRENCH".into(),
            mediainfo: "mediainfo".into(),
            bdinfo: String::new(),
            bdinfo_scan: true,
            nfo_create: true,
            nfo_keep: true,
            log_detail: false,
            log_h_post: 220,
            log_h_api: 300,
            log_font: 12.0,
            log_wrap: true,
            ui_expert: false,
            cat_force: Vec::new(),
            groups_force: String::new(),
            bd_sections: crate::disc::BD_DEFAULT_SECTIONS.into(),
            bd_template: crate::disc::BD_DEFAULT_TEMPLATE.into(),
            mi_override: None,
            nfo_override: None,
            bd_override: None,
            nfo_bd: "orig".into(),
            dl_client: String::new(),
            dl_url: String::new(),
            dl_key: String::new(),
            dl_user: String::new(),
            dl_pass: String::new(),
            dl_cat: "nzbff".into(),
            dl_path_map: String::new(),
            dl_after: "move".into(),
            dl_auto: true,
            dl_origin: String::new(),
            dl_nfo: "auto".into(),
            dl_mi: true,
            dl_watch: String::new(),
            dl_watch_on: false,
            confirm_recap: true,
            dupe_check: "skip".into(),
            setup_done: false,
            dl_watch_after: "keep".into(),
            dl_watch_dupes: true,
            dl_routes: Vec::new(),
            api_probe_delay: 3,
            api_probe_max: 15,
            dl_watch_interval: 3,
            dl_watch_stable: 3,
            dl_watch_subdirs: true,
            dl_watch_notify: true,
            dl_prio: "default".into(),
            dl_new_mode: String::new(),
            dl_targets: Vec::new(),
            nzb_open: "ask".into(),
            shell_menu: false,
            nzb_assoc: false,
            ng_par2_tool: String::new(),
            ng_turbo: String::new(),
            ng_parpar: String::new(),
            nfo_send: true,
            mi_format: "xml-full".into(),
            interval: 30,
            stable_secs: 10,
            transfer: "auto".into(),
            skip_duplicates: true,
            autostart: false,
            ngpost_exe: String::new(),
            ngpost_args: String::new(),
            ngpost_autostart: true,
            nzb_dir: String::new(),
            indexer_dir: String::new(),
            indexers: Vec::new(),
            post_mode: "cli".into(),
            post_args: String::new(),
            post_obfuscate: true,
            post_move_done: true,
            post_after: "trash".into(),
            ng_imported: false,
            ng_compress: false,
            ng_gen_name: false,
            ng_gen_pass: false,
            ng_rar_pass: String::new(),
            ng_rar_size: 0,
            ng_rar_max: 0,
            ng_par2: false,
            ng_par2_pct: 0,
            ng_groups: String::new(),
            ng_from: String::new(),
            ng_threads: 0,
            ng_article_size: 0,
            ng_retry: 0,
            ng_tmp_dir: String::new(),
            ng_rar_path: String::new(),
            ng_debug: false,
            post_verify: true,
            action: "post".into(),
            targets: Vec::new(),
            api_upload: true,
            close_action: "ask".into(),
            start_in_tray: false,
            tray_notify: true,
            accent: "Thème".into(),
            theme: "nuit".into(),
            auto_backup: true,
            series_template: crate::series::SERIES_TEMPLATE.into(),
            episode_template: crate::series::EPISODE_TEMPLATE.into(),
            complete_word: "COMPLETE".into(),
            series_rename: false,
            series_split: false,
            series_round_whole: true,
            series_round_seasons: false,
            series_round_episodes: false,
            series_auto_complete: true,
            ng_lots: false,
            mail_enabled: false,
            mail_server: String::new(),
            mail_port: 465,
            mail_security: "ssl".into(),
            mail_user: String::new(),
            mail_pass: String::new(),
            mail_from: String::new(),
            mail_to: String::new(),
            mail_subject: String::new(),
            mail_body: String::new(),
            mail_html: true,
            net_ua: String::new(),
            mail_providers: Vec::new(),
            usenet_managed: false,
            usenet_auto_best: false,
            usenet_servers: Vec::new(),
            content_cats: crate::content::defaults(),
            groups_film: String::new(),
            group_policy: String::new(),
            groups_tv: String::new(),
            usenet_sync_conf: false,
            backup_on_change: true,
            backup_every_min: 10,
            backup_on_close: true,
            backup_on_start: false,
            backup_keep: 10,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Config {
        let mut c = Config::default();
        let mut api_lines: Vec<String> = Vec::new();
        let mut up_lines: Vec<String> = Vec::new();
        let mut grp_lines: Vec<String> = Vec::new();
        let mut cat_lines: Vec<String> = Vec::new();
        let mut catlist_lines: Vec<String> = Vec::new();
        let mut catfr_lines: Vec<String> = Vec::new();
        let mut bdfield_lines: Vec<(String, String)> = Vec::new();
        let mut search_lines: Vec<String> = Vec::new();
        let mut action_lines: Vec<String> = Vec::new();
        let mut profile_lines: Vec<String> = Vec::new();
        let mut nfo_lines: Vec<String> = Vec::new();
        let mut cats: Option<Vec<crate::content::Category>> = None;
        let mut cats_rev: u32 = 0;
        let mut rounds_seen = false;
        let Ok(text) = fs::read_to_string(path) else {
            return c;
        };
        // configuration d'une version d'avant le mode Simple : on garde tout visible (Expert)
        let mut expert_seen = false;
        let mut setup_seen = false;
        // catégories des autres contenus pas encore déduites des listes d'indexer importées
        let mut cat_fill_rev = 0u32;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim().to_string();
            let b = v == "true";
            match k.trim() {
                "inbox" => c.inbox = v,
                "ngpost_dir" => c.ngpost_dir = v,
                "xml_dir" => {
                    if !v.is_empty() && !text.contains("mi_dest=") {
                        c.mi_dest = "dir".into(); // ancienne config : dossier réglé = dossier dédié
                    }
                    c.xml_dir = v
                }
                "mi_dest" if v == "video" || v == "dir" || v == "indexer" => c.mi_dest = v,
                "ngpost_exe" => c.ngpost_exe = v,
                "ngpost_args" => c.ngpost_args = v,
                "ngpost_autostart" => c.ngpost_autostart = b,
                "nzb_dir" => c.nzb_dir = v,
                "indexer_dir" => c.indexer_dir = v,
                "post_mode" if v == "cli" || v == "watch" => c.post_mode = v,
                "post_args" => c.post_args = v,
                "post_obfuscate" => c.post_obfuscate = b,
                "post_move_done" => {
                    c.post_move_done = b;
                    if !text.contains("post_after=") {
                        // ancienne config : « ranger dans _Postés » ou rien
                        c.post_after = if b { "posted".into() } else { "keep".into() };
                    }
                }
                "post_after" if POST_AFTER.iter().any(|x| x.0 == v) => c.post_after = v,
                "ng_imported" => c.ng_imported = b,
                "ng_compress" => c.ng_compress = b,
                "ng_gen_name" => c.ng_gen_name = b,
                "ng_gen_pass" => c.ng_gen_pass = b,
                "ng_rar_pass" => c.ng_rar_pass = v,
                "ng_rar_size" => c.ng_rar_size = v.parse().unwrap_or(0),
                "ng_rar_max" => c.ng_rar_max = v.parse().unwrap_or(0),
                "ng_par2" => c.ng_par2 = b,
                "ng_par2_pct" => c.ng_par2_pct = v.parse().unwrap_or(0),
                "ng_groups" => c.ng_groups = v,
                "ng_from" => c.ng_from = v,
                "ng_threads" => c.ng_threads = v.parse().unwrap_or(0),
                "ng_article_size" => c.ng_article_size = v.parse().unwrap_or(0),
                "ng_retry" => c.ng_retry = v.parse().unwrap_or(0),
                "ng_tmp_dir" => c.ng_tmp_dir = v,
                "ng_rar_path" => c.ng_rar_path = v,
                "ng_debug" => c.ng_debug = b,
                "post_verify" => c.post_verify = b,
                "action" if ACTIONS.iter().any(|a| a.0 == v) => c.action = v,
                "api_upload" => c.api_upload = b,
                "close_action" if ["ask", "tray", "quit"].contains(&v.as_str()) => c.close_action = v,
                // 3.20 : « start_in_tray » ne vaut plus que pour un lancement à la main (le lancement
                // avec Windows est toujours discret) ; l'ancienne clé, cochée pour l'autostart, est oubliée
                "start_in_tray_manual" => c.start_in_tray = b,
                "tray_notify" => c.tray_notify = b,
                "targets" => c.targets = v.split('|').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect(),
                "indexer_api" => api_lines.push(v.to_string()),
                "indexer_cat" => cat_lines.push(v.to_string()),
                "indexer_catlist" => catlist_lines.push(v.to_string()),
                "indexer_cat_fr" => catfr_lines.push(v.trim().to_string()),
                "indexer_search" => search_lines.push(v.to_string()),
                "indexer_nfo" => nfo_lines.push(v.to_string()),
                "indexer_action" => action_lines.push(v.to_string()),
                "indexer_profile" => profile_lines.push(v.to_string()),
                "indexer_up" => up_lines.push(v.to_string()),
                "indexer_bdfield" => {
                    if let Some((n, f)) = v.split_once('\t') {
                        bdfield_lines.push((n.trim().to_string(), f.trim().to_string()));
                    }
                }
                "content_cats" if v == "none" => cats = Some(cats.unwrap_or_default()),
                "content_cat" => {
                    if let Some(cc) = crate::content::Category::parse_line(&v) {
                        cats.get_or_insert_with(Vec::new).push(cc);
                    }
                }
                "indexer" => {
                    let f: Vec<&str> = v.splitn(3, '|').collect();
                    let name = f.first().map(|x| x.trim().to_string()).unwrap_or_default();
                    if !name.is_empty() {
                        let mut i = IndexerCfg::new(&name);
                        i.url = f.get(1).map(|x| x.trim().to_string()).unwrap_or_default();
                        i.active = f.get(2).map(|x| x.trim() != "false").unwrap_or(true);
                        c.indexers.push(i);
                    }
                }
                "quarantine" => c.quarantine = v,
                "group" => c.group = v,
                "template" => c.template = if v.is_empty() { DEFAULT_TEMPLATE.into() } else { v },
                "undefined_lang" => c.undefined_lang = v,
                "mediainfo" => c.mediainfo = if v.is_empty() { "mediainfo".into() } else { v },
                "bdinfo" => c.bdinfo = v,
                "bdinfo_scan" => c.bdinfo_scan = b,
                "nfo_create" => c.nfo_create = b,
                "nfo_keep" => c.nfo_keep = b,
                "log_detail" => c.log_detail = b,
                "log_h_post" => c.log_h_post = v.parse().unwrap_or(220).clamp(80, 3000),
                "log_h_api" => c.log_h_api = v.parse().unwrap_or(300).clamp(80, 3000),
                "log_font" => c.log_font = v.parse::<f32>().unwrap_or(12.0).clamp(8.0, 24.0),
                "log_wrap" => c.log_wrap = b,
                "bd_sections" => c.bd_sections = v.to_string(),
                "dl_client" => c.dl_client = v.trim().to_string(),
                "dl_url" => c.dl_url = v.trim().to_string(),
                "dl_key" => c.dl_key = v.trim().to_string(),
                "dl_user" => c.dl_user = v.trim().to_string(),
                "dl_pass" => c.dl_pass = v.to_string(),
                "dl_cat" => c.dl_cat = v.trim().to_string(),
                "dl_path_map" => c.dl_path_map = v.trim().to_string(),
                "dl_after" if v.trim() == "move" || v.trim() == "copy" => c.dl_after = v.trim().to_string(),
                "dl_auto" => c.dl_auto = b,
                "dl_origin" => c.dl_origin = v.trim().to_string(),
                "dl_nfo" if ["auto", "keep", "none"].contains(&v.trim()) => c.dl_nfo = v.trim().to_string(),
                "dl_mi" => c.dl_mi = b,
                "dl_watch" => c.dl_watch = v.trim().to_string(),
                "dl_watch_on" => c.dl_watch_on = b,
                "confirm_recap" => c.confirm_recap = b,
                "dupe_check" if ["skip", "warn", "off"].contains(&v.trim()) => c.dupe_check = v.trim().to_string(),
                "setup_done" => {
                    c.setup_done = b;
                    setup_seen = true;
                }
                "dl_watch_after" if v.trim() == "keep" || v.trim() == "delete" => c.dl_watch_after = v.trim().to_string(),
                "dl_watch_dupes" => c.dl_watch_dupes = b,
                "dl_route" => {
                    if let Some(r) = crate::dl::WatchRoute::from_line(&v) {
                        c.dl_routes.retain(|x| !x.folder.eq_ignore_ascii_case(&r.folder));
                        c.dl_routes.push(r);
                    }
                }
                "api_probe_delay" => c.api_probe_delay = v.trim().parse().unwrap_or(3).clamp(1, 120),
                "api_probe_max" => c.api_probe_max = v.trim().parse().unwrap_or(15).clamp(3, 60),
                "dl_watch_interval" => c.dl_watch_interval = v.trim().parse().unwrap_or(3).clamp(1, 600),
                "dl_watch_stable" => c.dl_watch_stable = v.trim().parse().unwrap_or(3).min(600),
                "dl_watch_subdirs" => c.dl_watch_subdirs = b,
                "dl_watch_notify" => c.dl_watch_notify = b,
                "dl_prio" if crate::dl::PRIOS.iter().any(|p| p.0 == v.trim()) => c.dl_prio = v.trim().to_string(),
                "dl_new_mode" if ["", "dl", "repost"].contains(&v.trim()) => c.dl_new_mode = v.trim().to_string(),
                "dl_targets" => c.dl_targets = v.split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect(),
                "nzb_open" if ["ask", "auto", "dl", "send"].contains(&v.trim()) => c.nzb_open = v.trim().to_string(),
                "shell_menu" => c.shell_menu = b,
                "nzb_assoc" => c.nzb_assoc = b,
                "ng_par2_tool" if ["", "turbo", "parpar"].contains(&v.trim()) => c.ng_par2_tool = v.trim().to_string(),
                "ng_parpar" => c.ng_parpar = v.trim().to_string(),
                "ng_turbo" => c.ng_turbo = v.trim().to_string(),
                "nfo_bd" if ["orig", "bdinfo", "both"].contains(&v.trim()) => c.nfo_bd = v.trim().to_string(),
                "bd_template" => c.bd_template = unesc_lines(&v),
                "cat_fill_rev" => cat_fill_rev = v.trim().parse().unwrap_or(0),
                "ui_expert" => {
                    c.ui_expert = b;
                    expert_seen = true;
                }
                "nfo_send" => c.nfo_send = b,
                "content_cats_rev" => cats_rev = v.trim().parse().unwrap_or(0),
                "mi_format" if MI_FORMATS.iter().any(|f| f.0 == v) => c.mi_format = v,
                "old_xml" if b => c.mi_format = "oldxml".into(), // ancienne config
                "interval" => c.interval = v.parse().unwrap_or(30).max(1),
                "stable_secs" => c.stable_secs = v.parse().unwrap_or(10),
                "transfer" if TRANSFERS.iter().any(|t| t.0 == v) => c.transfer = v,
                "skip_duplicates" => c.skip_duplicates = b,
                "autostart" => c.autostart = b,
                "accent" => c.accent = v,
                "theme" => c.theme = v,
                "auto_backup" => c.auto_backup = b,
                "series_template" if !v.is_empty() => c.series_template = v,
                "episode_template" if !v.is_empty() => c.episode_template = v,
                "complete_word" => c.complete_word = v,
                "series_rename" => c.series_rename = b,
                "series_split" => c.series_split = b,
                "series_round_whole" => {
                    c.series_round_whole = b;
                    rounds_seen = true;
                }
                "series_round_seasons" => c.series_round_seasons = b,
                "series_round_episodes" => c.series_round_episodes = b,
                "series_auto_complete" => c.series_auto_complete = b,
                "ng_lots" => c.ng_lots = b,
                "mail_enabled" => c.mail_enabled = b,
                "mail_server" => c.mail_server = v,
                "mail_port" => c.mail_port = v.parse().unwrap_or(465),
                "mail_security" if v == "ssl" || v == "starttls" => c.mail_security = v,
                "mail_user" => c.mail_user = v,
                "mail_pass" => c.mail_pass = v,
                "mail_from" => c.mail_from = v,
                "mail_to" => c.mail_to = v,
                "mail_subject" => c.mail_subject = v,
                "usenet_managed" => c.usenet_managed = b,
                "usenet_sync_conf" => c.usenet_sync_conf = b,
                "backup_on_change" => c.backup_on_change = b,
                "backup_every_min" => c.backup_every_min = v.parse().unwrap_or(10),
                "backup_on_close" => c.backup_on_close = b,
                "backup_on_start" => c.backup_on_start = b,
                "backup_keep" => c.backup_keep = v.parse::<u32>().unwrap_or(10).clamp(1, 365),
                "groups_film" => c.groups_film = v,
                "group_policy" => c.group_policy = v,
                "groups_tv" => c.groups_tv = v,
                "usenet_auto_best" => c.usenet_auto_best = b,
                "usenet_server" => {
                    if let Some(sv) = crate::usenet::Server::parse_line(&v) {
                        c.usenet_servers.push(sv);
                    }
                }
                "mail_body" => c.mail_body = unesc_lines(&v),
                "mail_html" => c.mail_html = b,
                "net_ua" => c.net_ua = v.trim().to_string(),
                "indexer_grp" => grp_lines.push(v.to_string()),
                "mail_provider" => {
                    let f: Vec<&str> = v.split('|').map(str::trim).collect();
                    if f.len() >= 4 && !f[0].is_empty() && !f[1].is_empty() {
                        c.mail_providers.push(MailProvider {
                            name: f[0].to_string(),
                            server: f[1].to_string(),
                            port: f[2].parse().unwrap_or(465),
                            security: if f[3] == "starttls" { "starttls".into() } else { "ssl".into() },
                        });
                    }
                }
                _ => {}
            }
        }
        // anciennes versions : « une release par saison » ou une seule
        if !expert_seen {
            c.ui_expert = true;
        }
        // configuration d'une version d'avant l'assistant : déjà réglée, pas d'assistant imposé
        if !setup_seen && !c.ngpost_dir.trim().is_empty() {
            c.setup_done = true;
        }
        if !rounds_seen {
            c.series_round_whole = !c.series_split;
            c.series_round_seasons = c.series_split;
            c.series_round_episodes = false;
        }
        if let Some(mut cats) = cats {
            // catégories ajoutées depuis (disques complets) : proposées une fois
            if cats_rev < 2 {
                for d in crate::content::defaults().into_iter().filter(|d| d.key == "bluray" || d.key == "dvd") {
                    if !cats.iter().any(|x| x.key == d.key) {
                        cats.push(d);
                    }
                }
            }
            c.content_cats = cats;
        }
        for l in catlist_lines {
            let f: Vec<&str> = l.split('\t').map(str::trim).collect();
            if f.len() >= 3 && !f[1].is_empty() {
                if let Some(i) = c.indexers.iter_mut().find(|i| i.name.trim() == f[0]) {
                    i.cat_list.push((f[1].to_string(), f[2].to_string()));
                }
            }
        }
        for l in nfo_lines {
            if let Some((n, v)) = l.split_once('\t') {
                if let Some(i) = c.indexers.iter_mut().find(|i| i.name.trim() == n.trim()) {
                    i.nfo_created = v.trim() != "false";
                }
            }
        }
        // profils d'API (3.21) ; sinon réglages de recherche de la 3.20, convertis
        for l in profile_lines {
            if let Some((n, j)) = l.split_once('\t') {
                if let (Some(i), Ok(p)) = (c.indexers.iter_mut().find(|i| i.name.trim() == n.trim()), crate::apidef::Profile::from_json(j)) {
                    i.api = p;
                }
            }
        }
        for i in c.indexers.iter_mut().filter(|i| i.api == crate::apidef::Profile::default()) {
            let s = search_lines.iter().map(|l| l.split('\t').collect::<Vec<&str>>()).find(|f| f.first().map(|x| x.trim()) == Some(i.name.trim()));
            let (mode, url) = s.map(|f| (f.get(1).map(|x| x.trim().to_string()).unwrap_or_default(), f.get(2).map(|x| x.trim().to_string()).unwrap_or_default())).unwrap_or_default();
            let actions: Vec<(String, String, String, String)> = action_lines
                .iter()
                .map(|l| l.split('\t').collect::<Vec<&str>>())
                .filter(|f| f.len() >= 4 && f[0].trim() == i.name.trim())
                .map(|f| (f[1].trim().to_string(), f[2].trim().to_string(), f[3].trim().to_string(), f.get(4).map(|x| x.trim().to_string()).unwrap_or_default()))
                .collect();
            if !mode.is_empty() || !actions.is_empty() {
                i.api = migrate_320(i, &mode, &url, &actions);
            }
        }
        for (n, f) in bdfield_lines {
            if let Some(i) = c.indexers.iter_mut().find(|i| i.name.trim() == n) {
                i.bdinfo_field = f;
            }
        }
        for n in catfr_lines {
            if let Some(i) = c.indexers.iter_mut().find(|i| i.name.trim() == n) {
                i.cat_lang_fr = true;
            }
        }
        for l in cat_lines {
            let f: Vec<&str> = l.split('\t').map(str::trim).collect();
            if f.len() >= 3 && !f[2].is_empty() {
                if let Some(i) = c.indexers.iter_mut().find(|i| i.name.trim() == f[0]) {
                    i.cat_map.push((f[1].to_string(), f[2].to_string()));
                }
            }
        }
        // une fois : cases vides des autres contenus remplies depuis la liste déjà importée de
        // chaque indexer (les versions d'avant ne remplissaient que films et séries)
        if cat_fill_rev < 1 {
            for i in c.indexers.iter_mut().filter(|i| !i.cat_list.is_empty()) {
                for (k, v) in crate::upload::suggest_content(&i.cat_list) {
                    if !i.cat_map.iter().any(|(x, _)| *x == k) {
                        i.cat_map.push((k, v));
                    }
                }
            }
        }
        for l in grp_lines {
            let f: Vec<&str> = l.split('\t').map(str::trim).collect();
            if let (Some(n), Some(cid), Some(g)) = (f.first(), f.get(1), f.get(2)) {
                if let Some(i) = c.indexers.iter_mut().find(|i| i.name.trim() == *n) {
                    if !cid.is_empty() && !g.is_empty() {
                        i.group_rules.push((cid.to_string(), g.to_string()));
                    }
                }
            }
        }
        for l in up_lines {
            let f: Vec<&str> = l.split('\t').collect();
            let g = |k: usize| f.get(k).map(|x| x.trim().to_string()).unwrap_or_default();
            if let Some(i) = c.indexers.iter_mut().find(|i| i.name.trim() == g(0)) {
                i.f_nzb = g(1);
                i.f_nfo = g(2);
                i.f_mi = g(3);
                i.groups_rule = if ["", "add", "off"].contains(&g(4).as_str()) { g(4) } else { String::new() };
                i.groups = g(5).split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect();
                i.release_page = g(6);
                i.doc_notes = g(7).split(" ¶ ").map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n");
            }
        }
        for l in api_lines {
            let f: Vec<&str> = l.split('\t').collect();
            let Some(i) = c.indexers.iter_mut().find(|i| Some(i.name.trim()) == f.first().map(|x| x.trim())) else { continue };
            let g = |k: usize| f.get(k).map(|x| x.trim().to_string()).unwrap_or_default();
            if ["web", "newznab", "rest", "cmd"].contains(&g(1).as_str()) {
                i.mode = g(1);
            }
            i.api_url = g(2);
            i.api_key = g(3);
            if !g(4).is_empty() {
                i.cat = g(4);
            }
            for (k, slot) in [(5, &mut i.cat_sd), (6, &mut i.cat_hd), (7, &mut i.cat_uhd)] {
                let v = g(k);
                if !v.is_empty() {
                    *slot = v;
                }
            }
            i.send_mi = g(8) != "false";
            // champs à place fixe ; la ligne se termine par « fin » pour garder les champs vides
            i.send_name = g(9) != "false";
            i.send_nfo = g(10) == "true";
            i.includemeta = g(11) != "false";
            i.dupecheck = g(12) != "false";
            i.extra_query = g(13);
            i.extra_fields = g(14);
            i.cmd = g(15);
            if f.len() >= 20 {
                if !g(16).is_empty() {
                    i.rest_path = g(16);
                }
                if ["header", "bearer", "query"].contains(&g(17).as_str()) {
                    i.auth = g(17);
                }
                if !g(18).is_empty() {
                    i.auth_name = g(18);
                }
            }
            if f.len() >= 22 {
                i.rest_url = g(19);
                i.rest_key = g(20);
            } else if i.mode == "rest" {
                // ancienne config : une seule adresse, celle du mode REST alors choisi
                i.rest_url = std::mem::take(&mut i.api_url);
            }
            if f.len() >= 25 {
                for (k, slot) in [(21, &mut i.cat_tv_sd), (22, &mut i.cat_tv_hd), (23, &mut i.cat_tv_uhd)] {
                    let v = g(k);
                    if !v.is_empty() {
                        *slot = v;
                    }
                }
            }
        }
        set_detail(c.log_detail);
        register_secrets(&c);
        c
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        register_secrets(self);
        let s = format!(
            "# NZB FF Tool Pro - configuration\n\
             inbox={}\nngpost_dir={}\nxml_dir={}\nmi_dest={}\nquarantine={}\n\
             group={}\ntemplate={}\nundefined_lang={}\n\
             mediainfo={}\nmi_format={}\n\
             interval={}\nstable_secs={}\n\
             transfer={}\nskip_duplicates={}\nautostart={}\naccent={}\n\
             ngpost_exe={}\nngpost_args={}\nngpost_autostart={}\nnzb_dir={}\nindexer_dir={}\n\
             post_mode={}\npost_args={}\npost_obfuscate={}\npost_move_done={}\npost_after={}\n",
            self.inbox, self.ngpost_dir, self.xml_dir, self.mi_dest, self.quarantine,
            self.group, self.template, self.undefined_lang,
            self.mediainfo, self.mi_format,
            self.interval, self.stable_secs,
            self.transfer, self.skip_duplicates, self.autostart, self.accent,
            self.ngpost_exe, self.ngpost_args, self.ngpost_autostart, self.nzb_dir, self.indexer_dir,
            self.post_mode, self.post_args, self.post_obfuscate, self.post_move_done, self.post_after
        );
        let mut s = s;
        s.push_str(&format!("bdinfo={}\nbdinfo_scan={}\nnfo_create={}\nnfo_keep={}\nnfo_send={}\nlog_detail={}\nlog_h_post={}\nlog_h_api={}\nlog_font={}\nlog_wrap={}\nui_expert={}\ncat_fill_rev=1\ncontent_cats_rev=2\nbd_sections={}\nbd_template={}\nnfo_bd={}\ndl_client={}\ndl_url={}\ndl_key={}\ndl_user={}\ndl_pass={}\ndl_cat={}\ndl_path_map={}\ndl_after={}\ndl_auto={}\ndl_origin={}\ndl_nfo={}\ndl_mi={}\ndl_watch={}\ndl_watch_on={}\nconfirm_recap={}\ndupe_check={}\nsetup_done={}\ndl_watch_after={}\ndl_watch_dupes={}\ndl_targets={}\nnzb_open={}\nshell_menu={}\nnzb_assoc={}\nng_par2_tool={}\nng_turbo={}\nng_parpar={}\n", self.bdinfo, self.bdinfo_scan, self.nfo_create, self.nfo_keep, self.nfo_send, self.log_detail, self.log_h_post, self.log_h_api, self.log_font, self.log_wrap, self.ui_expert, self.bd_sections.replace(['\r', '\n'], ""), esc_lines(&self.bd_template), self.nfo_bd, self.dl_client, one(&self.dl_url), one(&self.dl_key), one(&self.dl_user), one(&self.dl_pass), one(&self.dl_cat), one(&self.dl_path_map), self.dl_after, self.dl_auto, one(&self.dl_origin), self.dl_nfo, self.dl_mi, one(&self.dl_watch), self.dl_watch_on, self.confirm_recap, self.dupe_check, self.setup_done, self.dl_watch_after, self.dl_watch_dupes, one(&self.dl_targets.join(",")), self.nzb_open, self.shell_menu, self.nzb_assoc, self.ng_par2_tool, one(&self.ng_turbo), one(&self.ng_parpar)));
        s.push_str(&format!(
            "series_round_whole={}\nseries_round_seasons={}\nseries_round_episodes={}\n",
            self.series_round_whole, self.series_round_seasons, self.series_round_episodes
        ));
        s.push_str(&format!(
            "ng_imported={}\nng_compress={}\nng_gen_name={}\nng_gen_pass={}\nng_rar_pass={}\nng_rar_size={}\nng_rar_max={}\n\
             ng_par2={}\nng_par2_pct={}\nng_groups={}\nng_from={}\nng_threads={}\nng_article_size={}\nng_retry={}\n\
             ng_tmp_dir={}\nng_rar_path={}\nng_debug={}\npost_verify={}\naction={}\ntargets={}\napi_upload={}\n\
             close_action={}\nstart_in_tray_manual={}\ntray_notify={}\ntheme={}\nauto_backup={}\n\
             series_template={}\nepisode_template={}\ncomplete_word={}\nseries_rename={}\nseries_split={}\nseries_auto_complete={}\nng_lots={}\n\
             mail_enabled={}\nmail_server={}\nmail_port={}\nmail_security={}\nmail_user={}\nmail_pass={}\nmail_from={}\nmail_to={}\n",
            self.ng_imported, self.ng_compress, self.ng_gen_name, self.ng_gen_pass, self.ng_rar_pass, self.ng_rar_size, self.ng_rar_max,
            self.ng_par2, self.ng_par2_pct, self.ng_groups, self.ng_from, self.ng_threads, self.ng_article_size, self.ng_retry,
            self.ng_tmp_dir, self.ng_rar_path, self.ng_debug, self.post_verify, self.action, self.targets.join("|"), self.api_upload,
            self.close_action, self.start_in_tray, self.tray_notify, self.theme, self.auto_backup,
            self.series_template, self.episode_template, self.complete_word, self.series_rename, self.series_split, self.series_auto_complete, self.ng_lots,
            self.mail_enabled, self.mail_server, self.mail_port, self.mail_security, self.mail_user, self.mail_pass, self.mail_from, self.mail_to
        ));
        s.push_str(&format!("mail_subject={}\nmail_body={}\nmail_html={}\n", self.mail_subject.replace(['\r', '\n'], " "), esc_lines(&self.mail_body), self.mail_html));
        if !self.net_ua.trim().is_empty() {
            s.push_str(&format!("net_ua={}\n", self.net_ua.trim().replace(['\r', '\n'], " ")));
        }
        s.push_str(&format!("usenet_managed={}\nusenet_auto_best={}\nusenet_sync_conf={}\ngroups_film={}\ngroups_tv={}\ngroup_policy={}\n", self.usenet_managed, self.usenet_auto_best, self.usenet_sync_conf, self.groups_film, self.groups_tv, self.group_policy));
        s.push_str(&format!(
            "backup_on_change={}\nbackup_every_min={}\nbackup_on_close={}\nbackup_on_start={}\nbackup_keep={}\n",
            self.backup_on_change, self.backup_every_min, self.backup_on_close, self.backup_on_start, self.backup_keep
        ));
        for u in &self.usenet_servers {
            s.push_str(&u.line());
        }
        for m in &self.mail_providers {
            s.push_str(&format!("mail_provider={}|{}|{}|{}\n", m.name.replace('|', "/"), m.server.trim(), m.port, m.security));
        }
        for i in self.indexers.iter().filter(|i| !i.name.trim().is_empty()) {
            s.push_str(&format!("indexer={}|{}|{}\n", i.name.trim().replace('|', "/"), i.url.trim(), i.active));
        }
        for i in self.indexers.iter().filter(|i| !i.name.trim().is_empty()) {
            s.push_str(&i.api_line());
            for (k, v) in i.cat_map.iter().filter(|(_, v)| !v.trim().is_empty()) {
                s.push_str(&format!("indexer_cat={}\t{}\t{}\n", i.name.trim(), k, v.trim()));
            }
            for (id, n) in &i.cat_list {
                s.push_str(&format!("indexer_catlist={}\t{}\t{}\n", i.name.trim(), id, n.replace(['\t', '\n', '\r'], " ")));
            }
            if !i.bdinfo_field.trim().is_empty() {
                s.push_str(&format!("indexer_bdfield={}\t{}\n", i.name.trim(), i.bdinfo_field.trim().replace(['\t', '\n', '\r'], "")));
            }
            if i.cat_lang_fr {
                s.push_str(&format!("indexer_cat_fr={}\n", i.name.trim()));
            }
            if !i.nfo_created {
                s.push_str(&format!("indexer_nfo={}\tfalse\n", i.name.trim()));
            }
            if i.api != crate::apidef::Profile::default() {
                s.push_str(&format!("indexer_profile={}\t{}\n", i.name.trim(), i.api.to_json(true)));
            }
            if !i.up_default() {
                s.push_str(&i.up_line());
            }
            for (c, g) in i.group_rules.iter().filter(|(c, g)| !c.trim().is_empty() && !g.trim().is_empty()) {
                s.push_str(&format!("indexer_grp={}\t{}\t{}\n", i.name.trim(), c.trim(), g.replace(['\t', '\n', '\r', ' '], "")));
            }
        }
        s.push_str(&format!("api_probe_delay={}\napi_probe_max={}\n", self.api_probe_delay, self.api_probe_max));
        s.push_str(&format!("dl_watch_interval={}\ndl_watch_stable={}\ndl_watch_subdirs={}\ndl_watch_notify={}\ndl_prio={}\ndl_new_mode={}\n", self.dl_watch_interval, self.dl_watch_stable, self.dl_watch_subdirs, self.dl_watch_notify, self.dl_prio, self.dl_new_mode));
        for r in &self.dl_routes {
            s.push_str(&format!("dl_route={}\n", r.to_line()));
        }
        if self.content_cats.is_empty() {
            s.push_str("content_cats=none\n");
        }
        for c in &self.content_cats {
            s.push_str(&c.line());
        }
        fs::write(path, s)
    }

    /// Dossier de sortie (celui que surveille ton logiciel de post), s'il est renseigné.
    pub fn output_dir(&self) -> Option<PathBuf> {
        let o = self.ngpost_dir.trim();
        (!o.is_empty()).then(|| PathBuf::from(o))
    }

    /// Dossier de sortie indispensable quand aucune autre destination n'est choisie.
    pub fn ready_send(&self) -> Result<(), String> {
        if self.ngpost_dir.trim().is_empty() {
            return Err("dossier de sortie non renseigné (Réglages)".into());
        }
        Ok(())
    }

    /// Dossiers indispensables pour la surveillance.
    pub fn ready_watch(&self) -> Result<(), String> {
        self.ready_send()?;
        for (name, v) in [("dossier inbox", &self.inbox), ("dossier quarantaine", &self.quarantine)] {
            if v.trim().is_empty() {
                return Err(format!("{name} non renseigné (Réglages)"));
            }
        }
        Ok(())
    }

    pub fn make_dirs(&self) -> std::io::Result<()> {
        for d in [&self.inbox, &self.ngpost_dir, &self.xml_dir, &self.quarantine] {
            if !d.trim().is_empty() {
                fs::create_dir_all(d.trim())?;
            }
        }
        Ok(())
    }

    fn exe(&self) -> &str {
        if self.mediainfo.trim().is_empty() {
            "mediainfo"
        } else {
            self.mediainfo.trim()
        }
    }

    /// Extension du fichier d'export MediaInfo selon le format choisi.
    pub fn mi_ext(&self) -> &'static str {
        mi_spec(&self.mi_format).1
    }
}

/// Arguments MediaInfo et extension pour un format d'export.
pub fn mi_spec(key: &str) -> (&'static [&'static str], &'static str) {
    match key {
        "xml" => (&["--Output=XML"], "xml"),
        "json-full" => (&["--Full", "--Output=JSON"], "json"),
        "text-full" => (&["--Full"], "txt"),
        "text" => (&[], "txt"),
        "html" => (&["--Full", "--Output=HTML"], "html"),
        "oldxml" => (&["--Full", "--Output=OLDXML"], "xml"),
        "ebucore" => (&["--Output=EBUCore"], "xml"),
        _ => (&["--Full", "--Output=XML"], "xml"),
    }
}

// ----------------------------------------------------------------- MediaInfo

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub width: u32,
    pub height: u32,
    pub lib: String,
    pub format: String,
    /// Langue de chaque piste audio, normalisée ("fr", "en"…, "und" si non renseignée).
    pub langs: Vec<String>,
    /// Description de chaque piste audio (« fr (VFF) », « en »…).
    pub audio_desc: Vec<String>,
    /// Langue de chaque piste de sous-titres.
    pub subs: Vec<String>,
    pub duration: String,
    /// Tags du conteneur (souvent le nom de release d'origine).
    pub title_tag: String,
    pub movie_tag: String,
    /// Support d'origine déclaré (ex. "Blu-ray").
    pub source_medium: String,
    /// Version du format vidéo ("Version 2" pour le MPEG-2).
    pub format_version: String,
    /// Première piste audio : format, fonctions (XLL, JOC, 16-ch…), canaux.
    pub audio_format: String,
    pub audio_features: String,
    pub audio_channels: String,
}

const SEP: &str = "|~|";

/// Langue MediaInfo → code court ("fr", "en"…, "und").
pub fn norm_lang(s: &str) -> String {
    let l = s.trim().to_lowercase();
    let head = l.split(['-', '_', '/', ' ']).next().unwrap_or("").to_string();
    match head.as_str() {
        "" | "und" | "undetermined" | "zxx" | "mis" => "und".into(),
        "fre" | "fra" | "french" | "français" | "francais" => "fr".into(),
        "eng" | "english" | "anglais" => "en".into(),
        _ => head,
    }
}

/// Langue déduite du titre d'une piste (« VFF », « Français 5.1 », « English »…).
pub fn lang_from_title(title: &str) -> Option<&'static str> {
    let t = title.to_lowercase();
    let toks: Vec<&str> = t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |w: &[&str]| toks.iter().any(|t| w.contains(t));
    if has(&["vff", "vfq", "vfi", "vf", "vf2", "truefrench", "french", "français", "francais", "fra", "fre", "fr"]) {
        Some("fr")
    } else if has(&["english", "anglais", "eng", "en"]) {
        Some("en")
    } else if has(&["vo", "vost", "original"]) {
        Some("vo")
    } else {
        None
    }
}

/// Pistes au format « langue|~|titre » séparées par « |#| » (ou ancien format « fr;en; »).
/// Renvoie (langue normalisée, description lisible) pour chaque piste.
fn split_tracks(s: &str) -> Vec<(String, String)> {
    let parts: Vec<&str> = if s.contains("|#|") { s.split("|#|").collect() } else { s.split(';').collect() };
    let mut parts: Vec<&str> = parts.into_iter().map(str::trim).collect();
    // le dernier séparateur termine la dernière piste
    if parts.last().map(|x| x.is_empty()).unwrap_or(false) {
        parts.pop();
    }
    parts
        .into_iter()
        .map(|t| {
            let (raw, title) = t.split_once("|~|").unwrap_or((t, ""));
            let mut l = norm_lang(raw);
            if l == "und" {
                if let Some(g) = lang_from_title(title) {
                    l = g.to_string();
                }
            }
            let desc = if title.trim().is_empty() { l.clone() } else { format!("{l} ({})", title.trim()) };
            (l, desc)
        })
        .collect()
}

/// `video` : "1920|~|1080|~|x264|~|AVC|~|Blu-ray;"   `audio`/`text` : "fr;en;;"
/// `general` : "01:52:10.120|~|Titre|~|Film|~|Blu-ray"
pub fn parse_facts(video: &str, audio: &str, text: &str, general: &str) -> Facts {
    let first = video.split(';').next().unwrap_or("");
    let v: Vec<&str> = first.split(SEP).map(str::trim).collect();
    let g: Vec<&str> = general.trim_end_matches(';').split(SEP).map(str::trim).collect();
    let get = |a: &Vec<&str>, i: usize| a.get(i).map(|s| s.to_string()).unwrap_or_default();
    let mut source_medium = get(&g, 3);
    if source_medium.is_empty() {
        source_medium = get(&v, 4);
    }
    Facts {
        width: v.first().and_then(|s| s.parse().ok()).unwrap_or(0),
        height: v.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
        lib: get(&v, 2),
        format: get(&v, 3),
        langs: split_tracks(audio).into_iter().map(|t| t.0).collect(),
        audio_desc: split_tracks(audio).into_iter().map(|t| t.1).collect(),
        subs: split_tracks(text).into_iter().map(|t| t.0).collect(),
        duration: get(&g, 0).split('.').next().unwrap_or("").to_string(),
        title_tag: get(&g, 1),
        movie_tag: get(&g, 2),
        source_medium,
        format_version: get(&v, 5),
        ..Default::default()
    }
}

fn mediainfo_cmd(exe: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW : pas de console qui clignote
    }
    cmd
}

fn run_mediainfo(exe: &str, args: &[&str], target: Option<&Path>, cwd: Option<&Path>) -> Result<String, String> {
    let mut cmd = mediainfo_cmd(exe);
    cmd.args(args);
    if let Some(p) = target {
        cmd.arg(p);
    }
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let out = cmd.output().map_err(|e| match e.kind() {
        ErrorKind::NotFound => "MediaInfo CLI introuvable (Réglages → MediaInfo)".to_string(),
        _ => format!("MediaInfo : {e}"),
    })?;
    if !out.status.success() {
        return Err(format!("MediaInfo : {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Version de MediaInfo CLI, pour le bouton « Tester ».
pub fn mediainfo_version(cfg: &Config) -> Result<String, String> {
    let out = run_mediainfo(cfg.exe(), &["--Version"], None, None)?;
    let v = out.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("").to_string();
    if v.is_empty() {
        Err("réponse vide : est-ce bien la version CLI ?".into())
    } else {
        Ok(v)
    }
}

pub fn query_facts(cfg: &Config, path: &Path) -> Result<Facts, String> {
    let q = |tpl: &str| run_mediainfo(cfg.exe(), &[&format!("--Inform={tpl}")], Some(path), None);
    let video = q("Video;%Width%|~|%Height%|~|%Encoded_Library_Name%|~|%Format%|~|%OriginalSourceMedium%|~|%Format_Version%;")?;
    let audio = q("Audio;%Language%|~|%Title%|#|")?;
    let text = q("Text;%Language%|~|%Title%|#|").unwrap_or_default();
    let general = q("General;%Duration/String3%|~|%Title%|~|%Movie%|~|%OriginalSourceMedium%").unwrap_or_default();
    let mut f = parse_facts(&video, &audio, &text, &general);
    // première piste audio : format et canaux (repère {audio})
    let tech = q("Audio;%Format%|~|%Format_AdditionalFeatures%|~|%Channel(s)%|#|").unwrap_or_default();
    if let Some(first) = tech.split("|#|").map(str::trim).find(|t| !t.is_empty()) {
        let a: Vec<&str> = first.split(SEP).map(str::trim).collect();
        f.audio_format = a.first().unwrap_or(&"").to_string();
        f.audio_features = a.get(1).unwrap_or(&"").to_string();
        f.audio_channels = a.get(2).unwrap_or(&"").to_string();
    }
    if f.width == 0 && f.height == 0 {
        return Err("aucune piste vidéo lisible par MediaInfo".into());
    }
    Ok(f)
}

/// Rapport MediaInfo en texte lisible, pour l'affichage dans l'interface.
pub fn mediainfo_text(cfg: &Config, path: &Path, full: bool) -> Result<String, String> {
    let args: &[&str] = if full { &["--Full"] } else { &[] };
    let name = path.file_name().map(PathBuf::from).ok_or("nom de fichier invalide")?;
    run_mediainfo(cfg.exe(), args, Some(&name), path.parent())
}

/// Export MediaInfo (format choisi) d'un fichier, lancé DANS son dossier :
/// le rapport ne contient que le nom du fichier, pas ton chemin local.
pub fn mediainfo_export(cfg: &Config, file: &Path) -> Result<String, String> {
    // rapport modifié à la main avant le traitement : livré tel quel
    if let Some(t) = cfg.mi_override.as_ref().filter(|t| !t.trim().is_empty()) {
        return Ok(t.clone());
    }
    let (args, _) = mi_spec(&cfg.mi_format);
    let name = file.file_name().map(PathBuf::from).ok_or("nom de fichier invalide")?;
    run_mediainfo(cfg.exe(), args, Some(&name), file.parent())
}

// -------------------------------------------------------------------- Nommage

pub fn resolution(f: &Facts) -> String {
    let (w, h) = (f.width, f.height);
    if w >= 3800 || h >= 2100 {
        "2160p".into()
    } else if w >= 1900 || h >= 1000 {
        "1080p".into()
    } else if w >= 1260 || h >= 700 {
        "720p".into()
    } else if h > 0 {
        format!("{h}p")
    } else {
        String::new()
    }
}

/// x264 / x265 / H264 / H265 / AV1, et pour les DVD et vieux encodages MPEG2 / VC-1 / XviD.
/// Un format inconnu est omis.
pub fn video_codec(f: &Facts) -> String {
    let lib = f.lib.to_lowercase();
    match f.format.as_str() {
        "AVC" if lib == "x264" => "x264".into(),
        "HEVC" if lib == "x265" => "x265".into(),
        "AVC" => "H264".into(),
        "HEVC" => "H265".into(),
        "AV1" => "AV1".into(),
        "MPEG Video" if f.format_version.contains('1') => "MPEG1".into(),
        "MPEG Video" => "MPEG2".into(),
        "VC-1" => "VC-1".into(),
        "MPEG-4 Visual" if lib.contains("xvid") => "XviD".into(),
        "MPEG-4 Visual" if lib.contains("divx") => "DivX".into(),
        _ => String::new(),
    }
}

/// Piste audio au format des releases : « AC3.2.0 », « DTS-HD.MA.5.1 », « TrueHD.7.1.Atmos »,
/// « DDP5.1 »… Vide si MediaInfo ne dit rien.
pub fn audio_tag(f: &Facts) -> String {
    let fmt = f.audio_format.trim();
    if fmt.is_empty() {
        return String::new();
    }
    let feat = f.audio_features.to_uppercase();
    let ch: u32 = f.audio_channels.split(|c: char| !c.is_ascii_digit()).find(|x| !x.is_empty()).and_then(|x| x.parse().ok()).unwrap_or(0);
    let layout = match ch {
        0 => String::new(),
        1 => "1.0".into(),
        2 => "2.0".into(),
        3 => "2.1".into(),
        6 => "5.1".into(),
        7 => "6.1".into(),
        8 => "7.1".into(),
        n => format!("{}.1", n.saturating_sub(1)),
    };
    let join = |base: &str, extra: &str| {
        let mut v = vec![base.to_string()];
        if !layout.is_empty() {
            v.push(layout.clone());
        }
        if !extra.is_empty() {
            v.push(extra.to_string());
        }
        v.join(".")
    };
    match fmt {
        "AC-3" => join("AC3", ""),
        "E-AC-3" if feat.contains("JOC") => {
            if layout.is_empty() { "DDP.Atmos".into() } else { format!("DDP{layout}.Atmos") }
        }
        "E-AC-3" => if layout.is_empty() { "DDP".into() } else { format!("DDP{layout}") },
        "MLP FBA" | "TrueHD" => join("TrueHD", if feat.contains("16-CH") || feat.contains("ATMOS") { "Atmos" } else { "" }),
        "DTS" if feat.contains("XLL X") || feat.contains("IMAX") => join("DTS-X", ""),
        "DTS" if feat.contains("XLL") => join("DTS-HD.MA", ""),
        "DTS" if feat.contains("XBR") => join("DTS-HD.HRA", ""),
        "DTS" => join("DTS", ""),
        "AAC" => join("AAC", ""),
        "FLAC" => join("FLAC", ""),
        "PCM" => join("LPCM", ""),
        "Opus" => join("Opus", ""),
        "MPEG Audio" => join("MP3", ""),
        _ => String::new(),
    }
}

/// FRENCH / MULTi / VOSTFR, ou la langue par défaut si les pistes ne sont pas renseignées.
/// Une piste sans langue renseignée compte comme française si `undefined_default` est réglé.
pub fn language_tag(audio: &[String], subs: &[String], undefined_default: &str) -> String {
    let und_is_fr = !undefined_default.trim().is_empty();
    let fr = audio.iter().any(|l| l == "fr" || (und_is_fr && l == "und"));
    let other = audio.iter().any(|l| l != "fr" && l != "und");
    if fr && other {
        "MULTi".into()
    } else if fr {
        "FRENCH".into()
    } else if !other {
        undefined_default.trim().to_string()
    } else if subs.iter().any(|l| l == "fr") {
        "VOSTFR".into()
    } else {
        String::new()
    }
}

/// Dernière année plausible (19xx / 20xx) entourée de non-chiffres, pas en tout début de nom.
pub fn title_year(stem: &str) -> (String, String) {
    let chars: Vec<char> = stem.chars().collect();
    let mut found: Option<(usize, String)> = None;
    let mut i = 1;
    while i + 4 <= chars.len() {
        let w: String = chars[i..i + 4].iter().collect();
        let before_ok = !chars[i - 1].is_ascii_digit();
        let after_ok = i + 4 == chars.len() || !chars[i + 4].is_ascii_digit();
        if before_ok && after_ok && w.chars().all(|c| c.is_ascii_digit()) && (w.starts_with("19") || w.starts_with("20")) {
            found = Some((i, w));
        }
        i += 1;
    }
    let (end, year) = match found {
        Some((i, y)) => (i, y),
        None => {
            // pas d'année : le titre s'arrête au premier mot de release (langue, source…)
            let mut pos = 0;
            let mut cut = chars.len();
            for w in stem.split(|c: char| !c.is_alphanumeric() && c != '-').collect::<Vec<_>>() {
                if pos > 0 && !w.is_empty() && scene_kind(w).is_some() {
                    cut = stem[..pos].chars().count();
                    break;
                }
                pos += w.len() + 1;
                if pos > stem.len() {
                    break;
                }
            }
            (cut, String::new())
        }
    };
    let raw: String = chars[..end].iter().collect();
    let cleaned: String = raw.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    (cleaned.split_whitespace().collect::<Vec<_>>().join("."), year)
}

/// Un nom « scene » : sans espace, année, résolution, et tag `-GROUPE`.
pub fn is_release_name(stem: &str) -> bool {
    if stem.is_empty() || stem.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    let Some(dash) = stem.rfind('-') else {
        return false;
    };
    let tag = &stem[dash + 1..];
    if tag.len() < 2 || !tag.chars().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    let parts: Vec<&str> = stem[..dash].split('.').collect();
    let is_year = |p: &str| p.len() == 4 && p.chars().all(|c| c.is_ascii_digit()) && (p.starts_with("19") || p.starts_with("20"));
    let is_res = |p: &str| matches!(p.to_lowercase().as_str(), "480p" | "576p" | "720p" | "1080p" | "2160p");
    if let Some(y) = parts.iter().position(|p| is_year(p)) {
        if y >= 1 && parts[y + 1..].iter().any(|p| is_res(p)) {
            return true;
        }
    }
    // sans année ni résolution (DVD, vieux films) : au moins une langue ou une source, et trois
    // mots de release en tout après le titre (ex. Film.FRENCH.VOF.DVDRemux.AC3.2.0.MPEG2-LiHDL)
    if parts.len() < 3 {
        return false;
    }
    let kinds: Vec<Scene> = parts[1..].iter().filter_map(|p| scene_kind(p)).collect();
    kinds.len() >= 3 && kinds.iter().any(|k| matches!(k, Scene::Lang | Scene::Source))
}

pub fn release_tag(name: &str) -> String {
    name.rsplit_once('-').map(|(_, t)| t.to_string()).unwrap_or_default()
}

const LANG_TOKENS: &[&str] = &["french", "truefrench", "multi", "vff", "vfq", "vfi", "vf2", "vof", "vostfr", "subfrench", "vf", "vo"];

/// Nature d'un mot d'un nom de release.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scene {
    Lang,
    Res,
    Source,
    Codec,
    Audio,
    /// HDR, 10bit, REPACK, PROPER, EXTENDED… : gardé tel quel.
    Extra,
}

const EXTRA_TOKENS: &[&str] = &[
    "hdr", "hdr10", "hdr10plus", "hdr10+", "dv", "dovi", "sdr", "10bit", "8bit", "hfr", "3d", "hsbs", "hou", "imax", "repack", "proper", "rerip", "internal", "extended", "unrated",
    "remastered", "uncut", "directors", "dc", "limited", "final", "custom", "subforced", "hybrid", "complete", "uhd", "pal", "ntsc", "dual", "light", "mhd",
];

fn audio_word(l: &str) -> bool {
    let bases = ["ac3", "eac3", "dd", "ddp", "dd+", "dts", "dtshd", "dts-hd", "dts-x", "dtsx", "truehd", "atmos", "aac", "flac", "lpcm", "pcm", "opus", "mp3", "ma", "hra", "dts-hdma"];
    if bases.contains(&l) {
        return true;
    }
    // « ddp5 », « dd5 », « aac2 », « eac3 » suivis de chiffres (DDP5.1 coupé en DDP5 / 1)
    ["ddp", "dd", "aac", "ac3", "eac3", "dts", "truehd", "flac"].iter().any(|b| l.strip_prefix(b).map(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit())).unwrap_or(false))
}

/// Nature d'un mot (insensible à la casse) ; None : mot du titre ou inconnu.
pub fn scene_kind(word: &str) -> Option<Scene> {
    let l = word.to_lowercase();
    if LANG_TOKENS.contains(&l.as_str()) {
        Some(Scene::Lang)
    } else if matches!(l.as_str(), "480p" | "576p" | "720p" | "1080p" | "1080i" | "2160p" | "4k") {
        Some(Scene::Res)
    } else if matches!(l.as_str(), "x264" | "x265" | "h264" | "h265" | "hevc" | "avc" | "av1" | "mpeg2" | "mpeg-2" | "mpeg1" | "vc-1" | "vc1" | "xvid" | "divx" | "vp9") {
        Some(Scene::Codec)
    } else if source_from_text(word).is_some() || matches!(l.as_str(), "bluray" | "dvd" | "dvdr" | "dvd5" | "dvd9" | "bdmv") {
        Some(Scene::Source)
    } else if audio_word(&l) {
        Some(Scene::Audio)
    } else if EXTRA_TOKENS.contains(&l.as_str()) {
        Some(Scene::Extra)
    } else {
        None
    }
}

/// Ajoute la langue après l'année si le nom n'en contient aucune (l'indexer en a besoin).
pub fn ensure_lang(name: &str, lang: &str) -> String {
    if lang.is_empty() {
        return name.to_string();
    }
    let (body, tag) = match name.rsplit_once('-') {
        Some((b, t)) => (b, format!("-{t}")),
        None => (name, String::new()),
    };
    let parts: Vec<&str> = body.split('.').collect();
    if parts.iter().any(|p| LANG_TOKENS.contains(&p.to_lowercase().as_str())) {
        return name.to_string();
    }
    let is_year = |p: &str| p.len() == 4 && p.chars().all(|c| c.is_ascii_digit()) && (p.starts_with("19") || p.starts_with("20"));
    let Some(y) = parts.iter().rposition(|p| is_year(p)) else {
        return name.to_string();
    };
    let mut out: Vec<&str> = parts[..=y].to_vec();
    out.push(lang);
    out.extend_from_slice(&parts[y + 1..]);
    format!("{}{tag}", out.join("."))
}

/// Source reconnue dans un texte (nom de fichier, tag MKV…).
pub fn source_from_text(s: &str) -> Option<&'static str> {
    let t: String = s.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { ' ' }).collect();
    let toks: Vec<&str> = t.split_whitespace().flat_map(|w| std::iter::once(w).chain(w.split('-'))).collect();
    let has = |w: &str| toks.contains(&w);
    let joined = format!(" {} ", t.split_whitespace().collect::<Vec<_>>().join(" "));
    if has("dvdremux") || (has("dvd") && has("remux")) {
        Some("DVDRemux")
    } else if has("remux") || has("bdremux") {
        Some("REMUX")
    } else if has("bdrip") {
        Some("BDRip")
    } else if has("brrip") {
        Some("BRRip")
    } else if (has("complete") || has("bdmv")) && (has("bluray") || has("blu-ray")) {
        Some(if has("uhd") { "COMPLETE.UHD.BLURAY" } else { "COMPLETE.BLURAY" })
    } else if has("bluray") || has("blu-ray") || joined.contains(" blu ray ") {
        Some("BluRay")
    } else if has("web-dl") || has("webdl") || joined.contains(" web dl ") {
        Some("WEB-DL")
    } else if has("webrip") || has("web-rip") {
        Some("WEBRip")
    } else if has("hdlight") || has("4klight") {
        Some("HDLight")
    } else if has("hdtv") {
        Some("HDTV")
    } else if has("dvdrip") {
        Some("DVDRip")
    } else if has("dvdr") || has("dvd9") || has("dvd5") {
        Some("DVDR")
    } else if has("dvd") {
        Some("DVD")
    } else if s.split(|c: char| !c.is_alphanumeric()).any(|w| w == "WEB") {
        Some("WEB")
    } else {
        None
    }
}

/// Support d'origine MediaInfo ("Blu-ray", "DVD-Video"…) → source.
pub fn source_from_medium(m: &str) -> Option<&'static str> {
    let l = m.to_lowercase();
    if l.is_empty() {
        None
    } else if l.contains("blu") {
        Some("BluRay")
    } else if l.contains("dvd") {
        Some("DVDRip")
    } else {
        None
    }
}

fn parent_name(path: &Path) -> String {
    path.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn dotted(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(".")
}

/// Cherche la source : tags MediaInfo, support d'origine, nom du fichier, dossier parent.
pub fn detect_source(path: &Path, f: &Facts) -> Option<(&'static str, &'static str)> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(s) = source_from_text(&format!("{} {}", f.title_tag, f.movie_tag)) {
        return Some((s, "titre MediaInfo"));
    }
    if let Some(s) = source_from_medium(&f.source_medium) {
        return Some((s, "support d'origine MediaInfo"));
    }
    if let Some(s) = source_from_text(&stem) {
        return Some((s, "nom du fichier"));
    }
    source_from_text(&parent_name(path)).map(|s| (s, "dossier parent"))
}

/// Un nom de release complet déjà présent : nom du fichier, tags MKV ou dossier parent.
pub fn find_release_name(path: &Path, f: &Facts) -> Option<(String, &'static str)> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let cands = [
        (stem, "nom du fichier"),
        (dotted(&f.title_tag), "titre MediaInfo"),
        (dotted(&f.movie_tag), "tag « Movie » MediaInfo"),
        (parent_name(path), "dossier parent"),
    ];
    cands.into_iter().find(|(c, _)| is_release_name(c)).map(|(c, o)| (clean_name(&c), o))
}

pub fn clean_name(name: &str) -> String {
    name.trim().replace(' ', ".").chars().filter(|c| !"\\/:*?\"<>|".contains(*c)).collect()
}

pub struct NameParts<'a> {
    pub title: &'a str,
    pub year: &'a str,
    pub lang: &'a str,
    pub resolution: &'a str,
    pub source: &'a str,
    pub codec: &'a str,
    pub tag: &'a str,
    /// Piste audio (« AC3.2.0 ») et mots en plus (« HDR.10bit »), placés avant le codec si le
    /// modèle n'a pas de repère {audio} / {extra}.
    pub audio: &'a str,
    pub extra: &'a str,
}

/// Applique le modèle ; les éléments vides disparaissent proprement (pas de « .. » ni de « - » final).
pub fn render_template(tpl: &str, p: &NameParts) -> String {
    let tpl = if tpl.trim().is_empty() { DEFAULT_TEMPLATE } else { tpl };
    let mut tpl = tpl.to_string();
    for (key, val) in [("{extra}", p.extra), ("{audio}", p.audio)] {
        if !val.trim().is_empty() && !tpl.contains(key) {
            tpl = if tpl.contains("{codec}") {
                tpl.replacen("{codec}", &format!("{key}.{{codec}}"), 1)
            } else if tpl.contains("-{tag}") {
                tpl.replacen("-{tag}", &format!(".{key}-{{tag}}"), 1)
            } else {
                format!("{tpl}.{key}")
            };
        }
    }
    // dans l'ordre des releases : …source.extra.audio.codec-tag
    if let (Some(a), Some(e)) = (tpl.find("{audio}"), tpl.find("{extra}")) {
        if e > a && !p.extra.is_empty() && !p.audio.is_empty() {
            tpl = tpl.replace("{extra}", "\u{1}").replace("{audio}", "{extra}").replace('\u{1}', "{audio}");
        }
    }
    let s = tpl
        .replace("{audio}", p.audio)
        .replace("{extra}", p.extra)
        .replace("{titre}", p.title)
        .replace("{annee}", p.year)
        .replace("{année}", p.year)
        .replace("{langue}", p.lang)
        .replace("{resolution}", p.resolution)
        .replace("{résolution}", p.resolution)
        .replace("{source}", p.source)
        .replace("{codec}", p.codec)
        .replace("{tag}", p.tag);
    let s = clean_name(&s);
    let mut s = s.split('.').filter(|x| !x.is_empty()).collect::<Vec<_>>().join(".");
    while s.contains("-.") || s.contains(".-") || s.contains("--") {
        s = s.replace("-.", ".").replace(".-", "-").replace("--", "-");
    }
    s.trim_matches(|c| c == '.' || c == '-').to_string()
}

/// Les éléments du nom, tous modifiables dans l'interface.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fields {
    pub title: String,
    pub year: String,
    pub lang: String,
    pub resolution: String,
    pub source: String,
    pub codec: String,
    pub tag: String,
    /// Piste audio (« AC3.2.0 ») et autres mots du nom d'origine (« HDR.10bit »).
    pub audio: String,
    pub extra: String,
}

impl Fields {
    pub fn render(&self, template: &str) -> String {
        render_template(
            template,
            &NameParts {
                title: self.title.trim(),
                year: self.year.trim(),
                lang: self.lang.trim(),
                resolution: self.resolution.trim(),
                source: self.source.trim(),
                codec: self.codec.trim(),
                tag: self.tag.trim(),
                audio: self.audio.trim(),
                extra: self.extra.trim(),
            },
        )
    }

    /// Éléments indispensables encore vides.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut m = Vec::new();
        for (v, n) in [(&self.title, "titre"), (&self.year, "année"), (&self.lang, "langue"), (&self.resolution, "résolution")] {
            if v.trim().is_empty() {
                m.push(n);
            }
        }
        m
    }

    /// Complète les éléments vides avec ceux de `other`.
    pub fn fill_from(&mut self, other: &Fields) {
        for (a, b) in [
            (&mut self.title, &other.title),
            (&mut self.year, &other.year),
            (&mut self.lang, &other.lang),
            (&mut self.resolution, &other.resolution),
            (&mut self.source, &other.source),
            (&mut self.codec, &other.codec),
            (&mut self.tag, &other.tag),
            (&mut self.audio, &other.audio),
            (&mut self.extra, &other.extra),
        ] {
            if a.is_empty() {
                *a = b.clone();
            }
        }
    }
}

/// Découpe un nom de release (Titre.Année.LANGUE.1080p.Source.Codec-TAG) en éléments.
/// Sans année (DVD…), le titre s'arrête au premier mot de release. Plusieurs langues
/// (FRENCH.VOF, MULTi.VFF) restent ensemble ; audio (AC3.2.0, DTS-HD.MA.5.1) et mots en plus
/// (HDR, 10bit, REPACK…) sont gardés.
pub fn parse_release(name: &str) -> Fields {
    let (body, tag) = match name.rsplit_once('-') {
        Some((b, t)) if !t.contains('.') && !t.is_empty() => (b, t.to_string()),
        _ => (name, String::new()),
    };
    // « DTS-HD.MA » : le tiret appartient à l'audio, pas au tag
    let (body, tag) = if audio_word(&tag.to_lowercase()) || tag.eq_ignore_ascii_case("HD") || tag.eq_ignore_ascii_case("DL") { (name, String::new()) } else { (body, tag) };
    let parts: Vec<&str> = body.split('.').filter(|p| !p.is_empty()).collect();
    let is_year = |p: &str| p.len() == 4 && p.chars().all(|c| c.is_ascii_digit()) && (p.starts_with("19") || p.starts_with("20"));
    let mut f = Fields { tag, ..Default::default() };
    let y = parts.iter().enumerate().skip(1).filter(|(_, p)| is_year(p)).map(|(i, _)| i).last();
    let rest: &[&str] = match y {
        Some(i) => {
            f.title = parts[..i].join(".");
            f.year = parts[i].to_string();
            &parts[i + 1..]
        }
        None => match parts.iter().enumerate().skip(1).find(|(_, p)| scene_kind(p).is_some()).map(|(i, _)| i) {
            Some(i) => {
                f.title = parts[..i].join(".");
                &parts[i..]
            }
            None => &parts[..0],
        },
    };
    let (mut langs, mut audio, mut extra): (Vec<&str>, Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new(), Vec::new());
    let mut i = 0;
    while i < rest.len() {
        let p = rest[i];
        let l = p.to_lowercase();
        match scene_kind(p) {
            Some(Scene::Lang) => langs.push(p),
            Some(Scene::Res) if f.resolution.is_empty() => f.resolution = p.to_string(),
            Some(Scene::Codec) if f.codec.is_empty() => f.codec = p.to_string(),
            Some(Scene::Audio) => {
                audio.push(p);
                // canaux « 2.0 », « 5.1 » (coupés par les points), « Atmos », « MA »
                while i + 1 < rest.len() {
                    let n = rest[i + 1];
                    let nl = n.to_lowercase();
                    if (n.len() == 1 && n.chars().all(|c| c.is_ascii_digit())) || matches!(nl.as_str(), "ma" | "hra" | "atmos" | "x") {
                        audio.push(n);
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            Some(Scene::Source) if f.source.is_empty() => {
                // « COMPLETE.BLURAY » / « COMPLETE.UHD.BLURAY »
                f.source = p.to_string();
            }
            Some(Scene::Extra) if l == "complete" || l == "uhd" => {
                let mut j = i;
                let mut words = Vec::new();
                while j < rest.len() && matches!(rest[j].to_lowercase().as_str(), "complete" | "uhd" | "bluray" | "blu-ray") {
                    words.push(rest[j]);
                    j += 1;
                }
                if f.source.is_empty() && words.iter().any(|w| w.eq_ignore_ascii_case("bluray") || w.eq_ignore_ascii_case("blu-ray")) {
                    f.source = words.join(".");
                    i = j - 1;
                } else {
                    extra.push(p);
                }
            }
            Some(_) => extra.push(p),
            None => {
                // mot inconnu après le titre : gardé (ex. « iNTERNAL » mal orthographié)
                if !(p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit())) {
                    extra.push(p);
                }
            }
        }
        i += 1;
    }
    f.lang = langs.join(".");
    f.audio = audio.join(".");
    f.extra = extra.join(".");
    f
}

// ------------------------------------------------------------------- Analyse

/// Tout ce qu'on sait d'un fichier avant de l'envoyer (affiché dans l'interface).
#[derive(Clone, Debug)]
pub struct Analysis {
    pub path: PathBuf,
    pub size: u64,
    pub network: bool,
    pub facts: Option<Facts>,
    pub title: String,
    pub year: String,
    pub lang: String,
    pub resolution: String,
    pub codec: String,
    pub source: String,
    /// D'où vient la source ("titre MediaInfo", "nom du fichier"…), vide si omise.
    pub source_origin: String,
    pub group: String,
    /// D'où vient le nom ("nom du fichier", "titre MediaInfo"…, ou "reconstruit").
    pub name_origin: String,
    pub kept_original_name: bool,
    pub forced: bool,
    /// Éléments du nom (repris du nom de release existant, sinon détectés).
    pub fields: Fields,
    pub name: Result<String, String>,
}

impl Analysis {
    pub fn file_name(&self) -> String {
        file_name(&self.path)
    }
}

pub fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Analyse complète. `source_override` : Some("") = pas de source, Some("BluRay") = imposée, None = auto.
pub fn analyze(path: &Path, cfg: &Config, forced: Option<&str>, source_override: Option<&str>) -> Analysis {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let (title, year) = title_year(&stem);
    let mut a = Analysis {
        path: path.to_path_buf(),
        size: fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        network: is_network_path(path),
        facts: None,
        title,
        year,
        lang: String::new(),
        resolution: String::new(),
        codec: String::new(),
        source: String::new(),
        source_origin: String::new(),
        group: cfg.group.trim().to_string(),
        name_origin: String::new(),
        kept_original_name: false,
        forced: false,
        fields: Fields::default(),
        name: Err(String::new()),
    };
    let f = match query_facts(cfg, path) {
        Ok(f) => f,
        Err(e) => {
            a.name = Err(e);
            return a;
        }
    };
    a.lang = language_tag(&f.langs, &f.subs, &cfg.undefined_lang);
    a.resolution = resolution(&f);
    a.codec = video_codec(&f);
    match source_override {
        Some(s) => {
            a.source = s.trim().to_string();
            a.source_origin = if a.source.is_empty() { String::new() } else { "choisie à la main".into() };
        }
        None => {
            // jamais inventée : uniquement si on en trouve une trace
            if let Some((s, o)) = detect_source(path, &f) {
                a.source = s.into();
                a.source_origin = o.into();
            }
        }
    }

    let detected = Fields {
        title: a.title.clone(),
        year: a.year.clone(),
        lang: a.lang.clone(),
        resolution: a.resolution.clone(),
        source: a.source.clone(),
        codec: a.codec.clone(),
        tag: a.group.clone(),
        // piste audio seulement si le modèle la demande (repère {audio})
        audio: if cfg.template.contains("{audio}") { audio_tag(&f) } else { String::new() },
        extra: String::new(),
    };
    a.fields = detected.clone();
    if let Some(n) = forced.map(clean_name).filter(|n| !n.is_empty()) {
        a.forced = true;
        a.name_origin = "saisi à la main".into();
        a.name = Ok(n);
    } else if let Some((rel, origin)) = find_release_name(path, &f) {
        a.kept_original_name = true;
        a.name_origin = origin.into();
        a.group = release_tag(&rel);
        let named = ensure_lang(&rel, &a.lang);
        let mut fields = parse_release(&named);
        fields.fill_from(&Fields { tag: String::new(), ..detected });
        a.fields = fields;
        a.name = Ok(named);
    } else {
        a.name_origin = "reconstruit".into();
        a.name = if a.title.is_empty() {
            Err("titre introuvable dans le nom du fichier".into())
        } else if a.year.is_empty() {
            Err("année introuvable dans le nom du fichier".into())
        } else if a.lang.is_empty() {
            Err("aucune piste audio française (ni sous-titres français)".into())
        } else if a.resolution.is_empty() {
            Err("résolution introuvable".into())
        } else {
            Ok(render_template(
                &cfg.template,
                &NameParts {
                    title: &a.title,
                    year: &a.year,
                    lang: &a.lang,
                    resolution: &a.resolution,
                    source: &a.source,
                    codec: &a.codec,
                    tag: &a.group,
                    audio: &a.fields.audio,
                    extra: "",
                },
            ))
        };
    }
    a.facts = Some(f);
    a
}

// --------------------------------------------------------------- Transferts

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Transfer {
    Move,
    Copy,
    /// Lien physique (même disque) : une deuxième release des mêmes fichiers sans les copier ;
    /// copie si le disque ne le permet pas. L'original reste toujours.
    Link,
}

/// Place une copie de `from` en `to` : lien physique si `how` le demande et que le disque le
/// permet (instantané, aucune place prise), sinon copie suivie.
pub fn place_copy(from: &Path, to: &Path, how: Transfer, progress: &mut dyn FnMut(u64, u64)) -> std::io::Result<()> {
    if how == Transfer::Link && fs::hard_link(from, to).is_ok() {
        let n = fs::metadata(to).map(|m| m.len()).unwrap_or(0);
        progress(n, n);
        return Ok(());
    }
    copy_with_progress(from, to, progress)
}

/// Lecteur réseau (\\serveur\partage, ou lettre de lecteur réseau) ?
#[cfg(windows)]
pub fn is_network_path(p: &Path) -> bool {
    let s = p.display().to_string();
    if s.starts_with("\\\\?\\UNC\\") || (s.starts_with("\\\\") && !s.starts_with("\\\\?\\")) {
        return true;
    }
    let s = s.trim_start_matches("\\\\?\\");
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetDriveTypeW(root: *const u16) -> u32;
        }
        let root: Vec<u16> = format!("{}:\\", b[0] as char).encode_utf16().chain(Some(0)).collect();
        const DRIVE_REMOTE: u32 = 4;
        return unsafe { GetDriveTypeW(root.as_ptr()) } == DRIVE_REMOTE;
    }
    false
}

/// Lecteur réseau : point de montage NFS / SMB / SSHFS.
#[cfg(not(windows))]
pub fn is_network_path(p: &Path) -> bool {
    let Ok(mounts) = fs::read_to_string("/proc/mounts") else {
        return false;
    };
    let abs = fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let mut best: Option<(usize, bool)> = None;
    for line in mounts.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 3 {
            continue;
        }
        let (mnt, fstype) = (f[1], f[2]);
        if abs.starts_with(mnt) && best.map(|(l, _)| mnt.len() > l).unwrap_or(true) {
            let net = matches!(fstype, "nfs" | "nfs4" | "cifs" | "smb3" | "smbfs" | "fuse.sshfs" | "9p");
            best = Some((mnt.len(), net));
        }
    }
    best.map(|(_, n)| n).unwrap_or(false)
}

/// Mode de transfert pour ce fichier selon le réglage.
pub fn transfer_for(path: &Path, cfg: &Config) -> Transfer {
    match cfg.transfer.as_str() {
        "copy" => Transfer::Copy,
        "move" => Transfer::Move,
        _ if is_network_path(path) => Transfer::Copy,
        _ => Transfer::Move,
    }
}

pub struct Done {
    pub dest: PathBuf,
    pub xml: PathBuf,
    /// Originaux qui devaient être supprimés (déplacement entre deux disques) et sont restés.
    pub left: Vec<(PathBuf, String)>,
}

/// Supprime l'original après une copie réussie (déplacement d'un disque à l'autre). Un
/// fichier en lecture seule est d'abord rendu modifiable ; un fichier encore ouvert ailleurs
/// (antivirus, aperçu de l'Explorateur) est réessayé quelques secondes.
pub fn remove_original(p: &Path) -> Result<(), String> {
    let mut last = String::new();
    for i in 0..12 {
        match fs::remove_file(p) {
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                last = e.to_string();
                if let Ok(m) = fs::metadata(p) {
                    let mut perm = m.permissions();
                    if perm.readonly() {
                        #[allow(clippy::permissions_set_readonly_false)]
                        perm.set_readonly(false);
                        let _ = fs::set_permissions(p, perm);
                        continue;
                    }
                }
                thread::sleep(Duration::from_millis(if i < 4 { 250 } else { 750 }));
            }
        }
    }
    Err(last)
}

/// Étapes signalées pendant un envoi (pour la barre de progression).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Stage {
    Copy,
    MediaInfo,
    Publish,
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let ca = fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let cb = fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

/// Dossier temporaire sur le même disque que la destination.
/// Pour le dossier de sortie (surveillé par le logiciel de post), il est placé À CÔTÉ, pas dedans :
/// le logiciel ne voit le fichier qu'une fois complet (renommage atomique final).
pub fn staging_for(dest_dir: &Path, cfg: &Config) -> PathBuf {
    if let Some(out) = cfg.output_dir() {
        if same_dir(dest_dir, &out) {
            if let Some(p) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                return p.join(".nzbff_tmp");
            }
        }
    }
    dest_dir.join(".nzbff_tmp")
}

// ------------------------------------------------------------- arrêt d'un traitement

/// Génération d'arrêt : chaque « Arrêter » l'incrémente ; un traitement commencé avant
/// s'arrête à la prochaine étape de copie (fichiers temporaires retirés, originaux remis).
static CANCEL_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

thread_local! {
    static JOB_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Message d'erreur d'un traitement arrêté à la demande.
pub const CANCELLED: &str = "arrêté à ta demande";

/// Début d'un traitement sur ce fil : il pourra être arrêté par `cancel_jobs`.
pub fn begin_job() -> u64 {
    let g = CANCEL_GEN.load(std::sync::atomic::Ordering::SeqCst);
    JOB_GEN.with(|c| c.set(g));
    g
}

/// Arrête les traitements en cours (copies).
pub fn cancel_jobs() {
    CANCEL_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// Le traitement commencé à la génération `g` a-t-il été arrêté ?
pub fn cancelled_since(g: u64) -> bool {
    CANCEL_GEN.load(std::sync::atomic::Ordering::SeqCst) != g
}

pub fn job_cancelled() -> bool {
    JOB_GEN.with(|c| cancelled_since(c.get()))
}

/// Erreur due à un arrêt demandé (pas de quarantaine dans ce cas).
pub fn is_cancel(reason: &str) -> bool {
    reason.contains(CANCELLED)
}

/// Tests qui copient ou postent : un à la fois (l'arrêt est global).
#[cfg(test)]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
thread_local! {
    /// Espace libre simulé (tests).
    pub static FAKE_FREE: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Espace libre (octets) sur le disque qui contient `path` (ou son plus proche parent existant).
pub fn free_space(path: &Path) -> Option<u64> {
    let mut p = path.to_path_buf();
    while !p.exists() {
        p = p.parent()?.to_path_buf();
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
        }
        let w: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let (mut a, mut t, mut f) = (0u64, 0u64, 0u64);
        let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut a, &mut t, &mut f) };
        if ok != 0 {
            Some(a)
        } else {
            None
        }
    }
    #[cfg(not(windows))]
    {
        #[cfg(test)]
        if let Some(v) = FAKE_FREE.with(|c| c.get()) {
            return Some(v);
        }
        let out = Command::new("df").args(["-Pk"]).arg(&p).output().ok()?;
        let t = String::from_utf8_lossy(&out.stdout).to_string();
        let line = t.lines().nth(1)?;
        line.split_whitespace().nth(3)?.parse::<u64>().ok().map(|k| k * 1024)
    }
}

/// Disques disponibles et leur place libre (lecteurs C:, D:… sous Windows).
pub fn drives() -> Vec<(PathBuf, u64)> {
    #[cfg(windows)]
    {
        ('C'..='Z')
            .filter_map(|c| {
                let p = PathBuf::from(format!("{c}:\\"));
                if p.exists() {
                    free_space(&p).map(|f| (p, f))
                } else {
                    None
                }
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        let mut v: Vec<(PathBuf, u64)> = Vec::new();
        for p in [PathBuf::from("/"), std::env::temp_dir()] {
            if let Some(f) = free_space(&p) {
                if !v.iter().any(|(q, _)| same_volume(q, &p)) {
                    v.push((p, f));
                }
            }
        }
        v
    }
}

// ------------------------------------------------- sondes disque sans blocage (interface)

#[derive(Clone)]
enum Probe {
    Bool(bool),
    Size(Option<u64>),
}

/// Valeur gardée 5 s, recalculée en arrière-plan : l'interface ne lit jamais un NAS chargé
/// (copie en cours) elle-même. Premier appel : `first` en attendant le résultat.
fn probe(kind: &str, p: &Path, first: Probe, f: fn(&Path) -> Probe) -> Probe {
    use std::sync::{Mutex, OnceLock};
    type Map = HashMap<String, (Instant, Probe, bool)>;
    static CACHE: OnceLock<Mutex<Map>> = OnceLock::new();
    let c = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{kind}|{}", p.display());
    let mut g = match c.lock() {
        Ok(g) => g,
        Err(_) => return f(p),
    };
    let (val, stale, pending) = match g.get(&key) {
        Some((t, v, pend)) => (v.clone(), t.elapsed() > Duration::from_secs(5), *pend),
        None => (first, true, false),
    };
    if stale && !pending {
        let ins = g.get(&key).map(|e| e.0).unwrap_or_else(|| Instant::now() - Duration::from_secs(60));
        g.insert(key.clone(), (ins, val.clone(), true));
        let path = p.to_path_buf();
        thread::spawn(move || {
            let v = f(&path);
            if let Ok(mut g) = c.lock() {
                g.insert(key, (Instant::now(), v, false));
            }
        });
    }
    val
}

/// `exists()` sans bloquer l'interface (vrai en attendant la première réponse : rien n'est
/// signalé à tort comme disparu).
pub fn ui_exists(p: &Path) -> bool {
    match probe("e", p, Probe::Bool(true), |p| Probe::Bool(p.exists())) {
        Probe::Bool(b) => b,
        _ => true,
    }
}

/// `is_dir()` sans bloquer l'interface (faux en attendant).
pub fn ui_is_dir(p: &Path) -> bool {
    match probe("d", p, Probe::Bool(false), |p| Probe::Bool(p.is_dir())) {
        Probe::Bool(b) => b,
        _ => false,
    }
}

/// Place libre sans bloquer l'interface (None en attendant).
pub fn ui_free_space(p: &Path) -> Option<u64> {
    match probe("f", p, Probe::Size(None), |p| Probe::Size(free_space(p))) {
        Probe::Size(v) => v,
        _ => None,
    }
}

/// Lecteur réseau ? (sans bloquer l'interface ; un chemin \\serveur est su tout de suite).
pub fn ui_is_network(p: &Path) -> bool {
    let s = p.display().to_string();
    let unc = s.starts_with("\\\\") && !s.starts_with("\\\\?\\");
    match probe("n", p, Probe::Bool(unc), |p| Probe::Bool(is_network_path(p))) {
        Probe::Bool(b) => b,
        _ => unc,
    }
}

/// Même disque (même lecteur sous Windows) ?
pub fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        let root = |p: &Path| p.components().next().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).unwrap_or_default();
        root(a) == root(b)
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::MetadataExt;
        let dev = |p: &Path| {
            let mut q = p.to_path_buf();
            while !q.exists() {
                match q.parent() {
                    Some(x) => q = x.to_path_buf(),
                    None => return None,
                }
            }
            fs::metadata(&q).ok().map(|m| m.dev())
        };
        dev(a).is_some() && dev(a) == dev(b)
    }
}

/// Copie par blocs de 8 Mo avec suivi (octets copiés, total) ; fichier « .part » puis renommage.
/// S'arrête (fichier partiel supprimé) si le traitement est arrêté.
pub fn copy_with_progress(from: &Path, to: &Path, progress: &mut dyn FnMut(u64, u64)) -> std::io::Result<()> {
    let total = fs::metadata(from)?.len();
    let tmp = to.with_file_name(format!("{}.part", file_name(to)));
    let res = (|| -> std::io::Result<()> {
        let mut r = File::open(from)?;
        let mut w = File::create(&tmp)?;
        let mut buf = vec![0u8; 8 << 20];
        let mut done = 0u64;
        let mut last = Instant::now();
        progress(0, total);
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            if job_cancelled() {
                return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, CANCELLED));
            }
            w.write_all(&buf[..n])?;
            done += n as u64;
            if last.elapsed() >= Duration::from_millis(150) {
                progress(done, total);
                last = Instant::now();
            }
        }
        w.flush()?;
        drop(w);
        progress(total, total);
        fs::rename(&tmp, to)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// Déplacement ou copie simple (quarantaine, remise en inbox).
fn transfer_file(from: &Path, to: &Path, how: Transfer) -> std::io::Result<()> {
    if how == Transfer::Move && fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_with_progress(from, to, &mut |_, _| {})?;
    if how == Transfer::Move {
        fs::remove_file(from)?;
    }
    Ok(())
}

/// Chemin du rapport MediaInfo : à côté de la vidéo (`video`), dossier dédié (`dir`)
/// ou sous-dossier de la release dans le dossier indexer (`indexer`, avec le futur NZB).
pub fn report_path(cfg: &Config, dest_dir: &Path, release: &str) -> PathBuf {
    let dir = if cfg.mi_dest == "indexer" && !cfg.indexer_dir.trim().is_empty() {
        // la release attend son NZB dans « _En attente du NZB », puis est distribuée à chaque indexer
        Path::new(cfg.indexer_dir.trim()).join(crate::ngpost::WAITING_DIR).join(release)
    } else if cfg.mi_dest == "dir" && !cfg.xml_dir.trim().is_empty() {
        PathBuf::from(cfg.xml_dir.trim())
    } else {
        dest_dir.to_path_buf()
    };
    dir.join(format!("{release}.{}", cfg.mi_ext()))
}

/// 1. place la vidéo renommée dans un dossier temporaire (déplacement instantané ou copie suivie),
/// 2. génère le rapport MediaInfo sur ce fichier renommé (il porte donc le bon nom),
/// 3. publie la vidéo d'un coup dans `dest_dir`.
pub fn execute(
    path: &Path,
    release: &str,
    cfg: &Config,
    how: Transfer,
    dest_dir: &Path,
    progress: &mut dyn FnMut(Stage, u64, u64),
) -> Result<Done, String> {
    fs::create_dir_all(dest_dir).map_err(|e| format!("dossier de destination : {e}"))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("mkv");
    let final_name = format!("{release}.{ext}");
    let dest = dest_dir.join(&final_name);
    let report = report_path(cfg, dest_dir, release);
    if let Some(d) = report.parent() {
        fs::create_dir_all(d).map_err(|e| format!("dossier MediaInfo : {e}"))?;
    }
    if dest.exists() && !same_dir(&dest, path) {
        return Err(format!("{final_name} existe déjà dans la destination"));
    }

    // 1. vidéo renommée dans le dossier temporaire
    let st = staging_for(dest_dir, cfg);
    fs::create_dir_all(&st).map_err(|e| format!("dossier temporaire : {e}"))?;
    let staged = st.join(&final_name);
    let _ = fs::remove_file(&staged);
    let renamed = how == Transfer::Move && fs::rename(path, &staged).is_ok();
    if !renamed {
        place_copy(path, &staged, how, &mut |d, t| progress(Stage::Copy, d, t)).map_err(|e| format!("copie : {e}"))?;
    }
    let rollback = |staged: &Path| {
        if renamed {
            let _ = fs::rename(staged, path);
        } else {
            let _ = fs::remove_file(staged);
        }
    };

    // 2. MediaInfo sur le fichier renommé (écrit en .tmp puis renommé)
    progress(Stage::MediaInfo, 0, 1);
    let text = match mediainfo_export(cfg, &staged) {
        Ok(r) => r,
        Err(e) => {
            rollback(&staged);
            return Err(e);
        }
    };
    let report_tmp = report.with_file_name(format!("{}.tmp", file_name(&report)));
    if let Err(e) = fs::write(&report_tmp, text).and_then(|_| fs::rename(&report_tmp, &report)) {
        let _ = fs::remove_file(&report_tmp);
        rollback(&staged);
        return Err(format!("écriture du rapport MediaInfo : {e}"));
    }

    // 3. publication atomique
    progress(Stage::Publish, 0, 1);
    if fs::rename(&staged, &dest).is_err() {
        if let Err(e) = copy_with_progress(&staged, &dest, &mut |d, t| progress(Stage::Copy, d, t)) {
            rollback(&staged);
            let _ = fs::remove_file(&report);
            return Err(format!("transfert vers la destination : {e}"));
        }
        let _ = fs::remove_file(&staged);
    }
    // 4. déplacement : l'original copié est supprimé une fois tout réussi (autre disque)
    let mut left = Vec::new();
    if how == Transfer::Move && !renamed {
        if let Err(e) = remove_original(path) {
            left.push((path.to_path_buf(), e));
        }
    }
    let _ = fs::remove_dir(&st); // seulement s'il est vide
    progress(Stage::Publish, 1, 1);
    Ok(Done { dest, xml: report, left })
}

pub fn quarantine(path: &Path, cfg: &Config) -> Result<PathBuf, String> {
    let q = Path::new(cfg.quarantine.trim());
    fs::create_dir_all(q).map_err(|e| e.to_string())?;
    let mut target = q.join(file_name(path));
    if target.exists() {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("fichier");
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        target = q.join(format!("{stem}_{}.{ext}", now_secs()));
    }
    transfer_file(path, &target, Transfer::Move).map_err(|e| e.to_string())?;
    Ok(target)
}

/// Déplace un fichier dans un dossier (remise en inbox depuis la quarantaine).
pub fn move_into(path: &Path, dir: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(dir.trim()).map_err(|e| e.to_string())?;
    let target = Path::new(dir.trim()).join(file_name(path));
    if target.exists() {
        return Err(format!("{} existe déjà", file_name(&target)));
    }
    transfer_file(path, &target, Transfer::Move).map_err(|e| e.to_string())?;
    Ok(target)
}

// ------------------------------------------------------------------ Fichiers

pub fn is_video(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| VIDEO_EXT.contains(&e.to_lowercase().as_str())).unwrap_or(false)
}

/// Dossier d'à-côtés d'une release : échantillon, preuves, captures, couvertures.
pub fn is_aside_dir(name: &str) -> bool {
    matches!(name.to_lowercase().as_str(), "sample" | "samples" | "proof" | "proofs" | "screens" | "screenshots" | "covers" | "cover" | "artwork")
}

/// Échantillon d'une release (« sample.mkv », « film-sample.mkv », « film.sample.mkv », ou
/// rangé dans un dossier Sample).
pub fn is_sample_file(p: &Path) -> bool {
    let l = file_name(p).to_lowercase();
    let stem = l.rsplit_once('.').map(|(a, _)| a).unwrap_or(&l);
    stem == "sample" || stem.starts_with("sample-") || stem.starts_with("sample.") || stem.ends_with("-sample") || stem.ends_with(".sample") || stem.ends_with("_sample") || l.contains(".sample.")
        || p.parent().map(|d| is_aside_dir(&file_name(d))).unwrap_or(false)
}

/// Dossier d'un seul film (avec ou sans NFO, échantillon, preuves) : sa vidéo. Ni disque
/// complet, ni plusieurs vidéos.
pub fn single_movie(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() || crate::disc::detect(dir).is_some() {
        return None;
    }
    let v = list_videos(dir, true);
    (v.len() == 1).then(|| v[0].clone())
}

pub fn list_videos(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            // disque complet (BDMV, VIDEO_TS, ISO) : un contenu à part, pas des vidéos ;
            // dossiers d'échantillons et de preuves : jamais traités
            let n = file_name(&p);
            if recursive && !n.starts_with('.') && !is_aside_dir(&n) && !crate::disc::is_disc_dir_name(&n) && crate::disc::detect(&p).is_none() {
                out.extend(list_videos(&p, true));
            }
        } else if is_video(&p) && !crate::disc::inside_disc(&p) && !is_sample_file(&p) {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Parmi `files`, ceux dont la taille n'a pas bougé pendant `secs` secondes
/// et qui peuvent être ouverts en écriture (copie terminée).
pub fn stable_files(files: &[PathBuf], secs: u64) -> Vec<PathBuf> {
    let before: HashMap<&PathBuf, u64> = files.iter().filter_map(|p| fs::metadata(p).ok().map(|m| (p, m.len()))).collect();
    if secs > 0 {
        thread::sleep(Duration::from_secs(secs));
    }
    files
        .iter()
        .filter(|p| {
            let Some(b) = before.get(p) else { return false };
            let Ok(m) = fs::metadata(p) else { return false };
            m.len() == *b && OpenOptions::new().append(true).open(p).is_ok()
        })
        .cloned()
        .collect()
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Taille lisible, en unités de 1024 comme l'Explorateur de Windows et les indexers
/// (« 144 Go » ici = « 144 Go » dans l'Explorateur = « 144 GB » sur un indexer).
pub fn human_size(b: u64) -> String {
    let b = b as f64;
    const K: f64 = 1024.0;
    if b >= K * K * K * K {
        format!("{:.2} To", b / (K * K * K * K))
    } else if b >= K * K * K {
        format!("{:.2} Go", b / (K * K * K))
    } else if b >= K * K {
        format!("{:.0} Mo", b / (K * K))
    } else if b >= K {
        format!("{:.0} Ko", b / K)
    } else {
        format!("{b:.0} o")
    }
}

/// Ouvre un dossier (ou sélectionne un fichier) dans l'explorateur.
pub fn reveal(path: &Path) {
    // fichier disparu (déplacé) : on remonte jusqu'au premier dossier qui existe encore.
    // Explorer, lui, ouvre « Documents » quand on lui donne un chemin inexistant.
    let mut path = path;
    while !path.exists() {
        match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => path = p,
            _ => return,
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer.exe a sa propre lecture de la ligne de commande : « /select, » doit rester
        // hors des guillemets, et le chemin entre guillemets (sinon il ouvre « Documents »).
        let p = path.display().to_string().replace('/', "\\");
        let mut cmd = Command::new("explorer.exe");
        if path.is_file() {
            cmd.raw_arg(format!("/select,\"{p}\""));
        } else {
            cmd.raw_arg(format!("\"{p}\""));
        }
        let _ = cmd.spawn();
    }
    #[cfg(not(windows))]
    {
        let target = if path.is_file() { path.parent().unwrap_or(path) } else { path };
        let _ = Command::new("xdg-open").arg(target).spawn();
    }
}

/// Envoie un fichier à la corbeille (Windows) ; ailleurs, le supprime.
pub fn trash(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        struct ShFileOp {
            hwnd: *mut std::ffi::c_void,
            func: u32,
            from: *const u16,
            to: *const u16,
            flags: u16,
            aborted: i32,
            mappings: *mut std::ffi::c_void,
            title: *const u16,
        }
        #[link(name = "shell32")]
        extern "system" {
            fn SHFileOperationW(op: *mut ShFileOp) -> i32;
        }
        let abs = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let s = abs.display().to_string();
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
        // liste terminée par deux zéros
        let from: Vec<u16> = s.encode_utf16().chain([0u16, 0u16]).collect();
        let mut op = ShFileOp {
            hwnd: std::ptr::null_mut(),
            func: 3, // FO_DELETE
            from: from.as_ptr(),
            to: std::ptr::null(),
            flags: 0x0040 | 0x0010 | 0x0004 | 0x0400, // ALLOWUNDO | NOCONFIRMATION | SILENT | NOERRORUI
            aborted: 0,
            mappings: std::ptr::null_mut(),
            title: std::ptr::null(),
        };
        let r = unsafe { SHFileOperationW(&mut op) };
        if r != 0 || op.aborted != 0 || path.exists() {
            return Err(format!("mise à la corbeille impossible (code {r})"));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        if path.is_dir() {
            fs::remove_dir_all(path).map_err(|e| e.to_string())
        } else {
            fs::remove_file(path).map_err(|e| e.to_string())
        }
    }
}

/// Ouvre une adresse web dans le navigateur par défaut.
pub fn open_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("adresse vide".into());
    }
    let url = if url.contains("://") { url.to_string() } else { format!("https://{url}") };
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(hwnd: *mut std::ffi::c_void, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
        }
        let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (op, file) = (w("open"), w(&url));
        let r = unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1) };
        if r <= 32 {
            return Err(format!("ouverture de {url} impossible (code {r})"));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open").arg(&url).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------- Tests

#[cfg(test)]
mod tests_secrets {
    use super::*;

    #[test]
    fn secrets_masques_partout() {
        let mut c = Config::default();
        c.usenet_servers.push(crate::usenet::Server { host: "h".into(), pass: "MotDePasseFournisseur".into(), ..Default::default() });
        c.dl_key = "abcdef0123456789".into();
        register_secrets(&c);
        let t = redact("serveur h pass MotDePasseFournisseur, url http://sab/api?apikey=abcdef0123456789&mode=queue");
        assert!(!t.contains("MotDePasseFournisseur") && !t.contains("abcdef0123456789"), "{t}");
        // motifs « pass = … », même pour un secret inconnu
        assert_eq!(redact("user = moi\npass = zz9"), format!("user = moi\npass = {MASK}"));
        assert_eq!(redact("password: x1y2"), format!("password: {MASK}"));
        // le mot de passe des archives reste montré exprès, comme le texte français
        assert_eq!(redact("--rar_pass abc rar_pass=abc"), "--rar_pass abc rar_pass=abc");
        assert_eq!(redact("mot de passe des archives : Zx81"), "mot de passe des archives : Zx81");
        assert_eq!(redact("Compression : oui · mot de passe aucun"), "Compression : oui · mot de passe aucun");
        // texte accentué intact
        assert_eq!(redact("Échec : déjà là, pass: é"), format!("Échec : déjà là, pass: {MASK}"));
        // trop court pour être cherché tel quel : non enregistré
        add_secret("ab");
        assert_eq!(redact("abc ab"), "abc ab");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dossier d'un film (vidéo + NFO + échantillon) : la vidéo ; plusieurs vidéos : rien.
    #[test]
    fn dossier_d_un_seul_film() {
        let d = std::env::temp_dir().join(format!("nzbff_unfilm_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let f = d.join("Film.2020.FRENCH.1080p.WEB.x264-GRP");
        fs::create_dir_all(f.join("Sample")).unwrap();
        fs::write(f.join("film.mkv"), b"x").unwrap();
        fs::write(f.join("film.nfo"), b"nfo").unwrap();
        fs::write(f.join("Sample").join("film-sample.mkv"), b"x").unwrap();
        fs::write(f.join("film.sample.mkv"), b"x").unwrap();
        assert_eq!(single_movie(&f), Some(f.join("film.mkv")));
        assert_eq!(single_movie(&d), Some(f.join("film.mkv")), "dossier qui contient le dossier du film");
        fs::write(f.join("autre.mkv"), b"x").unwrap();
        assert_eq!(single_movie(&f), None);
        let _ = fs::remove_dir_all(&d);
    }

    /// Liste d'indexer importée avec une version d'avant : les autres contenus sont remplis une
    /// fois au chargement, sans toucher une case déjà choisie, et plus jamais ensuite.
    #[test]
    fn categories_des_contenus_remplies_une_fois() {
        let d = std::env::temp_dir().join(format!("nzbff_catfill_{}", std::process::id()));
        let _ = fs::create_dir_all(&d);
        let f = d.join("nzbff.conf");
        let list = crate::upload::parse_category_list(include_str!("testdata_categories.xml"));
        let mut conf = String::from("indexer=altHUB|https://althub.co.za/upload|true\nindexer_cat=altHUB\tjeux-pc\t4051\n");
        for (id, n) in &list {
            conf.push_str(&format!("indexer_catlist=altHUB\t{id}\t{n}\n"));
        }
        fs::write(&f, conf).unwrap();
        let mut c = Config::load(&f);
        let i = &c.indexers[0];
        assert_eq!(i.content_cat("jeux-pc"), Some("4051"), "choix gardé");
        assert_eq!(i.content_cat("ebooks"), Some("7020"));
        assert_eq!(i.content_cat("fr.ebooks"), Some("7320"));
        assert_eq!(i.content_cat("bluray"), Some("2050"));
        // case remise à « défaut » par l'utilisateur : pas re-remplie
        c.indexers[0].cat_map.retain(|(k, _)| k != "ebooks");
        c.save(&f).unwrap();
        let c = Config::load(&f);
        assert_eq!(c.indexers[0].content_cat("ebooks"), None);
        let _ = fs::remove_dir_all(&d);
    }

    fn facts(video: &str, audio: &str, text: &str, general: &str) -> Facts {
        parse_facts(video, audio, text, general)
    }

    fn fr_1080() -> Facts {
        facts("1920|~|1080|~|x264|~|AVC|~|;", "fr;", "", "")
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ngprep_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn parts<'a>(t: &'a str, y: &'a str, l: &'a str, s: &'a str, tag: &'a str) -> NameParts<'a> {
        NameParts { title: t, year: y, lang: l, resolution: "1080p", source: s, codec: "x264", tag, audio: "", extra: "" }
    }

    #[test]
    fn original_en_lecture_seule_supprime() {
        let d = std::env::temp_dir().join(format!("nzbff-ro-{}", std::process::id()));
        let _ = fs::create_dir_all(&d);
        let f = d.join("film.mkv");
        fs::write(&f, b"x").unwrap();
        let mut p = fs::metadata(&f).unwrap().permissions();
        p.set_readonly(true);
        fs::set_permissions(&f, p).unwrap();
        assert!(remove_original(&f).is_ok() && !f.exists());
        assert!(remove_original(&f).is_ok(), "déjà parti : rien à faire");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn noms_dvd_vof_audio_et_tags() {
        // nom sans année ni résolution : reconnu comme release, gardé tel quel
        let n = "film.FRENCH.VOF.DVDRemux.AC3.2.0.MPEG2-LiHDL";
        assert!(is_release_name(n));
        let f = parse_release(n);
        assert_eq!((f.title.as_str(), f.lang.as_str(), f.source.as_str(), f.audio.as_str(), f.codec.as_str(), f.tag.as_str()), ("film", "FRENCH.VOF", "DVDRemux", "AC3.2.0", "MPEG2", "LiHDL"), "{f:?}");
        // reconstruit depuis les éléments : rien de perdu
        assert_eq!(f.render(DEFAULT_TEMPLATE), "film.FRENCH.VOF.DVDRemux.AC3.2.0.MPEG2-LiHDL");
        let (n2, o) = find_release_name(Path::new("/nas/film.FRENCH.VOF.DVDRemux.AC3.2.0.MPEG2-LiHDL.mkv"), &Facts::default()).unwrap();
        assert_eq!((n2.as_str(), o), (n, "nom du fichier"));
        let (t, y) = title_year("Le.Film.FRENCH.VOF.DVDRemux-LiHDL");
        assert_eq!((t.as_str(), y.as_str()), ("Le.Film", ""));
        // avec année, audio HD, HDR, Atmos
        let f = parse_release("Dune.2021.MULTi.VFF.2160p.UHD.BluRay.REMUX.HDR.DV.TrueHD.7.1.Atmos.HEVC-GRP");
        assert_eq!((f.lang.as_str(), f.resolution.as_str(), f.audio.as_str(), f.codec.as_str(), f.tag.as_str()), ("MULTi.VFF", "2160p", "TrueHD.7.1.Atmos", "HEVC", "GRP"), "{f:?}");
        assert!(f.extra.contains("HDR") && f.extra.contains("DV"), "{f:?}");
        let f = parse_release("Film.2010.FRENCH.1080p.BluRay.DTS-HD.MA.5.1.x264-GRP");
        assert_eq!((f.audio.as_str(), f.codec.as_str(), f.tag.as_str()), ("DTS-HD.MA.5.1", "x264", "GRP"), "{f:?}");
        assert_eq!(f.render(DEFAULT_TEMPLATE), "Film.2010.FRENCH.1080p.BluRay.DTS-HD.MA.5.1.x264-GRP");
        let f = parse_release("Film.2010.MULTi.COMPLETE.BLURAY-GRP");
        assert_eq!((f.source.as_str(), f.tag.as_str()), ("COMPLETE.BLURAY", "GRP"), "{f:?}");
        assert!(!is_release_name("Les.Tontons.Flingueurs-Restauration"));
        // MediaInfo : MPEG-2, audio
        let mut fa = Facts { format: "MPEG Video".into(), format_version: "Version 2".into(), audio_format: "AC-3".into(), audio_channels: "2".into(), ..Default::default() };
        assert_eq!((video_codec(&fa).as_str(), audio_tag(&fa).as_str()), ("MPEG2", "AC3.2.0"));
        fa.audio_format = "DTS".into();
        fa.audio_features = "XLL".into();
        fa.audio_channels = "6".into();
        assert_eq!(audio_tag(&fa), "DTS-HD.MA.5.1");
        fa.audio_format = "E-AC-3".into();
        fa.audio_features = "JOC".into();
        assert_eq!(audio_tag(&fa), "DDP5.1.Atmos");
        assert_eq!(source_from_text("film.FRENCH.VOF.DVDRemux.AC3"), Some("DVDRemux"));
        assert_eq!(source_from_text("Film.2001.BluRay.REMUX-X"), Some("REMUX"));
    }

    #[test]
    fn parse_facts_complet() {
        let f = facts(
            "1920|~|1080|~|x264|~|AVC|~|;",
            "fre;en;;",
            "fr;",
            "01:52:10.120|~|Monfilm.1975.FRENCH.1080p.HDTV.x264-UKDHD|~||~|Blu-ray",
        );
        assert_eq!((f.width, f.height), (1920, 1080));
        assert_eq!(f.langs, vec!["fr", "en", "und"]);
        assert_eq!(f.subs, vec!["fr"]);
        assert_eq!(f.duration, "01:52:10");
        assert_eq!(f.title_tag, "Monfilm.1975.FRENCH.1080p.HDTV.x264-UKDHD");
        assert_eq!(f.source_medium, "Blu-ray");
    }

    #[test]
    fn langues() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(language_tag(&v(&["fr"]), &[], "FRENCH"), "FRENCH");
        assert_eq!(language_tag(&v(&["fr", "en"]), &[], "FRENCH"), "MULTi");
        assert_eq!(language_tag(&v(&["fr", "und"]), &[], ""), "FRENCH");
        assert_eq!(language_tag(&v(&["und"]), &[], "FRENCH"), "FRENCH"); // non renseignée
        assert_eq!(language_tag(&v(&["und"]), &[], ""), "");
        assert_eq!(language_tag(&v(&["en"]), &v(&["fr"]), "FRENCH"), "VOSTFR");
        assert_eq!(language_tag(&v(&["en"]), &[], "FRENCH"), "");
        assert_eq!(norm_lang("French"), "fr");
        assert_eq!(norm_lang("fr-FR"), "fr");
    }

    #[test]
    fn langues_multi_pistes() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // piste française sans langue renseignée + anglaise → MULTi
        assert_eq!(language_tag(&v(&["und", "en"]), &[], "FRENCH"), "MULTi");
        assert_eq!(language_tag(&v(&["und", "en"]), &[], ""), "");
        // langue déduite du titre de la piste
        let f = parse_facts("1920|~|1080|~|x264|~|AVC|~|;", "|~|VFF 5.1|#|eng|~|English|#|", "", "");
        assert_eq!(f.langs, vec!["fr", "en"]);
        assert_eq!(f.audio_desc, vec!["fr (VFF 5.1)", "en (English)"]);
        assert_eq!(language_tag(&f.langs, &f.subs, ""), "MULTi");
        let f = parse_facts("1920|~|1080|~|x264|~|AVC|~|;", "fr|~||#|en|~||#|fr|~|Commentaires|#|", "", "");
        assert_eq!(language_tag(&f.langs, &f.subs, "FRENCH"), "MULTi");
        assert_eq!(lang_from_title("Français AC3"), Some("fr"));
        assert_eq!(lang_from_title("Commentaire"), None);
    }

    #[test]
    fn sources() {
        assert_eq!(source_from_text("Monfilm.1975.FRENCH.1080p.HDTV.x264-UKDHD"), Some("HDTV"));
        assert_eq!(source_from_text("Film 2001 1080p Blu-Ray"), Some("BluRay"));
        assert_eq!(source_from_text("Film.2001.MULTi.1080p.WEB-DL.H264-X"), Some("WEB-DL"));
        assert_eq!(source_from_text("Film.2001.1080p.WEB.H264-X"), Some("WEB"));
        assert_eq!(source_from_text("Film.2001.BluRay.REMUX-X"), Some("REMUX"));
        assert_eq!(source_from_text("Web of lies (2001)"), None); // « WEB » seul : en majuscules uniquement
        assert_eq!(source_from_text("Monfilm (1975)"), None);
        assert_eq!(source_from_medium("Blu-ray"), Some("BluRay"));
        assert_eq!(source_from_medium(""), None);
    }

    #[test]
    fn modele_sans_source_ni_tag() {
        let t = DEFAULT_TEMPLATE;
        assert_eq!(render_template(t, &parts("Monfilm", "1975", "FRENCH", "", "")), "Monfilm.1975.FRENCH.1080p.x264");
        assert_eq!(render_template(t, &parts("Monfilm", "1975", "FRENCH", "BluRay", "TAG")), "Monfilm.1975.FRENCH.1080p.BluRay.x264-TAG");
        assert_eq!(render_template("{titre} {annee} {langue} {resolution}", &parts("Mon.Film", "1975", "FRENCH", "", "")), "Mon.Film.1975.FRENCH.1080p");
    }

    #[test]
    fn langue_ajoutee_au_nom_existant() {
        assert_eq!(ensure_lang("Movie.2010.1080p.BluRay.x264-GRP", "FRENCH"), "Movie.2010.FRENCH.1080p.BluRay.x264-GRP");
        let n = "Monfilm.1975.FRENCH.1080p.HDTV.x264-UKDHD";
        assert_eq!(ensure_lang(n, "MULTi"), n);
        assert_eq!(ensure_lang("Movie.2010.1080p.x264-GRP", ""), "Movie.2010.1080p.x264-GRP");
    }

    #[test]
    fn titre_annee() {
        assert_eq!(title_year("Monfilm (1975)"), ("Monfilm".into(), "1975".into()));
        assert_eq!(title_year("mon.film.1975"), ("mon.film".into(), "1975".into()));
        assert_eq!(title_year("1917 (2019)"), ("1917".into(), "2019".into()));
        assert_eq!(title_year("Sans annee"), ("Sans.annee".into(), "".into()));
        assert_eq!(title_year("Amélie (2001)"), ("Amélie".into(), "2001".into()));
    }

    #[test]
    fn noms_release() {
        assert!(is_release_name("Monfilm.1975.FRENCH.1080p.HDTV.x264-UKDHD"));
        assert!(!is_release_name("Monfilm (1975)"));
        assert!(!is_release_name("Monfilm.1975.FRENCH.1080p.HDTV.x264"));
        assert!(!is_release_name("monfilm.1975"));
        assert_eq!(release_tag("A.1975.FRENCH.1080p.x264-UKDHD"), "UKDHD");
    }

    #[test]
    fn release_recuperee_du_tag_mkv() {
        let f = facts("1920|~|1080|~|x264|~|AVC|~|;", "fr;", "", "|~|Monfilm 1975 FRENCH 1080p HDTV x264-UKDHD|~||~|");
        let (n, o) = find_release_name(Path::new("/nas/Films/Monfilm (1975).mkv"), &f).unwrap();
        assert_eq!(n, "Monfilm.1975.FRENCH.1080p.HDTV.x264-UKDHD");
        assert_eq!(o, "titre MediaInfo");
        let (n, o) = find_release_name(Path::new("/nas/Monfilm.1975.FRENCH.1080p.BluRay.x264-AB/m.mkv"), &fr_1080()).unwrap();
        assert_eq!((n.as_str(), o), ("Monfilm.1975.FRENCH.1080p.BluRay.x264-AB", "dossier parent"));
        assert!(find_release_name(Path::new("/nas/Films/Monfilm (1975).mkv"), &fr_1080()).is_none());
    }

    #[test]
    fn source_detectee() {
        let f = facts("1920|~|1080|~|x264|~|AVC|~|;", "fr;", "", "|~||~||~|Blu-ray");
        assert_eq!(detect_source(Path::new("/x/Film (2001).mkv"), &f), Some(("BluRay", "support d'origine MediaInfo")));
        assert_eq!(detect_source(Path::new("/x/Film (2001) HDLight.mkv"), &fr_1080()), Some(("HDLight", "nom du fichier")));
        assert_eq!(detect_source(Path::new("/x/Film (2001).mkv"), &fr_1080()), None);
    }

    #[test]
    fn nettoyage() {
        assert_eq!(clean_name(" Mon Film 1975: FRENCH? "), "Mon.Film.1975.FRENCH");
    }

    /// Réglages de recherche de la 3.20 convertis en profil d'API.
    #[test]
    fn recherche_320_convertie() {
        let dir = tmpdir("cfg320");
        let p = dir.join("c.conf");
        fs::write(&p, "indexer=A|https://a/upload|true\nindexer=B|https://b/upload|true\nindexer_search=A\turl\thttps://a.example/s?q={q}&apikey={key}\nindexer_search=B\toff\t\nindexer_action=B\tDétails\tGET\t{api}?t=details&id={id}&apikey={key}\taide\n").unwrap();
        let c = Config::load(&p);
        let a = &c.indexers[0].api;
        assert_eq!((a.kind.as_str(), a.auth.as_str()), ("json", "none"));
        assert_eq!(a.requests[0].url, "https://a.example/s?q={release}&apikey={key}");
        let mut i = c.indexers[0].clone();
        i.api_key = "K3YK3Y".into();
        let b = crate::apidef::build(&i, &crate::apidef::profile(&i), &a.requests[0], &[("release".into(), "Le Film".into())]).unwrap();
        assert_eq!(b.url, "https://a.example/s?q=Le%20Film&apikey=K3YK3Y");
        let bp = &c.indexers[1].api;
        assert_eq!(bp.check, "off");
        assert!(bp.requests.iter().any(|r| r.label == "Détails" && r.params.iter().any(|x| x.name == "id")));
        // sauvegardé sous la nouvelle forme, relu à l'identique
        c.save(&p).unwrap();
        let txt = fs::read_to_string(&p).unwrap();
        assert!(txt.contains("indexer_profile=A\t{") && !txt.contains("indexer_search="));
        assert_eq!(Config::load(&p).indexers, c.indexers);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_aller_retour_et_ancienne() {
        let dir = tmpdir("cfg");
        let p = dir.join("c.conf");
        let c = Config { inbox: "D:\\inbox".into(), mi_format: "json-full".into(), transfer: "copy".into(), accent: "Rose".into(), interval: 5, ..Default::default() };
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p), c);
        fs::write(&p, "old_xml=true\nkeep_original=true\nsource=BluRay\n").unwrap();
        let o = Config::load(&p);
        assert_eq!((o.mi_format.as_str(), o.transfer.as_str(), o.template.as_str()), ("oldxml", "auto", DEFAULT_TEMPLATE));
        fs::write(&p, "xml_dir=D:\\rapports\n").unwrap();
        assert_eq!(Config::load(&p).mi_dest, "dir"); // ancienne config : dossier dédié
        let c2 = Config {
            xml_dir: "D:\\r".into(),
            mi_dest: "video".into(),
            indexers: vec![
                IndexerCfg { url: "https://a.example/upload".into(), mode: "newznab".into(), api_url: "https://a.example/api".into(), api_key: "k|ey".into(), send_nfo: true, dupecheck: false, extra_query: "private=no".into(), extra_fields: "foo=1; bar=2".into(), ..IndexerCfg::new("Idx A") },
                IndexerCfg { active: false, mode: "rest".into(), rest_path: "/v1/nzb".into(), auth: "bearer".into(), cmd: "curl -F \"file=@{nzb}\" https://b.example/x | tee".into(), ..IndexerCfg::new("Idx B") },
                IndexerCfg {
                    mode: "rest".into(),
                    api_url: "https://c.example/api".into(),
                    api_key: "NZ".into(),
                    rest_url: "https://c.example/rest/v1".into(),
                    rest_key: "RK".into(),
                    api: crate::apidef::from_preset(&crate::apidef::preset("nzbindex").unwrap()),
                    ..IndexerCfg::new("Idx C")
                },
            ],
            dupe_check: "warn".into(),
            start_in_tray: true,
            ..Default::default()
        };
        c2.save(&p).unwrap();
        assert_eq!(Config::load(&p).mi_dest, "video");
        assert_eq!(Config::load(&p), c2);
        // 3.20 : l'ancienne case « démarrer réduit » (cochée pour le lancement avec Windows) est oubliée
        fs::write(&p, "start_in_tray=true\n").unwrap();
        assert!(!Config::load(&p).start_in_tray);
        // chaque mode garde son adresse et sa clé
        let ic = &c2.indexers[2];
        assert_eq!((ic.rest_view().api_url.as_str(), ic.rest_view().api_key.as_str(), ic.key_for_mode()), ("https://c.example/rest/v1", "RK", "RK".to_string()));
        let mut nz = ic.clone();
        nz.rest_key.clear();
        assert_eq!(nz.rest_key_eff(), "NZ"); // clé REST vide : celle de Newznab
        // ancienne ligne (une seule adresse, mode REST) : l'adresse devient celle de REST
        fs::write(&p, "indexer=Old|https://o.example|true\nindexer_api=Old\trest\thttps://o.example/api/v1\tKEY\t-1\t2030\t2040\t2045\ttrue\ttrue\tfalse\ttrue\ttrue\t\t\t\t/nzb\theader\tX-API-Key\tfin\n").unwrap();
        let o = Config::load(&p);
        assert_eq!((o.indexers[0].rest_url.as_str(), o.indexers[0].api_url.as_str(), o.indexers[0].has_api()), ("https://o.example/api/v1", "", true));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn formats_mediainfo() {
        assert_eq!(mi_spec("xml-full"), (&["--Full", "--Output=XML"][..], "xml"));
        assert_eq!(mi_spec("json-full").1, "json");
        assert_eq!(mi_spec("text").0.len(), 0);
        assert_eq!(mi_spec("inconnu").1, "xml");
    }

    #[test]
    fn transferts() {
        let _verrou = crate::prep::test_lock();
        let d = tmpdir("copy");
        let cfg = Config { ngpost_dir: d.join("ng").display().to_string(), xml_dir: d.join("xml").display().to_string(), ..Default::default() };
        fs::create_dir_all(&cfg.ngpost_dir).unwrap();
        let src = d.join("a.mkv");
        fs::write(&src, b"data").unwrap();
        let dst = Path::new(&cfg.ngpost_dir).join("b.mkv");
        transfer_file(&src, &dst, Transfer::Copy).unwrap();
        assert!(src.exists() && dst.exists());
        transfer_file(&dst, &d.join("c.mkv"), Transfer::Move).unwrap();
        assert!(!dst.exists());
        assert!(!is_network_path(&src)); // disque local
        assert_eq!(transfer_for(&src, &cfg), Transfer::Move);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn codecs_anciens_nommes() {
        let f = |fmt: &str, lib: &str| video_codec(&Facts { format: fmt.into(), lib: lib.into(), ..Default::default() });
        assert_eq!(f("AVC", "x264"), "x264");
        assert_eq!(f("AVC", ""), "H264");
        assert_eq!(f("HEVC", "x265"), "x265");
        // DVD et vieux encodages : nommés comme dans les releases
        assert_eq!(f("MPEG Video", ""), "MPEG2");
        assert_eq!(f("MPEG-4 Visual", "XviD"), "XviD");
        assert_eq!(f("VC-1", ""), "VC-1");
        assert_eq!(f("MPEG-4 Visual", ""), "");
        assert_eq!(f("ProRes", ""), "");
    }

    #[test]
    fn decoupage_release_et_champs() {
        let f = parse_release("Monfilm.1975.FRENCH.1080p.HDTV.x264-UKDHD");
        assert_eq!(
            f,
            Fields { title: "Monfilm".into(), year: "1975".into(), lang: "FRENCH".into(), resolution: "1080p".into(), source: "HDTV".into(), codec: "x264".into(), tag: "UKDHD".into(), ..Default::default() }
        );
        assert_eq!(f.render(DEFAULT_TEMPLATE), "Monfilm.1975.FRENCH.1080p.HDTV.x264-UKDHD");
        let mut g = Fields { title: "Mon.Film".into(), ..Default::default() };
        assert_eq!(g.missing(), vec!["année", "langue", "résolution"]);
        g.fill_from(&f);
        assert_eq!(g.render(DEFAULT_TEMPLATE), "Mon.Film.1975.FRENCH.1080p.HDTV.x264-UKDHD");
    }

    #[cfg(unix)]
    #[test]
    fn envoi_complet_avec_progression() {
        let _verrou = crate::prep::test_lock();
        use std::os::unix::fs::PermissionsExt;
        let d = tmpdir("exec");
        let mi = d.join("mi.sh");
        fs::write(&mi, "#!/bin/sh\necho \"rapport de $3 dans $PWD\"\n").unwrap();
        fs::set_permissions(&mi, fs::Permissions::from_mode(0o755)).unwrap();
        let out = d.join("sortie");
        let cfg = Config { ngpost_dir: out.display().to_string(), mediainfo: mi.display().to_string(), ..Default::default() };
        let src = d.join("Mon film (1975).mkv");
        fs::write(&src, vec![7u8; 20 << 20]).unwrap();
        // copie vers le dossier de sortie, rapport à côté de la vidéo (aucun dossier MediaInfo réglé)
        let mut steps = Vec::new();
        let done = execute(&src, "Mon.Film.1975.FRENCH.1080p.x264", &cfg, Transfer::Copy, &out, &mut |st, a, b| steps.push((st, a, b))).unwrap();
        assert!(src.exists());
        assert_eq!(fs::metadata(&done.dest).unwrap().len(), 20 << 20);
        assert_eq!(done.xml, out.join("Mon.Film.1975.FRENCH.1080p.x264.xml"));
        let rep = fs::read_to_string(&done.xml).unwrap();
        assert!(rep.contains("Mon.Film.1975.FRENCH.1080p.x264.mkv") && !rep.contains("Mon film"));
        assert!(steps.iter().any(|(s, a, b)| *s == Stage::Copy && a == b && *b == 20 << 20));
        assert!(!d.join(".nzbff_tmp").exists()); // dossier temporaire nettoyé
        // renommage sur place (déplacement instantané)
        let done2 = execute(&src, "Mon.Film.1975.MULTi.1080p.x264", &cfg, Transfer::Move, &d, &mut |_, _, _| {}).unwrap();
        assert!(!src.exists() && done2.dest == d.join("Mon.Film.1975.MULTi.1080p.x264.mkv"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn liste_videos_recursive() {
        let d = tmpdir("list");
        fs::create_dir_all(d.join("sous")).unwrap();
        fs::write(d.join("a.mkv"), b"").unwrap();
        fs::write(d.join("b.txt"), b"").unwrap();
        fs::write(d.join("sous/c.MP4"), b"").unwrap();
        assert_eq!(list_videos(&d, false).len(), 1);
        assert_eq!(list_videos(&d, true).len(), 2);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn tailles() {
        assert_eq!(human_size(4_700_000_000), "4.38 Go"); // comme l'Explorateur de Windows
        assert_eq!(human_size(850_000_000), "811 Mo");
        assert_eq!(human_size(144 << 30), "144.00 Go");
    }
}

#[cfg(test)]
mod tests_arret {
    use super::*;

    #[test]
    fn arret_d_une_copie() {
        let _verrou = test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_stopcopy_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let src = d.join("gros.mkv");
        fs::write(&src, vec![1u8; 40 << 20]).unwrap();
        begin_job();
        let mut calls = 0;
        // arrêt demandé pendant la copie (après le premier bloc)
        let e = copy_with_progress(&src, &d.join("copie.mkv"), &mut |_, _| {
            calls += 1;
            cancel_jobs(); // « Arrêter » cliqué dès le début de la copie
        })
        .unwrap_err();
        assert!(e.to_string().contains(CANCELLED) && is_cancel(&e.to_string()));
        assert!(!d.join("copie.mkv").exists() && !d.join("copie.mkv.part").exists(), "copie partielle restée");
        assert!(src.exists() && calls > 0);
        // un nouveau traitement repart normalement
        begin_job();
        copy_with_progress(&src, &d.join("copie.mkv"), &mut |_, _| {}).unwrap();
        let _ = fs::remove_dir_all(&d);
    }
}


#[cfg(test)]
mod tests_326 {
    use super::*;

    /// User-Agent général et groupes par catégorie : enregistrés puis relus.
    #[test]
    fn identite_et_groupes_par_categorie_enregistres() {
        let d = std::env::temp_dir().join(format!("nzbff_cfg326_{}", std::process::id()));
        let _ = fs::create_dir_all(&d);
        let f = d.join("nzbff.conf");
        let mut i = IndexerCfg::new("Idx");
        i.group_rules = vec![("2045".into(), "alt.binaries.uhd, alt.binaries.hdtv".into()), ("5000".into(), "alt.binaries.teevee".into())];
        let c = Config { net_ua: "rotate".into(), indexers: vec![i], ..Default::default() };
        c.save(&f).unwrap();
        let back = Config::load(&f);
        assert_eq!(back.net_ua, "rotate");
        assert_eq!(back.indexers[0].group_rules, vec![("2045".to_string(), "alt.binaries.uhd,alt.binaries.hdtv".to_string()), ("5000".to_string(), "alt.binaries.teevee".to_string())]);
        assert!(back.groups_force.is_empty(), "jamais enregistré");
        let _ = fs::remove_dir_all(&d);
    }
}
