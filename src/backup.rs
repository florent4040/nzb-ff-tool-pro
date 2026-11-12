//! Sauvegarde et restauration de la configuration (et, au choix, de l'historique et des
//! statistiques), dans un seul fichier `.nzbff`, chiffré ou non par mot de passe.
//!
//! Chiffrement : ChaCha20-Poly1305 (RFC 8439), clé tirée du mot de passe par
//! PBKDF2-HMAC-SHA256 (sel aléatoire de 16 octets). Algorithmes standard, vérifiés par les
//! vecteurs de test officiels (voir les tests en bas de ce fichier).
//!
//! Il y a aussi une sauvegarde automatique, hors du dossier de l'application (profil
//! Windows) : une nouvelle version installée dans un autre dossier reprend tout seule
//! les réglages, l'historique et les statistiques.

use std::fs;
use std::path::{Path, PathBuf};

/// Fichiers qu'une sauvegarde peut contenir (rien d'autre n'est jamais écrit à la restauration).
pub const FILES: &[(&str, &str)] = &[
    ("nzbff.conf", "Configuration complète"),
    ("nzbff-history.tsv", "Historique"),
    ("nzbff-posts.tsv", "Statistiques des posts"),
    ("nzbff-speed.tsv", "Statistiques de débit"),
];

const MAGIC: &[u8] = b"NZBFF-SAUVEGARDE";
const FORMAT: u8 = 1;
const FLAG_CRYPT: u8 = 1;
/// Itérations PBKDF2 (recommandation OWASP pour SHA-256).
pub const ITERATIONS: u32 = 600_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub enum BackupError {
    /// Fichier chiffré : il faut le mot de passe.
    NeedPassword,
    /// Mot de passe faux (ou fichier modifié).
    WrongPassword,
    /// Pas une sauvegarde de l'application, ou fichier abîmé.
    Invalid(String),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::NeedPassword => write!(f, "sauvegarde chiffrée : mot de passe nécessaire"),
            BackupError::WrongPassword => write!(f, "mot de passe incorrect (ou fichier modifié)"),
            BackupError::Invalid(m) => write!(f, "{m}"),
        }
    }
}

/// Libellé lisible d'un fichier de sauvegarde.
pub fn label(name: &str) -> &'static str {
    FILES.iter().find(|f| f.0 == name).map(|f| f.1).unwrap_or("?")
}

/// Contenu à sauvegarder : la configuration, l'historique et les statistiques, au choix.
pub fn collect(dir: &Path, conf: bool, history: bool, stats: bool) -> Vec<Entry> {
    FILES
        .iter()
        .filter(|(n, _)| match *n {
            "nzbff.conf" => conf,
            "nzbff-history.tsv" => history,
            _ => stats,
        })
        .filter_map(|(n, _)| fs::read(dir.join(n)).ok().map(|data| Entry { name: n.to_string(), data }))
        .collect()
}

fn serialize(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in entries {
        out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        out.extend_from_slice(e.name.as_bytes());
        out.extend_from_slice(&(e.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&e.data);
    }
    out
}

fn deserialize(mut b: &[u8]) -> Result<Vec<Entry>, BackupError> {
    let bad = || BackupError::Invalid("contenu de la sauvegarde illisible".into());
    let mut out = Vec::new();
    while !b.is_empty() {
        if b.len() < 2 {
            return Err(bad());
        }
        let n = u16::from_le_bytes([b[0], b[1]]) as usize;
        b = &b[2..];
        if b.len() < n + 4 {
            return Err(bad());
        }
        let name = String::from_utf8(b[..n].to_vec()).map_err(|_| bad())?;
        b = &b[n..];
        let len = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
        b = &b[4..];
        if b.len() < len {
            return Err(bad());
        }
        out.push(Entry { name, data: b[..len].to_vec() });
        b = &b[len..];
    }
    Ok(out)
}

/// Octets aléatoires (sel, nonce) : générateur du système.
fn random_bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    if !os_random(&mut v) {
        // repli : mélange d'horloge, d'adresse et d'identifiant de processus passé par SHA-256
        let mut seed = Vec::new();
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        seed.extend_from_slice(&t.to_le_bytes());
        seed.extend_from_slice(&std::process::id().to_le_bytes());
        seed.extend_from_slice(&(&v as *const _ as usize).to_le_bytes());
        let mut k = 0u32;
        let mut i = 0;
        while i < n {
            let mut s = seed.clone();
            s.extend_from_slice(&k.to_le_bytes());
            for &x in sha256(&s).iter() {
                if i < n {
                    v[i] = x;
                    i += 1;
                }
            }
            k += 1;
        }
    }
    v
}

