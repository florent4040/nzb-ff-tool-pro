//! Compréhension de la documentation d'un indexer, pour régler l'envoi et la recherche sans
//! rien deviner à la main : exemples `curl` (`-F file=@…`, `-H`, clé dans l'adresse), pages
//! Newznab (« Base URL », fonctions `/api?t=…`, tableaux de paramètres, « Multipart form
//! fields »), wikis (« Use with `/api?t=search&apikey=…` », page de la release), exigences
//! (droits d'upload, NFO conseillé, catégorie facultative, limites).
//!
//! Aussi : diagnostic d'un dépôt refusé (405, 404, clé refusée, champ manquant, codes
//! Newznab) avec la correction proposée, et recherche de l'adresse d'envoi d'un indexer
//! (requêtes GET sans clé ni fichier : rien n'est envoyé).

use crate::prep::IndexerCfg;

/// Champ de fichier du formulaire d'envoi.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Field {
    pub name: String,
    /// « nzb », « nfo », « mediainfo », « autre ».
    pub kind: String,
    pub required: bool,
}

/// Envoi des NZB tel que la documentation le décrit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Upload {
    pub method: String,
    /// Adresse sans paramètres (« https://indexer.example/v1/upload », « …/api »).
    pub url: String,
    /// Fonction Newznab `t=nzbadd` (sinon : API d'envoi propre à l'indexer).
    pub newznab: bool,
    pub fields: Vec<Field>,
    /// Paramètres fixes vus dans l'exemple (hors clé, `t`, `cat`).
    pub flags: Vec<(String, String)>,
    /// Catégorie : « query », « form », vide si non citée.
    pub cat: String,
    /// La catégorie est facultative (détectée par l'indexer si absente).
    pub cat_optional: bool,
    /// Ligne de la doc d'où vient l'exemple.
    pub example: String,
}

/// Ce que la documentation apprend.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Analysis {
    pub upload: Option<Upload>,
    /// Adresse de l'API Newznab de recherche (« https://indexer.example/api »).
    pub api: Option<String>,
    /// Adresse du site (« https://indexer.example »).
    pub site: Option<String>,
    /// Transmission de la clé : (« query » | « header » | « bearer », nom).
    pub key: Option<(String, String)>,
    /// Page d'une release après l'envoi, avec {guid}.
    pub release_page: Option<String>,
    /// Exigences de l'indexer (droits, fichiers obligatoires…).
    pub needs: Vec<String>,
    /// Remarques utiles (NFO conseillé, catégorie facultative, limites…).
    pub notes: Vec<String>,
    /// Fonctions Newznab citées (search, tvsearch, movie, nzbadd…).
    pub functions: Vec<String>,
    /// Résultats par requête au plus.
    pub limit_max: Option<u32>,
}

// ------------------------------------------------------------------ lecture

/// Texte brut : balises HTML retirées, lignes « \ » de shell recollées.
fn plain(text: &str) -> String {
    let mut t = text.replace("\r\n", "\n").replace('\r', "\n");
    if t.contains("</") || t.contains("<br") {
        for tag in ["<br>", "<br/>", "<br />", "</p>", "</tr>", "</li>", "</h1>", "</h2>", "</h3>", "</h4>", "</pre>", "</div>", "</code>"] {
            t = t.replace(tag, &format!("{tag}\n"));
        }
        t = t.replace("</td>", "\t").replace("</th>", "\t");
        let mut out = String::with_capacity(t.len());
        let mut inside = false;
        for c in t.chars() {
            match c {
                '<' => inside = true,
                '>' if inside => inside = false,
                _ if !inside => out.push(c),
                _ => {}
            }
        }
        t = out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&nbsp;", " ");
    }
    // « curl -X POST "…" \ » + ligne suivante : une seule commande
    let mut joined = String::new();
    for l in t.lines() {
        let tl = l.trim_end();
        if let Some(head) = tl.strip_suffix('\\') {
            joined.push_str(head);
            joined.push(' ');
        } else {
            joined.push_str(tl);
            joined.push('\n');
        }
    }
    joined
}

/// Découpe façon shell (guillemets simples et doubles).
fn words(s: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut q: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match q {
            Some(x) if c == x => q = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                q = Some(c);
                any = true;
            }
            None if c.is_whitespace() => {
                if !cur.is_empty() || any {
                    v.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            None => cur.push(c),
        }
    }
    if !cur.is_empty() || any {
        v.push(cur);
    }
    v
}

/// Valeur d'exemple de la clé (« YOUR_API_KEY », « <apikey> », « … »).
fn is_placeholder(v: &str) -> bool {
    let l = v.to_lowercase();
    v.is_empty() || l.contains("your") || l.contains("key") || l.contains("token") || v.starts_with(['<', '{', '$']) || v.contains('…') || v.contains("...") || l.contains("xxx")
}

const KEY_NAMES: &[&str] = &["apikey", "api_key", "api-key", "key", "token", "access_token", "passkey"];

/// « https://a.b/c/d?x » → « https://a.b ».
pub fn origin(url: &str) -> String {
    let u = url.trim();
    match u.find("://") {
        Some(p) => {
            let rest = &u[p + 3..];
            let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
            format!("{}{host}", &u[..p + 3])
        }
        None => String::new(),
    }
}

fn kind_of(name: &str, file: &str) -> String {
    let n = name.to_lowercase();
    let f = file.to_lowercase();
    if f.ends_with(".nzb") || n == "file" || n == "nzb" || n == "nzbfile" || n == "upload" || f.contains("x-nzb") {
        "nzb"
    } else if n.contains("mediainfo") || n == "mi" || (f.ends_with(".xml") && (n.contains("media") || f.contains("mediainfo"))) {
        // avant le NFO : « mediainfo » contient « nfo »
        "mediainfo"
    } else if f.ends_with(".nfo") || n.contains("nfo") {
        "nfo"
    } else {
        "autre"
    }
    .into()
}

/// Commande curl de la doc : (méthode, adresse, champs, en-têtes).
struct Curl {
    method: String,
    url: String,
    fields: Vec<(String, String)>,
    headers: Vec<String>,
}

fn parse_curl(line: &str) -> Option<Curl> {
    let w = words(line);
    let start = w.iter().position(|x| x == "curl" || x.ends_with("/curl") || x.ends_with("curl.exe"))?;
    let mut c = Curl { method: String::new(), url: String::new(), fields: Vec::new(), headers: Vec::new() };
    let mut i = start + 1;
    while i < w.len() {
        let a = w[i].as_str();
        let next = w.get(i + 1).cloned().unwrap_or_default();
        match a {
            "-X" | "--request" => {
                c.method = next.to_uppercase();
                i += 1;
            }
            "-F" | "--form" | "--form-string" => {
                if let Some((k, v)) = next.split_once('=') {
                    c.fields.push((k.trim().to_string(), v.trim().to_string()));
                }
                i += 1;
            }
            "-H" | "--header" => {
                c.headers.push(next);
                i += 1;
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" => {
                if c.method.is_empty() {
                    c.method = "POST".into();
                }
                i += 1;
            }
            _ if (a.starts_with("http://") || a.starts_with("https://")) && c.url.is_empty() => c.url = a.to_string(),
            _ => {}
        }
        i += 1;
    }
    if c.url.is_empty() {
        return None;
    }
    if c.method.is_empty() {
        c.method = if c.fields.is_empty() { "GET".into() } else { "POST".into() };
    }
    Some(c)
}

