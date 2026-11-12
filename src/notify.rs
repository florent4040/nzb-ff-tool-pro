//! Notification par e-mail quand tous les traitements sont terminés (copies, posts, dépôts
//! sur les indexers), avec le résumé de ce qui s'est passé.
//!
//! Envoi par SMTP avec curl (fourni avec Windows 10 et 11) : SSL direct (port 465) ou
//! STARTTLS (port 587). Le mot de passe passe par un fichier de configuration curl temporaire,
//! jamais par la ligne de commande. Le suivi tourne dans un fil à part : il fonctionne aussi
//! fenêtre fermée (zone de notification) et en mode --watch.

use crate::prep::{self, Config};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Fournisseurs courants : (nom, serveur, port, sécurité, conseil).
pub const PRESETS: &[(&str, &str, u16, &str, &str)] = &[
    ("Gmail", "smtp.gmail.com", 465, "ssl", "mot de passe d'application (compte Google → Sécurité → Validation en deux étapes → Mots de passe des applications)"),
    ("Proton Mail", "smtp.protonmail.ch", 587, "starttls", "jeton SMTP (Proton → Paramètres → IMAP/SMTP ; offres payantes, adresse de domaine personnalisé)"),
    ("Yahoo", "smtp.mail.yahoo.com", 465, "ssl", "mot de passe d'application Yahoo"),
    ("iCloud", "smtp.mail.me.com", 587, "starttls", "mot de passe pour app (identifiant Apple)"),
    ("Orange", "smtp.orange.fr", 465, "ssl", "mot de passe de la messagerie Orange"),
    ("Free", "smtp.free.fr", 465, "ssl", "mot de passe de la messagerie Free"),
    ("GMX", "mail.gmx.net", 587, "starttls", "activer l'accès POP3/IMAP dans les paramètres GMX"),
];

// ---------------------------------------------------------------- suivi

/// Traitements en cours (copies, posts, dépôts).
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn journal() -> &'static Mutex<Vec<String>> {
    static J: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    J.get_or_init(|| Mutex::new(Vec::new()))
}

/// Un traitement en cours ; terminé quand la garde est relâchée.
pub struct Busy;

impl Drop for Busy {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn busy() -> Busy {
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    Busy
}

pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}

/// Travail encore attendu sans traitement en cours : posts en file, dépôts en file, releases
/// copiées dont le NZB n'est pas encore là (NgPost en surveillance de dossier, post en cours).
/// Tenu à jour par le suivi des releases ; le résumé attend qu'il retombe à zéro, sinon un
/// premier e-mail partait après le post et un second après le dépôt sur l'indexer.
static PENDING: AtomicUsize = AtomicUsize::new(0);

pub fn set_pending(n: usize) {
    PENDING.store(n, Ordering::SeqCst);
}

pub fn pending() -> usize {
    PENDING.load(Ordering::SeqCst) + PENDING_DL.load(Ordering::SeqCst)
}

/// Releases téléchargées pour être repostées, pas encore repostées : le résumé les attend.
static PENDING_DL: AtomicUsize = AtomicUsize::new(0);

pub fn set_pending_dl(n: usize) {
    PENDING_DL.store(n, Ordering::SeqCst);
}

/// Déjà envoyé (même contenu, il y a moins de 30 min, par cette fenêtre ou une autre ouverte
/// en même temps) ? Sinon, le note.
fn already_sent(lines: &[String]) -> bool {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    lines.hash(&mut h);
    let key = format!("{:016x}", h.finish());
    let f = crate::app_dir().join(".nzbff-mail-dernier");
    let now = prep::now_secs();
    if let Ok(t) = std::fs::read_to_string(&f) {
        if let Some((k, at)) = t.trim().split_once(' ') {
            if k == key && now.saturating_sub(at.parse().unwrap_or(0)) < 1800 {
                return true;
            }
        }
    }
    let _ = std::fs::write(&f, format!("{key} {now}"));
    false
}

/// Ligne du résumé envoyé à la fin (« ✔ Posté : … », « ✖ Échec : … »).
pub fn record(line: impl Into<String>) {
    let l = crate::history::local_time(prep::now_secs());
    if let Ok(mut j) = journal().lock() {
        j.push(format!("{:02}:{:02}  {}", l.hour, l.minute, prep::redact(&line.into())));
        if j.len() > 500 {
            j.drain(0..100);
        }
    }
}

