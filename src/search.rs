//! Présence d'une release sur les indexers, avant de poster ou de déposer.
//!
//! La recherche passe par le profil d'API de chaque indexer (`apidef`) : sa requête de
//! vérification (Newznab `t=search`, API JSON, page de recherche…), avec le nom de la release
//! puis ses noms d'origine. Une release est « présente » si un titre correspond exactement
//! (casse, points, espaces et tirets bas ignorés), « proche » si seuls titre, année ou
//! épisode et résolution correspondent.
//!
//! La clé ne passe jamais sur une ligne de commande (fichier de configuration curl `-K`) et
//! est masquée partout où la requête est affichée.

use crate::prep::{self, Config, IndexerCfg};

/// Un résultat de recherche.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hit {
    pub title: String,
    pub link: String,
    pub size: u64,
}

/// Verdict pour une release sur un indexer.
#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    /// Même nom de release.
    Exact(Vec<Hit>),
    /// Même titre, année ou épisode, et résolution, sous un autre nom.
    Similar(Vec<Hit>),
    Absent,
    /// Vérification impossible (pas de recherche réglée, erreur réseau, clé refusée…).
    Unknown(String),
}

impl Found {
    pub fn label(&self) -> String {
        match self {
            Found::Exact(h) => format!("déjà présente ({})", h.first().map(|h| h.title.as_str()).unwrap_or("")),
            Found::Similar(h) => format!("proche : {}", h.iter().take(3).map(|h| h.title.as_str()).collect::<Vec<_>>().join(", ")),
            Found::Absent => "absente".into(),
            Found::Unknown(e) => format!("non vérifiée : {e}"),
        }
    }
}

/// Résultat complet d'une vérification sur un indexer (pour l'affichage et les journaux).
#[derive(Clone, Debug, PartialEq)]
pub struct Lookup {
    pub indexer: String,
    /// Noms cherchés.
    pub queries: Vec<String>,
    /// Requêtes envoyées, clé masquée.
    pub requests: Vec<String>,
    pub http: Option<u16>,
    pub hits: Vec<Hit>,
    pub found: Found,
}

/// Comment cet indexer est interrogé (None : aucune vérification possible).
pub fn method(idx: &IndexerCfg) -> Option<String> {
    let p = crate::apidef::profile(idx);
    let r = p.check_req()?;
    crate::apidef::ready(idx, &p).ok()?;
    let how = match p.kind.as_str() {
        "" | "newznab" => "API Newznab".to_string(),
        "json" => format!("API ({})", r.label),
        "html" => "page de recherche".into(),
        k => k.to_string(),
    };
    Some(if p.preset.is_empty() || p.is_auto() { how } else { format!("{how} · {}", crate::apidef::preset(&p.preset).map(|x| x.name).unwrap_or("")) })
}

/// Pourquoi la vérification est impossible sur cet indexer.
pub fn why_not(idx: &IndexerCfg) -> String {
    let p = crate::apidef::profile(idx);
    if p.check == "off" {
        return "vérification désactivée pour cet indexer".into();
    }
    if p.check_req().is_none() {
        return "aucune requête de recherche dans son profil d'API".into();
    }
    match crate::apidef::ready(idx, &p) {
        Err(e) => e,
        Ok(()) => "aucune recherche réglée".into(),
    }
}