#[cfg(windows)]
fn os_random(buf: &mut [u8]) -> bool {
    #[link(name = "advapi32")]
    extern "system" {
        #[link_name = "SystemFunction036"]
        fn RtlGenRandom(buf: *mut u8, len: u32) -> u8;
    }
    unsafe { RtlGenRandom(buf.as_mut_ptr(), buf.len() as u32) != 0 }
}

#[cfg(not(windows))]
fn os_random(buf: &mut [u8]) -> bool {
    use std::io::Read;
    fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf)).is_ok()
}

/// Jeton aléatoire alphanumérique (mot de passe d'archives commun à plusieurs lots).
pub fn random_token(n: usize) -> String {
    const A: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    random_bytes(n).iter().map(|b| A[*b as usize % A.len()] as char).collect()
}

/// Fichier de sauvegarde complet (chiffré si un mot de passe est donné).
pub fn pack(entries: &[Entry], password: Option<&str>) -> Vec<u8> {
    pack_with(entries, password, ITERATIONS)
}

fn pack_with(entries: &[Entry], password: Option<&str>, iterations: u32) -> Vec<u8> {
    let plain = serialize(entries);
    let mut out = MAGIC.to_vec();
    out.push(FORMAT);
    match password.filter(|p| !p.is_empty()) {
        None => {
            out.push(0);
            out.extend_from_slice(&plain);
        }
        Some(pw) => {
            out.push(FLAG_CRYPT);
            let salt = random_bytes(16);
            let nonce = random_bytes(12);
            out.extend_from_slice(&salt);
            out.extend_from_slice(&iterations.to_le_bytes());
            out.extend_from_slice(&nonce);
            let key = pbkdf2_sha256(pw.as_bytes(), &salt, iterations);
            // l'en-tête entier est authentifié avec le contenu
            let aad = out.clone();
            let n: [u8; 12] = nonce.try_into().unwrap_or([0; 12]);
            out.extend_from_slice(&aead_seal(&key, &n, &aad, &plain));
        }
    }
    out
}

/// Sauvegarde chiffrée ?
pub fn is_encrypted(bytes: &[u8]) -> bool {
    bytes.len() > MAGIC.len() + 1 && bytes.starts_with(MAGIC) && bytes[MAGIC.len() + 1] & FLAG_CRYPT != 0
}

/// Lit un fichier de sauvegarde.
pub fn unpack(bytes: &[u8], password: Option<&str>) -> Result<Vec<Entry>, BackupError> {
    if !bytes.starts_with(MAGIC) || bytes.len() < MAGIC.len() + 2 {
        return Err(BackupError::Invalid("ce fichier n'est pas une sauvegarde de NZB FF Tool Pro".into()));
    }
    let h = MAGIC.len();
    if bytes[h] != FORMAT {
        return Err(BackupError::Invalid(format!("format de sauvegarde {} inconnu (version plus récente de l'application ?)", bytes[h])));
    }
    let entries = if bytes[h + 1] & FLAG_CRYPT == 0 {
        deserialize(&bytes[h + 2..])?
    } else {
        let pw = password.filter(|p| !p.is_empty()).ok_or(BackupError::NeedPassword)?;
        let start = h + 2;
        let body = start + 16 + 4 + 12;
        if bytes.len() < body + 16 {
            return Err(BackupError::Invalid("sauvegarde chiffrée tronquée".into()));
        }
        let salt = &bytes[start..start + 16];
        let iterations = u32::from_le_bytes(bytes[start + 16..start + 20].try_into().unwrap_or([0; 4]));
        if iterations == 0 || iterations > 10_000_000 {
            return Err(BackupError::Invalid("en-tête de sauvegarde invalide".into()));
        }
        let nonce: [u8; 12] = bytes[start + 20..body].try_into().unwrap_or([0; 12]);
        let key = pbkdf2_sha256(pw.as_bytes(), salt, iterations);
        let plain = aead_open(&key, &nonce, &bytes[..body], &bytes[body..]).ok_or(BackupError::WrongPassword)?;
        deserialize(&plain)?
    };
    // une sauvegarde peut ne contenir que l'historique ou les statistiques (export au choix)
    if !entries.iter().any(|e| FILES.iter().any(|f| f.0 == e.name)) {
        return Err(BackupError::Invalid("la sauvegarde ne contient rien de cette application".into()));
    }
    Ok(entries)
}

