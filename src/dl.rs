//! Téléchargement par le client Usenet (SABnzbd en priorité, NZBGet aussi) : file d'attente,
//! historique, débit, ajout d'un NZB, pause, et suivi des releases téléchargées pour être
//! repostées sur d'autres indexers que celui où elles ont été trouvées.
//!
//! Tout passe par l'API du client avec curl : la clé SABnzbd (dans l'adresse) et le mot de passe
//! NZBGet vont dans un fichier de configuration curl temporaire, jamais sur la ligne de commande
//! ni dans les journaux.

use crate::prep::{self, Config};
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- JSON minimal

/// Valeur JSON (réponses de SABnzbd et NZBGet).
#[derive(Clone, Debug, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    pub fn parse(s: &str) -> Option<J> {
        let b = s.as_bytes();
        let mut i = 0;
        let v = parse_value(b, &mut i)?;
        Some(v)
    }
    pub fn get(&self, k: &str) -> &J {
        static NULL: J = J::Null;
        match self {
            J::Obj(v) => v.iter().find(|(x, _)| x == k).map(|(_, y)| y).unwrap_or(&NULL),
            _ => &NULL,
        }
    }
    pub fn arr(&self) -> &[J] {
        match self {
            J::Arr(v) => v,
            _ => &[],
        }
    }
    /// Texte (un nombre est rendu en texte).
    pub fn s(&self) -> String {
        match self {
            J::Str(s) => s.clone(),
            J::Num(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{}", *n as i64),
            J::Num(n) => format!("{n}"),
            J::Bool(b) => b.to_string(),
            _ => String::new(),
        }
    }
    /// Nombre (SABnzbd envoie souvent des nombres entre guillemets).
    pub fn f(&self) -> f64 {
        match self {
            J::Num(n) => *n,
            J::Str(s) => s.trim().parse().unwrap_or(0.0),
            J::Bool(b) => *b as u8 as f64,
            _ => 0.0,
        }
    }
    pub fn b(&self) -> bool {
        match self {
            J::Bool(b) => *b,
            J::Str(s) => s == "true" || s == "True" || s == "1",
            J::Num(n) => *n != 0.0,
            _ => false,
        }
    }
}

fn ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && (b[*i] as char).is_ascii_whitespace() {
        *i += 1;
    }
}

fn parse_value(b: &[u8], i: &mut usize) -> Option<J> {
    ws(b, i);
    match *b.get(*i)? {
        b'{' => {
            *i += 1;
            let mut v = Vec::new();
            loop {
                ws(b, i);
                if *b.get(*i)? == b'}' {
                    *i += 1;
                    return Some(J::Obj(v));
                }
                let k = match parse_value(b, i)? {
                    J::Str(s) => s,
                    _ => return None,
                };
                ws(b, i);
                if *b.get(*i)? != b':' {
                    return None;
                }
                *i += 1;
                let val = parse_value(b, i)?;
                v.push((k, val));
                ws(b, i);
                match *b.get(*i)? {
                    b',' => *i += 1,
                    b'}' => {}
                    _ => return None,
                }
            }
        }
        b'[' => {
            *i += 1;
            let mut v = Vec::new();
            loop {
                ws(b, i);
                if *b.get(*i)? == b']' {
                    *i += 1;
                    return Some(J::Arr(v));
                }
                v.push(parse_value(b, i)?);
                ws(b, i);
                match *b.get(*i)? {
                    b',' => *i += 1,
                    b']' => {}
                    _ => return None,
                }
            }
        }
        b'"' => {
            *i += 1;
            let mut out: Vec<u8> = Vec::new();
            loop {
                let c = *b.get(*i)?;
                *i += 1;
                match c {
                    b'"' => return Some(J::Str(String::from_utf8_lossy(&out).into_owned())),
                    b'\\' => {
                        let e = *b.get(*i)?;
                        *i += 1;
                        match e {
                            b'n' => out.push(b'\n'),
                            b't' => out.push(b'\t'),
                            b'r' => out.push(b'\r'),
                            b'b' | b'f' => {}
                            b'u' => {
                                let hex = std::str::from_utf8(b.get(*i..*i + 4)?).ok()?;
                                *i += 4;
                                let mut cp = u32::from_str_radix(hex, 16).ok()?;
                                // paire de substitution (caractères hors du plan de base)
                                if (0xD800..0xDC00).contains(&cp) && b.get(*i) == Some(&b'\\') && b.get(*i + 1) == Some(&b'u') {
                                    if let Some(lo) = b.get(*i + 2..*i + 6).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u32::from_str_radix(h, 16).ok()) {
                                        if (0xDC00..0xE000).contains(&lo) {
                                            cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                            *i += 6;
                                        }
                                    }
                                }
                                let ch = char::from_u32(cp).unwrap_or('?');
                                let mut buf = [0u8; 4];
                                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                            }
                            x => out.push(x),
                        }
                    }
                    x => out.push(x),
                }
            }
        }
        b't' if b[*i..].starts_with(b"true") => {
            *i += 4;
            Some(J::Bool(true))
        }
        b'f' if b[*i..].starts_with(b"false") => {
            *i += 5;
            Some(J::Bool(false))
        }
        b'n' if b[*i..].starts_with(b"null") => {
            *i += 4;
            Some(J::Null)
        }
        _ => {
            let st = *i;
            while *i < b.len() && matches!(b[*i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                *i += 1;
            }
            std::str::from_utf8(&b[st..*i]).ok()?.parse().ok().map(J::Num)
        }
    }
}

/// Texte JSON (chaîne échappée).
pub fn jstr(s: &str) -> String {
    let mut o = String::from("\"");
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

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(data.len() / 3 * 4 + 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        o.push(T[(n >> 18) as usize & 63] as char);
        o.push(T[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    o
}

// ---------------------------------------------------------------- état du client

/// Téléchargement en cours ou en file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub pct: f32,
    pub size: u64,
    pub left: u64,
    pub status: String,
    pub eta: String,
    pub cat: String,
    /// Marques du client (SABnzbd : DUPLICATE, ENCRYPTED, PROPAGATING…) et priorité.
    pub labels: Vec<String>,
    pub priority: String,
}

/// Téléchargement terminé (réussi ou non).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Done {
    pub id: String,
    pub name: String,
    pub ok: bool,
    pub status: String,
    pub size: u64,
    /// Dossier du téléchargement, tel que le client le voit.
    pub path: String,
    pub completed: u64,
    pub fail: String,
    pub cat: String,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub client: String,
    pub version: String,
    pub speed_bps: f64,
    pub paused: bool,
    pub left: u64,
    pub queue: Vec<Job>,
    pub history: Vec<Done>,
    pub warnings: Vec<String>,
    /// Place libre dans le dossier des téléchargements terminés (SABnzbd), limite de débit
    /// (octets/s, 0 : aucune).
    pub disk_free: Option<u64>,
    pub limit_bps: f64,
    /// Pourquoi tout le client est en pause (minuterie, quota, place…), en clair.
    pub why_paused: Vec<String>,
}

impl Done {
    /// Le client en a fini (réussi ou non) : plus de vérification, réparation ni extraction.
    pub fn finished(&self) -> bool {
        let s = self.status.to_uppercase();
        self.ok || s == "FAILED" || ["SUCCESS", "FAILURE", "WARNING", "DELETED"].iter().any(|p| s.starts_with(p))
    }
}

impl State {
    /// Volume téléchargé avec succès depuis `since` (horodatage).
    pub fn bytes_since(&self, since: u64) -> u64 {
        self.history.iter().filter(|d| d.ok && d.completed >= since).map(|d| d.size).sum()
    }
}

pub fn client_name(cfg: &Config) -> &'static str {
    match cfg.dl_client.as_str() {
        "sabnzbd" => "SABnzbd",
        "nzbget" => "NZBGet",
        _ => "",
    }
}

pub fn configured(cfg: &Config) -> bool {
    !client_name(cfg).is_empty() && !cfg.dl_url.trim().is_empty()
}

/// Adresse de base (« http://192.168.1.10:8080 »), sans « / » final.
fn base(cfg: &Config) -> String {
    let u = cfg.dl_url.trim().trim_end_matches('/');
    let u = if u.contains("://") { u.to_string() } else { format!("http://{u}") };
    // adresse de l'interface collée telle quelle (…/sabnzbd/, …/api) : on garde la racine
    u.trim_end_matches("/api").trim_end_matches("/jsonrpc").to_string()
}

fn mask(s: &str, cfg: &Config) -> String {
    let mut s = s.to_string();
    for k in [cfg.dl_key.trim(), cfg.dl_pass.as_str()] {
        if k.len() >= 3 {
            s = s.replace(k, "••••");
        }
    }
    s
}

fn http(args: Vec<String>, cfg: &Config) -> Result<String, String> {
    let (body, code) = crate::upload::run_curl(&args).map_err(|e| mask(&e.message, cfg))?;
    match code {
        Some(c) if (200..300).contains(&c) => Ok(body),
        Some(401) | Some(403) => Err(format!("{} refuse la connexion (HTTP {}) : clé API ou identifiants à vérifier", client_name(cfg), code.unwrap_or(0))),
        Some(c) => Err(format!("{} : HTTP {c} {}", client_name(cfg), crate::upload::http_reason(c))),
        None => Ok(body),
    }
}

fn base_args() -> Vec<String> {
    ["-sS", "--max-time", "20", "-w", "\n@@HTTP@@%{http_code}"].iter().map(|s| s.to_string()).collect()
}

/// Requête SABnzbd (`/api?mode=…&output=json&apikey=…`), réponse JSON.
fn sab(cfg: &Config, query: &[(&str, &str)]) -> Result<J, String> {
    let mut q: Vec<(String, String)> = vec![("output".into(), "json".into()), ("apikey".into(), cfg.dl_key.trim().to_string())];
    q.extend(query.iter().map(|(a, b)| (a.to_string(), b.to_string())));
    let url = format!("{}/api?{}", base(cfg), q.iter().map(|(a, b)| format!("{a}={}", crate::upload::encode(b))).collect::<Vec<_>>().join("&"));
    let mut a = base_args();
    a.push(url);
    let body = http(a, cfg)?;
    let j = J::parse(body.trim()).ok_or_else(|| format!("réponse de SABnzbd illisible : {}", mask(&body.chars().take(160).collect::<String>(), cfg)))?;
    if let J::Str(e) = j.get("error") {
        return Err(format!("SABnzbd : {}", mask(e, cfg)));
    }
    Ok(j)
}