/// Requête écrite à la main : une adresse (« https://x/api?t=search&q=Dune&apikey=… »),
/// « POST https://x/… », ou une commande curl (-X, -F, -H, -d). La clé de l'indexer, si elle y
/// est écrite en clair, est remplacée par {key} (elle ne s'affiche ni ne s'enregistre jamais).
/// `site` : base pour une adresse relative (« /api?t=caps »).
pub fn req_from_text(text: &str, key: &str, site: &str) -> Result<crate::apidef::Req, String> {
    use crate::apidef::{Param, Req, RespMap};
    let t = text.trim().replace(['\r', '\n'], " ").replace(" \\ ", " ");
    if t.is_empty() {
        return Err("écris une adresse, « GET https://… », « POST https://… » ou une commande curl".into());
    }
    let (method, mut url, fields, headers, body) = if t.split_whitespace().next().map(|w| w == "curl" || w.ends_with("curl.exe")).unwrap_or(false) {
        let c = parse_curl(&t).ok_or("commande curl sans adresse http(s)")?;
        let w = words(&t);
        let body = w.iter().position(|x| matches!(x.as_str(), "-d" | "--data" | "--data-raw")).and_then(|i| w.get(i + 1).cloned()).unwrap_or_default();
        (c.method, c.url, c.fields, c.headers, body)
    } else {
        let mut it = t.splitn(2, char::is_whitespace);
        let first = it.next().unwrap_or("").to_string();
        let up = first.to_uppercase();
        if ["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&up.as_str()) {
            (up, it.next().unwrap_or("").trim().to_string(), Vec::new(), Vec::new(), String::new())
        } else {
            ("GET".to_string(), t.clone(), Vec::new(), Vec::new(), String::new())
        }
    };
    if url.starts_with('/') {
        url = format!("{}{url}", site.trim_end_matches('/'));
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("adresse non comprise : « {} » (elle doit commencer par http:// ou https://)", url.chars().take(60).collect::<String>()));
    }
    let key = key.trim();
    let hide = |v: &str| if !key.is_empty() && v.contains(key) { v.replace(key, "{key}") } else { v.to_string() };
    let base = url.split('?').next().unwrap_or("").to_string();
    let mut params: Vec<Param> = Vec::new();
    for (k, v) in query_of(&url) {
        let v = pct_decode(&v);
        let v = if KEY_NAMES.contains(&k.to_lowercase().as_str()) && (is_placeholder(&v) || v.contains(key) && !key.is_empty()) { "{key}".to_string() } else { hide(&v) };
        params.push(Param::new(&k, &v, &k));
    }
    for (k, v) in &fields {
        let mut p = Param::new(k, &hide(v), k);
        p.place = "form".into();
        params.push(p);
    }
    let hdrs: Vec<(String, String)> = headers.iter().filter_map(|h| h.split_once(':').map(|(a, b)| (a.trim().to_string(), hide(b.trim())))).collect();
    let read = method == "GET" && !params.iter().any(|p| p.name == "t" && matches!(p.value.as_str(), "nzbadd" | "na" | "commentadd" | "cartadd" | "cartdel"));
    let label = params.iter().find(|p| p.name == "t").map(|p| format!("Ma requête : t={}", p.value)).unwrap_or_else(|| format!("Ma requête : {}", base.rsplit('/').next().filter(|x| !x.is_empty()).unwrap_or("/")));
    Ok(Req { id: String::new(), label, method, url: hide(&base), params, headers: hdrs, body: hide(&body), read, origin: "manuel".into(), resp: RespMap { format: "auto".into(), ..Default::default() }, ..Default::default() })
}

/// « Dune%202021 » / « Dune+2021 » → « Dune 2021 ».
fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Paramètres d'une adresse : (nom, valeur).
fn query_of(url: &str) -> Vec<(String, String)> {
    url.split_once('?')
        .map(|(_, q)| q.split('&').filter(|x| !x.is_empty()).map(|kv| kv.split_once('=').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((kv.to_string(), String::new()))).collect())
        .unwrap_or_default()
}

/// Comprend une documentation d'indexer (texte, HTML, wiki, page Newznab).
pub fn analyze(text: &str) -> Analysis {
    let t = plain(text);
    let low = t.to_lowercase();
    let mut a = Analysis::default();
    let mut site_votes: Vec<String> = Vec::new();

    // ---- exemples curl
    for line in t.lines().filter(|l| l.contains("curl ")) {
        let Some(c) = parse_curl(line) else { continue };
        let base = c.url.split('?').next().unwrap_or("").to_string();
        let q = query_of(&c.url);
        let has_file = c.fields.iter().any(|(_, v)| v.starts_with('@'));
        let is_upload = has_file || c.url.to_lowercase().contains("upload") || q.iter().any(|(k, v)| k == "t" && (v == "nzbadd" || v == "na"));
        site_votes.push(origin(&c.url));
        for (k, v) in &q {
            if KEY_NAMES.contains(&k.to_lowercase().as_str()) && is_placeholder(v) && a.key.is_none() {
                a.key = Some(("query".into(), k.clone()));
            }
        }
        for h in &c.headers {
            if let Some((n, v)) = h.split_once(':') {
                let (n, v) = (n.trim(), v.trim());
                if n.eq_ignore_ascii_case("authorization") && v.to_lowercase().starts_with("bearer") {
                    a.key = Some(("bearer".into(), String::new()));
                } else if (n.to_lowercase().contains("key") || n.to_lowercase().contains("token")) && is_placeholder(v) {
                    a.key = Some(("header".into(), n.to_string()));
                }
            }
        }
        if !is_upload {
            continue;
        }
        let newznab = q.iter().any(|(k, v)| k == "t" && (v == "nzbadd" || v == "na"));
        let mut up = Upload { method: c.method.clone(), url: base.clone(), newznab, example: line.trim().chars().take(300).collect(), ..Default::default() };
        for (k, v) in &c.fields {
            if let Some(file) = v.strip_prefix('@') {
                let file = file.split(';').next().unwrap_or(file);
                let kind = kind_of(k, &format!("{file} {v}"));
                if !up.fields.iter().any(|f| &f.name == k) {
                    up.fields.push(Field { name: k.clone(), required: kind == "nzb", kind });
                }
            } else if k == "cat" || k == "category" {
                up.cat = "form".into();
            }
        }
        for (k, v) in &q {
            let lk = k.to_lowercase();
            if KEY_NAMES.contains(&lk.as_str()) || lk == "t" {
                continue;
            }
            if lk == "cat" || lk == "category" {
                up.cat = "query".into();
                continue;
            }
            up.flags.push((k.clone(), v.clone()));
        }
        // le plus complet l'emporte (un exemple avec la catégorie, ou avec plus de champs)
        let better = match &a.upload {
            None => true,
            Some(o) => up.fields.len() > o.fields.len() || (o.cat.is_empty() && !up.cat.is_empty() && up.url == o.url),
        };
        if better {
            if let Some(o) = &a.upload {
                if o.url == up.url {
                    for f in &o.fields {
                        if !up.fields.iter().any(|x| x.name == f.name) {
                            up.fields.push(f.clone());
                        }
                    }
                }
            }
            a.upload = Some(up);
        } else if let Some(o) = a.upload.as_mut() {
            if o.url == up.url && o.cat.is_empty() {
                o.cat = up.cat.clone();
            }
        }
    }

    // ---- adresses de la doc (« Base URL », exemples de fonctions Newznab)
    let lines: Vec<&str> = t.lines().map(str::trim).collect();
    for (n, l) in lines.iter().enumerate() {
        let ll = l.to_lowercase();
        if ll == "base url" || ll.starts_with("base url") {
            let cand = l.split_whitespace().find(|w| w.starts_with("http")).map(str::to_string).or_else(|| lines.iter().skip(n + 1).find(|x| !x.is_empty()).filter(|x| x.starts_with("http")).map(|x| x.split_whitespace().next().unwrap_or("").to_string()));
            if let Some(u) = cand {
                let u = u.trim_end_matches(['/', '.', ',']).to_string();
                site_votes.push(origin(&u));
                if u.to_lowercase().ends_with("/api") || u.contains("/api/") {
                    a.api.get_or_insert(u);
                }
            }
        }
        for w in l.split_whitespace() {
            let w = w.trim_matches(['`', '"', '\'', '(', ')', ',', '.']);
            if w.starts_with("http") {
                site_votes.push(origin(w));
                if let Some((p, _)) = w.split_once("?t=").or_else(|| w.split_once("&t=")) {
                    a.api.get_or_insert(p.trim_end_matches('/').to_string());
                }
            }
        }
    }
    // fonctions Newznab citées (« /api?t=search », « t=nzbadd », « GET tvsearch »)
    let mut rest = low.as_str();
    while let Some(p) = rest.find("t=") {
        let before_ok = p == 0 || matches!(rest.as_bytes()[p - 1], b'?' | b'&' | b' ' | b'`');
        let f: String = rest[p + 2..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        if before_ok && f.starts_with(|c: char| c.is_ascii_alphabetic()) && f.len() < 16 && !a.functions.contains(&f) {
            a.functions.push(f);
        }
        rest = &rest[p + 2..];
    }
    let newznab_doc = a.functions.iter().any(|f| ["search", "caps", "tvsearch", "movie", "nzbadd"].contains(&f.as_str())) || low.contains("newznab");

    // site : le plus cité
    site_votes.retain(|s| !s.is_empty());
    let mut best: Option<(String, usize)> = None;
    for s in &site_votes {
        let n = site_votes.iter().filter(|x| *x == s).count();
        if best.as_ref().map(|b| n > b.1).unwrap_or(true) {
            best = Some((s.clone(), n));
        }
    }
    a.site = best.map(|b| b.0);
    // « Use with `/api?t=search&apikey=…` » : chemin relatif au site
    if a.api.is_none() {
        if let Some(p) = low.find("/api?t=") {
            let start = low[..p].rfind(|c: char| c.is_whitespace() || c == '`' || c == '"').map(|x| x + 1).unwrap_or(p);
            let path: String = t[start..p + 4].trim_matches(['`', '"']).to_string();
            if let Some(site) = &a.site {
                let path = if path.starts_with('/') { path } else { "/api".into() };
                a.api = Some(format!("{site}{path}"));
            }
        }
    }
    if a.key.is_none() && (low.contains("apikey=") || low.contains("pass the key as apikey")) {
        a.key = Some(("query".into(), "apikey".into()));
    }

    // ---- envoi Newznab décrit sans curl (« POST nzbadd », « Multipart form fields »)
    if a.upload.is_none() && a.functions.iter().any(|f| f == "nzbadd") {
        if let Some(api) = &a.api {
            a.upload = Some(Upload { method: "POST".into(), url: api.clone(), newznab: true, ..Default::default() });
        }
    }
    if let Some(up) = a.upload.as_mut() {
        // champs cités en toutes lettres (« file (required), optional nfo and mediainfo »)
        if let Some(p) = low.find("multipart form fields") {
            let seg: String = low[p..].chars().take(160).collect::<String>().split('.').next().unwrap_or("").to_string();
            for (name, kind) in [("file", "nzb"), ("nzb", "nzb"), ("nfo", "nfo"), ("mediainfo", "mediainfo")] {
                let found = seg.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == name);
                if found && !up.fields.iter().any(|f| f.name == name) && !(name == "nzb" && up.fields.iter().any(|f| f.kind == "nzb")) {
                    up.fields.push(Field { name: name.into(), kind: kind.into(), required: kind == "nzb" });
                }
            }
        }
        // tableaux « nom  type  obligatoire  description » de la section de l'envoi
        let sec = section(&lines, &["nzbadd", "upload"]);
        for row in sec.iter().map(|l| l.split('\t').map(str::trim).filter(|c| !c.is_empty()).collect::<Vec<_>>()).filter(|c| c.len() >= 3) {
            let name = row[0].to_lowercase();
            let typ = row[1].to_lowercase();
            let req = row.get(2).map(|x| x.eq_ignore_ascii_case("yes") || x.eq_ignore_ascii_case("oui") || x.eq_ignore_ascii_case("required")).unwrap_or(false);
            if typ == "file" {
                let kind = kind_of(&name, "");
                match up.fields.iter_mut().find(|f| f.name == name) {
                    Some(f) => f.required = req || f.required,
                    None => up.fields.push(Field { name: name.clone(), kind, required: req }),
                }
            } else if name == "cat" && up.cat.is_empty() {
                up.cat = "query".into();
                up.cat_optional = !req;
            } else if (typ == "boolean" || typ == "bool") && !up.flags.iter().any(|(k, _)| k == &name) {
                let dflt = row.last().map(|d| d.to_lowercase()).unwrap_or_default();
                let v = if dflt.contains("default true") { "true" } else if dflt.contains("default false") { "false" } else { "" };
                up.flags.push((name.clone(), v.into()));
            }
        }
        if up.fields.is_empty() {
            up.fields.push(Field { name: "file".into(), kind: "nzb".into(), required: true });
        }
        if low.contains("category is optional") || low.contains("leave it blank") || low.contains("instead of auto-detect") || low.contains("detect it automatically") || low.contains("catégorie facultative") {
            up.cat_optional = true;
        }
    }

    // ---- page de la release (« The release page is /account/releases/<guid> »)
    for l in &lines {
        let ll = l.to_lowercase();
        if !(ll.contains("release page") || ll.contains("page de la release") || ll.contains("details page")) {
            continue;
        }
        if let Some(w) = l.split_whitespace().map(|w| w.trim_matches(['`', '"', '.', ','])).find(|w| w.contains("<guid>") || w.contains("{guid}") || w.contains(":guid")) {
            let w = w.replace("<guid>", "{guid}").replace(":guid", "{guid}");
            let full = if w.starts_with("http") { w } else { format!("{}{}", a.site.clone().unwrap_or_default(), if w.starts_with('/') { w } else { format!("/{w}") }) };
            a.release_page = Some(full);
        }
    }

    // ---- exigences et remarques (traduites quand elles sont reconnues)
    let has = |s: &str| low.contains(s);
    if has("posting privileges") || has("upload privileges") || has("uploader rights") || has("droits d'upload") {
        a.needs.push("Le compte doit avoir le droit d'upload (posting privileges) : sinon refus « droits insuffisants » (code 102)".into());
    }
    if let Some(up) = &a.upload {
        for f in up.fields.iter().filter(|f| f.required) {
            a.needs.push(format!("Champ « {} » obligatoire ({})", f.name, kind_label(&f.kind)));
        }
    }
    if has("nfo is recommended") || has("matching nfo is recommended") || has("nfo recommended") {
        a.notes.push("NFO conseillé (pas obligatoire) : il donne un meilleur nom de release".into());
    }
    if has("same filename") || has("same file name") {
        a.notes.push("Le NFO doit porter le même nom que le NZB pour être associé (c'est le cas : les deux prennent le nom de la release)".into());
    }
    if a.upload.as_ref().map(|u| u.cat_optional).unwrap_or(false) {
        a.notes.push("Catégorie facultative : sans elle, l'indexer la détecte ; elle ne sert qu'à forcer la sienne".into());
    }
    if has("adult/xxx is not a separate category") || has("xxx is not a separate") {
        a.notes.push("Pas de catégorie Adulte/XXX à part : laisser la catégorie vide pour ces releases".into());
    }
    if has("returns a guid") || has("api returns a guid") {
        a.notes.push("La réponse donne le guid de la release (sa page est ouverte d'un clic après le dépôt)".into());
    }
    if has("http 429") || has("rate limit") {
        a.notes.push("Trop de requêtes : HTTP 429 (l'application attend puis réessaie)".into());
    }
    if has("empty q is rejected") {
        a.notes.push("Recherche : un texte vide est refusé".into());
    }
    if has("includemeta") && has("default false") {
        a.notes.push("includemeta=true garde les balises <head> du NZB (mot de passe…) ; false par défaut chez l'indexer".into());
    }
    if has("dupecheck") {
        a.notes.push("dupecheck=false désactive la vérification des doublons de l'indexer (laisser true)".into());
    }
    if let Some(p) = low.find("maximum ") {
        let n: String = low[p + 8..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(v) = n.parse::<u32>() {
            if (10..=1000).contains(&v) {
                a.limit_max = Some(v);
                a.notes.push(format!("Au plus {v} résultats par recherche"));
            }
        }
    }
    if newznab_doc && a.upload.as_ref().map(|u| !u.newznab).unwrap_or(false) {
        a.notes.push("Recherche par l'API Newznab, mais envoi des NZB par une adresse à part : les deux sont réglés séparément".into());
    }
    a
}

/// Lignes de la section d'une fonction (de son titre au titre suivant).
fn section<'a>(lines: &[&'a str], keys: &[&str]) -> Vec<&'a str> {
    let is_head = |l: &str| {
        let ll = l.to_lowercase();
        ll.starts_with("/api?t=") || ll.starts_with("get ") || ll.starts_with("post ") || ll.starts_with("## ") || ll.starts_with("### ")
    };
    let mut out = Vec::new();
    let mut inside = false;
    for l in lines {
        if is_head(l) {
            let ll = l.to_lowercase();
            inside = keys.iter().any(|k| ll.contains(k));
            continue;
        }
        if inside {
            out.push(*l);
        }
    }
    out
}

