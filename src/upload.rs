//! Dépôt du NZB sur un indexer, sans passer par son site :
//! API Newznab (`t=nzbadd`, champs `file`, `nzbname`, `mediainfo`, `nfo`), API REST
//! (`POST /nzb`, mêmes champs, clé en en-tête ou en paramètre) ou commande personnalisée. Les requêtes passent par curl, fourni avec Windows 10 et 11.

use crate::prep::{self, IndexerCfg};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Fichiers d'une release prête à déposer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Files {
    pub nzb: PathBuf,
    /// Rapport MediaInfo XML (le champ « mediainfo » attend la sortie XML officielle).
    pub mediainfo: Option<PathBuf>,
    /// Autre format de rapport (JSON, texte…) : non envoyé comme « mediainfo ».
    pub other_report: Option<PathBuf>,
    pub nfo: Option<PathBuf>,
    /// NFO d'origine de la release (toujours envoyé), et non créé par l'application.
    pub nfo_original: bool,
    /// Rapport BDInfo d'un Blu-ray (repris dans le NFO s'il n'y en a pas d'origine).
    pub bdinfo: Option<PathBuf>,
    /// Release d'une catégorie de contenu : (clé, catégorie Newznab par défaut).
    pub content: Option<(String, String)>,
    /// Préparée comme série (page Série) : catégories des séries, quel que soit le nom.
    pub series: bool,
    /// Catégorie choisie à la main pour un indexer (page de traitement) : (indexer, id).
    pub cat_force: Vec<(String, String)>,
}

/// Le NFO part-il chez cet indexer ? Celui d'origine (ou écrit à la main avant le
/// traitement) toujours ; un NFO créé par l'application (MediaInfo, BDInfo) si l'indexer
/// accepte les NFO créés (Réglages → Indexers → nfo).
pub fn send_nfo(idx: &IndexerCfg, f: &Files) -> bool {
    f.nfo.is_some() && (f.nfo_original || idx.nfo_created)
}

/// Fichiers trouvés dans le dossier d'une release.
pub fn release_files(dir: &Path) -> Option<Files> {
    let mut f = Files::default();
    for p in fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_file()) {
        let name = prep::file_name(&p);
        if name.starts_with('.') {
            continue;
        }
        if name.to_lowercase().ends_with(".bdinfo.txt") {
            f.bdinfo = Some(p);
            continue;
        }
        match p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default().as_str() {
            "nzb" => f.nzb = p,
            "xml" => f.mediainfo = Some(p),
            "nfo" => f.nfo = Some(p),
            "json" | "txt" | "html" => f.other_report = Some(p),
            _ => {}
        }
    }
    f.content = crate::content::read_cat(dir).map(|(k, n, _)| (k, n));
    f.cat_force = crate::ngpost::read_cat_force(dir);
    // d'origine, ou écrit à la main avant le traitement : toujours joint
    let origin = crate::ngpost::nfo_origin(dir);
    f.nfo_original = origin.starts_with("origine") || origin == "modifie";
    // série : marquée à la préparation, sinon épisode nommé dans le rapport MediaInfo
    // (« Complete name : Titre.S01E01… ») — pour les releases préparées avant le marqueur
    f.series = crate::ngpost::is_marked_series(dir) || f.mediainfo.as_ref().or(f.other_report.as_ref()).map(|r| report_names_episode(r)).unwrap_or(false);
    (!f.nzb.as_os_str().is_empty()).then_some(f)
}

/// Le rapport MediaInfo est-il celui d'un épisode ? (nom du fichier analysé : S01E02, 1x02…)
fn report_names_episode(report: &Path) -> bool {
    use std::io::Read;
    let mut buf = vec![0u8; 16 * 1024];
    let n = fs::File::open(report).and_then(|mut f| f.read(&mut buf)).unwrap_or(0);
    let text = String::from_utf8_lossy(&buf[..n]);
    // XML : <CompleteName>…</CompleteName> ou <Complete_name> ; texte : « Complete name : … »
    let name = ["<CompleteName>", "<Complete_name>", "<FileName>"]
        .iter()
        .find_map(|tag| text.find(tag).map(|i| &text[i + tag.len()..]).and_then(|r| r.split('<').next()))
        .or_else(|| text.lines().find(|l| l.trim_start().to_lowercase().starts_with("complete name")).and_then(|l| l.split_once(':').map(|x| x.1)))
        .map(|s| s.trim().rsplit(['/', '\\']).next().unwrap_or("").to_string())
        .unwrap_or_default();
    !name.is_empty() && (crate::series::parse_episode(&name).map(|(s, _)| s.is_some()).unwrap_or(false) || crate::series::is_series_name(&name))
}

/// Catégorie à envoyer pour ces fichiers : celle du contenu (propre à l'indexer, sinon celle
/// par défaut du contenu), sinon film / série selon le nom.
pub fn category_for(idx: &IndexerCfg, release: &str, f: &Files) -> String {
    let none = |c: &str| c.eq_ignore_ascii_case("none") || c.eq_ignore_ascii_case("aucune");
    // choisie à la main pour cet indexer avant le traitement : elle prime
    if let Some((_, c)) = f.cat_force.iter().find(|(i, _)| i.trim().eq_ignore_ascii_case(idx.name.trim())) {
        return if none(c) { String::new() } else { c.trim().to_string() };
    }
    if let Some((key, default)) = &f.content {
        // catégorie propre à cet indexer pour ce contenu (« aucune » : rien n'est envoyé) ;
        // en français : sa catégorie FR d'abord (ex. Books - FR > Ebook)
        let fr = if idx.cat_lang_fr && is_french(release) { idx.content_cat(&format!("fr.{key}")) } else { None };
        if let Some(c) = fr.or_else(|| idx.content_cat(key)) {
            if none(c) {
                return String::new();
            }
            // plateforme dans le nom (PS2, Switch…) : sa catégorie chez cet indexer
            return platform_category(idx, c, release).unwrap_or_else(|| c.to_string());
        }
        // indexer réglé sur « aucune » ou « l'indexer la détecte » : vaut aussi pour les contenus
        let ic = idx.cat.trim();
        if none(ic) || ic == "-1" {
            return category(idx, release);
        }
        if let Some(c) = platform_category(idx, default.trim(), release) {
            return c;
        }
        if !default.trim().is_empty() {
            return default.trim().to_string();
        }
    }
    // pas de résolution dans le nom : celle du rapport MediaInfo
    let tv = f.series || crate::series::is_series_name(release);
    if res_from_name(release).is_none() {
        if let Some(h) = f.mediainfo.as_deref().and_then(report_height) {
            let r = if h >= 2000 { "2160p" } else if h >= 1000 { "1080p" } else if h >= 700 { "720p" } else { "576p" };
            return category_kind(idx, &format!("{release}.{r}"), tv);
        }
    }
    category_kind(idx, release, tv)
}

/// Résolution d'une release pour le tableau des catégories : « sd », « 720 », « 1080 », « uhd ».
pub fn resolution_key(release: &str) -> &'static str {
    res_from_name(release).unwrap_or("1080")
}

/// Résolution écrite dans le nom (ou déduite de la source : DVD, DVDRemux, XviD… = SD).
pub fn res_from_name(release: &str) -> Option<&'static str> {
    let l = release.to_lowercase();
    let t: Vec<&str> = l.split(|c: char| !c.is_alphanumeric()).collect();
    if t.iter().any(|x| ["2160p", "uhd", "4k"].contains(x)) {
        Some("uhd")
    } else if t.contains(&"1080p") || t.contains(&"1080i") {
        Some("1080")
    } else if t.contains(&"720p") {
        Some("720")
    } else if t.iter().any(|x| ["576p", "480p", "576i", "480i", "dvdrip", "sd", "xvid", "divx", "dvd", "dvdremux", "dvdr", "dvd5", "dvd9", "pal", "ntsc", "mpeg2", "vhsrip", "tvrip"].contains(x)) {
        Some("sd")
    } else {
        None
    }
}

/// Hauteur de l'image lue dans un rapport MediaInfo XML (« <Height>576</Height> »).
fn report_height(p: &Path) -> Option<u32> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut buf = vec![0u8; 256 * 1024];
    let n = std::io::Read::read(&mut f, &mut buf).ok()?;
    let t = String::from_utf8_lossy(&buf[..n]);
    let i = t.find("<Height>")? + 8;
    t[i..].chars().take_while(|c| c.is_ascii_digit() || *c == ' ').filter(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

/// Release en français (audio) : FRENCH, TRUEFRENCH, VFF, VFQ, VFI, VF2, MULTi.
pub fn is_french(release: &str) -> bool {
    release.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|x| ["french", "truefrench", "vff", "vfq", "vfi", "vf2", "vof", "multi", "vf"].contains(&x))
}

/// Tableau des catégories rempli d'après la liste de l'indexer (noms « Movies > 1080p »,
/// « TV - FR > HD »…) : (case, numéro). Ce qui n'est pas trouvé reste vide (valeur par défaut).
pub fn suggest_rules(list: &[(String, String)]) -> Vec<(String, String)> {
    let norm = |s: &str| s.to_lowercase().replace(['é', 'è'], "e");
    // famille d'une catégorie : (FR ?, film ?) ; autres langues (DE, ES…) ignorées
    let fam = |name: &str| -> Option<(bool, bool)> {
        let n = norm(name);
        let parent = n.split('>').next().unwrap_or("").trim().to_string();
        let (base, lang) = match parent.split_once(" - ") {
            Some((b, l)) => (b.trim().to_string(), l.trim().to_string()),
            None => (parent.clone(), String::new()),
        };
        let fr = lang == "fr" || lang == "french" || lang == "vf";
        if !lang.is_empty() && !fr {
            return None;
        }
        if ["movies", "films", "film", "movie"].contains(&base.as_str()) {
            Some((fr, true))
        } else if ["tv", "series", "serie", "television"].contains(&base.as_str()) {
            Some((fr, false))
        } else {
            None
        }
    };
    let res_of = |name: &str| -> Option<&'static str> {
        let n = norm(name);
        let sub = n.split('>').nth(1)?.trim().to_string();
        match sub.as_str() {
            "sd" => Some("sd"),
            "720p" => Some("720"),
            "1080p" => Some("1080"),
            "uhd" | "2160p" | "4k" => Some("uhd"),
            "hd" => Some("hd"),
            _ => None,
        }
    };
    let mut out: Vec<(String, String)> = Vec::new();
    let mut hd: Vec<(String, String)> = Vec::new();
    for (id, name) in list {
        let (Some((fr, film)), Some(r)) = (fam(name), res_of(name)) else { continue };
        let key = format!("{}{}.", if fr { "fr." } else { "" }, if film { "film" } else { "tv" });
        if r == "hd" {
            hd.push((key, id.clone()));
        } else {
            out.push((format!("{key}{r}"), id.clone()));
        }
    }
    // « HD » sert pour 720p et 1080p quand l'indexer n'a pas de sous-catégorie à part
    for (key, id) in hd {
        for r in ["720", "1080"] {
            let k = format!("{key}{r}");
            if !out.iter().any(|(x, _)| *x == k) {
                out.push((k, id.clone()));
            }
        }
    }
    out
}

