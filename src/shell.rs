//! Intégration à Windows : ouverture des .nzb par l'application, menu du clic droit sur les
//! fichiers, dossiers et .nzb, et demandes transmises à l'instance déjà ouverte.
//!
//! Tout est écrit dans HKEY_CURRENT_USER (aucun droit administrateur) par un fichier .reg
//! importé avec `reg import`. Chaque lancement depuis l'Explorateur dépose sa demande dans un
//! dossier que l'instance ouverte relit : plusieurs fichiers sélectionnés d'un coup arrivent
//! tous, même si Windows lance un processus par fichier.

use std::fs;
use std::path::{Path, PathBuf};

/// Type de fichier « .nzb » de l'application.
pub const PROG_ID: &str = "NZBFFTool.nzb";
/// Nom des menus du clic droit dans le registre.
const MENU_KEY: &str = "NZBFFTool";
const CLASSES: &str = r"HKEY_CURRENT_USER\Software\Classes";
const USER_CHOICE: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.nzb\UserChoice";

/// Ce que l'Explorateur demande à l'application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Fichier ou dossier traité tout seul avec les réglages par défaut.
    Auto,
    /// Ouvert dans l'application pour choisir les réglages.
    Open,
    /// .nzb ouvert d'un double clic : selon le réglage « Ouverture des .nzb ».
    Nzb,
    /// .nzb téléchargé puis reposté sur les autres indexers.
    Repost,
    /// .nzb téléchargé seulement.
    DlOnly,
    /// .nzb déposé sur les indexers sans le télécharger.
    Send,
}

impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Open => "ouvrir",
            Mode::Nzb => "nzb",
            Mode::Repost => "reposter",
            Mode::DlOnly => "telecharger",
            Mode::Send => "deposer",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        [Mode::Auto, Mode::Open, Mode::Nzb, Mode::Repost, Mode::DlOnly, Mode::Send].into_iter().find(|m| m.key() == s.trim())
    }
}

/// Une demande : un mode et un chemin.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub mode: Mode,
    pub path: PathBuf,
}

pub fn is_nzb(p: &Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("nzb")).unwrap_or(false)
}

/// Mode réellement appliqué à un .nzb ouvert d'un double clic.
pub fn nzb_mode(cfg: &crate::prep::Config) -> Mode {
    match cfg.nzb_open.as_str() {
        "auto" => Mode::Repost,
        "dl" => Mode::DlOnly,
        "send" => Mode::Send,
        _ => Mode::Open,
    }
}

/// Ce que fait un double clic sur un .nzb (réglage « nzb_open »).
pub fn nzb_open_label(k: &str) -> &'static str {
    match k {
        "auto" => "téléchargé et reposté tout seul",
        "dl" => "téléchargé seulement",
        "send" => "déposé sur tes indexers sans téléchargement",
        _ => "à régler sur la page Téléchargements",
    }
}

/// Idem, d'après les réglages.
pub fn nzb_mode_label(cfg: &crate::prep::Config) -> &'static str {
    nzb_open_label(&cfg.nzb_open)
}

/// La demande a-t-elle besoin de la fenêtre (choix à faire) ? Sinon elle se fait en fond.
pub fn needs_window(cfg: &crate::prep::Config, r: &Request) -> bool {
    let m = if r.mode == Mode::Nzb || (r.mode == Mode::Auto && is_nzb(&r.path)) { nzb_mode(cfg) } else { r.mode };
    match m {
        Mode::Auto | Mode::DlOnly => false,
        Mode::Repost => crate::dl::default_origin(cfg).is_none(),
        _ => true,
    }
}

/// « --envoyer <mode> <chemin>… » (ligne de commande écrite par l'Explorateur).
pub fn parse_args(args: &[String]) -> Option<Vec<Request>> {
    let k = args.iter().position(|a| a == "--envoyer")?;
    let mode = args.get(k + 1).and_then(|m| Mode::parse(m))?;
    let v: Vec<Request> = args[k + 2..].iter().filter(|a| !a.trim().is_empty()).map(|a| Request { mode, path: PathBuf::from(a.trim().trim_matches('"')) }).collect();
    Some(v)
}