/// Appel JSON-RPC NZBGet (`/jsonrpc`), résultat.
fn nzbget(cfg: &Config, method: &str, params: &str) -> Result<J, String> {
    let body = format!("{{\"method\":{},\"params\":{params},\"id\":1}}", jstr(method));
    let dir = crate::app_dir().join("tmp");
    let _ = fs::create_dir_all(&dir);
    let f = dir.join(format!("nzbget-{}-{}.json", std::process::id(), prep::now_secs()));
    fs::write(&f, body).map_err(|e| format!("fichier temporaire : {e}"))?;
    let mut a = base_args();
    a.extend(["-X".into(), "POST".into(), "-H".into(), "Content-Type: application/json".into(), "--data-binary".into(), format!("@{}", f.display())]);
    if !cfg.dl_user.trim().is_empty() {
        a.push("-u".into());
        a.push(format!("{}:{}", cfg.dl_user.trim(), cfg.dl_pass));
    }
    a.push(format!("{}/jsonrpc", base(cfg)));
    let r = http(a, cfg);
    let _ = fs::remove_file(&f);
    let body = r?;
    let j = J::parse(body.trim()).ok_or_else(|| format!("réponse de NZBGet illisible : {}", mask(&body.chars().take(160).collect::<String>(), cfg)))?;
    if !matches!(j.get("error"), J::Null) {
        return Err(format!("NZBGet : {}", mask(&j.get("error").get("message").s(), cfg)));
    }
    Ok(j.get("result").clone())
}

const MB: f64 = 1024.0 * 1024.0;