/// Fil de suivi : quand des traitements ont eu lieu et que tout est terminé depuis
/// `quiet` (pas de nouveau traitement entre deux vidéos de l'inbox), envoie le résumé.
pub fn spawn(shared: Arc<Mutex<Config>>, on_sent: Box<dyn Fn(Result<String, String>) + Send>) {
    std::thread::spawn(move || {
        let quiet = Duration::from_secs(45);
        let mut idle_since: Option<Instant> = None;
        // garde-fou : une release coincée (NgPost arrêté…) ne retient pas le résumé plus de 6 h
        let mut waiting_since: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_secs(3));
            let has_news = journal().lock().map(|j| !j.is_empty()).unwrap_or(false);
            let waiting = pending() > 0 && waiting_since.get_or_insert_with(Instant::now).elapsed() < Duration::from_secs(6 * 3600);
            if pending() == 0 {
                waiting_since = None;
            }
            if active() > 0 || waiting || !has_news {
                idle_since = None;
                continue;
            }
            let t = *idle_since.get_or_insert_with(Instant::now);
            if t.elapsed() < quiet {
                continue;
            }
            idle_since = None;
            let lines: Vec<String> = journal().lock().map(|mut j| std::mem::take(&mut *j)).unwrap_or_default();
            let cfg = shared.lock().map(|c| c.clone()).unwrap_or_default();
            if lines.is_empty() || !cfg.mail_enabled || cfg.mail_to.trim().is_empty() || already_sent(&lines) {
                continue;
            }
            let (subject, body) = summary(&cfg, &lines);
            on_sent(send(&cfg, &subject, &body));
        }
    });
}

/// Modèles par défaut de l'objet et du texte.
pub const DEFAULT_SUBJECT: &str = "{app} : traitements terminés ({bilan})";
pub const DEFAULT_BODY: &str = "Bonjour,\n\nTous les traitements en cours sont terminés : {bilan}.\n\n{resume}\n\n{app} {version} sur {machine}";

/// Champs utilisables dans l'objet et le texte : (champ, explication).
pub const FIELDS: &[(&str, &str)] = &[
    ("{bilan}", "« 3 réussi(s), 1 échec(s) »"),
    ("{reussis}", "nombre de réussites"),
    ("{echecs}", "nombre d'échecs"),
    ("{total}", "nombre de lignes du résumé"),
    ("{resume}", "le détail, une ligne par traitement"),
    ("{reussites}", "seulement les lignes réussies"),
    ("{erreurs}", "seulement les échecs (vide s'il n'y en a pas)"),
    ("{date}", "date du jour (03/11/2026)"),
    ("{heure}", "heure de l'envoi (14:05)"),
    ("{machine}", "nom du PC"),
    ("{app}", "NZB FF Tool Pro"),
    ("{version}", "version de l'application"),
];

/// Remplace les {champs} d'un modèle.
pub fn render(tpl: &str, lines: &[String]) -> String {
    let ok: Vec<&String> = lines.iter().filter(|l| l.contains("✔")).collect();
    let ko: Vec<&String> = lines.iter().filter(|l| l.contains("✖")).collect();
    let bilan = match (ok.len(), ko.len()) {
        (n, 0) => format!("{n} réussi(s)"),
        (0, k) => format!("{k} échec(s)"),
        (n, k) => format!("{n} réussi(s), {k} échec(s)"),
    };
    let join = |v: &[&String]| v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
    let l = crate::history::local_time(prep::now_secs());
    let machine = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "ce PC".into());
    let pairs: [(&str, String); 12] = [
        ("{bilan}", bilan),
        ("{reussis}", ok.len().to_string()),
        ("{echecs}", ko.len().to_string()),
        ("{total}", lines.len().to_string()),
        ("{resume}", lines.join("\n")),
        ("{reussites}", join(&ok)),
        ("{erreurs}", join(&ko)),
        ("{date}", format!("{:02}/{:02}/{}", l.day, l.month, l.year)),
        ("{heure}", format!("{:02}:{:02}", l.hour, l.minute)),
        ("{machine}", machine),
        ("{app}", crate::APP_NAME.to_string()),
        ("{version}", crate::VERSION.to_string()),
    ];
    let mut out = tpl.to_string();
    for (k, v) in pairs {
        // sans accent ou avec : {réussis}, {échecs}
        out = out.replace(k, &v);
        let accented = k.replace("reussi", "réussi").replace("echecs", "échecs");
        if accented != k {
            out = out.replace(&accented, &v);
        }
    }
    out
}

