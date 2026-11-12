//! Recherche sur tous les indexers à la fois (à la Prowlarr, en plus simple).
//!
//! Chaque indexer actif qui sait chercher est interrogé par son profil d'API (`apidef`) : même
//! requête que la vérification avant post, donc même clé (jamais sur une ligne de commande),
//! même limite de requêtes par minute. Les résultats sont regroupés par nom de release : pour
//! chacune on voit sur quels indexers elle est, et surtout où elle manque, pour la
//! télécharger puis la reposter là.

use crate::prep::{Config, IndexerCfg};
use crate::search::norm;
use std::path::{Path, PathBuf};

/// Un résultat d'un indexer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub indexer: String,
    pub title: String,
    pub size: u64,
    /// Date de publication (secondes Unix ; 0 : inconnue).
    pub posted: u64,
    pub cat: String,
    /// Page de la release sur l'indexer.
    pub page: String,
    /// Identifiant Newznab (guid).
    pub guid: String,
    /// Lien direct du NZB, s'il est donné.
    pub nzb: String,
}

/// Une release, telle que trouvée sur un ou plusieurs indexers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub key: String,
    pub title: String,
    pub size: u64,
    pub posted: u64,
    pub family: &'static str,
    pub on: Vec<Item>,
}

impl Group {
    pub fn has(&self, indexer: &str) -> bool {
        self.on.iter().any(|i| i.indexer.eq_ignore_ascii_case(indexer))
    }
    pub fn item(&self, indexer: &str) -> Option<&Item> {
        self.on.iter().find(|i| i.indexer.eq_ignore_ascii_case(indexer))
    }
}

/// Bilan de la recherche sur un indexer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub indexer: String,
    pub count: usize,
    pub error: Option<String>,
    pub http: Option<u16>,
    pub secs: f64,
    /// Requête envoyée, clé masquée.
    pub request: String,
}

/// Familles proposées au filtre : (clé, libellé).
pub const FAMILIES: [(&str, &str); 6] = [("", "Tout"), ("movies", "Films"), ("tv", "Séries"), ("audio", "Musique"), ("books", "Livres"), ("other", "Autre")];

/// Famille d'une release : d'après sa catégorie Newznab, sinon d'après son nom.
pub fn family(cat: &str, title: &str) -> &'static str {
    let c = cat.split([',', ' ']).map(str::trim).find(|c| !c.is_empty()).unwrap_or("");
    if c.len() == 4 && c.chars().all(|x| x.is_ascii_digit()) {
        return match &c[..1] {
            "2" => "movies",
            "5" => "tv",
            "3" => "audio",
            "7" => "books",
            _ => "other",
        };
    }
    let n = norm(title);
    let toks: Vec<&str> = n.split(['.', '-']).collect();
    if toks.iter().any(|t| t.len() >= 3 && t.starts_with('s') && t[1..3].chars().all(|c| c.is_ascii_digit())) {
        return "tv";
    }
    if toks.iter().any(|t| matches!(*t, "flac" | "mp3" | "aac" | "alac" | "web.flac")) {
        return "audio";
    }
    if toks.iter().any(|t| matches!(*t, "epub" | "pdf" | "mobi" | "azw3" | "ebook")) {
        return "books";
    }
    if toks.iter().any(|t| t.len() == 4 && (t.starts_with("19") || t.starts_with("20")) && t.chars().all(|c| c.is_ascii_digit())) {
        return "movies";
    }
    "other"
}

pub fn family_label(f: &str) -> &'static str {
    FAMILIES.iter().find(|x| x.0 == f).map(|x| x.1).unwrap_or("Autre")
}

/// Indexers interrogeables : actifs, avec une requête de recherche et ce qu'il faut pour
/// l'envoyer.
pub fn searchable(cfg: &Config) -> Vec<IndexerCfg> {
    cfg.indexers.iter().filter(|i| i.active && !i.name.trim().is_empty() && crate::search::method(i).is_some()).cloned().collect()
}

/// Indexers actifs qui ne savent pas chercher, avec la raison.
pub fn not_searchable(cfg: &Config) -> Vec<(String, String)> {
    cfg.indexers.iter().filter(|i| i.active && !i.name.trim().is_empty() && crate::search::method(i).is_none()).map(|i| (i.name.trim().to_string(), crate::search::why_not(i))).collect()
}

