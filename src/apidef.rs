//! Profils d'API des indexers : comment interroger chaque indexer, quel qu'il soit.
//!
//! Inspiré des projets *arr (Prowlarr, Sonarr, Radarr) et de NZBHydra2 :
//!
//! - **Préréglages** : les indexers Usenet connus de ces projets (adresse, chemin de l'API),
//!   les moteurs publics (NZBIndex en JSON, BinSearch en HTML), et des modèles génériques
//!   (Newznab, API JSON, page de recherche) pour tout indexer à venir.
//! - **Capacités** (`t=caps` de Newznab) : types de recherche disponibles, paramètres
//!   acceptés, catégories, limites. Les requêtes en sont tirées automatiquement.
//! - **Requêtes** : méthode (GET, POST…), adresse modèle, paramètres placés dans l'adresse,
//!   le chemin, un formulaire, du JSON ou un en-tête, avec l'encodage voulu (`%20` ou `+`),
//!   en-têtes, corps, et lecture de la réponse (XML, JSON ou motif HTML).
//! - **Import** : documentation OpenAPI / Swagger (JSON ou YAML), collection Postman,
//!   réponse `t=caps`, page wiki (adresses d'exemple, lignes « GET /chemin », tableaux de
//!   paramètres), profil exporté par l'application.
//!
//! La clé n'est jamais sur une ligne de commande : adresse, en-têtes et corps passent par un
//! fichier de configuration curl `-K`. Elle est masquée dans tout ce qui est affiché.

use crate::dl::J;
use crate::prep::IndexerCfg;
use crate::upload::{encode, endpoint, run_curl};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ modèle

/// Profil d'API d'un indexer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    /// "" automatique (Newznab avec l'adresse et la clé du dépôt), "newznab", "json"
    /// (API REST qui répond en JSON), "html" (page de recherche du site).
    pub kind: String,
    /// Identifiant du préréglage d'origine (« althub », « nzbindex »…), vide sinon.
    pub preset: String,
    /// D'où vient le profil (« Préréglage altHUB (Prowlarr) », « Importé de openapi.yaml »…).
    pub source: String,
    /// Adresse de base (https://api.althub.co.za) ; vide : celle du dépôt Newznab.
    pub base: String,
    /// Chemin de l'API Newznab (« /api », « /api/v1/api »).
    pub api_path: String,
    /// Clé propre à la recherche ; vide : celle du dépôt.
    pub key: String,
    /// Transmission de la clé : "query" (paramètre d'adresse `auth_name`), "header" (en-tête
    /// `auth_name`), "bearer", "basic" (`user` + clé comme mot de passe), "cookie", "none".
    pub auth: String,
    pub auth_name: String,
    pub user: String,
    pub user_agent: String,
    /// Requêtes par minute au plus (0 : pas de limite).
    pub rate: u32,
    /// Délai maximal d'une requête, en secondes (0 : 40 s).
    pub timeout: u32,
    pub caps: Caps,
    pub requests: Vec<Req>,
    /// Vérification avant le post : "" (requête « search »), "off", ou l'identifiant d'une requête.
    pub check: String,
    /// Nom envoyé : "auto" (tel quel, puis avec des espaces si rien n'est trouvé), "asis", "spaces".
    pub query_style: String,
    pub notes: String,
}

/// Une requête de l'API.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Req {
    pub id: String,
    pub label: String,
    pub help: String,
    /// GET, POST, PUT, PATCH, DELETE.
    pub method: String,
    /// Adresse modèle : {base} {api} {key} {release}, et {nom} de chaque paramètre « chemin ».
    pub url: String,
    pub params: Vec<Param>,
    pub headers: Vec<(String, String)>,
    /// Corps brut modèle (JSON ou texte) ; vide : construit d'après les paramètres.
    pub body: String,
    pub resp: RespMap,
    /// Lecture seule (recherche, consultation) : lancée sans confirmation, même en POST.
    pub read: bool,
    /// « newznab », « caps », « préréglage », « import », « manuel ».
    pub origin: String,
}

/// Paramètre d'une requête.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Param {
    pub name: String,
    /// Valeur modèle : texte fixe, {release} (nom cherché), {key}, {cat}… ; vide : non envoyé.
    pub value: String,
    pub label: String,
    /// "query" (adresse), "path" (chemin : {nom} dans l'adresse), "form", "json", "header".
    pub place: String,
    /// Encodage dans l'adresse : "%20" (standard), "+" (formulaire), "raw" (brut).
    pub enc: String,
    pub required: bool,
    /// Valeurs proposées (valeur, libellé).
    pub choices: Vec<(String, String)>,
}

/// Lecture de la réponse.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RespMap {
    /// "auto", "xml", "json", "html", "text".
    pub format: String,
    /// XML : élément (« item », « channel/item ») ; JSON : chemin du tableau (« data.content ») ;
    /// HTML : motif d'un résultat avec {title} {size} {link} {id} et * (n'importe quoi).
    pub items: String,
    /// Champs (XML : « title », « enclosure@length », « newznab:attr[name=size]@value » ;
    /// JSON : « name », « files.0.name ») ; plusieurs possibles séparés par « | ».
    pub title: String,
    pub size: String,
    pub link: String,
    pub id: String,
    pub date: String,
    /// Message d'erreur de l'API (XML « error@description », JSON « error|message »).
    pub error: String,
    /// Nettoyage du titre : motif avec {title} (« "{title}" » garde ce qui est entre guillemets).
    pub clean: String,
}

/// Capacités Newznab (`t=caps`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Caps {
    pub title: String,
    pub limit_max: u32,
    pub limit_default: u32,
    /// (type : search, tv-search, movie-search, audio-search, book-search ; paramètres).
    pub modes: Vec<(String, Vec<String>)>,
    pub cats: Vec<(String, String)>,
    /// Groupes Usenet indexés (`<groups>`), en minuscules.
    pub groups: Vec<String>,
    /// Date de lecture (secondes).
    pub read_at: u64,
}

/// Un résultat lu dans une réponse.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub title: String,
    pub size: u64,
    pub link: String,
    pub id: String,
    pub date: String,
    /// Catégorie (Newznab : 2040, 5030…), si l'API la donne.
    pub cat: String,
    /// Lien direct du NZB (Newznab : `enclosure@url`), si l'API le donne.
    pub nzb: String,
}

impl Profile {
    pub fn is_auto(&self) -> bool {
        self.kind.is_empty()
    }
    pub fn kind_label(&self) -> &'static str {
        match self.kind.as_str() {
            "" => "automatique (API Newznab du dépôt)",
            "newznab" => "API Newznab",
            "json" => "API JSON / REST",
            "html" => "page de recherche (HTML)",
            _ => "personnalisé",
        }
    }
    pub fn req(&self, id: &str) -> Option<&Req> {
        self.requests.iter().find(|r| r.id == id)
    }
    /// Requête de la vérification avant le post.
    pub fn check_req(&self) -> Option<&Req> {
        match self.check.as_str() {
            "off" => None,
            "" => self.req("search").or_else(|| self.requests.iter().find(|r| r.read && r.params.iter().any(|p| p.value.contains("{release}")))),
            id => self.req(id),
        }
    }
}

impl Req {
    pub fn method(&self) -> String {
        let m = self.method.trim().to_uppercase();
        if m.is_empty() {
            "GET".into()
        } else {
            m
        }
    }
    /// Paramètres qui prennent le nom cherché (ou qui attendent une saisie).
    pub fn wants_release(&self) -> bool {
        self.params.iter().any(|p| p.value.contains("{release}")) || self.url.contains("{release}") || self.body.contains("{release}")
    }
}

impl Param {
    pub fn new(name: &str, value: &str, label: &str) -> Param {
        Param { name: name.into(), value: value.into(), label: label.into(), place: "query".into(), enc: "%20".into(), ..Default::default() }
    }
    pub fn place_label(p: &str) -> &'static str {
        match p {
            "path" => "chemin",
            "form" => "formulaire",
            "json" => "JSON",
            "header" => "en-tête",
            _ => "adresse",
        }
    }
}

// ------------------------------------------------------------------ JSON (profil exporté, sauvegarde)

fn js(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn obj(fields: &[(&str, String)]) -> String {
    format!("{{{}}}", fields.iter().map(|(k, v)| format!("{}:{v}", js(k))).collect::<Vec<_>>().join(","))
}

fn arr(items: impl Iterator<Item = String>) -> String {
    format!("[{}]", items.collect::<Vec<_>>().join(","))
}

fn pairs_js(v: &[(String, String)]) -> String {
    arr(v.iter().map(|(a, b)| format!("[{},{}]", js(a), js(b))))
}

fn read_pairs(j: &J) -> Vec<(String, String)> {
    j.arr().iter().map(|p| (p.arr().first().map(|x| x.s()).unwrap_or_default(), p.arr().get(1).map(|x| x.s()).unwrap_or_default())).collect()
}

impl RespMap {
    fn to_json(&self) -> String {
        obj(&[
            ("format", js(&self.format)),
            ("items", js(&self.items)),
            ("title", js(&self.title)),
            ("size", js(&self.size)),
            ("link", js(&self.link)),
            ("id", js(&self.id)),
            ("date", js(&self.date)),
            ("error", js(&self.error)),
            ("clean", js(&self.clean)),
        ])
    }
    fn from_json(j: &J) -> RespMap {
        RespMap { format: j.get("format").s(), items: j.get("items").s(), title: j.get("title").s(), size: j.get("size").s(), link: j.get("link").s(), id: j.get("id").s(), date: j.get("date").s(), error: j.get("error").s(), clean: j.get("clean").s() }
    }
}

impl Req {
    fn to_json(&self) -> String {
        let params = arr(self.params.iter().map(|p| {
            obj(&[
                ("name", js(&p.name)),
                ("value", js(&p.value)),
                ("label", js(&p.label)),
                ("place", js(&p.place)),
                ("enc", js(&p.enc)),
                ("required", p.required.to_string()),
                ("choices", pairs_js(&p.choices)),
            ])
        }));
        obj(&[
            ("id", js(&self.id)),
            ("label", js(&self.label)),
            ("help", js(&self.help)),
            ("method", js(&self.method)),
            ("url", js(&self.url)),
            ("params", params),
            ("headers", pairs_js(&self.headers)),
            ("body", js(&self.body)),
            ("resp", self.resp.to_json()),
            ("read", self.read.to_string()),
            ("origin", js(&self.origin)),
        ])
    }
    fn from_json(j: &J) -> Req {
        Req {
            id: j.get("id").s(),
            label: j.get("label").s(),
            help: j.get("help").s(),
            method: j.get("method").s(),
            url: j.get("url").s(),
            params: j
                .get("params")
                .arr()
                .iter()
                .map(|p| Param {
                    name: p.get("name").s(),
                    value: p.get("value").s(),
                    label: p.get("label").s(),
                    place: p.get("place").s(),
                    enc: p.get("enc").s(),
                    required: p.get("required").b(),
                    choices: read_pairs(p.get("choices")),
                })
                .collect(),
            headers: read_pairs(j.get("headers")),
            body: j.get("body").s(),
            resp: RespMap::from_json(j.get("resp")),
            read: j.get("read").b(),
            origin: j.get("origin").s(),
        }
    }
}

/// Marque d'un profil exporté (fichier .nzbff-api.json).
pub const EXPORT_MARK: &str = "nzbff_api_profile";

impl Profile {
    /// JSON sur une ligne (sauvegarde dans nzbff.conf). `with_key` : clé comprise (jamais à
    /// l'export : un profil se partage sans la clé).
    pub fn to_json(&self, with_key: bool) -> String {
        obj(&[
            (EXPORT_MARK, "1".into()),
            ("kind", js(&self.kind)),
            ("preset", js(&self.preset)),
            ("source", js(&self.source)),
            ("base", js(&self.base)),
            ("api_path", js(&self.api_path)),
            ("key", js(if with_key { &self.key } else { "" })),
            ("auth", js(&self.auth)),
            ("auth_name", js(&self.auth_name)),
            ("user", js(if with_key { &self.user } else { "" })),
            ("user_agent", js(&self.user_agent)),
            ("rate", self.rate.to_string()),
            ("timeout", self.timeout.to_string()),
            (
                "caps",
                obj(&[
                    ("title", js(&self.caps.title)),
                    ("limit_max", self.caps.limit_max.to_string()),
                    ("limit_default", self.caps.limit_default.to_string()),
                    ("modes", arr(self.caps.modes.iter().map(|(m, p)| format!("[{},{}]", js(m), arr(p.iter().map(|x| js(x))))))),
                    ("cats", pairs_js(&self.caps.cats)),
                    ("groups", arr(self.caps.groups.iter().map(|g| js(g)))),
                    ("read_at", self.caps.read_at.to_string()),
                ]),
            ),
            ("requests", arr(self.requests.iter().map(|r| r.to_json()))),
            ("check", js(&self.check)),
            ("query_style", js(&self.query_style)),
            ("notes", js(&self.notes)),
        ])
    }

    pub fn from_json(text: &str) -> Result<Profile, String> {
        let j = J::parse(text.trim().trim_start_matches('\u{feff}')).ok_or("profil illisible (JSON attendu)")?;
        if matches!(j.get(EXPORT_MARK), J::Null) {
            return Err("ce fichier n'est pas un profil d'API NZB FF Tool Pro".into());
        }
        let c = j.get("caps");
        Ok(Profile {
            kind: j.get("kind").s(),
            preset: j.get("preset").s(),
            source: j.get("source").s(),
            base: j.get("base").s(),
            api_path: j.get("api_path").s(),
            key: j.get("key").s(),
            auth: j.get("auth").s(),
            auth_name: j.get("auth_name").s(),
            user: j.get("user").s(),
            user_agent: j.get("user_agent").s(),
            rate: j.get("rate").f().max(0.0) as u32,
            timeout: j.get("timeout").f().max(0.0) as u32,
            caps: Caps {
                title: c.get("title").s(),
                limit_max: c.get("limit_max").f().max(0.0) as u32,
                limit_default: c.get("limit_default").f().max(0.0) as u32,
                modes: c.get("modes").arr().iter().map(|m| (m.arr().first().map(|x| x.s()).unwrap_or_default(), m.arr().get(1).map(|x| x.arr().iter().map(|p| p.s()).collect()).unwrap_or_default())).collect(),
                cats: read_pairs(c.get("cats")),
                groups: c.get("groups").arr().iter().map(|g| g.s()).filter(|g| !g.is_empty()).collect(),
                read_at: c.get("read_at").f().max(0.0) as u64,
            },
            requests: j.get("requests").arr().iter().map(Req::from_json).collect(),
            check: j.get("check").s(),
            query_style: j.get("query_style").s(),
            notes: j.get("notes").s(),
        })
    }
}

// ------------------------------------------------------------------ préréglages

/// Indexer connu : de quoi créer son profil d'un clic.
#[derive(Clone, Debug)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    /// "newznab", "nzbindex", "binsearch", "json", "html".
    pub kind: &'static str,
    pub base: &'static str,
    pub api_path: &'static str,
    /// Grandes catégories Newznab proposées par l'indexer (si connues).
    pub cats: &'static [u32],
    /// Moteur de recherche public (on y cherche, on n'y dépose pas).
    pub engine: bool,
    pub note: &'static str,
    /// Où l'adresse a été relevée.
    pub source: &'static str,
}

const ALL: &[u32] = &[1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000];