/// Nom à envoyer selon le style : tel quel, ou avec des espaces à la place des points.
fn spaced(q: &str) -> String {
    q.replace(['.', '_'], " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Nom comparable : minuscules, séparateurs unifiés, extension retirée.
pub fn norm(s: &str) -> String {
    let mut t = s.trim().to_lowercase();
    for ext in [".nzb", ".mkv", ".mp4", ".avi", ".ts", ".m2ts", ".iso"] {
        if t.ends_with(ext) {
            t.truncate(t.len() - ext.len());
        }
    }
    let t: String = t.chars().map(|c| if c == ' ' || c == '_' { '.' } else { c }).collect();
    t.split('.').filter(|p| !p.is_empty()).collect::<Vec<_>>().join(".")
}

/// Clé « titre + année (ou épisode) + résolution » pour repérer une release proche.
pub fn rough_key(s: &str) -> Option<String> {
    let n = norm(s);
    let toks: Vec<&str> = n.split(|c| c == '.' || c == '-').filter(|t| !t.is_empty()).collect();
    let is_year = |t: &str| t.len() == 4 && (t.starts_with("19") || t.starts_with("20")) && t.chars().all(|c| c.is_ascii_digit());
    let is_ep = |t: &str| t.starts_with('s') && t.len() >= 3 && t[1..3].chars().all(|c| c.is_ascii_digit());
    let pos = toks.iter().position(|t| is_year(t) || is_ep(t))?;
    if pos == 0 {
        return None;
    }
    let res = toks.iter().find(|t| matches!(**t, "480p" | "576p" | "720p" | "1080p" | "1080i" | "2160p" | "4k")).map(|t| if *t == "4k" { "2160p" } else { t }).unwrap_or("");
    Some(format!("{}|{}|{res}", toks[..=pos].join("."), if is_ep(toks[pos]) { "ep" } else { "y" }))
}

/// Verdict d'après les titres trouvés (ou le texte d'une page HTML) pour ces noms.
pub fn classify(hits: &[Hit], body: &str, names: &[String]) -> Found {
    let wanted: Vec<String> = names.iter().map(|n| norm(n)).filter(|n| !n.is_empty()).collect();
    let exact: Vec<Hit> = hits.iter().filter(|h| wanted.contains(&norm(&h.title))).cloned().collect();
    if !exact.is_empty() {
        return Found::Exact(exact);
    }
    // page HTML sans titres lisibles : le nom apparaît-il dans le texte ?
    if hits.is_empty() && body.trim_start().starts_with('<') && !body.to_lowercase().contains("<rss") {
        let plain = strip_tags(body).to_lowercase();
        // page « aucun résultat » qui rappelle le nom cherché : absente
        let none = ["aucun résultat", "aucun resultat", "no results", "nothing found", "0 results", "0 résultat", "no matches", "rien trouvé"].iter().any(|x| plain.contains(x));
        let text = norm(&plain);
        if let Some(n) = wanted.iter().find(|n| !none && text.contains(n.as_str())) {
            return Found::Exact(vec![Hit { title: n.clone(), ..Default::default() }]);
        }
        return Found::Absent;
    }
    let keys: Vec<String> = names.iter().filter_map(|n| rough_key(n)).collect();
    let similar: Vec<Hit> = hits.iter().filter(|h| rough_key(&h.title).map(|k| keys.contains(&k)).unwrap_or(false)).cloned().collect();
    if similar.is_empty() {
        Found::Absent
    } else {
        Found::Similar(similar)
    }
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut inside = false;
    for c in html.chars() {
        match c {
            '<' => {
                inside = true;
                out.push(' ');
            }
            '>' => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

/// Cherche une release sur un indexer, sous son nom et ses noms d'origine (s'arrête au
/// premier nom trouvé exactement).
pub fn lookup(idx: &IndexerCfg, names: &[String]) -> Lookup {
    let mut l = Lookup { indexer: idx.name.trim().to_string(), queries: Vec::new(), requests: Vec::new(), http: None, hits: Vec::new(), found: Found::Absent };
    let mut uniq: Vec<String> = Vec::new();
    for n in names {
        let n = origin_stem(n.trim()).to_string();
        if !n.is_empty() && !uniq.iter().any(|u| norm(u) == norm(&n)) {
            uniq.push(n);
        }
    }
    let p = crate::apidef::profile(idx);
    let Some(req) = p.check_req().cloned() else {
        l.found = Found::Unknown(why_not(idx));
        return l;
    };
    if let Err(e) = crate::apidef::ready(idx, &p) {
        l.found = Found::Unknown(e);
        return l;
    }
    let mut similar: Vec<Hit> = Vec::new();
    for q in &uniq {
        // « auto » : tel quel, puis avec des espaces si rien n'est revenu
        let tries: Vec<String> = match p.query_style.as_str() {
            "spaces" => vec![spaced(q)],
            "asis" => vec![q.clone()],
            _ if spaced(q) != *q => vec![q.clone(), spaced(q)],
            _ => vec![q.clone()],
        };
        for (n, t) in tries.iter().enumerate() {
            l.queries.push(t.clone());
            let rep = match crate::apidef::send(idx, &p, &req, &[("release".into(), t.clone())]) {
                Ok(r) => r,
                Err(e) => {
                    l.found = Found::Unknown(e);
                    return l;
                }
            };
            l.requests.push(rep.preview.first().cloned().unwrap_or_default());
            l.http = rep.http;
            if let Some(e) = rep.error {
                l.found = Found::Unknown(e);
                return l;
            }
            let hits: Vec<Hit> = rep.rows.iter().map(|r| Hit { title: r.title.clone(), link: if r.link.is_empty() { r.id.clone() } else { r.link.clone() }, size: r.size }).collect();
            let body = if rep.format == "html" { rep.body.clone() } else { String::new() };
            let got = !hits.is_empty();
            match classify(&hits, &body, &uniq) {
                Found::Exact(h) => {
                    l.hits.extend(hits);
                    l.found = Found::Exact(h);
                    return l;
                }
                Found::Similar(h) => similar.extend(h),
                _ => {}
            }
            l.hits.extend(hits);
            // des résultats sont revenus : inutile d'essayer l'autre forme du nom
            if got || n + 1 == tries.len() {
                break;
            }
        }
    }
    l.found = if similar.is_empty() { Found::Absent } else { Found::Similar(similar) };
    l
}

/// Vérifie une release sur chaque indexer visé. Indexers inconnus ignorés.
pub fn check(cfg: &Config, names: &[String], targets: &[String]) -> Vec<Lookup> {
    cfg.indexers
        .iter()
        .filter(|i| i.active && targets.iter().any(|t| t.trim().eq_ignore_ascii_case(i.name.trim())))
        .map(|i| match marked_any(names, i.name.trim()) {
            // uploadée à la main et notée : présente, sans requête
            Some((n, when)) => {
                let hit = Hit { title: format!("{n} (uploadée à la main, notée le {})", fmt_day(when)), ..Default::default() };
                Lookup { indexer: i.name.trim().to_string(), queries: names.to_vec(), requests: Vec::new(), http: None, hits: vec![hit.clone()], found: Found::Exact(vec![hit]) }
            }
            None => lookup(i, names),
        })
        .collect()
}

// ------------------------------------------------------------------ présence notée à la main

fn fmt_day(ts: u64) -> String {
    let l = crate::history::local_time(ts);
    format!("{:02}/{:02}/{}", l.day, l.month, l.year)
}

fn presence_path() -> std::path::PathBuf {
    crate::app_dir().join("nzbff-presence.tsv")
}

/// Releases notées « déjà sur cet indexer » (uploadées à la main) : (nom normalisé, indexer,
/// date, nom tel que noté).
pub fn presence() -> Vec<(String, String, u64, String)> {
    std::fs::read_to_string(presence_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 4).then(|| (f[0].to_string(), f[1].to_string(), f[2].parse().unwrap_or(0), f[3].to_string()))
        })
        .collect()
}

/// Indexers où cette release est notée présente.
pub fn marked(name: &str) -> Vec<String> {
    let n = norm(name);
    presence().into_iter().filter(|p| p.0 == n).map(|p| p.1).collect()
}

/// Un des noms est-il noté présent sur cet indexer ? (nom noté, date)
pub fn marked_any(names: &[String], indexer: &str) -> Option<(String, u64)> {
    let keys: Vec<String> = names.iter().map(|n| norm(n)).collect();
    presence().into_iter().find(|p| keys.contains(&p.0) && p.1.eq_ignore_ascii_case(indexer)).map(|p| (p.3, p.2))
}

/// Note (ou retire) une release comme uploadée à la main sur un indexer.
pub fn mark(name: &str, indexer: &str, on: bool) {
    let n = norm(name);
    if n.is_empty() || indexer.trim().is_empty() {
        return;
    }
    let mut all = presence();
    all.retain(|p| !(p.0 == n && p.1.eq_ignore_ascii_case(indexer.trim())));
    if on {
        all.push((n, indexer.trim().to_string(), crate::prep::now_secs(), name.trim().to_string()));
    }
    let text: String = all.iter().map(|p| format!("{}\t{}\t{}\t{}\n", p.0, p.1.replace(['\t', '\n'], " "), p.2, p.3.replace(['\t', '\n'], " "))).collect();
    let tmp = presence_path().with_extension("tsv.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, presence_path());
    }
}

/// Réponse lisible : balises XML à la ligne, JSON indenté (simplement).
pub fn pretty(body: &str) -> String {
    let t = body.trim();
    if t.starts_with('<') {
        return t.replace("><", ">\n<");
    }
    if t.starts_with('{') || t.starts_with('[') {
        let mut out = String::new();
        let mut depth = 0usize;
        let mut in_str = false;
        let mut esc = false;
        for c in t.chars() {
            if in_str {
                out.push(c);
                match c {
                    _ if esc => esc = false,
                    '\\' => esc = true,
                    '"' => in_str = false,
                    _ => {}
                }
                continue;
            }
            match c {
                '"' => {
                    in_str = true;
                    out.push(c);
                }
                '{' | '[' => {
                    depth += 1;
                    out.push(c);
                    out.push('\n');
                    out.push_str(&"  ".repeat(depth));
                }
                '}' | ']' => {
                    depth = depth.saturating_sub(1);
                    out.push('\n');
                    out.push_str(&"  ".repeat(depth));
                    out.push(c);
                }
                ',' => {
                    out.push(c);
                    out.push('\n');
                    out.push_str(&"  ".repeat(depth));
                }
                ':' => out.push_str(": "),
                c if c.is_whitespace() => {}
                c => out.push(c),
            }
        }
        return out;
    }
    t.to_string()
}

/// Masque la clé partout (même courte) et la valeur de tout paramètre d'adresse qui en porte
/// une (apikey, api_key, key, token, passkey) : rien de secret à l'écran ni dans les journaux.
pub fn mask(s: &str, key: &str) -> String {
    let mut out = crate::upload::mask(s, key);
    let k = key.trim();
    if !k.is_empty() && k.len() < 4 {
        // clé très courte : seulement là où elle est une valeur de paramètre
        for sep in ["=", ": "] {
            out = out.replace(&format!("{sep}{k}"), &format!("{sep}••••"));
        }
    }
    let low = out.to_lowercase();
    let mut res = String::with_capacity(out.len());
    let mut i = 0;
    let b = out.as_bytes();
    while i < out.len() {
        let hit = ["apikey=", "api_key=", "key=", "token=", "passkey="].iter().find(|n| {
            low[i..].starts_with(*n) && (i == 0 || matches!(b[i - 1], b'?' | b'&' | b' ' | b'"'))
        });
        if let Some(n) = hit {
            res.push_str(&out[i..i + n.len()]);
            i += n.len();
            let end = out[i..].find(|c: char| c == '&' || c == '"' || c == '\'' || c.is_whitespace() || c == '<').map(|e| i + e).unwrap_or(out.len());
            if end > i {
                res.push_str("••••");
            }
            i = end;
        } else {
            let c = out[i..].chars().next().unwrap();
            res.push(c);
            i += c.len_utf8();
        }
    }
    res
}

/// Nom d'origine sans extension de fichier connue (« Film.2020.1080p.mkv » → « Film.2020.1080p ») ;
/// un nom de dossier à points garde tout (« Film.2020.1080p » reste entier).
pub fn origin_stem(name: &str) -> &str {
    const EXT: &[&str] = &["mkv", "mp4", "m4v", "avi", "ts", "m2ts", "mts", "iso", "wmv", "mov", "mpg", "mpeg", "webm", "nzb", "rar", "zip", "7z", "flac", "mp3", "epub", "pdf"];
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && EXT.iter().any(|e| e.eq_ignore_ascii_case(ext)) => stem,
        _ => name,
    }
}

/// Noms cherchés pour une release : son nom (renommé ou non), puis ses noms d'origine.
pub fn names_for(release: &str, origins: &[String]) -> Vec<String> {
    let mut v = vec![release.to_string()];
    for o in origins {
        let o = prep::clean_name(origin_stem(o.trim()));
        if !o.is_empty() && norm(&o) != norm(release) && !v.iter().any(|x| norm(x) == norm(&o)) {
            v.push(o);
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noms_comparables() {
        assert_eq!(norm("Le Film 2020 FRENCH 1080p_WEB.mkv"), "le.film.2020.french.1080p.web");
        assert_eq!(norm("Le.Film.2020.FRENCH.1080p.WEB-GRP.nzb"), "le.film.2020.french.1080p.web-grp");
        assert_eq!(rough_key("Le.Film.2020.MULTi.1080p.BluRay.x264-AAA"), rough_key("Le Film 2020 FRENCH 1080p WEB H264-BBB"));
        assert_ne!(rough_key("Le.Film.2020.1080p-A"), rough_key("Le.Film.2020.2160p-A"));
        assert_eq!(rough_key("Serie.S01E02.720p-A").unwrap(), "serie.s01e02|ep|720p");
        assert_eq!(rough_key("2020.Le.Film"), None);
        assert_eq!(names_for("Rel-A", &["rel-a.mkv".into(), "Original.Name".into()]), vec!["Rel-A".to_string(), "Original.Name".to_string()]);
        assert_eq!(names_for("Le.Film.2020.FRENCH.1080p.WEB-GRP", &["le film 2020 vf.mkv".into(), "Le.Film.2020.1080p".into()]), vec!["Le.Film.2020.FRENCH.1080p.WEB-GRP".to_string(), "le.film.2020.vf".to_string(), "Le.Film.2020.1080p".to_string()]);
    }

    #[test]
    fn verdicts() {
        let h = vec![Hit { title: "Le.Film.2020.FRENCH.1080p.WEB-GRP".into(), ..Default::default() }, Hit { title: "Le Film 2020 MULTi 1080p BluRay-ZZZ".into(), ..Default::default() }];
        assert!(matches!(classify(&h, "", &["le.film.2020.french.1080p.web-grp".into()]), Found::Exact(_)));
        assert!(matches!(classify(&h, "", &["Le.Film.2020.TRUEFRENCH.1080p.HDLight-X".into()]), Found::Similar(_)));
        assert_eq!(classify(&h, "", &["Autre.2021.1080p-X".into()]), Found::Absent);
        let html = "<html><body><td><a>Le.Film.2020.FRENCH.1080p.WEB-GRP</a></td></body></html>";
        assert!(matches!(classify(&[], html, &["Le Film 2020 FRENCH 1080p WEB-GRP".into()]), Found::Exact(_)));
        assert_eq!(classify(&[], html, &["Autre-GRP".into()]), Found::Absent);
        // page « aucun résultat » qui rappelle le nom cherché
        let none = "<html><h1>Le.Film.2020.FRENCH.1080p.WEB-GRP</h1><p>Aucun résultat</p></html>";
        assert_eq!(classify(&[], none, &["Le.Film.2020.FRENCH.1080p.WEB-GRP".into()]), Found::Absent);
        assert_eq!(mask("https://x/api?t=search&apikey=KEY&q=a", "KEY"), "https://x/api?t=search&apikey=••••&q=a");
        assert_eq!(mask("https://x/s?token=abc&monkey=1", ""), "https://x/s?token=••••&monkey=1");
        assert_eq!(mask("Le Film été", "zz"), "Le Film été");
    }

    /// Profil automatique (Newznab du dépôt) : nom tel quel, puis avec des espaces ; profil
    /// désactivé ; indexer sans adresse.
    #[test]
    fn recherche_par_profil() {
        let _g = prep::test_lock();
        let (addr, seen) = crate::dl::tests::fake_server(|head, _| {
            if head.contains("q=Le%20Film%202020%20FRENCH") {
                r#"<?xml version="1.0"?><rss><channel><item><title>Le.Film.2020.FRENCH.1080p.WEB-GRP</title></item></channel></rss>"#.to_string()
            } else {
                r#"<?xml version="1.0"?><rss><channel></channel></rss>"#.to_string()
            }
        });
        let mut i = IndexerCfg::new("A");
        i.api_url = format!("{addr}/api");
        i.api_key = "SECRET123".into();
        assert_eq!(method(&i).as_deref(), Some("API Newznab"));
        let l = lookup(&i, &["Le.Film.2020.FRENCH.1080p.WEB-GRP".into()]);
        assert!(matches!(l.found, Found::Exact(_)), "{l:?}");
        assert_eq!(l.queries, vec!["Le.Film.2020.FRENCH.1080p.WEB-GRP".to_string(), "Le Film 2020 FRENCH 1080p WEB-GRP".to_string()]);
        assert!(l.requests.iter().all(|r| !r.contains("SECRET123") && r.contains("apikey=••••")), "{:?}", l.requests);
        assert!(seen.lock().unwrap().iter().any(|r| r.contains("apikey=SECRET123") && r.contains("t=search")));
        i.api.check = "off".into();
        assert!(method(&i).is_none());
        assert!(matches!(lookup(&i, &["A".into()]).found, Found::Unknown(_)));
        assert!(matches!(lookup(&IndexerCfg::new("Manuel"), &["A".into()]).found, Found::Unknown(_)));
    }

    /// « Uploadée à la main » : présente sans requête, sous le nom renommé ou d'origine.
    #[test]
    fn presence_notee_a_la_main() {
        let _verrou = crate::prep::test_lock();
        let rel = format!("Test.Presence.{}.2020.1080p.WEB-X", std::process::id());
        mark(&rel, "IdxMain", true);
        assert_eq!(marked(&rel.replace('.', " ")), vec!["IdxMain".to_string()]);
        assert!(marked_any(&["autre".into(), rel.clone()], "idxmain").is_some());
        let mut i = crate::prep::IndexerCfg::new("IdxMain");
        i.active = true;
        let cfg = crate::prep::Config { indexers: vec![i], ..Default::default() };
        let l = check(&cfg, &["Nom.Original".into(), rel.clone()], &["IdxMain".into()]);
        assert!(matches!(&l[0].found, Found::Exact(h) if h[0].title.contains("à la main")), "{:?}", l[0].found);
        assert!(l[0].requests.is_empty(), "aucune requête envoyée");
        mark(&rel, "IdxMain", false);
        assert!(marked(&rel).is_empty());
    }

}