/// Cherche sur un indexer.
pub fn search_one(idx: &IndexerCfg, q: &str) -> (Status, Vec<Item>) {
    let name = idx.name.trim().to_string();
    let mut st = Status { indexer: name.clone(), ..Default::default() };
    let p = crate::apidef::profile(idx);
    let Some(req) = p.check_req().cloned() else {
        st.error = Some(crate::search::why_not(idx));
        return (st, Vec::new());
    };
    if let Err(e) = crate::apidef::ready(idx, &p) {
        st.error = Some(e);
        return (st, Vec::new());
    }
    let rep = match crate::apidef::send(idx, &p, &req, &[("release".into(), q.trim().to_string())]) {
        Ok(r) => r,
        Err(e) => {
            st.error = Some(e);
            return (st, Vec::new());
        }
    };
    st.request = rep.preview.first().cloned().unwrap_or_default();
    st.http = rep.http;
    st.secs = rep.secs;
    if let Some(e) = rep.error {
        st.error = Some(e);
        return (st, Vec::new());
    }
    let items: Vec<Item> = rep
        .rows
        .into_iter()
        .map(|r| {
            let guid = if !r.id.is_empty() && !r.id.contains('/') { r.id.clone() } else { r.id.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string() };
            Item { indexer: name.clone(), posted: parse_date(&r.date), title: r.title, size: r.size, cat: r.cat, page: r.link, guid, nzb: r.nzb }
        })
        .collect();
    st.count = items.len();
    (st, items)
}

/// Regroupe les résultats de tous les indexers par nom de release (le plus récent d'abord).
pub fn merge(items: &[Item]) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for it in items {
        let key = norm(&it.title);
        if key.is_empty() {
            continue;
        }
        match out.iter_mut().find(|g| g.key == key) {
            Some(g) => {
                if !g.has(&it.indexer) {
                    g.on.push(it.clone());
                }
                g.size = g.size.max(it.size);
                g.posted = if g.posted == 0 { it.posted } else if it.posted == 0 { g.posted } else { g.posted.min(it.posted) };
                if g.family == "other" {
                    g.family = family(&it.cat, &it.title);
                }
            }
            None => out.push(Group { key, title: it.title.clone(), size: it.size, posted: it.posted, family: family(&it.cat, &it.title), on: vec![it.clone()] }),
        }
    }
    out.sort_by(|a, b| b.posted.cmp(&a.posted).then_with(|| b.on.len().cmp(&a.on.len())));
    out
}

/// Tri : « date », « size », « name », « spread » (présente sur le plus d'indexers).
pub fn sort(groups: &mut [Group], by: &str) {
    match by {
        "size" => groups.sort_by(|a, b| b.size.cmp(&a.size)),
        "name" => groups.sort_by(|a, b| a.key.cmp(&b.key)),
        "spread" => groups.sort_by(|a, b| b.on.len().cmp(&a.on.len()).then_with(|| b.posted.cmp(&a.posted))),
        _ => groups.sort_by(|a, b| b.posted.cmp(&a.posted).then_with(|| b.on.len().cmp(&a.on.len()))),
    }
}

/// Où la release manque : indexers interrogés sans erreur qui ne l'ont pas renvoyée, moins
/// ceux où elle est notée « uploadée à la main ».
pub fn missing_on(g: &Group, searched: &[Status]) -> Vec<String> {
    let marked = crate::search::marked(&g.title);
    searched
        .iter()
        .filter(|s| s.error.is_none() && !g.has(&s.indexer) && !marked.iter().any(|m| m.eq_ignore_ascii_case(&s.indexer)))
        .map(|s| s.indexer.clone())
        .collect()
}