/// Préréglages : indexers Usenet repris de Prowlarr, Sonarr, Radarr et NZBHydra2 (adresses
/// relevées dans leur code en novembre 2026), moteurs publics, et modèles génériques.
pub fn presets() -> Vec<Preset> {
    fn nz(id: &'static str, name: &'static str, base: &'static str, api_path: &'static str, cats: &'static [u32], source: &'static str) -> Preset {
        Preset { id, name, kind: "newznab", base, api_path, cats, engine: false, note: "", source }
    }
    let mut v = vec![
        nz("abnzb", "abNZB", "https://abnzb.com", "/api", &[2000, 3000, 4000, 5000, 6000, 7000, 8000], "Prowlarr, NZBHydra2"),
        nz("althub", "altHUB", "https://api.althub.co.za", "/api", &[2000, 3000, 4000, 5000, 7000], "Prowlarr, NZBHydra2"),
        nz("amenzb", "ameNZB", "https://amenzb.moe", "/api", &[], "NZBHydra2"),
        nz("animetosho", "AnimeTosho (Usenet)", "https://feed.animetosho.org", "/api", &[2020, 5070], "Prowlarr"),
        nz("bluraynzb", "BlurayNZB", "https://www.bluraynzb.org", "/api", &[], "NZBHydra2"),
        nz("digitalcarnage", "Digital Carnage", "https://digitalcarnage.info", "/api", &[], "NZBHydra2"),
        nz("dognzb", "DOGnzb", "https://api.dognzb.cr", "/api", ALL, "Prowlarr, Sonarr, Radarr, NZBHydra2"),
        nz("drunkenslug", "DrunkenSlug", "https://drunkenslug.com", "/api", &[1000, 2000, 3000, 4000, 5000, 6000, 7000], "Prowlarr, Sonarr, Radarr"),
        nz("gingadaddy", "GingaDADDY", "https://www.gingadaddy.com", "/api", &[], "Prowlarr"),
        nz("miatrix", "Miatrix", "https://www.miatrix.com", "/api", ALL, "Prowlarr, NZBHydra2"),
        nz("newz69", "Newz69", "https://newz69.keagaming.com", "/api", ALL, "Prowlarr"),
        nz("ninjacentral", "NinjaCentral", "https://ninjacentral.co.za", "/api", ALL, "Prowlarr"),
        nz("nzblife", "Nzb.life", "https://api.nzb.life", "/api", ALL, "Prowlarr, Sonarr, NZBHydra2"),
        nz("nzbsu", "Nzb.su", "https://api.nzb.su", "/api", &[], "Radarr"),
        nz("nzbcat", "NZBCat", "https://nzb.cat", "/api", &[1000, 2000, 3000, 4000, 5000, 6000, 7000], "Prowlarr, Sonarr, Radarr, NZBHydra2"),
        nz("nzbfinder", "NZBFinder", "https://nzbfinder.ws", "/api", &[2000, 3000, 5000, 6000, 7000], "Prowlarr, Sonarr, Radarr, NZBHydra2"),
        nz("nzbgeek", "NZBgeek", "https://api.nzbgeek.info", "/api", ALL, "Prowlarr, Sonarr, Radarr, NZBHydra2"),
        nz("nzbnation", "NzbNation", "http://www.nzbnation.com", "/api", &[], "NZBHydra2"),
        nz("nzbnoob", "NzbNoob", "https://www.nzbnoob.com", "/api", &[2000, 3000, 4000, 5000, 6000, 7000, 8000], "Prowlarr, NZBHydra2"),
        nz("nzbndx", "NZBNDX", "https://www.nzbndx.com", "/api", ALL, "Prowlarr"),
        nz("nzbplanet", "NzbPlanet", "https://api.nzbplanet.net", "/api", ALL, "Prowlarr, Sonarr, Radarr"),
        nz("nzbstars", "NZBStars", "https://nzbstars.com", "/api", &[1000, 2000, 3000, 4000, 5000, 6000, 7000], "Prowlarr"),
        nz("omgwtfnzbs", "omgwtfnzbs", "https://api.omgwtfnzbs.org", "/api", &[], "NZBHydra2"),
        nz("onlyusenet", "Onlyusenet", "https://hou-indexer.onlyusenet.com", "/api/v1/api", &[], "NZBHydra2"),
        nz("simplynzbs", "SimplyNZBs", "https://simplynzbs.com", "/api", &[], "Sonarr, Radarr"),
        nz("spotweb", "Spotweb (spotweb.me)", "https://spotweb.me", "/api", &[], "NZBHydra2"),
        nz("tabularasa", "Tabula Rasa", "https://www.tabula-rasa.pw", "/api/v1/api", &[1000, 2000, 3000, 4000, 5000, 6000, 7000], "Prowlarr, Sonarr, Radarr, NZBHydra2"),
        Preset { note: "Catégories par langue (ex. 2145 films UHD allemands) : voir sa page RSS", ..nz("treasuremaps", "Treasure Maps", "https://treasure-maps.com", "/api", &[], "NZBHydra2") },
        nz("usenetcrawler", "Usenet Crawler", "https://www.usenet-crawler.com", "/api", &[], "Radarr"),
        Preset { id: "nzbindex", name: "NZBIndex (API JSON)", kind: "nzbindex", base: "https://nzbindex.com", api_path: "", cats: &[], engine: true, note: "Moteur public : cherche dans les en-têtes Usenet (clé facultative)", source: "Prowlarr, NZBHydra2" },
        Preset { id: "binsearch", name: "BinSearch", kind: "binsearch", base: "https://binsearch.info", api_path: "", cats: &[], engine: true, note: "Moteur public : page de résultats lue telle quelle (sans clé)", source: "Prowlarr, NZBHydra2" },
    ];
    v.sort_by_key(|p| p.name.to_lowercase());
    v.push(Preset { id: "generic_newznab", name: "Newznab générique (nZEDb, NNTmux, Spotweb, newznab+…)", kind: "newznab", base: "", api_path: "/api", cats: &[], engine: false, note: "Tout indexer compatible Newznab : adresse et clé, puis « Lire les capacités »", source: "" });
    v.push(Preset { id: "generic_json", name: "API REST / JSON (à régler)", kind: "json", base: "", api_path: "", cats: &[], engine: false, note: "Une requête de recherche à compléter ; importer sa documentation la remplit", source: "" });
    v.push(Preset { id: "generic_html", name: "Page de recherche du site (HTML)", kind: "html", base: "", api_path: "", cats: &[], engine: false, note: "Adresse de recherche du site (ex. https://indexer/search/{q} ou ?q={q})", source: "" });
    v
}

pub fn preset(id: &str) -> Option<Preset> {
    presets().into_iter().find(|p| p.id == id)
}

/// Profil créé depuis un préréglage (la clé reste celle de l'indexer).
pub fn from_preset(p: &Preset) -> Profile {
    let mut prof = Profile {
        kind: match p.kind {
            "nzbindex" => "json".into(),
            "binsearch" => "html".into(),
            k => k.into(),
        },
        preset: p.id.into(),
        source: if p.source.is_empty() { format!("Modèle « {} »", p.name) } else { format!("Préréglage {} ({})", p.name, p.source) },
        base: p.base.into(),
        api_path: p.api_path.into(),
        auth: "query".into(),
        auth_name: "apikey".into(),
        query_style: "auto".into(),
        notes: p.note.into(),
        ..Default::default()
    };
    match p.kind {
        "newznab" => {
            prof.caps.cats = p.cats.iter().map(|c| (c.to_string(), std_cat(*c).to_string())).collect();
            prof.requests = newznab_requests(&prof.caps);
        }
        "nzbindex" => {
            prof.auth_name = "key".into();
            let mut r = Req {
                id: "search".into(),
                label: "Rechercher".into(),
                help: "Recherche dans les en-têtes Usenet indexés".into(),
                method: "GET".into(),
                url: "{base}/api/search".into(),
                read: true,
                origin: "préréglage".into(),
                resp: RespMap { format: "json".into(), items: "data.content".into(), title: "name".into(), size: "size".into(), link: "id".into(), id: "id".into(), date: "posted".into(), error: "errorMessage".into(), clean: "\"{title}\"".into() },
                ..Default::default()
            };
            r.params = vec![Param::new("q", "{release}", "Nom cherché"), Param::new("max", "100", "Résultats")];
            prof.requests = vec![r];
        }
        "binsearch" => {
            prof.auth = "none".into();
            let mut r = Req {
                id: "search".into(),
                label: "Rechercher".into(),
                help: "Page de résultats de BinSearch".into(),
                method: "GET".into(),
                url: "{base}/".into(),
                read: true,
                origin: "préréglage".into(),
                resp: RespMap { format: "html".into(), items: "<span class=\"s\">{title}</span>".into(), clean: "\"{title}\"".into(), ..Default::default() },
                ..Default::default()
            };
            r.params = vec![Param::new("q", "{release}", "Nom cherché"), Param::new("max", "100", "Résultats"), Param::new("adv_col", "on", "Toutes les collections")];
            prof.requests = vec![r];
        }
        "json" => {
            let mut r = Req { id: "search".into(), label: "Rechercher".into(), method: "GET".into(), url: "{base}/api/search".into(), read: true, origin: "modèle".into(), resp: RespMap { format: "json".into(), ..Default::default() }, ..Default::default() };
            r.params = vec![Param::new("q", "{release}", "Nom cherché")];
            prof.requests = vec![r];
        }
        "html" => {
            prof.auth = "none".into();
            let mut r = Req { id: "search".into(), label: "Rechercher".into(), method: "GET".into(), url: "{base}/search".into(), read: true, origin: "modèle".into(), resp: RespMap { format: "html".into(), ..Default::default() }, ..Default::default() };
            r.params = vec![Param { enc: "+".into(), ..Param::new("q", "{release}", "Nom cherché") }];
            prof.requests = vec![r];
        }
        _ => {}
    }
    prof
}

/// Nom des catégories Newznab standard.
pub fn std_cat(id: u32) -> &'static str {
    match id {
        0 => "Réservé",
        1000 => "Console",
        2000 => "Films",
        2010 => "Films > Étrangers",
        2020 => "Films > Autres",
        2030 => "Films > SD",
        2040 => "Films > HD",
        2045 => "Films > UHD",
        2050 => "Films > BluRay",
        2060 => "Films > 3D",
        2070 => "Films > DVD",
        3000 => "Audio",
        3010 => "Audio > MP3",
        3020 => "Audio > Vidéo",
        3030 => "Audio > Livres audio",
        3040 => "Audio > Lossless",
        4000 => "PC",
        4050 => "PC > Jeux",
        5000 => "TV",
        5020 => "TV > Étrangères",
        5030 => "TV > SD",
        5040 => "TV > HD",
        5045 => "TV > UHD",
        5050 => "TV > Autres",
        5060 => "TV > Sport",
        5070 => "TV > Anime",
        5080 => "TV > Documentaires",
        6000 => "XXX",
        7000 => "Livres",
        7020 => "Livres > Ebooks",
        7030 => "Livres > BD",
        8000 => "Autres",
        _ => "",
    }
}

/// Paramètres Newznab : libellé lisible.
fn nz_label(p: &str) -> &'static str {
    match p {
        "q" => "Nom cherché",
        "cat" => "Catégories (ex. 2040,5040)",
        "season" => "Saison",
        "ep" => "Épisode",
        "imdbid" => "IMDb (sans « tt »)",
        "tmdbid" => "TMDb",
        "tvdbid" => "TVDB",
        "rid" => "TVRage",
        "tvmazeid" => "TVmaze",
        "traktid" => "Trakt",
        "artist" => "Artiste",
        "album" => "Album",
        "label" => "Label",
        "track" => "Piste",
        "year" => "Année",
        "genre" => "Genre",
        "author" => "Auteur",
        "title" => "Titre",
        "id" => "Identifiant (guid)",
        "limit" => "Nombre de résultats",
        "offset" => "À partir du résultat",
        "maxage" => "Âge maximal (jours)",
        "extended" => "Attributs complets (1)",
        _ => "",
    }
}

/// Requêtes Newznab, d'après les capacités lues (sinon l'ensemble standard).
pub fn newznab_requests(caps: &Caps) -> Vec<Req> {
    let modes: Vec<(String, Vec<String>)> = if caps.modes.is_empty() {
        vec![
            ("search".into(), vec!["q".into()]),
            ("tv-search".into(), vec!["q".into(), "season".into(), "ep".into()]),
            ("movie-search".into(), vec!["q".into(), "imdbid".into()]),
        ]
    } else {
        caps.modes.clone()
    };
    let limit = if caps.limit_max > 0 { caps.limit_max.min(100) } else { 100 }.to_string();
    let cat_choices: Vec<(String, String)> = caps.cats.iter().filter(|(id, _)| id.ends_with("000")).cloned().collect();
    let mk = |id: &str, label: &str, help: &str, t: &str, params: Vec<Param>| Req {
        id: id.into(),
        label: label.into(),
        help: help.into(),
        method: "GET".into(),
        url: format!("{{api}}?t={t}"),
        params,
        read: true,
        origin: if caps.modes.is_empty() { "newznab".into() } else { "capacités".into() },
        resp: RespMap { format: "xml".into(), ..newznab_map() },
        ..Default::default()
    };
    let p = |name: &str, value: &str| {
        let mut x = Param::new(name, value, nz_label(name));
        if name == "cat" {
            x.choices = cat_choices.clone();
            x.enc = "raw".into();
        }
        x
    };
    let mut out = vec![mk("caps", "Capacités et catégories", "Ce que l'indexer sait faire : types de recherche, paramètres, catégories, limites", "caps", vec![])];
    for (m, params) in &modes {
        let (id, label, help, t) = match m.as_str() {
            "search" => ("search", "Rechercher une release", "Recherche par nom ; c'est elle qui vérifie avant le post", "search"),
            "tv-search" => ("tvsearch", "Rechercher une série", "Nom de la série, saison et épisode, ou identifiants", "tvsearch"),
            "movie-search" => ("movie", "Rechercher un film", "Nom du film, ou identifiant IMDb / TMDb", "movie"),
            "audio-search" | "music-search" => ("music", "Rechercher de la musique", "Artiste, album…", "music"),
            "book-search" => ("book", "Rechercher un livre", "Auteur, titre…", "book"),
            _ => continue,
        };
        let mut ps: Vec<Param> = params.iter().map(|n| p(n, if n == "q" && id == "search" { "{release}" } else { "" })).collect();
        if !ps.iter().any(|x| x.name == "q") && id == "search" {
            ps.insert(0, p("q", "{release}"));
        }
        ps.push(p("cat", ""));
        ps.push(p("limit", &limit));
        ps.push(p("extended", "1"));
        out.push(mk(id, label, help, t, ps));
    }
    out.push(mk("details", "Détails d'une release", "Identifiant (guid) d'un résultat de recherche", "details", vec![Param { required: true, ..p("id", "") }]));
    out.push(mk("latest", "Dernières releases", "Les plus récentes, dans une catégorie facultative", "search", vec![p("cat", ""), p("limit", "25"), p("extended", "1")]));
    out.push(Req { resp: RespMap { format: "text".into(), ..Default::default() }, ..mk("get", "Télécharger un NZB", "Renvoie le NZB d'un résultat (identifiant guid)", "get", vec![Param { required: true, ..p("id", "") }]) });
    out
}

/// Lecture standard d'une réponse Newznab (RSS).
pub fn newznab_map() -> RespMap {
    RespMap { format: "xml".into(), items: "item".into(), title: "title".into(), size: "newznab:attr[name=size]@value|enclosure@length|size".into(), link: "guid|link|comments".into(), id: "newznab:attr[name=guid]@value|guid".into(), date: "pubDate".into(), error: "error@description".into(), clean: String::new() }
}

/// Lit une réponse `t=caps`.
pub fn parse_caps(xml: &str) -> Result<Caps, String> {
    let doc = Xml::parse(xml).ok_or("réponse illisible (XML attendu)")?;
    if let Some(e) = doc.find("error") {
        return Err(newznab_error(&e.attr("code"), &e.attr("description")));
    }
    let root = doc.find("caps").ok_or("ce n'est pas une réponse « caps » Newznab")?;
    let mut c = Caps { read_at: crate::prep::now_secs(), ..Default::default() };
    if let Some(s) = root.child("server") {
        c.title = s.attr("title");
    }
    if let Some(l) = root.child("limits") {
        c.limit_max = l.attr("max").parse().unwrap_or(0);
        c.limit_default = l.attr("default").parse().unwrap_or(0);
    }
    if let Some(s) = root.child("searching") {
        for m in &s.children {
            if m.attr("available").eq_ignore_ascii_case("yes") {
                let params: Vec<String> = m.attr("supportedParams").split(',').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect();
                c.modes.push((m.name.to_lowercase(), if params.is_empty() { vec!["q".into()] } else { params }));
            }
        }
    }
    if let Some(cats) = root.child("categories") {
        for cat in cats.children.iter().filter(|x| x.name == "category") {
            let (id, name) = (cat.attr("id"), cat.attr("name"));
            c.cats.push((id.clone(), name.clone()));
            for s in cat.children.iter().filter(|x| x.name == "subcat") {
                c.cats.push((s.attr("id"), format!("{name} > {}", s.attr("name"))));
            }
        }
    }
    c.groups = crate::upload::caps_groups(xml);
    Ok(c)
}