/// Dossier des demandes en attente, à côté de l'application.
pub fn queue_dir() -> PathBuf {
    crate::app_dir().join(".nzbff-demandes")
}

/// Dépose des demandes pour l'instance ouverte (écriture puis renommage : jamais lue à moitié).
pub fn enqueue(dir: &Path, reqs: &[Request]) -> Result<(), String> {
    if reqs.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|e| format!("{} : {e}", dir.display()))?;
    let body: String = reqs.iter().map(|r| format!("{}\t{}\n", r.mode.key(), r.path.display())).collect();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let name = format!("{stamp:020}-{}", std::process::id());
    let tmp = dir.join(format!("{name}.part"));
    fs::write(&tmp, body).map_err(|e| e.to_string())?;
    fs::rename(&tmp, dir.join(format!("{name}.txt"))).map_err(|e| e.to_string())
}

/// Reprend (et efface) les demandes en attente, dans l'ordre d'arrivée.
pub fn take_all(dir: &Path) -> Vec<Request> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "txt").unwrap_or(false)).collect()).unwrap_or_default();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let text = fs::read_to_string(&f).unwrap_or_default();
        let _ = fs::remove_file(&f);
        for l in text.lines() {
            if let Some((m, p)) = l.split_once('\t') {
                if let (Some(mode), false) = (Mode::parse(m), p.trim().is_empty()) {
                    let r = Request { mode, path: PathBuf::from(p.trim()) };
                    if !out.contains(&r) {
                        out.push(r);
                    }
                }
            }
        }
    }
    out
}

// ------------------------------------------------------------------ registre

/// Chaîne d'un fichier .reg : antislashs et guillemets échappés.
fn reg_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Contenu du fichier .reg : retire puis réinstalle ce qui est demandé. `menu` : menus du clic
/// droit ; `assoc` : l'application ouvre les .nzb ; `drop_default` : le type .nzb nous
/// appartient et doit être rendu (désactivation).
pub fn reg_text(exe: &Path, menu: bool, assoc: bool, drop_default: bool) -> String {
    let exe = exe.display().to_string();
    let cmd = |mode: Mode, arg: &str| reg_str(&format!("\"{exe}\" --envoyer {} \"{arg}\"", mode.key()));
    let icon = reg_str(&format!("\"{exe}\",0"));
    let mut t = String::from("Windows Registry Editor Version 5.00\r\n\r\n");
    // tout retirer d'abord : réinstallation propre (chemin de l'application changé…)
    for k in [format!("*\\shell\\{MENU_KEY}"), format!("Directory\\shell\\{MENU_KEY}"), format!("SystemFileAssociations\\.nzb\\shell\\{MENU_KEY}"), PROG_ID.to_string()] {
        t.push_str(&format!("[-{CLASSES}\\{k}]\r\n\r\n"));
    }
    if menu || assoc {
        reg_key(&mut t, PROG_ID, &[("", reg_str("Fichier NZB")), ("FriendlyTypeName", reg_str("Fichier NZB (NZB FF Tool)"))]);
        reg_key(&mut t, &format!("{PROG_ID}\\DefaultIcon"), &[("", icon.clone())]);
        reg_key(&mut t, &format!("{PROG_ID}\\shell"), &[("", reg_str("open"))]);
        reg_key(&mut t, &format!("{PROG_ID}\\shell\\open"), &[("", reg_str("Ouvrir avec NZB FF Tool"))]);
        reg_key(&mut t, &format!("{PROG_ID}\\shell\\open\\command"), &[("", cmd(Mode::Nzb, "%1"))]);
        // proposé dans « Ouvrir avec »
        reg_key(&mut t, ".nzb\\OpenWithProgids", &[(PROG_ID, reg_str(""))]);
    } else {
        reg_key(&mut t, ".nzb\\OpenWithProgids", &[(PROG_ID, "-".into())]);
    }
    if assoc {
        reg_key(&mut t, ".nzb", &[("", reg_str(PROG_ID))]);
    } else if drop_default {
        reg_key(&mut t, ".nzb", &[("", "-".into())]);
    }
    if menu {
        let sub = |t: &mut String, base: &str, items: &[(&str, &str, Mode)], applies: Option<&str>| {
            let mut v = vec![("MUIVerb", reg_str("NZB FF Tool")), ("Icon", icon.clone()), ("SubCommands", reg_str(""))];
            if let Some(a) = applies {
                v.push(("AppliesTo", reg_str(a)));
            }
            reg_key(t, &format!("{base}\\{MENU_KEY}"), &v);
            for (id, label, mode) in items {
                reg_key(t, &format!("{base}\\{MENU_KEY}\\shell\\{id}"), &[("MUIVerb", reg_str(label)), ("MultiSelectModel", reg_str("Player"))]);
                reg_key(t, &format!("{base}\\{MENU_KEY}\\shell\\{id}\\command"), &[("", cmd(*mode, "%1"))]);
            }
        };
        let files = [("1auto", "Traiter automatiquement (réglages par défaut)", Mode::Auto), ("2ouvrir", "Ouvrir dans NZB FF Tool (choisir les réglages)", Mode::Open)];
        sub(&mut t, "*\\shell", &files, Some("NOT System.FileExtension:=.nzb"));
        sub(&mut t, "Directory\\shell", &files, None);
        let nzb = [
            ("1reposter", "Télécharger et reposter sur mes autres indexers", Mode::Repost),
            ("2telecharger", "Télécharger seulement", Mode::DlOnly),
            ("3deposer", "Déposer sur mes indexers (sans télécharger)", Mode::Send),
            ("4ouvrir", "Ouvrir dans NZB FF Tool (choisir les réglages)", Mode::Open),
        ];
        sub(&mut t, "SystemFileAssociations\\.nzb\\shell", &nzb, None);
    }
    t
}