/// Écrit les fichiers d'une sauvegarde dans le dossier de l'application. Chaque fichier
/// remplacé est d'abord copié en `.avant-restauration`. Renvoie les libellés restaurés.
pub fn restore(dir: &Path, entries: &[Entry]) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    for e in entries {
        // seuls les fichiers connus, par leur nom exact (jamais de chemin)
        if !FILES.iter().any(|f| f.0 == e.name) {
            continue;
        }
        let p = dir.join(&e.name);
        if p.exists() {
            let _ = fs::copy(&p, dir.join(format!("{}.avant-restauration", e.name)));
        }
        let tmp = dir.join(format!("{}.restauration", e.name));
        fs::write(&tmp, &e.data).map_err(|err| format!("écriture de {} impossible : {err}", e.name))?;
        fs::rename(&tmp, &p).map_err(|err| format!("remplacement de {} impossible : {err}", e.name))?;
        done.push(label(&e.name).to_string());
    }
    Ok(done)
}

// ------------------------------------------------------------ sauvegarde automatique

/// Dossier de la sauvegarde automatique, hors du dossier de l'application :
/// `%APPDATA%\NZB FF Tool Pro\sauvegarde` sous Windows.
pub fn auto_dir() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("NZBFF_AUTO_BACKUP") {
        return Some(PathBuf::from(v));
    }
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(crate::APP_NAME).join("sauvegarde"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("nzb-ff-tool-pro").join("sauvegarde"))
    }
}

/// Copie les fichiers de l'application qui ont changé dans la sauvegarde automatique,
/// plus une copie datée des réglages par jour (les 10 dernières sont gardées).
pub fn auto_save(app: &Path, to: &Path) -> std::io::Result<usize> {
    fs::create_dir_all(to)?;
    let mut n = 0;
    for (name, _) in FILES {
        let src = app.join(name);
        let Ok(data) = fs::read(&src) else { continue };
        let dst = to.join(name);
        if fs::read(&dst).ok().as_deref() != Some(&data[..]) {
            let tmp = to.join(format!("{name}.tmp"));
            fs::write(&tmp, &data)?;
            fs::rename(&tmp, &dst)?;
            n += 1;
        }
        if *name == "nzbff.conf" {
            let day = crate::history::local_time(crate::prep::now_secs());
            let dated = to.join(format!("nzbff-{:04}-{:02}-{:02}.conf", day.year, day.month, day.day));
            if fs::read(&dated).ok().as_deref() != Some(&data[..]) {
                fs::write(&dated, &data)?;
            }
            let mut old: Vec<PathBuf> = fs::read_dir(to)
                .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.file_name().map(|f| f.to_string_lossy().starts_with("nzbff-20")).unwrap_or(false)).collect())
                .unwrap_or_default();
            old.sort();
            while old.len() > KEEP.load(std::sync::atomic::Ordering::Relaxed).max(1) {
                let _ = fs::remove_file(old.remove(0));
            }
        }
    }
    // heure du dernier passage, même si rien n'avait changé (la sauvegarde est à jour)
    let _ = fs::write(to.join(STAMP), crate::prep::now_secs().to_string());
    Ok(n)
}

/// Heure du dernier passage de la sauvegarde automatique.
const STAMP: &str = "derniere-sauvegarde.txt";

/// Copies datées des réglages gardées, et sauvegarde à la fermeture (réglages de l'utilisateur).
static KEEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(10);
static ON_EXIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Reprend les réglages de la sauvegarde automatique (appelé à chaque changement).
pub fn apply_settings(cfg: &crate::prep::Config) {
    use std::sync::atomic::Ordering;
    KEEP.store(cfg.backup_keep.max(1) as usize, Ordering::Relaxed);
    ON_EXIT.store(cfg.auto_backup && cfg.backup_on_close, Ordering::Relaxed);
}