fn kind_label(k: &str) -> &'static str {
    match k {
        "nzb" => "le NZB",
        "nfo" => "le NFO",
        "mediainfo" => "le rapport MediaInfo",
        _ => "fichier",
    }
}

/// Ce qui a été compris, en clair (une ligne par point).
pub fn summary(a: &Analysis) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(s) = &a.site {
        v.push(format!("Site : {s}"));
    }
    if let Some(api) = &a.api {
        v.push(format!("API de recherche (Newznab) : {api}"));
    }
    if let Some(u) = &a.upload {
        v.push(format!("Envoi des NZB : {} {}{}", u.method, u.url, if u.newznab { "?t=nzbadd (Newznab)" } else { " (adresse propre à l'envoi)" }));
        let f: Vec<String> = u.fields.iter().map(|f| format!("« {} » = {}{}", f.name, kind_label(&f.kind), if f.required { " (obligatoire)" } else { "" })).collect();
        v.push(format!("Champs du formulaire : {}", f.join(", ")));
        match u.cat.as_str() {
            "query" => v.push(format!("Catégorie : paramètre « cat » de l'adresse{}", if u.cat_optional { " (facultatif)" } else { "" })),
            "form" => v.push(format!("Catégorie : champ « cat » du formulaire{}", if u.cat_optional { " (facultatif)" } else { "" })),
            _ => {}
        }
        if !u.flags.is_empty() {
            v.push(format!("Options : {}", u.flags.iter().map(|(k, x)| if x.is_empty() { k.clone() } else { format!("{k}={x}") }).collect::<Vec<_>>().join(", ")));
        }
        if !u.fields.iter().any(|f| f.kind == "mediainfo") {
            v.push("Pas de champ MediaInfo dans l'envoi : le rapport n'est pas joint (il reste créé)".into());
        }
    }
    if let Some((p, n)) = &a.key {
        v.push(format!("Clé : {}", crate::apidef::auth_label(p, n)));
    }
    if let Some(r) = &a.release_page {
        v.push(format!("Page de la release : {r}"));
    }
    if !a.functions.is_empty() {
        v.push(format!("Fonctions Newznab : {}", a.functions.join(", ")));
    }
    v
}