/// Catégories des autres contenus (clés de Réglages → Catégories de contenu) trouvées dans la
/// liste de l'indexer : `clé` → catégorie générale, `fr.clé` → catégorie FR (ex. « Books - FR >
/// Ebook »). Seulement ce que la liste contient vraiment.
pub fn suggest_content(list: &[(String, String)]) -> Vec<(String, String)> {
    let norm = |s: &str| s.to_lowercase().replace(['é', 'è'], "e");
    // (famille, langue, sous-catégorie) : « Books - FR > Ebook » → (books, fr, ebook)
    let parts: Vec<(String, String, String, &String)> = list
        .iter()
        .map(|(id, name)| {
            let n = norm(name);
            let (parent, sub) = match n.split_once('>') {
                Some((p, s)) => (p.trim().to_string(), s.trim().to_string()),
                None => (n.trim().to_string(), String::new()),
            };
            let (base, lang) = match parent.split_once(" - ") {
                Some((b, l)) => (b.trim().to_string(), l.trim().to_string()),
                None => (parent, String::new()),
            };
            let lang = if ["fr", "french", "vf"].contains(&lang.as_str()) { "fr".to_string() } else { lang };
            (base, lang, sub, id)
        })
        .collect();
    // candidats par ordre de préférence : (familles, sous-catégories ; "" = la famille elle-même)
    let table: [(&str, &[&str], &[&str]); 13] = [
        ("xxx", &["xxx", "adult", "adulte"], &[""]),
        ("jeux-pc", &["pc", "games", "jeux"], &["games", "game", "jeux", ""]),
        ("consoles", &["console", "consoles"], &[""]),
        ("logiciels-pc", &["pc", "apps", "software", "logiciels"], &["0day", "iso", "windows", ""]),
        ("logiciels-mac", &["pc", "apps", "mac"], &["mac", "macos", ""]),
        ("musique", &["audio", "music", "musique"], &[""]),
        ("ebooks", &["books", "book", "livres", "ebooks"], &["ebook", "ebooks", ""]),
        ("mangas", &["books", "book", "livres", "comics"], &["comics", "comic", "manga", "bd"]),
        ("animation", &["tv", "anime"], &["anime", "animation", ""]),
        ("education", &["pc", "books", "other"], &["tutorials", "tutorial", "technical", "formation"]),
        ("autre", &["other", "misc", "autre"], &["misc", "other", ""]),
        ("bluray", &["movies", "films", "movie"], &["bluray", "blu-ray", "bd"]),
        ("dvd", &["movies", "films", "movie"], &["dvd", "sd"]),
    ];
    let mut out = Vec::new();
    for (key, bases, subs) in table {
        for lang in ["", "fr"] {
            // famille seule (« Console ») : seulement pour les familles qui n'ont pas de sens
            // ailleurs (« Movies » seul n'est pas « Film Blu-ray »)
            let pick = subs.iter().find_map(|s| {
                bases.iter().find_map(|b| {
                    // « pc » seul n'est ni un jeu ni un logiciel Mac : il faut la sous-catégorie
                    if s.is_empty() && *b == "pc" && key != "logiciels-pc" {
                        return None;
                    }
                    if s.is_empty() && *b == "tv" {
                        return None;
                    }
                    parts.iter().find(|(pb, pl, ps, _)| pb == b && pl == lang && ps == s).map(|p| p.3.clone())
                })
            });
            if let Some(id) = pick {
                out.push((if lang.is_empty() { key.to_string() } else { format!("fr.{key}") }, id));
            }
        }
    }
    out
}

/// Remplit les catégories de l'indexer (films, séries, FR, autres contenus) d'après sa liste.
/// `keep` : les cases déjà choisies sont gardées (seules les vides sont remplies). Renvoie le
/// nombre de cases remplies.
pub fn fill_categories(i: &mut IndexerCfg, list: &[(String, String)], keep: bool) -> usize {
    let rules = suggest_all(list);
    if rules.iter().any(|(x, _)| x.starts_with("fr.")) {
        i.cat_lang_fr = true;
    }
    // « Selon la résolution » : c'est ce tableau qui sert
    if i.cat.trim().is_empty() {
        i.cat = "auto".into();
    }
    if keep {
        let mut n = 0;
        for (k, v) in rules {
            if !i.cat_map.iter().any(|(x, val)| x == &k && !val.trim().is_empty()) {
                i.cat_map.retain(|(x, _)| x != &k);
                i.cat_map.push((k, v));
                n += 1;
            }
        }
        return n;
    }
    let keys: Vec<String> = rules.iter().map(|(k, _)| k.clone()).collect();
    i.cat_map.retain(|(x, _)| !(x.starts_with("film.") || x.starts_with("tv.") || x.starts_with("fr.film.") || x.starts_with("fr.tv.")) && !keys.contains(x));
    let n = rules.len();
    i.cat_map.extend(rules);
    n
}

/// Libellé d'un numéro de catégorie dans la liste de l'indexer (« 2040 » → « Movies > HD »).
pub fn cat_label(i: &IndexerCfg, id: &str) -> Option<String> {
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    i.cat_list.iter().find(|(n, _)| n.trim() == id).map(|(_, l)| l.trim().to_string()).filter(|l| !l.is_empty())
}

/// Libellé d'une case de catégorie : « fr.film.1080 » → « Film FR 1080p ».
pub fn rule_label(key: &str) -> String {
    let (fr, rest) = match key.strip_prefix("fr.") {
        Some(r) => (true, r),
        None => (false, key),
    };
    let (kind, res) = rest.split_once('.').unwrap_or((rest, ""));
    let kind = match kind {
        "film" => "Film",
        "tv" => "Série",
        k => return format!("{}{}", k, if fr { " FR" } else { "" }),
    };
    let res = match res {
        "sd" => "SD",
        "720" => "720p",
        "1080" => "1080p",
        "uhd" => "UHD",
        r => r,
    };
    format!("{kind}{} {res}", if fr { " FR" } else { "" })
}

/// Toutes les catégories déduites de la liste de l'indexer : films et séries par résolution
/// (et FR), puis les autres contenus.
pub fn suggest_all(list: &[(String, String)]) -> Vec<(String, String)> {
    let mut v = suggest_rules(list);
    v.extend(suggest_content(list));
    v
}

/// Liste de catégories importée (voir `parse_category_report`).
#[cfg(test)]
pub fn parse_category_list(text: &str) -> Vec<(String, String)> {
    parse_category_report(text).0
}

/// Lit une liste de catégories sous toutes ses formes et dit ce qui n'a pas été compris :
/// - XML `<categories>` / `<caps>` (réponse de l'API `t=caps`) ;
/// - JSON (`[{"id":1080,"name":"PS2"}]`, avec `subcats` / `children`) ;
/// - liste HTML copiée d'une page d'upload (`<optgroup label="Console">` + `<option value="1080">PS2</option>`) ;
/// - texte : « 1080 Console > PS2 », « 1080 - PS2 », « 1080;Console;PS2 », « 1080,Console,PS2 »,
///   « [1080] PS2 », « | 1080 | Console | PS2 | », « Console > PS2 (1080) », « PS2 - 1080 »,
///   et sous-catégories indentées sous leur famille (« Console » puis «   1080 PS2 »).
///
/// Renvoie (catégories, lignes ignorées).
pub fn parse_category_report(text: &str) -> (Vec<(String, String)>, Vec<String>) {
    let low = text.to_lowercase();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let push = |out: &mut Vec<(String, String)>, id: String, name: String| {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if !id.is_empty() && !name.is_empty() && !out.iter().any(|(i, _)| *i == id) {
            out.push((id, name));
        }
    };
    if low.contains("<category") {
        let body = if low.contains("<caps") { text.to_string() } else { format!("<caps>{text}</caps>") };
        return (parse_caps(&body, None).map(|(_, c)| c).unwrap_or_default(), Vec::new());
    }
    let t = text.trim_start_matches('\u{feff}').trim();
    if t.starts_with('[') || t.starts_with('{') {
        if let Some(j) = crate::dl::J::parse(t) {
            fn walk(j: &crate::dl::J, parent: &str, out: &mut Vec<(String, String)>, depth: usize) {
                use crate::dl::J;
                if depth > 6 {
                    return;
                }
                match j {
                    J::Arr(v) => v.iter().for_each(|x| walk(x, parent, out, depth + 1)),
                    J::Obj(m) => {
                        let id = crate::apidef::jfield(j, "id|cat|category_id|value|key");
                        let name = crate::apidef::jfield(j, "name|title|label|text");
                        let full = if !parent.is_empty() && !name.contains('>') { format!("{parent} > {name}") } else { name.clone() };
                        if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) && !name.is_empty() && !out.iter().any(|(i, _)| *i == id) {
                            out.push((id, full.clone()));
                        }
                        for (k, v) in m {
                            if matches!(v, J::Arr(_) | J::Obj(_)) {
                                let p = if ["subcats", "subcat", "children", "subcategories", "sub", "items"].contains(&k.as_str()) && !name.is_empty() { full.as_str() } else { parent };
                                walk(v, p, out, depth + 1);
                            }
                        }
                    }
                    _ => {}
                }
            }
            walk(&j, "", &mut out, 0);
            if !out.is_empty() {
                return (out, skipped);
            }
        }
    }
    // liste HTML d'une page d'upload
    if low.contains("<option") {
        let mut group = String::new();
        let mut pos = 0;
        while let Some(k) = low[pos..].find('<').map(|k| pos + k) {
            let end = text[k..].find('>').map(|e| k + e).unwrap_or(text.len());
            let tag = &text[k..end];
            let lt = tag.to_lowercase();
            if lt.starts_with("<optgroup") {
                group = attr(tag, "label").map(|l| crate::apidef::unescape(&l)).unwrap_or_default();
            } else if lt.starts_with("</optgroup") {
                group.clear();
            } else if lt.starts_with("<option") {
                let close = low[end..].find("</option").map(|c| end + c).or_else(|| low[end..].find('<').map(|c| end + c)).unwrap_or(text.len());
                let label = crate::apidef::clean_text(&text[(end + 1).min(close)..close]);
                let value = attr(tag, "value").unwrap_or_default();
                let id: String = if value.trim().chars().all(|c| c.is_ascii_digit()) { value.trim().to_string() } else { String::new() };
                let label = label.trim_start_matches(['-', ' ', '\u{a0}']).to_string();
                if id.is_empty() {
                    if !label.is_empty() && !label.to_lowercase().contains("choisi") && !label.to_lowercase().contains("select") {
                        skipped.push(label);
                    }
                } else {
                    let name = if !group.is_empty() && !label.contains('>') { format!("{group} > {label}") } else { label };
                    push(&mut out, id, name);
                }
            }
            pos = end.max(k + 1);
        }
        return (out, skipped);
    }
    // texte : une catégorie par ligne, l'identifiant n'importe où
    let mut parent: Option<(String, String)> = None; // (id, nom) de la famille en cours
    for raw in text.lines() {
        let indented = raw.starts_with([' ', '\t']) || raw.trim_start().starts_with(['-', '*', '•', '└', '├', '>']) && raw.trim_start().len() > 2;
        let line = raw.trim().trim_matches('|').trim();
        if line.is_empty() || line.starts_with('#') || line.chars().all(|c| "-=|:+ ".contains(c)) {
            continue;
        }
        // identifiant : premier nombre isolé de 3 à 6 chiffres
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let mut found: Option<(usize, usize)> = None;
        let mut n = 0;
        while n < chars.len() {
            if chars[n].1.is_ascii_digit() {
                let st = n;
                while n < chars.len() && chars[n].1.is_ascii_digit() {
                    n += 1;
                }
                let before_ok = st == 0 || !chars[st - 1].1.is_alphanumeric();
                let after_ok = n == chars.len() || !chars[n].1.is_alphanumeric();
                if (3..=6).contains(&(n - st)) && before_ok && after_ok {
                    let a = chars[st].0;
                    let b = if n < chars.len() { chars[n].0 } else { line.len() };
                    found = Some((a, b));
                    break;
                }
            } else {
                n += 1;
            }
        }
        let Some((a, b)) = found else {
            // titre de famille sans numéro (« Console ») : ses lignes suivantes en dépendent
            if line.chars().count() <= 40 && !line.contains(['=', '{', '<']) {
                parent = Some((String::new(), line.trim_end_matches(':').trim().to_string()));
            } else {
                skipped.push(line.to_string());
            }
            continue;
        };
        let id = line[a..b].to_string();
        let rest = format!("{} {}", &line[..a], &line[b..]);
        // morceaux : séparateurs de colonnes (; , tab |), puis habillage retiré
        let parts: Vec<String> = rest
            .split([';', '\t', '|', ','])
            .map(|p| p.trim().trim_matches(['[', ']', '(', ')', '-', ':', '=', '.', '–', '—', ' ']).trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        if parts.is_empty() {
            skipped.push(line.to_string());
            continue;
        }
        let mut name = parts.join(" > ");
        // « Console > PS2 » écrit « Console/PS2 » : gardé tel quel ; sous-catégorie sans famille
        let fam = |x: &str| if x.len() == 4 { x[..1].to_string() } else { String::new() };
        let is_family = id.len() == 4 && id.ends_with("000");
        if !name.contains('>') {
            if let Some((pid, pname)) = &parent {
                let same = !pid.is_empty() && fam(pid) == fam(&id) && !fam(&id).is_empty();
                if !is_family && (indented || same || pid.is_empty()) && !pname.is_empty() {
                    name = format!("{pname} > {name}");
                }
            }
        }
        if is_family || (!indented && !name.contains('>') && parent.as_ref().map(|(pid, _)| fam(pid) != fam(&id)).unwrap_or(true)) {
            parent = Some((id.clone(), name.clone()));
        }
        push(&mut out, id, name);
    }
    (out, skipped)
}