/// Objet et texte du résumé, d'après les modèles des réglages.
pub fn summary(cfg: &Config, lines: &[String]) -> (String, String) {
    let st = if cfg.mail_subject.trim().is_empty() { DEFAULT_SUBJECT } else { cfg.mail_subject.trim() };
    let bt = if cfg.mail_body.trim().is_empty() { DEFAULT_BODY } else { cfg.mail_body.as_str() };
    let subject = render(st, lines).replace(['\r', '\n'], " ");
    let mut body = render(bt, lines);
    if !body.ends_with('\n') {
        body.push('\n');
    }
    (subject, body)
}

/// Lignes d'exemple pour l'aperçu et l'e-mail de test.
pub fn sample_lines() -> Vec<String> {
    vec![
        "14:02  ✔ Posté : Ma.Serie.S01.MULTi.1080p.WEB.H264-NZBFF".into(),
        "14:05  ✔ Déposé sur MonIndexer : Ma.Serie.S01.MULTi.1080p.WEB.H264-NZBFF".into(),
        "14:31  ✖ Échec du post : Un.Film.2024.FRENCH.1080p.BluRay.x264-NZBFF (espace insuffisant)".into(),
    ]
}

/// Fournisseurs proposés + ceux ajoutés dans les réglages : (nom, serveur, port, sécurité, conseil).
pub fn providers(cfg: &Config) -> Vec<(String, String, u16, String, String)> {
    let mut v: Vec<_> = PRESETS.iter().map(|(n, s, p, sec, h)| (n.to_string(), s.to_string(), *p, sec.to_string(), h.to_string())).collect();
    for m in &cfg.mail_providers {
        v.push((m.name.clone(), m.server.clone(), m.port, m.security.clone(), String::new()));
    }
    v
}

// ------------------------------------------------------------------ envoi

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// En-tête encodé (accents dans l'objet).
fn header_word(s: &str) -> String {
    if s.is_ascii() {
        s.to_string()
    } else {
        format!("=?UTF-8?B?{}?=", b64(s.as_bytes()))
    }
}

/// Date au format des e-mails (« Tue, 03 Nov 2026 14:05:00 +0100 »).
fn mail_date() -> String {
    let now = prep::now_secs() as i64;
    let off = crate::stats::local_offset(now as u64);
    let t = now + off;
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // jour de la semaine : 1970-01-01 était un jeudi
    let wd = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][days.rem_euclid(7) as usize];
    let (y, m, d) = civil(days);
    let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][(m - 1) as usize];
    let sign = if off < 0 { '-' } else { '+' };
    let o = off.abs();
    format!("{wd}, {d:02} {mon} {y} {:02}:{:02}:{:02} {sign}{:02}{:02}", secs / 3600, (secs / 60) % 60, secs % 60, o / 3600, (o / 60) % 60)
}

/// Jours depuis 1970 → (année, mois, jour).
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Destinataires (séparés par virgule, point-virgule ou espace).
/// Une même adresse écrite deux fois n'est gardée qu'une fois (sinon deux e-mails).
pub fn recipients(s: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for x in s.split([',', ';', ' ']).map(str::trim).filter(|x| x.contains('@')) {
        if !v.iter().any(|y| y.eq_ignore_ascii_case(x)) {
            v.push(x.to_string());
        }
    }
    v
}

/// Gravité d'une ligne du résumé : 2 erreur, 1 alerte, 0 réussite, -1 neutre.
pub fn severity(line: &str) -> i8 {
    let l = line.to_lowercase();
    if line.contains('✖') || l.contains("échec") || l.contains("refusé") || l.contains("erreur") {
        2
    } else if line.contains('⚠') || line.contains('⏭') || l.contains("alerte") || l.contains("non envoyé") || l.contains("non joint") {
        1
    } else if line.contains('✔') {
        0
    } else {
        -1
    }
}

/// Gravité de tout le résumé (la pire ligne).
pub fn overall(lines: &[&str]) -> i8 {
    lines.iter().map(|l| severity(l)).max().unwrap_or(-1).max(0)
}

