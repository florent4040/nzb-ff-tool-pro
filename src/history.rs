//! Historique persistant (fichier TSV à côté de l'exécutable) + heure locale sans dépendance.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Sent,
    Quarantine,
    Failed,
}

impl Status {
    pub fn code(self) -> &'static str {
        match self {
            Status::Sent => "OK",
            Status::Quarantine => "QUARANTAINE",
            Status::Failed => "ECHEC",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Status::Sent => "Envoyé",
            Status::Quarantine => "Quarantaine",
            Status::Failed => "Échec",
        }
    }
    fn parse(s: &str) -> Option<Status> {
        match s {
            "OK" => Some(Status::Sent),
            "QUARANTAINE" => Some(Status::Quarantine),
            "ECHEC" => Some(Status::Failed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub ts: u64,
    pub status: Status,
    /// Origine du traitement : « Surveillance », « Traitement » ou « Lot » (avant la 3.23 :
    /// « Auto » / « Manuel », voir `origin_label`)
    pub mode: String,
    pub original: String,
    pub release: String,
    pub size: u64,
    pub resolution: String,
    pub codec: String,
    pub langs: String,
    pub duration: String,
    pub source_path: String,
    /// Fichier dans le dossier NgPost, ou en quarantaine
    pub dest: String,
    pub xml: String,
    pub reason: String,
    /// Suite du post : « auto|file », « auto|ok|<détail> », « main|echec|<erreur> »… (vide :
    /// aucun post suivi). Champ ajouté en 3.23 (absent des lignes plus anciennes).
    pub post: String,
}

const FIELDS: usize = 14;

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n").replace('\r', "")
}

fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl Record {
    pub fn to_line(&self) -> String {
        [
            self.ts.to_string(),
            self.status.code().to_string(),
            esc(&self.mode),
            esc(&self.original),
            esc(&self.release),
            self.size.to_string(),
            esc(&self.resolution),
            esc(&self.codec),
            esc(&self.langs),
            esc(&self.duration),
            esc(&self.source_path),
            esc(&self.dest),
            esc(&self.xml),
            esc(&self.reason),
            esc(&self.post),
        ]
        .join("\t")
    }

    pub fn from_line(line: &str) -> Option<Record> {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < FIELDS {
            return None;
        }
        Some(Record {
            ts: f[0].parse().ok()?,
            status: Status::parse(f[1])?,
            mode: unesc(f[2]),
            original: unesc(f[3]),
            release: unesc(f[4]),
            size: f[5].parse().unwrap_or(0),
            resolution: unesc(f[6]),
            codec: unesc(f[7]),
            langs: unesc(f[8]),
            duration: unesc(f[9]),
            source_path: unesc(f[10]),
            dest: unesc(f[11]),
            xml: unesc(f[12]),
            reason: unesc(f[13]),
            post: f.get(14).map(|v| unesc(v)).unwrap_or_default(),
        })
    }

    /// D'où vient le traitement : « Surveillance », « Traitement » (page Traitement) ou « Lot ».
    /// Les lignes d'avant la 3.23 disaient « Auto » / « Manuel », ce qui se lisait comme le
    /// mode du post.
    pub fn origin_label(&self) -> &str {
        match self.mode.as_str() {
            "Auto" => "Surveillance",
            "Manuel" => "Traitement",
            m => m,
        }
    }

    /// Suite du post : (auto ?, état, détail). États : file, cours, ok, echec, arret, doublon.
    pub fn post_state(&self) -> Option<(bool, &str, &str)> {
        if self.post.is_empty() {
            return None;
        }
        let mut it = self.post.splitn(3, '|');
        let how = it.next()?;
        let state = it.next().unwrap_or("");
        Some((how == "auto", state, it.next().unwrap_or("")))
    }

    /// Post en clair, court (colonne) et long (détails).
    pub fn post_text(&self) -> Option<(String, String)> {
        let (auto, state, detail) = self.post_state()?;
        let how = if auto { "auto" } else { "manuel" };
        let (short, long) = match state {
            "file" => (format!("{how} · en file"), format!("Post {how} : en file d'attente")),
            "cours" => (format!("{how} · en cours"), format!("Post {how} : en cours")),
            "ok" => (format!("{how} ✔"), format!("Post {how} réussi{}", if detail.is_empty() { String::new() } else { format!(" : {detail}") })),
            "echec" => (format!("{how} ✖"), format!("Post {how} en échec : {detail}")),
            "arret" => (format!("{how} ⏹"), format!("Post {how} arrêté à ta demande")),
            "doublon" => (format!("{how} ⏭"), format!("Post {how} pas fait : déjà sur les indexers visés{}", if detail.is_empty() { String::new() } else { format!(" ({detail})") })),
            o => (format!("{how} · {o}"), format!("Post {how} : {o}")),
        };
        Some((short, long))
    }

    /// Nom à afficher : la release si connue, sinon le fichier d'origine.
    pub fn display_name(&self) -> &str {
        if self.release.is_empty() {
            &self.original
        } else {
            &self.release
        }
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    pub total: usize,
    pub sent: usize,
    pub quarantine: usize,
    pub failed: usize,
    pub sent_today: usize,
    pub sent_bytes: u64,
    pub sent_week: usize,
    /// Releases préparées dont le post a échoué (dernier état connu).
    pub post_failed: usize,
}

pub struct History {
    path: PathBuf,
    pub items: Vec<Record>,
    /// Incrémenté à chaque modification (l'interface s'en sert pour rafraîchir ses caches).
    pub revision: u64,
}

impl History {
    pub fn load(path: &Path) -> History {
        let items = fs::read_to_string(path)
            .map(|t| t.lines().filter_map(Record::from_line).collect())
            .unwrap_or_default();
        History { path: path.to_path_buf(), items, revision: 1 }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn push(&mut self, r: Record) {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "{}", r.to_line());
        }
        self.items.push(r);
        self.revision += 1;
    }

    /// Note la suite du post de la release préparée `dest` (ou, à défaut, de la dernière
    /// release envoyée sous ce nom). Réécrit le fichier (écriture puis renommage).
    pub fn set_post(&mut self, dest: &Path, release: &str, value: &str) -> bool {
        let d = dest.display().to_string();
        let k = self.items.iter().rposition(|r| r.status == Status::Sent && !d.is_empty() && r.dest == d).or_else(|| self.items.iter().rposition(|r| r.status == Status::Sent && r.release.eq_ignore_ascii_case(release)));
        let Some(k) = k else { return false };
        if self.items[k].post == value {
            return true;
        }
        self.items[k].post = value.replace(['\t', '\n', '\r'], " ");
        self.revision += 1;
        let text: String = self.items.iter().map(|r| r.to_line() + "\n").collect();
        let tmp = self.path.with_extension("tsv.tmp");
        if fs::write(&tmp, text).is_ok() {
            let _ = fs::rename(&tmp, &self.path);
        }
        true
    }

    /// Vide l'historique (une copie .bak est conservée).
    pub fn clear(&mut self) -> std::io::Result<()> {
        if self.path.exists() {
            let bak = self.path.with_extension("tsv.bak");
            fs::copy(&self.path, bak)?;
            fs::write(&self.path, "")?;
        }
        self.items.clear();
        self.revision += 1;
        Ok(())
    }

    /// Dernier envoi réussi portant ce nom de release.
    pub fn find_sent(&self, release: &str) -> Option<&Record> {
        self.items.iter().rev().find(|r| r.status == Status::Sent && r.release.eq_ignore_ascii_case(release))
    }

    /// Dernière raison connue pour un fichier mis en quarantaine.
    pub fn quarantine_reason(&self, path: &Path) -> Option<&Record> {
        let p = path.display().to_string();
        self.items.iter().rev().find(|r| r.status == Status::Quarantine && r.dest == p)
    }

    pub fn stats(&self) -> Stats {
        let now = crate::prep::now_secs();
        let today = day_key(now);
        let mut s = Stats { total: self.items.len(), ..Default::default() };
        for r in &self.items {
            if matches!(r.post_state(), Some((_, "echec", _))) {
                s.post_failed += 1;
            }
            match r.status {
                Status::Sent => {
                    s.sent += 1;
                    s.sent_bytes += r.size;
                    if day_key(r.ts) == today {
                        s.sent_today += 1;
                    }
                    if now.saturating_sub(r.ts) < 7 * 86_400 {
                        s.sent_week += 1;
                    }
                }
                Status::Quarantine => s.quarantine += 1,
                Status::Failed => s.failed += 1,
            }
        }
        s
    }

    /// Envois réussis par jour sur les `days` derniers jours (le plus ancien d'abord).
    pub fn sent_per_day(&self, days: usize) -> Vec<(String, usize)> {
        let now = crate::prep::now_secs();
        (0..days)
            .rev()
            .map(|i| {
                let t = now.saturating_sub(i as u64 * 86_400);
                let key = day_key(t);
                let n = self
                    .items
                    .iter()
                    .filter(|r| r.status == Status::Sent && day_key(r.ts) == key)
                    .count();
                let lt = local_time(t);
                (format!("{:02}/{:02}", lt.day, lt.month), n)
            })
            .collect()
    }

    /// Export CSV (séparateur « ; », UTF-8 avec BOM : s'ouvre proprement dans Excel / LibreOffice).
    pub fn export_csv(&self, path: &Path) -> std::io::Result<()> {
        let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\"").replace(['\n', '\r'], " "));
        let mut out = String::from("\u{feff}Date;Statut;Origine;Post;Fichier d'origine;Nom de release;Taille (octets);Résolution;Codec;Langues;Durée;Destination;XML;Détail\r\n");
        for r in &self.items {
            out.push_str(
                &[
                    q(&format_ts(r.ts)),
                    q(r.status.label()),
                    q(r.origin_label()),
                    q(&r.post_text().map(|p| p.1).unwrap_or_default()),
                    q(&r.original),
                    q(&r.release),
                    r.size.to_string(),
                    q(&r.resolution),
                    q(&r.codec),
                    q(&r.langs),
                    q(&r.duration),
                    q(&r.dest),
                    q(&r.xml),
                    q(&r.reason),
                ]
                .join(";"),
            );
            out.push_str("\r\n");
        }
        fs::write(path, out)
    }
}