/// Date d'un résultat : RFC 2822 (« Tue, 10 Nov 2026 12:00:00 +0100 ») ou ISO 8601, ou
/// secondes Unix.
pub fn parse_date(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    if s.len() >= 9 && s.chars().all(|c| c.is_ascii_digit()) {
        return s.parse().unwrap_or(0);
    }
    let month = |m: &str| ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"].iter().position(|x| m.to_lowercase().starts_with(x)).map(|p| p as u32 + 1);
    // ISO : 2026-11-10T12:00:00Z / 2026-11-10 12:00:00
    let b = s.as_bytes();
    if s.len() >= 10 && b[4] == b'-' && b[7] == b'-' {
        let n = |a: usize, z: usize| s.get(a..z).and_then(|x| x.parse::<i64>().ok());
        let (Some(y), Some(mo), Some(d)) = (n(0, 4), n(5, 7), n(8, 10)) else { return 0 };
        let (h, mi, se) = (n(11, 13).unwrap_or(0), n(14, 16).unwrap_or(0), n(17, 19).unwrap_or(0));
        let rest = s.get(19..).unwrap_or("");
        let off = tz_offset(rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit()));
        return epoch(y, mo as u32, d as u32, h, mi, se, off);
    }
    // RFC 2822
    let parts: Vec<&str> = s.split_whitespace().filter(|p| !p.ends_with(',')).collect();
    let parts: Vec<&str> = if parts.first().map(|p| p.chars().all(|c| c.is_alphabetic())).unwrap_or(false) { parts[1..].to_vec() } else { parts };
    if parts.len() < 4 {
        return 0;
    }
    let (Ok(d), Some(mo), Ok(y)) = (parts[0].parse::<u32>(), month(parts[1]), parts[2].parse::<i64>()) else { return 0 };
    let hms: Vec<i64> = parts[3].split(':').map(|x| x.parse().unwrap_or(0)).collect();
    let off = parts.get(4).map(|z| tz_offset(z)).unwrap_or(0);
    epoch(y, mo, d, *hms.first().unwrap_or(&0), *hms.get(1).unwrap_or(&0), *hms.get(2).unwrap_or(&0), off)
}

fn tz_offset(z: &str) -> i64 {
    let z = z.trim();
    if z.is_empty() || z.eq_ignore_ascii_case("z") || z.eq_ignore_ascii_case("gmt") || z.eq_ignore_ascii_case("utc") {
        return 0;
    }
    let sign = if z.starts_with('-') { -1 } else { 1 };
    let d: String = z.chars().filter(|c| c.is_ascii_digit()).collect();
    if d.len() < 4 {
        return 0;
    }
    let h: i64 = d[..2].parse().unwrap_or(0);
    let m: i64 = d[2..4].parse().unwrap_or(0);
    sign * (h * 3600 + m * 60)
}

fn epoch(y: i64, mo: u32, d: u32, h: i64, mi: i64, s: i64, off: i64) -> u64 {
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return 0;
    }
    // jours depuis 1970 (algorithme civil de H. Hinnant)
    let (y2, m2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 as i64 + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let t = days * 86_400 + h * 3600 + mi * 60 + s - off;
    t.max(0) as u64
}

/// Âge lisible : « 3 h », « 12 j », « 4 mois ».
pub fn age(posted: u64, now: u64) -> String {
    if posted == 0 {
        return String::new();
    }
    let s = now.saturating_sub(posted);
    match s {
        0..=3599 => format!("{} min", (s / 60).max(1)),
        3600..=86_399 => format!("{} h", s / 3600),
        86_400..=5_183_999 => format!("{} j", s / 86_400),
        _ if s < 2 * 31_536_000 => format!("{} mois", s / 2_592_000),
        _ => format!("{} ans", s / 31_536_000),
    }
}

/// Adresse du NZB : le lien donné par l'indexer, sinon (Newznab) `t=get&id=…`.
pub fn nzb_url(idx: &IndexerCfg, it: &Item) -> Option<String> {
    let p = crate::apidef::profile(idx);
    let c = crate::apidef::conn(idx, &p);
    let nz = p.is_auto() || p.kind == "newznab";
    let direct = it.nzb.trim();
    let key = c.key.trim();
    // lien direct qui porte déjà la clé (r=, apikey=…) ou indexer sans clé
    if direct.starts_with("http") && (key.is_empty() || direct.contains(key) || !nz || it.guid.is_empty()) {
        return Some(direct.to_string());
    }
    if nz && !it.guid.is_empty() && !c.api.is_empty() {
        let sep = if c.api.contains('?') { '&' } else { '?' };
        return Some(format!("{}{sep}t=get&id={}&apikey={}", c.api, crate::upload::encode(&it.guid), crate::upload::encode(key)));
    }
    if direct.starts_with("http") {
        return Some(direct.to_string());
    }
    None
}