fn html_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Version HTML du résumé : bandeau rouge (erreur), jaune (alertes sans conséquence) ou vert
/// (tout est bon), puis chaque ligne avec la couleur de sa gravité. Styles en ligne : lisible
/// dans Outlook, Gmail, Thunderbird et les clients mobiles.
pub fn html_body(subject: &str, body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let worst = overall(&lines);
    let (band, band_bg, title) = match worst {
        2 => ("#b42318", "#fee4e2", "Des erreurs demandent ton attention"),
        1 => ("#b54708", "#fef0c7", "Terminé, avec des alertes sans conséquence"),
        _ => ("#067647", "#dcfae6", "Tout s'est bien passé"),
    };
    let mut rows = String::new();
    let mut last: i8 = -1;
    for l in &lines {
        if l.trim().is_empty() {
            rows.push_str("<tr><td style=\"height:8px\"></td></tr>\n");
            continue;
        }
        // ligne de détail (indentée) : couleur de la ligne au-dessus, plus petite
        let detail = l.starts_with("  ");
        let sev = if detail { last } else { severity(l) };
        if !detail {
            last = sev;
        }
        let (fg, bg, bar) = match sev {
            2 => ("#b42318", "#fef3f2", "#f04438"),
            1 => ("#93370d", "#fffaeb", "#f79009"),
            0 => ("#05603a", "#f6fef9", "#12b76a"),
            _ => ("#344054", "#ffffff", "#d0d5dd"),
        };
        let size = if detail { "12px" } else { "14px" };
        rows.push_str(&format!(
            "<tr><td style=\"padding:6px 12px;border-left:4px solid {bar};background:{bg};color:{fg};font-size:{size};font-family:Segoe UI,Arial,sans-serif;line-height:1.45\">{}</td></tr>\n",
            html_esc(l.trim())
        ));
    }
    format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><meta name=\"color-scheme\" content=\"light\"></head>\n<body style=\"margin:0;padding:24px;background:#f2f4f7\">\n<table role=\"presentation\" width=\"100%\" cellspacing=\"0\" cellpadding=\"0\" style=\"max-width:680px;margin:0 auto;background:#ffffff;border-radius:10px;overflow:hidden;border:1px solid #eaecf0\">\n<tr><td style=\"padding:16px 20px;background:{band_bg};border-bottom:3px solid {band};font-family:Segoe UI,Arial,sans-serif\"><div style=\"font-size:12px;color:{band};letter-spacing:.3px\">{}</div><div style=\"font-size:18px;font-weight:600;color:{band};margin-top:2px\">{title}</div><div style=\"font-size:13px;color:#475467;margin-top:4px\">{}</div></td></tr>\n<tr><td style=\"padding:12px 16px\"><table role=\"presentation\" width=\"100%\" cellspacing=\"0\" cellpadding=\"0\" style=\"border-collapse:separate;border-spacing:0 4px\">\n{rows}</table></td></tr>\n<tr><td style=\"padding:10px 20px 16px;font-size:11px;color:#98a2b3;font-family:Segoe UI,Arial,sans-serif\">{} {}</td></tr>\n</table>\n</body></html>\n",
        html_esc(crate::APP_NAME),
        html_esc(subject),
        html_esc(crate::APP_NAME),
        html_esc(crate::VERSION)
    )
}

/// Texte en base64, lignes de 76 caractères (sûr pour tout serveur SMTP).
fn b64_lines(data: &[u8]) -> String {
    let b = b64(data);
    let mut out = String::with_capacity(b.len() + b.len() / 76 * 2 + 2);
    for c in b.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(c).unwrap_or(""));
        out.push_str("\r\n");
    }
    out
}