// ------------------------------------------------------------- plateformes (jeux, logiciels)

/// Plateforme reconnue dans un mot ou un nom de catégorie : clé commune (« ps2 », « xbox360 »…).
pub fn platform_key(s: &str) -> Option<&'static str> {
    let k: String = s.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    Some(match k.as_str() {
        "ps1" | "psx" | "psone" | "playstation" | "playstation1" => "ps1",
        "ps2" | "playstation2" => "ps2",
        "ps3" | "playstation3" => "ps3",
        "ps4" | "playstation4" => "ps4",
        "ps5" | "playstation5" => "ps5",
        "psp" => "psp",
        "psvita" | "vita" | "psv" => "psvita",
        "xbox" | "xboxoriginal" => "xbox",
        "x360" | "xbox360" | "xb360" => "xbox360",
        "xboxone" | "xone" | "xb1" => "xboxone",
        "xboxseries" | "xboxseriesx" | "xsx" | "xbsx" => "xboxseries",
        "wii" => "wii",
        "wiiu" => "wiiu",
        "wiiware" => "wiiware",
        "switch" | "nsw" | "nintendoswitch" => "switch",
        "gamecube" | "ngc" | "gcn" => "gamecube",
        "n64" | "nintendo64" => "n64",
        "nds" | "ds" | "nintendods" => "nds",
        "3ds" | "n3ds" | "nintendo3ds" => "3ds",
        "gba" | "gameboyadvance" => "gba",
        "gbc" | "gameboycolor" => "gbc",
        "gameboy" | "gb" => "gb",
        "dreamcast" | "dc" => "dreamcast",
        "saturn" => "saturn",
        "megadrive" | "genesis" => "megadrive",
        "snes" | "supernintendo" => "snes",
        "nes" => "nes",
        "retro" => "retro",
        _ => return None,
    })
}

/// Plateformes de console écrites dans un nom de release (« Jeu.PS2.PAL-GRP » → ps2), y
/// compris en deux mots (« Xbox.360 », « Wii.U », « PS.Vita »).
pub fn platforms_in(release: &str) -> Vec<&'static str> {
    let t: Vec<&str> = release.split(|c: char| !c.is_alphanumeric()).filter(|x| !x.is_empty()).collect();
    let mut v: Vec<&'static str> = Vec::new();
    for (n, w) in t.iter().enumerate() {
        // deux mots d'abord (« Xbox 360 » plutôt que « Xbox »)
        let two = t.get(n + 1).and_then(|x| platform_key(&format!("{w}{x}")));
        // mots trop courants pour être pris seuls dans un nom (« DS », « GB », « DC »)
        let one = platform_key(w).filter(|_| !["ds", "gb", "dc", "vita", "retro"].contains(&w.to_lowercase().as_str()));
        if let Some(k) = two.or(one) {
            if !v.contains(&k) {
                v.push(k);
            }
        }
    }
    v
}

/// Catégorie de la plateforme d'une release chez cet indexer (« 1080 Console > PS2 »), d'après
/// sa liste de catégories : de préférence dans la même famille que `base`.
pub fn platform_category(idx: &IndexerCfg, base: &str, release: &str) -> Option<String> {
    let want = platforms_in(release);
    if want.is_empty() || idx.cat_list.is_empty() {
        return None;
    }
    let leaf = |n: &str| n.rsplit('>').next().unwrap_or(n).trim().to_string();
    let parent_of = |n: &str| n.split('>').next().unwrap_or("").trim().to_lowercase();
    let cands: Vec<&(String, String)> = idx.cat_list.iter().filter(|(_, n)| platform_key(&leaf(n)).map(|k| want.contains(&k)).unwrap_or(false)).collect();
    if cands.is_empty() {
        return None;
    }
    let base_name = idx.cat_list.iter().find(|(i, _)| i == base).map(|(_, n)| parent_of(n)).unwrap_or_default();
    let same_family = |id: &str| !base.is_empty() && base.len() == id.len() && base.len() >= 4 && base[..base.len() - 3] == id[..id.len() - 3];
    cands
        .iter()
        .find(|(id, n)| same_family(id) || (!base_name.is_empty() && parent_of(n) == base_name))
        .or_else(|| cands.iter().find(|(_, n)| ["console", "game", "jeu", "jeux"].iter().any(|w| parent_of(n).contains(w))))
        .or(cands.first())
        .map(|(id, _)| id.clone())
}

/// Plateformes proposées pour le nom d'un contenu : la liste courante, plus celles des
/// catégories des indexers visés (« Console > PS2 » → « PS2 »).
pub fn platform_choices(idx: &[&IndexerCfg]) -> Vec<String> {
    let mut v: Vec<String> = crate::content::PLATFORM_CHOICES.iter().map(|s| s.to_string()).collect();
    for i in idx {
        for (_, n) in &i.cat_list {
            let leaf = n.rsplit('>').next().unwrap_or(n).trim();
            if platform_key(leaf).is_some() {
                let short: String = leaf.split_whitespace().collect::<Vec<_>>().join("");
                if !v.iter().any(|x| platform_key(x) == platform_key(&short)) {
                    v.push(short);
                }
            }
        }
    }
    v
}

/// Libellé de la catégorie envoyée (pour l'affichage).
pub fn category_label(c: &str) -> String {
    match c {
        "" => "sans catégorie (l'indexer choisit)".into(),
        "-1" => "catégorie détectée par l'indexer (-1)".into(),
        c => format!("catégorie {c}"),
    }
}

/// Catégorie à envoyer : aucune, détectée par l'indexer, fixe (films / séries), ou selon la
/// résolution lue dans le nom de release. Vide : aucun paramètre de catégorie n'est envoyé.
pub fn category(idx: &IndexerCfg, release: &str) -> String {
    // séries (S01, S01-S05, S01E02, Saison.1…) : catégories TV
    category_kind(idx, release, crate::series::is_series_name(release))
}

/// Catégorie d'une release dont on sait si c'est une série.
pub fn category_kind(idx: &IndexerCfg, release: &str, tv: bool) -> String {
    let c = idx.cat.trim();
    if c.eq_ignore_ascii_case("none") {
        return String::new();
    }
    if !c.is_empty() && !c.eq_ignore_ascii_case("auto") {
        // fixe : une pour les films, une pour les séries (jamais un film pour une série)
        if tv && c != "-1" && !idx.cat_tv_hd.trim().is_empty() {
            return idx.cat_tv_hd.trim().to_string();
        }
        return c.to_string();
    }
    // tableau par résolution (SD / 720p / 1080p / UHD), et catégories FR pour les releases en français
    let kind = if tv { "tv" } else { "film" };
    let res = resolution_key(release);
    if idx.cat_lang_fr && is_french(release) {
        if let Some(c) = idx.cat_rule(&format!("fr.{kind}.{res}")) {
            return c.to_string();
        }
    }
    if let Some(c) = idx.cat_rule(&format!("{kind}.{res}")) {
        return c.to_string();
    }
    let (sd, hd, uhd) = if tv { (&idx.cat_tv_sd, &idx.cat_tv_hd, &idx.cat_tv_uhd) } else { (&idx.cat_sd, &idx.cat_hd, &idx.cat_uhd) };
    let pick = match res {
        "uhd" => uhd,
        "sd" => sd,
        _ => hd,
    };
    pick.trim().to_string()
}

/// Adresse de l'API : accepte l'adresse du site (« https://indexer.example ») ou de l'API.
pub fn endpoint(base: &str) -> String {
    let b = base.trim().trim_end_matches(['?', '&']);
    let b = if b.contains("://") { b.to_string() } else { format!("https://{b}") };
    let path_part = b.splitn(4, '/').nth(3).unwrap_or("");
    if path_part.split(['?', '#']).next().unwrap_or("").trim_end_matches('/').ends_with("api") {
        b
    } else {
        format!("{}/api", b.trim_end_matches('/'))
    }
}

fn with_query(url: &str, params: &[(String, String)], extra: &str) -> String {
    let mut u = url.to_string();
    for (k, v) in params {
        u.push(if u.contains('?') { '&' } else { '?' });
        u.push_str(&format!("{k}={}", encode(v)));
    }
    let extra = extra.trim().trim_start_matches(['?', '&']);
    if !extra.is_empty() {
        u.push(if u.contains('?') { '&' } else { '?' });
        u.push_str(extra);
    }
    u
}

/// Encodage d'une valeur de paramètre d'URL.
pub fn encode(v: &str) -> String {
    v.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// « clé=valeur » par ligne ou séparés par « ; ».
fn pairs(s: &str) -> Vec<(String, String)> {
    s.split(['\n', ';']).filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).filter(|(k, _)| !k.is_empty()).collect()
}

/// Clé API masquée dans tout ce qui est affiché ou journalisé.
pub fn mask(s: &str, key: &str) -> String {
    let k = key.trim();
    if k.len() < 4 {
        return s.to_string();
    }
    s.replace(k, "••••").replace(&encode(k), "••••")
}

fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Arguments curl de l'envoi Newznab (`t=nzbadd`).
pub fn nzbadd_args(idx: &IndexerCfg, f: &Files, release: &str) -> Vec<String> {
    let mut q: Vec<(String, String)> = vec![("t".into(), "nzbadd".into()), ("apikey".into(), idx.api_key.trim().to_string())];
    let cat = category_for(idx, release, f);
    if !cat.is_empty() {
        q.push(("cat".into(), cat));
    }
    if idx.includemeta {
        q.push(("includemeta".into(), "true".into()));
    }
    if idx.dupecheck {
        q.push(("dupecheck".into(), "true".into()));
    }
    let url = with_query(&endpoint(&idx.api_url), &q, &idx.extra_query);
    let mut a: Vec<String> = ["-sS", "--max-time", "900", "-X", "POST", "-w", "\n@@HTTP@@%{http_code}"].iter().map(|s| s.to_string()).collect();
    a.extend(ua_args(idx));
    a.push("-F".into());
    a.push(format!("{}=@{};type=application/x-nzb", idx.field_nzb(), f.nzb.display()));
    // catégorie aussi en champ du formulaire : certains indexers ne lisent qu'elle
    let cat = category_for(idx, release, f);
    if !cat.is_empty() {
        a.push("--form-string".into());
        a.push(format!("cat={cat}"));
    }
    if idx.send_name {
        a.push("--form-string".into());
        a.push(format!("nzbname={release}"));
    }
    if idx.send_mi {
        if let Some(m) = &f.mediainfo {
            a.push("-F".into());
            a.push(format!("{}=@{};type=text/xml", idx.field_mi(), m.display()));
        }
    }
    if send_nfo(idx, f) {
        if let Some(n) = &f.nfo {
            a.push("-F".into());
            a.push(format!("{}=@{};type=text/plain", idx.field_nfo(), n.display()));
        }
    }
    // rapport BDInfo d'un Blu-ray dans son propre champ (réglé par indexer)
    if let (Some(b), field) = (&f.bdinfo, idx.bdinfo_field.trim()) {
        if !field.is_empty() {
            a.push("-F".into());
            a.push(format!("{field}=@{};type=text/plain", b.display()));
        }
    }
    for (k, v) in pairs(&idx.extra_fields) {
        a.push("--form-string".into());
        a.push(format!("{k}={v}"));
    }
    a.push(url);
    a
}