/// Message d'erreur Newznab en clair (codes de la spécification).
pub fn newznab_error(code: &str, desc: &str) -> String {
    let fr = match code.trim() {
        "100" => "identifiants refusés : clé API incorrecte",
        "101" => "compte suspendu",
        "102" => "droits insuffisants pour cette fonction",
        "103" | "104" | "105" | "106" | "107" => "inscription refusée",
        "200" => "paramètre manquant",
        "201" => "paramètre incorrect",
        "202" => "fonction inconnue",
        "203" => "fonction non disponible sur cet indexer",
        "300" => "élément introuvable",
        "500" => "limite de requêtes atteinte",
        "501" => "limite de téléchargements atteinte",
        "910" => "API désactivée",
        _ => "",
    };
    match (fr.is_empty(), desc.trim().is_empty()) {
        (true, true) => format!("erreur {code} de l'API"),
        (true, false) => format!("erreur {code} de l'API : {}", desc.trim()),
        (false, true) => format!("{fr} (code {code})"),
        (false, false) => format!("{fr} (code {code} : {})", desc.trim()),
    }
}

// ------------------------------------------------------------------ XML (petit lecteur tolérant)

/// Élément XML : nom, attributs, enfants, texte.
#[derive(Clone, Debug, Default)]
pub struct Xml {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Xml>,
    pub text: String,
}

pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let end = after[..after.len().min(12)].find(';');
        match end {
            Some(e) => {
                let ent = &after[1..e];
                let rep = match ent {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
                    _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
                    _ => None,
                };
                match rep {
                    Some(c) => {
                        out.push(c);
                        rest = &after[e + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = &after[1..];
                    }
                }
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

impl Xml {
    /// Document entier (racine fictive « # »). None si ce n'est pas du XML.
    pub fn parse(src: &str) -> Option<Xml> {
        let t = src.trim_start_matches('\u{feff}').trim_start();
        if !t.starts_with('<') {
            return None;
        }
        let mut stack: Vec<Xml> = vec![Xml { name: "#".into(), ..Default::default() }];
        let b = t.as_bytes();
        let mut i = 0;
        while i < t.len() {
            if b[i] == b'<' {
                if t[i..].starts_with("<!--") {
                    i = t[i..].find("-->").map(|e| i + e + 3).unwrap_or(t.len());
                } else if t[i..].starts_with("<![CDATA[") {
                    let e = t[i..].find("]]>").map(|e| i + e).unwrap_or(t.len());
                    // contenu brut ; les flux qui y échappent quand même « &amp; » sont tolérés
                    stack.last_mut()?.text.push_str(&unescape(&t[i + 9..e]));
                    i = (e + 3).min(t.len());
                } else if t[i..].starts_with("<?") || t[i..].starts_with("<!") {
                    i = t[i..].find('>').map(|e| i + e + 1).unwrap_or(t.len());
                } else if t[i..].starts_with("</") {
                    let e = t[i..].find('>').map(|e| i + e).unwrap_or(t.len());
                    let name = t[i + 2..e].trim().to_string();
                    // ferme jusqu'à l'élément du même nom (tolérant aux balises non fermées)
                    if stack.iter().skip(1).any(|x| x.name == name) {
                        while stack.len() > 1 {
                            let x = stack.pop()?;
                            let done = x.name == name;
                            stack.last_mut()?.children.push(x);
                            if done {
                                break;
                            }
                        }
                    }
                    i = (e + 1).min(t.len());
                } else {
                    // balise ouvrante : nom, attributs (guillemets respectés), « /> »
                    let mut j = i + 1;
                    let mut quote: Option<u8> = None;
                    while j < t.len() {
                        match (quote, b[j]) {
                            (Some(q), c) if c == q => quote = None,
                            (None, b'"') | (None, b'\'') => quote = Some(b[j]),
                            (None, b'>') => break,
                            _ => {}
                        }
                        j += 1;
                    }
                    let inner = &t[i + 1..j.min(t.len())];
                    let self_close = inner.ends_with('/');
                    let inner = inner.trim_end_matches('/');
                    let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
                    let mut el = Xml { name: inner[..name_end].to_string(), ..Default::default() };
                    el.attrs = parse_attrs(&inner[name_end..]);
                    if self_close {
                        stack.last_mut()?.children.push(el);
                    } else {
                        stack.push(el);
                    }
                    i = (j + 1).min(t.len());
                }
            } else {
                let e = t[i..].find('<').map(|e| i + e).unwrap_or(t.len());
                stack.last_mut()?.text.push_str(&unescape(&t[i..e]));
                i = e;
            }
        }
        while stack.len() > 1 {
            let x = stack.pop()?;
            stack.last_mut()?.children.push(x);
        }
        stack.pop()
    }

    pub fn attr(&self, n: &str) -> String {
        self.attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone()).unwrap_or_default()
    }
    pub fn child(&self, n: &str) -> Option<&Xml> {
        self.children.iter().find(|c| c.name.eq_ignore_ascii_case(n))
    }
    /// Premier élément de ce nom, à n'importe quelle profondeur.
    pub fn find(&self, n: &str) -> Option<&Xml> {
        if self.name.eq_ignore_ascii_case(n) {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(n))
    }
    /// Tous les éléments de ce nom (chemin « a/b » : b enfants de a), à n'importe quelle profondeur.
    pub fn all<'a>(&'a self, path: &str, out: &mut Vec<&'a Xml>) {
        let parts: Vec<&str> = path.split('/').map(str::trim).filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return;
        }
        if self.name.eq_ignore_ascii_case(parts[0]) {
            let mut cur: Vec<&Xml> = vec![self];
            for p in &parts[1..] {
                cur = cur.iter().flat_map(|x| x.children.iter().filter(|c| c.name.eq_ignore_ascii_case(p))).collect();
            }
            out.extend(cur);
            return;
        }
        for c in &self.children {
            c.all(path, out);
        }
    }
    /// Texte complet (descendants compris).
    pub fn full_text(&self) -> String {
        let mut s = self.text.clone();
        for c in &self.children {
            s.push_str(&c.full_text());
        }
        s
    }
    /// Valeur d'un champ : « title », « enclosure@length », « @id »,
    /// « newznab:attr[name=size]@value » ; alternatives séparées par « | ».
    pub fn field(&self, spec: &str) -> String {
        for alt in spec.split('|').map(str::trim).filter(|a| !a.is_empty()) {
            let (path, at) = match alt.rsplit_once('@') {
                Some((p, a)) => (p, Some(a)),
                None => (alt, None),
            };
            let el: Option<&Xml> = if path.is_empty() {
                Some(self)
            } else {
                // filtre [attr=valeur] sur le dernier élément
                let (p, filt) = match path.split_once('[') {
                    Some((p, f)) => (p, f.trim_end_matches(']').split_once('=').map(|(a, v)| (a.trim().to_string(), v.trim().trim_matches(['"', '\'']).to_string()))),
                    None => (path, None),
                };
                let mut found = Vec::new();
                for c in &self.children {
                    c.all(p, &mut found);
                }
                found.into_iter().find(|x| filt.as_ref().map(|(a, v)| x.attr(a).eq_ignore_ascii_case(v)).unwrap_or(true))
            };
            if let Some(el) = el {
                let v = match at {
                    Some(a) => el.attr(a),
                    None => el.full_text(),
                };
                let v = v.trim().to_string();
                if !v.is_empty() {
                    return v;
                }
            }
        }
        String::new()
    }
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < s.len() {
        while i < s.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let ns = i;
        while i < s.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' {
            i += 1;
        }
        let name = s[ns..i].to_string();
        while i < s.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < s.len() && b[i] == b'=' {
            i += 1;
            while i < s.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            let v = if i < s.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let st = i + 1;
                let e = s[st..].bytes().position(|c| c == q).map(|e| st + e).unwrap_or(s.len());
                i = (e + 1).min(s.len());
                &s[st..e]
            } else {
                let st = i;
                while i < s.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                &s[st..i]
            };
            if !name.is_empty() {
                out.push((name, unescape(v)));
            }
        } else if !name.is_empty() {
            out.push((name, String::new()));
        }
    }
    out
}

// ------------------------------------------------------------------ JSON : chemins

/// Valeur au chemin « a.b.0.c » (alternatives « | »), en texte.
pub fn jpath<'a>(j: &'a J, path: &str) -> &'a J {
    let mut cur = j;
    for part in path.split('.').map(str::trim).filter(|p| !p.is_empty()) {
        cur = match cur {
            J::Arr(v) => part.parse::<usize>().ok().and_then(|i| v.get(i)).unwrap_or(&J::Null),
            J::Obj(_) => cur.get(part),
            _ => &J::Null,
        };
    }
    cur
}

