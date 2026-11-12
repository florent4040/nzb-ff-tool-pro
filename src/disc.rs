//! Disques complets : Blu-ray (dossier BDMV ou ISO) et DVD (dossier VIDEO_TS ou ISO).
//!
//! - repérage du disque (dossier, sous-dossier, image ISO) ;
//! - titre principal : playlist .mpls la plus lourde (Blu-ray), jeu de titres VTS le plus
//!   lourd (DVD) ; c'est sur lui que MediaInfo est lancé (langues, résolution, codecs) ;
//! - image ISO : montée par Windows le temps de la lecture (lecture seule), puis démontée ;
//! - rapport BDInfo (bdinfo-rs ou BDInfoCLI) du titre principal, pour le NFO des Blu-ray.

use crate::prep::{self, Config, Facts};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Bluray,
    Dvd,
}

impl Kind {
    pub fn label(&self) -> &'static str {
        match self {
            Kind::Bluray => "Blu-ray",
            Kind::Dvd => "DVD",
        }
    }
    /// Clé de la catégorie de contenu correspondante.
    pub fn cat_key(&self) -> &'static str {
        match self {
            Kind::Bluray => "bluray",
            Kind::Dvd => "dvd",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Disc {
    pub kind: Kind,
    /// Ce qui est livré : dossier de la release (NFO, Sample, Proof… compris) ou image ISO seule.
    pub root: PathBuf,
    /// Données du disque : dossier qui contient BDMV / VIDEO_TS, ou fichier ISO.
    pub image: PathBuf,
    /// Le disque est une image ISO (montée le temps de la lecture).
    pub iso: bool,
}

/// Dossiers propres aux disques : leurs fichiers ne sont jamais des vidéos à part.
pub const DISC_DIRS: &[&str] = &["BDMV", "VIDEO_TS", "AUDIO_TS", "CERTIFICATE", "AACS", "BDSVM"];

pub fn is_disc_dir_name(name: &str) -> bool {
    DISC_DIRS.iter().any(|d| d.eq_ignore_ascii_case(name))
}

fn child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && prep::file_name(p).eq_ignore_ascii_case(name))
}

fn is_bdmv(dir: &Path) -> bool {
    // index.bdmv, ou au moins PLAYLIST + STREAM
    fs::read_dir(dir)
        .map(|rd| {
            let names: Vec<String> = rd.flatten().map(|e| prep::file_name(&e.path()).to_lowercase()).collect();
            names.iter().any(|n| n == "index.bdmv") || (names.iter().any(|n| n == "playlist") && names.iter().any(|n| n == "stream"))
        })
        .unwrap_or(false)
}

fn is_video_ts(dir: &Path) -> bool {
    fs::read_dir(dir).map(|rd| rd.flatten().any(|e| prep::file_name(&e.path()).to_lowercase().ends_with(".ifo"))).unwrap_or(false)
}

fn is_image_file(p: &Path) -> bool {
    p.is_file() && p.extension().map(|e| e.eq_ignore_ascii_case("iso") || e.eq_ignore_ascii_case("img")).unwrap_or(false)
}

/// Le disque d'un chemin :
/// - dossier du disque (ou son BDMV / VIDEO_TS) ;
/// - dossier de release qui contient l'image ISO du disque (« Film\film.iso », avec NFO,
///   Sample, Proof à côté) ;
/// - dossier de release qui contient le disque un niveau plus bas (« Film\DISC\BDMV ») ;
/// - image ISO seule.
pub fn detect(p: &Path) -> Option<Disc> {
    if p.is_file() {
        return is_image_file(p).then(|| iso_kind(p)).flatten().map(|kind| Disc { kind, root: p.to_path_buf(), image: p.to_path_buf(), iso: true });
    }
    if !p.is_dir() {
        return None;
    }
    let name = prep::file_name(p);
    if name.eq_ignore_ascii_case("BDMV") && is_bdmv(p) {
        let r = p.parent().unwrap_or(p).to_path_buf();
        return Some(Disc { kind: Kind::Bluray, root: r.clone(), image: r, iso: false });
    }
    if name.eq_ignore_ascii_case("VIDEO_TS") && is_video_ts(p) {
        let r = p.parent().unwrap_or(p).to_path_buf();
        return Some(Disc { kind: Kind::Dvd, root: r.clone(), image: r, iso: false });
    }
    let here = |d: &Path| -> Option<Kind> {
        if child_ci(d, "BDMV").filter(|b| is_bdmv(b)).is_some() {
            return Some(Kind::Bluray);
        }
        if child_ci(d, "VIDEO_TS").filter(|v| is_video_ts(v)).is_some() {
            return Some(Kind::Dvd);
        }
        None
    };
    if let Some(kind) = here(p) {
        return Some(Disc { kind, root: p.to_path_buf(), image: p.to_path_buf(), iso: false });
    }
    let entries: Vec<PathBuf> = fs::read_dir(p).ok()?.flatten().map(|e| e.path()).filter(|x| !prep::file_name(x).starts_with('.')).collect();
    // image ISO du disque dans le dossier de la release (la plus grosse s'il y en a plusieurs)
    let mut isos: Vec<&PathBuf> = entries.iter().filter(|x| is_image_file(x)).collect();
    isos.sort_by_key(|x| std::cmp::Reverse(fs::metadata(x).map(|m| m.len()).unwrap_or(0)));
    for iso in isos {
        if let Some(kind) = iso_kind(iso) {
            return Some(Disc { kind, root: p.to_path_buf(), image: iso.clone(), iso: true });
        }
    }
    // disque dans un sous-dossier (DISC, BD, nom du film…)
    let found: Vec<(Kind, &PathBuf)> = entries.iter().filter(|x| x.is_dir()).filter_map(|s| here(s).map(|k| (k, s))).collect();
    if found.len() == 1 {
        return Some(Disc { kind: found[0].0, root: p.to_path_buf(), image: found[0].1.clone(), iso: false });
    }
    None
}

/// Le fichier fait-il partie d'une structure de disque (BDMV, VIDEO_TS…) ?
pub fn inside_disc(f: &Path) -> bool {
    f.ancestors().skip(1).any(|a| is_disc_dir_name(&prep::file_name(a)))
}

/// Image ISO d'un Blu-ray ou d'un DVD vidéo ? Cherche les noms des dossiers dans le début de
/// l'image (répertoires ISO 9660 et UDF), sans la monter ; sinon d'après le nom et la taille
/// (BLURAY, BD25, BD50, UHD, COMPLETE… / DVD, DVD9, PAL…). Résultat gardé en mémoire.
pub fn iso_kind(p: &Path) -> Option<Kind> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, u64, Option<std::time::SystemTime>), Option<Kind>>>> = OnceLock::new();
    let meta = fs::metadata(p).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let key = (p.to_path_buf(), size, meta.and_then(|m| m.modified().ok()));
    let c = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(k) = c.lock().ok().and_then(|g| g.get(&key).copied()) {
        return k;
    }
    // image de jeu reconnue à son contenu : jamais un film, quel que soit son nom (« PAL »…)
    let k = match scan_iso(p) {
        Scan::Disc(k) => Some(k),
        Scan::Game => None,
        Scan::Unknown => iso_kind_by_name(p, size),
    };
    if let Ok(mut g) = c.lock() {
        g.insert(key, k);
    }
    k
}

enum Scan {
    Disc(Kind),
    Game,
    Unknown,
}