/// Adresse d'envoi REST sans paramètres : adresse complète donnée dans le chemin
/// (« https://indexer.example/v1/upload », souvent différente de l'adresse de l'API de
/// recherche), sinon adresse de base + chemin.
pub fn rest_target(idx: &IndexerCfg) -> String {
    let path = idx.rest_path.trim();
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    let base = idx.api_url.trim().trim_end_matches('/');
    let base = if base.contains("://") { base.to_string() } else { format!("https://{base}") };
    let path = if path.is_empty() { "/nzb".to_string() } else if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
    format!("{base}{path}")
}

/// Adresse complète d'envoi REST (+ clé si elle passe dans l'URL, + catégorie).
pub fn rest_url(idx: &IndexerCfg) -> String {
    rest_url_cat(idx, "")
}

fn rest_url_cat(idx: &IndexerCfg, cat: &str) -> String {
    let mut q: Vec<(String, String)> = Vec::new();
    if idx.auth == "query" {
        q.push((if idx.auth_name.trim().is_empty() { "apikey".into() } else { idx.auth_name.trim().to_string() }, idx.api_key.trim().to_string()));
    }
    // catégorie aussi dans l'adresse : beaucoup d'API d'envoi ne la lisent que là (« ?cat=2040 »)
    if !cat.is_empty() && cat != "-1" {
        q.push(("cat".into(), cat.to_string()));
    }
    with_query(&rest_target(idx), &q, &idx.extra_query)
}

/// En-tête d'authentification REST (vide si la clé passe dans l'URL).
fn rest_auth_header(idx: &IndexerCfg) -> Option<String> {
    let key = idx.api_key.trim();
    match idx.auth.as_str() {
        "bearer" => Some(format!("Authorization: Bearer {key}")),
        "query" => None,
        _ => Some(format!("{}: {key}", if idx.auth_name.trim().is_empty() { "X-API-Key" } else { idx.auth_name.trim() })),
    }
}

/// Arguments curl de l'envoi REST (`POST <base>/nzb`, multipart).
pub fn rest_args(idx: &IndexerCfg, f: &Files, release: &str) -> Vec<String> {
    let mut a: Vec<String> = ["-sS", "--max-time", "900", "-X", "POST", "-w", "\n@@HTTP@@%{http_code}", "-H", "Accept: application/json"].iter().map(|s| s.to_string()).collect();
    a.extend(ua_args(idx));
    if let Some(h) = rest_auth_header(idx) {
        a.push("-H".into());
        a.push(h);
    }
    a.push("-F".into());
    a.push(format!("{}=@{};type=application/x-nzb", idx.field_nzb(), f.nzb.display()));
    let cat = category_for(idx, release, f);
    if !cat.is_empty() {
        a.push("--form-string".into());
        a.push(format!("cat={cat}"));
    }
    if idx.send_name {
        a.push("--form-string".into());
        a.push(format!("nzbname={release}"));
    }
    if idx.send_mi {
        if let Some(m) = &f.mediainfo {
            a.push("-F".into());
            a.push(format!("{}=@{};type=text/xml", idx.field_mi(), m.display()));
        }
    }
    if send_nfo(idx, f) {
        if let Some(n) = &f.nfo {
            a.push("-F".into());
            a.push(format!("{}=@{};type=text/plain", idx.field_nfo(), n.display()));
        }
    }
    // rapport BDInfo d'un Blu-ray dans son propre champ (réglé par indexer)
    if let (Some(b), field) = (&f.bdinfo, idx.bdinfo_field.trim()) {
        if !field.is_empty() {
            a.push("-F".into());
            a.push(format!("{field}=@{};type=text/plain", b.display()));
        }
    }
    for (k, v) in pairs(&idx.extra_fields) {
        a.push("--form-string".into());
        a.push(format!("{k}={v}"));
    }
    a.push(rest_url_cat(idx, &category_for(idx, release, f)));
    a
}

/// Texte d'un champ JSON simple (« "message": "…" »).
fn json_str(body: &str, key: &str) -> Option<String> {
    let k = body.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = body[k..].trim_start().strip_prefix(':')?.trim_start();
    if let Some(r) = rest.strip_prefix('"') {
        let e = r.find('"')?;
        return Some(r[..e].to_string());
    }
    let v: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (!v.is_empty()).then_some(v)
}

/// Échec d'un dépôt. `retry_after` : erreur passagère (trop de requêtes, serveur indisponible,
/// coupure réseau), à retenter après ce délai en secondes.
#[derive(Clone, Debug, PartialEq)]
pub struct UpError {
    pub message: String,
    pub retry_after: Option<u64>,
    /// La requête a pu arriver à l'indexer (délai dépassé, réponse perdue, 502/504…) :
    /// jamais de nouvel envoi automatique, pour ne pas créer de doublon.
    pub uncertain: bool,
}

impl UpError {
    pub fn fatal(m: impl Into<String>) -> UpError {
        UpError { message: m.into(), retry_after: None, uncertain: false }
    }
    /// Erreur sans conséquence côté indexer (rien n'a été reçu) : nouvel essai possible.
    pub fn retry(m: impl Into<String>, secs: u64) -> UpError {
        UpError { message: m.into(), retry_after: Some(secs.clamp(5, 3600)), uncertain: false }
    }
    pub fn uncertain(m: impl Into<String>) -> UpError {
        UpError { message: m.into(), retry_after: None, uncertain: true }
    }
}

impl std::fmt::Display for UpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<String> for UpError {
    fn from(m: String) -> UpError {
        UpError::fatal(m)
    }
}

/// Nombre dans un champ JSON (« "retry_after": 60 »).
fn json_num(body: &str, key: &str) -> Option<u64> {
    json_str(body, key).and_then(|v| v.parse().ok())
}

/// Réponse de l'indexer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub message: String,
    pub duplicate: bool,
    pub guid: Option<String>,
    /// Catégorie attribuée par l'indexer (si sa réponse l'indique).
    pub category: Option<String>,
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let k = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let e = tag[k..].find('"')?;
    Some(tag[k..k + e].to_string())
}

/// Lit la réponse : `<success …/>`, `<error code=… description=…/>`, ou JSON simple.
pub fn parse_reply(body: &str, http: Option<u16>) -> Result<Reply, UpError> {
    let b = body.trim();
    let low = b.to_lowercase();
    if let Some(k) = low.find("<success") {
        let tag = &b[k..b[k..].find('>').map(|e| k + e).unwrap_or(b.len())];
        let guid = attr(tag, "guid").or_else(|| attr(tag, "ID")).or_else(|| attr(tag, "id"));
        let name = attr(tag, "name").unwrap_or_default();
        let category = attr(tag, "categoryID").or_else(|| attr(tag, "categoryid")).or_else(|| attr(tag, "category")).or_else(|| attr(tag, "cat"));
        return Ok(Reply { message: format!("accepté{}", if name.is_empty() { String::new() } else { format!(" ({name})") }), duplicate: false, guid, category });
    }
    if let Some(k) = low.find("<error") {
        let tag = &b[k..b[k..].find('>').map(|e| k + e).unwrap_or(b.len())];
        let code = attr(tag, "code").unwrap_or_default();
        let desc = attr(tag, "description").unwrap_or_else(|| b.chars().take(200).collect());
        if code == "601" || desc.to_lowercase().contains("dup") {
            return Ok(Reply { message: format!("déjà présent sur l'indexer ({desc})"), duplicate: true, guid: None, category: None });
        }
        return Err(UpError::fatal(format!("refusé par l'indexer : {desc}{}", if code.is_empty() { String::new() } else { format!(" (code {code})") })));
    }
    let compact: String = low.chars().filter(|c| !c.is_whitespace()).collect();
    let jmsg = json_str(b, "message").or_else(|| json_str(b, "error")).or_else(|| json_str(b, "detail")).or_else(|| json_str(b, "title"));
    let jguid = json_str(b, "guid").or_else(|| json_str(b, "id")).or_else(|| json_str(b, "uuid"));
    let is_dup = compact.contains("duplicate") || compact.contains("doublon") || compact.contains("alreadyexists");
    if http == Some(409) || (is_dup && http.map(|c| c < 500).unwrap_or(true) && !compact.contains("\"success\":true")) {
        return Ok(Reply { message: format!("déjà présent sur l'indexer{}", jmsg.map(|m| format!(" ({m})")).unwrap_or_default()), duplicate: true, guid: None, category: None });
    }
    // refus sans traitement : trop de requêtes (429), service indisponible (503) → nouvel essai
    if let Some(c @ (429 | 503)) = http {
        let wait = json_num(b, "retry_after").unwrap_or(if c == 429 { 60 } else { 120 });
        let what = if c == 429 { "trop de requêtes" } else { "service momentanément indisponible" };
        return Err(UpError::retry(format!("{what} (HTTP {c}){}", jmsg.map(|m| format!(" : {m}")).unwrap_or_default()), wait));
    }
    // erreur du serveur ou de sa passerelle pendant le traitement : la release a pu être
    // enregistrée quand même (gros NZB traité lentement) → pas de nouvel envoi à l'aveugle
    if let Some(c @ (500 | 502 | 504 | 520..=524)) = http {
        return Err(UpError::uncertain(format!("erreur du serveur de l'indexer (HTTP {c}){}", jmsg.map(|m| format!(" : {m}")).unwrap_or_default())));
    }
    if matches!(http, Some(401) | Some(403)) {
        return Err(UpError::fatal(format!(
            "{} (HTTP {}){}",
            if http == Some(403) { "droits insuffisants sur l'indexer (upload non autorisé pour ce compte ?)" } else { "clé API refusée" },
            http.unwrap_or(0),
            jmsg.clone().map(|m| format!(" : {m}")).unwrap_or_default()
        )));
    }
    if compact.contains("\"success\":true") || compact.contains("\"status\":\"ok\"") || compact.contains("\"status\":\"success\"") {
        return Ok(Reply { message: format!("accepté{}", jmsg.map(|m| format!(" ({m})")).unwrap_or_default()), duplicate: false, guid: jguid, category: None });
    }
    let http_ok = http.map(|c| (200..300).contains(&c)).unwrap_or(true);
    if compact.contains("\"success\":false") || (!http_ok && compact.starts_with('{')) || (http_ok && compact.contains("\"error\":\"")) {
        return Err(UpError::fatal(format!(
            "refusé par l'indexer{} : {}",
            http.map(|c| format!(" (HTTP {c})")).unwrap_or_default(),
            jmsg.unwrap_or_else(|| b.chars().take(300).collect())
        )));
    }
    if http_ok && compact.starts_with('{') {
        let detail: Vec<String> = jmsg.or_else(|| json_str(b, "name")).into_iter().chain(jguid.as_ref().map(|g| format!("guid {g}"))).collect();
        return Ok(Reply { message: format!("accepté{}", if detail.is_empty() { String::new() } else { format!(" ({})", detail.join(", ")) }), duplicate: false, guid: jguid, category: None });
    }
    match http {
        Some(c) if (200..300).contains(&c) => Ok(Reply { message: if b.is_empty() { format!("HTTP {c}") } else { b.chars().take(200).collect() }, duplicate: false, guid: None, category: None }),
        Some(c) => Err(UpError::fatal(format!("HTTP {c} : {}", b.chars().take(300).collect::<String>()))),
        None => Ok(Reply { message: b.chars().take(200).collect(), duplicate: false, guid: None, category: None }),
    }
}