pub fn jtext(j: &J) -> String {
    match j {
        J::Str(s) => s.clone(),
        J::Num(n) if n.fract() == 0.0 && n.abs() < 1e17 => format!("{}", *n as i64),
        J::Num(n) => n.to_string(),
        J::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

pub fn jfield(j: &J, spec: &str) -> String {
    spec.split('|').map(str::trim).filter(|a| !a.is_empty()).map(|a| jtext(jpath(j, a))).find(|v| !v.trim().is_empty()).unwrap_or_default()
}

// ------------------------------------------------------------------ motifs (pages HTML, nettoyage)

#[derive(Debug, PartialEq)]
enum Tok {
    Lit(String),
    Cap(String),
    Any,
}

fn motif(p: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut lit = String::new();
    let mut rest = p;
    while !rest.is_empty() {
        if rest.starts_with('*') {
            if !lit.is_empty() {
                out.push(Tok::Lit(std::mem::take(&mut lit)));
            }
            out.push(Tok::Any);
            rest = &rest[1..];
        } else if rest.starts_with('{') && rest.find('}').map(|e| rest[1..e].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && e > 1).unwrap_or(false) {
            if !lit.is_empty() {
                out.push(Tok::Lit(std::mem::take(&mut lit)));
            }
            let e = rest.find('}').unwrap_or(rest.len());
            out.push(Tok::Cap(rest[1..e].to_string()));
            rest = &rest[e + 1..];
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            lit.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    if !lit.is_empty() {
        out.push(Tok::Lit(lit));
    }
    out
}

/// Toutes les correspondances d'un motif dans un texte : champs capturés (texte sans balises).
/// `{nom}` capture jusqu'au texte fixe qui suit ; `*` saute n'importe quoi.
pub fn motif_all(text: &str, pattern: &str) -> Vec<Vec<(String, String)>> {
    let toks = motif(pattern);
    let mut out = Vec::new();
    if !toks.iter().any(|t| matches!(t, Tok::Cap(_))) {
        return out;
    }
    let mut pos = 0;
    'outer: while pos < text.len() && out.len() < 2000 {
        let mut i = pos;
        let mut caps: Vec<(String, String)> = Vec::new();
        // Some(None) : « * » en attente ; Some(Some(n)) : capture {n} en attente
        let mut pending: Option<Option<String>> = None;
        for t in &toks {
            match t {
                Tok::Lit(l) => {
                    let Some(f) = text[i..].find(l.as_str()).map(|f| i + f) else { break 'outer };
                    if let Some(Some(n)) = pending.take() {
                        caps.push((n, clean_text(&text[i..f])));
                    }
                    i = f + l.len();
                }
                Tok::Cap(n) => pending = Some(Some(n.clone())),
                Tok::Any => pending = Some(None),
            }
        }
        // capture finale : jusqu'à la prochaine balise ou la fin de la ligne
        if let Some(Some(n)) = pending {
            let e = text[i..].find(['<', '\n']).map(|e| i + e).unwrap_or(text.len());
            caps.push((n, clean_text(&text[i..e])));
            i = e;
        }
        out.push(caps);
        pos = if i > pos { i } else { pos + text[pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(1) };
    }
    out
}

/// Texte d'un morceau de HTML : balises retirées, entités décodées, espaces réduits.
pub fn clean_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    unescape(&out).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Applique un motif de nettoyage (« "{title}" ») ; inchangé s'il ne correspond pas.
pub fn clean_title(t: &str, pattern: &str) -> String {
    let t = t.trim();
    let mut v = if pattern.trim().is_empty() {
        t.to_string()
    } else {
        motif_all(t, pattern).into_iter().next().and_then(|c| c.into_iter().find(|(n, _)| n == "title").map(|(_, v)| v)).filter(|v| !v.is_empty()).unwrap_or_else(|| t.to_string())
    };
    // extension d'un fichier (« Nom.mkv ») retirée, comme sur les moteurs publics
    let stem = crate::search::origin_stem(&v).to_string();
    if stem.len() < v.len() {
        v = stem;
    }
    v
}

/// « 1.5 GB », « 700 Mo », « 123456 » → octets.
pub fn parse_size(s: &str) -> u64 {
    let t = s.trim().replace(',', ".").replace('\u{a0}', " ");
    if let Ok(n) = t.parse::<f64>() {
        return n.max(0.0) as u64;
    }
    let num: String = t.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let unit = t[num.len()..].trim().to_lowercase();
    let n: f64 = num.parse().unwrap_or(0.0);
    let m = match unit.chars().next() {
        Some('k') => 1024.0,
        Some('m') => 1024.0 * 1024.0,
        Some('g') => 1024.0 * 1024.0 * 1024.0,
        Some('t') => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (n * m) as u64
}

// ------------------------------------------------------------------ profil effectif, connexion

/// Profil utilisé pour cet indexer : le sien, complété (automatique : Newznab du dépôt).
pub fn profile(idx: &IndexerCfg) -> Profile {
    let mut p = idx.api.clone();
    if p.is_auto() {
        p.requests = newznab_requests(&p.caps);
    }
    if p.auth.is_empty() {
        p.auth = "query".into();
    }
    if p.auth_name.trim().is_empty() && p.auth != "basic" {
        p.auth_name = match p.auth.as_str() {
            "header" => "X-Api-Key".into(),
            "cookie" => "session".into(),
            _ => "apikey".into(),
        };
    }
    if p.query_style.is_empty() {
        p.query_style = "auto".into();
    }
    p
}

/// Adresses et clé résolues : {base}, {api}, {key}.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conn {
    pub base: String,
    pub api: String,
    pub key: String,
}

pub fn conn(idx: &IndexerCfg, p: &Profile) -> Conn {
    let key = [p.key.trim(), idx.api_key.trim(), idx.rest_key.trim()].into_iter().find(|k| !k.is_empty()).unwrap_or("").to_string();
    let nz = p.is_auto() || p.kind == "newznab";
    let path = if p.api_path.trim().is_empty() && nz { "/api".to_string() } else { p.api_path.trim().to_string() };
    let (base, api) = if !p.base.trim().is_empty() {
        let b = p.base.trim().trim_end_matches('/').to_string();
        let b = if b.contains("://") { b } else { format!("https://{b}") };
        (b.clone(), format!("{b}{}", if path.is_empty() || path.starts_with('/') { path.clone() } else { format!("/{path}") }))
    } else if !idx.api_url.trim().is_empty() && (nz || idx.rest_url.trim().is_empty()) {
        let api = if nz { endpoint(&idx.api_url) } else { idx.api_url.trim().trim_end_matches('/').to_string() };
        let base = site_root(&api, &path);
        (base, api)
    } else if !idx.rest_url.trim().is_empty() {
        let b = idx.rest_url.trim().trim_end_matches('/').to_string();
        (site_root(&b, ""), b)
    } else {
        (String::new(), String::new())
    };
    Conn { base, api, key }
}

/// Racine d'une adresse d'API : « https://x/api » → « https://x » (chemin de l'API retiré).
fn site_root(api: &str, path: &str) -> String {
    let a = api.trim_end_matches('/');
    if !path.is_empty() && a.ends_with(path.trim_end_matches('/')) {
        return a[..a.len() - path.trim_end_matches('/').len()].to_string();
    }
    if let Some(i) = a.find("://") {
        let host_end = a[i + 3..].find('/').map(|e| i + 3 + e).unwrap_or(a.len());
        return a[..host_end].to_string();
    }
    a.to_string()
}

/// Le profil permet-il de chercher (adresse, et clé si l'authentification en demande une) ?
pub fn ready(idx: &IndexerCfg, p: &Profile) -> Result<(), String> {
    let c = conn(idx, p);
    if c.base.is_empty() {
        return Err("adresse de l'indexer à renseigner".into());
    }
    if c.key.is_empty() && p.auth != "none" && (p.is_auto() || p.kind == "newznab") {
        return Err("clé API à renseigner".into());
    }
    Ok(())
}

// ------------------------------------------------------------------ construction d'une requête

/// Requête prête à partir.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Built {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// (type de contenu, corps).
    pub body: Option<(String, String)>,
    /// Identifiant:mot de passe (authentification Basic).
    pub basic: Option<String>,
}

fn enc_value(v: &str, enc: &str) -> String {
    match enc {
        "raw" => v.to_string(),
        "+" => encode(v).replace("%20", "+"),
        _ => encode(v),
    }
}

fn subst(t: &str, vars: &[(String, String)], json_escape: bool) -> String {
    let mut s = t.to_string();
    for (k, v) in vars {
        let needle = format!("{{{k}}}");
        if s.contains(&needle) {
            let v = if json_escape { js(v).trim_matches('"').to_string() } else { v.clone() };
            s = s.replace(&needle, &v);
        }
    }
    s
}

/// Placeholders encore présents (« {xxx} »).
fn leftovers(s: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut rest = s;
    while let Some(a) = rest.find('{') {
        let after = &rest[a + 1..];
        match after.find('}') {
            Some(b) if b > 0 && after[..b].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
                v.push(after[..b].to_string());
                rest = &after[b..];
            }
            _ => rest = after,
        }
    }
    v
}

/// Construit une requête : valeurs saisies (`inputs`, dont « release ») sinon valeurs modèles.
pub fn build(idx: &IndexerCfg, p: &Profile, r: &Req, inputs: &[(String, String)]) -> Result<Built, String> {
    let c = conn(idx, p);
    let input = |n: &str| inputs.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
    let mut vars: Vec<(String, String)> = vec![("base".into(), c.base.clone()), ("api".into(), c.api.clone()), ("key".into(), c.key.clone()), ("apikey".into(), c.key.clone()), ("user".into(), p.user.clone()), ("release".into(), input("release").unwrap_or_default())];
    // valeurs des paramètres (modèles remplis)
    let mut values: Vec<(&Param, String)> = Vec::new();
    for prm in &r.params {
        let raw = input(&prm.name).unwrap_or_else(|| prm.value.clone());
        let v = subst(&raw, &vars, false).trim().to_string();
        if v.is_empty() || !leftovers(&v).is_empty() {
            if prm.required {
                return Err(format!("« {} » est à remplir", if prm.label.is_empty() { &prm.name } else { &prm.label }));
            }
            continue;
        }
        values.push((prm, v));
    }
    for (prm, v) in &values {
        vars.push((prm.name.clone(), v.clone()));
    }
    if r.url.contains("{release}") && vars[5].1.trim().is_empty() {
        return Err("« Nom cherché » est à remplir".into());
    }
    // adresse : {base} {api} tels quels ; clé, identifiant et nom cherché encodés
    let mut url = subst(r.url.trim(), &vars[..2], false);
    for (k, v) in &vars[2..6] {
        url = url.replace(&format!("{{{k}}}"), &encode(v));
    }
    if url.is_empty() {
        return Err("adresse de la requête à renseigner".into());
    }
    // paramètres de chemin : {nom} dans l'adresse
    let mut query: Vec<String> = Vec::new();
    let mut form: Vec<String> = Vec::new();
    let mut json: Vec<(String, String)> = Vec::new();
    let mut headers: Vec<(String, String)> = Vec::new();
    for (prm, v) in &values {
        let needle = format!("{{{}}}", prm.name);
        if url.contains(&needle) || prm.place == "path" {
            url = url.replace(&needle, &enc_value(v, if prm.enc.is_empty() { "%20" } else { &prm.enc }));
            continue;
        }
        match prm.place.as_str() {
            "form" => form.push(format!("{}={}", encode(&prm.name), enc_value(v, if prm.enc == "raw" { "raw" } else { "+" }))),
            "json" => json.push((prm.name.clone(), v.clone())),
            "header" => headers.push((prm.name.clone(), v.clone())),
            _ => query.push(format!("{}={}", prm.name, enc_value(v, if prm.enc.is_empty() { "%20" } else { &prm.enc }))),
        }
    }
    // valeurs facultatives laissées vides dans l'adresse : {nom} retiré avec son paramètre
    let left = leftovers(&url);
    if !left.is_empty() {
        let (head, q) = url.split_once('?').map(|(a, b)| (a.to_string(), Some(b.to_string()))).unwrap_or((url.clone(), None));
        if !leftovers(&head).is_empty() {
            return Err(format!("valeur manquante dans l'adresse : {}", leftovers(&head).iter().map(|x| format!("{{{x}}}")).collect::<Vec<_>>().join(", ")));
        }
        url = match q {
            Some(q) => {
                let kept: Vec<&str> = q.split('&').filter(|x| !x.is_empty() && leftovers(x).is_empty()).collect();
                if kept.is_empty() {
                    head
                } else {
                    format!("{head}?{}", kept.join("&"))
                }
            }
            None => head,
        };
    }
    if !url.contains("://") {
        return Err(format!("adresse incomplète : « {url} » (adresse de l'indexer à renseigner)"));
    }
    // authentification
    let mut basic = None;
    // clé déjà placée par la requête elle-même (paramètre {key}, en-tête) : pas en double
    let placed = url.contains(&encode(&c.key)) || url.contains(&c.key) || query.iter().any(|q| q.contains(&encode(&c.key)) || q.contains(&c.key)) || headers.iter().any(|(_, v)| v.contains(&c.key));
    if !c.key.is_empty() && !placed {
        match p.auth.as_str() {
            "query" => query.push(format!("{}={}", if p.auth_name.trim().is_empty() { "apikey" } else { p.auth_name.trim() }, encode(&c.key))),
            "header" => headers.push((p.auth_name.trim().to_string(), c.key.clone())),
            "bearer" => headers.push(("Authorization".into(), format!("Bearer {}", c.key))),
            "cookie" => headers.push(("Cookie".into(), format!("{}={}", p.auth_name.trim(), c.key))),
            "basic" => basic = Some(format!("{}:{}", p.user.trim(), c.key)),
            _ => {}
        }
    }
    if !query.is_empty() {
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str(&query.join("&"));
    }
    for (k, v) in &r.headers {
        if !k.trim().is_empty() {
            headers.push((k.trim().to_string(), subst(v, &vars, false)));
        }
    }
    // identité réseau : celle de l'indexer, sinon le réglage général (aucun en-tête : curl
    // envoie le sien, ou celui du réglage général ajouté par `run_curl`)
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("user-agent")) {
        if let Some(ua) = ua_for(&p.user_agent) {
            headers.push(("User-Agent".into(), ua));
        }
    }
    let body = if !r.body.trim().is_empty() {
        let t = r.body.trim();
        let is_json = t.starts_with('{') || t.starts_with('[');
        Some((if is_json { "application/json" } else { "text/plain" }.to_string(), subst(t, &vars, is_json)))
    } else if !json.is_empty() {
        let fields: Vec<String> = json
            .iter()
            .map(|(k, v)| {
                let lit = v.parse::<f64>().is_ok() && !v.starts_with('0') || v == "0" || v == "true" || v == "false";
                format!("{}:{}", js(k), if lit { v.clone() } else { js(v) })
            })
            .collect();
        Some(("application/json".into(), format!("{{{}}}", fields.join(","))))
    } else if !form.is_empty() {
        Some(("application/x-www-form-urlencoded".into(), form.join("&")))
    } else {
        None
    };
    Ok(Built { method: r.method(), url, headers, body, basic })
}

/// La requête telle qu'elle partira, clé masquée.
pub fn preview(b: &Built, key: &str) -> Vec<String> {
    let m = |s: &str| crate::search::mask(s, key);
    let mut v = vec![format!("{} {}", b.method, m(&b.url))];
    for (k, val) in &b.headers {
        let shown = if ["authorization", "cookie"].contains(&k.to_lowercase().as_str()) || val.contains(key) && !key.is_empty() { "••••".to_string() } else { m(val) };
        v.push(format!("en-tête  {k}: {shown}"));
    }
    if b.basic.is_some() {
        v.push("identifiant et mot de passe : envoyés (Basic)".into());
    }
    if let Some((ct, body)) = &b.body {
        v.push(format!("corps ({ct}) : {}", m(&body.chars().take(600).collect::<String>())));
    }
    v
}

// ------------------------------------------------------------------ envoi

/// Réponse d'une requête, lue.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub preview: Vec<String>,
    pub http: Option<u16>,
    pub secs: f64,
    /// Corps (clé masquée).
    pub body: String,
    /// Format lu : xml, json, html, text.
    pub format: String,
    pub rows: Vec<Row>,
    /// Erreur signalée par l'API (ou HTTP).
    pub error: Option<String>,
    pub summary: String,
}

static RATE: Mutex<Option<HashMap<String, Vec<Instant>>>> = Mutex::new(None);

/// Attend s'il le faut pour rester sous `per_min` requêtes par minute vers cet indexer.
fn rate_wait(name: &str, per_min: u32) {
    if per_min == 0 {
        return;
    }
    loop {
        let wait = {
            let Ok(mut g) = RATE.lock() else { return };
            let m = g.get_or_insert_with(HashMap::new);
            let v = m.entry(name.to_lowercase()).or_default();
            v.retain(|t| t.elapsed() < Duration::from_secs(60));
            if (v.len() as u32) < per_min {
                v.push(Instant::now());
                None
            } else {
                v.first().map(|t| Duration::from_secs(60).saturating_sub(t.elapsed()) + Duration::from_millis(50))
            }
        };
        match wait {
            None => return,
            Some(d) => std::thread::sleep(d.min(Duration::from_secs(60))),
        }
    }
}

/// Arguments curl (la partie secrète passera par `-K`).
fn curl_args(b: &Built, timeout: u32) -> Vec<String> {
    let mut a: Vec<String> = vec!["-sS".into(), "-L".into(), "--max-time".into(), (if timeout == 0 { 40 } else { timeout }).to_string(), "-w".into(), "\n@@HTTP@@%{http_code}".into()];
    if b.method != "GET" {
        a.push("-X".into());
        a.push(b.method.clone());
    }
    if !b.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("accept")) {
        a.push("-H".into());
        a.push("Accept: application/json, application/xml;q=0.9, text/html;q=0.8, */*;q=0.5".into());
    }
    for (k, v) in &b.headers {
        a.push("-H".into());
        a.push(format!("{k}: {v}"));
    }
    if let Some(u) = &b.basic {
        a.push("-u".into());
        a.push(u.clone());
    }
    if let Some((ct, body)) = &b.body {
        a.push("-H".into());
        a.push(format!("Content-Type: {ct}"));
        a.push("--data-raw".into());
        a.push(body.clone());
    }
    a.push(b.url.clone());
    a
}

/// Envoie une requête et lit la réponse.
pub fn send(idx: &IndexerCfg, p: &Profile, r: &Req, inputs: &[(String, String)]) -> Result<Reply, String> {
    let c = conn(idx, p);
    let b = build(idx, p, r, inputs)?;
    let mut rep = Reply { preview: preview(&b, &c.key), ..Default::default() };
    rate_wait(idx.name.trim(), p.rate);
    let t = Instant::now();
    let (body, http) = run_curl(&curl_args(&b, p.timeout)).map_err(|e| crate::search::mask(&e.message, &c.key))?;
    rep.secs = t.elapsed().as_secs_f64();
    rep.http = http;
    rep.format = detect(&body, &r.resp.format);
    let read = extract(&body, &r.resp, &rep.format);
    match read {
        Ok(rows) => rep.rows = rows,
        Err(e) => rep.error = Some(crate::search::mask(&e, &c.key)),
    }
    if rep.error.is_none() {
        if let Some(code) = http.filter(|c| !(200..300).contains(c)) {
            rep.error = Some(match code {
                401 | 403 => format!("accès refusé (HTTP {code}) : clé ou identifiants à vérifier"),
                404 => "adresse introuvable (HTTP 404) : adresse ou chemin de l'API à vérifier".into(),
                429 => "trop de requêtes (HTTP 429) : réduire la limite par minute".into(),
                c => format!("HTTP {c} {}", crate::upload::http_reason(c)),
            });
        }
    }
    rep.body = crate::search::mask(&crate::search::pretty(&body), &c.key);
    rep.summary = summary(&rep);
    Ok(rep)
}

fn summary(r: &Reply) -> String {
    let code = r.http.map(|c| format!("HTTP {c} {}", crate::upload::http_reason(c))).unwrap_or_else(|| "pas de code HTTP".into());
    if let Some(e) = &r.error {
        return format!("{code} · ✖ {e}");
    }
    let low = r.body.to_lowercase();
    if low.contains("<caps") {
        return format!("{code} · capacités : {} catégorie(s)", low.matches("<category ").count() + low.matches("<subcat ").count());
    }
    match r.rows.len() {
        0 if r.format == "html" => format!("{code} · page reçue ({} Ko) ; aucun résultat lu avec le motif", r.body.len() / 1024),
        0 => format!("{code} · aucun résultat"),
        n => format!("{code} · {n} résultat(s)"),
    }
}

/// Format d'une réponse : celui réglé, sinon d'après son contenu.
pub fn detect(body: &str, want: &str) -> String {
    if !want.is_empty() && want != "auto" {
        return want.to_string();
    }
    let t = body.trim_start_matches('\u{feff}').trim_start();
    let low: String = t.chars().take(400).collect::<String>().to_lowercase();
    if t.starts_with('{') || t.starts_with('[') {
        "json".into()
    } else if low.starts_with("<!doctype html") || low.contains("<html") {
        "html".into()
    } else if t.starts_with('<') {
        "xml".into()
    } else {
        "text".into()
    }
}

// ------------------------------------------------------------------ lecture des réponses

const T_TITLE: &str = "title|name|release|searchname|release_name|releaseName|filename|subject";
const T_SIZE: &str = "size|bytes|filesize|size_bytes|length";
const T_LINK: &str = "link|url|guid|download|download_url|details|details_url";
const T_ID: &str = "id|guid|uuid";
const T_DATE: &str = "pubDate|posted|date|added|created_at|usenetdate";

/// Résultats d'une réponse, selon la lecture réglée.
pub fn extract(body: &str, m: &RespMap, format: &str) -> Result<Vec<Row>, String> {
    let or = |s: &str, d: &str| if s.trim().is_empty() { d.to_string() } else { s.to_string() };
    match format {
        "xml" => {
            let doc = Xml::parse(body).ok_or("réponse XML illisible")?;
            // erreur de l'API (Newznab : <error code="100" description="…"/>)
            let err_spec = or(&m.error, "error@description");
            let err_el = err_spec.split('|').next().unwrap_or("").split('@').next().unwrap_or("").to_string();
            if !err_el.is_empty() {
                if let Some(e) = doc.find(&err_el) {
                    let code = e.attr("code");
                    let msg = e.field(&format!("@{}", err_spec.split('@').nth(1).unwrap_or("description")));
                    let msg = if msg.is_empty() { e.full_text().trim().to_string() } else { msg };
                    return Err(if code.is_empty() { format!("erreur de l'API : {msg}") } else { newznab_error(&code, &msg) });
                }
            }
            let mut items = Vec::new();
            doc.all(&or(&m.items, "item"), &mut items);
            Ok(items
                .iter()
                .map(|x| Row { title: clean_title(&x.field(&or(&m.title, "title")), &m.clean), size: parse_size(&x.field(&or(&m.size, "newznab:attr[name=size]@value|enclosure@length|size"))), link: x.field(&or(&m.link, "guid|link")), id: x.field(&or(&m.id, "newznab:attr[name=guid]@value|guid")), date: x.field(&or(&m.date, "pubDate")), cat: x.field("newznab:attr[name=category]@value|category"), nzb: x.field("enclosure@url") })
                .filter(|r| !r.title.is_empty())
                .collect())
        }
        "json" => {
            let j = J::parse(body.trim().trim_start_matches('\u{feff}')).ok_or("réponse JSON illisible")?;
            if let Some(e) = json_error(&j, &m.error) {
                return Err(e);
            }
            let list: Vec<&J> = if m.items.trim().is_empty() {
                auto_items(&j).map(|(_, v)| v.iter().collect()).unwrap_or_default()
            } else {
                match jpath(&j, m.items.trim()) {
                    J::Arr(v) => v.iter().collect(),
                    J::Obj(_) => vec![jpath(&j, m.items.trim())],
                    _ => Vec::new(),
                }
            };
            Ok(list
                .into_iter()
                .map(|x| Row { title: clean_title(&jfield(x, &or(&m.title, T_TITLE)), &m.clean), size: parse_size(&jfield(x, &or(&m.size, T_SIZE))), link: jfield(x, &or(&m.link, T_LINK)), id: jfield(x, &or(&m.id, T_ID)), date: jfield(x, &or(&m.date, T_DATE)), cat: jfield(x, "category|categoryId|category_id|cat"), nzb: jfield(x, "enclosure.url|enclosure|nzb_url|nzbUrl|nzb|download_url|downloadUrl") })
                .filter(|r| !r.title.is_empty())
                .collect())
        }
        "html" if !m.items.trim().is_empty() => Ok(motif_all(body, m.items.trim())
            .into_iter()
            .map(|caps| {
                let g = |n: &str| caps.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone()).unwrap_or_default();
                Row { title: clean_title(&g("title"), &m.clean), size: parse_size(&g("size")), link: g("link"), id: g("id"), date: g("date"), cat: g("cat"), nzb: g("nzb") }
            })
            .filter(|r| !r.title.is_empty())
            .collect()),
        _ => Ok(Vec::new()),
    }
}