fn scan_iso(p: &Path) -> Scan {
    let Ok(mut f) = fs::File::open(p) else { return Scan::Unknown };
    let mut buf = vec![0u8; 1 << 20];
    let mut tail: Vec<u8> = Vec::new();
    let mut dvd = false;
    for _ in 0..64 {
        let Ok(n) = f.read(&mut buf) else { return Scan::Unknown };
        if n == 0 {
            break;
        }
        let mut hay = std::mem::take(&mut tail);
        hay.extend_from_slice(&buf[..n]);
        // UDF : nom en « compression 8 » (octet 8 puis ASCII) ou 16 bits ; ISO 9660 : ASCII
        if find(&hay, b"\x08BDMV").is_some() || find(&hay, b"\x10\x00B\x00D\x00M\x00V").is_some() || find(&hay, b"INDX0200").is_some() || find(&hay, b"INDX0300").is_some() || find(&hay, b"MPLS0200").is_some() || find(&hay, b"MPLS0300").is_some() {
            return Scan::Disc(Kind::Bluray);
        }
        // disque de jeu (PlayStation : identifiant système « PLAYSTATION », fichier SYSTEM.CNF) :
        // jamais un film, même s'il contient des vidéos
        if find(&hay, b"PLAYSTATION").is_some() || find(&hay, b"SYSTEM.CNF").is_some() || find(&hay, b"PSP_GAME").is_some() || find(&hay, b"PS3_GAME").is_some() {
            return Scan::Game;
        }
        if find(&hay, b"VIDEO_TS").is_some() || find(&hay, b"\x10\x00V\x00I\x00D\x00E\x00O\x00_\x00T\x00S").is_some() || find(&hay, b"DVDVIDEO-VTS").is_some() {
            dvd = true;
        }
        tail = hay[hay.len().saturating_sub(32)..].to_vec();
    }
    if dvd {
        Scan::Disc(Kind::Dvd)
    } else {
        Scan::Unknown
    }
}

/// Image non reconnue à son contenu : d'après son nom (et celui de son dossier) et sa taille.
pub fn iso_kind_by_name(p: &Path, size: u64) -> Option<Kind> {
    let name = format!("{} {}", prep::file_name(p), p.parent().map(prep::file_name).unwrap_or_default()).to_lowercase();
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |l: &[&str]| words.iter().any(|w| l.contains(w));
    const GB: u64 = 1 << 30;
    // jeux et logiciels : jamais un film
    if !crate::upload::platforms_in(&name).is_empty() || has(&["xbox", "codex", "fitgirl", "gog", "plaza", "skidrow", "rune", "tenoke", "elamigos", "dodi", "win", "win64", "x64", "macos", "linux"]) {
        return None;
    }
    if (has(&["bluray", "bdmv", "bd25", "bd50", "bd66", "bd100", "uhd", "bdiso"]) || name.contains("blu-ray")) && size >= 10 * GB {
        return Some(Kind::Bluray);
    }
    if has(&["dvd", "dvd5", "dvd9", "dvdr", "pal", "ntsc", "dvdiso"]) && size <= 10 * GB {
        return Some(Kind::Dvd);
    }
    // grosse image avec une année dans le nom : un film en Blu-ray
    let year = words.iter().any(|w| w.len() == 4 && (w.starts_with("19") || w.starts_with("20")) && w.chars().all(|c| c.is_ascii_digit()));
    if year && size >= 15 * GB {
        return Some(Kind::Bluray);
    }
    None
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

// ------------------------------------------------------------- titre principal

/// Playlist d'un Blu-ray : nom, durée (s), clips, poids des clips distincts.
#[derive(Clone, Debug, PartialEq)]
pub struct Playlist {
    pub file: PathBuf,
    pub secs: f64,
    pub clips: Vec<String>,
    pub bytes: u64,
}

/// Lit un fichier .mpls : clips et durée (heures en ticks de 45 kHz).
pub fn parse_mpls(data: &[u8]) -> Option<(Vec<String>, f64)> {
    if data.len() < 20 || &data[0..4] != b"MPLS" {
        return None;
    }
    let be32 = |o: usize| -> Option<u32> { data.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])) };
    let be16 = |o: usize| -> Option<u16> { data.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]])) };
    let pl = be32(8)? as usize;
    let n = be16(pl + 6)? as usize;
    let mut pos = pl + 10;
    let mut clips = Vec::new();
    let mut ticks: u64 = 0;
    for _ in 0..n {
        let len = be16(pos)? as usize;
        let item = pos + 2;
        let name = String::from_utf8_lossy(data.get(item..item + 5)?).to_string();
        let tin = be32(item + 12)? as u64;
        let tout = be32(item + 16)? as u64;
        ticks += tout.saturating_sub(tin);
        clips.push(name);
        pos = item + len;
    }
    Some((clips, ticks as f64 / 45_000.0))
}

/// Playlists d'un Blu-ray (dossier racine ou BDMV), la principale en premier : poids des clips
/// distincts le plus grand (les playlists qui bouclent sur le même clip ne trichent pas), puis
/// la plus longue.
pub fn playlists(root: &Path) -> Vec<Playlist> {
    let bdmv = if prep::file_name(root).eq_ignore_ascii_case("BDMV") { root.to_path_buf() } else { child_ci(root, "BDMV").unwrap_or_else(|| root.join("BDMV")) };
    let Some(pl_dir) = child_ci(&bdmv, "PLAYLIST") else { return Vec::new() };
    let stream = child_ci(&bdmv, "STREAM").unwrap_or_else(|| bdmv.join("STREAM"));
    let sizes: std::collections::HashMap<String, u64> = fs::read_dir(&stream)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let n = prep::file_name(&p);
                    let (stem, ext) = n.rsplit_once('.')?;
                    ext.eq_ignore_ascii_case("m2ts").then(|| (stem.to_string(), e.metadata().map(|m| m.len()).unwrap_or(0)))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut out: Vec<Playlist> = fs::read_dir(&pl_dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("mpls")).unwrap_or(false))
                .filter_map(|p| {
                    let data = fs::read(&p).ok()?;
                    let (clips, secs) = parse_mpls(&data)?;
                    let mut uniq: Vec<&String> = clips.iter().collect();
                    uniq.sort();
                    uniq.dedup();
                    let bytes = uniq.iter().map(|c| sizes.get(c.as_str()).copied().unwrap_or(0)).sum();
                    Some(Playlist { file: p, secs, clips, bytes })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(b.secs.partial_cmp(&a.secs).unwrap_or(std::cmp::Ordering::Equal)).then(a.file.cmp(&b.file)));
    out
}