thread_local! {
    /// Détail du dernier dépôt (requête, réponse), lu par l'appelant pour le journal.
    static TRACE: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn trace(line: String) {
    TRACE.with(|t| t.borrow_mut().push(line));
}

/// Détail du dernier dépôt fait par ce fil (vidé à la lecture).
pub fn take_trace() -> Vec<String> {
    TRACE.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

/// Requête lisible (méthode, adresse, en-têtes, champs) à partir des arguments curl, clé masquée.
pub fn describe_args(args: &[String], key: &str) -> Vec<String> {
    let mut method = "GET".to_string();
    let mut url = String::new();
    let mut headers = Vec::new();
    let mut fields = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-X" => method = it.next().cloned().unwrap_or_default(),
            "-H" => {
                if let Some(h) = it.next() {
                    if !h.starts_with("Accept:") {
                        headers.push(mask(h, key));
                    }
                }
            }
            "-F" | "--form-string" => {
                if let Some(v) = it.next() {
                    if method == "GET" {
                        method = "POST".into();
                    }
                    let (k, val) = v.split_once('=').unwrap_or((v.as_str(), ""));
                    // fichier joint : son nom seulement
                    let val = match val.strip_prefix('@') {
                        Some(path) => {
                            let p = Path::new(path.split(';').next().unwrap_or(path));
                            match std::fs::metadata(p) {
                                Ok(m) => format!("fichier {} ({})", prep::file_name(p), prep::human_size(m.len())),
                                Err(_) => format!("fichier {} (introuvable !)", prep::file_name(p)),
                            }
                        }
                        None => val.to_string(),
                    };
                    fields.push(format!("{k} = {}", mask(&val, key)));
                }
            }
            "-w" | "--max-time" => {
                it.next();
            }
            x if x.starts_with("http://") || x.starts_with("https://") => url = hide_apikey(&mask(x, key)),
            _ => {}
        }
    }
    let mut out = vec![format!("{method} {url}")];
    out.extend(headers.into_iter().map(|h| format!("en-tête : {h}")));
    out.extend(fields.into_iter().map(|f| format!("champ : {f}")));
    out
}

/// Valeur du paramètre `apikey=` d'une adresse toujours masquée (même une clé très courte).
fn hide_apikey(url: &str) -> String {
    let Some(i) = url.to_lowercase().find("apikey=") else { return url.to_string() };
    let start = i + "apikey=".len();
    let end = url[start..].find('&').map(|e| start + e).unwrap_or(url.len());
    format!("{}••••{}", &url[..start], &url[end..])
}

/// Valeur entre guillemets pour un fichier de configuration curl (`-K`).
fn curl_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t"))
}

/// Arguments sans adresse ni en-têtes (ils contiennent la clé API) : ceux-ci passent par un
/// fichier de configuration `-K`, jamais par la ligne de commande. Renvoie (arguments, config).
pub fn split_secret_args(args: &[String]) -> (Vec<String>, String) {
    let mut out = Vec::new();
    let mut conf = String::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-H" {
            if let Some(h) = it.next() {
                conf.push_str(&format!("header = {}\n", curl_quote(h)));
            }
        } else if a == "-u" {
            // identifiant:mot de passe (NZBGet) : jamais sur la ligne de commande
            if let Some(u) = it.next() {
                conf.push_str(&format!("user = {}\n", curl_quote(u)));
            }
        } else if a == "--data-raw" {
            // corps d'une requête (peut contenir la clé) : par le fichier de configuration aussi
            if let Some(d) = it.next() {
                conf.push_str(&format!("data-raw = {}\n", curl_quote(d)));
            }
        } else if a.starts_with("http://") || a.starts_with("https://") {
            conf.push_str(&format!("url = {}\n", curl_quote(a)));
        } else {
            out.push(a.clone());
        }
    }
    (out, conf)
}

/// « -A <User-Agent> » propre à cet indexer (vide : le réglage général, ajouté par `run_curl`).
pub(crate) fn ua_args(idx: &IndexerCfg) -> Vec<String> {
    if idx.api.user_agent.trim().is_empty() {
        return Vec::new();
    }
    crate::apidef::ua_for(&idx.api.user_agent).map(|u| vec!["-A".to_string(), u]).unwrap_or_default()
}

pub(crate) fn run_curl(args: &[String]) -> Result<(String, Option<u16>), UpError> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    // identité réseau : le réglage général, si la requête n'en porte pas déjà une
    let with_ua;
    let args = if args.iter().any(|a| a == "-A" || a == "--user-agent" || a.to_lowercase().starts_with("user-agent:")) {
        args
    } else if let Some(ua) = crate::apidef::ua_for("") {
        with_ua = [vec!["-A".to_string(), ua], args.to_vec()].concat();
        &with_ua[..]
    } else {
        args
    };
    let (plain, conf) = split_secret_args(args);
    let dir = crate::app_dir().join("tmp");
    let _ = fs::create_dir_all(&dir);
    let kfile = dir.join(format!("api-{}-{}-{}.curl", prep::now_secs(), std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    fs::write(&kfile, conf).map_err(|e| UpError::fatal(format!("fichier temporaire : {e}")))?;
    let mut cmd = Command::new("curl");
    cmd.args(&plain).arg("-K").arg(&kfile).stdin(Stdio::null());
    hidden(&mut cmd);
    let out = cmd.output();
    let _ = fs::remove_file(&kfile);
    let out = out.map_err(|e| UpError::fatal(format!("curl introuvable ({e}) : il est fourni avec Windows 10 et 11")))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let (body, http) = match text.rsplit_once("@@HTTP@@") {
        Some((b, c)) => (b.to_string(), c.trim().parse::<u16>().ok().filter(|c| *c > 0)),
        None => (text.clone(), None),
    };
    if !out.status.success() && http.is_none() {
        let msg = if err.is_empty() { format!("curl a échoué (code {:?})", out.status.code()) } else { err };
        // 6/7 : rien n'est parti (nom introuvable, connexion impossible) → nouvel essai ;
        // 28 délai dépassé, 52 réponse vide, 55/56 coupure : l'indexer a pu tout recevoir
        return Err(match out.status.code() {
            Some(6) | Some(7) => UpError::retry(format!("réseau : {msg}"), 60),
            Some(28) | Some(52) | Some(55) | Some(56) => UpError::uncertain(format!("réseau : {msg}")),
            _ => UpError::fatal(msg),
        });
    }
    Ok((body, http))
}

/// Envoi avec trace : requête (clé masquée), code HTTP et réponse complète de l'indexer.
fn traced_curl(args: &[String], key: &str) -> Result<(String, Option<u16>), UpError> {
    for l in describe_args(args, key) {
        trace(format!("  → {l}"));
    }
    let t = std::time::Instant::now();
    let r = run_curl(args);
    match &r {
        Ok((body, http)) => {
            let code = http.map(|c| format!("{c} {}", http_reason(c))).unwrap_or_else(|| "?".into());
            let flat: String = body.trim().replace(['\r', '\n'], " ").chars().take(1500).collect();
            trace(format!("  ← HTTP {code} en {:.1} s : {}", t.elapsed().as_secs_f64(), mask(&flat, key)));
            // réponse complète, ligne par ligne (balises XML séparées), pour la lire facilement
            let pretty = body.trim().replace("><", ">\n<");
            let lines: Vec<&str> = pretty.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            // journal détaillé : réponse entière (lignes longues comprises)
            let (max_lines, max_len) = if crate::prep::detail() { (1000, 4000) } else { (40, 300) };
            if lines.len() > 1 {
                for l in lines.iter().take(max_lines) {
                    trace(format!("      │ {}", mask(&l.chars().take(max_len).collect::<String>(), key)));
                }
                if lines.len() > max_lines {
                    trace(format!("      │ … ({} lignes de plus, voir logs\\api-date.log)", lines.len() - max_lines));
                }
            }
        }
        Err(e) => trace(format!("  ← échec après {:.1} s : {}", t.elapsed().as_secs_f64(), mask(&e.message, key))),
    }
    r
}

/// Signification courte d'un code HTTP.
pub fn http_reason(c: u16) -> &'static str {
    match c {
        200 => "OK",
        201 => "créé",
        202 => "accepté",
        204 => "sans contenu",
        301 | 302 | 307 | 308 => "redirection",
        400 => "requête invalide",
        401 => "clé refusée",
        403 => "interdit (droits)",
        404 => "adresse introuvable",
        405 => "méthode refusée",
        409 => "conflit (doublon)",
        413 => "fichier trop gros",
        422 => "données refusées",
        429 => "trop de requêtes",
        500 => "erreur du serveur",
        502 => "passerelle en erreur",
        503 => "service indisponible",
        504 => "délai de la passerelle dépassé",
        520..=530 => "erreur Cloudflare",
        _ => "",
    }
}

/// La release est-elle déjà sur l'indexer ? (recherche Newznab `t=search` sur son nom).
/// None : impossible à vérifier (pas d'API Newznab, erreur).
pub fn find_on_indexer(idx: &IndexerCfg, release: &str) -> Option<bool> {
    if idx.mode != "newznab" || idx.api_url.trim().is_empty() || idx.api_key.trim().is_empty() {
        return None;
    }
    let q = vec![("t".to_string(), "search".to_string()), ("q".to_string(), release.to_string()), ("limit".to_string(), "100".to_string()), ("apikey".to_string(), idx.api_key.trim().to_string())];
    let url = with_query(&endpoint(&idx.api_url), &q, "");
    let args: Vec<String> = vec!["-sS".into(), "--max-time".into(), "60".into(), "-w".into(), "\n@@HTTP@@%{http_code}".into(), url];
    let (body, http) = run_curl(&args).ok()?;
    if !http.map(|c| (200..300).contains(&c)).unwrap_or(true) {
        return None;
    }
    let norm = |s: &str| s.trim().to_lowercase().replace([' ', '_'], ".");
    let want = norm(release);
    let mut found = false;
    let mut rest = body.as_str();
    while let Some(a) = rest.find("<title>") {
        let after = &rest[a + 7..];
        let Some(b) = after.find("</title>") else { break };
        let t = after[..b].trim().trim_start_matches("<![CDATA[").trim_end_matches("]]>");
        if norm(t) == want {
            found = true;
        }
        rest = &after[b..];
    }
    Some(found)
}

/// Dépose une release (dossier contenant NZB, rapport, NFO) sur un indexer.
pub fn upload(idx: &IndexerCfg, release: &str, dir: &Path) -> Result<Reply, UpError> {
    let f = release_files(dir).ok_or_else(|| format!("aucun NZB dans {}", dir.display()))?;
    let key = idx.key_for_mode();
    let res = match idx.mode.as_str() {
        "newznab" => {
            if idx.api_url.trim().is_empty() || idx.api_key.trim().is_empty() {
                return Err(UpError::fatal("adresse ou clé API manquante (Réglages → Indexers)"));
            }
            let args = nzbadd_args(idx, &f, release);
            let (body, http) = traced_curl(&args, &key)?;
            parse_reply(&body, http)
        }
        "rest" => {
            let idx = &idx.rest_view();
            if (idx.api_url.trim().is_empty() && !idx.rest_path.trim().starts_with("http")) || idx.api_key.trim().is_empty() {
                return Err(UpError::fatal("adresse d'envoi ou clé manquante (Réglages → Indexers → l'indexer : « Configurer automatiquement »)"));
            }
            let args = rest_args(idx, &f, release);
            let (body, http) = traced_curl(&args, &key)?;
            parse_reply(&body, http)
        }
        "cmd" => {
            let fill = |s: &str| {
                s.replace("{nzb}", &f.nzb.display().to_string())
                    .replace("{mediainfo}", &f.mediainfo.as_ref().or(f.other_report.as_ref()).map(|p| p.display().to_string()).unwrap_or_default())
                    .replace("{nfo}", &f.nfo.as_ref().map(|p| p.display().to_string()).unwrap_or_default())
                    .replace("{bdinfo}", &f.bdinfo.as_ref().map(|p| p.display().to_string()).unwrap_or_default())
                    .replace("{release}", release)
                    .replace("{apikey}", idx.api_key.trim())
                    .replace("{cat}", &category_for(idx, release, &f))
                    .replace("{api}", &endpoint(&idx.api_url))
            };
            let args: Vec<String> = crate::ngpost::split_args(&idx.cmd).into_iter().map(|a| fill(&a)).collect();
            let Some((prog, rest)) = args.split_first() else { return Err(UpError::fatal("commande vide")) };
            let mut cmd = Command::new(prog);
            cmd.args(rest).stdin(Stdio::null());
            hidden(&mut cmd);
            let out = cmd.output().map_err(|e| UpError::fatal(format!("lancement de « {prog} » impossible : {e}")))?;
            let body = String::from_utf8_lossy(&out.stdout).to_string();
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr).trim().chars().take(300).collect::<String>();
                Err(UpError::fatal(format!("la commande a échoué (code {:?}) : {}", out.status.code(), if err.is_empty() { body.trim().chars().take(300).collect() } else { err })))
            } else {
                parse_reply(&body, None)
            }
        }
        _ => return Err(UpError::fatal("cet indexer est réglé en dépôt manuel (page web)")),
    };
    let mut res = res.map_err(|mut e| {
        e.message = mask(&e.message, &key);
        e
    });
    if let Ok(r) = &mut res {
        r.message = mask(&r.message, &key);
        if (idx.mode == "newznab" || idx.mode == "rest") && idx.send_mi && f.mediainfo.is_none() && f.other_report.is_some() {
            r.message.push_str(" — rapport MediaInfo non joint : l'indexer attend du XML");
        }
    }
    res
}