/// Erreur d'une réponse JSON : texte, objet { message }, ou booléen vrai (+ message à côté).
fn json_error(j: &J, spec: &str) -> Option<String> {
    let spec = if spec.trim().is_empty() { "error|errors|errorMessage|error_message|message|detail" } else { spec };
    let side = || ["errorMessage", "error_message", "message", "detail", "description"].iter().map(|k| jtext(j.get(k))).find(|s| !s.is_empty());
    for alt in spec.split('|').map(str::trim).filter(|a| !a.is_empty()) {
        match jpath(j, alt) {
            J::Str(s) if !s.trim().is_empty() && !["false", "0", "ok", "success"].contains(&s.trim().to_lowercase().as_str()) => {
                // « message » seul n'est une erreur que s'il n'y a pas de résultats
                if ["message", "detail"].contains(&alt) && auto_items(j).is_some() {
                    continue;
                }
                return Some(format!("réponse de l'API : {}", s.trim()));
            }
            J::Bool(true) => return Some(format!("réponse de l'API : {}", side().unwrap_or_else(|| "erreur signalée".into()))),
            v @ J::Obj(_) => {
                let t = ["message", "description", "detail", "error"].iter().map(|k| jtext(v.get(k))).find(|s| !s.is_empty());
                if let Some(t) = t {
                    return Some(format!("réponse de l'API : {t}"));
                }
            }
            J::Arr(v) if !v.is_empty() && alt == "errors" => {
                let t: Vec<String> = v.iter().map(|e| if let J::Str(s) = e { s.clone() } else { jfield(e, "message|detail|title") }).filter(|s| !s.is_empty()).collect();
                if !t.is_empty() {
                    return Some(format!("réponse de l'API : {}", t.join(" ; ")));
                }
            }
            _ => {}
        }
    }
    None
}

/// Premier tableau d'objets qui ressemble à une liste de résultats : (chemin, éléments).
pub fn auto_items(j: &J) -> Option<(String, &Vec<J>)> {
    fn walk<'a>(j: &'a J, path: String, depth: usize, best: &mut Option<(String, &'a Vec<J>, usize)>) {
        if depth > 6 {
            return;
        }
        match j {
            J::Arr(v) => {
                let score = v.iter().filter(|x| matches!(x, J::Obj(_)) && !jfield(x, T_TITLE).is_empty()).count();
                if score > 0 && best.as_ref().map(|b| score > b.2).unwrap_or(true) {
                    *best = Some((path.clone(), v, score));
                }
            }
            J::Obj(m) => {
                for (k, v) in m {
                    walk(v, if path.is_empty() { k.clone() } else { format!("{path}.{k}") }, depth + 1, best);
                }
            }
            _ => {}
        }
    }
    let mut best = None;
    walk(j, String::new(), 0, &mut best);
    best.map(|b| (b.0, b.1))
}

/// Lecture proposée d'après une réponse reçue (bouton « Deviner »).
pub fn guess(body: &str) -> RespMap {
    match detect(body, "").as_str() {
        "xml" => {
            let Some(doc) = Xml::parse(body) else { return RespMap { format: "xml".into(), ..Default::default() } };
            if doc.find("item").is_some() {
                return newznab_map();
            }
            // élément le plus répété qui a un titre
            let mut counts: Vec<(String, usize)> = Vec::new();
            fn count(x: &Xml, c: &mut Vec<(String, usize)>) {
                for ch in &x.children {
                    if ch.children.iter().any(|g| ["title", "name", "release"].contains(&g.name.to_lowercase().as_str())) {
                        match c.iter_mut().find(|(n, _)| *n == ch.name) {
                            Some(e) => e.1 += 1,
                            None => c.push((ch.name.clone(), 1)),
                        }
                    }
                    count(ch, c);
                }
            }
            count(&doc, &mut counts);
            counts.sort_by_key(|c| std::cmp::Reverse(c.1));
            let items = counts.first().map(|c| c.0.clone()).unwrap_or_else(|| "item".into());
            RespMap { format: "xml".into(), items, title: "title|name|release".into(), size: "size|@size".into(), link: "link|url|guid".into(), id: "id|guid|@id".into(), error: "error@description|error".into(), ..Default::default() }
        }
        "json" => {
            let Some(j) = J::parse(body.trim()) else { return RespMap { format: "json".into(), ..Default::default() } };
            let Some((path, items)) = auto_items(&j) else { return RespMap { format: "json".into(), ..Default::default() } };
            let first = items.iter().find(|x| matches!(x, J::Obj(_))).unwrap_or(&J::Null);
            let pick = |cands: &str| cands.split('|').find(|k| !jtext(first.get(k)).is_empty() || matches!(first.get(k), J::Num(_))).map(|k| k.to_string()).unwrap_or_default();
            RespMap { format: "json".into(), items: path, title: pick(T_TITLE), size: pick(T_SIZE), link: pick(T_LINK), id: pick(T_ID), date: pick(T_DATE), ..Default::default() }
        }
        f => RespMap { format: f.to_string(), ..Default::default() },
    }
}

// ------------------------------------------------------------------ YAML (assez pour OpenAPI)

struct YLine {
    indent: usize,
    text: String,
}

/// Lit un document YAML simple (OpenAPI, Swagger) : blocs, listes, scalaires, [..] et {..}.
pub fn yaml(src: &str) -> Option<J> {
    let mut lines: Vec<YLine> = Vec::new();
    for raw in src.lines() {
        let l = raw.trim_end();
        let content = l.trim_start();
        if content.is_empty() || content.starts_with('#') || content == "---" || content == "..." {
            // les lignes vides comptent dans les blocs « | » : gardées marquées
            if content.is_empty() && !lines.is_empty() {
                lines.push(YLine { indent: usize::MAX, text: String::new() });
            }
            continue;
        }
        lines.push(YLine { indent: l.len() - content.len(), text: strip_comment(content) });
    }
    let mut i = 0;
    skip_blank(&lines, &mut i);
    if i >= lines.len() {
        return None;
    }
    let ind = lines[i].indent;
    Some(yblock(&mut lines, &mut i, ind))
}

fn skip_blank(lines: &[YLine], i: &mut usize) {
    while *i < lines.len() && lines[*i].indent == usize::MAX {
        *i += 1;
    }
}

fn strip_comment(s: &str) -> String {
    let mut q: Option<char> = None;
    let mut prev = ' ';
    for (k, c) in s.char_indices() {
        match (q, c) {
            (Some(x), c) if c == x => q = None,
            (None, '"') | (None, '\'') if prev == ' ' || prev == ':' || k == 0 || prev == '[' || prev == ',' => q = Some(c),
            (None, '#') if prev == ' ' => return s[..k].trim_end().to_string(),
            _ => {}
        }
        prev = c;
    }
    s.to_string()
}

/// Sépare « clé: valeur » (deux-points hors guillemets, suivi d'un espace ou en fin de ligne).
fn ysplit(s: &str) -> Option<(String, String)> {
    let mut q: Option<char> = None;
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    for (n, (k, c)) in chars.iter().enumerate() {
        match (q, *c) {
            (Some(x), c) if c == x => q = None,
            (None, '"') | (None, '\'') if n == 0 => q = Some(*c),
            (None, ':') => {
                let next = chars.get(n + 1).map(|x| x.1);
                if next.is_none() || next == Some(' ') {
                    let key = yscalar_str(s[..*k].trim());
                    return Some((key, s[k + 1..].trim().to_string()));
                }
            }
            (None, '{') | (None, '[') if n == 0 => return None,
            _ => {}
        }
    }
    None
}

fn yscalar_str(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\''))) {
        let inner = &t[1..t.len() - 1];
        if t.starts_with('"') {
            inner.replace("\\n", "\n").replace("\\t", "\t").replace("\\\"", "\"").replace("\\\\", "\\")
        } else {
            inner.replace("''", "'")
        }
    } else {
        t.to_string()
    }
}

fn yscalar(s: &str) -> J {
    let t = s.trim();
    if t.starts_with('[') || t.starts_with('{') {
        let mut i = 0;
        let b: Vec<char> = t.chars().collect();
        return yflow(&b, &mut i);
    }
    if t.starts_with('"') || t.starts_with('\'') {
        return J::Str(yscalar_str(t));
    }
    match t {
        "" | "~" | "null" | "Null" | "NULL" => J::Null,
        "true" | "True" | "TRUE" => J::Bool(true),
        "false" | "False" | "FALSE" => J::Bool(false),
        _ => match t.parse::<f64>() {
            Ok(n) if !t.starts_with('0') || t == "0" || t.starts_with("0.") => J::Num(n),
            _ => J::Str(t.to_string()),
        },
    }
}

fn yflow(b: &[char], i: &mut usize) -> J {
    let skip = |i: &mut usize| {
        while *i < b.len() && b[*i].is_whitespace() {
            *i += 1;
        }
    };
    skip(i);
    if *i >= b.len() {
        return J::Null;
    }
    match b[*i] {
        '[' => {
            *i += 1;
            let mut v = Vec::new();
            loop {
                skip(i);
                if *i >= b.len() || b[*i] == ']' {
                    *i += 1;
                    break;
                }
                v.push(yflow(b, i));
                skip(i);
                if *i < b.len() && b[*i] == ',' {
                    *i += 1;
                }
            }
            J::Arr(v)
        }
        '{' => {
            *i += 1;
            let mut m = Vec::new();
            loop {
                skip(i);
                if *i >= b.len() || b[*i] == '}' {
                    *i += 1;
                    break;
                }
                let st = *i;
                while *i < b.len() && b[*i] != ':' && b[*i] != ',' && b[*i] != '}' {
                    *i += 1;
                }
                let k = yscalar_str(&b[st..*i].iter().collect::<String>());
                let v = if *i < b.len() && b[*i] == ':' {
                    *i += 1;
                    yflow(b, i)
                } else {
                    J::Null
                };
                m.push((k, v));
                skip(i);
                if *i < b.len() && b[*i] == ',' {
                    *i += 1;
                }
            }
            J::Obj(m)
        }
        q @ ('"' | '\'') => {
            let st = *i;
            *i += 1;
            while *i < b.len() && b[*i] != q {
                if b[*i] == '\\' && q == '"' {
                    *i += 1;
                }
                *i += 1;
            }
            *i += 1;
            J::Str(yscalar_str(&b[st..(*i).min(b.len())].iter().collect::<String>()))
        }
        _ => {
            let st = *i;
            while *i < b.len() && b[*i] != ',' && b[*i] != ']' && b[*i] != '}' {
                *i += 1;
            }
            yscalar(&b[st..*i].iter().collect::<String>())
        }
    }
}

fn yblock(lines: &mut Vec<YLine>, i: &mut usize, ind: usize) -> J {
    skip_blank(lines, i);
    if *i >= lines.len() {
        return J::Null;
    }
    if lines[*i].text == "-" || lines[*i].text.starts_with("- ") {
        let mut v = Vec::new();
        while *i < lines.len() {
            skip_blank(lines, i);
            if *i >= lines.len() || lines[*i].indent != ind || !(lines[*i].text == "-" || lines[*i].text.starts_with("- ")) {
                break;
            }
            let rest = lines[*i].text[1..].trim_start().to_string();
            if rest.is_empty() {
                *i += 1;
                skip_blank(lines, i);
                if *i < lines.len() && lines[*i].indent > ind {
                    let ci = lines[*i].indent;
                    v.push(yblock(lines, i, ci));
                } else {
                    v.push(J::Null);
                }
            } else if ysplit(&rest).is_some() {
                // « - clé: valeur » : objet dont la première clé est sur cette ligne
                let ci = ind + 2;
                lines[*i] = YLine { indent: ci, text: rest };
                v.push(yblock(lines, i, ci));
            } else {
                *i += 1;
                v.push(yscalar(&rest));
            }
        }
        return J::Arr(v);
    }
    let mut m: Vec<(String, J)> = Vec::new();
    while *i < lines.len() {
        skip_blank(lines, i);
        if *i >= lines.len() || lines[*i].indent != ind {
            break;
        }
        let Some((k, v)) = ysplit(&lines[*i].text) else {
            *i += 1;
            continue;
        };
        *i += 1;
        let val = if v.is_empty() {
            skip_blank(lines, i);
            if *i < lines.len() && lines[*i].indent > ind {
                let ci = lines[*i].indent;
                yblock(lines, i, ci)
            } else if *i < lines.len() && lines[*i].indent == ind && lines[*i].text.starts_with("- ") {
                yblock(lines, i, ind)
            } else {
                J::Null
            }
        } else if v.starts_with('|') || v.starts_with('>') {
            let fold = v.starts_with('>');
            let mut parts: Vec<String> = Vec::new();
            while *i < lines.len() && (lines[*i].indent == usize::MAX || lines[*i].indent > ind) {
                parts.push(lines[*i].text.clone());
                *i += 1;
            }
            J::Str(parts.join(if fold { " " } else { "\n" }).trim().to_string())
        } else {
            // valeur simple continuée sur les lignes plus indentées
            let mut text = v.clone();
            if !(v.starts_with('[') || v.starts_with('{') || v.starts_with('"') || v.starts_with('\'')) {
                while *i < lines.len() && lines[*i].indent != usize::MAX && lines[*i].indent > ind && ysplit(&lines[*i].text).is_none() && !lines[*i].text.starts_with("- ") {
                    text.push(' ');
                    text.push_str(&lines[*i].text);
                    *i += 1;
                }
            }
            yscalar(&text)
        };
        m.push((k, val));
    }
    J::Obj(m)
}

// ------------------------------------------------------------------ import de documentation

/// Ce qu'un document d'API a appris.
#[derive(Clone, Debug, Default)]
pub struct Import {
    /// Format reconnu (« OpenAPI 3 », « Postman », « page wiki »…).
    pub format: String,
    /// "newznab", "json", "html" ou vide.
    pub kind: String,
    pub base: Option<String>,
    /// (transmission, nom) : ("query", "apikey"), ("header", "X-Api-Key"), ("bearer", "")…
    pub auth: Option<(String, String)>,
    pub requests: Vec<Req>,
    pub caps: Option<Caps>,
    pub profile: Option<Profile>,
    pub notes: Vec<String>,
}

const KEY_NAMES: &[&str] = &["apikey", "api_key", "api-key", "key", "token", "access_token", "auth", "passkey", "x-api-key"];
const Q_NAMES: &[&str] = &["q", "query", "search", "term", "keyword", "keywords", "s", "name", "title", "release", "searchterm"];

fn slug(s: &str) -> String {
    let mut o = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            o.push(c);
        } else if !o.ends_with('_') && !o.is_empty() {
            o.push('_');
        }
    }
    o.trim_end_matches('_').chars().take(40).collect()
}