/// Télécharge le NZB d'un résultat dans `dir` ; renvoie son chemin. La clé passe par le
/// fichier de configuration de curl et n'apparaît dans aucun message.
pub fn fetch_nzb(idx: &IndexerCfg, it: &Item, dir: &Path) -> Result<PathBuf, String> {
    let p = crate::apidef::profile(idx);
    let key = crate::apidef::conn(idx, &p).key;
    let url = nzb_url(idx, it).ok_or_else(|| format!("{} ne donne pas de lien de téléchargement pour ce résultat", it.indexer))?;
    let mut args: Vec<String> = crate::upload::ua_args(idx);
    args.extend(["-sS".into(), "-L".into(), "--compressed".into(), "--max-time".into(), "180".into(), "-w".into(), "\n@@HTTP@@%{http_code}".into(), url]);
    let (body, http) = crate::upload::run_curl(&args).map_err(|e| crate::search::mask(&e.message, &key))?;
    check_nzb(&body, http).map_err(|e| crate::search::mask(&e, &key))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("dossier {} : {e}", dir.display()))?;
    let mut name = crate::prep::clean_name(&it.title);
    if name.is_empty() {
        name = format!("nzb-{}", crate::prep::now_secs());
    }
    let path = dir.join(format!("{name}.nzb"));
    std::fs::write(&path, body.as_bytes()).map_err(|e| format!("écriture de {} : {e}", path.display()))?;
    Ok(path)
}

/// La réponse est-elle un NZB ? Sinon, l'erreur de l'indexer en clair.
pub fn check_nzb(body: &str, http: Option<u16>) -> Result<(), String> {
    let head: String = body.chars().take(4000).collect::<String>().to_lowercase();
    if head.contains("<nzb") {
        return Ok(());
    }
    if let Some(i) = head.find("<error") {
        let tail = &body[i.min(body.len())..];
        let attr = |n: &str| tail.split(&format!("{n}=\"")).nth(1).and_then(|x| x.split('"').next()).unwrap_or("").to_string();
        let (code, desc) = (attr("code"), attr("description"));
        return Err(format!("l'indexer a refusé le téléchargement{}{}", if code.is_empty() { String::new() } else { format!(" (code {code})") }, if desc.is_empty() { String::new() } else { format!(" : {desc}") }));
    }
    let code = http.map(|c| format!("HTTP {c} {}", crate::upload::http_reason(c))).unwrap_or_else(|| "pas de réponse".into());
    if http.map(|c| !(200..300).contains(&c)).unwrap_or(true) {
        return Err(format!("téléchargement du NZB impossible ({code})"));
    }
    let start: String = body.trim().chars().take(80).collect();
    Err(format!("la réponse n'est pas un NZB ({code}) : « {} »", start.replace(['\r', '\n'], " ")))
}