/// État du client : version, débit, file d'attente, historique (les 100 derniers).
pub fn poll(cfg: &Config) -> Result<State, String> {
    match cfg.dl_client.as_str() {
        "sabnzbd" => {
            let q = sab(cfg, &[("mode", "queue"), ("limit", "200")])?;
            let q = q.get("queue");
            let h = sab(cfg, &[("mode", "history"), ("limit", "100")])?;
            let version = sab(cfg, &[("mode", "version")]).map(|v| v.get("version").s()).unwrap_or_default();
            let warnings: Vec<String> = sab(cfg, &[("mode", "warnings")])
                .map(|w| {
                    w.get("warnings")
                        .arr()
                        .iter()
                        .map(|x| match x {
                            J::Str(s) => s.clone(),
                            o => format!("{} {}", o.get("type").s(), o.get("text").s()).trim().to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let gb = 1024.0 * 1024.0 * 1024.0;
            let free = q.get("diskspace2").f().max(0.0);
            Ok(State {
                client: "SABnzbd".into(),
                version,
                disk_free: (!matches!(q.get("diskspace2"), J::Null)).then_some((free * gb) as u64),
                limit_bps: q.get("speedlimit_abs").f(),
                speed_bps: q.get("kbpersec").f() * 1024.0,
                paused: q.get("paused").b(),
                left: (q.get("mbleft").f() * MB) as u64,
                queue: q
                    .get("slots")
                    .arr()
                    .iter()
                    .map(|s| Job {
                        id: s.get("nzo_id").s(),
                        name: s.get("filename").s(),
                        pct: s.get("percentage").f() as f32,
                        size: (s.get("mb").f() * MB) as u64,
                        left: (s.get("mbleft").f() * MB) as u64,
                        status: s.get("status").s(),
                        eta: s.get("timeleft").s(),
                        cat: s.get("cat").s(),
                        labels: s.get("labels").arr().iter().map(|l| l.s()).filter(|l| !l.is_empty()).collect(),
                        priority: s.get("priority").s(),
                    })
                    .collect(),
                history: h
                    .get("history")
                    .get("slots")
                    .arr()
                    .iter()
                    .map(|s| {
                        let status = s.get("status").s();
                        Done {
                            id: s.get("nzo_id").s(),
                            name: s.get("name").s(),
                            ok: status == "Completed",
                            status,
                            size: s.get("bytes").f() as u64,
                            path: s.get("storage").s(),
                            completed: s.get("completed").f() as u64,
                            fail: s.get("fail_message").s(),
                            cat: s.get("category").s(),
                        }
                    })
                    .collect(),
                why_paused: sab_why_paused(&q, &warnings),
                warnings,
            })
        }
        "nzbget" => {
            let st = nzbget(cfg, "status", "[]")?;
            let version = nzbget(cfg, "version", "[]").map(|v| v.s()).unwrap_or_default();
            let groups = nzbget(cfg, "listgroups", "[]")?;
            let hist = nzbget(cfg, "history", "[false]")?;
            let mut history: Vec<Done> = hist
                .arr()
                .iter()
                .map(|s| {
                    let status = s.get("Status").s();
                    Done {
                        id: s.get("NZBID").s(),
                        name: s.get("Name").s(),
                        ok: status.starts_with("SUCCESS"),
                        status,
                        size: (s.get("FileSizeMB").f() * MB) as u64,
                        path: s.get("DestDir").s(),
                        completed: s.get("HistoryTime").f() as u64,
                        fail: String::new(),
                        cat: s.get("Category").s(),
                    }
                })
                .collect();
            history.truncate(100);
            Ok(State {
                client: "NZBGet".into(),
                version,
                disk_free: (!matches!(st.get("FreeDiskSpaceMB"), J::Null)).then_some((st.get("FreeDiskSpaceMB").f() * MB) as u64),
                limit_bps: st.get("DownloadLimit").f(),
                speed_bps: st.get("DownloadRate").f(),
                paused: st.get("DownloadPaused").b(),
                left: (st.get("RemainingSizeMB").f() * MB) as u64,
                queue: groups
                    .arr()
                    .iter()
                    .map(|g| {
                        let size = g.get("FileSizeMB").f();
                        let left = g.get("RemainingSizeMB").f();
                        Job {
                            id: g.get("NZBID").s(),
                            name: g.get("NZBName").s(),
                            pct: if size > 0.0 { ((size - left) / size * 100.0) as f32 } else { 0.0 },
                            size: (size * MB) as u64,
                            left: (left * MB) as u64,
                            status: g.get("Status").s(),
                            eta: String::new(),
                            cat: g.get("Category").s(),
                            labels: Vec::new(),
                            priority: g.get("MaxPriority").s(),
                        }
                    })
                    .collect(),
                history,
                warnings: Vec::new(),
                why_paused: {
                    let mut w = Vec::new();
                    if st.get("QuotaReached").b() {
                        w.push("quota de téléchargement atteint (NZBGet → Settings → Download Queue → quota)".to_string());
                    }
                    if st.get("Download2Paused").b() {
                        w.push("pause programmée par le planificateur de NZBGet (Settings → Scheduler)".into());
                    }
                    if st.get("DownloadPaused").b() && w.is_empty() {
                        w.push("NZBGet est en pause (bouton pause de NZBGet ou d'ici)".into());
                    }
                    w
                },
            })
        }
        _ => Err("aucun client de téléchargement réglé".into()),
    }
}

/// Pourquoi SABnzbd est en pause, d'après sa file (minuterie, quota, place, réglage).
fn sab_why_paused(q: &J, warnings: &[String]) -> Vec<String> {
    let mut w = Vec::new();
    let pause_int = q.get("pause_int").s();
    if q.get("have_quota").b() && q.get("left_quota").f() <= 0.0 && !matches!(q.get("left_quota"), J::Null) {
        w.push("quota de téléchargement atteint (SABnzbd → Config → Switches → Quota)".to_string());
    }
    if !pause_int.is_empty() && pause_int != "0" {
        w.push(format!("pause minutée : reprise toute seule dans {pause_int}"));
    }
    if warnings.iter().any(|x| { let l = x.to_lowercase(); l.contains("disk") || l.contains("diskspace") || l.contains("espace") }) {
        w.push("manque de place sur le disque des téléchargements (avertissement de SABnzbd)".into());
    }
    if q.get("paused").b() && w.is_empty() {
        w.push("SABnzbd est en pause (bouton pause, planificateur, ou « démarrer en pause » dans ses réglages)".into());
    }
    w
}

/// Pourquoi ce téléchargement ne progresse pas, en clair (vide : rien d'anormal).
pub fn job_why(st: &State, j: &Job) -> Vec<String> {
    let mut w: Vec<String> = Vec::new();
    let status = j.status.to_lowercase();
    for l in &j.labels {
        let u = l.to_uppercase();
        let t = if u.starts_with("DUPLICATE") {
            "le client l'a marqué DOUBLON (même nom déjà téléchargé ou en file) et l'a mis en pause : « Forcer » le lance quand même".to_string()
        } else if u.starts_with("ENCRYPTED") {
            "archive protégée par un mot de passe (le client attend le mot de passe)".into()
        } else if u.starts_with("TOO LARGE") {
            "plus gros que la taille maximale réglée dans le client".into()
        } else if u.starts_with("UNWANTED") {
            "contient une extension refusée par les réglages du client".into()
        } else if u.starts_with("PROPAGATING") {
            format!("attend que les articles soient propagés sur ton serveur ({l})")
        } else if u.starts_with("WAIT") {
            format!("en attente avant nouvel essai ({l})")
        } else if u.starts_with("INCOMPLETE") {
            "NZB incomplet (des articles manquent)".into()
        } else {
            l.clone()
        };
        w.push(t);
    }
    let prio = j.priority.to_lowercase();
    if status.contains("pause") {
        if prio == "-2" || prio.contains("pause") {
            w.push(format!("ajouté en pause par le client (priorité « Pause »{}) : « Forcer » ou « Reprendre » le lance", if j.cat.is_empty() { String::new() } else { format!(", catégorie « {} »", j.cat) }));
        } else if w.is_empty() && !st.paused {
            w.push("ce téléchargement est en pause (dans le client ou d'ici) : « Reprendre »".into());
        }
    }
    if st.paused {
        w.extend(st.why_paused.iter().cloned());
    }
    if status.contains("grab") || status.contains("fetch") {
        w.push("le client récupère le NZB à son adresse".into());
    } else if status.contains("queued") && !st.paused && st.queue.iter().position(|x| x.id == j.id).unwrap_or(0) > 0 {
        w.push("en file : attend la fin des téléchargements avant lui".into());
    } else if status.contains("checking") || status.contains("verif") {
        w.push("vérification en cours".into());
    }
    w
}

/// Lance un téléchargement même s'il est en pause ou marqué doublon (priorité « Forcer »).
pub fn force(cfg: &Config, id: &str) -> Result<(), String> {
    match cfg.dl_client.as_str() {
        "sabnzbd" => {
            sab(cfg, &[("mode", "queue"), ("name", "priority"), ("value", id), ("value2", "2")])?;
            sab(cfg, &[("mode", "queue"), ("name", "resume"), ("value", id)]).map(|_| ())
        }
        "nzbget" => {
            let n = id.parse::<i64>().unwrap_or(0);
            nzbget(cfg, "editqueue", &format!("[\"GroupSetPriority\",\"900\",[{n}]]"))?;
            nzbget(cfg, "editqueue", &format!("[\"GroupResume\",\"\",[{n}]]")).map(|_| ())
        }
        _ => Err("aucun client".into()),
    }
}

/// Adresse de l'interface web du client (pour l'ouvrir dans le navigateur).
pub fn web_url(cfg: &Config) -> String {
    base(cfg)
}

/// Comment le NZB est confié au client : catégorie (vide : celle des réglages), priorité,
/// ajouté en pause.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AddOpts {
    pub cat: String,
    /// « » (réglage), « default » (celle du client), « low », « normal », « high », « force ».
    pub prio: String,
    pub paused: bool,
}

/// Priorités proposées : (clé, libellé).
pub const PRIOS: [(&str, &str); 5] = [("default", "celle du client"), ("low", "basse"), ("normal", "normale"), ("high", "haute"), ("force", "forcée (même client en pause)")];

impl AddOpts {
    /// Catégorie réellement envoyée.
    pub fn category<'a>(&'a self, cfg: &'a Config) -> &'a str {
        if self.cat.trim().is_empty() { cfg.dl_cat.trim() } else { self.cat.trim() }
    }

    /// Priorité réellement envoyée (« » ou « default » : celle du client).
    pub fn priority<'a>(&'a self, cfg: &'a Config) -> &'a str {
        match self.prio.trim() {
            "" => cfg.dl_prio.trim(),
            p => p,
        }
    }

    /// En clair : « catégorie movies · priorité haute · en pause ».
    pub fn label(&self, cfg: &Config) -> String {
        let mut v = Vec::new();
        let c = self.category(cfg);
        v.push(if c.is_empty() { "catégorie du client par défaut".to_string() } else { format!("catégorie {c}") });
        let p = self.priority(cfg);
        if !p.is_empty() && p != "default" {
            v.push(format!("priorité {}", PRIOS.iter().find(|x| x.0 == p).map(|x| x.1).unwrap_or(p)));
        }
        if self.paused {
            v.push("ajouté en pause".into());
        }
        v.join(" · ")
    }
}

/// Priorité SABnzbd (-1 basse … 2 forcée ; -2 : en pause).
fn sab_priority(p: &str, paused: bool) -> Option<&'static str> {
    if paused {
        return Some("-2");
    }
    match p {
        "low" => Some("-1"),
        "normal" => Some("0"),
        "high" => Some("1"),
        "force" => Some("2"),
        _ => None,
    }
}

/// Priorité NZBGet (-50 basse, 0 normale, 50 haute, 900 forcée).
fn nzbget_priority(p: &str) -> i32 {
    match p {
        "low" => -50,
        "high" => 50,
        "force" => 900,
        _ => 0,
    }
}

/// Ajoute un NZB (fichier ou adresse) au client avec ces options ; renvoie son identifiant.
pub fn add_with(cfg: &Config, nzb: &str, name: &str, o: &AddOpts) -> Result<String, String> {
    let is_url = nzb.starts_with("http://") || nzb.starts_with("https://");
    let cat = o.category(cfg);
    let prio = sab_priority(o.priority(cfg), o.paused);
    match cfg.dl_client.as_str() {
        "sabnzbd" => {
            let j = if is_url {
                let mut q = vec![("mode", "addurl"), ("name", nzb)];
                if !name.trim().is_empty() {
                    q.push(("nzbname", name));
                }
                if !cat.is_empty() {
                    q.push(("cat", cat));
                }
                if let Some(p) = prio {
                    q.push(("priority", p));
                }
                sab(cfg, &q)?
            } else {
                let mut q: Vec<(String, String)> = vec![("mode".into(), "addfile".into()), ("output".into(), "json".into()), ("apikey".into(), cfg.dl_key.trim().to_string()), ("nzbname".into(), name.to_string())];
                if !cat.is_empty() {
                    q.push(("cat".into(), cat.to_string()));
                }
                if let Some(p) = prio {
                    q.push(("priority".into(), p.to_string()));
                }
                let url = format!("{}/api?{}", base(cfg), q.iter().map(|(a, b)| format!("{a}={}", crate::upload::encode(b))).collect::<Vec<_>>().join("&"));
                let mut a = base_args();
                a.extend(["-X".into(), "POST".into(), "-F".into(), format!("name=@{};filename={name}.nzb;type=application/x-nzb", nzb)]);
                a.push(url);
                let body = http(a, cfg)?;
                J::parse(body.trim()).ok_or("réponse de SABnzbd illisible")?
            };
            let ids = j.get("nzo_ids").arr();
            match ids.first() {
                Some(i) if j.get("status").b() || !i.s().is_empty() => Ok(i.s()),
                _ => Err(format!("SABnzbd a refusé le NZB{}", if j.get("error").s().is_empty() { String::new() } else { format!(" : {}", j.get("error").s()) })),
            }
        }
        "nzbget" => {
            let content = if is_url { nzb.to_string() } else { b64(&fs::read(nzb).map_err(|e| format!("lecture du NZB : {e}"))?) };
            let file = if name.trim().is_empty() { nzb.rsplit(['/', '\\']).next().unwrap_or("nzb").split('?').next().unwrap_or("nzb").to_string() } else { format!("{name}.nzb") };
            let params = format!("[{},{},{},{},false,{},\"\",0,\"SCORE\",[]]", jstr(&file), jstr(&content), jstr(cat), nzbget_priority(o.priority(cfg)), o.paused);
            let r = nzbget(cfg, "append", &params)?;
            let id = r.f() as i64;
            if id > 0 {
                Ok(id.to_string())
            } else {
                Err("NZBGet a refusé le NZB".into())
            }
        }
        _ => Err("aucun client de téléchargement réglé".into()),
    }
}

/// Pause / reprise de tous les téléchargements.
pub fn set_paused(cfg: &Config, pause: bool) -> Result<(), String> {
    match cfg.dl_client.as_str() {
        "sabnzbd" => sab(cfg, &[("mode", if pause { "pause" } else { "resume" })]).map(|_| ()),
        "nzbget" => nzbget(cfg, if pause { "pausedownload" } else { "resumedownload" }, "[]").map(|_| ()),
        _ => Err("aucun client".into()),
    }
}

/// Limite de débit en Ko/s (0 : aucune).
pub fn set_limit(cfg: &Config, kbps: u64) -> Result<(), String> {
    match cfg.dl_client.as_str() {
        "sabnzbd" => {
            let v = if kbps == 0 { "100".to_string() } else { format!("{kbps}K") };
            sab(cfg, &[("mode", "config"), ("name", "speedlimit"), ("value", &v)]).map(|_| ())
        }
        "nzbget" => nzbget(cfg, "rate", &format!("[{kbps}]")).map(|_| ()),
        _ => Err("aucun client".into()),
    }
}

/// Action sur un téléchargement terminé : « retry » (le retélécharger) ou « delete » (le retirer
/// de l'historique, les fichiers restent).
pub fn history_action(cfg: &Config, id: &str, what: &str) -> Result<(), String> {
    match cfg.dl_client.as_str() {
        "sabnzbd" if what == "retry" => sab(cfg, &[("mode", "retry"), ("value", id)]).map(|_| ()),
        "sabnzbd" => sab(cfg, &[("mode", "history"), ("name", "delete"), ("value", id)]).map(|_| ()),
        "nzbget" => {
            let cmd = if what == "retry" { "HistoryRedownload" } else { "HistoryDelete" };
            nzbget(cfg, "editqueue", &format!("[{},\"\",[{}]]", jstr(cmd), id.parse::<i64>().unwrap_or(0))).map(|_| ())
        }
        _ => Err("aucun client".into()),
    }
}

/// Action sur un téléchargement de la file : « pause », « resume » ou « delete ».
pub fn job_action(cfg: &Config, id: &str, what: &str) -> Result<(), String> {
    match cfg.dl_client.as_str() {
        "sabnzbd" => sab(cfg, &[("mode", "queue"), ("name", what), ("value", id)]).map(|_| ()),
        "nzbget" => {
            let cmd = match what {
                "pause" => "GroupPause",
                "resume" => "GroupResume",
                _ => "GroupDelete",
            };
            nzbget(cfg, "editqueue", &format!("[{},\"\",[{}]]", jstr(cmd), id.parse::<i64>().unwrap_or(0))).map(|_| ())
        }
        _ => Err("aucun client".into()),
    }
}

/// Dossier d'un téléchargement vu depuis ce PC : correspondance « dossier du client = dossier
/// local » (client sur un NAS ou dans Docker), barres remises à l'endroit sous Windows.
pub fn local_path(cfg: &Config, remote: &str) -> PathBuf {
    let mut p = remote.to_string();
    for rule in cfg.dl_path_map.split(';').map(str::trim).filter(|r| !r.is_empty()) {
        if let Some((from, to)) = rule.split_once('=') {
            let (from, to) = (from.trim().trim_end_matches(['/', '\\']), to.trim().trim_end_matches(['/', '\\']));
            if !from.is_empty() && (p == from || p.starts_with(&format!("{from}/")) || p.starts_with(&format!("{from}\\"))) {
                p = format!("{to}{}", &p[from.len()..]);
                break;
            }
        }
    }
    if cfg!(windows) {
        p = p.replace('/', "\\");
    }
    PathBuf::from(p)
}

// ---------------------------------------------------------------- reposts suivis

/// Téléchargement lancé pour être reposté : identifiant chez le client, release, indexer où le
/// NZB a été trouvé (jamais reposté là), indexers visés, ajout, état, note.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Repost {
    pub id: String,
    pub release: String,
    pub source: String,
    pub targets: Vec<String>,
    pub added: u64,
    /// « download », « ready » (attend ta validation), « processing », « done », « failed ».
    pub state: String,
    pub auto: bool,
    pub note: String,
    /// Traitement choisi avant le téléchargement : NFO, MediaInfo joint, catégories, suite.
    pub opts: RepostOpts,
    /// Releases préparées à partir du téléchargement (suivies jusqu'au dépôt).
    pub out: Vec<String>,
}

/// Traitement d'un repost, choisi avant le téléchargement.
#[derive(Clone, Debug, PartialEq)]
pub struct RepostOpts {
    /// « auto » (NFO du téléchargement, sinon créé), « keep » (celui du téléchargement
    /// seulement), « none » (aucun).
    pub nfo: String,
    pub mi: bool,
    /// Catégorie choisie par indexer (sinon calculée).
    pub cats: Vec<(String, String)>,
    /// « move » ou « copy » (vide : réglage du client).
    pub after: String,
    /// Rapports seuls, sans traitement ni repost : « nfo », « mi », « bd » séparés par « + »
    /// (vide : repost normal).
    pub reports: String,
}

impl Default for RepostOpts {
    fn default() -> Self {
        RepostOpts { nfo: "auto".into(), mi: true, cats: Vec::new(), after: String::new(), reports: String::new() }
    }
}

impl RepostOpts {
    /// Rapports seuls demandés (None : repost normal).
    pub fn reports_only(&self) -> Option<crate::jobs::ReportsOnly> {
        let r = self.reports.as_str();
        (!r.is_empty()).then(|| crate::jobs::ReportsOnly { nfo: r.contains("nfo"), mediainfo: r.contains("mi"), bdinfo: r.contains("bd") })
    }

    fn to_field(&self) -> String {
        let cats: Vec<String> = self.cats.iter().map(|(i, c)| format!("{}:{}", i.replace([':', ',', ';', '\t'], " "), c.replace([':', ',', ';', '\t'], ""))).collect();
        format!("nfo={};mi={};after={};cats={};rep={}", self.nfo, self.mi as u8, self.after, cats.join(","), self.reports)
    }
    fn from_field(s: &str) -> Self {
        let mut o = RepostOpts::default();
        for kv in s.split(';') {
            match kv.split_once('=') {
                Some(("nfo", v)) if ["auto", "keep", "none"].contains(&v) => o.nfo = v.into(),
                Some(("mi", v)) => o.mi = v != "0",
                Some(("after", v)) if v == "move" || v == "copy" => o.after = v.into(),
                Some(("rep", v)) => o.reports = v.into(),
                Some(("cats", v)) => o.cats = v.split(',').filter_map(|x| x.split_once(':')).map(|(i, c)| (i.to_string(), c.to_string())).filter(|(_, c)| !c.is_empty()).collect(),
                _ => {}
            }
        }
        o
    }
    /// Config du traitement du repost.
    pub fn apply(&self, c: &mut Config) {
        match self.nfo.as_str() {
            "none" => c.nfo_send = false,
            "keep" => {
                c.nfo_send = true;
                c.nfo_create = false;
                for i in c.indexers.iter_mut() {
                    i.send_nfo = false;
                }
            }
            _ => {
                c.nfo_send = true;
                c.nfo_create = true;
            }
        }
        for i in c.indexers.iter_mut() {
            i.send_mi = self.mi;
        }
        c.cat_force = self.cats.clone();
        if !self.after.is_empty() {
            c.dl_after = self.after.clone();
        }
    }
}

fn repost_file() -> PathBuf {
    crate::app_dir().join("nzbff-repost.tsv")
}

pub fn load_reposts() -> Vec<Repost> {
    load_reposts_from(&repost_file())
}

pub fn load_reposts_from(f: &Path) -> Vec<Repost> {
    fs::read_to_string(f)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            (v.len() >= 7).then(|| Repost {
                id: v[0].to_string(),
                release: v[1].to_string(),
                source: v[2].to_string(),
                targets: v[3].split('|').filter(|x| !x.is_empty()).map(String::from).collect(),
                added: v[4].parse().unwrap_or(0),
                state: v[5].to_string(),
                auto: v[6] == "auto",
                note: v.get(7).map(|s| s.to_string()).unwrap_or_default(),
                opts: v.get(8).map(|s| RepostOpts::from_field(s)).unwrap_or_default(),
                out: v.get(9).map(|s| s.split('|').filter(|x| !x.is_empty()).map(String::from).collect()).unwrap_or_default(),
            })
        })
        .collect()
}