fn short(s: &str, n: usize) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let first = t.split(". ").next().unwrap_or(&t).to_string();
    if first.chars().count() > n {
        format!("{}…", first.chars().take(n).collect::<String>())
    } else {
        first
    }
}

/// Lit une documentation d'API (ou un profil exporté, ou une réponse `t=caps`).
pub fn import(text: &str) -> Import {
    let t = text.trim().trim_start_matches('\u{feff}');
    if t.contains(EXPORT_MARK) {
        return match Profile::from_json(t) {
            Ok(p) => Import { format: "profil NZB FF Tool Pro".into(), kind: p.kind.clone(), profile: Some(p), ..Default::default() },
            Err(e) => Import { notes: vec![e], ..Default::default() },
        };
    }
    if t.starts_with('<') && t.to_lowercase().contains("<caps") {
        return match parse_caps(t) {
            Ok(c) => Import { format: "capacités Newznab (t=caps)".into(), kind: "newznab".into(), requests: newznab_requests(&c), caps: Some(c), ..Default::default() },
            Err(e) => Import { notes: vec![e], ..Default::default() },
        };
    }
    if t.starts_with('{') {
        if let Some(j) = J::parse(t) {
            if !matches!(j.get("openapi"), J::Null) || !matches!(j.get("swagger"), J::Null) {
                return openapi(&j);
            }
            if !matches!(j.get("item"), J::Null) && !matches!(j.get("info"), J::Null) {
                return postman(&j);
            }
        }
    }
    let low = t.to_lowercase();
    if low.starts_with("openapi:") || low.starts_with("swagger:") || low.contains("\nopenapi:") || low.contains("\nswagger:") {
        if let Some(j) = yaml(t) {
            return openapi(&j);
        }
    }
    wiki(t)
}

fn resolve<'a>(root: &'a J, j: &'a J) -> &'a J {
    let r = j.get("$ref").s();
    match r.strip_prefix("#/") {
        Some(path) => {
            let mut cur = root;
            for part in path.split('/') {
                cur = cur.get(part);
            }
            cur
        }
        None => j,
    }
}

fn openapi(j: &J) -> Import {
    let v3 = !matches!(j.get("openapi"), J::Null);
    let mut imp = Import { format: if v3 { format!("OpenAPI {}", jtext(j.get("openapi"))) } else { "Swagger 2".into() }, kind: "json".into(), ..Default::default() };
    // adresse
    let server = if v3 { j.get("servers").arr().first().map(|s| s.get("url").s()).unwrap_or_default() } else {
        let host = j.get("host").s();
        let scheme = j.get("schemes").arr().first().map(|s| s.s()).unwrap_or_else(|| "https".into());
        if host.is_empty() { j.get("basePath").s() } else { format!("{scheme}://{host}{}", j.get("basePath").s()) }
    };
    let (root, prefix) = if server.starts_with("http") {
        let r = site_root(&server, "");
        let pre = server[r.len()..].trim_end_matches('/').to_string();
        imp.base = Some(r);
        ("{base}".to_string(), pre)
    } else {
        ("{base}".to_string(), server.trim_end_matches('/').to_string())
    };
    // authentification
    let schemes = if v3 { j.get("components").get("securitySchemes") } else { j.get("securityDefinitions") };
    let mut key_param = String::new();
    if let J::Obj(m) = schemes {
        for (_, sch) in m {
            let ty = sch.get("type").s().to_lowercase();
            let auth = match ty.as_str() {
                "apikey" => {
                    let name = sch.get("name").s();
                    key_param = name.clone();
                    Some((match sch.get("in").s().as_str() { "header" => "header", "cookie" => "cookie", _ => "query" }.to_string(), name))
                }
                "http" if sch.get("scheme").s().eq_ignore_ascii_case("bearer") => Some(("bearer".into(), String::new())),
                "http" | "basic" => Some(("basic".into(), String::new())),
                "oauth2" => Some(("bearer".into(), String::new())),
                _ => None,
            };
            if auth.is_some() {
                imp.auth = auth;
                break;
            }
        }
    }
    let J::Obj(paths) = j.get("paths") else {
        imp.notes.push("aucun chemin (« paths ») dans ce document".into());
        return imp;
    };
    for (path, item) in paths {
        let J::Obj(ops) = item else { continue };
        let common: Vec<&J> = item.get("parameters").arr().iter().collect();
        for (m, op) in ops {
            let method = m.to_uppercase();
            if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str()) {
                continue;
            }
            let label = [op.get("summary").s(), op.get("operationId").s()].into_iter().find(|s| !s.is_empty()).unwrap_or_else(|| format!("{method} {path}"));
            let id = { let o = op.get("operationId").s(); if o.is_empty() { slug(&format!("{method}_{path}")) } else { slug(&o) } };
            let mut r = Req { id, label: short(&label, 70), help: short(&op.get("description").s(), 300), method: method.clone(), url: format!("{root}{prefix}{path}"), origin: "import".into(), ..Default::default() };
            let low = format!("{path} {label}").to_lowercase();
            r.read = method == "GET" || low.contains("search") || low.contains("recherch") || low.contains("find") || low.contains("query");
            for p in common.iter().copied().chain(op.get("parameters").arr().iter()) {
                let p = resolve(j, p);
                let name = p.get("name").s();
                let place = match p.get("in").s().as_str() {
                    "path" => "path",
                    "header" => "header",
                    "formData" => "form",
                    "query" => "query",
                    _ => continue,
                };
                if name.is_empty() || (!key_param.is_empty() && name.eq_ignore_ascii_case(&key_param)) || KEY_NAMES.contains(&name.to_lowercase().as_str()) {
                    if imp.auth.is_none() && KEY_NAMES.contains(&name.to_lowercase().as_str()) {
                        imp.auth = Some((if place == "header" { "header" } else { "query" }.into(), name.clone()));
                    }
                    continue;
                }
                let schema = if v3 { resolve(j, p.get("schema")) } else { p };
                let choices: Vec<(String, String)> = schema.get("enum").arr().iter().map(|e| (jtext(e), jtext(e))).filter(|e| !e.0.is_empty()).collect();
                let default = jtext(schema.get("default"));
                let value = if r.read && Q_NAMES.contains(&name.to_lowercase().as_str()) { "{release}".to_string() } else { default };
                if place == "path" && !r.url.contains(&format!("{{{name}}}")) {
                    r.url = r.url.replace(&format!(":{name}"), &format!("{{{name}}}"));
                }
                r.params.push(Param { name: name.clone(), value, label: short(&p.get("description").s(), 60), place: place.into(), enc: "%20".into(), required: p.get("required").b() || place == "path", choices });
            }
            // corps (OpenAPI 3)
            if v3 {
                if let J::Obj(content) = op.get("requestBody").get("content") {
                    for (ct, c) in content {
                        let place = if ct.contains("json") { "json" } else if ct.contains("form") { "form" } else { continue };
                        let schema = resolve(j, c.get("schema"));
                        let required: Vec<String> = schema.get("required").arr().iter().map(|x| x.s()).collect();
                        if let J::Obj(props) = schema.get("properties") {
                            for (name, pr) in props {
                                let pr = resolve(j, pr);
                                if pr.get("format").s() == "binary" {
                                    continue;
                                }
                                let value = if r.read && Q_NAMES.contains(&name.to_lowercase().as_str()) { "{release}".to_string() } else { jtext(pr.get("default")) };
                                r.params.push(Param { name: name.clone(), value, label: short(&pr.get("description").s(), 60), place: place.into(), enc: "%20".into(), required: required.contains(name), choices: pr.get("enum").arr().iter().map(|e| (jtext(e), jtext(e))).collect() });
                            }
                        }
                        break;
                    }
                }
            }
            let ok = op.get("responses").get("200");
            let ct = if let J::Obj(c) = ok.get("content") { c.first().map(|x| x.0.clone()).unwrap_or_default() } else { op.get("produces").arr().first().map(|x| x.s()).unwrap_or_default() };
            r.resp.format = if ct.contains("json") { "json" } else if ct.contains("xml") || ct.contains("rss") { "xml" } else if ct.contains("html") { "html" } else { "auto" }.into();
            imp.requests.push(r);
        }
    }
    imp
}

/// Variable Postman « {{nom}} » → placeholder du profil.
fn postman_vars(s: &str, base_var: &str, imp: &mut Import, key_seen: &mut bool) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find("{{") {
        out.push_str(&rest[..a]);
        let after = &rest[a + 2..];
        let Some(b) = after.find("}}") else {
            out.push_str(&rest[a..]);
            return out;
        };
        let v = after[..b].trim();
        let lv = v.to_lowercase();
        if !base_var.is_empty() && v == base_var || lv == "baseurl" || lv == "base_url" || lv == "url" || lv == "host" {
            out.push_str("{base}");
        } else if KEY_NAMES.iter().any(|k| lv.replace(['-', '_'], "") == k.replace(['-', '_'], "")) || lv.contains("apikey") || lv.contains("token") {
            out.push_str("{key}");
            *key_seen = true;
        } else {
            out.push_str(&format!("{{{v}}}"));
        }
        rest = &after[b + 2..];
    }
    out.push_str(rest);
    let _ = imp;
    out
}

fn postman(j: &J) -> Import {
    let mut imp = Import { format: "collection Postman".into(), kind: "json".into(), ..Default::default() };
    let mut base_var = String::new();
    for v in j.get("variable").arr() {
        let val = v.get("value").s();
        if val.starts_with("http") {
            base_var = v.get("key").s();
            imp.base = Some(val.trim_end_matches('/').to_string());
            break;
        }
    }
    let a = j.get("auth");
    match a.get("type").s().as_str() {
        "apikey" => {
            let get = |k: &str| a.get("apikey").arr().iter().find(|x| x.get("key").s() == k).map(|x| x.get("value").s()).unwrap_or_default();
            imp.auth = Some((if get("in") == "header" { "header" } else { "query" }.into(), get("key")));
        }
        "bearer" => imp.auth = Some(("bearer".into(), String::new())),
        "basic" => imp.auth = Some(("basic".into(), String::new())),
        _ => {}
    }
    fn walk(items: &[J], base_var: &str, imp: &mut Import, depth: usize) {
        for it in items {
            if !it.get("item").arr().is_empty() && depth < 6 {
                walk(it.get("item").arr(), base_var, imp, depth + 1);
                continue;
            }
            let rq = it.get("request");
            if matches!(rq, J::Null) {
                continue;
            }
            let mut key_seen = false;
            let method = { let m = rq.get("method").s().to_uppercase(); if m.is_empty() { "GET".to_string() } else { m } };
            let url_j = rq.get("url");
            let raw = if let J::Str(s) = url_j { s.clone() } else { url_j.get("raw").s() };
            let (path_part, query_part) = raw.split_once('?').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((raw.clone(), String::new()));
            let name = it.get("name").s();
            let mut r = Req { id: slug(&name), label: short(&name, 70), help: short(&rq.get("description").s(), 300), method: method.clone(), origin: "import".into(), ..Default::default() };
            r.url = postman_vars(&path_part, base_var, imp, &mut key_seen);
            // « :id » → {id}
            r.url = r.url.split('/').map(|s| if let Some(p) = s.strip_prefix(':') { format!("{{{p}}}") } else { s.to_string() }).collect::<Vec<_>>().join("/");
            let low = format!("{} {}", r.url, name).to_lowercase();
            r.read = method == "GET" || low.contains("search") || low.contains("recherch");
            let qlist: Vec<(String, String, String)> = if !url_j.get("query").arr().is_empty() {
                url_j.get("query").arr().iter().filter(|q| !q.get("disabled").b()).map(|q| (q.get("key").s(), q.get("value").s(), q.get("description").s())).collect()
            } else {
                query_part.split('&').filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string(), String::new()))).collect()
            };
            for (k, v, d) in qlist {
                let mut ks = false;
                let val = postman_vars(&v, base_var, imp, &mut ks);
                if ks || KEY_NAMES.contains(&k.to_lowercase().as_str()) {
                    if imp.auth.is_none() {
                        imp.auth = Some(("query".into(), k.clone()));
                    }
                    continue;
                }
                let val = if r.read && Q_NAMES.contains(&k.to_lowercase().as_str()) { "{release}".to_string() } else { val };
                r.params.push(Param { name: k, value: val, label: short(&d, 60), place: "query".into(), enc: "%20".into(), ..Default::default() });
            }
            for h in rq.get("header").arr() {
                let (k, v) = (h.get("key").s(), h.get("value").s());
                let mut ks = false;
                let val = postman_vars(&v, base_var, imp, &mut ks);
                if ks {
                    if imp.auth.is_none() {
                        imp.auth = Some(if k.eq_ignore_ascii_case("authorization") { ("bearer".into(), String::new()) } else { ("header".into(), k.clone()) });
                    }
                    continue;
                }
                if !k.eq_ignore_ascii_case("content-type") && !k.is_empty() {
                    r.headers.push((k, val));
                }
            }
            let body = rq.get("body");
            match body.get("mode").s().as_str() {
                "raw" => {
                    let mut ks = false;
                    r.body = postman_vars(&body.get("raw").s(), base_var, imp, &mut ks);
                }
                m @ ("urlencoded" | "formdata") => {
                    for f in body.get(m).arr() {
                        if f.get("type").s() == "file" {
                            continue;
                        }
                        let mut ks = false;
                        let k = f.get("key").s();
                        let v = postman_vars(&f.get("value").s(), base_var, imp, &mut ks);
                        let v = if r.read && Q_NAMES.contains(&k.to_lowercase().as_str()) { "{release}".to_string() } else { v };
                        r.params.push(Param { name: k, value: v, place: "form".into(), enc: "+".into(), ..Default::default() });
                    }
                }
                _ => {}
            }
            if key_seen && imp.auth.is_none() {
                imp.auth = Some(("query".into(), "apikey".into()));
            }
            // paramètres de chemin laissés en {nom}
            for l in leftovers(&r.url) {
                if !["base", "key", "release"].contains(&l.as_str()) && !r.params.iter().any(|p| p.name == l) {
                    r.params.push(Param { name: l.clone(), place: "path".into(), enc: "%20".into(), required: true, ..Default::default() });
                }
            }
            imp.requests.push(r);
        }
    }
    walk(j.get("item").arr(), &base_var, &mut imp, 0);
    imp
}