/// Sauvegarde à la fermeture de l'application, si elle est demandée.
pub fn on_exit() {
    if ON_EXIT.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(d) = auto_dir() {
            let _ = auto_save(&crate::app_dir(), &d);
        }
    }
}

/// Prochaine sauvegarde programmée (secondes depuis 1970), si un intervalle est réglé.
pub fn next_due(cfg: &crate::prep::Config, dir: &Path) -> Option<u64> {
    (cfg.auto_backup && cfg.backup_every_min > 0).then(|| auto_last(dir).unwrap_or(0) + cfg.backup_every_min as u64 * 60)
}

/// Sauvegarde à intervalle régulier, fenêtre ouverte ou cachée : vérifie chaque minute.
pub fn spawn_scheduler(shared: std::sync::Arc<std::sync::Mutex<crate::prep::Config>>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let cfg = shared.lock().map(|c| c.clone()).unwrap_or_default();
        apply_settings(&cfg);
        let Some(d) = auto_dir() else { continue };
        if next_due(&cfg, &d).map(|t| crate::prep::now_secs() >= t).unwrap_or(false) {
            let _ = auto_save(&crate::app_dir(), &d);
        }
    });
}

pub fn auto_last(dir: &Path) -> Option<u64> {
    fs::read_to_string(dir.join(STAMP)).ok().and_then(|t| t.trim().parse().ok())
}

/// État de la sauvegarde automatique : (libellé, date de la dernière copie) de chaque fichier
/// sauvegardé, et copies datées des réglages (du plus ancien au plus récent).
pub fn auto_status(dir: &Path) -> (Vec<(&'static str, u64)>, Vec<String>) {
    let mtime = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
    let files = FILES.iter().filter_map(|(n, l)| mtime(&dir.join(n)).map(|t| (*l, t))).collect();
    let mut dated: Vec<String> = fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("nzbff-20") && n.ends_with(".conf")).collect())
        .unwrap_or_default();
    dated.sort();
    (files, dated)
}

/// Au démarrage : fichiers absents du dossier de l'application (nouvelle version installée
/// ailleurs) repris de la sauvegarde automatique. Renvoie les libellés repris.
pub fn auto_restore_missing(app: &Path, from: &Path) -> Vec<String> {
    let mut got = Vec::new();
    // seulement si les réglages manquent : sinon c'est une installation déjà en service
    if app.join("nzbff.conf").exists() || !from.join("nzbff.conf").is_file() {
        return got;
    }
    for (name, label) in FILES {
        let (src, dst) = (from.join(name), app.join(name));
        if src.is_file() && !dst.exists() && fs::copy(&src, &dst).is_ok() {
            got.push(label.to_string());
        }
    }
    got
}

// ------------------------------------------------------------------- SHA-256

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[derive(Clone)]
struct Sha256 {
    h: [u32; 8],
    buf: Vec<u8>,
    len: u64,
}

impl Sha256 {
    fn new() -> Sha256 {
        Sha256 { h: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19], buf: Vec::with_capacity(64), len: 0 }
    }
    fn block(&mut self, b: &[u8]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = self.h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            self.h[i] = self.h[i].wrapping_add(v[i]);
        }
    }
    fn update(&mut self, data: &[u8]) {
        self.len += data.len() as u64;
        let mut d = data;
        if !self.buf.is_empty() {
            let take = (64 - self.buf.len()).min(d.len());
            self.buf.extend_from_slice(&d[..take]);
            d = &d[take..];
            if self.buf.len() == 64 {
                let b = std::mem::take(&mut self.buf);
                self.block(&b);
            }
        }
        while d.len() >= 64 {
            self.block(&d[..64]);
            d = &d[64..];
        }
        self.buf.extend_from_slice(d);
    }
    fn finish(mut self) -> [u8; 32] {
        let bits = self.len.wrapping_mul(8);
        let mut pad = vec![0x80u8];
        while (self.buf.len() + pad.len()) % 64 != 56 {
            pad.push(0);
        }
        pad.extend_from_slice(&bits.to_be_bytes());
        let len = self.len;
        self.update(&pad);
        self.len = len;
        let mut out = [0u8; 32];
        for (i, x) in self.h.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
        }
        out
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finish()
}