pub fn save_reposts(v: &[Repost]) {
    save_reposts_to(&repost_file(), v)
}

pub fn save_reposts_to(f: &Path, v: &[Repost]) {
    let c = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let text: String = v
        .iter()
        .map(|r| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                c(&r.id),
                c(&r.release),
                c(&r.source),
                r.targets.iter().map(|t| c(t)).collect::<Vec<_>>().join("|"),
                r.added,
                r.state,
                if r.auto { "auto" } else { "manuel" },
                c(&r.note),
                c(&r.opts.to_field()),
                r.out.iter().map(|t| c(t)).collect::<Vec<_>>().join("|")
            )
        })
        .collect();
    let _ = fs::write(f, text);
}

/// Téléchargement en cours chez le client : celui qui télécharge, sinon le premier pas en pause.
pub fn active_job(s: &State) -> Option<&Job> {
    let low = |j: &Job| j.status.to_lowercase();
    s.queue.iter().find(|j| low(j).contains("download")).or_else(|| s.queue.iter().find(|j| !low(j).contains("pause")))
}

/// Indexers où reposter : ceux choisis, jamais celui d'origine.
pub fn repost_targets(source: &str, chosen: &[String]) -> Vec<String> {
    chosen.iter().filter(|t| !t.trim().eq_ignore_ascii_case(source.trim())).cloned().collect()
}

/// Suite d'un repost d'après l'état du client : terminé (réussi ou non) dans l'historique.
pub fn follow(r: &Repost, st: &State) -> Option<(bool, Done)> {
    if r.state != "download" {
        return None;
    }
    // encore en post-traitement chez le client (vérification, réparation, extraction) : on attend
    st.history.iter().find(|d| (d.id == r.id || (r.id.is_empty() && d.name == r.release)) && d.finished()).map(|d| (d.ok, d.clone()))
}

// ------------------------------------------------------------------ réglages par défaut

/// Traitement par défaut d'un repost (Réglages → Téléchargements).
pub fn default_opts(cfg: &Config) -> RepostOpts {
    let nfo = if ["auto", "keep", "none"].contains(&cfg.dl_nfo.as_str()) { cfg.dl_nfo.clone() } else { "auto".into() };
    RepostOpts { nfo, mi: cfg.dl_mi, cats: Vec::new(), after: cfg.dl_after.clone(), reports: String::new() }
}

/// Indexer où les NZB ouverts ont été trouvés, d'après les réglages : None = à demander,
/// Some("") = ailleurs (aucun de mes indexers), Some(nom) = un indexer actif.
pub fn default_origin(cfg: &Config) -> Option<String> {
    match cfg.dl_origin.trim() {
        "" => None,
        "*" => Some(String::new()),
        o => crate::ngpost::active_indexers(cfg).into_iter().find(|i| i.eq_ignore_ascii_case(o)),
    }
}

// ------------------------------------------------------------------ dossier de NZB surveillé

/// Sous-dossiers du dossier surveillé : NZB trouvés ailleurs (reposté partout), à télécharger
/// sans repost, et rangement des NZB pris en charge ou refusés.
pub const WATCH_ELSEWHERE: &str = "_Ailleurs";
pub const WATCH_DL_ONLY: &str = "_Télécharger seulement";
pub const WATCH_DONE: &str = "_Pris en charge";
pub const WATCH_ERR: &str = "_Erreurs";

/// Règle d'un emplacement du dossier surveillé (racine ou sous-dossier) : ce que devient un
/// .nzb qui y est déposé. Les emplacements de base (racine, un par indexer, « _Ailleurs »,
/// « _Télécharger seulement ») ont une règle par défaut, modifiable ; on peut en ajouter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WatchRoute {
    /// Sous-dossier (vide : la racine).
    pub folder: String,
    /// « repost » (téléchargé puis reposté), « dl » (téléchargé seulement), « ask » (réglé sur
    /// la page Téléchargements), « send » (déposé tel quel sur les indexers), « open » (racine :
    /// comme un double clic), « off » (ignoré).
    pub action: String,
    /// Repost : indexer où le NZB a été trouvé (vide : ailleurs), jamais reposté dessus.
    pub origin: String,
    /// Repost : indexers visés (vide : réglage « Reposter sur »).
    pub targets: Vec<String>,
    /// Confié au client avec : catégorie, priorité, en pause.
    pub add: AddOpts,
    /// Repost : « » (réglage), « auto » (dès la fin du téléchargement), « manual » (bouton).
    pub when: String,
}

/// Actions d'une règle : (clé, libellé court).
pub const ROUTE_ACTIONS: [(&str, &str); 6] = [("repost", "télécharger puis reposter"), ("dl", "télécharger seulement"), ("ask", "me demander (page Téléchargements)"), ("send", "déposer tel quel sur les indexers"), ("open", "comme un double clic sur un .nzb"), ("off", "ignorer")];

/// D'où vient une règle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RouteKind {
    Root,
    Indexer,
    Elsewhere,
    DlOnly,
    Custom,
}

impl WatchRoute {
    pub fn to_line(&self) -> String {
        let f = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        // racine écrite « . » : une ligne de réglage est lue sans ses espaces de tête
        let folder = if self.folder.trim().is_empty() { ".".to_string() } else { f(&self.folder) };
        format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", folder, f(&self.action), f(&self.origin), f(&self.targets.join(",")), f(&self.add.cat), f(&self.add.prio), self.add.paused as u8, f(&self.when))
    }

    pub fn from_line(l: &str) -> Option<Self> {
        let c: Vec<&str> = l.split('\t').collect();
        let action = c.get(1)?.trim();
        if !ROUTE_ACTIONS.iter().any(|a| a.0 == action) {
            return None;
        }
        let g = |k: usize| c.get(k).map(|x| x.trim().to_string()).unwrap_or_default();
        Some(WatchRoute {
            folder: if g(0) == "." { String::new() } else { g(0) },
            action: action.into(),
            origin: g(2),
            targets: g(3).split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect(),
            add: AddOpts { cat: g(4), prio: g(5), paused: g(6) == "1" },
            when: g(7),
        })
    }

    /// Le .nzb doit-il être reposté tout seul à la fin du téléchargement ?
    pub fn auto(&self, cfg: &Config) -> bool {
        match self.when.as_str() {
            "auto" => true,
            "manual" => false,
            _ => cfg.dl_auto,
        }
    }

    /// Indexers où reposter (origine retirée).
    pub fn repost_to(&self, cfg: &Config) -> Vec<String> {
        if self.targets.is_empty() {
            default_targets(cfg, &self.origin)
        } else {
            let active = crate::ngpost::active_indexers(cfg);
            let chosen: Vec<String> = active.into_iter().filter(|i| self.targets.iter().any(|t| t.eq_ignore_ascii_case(i))).collect();
            repost_targets(&self.origin, &chosen)
        }
    }