/// Test de connexion (`t=caps`) : nom du serveur et catégories (id, nom).
/// Test complet d'un indexer : capacités de l'API de recherche (titre, catégories, groupes
/// indexés) et, si l'envoi a son adresse à part, test de cette adresse (sans fichier).
pub fn test_indexer(idx: &IndexerCfg) -> Result<(String, Vec<(String, String)>, Vec<String>), String> {
    let caps = if idx.api_url.trim().is_empty() { None } else { Some(caps_full(idx)) };
    if idx.mode != "rest" {
        return caps.unwrap_or_else(|| Err("adresse de l'API manquante".into()));
    }
    let up = rest_test(&idx.rest_view());
    match (caps, up) {
        (Some(Ok((t, c, g))), Ok(m)) => Ok((format!("{t} · envoi : {m}"), c, g)),
        (Some(Ok((t, c, _))), Err(e)) => Err(format!("recherche OK ({t}, {} catégorie(s)) mais envoi : {e}", c.len())),
        (Some(Err(e)), Ok(m)) => Err(format!("envoi OK ({m}) mais recherche : {e}")),
        (Some(Err(e)), Err(u)) => Err(format!("recherche : {e} · envoi : {u}")),
        (None, r) => r.map(|m| (m, Vec::new(), Vec::new())),
    }
}

/// Capacités Newznab : titre, catégories (id, nom), groupes indexés.
pub fn caps_full(idx: &IndexerCfg) -> Result<(String, Vec<(String, String)>, Vec<String>), String> {
    if idx.api_url.trim().is_empty() {
        return Err("adresse de l'API manquante".into());
    }
    let mut q = vec![("t".to_string(), "caps".to_string())];
    if !idx.api_key.trim().is_empty() {
        q.push(("apikey".into(), idx.api_key.trim().to_string()));
    }
    let url = with_query(&endpoint(&idx.api_url), &q, "");
    let args: Vec<String> = [ua_args(idx), ["-sS", "--max-time", "30", "-w", "\n@@HTTP@@%{http_code}", &url].iter().map(|s| s.to_string()).collect()].concat();
    let (body, http) = run_curl(&args).map_err(|e| mask(&e.message, &idx.api_key))?;
    let (t, c) = parse_caps(&body, http).map_err(|e| mask(&e, &idx.api_key))?;
    Ok((t, c, caps_groups(&body)))
}

#[allow(dead_code)]
pub fn caps(idx: &IndexerCfg) -> Result<(String, Vec<(String, String)>), String> {
    if idx.api_url.trim().is_empty() {
        return Err("adresse de l'API manquante".into());
    }
    let mut q = vec![("t".to_string(), "caps".to_string())];
    if !idx.api_key.trim().is_empty() {
        q.push(("apikey".into(), idx.api_key.trim().to_string()));
    }
    let url = with_query(&endpoint(&idx.api_url), &q, "");
    let args: Vec<String> = [ua_args(idx), ["-sS", "--max-time", "30", "-w", "\n@@HTTP@@%{http_code}", &url].iter().map(|s| s.to_string()).collect()].concat();
    let (body, http) = run_curl(&args).map_err(|e| mask(&e.message, &idx.api_key))?;
    parse_caps(&body, http).map_err(|e| mask(&e, &idx.api_key))
}

/// Test REST sans envoyer de fichier : une clé refusée donne 401/403, une adresse fausse 404 ;
/// une requête simplement incomplète (400/422) prouve que l'adresse et la clé sont bonnes.
pub fn rest_test(idx: &IndexerCfg) -> Result<String, String> {
    let idx = &idx.rest_view();
    if idx.api_url.trim().is_empty() || idx.api_key.trim().is_empty() {
        return Err("adresse ou clé API manquante".into());
    }
    let mut a: Vec<String> = ["-sS", "--max-time", "30", "-X", "POST", "-w", "\n@@HTTP@@%{http_code}", "-H", "Accept: application/json", "-F", "nzbname=test-connexion"].iter().map(|s| s.to_string()).collect();
    if let Some(h) = rest_auth_header(idx) {
        a.push("-H".into());
        a.push(h);
    }
    a.push(rest_url(idx));
    let (body, http) = run_curl(&a).map_err(|e| mask(&e.message, &idx.api_key))?;
    let msg = json_str(&body, "message").or_else(|| json_str(&body, "error")).or_else(|| json_str(&body, "detail")).unwrap_or_default();
    let r = match http {
        Some(400) | Some(415) | Some(422) => Ok(format!("connexion OK, clé acceptée (HTTP {} sans fichier, normal){}", http.unwrap_or(0), if msg.is_empty() { String::new() } else { format!(" : {msg}") })),
        Some(c) if (200..300).contains(&c) => Ok(format!("connexion OK (HTTP {c})")),
        Some(401) | Some(403) => Err(format!("clé API refusée (HTTP {}){}", http.unwrap_or(0), if msg.is_empty() { String::new() } else { format!(" : {msg}") })),
        Some(404) => Err("adresse introuvable (HTTP 404) : vérifie l'adresse de base et le chemin".into()),
        Some(405) => Err("méthode refusée (HTTP 405) : le chemin n'accepte pas d'envoi".into()),
        Some(c) => Err(format!("réponse HTTP {c}{}", if msg.is_empty() { String::new() } else { format!(" : {msg}") })),
        None => Err("pas de réponse".into()),
    };
    r.map_err(|e| mask(&e, &idx.api_key)).map(|m| mask(&m, &idx.api_key))
}

/// Groupes Usenet indexés, lus dans une réponse `t=caps` (`<groups><group name="…"/>`).
pub fn caps_groups(body: &str) -> Vec<String> {
    let low = body.to_lowercase();
    let mut out: Vec<String> = Vec::new();
    let mut pos = 0;
    while let Some(k) = low[pos..].find("<group ").map(|k| pos + k) {
        let end = body[k..].find('>').map(|e| k + e).unwrap_or(body.len());
        if let Some(n) = attr(&body[k..end], "name") {
            let n = n.trim().to_lowercase();
            if n.contains('.') && !out.contains(&n) {
                out.push(n);
            }
        }
        pos = end.max(k + 1);
    }
    out
}