/// Jeu de titres principal d'un DVD : (fichier IFO, poids de ses VOB).
pub fn dvd_main(root: &Path) -> Option<(PathBuf, u64)> {
    let vts = if prep::file_name(root).eq_ignore_ascii_case("VIDEO_TS") { root.to_path_buf() } else { child_ci(root, "VIDEO_TS")? };
    let mut sets: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for e in fs::read_dir(&vts).ok()?.flatten() {
        let n = prep::file_name(&e.path()).to_uppercase();
        // VTS_01_1.VOB … VTS_01_9.VOB (le _0 est le menu)
        if n.starts_with("VTS_") && n.ends_with(".VOB") && n.len() == 12 && &n[7..8] != "0" {
            *sets.entry(n[4..6].to_string()).or_default() += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    let (set, bytes) = sets.into_iter().max_by_key(|(_, b)| *b)?;
    let ifo = fs::read_dir(&vts).ok()?.flatten().map(|e| e.path()).find(|p| prep::file_name(p).eq_ignore_ascii_case(&format!("VTS_{set}_0.IFO")))?;
    Some((ifo, bytes))
}

/// Fichier sur lequel lancer MediaInfo dans un disque (monté s'il s'agit d'une ISO).
pub fn media_target(kind: Kind, root: &Path) -> Option<PathBuf> {
    media_targets(kind, root).into_iter().next()
}

/// Fichiers à lire, dans l'ordre : playlist principale (langues de toutes les pistes) puis son
/// plus gros clip .m2ts ; DVD : IFO du titre principal puis son plus gros VOB.
pub fn media_targets(kind: Kind, root: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    match kind {
        Kind::Bluray => {
            if let Some(p) = playlists(root).into_iter().next() {
                let stream = p.file.parent().and_then(|d| d.parent()).and_then(|b| child_ci(b, "STREAM"));
                let big = stream.and_then(|st| {
                    p.clips.iter().map(|c| st.join(format!("{c}.m2ts"))).filter(|f| f.is_file()).max_by_key(|f| fs::metadata(f).map(|m| m.len()).unwrap_or(0))
                });
                v.push(p.file);
                v.extend(big);
            }
        }
        Kind::Dvd => {
            if let Some((ifo, _)) = dvd_main(root) {
                let stem = prep::file_name(&ifo).to_uppercase().replace("_0.IFO", "_");
                let big = ifo.parent().and_then(|d| fs::read_dir(d).ok()).and_then(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter(|p| {
                            let n = prep::file_name(p).to_uppercase();
                            n.starts_with(&stem) && n.ends_with(".VOB") && !n.ends_with("_0.VOB")
                        })
                        .max_by_key(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
                });
                v.push(ifo);
                v.extend(big);
            }
        }
    }
    v
}

/// Description du titre principal, pour l'interface et le journal.
pub fn main_title_text(kind: Kind, root: &Path) -> String {
    match kind {
        Kind::Bluray => match playlists(root).first() {
            Some(p) => format!("playlist {} · {} · {} clip(s) · {}", prep::file_name(&p.file), hms(p.secs), p.clips.len(), prep::human_size(p.bytes)),
            None => "aucune playlist lisible".into(),
        },
        Kind::Dvd => match dvd_main(root) {
            Some((ifo, b)) => format!("titre {} · {}", prep::file_name(&ifo), prep::human_size(b)),
            None => "aucun titre lisible".into(),
        },
    }
}

fn hms(s: f64) -> String {
    let s = s as u64;
    format!("{}h{:02}", s / 3600, (s % 3600) / 60)
}

// ------------------------------------------------------------------ ISO

/// Monte une image ISO (Windows, lecture seule), appelle `f` avec la racine du disque monté,
/// puis la démonte (sauf si elle était déjà montée avant).
pub fn with_root<T>(d: &Disc, f: impl FnOnce(&Path) -> T) -> Result<T, String> {
    if !d.iso {
        return Ok(f(&d.image));
    }
    let (root, was) = mount(&d.image)?;
    let r = f(&root);
    if !was {
        unmount(&d.image);
    }
    Ok(r)
}

#[cfg(windows)]
fn powershell(script: &str, iso: &Path, secs: u64) -> Result<String, String> {
    powershell_env(script, &[("NZBFF_ISO", iso.display().to_string())], secs)
}

#[cfg(windows)]
fn powershell_env(script: &str, env: &[(&str, String)], secs: u64) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000)
        .spawn()
        .map_err(|e| format!("PowerShell : {e}"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(Ok(o)) => {
            let out = String::from_utf8_lossy(&o.stdout).to_string();
            let err = String::from_utf8_lossy(&o.stderr).lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string();
            if o.status.success() {
                Ok(out)
            } else {
                Err(if err.is_empty() { out.trim().to_string() } else { err })
            }
        }
        Ok(Err(e)) => Err(format!("PowerShell : {e}")),
        Err(_) => Err("PowerShell ne répond pas".into()),
    }
}

#[cfg(windows)]
const PS_MOUNT: &str = r#"$ErrorActionPreference='Stop'; $p=$env:NZBFF_ISO; $i=Get-DiskImage -ImagePath $p; $was=$i.Attached; if(-not $was){ $i=Mount-DiskImage -ImagePath $p -StorageType ISO -Access ReadOnly -PassThru }; $l=($i|Get-Volume).DriveLetter; if(-not $l){ Start-Sleep -Milliseconds 1500; $l=(Get-DiskImage -ImagePath $p|Get-Volume).DriveLetter }; "LETTER=$l"; "WAS=$was""#;

#[cfg(windows)]
fn mount(iso: &Path) -> Result<(PathBuf, bool), String> {
    let full = fs::canonicalize(iso).map(|p| PathBuf::from(p.display().to_string().trim_start_matches(r"\\?\"))).unwrap_or_else(|_| iso.to_path_buf());
    let out = powershell(PS_MOUNT, &full, 60).map_err(|e| format!("montage de l'image ISO impossible : {e}"))?;
    let get = |k: &str| out.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim().to_string()));
    let letter = get("LETTER=").filter(|l| l.len() == 1).ok_or_else(|| format!("montage de l'image ISO : pas de lettre de lecteur ({})", out.trim()))?;
    let was = get("WAS=").map(|v| v.eq_ignore_ascii_case("true")).unwrap_or(false);
    Ok((PathBuf::from(format!("{letter}:\\")), was))
}

#[cfg(windows)]
fn unmount(iso: &Path) {
    let full = fs::canonicalize(iso).map(|p| PathBuf::from(p.display().to_string().trim_start_matches(r"\\?\"))).unwrap_or_else(|_| iso.to_path_buf());
    let _ = powershell("Dismount-DiskImage -ImagePath $env:NZBFF_ISO | Out-Null", &full, 60);
}

#[cfg(not(windows))]
fn mount(_iso: &Path) -> Result<(PathBuf, bool), String> {
    Err("montage d'une image ISO : seulement sous Windows".into())
}

#[cfg(not(windows))]
fn unmount(_iso: &Path) {}

// ------------------------------------------------------------- MediaInfo

/// Ce qu'on sait d'un disque, pour le nom et l'interface.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub facts: Option<Facts>,
    /// « playlist 00800.mpls · 2h11 · 3 clip(s) · 38,2 Go ».
    pub title: String,
    /// Rapport MediaInfo en texte (aperçu).
    pub text: String,
    pub error: String,
}

/// Analyse MediaInfo du titre principal (image ISO montée le temps de la lecture). Une ISO de
/// DVD qui ne se monte pas est lue directement (MediaInfo 23.07 ou plus récent).
pub fn analyze(cfg: &Config, d: &Disc) -> Info {
    let run = |root: &Path| -> Info {
        let mut info = Info { title: main_title_text(d.kind, root), ..Default::default() };
        let targets = media_targets(d.kind, root);
        if targets.is_empty() {
            info.error = format!("titre principal du {} introuvable", d.kind.label());
        }
        // playlist / IFO d'abord ; si MediaInfo n'y lit pas de vidéo, le plus gros fichier
        for t in &targets {
            match prep::query_facts(cfg, t) {
                Ok(f) => {
                    info.facts = Some(f);
                    info.error.clear();
                    info.text = prep::mediainfo_text(cfg, t, false).unwrap_or_default();
                    break;
                }
                Err(e) => info.error = format!("{} : {e}", prep::file_name(t)),
            }
        }
        info
    };
    match with_root(d, run) {
        Ok(i) => i,
        Err(e) if d.iso => {
            // lecture directe de l'image (DVD : MediaInfo sait lire les ISO)
            let mut info = Info { title: format!("image ISO non montée : {e}"), ..Default::default() };
            match prep::query_facts(cfg, &d.image) {
                Ok(f) => info.facts = Some(f),
                Err(e2) => info.error = format!("{e} ; lecture directe : {e2}"),
            }
            info.text = prep::mediainfo_text(cfg, &d.image, false).unwrap_or_default();
            info
        }
        Err(e) => Info { error: e, ..Default::default() },
    }
}

/// Rapport MediaInfo (format des réglages) du titre principal.
pub fn mediainfo_export(cfg: &Config, d: &Disc) -> Result<String, String> {
    let r = with_root(d, |root| {
        let targets = media_targets(d.kind, root);
        if targets.is_empty() {
            return Err(format!("titre principal du {} introuvable (playlist / VTS)", d.kind.label()));
        }
        // le premier fichier où MediaInfo lit une vidéo
        let mut last = String::new();
        for t in &targets {
            if prep::query_facts(cfg, t).is_ok() {
                return prep::mediainfo_export(cfg, t);
            }
            last = prep::file_name(t);
        }
        Err(format!("MediaInfo ne lit aucune vidéo dans le titre principal ({last})"))
    });
    match r {
        Ok(r) => r,
        Err(e) if d.iso => prep::mediainfo_export(cfg, &d.image).map_err(|e2| format!("{e} ; lecture directe de l'ISO : {e2}")),
        Err(e) => Err(e),
    }
}

/// Valeurs du nom (catégorie de contenu) tirées de MediaInfo : langue, résolution, codec,
/// audio, format PAL / NTSC, source (COMPLETE.BLURAY, COMPLETE.UHD.BLURAY, DVDR).
pub fn name_values(cfg: &Config, d: &Disc, f: &Facts) -> Vec<(&'static str, String)> {
    let mut v = Vec::new();
    let lang = prep::language_tag(&f.langs, &f.subs, &cfg.undefined_lang);
    if !lang.is_empty() {
        v.push(("langue", lang));
    }
    let res = prep::resolution(f);
    let uhd = f.width >= 3800 || f.height >= 2100;
    match d.kind {
        Kind::Bluray => {
            if !res.is_empty() {
                v.push(("resolution", res));
            }
            v.push(("source", if uhd { "COMPLETE.UHD.BLURAY".into() } else { "COMPLETE.BLURAY".into() }));
            let c = match f.format.as_str() {
                "AVC" => "AVC".to_string(),
                "HEVC" => "HEVC".to_string(),
                _ => prep::video_codec(f),
            };
            if !c.is_empty() {
                v.push(("codec", c));
            }
        }
        Kind::Dvd => {
            v.push(("format", if f.height >= 560 || f.height == 0 { "PAL".into() } else { "NTSC".into() }));
            v.push(("source", "DVDR".into()));
            let c = prep::video_codec(f);
            if !c.is_empty() {
                v.push(("codec", c));
            }
        }
    }
    let a = prep::audio_tag(f);
    if !a.is_empty() {
        v.push(("audio", a));
    }
    v
}

// --------------------------------------------------------------- BDInfo

/// Exécutable BDInfo : réglage, sinon bdinfo-rs ou BDInfo dans le PATH, à côté de
/// l'application, ou dans un dossier « bdinfo* » des Téléchargements / Program Files.
pub fn bdinfo_exe(cfg: &Config) -> Option<PathBuf> {
    // recherche gardée 10 s (l'interface la demande à chaque image)
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<Option<(String, Instant, Option<PathBuf>)>>> = OnceLock::new();
    let c = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(g) = c.lock() {
        if let Some((k, t, v)) = g.as_ref() {
            if k == cfg.bdinfo.trim() && t.elapsed() < Duration::from_secs(10) {
                return v.clone();
            }
        }
    }
    let v = find_bdinfo(cfg);
    if let Ok(mut g) = c.lock() {
        *g = Some((cfg.bdinfo.trim().to_string(), Instant::now(), v.clone()));
    }
    v
}

fn find_bdinfo(cfg: &Config) -> Option<PathBuf> {
    let set = cfg.bdinfo.trim();
    if !set.is_empty() {
        let p = PathBuf::from(set);
        return p.is_file().then_some(p);
    }
    let names: &[&str] = if cfg!(windows) { &["bdinfo-rs.exe", "BDInfo.exe", "bdinfo.exe"] } else { &["bdinfo-rs", "bdinfo", "BDInfo"] };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // installé par l'application (« Installer bdinfo-rs »)
    if let Some(p) = find_in(&crate::app_dir().join("bdinfo"), 3) {
        return Some(p);
    }
    dirs.push(crate::app_dir());
    for base in ["USERPROFILE", "ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"].iter().filter_map(|v| std::env::var_os(v)).map(PathBuf::from) {
        for sub in [base.join("Downloads"), base.join("Téléchargements"), base.join("Programs"), base.clone()] {
            if let Ok(rd) = fs::read_dir(&sub) {
                for e in rd.flatten() {
                    let n = prep::file_name(&e.path()).to_lowercase();
                    if e.path().is_dir() && n.starts_with("bdinfo") {
                        dirs.push(e.path());
                        dirs.push(e.path().join("bin"));
                    }
                }
            }
        }
    }
    for d in dirs {
        for n in names {
            let p = d.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn is_rs(exe: &Path) -> bool {
    prep::file_name(exe).to_lowercase().contains("bdinfo-rs")
}

/// Clé du cache BDInfo d'un disque : nom de l'image ou du film principal + poids (le même
/// disque déplacé du dossier de traitement vers la sortie garde sa clé).
fn cache_key(d: &Disc) -> String {
    let (name, size) = if d.iso {
        (prep::file_name(&d.image), fs::metadata(&d.image).map(|m| m.len()).unwrap_or(0))
    } else {
        let pl = playlists(&d.image).into_iter().next();
        (pl.as_ref().map(|p| prep::file_name(&p.file)).unwrap_or_default(), pl.map(|p| p.bytes).unwrap_or(0))
    };
    let clean: String = name.chars().map(|c| if c.is_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    format!("{clean}-{size}")
}

fn cache_file(d: &Disc) -> PathBuf {
    crate::app_dir().join("cache").join("bdinfo").join(format!("{}.txt", cache_key(d)))
}

/// Oublie le rapport BDInfo gardé pour ce disque (« Relancer BDInfo »).
pub fn forget_cache(d: &Disc) {
    let _ = fs::remove_file(cache_file(d));
}

/// Rapport BDInfo déjà fait pour ce disque (aperçu, puis réutilisé au traitement).
pub fn bdinfo_cached(d: &Disc) -> Option<String> {
    fs::read(cache_file(d)).ok().map(|b| crate::ngpost::decode_text(&b)).filter(|t| !t.trim().is_empty())
}

/// Playlist principale d'après la liste de BDInfo (`--list`) : la ligne au plus gros poids.
pub fn main_from_list(out: &str) -> Option<String> {
    let mut best: Option<(u64, String)> = None;
    for l in out.lines() {
        let low = l.to_lowercase();
        // « 00800.MPLS » ou « 00800 » en début de colonne
        let pl = low.split(|c: char| !c.is_ascii_alphanumeric() && c != '.').find_map(|w| {
            let stem = w.strip_suffix(".mpls").unwrap_or(w);
            (stem.len() == 5 && stem.chars().all(|c| c.is_ascii_digit()) && (w.ends_with(".mpls") || low.contains("mpls") || low.contains(':'))).then(|| stem.to_string())
        });
        let Some(pl) = pl else { continue };
        // poids : le plus grand nombre de la ligne (séparateurs de milliers retirés)
        let bytes = l.split_whitespace().filter_map(|w| {
            let digits: String = w.chars().filter(|c| c.is_ascii_digit()).collect();
            (w.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.' || c == ' ' || c == '\u{a0}') && digits.len() >= 6).then(|| digits.parse::<u64>().ok()).flatten()
        }).max().unwrap_or(0);
        if best.as_ref().map(|(b, _)| bytes > *b).unwrap_or(true) {
            best = Some((bytes, pl));
        }
    }
    best.map(|(_, p)| p)
}

/// Rapport BDInfo du film principal (la plus grosse playlist, comme on la choisirait à la
/// main : BDInfo demande quelle playlist analyser). Scan complet de ses fichiers : plusieurs
/// minutes pour un Blu-ray de 40 Go. Image ISO : montée pour choisir la playlist, sinon liste
/// de BDInfo. Résultat gardé en cache (aperçu avant dépôt, puis traitement sans nouveau scan).
/// `cancel` est consulté deux fois par seconde.
pub fn bdinfo_report(cfg: &Config, d: &Disc, on_line: &mut dyn FnMut(&str), cancel: &dyn Fn() -> bool) -> Result<String, String> {
    if d.kind != Kind::Bluray {
        return Err("BDInfo ne lit que les Blu-ray".into());
    }
    if let Some(t) = bdinfo_cached(d) {
        on_line("BDInfo : rapport déjà fait pour ce disque (réutilisé)");
        return Ok(t);
    }
    let exe = bdinfo_exe(cfg).ok_or("BDInfo introuvable (Réglages → MediaInfo → BDInfo : « Installer bdinfo-rs »)")?;
    let rs = is_rs(&exe);
    let out_dir = crate::app_dir().join("tmp").join(format!("bdinfo-{}-{}", std::process::id(), prep::now_secs()));
    fs::create_dir_all(&out_dir).map_err(|e| format!("dossier temporaire : {e}"))?;
    let scan = |path: &Path, mpls: Option<&str>, on_line: &mut dyn FnMut(&str)| -> Result<String, String> {
        let mut args: Vec<String> = Vec::new();
        if rs {
            args.push("--no-banner".into());
        }
        match mpls {
            Some(stem) if rs => args.extend(["--mpls".into(), stem.to_string()]),
            Some(stem) => args.extend(["-m".into(), format!("{stem}.MPLS")]),
            None => args.push(if rs { "--whole".into() } else { "-w".into() }),
        }
        args.push(path.display().to_string());
        args.push(out_dir.display().to_string());
        on_line(&format!("BDInfo ({}) : {}", prep::file_name(&exe), mpls.map(|m| format!("playlist {m}.mpls")).unwrap_or_else(|| "disque entier".into())));
        let res = run_tool(&exe, &args, cancel);
        let report = fs::read_dir(&out_dir)
            .ok()
            .and_then(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("txt")).unwrap_or(false)).max_by_key(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0)));
        let text = report.and_then(|p| fs::read(p).ok()).map(|b| crate::ngpost::decode_text(&b));
        match (text, res) {
            (_, Err(e)) if e == prep::CANCELLED => Err(e),
            (Some(t), _) if !t.trim().is_empty() => Ok(t),
            (_, Err(e)) => Err(format!("BDInfo : {e}")),
            (_, Ok(out)) => Err(format!("BDInfo n'a pas écrit de rapport ({})", out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("aucune sortie").trim())),
        }
    };
    let stem = |p: &Playlist| p.file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let res = if !d.iso {
        let main = playlists(&d.image).into_iter().next();
        scan(&d.image, main.as_ref().map(stem).as_deref(), on_line)
    } else {
        match mount(&d.image) {
            Ok((root, was)) => {
                let main = playlists(&root).into_iter().next();
                if let Some(m) = &main {
                    on_line(&format!("Image montée en {} : playlist principale {} ({}, {})", root.display(), prep::file_name(&m.file), hms(m.secs), prep::human_size(m.bytes)));
                }
                let r = scan(&root, main.as_ref().map(stem).as_deref(), on_line);
                if !was {
                    unmount(&d.image);
                }
                r
            }
            Err(e) => {
                // pas de montage : la liste des playlists de BDInfo donne la plus grosse
                on_line(&format!("{e} : playlist choisie d'après la liste de BDInfo"));
                let mut largs: Vec<String> = Vec::new();
                if rs {
                    largs.push("--no-banner".into());
                }
                largs.push(if rs { "--list".into() } else { "-l".into() });
                largs.push(d.image.display().to_string());
                largs.push(out_dir.display().to_string());
                let main = run_tool(&exe, &largs, cancel).ok().and_then(|o| main_from_list(&o));
                scan(&d.image, main.as_deref(), on_line)
            }
        }
    };
    let _ = fs::remove_dir_all(&out_dir);
    if let Ok(t) = &res {
        let f = cache_file(d);
        if let Some(dir) = f.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(f, t);
    }
    res
}

/// Installe bdinfo-rs (Windows) : dernière version publiée sur GitHub, archive vérifiée par son
/// empreinte SHA-256, décompressée dans le dossier « bdinfo » de l'application.
#[cfg(windows)]
pub fn install_bdinfo() -> Result<PathBuf, String> {
    let dest = crate::app_dir().join("bdinfo");
    let out = powershell_env(PS_INSTALL, &[("NZBFF_DEST", dest.display().to_string())], 600).map_err(|e| format!("installation de bdinfo-rs : {e}"))?;
    let tag = out.lines().find_map(|l| l.trim().strip_prefix("TAG=").map(str::to_string)).unwrap_or_default();
    let exe = find_in(&dest, 3).ok_or_else(|| format!("bdinfo-rs {tag} téléchargé mais bdinfo-rs.exe introuvable dans {}", dest.display()))?;
    Ok(exe)
}

#[cfg(not(windows))]
pub fn install_bdinfo() -> Result<PathBuf, String> {
    Err("installation automatique : Windows seulement".into())
}

#[cfg(windows)]
const PS_INSTALL: &str = r#"$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$h=@{ 'User-Agent'='nzb-ff-tool-pro' }; $n='bdinfo-rs-x86_64-pc-windows-msvc.zip'; $dest=$env:NZBFF_DEST
$rel=Invoke-RestMethod -Uri 'https://api.github.com/repos/agentjp/bdinfo-rs/releases?per_page=30' -Headers $h
$z=$null; $s=$null; $tag=''
foreach($r in $rel){ if($r.prerelease -or $r.draft){ continue }; $z=$r.assets | Where-Object { $_.name -eq $n } | Select-Object -First 1; if($z){ $s=$r.assets | Where-Object { $_.name -eq ($n+'.sha256') } | Select-Object -First 1; $tag=$r.tag_name; break } }
if(-not $z){ throw "archive $n introuvable dans les versions de bdinfo-rs" }
New-Item -ItemType Directory -Force -Path $dest | Out-Null
$zip=Join-Path $dest $n
Invoke-WebRequest -Uri $z.browser_download_url -OutFile $zip -UseBasicParsing -Headers $h
if($s){ $sf=$zip+'.sha256'; Invoke-WebRequest -Uri $s.browser_download_url -OutFile $sf -UseBasicParsing -Headers $h; $want=((Get-Content -Raw $sf).Trim() -split '\s+')[0].ToLower(); Remove-Item $sf; $got=(Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower(); if($want -ne $got){ Remove-Item $zip; throw "empreinte SHA-256 différente : archive refusée" }; "SHA=ok" } else { "SHA=absente" }
Expand-Archive -Path $zip -DestinationPath $dest -Force
Remove-Item $zip
"TAG=$tag""#;

/// bdinfo-rs.exe / BDInfo.exe dans un dossier et ses sous-dossiers (profondeur limitée).
fn find_in(dir: &Path, depth: u32) -> Option<PathBuf> {
    let names = ["bdinfo-rs.exe", "bdinfo-rs", "bdinfo.exe"];
    let rd: Vec<PathBuf> = fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    if let Some(p) = rd.iter().find(|p| p.is_file() && names.iter().any(|n| prep::file_name(p).eq_ignore_ascii_case(n))) {
        return Some(p.clone());
    }
    if depth == 0 {
        return None;
    }
    rd.iter().filter(|p| p.is_dir()).find_map(|p| find_in(p, depth - 1))
}

fn run_tool(exe: &Path, args: &[String], cancel: &dyn Fn() -> bool) -> Result<String, String> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut so = child.stdout.take();
    let mut se = child.stderr.take();
    let h1 = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = so.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    let h2 = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = se.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st;
        }
        if cancel() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(prep::CANCELLED.to_string());
        }
        // garde-fou : 3 h au plus
        if start.elapsed() > Duration::from_secs(3 * 3600) {
            let _ = child.kill();
            return Err("plus de 3 h : arrêté".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    let out = h1.join().unwrap_or_default();
    let err = h2.join().unwrap_or_default();
    if status.success() || status.code() == Some(3) {
        Ok(out)
    } else {
        let e = err.lines().chain(out.lines()).map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("").to_string();
        Err(match status.code() {
            Some(4) => "disque chiffré (AACS) : BDInfo ne lit que les disques déchiffrés".into(),
            c => format!("code {} {e}", c.map(|c| c.to_string()).unwrap_or_else(|| "?".into())),
        })
    }
}

// ---------------------------------------------------------------- rapport BDInfo propre

/// Parties d'un rapport BDInfo : (clé, titre dans le rapport, nom affiché).
pub const BD_SECTIONS: &[(&str, &str, &str)] = &[
    ("disc", "DISC INFO", "Infos du disque (titre, taille, protection)"),
    ("forum", "", "Ligne de résumé « forum » (titre, codec, durée, tailles, débit)"),
    ("playlist", "PLAYLIST REPORT", "Playlist (nom, durée, taille, débit total)"),
    ("video", "VIDEO", "Vidéo"),
    ("audio", "AUDIO", "Audio"),
    ("subtitles", "SUBTITLES", "Sous-titres"),
    ("files", "FILES", "Fichiers .m2ts"),
    ("chapters", "CHAPTERS", "Chapitres"),
    ("diagnostics", "STREAM DIAGNOSTICS", "Diagnostic des flux"),
    ("summary", "QUICK SUMMARY", "Résumé rapide"),
];

/// Exemple de rapport BDInfoCLI (forums paste, publicité, notes) : aperçu du modèle dans les
/// réglages, et tests.
pub const BD_SAMPLE: &str = "Disc Title:     MOVIE_2010\nDisc Label:     MOVIE_2010\nDisc Size:      46 212 345 678 bytes\nProtection:     AACS\nExtras:         BD-Java\nBDInfo:         0.7.5.6\n\nNotes:\n\nBDINFO HOME:\n  Cinema Squid (old)\n    http://www.cinemasquid.com/blu-ray/tools/bdinfo\n  UniqProject GitHub (new)\n   https://github.com/UniqProject/BDInfo\n\nINCLUDES FORUMS REPORT FOR:\n  AVS Forum Blu-ray Audio and Video Specifications Thread\n    http://www.avsforum.com/avs-vb/showthread.php?t=1155731\n\n********************\nPLAYLIST: 00800.MPLS\n********************\n\n<--- BEGIN FORUMS PASTE --->\n[code]\n                                                     Total   Video\nTitle            Codec   Length  Movie Size   Disc Size   Bitrate  Bitrate  Main Audio Track\n-----            ------  ------- ----------   ---------   -------  -------  ----------------\n00800.MPLS       AVC     2:13:41 41 123 456   46 212 345  41.01    29.50    DTS-HD Master Audio\n[/code]\n\n[code]\n\nDISC INFO:\n\nDisc Title:     MOVIE_2010\nDisc Size:      46 212 345 678 bytes\nProtection:     AACS\nBD-Java:        Yes\nBDInfo:         0.7.5.6\n\nPLAYLIST REPORT:\n\nName:                   00800.MPLS\nLength:                 2:13:41.894 (h:m:s.ms)\nTotal Bitrate:          41.01 Mbps\n\nVIDEO:\n\nCodec                   Bitrate             Description\n-----                   -------             -----------\nMPEG-4 AVC Video        29498 kbps          1080p / 23.976 fps / 16:9 / High Profile 4.1\n\nAUDIO:\n\nCodec                           Language        Bitrate         Description\n-----                           --------        -------         -----------\nDTS-HD Master Audio             French          3958 kbps       5.1 / 48 kHz\n\nSUBTITLES:\n\nCodec                           Language        Bitrate         Description\n-----                           --------        -------         -----------\nPresentation Graphics           French          31.893 kbps\n\nFILES:\n\nName            Time In         Length\n00001.M2TS      0:00:00.000     2:13:41.894\n\nCHAPTERS:\n\nNumber          Time In\n1               0:00:00.000\n\nSTREAM DIAGNOSTICS:\n\nFile            PID\n00001.M2TS      4113\n\n[/code]\n<---- END FORUMS PASTE ---->\n\nQUICK SUMMARY:\n\nDisc Title: MOVIE_2010\nVideo: MPEG-4 AVC Video / 29498 kbps / 1080p\nAudio: French / DTS-HD Master Audio / 5.1\n";

/// Parties gardées par défaut : ce qu'attendent les indexers, sans le superflu.
pub const BD_DEFAULT_SECTIONS: &str = "disc,playlist,video,audio,subtitles";
pub const BD_DEFAULT_TEMPLATE: &str = "{bdinfo}";

/// Champs du modèle BDInfo : (champ, explication).
pub const BD_FIELDS: &[(&str, &str)] = &[
    ("{bdinfo}", "les parties cochées, dans l'ordre du rapport"),
    ("{release}", "nom de la release"),
    ("{disque}", "infos du disque"),
    ("{playlist}", "playlist principale"),
    ("{video}", "pistes vidéo"),
    ("{audio}", "pistes audio"),
    ("{soustitres}", "sous-titres"),
    ("{resume}", "résumé rapide"),
];

/// Titres de sections reconnus (publicité et notes comprises, pour les retirer).
fn bd_heading(line: &str) -> Option<&'static str> {
    let t = line.trim().trim_end_matches(':').trim();
    if !line.trim_end().ends_with(':') {
        return None;
    }
    let up = t.to_ascii_uppercase();
    for (k, h, _) in BD_SECTIONS {
        if !h.is_empty() && up == *h {
            return Some(k);
        }
    }
    match up.as_str() {
        "BDINFO HOME" | "INCLUDES FORUMS REPORT FOR" | "NOTES" => Some("ads"),
        _ => None,
    }
}

/// Découpe un rapport BDInfo (BDInfo, BDInfoCLI ou bdinfo-rs) en parties, sans la publicité :
/// liens « BDInfo Home », « Includes forums report for », notes, balises [code], bandeaux.
pub fn bd_parts(raw: &str) -> Vec<(&'static str, String)> {
    let mut parts: Vec<(&'static str, Vec<String>)> = Vec::new();
    let mut cur: &'static str = "head";
    let mut head: Vec<String> = Vec::new();
    let mut lines = raw.lines().map(|l| l.trim_end().replace('\t', "    ")).peekable();
    while let Some(l) = lines.next() {
        let t = l.trim();
        // emballage « forum » et bandeaux : jamais gardés
        if t.eq_ignore_ascii_case("[code]") || t.eq_ignore_ascii_case("[/code]") || t.contains("FORUMS PASTE") || (!t.is_empty() && t.chars().all(|c| c == '*')) || t.to_ascii_uppercase().starts_with("PLAYLIST: ") {
            continue;
        }
        // ligne de résumé du forum (tableau Title / Codec / Length…)
        if t.starts_with("Title") && t.contains("Codec") && t.contains("Length") {
            let mut v = vec![l.clone()];
            while let Some(n) = lines.peek() {
                if n.trim().is_empty() || n.trim().eq_ignore_ascii_case("[/code]") {
                    break;
                }
                v.push(lines.next().unwrap_or_default());
            }
            parts.push(("forum", v));
            cur = "after-forum";
            continue;
        }
        if t.starts_with("Total") && lines.peek().map(|n| n.trim_start().starts_with("Title")).unwrap_or(false) {
            // première ligne d'en-tête du tableau du forum (« Total Video »)
            continue;
        }
        if let Some(k) = bd_heading(&l) {
            cur = k;
            parts.push((k, Vec::new()));
            continue;
        }
        // adresses (publicité) hors des parties utiles
        if t.starts_with("http://") || t.starts_with("https://") {
            continue;
        }
        match cur {
            "head" => head.push(l),
            "after-forum" => {}
            _ => {
                if let Some(p) = parts.last_mut() {
                    p.1.push(l);
                }
            }
        }
    }
    let clean = |v: &[String]| {
        let s = v.join("\n");
        s.trim_matches('\n').trim_end().to_string()
    };
    let mut out: Vec<(&'static str, String)> = Vec::new();
    // pas de « DISC INFO » : l'en-tête du rapport (Disc Title, Disc Size…) en tient lieu
    if !parts.iter().any(|(k, _)| *k == "disc") {
        let h: Vec<String> = head.into_iter().filter(|l| l.contains(':') && !l.trim().is_empty()).collect();
        if !h.is_empty() {
            out.push(("disc", clean(&h)));
        }
    }
    for (k, v) in parts {
        if k == "ads" {
            continue;
        }
        let body = clean(&v);
        if body.is_empty() {
            continue;
        }
        // une partie peut revenir (bdinfo-rs répète l'en-tête) : la première suffit
        if !out.iter().any(|(x, _)| *x == k) {
            out.push((k, body));
        }
    }
    out
}

/// Rapport BDInfo propre, selon le modèle des réglages (parties gardées + texte du modèle).
pub fn bdinfo_render(raw: &str, sections: &str, template: &str, release: &str) -> String {
    let parts = bd_parts(raw);
    let keep: Vec<&str> = sections.split([',', ' ', ';']).map(str::trim).filter(|s| !s.is_empty()).collect();
    let block = |k: &str| -> String {
        let Some((_, body)) = parts.iter().find(|(x, _)| *x == k) else { return String::new() };
        match BD_SECTIONS.iter().find(|(x, _, _)| *x == k) {
            Some((_, h, _)) if !h.is_empty() => format!("{h}:\n\n{body}"),
            _ => body.clone(),
        }
    };
    let order = ["disc", "forum", "playlist", "video", "audio", "subtitles", "files", "chapters", "diagnostics", "summary"];
    let all: Vec<String> = order.iter().filter(|k| keep.contains(k)).map(|k| block(k)).filter(|b| !b.is_empty()).collect();
    let tpl = if template.trim().is_empty() { BD_DEFAULT_TEMPLATE } else { template };
    let out = tpl
        .replace("{bdinfo}", &all.join("\n\n"))
        .replace("{release}", release)
        .replace("{disque}", &block("disc"))
        .replace("{playlist}", &block("playlist"))
        .replace("{video}", &block("video"))
        .replace("{audio}", &block("audio"))
        .replace("{soustitres}", &block("subtitles"))
        .replace("{resume}", &block("summary"));
    // pas plus d'une ligne vide de suite
    let mut res = String::new();
    let mut blank = 0;
    for l in out.lines() {
        if l.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        res.push_str(l.trim_end());
        res.push('\n');
    }
    res.trim().to_string() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn bdinfo_propre_selon_le_modele() {
        let out = bdinfo_render(BD_SAMPLE, BD_DEFAULT_SECTIONS, BD_DEFAULT_TEMPLATE, "Film.2010.COMPLETE.BLURAY-GRP");
        for absent in ["http", "cinemasquid", "BDINFO HOME", "Notes:", "INCLUDES FORUMS", "[code]", "FORUMS PASTE", "*****", "FILES:", "CHAPTERS:", "STREAM DIAGNOSTICS:", "QUICK SUMMARY:", "Title            Codec"] {
            assert!(!out.contains(absent), "{absent} :\n{out}");
        }
        for present in ["DISC INFO:\n\nDisc Title:     MOVIE_2010", "PLAYLIST REPORT:", "VIDEO:", "MPEG-4 AVC Video        29498 kbps", "AUDIO:", "DTS-HD Master Audio             French", "SUBTITLES:"] {
            assert!(out.contains(present), "{present} :\n{out}");
        }
        assert!(!out.contains("\n\n\n"));
        // modèle avec le nom de la release, résumé seulement
        let out = bdinfo_render(BD_SAMPLE, "summary", "{release}\n\n{resume}", "Film.2010.COMPLETE.BLURAY-GRP");
        assert_eq!(out, "Film.2010.COMPLETE.BLURAY-GRP\n\nQUICK SUMMARY:\n\nDisc Title: MOVIE_2010\nVideo: MPEG-4 AVC Video / 29498 kbps / 1080p\nAudio: French / DTS-HD Master Audio / 5.1\n");
        // ligne du forum
        let out = bdinfo_render(BD_SAMPLE, "forum", "{bdinfo}", "x");
        assert!(out.starts_with("Title") && out.contains("00800.MPLS       AVC"), "{out}");
        // rapport sans « DISC INFO » (en-tête seul) : l'en-tête sert d'infos du disque, sans les notes
        let out = bdinfo_render("Disc Title: X\nDisc Size: 1 bytes\n\nNotes:\nhttp://pub\n\nVIDEO:\n\nAVC 1080p\n", "disc,video", "{bdinfo}", "x");
        assert_eq!(out, "DISC INFO:\n\nDisc Title: X\nDisc Size: 1 bytes\n\nVIDEO:\n\nAVC 1080p\n");
    }

    fn mpls(items: &[(&str, u32, u32)]) -> Vec<u8> {
        let mut d = b"MPLS0200".to_vec();
        d.extend_from_slice(&40u32.to_be_bytes()); // PlayList
        d.extend_from_slice(&0u32.to_be_bytes()); // PlayListMark
        d.extend_from_slice(&0u32.to_be_bytes()); // extension
        d.resize(40, 0);
        let mut pl = vec![0u8, 0]; // reserved
        pl.extend_from_slice(&(items.len() as u16).to_be_bytes());
        pl.extend_from_slice(&0u16.to_be_bytes());
        for (name, tin, tout) in items {
            let mut it = name.as_bytes().to_vec();
            it.extend_from_slice(b"M2TS");
            it.extend_from_slice(&[0, 1, 0]);
            it.extend_from_slice(&tin.to_be_bytes());
            it.extend_from_slice(&tout.to_be_bytes());
            it.extend_from_slice(&[0u8; 12]);
            pl.extend_from_slice(&(it.len() as u16).to_be_bytes());
            pl.extend_from_slice(&it);
        }
        d.extend_from_slice(&(pl.len() as u32).to_be_bytes());
        d.extend_from_slice(&pl);
        d
    }

    #[test]
    fn bluray_dvd_et_titre_principal() {
        let base = std::env::temp_dir().join(format!("nzbff-disc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        // Blu-ray : une playlist principale de 2 h, une de bonus, une qui boucle sur un petit clip
        let bd = base.join("Film.2010.BLURAY");
        let pl = bd.join("BDMV/PLAYLIST");
        let st = bd.join("BDMV/STREAM");
        fs::create_dir_all(&pl).unwrap();
        fs::create_dir_all(&st).unwrap();
        fs::write(bd.join("BDMV/index.bdmv"), b"INDX0200").unwrap();
        fs::write(st.join("00001.m2ts"), vec![0u8; 5000]).unwrap();
        fs::write(st.join("00002.m2ts"), vec![0u8; 3000]).unwrap();
        fs::write(st.join("00009.m2ts"), vec![0u8; 100]).unwrap();
        fs::write(pl.join("00800.mpls"), mpls(&[("00001", 0, 45_000 * 3600), ("00002", 0, 45_000 * 3600)])).unwrap();
        fs::write(pl.join("00010.mpls"), mpls(&[("00009", 0, 45_000 * 60)])).unwrap();
        fs::write(pl.join("00099.mpls"), mpls(&[("00009", 0, 45_000 * 3000); 6])).unwrap();
        let d = detect(&bd).unwrap();
        assert_eq!((d.kind, d.iso), (Kind::Bluray, false));
        assert_eq!(detect(&bd.join("BDMV")).unwrap().root, bd);
        let p = playlists(&bd);
        assert_eq!(prep::file_name(&p[0].file), "00800.mpls");
        assert_eq!((p[0].secs as u64, p[0].bytes, p[0].clips.len()), (7200, 8000, 2));
        assert!(main_title_text(Kind::Bluray, &bd).contains("2h00"));
        let t: Vec<String> = media_targets(Kind::Bluray, &bd).iter().map(|p| prep::file_name(p)).collect();
        assert_eq!(t, vec!["00800.mpls", "00001.m2ts"]);
        assert!(inside_disc(&st.join("00001.m2ts")) && !inside_disc(&bd.join("film.nfo")));
        // le dossier qui contient le disque (avec son NFO) reste la racine livrée
        let outer = base.join("Mon Film (2010)");
        fs::create_dir_all(&outer).unwrap();
        fs::rename(&bd, outer.join("DISC")).unwrap();
        fs::write(outer.join("film.nfo"), b"nfo").unwrap();
        let d = detect(&outer).unwrap();
        assert_eq!((d.kind, d.root.clone(), d.image.clone()), (Kind::Bluray, outer.clone(), outer.join("DISC")));
        // MediaInfo et BDInfo lisent bien le disque du sous-dossier
        let t: Vec<String> = with_root(&d, |r| media_targets(d.kind, r)).unwrap().iter().map(|p| prep::file_name(p)).collect();
        assert_eq!(t, vec!["00800.mpls", "00001.m2ts"]);
        assert_eq!(prep::list_videos(&outer, true), Vec::<PathBuf>::new());
        // DVD : le jeu de titres le plus lourd
        let dvd = base.join("DVD").join("VIDEO_TS");
        fs::create_dir_all(&dvd).unwrap();
        for (n, sz) in [("VIDEO_TS.IFO", 10), ("VTS_01_0.IFO", 10), ("VTS_01_1.VOB", 200), ("VTS_02_0.IFO", 10), ("VTS_02_0.VOB", 9000), ("VTS_02_1.VOB", 5000), ("VTS_02_2.VOB", 4000)] {
            fs::write(dvd.join(n), vec![0u8; sz]).unwrap();
        }
        let d = detect(&base.join("DVD")).unwrap();
        assert_eq!(d.kind, Kind::Dvd);
        let (ifo, b) = dvd_main(&base.join("DVD")).unwrap();
        assert_eq!((prep::file_name(&ifo).as_str(), b), ("VTS_02_0.IFO", 9000));
        let t: Vec<String> = media_targets(Kind::Dvd, &base.join("DVD")).iter().map(|p| prep::file_name(p)).collect();
        assert_eq!(t, vec!["VTS_02_0.IFO", "VTS_02_1.VOB"]);
        // images ISO : repérées sans être montées
        let iso = base.join("film.iso");
        let mut data = vec![0u8; 300_000];
        data[200_000..200_005].copy_from_slice(b"\x08BDMV");
        fs::write(&iso, &data).unwrap();
        assert_eq!(detect(&iso).map(|d| (d.kind, d.iso)), Some((Kind::Bluray, true)));
        let mut data = vec![0u8; 300_000];
        data[40_000..40_008].copy_from_slice(b"VIDEO_TS");
        fs::write(&iso, &data).unwrap();
        assert_eq!(iso_kind(&iso), Some(Kind::Dvd));
        fs::write(&iso, vec![7u8; 100_000]).unwrap();
        assert_eq!(detect(&iso), None, "ISO d'un jeu ou d'un logiciel : pas un disque vidéo");
        // dossier de release : Sample, Proof, film.iso, film.nfo (rangement des releases FBR)
        let rel = base.join("Film");
        fs::create_dir_all(rel.join("Sample")).unwrap();
        fs::create_dir_all(rel.join("Proof")).unwrap();
        fs::write(rel.join("Sample").join("film-sample.mkv"), b"x").unwrap();
        fs::write(rel.join("Proof").join("proof.jpg"), b"x").unwrap();
        fs::write(rel.join("film.nfo"), b"nfo").unwrap();
        let mut data = vec![0u8; 400_000];
        data[300_000..300_005].copy_from_slice(b"\x08BDMV");
        fs::write(rel.join("film.iso"), &data).unwrap();
        let d = detect(&rel).unwrap();
        assert_eq!((d.kind, d.iso, d.root.clone(), d.image.clone()), (Kind::Bluray, true, rel.clone(), rel.join("film.iso")));
        assert!(prep::list_videos(&rel, true).is_empty(), "l'échantillon n'est pas un film");
        assert!(prep::list_videos(&base, true).is_empty());
        let cats = crate::content::defaults();
        assert_eq!(crate::content::guess(&rel, &cats).as_deref(), Some("bluray"));
        // image non reconnue à son contenu : d'après le nom et la taille
        assert_eq!(iso_kind_by_name(Path::new("/x/Film.2010.MULTi.COMPLETE.BLURAY-GRP/f.iso"), 40 << 30), Some(Kind::Bluray));
        assert_eq!(iso_kind_by_name(Path::new("/x/Film.2010/film.iso"), 40 << 30), Some(Kind::Bluray));
        assert_eq!(iso_kind_by_name(Path::new("/x/Jeu.2010.PS4-GRP/jeu.iso"), 40 << 30), None);
        assert_eq!(iso_kind_by_name(Path::new("/x/Film.1985.FRENCH.PAL.DVDR/f.iso"), 7 << 30), Some(Kind::Dvd));
        assert_eq!(iso_kind_by_name(Path::new("/x/Logiciel/setup.iso"), 3 << 30), None);
        // playlist principale d'après la liste de BDInfo
        let list = "#   Group  Playlist File  Length    Estimated Bytes  Measured Bytes\n1   1      00020.MPLS     00:02:00  210,000,000      -\n2   2      00800.MPLS     02:28:00  45,123,456,789   -\n3   3      00801.MPLS     02:28:00  44,000,000,000   -\n";
        assert_eq!(main_from_list(list).as_deref(), Some("00800"));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn valeurs_du_nom() {
        let cfg = Config::default();
        let d = Disc { kind: Kind::Bluray, root: PathBuf::from("x"), image: PathBuf::from("x"), iso: false };
        let f = Facts { width: 3840, height: 2160, format: "HEVC".into(), langs: vec!["fr".into(), "en".into()], audio_format: "MLP FBA".into(), audio_features: "16-ch".into(), audio_channels: "8".into(), ..Default::default() };
        let v = name_values(&cfg, &d, &f);
        let get = |k: &str| v.iter().find(|(x, _)| *x == k).map(|(_, y)| y.clone()).unwrap_or_default();
        assert_eq!((get("langue").as_str(), get("resolution").as_str(), get("source").as_str(), get("codec").as_str(), get("audio").as_str()), ("MULTi", "2160p", "COMPLETE.UHD.BLURAY", "HEVC", "TrueHD.7.1.Atmos"));
        let d = Disc { kind: Kind::Dvd, ..d };
        let f = Facts { width: 720, height: 576, format: "MPEG Video".into(), format_version: "Version 2".into(), langs: vec!["fr".into()], audio_format: "AC-3".into(), audio_channels: "2".into(), ..Default::default() };
        let v = name_values(&cfg, &d, &f);
        let get = |k: &str| v.iter().find(|(x, _)| *x == k).map(|(_, y)| y.clone()).unwrap_or_default();
        assert_eq!((get("langue").as_str(), get("format").as_str(), get("source").as_str(), get("codec").as_str(), get("audio").as_str()), ("FRENCH", "PAL", "DVDR", "MPEG2", "AC3.2.0"));
    }
}