    /// Ce que fait la règle, en clair.
    pub fn describe(&self, cfg: &Config) -> String {
        let add = self.add.label(cfg);
        match self.action.as_str() {
            "repost" => {
                let t = self.repost_to(cfg);
                format!(
                    "{} → téléchargé ({add}) puis reposté {} sur {}",
                    if self.origin.is_empty() { "trouvé ailleurs".to_string() } else { format!("trouvé sur {}", self.origin) },
                    if self.auto(cfg) { "tout seul" } else { "d'un clic (« Reposter »)" },
                    if t.is_empty() { "aucun indexer : rien ne sera reposté".to_string() } else { t.join(", ") }
                )
            }
            "dl" => format!("téléchargé seulement ({add}), pas de repost"),
            "ask" => "ajouté à la page Téléchargements, où tu règles tout avant de lancer".into(),
            "send" => "déposé tel quel sur les indexers (page Envoyer des NZB)".into(),
            "open" => format!("comme un double clic sur un .nzb : {}", crate::shell::nzb_mode_label(cfg)),
            _ => "ignoré (le .nzb reste où il est)".into(),
        }
    }
}

/// Règle par défaut d'un emplacement de base.
fn base_route(kind: RouteKind, folder: &str, origin: &str) -> WatchRoute {
    let action = match kind {
        RouteKind::Root => "open",
        RouteKind::DlOnly => "dl",
        _ => "repost",
    };
    WatchRoute { folder: folder.into(), action: action.into(), origin: origin.into(), ..Default::default() }
}

/// Toutes les règles du dossier surveillé, dans l'ordre d'affichage : racine, un sous-dossier
/// par indexer actif (si réglé), « _Ailleurs », « _Télécharger seulement », puis les
/// sous-dossiers ajoutés. Une règle de base modifiée est lue dans `cfg.dl_routes`.
pub fn watch_routes(cfg: &Config) -> Vec<(WatchRoute, RouteKind)> {
    let custom = |folder: &str| cfg.dl_routes.iter().find(|r| r.folder.eq_ignore_ascii_case(folder)).cloned();
    let mut v: Vec<(WatchRoute, RouteKind)> = Vec::new();
    v.push((custom("").unwrap_or_else(|| base_route(RouteKind::Root, "", "")), RouteKind::Root));
    if cfg.dl_watch_subdirs {
        for i in crate::ngpost::active_indexers(cfg) {
            let f = crate::ngpost::folder_name(&i);
            // l'origine d'un dossier d'indexer reste cet indexer
            let r = custom(&f).map(|mut r| {
                r.origin = i.clone();
                r
            });
            v.push((r.unwrap_or_else(|| base_route(RouteKind::Indexer, &f, &i)), RouteKind::Indexer));
        }
    }
    v.push((custom(WATCH_ELSEWHERE).map(|mut r| { r.origin.clear(); r }).unwrap_or_else(|| base_route(RouteKind::Elsewhere, WATCH_ELSEWHERE, "")), RouteKind::Elsewhere));
    v.push((custom(WATCH_DL_ONLY).unwrap_or_else(|| base_route(RouteKind::DlOnly, WATCH_DL_ONLY, "")), RouteKind::DlOnly));
    for r in &cfg.dl_routes {
        let f = r.folder.trim();
        if f.is_empty() || v.iter().any(|(x, _)| x.folder.eq_ignore_ascii_case(f)) || is_reserved(f) {
            continue;
        }
        v.push((r.clone(), RouteKind::Custom));
    }
    v
}

/// Noms qu'un sous-dossier ajouté ne peut pas prendre (rangement) ; « _Ailleurs » et
/// « _Télécharger seulement » se modifient, ils ne se recréent pas.
pub fn is_reserved(folder: &str) -> bool {
    [WATCH_DONE, WATCH_ERR].iter().any(|x| x.eq_ignore_ascii_case(folder.trim()))
}

/// Nom de sous-dossier valable sous Windows (vide si rien ne reste).
pub fn clean_folder(name: &str) -> String {
    let s: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { ' ' } else { c }).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches(['.', ' ']).to_string()
}

/// Enregistre une règle : celle d'un emplacement de base revenue à son défaut est oubliée.
pub fn set_route(cfg: &mut Config, r: WatchRoute, kind: RouteKind) {
    cfg.dl_routes.retain(|x| !x.folder.eq_ignore_ascii_case(&r.folder));
    let is_default = match kind {
        RouteKind::Root => r == base_route(kind, "", ""),
        RouteKind::Indexer => r == base_route(kind, &r.folder, &r.origin),
        RouteKind::Elsewhere => r == base_route(kind, WATCH_ELSEWHERE, ""),
        RouteKind::DlOnly => r == base_route(kind, WATCH_DL_ONLY, ""),
        RouteKind::Custom => false,
    };
    if !is_default {
        cfg.dl_routes.push(r);
    }
}

/// Règle d'un sous-dossier (None : non surveillé). Un dossier d'indexer inactif ou inconnu,
/// créé à la main, garde l'ancien sens : « trouvé sur <nom> ».
pub fn route_for(cfg: &Config, folder: &str) -> Option<WatchRoute> {
    if let Some((r, _)) = watch_routes(cfg).into_iter().find(|(r, _)| r.folder.eq_ignore_ascii_case(folder)) {
        return (r.action != "off").then_some(r);
    }
    if folder.starts_with(['_', '.', '@']) || !cfg.dl_watch_subdirs {
        return None;
    }
    Some(WatchRoute { folder: folder.into(), action: "repost".into(), origin: folder.into(), ..Default::default() })
}

pub fn watch_root(cfg: &Config) -> Option<PathBuf> {
    let r = cfg.dl_watch.trim();
    (!r.is_empty()).then(|| PathBuf::from(r))
}

/// Crée le dossier surveillé, un sous-dossier par règle (sauf « ignorer ») et un mode d'emploi
/// qui reprend les règles.
pub fn ensure_watch(cfg: &Config) -> Result<PathBuf, String> {
    let r = watch_root(cfg).ok_or("aucun dossier de NZB surveillé")?;
    fs::create_dir_all(&r).map_err(|e| format!("{} : {e}", r.display()))?;
    let routes = watch_routes(cfg);
    for (route, _) in routes.iter().filter(|(x, _)| !x.folder.is_empty() && x.action != "off") {
        let d = r.join(&route.folder);
        fs::create_dir_all(&d).map_err(|e| format!("{} : {e}", d.display()))?;
    }
    let mut help = String::from("NZB FF Tool - dossier de NZB surveillé\r\n\r\nDépose un .nzb à l'endroit qui correspond à ce que tu veux :\r\n");
    for (route, _) in &routes {
        let place = if route.folder.is_empty() { "ici, à la racine".to_string() } else { format!("{}\\", route.folder) };
        help.push_str(&format!("- {place} : {}\r\n", route.describe(cfg)));
    }
    help.push_str(&format!("\r\nEnsuite le .nzb est {} ({WATCH_ERR}\\ avec la raison s'il est refusé).\r\nRègles : Réglages > Téléchargements > Dossier de NZB surveillé.\r\n", if cfg.dl_watch_after == "delete" { "supprimé".to_string() } else { format!("rangé dans {WATCH_DONE}\\") }));
    let _ = fs::write(r.join("À LIRE.txt"), help);
    Ok(r)
}

/// Un sous-dossier de règle manque-t-il (dossier racine présent) ?
pub fn watch_dirs_missing(cfg: &Config) -> bool {
    let Some(r) = watch_root(cfg) else { return false };
    r.is_dir() && watch_routes(cfg).iter().any(|(x, _)| !x.folder.is_empty() && x.action != "off" && !r.join(&x.folder).is_dir())
}

/// NZB prêts du dossier surveillé et la règle qui s'applique. Un .nzb est prêt quand sa
/// taille n'a pas bougé depuis `cfg.dl_watch_stable` secondes (et deux passages au moins).
/// `seen` : taille et instant où elle a été vue pour la première fois.
pub fn scan_watch(cfg: &Config, seen: &mut std::collections::HashMap<PathBuf, (u64, u64)>) -> Vec<(PathBuf, WatchRoute)> {
    scan_watch_at(cfg, seen, prep::now_secs())
}

pub fn scan_watch_at(cfg: &Config, seen: &mut std::collections::HashMap<PathBuf, (u64, u64)>, now: u64) -> Vec<(PathBuf, WatchRoute)> {
    let Some(root) = watch_root(cfg) else { return Vec::new() };
    let mut places: Vec<(PathBuf, WatchRoute)> = Vec::new();
    if let Some(r) = route_for(cfg, "") {
        places.push((root.clone(), r));
    }
    if let Ok(rd) = fs::read_dir(&root) {
        for d in rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            if let Some(r) = route_for(cfg, &prep::file_name(&d)) {
                places.push((d, r));
            }
        }
    }
    let mut out = Vec::new();
    let mut now_seen = std::collections::HashMap::new();
    for (dir, route) in places {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for f in rd.flatten().map(|e| e.path()) {
            let n = prep::file_name(&f);
            if !f.is_file() || n.starts_with(['.', '~']) || !n.to_lowercase().ends_with(".nzb") {
                continue;
            }
            let size = fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
            match seen.get(&f) {
                Some(&(s, since)) if s == size && size > 0 && now.saturating_sub(since) >= cfg.dl_watch_stable => out.push((f, route.clone())),
                Some(&(s, since)) if s == size => {
                    now_seen.insert(f, (s, since));
                }
                _ => {
                    now_seen.insert(f, (size, now));
                }
            }
        }
    }
    *seen = now_seen;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Indexers où reposter par défaut, l'indexer d'origine retiré (réglage « Reposter sur »,
/// vide : tous les autres indexers actifs).
pub fn default_targets(cfg: &Config, origin: &str) -> Vec<String> {
    let active = crate::ngpost::active_indexers(cfg);
    let chosen: Vec<String> = if cfg.dl_targets.is_empty() { active } else { active.into_iter().filter(|i| cfg.dl_targets.iter().any(|t| t.eq_ignore_ascii_case(i))).collect() };
    repost_targets(origin, &chosen)
}

/// Ce qui est arrivé à un NZB du dossier surveillé (journal gardé à côté de l'application).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WatchEntry {
    pub when: u64,
    /// Nom du fichier .nzb.
    pub file: String,
    /// Sous-dossier où il a été déposé (vide : racine).
    pub place: String,
    pub release: String,
    /// « repost », « dl », « ask » (à régler), « send » (à déposer), « err ».
    pub result: String,
    /// Raison d'un refus, ou indexers du repost.
    pub note: String,
    /// Où est le .nzb maintenant (vide : supprimé).
    pub path: String,
}

const WATCH_LOG: &str = "nzbff-nzb-surveilles.tsv";