/// HMAC-SHA256 avec les états intérieur et extérieur précalculés (PBKDF2 en fait un grand usage).
#[derive(Clone)]
struct Hmac {
    inner: Sha256,
    outer: Sha256,
}

impl Hmac {
    fn new(key: &[u8]) -> Hmac {
        let mut k = [0u8; 64];
        if key.len() > 64 {
            k[..32].copy_from_slice(&sha256(key));
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let mut inner = Sha256::new();
        let mut outer = Sha256::new();
        inner.update(&k.map(|b| b ^ 0x36));
        outer.update(&k.map(|b| b ^ 0x5c));
        Hmac { inner, outer }
    }
    fn mac(&self, data: &[u8]) -> [u8; 32] {
        let mut i = self.inner.clone();
        i.update(data);
        let mut o = self.outer.clone();
        o.update(&i.finish());
        o.finish()
    }
}

#[cfg(test)]
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    Hmac::new(key).mac(data)
}

/// PBKDF2-HMAC-SHA256, clé de 32 octets.
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let h = Hmac::new(password);
    let mut s = salt.to_vec();
    s.extend_from_slice(&1u32.to_be_bytes());
    let mut u = h.mac(&s);
    let mut t = u;
    for _ in 1..iterations {
        u = h.mac(&u);
        for (a, b) in t.iter_mut().zip(u.iter()) {
            *a ^= b;
        }
    }
    t
}

// ------------------------------------------------------- ChaCha20-Poly1305 (RFC 8439)

fn quarter(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

fn chacha20_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut st = [0u32; 16];
    st[0] = 0x61707865;
    st[1] = 0x3320646e;
    st[2] = 0x79622d32;
    st[3] = 0x6b206574;
    for i in 0..8 {
        st[4 + i] = u32::from_le_bytes([key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]]);
    }
    st[12] = counter;
    for i in 0..3 {
        st[13 + i] = u32::from_le_bytes([nonce[4 * i], nonce[4 * i + 1], nonce[4 * i + 2], nonce[4 * i + 3]]);
    }
    let mut w = st;
    for _ in 0..10 {
        quarter(&mut w, 0, 4, 8, 12);
        quarter(&mut w, 1, 5, 9, 13);
        quarter(&mut w, 2, 6, 10, 14);
        quarter(&mut w, 3, 7, 11, 15);
        quarter(&mut w, 0, 5, 10, 15);
        quarter(&mut w, 1, 6, 11, 12);
        quarter(&mut w, 2, 7, 8, 13);
        quarter(&mut w, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[4 * i..4 * i + 4].copy_from_slice(&w[i].wrapping_add(st[i]).to_le_bytes());
    }
    out
}

fn chacha20_xor(key: &[u8; 32], counter: u32, nonce: &[u8; 12], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for (k, chunk) in data.chunks(64).enumerate() {
        let ks = chacha20_block(key, counter.wrapping_add(k as u32), nonce);
        out.extend(chunk.iter().zip(ks.iter()).map(|(a, b)| a ^ b));
    }
    out
}