/// Page de documentation (wiki, Markdown, HTML, texte) : adresses d'exemple, lignes
/// « GET /chemin », fonctions Newznab, et descriptions des paramètres dans les tableaux.
fn wiki(text: &str) -> Import {
    let mut imp = Import { format: "page de documentation".into(), ..Default::default() };
    // HTML → texte en lignes
    let mut t = text.to_string();
    for tag in ["<br>", "<br/>", "<br />", "</p>", "</tr>", "</li>", "</h1>", "</h2>", "</h3>", "</h4>", "</pre>", "</div>"] {
        t = t.replace(tag, &format!("{tag}\n")).replace(&tag.to_uppercase(), &format!("{tag}\n"));
    }
    t = t.replace("</td>", " | ").replace("</th>", " | ");
    let plain: String = t.lines().map(|l| if l.contains('<') { clean_text(l) } else { unescape(l) }).collect::<Vec<_>>().join("\n");
    let low = plain.to_lowercase();
    let newznab = low.contains("t=search") || low.contains("t=caps") || low.contains("t=tvsearch") || low.contains("newznab");
    // descriptions de paramètres : « | nom | … | description | » ou « nom - description » / « `nom` : description »
    let mut desc: HashMap<String, (String, bool)> = HashMap::new();
    for line in plain.lines() {
        let l = line.trim();
        let cells: Vec<String> = l.trim_matches('|').split('|').map(|c| c.trim().trim_matches('`').trim().to_string()).collect();
        let (name, rest) = if cells.len() >= 2 {
            (cells[0].clone(), cells[1..].join(" — "))
        } else if let Some((a, b)) = l.trim_start_matches(['-', '*', ' ']).split_once(" - ").or_else(|| l.trim_start_matches(['-', '*', ' ']).split_once(" : ")) {
            (a.trim().trim_matches('`').to_string(), b.trim().to_string())
        } else {
            continue;
        };
        if name.is_empty() || name.len() > 30 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') || rest.trim_matches(['-', ' ', '—']).is_empty() {
            continue;
        }
        let req = rest.to_lowercase().contains("required") || rest.to_lowercase().contains("obligatoire");
        desc.entry(name.to_lowercase()).or_insert((short(rest.trim_matches(['—', ' ']), 70), req));
    }
    let mut origins: Vec<String> = Vec::new();
    let add = |imp: &mut Import, r: Req| {
        if !imp.requests.iter().any(|x| x.method == r.method && x.url == r.url && x.params.iter().map(|p| &p.name).collect::<Vec<_>>() == r.params.iter().map(|p| &p.name).collect::<Vec<_>>()) {
            imp.requests.push(r);
        }
    };
    // adresses d'exemple
    for (n, line) in plain.lines().enumerate() {
        let mut rest = line;
        while let Some(a) = rest.find("http") {
            let cand = &rest[a..];
            if !(cand.starts_with("http://") || cand.starts_with("https://")) {
                rest = &rest[a + 4..];
                continue;
            }
            let end = cand.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '<' || c == '`' || c == ')' || c == ']').unwrap_or(cand.len());
            let url = cand[..end].trim_end_matches(['.', ',', ';']);
            rest = &cand[end..];
            let Some((path, q)) = url.split_once('?') else { continue };
            let root = site_root(path, "");
            origins.push(root.clone());
            let mut r = Req { method: "GET".into(), url: format!("{{base}}{}", &path[root.len()..]), origin: "import".into(), read: true, ..Default::default() };
            let mut tval = String::new();
            for kv in q.split('&') {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                if k.is_empty() {
                    continue;
                }
                let lk = k.to_lowercase();
                if KEY_NAMES.contains(&lk.as_str()) {
                    if imp.auth.is_none() {
                        imp.auth = Some(("query".into(), k.to_string()));
                    }
                    continue;
                }
                if lk == "t" {
                    tval = v.to_string();
                }
                let v = url_decode(v);
                let val = if Q_NAMES.contains(&lk.as_str()) { "{release}".to_string() } else if v.starts_with('<') || v.starts_with('{') || v.to_lowercase().contains("your") || v.contains("xxx") { String::new() } else { v };
                let (label, required) = desc.get(&lk).cloned().unwrap_or_default();
                r.params.push(Param { name: k.to_string(), value: val, label, place: "query".into(), enc: if v_has_plus(kv) { "+".into() } else { "%20".into() }, required, ..Default::default() });
            }
            // libellé : texte de la ligne autour, ou la fonction Newznab
            let around = clean_text(&line.replace(url, " ")).trim_matches([':', '-', '—', ' ', '|']).to_string();
            r.label = if !tval.is_empty() { format!("t={tval}") } else if !around.is_empty() && around.len() < 80 { around } else { format!("Exemple {}", n + 1) };
            r.id = slug(&if tval.is_empty() { r.url.clone() } else { tval.clone() });
            add(&mut imp, r);
        }
    }
    // lignes « GET /chemin »
    for line in plain.lines() {
        let l = line.trim().trim_start_matches(['#', '*', '-', '`', ' ']);
        for m in ["GET", "POST", "PUT", "DELETE", "PATCH"] {
            let Some(rest) = l.strip_prefix(m) else { continue };
            let rest = rest.trim_start();
            if !rest.starts_with('/') {
                continue;
            }
            let path: String = rest.chars().take_while(|c| !c.is_whitespace() && *c != '`' && *c != '"').collect();
            if path.len() < 2 {
                continue;
            }
            let (p, q) = path.split_once('?').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((path.clone(), String::new()));
            let p = p.split('/').map(|s| if let Some(x) = s.strip_prefix(':') { format!("{{{x}}}") } else { s.to_string() }).collect::<Vec<_>>().join("/");
            let label = rest[rest.find(char::is_whitespace).unwrap_or(rest.len())..].trim().trim_start_matches(['-', '—', ':', '`', ' ', '|']).trim().to_string();
            let lowl = format!("{p} {label}").to_lowercase();
            let mut r = Req { id: slug(&format!("{m}{p}")), label: if label.is_empty() { format!("{m} {p}") } else { short(&label, 70) }, method: m.into(), url: format!("{{base}}{p}"), origin: "import".into(), read: m == "GET" || lowl.contains("search") || lowl.contains("recherch"), ..Default::default() };
            for l in leftovers(&p) {
                let (label, _) = desc.get(&l.to_lowercase()).cloned().unwrap_or_default();
                r.params.push(Param { name: l, label, place: "path".into(), enc: "%20".into(), required: true, ..Default::default() });
            }
            for kv in q.split('&').filter(|x| !x.is_empty()) {
                let k = kv.split('=').next().unwrap_or("");
                if KEY_NAMES.contains(&k.to_lowercase().as_str()) {
                    continue;
                }
                let (label, required) = desc.get(&k.to_lowercase()).cloned().unwrap_or_default();
                let val = if r.read && Q_NAMES.contains(&k.to_lowercase().as_str()) { "{release}".into() } else { String::new() };
                r.params.push(Param { name: k.into(), value: val, label, place: if m == "GET" { "query" } else { "form" }.into(), enc: "%20".into(), required, ..Default::default() });
            }
            add(&mut imp, r);
        }
    }
    // adresse de base : la plus citée
    origins.sort();
    let mut best: Option<(String, usize)> = None;
    for o in &origins {
        let n = origins.iter().filter(|x| *x == o).count();
        if best.as_ref().map(|b| n > b.1).unwrap_or(true) {
            best = Some((o.clone(), n));
        }
    }
    imp.base = best.map(|b| b.0);
    if newznab {
        imp.kind = "newznab".into();
        imp.format = "page de documentation (API Newznab)".into();
        // les requêtes Newznab sont tirées des capacités : celles de l'exemple ne sont gardées
        // que si elles appellent une fonction non standard
        let std = ["caps", "search", "tvsearch", "movie", "music", "book", "details", "get", "nzbadd"];
        imp.requests.retain(|r| !r.params.iter().any(|p| p.name == "t" && std.contains(&p.value.as_str())));
        for r in imp.requests.iter_mut() {
            if r.url.ends_with("/api") || r.url.contains("/api?") {
                r.url = r.url.replace("{base}/api", "{api}").replace("{base}/api/v1/api", "{api}");
            }
        }
    } else if !imp.requests.is_empty() {
        imp.kind = "json".into();
    }
    if imp.requests.is_empty() && !newznab {
        imp.notes.push("rien de reconnu : ni adresse d'exemple avec paramètres, ni ligne « GET /chemin », ni fonction Newznab".into());
    }
    imp
}