fn watch_log_path() -> PathBuf {
    crate::app_dir().join(WATCH_LOG)
}

pub fn watch_log_load() -> Vec<WatchEntry> {
    watch_log_parse(&fs::read_to_string(watch_log_path()).unwrap_or_default())
}

fn watch_log_parse(text: &str) -> Vec<WatchEntry> {
    text.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            (c.len() >= 7).then(|| WatchEntry { when: c[0].parse().unwrap_or(0), file: c[1].into(), place: c[2].into(), release: c[3].into(), result: c[4].into(), note: c[5].into(), path: c[6].into() })
        })
        .collect()
}

fn watch_log_text(v: &[WatchEntry]) -> String {
    let f = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    v.iter().map(|e| format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\n", e.when, f(&e.file), f(&e.place), f(&e.release), f(&e.result), f(&e.note), f(&e.path))).collect()
}

/// Ajoute une ligne au journal (500 dernières gardées).
pub fn watch_log_add(e: WatchEntry) -> Vec<WatchEntry> {
    let mut v = watch_log_load();
    v.push(e);
    if v.len() > 500 {
        v.drain(0..v.len() - 500);
    }
    let _ = fs::write(watch_log_path(), watch_log_text(&v));
    v
}

/// Remet un NZB refusé (« _Erreurs ») là où il avait été déposé : il est repris au passage
/// suivant.
pub fn watch_retry(cfg: &Config, e: &WatchEntry) -> Result<PathBuf, String> {
    let root = watch_root(cfg).ok_or("aucun dossier de NZB surveillé")?;
    let from = PathBuf::from(&e.path);
    if !from.is_file() {
        return Err(format!("{} n'est plus dans {WATCH_ERR}", e.file));
    }
    let dir = if e.place.is_empty() { root } else { root.join(&e.place) };
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let to = dir.join(&e.file);
    fs::rename(&from, &to).map_err(|err| err.to_string())?;
    let _ = fs::remove_file(from.with_extension("erreur.txt"));
    Ok(to)
}

/// Sous-dossier (sous la racine surveillée) d'un NZB, vide à la racine.
pub fn watch_place(cfg: &Config, nzb: &Path) -> String {
    match (watch_root(cfg), nzb.parent()) {
        (Some(r), Some(p)) if p != r.as_path() => prep::file_name(p),
        _ => String::new(),
    }
}

/// Le même nom de release est-il déjà téléchargé, en cours, ou reposté ?
pub fn already_seen(name: &str, reps: &[Repost], st: Option<&State>) -> Option<String> {
    let n = name.trim();
    if let Some(r) = reps.iter().find(|r| r.release.eq_ignore_ascii_case(n) && r.state != "failed" && r.state != "cancelled") {
        return Some(format!("déjà suivi (repost {})", match r.state.as_str() {
            "download" => "en téléchargement",
            "processing" => "en traitement",
            "ready" => "téléchargé, à reposter",
            _ => "fait",
        }));
    }
    if let Some(s) = st {
        if s.queue.iter().any(|j| j.name.eq_ignore_ascii_case(n)) {
            return Some("déjà dans la file du client".into());
        }
        if s.history.iter().any(|d| d.ok && d.name.eq_ignore_ascii_case(n)) {
            return Some("déjà téléchargé (historique du client)".into());
        }
    }
    None
}

/// Range un NZB du dossier surveillé une fois pris en charge (ou refusé, avec la raison).
pub fn finish_watch(cfg: &Config, nzb: &Path, error: Option<&str>) -> Option<PathBuf> {
    let root = watch_root(cfg)?;
    if error.is_none() && cfg.dl_watch_after == "delete" {
        let _ = fs::remove_file(nzb);
        return None;
    }
    let dir = root.join(if error.is_some() { WATCH_ERR } else { WATCH_DONE });
    fs::create_dir_all(&dir).ok()?;
    let stem = nzb.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut dst = dir.join(prep::file_name(nzb));
    let mut k = 2;
    while dst.exists() {
        dst = dir.join(format!("{stem} ({k}).nzb"));
        k += 1;
    }
    if fs::rename(nzb, &dst).is_err() {
        fs::copy(nzb, &dst).ok()?;
        let _ = fs::remove_file(nzb);
    }
    if let Some(e) = error {
        let _ = fs::write(dst.with_extension("erreur.txt"), format!("{e}\r\n"));
    }
    Some(dst)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Un téléchargement en post-traitement (vérification, extraction) n'est ni réussi ni raté.
    #[test]
    fn post_traitement_du_client() {
        let r = Repost { id: "a".into(), state: "download".into(), ..Default::default() };
        let mut st = State { history: vec![Done { id: "a".into(), status: "Extracting".into(), ..Default::default() }], ..Default::default() };
        assert!(follow(&r, &st).is_none());
        st.history[0].status = "Failed".into();
        assert_eq!(follow(&r, &st).map(|x| x.0), Some(false));
        st.history[0].status = "Completed".into();
        st.history[0].ok = true;
        assert_eq!(follow(&r, &st).map(|x| x.0), Some(true));
        for (s, f) in [("SUCCESS/ALL", true), ("FAILURE/PAR", true), ("PP_QUEUED", false), ("Verifying", false), ("Repairing", false), ("Moving", false)] {
            assert_eq!(Done { status: s.into(), ..Default::default() }.finished(), f, "{s}");
        }
    }

    /// Raisons d'une pause, en clair.
    #[test]
    fn raisons_d_une_pause() {
        let q = J::parse(r#"{"queue":{"paused":true,"pause_int":"12:30","have_quota":false,"slots":[{"nzo_id":"a","filename":"F","status":"Paused","labels":["DUPLICATE"],"priority":"Normal","cat":"nzbff"},{"nzo_id":"b","filename":"G","status":"Paused","labels":[],"priority":"Paused","cat":"tv"}]}}"#).unwrap();
        let q = q.get("queue");
        let why = sab_why_paused(&q, &["Too little diskspace".to_string()]);
        assert!(why.iter().any(|w| w.contains("12:30")) && why.iter().any(|w| w.contains("place")));
        let st = State { paused: true, why_paused: why, ..Default::default() };
        let mk = |s: &J| Job { id: s.get("nzo_id").s(), status: s.get("status").s(), labels: s.get("labels").arr().iter().map(|l| l.s()).collect(), priority: s.get("priority").s(), cat: s.get("cat").s(), ..Default::default() };
        let a = mk(&q.get("slots").arr()[0]);
        let b = mk(&q.get("slots").arr()[1]);
        assert!(job_why(&st, &a)[0].contains("DOUBLON"));
        assert!(job_why(&st, &b)[0].contains("priorité « Pause »") && job_why(&st, &b)[0].contains("« tv »"));
        // client en marche, rien d'anormal
        let ok = State { queue: vec![Job { id: "x".into(), status: "Downloading".into(), ..Default::default() }], ..Default::default() };
        assert!(job_why(&ok, &ok.queue[0]).is_empty());
        let q2 = J::parse(r#"{"paused":true,"pause_int":"0"}"#).unwrap();
        assert!(sab_why_paused(&q2, &[])[0].contains("SABnzbd est en pause"));
    }

    /// Journal du dossier surveillé, nouvel essai, doublons, indexers par défaut.
    #[test]
    fn journal_et_doublons_du_dossier_surveille() {
        let e = WatchEntry { when: 5, file: "A.nzb".into(), place: "altHUB".into(), release: "A".into(), result: "err".into(), note: "x\ty".into(), path: "/e/A.nzb".into() };
        let v = watch_log_parse(&watch_log_text(&[e.clone()]));
        assert_eq!(v, vec![WatchEntry { note: "x y".into(), ..e.clone() }]);
        let root = std::env::temp_dir().join(format!("nzbff-watch2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mut cfg = Config { dl_watch: root.display().to_string(), ..Default::default() };
        cfg.indexers = vec![prep::IndexerCfg::new("A1"), prep::IndexerCfg::new("B2"), prep::IndexerCfg::new("C3")];
        ensure_watch(&cfg).unwrap();
        let f = root.join("A1").join("R.nzb");
        fs::write(&f, "<nzb/>").unwrap();
        assert_eq!(watch_place(&cfg, &f), "A1");
        assert_eq!(watch_place(&cfg, &root.join("R.nzb")), "");
        let err = finish_watch(&cfg, &f, Some("boum")).unwrap();
        let back = watch_retry(&cfg, &WatchEntry { file: "R.nzb".into(), place: "A1".into(), path: err.display().to_string(), ..Default::default() }).unwrap();
        assert_eq!(back, f);
        assert!(f.is_file() && !err.exists() && !err.with_extension("erreur.txt").exists());
        // supprimé au lieu d'être rangé
        cfg.dl_watch_after = "delete".into();
        assert_eq!(finish_watch(&cfg, &f, None), None);
        assert!(!f.exists());
        // indexers par défaut
        assert_eq!(default_targets(&cfg, "A1"), vec!["B2".to_string(), "C3".to_string()]);
        cfg.dl_targets = vec!["c3".into(), "A1".into()];
        assert_eq!(default_targets(&cfg, "A1"), vec!["C3".to_string()]);
        // doublons
        let reps = vec![Repost { release: "Film.2020-GRP".into(), state: "done".into(), ..Default::default() }, Repost { release: "Raté-GRP".into(), state: "failed".into(), ..Default::default() }];
        assert!(already_seen("film.2020-grp", &reps, None).is_some());
        assert!(already_seen("Raté-GRP", &reps, None).is_none());
        let st = State { queue: vec![Job { name: "Q-GRP".into(), ..Default::default() }], history: vec![Done { name: "H-GRP".into(), ok: true, ..Default::default() }], ..Default::default() };
        assert!(already_seen("Q-GRP", &[], Some(&st)).is_some() && already_seen("H-GRP", &[], Some(&st)).is_some());
        assert!(already_seen("Neuf-GRP", &reps, Some(&st)).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    /// Dossier de NZB surveillé : sous-dossier = indexer d'origine, NZB pris quand sa taille
    /// ne bouge plus, puis rangé.
    #[test]
    fn dossier_de_nzb_surveille() {
        let root = std::env::temp_dir().join(format!("nzbff-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // deux passages suffisent (délai de stabilité testé à part)
        let mut cfg = Config { dl_watch: root.display().to_string(), dl_watch_stable: 0, ..Default::default() };
        cfg.indexers = vec![prep::IndexerCfg::new("altHUB"), prep::IndexerCfg { active: false, ..prep::IndexerCfg::new("Vieux") }];
        cfg.indexers[0].active = true;
        ensure_watch(&cfg).unwrap();
        assert!(root.join("altHUB").is_dir() && root.join(WATCH_ELSEWHERE).is_dir() && root.join(WATCH_DL_ONLY).is_dir());
        assert!(!root.join("Vieux").exists());
        fs::create_dir_all(root.join("Vieux")).unwrap();
        for (d, n) in [("altHUB", "a.nzb"), (WATCH_ELSEWHERE, "b.nzb"), (WATCH_DL_ONLY, "c.nzb"), ("Vieux", "d.nzb"), ("", "e.NZB"), ("", "f.txt"), (WATCH_DONE, "g.nzb")] {
            fs::create_dir_all(root.join(d)).unwrap();
            fs::write(root.join(d).join(n), "<nzb/>").unwrap();
        }
        let mut seen = std::collections::HashMap::new();
        assert!(scan_watch(&cfg, &mut seen).is_empty(), "taille pas encore stable");
        let got = scan_watch(&cfg, &mut seen);
        let kind = |n: &str| got.iter().find(|(p, _)| prep::file_name(p) == n).map(|(_, k)| (k.action.clone(), k.origin.clone()));
        assert_eq!(got.len(), 5);
        assert_eq!(kind("a.nzb"), Some(("repost".into(), "altHUB".into())));
        assert_eq!(kind("b.nzb"), Some(("repost".into(), String::new())));
        assert_eq!(kind("c.nzb"), Some(("dl".into(), String::new())));
        assert_eq!(kind("d.nzb"), Some(("repost".into(), "Vieux".into())));
        assert_eq!(kind("e.NZB"), Some(("open".into(), String::new())));
        // fichier en cours d'écriture : attendu au passage suivant (les autres, pris en
        // charge, sont rangés ; a et c servent plus bas)
        for (p, _) in &got {
            if !["a.nzb", "c.nzb"].contains(&prep::file_name(p).as_str()) {
                fs::remove_file(p).unwrap();
            }
        }
        let mut seen = std::collections::HashMap::new();
        fs::write(root.join("h.nzb"), "<n").unwrap();
        assert!(scan_watch(&cfg, &mut seen).is_empty());
        fs::write(root.join("h.nzb"), "<nzb/>").unwrap();
        assert_eq!(scan_watch(&cfg, &mut seen).len(), 2, "a et c stables ; h vient de changer");
        let last = scan_watch(&cfg, &mut seen);
        assert!(last.iter().any(|(p, k)| prep::file_name(p) == "h.nzb" && k.action == "open"));
        // rangement
        let a = root.join("altHUB").join("a.nzb");
        let d1 = finish_watch(&cfg, &a, None).unwrap();
        assert!(!a.exists() && d1 == root.join(WATCH_DONE).join("a.nzb"));
        fs::write(&a, "x").unwrap();
        assert_eq!(finish_watch(&cfg, &a, None).unwrap(), root.join(WATCH_DONE).join("a (2).nzb"));
        let c = root.join(WATCH_DL_ONLY).join("c.nzb");
        let e = finish_watch(&cfg, &c, Some("client injoignable")).unwrap();
        assert!(e.starts_with(root.join(WATCH_ERR)));
        assert_eq!(fs::read_to_string(e.with_extension("erreur.txt")).unwrap().trim(), "client injoignable");
        // réglages par défaut
        cfg.dl_origin = "*".into();
        assert_eq!(default_origin(&cfg), Some(String::new()));
        cfg.dl_origin = "althub".into();
        assert_eq!(default_origin(&cfg), Some("altHUB".into()));
        cfg.dl_origin = "Vieux".into();
        assert_eq!(default_origin(&cfg), None, "indexer inactif : on redemande");
        cfg.dl_nfo = "keep".into();
        cfg.dl_mi = false;
        cfg.dl_after = "copy".into();
        assert_eq!(default_opts(&cfg), RepostOpts { nfo: "keep".into(), mi: false, cats: vec![], after: "copy".into(), reports: String::new() });
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn json_des_clients() {
        let q = J::parse(r#"{"queue":{"paused":false,"kbpersec":"51200.0","mbleft":"1024","slots":[{"nzo_id":"SABnzbd_nzo_1","filename":"Film.2020","percentage":"73","mb":"4096","mbleft":"1105.9","status":"Downloading","timeleft":"0:05:12","cat":"nzbff"}]}}"#).unwrap();
        let q = q.get("queue");
        assert_eq!(q.get("kbpersec").f() * 1024.0, 50.0 * 1024.0 * 1024.0);
        assert_eq!(q.get("slots").arr()[0].get("filename").s(), "Film.2020");
        assert_eq!(q.get("slots").arr()[0].get("percentage").f(), 73.0);
        let h = J::parse(r#"{"history":{"slots":[{"nzo_id":"x","name":"Caf\u00e9 \"A\"","status":"Completed","bytes":1234,"storage":"/data/complete/A","completed":1700000000,"fail_message":""}]}}"#).unwrap();
        let s = &h.get("history").get("slots").arr()[0];
        assert_eq!(s.get("name").s(), "Café \"A\"");
        assert_eq!(s.get("bytes").f() as u64, 1234);
        assert_eq!(J::parse("[1, -2.5e1, true, null, \"\\ud83d\\ude00\"]").unwrap(), J::Arr(vec![J::Num(1.0), J::Num(-25.0), J::Bool(true), J::Null, J::Str("😀".into())]));
        assert!(J::parse("{\"a\":}").is_none());
        assert_eq!(jstr("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
        assert_eq!(b64(b"Man"), "TWFu");
        assert_eq!(b64(b"Ma"), "TWE=");
    }

    #[test]
    fn chemins_et_reposts() {
        let cfg = Config { dl_path_map: "/data/complete=D:\\Usenet\\Fini".into(), ..Default::default() };
        let p = local_path(&cfg, "/data/complete/Film.2020");
        if cfg!(windows) {
            assert_eq!(p, PathBuf::from("D:\\Usenet\\Fini\\Film.2020"));
        } else {
            assert_eq!(p, PathBuf::from("D:\\Usenet\\Fini/Film.2020"));
        }
        assert_eq!(local_path(&cfg, "/autre/x"), PathBuf::from(if cfg!(windows) { "\\autre\\x" } else { "/autre/x" }));
        // jamais reposté là où il a été trouvé
        assert_eq!(repost_targets("altHUB", &["altHUB".into(), "NZBFrance".into()]), vec!["NZBFrance".to_string()]);
        let d = std::env::temp_dir().join(format!("nzbff_repost_{}.tsv", std::process::id()));
        let opts = RepostOpts { nfo: "keep".into(), mi: false, cats: vec![("NZBFrance".into(), "2042".into())], after: "copy".into(), reports: String::new() };
        let v = vec![Repost { id: "SAB_1".into(), release: "Film.2020".into(), source: "altHUB".into(), targets: vec!["NZBFrance".into()], added: 5, state: "download".into(), auto: true, note: String::new(), opts, out: vec!["Film.2020.FRENCH.1080p".into()] }];
        save_reposts_to(&d, &v);
        assert_eq!(load_reposts_from(&d), v);
        let st = State { history: vec![Done { id: "SAB_1".into(), name: "Film.2020".into(), ok: true, path: "/x".into(), ..Default::default() }], ..Default::default() };
        assert!(follow(&v[0], &st).unwrap().0);
        // ancien fichier (8 colonnes) : options par défaut
        fs::write(&d, "SAB_2\tX\t\tA\t1\tdone\tauto\tnote\n").unwrap();
        let old = load_reposts_from(&d);
        assert_eq!((old[0].opts.clone(), old[0].out.len()), (RepostOpts::default(), 0));
        // options appliquées au traitement
        let mut c = Config { indexers: vec![crate::prep::IndexerCfg { send_mi: true, ..crate::prep::IndexerCfg::new("NZBFrance") }], ..Default::default() };
        v[0].opts.apply(&mut c);
        assert!(!c.indexers[0].send_mi && !c.nfo_create && c.nfo_send && c.dl_after == "copy" && c.cat_force == vec![("NZBFrance".to_string(), "2042".to_string())]);
        let _ = fs::remove_file(&d);
    }

    /// Petit serveur HTTP local qui répond comme SABnzbd / NZBGet : requêtes réelles par curl.
    pub(crate) fn fake_server(answer: fn(&str, &str) -> String) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", l.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let mut c = c;
                let mut buf = Vec::new();
                let mut tmp = [0u8; 65536];
                // en-têtes, puis le corps (Content-Length)
                loop {
                    let n = c.read(&mut tmp).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    let t = String::from_utf8_lossy(&buf).to_string();
                    if let Some(h) = t.find("\r\n\r\n") {
                        let len = t[..h].lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if buf.len() >= h + 4 + len {
                            break;
                        }
                    }
                }
                let t = String::from_utf8_lossy(&buf).to_string();
                let head = t.lines().next().unwrap_or("").to_string();
                s2.lock().unwrap().push(t.clone());
                let body = answer(&head, &t);
                // « 405 {…} » : réponse avec ce code
                let (code, body) = match body.split_once(' ').filter(|(c, _)| c.len() == 3 && c.chars().all(|x| x.is_ascii_digit())) {
                    Some((c, b)) => (c.to_string(), b.to_string()),
                    None => ("200".to_string(), body),
                };
                let _ = write!(c, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (addr, seen)
    }

    /// Règles du dossier surveillé : défauts, modification (et retour au défaut), sous-dossier
    /// ajouté, « ignorer », ligne de réglage aller-retour, délai de stabilité.
    #[test]
    fn regles_du_dossier_surveille() {
        let _verrou = prep::test_lock();
        let root = std::env::temp_dir().join(format!("nzbff-routes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mut cfg = Config { dl_watch: root.display().to_string(), dl_watch_stable: 10, ..Default::default() };
        cfg.indexers = vec![prep::IndexerCfg::new("altHUB"), prep::IndexerCfg::new("NZBFrance")];
        for i in &mut cfg.indexers {
            i.active = true;
        }
        let r = watch_routes(&cfg);
        let names: Vec<(&str, RouteKind)> = r.iter().map(|(x, k)| (x.folder.as_str(), *k)).collect();
        assert_eq!(names, vec![("", RouteKind::Root), ("altHUB", RouteKind::Indexer), ("NZBFrance", RouteKind::Indexer), (WATCH_ELSEWHERE, RouteKind::Elsewhere), (WATCH_DL_ONLY, RouteKind::DlOnly)]);
        assert_eq!(r[1].0.repost_to(&cfg), vec!["NZBFrance".to_string()]);
        // racine : télécharger seulement, catégorie et priorité à elle
        let mut root_r = r[0].0.clone();
        root_r.action = "dl".into();
        root_r.add = AddOpts { cat: "films".into(), prio: "high".into(), paused: false };
        set_route(&mut cfg, root_r.clone(), RouteKind::Root);
        // sous-dossier ajouté : repost seulement sur altHUB, à la main
        let custom = WatchRoute { folder: "Séries 4K".into(), action: "repost".into(), origin: String::new(), targets: vec!["altHUB".into()], add: AddOpts { cat: "tv4k".into(), prio: "low".into(), paused: true }, when: "manual".into() };
        set_route(&mut cfg, custom.clone(), RouteKind::Custom);
        // dossier d'indexer ignoré
        let mut nf = r[2].0.clone();
        nf.action = "off".into();
        set_route(&mut cfg, nf, RouteKind::Indexer);
        // modifié puis remis comme avant : oublié
        let mut el = r[3].0.clone();
        el.add.cat = "x".into();
        set_route(&mut cfg, el.clone(), RouteKind::Elsewhere);
        el.add.cat.clear();
        set_route(&mut cfg, el, RouteKind::Elsewhere);
        assert_eq!(cfg.dl_routes.len(), 3, "{:?}", cfg.dl_routes);
        // relu depuis le fichier de réglages
        let f = root.with_extension("conf");
        cfg.save(&f).unwrap();
        let back = Config::load(&f);
        assert_eq!(back.dl_routes, cfg.dl_routes);
        assert_eq!(back.dl_watch_stable, 10);
        let rr = watch_routes(&back);
        assert_eq!(rr[0].0, root_r);
        assert_eq!(rr.last().unwrap().0, custom);
        assert_eq!(rr.last().unwrap().1, RouteKind::Custom);
        assert!(!custom.auto(&back) && custom.repost_to(&back) == vec!["altHUB".to_string()]);
        assert!(route_for(&back, "NZBFrance").is_none(), "règle « ignorer »");
        assert!(route_for(&back, WATCH_DONE).is_none() && route_for(&back, "_Autre").is_none());
        // dossiers : règles créées, dossier ignoré non, mode d'emploi qui les reprend
        ensure_watch(&back).unwrap();
        assert!(root.join("Séries 4K").is_dir() && root.join("altHUB").is_dir() && !root.join("NZBFrance").exists());
        let help = fs::read_to_string(root.join("À LIRE.txt")).unwrap();
        assert!(help.contains("Séries 4K") && help.contains("tv4k") && help.contains("ajouté en pause"), "{help}");
        assert!(!watch_dirs_missing(&back));
        // stabilité : 10 s de taille identique
        fs::write(root.join("Séries 4K").join("S.S01.nzb"), "<nzb/>").unwrap();
        fs::write(root.join("x.nzb"), "<nzb/>").unwrap();
        let mut seen = std::collections::HashMap::new();
        assert!(scan_watch_at(&back, &mut seen, 1000).is_empty());
        assert!(scan_watch_at(&back, &mut seen, 1005).is_empty(), "5 s seulement");
        let got = scan_watch_at(&back, &mut seen, 1010);
        assert_eq!(got.len(), 2);
        let s4k = got.iter().find(|(p, _)| prep::file_name(p) == "S.S01.nzb").unwrap();
        assert_eq!(s4k.1, custom);
        let x = got.iter().find(|(p, _)| prep::file_name(p) == "x.nzb").unwrap();
        assert_eq!(x.1.action, "dl");
        // noms de sous-dossier
        assert_eq!(clean_folder(" Films: 4K?. "), "Films 4K");
        assert!(is_reserved("_pris en charge"));
        // sans sous-dossiers d'indexer
        let mut no = back.clone();
        no.dl_watch_subdirs = false;
        assert!(watch_routes(&no).iter().all(|(_, k)| *k != RouteKind::Indexer));
        assert!(route_for(&no, "altHUB").is_none());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(&f);
    }

    #[test]
    fn sabnzbd_par_son_api() {
        let (url, seen) = fake_server(|head, _| {
            if head.contains("mode=queue") {
                r#"{"queue":{"paused":false,"kbpersec":"51200","mbleft":"100","slots":[{"nzo_id":"SABnzbd_nzo_1","filename":"Film.2020","percentage":"40","mb":"200","mbleft":"100","status":"Downloading","timeleft":"0:01:00","cat":"nzbff"}]}}"#.into()
            } else if head.contains("mode=history") {
                r#"{"history":{"slots":[{"nzo_id":"SABnzbd_nzo_0","name":"Ancien","status":"Completed","bytes":2048,"storage":"/data/complete/Ancien","completed":1700000000,"fail_message":"","category":"nzbff"}]}}"#.into()
            } else if head.contains("mode=version") {
                r#"{"version":"4.3.3"}"#.into()
            } else if head.contains("mode=warnings") {
                r#"{"warnings":[{"text":"disque presque plein","type":"WARNING","time":1}]}"#.into()
            } else if head.contains("mode=addfile") || head.contains("mode=addurl") {
                r#"{"status":true,"nzo_ids":["SABnzbd_nzo_2"]}"#.into()
            } else {
                r#"{"status":true}"#.into()
            }
        });
        let cfg = Config { dl_client: "sabnzbd".into(), dl_url: url, dl_key: "CLEF123".into(), dl_cat: "nzbff".into(), ..Default::default() };
        let st = poll(&cfg).unwrap();
        assert_eq!((st.version.as_str(), st.queue.len(), st.history.len()), ("4.3.3", 1, 1));
        assert_eq!(st.speed_bps, 50.0 * 1024.0 * 1024.0);
        assert_eq!(st.queue[0].pct, 40.0);
        assert!(st.history[0].ok && st.history[0].path == "/data/complete/Ancien");
        assert_eq!(st.warnings, vec!["WARNING disque presque plein".to_string()]);
        let nzb = std::env::temp_dir().join(format!("nzbff_dl_{}.nzb", std::process::id()));
        fs::write(&nzb, "<nzb></nzb>").unwrap();
        assert_eq!(add_with(&cfg, &nzb.display().to_string(), "Film.2020.FRENCH.1080p", &AddOpts::default()).unwrap(), "SABnzbd_nzo_2");
        // catégorie et priorité d'une règle, adresse sans nom (téléchargement seul)
        add_with(&cfg, &nzb.display().to_string(), "Autre.2021", &AddOpts { cat: "films4k".into(), prio: "high".into(), paused: false }).unwrap();
        add_with(&cfg, "https://idx.example/getnzb/42", "", &AddOpts { paused: true, ..Default::default() }).unwrap();
        set_paused(&cfg, true).unwrap();
        job_action(&cfg, "SABnzbd_nzo_1", "delete").unwrap();
        let reqs = seen.lock().unwrap().clone();
        let add_req = reqs.iter().find(|r| r.contains("mode=addfile")).unwrap();
        assert!(add_req.contains("apikey=CLEF123") && add_req.contains("nzbname=Film.2020.FRENCH.1080p") && add_req.contains("cat=nzbff") && add_req.contains("<nzb></nzb>"), "{add_req}");
        assert!(!add_req.contains("priority="), "priorité du client gardée par défaut : {add_req}");
        let r2 = reqs.iter().find(|r| r.contains("nzbname=Autre.2021")).unwrap();
        assert!(r2.contains("cat=films4k") && r2.contains("priority=1"), "{r2}");
        let r3 = reqs.iter().find(|r| r.contains("mode=addurl")).unwrap();
        assert!(r3.contains("priority=-2") && !r3.contains("nzbname=") && r3.contains("cat=nzbff"), "{r3}");
        assert!(reqs.iter().any(|r| r.starts_with("GET /api?output=json&apikey=CLEF123&mode=pause")));
        assert!(reqs.iter().any(|r| r.contains("mode=queue&name=delete&value=SABnzbd_nzo_1")));
        let _ = fs::remove_file(&nzb);
    }

    #[test]
    fn nzbget_par_json_rpc() {
        let (url, seen) = fake_server(|_, req| {
            let m = |x: &str| req.contains(&format!("\"method\":\"{x}\""));
            if m("status") {
                r#"{"version":"1.1","result":{"DownloadRate":1048576,"DownloadPaused":false,"RemainingSizeMB":10}}"#.into()
            } else if m("version") {
                r#"{"result":"24.1"}"#.into()
            } else if m("listgroups") {
                r#"{"result":[{"NZBID":7,"NZBName":"Serie.S01","Category":"nzbff","FileSizeMB":100,"RemainingSizeMB":25,"Status":"DOWNLOADING"}]}"#.into()
            } else if m("history") {
                r#"{"result":[{"NZBID":6,"Name":"Vieux","Status":"SUCCESS/ALL","FileSizeMB":1,"DestDir":"/dl/Vieux","HistoryTime":1700000000,"Category":""}]}"#.into()
            } else if m("append") {
                r#"{"result":8}"#.into()
            } else {
                r#"{"result":true}"#.into()
            }
        });
        let cfg = Config { dl_client: "nzbget".into(), dl_url: url, dl_user: "nzbget".into(), dl_pass: "secret".into(), ..Default::default() };
        let st = poll(&cfg).unwrap();
        assert_eq!((st.version.as_str(), st.speed_bps), ("24.1", 1048576.0));
        assert_eq!(st.queue[0].pct, 75.0);
        assert!(st.history[0].ok);
        let nzb = std::env::temp_dir().join(format!("nzbff_dl2_{}.nzb", std::process::id()));
        fs::write(&nzb, "Man").unwrap();
        assert_eq!(add_with(&cfg, &nzb.display().to_string(), "Serie.S01", &AddOpts::default()).unwrap(), "8");
        add_with(&cfg, &nzb.display().to_string(), "Autre.S02", &AddOpts { cat: "series".into(), prio: "force".into(), paused: true }).unwrap();
        let reqs = seen.lock().unwrap().clone();
        let ap = reqs.iter().find(|r| r.contains("\"append\"") && r.contains("Serie.S01")).unwrap();
        // contenu du NZB en base64, identifiants en en-tête (Basic nzbget:secret)
        assert!(ap.contains("\"TWFu\"") && ap.contains("\"Serie.S01.nzb\"") && ap.contains("Authorization: Basic bnpiZ2V0OnNlY3JldA=="), "{ap}");
        assert!(ap.contains("\"nzbff\",0,false,false,"), "catégorie des réglages, priorité normale, pas en pause : {ap}");
        let ap2 = reqs.iter().find(|r| r.contains("Autre.S02")).unwrap();
        assert!(ap2.contains("\"series\",900,false,true,"), "{ap2}");
        let _ = fs::remove_file(&nzb);
    }
}