/// Dossier où ranger les NZB téléchargés depuis la recherche.
pub fn nzb_dir() -> PathBuf {
    crate::app_dir().join("recherche")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(ix: &str, t: &str, posted: u64, cat: &str) -> Item {
        Item { indexer: ix.into(), title: t.into(), size: 1000, posted, cat: cat.into(), guid: format!("g-{ix}"), ..Default::default() }
    }

    #[test]
    fn regroupe_et_trouve_les_absents() {
        let items = vec![it("A", "Le.Film.2020.FRENCH.1080p.WEB-GRP", 100, "2040"), it("B", "Le Film 2020 FRENCH 1080p WEB-GRP", 90, ""), it("A", "Une.Serie.S01E02.720p-X", 200, "")];
        let g = merge(&items);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].title, "Une.Serie.S01E02.720p-X");
        assert_eq!(g[0].family, "tv");
        assert_eq!(g[1].on.len(), 2);
        assert_eq!(g[1].family, "movies");
        assert_eq!(g[1].posted, 90, "la plus ancienne publication");
        let st = vec![
            Status { indexer: "A".into(), ..Default::default() },
            Status { indexer: "B".into(), ..Default::default() },
            Status { indexer: "C".into(), ..Default::default() },
            Status { indexer: "D".into(), error: Some("HTTP 500".into()), ..Default::default() },
        ];
        assert_eq!(missing_on(&g[1], &st), vec!["C".to_string()], "D en erreur : on ne sait pas");
        assert_eq!(missing_on(&g[0], &st), vec!["B".to_string(), "C".to_string()]);
        let mut s = g.clone();
        sort(&mut s, "spread");
        assert_eq!(s[0].on.len(), 2);
        sort(&mut s, "name");
        assert!(s[0].key < s[1].key);
    }

    #[test]
    fn familles() {
        assert_eq!(family("5030", "x"), "tv");
        assert_eq!(family("2000,2040", "x"), "movies");
        assert_eq!(family("3040", "x"), "audio");
        assert_eq!(family("7020", "x"), "books");
        assert_eq!(family("1010", "x"), "other");
        assert_eq!(family("", "Show.S02E01.1080p"), "tv");
        assert_eq!(family("", "Album-2021-WEB-FLAC"), "audio");
        assert_eq!(family("", "Film.1999.MULTi.2160p"), "movies");
        assert_eq!(family("", "Logiciel.v2"), "other");
    }

    #[test]
    fn dates() {
        assert_eq!(parse_date("Thu, 01 Jan 1970 00:00:10 +0000"), 10);
        assert_eq!(parse_date("Tue, 10 Nov 2026 12:00:00 +0100"), 1_794_308_400);
        assert_eq!(parse_date("2026-11-10T11:00:00Z"), 1_794_308_400);
        assert_eq!(parse_date("2026-11-10 13:00:00+02:00"), 1_794_308_400);
        assert_eq!(parse_date("1794308400"), 1_794_308_400);
        assert_eq!(parse_date("n'importe quoi"), 0);
        assert_eq!(age(0, 5), "");
        assert_eq!(age(1000, 1000 + 7200), "2 h");
        assert_eq!(age(0 + 1, 1 + 3 * 86_400), "3 j");
    }

    #[test]
    fn lien_du_nzb() {
        let idx = IndexerCfg { name: "A".into(), api_url: "https://ix.example/api".into(), api_key: "SECRETKEY99".into(), active: true, ..Default::default() };
        let mut i = it("A", "R", 0, "");
        i.guid = "abc123".into();
        assert_eq!(nzb_url(&idx, &i).unwrap(), "https://ix.example/api?t=get&id=abc123&apikey=SECRETKEY99");
        i.nzb = "https://ix.example/getnzb/abc123.nzb&i=1&r=SECRETKEY99".into();
        assert_eq!(nzb_url(&idx, &i).unwrap(), i.nzb, "le lien direct porte la clé");
        i.nzb = "https://ix.example/getnzb/abc123.nzb".into();
        assert!(nzb_url(&idx, &i).unwrap().contains("t=get"), "lien sans clé : t=get");
        assert!(check_nzb("<?xml version=\"1.0\"?>\n<!DOCTYPE nzb><nzb xmlns=\"x\"></nzb>", Some(200)).is_ok());
        let e = check_nzb("<error code=\"100\" description=\"Incorrect user credentials\"/>", Some(200)).unwrap_err();
        assert!(e.contains("100") && e.contains("Incorrect"), "{e}");
        assert!(check_nzb("", Some(404)).unwrap_err().contains("404"));
        assert!(check_nzb("<html>login</html>", Some(200)).unwrap_err().contains("pas un NZB"));
    }

    #[test]
    fn recherche_sur_un_faux_indexer() {
        fn answer(head: &str, _all: &str) -> String {
            assert!(head.contains("t=search"), "{head}");
            RSS.to_string()
        }
        const RSS: &str = "200 <?xml version=\"1.0\"?><rss xmlns:newznab=\"http://www.newznab.com/DTD/2010/feeds/attributes/\"><channel><item><title>Le.Film.2020.FRENCH.1080p.WEB-GRP</title><guid isPermaLink=\"true\">https://ix/details/9ca5</guid><pubDate>Tue, 10 Nov 2026 12:00:00 +0100</pubDate><enclosure url=\"https://ix/getnzb/9ca5.nzb&amp;r=KEY\" length=\"5\" type=\"application/x-nzb\"/><newznab:attr name=\"category\" value=\"2040\"/><newznab:attr name=\"size\" value=\"734003200\"/><newznab:attr name=\"guid\" value=\"9ca5\"/></item></channel></rss>";
        let (url, seen) = crate::dl::tests::fake_server(answer);
        let idx = IndexerCfg { name: "Faux".into(), api_url: format!("{url}/api"), api_key: "KEY".into(), active: true, ..Default::default() };
        let (st, items) = search_one(&idx, "Le Film 2020");
        assert!(st.error.is_none(), "{st:?}");
        assert_eq!(items.len(), 1);
        let i = &items[0];
        assert_eq!((i.guid.as_str(), i.cat.as_str(), i.size, i.posted), ("9ca5", "2040", 734_003_200, 1_794_308_400));
        assert!(i.nzb.contains("getnzb"));
        assert!(!st.request.contains("KEY"), "clé masquée : {}", st.request);
        // la clé est bien partie (par le fichier de configuration de curl)
        assert!(seen.lock().unwrap().iter().any(|r| r.contains("apikey=KEY")));
    }
}