// --------------------------------------------------------------- Heure locale

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

#[cfg(not(windows))]
#[repr(C)]
#[allow(dead_code)]
struct Tm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
    // place pour les champs supplémentaires de glibc (tm_gmtoff, tm_zone)
    _extra: [u64; 4],
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct WinSystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    millis: u16,
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    // Convertit une heure UTC en heure locale, en tenant compte de l'heure d'été à cette date.
    fn SystemTimeToTzSpecificLocalTime(tz: *const u8, utc: *const WinSystemTime, local: *mut WinSystemTime) -> i32;
}

#[cfg(unix)]
extern "C" {
    fn localtime_r(time: *const i64, tm: *mut Tm) -> *mut Tm;
}

fn civil_utc(ts: u64) -> LocalTime {
    // Algorithme « days from civil » (Howard Hinnant), repli UTC.
    let secs = ts as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + if m <= 2 { 1 } else { 0 }) as i32;
    LocalTime { year: y, month: m, day: d, hour: rem / 3600, minute: (rem / 60) % 60, second: rem % 60 }
}

#[cfg(windows)]
pub fn local_time(ts: u64) -> LocalTime {
    let u = civil_utc(ts);
    let utc = WinSystemTime {
        year: u.year as u16,
        month: u.month as u16,
        day: u.day as u16,
        hour: u.hour as u16,
        minute: u.minute as u16,
        second: u.second as u16,
        ..Default::default()
    };
    let mut l = WinSystemTime::default();
    let ok = unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut l) != 0 };
    if !ok {
        return u;
    }
    LocalTime {
        year: l.year as i32,
        month: l.month as u32,
        day: l.day as u32,
        hour: l.hour as u32,
        minute: l.minute as u32,
        second: l.second as u32,
    }
}