/// Décodage d'une valeur d'adresse (« %20 », « + »).
pub fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => {
                let h = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
                out.push(h(b[i + 1]) * 16 + h(b[i + 2]));
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn v_has_plus(kv: &str) -> bool {
    kv.split_once('=').map(|(_, v)| v.contains('+')).unwrap_or(false)
}

/// Ajoute ce qu'un import a appris au profil. Renvoie ce qui a été fait, en clair.
pub fn apply(p: &mut Profile, imp: &Import, from: &str) -> Vec<String> {
    let mut done = Vec::new();
    if let Some(np) = &imp.profile {
        let key = p.key.clone();
        let user = p.user.clone();
        *p = np.clone();
        p.key = key;
        p.user = user;
        done.push(format!("profil importé : {} requête(s)", p.requests.len()));
        return done;
    }
    if p.is_auto() {
        p.kind = if imp.kind.is_empty() { "newznab".into() } else { imp.kind.clone() };
        if p.kind == "newznab" && p.requests.is_empty() {
            p.requests = newznab_requests(&p.caps);
        }
        done.push(format!("type : {}", p.kind_label()));
    } else if p.kind != "newznab" && imp.kind == "newznab" && !imp.requests.iter().any(|r| r.origin == "import") {
        done.push("documentation Newznab : passer le type en « API Newznab » pour ses requêtes standard".into());
    }
    if let Some(b) = &imp.base {
        if p.base.trim().is_empty() {
            p.base = b.clone();
            done.push(format!("adresse : {b}"));
        }
    }
    if let Some((a, n)) = &imp.auth {
        p.auth = a.clone();
        if !n.is_empty() {
            p.auth_name = n.clone();
        }
        done.push(format!("clé transmise par {}", auth_label(a, n)));
    }
    if let Some(c) = &imp.caps {
        p.caps = c.clone();
        let custom: Vec<Req> = p.requests.iter().filter(|r| !["newznab", "capacités"].contains(&r.origin.as_str())).cloned().collect();
        p.requests = newznab_requests(c);
        p.requests.extend(custom);
        done.push(format!("capacités : {} type(s) de recherche, {} catégorie(s)", c.modes.len(), c.cats.len()));
    }
    let mut added = 0;
    for r in &imp.requests {
        if imp.caps.is_some() {
            break;
        }
        if p.requests.iter().any(|x| x.method() == r.method() && x.url == r.url && x.params.iter().map(|q| &q.name).collect::<Vec<_>>() == r.params.iter().map(|q| &q.name).collect::<Vec<_>>()) {
            continue;
        }
        let mut r = r.clone();
        let mut id = if r.id.is_empty() { "requete".to_string() } else { r.id.clone() };
        let stem = id.clone();
        let mut n = 2;
        while p.requests.iter().any(|x| x.id == id) {
            id = format!("{stem}_{n}");
            n += 1;
        }
        r.id = id;
        p.requests.push(r);
        added += 1;
    }
    if added > 0 {
        done.push(format!("{added} requête(s) ajoutée(s)"));
    }
    if !from.is_empty() {
        p.source = if p.source.is_empty() { format!("Importé de {from}") } else { format!("{} + {from}", p.source) };
    }
    done
}

pub fn auth_label(a: &str, n: &str) -> String {
    match a {
        "header" => format!("l'en-tête « {n} »"),
        "bearer" => "l'en-tête Authorization: Bearer".into(),
        "basic" => "identifiant + mot de passe (Basic)".into(),
        "cookie" => format!("le cookie « {n} »"),
        "none" => "rien (pas de clé)".into(),
        _ => format!("le paramètre d'adresse « {} »", if n.is_empty() { "apikey" } else { n }),
    }
}

/// Lit les capacités d'un indexer Newznab.
pub fn read_caps(idx: &IndexerCfg, p: &Profile) -> Result<(Caps, Reply), String> {
    let r = newznab_requests(&Caps::default()).into_iter().find(|r| r.id == "caps").ok_or("requête caps")?;
    let rep = send(idx, p, &r, &[])?;
    if let Some(e) = &rep.error {
        return Err(e.clone());
    }
    parse_caps(&rep.body).map(|c| (c, rep))
}

/// Télécharge une page de documentation (sans clé).
pub fn fetch_doc(url: &str) -> Result<String, String> {
    let u = url.trim();
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return Err("adresse http(s) attendue".into());
    }
    let args: Vec<String> = ["-sS", "-L", "--max-time", "40", "-w", "\n@@HTTP@@%{http_code}", "-H", "Accept: application/json, application/yaml, text/yaml, text/html;q=0.9, */*;q=0.5", u].iter().map(|s| s.to_string()).collect();
    let (body, http) = run_curl(&args).map_err(|e| e.message)?;
    match http {
        Some(c) if !(200..300).contains(&c) => Err(format!("HTTP {c} {} (page protégée par connexion ? enregistre-la puis importe le fichier)", crate::upload::http_reason(c))),
        _ => Ok(body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(api: &str) -> IndexerCfg {
        let mut i = IndexerCfg::new("X");
        i.api_url = api.into();
        i.api_key = "K3Y-LONG".into();
        i
    }

    fn rq(url: &str, params: Vec<Param>) -> Req {
        Req { id: "t".into(), label: "t".into(), method: "GET".into(), url: url.into(), params, read: true, ..Default::default() }
    }

    #[test]
    fn prereglages() {
        let ps = presets();
        let mut ids: Vec<&str> = ps.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ps.len());
        assert!(ps.iter().filter(|p| !p.id.starts_with("generic")).all(|p| p.base.starts_with("http")));
        let a = from_preset(&preset("althub").unwrap());
        assert_eq!((a.kind.as_str(), a.base.as_str(), a.api_path.as_str()), ("newznab", "https://api.althub.co.za", "/api"));
        let s = a.req("search").unwrap();
        assert!(s.params.iter().any(|p| p.name == "q" && p.value == "{release}"));
        let tr = from_preset(&preset("tabularasa").unwrap());
        let c = conn(&IndexerCfg::new("T"), &tr);
        assert_eq!(c.api, "https://www.tabula-rasa.pw/api/v1/api");
        // profil automatique : adresse du dépôt
        let i = idx("https://ix.example/api");
        let c = conn(&i, &profile(&i));
        assert_eq!((c.base.as_str(), c.api.as_str(), c.key.as_str()), ("https://ix.example", "https://ix.example/api", "K3Y-LONG"));
    }

    #[test]
    fn capacites() {
        let xml = r#"<?xml version="1.0"?><caps><server title="Ix"/><limits max="200" default="75"/><searching><search available="yes" supportedParams="q"/><tv-search available="yes" supportedParams="q,season,ep,tvdbid"/><movie-search available="no" supportedParams="q,imdbid"/><audio-search available="yes"/></searching><categories><category id="2000" name="Movies"><subcat id="2040" name="HD"/></category><category id="5000" name="TV"/></categories></caps>"#;
        let c = parse_caps(xml).unwrap();
        assert_eq!((c.title.as_str(), c.limit_max, c.limit_default), ("Ix", 200, 75));
        assert_eq!(c.modes.len(), 3);
        assert_eq!(c.cats[1], ("2040".to_string(), "Movies > HD".to_string()));
        let r = newznab_requests(&c);
        let tv = r.iter().find(|x| x.id == "tvsearch").unwrap();
        assert!(tv.params.iter().any(|p| p.name == "tvdbid" && p.label == "TVDB"));
        assert!(r.iter().all(|x| x.id != "movie"));
        assert_eq!(r.iter().find(|x| x.id == "search").unwrap().params.iter().find(|p| p.name == "limit").unwrap().value, "100");
        assert!(parse_caps(r#"<error code="100" description="Incorrect user credentials"/>"#).unwrap_err().contains("clé API incorrecte"));
    }

    #[test]
    fn construction_des_requetes() {
        let i = idx("https://ix.example/api");
        let p = profile(&i);
        let s = p.req("search").unwrap();
        let b = build(&i, &p, s, &[("release".into(), "Le Film".into())]).unwrap();
        assert_eq!(b.url, "https://ix.example/api?t=search&q=Le%20Film&limit=100&extended=1&apikey=K3Y-LONG");
        let pv = preview(&b, "K3Y-LONG").join("\n");
        assert!(!pv.contains("K3Y-LONG") && pv.contains("apikey=••••"), "{pv}");
        // nom dans le chemin ; encodage « + » ; {cat} vide retiré
        let mut c = Profile { kind: "json".into(), base: "https://ix.example".into(), auth: "header".into(), auth_name: "X-Api-Key".into(), ..Default::default() };
        let r = rq("{base}/search/{release}", vec![]);
        assert_eq!(build(&i, &c, &r, &[("release".into(), "mon ami".into())]).unwrap().url, "https://ix.example/search/mon%20ami");
        let r = rq("{base}/search?cat={cat}", vec![Param { enc: "+".into(), ..Param::new("q", "{release}", "") }, Param::new("cat", "", "")]);
        let b = build(&i, &c, &r, &[("release".into(), "mon ami".into())]).unwrap();
        assert_eq!(b.url, "https://ix.example/search?q=mon+ami");
        assert!(b.headers.contains(&("X-Api-Key".to_string(), "K3Y-LONG".to_string())));
        // paramètre de chemin, obligatoire
        let r = rq("{base}/release/{id}", vec![Param { required: true, place: "path".into(), ..Param::new("id", "", "Identifiant") }]);
        assert!(build(&i, &c, &r, &[]).unwrap_err().contains("Identifiant"));
        assert_eq!(build(&i, &c, &r, &[("id".into(), "a/b".into())]).unwrap().url, "https://ix.example/release/a%2Fb");
        // POST : formulaire, JSON, corps modèle
        let mut r = rq("{base}/api/search", vec![Param { place: "form".into(), ..Param::new("q", "{release}", "") }, Param { place: "form".into(), ..Param::new("limit", "10", "") }]);
        r.method = "POST".into();
        let b = build(&i, &c, &r, &[("release".into(), "Le Film".into())]).unwrap();
        assert_eq!(b.body, Some(("application/x-www-form-urlencoded".to_string(), "q=Le+Film&limit=10".to_string())));
        for x in r.params.iter_mut() {
            x.place = "json".into();
        }
        let b = build(&i, &c, &r, &[("release".into(), "Le \"Film\"".into())]).unwrap();
        assert_eq!(b.body.unwrap().1, r#"{"q":"Le \"Film\"","limit":10}"#);
        r.params.clear();
        r.body = r#"{"query":"{release}","key":"{key}"}"#.into();
        let b = build(&i, &c, &r, &[("release".into(), "A\"B".into())]).unwrap();
        assert_eq!(b.body.clone().unwrap().1, r#"{"query":"A\"B","key":"K3Y-LONG"}"#);
        assert!(!preview(&b, "K3Y-LONG").join(" ").contains("K3Y-LONG"));
        // Bearer, Basic, sans clé
        c.auth = "bearer".into();
        assert!(build(&i, &c, &rq("{base}/x", vec![]), &[]).unwrap().headers.contains(&("Authorization".to_string(), "Bearer K3Y-LONG".to_string())));
        c.auth = "basic".into();
        c.user = "moi".into();
        assert_eq!(build(&i, &c, &rq("{base}/x", vec![]), &[]).unwrap().basic.as_deref(), Some("moi:K3Y-LONG"));
        c.auth = "none".into();
        assert_eq!(build(&i, &c, &rq("{base}/x", vec![]), &[]).unwrap().url, "https://ix.example/x");
        // adresse manquante
        assert!(build(&IndexerCfg::new("V"), &Profile { kind: "json".into(), ..Default::default() }, &rq("{base}/x", vec![]), &[]).is_err());
    }

    #[test]
    fn lecture_des_reponses() {
        let rss = r#"<?xml version="1.0"?><rss xmlns:newznab="x"><channel><item><title>Le.Film.2020.FRENCH.1080p.WEB-GRP</title><guid isPermaLink="true">https://ix/details/abc</guid><newznab:attr name="size" value="1234"/><newznab:attr name="guid" value="abc"/></item><item><title><![CDATA[Le Film &amp; Co 2020]]></title><enclosure url="u" length="99"/></item></channel></rss>"#;
        let rows = extract(rss, &newznab_map(), "xml").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].size, rows[0].id.as_str(), rows[0].link.as_str()), (1234, "abc", "https://ix/details/abc"));
        assert_eq!((rows[1].title.as_str(), rows[1].size), ("Le Film & Co 2020", 99));
        assert!(extract(r#"<?xml version="1.0"?><error code="500" description="Request limit reached"/>"#, &newznab_map(), "xml").unwrap_err().contains("limite de requêtes"));
        // NZBIndex : sujets Usenet nettoyés, « error »: false n'est pas une erreur
        let nzbi = from_preset(&preset("nzbindex").unwrap());
        let m = &nzbi.req("search").unwrap().resp;
        let json = r#"{"error":false,"data":{"content":[{"id":"7","name":"[1/9] - \"Le.Film.2020.FRENCH.1080p.WEB-GRP.mkv\" yEnc (1/50)","size":5000,"posted":1700000000}]}}"#;
        let rows = extract(json, m, "json").unwrap();
        assert_eq!(rows[0].title, "Le.Film.2020.FRENCH.1080p.WEB-GRP");
        assert_eq!((rows[0].size, rows[0].id.as_str()), (5000, "7"));
        assert!(extract(r#"{"error":true,"errorMessage":"Invalid key"}"#, m, "json").unwrap_err().contains("Invalid key"));
        assert!(extract(r#"{"errors":["Unauthorized"]}"#, &RespMap::default(), "json").unwrap_err().contains("Unauthorized"));
        // BinSearch : motif HTML
        let bins = from_preset(&preset("binsearch").unwrap());
        let html = r#"<table><tr><td><span class="s">"Le.Film.2020.FRENCH.1080p.WEB-GRP.part01.rar" yEnc</span><span class="d">size: 4.2 GB</span></td></tr><tr><td><span class="s">Autre &quot;Chose.2021.mkv&quot;</span></td></tr></table>"#;
        let rows = extract(html, &bins.requests[0].resp, "html").unwrap();
        assert_eq!(rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), vec!["Le.Film.2020.FRENCH.1080p.WEB-GRP.part01", "Chose.2021"]);
        let m = RespMap { format: "html".into(), items: r#"<span class="s">"{title}"*<span class="d">size: {size}</span>"#.into(), ..Default::default() };
        let rows = extract(html, &m, "html").unwrap();
        assert_eq!((rows[0].title.as_str(), rows[0].size), ("Le.Film.2020.FRENCH.1080p.WEB-GRP.part01", 4509715660));
        assert_eq!(parse_size("1.5 GB"), 1610612736);
        assert_eq!(parse_size("700 Mo"), 734003200);
        // lecture devinée d'une API JSON inconnue
        let body = r#"{"status":"ok","results":{"total":2,"items":[{"release_name":"A.2020-G","size_bytes":10,"link":"https://x/1"},{"release_name":"B.2021-G","size_bytes":20,"link":"https://x/2"}]}}"#;
        let g = guess(body);
        assert_eq!((g.items.as_str(), g.title.as_str(), g.size.as_str(), g.link.as_str()), ("results.items", "release_name", "size_bytes", "link"));
        assert_eq!(extract(body, &g, "json").unwrap().len(), 2);
        assert_eq!(extract(body, &RespMap::default(), "json").unwrap().len(), 2);
        assert_eq!(detect("<!DOCTYPE html><html>", ""), "html");
    }

    #[test]
    fn import_openapi_yaml_postman_wiki() {
        let y = "openapi: 3.0.1\ninfo:\n  title: Ix API # commentaire\n  version: '1.0'\nservers:\n  - url: https://ix.example/api/v2\ncomponents:\n  securitySchemes:\n    key:\n      type: apiKey\n      in: header\n      name: X-Api-Key\npaths:\n  /releases/search:\n    get:\n      summary: Search releases\n      description: >\n        Full text search\n        over releases.\n      parameters:\n        - name: q\n          in: query\n          required: true\n          description: Search terms\n          schema:\n            type: string\n        - name: cat\n          in: query\n          schema:\n            type: integer\n            enum: [2000, 5000]\n      responses:\n        '200':\n          content:\n            application/json: {}\n  /releases/{id}:\n    get:\n      operationId: getRelease\n      parameters:\n        - name: id\n          in: path\n          required: true\n  /nzb:\n    post:\n      summary: Upload NZB\n      requestBody:\n        content:\n          application/json:\n            schema:\n              type: object\n              required: [name]\n              properties:\n                name:\n                  type: string\n                file:\n                  type: string\n                  format: binary\n";
        let j = yaml(y).unwrap();
        assert_eq!(j.get("info").get("title").s(), "Ix API");
        assert_eq!(j.get("paths").get("/releases/search").get("get").get("description").s(), "Full text search over releases.");
        let imp = import(y);
        assert_eq!(imp.format, "OpenAPI 3.0.1");
        assert_eq!(imp.base.as_deref(), Some("https://ix.example"));
        assert_eq!(imp.auth, Some(("header".to_string(), "X-Api-Key".to_string())));
        assert_eq!(imp.requests.len(), 3);
        let s = &imp.requests[0];
        assert_eq!((s.url.as_str(), s.label.as_str(), s.resp.format.as_str()), ("{base}/api/v2/releases/search", "Search releases", "json"));
        assert_eq!(s.params[0].value, "{release}");
        assert!(s.params[0].required);
        assert_eq!(s.params[1].choices.len(), 2);
        assert_eq!(imp.requests[1].params[0].place, "path");
        let up = &imp.requests[2];
        assert_eq!((up.method.as_str(), up.read, up.params.len(), up.params[0].place.as_str()), ("POST", false, 1, "json"));
        // appliqué à un profil vide
        let mut p = Profile::default();
        let done = apply(&mut p, &imp, "ix.yaml");
        assert_eq!((p.kind.as_str(), p.base.as_str(), p.auth.as_str(), p.requests.len()), ("json", "https://ix.example", "header", 3));
        assert!(done.iter().any(|d| d.contains("3 requête(s)")), "{done:?}");
        assert_eq!(apply(&mut p, &imp, "ix.yaml").iter().filter(|d| d.contains("ajoutée")).count(), 0);
        // Postman
        let pm = r#"{"info":{"name":"Ix"},"variable":[{"key":"baseUrl","value":"https://ix.example"}],"item":[{"name":"Recherche","item":[{"name":"Search","request":{"method":"GET","url":{"raw":"{{baseUrl}}/api/search?query=test&apikey={{apiKey}}&limit=50","query":[{"key":"query","value":"test"},{"key":"apikey","value":"{{apiKey}}"},{"key":"limit","value":"50"}]}}}]},{"name":"Details","request":{"method":"GET","url":"{{baseUrl}}/api/release/:id"}}]}"#;
        let imp = import(pm);
        assert_eq!(imp.format, "collection Postman");
        assert_eq!(imp.auth, Some(("query".to_string(), "apikey".to_string())));
        assert_eq!(imp.requests[0].url, "{base}/api/search");
        assert_eq!(imp.requests[0].params.iter().map(|p| (p.name.as_str(), p.value.as_str())).collect::<Vec<_>>(), vec![("query", "{release}"), ("limit", "50")]);
        assert_eq!(imp.requests[1].url, "{base}/api/release/{id}");
        assert_eq!(imp.requests[1].params[0].place, "path");
        // page wiki : adresse d'exemple et tableau de paramètres
        let w = "<h2>Recherche</h2><p>Exemple : <code>https://ix.example/api/v1/search?q=mon+ami&amp;cat=2000&amp;apikey=YOURKEY</code></p><table><tr><td>q</td><td>Texte cherché (obligatoire)</td></tr><tr><td>cat</td><td>Catégorie</td></tr></table>";
        let imp = import(w);
        assert_eq!(imp.base.as_deref(), Some("https://ix.example"));
        assert_eq!(imp.auth, Some(("query".to_string(), "apikey".to_string())));
        let r = &imp.requests[0];
        assert_eq!(r.url, "{base}/api/v1/search");
        assert_eq!((r.params[0].value.as_str(), r.params[0].enc.as_str(), r.params[0].label.as_str(), r.params[0].required), ("{release}", "+", "Texte cherché (obligatoire)", true));
        assert_eq!((r.params[1].value.as_str(), r.params[1].label.as_str()), ("2000", "Catégorie"));
        // documentation Newznab : type Newznab, requêtes standard
        let imp = import("Our API follows Newznab: https://nz.example/api?t=search&q=test&apikey=KEY and t=caps");
        assert_eq!(imp.kind, "newznab");
        assert!(imp.requests.is_empty());
    }

    #[test]
    fn profil_json_aller_retour() {
        let mut p = from_preset(&preset("nzbindex").unwrap());
        p.key = "secret".into();
        p.notes = "ligne 1\nligne\t2 « é » \"q\"".into();
        p.requests[0].headers.push(("X-Test".into(), "a\\b".into()));
        let back = Profile::from_json(&p.to_json(true)).unwrap();
        assert_eq!(back, p);
        assert!(!p.to_json(true).contains('\n') && !p.to_json(true).contains('\t'));
        let shared = Profile::from_json(&p.to_json(false)).unwrap();
        assert_eq!(shared.key, "");
        assert!(Profile::from_json("{\"a\":1}").is_err());
        let imp = import(&p.to_json(false));
        let mut q = Profile { key: "mine".into(), ..Default::default() };
        apply(&mut q, &imp, "x");
        assert_eq!((q.key.as_str(), q.requests.len()), ("mine", 1));
    }

    #[test]
    fn envoi_post_json_bout_en_bout() {
        let _g = crate::prep::test_lock();
        let (addr, seen) = crate::dl::tests::fake_server(|head, req| {
            if head.starts_with("POST /v1/search") && req.contains("\"query\":\"Le Film\"") {
                r#"{"total":1,"hits":[{"title":"Le.Film.2020-G","bytes":"1.5 GB","url":"https://x/1"}]}"#.to_string()
            } else {
                r#"{"error":{"message":"bad request"}}"#.to_string()
            }
        });
        let i = idx("");
        let mut p = Profile { kind: "json".into(), base: addr.clone(), auth: "bearer".into(), ..Default::default() };
        let mut r = rq("{base}/v1/search", vec![Param { place: "json".into(), ..Param::new("query", "{release}", "") }]);
        r.method = "POST".into();
        p.requests.push(r.clone());
        let rep = send(&i, &p, &r, &[("release".into(), "Le Film".into())]).unwrap();
        assert_eq!(rep.error, None, "{rep:?}");
        assert_eq!(rep.rows, vec![Row { title: "Le.Film.2020-G".into(), size: 1610612736, link: "https://x/1".into(), ..Default::default() }]);
        assert_eq!(rep.summary, "HTTP 200 OK · 1 résultat(s)");
        assert!(rep.preview.iter().all(|l| !l.contains("K3Y-LONG")), "{:?}", rep.preview);
        let s = seen.lock().unwrap().join("\n");
        assert!(s.contains("Authorization: Bearer K3Y-LONG") && s.contains("Content-Type: application/json"), "{s}");
        let rep = send(&i, &p, &r, &[("release".into(), "Autre".into())]).unwrap();
        assert_eq!(rep.error.as_deref(), Some("réponse de l'API : bad request"));
    }
}


// ------------------------------------------------------------------ identité réseau (User-Agent)

/// Navigateurs courants (Windows, macOS, Linux) : un User-Agent qui ne dit pas « curl » ni le
/// nom de l'application. Mis à jour avec l'application.
pub const BROWSER_UAS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:144.0) Gecko/20100101 Firefox/144.0",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:143.0) Gecko/20100101 Firefox/143.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14.7; rv:144.0) Gecko/20100101 Firefox/144.0",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64; rv:144.0) Gecko/20100101 Firefox/144.0",
];

static UA_GLOBAL: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Réglage général (Réglages → Indexers → Identité des requêtes), noté à chaque image.
pub fn set_ua_global(spec: &str) {
    if let Ok(mut g) = UA_GLOBAL.lock() {
        if g.as_str() != spec.trim() {
            *g = spec.trim().to_string();
        }
    }
}

fn ua_global() -> String {
    UA_GLOBAL.lock().map(|g| g.clone()).unwrap_or_default()
}

/// Tirage pseudo-aléatoire (horloge + compteur), sans dépendance.
fn ua_pick() -> &'static str {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let mut x = t ^ N.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed) ^ (std::process::id() as u64).rotate_left(32);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    BROWSER_UAS[(x % BROWSER_UAS.len() as u64) as usize]
}

/// Navigateur tiré au sort une fois par lancement de l'application.
fn ua_session() -> &'static str {
    static S: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    S.get_or_init(ua_pick)
}

/// User-Agent à envoyer : réglage de l'indexer (`spec`), sinon réglage général. None : celui
/// de curl.
pub fn ua_for(spec: &str) -> Option<String> {
    let s = if spec.trim().is_empty() { ua_global() } else { spec.trim().to_string() };
    match s.as_str() {
        "" | "curl" => None,
        "app" => Some(format!("NZBFFToolPro/{}", crate::VERSION)),
        "browser" => Some(ua_session().to_string()),
        "rotate" => Some(ua_pick().to_string()),
        custom => Some(custom.to_string()),
    }
}

/// Libellé d'un réglage d'identité.
pub fn ua_label(spec: &str) -> String {
    match spec.trim() {
        "" => "réglage général".into(),
        "curl" => "celui de curl (« curl/8.x »)".into(),
        "app" => format!("NZBFFToolPro/{}", crate::VERSION),
        "browser" => "un navigateur courant, tiré au sort à chaque lancement".into(),
        "rotate" => "un navigateur différent à chaque requête (roulement)".into(),
        c => format!("« {c} »"),
    }
}

/// Résumé pour la fiche de l'indexer.
pub fn ua_summary(i: &IndexerCfg) -> String {
    let own = i.api.user_agent.trim();
    if own.is_empty() {
        let g = ua_global();
        format!("User-Agent : {} (réglage général)", ua_label(if g.is_empty() { "curl" } else { &g }))
    } else {
        format!("User-Agent : {} (propre à cet indexer)", ua_label(own))
    }
}
