//! Journaux sur disque : dossier `logs` à côté de l'application (journal de chaque post,
//! dépôts sur les indexers, journal de l'application jour par jour) et export en un .zip à
//! joindre à un message d'aide. Rien de ce qui est écrit ou exporté ne contient de mot de
//! passe ni de clé (`prep::redact` à l'écriture et de nouveau à l'export).

use crate::prep;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Dossier des journaux.
pub fn dir() -> PathBuf {
    crate::app_dir().join("logs")
}

/// Ajoute une ligne au journal de l'application du jour (`logs\app-AAAA-MM-JJ.log`).
pub fn append_app(level: &str, text: &str) {
    let l = crate::history::local_time(prep::now_secs());
    let d = dir();
    if fs::create_dir_all(&d).is_err() {
        return;
    }
    let p = d.join(format!("app-{:04}-{:02}-{:02}.log", l.year, l.month, l.day));
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "[{:02}:{:02}:{:02}] {level:<5} {}", l.hour, l.minute, l.second, prep::redact(text).replace('\n', " ⏎ "));
    }
}

/// Journaux du dossier (fichiers .log et .txt), du plus récent au plus ancien, modifiés
/// depuis moins de `days` jours (0 : tous).
pub fn files(days: u32) -> Vec<(PathBuf, u64, u64)> {
    let now = prep::now_secs();
    let mut v: Vec<(PathBuf, u64, u64)> = fs::read_dir(dir())
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().map(|x| x.eq_ignore_ascii_case("log") || x.eq_ignore_ascii_case("txt")).unwrap_or(false))
                .filter_map(|p| {
                    let m = fs::metadata(&p).ok()?;
                    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
                    Some((p, t, m.len()))
                })
                .filter(|(_, t, _)| days == 0 || now.saturating_sub(*t) <= days as u64 * 86_400)
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v
}

/// Exporte les journaux dans un .zip : ceux des `days` derniers jours, le journal de la
/// session (`session`) et une fiche d'informations. Renvoie (fichiers, octets).
pub fn export_zip(dest: &Path, session: &str, days: u32) -> Result<(usize, u64), String> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let l = crate::history::local_time(prep::now_secs());
    let info = format!(
        "{} {}\r\nExport du {:02}/{:02}/{} {:02}:{:02}\r\nSystème : {} {}\r\nDossier de l'application : {}\r\nJournaux : {} derniers jours\r\n\r\nMots de passe et clés masqués (••••••) dans tous les fichiers de cet export.\r\n",
        crate::APP_NAME,
        crate::VERSION,
        l.day,
        l.month,
        l.year,
        l.hour,
        l.minute,
        std::env::consts::OS,
        std::env::consts::ARCH,
        crate::app_dir().display(),
        if days == 0 { "tous les".to_string() } else { days.to_string() }
    );
    entries.push(("LISEZ-MOI.txt".into(), info.into_bytes()));
    if !session.trim().is_empty() {
        entries.push(("journal-de-la-session.txt".into(), prep::redact(session).into_bytes()));
    }
    for (p, _, _) in files(days) {
        let Ok(bytes) = fs::read(&p) else { continue };
        // texte : relu et masqué (anciens journaux d'avant le masquage compris)
        let text = crate::ngpost::decode_text(&bytes);
        let clean: String = text.lines().map(prep::redact).collect::<Vec<_>>().join("\r\n");
        entries.push((format!("logs/{}", prep::file_name(&p)), clean.into_bytes()));
    }
    let n = entries.len();
    let data = zip_store(&entries);
    if let Some(d) = dest.parent() {
        let _ = fs::create_dir_all(d);
    }
    fs::write(dest, &data).map_err(|e| format!("{} : {e}", dest.display()))?;
    Ok((n, data.len() as u64))
}

/// Archive ZIP sans compression (méthode « stored »), noms en UTF-8 : s'ouvre dans
/// l'Explorateur de Windows comme dans tout outil d'archive.
pub fn zip_store(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let (time, date) = dos_time();
    for (name, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let nb = name.as_bytes();
        // en-tête local
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version nécessaire
        out.extend_from_slice(&0x0800u16.to_le_bytes()); // noms en UTF-8
        out.extend_from_slice(&0u16.to_le_bytes()); // stored
        out.extend_from_slice(&time.to_le_bytes());
        out.extend_from_slice(&date.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(nb.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(nb);
        out.extend_from_slice(data);
        // entrée du répertoire central
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // créé par
        central.extend_from_slice(&20u16.to_le_bytes()); // version nécessaire
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&time.to_le_bytes());
        central.extend_from_slice(&date.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(nb.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 8]); // extra, commentaire, disque, attributs internes
        central.extend_from_slice(&0u32.to_le_bytes()); // attributs externes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(nb);
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]); // disques
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn dos_time() -> (u16, u16) {
    let l = crate::history::local_time(prep::now_secs());
    let time = ((l.hour as u16) << 11) | ((l.minute as u16) << 5) | ((l.second as u16) / 2);
    let date = (((l.year.max(1980) - 1980) as u16) << 9) | ((l.month as u16) << 5) | (l.day as u16);
    (time, date)
}

/// CRC-32 (IEEE 802.3), celui des archives ZIP.
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// Supprime les journaux de plus de `days` jours (0 : on garde tout).
pub fn prune(days: u32) -> usize {
    if days == 0 {
        return 0;
    }
    let keep: std::collections::HashSet<PathBuf> = files(days).into_iter().map(|f| f.0).collect();
    files(0).into_iter().filter(|f| !keep.contains(&f.0)).filter(|f| fs::remove_file(&f.0).is_ok()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_et_zip() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let z = zip_store(&[("a.txt".into(), b"bonjour".to_vec()), ("logs/b.log".into(), Vec::new())]);
        assert_eq!(&z[..4], b"PK\x03\x04");
        // fin du répertoire central : 2 entrées
        let end = z.len() - 22;
        assert_eq!(&z[end..end + 4], b"PK\x05\x06");
        assert_eq!(u16::from_le_bytes([z[end + 10], z[end + 11]]), 2);
        // le contenu est là, non compressé
        assert!(z.windows(7).any(|w| w == b"bonjour"));
        if let Ok(p) = std::env::var("NZBFF_ZIP_OUT") {
            fs::write(p, &z).unwrap();
        }
    }
}