/// Poly1305 (limbs de 26 bits, d'après poly1305-donna).
fn poly1305(key: &[u8; 32], msg: &[u8]) -> [u8; 16] {
    let le = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let r0 = le(&key[0..]) & 0x3ffffff;
    let r1 = (le(&key[3..]) >> 2) & 0x3ffff03;
    let r2 = (le(&key[6..]) >> 4) & 0x3ffc0ff;
    let r3 = (le(&key[9..]) >> 6) & 0x3f03fff;
    let r4 = (le(&key[12..]) >> 8) & 0x00fffff;
    let (s1, s2, s3, s4) = (r1 * 5, r2 * 5, r3 * 5, r4 * 5);
    let (mut h0, mut h1, mut h2, mut h3, mut h4) = (0u32, 0u32, 0u32, 0u32, 0u32);
    for chunk in msg.chunks(16) {
        let mut b = [0u8; 17];
        b[..chunk.len()].copy_from_slice(chunk);
        b[chunk.len()] = 1;
        let hibit = (b[16] as u32) << 24;
        h0 += le(&b[0..]) & 0x3ffffff;
        h1 += (le(&b[3..]) >> 2) & 0x3ffffff;
        h2 += (le(&b[6..]) >> 4) & 0x3ffffff;
        h3 += (le(&b[9..]) >> 6) & 0x3ffffff;
        h4 += (le(&b[12..]) >> 8) | hibit;
        let m = |a: u32, b: u32| a as u64 * b as u64;
        let d0 = m(h0, r0) + m(h1, s4) + m(h2, s3) + m(h3, s2) + m(h4, s1);
        let mut d1 = m(h0, r1) + m(h1, r0) + m(h2, s4) + m(h3, s3) + m(h4, s2);
        let mut d2 = m(h0, r2) + m(h1, r1) + m(h2, r0) + m(h3, s4) + m(h4, s3);
        let mut d3 = m(h0, r3) + m(h1, r2) + m(h2, r1) + m(h3, r0) + m(h4, s4);
        let mut d4 = m(h0, r4) + m(h1, r3) + m(h2, r2) + m(h3, r1) + m(h4, r0);
        let mut c = (d0 >> 26) as u32;
        h0 = d0 as u32 & 0x3ffffff;
        d1 += c as u64;
        c = (d1 >> 26) as u32;
        h1 = d1 as u32 & 0x3ffffff;
        d2 += c as u64;
        c = (d2 >> 26) as u32;
        h2 = d2 as u32 & 0x3ffffff;
        d3 += c as u64;
        c = (d3 >> 26) as u32;
        h3 = d3 as u32 & 0x3ffffff;
        d4 += c as u64;
        c = (d4 >> 26) as u32;
        h4 = d4 as u32 & 0x3ffffff;
        h0 += c * 5;
        c = h0 >> 26;
        h0 &= 0x3ffffff;
        h1 += c;
    }
    // réduction complète modulo 2^130 - 5
    let mut c = h1 >> 26;
    h1 &= 0x3ffffff;
    h2 += c;
    c = h2 >> 26;
    h2 &= 0x3ffffff;
    h3 += c;
    c = h3 >> 26;
    h3 &= 0x3ffffff;
    h4 += c;
    c = h4 >> 26;
    h4 &= 0x3ffffff;
    h0 += c * 5;
    c = h0 >> 26;
    h0 &= 0x3ffffff;
    h1 += c;
    let mut g0 = h0.wrapping_add(5);
    c = g0 >> 26;
    g0 &= 0x3ffffff;
    let mut g1 = h1.wrapping_add(c);
    c = g1 >> 26;
    g1 &= 0x3ffffff;
    let mut g2 = h2.wrapping_add(c);
    c = g2 >> 26;
    g2 &= 0x3ffffff;
    let mut g3 = h3.wrapping_add(c);
    c = g3 >> 26;
    g3 &= 0x3ffffff;
    let g4 = h4.wrapping_add(c).wrapping_sub(1 << 26);
    let mask = (g4 >> 31).wrapping_sub(1);
    let nm = !mask;
    h0 = (h0 & nm) | (g0 & mask);
    h1 = (h1 & nm) | (g1 & mask);
    h2 = (h2 & nm) | (g2 & mask);
    h3 = (h3 & nm) | (g3 & mask);
    h4 = (h4 & nm) | (g4 & mask);
    let h0 = h0 | (h1 << 26);
    let h1 = (h1 >> 6) | (h2 << 20);
    let h2 = (h2 >> 12) | (h3 << 14);
    let h3 = (h3 >> 18) | (h4 << 8);
    let mut f = h0 as u64 + le(&key[16..]) as u64;
    let o0 = f as u32;
    f = h1 as u64 + le(&key[20..]) as u64 + (f >> 32);
    let o1 = f as u32;
    f = h2 as u64 + le(&key[24..]) as u64 + (f >> 32);
    let o2 = f as u32;
    f = h3 as u64 + le(&key[28..]) as u64 + (f >> 32);
    let o3 = f as u32;
    let mut out = [0u8; 16];
    for (i, x) in [o0, o1, o2, o3].iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_le_bytes());
    }
    out
}