/// Message complet (en-têtes + texte, et version HTML en couleur si réglé), lignes en CRLF.
pub fn message(cfg: &Config, subject: &str, body: &str) -> String {
    let from = if cfg.mail_from.trim().is_empty() { cfg.mail_user.trim() } else { cfg.mail_from.trim() };
    let to = recipients(&cfg.mail_to).join(", ");
    let id = format!("<{}.{}@nzbff.local>", prep::now_secs(), std::process::id());
    let head = format!("From: {} <{from}>\r\nTo: {to}\r\nSubject: {}\r\nDate: {}\r\nMessage-ID: {id}\r\nMIME-Version: 1.0\r\n", header_word(crate::APP_NAME), header_word(subject), mail_date());
    let mut text = String::new();
    for l in body.lines() {
        // une ligne commençant par un point serait mal comprise par le serveur
        if l.starts_with('.') {
            text.push('.');
        }
        text.push_str(l);
        text.push_str("\r\n");
    }
    if !cfg.mail_html {
        return format!("{head}Content-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{text}");
    }
    let boundary = format!("nzbff-{}-{}", prep::now_secs(), std::process::id());
    format!(
        "{head}Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\nCe message est au format MIME.\r\n\r\n--{boundary}\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{text}\r\n--{boundary}\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n--{boundary}--\r\n",
        b64_lines(html_body(subject, body).as_bytes())
    )
}

/// Arguments curl de l'envoi (le mot de passe est dans le fichier `-K`).
pub fn curl_args(cfg: &Config, msg_file: &str, conf_file: &str) -> Vec<String> {
    let host = cfg.mail_server.trim();
    let port = if cfg.mail_port == 0 { if cfg.mail_security == "ssl" { 465 } else { 587 } } else { cfg.mail_port };
    // le chemin de l'adresse donne le nom présenté au serveur (EHLO)
    let url = if cfg.mail_security == "ssl" { format!("smtps://{host}:{port}/localhost") } else { format!("smtp://{host}:{port}/localhost") };
    let from = if cfg.mail_from.trim().is_empty() { cfg.mail_user.trim() } else { cfg.mail_from.trim() };
    let mut a: Vec<String> = ["-sS", "--max-time", "60", "--url", &url].iter().map(|s| s.to_string()).collect();
    if cfg.mail_security != "ssl" {
        a.push("--ssl-reqd".into());
    }
    a.push("--mail-from".into());
    a.push(from.to_string());
    for r in recipients(&cfg.mail_to) {
        a.push("--mail-rcpt".into());
        a.push(r);
    }
    a.push("-K".into());
    a.push(conf_file.to_string());
    a.push("--upload-file".into());
    a.push(msg_file.to_string());
    a
}