/// Règle l'indexer d'après la documentation. Renvoie ce qui a changé, en clair.
pub fn apply(idx: &mut IndexerCfg, a: &Analysis) -> Vec<String> {
    let mut done = Vec::new();
    if let Some(api) = &a.api {
        if idx.api_url.trim() != api {
            done.push(format!("adresse de l'API (recherche) : {api}"));
            idx.api_url = api.clone();
        }
    }
    if let Some(u) = &a.upload {
        if u.newznab {
            if idx.mode != "newznab" {
                done.push("envoi : API Newznab (t=nzbadd)".into());
                idx.mode = "newznab".into();
            }
            if a.api.is_none() && idx.api_url.trim() != u.url {
                done.push(format!("adresse de l'API : {}", u.url));
                idx.api_url = u.url.clone();
            }
            for (k, v) in &u.flags {
                match (k.as_str(), v.as_str()) {
                    ("dupecheck", _) if !idx.dupecheck => {
                        idx.dupecheck = true;
                        done.push("dupecheck=true (vérification des doublons de l'indexer)".into());
                    }
                    _ => {}
                }
            }
        } else {
            if idx.mode != "rest" {
                done.push("envoi : adresse à part (la recherche reste sur l'API Newznab)".into());
                idx.mode = "rest".into();
            }
            if idx.rest_path.trim() != u.url {
                done.push(format!("adresse d'envoi : {}", u.url));
                idx.rest_path = u.url.clone();
            }
            let site = origin(&u.url);
            if idx.rest_url.trim().is_empty() || idx.rest_url.trim() != site {
                idx.rest_url = site;
            }
            // paramètres fixes de l'exemple (ex. « private=no ») : gardés tels quels
            let extra: Vec<String> = u.flags.iter().filter(|(_, v)| !v.is_empty() && !is_placeholder(v)).map(|(k, v)| format!("{k}={v}")).collect();
            if !extra.is_empty() && idx.extra_query.trim().is_empty() {
                idx.extra_query = extra.join("&");
                done.push(format!("paramètres ajoutés à l'adresse : {}", idx.extra_query));
            }
        }
        if let Some((p, n)) = &a.key {
            if idx.auth != *p || (!n.is_empty() && idx.auth_name.trim() != n) {
                idx.auth = p.clone();
                if !n.is_empty() {
                    idx.auth_name = n.clone();
                }
                done.push(format!("clé transmise par {}", crate::apidef::auth_label(p, n)));
            }
        } else if !u.newznab && idx.auth != "query" {
            idx.auth = "query".into();
            idx.auth_name = "apikey".into();
            done.push("clé transmise par le paramètre d'adresse « apikey »".into());
        }
        let name_of = |k: &str| u.fields.iter().find(|f| f.kind == k).map(|f| f.name.clone());
        let set = |cur: &mut String, v: Option<String>, dflt: &str, label: &str, done: &mut Vec<String>| {
            if let Some(v) = v {
                let v = if v == dflt { String::new() } else { v };
                if *cur != v {
                    *cur = v.clone();
                    done.push(format!("champ {label} : « {} »", if v.is_empty() { dflt.to_string() } else { v }));
                }
            }
        };
        set(&mut idx.f_nzb, name_of("nzb"), "file", "du NZB", &mut done);
        set(&mut idx.f_nfo, name_of("nfo"), "nfo", "du NFO", &mut done);
        set(&mut idx.f_mi, name_of("mediainfo"), "mediainfo", "du MediaInfo", &mut done);
        if name_of("mediainfo").is_none() && idx.send_mi {
            idx.send_mi = false;
            done.push("rapport MediaInfo non joint (l'envoi n'a pas de champ pour lui)".into());
        } else if name_of("mediainfo").is_some() && !idx.send_mi {
            idx.send_mi = true;
            done.push("rapport MediaInfo joint (champ prévu par l'envoi)".into());
        }
        if u.cat_optional && idx.cat.trim().is_empty() {
            done.push("catégorie : laissée à l'indexer quand aucune ne correspond".into());
        }
    }
    if let Some(r) = &a.release_page {
        if idx.release_page.trim() != r {
            idx.release_page = r.clone();
            done.push(format!("page de la release : {r}"));
        }
    }
    let notes: Vec<String> = a.needs.iter().chain(a.notes.iter()).cloned().collect();
    if !notes.is_empty() {
        let joined = notes.join("\n");
        if idx.doc_notes != joined {
            idx.doc_notes = joined;
            done.push(format!("{} exigence(s) et remarque(s) notées", notes.len()));
        }
    }
    if let Some(m) = a.limit_max {
        if idx.api.caps.limit_max == 0 {
            idx.api.caps.limit_max = m;
        }
    }
    done
}

// ------------------------------------------------------------------ refus et corrections

/// Correction proposée après un refus.
#[derive(Clone, Debug, PartialEq)]
pub enum Fix {
    /// Chercher l'adresse d'envoi (requêtes GET sans clé).
    Probe,
    /// Coller ou importer la documentation de l'indexer.
    Doc,
    /// Régler l'indexer à la main.
    Settings,
    /// Utiliser ce nom pour le champ du NZB.
    FieldNzb(String),
    /// Vérifier la clé ou les droits du compte.
    Key,
    /// Réessayer plus tard.
    Later,
}

/// Diagnostic d'un dépôt refusé.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnosis {
    pub title: String,
    pub why: Vec<String>,
    pub fixes: Vec<Fix>,
}