pub fn parse_caps(body: &str, http: Option<u16>) -> Result<(String, Vec<(String, String)>), String> {
    let low = body.to_lowercase();
    if low.contains("<error") {
        return parse_reply(body, http).map(|_| (String::new(), Vec::new())).map_err(|e| e.message);
    }
    if !low.contains("<caps") {
        return Err(match http {
            Some(c) => format!("réponse inattendue (HTTP {c}) : ce n'est pas une API Newznab ?"),
            None => "réponse inattendue : ce n'est pas une API Newznab ?".into(),
        });
    }
    let title = low.find("<server").and_then(|k| {
        let tag = &body[k..body[k..].find('>').map(|e| k + e).unwrap_or(body.len())];
        attr(tag, "title").or_else(|| attr(tag, "strapline"))
    });
    let mut cats = Vec::new();
    let mut parent = String::new();
    let mut pos = 0;
    while let Some(k) = low[pos..].find('<').map(|k| pos + k) {
        let end = body[k..].find('>').map(|e| k + e).unwrap_or(body.len());
        let tag = &body[k..end];
        let lt = tag.to_lowercase();
        if lt.starts_with("<category ") {
            parent = attr(tag, "name").unwrap_or_default();
            if let Some(id) = attr(tag, "id") {
                cats.push((id, parent.clone()));
            }
        } else if lt.starts_with("<subcat ") {
            if let (Some(id), Some(n)) = (attr(tag, "id"), attr(tag, "name")) {
                cats.push((id, format!("{parent} > {n}")));
            }
        }
        pos = end.max(k + 1);
    }
    Ok((title.unwrap_or_else(|| "API Newznab".into()), cats))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Liste complète d'un indexer (exemple fourni, au format t=caps) : tout est rempli —
    /// films et séries par résolution et en FR, et chaque catégorie de contenu.
    #[test]
    fn categorie_choisie_a_la_main() {
        let d = std::env::temp_dir().join(format!("nzbff_catforce_{}", std::process::id()));
        let _ = fs::create_dir_all(&d);
        fs::write(d.join("R.nzb"), "<nzb></nzb>").unwrap();
        fs::write(d.join(crate::ngpost::CATID_FILE), "altHUB\t5342\nAutre\taucune\n").unwrap();
        let f = release_files(&d).unwrap();
        let i = IndexerCfg { cat: "auto".into(), ..IndexerCfg::new("altHUB") };
        assert_eq!(category_for(&i, "Serie.S01.FRENCH.1080p.WEB.x264-GRP", &f), "5342");
        assert_eq!(category_for(&IndexerCfg::new("Autre"), "Film.2020.1080p", &f), "");
        assert_ne!(category_for(&IndexerCfg::new("Trois"), "Film.2020.1080p", &f), "5342");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn import_de_toutes_les_categories() {
        let list = parse_category_list(include_str!("testdata_categories.xml"));
        assert_eq!(list.len(), 134, "chaque famille et chaque sous-catégorie");
        let all = suggest_all(&list);
        let get = |k: &str| all.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str());
        for (k, v) in [
            ("film.sd", "2030"),
            ("film.720", "2041"),
            ("film.1080", "2042"),
            ("film.uhd", "2045"),
            ("tv.1080", "5042"),
            ("fr.film.1080", "2342"),
            ("fr.tv.uhd", "5345"),
            ("xxx", "6000"),
            ("jeux-pc", "4050"),
            ("consoles", "1000"),
            ("logiciels-pc", "4010"),
            ("logiciels-mac", "4030"),
            ("musique", "3000"),
            ("ebooks", "7020"),
            ("fr.ebooks", "7320"),
            ("mangas", "7030"),
            ("fr.mangas", "7330"),
            ("animation", "5070"),
            ("fr.animation", "5370"),
            ("education", "4090"),
            ("autre", "8010"),
            ("bluray", "2050"),
            ("fr.bluray", "2350"),
            ("dvd", "2030"),
            ("fr.dvd", "2330"),
        ] {
            assert_eq!(get(k), Some(v), "{k} : {all:?}");
        }
        // pas de catégorie DE / ES prise
        assert!(!all.iter().any(|(_, v)| v.starts_with("21") || v.starts_with("22") || v.starts_with("51") || v.starts_with("52") || v.starts_with("71")), "{all:?}");
        // un ebook en français part dans la catégorie FR
        let mut i = IndexerCfg { cat_lang_fr: true, ..IndexerCfg::new("altHUB") };
        i.cat_map = all.clone();
        let eb = Files { content: Some(("ebooks".into(), "7020".into())), ..Default::default() };
        assert_eq!(category_for(&i, "Auteur-Titre.2020.FRENCH.EPUB-GRP", &eb), "7320");
        assert_eq!(category_for(&i, "Author-Title.2020.EPUB-GRP", &eb), "7020");
    }

    fn idx() -> IndexerCfg {
        IndexerCfg { mode: "newznab".into(), api_url: "https://monindexer.example".into(), api_key: "ABCD1234".into(), ..IndexerCfg::new("Mon indexer") }
    }

    #[test]
    fn adresses_et_categories() {
        assert_eq!(endpoint("https://monindexer.example"), "https://monindexer.example/api");
        assert_eq!(endpoint("monindexer.example/"), "https://monindexer.example/api");
        assert_eq!(endpoint("https://x.example/api"), "https://x.example/api");
        assert_eq!(endpoint("https://x.example/api/"), "https://x.example/api/");
        assert_eq!(endpoint("https://x.example/newznab/api?"), "https://x.example/newznab/api");
        let i = idx();
        assert_eq!(category(&i, "Film.2020.FRENCH.2160p.WEB.x265"), "2045");
        assert_eq!(category(&i, "Film.2020.FRENCH.1080p.BluRay.x264"), "2040");
        assert_eq!(category(&i, "monfilm.1983.FRENCH.576p.DVDRip"), "2030");
        // DVD remux sans résolution dans le nom : SD
        assert_eq!(category(&i, "film.FRENCH.VOF.DVDRemux.AC3.2.0.MPEG2-LiHDL"), "2030");
        // ni résolution ni source : celle du rapport MediaInfo
        let xml = std::env::temp_dir().join(format!("nzbff-h-{}.xml", std::process::id()));
        std::fs::write(&xml, "<MediaInfo><track type=\"Video\"><Width>720</Width><Height>576</Height></track></MediaInfo>").unwrap();
        let f = Files { mediainfo: Some(xml.clone()), ..Default::default() };
        assert_eq!(category_for(&i, "Vieux.Film.FRENCH-GRP", &f), "2030");
        assert_eq!(category_for(&i, "Vieux.Film.FRENCH.1080p-GRP", &f), "2040", "le nom l'emporte");
        let _ = std::fs::remove_file(&xml);
        assert_eq!(category(&IndexerCfg { cat: "5040".into(), ..i.clone() }, "x.2160p"), "5040");
        // fixe : une série ne part jamais dans la catégorie des films
        let fixe = IndexerCfg { cat: "2040".into(), ..i.clone() };
        assert_eq!(category(&fixe, "Film.2020.FRENCH.1080p.WEB.x264"), "2040");
        assert_eq!(category(&fixe, "Ma.Serie.S01.FRENCH.1080p.H264"), "5040");
        // aucune catégorie : le paramètre n'est pas envoyé
        let aucune = IndexerCfg { cat: "none".into(), ..i.clone() };
        assert_eq!(category(&aucune, "Ma.Serie.S01.FRENCH.1080p.H264"), "");
        let f = Files { nzb: "r.nzb".into(), ..Default::default() };
        assert!(!nzbadd_args(&aucune, &f, "Ma.Serie.S01").iter().any(|a| a.contains("cat=")));
        let jeu = Files { content: Some(("jeux-pc".into(), "4050".into())), ..f.clone() };
        assert_eq!(category_for(&aucune, "Jeu-GRP", &jeu), "", "indexer sans catégorie : vaut aussi pour les contenus");
        assert_eq!(category_for(&i, "Jeu-GRP", &jeu), "4050");
        // préparée comme série (page Série) : catégorie série même si le nom ne le dit pas
        let serie = Files { series: true, ..f.clone() };
        assert_eq!(category_for(&i, "Ma.Serie.FRENCH.1080p.H264", &serie), "5040");
        assert_eq!(category_for(&i, "Ma.Serie.FRENCH.1080p.H264", &f), "2040");
        assert_eq!(category_for(&i, "Ma serieSaison.1.FRENCH.1080p.H264", &f), "5040", "le cas du journal : Saison.1 = série");
        // liste de l'indexer (extrait de celle envoyée) : tableau rempli tout seul
        let xml = "<categories><category id=\"2000\" name=\"Movies\"><subcat id=\"2030\" name=\"SD\"/><subcat id=\"2040\" name=\"HD\"/><subcat id=\"2041\" name=\"720p\"/><subcat id=\"2042\" name=\"1080p\"/><subcat id=\"2045\" name=\"UHD\"/></category><category id=\"2100\" name=\"Movies - DE\"><subcat id=\"2142\" name=\"1080p\"/></category><category id=\"2300\" name=\"Movies - FR\"><subcat id=\"2330\" name=\"SD\"/><subcat id=\"2341\" name=\"720p\"/><subcat id=\"2342\" name=\"1080p\"/><subcat id=\"2345\" name=\"UHD\"/></category><category id=\"5000\" name=\"TV\"><subcat id=\"5030\" name=\"SD\"/><subcat id=\"5040\" name=\"HD\"/><subcat id=\"5041\" name=\"720p\"/><subcat id=\"5042\" name=\"1080p\"/><subcat id=\"5045\" name=\"UHD\"/></category><category id=\"5300\" name=\"TV - FR\"><subcat id=\"5330\" name=\"SD\"/><subcat id=\"5340\" name=\"HD\"/><subcat id=\"5342\" name=\"1080p\"/><subcat id=\"5345\" name=\"UHD\"/></category></categories>";
        let list = parse_category_list(xml);
        assert!(list.contains(&("5342".to_string(), "TV - FR > 1080p".to_string())));
        let rules = suggest_rules(&list);
        let get = |k: &str| rules.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str());
        assert_eq!((get("tv.1080"), get("tv.720"), get("film.1080"), get("fr.tv.1080"), get("fr.tv.720"), get("fr.film.sd")), (Some("5042"), Some("5041"), Some("2042"), Some("5342"), Some("5340"), Some("2330")));
        assert!(!rules.iter().any(|(_, v)| v == "2142"), "pas les catégories allemandes");
        let mut fine = i.clone();
        fine.cat_map = rules.clone();
        fine.cat_list = list.clone();
        assert_eq!(category(&fine, "Ma.Serie.S01.FRENCH.1080p.H264"), "5042", "1080p : 5042, plus 5040");
        assert_eq!(category(&fine, "Ma.Serie.S01.FRENCH.720p.H264"), "5041");
        assert_eq!(category(&fine, "Film.2024.FRENCH.2160p.WEB"), "2045");
        fine.cat_lang_fr = true;
        assert_eq!(category(&fine, "Ma.Serie.S01.FRENCH.1080p.H264"), "5342", "catégorie FR directement");
        assert_eq!(category(&fine, "Show.S01.1080p.WEB.H264"), "5042", "pas en français : catégorie générale");
        assert_eq!(fine.cat_label("5342"), "5342 (TV - FR > 1080p)");
        assert_eq!(parse_category_list("5040 TV > HD\n5342;TV - FR > 1080p\n"), vec![("5040".to_string(), "TV > HD".to_string()), ("5342".to_string(), "TV - FR > 1080p".to_string())]);
        // release préparée avant le marqueur : le rapport MediaInfo nomme un épisode → série
        let d = std::env::temp_dir().join(format!("nzbff-rapport-ep-{}", std::process::id()));
        let _ = fs::create_dir_all(&d);
        fs::write(d.join("X.nzb"), "<nzb/>").unwrap();
        fs::write(d.join("X.xml"), "<MediaInfo><media><track type=\"General\"><CompleteName>D:\\Sortie\\X\\Titre.S01E01.FRENCH.1080p.mkv</CompleteName></track></media></MediaInfo>").unwrap();
        let rf = release_files(&d).unwrap();
        assert!(rf.series);
        assert_eq!(category_for(&i, "Titre.FRENCH.1080p.H264", &rf), "5040");
        fs::write(d.join("X.xml"), "<MediaInfo><CompleteName>D:\\Films\\Mon.Film.2020.FRENCH.1080p.mkv</CompleteName></MediaInfo>").unwrap();
        assert!(!release_files(&d).unwrap().series, "un film reste un film");
        let _ = fs::remove_dir_all(&d);
        let mut perso = i.clone();
        perso.cat_map.push(("jeux-pc".into(), "aucune".into()));
        assert_eq!(category_for(&perso, "Jeu-GRP", &jeu), "");
        // la requête réelle, lisible et clé masquée : catégorie en paramètre et en champ
        let nz = IndexerCfg { mode: "newznab".into(), api_url: "https://monindexer/api/".into(), api_key: "CLE-SECRETE".into(), ..i.clone() };
        let f2 = Files { nzb: "Ma.Serie.S01.FRENCH.1080p.H264.nzb".into(), mediainfo: Some("Ma.Serie.S01.FRENCH.1080p.H264.xml".into()), ..Default::default() };
        let d = describe_args(&nzbadd_args(&nz, &f2, "Ma.Serie.S01.FRENCH.1080p.H264"), "CLE-SECRETE");
        let all = d.join("\n");
        assert!(d[0].starts_with("POST https://monindexer/api/?t=nzbadd&apikey=••••&cat=5040"), "{all}");
        assert!(all.contains("champ : cat = 5040") && all.contains("champ : nzbname = Ma.Serie.S01.FRENCH.1080p.H264") && all.contains("champ : mediainfo = fichier Ma.Serie.S01.FRENCH.1080p.H264.xml"), "{all}");
        assert!(!all.contains("CLE-SECRETE"));
        assert!(describe_args(&nzbadd_args(&IndexerCfg { api_key: "K".into(), ..nz.clone() }, &f2, "R"), "K")[0].contains("apikey=••••&"), "clé courte masquée aussi");
        // catégorie attribuée par l'indexer, lue dans sa réponse
        let r = parse_reply(r#"<?xml version="1.0"?><success ID="42" guid="abc" categoryID="2040" name="Ma.Serie"/>"#, Some(200)).unwrap();
        assert_eq!(r.category.as_deref(), Some("2040"));
    }

    #[test]
    fn requete_nzbadd() {
        let d = std::env::temp_dir().join(format!("nzbff_up_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        for n in ["R.nzb", "R.xml", "R.nfo", ".nzbff-plan"] {
            fs::write(d.join(n), "x").unwrap();
        }
        let f = release_files(&d).unwrap();
        assert!(f.mediainfo.is_some() && f.nfo.is_some());
        let mut i = idx();
        i.send_nfo = true;
        i.extra_query = "private=no".into();
        i.extra_fields = "comment=Posté avec NZB FF Tool Pro".into();
        let a = nzbadd_args(&i, &f, "Film.2020.FRENCH.1080p.BluRay.x264");
        let j = a.join(" ");
        assert!(j.contains(&format!("file=@{};type=application/x-nzb", d.join("R.nzb").display())), "{j}");
        assert!(j.contains("--form-string nzbname=Film.2020.FRENCH.1080p.BluRay.x264"));
        assert!(j.contains("mediainfo=@") && j.contains("nfo=@") && j.contains("comment=Posté avec NZB FF Tool Pro"));
        let url = a.last().unwrap();
        assert_eq!(url, "https://monindexer.example/api?t=nzbadd&apikey=ABCD1234&cat=2040&includemeta=true&dupecheck=true&private=no");
        assert_eq!(mask(url, "ABCD1234"), "https://monindexer.example/api?t=nzbadd&apikey=••••&cat=2040&includemeta=true&dupecheck=true&private=no");
        // options décochées
        i.send_name = false;
        i.send_mi = false;
        i.includemeta = false;
        i.dupecheck = false;
        i.send_nfo = false;
        let j = nzbadd_args(&i, &f, "R").join(" ");
        assert!(!j.contains("nzbname") && !j.contains("mediainfo=") && !j.contains("includemeta") && !j.contains("dupecheck"), "{j}");
        // le NFO de la release part toujours (d'origine, ou créé parce que demandé)
        assert!(j.contains("nfo=@"), "{j}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn reponses() {
        let ok = parse_reply(r#"<?xml version="1.0"?><success ID="1234" guid="563f4128ea6fc7c4c863c1fb8671fe13" categoryID="2040" name="The.File" />"#, Some(200)).unwrap();
        assert_eq!(ok.guid.as_deref(), Some("563f4128ea6fc7c4c863c1fb8671fe13"));
        assert!(!ok.duplicate);
        assert!(parse_reply(r#"<error code="601" description="NZB is a duplicate"/>"#, Some(200)).unwrap().duplicate);
        let e = parse_reply(r#"<error code="100" description="Incorrect user credentials"/>"#, Some(200)).unwrap_err().message;
        assert!(e.contains("Incorrect user credentials") && e.contains("100"));
        assert!(parse_reply("Not found", Some(404)).unwrap_err().message.contains("404"));
        assert!(parse_reply(r#"{"success": true, "id": 5}"#, Some(200)).is_ok());
        assert!(parse_reply(r#"{"success": false, "message": "bad"}"#, Some(200)).is_err());
    }

    #[test]
    fn requete_rest() {
        let d = std::env::temp_dir().join(format!("nzbff_rest_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("R.nzb"), "x").unwrap();
        fs::write(d.join("R.xml"), "x").unwrap();
        let f = release_files(&d).unwrap();
        let mut i = IndexerCfg { mode: "rest".into(), api_url: "https://monindexer.example/api/".into(), api_key: "K3Y-123".into(), cat: "-1".into(), ..IndexerCfg::new("REST") };
        let a = rest_args(&i, &f, "Film.2020.FRENCH.1080p.x264");
        let j = a.join(" ");
        assert_eq!(a.last().unwrap(), "https://monindexer.example/api/nzb");
        assert!(j.contains("-H X-API-Key: K3Y-123") && j.contains("--form-string cat=-1") && j.contains("nzbname=Film.2020.FRENCH.1080p.x264") && j.contains("mediainfo=@"), "{j}");
        i.auth = "bearer".into();
        i.cat = "auto".into();
        let j = rest_args(&i, &f, "Film.2020.FRENCH.2160p.x265").join(" ");
        assert!(j.contains("Authorization: Bearer K3Y-123") && j.contains("cat=2045"), "{j}");
        i.auth = "query".into();
        i.auth_name = "apikey".into();
        i.rest_path = "v2/upload".into();
        assert_eq!(rest_url(&i), "https://monindexer.example/api/v2/upload?apikey=K3Y-123");
        assert!(!rest_args(&i, &f, "R").iter().any(|x| x.contains("Bearer") || x.contains("X-API-Key")));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn reponses_rest() {
        let ok = parse_reply(r#"{"id": 9876, "guid": "a1b2c3", "message": "NZB uploaded"}"#, Some(201)).unwrap();
        assert_eq!(ok.guid.as_deref(), Some("a1b2c3"));
        assert!(parse_reply(r#"{"message": "NZB already exists"}"#, Some(409)).unwrap().duplicate);
        let e = parse_reply(r#"{"message": "Invalid API key"}"#, Some(401)).unwrap_err().message;
        assert!(e.contains("clé API refusée") && e.contains("Invalid API key"), "{e}");
        let e = parse_reply(r#"{"error": "Missing file"}"#, Some(422)).unwrap_err().message;
        assert!(e.contains("Missing file") && e.contains("422"), "{e}");
        assert!(parse_reply("", Some(204)).is_ok());
    }

    /// Réponses documentées de l'API REST d'un indexer (201, 400, 401, 403, 405, 409, 429, 500, 503).
    #[test]
    fn reponses_rest_documentees() {
        let ok = parse_reply(r#"{"release": {"id": 123, "guid": "9f8e7d", "category_id": 2040, "name": "Film.2020.FRENCH.1080p.x264", "magazininfo_id": 0}}"#, Some(201)).unwrap();
        assert_eq!(ok.guid.as_deref(), Some("9f8e7d"));
        assert!(!ok.duplicate && ok.message.contains("Film.2020.FRENCH.1080p.x264"), "{}", ok.message);
        let err = |status: u16, code: &str, msg: &str| format!(r#"{{"error": {{"status": {status}, "code": "{code}", "message": "{msg}"}}}}"#);
        let e = parse_reply(&err(400, "bad_request", "Missing or invalid parameter."), Some(400)).unwrap_err();
        assert!(e.message.contains("Missing or invalid parameter.") && e.retry_after.is_none(), "{e:?}");
        let e = parse_reply(&err(401, "unauthorized", "Invalid API key."), Some(401)).unwrap_err();
        assert!(e.message.contains("clé API refusée") && e.message.contains("Invalid API key."), "{e:?}");
        let e = parse_reply(&err(403, "forbidden", "Admin access required."), Some(403)).unwrap_err();
        assert!(e.message.contains("droits insuffisants") && e.retry_after.is_none(), "{e:?}");
        assert!(parse_reply(&err(405, "method_not_allowed", "Method not allowed."), Some(405)).unwrap_err().message.contains("405"));
        let d = parse_reply(&err(409, "duplicate", "NZB is a duplicate."), Some(409)).unwrap();
        assert!(d.duplicate && d.message.contains("NZB is a duplicate."));
        let r = parse_reply(r#"{"error": {"status": 429, "code": "rate_limited", "message": "Too many requests.", "retry_after": 60}}"#, Some(429)).unwrap_err();
        assert_eq!(r.retry_after, Some(60));
        // 500 : la release a pu être enregistrée → vérification, jamais de renvoi à l'aveugle
        let r = parse_reply(&err(500, "server_error", "Internal server error."), Some(500)).unwrap_err();
        assert!(r.uncertain && r.retry_after.is_none(), "{r:?}");
        let r = parse_reply(&err(503, "service_unavailable", "Upload temporarily unavailable: elasticsearch not ready."), Some(503)).unwrap_err();
        assert!(r.retry_after.is_some() && r.message.contains("indisponible") && r.message.contains("elasticsearch"), "{r:?}");
    }

    #[test]
    fn categories_caps() {
        let caps = r#"<caps><server version="1.0" title="Mon Indexer" /><categories><category id="2000" name="Movies"><subcat id="2030" name="SD"/><subcat id="2040" name="HD"/></category><category id="5000" name="TV"/></categories></caps>"#;
        let (t, c) = parse_caps(caps, Some(200)).unwrap();
        assert_eq!(t, "Mon Indexer");
        assert_eq!(c, vec![("2000".to_string(), "Movies".to_string()), ("2030".into(), "Movies > SD".into()), ("2040".into(), "Movies > HD".into()), ("5000".into(), "TV".into())]);
        assert!(parse_caps("<html>login</html>", Some(200)).is_err());
        assert!(parse_caps(r#"<error code="100" description="Incorrect user credentials"/>"#, Some(200)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn commande_personnalisee() {
        let d = std::env::temp_dir().join(format!("nzbff_cmd_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("R.nzb"), "x").unwrap();
        let rep = std::env::temp_dir().join(format!("nzbff_cmd_rep_{}.json", std::process::id()));
        fs::write(&rep, r#"{"success": true, "id": 7}"#).unwrap();
        let i = IndexerCfg { mode: "cmd".into(), api_key: "SECRETKEY".into(), cmd: format!("cat \"{}\"", rep.display()), ..IndexerCfg::new("REST") };
        let r = upload(&i, "Film.2020.FRENCH.1080p.x264", &d).unwrap();
        assert_eq!(r.message, "accepté");
        let i2 = IndexerCfg { cmd: "sh -c \"echo nope {apikey} {release} >&2; exit 2\"".into(), ..i };
        let e = upload(&i2, "R", &d).unwrap_err().message;
        assert!(e.contains("nope") && !e.contains("SECRETKEY"), "{e}");
        let _ = fs::remove_dir_all(&d);
        let _ = fs::remove_file(&rep);
    }

    #[test]
    fn listes_de_categories_tous_formats() {
        let has = |l: &[(String, String)], id: &str, n: &str| l.iter().any(|(i, x)| i == id && x == n);
        // texte : numéro devant, derrière, colonnes, familles sans numéro, indentation
        let (l, sk) = parse_category_report("Console\n   1010 - NDS\n   PS2 (1080)\n1000 Console\n4000;PC\n4050,PC,Games\n| 5040 | TV | HD |\n[2045] Movies > UHD\nTV - FR > 1080p 5342\nune ligne sans numéro qui est beaucoup trop longue pour être un titre de famille\n");
        assert!(has(&l, "1010", "Console > NDS"), "{l:?}");
        assert!(has(&l, "1080", "Console > PS2"), "{l:?}");
        assert!(has(&l, "1000", "Console"));
        assert!(has(&l, "4050", "PC > Games"), "{l:?}");
        assert!(has(&l, "5040", "TV > HD"), "{l:?}");
        assert!(has(&l, "2045", "Movies > UHD"));
        assert!(has(&l, "5342", "TV - FR > 1080p"), "{l:?}");
        assert_eq!(sk.len(), 1);
        // liste HTML copiée d'une page d'upload
        let (l, _) = parse_category_report(r#"<select name="cat"><option value="">-- Choisir --</option><optgroup label="Console"><option value="1080">PS2</option><option value="1010">NDS</option></optgroup><option value="4050">PC &gt; Jeux</option></select>"#);
        assert_eq!(l, vec![("1080".to_string(), "Console > PS2".to_string()), ("1010".to_string(), "Console > NDS".to_string()), ("4050".to_string(), "PC > Jeux".to_string())]);
        // JSON avec sous-catégories
        let (l, _) = parse_category_report(r#"[{"id":1000,"name":"Console","subcats":[{"id":1080,"name":"PS2"}]}]"#);
        assert!(has(&l, "1000", "Console") && has(&l, "1080", "Console > PS2"), "{l:?}");
        // XML t=caps : inchangé
        assert!(has(&parse_category_report(r#"<caps><categories><category id="1000" name="Console"><subcat id="1080" name="PS2"/></category></categories></caps>"#).0, "1080", "Console > PS2"));
    }

    #[test]
    fn plateformes_et_categories() {
        assert_eq!(platforms_in("Gran.Turismo.4.PAL.PS2-GRP"), vec!["ps2"]);
        assert_eq!(platforms_in("Halo.3.Xbox.360-GRP"), vec!["xbox360"]);
        assert_eq!(platforms_in("Zelda.Wii.U.EUR"), vec!["wiiu"]);
        assert!(platforms_in("Le.Film.2020.1080p.DS").is_empty(), "DS seul : trop courant");
        let mut i = IndexerCfg::new("A");
        i.cat_list = vec![("1000".into(), "Console".into()), ("1080".into(), "Console > PS2".into()), ("1050".into(), "Console > Xbox 360".into()), ("4050".into(), "PC > Games".into())];
        let f = Files { content: Some(("consoles".into(), "1000".into())), ..Default::default() };
        assert_eq!(category_for(&i, "GT4.PAL.PS2-GRP", &f), "1080");
        assert_eq!(category_for(&i, "Halo.3.X360-GRP", &f), "1050");
        assert_eq!(category_for(&i, "Jeu.Inconnu.PAL-GRP", &f), "1000");
        // catégorie propre réglée pour les consoles : affinée par la plateforme quand même
        i.cat_map.push(("consoles".into(), "1000".into()));
        assert_eq!(category_for(&i, "GT4.PAL.PS2-GRP", &f), "1080");
        // choisie à la main pour ce traitement : elle prime
        let forced = Files { cat_force: vec![("A".into(), "1000".into())], ..f.clone() };
        assert_eq!(category_for(&i, "GT4.PAL.PS2-GRP", &forced), "1000");
        let ch = platform_choices(&[&i]);
        assert!(ch.contains(&"PS2".to_string()) && ch.contains(&"Xbox360".to_string()) || ch.iter().any(|c| platform_key(c) == Some("xbox360")));
        // NFO : celui d'origine part toujours, un NFO créé selon l'indexer
        let mut f2 = Files { nfo: Some("x.nfo".into()), ..Default::default() };
        i.nfo_created = false;
        assert!(!send_nfo(&i, &f2));
        f2.nfo_original = true;
        assert!(send_nfo(&i, &f2));
    }

}