/// Une clé du fichier .reg et ses valeurs (nom vide : valeur par défaut « @ »).
fn reg_key(t: &mut String, k: &str, vals: &[(&str, String)]) {
    t.push_str(&format!("[{CLASSES}\\{k}]\r\n"));
    for (n, v) in vals {
        if n.is_empty() {
            t.push_str(&format!("@={v}\r\n"));
        } else {
            t.push_str(&format!("\"{n}\"={v}\r\n"));
        }
    }
    t.push_str("\r\n");
}

/// Exécute `reg` sans fenêtre ; renvoie la sortie standard si la commande réussit.
fn reg(args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new("reg");
    cmd.args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let o = cmd.output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Valeur d'une sortie de « reg query » (dernière colonne de la ligne qui porte `name`).
fn query_value(out: &str, name: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix(name)?;
        let (_, v) = rest.trim_start().split_once("REG_SZ")?;
        Some(v.trim().to_string())
    })
}

/// Programme choisi dans Windows pour les .nzb (« Toujours utiliser cette application »), s'il
/// y en a un : il passe avant tout le reste.
pub fn nzb_user_choice() -> Option<String> {
    query_value(&reg(&["query", USER_CHOICE, "/v", "ProgId"])?, "ProgId")
}

/// Type actuellement associé aux .nzb pour cet utilisateur.
fn nzb_default() -> Option<String> {
    // « (Par défaut) », « (Default) »… selon la langue de Windows
    query_value(&reg(&["query", r"HKCU\Software\Classes\.nzb", "/ve"])?, "(")
}

/// Les .nzb s'ouvrent-ils vraiment avec l'application ? Sinon : le programme qui les ouvre.
pub fn nzb_opened_by_us() -> Result<(), String> {
    // ce que Windows lance vraiment (choix « Toujours », type .nzb, ou rien)
    if let Some(h) = nzb_handler() {
        let me = std::env::current_exe().map(|p| p.display().to_string().to_lowercase()).unwrap_or_default();
        return if h.display().to_string().to_lowercase() == me { Ok(()) } else { Err(crate::prep::file_name(&h)) };
    }
    match nzb_user_choice() {
        Some(p) if p.eq_ignore_ascii_case(PROG_ID) => Ok(()),
        Some(p) => Err(p),
        None if cfg!(windows) => Err("aucun programme".into()),
        None => Ok(()),
    }
}