/// Lit le code HTTP dans un message d'erreur (« (HTTP 405) »).
pub fn http_in(msg: &str) -> Option<u16> {
    let p = msg.find("HTTP ")?;
    msg[p + 5..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

/// Code d'erreur Newznab (« code="102" », « code 102 », « error 102 »).
fn newznab_code(msg: &str) -> Option<u32> {
    let l = msg.to_lowercase();
    for pat in ["code=\"", "code=", "code ", "error "] {
        if let Some(p) = l.find(pat) {
            let n: String = l[p + pat.len()..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(v) = n.parse::<u32>() {
                if (100..=999).contains(&v) {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Comprend un refus de dépôt et propose la correction.
pub fn diagnose(idx: &IndexerCfg, msg: &str) -> Option<Diagnosis> {
    let l = msg.to_lowercase();
    let http = http_in(msg);
    let target = match idx.mode.as_str() {
        "rest" => crate::upload::rest_target(&idx.rest_view()),
        _ => format!("{}?t=nzbadd", crate::upload::endpoint(&idx.api_url)),
    };
    let mut d = Diagnosis { title: String::new(), why: Vec::new(), fixes: Vec::new() };
    match (http, newznab_code(msg)) {
        (Some(405), _) => {
            d.title = format!("{} refuse l'envoi à cette adresse (HTTP 405 : méthode refusée)", idx.name);
            d.why.push(format!("L'adresse {target} existe mais n'accepte pas un envoi de fichier (POST)."));
            if idx.mode == "newznab" {
                d.why.push("Beaucoup d'indexers récents cherchent par l'API Newznab (/api) mais reçoivent les NZB ailleurs, par exemple /v1/upload ou /api/v1/upload.".into());
            }
            d.why.push("L'application peut chercher la bonne adresse (requêtes de lecture, sans clé ni fichier), ou la lire dans la documentation de l'indexer.".into());
            d.fixes = vec![Fix::Probe, Fix::Doc, Fix::Settings];
        }
        (Some(404), _) => {
            d.title = format!("Adresse d'envoi introuvable sur {} (HTTP 404)", idx.name);
            d.why.push(format!("Rien à l'adresse {target}."));
            d.fixes = vec![Fix::Probe, Fix::Doc, Fix::Settings];
        }
        (Some(401 | 403), _) | (_, Some(100 | 101)) => {
            d.title = format!("{} refuse la clé API", idx.name);
            d.why.push("Clé absente, fausse, expirée, ou envoyée au mauvais endroit (paramètre d'adresse, en-tête…).".into());
            d.fixes = vec![Fix::Key, Fix::Doc, Fix::Settings];
        }
        (_, Some(102)) => {
            d.title = format!("Droits insuffisants sur {}", idx.name);
            d.why.push("Le compte n'a pas le droit d'upload (posting privileges) : à demander à l'indexer.".into());
            d.fixes = vec![Fix::Key, Fix::Settings];
        }
        (Some(422 | 400), _) if l.contains("field required") || l.contains("missing") || l.contains("\"loc\"") => {
            d.title = format!("{} attend un autre formulaire", idx.name);
            // FastAPI : {"detail":[{"loc":["body","upload"],"msg":"field required"}]}
            let field = l.find("\"loc\"").and_then(|p| {
                let seg: String = l[p..].chars().take(120).collect();
                seg.split('"').filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && *w != "loc" && *w != "body" && *w != "query").nth(0).map(str::to_string)
            });
            match field {
                Some(f) => {
                    d.why.push(format!("Champ manquant d'après l'indexer : « {f} ».", ));
                    if ["file", "nzb", "nzbfile", "upload", "nzb_file", "files"].contains(&f.as_str()) && f != idx.field_nzb() {
                        d.why.push(format!("Le NZB part actuellement dans le champ « {} ».", idx.field_nzb()));
                        d.fixes.push(Fix::FieldNzb(f));
                    } else if f == "apikey" || f == "api_key" || f == "key" {
                        d.why.push("La clé est attendue dans l'adresse.".into());
                        d.fixes.push(Fix::Key);
                    }
                }
                None => d.why.push("Un champ attendu manque au formulaire.".into()),
            }
            d.fixes.extend([Fix::Doc, Fix::Settings]);
        }
        (Some(413), _) => {
            d.title = "NZB trop gros pour l'indexer (HTTP 413)".into();
            d.why.push("L'indexer limite la taille des envois.".into());
            d.fixes = vec![Fix::Settings];
        }
        (Some(429), _) | (_, Some(500 | 501)) => {
            d.title = format!("Limite de requêtes atteinte sur {}", idx.name);
            d.why.push("Nouvel essai plus tard.".into());
            d.fixes = vec![Fix::Later];
        }
        (_, Some(200 | 201)) => {
            d.title = format!("{} : paramètre manquant ou incorrect", idx.name);
            d.why.push("L'indexer attend d'autres paramètres pour l'envoi (voir sa documentation).".into());
            d.fixes = vec![Fix::Doc, Fix::Settings];
        }
        (_, Some(202 | 203)) => {
            d.title = format!("{} n'a pas la fonction nzbadd", idx.name);
            d.why.push("Son API Newznab ne reçoit pas les NZB : l'envoi se fait à une autre adresse.".into());
            d.fixes = vec![Fix::Probe, Fix::Doc, Fix::Settings];
        }
        (_, Some(c @ 600..=699)) => {
            d.title = format!("NZB refusé par {} (code {c})", idx.name);
            d.why.push("Erreur de l'envoi lui-même (NZB invalide, déjà présent, catégorie refusée…) : voir la réponse de l'indexer.".into());
            d.fixes = vec![Fix::Settings];
        }
        _ => return None,
    }
    Some(d)
}

/// Adresse d'envoi trouvée en sondant l'indexer.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub url: String,
    pub http: Option<u16>,
    /// 3 : attend un envoi (405 à une lecture), 2 : existe (clé ou champs demandés),
    /// 1 : répond (à vérifier), 0 : absente.
    pub score: u8,
    pub verdict: String,
    /// Fonction Newznab (t=nzbadd) plutôt qu'une adresse d'envoi à part.
    pub newznab: bool,
}

/// Adresses candidates d'un indexer (sans requête).
pub fn candidates(idx: &IndexerCfg) -> Vec<(String, bool)> {
    let mut sites: Vec<String> = [idx.rest_path.as_str(), idx.rest_url.as_str(), idx.api_url.as_str(), idx.url.as_str(), idx.api.base.as_str()].iter().map(|u| origin(u)).filter(|o| !o.is_empty()).collect();
    sites.dedup();
    let mut out: Vec<(String, bool)> = Vec::new();
    if !idx.api_url.trim().is_empty() {
        out.push((format!("{}?t=nzbadd", crate::upload::endpoint(&idx.api_url)), true));
    }
    for s in &sites {
        for p in ["/v1/upload", "/api/v1/upload", "/api/upload", "/upload", "/api/v1/nzb", "/api/nzb", "/api/v2/upload", "/nzb/upload", "/api/v1/nzb/upload", "/v1/nzb"] {
            let u = format!("{s}{p}");
            if !out.iter().any(|(x, _)| x == &u) {
                out.push((u, false));
            }
        }
    }
    out
}

/// Verdict d'une réponse à une lecture (GET sans clé).
pub fn judge(url: &str, newznab: bool, http: Option<u16>, body: &str) -> Hit {
    let b = body.to_lowercase();
    let (score, verdict) = if newznab {
        match newznab_code(body) {
            Some(202 | 203) => (0, "fonction nzbadd absente".to_string()),
            Some(100 | 101 | 102 | 200 | 201) => (2, "fonction nzbadd présente (demande la clé et le fichier)".into()),
            _ if http == Some(405) => (3, "attend un envoi POST".into()),
            _ if http == Some(404) => (0, "absente".into()),
            _ => (1, format!("répond (HTTP {})", http.map(|h| h.to_string()).unwrap_or("?".into()))),
        }
    } else {
        match http {
            Some(405) => (3, "existe : attend un envoi (POST)".into()),
            Some(401 | 403) => (2, "existe : demande la clé".into()),
            Some(400 | 422) => (2, "existe : demande le fichier".into()),
            Some(404) | Some(410) | None => (0, "absente".into()),
            Some(200) if b.contains("<html") => (0, "page web, pas l'API".into()),
            Some(200) => (1, "répond (à vérifier)".into()),
            Some(c) if (300..400).contains(&c) => (0, "redirection (page du site)".into()),
            Some(c) => (0, format!("HTTP {c}")),
        }
    };
    Hit { url: url.into(), http, score, verdict, newznab }
}

/// Lecture simple (GET), sans clé sauf si l'adresse la contient déjà.
fn get(url: &str) -> (String, Option<u16>) {
    let args: Vec<String> = ["-sS", "--max-time", "15", "-X", "GET", "-w", "\n@@HTTP@@%{http_code}", "-H", "Accept: application/json, text/xml, */*"].iter().map(|s| s.to_string()).chain([url.to_string()]).collect();
    crate::upload::run_curl(&args).unwrap_or((String::new(), None))
}

/// Requêtes de l'auto-configuration et de la recherche d'adresse : jamais en rafale (risque
/// de bannissement), une à la fois, avec le délai réglé entre deux, et un nombre maximal.
pub struct Pace {
    pub delay: std::time::Duration,
    pub max: usize,
    pub sent: usize,
    last: Option<std::time::Instant>,
}

impl Pace {
    pub fn new(delay_secs: u64, max: usize) -> Self {
        Pace { delay: std::time::Duration::from_secs(delay_secs), max, sent: 0, last: None }
    }

    /// Attend le délai depuis la requête précédente ; faux si le maximum est atteint.
    fn wait(&mut self) -> bool {
        if self.sent >= self.max {
            return false;
        }
        if let Some(t) = self.last {
            let e = t.elapsed();
            if e < self.delay {
                std::thread::sleep(self.delay - e);
            }
        }
        self.sent += 1;
        self.last = Some(std::time::Instant::now());
        true
    }
}

/// Sonde les adresses candidates (GET, sans clé ni fichier : rien n'est envoyé), au rythme
/// réglé ; s'arrête à la première adresse qui attend un envoi.
pub fn probe(idx: &IndexerCfg, pace: &mut Pace, on: &mut dyn FnMut(&str)) -> Vec<Hit> {
    let mut hits: Vec<Hit> = Vec::new();
    for (url, nz) in candidates(idx) {
        if !pace.wait() {
            on(&format!("ℹ {} requêtes au plus : arrêt de la recherche", pace.max));
            break;
        }
        on(&format!("… GET {url}"));
        let (body, http) = get(&url);
        let h = judge(&url, nz, http, &body);
        on(&format!("{} {url} : {}", if h.score >= 2 { "✔" } else { "·" }, h.verdict));
        let strong = h.score >= 3;
        hits.push(h);
        if strong {
            break;
        }
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score));
    hits
}

/// Envoi multipart décrit par une documentation OpenAPI (FastAPI, Swagger) : (adresse, champs).
pub fn openapi_upload(doc: &str, site: &str) -> Option<Upload> {
    let j = crate::dl::J::parse(doc.trim())?;
    if matches!(j.get("openapi"), crate::dl::J::Null) && matches!(j.get("swagger"), crate::dl::J::Null) {
        return None;
    }
    let crate::dl::J::Obj(paths) = j.get("paths") else { return None };
    let deref = |x: &crate::dl::J| -> crate::dl::J {
        let r = x.get("$ref").s();
        match r.strip_prefix("#/") {
            Some(path) => {
                let mut cur = &j;
                for part in path.split('/') {
                    cur = cur.get(part);
                }
                cur.clone()
            }
            None => x.clone(),
        }
    };
    let mut best: Option<(u8, Upload)> = None;
    for (path, item) in paths {
        let post = item.get("post");
        if matches!(post, crate::dl::J::Null) {
            continue;
        }
        let schema = deref(post.get("requestBody").get("content").get("multipart/form-data").get("schema"));
        let crate::dl::J::Obj(props) = schema.get("properties") else { continue };
        let required: Vec<String> = schema.get("required").arr().iter().map(|x| x.s()).collect();
        let mut up = Upload { method: "POST".into(), url: format!("{}{}", site.trim_end_matches('/'), path), ..Default::default() };
        for (name, p) in props {
            let p = deref(p);
            let binary = p.get("format").s() == "binary" || p.get("items").get("format").s() == "binary" || p.get("contentMediaType").s().contains('/');
            if binary {
                up.fields.push(Field { name: name.clone(), kind: kind_of(name, ""), required: required.contains(name) });
            } else if name == "cat" || name == "category" {
                up.cat = "form".into();
                up.cat_optional = !required.contains(name);
            }
        }
        for prm in post.get("parameters").arr() {
            let prm = deref(prm);
            let n = prm.get("name").s();
            if (n == "cat" || n == "category") && prm.get("in").s() == "query" {
                up.cat = "query".into();
                up.cat_optional = !prm.get("required").b();
            }
        }
        if !up.fields.iter().any(|f| f.kind == "nzb") {
            continue;
        }
        let pl = path.to_lowercase();
        let score = if pl.contains("upload") { 3 } else if pl.contains("nzb") { 2 } else { 1 };
        if best.as_ref().map(|b| score > b.0).unwrap_or(true) {
            best = Some((score, up));
        }
    }
    best.map(|b| b.1)
}

/// Résultat de l'auto-configuration.
#[derive(Clone, Debug, Default)]
pub struct Auto {
    pub idx: IndexerCfg,
    /// (« ✔ » | « ⚠ » | « ✖ » | « ℹ », texte).
    pub lines: Vec<(char, String)>,
    pub requests: usize,
    pub caps: Option<crate::apidef::Caps>,
}

/// Configure un indexer avec seulement l'adresse de son API et la clé : capacités (titre,
/// types de recherche, catégories, groupes indexés), clé vérifiée, envoi Newznab ou adresse
/// d'envoi à part (documentation OpenAPI du site, sinon adresses courantes). Une requête à la
/// fois, `delay_secs` entre deux, `max` au plus.
pub fn autoconfig(base: &IndexerCfg, url: &str, key: &str, delay_secs: u64, max: usize, on: &mut dyn FnMut(&str)) -> Auto {
    let mut out = Auto { idx: base.clone(), ..Default::default() };
    let mut pace = Pace::new(delay_secs, max.max(3));
    let api = crate::upload::endpoint(url);
    let site = origin(&api);
    let key = key.trim().to_string();
    out.idx.api_url = api.clone();
    out.idx.api_key = key.clone();
    let say = |out: &mut Auto, c: char, t: String, on: &mut dyn FnMut(&str)| {
        on(&format!("{c} {t}"));
        out.lines.push((c, t));
    };
    let mask = |s: &str| crate::upload::mask(s, &key);
    // 1. capacités
    let mut newznab = false;
    if pace.wait() {
        on(&format!("… GET {}?t=caps", api));
        let q = if key.is_empty() { format!("{api}?t=caps") } else { format!("{api}?t=caps&apikey={}", crate::upload::encode(&key)) };
        let (body, http) = get(&q);
        match crate::apidef::parse_caps(&body) {
            Ok(c) => {
                newznab = true;
                let modes: Vec<String> = c.modes.iter().map(|m| m.0.clone()).collect();
                say(&mut out, '✔', format!("API Newznab : {}{}, recherche : {}", if c.title.is_empty() { "sans nom".into() } else { c.title.clone() }, if c.limit_max > 0 { format!(", {} résultats au plus", c.limit_max) } else { String::new() }, if modes.is_empty() { "?".into() } else { modes.join(", ") }), on);
                say(&mut out, '✔', format!("{} catégorie(s) lues{}", c.cats.len(), if c.groups.is_empty() { String::new() } else { format!(", {} groupe(s) Usenet indexé(s)", c.groups.len()) }), on);
                out.idx.cat_list = c.cats.clone();
                out.idx.groups = c.groups.clone();
                // numéros de catégorie : cases vides remplies (films et séries SD/HD/UHD, FR,
                // autres contenus) ; un choix déjà fait est gardé
                if !c.cats.is_empty() {
                    let n = crate::upload::fill_categories(&mut out.idx, &c.cats, true);
                    if n > 0 {
                        let ex: Vec<String> = ["film.1080", "tv.1080", "fr.film.1080"]
                            .iter()
                            .filter_map(|k| out.idx.cat_map.iter().find(|(x, _)| x == k).map(|(x, v)| format!("{} → {v}{}", crate::upload::rule_label(x), crate::upload::cat_label(&out.idx, v).map(|l| format!(" ({l})")).unwrap_or_default())))
                            .collect();
                        say(&mut out, '✔', format!("{n} case(s) de catégorie remplie(s){}", if ex.is_empty() { String::new() } else { format!(", ex. {}", ex.join(" · ")) }), on);
                    }
                }
                // groupes exigés : un groupe indexé adapté est ajouté au post s'il le faut
                if !c.groups.is_empty() && out.idx.groups_rule.is_empty() {
                    out.idx.groups_rule = "add".into();
                    say(&mut out, '✔', "groupes Usenet : un de ses groupes, adapté au contenu, sera ajouté au post s'il le faut".into(), on);
                }
                if out.idx.api.kind.is_empty() || out.idx.api.kind == "newznab" {
                    out.idx.api.kind = "newznab".into();
                    let custom: Vec<crate::apidef::Req> = out.idx.api.requests.iter().filter(|r| !["newznab", "capacités"].contains(&r.origin.as_str())).cloned().collect();
                    out.idx.api.requests = crate::apidef::newznab_requests(&c);
                    out.idx.api.requests.extend(custom);
                }
                out.idx.api.caps = c.clone();
                out.caps = Some(c);
            }
            Err(e) => say(&mut out, '⚠', format!("pas d'API Newznab à {api} ({}) : {}", http.map(|h| format!("HTTP {h}")).unwrap_or("pas de réponse".into()), mask(&e)), on),
        }
    }
    // 2. clé (une recherche d'un seul résultat)
    if newznab && !key.is_empty() && pace.wait() {
        on("… GET t=search&q=test&limit=1 (vérification de la clé)");
        let (body, _) = get(&format!("{api}?t=search&q=test&limit=1&apikey={}", crate::upload::encode(&key)));
        match newznab_code(&body) {
            Some(100) => say(&mut out, '✖', "clé API refusée (code 100) : vérifie-la sur le site de l'indexer".into(), on),
            Some(101) => say(&mut out, '✖', "compte suspendu (code 101)".into(), on),
            Some(c @ (500 | 501)) => say(&mut out, '⚠', format!("limite de requêtes atteinte (code {c}) : réessaie plus tard, avec un délai plus long"), on),
            _ => say(&mut out, '✔', "clé acceptée (recherche OK)".into(), on),
        }
    }
    // 3. envoi : fonction nzbadd de l'API ?
    let mut upload: Option<Upload> = None;
    if newznab && pace.wait() {
        on("… GET t=nzbadd sans fichier (la fonction existe-t-elle ?)");
        let q = if key.is_empty() { format!("{api}?t=nzbadd") } else { format!("{api}?t=nzbadd&apikey={}", crate::upload::encode(&key)) };
        let (body, http) = get(&q);
        let h = judge(&format!("{api}?t=nzbadd"), true, http, &body);
        if h.score >= 2 {
            say(&mut out, '✔', format!("envoi par l'API Newznab (t=nzbadd) : {}", h.verdict), on);
            if newznab_code(&body) == Some(102) {
                say(&mut out, '⚠', "le compte n'a pas encore le droit d'upload (code 102)".into(), on);
            }
            upload = Some(Upload { method: "POST".into(), url: api.clone(), newznab: true, fields: vec![Field { name: "file".into(), kind: "nzb".into(), required: true }, Field { name: "nfo".into(), kind: "nfo".into(), required: false }, Field { name: "mediainfo".into(), kind: "mediainfo".into(), required: false }], ..Default::default() });
        } else {
            say(&mut out, 'ℹ', format!("pas d'envoi par t=nzbadd ({}) : recherche d'une adresse d'envoi à part", h.verdict), on);
        }
    }
    // 4. documentation OpenAPI du site (FastAPI, Swagger)
    if upload.is_none() {
        for path in ["/openapi.json", "/api/openapi.json"] {
            if !pace.wait() {
                break;
            }
            on(&format!("… GET {site}{path}"));
            let (body, http) = get(&format!("{site}{path}"));
            if http != Some(200) {
                continue;
            }
            if let Some(u) = openapi_upload(&body, &site) {
                say(&mut out, '✔', format!("documentation OpenAPI : envoi {} {} (champs : {})", u.method, u.url, u.fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ")), on);
                upload = Some(u);
                break;
            }
        }
    }
    // 5. adresses courantes (sans clé)
    if upload.is_none() {
        // t=nzbadd déjà essayé : seules les adresses à part du site sont sondées
        let mut probe_idx = out.idx.clone();
        probe_idx.api_url.clear();
        probe_idx.rest_url = site.clone();
        let hits = probe(&probe_idx, &mut pace, &mut |l| on(l));
        match hits.iter().find(|h| h.score >= 2 && !h.newznab) {
            Some(h) => {
                say(&mut out, '✔', format!("adresse d'envoi trouvée : {} ({})", h.url, h.verdict), on);
                upload = Some(Upload { method: "POST".into(), url: h.url.clone(), fields: vec![Field { name: "file".into(), kind: "nzb".into(), required: true }, Field { name: "nfo".into(), kind: "nfo".into(), required: false }], ..Default::default() });
            }
            None => say(&mut out, '⚠', "adresse d'envoi non trouvée : colle la documentation de l'indexer (page d'upload ou API), elle sera lue".into(), on),
        }
    }
    if let Some(u) = upload {
        let a = Analysis { upload: Some(u.clone()), api: newznab.then(|| api.clone()), site: Some(site.clone()), key: if u.newznab { None } else { Some(("query".into(), "apikey".into())) }, ..Default::default() };
        for d in apply(&mut out.idx, &a) {
            out.lines.push(('ℹ', format!("réglé : {d}")));
        }
        if !u.newznab && out.idx.mode == "rest" && out.idx.rest_key.trim().is_empty() {
            out.lines.push(('ℹ', "même clé pour la recherche et l'envoi".into()));
        }
    }
    out.requests = pace.sent;
    out
}

/// Règle l'envoi sur une adresse trouvée.
pub fn use_hit(idx: &mut IndexerCfg, h: &Hit) -> Vec<String> {
    let mut done = Vec::new();
    if h.newznab {
        idx.mode = "newznab".into();
        done.push("envoi : API Newznab (t=nzbadd)".into());
    } else {
        idx.mode = "rest".into();
        idx.rest_path = h.url.clone();
        idx.rest_url = origin(&h.url);
        if idx.auth != "query" && idx.auth != "header" && idx.auth != "bearer" {
            idx.auth = "query".into();
            idx.auth_name = "apikey".into();
        }
        if idx.auth == "header" && idx.auth_name.trim().eq_ignore_ascii_case("x-api-key") {
            // réglage de départ jamais choisi : la clé dans l'adresse est la plus répandue
            idx.auth = "query".into();
            idx.auth_name = "apikey".into();
        }
        done.push(format!("adresse d'envoi : {} (clé : {})", h.url, crate::apidef::auth_label(&idx.auth, &idx.auth_name)));
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIKI2: &str = r#"How to upload

Use the uploader above, or send the NZB as file with your indexer2 API key. A matching NFO is recommended for a better release name, but it is not required.
Single upload

curl -X POST "https://indexer2.com/v1/upload?apikey=YOUR_API_KEY" \
  -F "file=@Release.Name.nzb;type=application/x-nzb" \
  -F "nfo=@Release.Name.nfo;type=text/plain"

Single upload with category override

curl -X POST "https://indexer2.com/v1/upload?apikey=YOUR_API_KEY&cat=2040" \
  -F "file=@Release.Name.nzb;type=application/x-nzb" \
  -F "nfo=@Release.Name.nfo;type=text/plain"

    Category is optional. Leave it blank and indexer2 will try to detect it automatically.
    NFO is recommended, not required. Use the same filename so it can be paired automatically.
    Adult/XXX is not a separate category right now; leave category blank and the upload will still be accepted.

The API returns a guid. The release page is /account/releases/<guid>.

API keys
Use with `/api?t=search&apikey=…`
"#;

    #[test]
    fn wiki_avec_curl_et_adresse_d_envoi_a_part() {
        let a = analyze(WIKI2);
        let u = a.upload.clone().expect("envoi compris");
        assert_eq!(u.url, "https://indexer2.com/v1/upload");
        assert!(!u.newznab);
        assert_eq!(u.method, "POST");
        assert_eq!(u.cat, "query");
        assert!(u.cat_optional);
        let f: Vec<(&str, &str)> = u.fields.iter().map(|f| (f.name.as_str(), f.kind.as_str())).collect();
        assert_eq!(f, vec![("file", "nzb"), ("nfo", "nfo")]);
        assert_eq!(a.key, Some(("query".into(), "apikey".into())));
        assert_eq!(a.site.as_deref(), Some("https://indexer2.com"));
        assert_eq!(a.api.as_deref(), Some("https://indexer2.com/api"));
        assert_eq!(a.release_page.as_deref(), Some("https://indexer2.com/account/releases/{guid}"));
        assert!(a.notes.iter().any(|n| n.contains("NFO conseillé")) && a.notes.iter().any(|n| n.contains("Adulte")));
        // appliqué à l'indexer réglé en Newznab (cas du refus HTTP 405)
        let mut i = IndexerCfg::new("indexer2");
        i.mode = "newznab".into();
        i.api_url = "https://indexer2.com/api".into();
        i.api_key = "K3Y-ABCDEF".into();
        let done = apply(&mut i, &a);
        assert!(!done.is_empty());
        assert_eq!(i.mode, "rest");
        assert_eq!(crate::upload::rest_target(&i.rest_view()), "https://indexer2.com/v1/upload");
        assert!(i.has_api(), "réglé et prêt");
        assert_eq!((i.auth.as_str(), i.auth_name.as_str()), ("query", "apikey"));
        assert!(!i.send_mi, "pas de champ MediaInfo dans cet envoi");
        assert_eq!(i.api_url, "https://indexer2.com/api", "la recherche reste sur /api");
        // requête d'envoi réellement construite
        let f = crate::upload::Files { nzb: "R.nzb".into(), nfo: Some("R.nfo".into()), nfo_original: true, mediainfo: Some("R.xml".into()), ..Default::default() };
        let args = crate::upload::rest_args(&i.rest_view(), &f, "Film.2020.FRENCH.1080p.BluRay.x264-GRP");
        let url = args.last().unwrap();
        assert!(url.starts_with("https://indexer2.com/v1/upload?apikey=K3Y-ABCDEF"), "{url}");
        assert!(url.contains("cat=2040"), "{url}");
        assert!(args.iter().any(|x| x.starts_with("file=@R.nzb")) && args.iter().any(|x| x.starts_with("nfo=@R.nfo")));
        assert!(!args.iter().any(|x| x.starts_with("mediainfo=")));
    }

    /// Fichier d'exemple du dossier tests\ (non publié) : le test est sauté s'il manque.
    fn sample(name: &str) -> Option<String> {
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(name)).ok()
    }

    #[test]
    fn doc_newznab_complete() {
        let Some(doc3) = sample("api_doc_indexer3.txt") else { return };
        let a = analyze(&doc3);
        assert_eq!(a.api.as_deref(), Some("https://api.indexer3/api"));
        let u = a.upload.clone().expect("nzbadd compris");
        assert!(u.newznab);
        assert_eq!(u.url, "https://api.indexer3/api");
        let names: Vec<&str> = u.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"file") && names.contains(&"nfo") && names.contains(&"mediainfo"), "{names:?}");
        assert!(u.fields.iter().find(|f| f.name == "file").unwrap().required);
        assert_eq!(u.cat, "query");
        assert!(a.functions.iter().any(|f| f == "nzbadd") && a.functions.iter().any(|f| f == "tvsearch"));
        assert!(a.needs.iter().any(|n| n.contains("droit d'upload")));
        assert_eq!(a.limit_max, Some(99));
        let mut i = IndexerCfg::new("indexer3");
        apply(&mut i, &a);
        assert_eq!(i.mode, "newznab");
        assert_eq!(i.api_url, "https://api.indexer3/api");
        assert!(i.send_mi);
    }

    #[test]
    fn refus_compris() {
        let mut i = IndexerCfg::new("indexer2");
        i.mode = "newznab".into();
        i.api_url = "https://indexer2.com/api".into();
        let d = diagnose(&i, "refusé par l'indexer (HTTP 405) : Method Not Allowed").unwrap();
        assert!(d.title.contains("405") && d.fixes[0] == Fix::Probe, "{d:?}");
        let d = diagnose(&i, r#"refusé (HTTP 422) : {"detail":[{"loc":["body","upload"],"msg":"field required","type":"value_error.missing"}]}"#).unwrap();
        assert_eq!(d.fixes[0], Fix::FieldNzb("upload".into()));
        let d = diagnose(&i, r#"<error code="102" description="Insufficient privileges"/>"#).unwrap();
        assert!(d.title.contains("Droits"));
        assert!(diagnose(&i, "réseau : délai dépassé").is_none());
        // sonde : verdicts
        assert_eq!(judge("u", false, Some(405), "").score, 3);
        assert_eq!(judge("u", false, Some(404), "").score, 0);
        assert_eq!(judge("u", false, Some(200), "<html>").score, 0);
        assert_eq!(judge("u", true, Some(200), r#"<error code="203" description="Function not available"/>"#).score, 0);
        assert_eq!(judge("u", true, Some(200), r#"<error code="100" description="Incorrect user credentials"/>"#).score, 2);
        let c = candidates(&i);
        assert!(c.iter().any(|(u, _)| u == "https://indexer2.com/v1/upload"));
        let h = judge("https://indexer2.com/v1/upload", false, Some(405), "");
        use_hit(&mut i, &h);
        assert_eq!(i.mode, "rest");
        assert_eq!(crate::upload::rest_target(&i.rest_view()), "https://indexer2.com/v1/upload");
    }

    #[test]
    fn openapi_et_rythme() {
        let doc = r##"{"openapi":"3.1.0","paths":{"/v1/upload":{"post":{"parameters":[{"name":"apikey","in":"query","required":true},{"name":"cat","in":"query","required":false}],"requestBody":{"content":{"multipart/form-data":{"schema":{"$ref":"#/components/schemas/Body_upload"}}}}}},"/api":{"get":{}}},"components":{"schemas":{"Body_upload":{"type":"object","required":["file"],"properties":{"file":{"type":"string","format":"binary"},"nfo":{"type":"string","format":"binary"}}}}}}"##;
        let u = openapi_upload(doc, "https://indexer2.com").unwrap();
        assert_eq!(u.url, "https://indexer2.com/v1/upload");
        assert_eq!(u.cat, "query");
        assert!(u.cat_optional);
        assert_eq!(u.fields.iter().map(|f| (f.name.as_str(), f.kind.as_str(), f.required)).collect::<Vec<_>>(), vec![("file", "nzb", true), ("nfo", "nfo", false)]);
        // rythme : délai respecté, maximum respecté
        let mut p = Pace::new(1, 2);
        let t = std::time::Instant::now();
        assert!(p.wait() && p.wait());
        assert!(t.elapsed() >= std::time::Duration::from_millis(990));
        assert!(!p.wait(), "maximum atteint");
    }

    /// Indexer type « indexer2 » : recherche Newznab sur /api, envoi sur /v1/upload, pas de
    /// t=nzbadd ni d'OpenAPI. Requêtes comptées, jamais d'envoi de fichier.
    #[test]
    fn autoconfiguration_de_bout_en_bout() {
        let _verrou = crate::prep::test_lock();
        let Some(caps) = sample("caps_indexer3.xml") else { return };
        let caps = caps.as_str();
        static CAPS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        CAPS.get_or_init(|| caps.to_string());
        let (addr, seen) = crate::dl::tests::fake_server(|head, _| {
            if head.contains("t=caps") {
                CAPS.get().unwrap().clone()
            } else if head.contains("t=search") {
                r#"<?xml version="1.0"?><rss><channel><newznab:response offset="0" total="0"/></channel></rss>"#.into()
            } else if head.contains("t=nzbadd") {
                r#"<error code="203" description="Function not available"/>"#.into()
            } else if head.starts_with("POST") && head.contains("/v1/upload") {
                r#"200 {"guid":"9ca52909ba9b","message":"Upload accepted"}"#.into()
            } else if head.contains("/v1/upload") {
                r#"405 {"detail":"Method Not Allowed"}"#.into()
            } else {
                r#"404 {"detail":"Not Found"}"#.into()
            }
        });
        let base = IndexerCfg::new("indexer2");
        let mut log = Vec::new();
        let a = autoconfig(&base, &format!("{addr}/api"), "CLE-12345", 0, 20, &mut |l| log.push(l.to_string()));
        let i = &a.idx;
        assert_eq!(i.api_url, format!("{addr}/api"));
        assert_eq!(i.cat_list.len(), 32, "{log:?}");
        assert!(i.groups.iter().any(|g| g == "alt.binaries.ebook") && i.groups.len() == 19);
        assert_eq!(i.api.caps.limit_max, 100);
        assert_eq!(i.mode, "rest", "{log:?}");
        assert_eq!(crate::upload::rest_target(&i.rest_view()), format!("{addr}/v1/upload"));
        assert!(i.has_api());
        // caps, clé, nzbadd, 2 OpenAPI, puis les adresses jusqu'à /v1/upload (la première)
        let reqs = seen.lock().unwrap().clone();
        assert_eq!(a.requests, reqs.len());
        assert_eq!(reqs.len(), 6, "caps, clé, nzbadd, 2 OpenAPI, /v1/upload");
        assert!(!reqs.iter().any(|r| r.starts_with("POST")), "aucun envoi");
        // la clé ne part qu'à l'API Newznab, jamais aux adresses sondées
        assert!(reqs.iter().filter(|r| r.contains("/v1/upload") || r.contains("openapi")).all(|r| !r.contains("CLE-12345")));
        assert!(log.iter().all(|l| !l.contains("CLE-12345")), "{log:?}");
        // tout est prêt : catégories remplies, groupes ajoutés s'il le faut, recherche Newznab
        // sur /api et envoi sur /v1/upload
        assert!(i.cat_map.iter().any(|(k, _)| k == "film.1080"), "{:?}", i.cat_map);
        assert!(i.cat_map.iter().any(|(k, _)| k.starts_with("tv.")));
        assert_eq!(i.groups_rule, "add");
        assert_eq!(crate::search::method(i).as_deref(), Some("API Newznab"));
        assert!(log.iter().any(|l| l.contains("case(s) de catégorie")), "{log:?}");
        let l = crate::search::check(&crate::prep::Config { indexers: vec![i.clone()], ..Default::default() }, &["Film.2020.FRENCH.1080p.WEB.x264-GRP".into()], &[i.name.clone()]);
        assert!(matches!(l[0].found, crate::search::Found::Absent), "{:?}", l[0].found);
        let d = std::env::temp_dir().join(format!("nzbff_auto_up_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Film.2020.FRENCH.1080p.WEB.x264-GRP.nzb"), "<nzb></nzb>").unwrap();
        let r = crate::upload::upload(&i.rest_view(), "Film.2020.FRENCH.1080p.WEB.x264-GRP", &d).map_err(|e| e.message).unwrap();
        assert_eq!(r.guid.as_deref(), Some("9ca52909ba9b"));
        let post = seen.lock().unwrap().iter().find(|r| r.starts_with("POST")).cloned().unwrap();
        assert!(post.contains("/v1/upload?apikey=CLE-12345"), "{post}");
        assert!(post.contains("cat="), "catégorie envoyée : {post}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn requete_ecrite_a_la_main() {
        let r = req_from_text("https://ix.example/api?t=search&q=Dune%202021&cat=2000&apikey=SECRET99", "SECRET99", "").unwrap();
        assert_eq!((r.method.as_str(), r.url.as_str()), ("GET", "https://ix.example/api"));
        let v: Vec<(&str, &str)> = r.params.iter().map(|p| (p.name.as_str(), p.value.as_str())).collect();
        assert_eq!(v, vec![("t", "search"), ("q", "Dune 2021"), ("cat", "2000"), ("apikey", "{key}")]);
        assert!(r.read && r.label.contains("t=search"));
        // la clé n'est envoyée qu'une fois, même si le profil l'ajoute aussi
        let mut i = IndexerCfg::new("X");
        i.api_url = "https://ix.example/api".into();
        i.api_key = "SECRET99".into();
        let p = crate::apidef::profile(&i);
        let b = crate::apidef::build(&i, &p, &r, &[("release".into(), "Dune".into())]).unwrap();
        let all = crate::apidef::preview(&b, "").join(" ");
        assert_eq!(all.matches("apikey=").count(), 1, "{all}");
        let r = req_from_text("POST /api?t=nzbadd&apikey=YOUR_API_KEY", "", "https://ix.example").unwrap();
        assert_eq!((r.method.as_str(), r.url.as_str()), ("POST", "https://ix.example/api"));
        assert!(!r.read, "un envoi demande confirmation");
        let r = req_from_text(r#"curl -X POST "https://ix.example/v1/upload?apikey=SECRET99&cat=2040" -H "X-Token: SECRET99" -F "file=@R.nzb""#, "SECRET99", "").unwrap();
        assert_eq!(r.method, "POST");
        assert!(r.params.iter().any(|p| p.name == "file" && p.place == "form"));
        assert_eq!(r.headers, vec![("X-Token".to_string(), "{key}".to_string())]);
        assert!(req_from_text("ftp://x", "", "").is_err());
        assert!(req_from_text("   ", "", "").is_err());
    }

    /// Identité réseau : réglage général, propre à l'indexer, et bien envoyée.
    #[test]
    fn user_agent_reglable() {
        let _verrou = crate::prep::test_lock();
        crate::apidef::set_ua_global("app");
        assert!(crate::apidef::ua_for("").unwrap().starts_with("NZBFFToolPro/"));
        let b1 = crate::apidef::ua_for("browser").unwrap();
        assert_eq!(crate::apidef::ua_for("browser").unwrap(), b1, "fixé pour la session");
        assert!(crate::apidef::BROWSER_UAS.contains(&b1.as_str()));
        assert!(crate::apidef::BROWSER_UAS.contains(&crate::apidef::ua_for("rotate").unwrap().as_str()));
        assert_eq!(crate::apidef::ua_for("Mon UA/1.0").as_deref(), Some("Mon UA/1.0"));
        assert!(crate::apidef::ua_for("curl").is_none());
        crate::apidef::set_ua_global("");
        assert!(crate::apidef::ua_for("").is_none(), "par défaut : celui de curl");
        // envoyée par la recherche (en-tête du profil) et par le téléchargement d'un NZB (-A)
        let (url, seen) = crate::dl::tests::fake_server(|head, _| {
            if head.contains("t=get") {
                "200 <?xml version=\"1.0\"?><nzb></nzb>".into()
            } else {
                "200 <rss><channel></channel></rss>".into()
            }
        });
        let mut i = IndexerCfg::new("UA");
        i.active = true;
        i.api_url = format!("{url}/api");
        i.api_key = "K".into();
        i.api.user_agent = "TestUA/7".into();
        let _ = crate::finder::search_one(&i, "x");
        let it = crate::finder::Item { indexer: "UA".into(), title: "R".into(), guid: "g1".into(), ..Default::default() };
        let d = std::env::temp_dir().join(format!("nzbff_ua_{}", std::process::id()));
        crate::finder::fetch_nzb(&i, &it, &d).unwrap();
        let reqs = seen.lock().unwrap().clone();
        assert_eq!(reqs.len(), 2);
        assert!(reqs.iter().all(|r| r.to_lowercase().contains("user-agent: testua/7")), "{reqs:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn decoupe_shell() {
        assert_eq!(words(r#"curl -X POST "a b" -F 'x=@c;type=t'"#), vec!["curl", "-X", "POST", "a b", "-F", "x=@c;type=t"]);
        assert_eq!(origin("https://api.x.co.za/api?t=caps"), "https://api.x.co.za");
    }
}