fn aead_tag(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ct: &[u8]) -> [u8; 16] {
    let block = chacha20_block(key, 0, nonce);
    let mut otk = [0u8; 32];
    otk.copy_from_slice(&block[..32]);
    let mut m = aad.to_vec();
    m.resize(m.len().div_ceil(16) * 16, 0);
    m.extend_from_slice(ct);
    m.resize(m.len().div_ceil(16) * 16, 0);
    m.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    m.extend_from_slice(&(ct.len() as u64).to_le_bytes());
    poly1305(&otk, &m)
}

/// Chiffre : texte chiffré suivi de l'étiquette de 16 octets.
fn aead_seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let mut ct = chacha20_xor(key, 1, nonce, plain);
    let tag = aead_tag(key, nonce, aad, &ct);
    ct.extend_from_slice(&tag);
    ct
}

/// Déchiffre ; None si l'étiquette ne correspond pas (mauvaise clé ou données modifiées).
fn aead_open(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < 16 {
        return None;
    }
    let (ct, tag) = sealed.split_at(sealed.len() - 16);
    let want = aead_tag(key, nonce, aad, ct);
    // comparaison en temps constant
    if want.iter().zip(tag.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) != 0 {
        return None;
    }
    Some(chacha20_xor(key, 1, nonce, ct))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
    fn unhex(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn vecteurs_sha256_hmac_pbkdf2() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
        assert_eq!(hex(&sha256(&vec![b'a'; 1_000_000])), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
        // RFC 4231, cas 2 et 6 (clé plus longue qu'un bloc)
        assert_eq!(hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        assert_eq!(hex(&hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
        assert_eq!(hex(&pbkdf2_sha256(b"password", b"salt", 1)), "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b");
        assert_eq!(hex(&pbkdf2_sha256(b"password", b"salt", 4096)), "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a");
    }

    #[test]
    fn vecteurs_chacha20_poly1305_rfc8439() {
        // 2.3.2 : bloc ChaCha20
        let key: [u8; 32] = core::array::from_fn(|i| i as u8);
        let nonce: [u8; 12] = unhex("000000090000004a00000000").try_into().unwrap();
        assert_eq!(hex(&chacha20_block(&key, 1, &nonce)[..16]), "10f1e7e4d13b5915500fdd1fa32071c4");
        // 2.5.2 : Poly1305
        let pk: [u8; 32] = unhex("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b").try_into().unwrap();
        assert_eq!(hex(&poly1305(&pk, b"Cryptographic Forum Research Group")), "a8061dc1305136c6c22b8baf0c0127a9");
        // 2.8.2 : AEAD
        let key: [u8; 32] = core::array::from_fn(|i| 0x80 + i as u8);
        let nonce: [u8; 12] = unhex("070000004041424344454647").try_into().unwrap();
        let aad = unhex("50515253c0c1c2c3c4c5c6c7");
        let pt = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let sealed = aead_seal(&key, &nonce, &aad, pt);
        assert_eq!(hex(&sealed[..16]), "d31a8d34648e60db7b86afbc53ef7ec2");
        assert_eq!(hex(&sealed[sealed.len() - 16..]), "1ae10b594f09e26a7e902ecbd0600691");
        assert_eq!(aead_open(&key, &nonce, &aad, &sealed).as_deref(), Some(&pt[..]));
        let mut bad = sealed.clone();
        bad[3] ^= 1;
        assert_eq!(aead_open(&key, &nonce, &aad, &bad), None);
    }

    #[test]
    fn sauvegarde_aller_retour() {
        let e = vec![Entry { name: "nzbff.conf".into(), data: b"inbox=D:\\inbox\nindexer_api=A\tnewznab\thttps://a\tCLE\tfin\n".to_vec() }, Entry { name: "nzbff-history.tsv".into(), data: vec![0, 1, 2, 255] }];
        // en clair
        let plain = pack(&e, None);
        assert!(!is_encrypted(&plain));
        assert_eq!(unpack(&plain, None).unwrap(), e);
        // chiffrée (peu d'itérations pour le test)
        let enc = pack_with(&e, Some("mot de passe é"), 1000);
        assert!(is_encrypted(&enc));
        assert!(!enc.windows(3).any(|w| w == b"CLE"), "la clé API ne doit pas apparaître en clair");
        assert_eq!(unpack(&enc, None), Err(BackupError::NeedPassword));
        assert_eq!(unpack(&enc, Some("faux")), Err(BackupError::WrongPassword));
        assert_eq!(unpack(&enc, Some("mot de passe é")).unwrap(), e);
        // en-tête modifié (itérations) : refusé
        let mut t = enc.clone();
        t[MAGIC.len() + 2 + 16] ^= 1;
        assert!(unpack(&t, Some("mot de passe é")).is_err());
        assert!(matches!(unpack(b"pas une sauvegarde", None), Err(BackupError::Invalid(_))));
    }

    #[test]
    fn restauration_et_sauvegarde_auto() {
        let d = std::env::temp_dir().join(format!("nzbff-bak-{}", std::process::id()));
        let (app, auto, neuf) = (d.join("app"), d.join("auto"), d.join("neuf"));
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&neuf).unwrap();
        fs::write(app.join("nzbff.conf"), "inbox=X\n").unwrap();
        fs::write(app.join("nzbff-history.tsv"), "h\n").unwrap();
        assert_eq!(auto_save(&app, &auto).unwrap(), 2);
        assert_eq!(auto_save(&app, &auto).unwrap(), 0); // rien de changé
        assert!(auto_last(&auto).unwrap() + 5 >= crate::prep::now_secs()); // mais l'heure est à jour
        // nouvelle version installée ailleurs : tout est repris
        assert_eq!(auto_restore_missing(&neuf, &auto), vec!["Configuration complète".to_string(), "Historique".to_string()]);
        assert_eq!(fs::read_to_string(neuf.join("nzbff.conf")).unwrap(), "inbox=X\n");
        // installation déjà réglée : on n'y touche pas
        fs::write(neuf.join("nzbff.conf"), "inbox=Y\n").unwrap();
        assert!(auto_restore_missing(&neuf, &auto).is_empty());
        // restauration : copie de l'ancien fichier, noms inconnus ignorés
        let e = vec![Entry { name: "nzbff.conf".into(), data: b"inbox=Z\n".to_vec() }, Entry { name: "../evil.txt".into(), data: b"x".to_vec() }];
        assert_eq!(restore(&neuf, &e).unwrap(), vec!["Configuration complète".to_string()]);
        assert_eq!(fs::read_to_string(neuf.join("nzbff.conf")).unwrap(), "inbox=Z\n");
        assert_eq!(fs::read_to_string(neuf.join("nzbff.conf.avant-restauration")).unwrap(), "inbox=Y\n");
        assert!(!d.join("evil.txt").exists());
        let _ = fs::remove_dir_all(&d);
    }

    /// Fichier chiffré écrit pour être relu par une implémentation de référence (Python).
    #[test]
    fn fichier_pour_verification_externe() {
        if let Ok(out) = std::env::var("NZBFF_BAK_OUT") {
            let e = vec![Entry { name: "nzbff.conf".into(), data: b"theme=papier\n".to_vec() }];
            fs::write(out, pack_with(&e, Some("secret"), 2000)).unwrap();
        }
    }

    #[test]
    fn moments_et_copies_gardees() {
        let d = std::env::temp_dir().join(format!("nzbff-bk-moments-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let (app, auto) = (d.join("app"), d.join("auto"));
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&auto).unwrap();
        fs::write(app.join("nzbff.conf"), "inbox=x\n").unwrap();
        for j in 1..=6 {
            fs::write(auto.join(format!("nzbff-2020-01-0{j}.conf")), "vieux").unwrap();
        }
        let cfg = crate::prep::Config { auto_backup: true, backup_keep: 3, backup_every_min: 180, ..Default::default() };
        apply_settings(&cfg);
        auto_save(&app, &auto).unwrap();
        let dated = auto_status(&auto).1;
        assert_eq!(dated.len(), 3, "{dated:?}");
        assert!(!dated.contains(&"nzbff-2020-01-01.conf".to_string()), "les plus anciennes partent");
        // prochaine sauvegarde : 3 h après la dernière
        let last = auto_last(&auto).unwrap();
        assert_eq!(next_due(&cfg, &auto), Some(last + 3 * 3600));
        assert_eq!(next_due(&crate::prep::Config { backup_every_min: 0, ..cfg.clone() }, &auto), None);
        apply_settings(&crate::prep::Config::default());
        let _ = fs::remove_dir_all(&d);
    }
}