/// Programme que Windows lance vraiment pour ouvrir un .nzb (double clic).
#[cfg(windows)]
pub fn nzb_handler() -> Option<PathBuf> {
    #[link(name = "shlwapi")]
    extern "system" {
        fn AssocQueryStringW(flags: u32, what: u32, assoc: *const u16, extra: *const u16, out: *mut u16, len: *mut u32) -> i32;
    }
    let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (assoc, verb) = (w(".nzb"), w("open"));
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    // ASSOCSTR_EXECUTABLE = 2
    let hr = unsafe { AssocQueryStringW(0, 2, assoc.as_ptr(), verb.as_ptr(), buf.as_mut_ptr(), &mut len) };
    if hr != 0 {
        return None;
    }
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    let p = String::from_utf16_lossy(&buf[..end]);
    (!p.trim().is_empty()).then(|| PathBuf::from(p.trim()))
}

#[cfg(not(windows))]
pub fn nzb_handler() -> Option<PathBuf> {
    None
}

/// Commande enregistrée pour ouvrir les .nzb (pour voir si l'application a été déplacée).
fn registered_command() -> Option<String> {
    let out = reg(&["query", &format!(r"HKCU\Software\Classes\{PROG_ID}\shell\open\command"), "/ve"])?;
    out.lines().find_map(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_string()))
}

/// Installe ou retire l'intégration (menus, ouverture des .nzb).
pub fn apply(menu: bool, assoc: bool) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("seulement sous Windows".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let ours = nzb_default().map(|d| d.eq_ignore_ascii_case(PROG_ID)).unwrap_or(false);
    let text = reg_text(&exe, menu, assoc, ours && !assoc);
    // .reg en UTF-16 (accents des menus)
    let mut bytes = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        bytes.extend(u.to_le_bytes());
    }
    let file = std::env::temp_dir().join(format!("nzbff-integration-{}.reg", std::process::id()));
    fs::write(&file, bytes).map_err(|e| e.to_string())?;
    let ok = reg(&["import", &file.display().to_string()]).is_some();
    let _ = fs::remove_file(&file);
    if !ok {
        return Err("le registre a refusé la modification".into());
    }
    notify_assoc_changed();
    Ok(())
}

/// Réinstalle si l'application a changé de dossier (menus et .nzb pointent sur l'ancien).
pub fn refresh_if_moved(menu: bool, assoc: bool) {
    if !cfg!(windows) || !(menu || assoc) {
        return;
    }
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    if !registered_command().map(|c| c.to_lowercase().contains(&exe.to_lowercase())).unwrap_or(false) {
        let _ = apply(menu, assoc);
    }
}

/// Fenêtre de Windows « Ouvrir avec » sur un .nzb, pour cocher « Toujours utiliser » : Windows
/// protège ce choix, aucune application ne peut le faire à la place de l'utilisateur.
pub fn choose_in_windows() -> Result<(), String> {
    if !cfg!(windows) {
        return Err("seulement sous Windows".into());
    }
    let sample = crate::app_dir().join("exemple.nzb");
    if !sample.exists() {
        fs::write(&sample, "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<nzb xmlns=\"http://www.newzbin.com/DTD/2003/nzb\"></nzb>\n").map_err(|e| e.to_string())?;
    }
    std::process::Command::new("rundll32.exe").arg("shell32.dll,OpenAs_RunDLL").arg(&sample).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(windows)]
fn notify_assoc_changed() {
    #[link(name = "shell32")]
    extern "system" {
        fn SHChangeNotify(event: i32, flags: u32, a: *const std::ffi::c_void, b: *const std::ffi::c_void);
    }
    // SHCNE_ASSOCCHANGED, SHCNF_IDLIST : l'Explorateur relit icônes et menus
    unsafe { SHChangeNotify(0x0800_0000, 0, std::ptr::null(), std::ptr::null()) }
}