#[cfg(not(windows))]
pub fn local_time(ts: u64) -> LocalTime {
    let t = ts as i64;
    let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, _extra: [0; 4] };
    #[cfg(unix)]
    let ok = unsafe { !localtime_r(&t, &mut tm).is_null() };
    #[cfg(not(unix))]
    let ok = false;
    if !ok || tm.mday == 0 {
        return civil_utc(ts);
    }
    LocalTime {
        year: tm.year + 1900,
        month: (tm.mon + 1) as u32,
        day: tm.mday as u32,
        hour: tm.hour as u32,
        minute: tm.min as u32,
        second: tm.sec as u32,
    }
}

fn day_key(ts: u64) -> (i32, u32, u32) {
    let l = local_time(ts);
    (l.year, l.month, l.day)
}

/// « 01/11/2026 14:32 »
pub fn format_ts(ts: u64) -> String {
    let l = local_time(ts);
    format!("{:02}/{:02}/{} {:02}:{:02}", l.day, l.month, l.year, l.hour, l.minute)
}

/// « 14:32:05 »
pub fn format_clock(ts: u64) -> String {
    let l = local_time(ts);
    format!("{:02}:{:02}:{:02}", l.hour, l.minute, l.second)
}

/// « il y a 3 min », « hier 21:04 », ou la date complète.
pub fn format_relative(ts: u64) -> String {
    let now = crate::prep::now_secs();
    let d = now.saturating_sub(ts);
    if d < 60 {
        "à l'instant".into()
    } else if d < 3600 {
        format!("il y a {} min", d / 60)
    } else if day_key(ts) == day_key(now) {
        format!("aujourd'hui {}", &format_ts(ts)[11..])
    } else if day_key(ts) == day_key(now.saturating_sub(86_400)) {
        format!("hier {}", &format_ts(ts)[11..])
    } else {
        format_ts(ts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(status: Status, release: &str, ts: u64) -> Record {
        Record {
            ts,
            status,
            mode: "Auto".into(),
            original: "Mon film (1975).mkv".into(),
            release: release.into(),
            size: 1000,
            resolution: "1080p".into(),
            codec: "x264".into(),
            langs: "fr".into(),
            duration: "01:40:00".into(),
            source_path: "C:\\in\\Mon film (1975).mkv".into(),
            dest: "C:\\ng\\x.mkv".into(),
            xml: "C:\\xml\\x.xml".into(),
            reason: "tab\there\nnewline \\ backslash".into(),
            post: String::new(),
        }
    }

    /// Suite du post notée sur la release préparée ; lignes d'avant la 3.23 (14 champs) lues.
    #[test]
    fn suite_du_post() {
        let d = std::env::temp_dir().join(format!("nzbff_hist_post_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let f = d.join("h.tsv");
        let old = rec(Status::Sent, "A.2020.FRENCH.1080p.x264", 10);
        // ancienne ligne : sans le 15e champ
        let line = old.to_line();
        let line14 = line.rsplit_once('\t').unwrap().0.to_string();
        fs::write(&f, format!("{line14}\n")).unwrap();
        let mut h = History::load(&f);
        assert_eq!(h.items.len(), 1);
        assert_eq!(h.items[0].origin_label(), "Surveillance");
        assert!(h.items[0].post_state().is_none());
        assert!(h.set_post(Path::new("C:\\ng\\x.mkv"), "", "auto|file"));
        assert_eq!(h.items[0].post_text().unwrap().0, "auto · en file");
        assert!(h.set_post(Path::new("ailleurs"), "a.2020.french.1080p.x264", "auto|echec|par2 : code 3"));
        let h2 = History::load(&f);
        let r = &h2.items[0];
        assert_eq!(r.post_state(), Some((true, "echec", "par2 : code 3")));
        assert!(r.post_text().unwrap().1.contains("Post auto en échec : par2"));
        assert_eq!(h2.stats().post_failed, 1);
        let mut m = rec(Status::Sent, "B", 11);
        m.mode = "Traitement".into();
        m.post = "main|ok|1 Go à 50 Mo/s".into();
        assert_eq!(m.origin_label(), "Traitement");
        assert_eq!(m.post_text().unwrap().0, "manuel ✔");
        assert!(!h.set_post(Path::new("x"), "inconnue", "auto|ok"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn ligne_aller_retour() {
        let r = rec(Status::Quarantine, "A.1975.FRENCH.1080p.BluRay.x264", 1_700_000_000);
        assert_eq!(Record::from_line(&r.to_line()), Some(r));
    }

    #[test]
    fn persistance_stats_doublons() {
        let d = std::env::temp_dir().join(format!("ngprep_hist_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let p = d.join("h.tsv");
        let now = crate::prep::now_secs();
        {
            let mut h = History::load(&p);
            h.push(rec(Status::Sent, "A.1975.FRENCH.1080p.BluRay.x264", now));
            h.push(rec(Status::Failed, "", now - 10 * 86_400));
            h.push(rec(Status::Sent, "B.2001.FRENCH.720p.WEB-DL.x264", now - 9 * 86_400));
        }
        let h = History::load(&p);
        assert_eq!(h.items.len(), 3);
        let s = h.stats();
        assert_eq!((s.total, s.sent, s.failed, s.sent_today, s.sent_week, s.sent_bytes), (3, 2, 1, 1, 1, 2000));
        assert!(h.find_sent("a.1975.french.1080p.bluray.x264").is_some());
        assert!(h.find_sent("C.1999").is_none());
        let per = h.sent_per_day(7);
        assert_eq!(per.len(), 7);
        assert_eq!(per.last().unwrap().1, 1);
        h.export_csv(&d.join("e.csv")).unwrap();
        let csv = fs::read_to_string(d.join("e.csv")).unwrap();
        assert!(csv.starts_with('\u{feff}') && csv.lines().count() == 4);
        let mut h = h;
        h.clear().unwrap();
        assert!(History::load(&p).items.is_empty());
        assert!(d.join("h.tsv.bak").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn dates() {
        let u = civil_utc(1_762_000_000); // 2025-11-01 12:26:40 UTC
        assert_eq!((u.year, u.month, u.day, u.hour, u.minute), (2025, 11, 1, 12, 26));
        let l = local_time(1_762_000_000);
        assert_eq!(l.year, 2025);
        assert_eq!(format_ts(1_762_000_000).len(), 16);
        assert_eq!(format_relative(crate::prep::now_secs()), "à l'instant");
    }
}