fn quote_conf(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Envoie un e-mail. Ok : destinataires ; Err : raison (mot de passe jamais affiché).
pub fn send(cfg: &Config, subject: &str, body: &str) -> Result<String, String> {
    if cfg.mail_server.trim().is_empty() || cfg.mail_to.trim().is_empty() {
        return Err("serveur SMTP ou destinataire manquant (Réglages → Notification par e-mail)".into());
    }
    if recipients(&cfg.mail_to).is_empty() {
        return Err("adresse du destinataire invalide".into());
    }
    let dir: PathBuf = crate::app_dir().join("tmp");
    let _ = std::fs::create_dir_all(&dir);
    let stamp = format!("{}-{}", prep::now_secs(), std::process::id());
    let msg = dir.join(format!("mail-{stamp}.eml"));
    let conf = dir.join(format!("mail-{stamp}.curl"));
    std::fs::write(&msg, message(cfg, subject, body)).map_err(|e| format!("message : {e}"))?;
    let user = if cfg.mail_user.trim().is_empty() { cfg.mail_from.trim() } else { cfg.mail_user.trim() };
    std::fs::write(&conf, format!("user = {}\n", quote_conf(&format!("{user}:{}", cfg.mail_pass)))).map_err(|e| format!("configuration : {e}"))?;
    let args = curl_args(cfg, &msg.display().to_string(), &conf.display().to_string());
    let mut cmd = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    cmd.args(&args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output();
    let _ = std::fs::remove_file(&conf);
    let _ = std::fs::remove_file(&msg);
    let out = out.map_err(|e| format!("curl introuvable ({e}) : il est fourni avec Windows 10 et 11"))?;
    if out.status.success() {
        return Ok(recipients(&cfg.mail_to).join(", "));
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let err = if cfg.mail_pass.is_empty() { err } else { err.replace(&cfg.mail_pass, "•••") };
    let hint = match out.status.code() {
        Some(67) => ", identifiant ou mot de passe refusé (souvent : il faut un mot de passe d'application)",
        Some(6) | Some(7) => ", serveur injoignable : vérifie le nom et le port",
        Some(28) => ", délai dépassé : vérifie le port et la sécurité (SSL 465 / STARTTLS 587)",
        Some(35) | Some(60) => ", échec de la connexion sécurisée : essaie l'autre sécurité (SSL / STARTTLS)",
        Some(55) | Some(56) => ", le serveur a coupé : adresse d'expéditeur refusée ?",
        _ => "",
    };
    Err(format!("envoi refusé (code {:?}) : {err}{hint}", out.status.code()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinataire_en_double_une_seule_fois() {
        assert_eq!(recipients("moi@x.fr; Moi@X.fr, autre@y.fr"), vec!["moi@x.fr".to_string(), "autre@y.fr".to_string()]);
    }

    #[test]
    fn message_et_arguments() {
        let cfg = Config {
            mail_enabled: true,
            mail_server: "smtp.exemple.fr".into(),
            mail_port: 0,
            mail_security: "starttls".into(),
            mail_user: "moi@exemple.fr".into(),
            mail_pass: "s3cr\"et".into(),
            mail_to: "a@x.fr; b@y.fr, pas-une-adresse".into(),
            mail_html: false,
            ..Default::default()
        };
        assert_eq!(recipients(&cfg.mail_to), vec!["a@x.fr", "b@y.fr"]);
        let m = message(&cfg, "Traitements terminés", "ligne 1\n.ligne avec point");
        assert!(m.contains("Subject: =?UTF-8?B?") && m.contains("\r\n\r\nligne 1\r\n..ligne avec point\r\n"), "{m}");
        assert!(m.contains("From: NZB FF Tool Pro <moi@exemple.fr>") && m.contains("To: a@x.fr, b@y.fr"));
        let a = curl_args(&cfg, "m.eml", "c.curl").join(" ");
        assert!(a.contains("--url smtp://smtp.exemple.fr:587/localhost --ssl-reqd") && a.contains("--mail-rcpt a@x.fr --mail-rcpt b@y.fr"), "{a}");
        assert!(!a.contains("s3cr"), "le mot de passe ne doit jamais être sur la ligne de commande");
        let ssl = Config { mail_security: "ssl".into(), ..cfg.clone() };
        assert!(curl_args(&ssl, "m", "c").join(" ").contains("smtps://smtp.exemple.fr:465/localhost"));
        assert_eq!(quote_conf("moi:s3cr\"et"), "\"moi:s3cr\\\"et\"");
        assert_eq!(b64(b"Ma"), "TWE=");
        assert_eq!(civil(20760), (2026, 11, 3));
        let lines: Vec<String> = vec!["10:00  ✔ Posté : A".into(), "11:00  ✖ Échec du post : B".into()];
        let (s, b) = summary(&cfg, &lines);
        assert!(s.contains("1 réussi(s), 1 échec(s)") && b.contains("Posté : A"), "{s}");
        // modèles personnalisés
        let perso = Config { mail_subject: "[{app}] {réussis} ok / {echecs} ko".into(), mail_body: "Erreurs :\n{erreurs}\nFin le {date}".into(), ..cfg.clone() };
        let (s, b) = summary(&perso, &lines);
        assert_eq!(s, "[NZB FF Tool Pro] 1 ok / 1 ko");
        assert!(b.starts_with("Erreurs :\n11:00  ✖ Échec du post : B\nFin le ") && !b.contains("Posté"), "{b}");
        // le texte sur plusieurs lignes survit à l'enregistrement
        let dir = std::env::temp_dir().join(format!("nzbff-mail-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("nzbff.conf");
        let mut c2 = perso.clone();
        c2.mail_body = "Ligne 1\nC:\\dossier\\n\n\nFin".into();
        c2.mail_providers.push(prep::MailProvider { name: "Mon FAI".into(), server: "smtp.fai.fr".into(), port: 587, security: "starttls".into() });
        c2.save(&f).unwrap();
        let back = Config::load(&f);
        assert_eq!(back.mail_body, c2.mail_body);
        assert_eq!(back.mail_subject, c2.mail_subject);
        assert_eq!(back.mail_providers, c2.mail_providers);
        assert!(providers(&back).iter().any(|p| p.0 == "Mon FAI" && p.2 == 587));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Aperçu : `NZBFF_MAIL_HTML=dossier cargo test apercu_html -- --ignored`.
    #[test]
    #[ignore]
    fn apercu_html() {
        let Ok(dir) = std::env::var("NZBFF_MAIL_HTML") else { return };
        let bad = "Résumé des traitements terminés à 21:42\n\n21:12  ✔ Posté : Le.Film.2020.FRENCH.1080p.WEB.H264-GRP (4,2 Go, 52 s)\n21:13  ✔ Déposé sur IndexerA\n21:13  ✖ Dépôt refusé par IndexerC : HTTP 405 Method Not Allowed\n        l'adresse d'envoi n'est pas la bonne : « Chercher l'adresse d'envoi » dans ses réglages\n21:30  ⚠ Une.Serie.S01E02 déjà présente sur IndexerB (déposée quand même)\n21:41  ✔ Posté : Une.Serie.S01E03.FRENCH.720p.WEB-X";
        let warn = "Résumé des traitements terminés à 08:05\n\n07:58  ✔ Posté : Album-2024-WEB-FLAC (612 Mo)\n07:59  ⏭ IndexerA ignoré : déjà présente (uploadée à la main)\n08:05  ✔ Déposé sur IndexerB";
        let good = "Résumé des traitements terminés à 13:20\n\n13:02  ✔ Posté : Dune.2021.MULTi.2160p.WEB.H265-SUPPLY (18,2 Go)\n13:20  ✔ Déposé sur IndexerA, IndexerB, IndexerD";
        for (n, b) in [("rouge", bad), ("jaune", warn), ("vert", good)] {
            std::fs::write(format!("{dir}/mail-{n}.html"), html_body("NZB FF Tool : 3 traitements terminés", b)).unwrap();
        }
    }

    #[test]
    fn e_mail_en_couleur() {
        assert_eq!(severity("10:00  ✔ Posté : A"), 0);
        assert_eq!(severity("10:01  ⚠ A déjà présente sur X (déposée quand même)"), 1);
        assert_eq!(severity("10:02  ✖ Dépôt refusé par X"), 2);
        assert_eq!(overall(&["✔ a", "⚠ b"]), 1);
        assert_eq!(overall(&["✔ a"]), 0);
        let h = html_body("Résumé", "10:00  ✔ Posté : A\n10:02  ✖ Dépôt refusé par X : <b>\n        réponse de l'indexer : 405");
        assert!(h.contains("#b42318") && h.contains("Des erreurs") && h.contains("&lt;b&gt;"), "{h}");
        assert!(html_body("R", "✔ ok").contains("Tout s'est bien passé"));
        assert!(html_body("R", "✔ ok\n⚠ attention").contains("alertes sans conséquence"));
        let cfg = Config { mail_to: "a@x.fr".into(), mail_user: "m@x.fr".into(), ..Default::default() };
        assert!(cfg.mail_html, "en couleur par défaut");
        let m = message(&cfg, "Sujet", "✔ ok\n.point");
        assert!(m.contains("multipart/alternative") && m.contains("text/plain") && m.contains("text/html") && m.contains("\r\n..point\r\n"), "{m}");
        // le HTML en base64 se relit
        let part = m.split("Content-Transfer-Encoding: base64\r\n\r\n").nth(1).unwrap().split("\r\n--").next().unwrap();
        assert!(part.split("\r\n").all(|l| l.len() <= 76));
        let joined: String = part.split("\r\n").collect();
        assert!(joined.len() > 200 && joined.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/' || c == b'='));
        assert!(m.lines().all(|l| l.len() < 998));
    }
}

#[cfg(test)]
mod tests_envoi {
    /// Envoi réel vers un serveur SMTPS de test (lancé à part, port dans NZBFF_SMTP_TEST).
    #[test]
    fn envoi_reel_smtps() {
        let Ok(port) = std::env::var("NZBFF_SMTP_TEST") else { return };
        let cfg = crate::prep::Config {
            mail_enabled: true,
            mail_server: "localhost".into(),
            mail_port: port.parse().unwrap(),
            mail_security: "ssl".into(),
            mail_user: "moi@exemple.fr".into(),
            mail_pass: "mot-de-passe".into(),
            mail_to: "toi@exemple.fr".into(),
            ..Default::default()
        };
        let r = super::send(&cfg, "Traitements terminés", "✔ Posté : Breaking.Bad.S01");
        assert_eq!(r, Ok("toi@exemple.fr".to_string()));
    }
}