#[cfg(not(windows))]
fn notify_assoc_changed() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_de_l_explorateur() {
        let a: Vec<String> = ["--envoyer", "reposter", r"C:\NZB\Film.nzb", r"D:\Autre.nzb"].iter().map(|s| s.to_string()).collect();
        let r = parse_args(&a).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], Request { mode: Mode::Repost, path: PathBuf::from(r"C:\NZB\Film.nzb") });
        assert!(parse_args(&["--tray".to_string()]).is_none());
        assert!(parse_args(&["--envoyer".to_string(), "inconnu".to_string(), "x".to_string()]).is_none());
        for m in [Mode::Auto, Mode::Open, Mode::Nzb, Mode::Repost, Mode::DlOnly, Mode::Send] {
            assert_eq!(Mode::parse(m.key()), Some(m));
        }
    }

    #[test]
    fn file_des_demandes() {
        let d = std::env::temp_dir().join(format!("nzbff-demandes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        assert!(take_all(&d).is_empty());
        enqueue(&d, &[Request { mode: Mode::Auto, path: "/a/film.mkv".into() }]).unwrap();
        enqueue(&d, &[Request { mode: Mode::DlOnly, path: "/b/x.nzb".into() }, Request { mode: Mode::Auto, path: "/a/film.mkv".into() }]).unwrap();
        // fichier en cours d'écriture : ignoré
        fs::write(d.join("zzz.part"), "auto\t/c").unwrap();
        let r = take_all(&d);
        assert_eq!(r, vec![Request { mode: Mode::Auto, path: "/a/film.mkv".into() }, Request { mode: Mode::DlOnly, path: "/b/x.nzb".into() }]);
        assert!(take_all(&d).is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn fichier_reg() {
        let exe = Path::new(r"C:\Outils\NZB FF Tool Pro.exe");
        let t = reg_text(exe, true, true, false);
        assert!(t.starts_with("Windows Registry Editor Version 5.00\r\n"));
        // anciennes clés retirées avant
        assert!(t.contains(r"[-HKEY_CURRENT_USER\Software\Classes\*\shell\NZBFFTool]"));
        assert!(t.contains(r"[-HKEY_CURRENT_USER\Software\Classes\NZBFFTool.nzb]"));
        assert!(t.find(r"[-HKEY_CURRENT_USER\Software\Classes\NZBFFTool.nzb]").unwrap() < t.find(r"[HKEY_CURRENT_USER\Software\Classes\NZBFFTool.nzb]").unwrap());
        assert!(t.contains(r#"@="\"C:\\Outils\\NZB FF Tool Pro.exe\" --envoyer nzb \"%1\"""#), "{t}");
        assert!(t.contains("[HKEY_CURRENT_USER\\Software\\Classes\\.nzb]\r\n@=\"NZBFFTool.nzb\""));
        assert!(t.contains(r#""AppliesTo"="NOT System.FileExtension:=.nzb""#));
        assert!(t.contains(r#"--envoyer telecharger \"%1\""#) && t.contains(r#"--envoyer auto \"%1\""#));
        assert!(t.contains(r"[HKEY_CURRENT_USER\Software\Classes\Directory\shell\NZBFFTool\shell\2ouvrir\command]"));
        // sans association : le type .nzb n'est pas pris, seulement proposé dans « Ouvrir avec »
        let t = reg_text(exe, true, false, false);
        assert!(!t.contains("[HKEY_CURRENT_USER\\Software\\Classes\\.nzb]\r\n"));
        assert!(t.contains("\"NZBFFTool.nzb\"=\"\""));
        // tout retiré, type .nzb rendu
        let t = reg_text(exe, false, false, true);
        assert!(!t.contains("--envoyer"));
        assert!(t.contains("[HKEY_CURRENT_USER\\Software\\Classes\\.nzb]\r\n@=-"));
        assert!(t.contains("\"NZBFFTool.nzb\"=-"));
    }

    #[test]
    fn lecture_de_reg_query() {
        let out = "\r\nHKEY_CURRENT_USER\\...\\UserChoice\r\n    ProgId    REG_SZ    SABnzbd.nzb\r\n    Hash    REG_SZ    abc=\r\n";
        assert_eq!(query_value(out, "ProgId").as_deref(), Some("SABnzbd.nzb"));
        assert_eq!(query_value(out, "Absent"), None);
        let d = "HKEY_CURRENT_USER\\Software\\Classes\\.nzb\r\n    (Par défaut)    REG_SZ    NZBFFTool.nzb\r\n";
        assert_eq!(query_value(d, "(").as_deref(), Some("NZBFFTool.nzb"));
    }
}
