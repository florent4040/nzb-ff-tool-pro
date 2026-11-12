//! Intégration NgPost : savoir s'il tourne, le lancer, lire sa configuration,
//! et rapatrier les NZB à côté des rapports MediaInfo (dossier « prêt pour l'indexer »).

use crate::prep::{self, Config};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};


fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Chemin de ngPost.exe tel que saisi (espaces et guillemets autour retirés).
pub fn exe_path(cfg: &Config) -> PathBuf {
    PathBuf::from(cfg.ngpost_exe.trim().trim_matches('"').trim())
}

/// Nom du processus à partir du chemin de l'exécutable (ex. « ngPost.exe »).
pub fn process_name(cfg: &Config) -> Option<String> {
    let exe = cfg.ngpost_exe.trim().trim_matches('"').trim();
    if exe.is_empty() {
        return None;
    }
    Path::new(exe).file_name().map(|s| s.to_string_lossy().into_owned())
}

/// NgPost est-il ouvert ?
pub fn is_running(cfg: &Config) -> bool {
    let Some(name) = process_name(cfg) else { return false };
    #[cfg(windows)]
    {
        let out = hidden(&mut Command::new("tasklist")).args(["/FI", &format!("IMAGENAME eq {name}"), "/NH", "/FO", "CSV"]).output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_lowercase().contains(&format!("\"{}\"", name.to_lowercase())),
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let Ok(rd) = fs::read_dir("/proc") else { return false };
        rd.flatten().any(|e| {
            fs::read_to_string(e.path().join("comm")).map(|c| c.trim().to_lowercase() == stem).unwrap_or(false)
        })
    }
}

/// Découpe les arguments comme une ligne de commande (guillemets respectés).
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut has = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

/// Lance NgPost (détaché : il reste ouvert même si on ferme l'application).
pub fn launch(cfg: &Config) -> Result<(), String> {
    let exe = exe_path(cfg);
    if exe.as_os_str().is_empty() {
        return Err("chemin de NgPost non renseigné (Réglages → NgPost)".into());
    }
    let p = exe.as_path();
    if !p.is_file() {
        return Err(format!("NgPost introuvable : {}", p.display()));
    }
    let mut cmd = Command::new(p);
    cmd.args(split_args(&cfg.ngpost_args));
    // NgPost ne doit pas hériter de nos entrées/sorties (sinon il retient la console ou le terminal)
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Some(d) = p.parent() {
        cmd.current_dir(d);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    }
    cmd.spawn().map(|_| ()).map_err(|e| format!("lancement de NgPost impossible : {e}"))
}

/// Lance NgPost s'il est fermé. Renvoie Ok(true) s'il vient d'être lancé.
pub fn ensure_running(cfg: &Config) -> Result<bool, String> {
    if cfg.ngpost_exe.trim().is_empty() || is_running(cfg) {
        return Ok(false);
    }
    launch(cfg).map(|_| true)
}

/// Configuration de NgPost lue dans ngPost.conf (toutes les clés « clé = valeur »).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NgConf {
    pub path: PathBuf,
    pub values: Vec<(String, String)>,
}

impl NgConf {
    pub fn parse(path: &Path, text: &str) -> NgConf {
        let mut values = Vec::new();
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') || l.starts_with(';') || l.starts_with('[') {
                continue;
            }
            if let Some((k, v)) = l.split_once('=') {
                let v = v.trim().trim_matches('"').trim();
                if !k.trim().is_empty() {
                    values.push((k.trim().to_string(), v.to_string()));
                }
            }
        }
        NgConf { path: path.to_path_buf(), values }
    }

    /// Valeur d'une clé (insensible à la casse), non vide.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.iter().find(|(k, v)| k.eq_ignore_ascii_case(key) && !v.is_empty()).map(|(_, v)| v.as_str())
    }

    /// Dossier où NgPost écrit les NZB (`nzbPath`).
    pub fn nzb_dir(&self) -> Option<String> {
        self.get("nzbPath").map(native_path)
    }

    /// Dossier que NgPost surveille / poste automatiquement.
    /// Priorité : dossiers de surveillance, puis dossier auto, puis `inputDir`.
    pub fn watch_dir(&self) -> Option<(String, String)> {
        let first = |v: &str| v.split([',', ';', '|']).map(str::trim).find(|x| !x.is_empty()).map(native_path);
        let keys = ["monitorFolders", "MONITOR_FOLDERS", "monitorFolder", "autoDir", "AUTO_DIR", "inputDir"];
        for k in keys {
            if let Some(v) = self.get(k).and_then(first) {
                let key = self.values.iter().find(|(kk, _)| kk.eq_ignore_ascii_case(k)).map(|(kk, _)| kk.clone()).unwrap_or_else(|| k.to_string());
                return Some((v, key));
            }
        }
        None
    }
}

/// « C:/ngPost/AutoNZB » → « C:\ngPost\AutoNZB » sous Windows.
pub fn native_path(v: &str) -> String {
    let v = v.trim().trim_matches('"').trim();
    if cfg!(windows) {
        v.replace('/', "\\")
    } else {
        v.to_string()
    }
}

/// Emplacements possibles de ngPost.conf : dossier de ngPost.exe, puis dossier utilisateur.
fn conf_candidates(cfg: &Config) -> Vec<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    let exe = exe_path(cfg);
    let dir = if exe.is_dir() { Some(exe.clone()) } else { exe.parent().map(Path::to_path_buf) };
    if let Some(d) = dir.filter(|d| !d.as_os_str().is_empty()) {
        c.push(d.join("ngPost.conf"));
        c.push(d.join("ngpost.conf"));
    }
    for var in ["USERPROFILE", "HOME"] {
        if let Ok(h) = std::env::var(var) {
            c.push(Path::new(&h).join("ngPost.conf"));
            c.push(Path::new(&h).join(".ngPost"));
        }
    }
    c
}

/// Lit ngPost.conf (quel que soit son encodage).
pub fn read_conf(cfg: &Config) -> Option<NgConf> {
    conf_candidates(cfg).into_iter().find_map(|p| {
        let bytes = fs::read(&p).ok()?;
        let text = decode_text(&bytes);
        // mots de passe des serveurs de ngPost.conf : masqués partout
        for s in crate::usenet::parse_conf_servers(&text) {
            prep::add_secret(&s.pass);
        }
        let c = NgConf::parse(&p, &text);
        (!c.values.is_empty()).then_some(c)
    })
}

/// Lit le dossier des NZB dans la configuration de NgPost.
#[allow(dead_code)]
pub fn read_nzb_dir(cfg: &Config) -> Option<String> {
    read_conf(cfg).and_then(|c| c.nzb_dir())
}

/// Cherche `ngPost.exe` dans `dir` et ses sous-dossiers (jusqu'à `depth` niveaux).
fn find_exe_in(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_lowercase();
        if p.is_file() && (name == "ngpost.exe" || (cfg!(not(windows)) && name == "ngpost")) {
            out.push(p);
        } else if depth > 0 && p.is_dir() && !name.starts_with('.') && !name.starts_with('$') {
            find_exe_in(&p, depth - 1, out);
        }
    }
}

/// Version lue dans le chemin (« v4.16_x64 » → [4, 16]) pour préférer la plus récente.
fn version_in(p: &Path) -> Vec<u32> {
    let s = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
    let mut v = Vec::new();
    let mut cur = String::new();
    for c in s.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            if cur.len() < 5 {
                v.push(cur.parse().unwrap_or(0));
            }
            cur.clear();
        }
    }
    v
}

/// Cherche ngPost.exe sous chaque dossier dont le nom contient « ngpost » (sous-dossiers compris,
/// ex. C:\ngPost\v4.16_x64\ngPost.exe). Renvoie la version la plus récente.
pub fn find_exe_under(roots: &[PathBuf]) -> Option<PathBuf> {
    let mut found = Vec::new();
    for r in roots {
        let n = prep::file_name(r).to_lowercase();
        if n.contains("ngpost") {
            find_exe_in(r, 3, &mut found);
            continue;
        }
        let Ok(rd) = fs::read_dir(r) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && e.file_name().to_string_lossy().to_lowercase().contains("ngpost") {
                find_exe_in(&p, 3, &mut found);
            } else if p.is_file() && e.file_name().to_string_lossy().eq_ignore_ascii_case("ngpost.exe") {
                found.push(p);
            }
        }
    }
    found.sort_by(|a, b| version_in(a).cmp(&version_in(b)).then(mtime_secs(a).cmp(&mtime_secs(b))));
    found.pop()
}

/// Chemin de ngPost.exe s'il est en cours d'exécution.
#[cfg(windows)]
fn running_exe() -> Option<PathBuf> {
    let out = hidden(&mut Command::new("powershell"))
        .args(["-NoProfile", "-NonInteractive", "-Command", "(Get-Process -Name ngPost -ErrorAction SilentlyContinue | Select-Object -First 1).Path"])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    p.is_file().then_some(p)
}

/// Cherche ngPost.exe : NgPost ouvert, puis dossiers « ngPost* » à la racine de chaque disque,
/// Program Files, AppData, Bureau, Téléchargements, Documents (sous-dossiers compris).
pub fn detect_exe() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(p) = running_exe() {
        return Some(p);
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for l in b'C'..=b'Z' {
            let r = PathBuf::from(format!("{}:\\", l as char));
            if r.is_dir() {
                roots.push(r);
            }
        }
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "LOCALAPPDATA", "APPDATA", "USERPROFILE", "HOME"] {
        if let Ok(b) = std::env::var(var) {
            let b = PathBuf::from(b);
            for sub in ["", "Programs", "Desktop", "Bureau", "Downloads", "Téléchargements", "Documents", "OneDrive\\Bureau", "OneDrive\\Desktop"] {
                let d = if sub.is_empty() { b.clone() } else { b.join(sub) };
                if d.is_dir() && !roots.contains(&d) {
                    roots.push(d);
                }
            }
        }
    }
    find_exe_under(&roots)
}

/// Décode un fichier texte : UTF-16 (avec BOM), UTF-8 (avec ou sans BOM) ou Windows-1252 / Latin-1.
pub fn decode_text(b: &[u8]) -> String {
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) {
        let le = b[0] == 0xFF;
        let units: Vec<u16> = b[2..].chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        return String::from_utf16_lossy(&units);
    }
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(), // Latin-1 : chaque octet = un caractère
    }
}

/// Valeur de `nzbPath` dans le texte de ngPost.conf (telle qu'écrite).
#[allow(dead_code)]
pub fn parse_nzb_path(text: &str) -> Option<String> {
    NgConf::parse(Path::new(""), text).get("nzbPath").map(str::to_string)
}

/// Nom de release correspondant à un NZB (« Release.nzb » ou « Release.mkv.nzb »).
pub fn release_of_nzb(p: &Path) -> String {
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let sp = Path::new(&stem);
    if prep::is_video(sp) {
        sp.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(stem)
    } else {
        stem
    }
}

fn age(p: &Path) -> Duration {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .unwrap_or_default()
}

fn mtime_secs(p: &Path) -> u64 {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(p) = to.parent() {
        fs::create_dir_all(p)?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        copy_dir(from, to)?;
        fs::remove_dir_all(from)
    } else {
        fs::copy(from, to)?;
        fs::remove_file(from)
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)?.flatten() {
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t)?;
        } else {
            fs::copy(&p, &t)?;
        }
    }
    Ok(())
}

// ------------------------------------------------------- post en ligne de commande

/// Sous-dossier du dossier de sortie où ranger les vidéos postées.
pub const POSTED_DIR: &str = "_Postés";

/// L'option `obfuscate` de ngPost.conf active-t-elle l'obfuscation des articles ?
pub fn conf_obfuscates(v: &str) -> bool {
    let l = v.trim().to_lowercase();
    l.contains("article") || l == "true" || l == "1" || l == "yes" || l == "on"
}

impl NgConf {
    /// Valeur numérique d'une clé (ex. RAR_SIZE = 42).
    #[allow(dead_code)]
    pub fn num(&self, key: &str) -> Option<u32> {
        self.get(key).and_then(|v| v.trim().parse().ok())
    }
    /// Mots-clés de `PACK` (COMPRESS, GEN_NAME, GEN_PASS, GEN_PAR2).
    pub fn pack(&self) -> Vec<String> {
        self.get("PACK").map(|v| v.split([',', ' ', ';']).map(|x| x.trim().to_uppercase()).filter(|x| !x.is_empty()).collect()).unwrap_or_default()
    }
}

/// Reprend les options de post de ngPost.conf (PACK, obfuscate, RAR_PASS) dans les réglages.
/// Les valeurs chiffrées restent à 0 = « celle de ngPost.conf ». Renvoie ce qui a été repris.
pub fn import_post_opts(cfg: &mut Config, c: &NgConf) -> Vec<String> {
    let mut msg = Vec::new();
    let pack = c.pack();
    if c.get("PACK").is_some() {
        let has = |k: &str| pack.iter().any(|x| x == k);
        cfg.ng_compress = has("COMPRESS");
        cfg.ng_gen_name = has("GEN_NAME");
        cfg.ng_gen_pass = has("GEN_PASS");
        cfg.ng_par2 = has("GEN_PAR2");
        msg.push(format!("PACK = {}", pack.join(", ")));
    }
    if let Some(o) = c.get("obfuscate") {
        cfg.post_obfuscate = conf_obfuscates(o);
        msg.push(format!("obfuscate = {o}"));
    }
    if let Some(p) = c.get("RAR_PASS") {
        if !cfg.ng_gen_pass {
            cfg.ng_rar_pass = p.to_string();
            msg.push("RAR_PASS".into());
        }
    }
    cfg.ng_imported = true;
    msg
}

fn path_arg(p: &Path) -> String {
    native_path(&p.display().to_string())
}

/// Dossier donné à NgPost qui y ajoute lui-même « /<nom> » : en barres « / » partout (Windows
/// accepte les deux), pour que le chemin final ne mélange jamais « \ » et « / ».
/// Limite : un dossier réseau (« \\serveur\partage ») redevient mélangé chez NgPost, qui
/// remet « \\ » en tête : `tmp_dir_problem` le signale avant le post.
pub fn ngpost_dir_arg(v: &str) -> String {
    let v = v.trim().trim_matches('"').trim();
    let s = v.replace('\\', "/");
    let t = s.trim_end_matches('/');
    // « D:/ » : la racine garde sa barre
    if t.len() == 2 && t.ends_with(':') {
        return format!("{t}/");
    }
    if t.is_empty() {
        s
    } else {
        t.to_string()
    }
}

/// Arguments de NgPost pour poster `input` et écrire le NZB dans `nzb`.
/// `conf` : ngPost.conf (serveurs, groupes, chemins de 7-Zip/par2…) passé avec -c.
pub fn post_args(cfg: &Config, conf: Option<&Path>, input: &Path, nzb: &Path) -> Vec<String> {
    post_args_multi(cfg, conf, &[input.to_path_buf()], nzb)
}

// --------------------------------------------------------- fichiers de réparation (par2)

/// Outils proposés : réglage, nom affiché.
pub const PAR2_TOOLS: [(&str, &str); 3] = [("", "par2 de NgPost"), ("turbo", "par2cmdline-turbo"), ("parpar", "ParPar")];

pub fn par2_tool_label(tool: &str) -> &'static str {
    PAR2_TOOLS.iter().find(|t| t.0 == tool).map(|t| t.1).unwrap_or("par2 de NgPost")
}

/// par2 livré avec NgPost (à côté de ngPost.exe).
pub fn ngpost_par2(cfg: &Config) -> Option<PathBuf> {
    let p = exe_path(cfg).parent()?.join(if cfg!(windows) { "par2.exe" } else { "par2" });
    p.is_file().then_some(p)
}

/// Chemin de l'outil : celui réglé, sinon cherché (dossier de NgPost, Program Files, dossier
/// des programmes de l'utilisateur, PATH).
pub fn find_par2_tool(cfg: &Config, tool: &str) -> Option<PathBuf> {
    let set = match tool {
        "turbo" => cfg.ng_turbo.trim(),
        "parpar" => cfg.ng_parpar.trim(),
        _ => return ngpost_par2(cfg),
    };
    if !set.is_empty() {
        let p = PathBuf::from(native_path(set.trim_matches('"')));
        return p.is_file().then_some(p);
    }
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let (dirs, file): (&[&str], String) = match tool {
        "turbo" => (&["par2cmdline-turbo", "par2-turbo", "par2turbo"], format!("par2{exe}")),
        _ => (&["ParPar", "parpar"], format!("parpar{exe}")),
    };
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Some(d) = exe_path(cfg).parent() {
        bases.push(d.to_path_buf());
    }
    for v in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(d) = std::env::var_os(v) {
            bases.push(PathBuf::from(&d));
            bases.push(PathBuf::from(d).join("Programs"));
        }
    }
    let mut cands: Vec<PathBuf> = Vec::new();
    for b in &bases {
        for d in dirs {
            cands.push(b.join(d).join(&file));
        }
    }
    if tool == "turbo" {
        // à côté de NgPost, renommé pour ne pas remplacer le par2 livré avec lui
        if let Some(d) = exe_path(cfg).parent() {
            cands.push(d.join(format!("par2-turbo{exe}")));
            cands.push(d.join(format!("par2turbo{exe}")));
        }
    } else if let Some(path) = std::env::var_os("PATH") {
        cands.extend(std::env::split_paths(&path).map(|d| d.join(&file)));
    }
    cands.into_iter().find(|p| p.is_file())
}

/// Redondance utilisée : réglage, sinon PAR2_PCT de ngPost.conf, sinon 10 %.
pub fn par2_pct(cfg: &Config, conf: Option<&NgConf>) -> u32 {
    if cfg.ng_par2_pct > 0 {
        return cfg.ng_par2_pct;
    }
    conf.and_then(|c| c.get("PAR2_PCT")).and_then(|v| v.trim().trim_end_matches('%').parse().ok()).filter(|v| *v > 0).unwrap_or(10)
}

/// Arguments de ParPar (ceux conseillés par NgPost, redondance en pourcentage).
pub fn parpar_args(pct: u32) -> String {
    format!("-s5M -r{pct}% -m2048M -p1l --progress stdout -q")
}

/// PAR2_ARGS écrits pour ParPar (pas compris par par2cmdline).
fn parpar_style(args: &str) -> bool {
    let a = args.trim();
    !a.is_empty() && !a.starts_with("c ") && !a.starts_with("create") && (a.contains("--progress") || a.contains("-r1n") || a.contains("-p1l") || a.contains('%') || a.contains("-o"))
}

/// Outil par2 choisi pour un post : arguments à ajouter à NgPost et, si besoin, une copie de
/// ngPost.conf (`base`) écrite dans `dir` avec PAR2_ARGS adaptés (ParPar n'a pas d'option de
/// ligne de commande pour eux). Outil introuvable : par2 de NgPost, signalé dans le journal.
pub fn par2_setup(cfg: &Config, base: Option<&Path>, dir: &Path, on_line: &mut dyn FnMut(&str)) -> (Option<PathBuf>, Vec<String>) {
    let tool = cfg.ng_par2_tool.as_str();
    if !cfg.ng_par2 || tool.is_empty() {
        return (None, Vec::new());
    }
    let label = par2_tool_label(tool);
    let Some(path) = find_par2_tool(cfg, tool) else {
        on_line(&format!("⚠ {label} introuvable : par2 de NgPost utilisé (Réglages → Options du post)"));
        return (None, Vec::new());
    };
    let text = base.and_then(|p| fs::read(p).ok()).map(|b| decode_text(&b)).unwrap_or_default();
    let conf = base.map(|p| NgConf::parse(p, &text));
    let pct = par2_pct(cfg, conf.as_ref());
    let mut args = vec!["--par2_path".to_string(), path_arg(&path)];
    // ce que le fichier est vraiment (ParPar mis dans le réglage de turbo, ou l'inverse)
    let kind = par2_kind(&path).0;
    if kind == Par2Kind::MultiPar {
        on_line("⚠ MultiPar n'est pas géré ici : par2 de NgPost utilisé");
        return (None, Vec::new());
    }
    let as_parpar = kind == Par2Kind::ParPar || (kind == Par2Kind::Unknown && tool == "parpar");
    if (tool == "parpar") != as_parpar {
        on_line(&format!("ℹ {} est {} : utilisé comme tel", path.display(), kind.label()));
    }
    let label = if kind == Par2Kind::Unknown { label } else { kind.label() };
    let new_text = match as_parpar {
        true => {
            if base.is_none() {
                on_line("⚠ ParPar : ngPost.conf introuvable (ses réglages PAR2_ARGS y sont écrits) : par2 de NgPost utilisé");
                return (None, Vec::new());
            }
            // NgPost ne reconnaît ParPar qu'à son nom
            if !prep::file_name(&path).to_lowercase().contains("parpar") && !path.display().to_string().to_lowercase().contains("parpar") {
                on_line("⚠ ParPar : son chemin doit contenir « parpar » pour que NgPost le reconnaisse : par2 de NgPost utilisé");
                return (None, Vec::new());
            }
            Some(crate::usenet::set_option(&text, "PAR2_ARGS", &parpar_args(pct)))
        }
        false => {
            // PAR2_ARGS écrits pour ParPar : retirés (par2cmdline ne les comprend pas)
            let pa = conf.as_ref().and_then(|c| c.get("PAR2_ARGS")).unwrap_or("").to_string();
            if cfg.ng_par2_pct == 0 {
                args.extend(["--par2_pct".to_string(), pct.to_string()]);
            }
            (base.is_some() && parpar_style(&pa)).then(|| crate::usenet::set_option(&text, "PAR2_ARGS", ""))
        }
    };
    on_line(&format!("Fichiers de réparation : {label} ({}) · {pct} % de redondance", path.display()));
    let Some(t) = new_text else { return (None, args) };
    let p = dir.join("ngPost-nzbff-par2.conf");
    match fs::write(&p, t) {
        Ok(()) => (Some(p), args),
        Err(e) => {
            on_line(&format!("⚠ {label} : configuration NgPost non écrite ({e}) : par2 de NgPost utilisé"));
            (None, Vec::new())
        }
    }
}

/// Ce qu'est vraiment un exécutable par2 (d'après son nom et ce qu'il annonce), quel que soit
/// le réglage où il a été mis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Par2Kind {
    /// par2cmdline classique (celui livré avec NgPost).
    Par2,
    Turbo,
    ParPar,
    /// MultiPar (par2j) : pas géré ici.
    MultiPar,
    /// Muet : traité comme par2cmdline.
    Unknown,
}

impl Par2Kind {
    pub fn label(self) -> &'static str {
        match self {
            Par2Kind::Par2 => "par2cmdline",
            Par2Kind::Turbo => "par2cmdline-turbo",
            Par2Kind::ParPar => "ParPar",
            Par2Kind::MultiPar => "MultiPar",
            Par2Kind::Unknown => "par2 (inconnu)",
        }
    }
}

/// Nature et version d'un outil par2 (lancé une fois avec « --version », puis gardé en
/// mémoire tant que le fichier ne change pas).
pub fn par2_kind(exe: &Path) -> (Par2Kind, Option<String>) {
    use std::collections::HashMap;
    static CACHE: std::sync::Mutex<Option<HashMap<PathBuf, (u64, Par2Kind, Option<String>)>>> = std::sync::Mutex::new(None);
    let stamp = fs::metadata(exe).ok().map(|m| m.len() ^ m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)).unwrap_or(0);
    if let Some((st, k, v)) = CACHE.lock().ok().and_then(|c| c.as_ref().and_then(|m| m.get(exe).cloned())) {
        if st == stamp {
            return (k, v);
        }
    }
    let name = exe.display().to_string().to_lowercase();
    let file = prep::file_name(exe).to_lowercase();
    let out = par2_version_text(exe);
    let low = out.as_deref().unwrap_or("").to_lowercase();
    let kind = if file.contains("parpar") || low.contains("parpar") {
        Par2Kind::ParPar
    } else if file.contains("par2j") || low.contains("multipar") {
        Par2Kind::MultiPar
    } else if low.contains("turbo") {
        Par2Kind::Turbo
    } else if low.contains("par2cmdline") {
        Par2Kind::Par2
    } else if name.contains("turbo") {
        Par2Kind::Turbo
    } else {
        Par2Kind::Unknown
    };
    let ver = out.and_then(|t| t.lines().map(str::trim).find(|l| !l.is_empty() && l.chars().any(|c| c.is_ascii_digit())).map(|l| l.chars().take(80).collect()));
    if let Ok(mut c) = CACHE.lock() {
        c.get_or_insert_with(HashMap::new).insert(exe.to_path_buf(), (stamp, kind, ver.clone()));
    }
    (kind, ver)
}

/// Ce qu'un outil par2 répond à « --version » (sinon « -V »).
fn par2_version_text(exe: &Path) -> Option<String> {
    if !exe.is_file() {
        return None;
    }
    for a in ["--version", "-V"] {
        if let Ok(o) = hidden(&mut Command::new(exe)).arg(a).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).output() {
            let t = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            if t.chars().any(|c| c.is_ascii_digit()) {
                return Some(t);
            }
        }
    }
    None
}

/// Mesure, sur ce PC, chaque outil par2 trouvé : même fichier (512 Mo, contenu aléatoire,
/// dans le dossier temporaire du post), même redondance (10 %), mêmes blocs (1 Mo), même
/// mémoire. Le fichier et les par2 sont supprimés ensuite. Renvoie (outil, secondes,
/// négatif : échec, exécutable).
pub fn par2_bench(cfg: &Config, on_step: &mut dyn FnMut(&str)) -> Result<Vec<(String, f64, PathBuf)>, String> {
    // chaque exécutable une fois, appelé d'après ce qu'il est vraiment
    let mut tools: Vec<(String, PathBuf, bool)> = Vec::new();
    for (k, _) in PAR2_TOOLS {
        if let Some(p) = find_par2_tool(cfg, k) {
            if !tools.iter().any(|t| t.1 == p) {
                let (kind, ver) = par2_kind(&p);
                if kind == Par2Kind::MultiPar {
                    continue;
                }
                let name = if k.is_empty() && matches!(kind, Par2Kind::Par2 | Par2Kind::Unknown) { "par2 de NgPost".to_string() } else { kind.label().to_string() };
                let v = ver.map(|v| format!(" ({v})")).unwrap_or_default();
                tools.push((format!("{name}{v}"), p, kind == Par2Kind::ParPar));
            }
        }
    }
    if tools.is_empty() {
        return Err("aucun outil par2 trouvé (par2 de NgPost, par2cmdline-turbo, ParPar)".into());
    }
    let conf = read_conf(cfg);
    let base = if !cfg.ng_tmp_dir.trim().is_empty() { PathBuf::from(native_path(cfg.ng_tmp_dir.trim())) } else { conf.as_ref().and_then(|c| c.get("TMP_DIR")).map(|t| PathBuf::from(native_path(t))).unwrap_or_else(std::env::temp_dir) };
    let dir = base.join(format!("nzbff-par2-test-{}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| format!("{} : {e}", dir.display()))?;
    let res = (|| {
        let file = dir.join("test.bin");
        on_step("Création du fichier de test (512 Mo)…");
        {
            use std::io::Write;
            let mut f = std::io::BufWriter::new(fs::File::create(&file).map_err(|e| e.to_string())?);
            let mut x: u64 = 0x9E37_79B9_7F4A_7C15 ^ prep::now_secs();
            let mut buf = vec![0u8; 1 << 20];
            for _ in 0..512 {
                for c in buf.chunks_mut(8) {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    c.copy_from_slice(&x.to_le_bytes()[..c.len()]);
                }
                f.write_all(&buf).map_err(|e| e.to_string())?;
            }
        }
        let mut out = Vec::new();
        for (k, (label, exe, parpar)) in tools.iter().enumerate() {
            on_step(&format!("Mesure de {label}…"));
            let par = dir.join(format!("t{k}.par2"));
            let args: Vec<String> = if *parpar {
                vec!["-s1M".into(), "-r10%".into(), "-m2048M".into(), "-q".into(), "-o".into(), path_arg(&par), path_arg(&file)]
            } else {
                vec!["c".into(), "-q".into(), "-r10".into(), "-s1048576".into(), "-m2048".into(), "-B".into(), path_arg(&dir), path_arg(&par), path_arg(&file)]
            };
            let t0 = std::time::Instant::now();
            let mut child = hidden(&mut Command::new(exe)).args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| format!("{label} : {e}"))?;
            let status = loop {
                if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
                    break st;
                }
                if t0.elapsed().as_secs() > 900 {
                    let _ = child.kill();
                    return Err(format!("{label} : plus de 15 minutes, arrêté"));
                }
                std::thread::sleep(Duration::from_millis(100));
            };
            let secs = t0.elapsed().as_secs_f64();
            if !status.success() {
                let mut e = String::new();
                if let Some(mut s) = child.stderr.take() {
                    use std::io::Read;
                    let _ = s.read_to_string(&mut e);
                }
                let why = e.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("code d'erreur").to_string();
                out.push((format!("{label}, échec : {why}"), -1.0, exe.clone()));
                continue;
            }
            out.push((label.clone(), secs, exe.clone()));
            // par2 de ce test supprimés avant le suivant (place sur le disque)
            if let Ok(rd) = fs::read_dir(&dir) {
                for f in rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("par2")).unwrap_or(false)) {
                    let _ = fs::remove_file(f);
                }
            }
        }
        Ok(out)
    })();
    let _ = fs::remove_dir_all(&dir);
    res
}

// --------------------------------------------------------- arrêt et espace disque

static POST_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Message d'un post arrêté à la demande.
pub const POST_CANCELLED: &str = "post arrêté à ta demande";

/// Arrête le post en cours (NgPost et ses compresseurs).
pub fn cancel_post() {
    POST_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// Devenir de la vidéo (ou du dossier de série) postée, si elle est dans le dossier de sortie :
/// corbeille, suppression, rangement dans « _Postés » ou rien (réglage « Une fois postée »).
fn after_post(cfg: &Config, video: &Path, verified: bool, warnings: &mut Vec<String>) -> String {
    let mut video_after = "laissée en place".to_string();
    // dossier de sortie, ou dossier de secours d'un autre disque (release déplacée faute de place)
    let out = match (cfg.output_dir(), video.parent()) {
        (_, Some(p)) if prep::file_name(p) == RELEASES_DIR => Some(p.to_path_buf()),
        (o, _) => o,
    };
    if let Some(out) = out {
        if video.parent().map(|p| p == out.as_path()).unwrap_or(false) {
            let to_posted = |w: &mut Vec<String>| match move_path(video, &out.join(POSTED_DIR).join(prep::file_name(video))) {
                Ok(_) => format!("rangée dans {POSTED_DIR}"),
                Err(e) => {
                    w.push(format!("vidéo non rangée : {e}"));
                    "laissée en place".to_string()
                }
            };
            video_after = match cfg.post_after.as_str() {
                "keep" => "laissée dans le dossier de sortie".to_string(),
                "trash" | "delete" if !verified => {
                    warnings.push(format!("vidéo gardée dans {POSTED_DIR} (pas supprimée) : NZB non vérifié ou incomplet"));
                    to_posted(warnings)
                }
                "trash" => match prep::trash(video) {
                    Ok(_) => "supprimée (corbeille)".to_string(),
                    Err(e) => {
                        warnings.push(format!("{e} : vidéo rangée dans {POSTED_DIR}"));
                        to_posted(warnings)
                    }
                },
                "delete" => match if video.is_dir() { fs::remove_dir_all(video) } else { fs::remove_file(video) } {
                    Ok(_) => "supprimée".to_string(),
                    Err(e) => {
                        warnings.push(format!("suppression impossible ({e}) : vidéo rangée dans {POSTED_DIR}"));
                        to_posted(warnings)
                    }
                },
                _ => to_posted(warnings),
            };
        }
    }
    video_after
}

/// Série trop grosse pour la place libre : elle est compressée et postée en plusieurs lots
/// (chaque archive garde `Série/Saison N/…`, un seul mot de passe), puis les NZB sont fusionnés
/// en un seul. Chez celui qui télécharge : un NZB, et l'arborescence complète une fois extrait.
fn post_in_lots(
    cfg: &Config,
    conf: Option<&NgConf>,
    dir: &Path,
    release: &str,
    why: &str,
    on_line: &mut dyn FnMut(&str),
    on_progress: &mut dyn FnMut(&Progress),
) -> Result<Posted, String> {
    let tmp = tmp_dir(cfg, conf);
    let free = prep::free_space(&tmp).unwrap_or(0);
    let lots_dir = dir.join(".nzbff-lots");
    let _ = fs::remove_dir_all(&lots_dir);
    // contenu de la release (épisodes ou dossiers « Saison N »), dans l'ordre de l'arborescence
    let show = dir.to_path_buf();
    let mut files: Vec<(PathBuf, u64)> = Vec::new();
    fn walk(base: &Path, d: &Path, out: &mut Vec<(PathBuf, u64)>) {
        let mut v: Vec<PathBuf> = fs::read_dir(d).map(|r| r.flatten().map(|e| e.path()).filter(|p| !prep::file_name(p).starts_with('.')).collect()).unwrap_or_default();
        v.sort();
        for p in v {
            if p.is_dir() {
                walk(base, &p, out);
            } else if let Ok(rel) = p.strip_prefix(base) {
                out.push((rel.to_path_buf(), fs::metadata(&p).map(|m| m.len()).unwrap_or(0)));
            }
        }
    }
    walk(&show, &show, &mut files);
    // lots : le plus de fichiers possible tant que archives + par2 tiennent dans la place libre
    let budget = free * 95 / 100;
    if let Some((f, sz)) = files.iter().find(|(_, sz)| tmp_needed(cfg, conf, *sz) > budget) {
        return Err(format!(
            "{why}. Même en plusieurs lots, impossible : {} ({}) demande à lui seul environ {} pour ses archives",
            prep::file_name(f),
            prep::human_size(*sz),
            prep::human_size(tmp_needed(cfg, conf, *sz))
        ));
    }
    let mut lots: Vec<Vec<(PathBuf, u64)>> = Vec::new();
    let mut cur: Vec<(PathBuf, u64)> = Vec::new();
    for f in files {
        let size: u64 = cur.iter().map(|x| x.1).sum::<u64>() + f.1;
        if !cur.is_empty() && tmp_needed(cfg, conf, size) > budget {
            lots.push(std::mem::take(&mut cur));
        }
        cur.push(f);
    }
    if !cur.is_empty() {
        lots.push(cur);
    }
    let n = lots.len();
    on_line(&format!(
        "Pas assez de place dans {} pour tout compresser d'un coup ({} libres) : compression en {n} lots, puis un seul NZB",
        tmp.display(),
        prep::human_size(free)
    ));
    // un seul mot de passe pour toutes les archives
    let mut sub = cfg.clone();
    if cfg.ng_gen_pass {
        sub.ng_gen_pass = false;
        sub.ng_rar_pass = crate::backup::random_token(20);
    }
    sub.indexer_dir.clear(); // NZB partiels : jamais distribués aux indexers
    sub.nzb_dir = lots_dir.join("nzb").display().to_string();
    sub.post_after = "keep".into();
    // mêmes groupes pour tous les lots (déjà choisis pour la release, tirage au sort compris)
    sub.groups_film = sub.ng_groups.clone();
    sub.groups_tv = sub.ng_groups.clone();
    if sub.group_policy == "each_post" {
        sub.group_policy = "all".into();
    }
    // serveur le plus rapide choisi une fois pour tous les lots
    if cfg.usenet_managed && cfg.usenet_auto_best {
        sub.usenet_servers = crate::usenet::servers_for_post(cfg, on_line);
        sub.usenet_auto_best = false;
    }
    let mut parts: Vec<Posted> = Vec::new();
    for (k, lot) in lots.iter().enumerate() {
        // le lot reprend l'arborescence de la release (épisodes, ou « Saison N/… »)
        let ldir = lots_dir.join(format!("lot{}", k + 1));
        let lshow = ldir.clone();
        // fichiers rangés le temps du lot (déplacement sur le même disque : instantané)
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut err = None;
        for (rel, _) in lot {
            let (from, to) = (show.join(rel), lshow.join(rel));
            if let Some(p) = to.parent() {
                let _ = fs::create_dir_all(p);
            }
            match fs::rename(&from, &to) {
                Ok(_) => moved.push((from, to)),
                Err(e) => {
                    err = Some(format!("préparation du lot {} : {e}", k + 1));
                    break;
                }
            }
        }
        let size: u64 = lot.iter().map(|x| x.1).sum();
        on_line(&format!("Lot {}/{n} : {} fichier(s), {}", k + 1, lot.len(), prep::human_size(size)));
        let res = match err {
            Some(e) => Err(e),
            None => post_file(&sub, &ldir, &format!("{release}.lot{}", k + 1), on_line, &mut |p| {
                let mut q = p.clone();
                q.phase = format!("Lot {}/{n} · {}", k + 1, p.phase);
                // progression globale : lots terminés + avancement du lot en cours
                q.pct = ((k as u32 * 100 + p.pct) / n as u32).min(100);
                on_progress(&q)
            }),
        };
        for (from, to) in moved.iter().rev() {
            let _ = fs::rename(to, from);
        }
        match res {
            Ok(p) => parts.push(p),
            Err(e) => {
                let _ = fs::remove_dir_all(&lots_dir);
                return Err(format!("lot {}/{n} : {e}", k + 1));
            }
        }
    }
    // un seul NZB : les fichiers de tous les lots dans le premier
    let mut merged = String::new();
    for (k, p) in parts.iter().enumerate() {
        let t = fs::read_to_string(&p.nzb).map_err(|e| format!("lecture du NZB du lot {} : {e}", k + 1))?;
        if k == 0 {
            merged = t.trim_end().trim_end_matches("</nzb>").trim_end().to_string();
            merged.push('\n');
        } else if let (Some(a), Some(b)) = (t.find("<file"), t.rfind("</file>")) {
            merged.push_str(&t[a..b + "</file>".len()]);
            merged.push('\n');
        }
    }
    merged.push_str("</nzb>\n");
    let target = nzb_target(cfg, release, dir);
    if let Some(d) = target.parent() {
        fs::create_dir_all(d).map_err(|e| format!("dossier du NZB : {e}"))?;
    }
    fs::write(&target, merged).map_err(|e| format!("écriture du NZB : {e}"))?;
    let _ = fs::remove_dir_all(&lots_dir);
    let info = nzb_info(&target)?;
    let mut warnings: Vec<String> = parts.iter().flat_map(|p| p.warnings.iter().cloned()).collect();
    warnings.insert(0, format!("posté en {n} lots (manque de place pour tout compresser d'un coup), NZB fusionné"));
    let verified = info.files > 0;
    let video_after = after_post(cfg, dir, verified, &mut warnings);
    let secs: u64 = parts.iter().map(|p| p.secs).sum();
    let bytes: u64 = parts.iter().map(|p| p.bytes).sum();
    on_line(&format!("✔ {n} lots postés, NZB fusionné : {}", info.summary()));
    Ok(Posted {
        nzb: target,
        info,
        log: parts.first().and_then(|p| p.log.clone()),
        secs,
        summary: parts.last().map(|p| p.summary.clone()).unwrap_or_default(),
        warnings,
        avg_bps: Some(bytes as f64 / secs.max(1) as f64),
        started: parts.first().map(|p| p.started).unwrap_or_else(prep::now_secs),
        bytes,
        articles: parts.iter().map(|p| p.articles).sum(),
        connections: parts.first().and_then(|p| p.connections),
        samples: parts.iter().flat_map(|p| p.samples.iter().cloned()).collect(),
        video_after,
    })
}

/// Arrête un processus et tous ses enfants (rar.exe, 7z.exe, par2…).
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut c = Command::new("taskkill");
        c.args(["/PID", &pid.to_string(), "/T", "/F"]);
        hidden(&mut c);
        let _ = c.output();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("pkill").args(["-KILL", "-P", &pid.to_string()]).output();
        let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).output();
    }
}

fn fmt_secs(s: u64) -> String {
    if s >= 3600 {
        format!("{}h{:02}", s / 3600, (s / 60) % 60)
    } else {
        format!("{} min {:02} s", s / 60, s % 60)
    }
}

/// Retire les archives laissées par un post arrêté : seulement les éléments apparus dans le
/// dossier temporaire pendant le post et qui contiennent des archives ou des par2.
fn clean_tmp(root: &Path, before: &std::collections::HashSet<PathBuf>) -> String {
    let is_arch = |p: &Path| {
        let n = prep::file_name(p).to_lowercase();
        [".rar", ".7z", ".par2", ".zip"].iter().any(|x| n.ends_with(x)) || n.rsplit('.').next().map(|e| e.len() == 3 && e.chars().all(|c| c.is_ascii_digit())).unwrap_or(false) || n.contains(".part")
    };
    let mut freed = 0u64;
    let mut n = 0;
    for e in fs::read_dir(root).map(|r| r.flatten().map(|e| e.path()).collect::<Vec<_>>()).unwrap_or_default() {
        if before.contains(&e) {
            continue;
        }
        let ok = if e.is_dir() { fs::read_dir(&e).map(|r| r.flatten().any(|f| is_arch(&f.path()))).unwrap_or(false) } else { is_arch(&e) };
        if ok {
            let size = path_size(&e);
            if (if e.is_dir() { fs::remove_dir_all(&e) } else { fs::remove_file(&e) }).is_ok() {
                freed += size;
                n += 1;
            }
        }
    }
    format!("{n} élément(s), {}", prep::human_size(freed))
}

/// Taille d'un fichier ou d'un dossier (récursif).
pub fn path_size(p: &Path) -> u64 {
    if p.is_dir() {
        fs::read_dir(p).map(|r| r.flatten().map(|e| path_size(&e.path())).sum()).unwrap_or(0)
    } else {
        fs::metadata(p).map(|m| m.len()).unwrap_or(0)
    }
}

/// Dossier où NgPost fabrique archives et par2 : réglage du post, sinon TMP_DIR de
/// ngPost.conf, sinon le dossier temporaire du système.
pub fn tmp_dir(cfg: &Config, conf: Option<&NgConf>) -> PathBuf {
    if !cfg.ng_tmp_dir.trim().is_empty() {
        return PathBuf::from(native_path(cfg.ng_tmp_dir.trim()));
    }
    if let Some(t) = conf.and_then(|c| c.get("TMP_DIR")).map(str::trim).filter(|t| !t.is_empty()) {
        return PathBuf::from(native_path(t));
    }
    std::env::temp_dir()
}

/// Dossier temporaire réseau (chemin UNC) avec un par2 de la famille
/// par2cmdline : NgPost remet la double barre inverse en tête d'un chemin en « / », que par2cmdline coupe au
/// mauvais endroit (code 3). ParPar n'est pas concerné.
pub fn tmp_dir_problem(cfg: &Config, conf: Option<&NgConf>) -> Option<String> {
    if !cfg.ng_par2 {
        return None;
    }
    let t = if cfg.ng_tmp_dir.trim().is_empty() { conf.and_then(|c| c.get("TMP_DIR")).unwrap_or("").trim().to_string() } else { cfg.ng_tmp_dir.trim().to_string() };
    let unc = t.starts_with("\\\\") || t.starts_with("//");
    (unc && cfg.ng_par2_tool != "parpar").then(|| format!("Dossier temporaire réseau ({t}) : par2cmdline n'y retrouve pas les archives créées par NgPost. Choisis un dossier sur un disque local (Réglages → Options du post) ou ParPar"))
}

/// Place nécessaire pour poster `size` octets : archives (compression) + par2, avec 3 % de marge.
pub fn tmp_needed(cfg: &Config, conf: Option<&NgConf>, size: u64) -> u64 {
    let pct = if cfg.ng_par2_pct > 0 { cfg.ng_par2_pct as u64 } else { conf.and_then(|c| c.get("PAR2_PCT")).and_then(|v| v.trim().trim_end_matches('%').parse().ok()).unwrap_or(10) };
    let par2 = if cfg.ng_par2 { size * pct / 100 } else { 0 };
    let arch = if cfg.ng_compress { size } else { 0 };
    (arch + par2) * 103 / 100
}

/// Problème d'espace avant un post (None : assez de place, ou impossible à savoir).
pub fn space_problem(cfg: &Config, conf: Option<&NgConf>, input: &Path) -> Option<String> {
    if cfg.post_args.contains("{input}") {
        return None; // ligne de commande écrite à la main : options inconnues
    }
    let need = tmp_needed(cfg, conf, path_size(input));
    if need == 0 {
        return None;
    }
    let dir = tmp_dir(cfg, conf);
    let free = prep::free_space(&dir)?;
    (free < need).then(|| {
        format!(
            "{SPACE_ERR} pour les archives temporaires de NgPost dans {} : {} libres, environ {} nécessaires ({}). NgPost n'a pas été lancé",
            dir.display(),
            prep::human_size(free),
            prep::human_size(need),
            if cfg.ng_compress { "archives + par2" } else { "par2" }
        )
    })
}

/// Dossier où sont déplacées les releases quand le disque de départ manque de place
/// (à la racine du disque choisi) : traité comme le dossier de sortie après le post.
pub const RELEASES_DIR: &str = "NZB FF Tool Pro - releases";

/// Début des messages de manque de place (reconnu par l'interface pour proposer des solutions).
pub const SPACE_ERR: &str = "Espace insuffisant";

/// L'échec ressemble-t-il à un manque de place ou à une compression ratée ?
pub fn is_space_error(e: &str) -> bool {
    let l = e.to_lowercase();
    ["espace insuffisant", "not enough space", "no space left", "disk full", "disque plein", "rar exit code", "erreur lors de la compression", "compression error"].iter().any(|k| l.contains(k))
}

/// Entrées à donner à NgPost pour une release de série (un dossier) :
/// - avec compression : le contenu de la release (épisodes, ou dossiers « Saison N »), chacun
///   en `-i` : l'archive contient exactement ce que contient la release, sans dossier en plus ;
/// - sans compression : chaque fichier (NgPost les poste à plat).
pub fn post_inputs(cfg: &Config, dir: &Path) -> Vec<PathBuf> {
    let visible = |p: &PathBuf| !prep::file_name(p).starts_with('.');
    if cfg.post_args.contains("{input}") {
        return vec![dir.to_path_buf()];
    }
    if cfg.ng_compress {
        let mut entries: Vec<PathBuf> = fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).filter(visible).collect()).unwrap_or_default();
        entries.sort();
        // des centaines d'épisodes : la ligne de commande de Windows (32 767 caractères) ne les
        // contient pas tous ; NgPost compresse alors le dossier lui-même
        if args_len(&entries) > ARGS_BUDGET {
            return vec![dir.to_path_buf()];
        }
        return entries;
    }
    let mut files = Vec::new();
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(r) = fs::read_dir(d) {
            let mut v: Vec<PathBuf> = r.flatten().map(|e| e.path()).filter(|p| !prep::file_name(p).starts_with('.')).collect();
            v.sort();
            for p in v {
                if p.is_dir() {
                    walk(&p, out);
                } else {
                    out.push(p);
                }
            }
        }
    }
    walk(dir, &mut files);
    if args_len(&files) > ARGS_BUDGET {
        return collapse_dirs(files);
    }
    files
}

/// Place laissée aux fichiers dans la ligne de commande (Windows : 32 767 caractères en tout,
/// chemins longs activés ou non).
const ARGS_BUDGET: usize = 24_000;

fn args_len(v: &[PathBuf]) -> usize {
    v.iter().map(|p| p.display().to_string().len() + 6).sum()
}

/// Remplace les fichiers d'un même dossier par le dossier, quand ils en sont tous les fichiers
/// visibles : NgPost poste alors les fichiers du dossier (« -i dossier », sans sous-dossiers).
pub fn collapse_dirs(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut by_dir: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
    for f in files {
        let d = f.parent().map(Path::to_path_buf).unwrap_or_default();
        match by_dir.iter_mut().find(|(x, _)| *x == d) {
            Some((_, v)) => v.push(f),
            None => by_dir.push((d, vec![f])),
        }
    }
    for (d, v) in by_dir {
        let all: Vec<PathBuf> = fs::read_dir(&d)
            .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_file() && !prep::file_name(p).starts_with('.')).collect())
            .unwrap_or_default();
        if v.len() > 1 && all.len() == v.len() && all.iter().all(|a| v.contains(a)) {
            out.push(d);
        } else {
            out.extend(v);
        }
    }
    out
}

/// Arguments de NgPost pour une ou plusieurs entrées (« -i » répété).
pub fn post_args_multi(cfg: &Config, conf: Option<&Path>, inputs: &[PathBuf], nzb: &Path) -> Vec<String> {
    let input = inputs.first().cloned().unwrap_or_default();
    let (i, n) = (path_arg(&input), path_arg(nzb));
    let extra = cfg.post_args.trim();
    if extra.contains("{input}") {
        // ancien réglage : ligne de commande complète écrite à la main
        let mut a: Vec<String> = split_args(extra).into_iter().map(|x| x.replace("{input}", &i).replace("{nzb}", &n)).collect();
        if cfg.post_obfuscate && !a.iter().any(|x| x == "-x" || x == "--obfuscate") {
            a.push("-x".into());
        }
        return a;
    }
    let mut a: Vec<String> = Vec::new();
    let mut push = |v: &[&str]| a.extend(v.iter().map(|x| x.to_string()));
    if let Some(c) = conf {
        push(&["-c", &path_arg(c)]);
    }
    for inp in inputs {
        push(&["-i", &path_arg(inp)]);
    }
    push(&["-o", &n, "--disp_progress", "BAR"]);
    if cfg.post_obfuscate {
        push(&["-x"]);
    }
    let groups: Vec<&str> = cfg.ng_groups.split([',', ';', ' ']).map(str::trim).filter(|g| !g.is_empty()).collect();
    if !groups.is_empty() {
        push(&["-g", &groups.join(",")]);
    }
    if !cfg.ng_from.trim().is_empty() {
        push(&["-f", cfg.ng_from.trim()]);
    }
    let tmp = cfg.ng_tmp_dir.trim();
    if cfg.ng_compress {
        push(&["--compress"]);
        if cfg.ng_gen_name {
            push(&["--gen_name"]);
        }
        if cfg.ng_gen_pass {
            push(&["--gen_pass"]);
        } else if !cfg.ng_rar_pass.trim().is_empty() {
            push(&["--rar_pass", cfg.ng_rar_pass.trim()]);
        }
        if cfg.ng_rar_size > 0 {
            push(&["--rar_size", &cfg.ng_rar_size.to_string()]);
        }
        if cfg.ng_rar_max > 0 {
            push(&["--rar_max", &cfg.ng_rar_max.to_string()]);
        }
        if !cfg.ng_rar_path.trim().is_empty() {
            push(&["--rar_path", &native_path(&cfg.ng_rar_path)]);
        }
    }
    if cfg.ng_par2 {
        push(&["--gen_par2"]);
        if cfg.ng_par2_pct > 0 {
            push(&["--par2_pct", &cfg.ng_par2_pct.to_string()]);
        }
    }
    // dossier temporaire toujours passé, en barres « / » : NgPost y ajoute « /<archive> » et
    // par2cmdline(-turbo) coupe un chemin mélangé (« D:\tmp/x/x.7z* ») à son dernier « \ »,
    // cherche alors « D:\x.7z.001 » et échoue (code 3, « Ignoring non-existent source file »)
    let conf_tmp = if tmp.is_empty() { conf.and_then(|c| fs::read(c).ok()).and_then(|b| NgConf::parse(Path::new(""), &decode_text(&b)).get("TMP_DIR").map(str::to_string)) } else { None };
    let tmp = if tmp.is_empty() { conf_tmp.as_deref().unwrap_or("").trim() } else { tmp };
    if (cfg.ng_compress || cfg.ng_par2) && !tmp.is_empty() {
        push(&["--tmp_dir", &ngpost_dir_arg(tmp)]);
    }
    if cfg.ng_threads > 0 {
        push(&["-t", &cfg.ng_threads.to_string()]);
    }
    if cfg.ng_article_size > 0 {
        push(&["-a", &cfg.ng_article_size.to_string()]);
    }
    if cfg.ng_retry > 0 {
        push(&["-r", &cfg.ng_retry.to_string()]);
    }
    if cfg.ng_debug {
        push(&["-d"]);
    }
    a.extend(split_args(extra));
    a
}

/// Réglages effectifs d'un post, en clair : ceux de l'application, sinon ceux de ngPost.conf.
pub fn post_settings(cfg: &Config, conf: Option<&NgConf>, art_size: f64) -> Vec<String> {
    let get = |k: &str| conf.and_then(|c| c.get(k)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let or_conf = |v: u32, k: &str, unit: &str| if v > 0 { format!("{v}{unit}") } else { get(k).map(|x| format!("{x}{unit} (ngPost.conf)")).unwrap_or_else(|| "par défaut de NgPost".into()) };
    let mut v = vec![format!(
        "Réglages du post : articles de {} Ko · fils : {} · nouveaux essais par article : {} · obfuscation des articles {}",
        (art_size / 1024.0).round() as u64,
        or_conf(cfg.ng_threads, "THREAD", ""),
        or_conf(cfg.ng_retry, "NB_RETRY", ""),
        if cfg.post_obfuscate { "oui (-x)" } else { "non" }
    )];
    v.push(if cfg.ng_compress {
        format!(
            "  Compression : oui · volumes de {}{} · noms {} · mot de passe {}",
            if cfg.ng_rar_size > 0 { format!("{} Mo", cfg.ng_rar_size) } else { get("RAR_SIZE").map(|x| format!("{x} Mo (ngPost.conf)")).unwrap_or_else(|| "taille par défaut".into()) },
            // NgPost agrandit les volumes pour ne pas dépasser RAR_MAX (ex. 15 Go, 99 volumes : 146 Mo)
            if cfg.ng_rar_max > 0 { format!(" (plus gros si plus de {} volumes)", cfg.ng_rar_max) } else { get("RAR_MAX").map(|x| format!(" (plus gros si plus de {x} volumes)")).unwrap_or_default() },
            if cfg.ng_gen_name { "aléatoires" } else { "de la release" },
            if cfg.ng_gen_pass { "aléatoire" } else if cfg.ng_rar_pass.trim().is_empty() { "aucun" } else { "fixe" }
        )
    } else {
        "  Compression : non (fichiers postés tels quels)".into()
    });
    v.push(if cfg.ng_par2 {
        format!(
            "  Par2 : oui · {} · {}",
            if cfg.ng_par2_pct > 0 { format!("{} % de redondance", cfg.ng_par2_pct) } else { get("PAR2_PCT").map(|x| format!("{x} % (ngPost.conf)")).unwrap_or_else(|| "redondance par défaut".into()) },
            match (cfg.ng_par2_tool.as_str(), find_par2_tool(cfg, &cfg.ng_par2_tool)) {
                ("", _) => "par2 de NgPost".to_string(),
                (t, Some(p)) => format!("{} ({})", par2_tool_label(t), p.display()),
                (t, None) => format!("{} introuvable : par2 de NgPost", par2_tool_label(t)),
            }
        )
    } else {
        "  Par2 : non".into()
    });
    v.push(format!("  Vérification du NZB après le post : {}", if cfg.post_verify { "oui" } else { "non" }));
    v
}

/// Ligne de commande affichable (arguments contenant des espaces entre guillemets).
pub fn command_line(exe: &Path, args: &[String]) -> String {
    let q = |a: &str| if a.is_empty() || a.contains(' ') { format!("\"{a}\"") } else { a.to_string() };
    let mut s = q(&exe.display().to_string());
    for a in args {
        s.push(' ');
        s.push_str(&q(a));
    }
    s
}

/// Aperçu de la commande lancée pour une vidéo.
pub fn command_preview(cfg: &Config, conf: Option<&Path>) -> String {
    let exe = exe_path(cfg);
    let exe = if exe.as_os_str().is_empty() { PathBuf::from("ngPost.exe") } else { exe };
    command_line(&exe, &post_args(cfg, conf, Path::new("<vidéo renommée>"), Path::new("<release>.nzb")))
}

/// Contrôles avant de poster : ce qui manque ou ne colle pas (en clair).
pub fn post_warnings(cfg: &Config, conf: Option<&NgConf>) -> Vec<String> {
    let mut w = Vec::new();
    let get = |k: &str| conf.and_then(|c| c.get(k)).map(str::to_string);
    if cfg.ng_compress {
        let rar = if cfg.ng_rar_path.trim().is_empty() { get("RAR_PATH") } else { Some(cfg.ng_rar_path.trim().to_string()) };
        match rar {
            None => w.push("Compression demandée mais aucun 7-Zip / RAR indiqué (RAR_PATH absent de ngPost.conf)".into()),
            Some(p) if cfg!(windows) && !Path::new(&native_path(&p)).is_file() => w.push(format!("7-Zip / RAR introuvable : {p}")),
            _ => {}
        }
    }
    if (cfg.ng_compress || cfg.ng_par2) && cfg.ng_tmp_dir.trim().is_empty() && get("TMP_DIR").is_none() {
        w.push("Aucun dossier temporaire (TMP_DIR) pour les archives et par2".into());
    }
    if let Some(e) = tmp_dir_problem(cfg, conf) {
        w.push(e);
    }
    // post intégré, mais le dossier de sortie est aussi le dossier auto / surveillé de NgPost :
    // NgPost ouvert à côté pourrait prendre les vidéos (et les supprimer après son post)
    if cfg.post_mode == "cli" {
        if let (Some(o), Some((wd, key))) = (cfg.output_dir(), conf.and_then(|c| c.watch_dir())) {
            let norm = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_lowercase();
            if !key.eq_ignore_ascii_case("inputDir") && norm(&o.display().to_string()) == norm(&wd) {
                w.push(format!("Le dossier de sortie est aussi le dossier « {key} » de NgPost ({wd}) : si NgPost est ouvert en surveillance (avec « supprimer après post »), il peut prendre les vidéos avant l'application. Ferme NgPost, ou choisis un autre dossier de sortie"));
            }
        }
    }
    if cfg.ng_par2 && !cfg.ng_par2_tool.is_empty() && find_par2_tool(cfg, &cfg.ng_par2_tool).is_none() {
        w.push(format!("{} introuvable : le par2 de NgPost sera utilisé (Réglages → Options du post)", par2_tool_label(&cfg.ng_par2_tool)));
    }
    if cfg.ng_par2 && !cfg.ng_compress {
        w.push("par2 sans compression : selon la version, NgPost ne crée le par2 qu'avec --compress".into());
    }
    if cfg.ng_gen_name && !cfg.ng_compress {
        w.push("Nom d'archive aléatoire sans compression : ignoré par NgPost".into());
    }
    if !cfg.post_obfuscate && get("obfuscate").map(|o| conf_obfuscates(&o)).unwrap_or(false) {
        w.push("ngPost.conf active déjà l'obfuscation (obfuscate = article) : elle s'appliquera quand même".into());
    }
    let groups = if cfg.ng_groups.trim().is_empty() { get("GROUPS") } else { Some(cfg.ng_groups.clone()) };
    if groups.is_none() && conf.is_some() {
        w.push("Aucun groupe Usenet (GROUPS) : NgPost utilisera son groupe par défaut".into());
    }
    w
}

/// Où ranger le NZB terminé : à côté du rapport MediaInfo (circuit indexers),
/// sinon dans le dossier des NZB, sinon à côté de la vidéo.
pub fn nzb_target(cfg: &Config, release: &str, video: &Path) -> PathBuf {
    if let Some(w) = waiting_dir(cfg) {
        return w.join(release).join(format!("{release}.nzb"));
    }
    let dir = if cfg.nzb_dir.trim().is_empty() { video.parent().map(Path::to_path_buf).unwrap_or_default() } else { PathBuf::from(cfg.nzb_dir.trim()) };
    dir.join(format!("{release}.nzb"))
}

/// Vidéos présentes dans le dossier de sortie (à poster).
pub fn list_unposted(cfg: &Config) -> Vec<PathBuf> {
    let Some(d) = cfg.output_dir() else { return Vec::new() };
    let mut v = prep::list_videos(&d, false);
    // releases en dossier (hors « _Postés », dossiers cachés…)
    if let Ok(rd) = fs::read_dir(&d) {
        for e in rd.flatten() {
            let p = e.path();
            let name = prep::file_name(&p);
            // séries (vidéos), ou release d'un autre contenu (jeu, logiciel… : tout dossier non vide)
            if p.is_dir() && !name.starts_with('_') && !name.starts_with('.') && !crate::content::files_of(&p).is_empty() {
                v.push(p);
            }
        }
    }
    v.sort();
    v
}

/// Pourcentage trouvé dans une ligne de sortie de NgPost (« 45% », « 45.3 % »).
pub fn parse_percent(line: &str) -> Option<u32> {
    let b = line.as_bytes();
    let mut last = None;
    for (k, &c) in b.iter().enumerate() {
        if c != b'%' {
            continue;
        }
        let mut j = k;
        while j > 0 && b[j - 1] == b' ' {
            j -= 1;
        }
        let end = j;
        while j > 0 && (b[j - 1].is_ascii_digit() || b[j - 1] == b'.' || b[j - 1] == b',') {
            j -= 1;
        }
        if j < end {
            if let Ok(v) = line[j..end].replace(',', ".").parse::<f32>() {
                if (0.0..=100.0).contains(&v) {
                    last = Some(v as u32);
                }
            }
        }
    }
    last
}

/// Avancement lu dans la sortie de NgPost.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    /// Compression, Par2, Envoi, Finalisation…
    pub phase: String,
    pub pct: u32,
    /// Articles envoyés / total (barre « (607 / 6022) »).
    pub done: u64,
    pub total: u64,
    /// Débit moyen annoncé par NgPost, en octets par seconde.
    pub avg_bps: Option<f64>,
    /// Débit actuel mesuré sur les 10 dernières secondes (envoi), octets par seconde.
    pub cur_bps: Option<f64>,
    /// Temps restant estimé pour l'étape en cours (compression, par2, envoi), en secondes.
    pub eta: Option<u64>,
}

/// Texte court de l'avancement (infobulle de la zone de notification, notifications).
pub fn progress_text(p: &Progress) -> String {
    let mut t = format!("{} {} %", p.phase, p.pct);
    if let Some(b) = p.cur_bps.filter(|b| *b > 0.0) {
        t.push_str(&format!(" · {}/s", prep::human_size(b as u64)));
    }
    if let Some(e) = p.eta {
        t.push_str(&format!(" · reste {}", fmt_secs_short(e)));
    }
    t
}

fn fmt_secs_short(s: u64) -> String {
    if s >= 3600 {
        format!("{}h{:02}", s / 3600, (s / 60) % 60)
    } else if s >= 60 {
        format!("{} min", s.div_ceil(60))
    } else {
        format!("{s} s")
    }
}

/// « (607 / 6022) » → (607, 6022).
pub fn parse_counts(line: &str) -> Option<(u64, u64)> {
    let mut rest = line;
    while let Some(k) = rest.find('(') {
        let inner = &rest[k + 1..];
        let end = inner.find(')').unwrap_or(inner.len());
        if let Some((a, b)) = inner[..end].split_once('/') {
            if let (Ok(a), Ok(b)) = (a.trim().parse::<u64>(), b.trim().parse::<u64>()) {
                if b > 0 && a <= b {
                    return Some((a, b));
                }
            }
        }
        rest = &inner[end.min(inner.len())..];
    }
    None
}

/// Débit après « speed: » ou « débit moyen: » (« 32.76 MB/s ») → octets par seconde.
pub fn parse_speed(line: &str) -> Option<f64> {
    let low = line.to_lowercase();
    let k = ["speed:", "débit moyen:", "debit moyen:", "débit:"].iter().find_map(|m| low.find(m).map(|k| k + m.len()))?;
    let rest = low[k..].trim_start();
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
    let v: f64 = num.replace(',', ".").parse().ok()?;
    let unit = rest[num.len()..].trim_start();
    let mult = if unit.starts_with("gb") || unit.starts_with("go") {
        1024.0 * 1024.0 * 1024.0
    } else if unit.starts_with("mb") || unit.starts_with("mo") {
        1024.0 * 1024.0
    } else if unit.starts_with("kb") || unit.starts_with("ko") {
        1024.0
    } else {
        1.0
    };
    Some(v * mult)
}

/// Lignes techniques de NgPost (ouverture / fermeture des connexions) : gardées dans le
/// fichier journal, masquées à l'écran.
pub fn is_noise(line: &str) -> bool {
    let l = line.to_lowercase();
    (l.contains("nntpcon") && (l.contains("no more articles") || l.contains("closing connection") || l.contains("connected") || l.contains("authenticated")))
        || l.ends_with("threads quit")
        || l.ends_with("connection thread done")
        || l.ends_with("threads stopped")
        || l.starts_with("all posters stopped")
}

/// Étape annoncée par une ligne de NgPost (compression, par2, envoi), si elle en parle.
pub fn phase_of(line: &str) -> Option<&'static str> {
    let l = line.to_lowercase();
    // les noms de fichiers (« Opening: xxx.7z.001 », « - xxx.par2 ») ne sont pas des étapes
    if l.starts_with("opening:") || l.starts_with("- ") || l.starts_with("+ ") {
        return None;
    }
    if l.contains("finishing") {
        Some("Finalisation")
    } else if l.contains("avg. speed") || l.contains(">>>") || l.contains("connexions disponibles") || l.contains("available connections") || l.contains("posting") || l.contains("nb article") {
        Some("Envoi")
    } else if l.contains("par2") || l.contains("parpar") || l.contains("multipar") || l.contains("redundan") || l.contains("reed solomon") || l.contains("recovery") {
        Some("Par2")
    } else if l.contains("compress") || l.contains("7-zip") || l.contains("creating archive") || l.contains("rar ") || l.contains("packing") {
        Some("Compression")
    } else {
        None
    }
}

/// Ligne interne : battement d'une seconde pendant la lecture de NgPost.
const TICK: &str = "\u{0}tick";

/// Rang d'une étape : on ne revient jamais en arrière (Compression → Par2 → Envoi → Finalisation).
pub fn phase_rank(p: &str) -> u8 {
    match p {
        "Compression" => 1,
        "Par2" => 2,
        "Envoi" => 3,
        "Finalisation" => 4,
        "Terminé" => 5,
        _ => 0,
    }
}

/// Gravité d'une ligne de NgPost : 2 = erreur, 1 = avertissement, 0 = normal.
/// « failed: 0 » n'est pas une erreur.
pub fn line_severity(line: &str) -> u8 {
    let l = line.to_lowercase();
    if ["error", "erreur", "fatal", "échec", "impossible", "abort"].iter().any(|k| l.contains(k)) {
        return 2;
    }
    if let Some(k) = l.find("fail") {
        let n: String = l[k..].chars().skip_while(|c| !c.is_ascii_digit() && *c != '\n').take_while(|c| c.is_ascii_digit()).collect();
        if n.parse::<u64>().map(|v| v > 0).unwrap_or(true) {
            return 2;
        }
    }
    if ["warn", "timeout", "reconnect", "retry"].iter().any(|k| l.contains(k)) {
        return 1;
    }
    0
}

/// Ce que contient un NZB (et s'il est complet).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NzbInfo {
    pub files: usize,
    pub segments: usize,
    pub bytes: u64,
    pub groups: Vec<String>,
    pub password: Option<String>,
    /// Sujets des premiers fichiers du NZB.
    pub subjects: Vec<String>,
}

fn xml_unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// Contrôle de l'obfuscation : (bon, explication).
pub fn obfuscation_report(cfg: &Config, info: &NzbInfo, release: &str) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    if cfg.post_obfuscate {
        out.push((true, "sur Usenet, le sujet de chaque article est remplacé par son identifiant (-x) : le post n'est retrouvable qu'avec le NZB".into()));
    } else {
        out.push((false, "obfuscation des articles désactivée : le post est visible en clair sur Usenet".into()));
    }
    if info.subjects.is_empty() {
        return out;
    }
    // le titre (avant l'année) ne doit pas apparaître dans les noms de fichiers postés
    let low_rel = release.to_lowercase();
    let title: String = low_rel.split('.').take_while(|t| !(t.len() == 4 && t.chars().all(|c| c.is_ascii_digit()))).collect::<Vec<_>>().join(".");
    let probe = if title.len() >= 4 { title } else { low_rel.clone() };
    match info.subjects.iter().find(|s| s.to_lowercase().contains(&probe)) {
        Some(s) => out.push((false, format!("le nom de la release apparaît dans le NZB ({s}) : active la compression avec nom d'archive aléatoire pour le masquer"))),
        None => out.push((true, format!("noms des fichiers postés aléatoires (ex. {})", info.subjects[0].chars().take(70).collect::<String>()))),
    }
    out
}

impl NzbInfo {
    pub fn summary(&self) -> String {
        let mut s = format!("{} fichier(s), {} articles, {}", self.files, self.segments, prep::human_size(self.bytes));
        if !self.groups.is_empty() {
            s.push_str(&format!(", {}", self.groups.join(", ")));
        }
        if self.password.is_some() {
            s.push_str(", mot de passe dans le NZB");
        }
        s
    }
}

/// Vérifie un NZB : XML terminé (`</nzb>`), au moins un fichier et des segments.
/// Un NZB en cours d'écriture par NgPost (en-tête seul) est refusé.
pub fn nzb_info(p: &Path) -> Result<NzbInfo, String> {
    let b = fs::read(p).map_err(|e| format!("lecture du NZB : {e}"))?;
    let t = decode_text(&b);
    let low = t.to_ascii_lowercase();
    if !low.contains("<nzb") {
        return Err("ce n'est pas un NZB".into());
    }
    if !low.trim_end().ends_with("</nzb>") {
        return Err("NZB incomplet (NgPost ne l'avait pas fini)".into());
    }
    let files = low.matches("<file ").count();
    if files == 0 {
        return Err("NZB vide : aucun fichier posté".into());
    }
    let segments = low.matches("<segment ").count();
    if segments == 0 {
        return Err("NZB sans article".into());
    }
    let mut info = NzbInfo { files, segments, ..Default::default() };
    let mut pos = 0;
    while let Some(k) = low[pos..].find("bytes=\"") {
        let st = pos + k + 7;
        let digits: String = low[st..].chars().take_while(|c| c.is_ascii_digit()).collect();
        info.bytes += digits.parse::<u64>().unwrap_or(0);
        pos = st;
    }
    let mut pos = 0;
    while let Some(k) = low[pos..].find("<group>") {
        let st = pos + k + 7;
        let Some(e) = low[st..].find("</group>") else { break };
        let g = t[st..st + e].trim().to_string();
        if !g.is_empty() && !info.groups.contains(&g) {
            info.groups.push(g);
        }
        pos = st + e;
    }
    let mut pos = 0;
    while info.subjects.len() < 5 {
        let Some(k) = low[pos..].find("subject=\"") else { break };
        let st = pos + k + 9;
        let Some(e) = low[st..].find('"') else { break };
        info.subjects.push(xml_unescape(&t[st..st + e]));
        pos = st + e;
    }
    if let Some(k) = low.find("type=\"password\"") {
        if let Some(gt) = low[k..].find('>') {
            let st = k + gt + 1;
            if let Some(e) = low[st..].find("</meta>") {
                let pw = t[st..st + e].trim().to_string();
                if !pw.is_empty() {
                    info.password = Some(pw);
                }
            }
        }
    }
    Ok(info)
}

/// Dossier des journaux de post (un fichier par post).
pub fn logs_dir() -> PathBuf {
    crate::logs::dir()
}

/// Résultat d'un post réussi.
#[derive(Clone, Debug)]
pub struct Posted {
    pub nzb: PathBuf,
    pub info: NzbInfo,
    pub log: Option<PathBuf>,
    pub secs: u64,
    /// Lignes de bilan de NgPost (taille, débit, articles en échec, archive…).
    pub summary: Vec<String>,
    /// Points à signaler sans que le post soit raté (code de sortie, articles manquants…).
    pub warnings: Vec<String>,
    /// Débit moyen final annoncé par NgPost (octets/s).
    pub avg_bps: Option<f64>,
    /// Début du post (horodatage).
    pub started: u64,
    /// Octets et articles postés.
    pub bytes: u64,
    pub articles: u64,
    /// Connexions utilisées (bilan de NgPost).
    pub connections: Option<u32>,
    /// Débit mesuré toutes les 10 s pendant l'envoi (horodatage, octets/s).
    pub samples: Vec<(u64, f64)>,
    /// Ce qu'est devenue la vidéo (supprimée, rangée dans _Postés…).
    pub video_after: String,
}

/// Vidéos de « _Postés » (dossier de sortie) et leur taille totale.
pub fn posted_videos(cfg: &Config) -> (Vec<PathBuf>, u64) {
    let Some(out) = cfg.output_dir() else { return (Vec::new(), 0) };
    let v = prep::list_videos(&out.join(POSTED_DIR), false);
    let size = v.iter().map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
    (v, size)
}

fn is_summary_line(l: &str) -> bool {
    let x = l.to_lowercase();
    if x.starts_with('[') && x.contains("] ") && x.contains('%') {
        return false; // barre de progression
    }
    ["size", "taille", "speed", "débit", "article", "fail", "error", "erreur", "rar", "archive", "pass", "nzb", "posted", "par2", "group", "warning"].iter().any(|k| x.contains(k))
}

/// Le NZB est-il prêt à être distribué ? Complet si la vérification est active,
/// sinon simplement présent, non vide et stable depuis quelques secondes.
pub fn nzb_ready(cfg: &Config, p: &Path) -> Result<(), String> {
    if cfg.post_verify {
        return nzb_info(p).map(|_| ());
    }
    let len = fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    if len == 0 {
        return Err("NZB vide".into());
    }
    if age(p) < Duration::from_secs(5) {
        return Err("NZB en cours d'écriture".into());
    }
    Ok(())
}

/// Poste une vidéo avec NgPost en ligne de commande et attend la fin.
/// `on_line` reçoit chaque ligne de NgPost (hors barre de progression),
/// `on_progress` l'étape en cours, son pourcentage, les articles et le débit.
/// Le NZB est produit dans un dossier temporaire puis rangé seulement s'il est complet.
/// Un NZB produit n'est jamais supprimé : en cas de problème il est gardé dans `logs`.
pub fn post_file(cfg: &Config, video: &Path, release: &str, on_line: &mut dyn FnMut(&str), on_progress: &mut dyn FnMut(&Progress)) -> Result<Posted, String> {
    // rien d'affiché ne contient un mot de passe (NgPost en mode debug détaille ses serveurs)
    let on_line = &mut |l: &str| on_line(&prep::redact(l));
    // déjà sur les indexers visés : pas de post pour eux (ou pas de post du tout)
    dupe_gate(cfg, release, video, on_line)?;
    // compression imposée par la catégorie de contenu, groupes selon le type de post
    let forced;
    let comp = read_plan(cfg, release).and_then(|p| p.compress).filter(|c| *c != cfg.ng_compress && !cfg.post_args.contains("{input}"));
    let (mut groups, mut kind) = groups_for(cfg, release);
    // groupes choisis à la main pour ce post (page Traitement) : ils priment
    let chosen = read_plan(cfg, release).map(|p| p.groups).unwrap_or_default();
    if !chosen.trim().is_empty() {
        groups = chosen.trim().to_string();
        kind = "choisis pour ce traitement".into();
    }
    // groupes réellement utilisés : ceux du type, sinon ceux par défaut, sinon ngPost.conf
    let used = if !groups.is_empty() {
        groups.clone()
    } else if !cfg.ng_groups.trim().is_empty() {
        cfg.ng_groups.trim().to_string()
    } else {
        read_conf(cfg).and_then(|c| c.get("GROUPS").map(|g| g.trim().to_string())).unwrap_or_default()
    };
    // groupes indexés par les indexers visés (lus dans leurs capacités) : ajoutés si réglé
    let content_cat = waiting_dir(cfg).and_then(|w| crate::content::read_cat(&w.join(release)));
    let marked = waiting_dir(cfg).map(|w| is_marked_series(&w.join(release))).unwrap_or(false);
    let family = group_family(content_cat.as_ref().map(|(k, _, n)| (k.as_str(), n.as_str())), marked, release);
    let targets = read_plan(cfg, release).unwrap_or_else(|| Plan::from_cfg(cfg)).resolve(cfg);
    let (fitted, fits) = fit_groups(cfg, &used, family, &targets, release, content_cat.as_ref().map(|(k, n, _)| (k.clone(), n.clone())));
    for f in &fits {
        if let Some((c, g)) = &f.rule {
            on_line(&format!("Groupes de {} pour sa catégorie {c} : {g}{}", f.idx, if f.added.is_some() { " (ajoutés au post)" } else { " (déjà dans le post)" }));
            continue;
        }
        match (&f.added, f.ok) {
            (Some(g), _) => on_line(&format!("Groupe {g} ajouté : {} n'indexe aucun des autres groupes du post", f.idx)),
            (None, false) => on_line(&format!("⚠ {} n'indexe aucun des groupes du post ({used}) : la release risque de ne pas y être trouvée{}", f.idx, if f.suggest.is_empty() { String::new() } else { format!(", ses groupes adaptés : {}", f.suggest.iter().take(4).cloned().collect::<Vec<_>>().join(", ")) })),
            _ => {}
        }
    }
    let used = if fits.iter().any(|f| f.added.is_some()) {
        groups = fitted.clone();
        fitted
    } else {
        used
    };
    let n_used = used.split(',').filter(|g| !g.trim().is_empty()).count();
    if cfg.group_policy == "each_post" && n_used >= 2 && !fits.iter().any(|f| f.added.is_some()) && chosen.trim().is_empty() {
        groups = crate::usenet::pick_group(&used);
        on_line(&format!("Groupe tiré au sort pour ce post ({kind}) parmi {n_used} : {groups}"));
    } else if !used.is_empty() {
        on_line(&format!("Groupes Usenet ({kind}{}) : {used}", if groups.is_empty() { ", par défaut" } else { "" }));
    }
    let cfg = if comp.is_some() || (!groups.is_empty() && groups != cfg.ng_groups.trim()) {
        let mut c = cfg.clone();
        if let Some(on) = comp {
            on_line(&format!("Catégorie de contenu : compression {}", if on { "activée" } else { "désactivée" }));
            c.ng_compress = on;
        }
        if !groups.is_empty() {
            c.ng_groups = groups;
        }
        forced = c;
        &forced
    } else {
        cfg
    };
    use std::io::{Read, Write};
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    let exe = exe_path(cfg);
    if !exe.is_file() {
        return Err(format!("NgPost introuvable : {}", exe.display()));
    }
    // une vidéo, ou le dossier d'une release de série
    if !video.is_file() && !video.is_dir() {
        return Err(format!("vidéo introuvable : {}", video.display()));
    }
    let inputs: Vec<PathBuf> = if video.is_dir() { post_inputs(cfg, video) } else { vec![video.to_path_buf()] };
    if inputs.is_empty() {
        return Err(format!("dossier vide : {}", video.display()));
    }
    let started = SystemTime::now();
    let stamp = prep::now_secs();
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let stage = crate::app_dir().join("tmp").join(format!("post-{stamp}-{}-{seq}", std::process::id()));
    fs::create_dir_all(&stage).map_err(|e| format!("dossier temporaire : {e}"))?;
    let tmp_nzb = stage.join(format!("{release}.nzb"));
    let ngc = read_conf(cfg);
    let mut conf = ngc.as_ref().map(|c| c.path.clone());
    // serveurs Usenet gérés dans l'application : copie de ngPost.conf avec nos [server]
    // (inutile pour un envoi en lots : chaque lot refait la sienne)
    let lots = video.is_dir() && cfg.ng_compress && cfg.ng_lots && space_problem(cfg, ngc.as_ref(), video).is_some();
    // (serveurs de ngPost.conf détaillés dans le journal, politique de groupes appliquée)
    if !lots {
        match crate::usenet::write_post_conf(cfg, conf.as_deref(), &stage, on_line) {
            Ok(Some(p)) => conf = Some(p),
            Ok(None) => {}
            Err(e) => {
                let _ = fs::remove_dir_all(&stage);
                return Err(e);
            }
        }
    }
    // outil par2 choisi (par2cmdline-turbo, ParPar)
    let mut par2_extra = Vec::new();
    if !lots {
        let (c, a) = par2_setup(cfg, conf.as_deref(), &stage, on_line);
        if c.is_some() {
            conf = c;
        }
        par2_extra = a;
    }
    let mut args = post_args_multi(cfg, conf.as_deref(), &inputs, &tmp_nzb);
    if !cfg.post_args.contains("{input}") {
        args.extend(par2_extra);
    }
    // série de centaines d'épisodes : la liste des fichiers ne tient pas dans la ligne de
    // commande de Windows, NgPost reçoit les dossiers (les fichiers gardent leurs noms)
    if video.is_dir() && inputs.iter().any(|i| i.is_dir()) && !cfg.post_args.contains("{input}") {
        let n = crate::content::files_of(video).len();
        if cfg.ng_compress && inputs.len() == 1 && inputs[0] == video {
            // compressé : le dossier de la release, sans lui-même comme dossier racine dans
            // l'archive (sinon : Release\Release\épisodes une fois extrait)
            let known = ngpost_help(cfg).map(|h| h.to_lowercase().contains("rar_no_root_folder")).unwrap_or(false);
            if known {
                args.push("--rar_no_root_folder".into());
                on_line(&format!("{n} fichiers : trop pour la ligne de commande de Windows, NgPost compresse le dossier de la release (--rar_no_root_folder : les épisodes arrivent directement dans « {release} », avec leurs noms)"));
                let seven = ngc.as_ref().and_then(|c| c.get("use_7z")).map(|v| matches!(v.trim().to_lowercase().as_str(), "true" | "1" | "on")).unwrap_or(false);
                if seven {
                    on_line("⚠ NgPost compresse avec 7-Zip : --rar_no_root_folder n'est garanti qu'avec RAR, l'archive pourrait contenir un dossier en plus");
                }
            } else {
                on_line(&format!("⚠ {n} fichiers : NgPost compresse le dossier de la release ; cette version de NgPost ne connaît pas --rar_no_root_folder, l'archive contiendra le dossier « {release} » (mets NgPost à jour)"));
            }
        } else {
            let dirs: Vec<String> = inputs.iter().filter(|i| i.is_dir()).map(|d| prep::file_name(d)).collect();
            on_line(&format!("{n} fichiers : trop pour la ligne de commande de Windows, NgPost reçoit le(s) dossier(s) {} et en poste tous les fichiers, avec leurs noms (celui qui télécharge reçoit « {release} » et ses épisodes)", dirs.join(", ")));
        }
    }
    // assez de place pour les archives et les par2 ? sinon : série compressée en plusieurs lots
    // (un seul NZB à la fin), ou refus immédiat sans lancer NgPost
    if let Some(e) = space_problem(cfg, ngc.as_ref(), video) {
        let _ = fs::remove_dir_all(&stage);
        if video.is_dir() && cfg.ng_compress && cfg.ng_lots {
            return post_in_lots(cfg, ngc.as_ref(), video, release, &e, on_line, on_progress);
        }
        return Err(e);
    }
    let art_size: f64 = if cfg.ng_article_size > 0 {
        cfg.ng_article_size as f64
    } else {
        ngc.as_ref().and_then(|c| c.get("article_size")).and_then(|v| v.trim().parse().ok()).unwrap_or(716_800.0)
    };

    // journal complet du post
    let log_path = {
        let l = crate::history::local_time(stamp);
        let safe: String = release.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect();
        logs_dir().join(format!("{:04}-{:02}-{:02}_{:02}h{:02}m{:02}_{safe}.log", l.year, l.month, l.day, l.hour, l.minute, l.second))
    };
    let mut log = fs::create_dir_all(logs_dir()).ok().and_then(|_| fs::File::create(&log_path).ok());
    let has_log = log.is_some();
    // chaque ligne du journal porte l'heure (les étapes de NgPost ne sont pas toutes datées)
    let mut write_log = |s: &str| {
        if let Some(f) = log.as_mut() {
            let s = prep::redact(s);
            let l = crate::history::local_time(prep::now_secs());
            let ms = SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_millis()).unwrap_or(0);
            let _ = writeln!(f, "[{:02}:{:02}:{:02}.{ms:03}] {s}", l.hour, l.minute, l.second);
        }
    };
    write_log(&format!("# {}, post de {release}", crate::APP_NAME));
    write_log(&format!("# vidéo : {}", video.display()));
    let cmdline = command_line(&exe, &args);
    // réglages effectifs du post (journal détaillé ; toujours dans le fichier journal)
    for l in post_settings(cfg, ngc.as_ref(), art_size) {
        write_log(&format!("# {l}"));
        if cfg.log_detail {
            on_line(&l);
        }
    }
    write_log(&format!("> {cmdline}"));
    on_line(&format!("> {cmdline}"));

    let mut cmd = Command::new(&exe);
    cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(d) = exe.parent() {
        cmd.current_dir(d);
    }
    hidden(&mut cmd);
    // archives temporaires présentes avant le post (pour ne retirer que les nôtres en cas d'arrêt)
    let tmp_root = tmp_dir(cfg, ngc.as_ref());
    let tmp_before: std::collections::HashSet<PathBuf> = fs::read_dir(&tmp_root).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    let gen = POST_GEN.load(std::sync::atomic::Ordering::SeqCst);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::remove_dir_all(&stage);
            return Err(format!("lancement de NgPost impossible : {e}"));
        }
    };
    // arrêt demandé : NgPost et ses processus (rar, 7z, par2) sont arrêtés
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = {
        let (f, k, pid) = (finished.clone(), killed.clone(), child.id());
        std::thread::spawn(move || {
            while !f.load(std::sync::atomic::Ordering::SeqCst) {
                if POST_GEN.load(std::sync::atomic::Ordering::SeqCst) != gen {
                    k.store(true, std::sync::atomic::Ordering::SeqCst);
                    kill_tree(pid);
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        })
    };
    // stderr lu dans un fil à part, ses lignes sont mêlées au fil de l'eau
    let err_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let err_pipe = child.stderr.take();
    let el = err_lines.clone();
    let err_handle = std::thread::spawn(move || {
        if let Some(mut e) = err_pipe {
            let mut buf = [0u8; 2048];
            let mut cur: Vec<u8> = Vec::new();
            while let Ok(n) = e.read(&mut buf) {
                if n == 0 {
                    break;
                }
                for &c in &buf[..n] {
                    if c == b'\n' || c == b'\r' {
                        let l = decode_text(&cur).trim().to_string();
                        cur.clear();
                        if !l.is_empty() {
                            if let Ok(mut v) = el.lock() {
                                v.push(l);
                            }
                        }
                    } else {
                        cur.push(c);
                    }
                }
            }
            let l = decode_text(&cur).trim().to_string();
            if !l.is_empty() {
                if let Ok(mut v) = el.lock() {
                    v.push(l);
                }
            }
        }
    });
    let mut tail: Vec<String> = Vec::new();
    let mut summary: Vec<String> = Vec::new();
    let mut phase = "Préparation";
    let mut last_logged_pct: Option<(String, u32)> = None;
    let mut last_log_t = std::time::Instant::now();
    let mut noise = 0usize;
    let mut total_articles = 0u64;
    let mut final_bps: Option<f64> = None;
    let mut connections: Option<u32> = None;
    let mut samples: Vec<(u64, f64)> = Vec::new();
    let mut last_sample: Option<(std::time::Instant, u64)> = None;
    // estimation du temps restant : début de l'étape (instant, % de départ), fenêtre d'articles
    let mut phase_t0: Option<(std::time::Instant, u32)> = None;
    let mut prev_pct = 0u32;
    // taille à compresser (avancement de la compression d'après l'archive écrite) et par2
    let input_size = path_size(video).max(1);
    let (mut par2_block, mut par2_total, mut par2_written) = (0u64, 0u64, 0u64);
    let mut window: std::collections::VecDeque<(std::time::Instant, u64)> = std::collections::VecDeque::new();
    let show_noise = cfg.ng_debug || cfg.log_detail;
    // erreurs reconnues dans la sortie de NgPost (message clair en cas d'échec)
    let mut fail = FailSigns::default();
    {
        let mut handle = |line: String, write_log: &mut dyn FnMut(&str)| {
            if line.is_empty() {
                return;
            }
            if line != TICK {
                // « Ignoring non-existent source file » répété pour chaque volume : 3 lignes
                // à l'écran, toutes dans le fichier journal
                if fail.note(&line) > 3 {
                    write_log(&line);
                    return;
                }
            }
            // battement : compression silencieuse, avancement = archives écrites / taille à compresser
            if line == TICK {
                if cfg.ng_compress && phase_rank(phase) <= 1 {
                    let written: u64 = fs::read_dir(&tmp_root)
                        .map(|r| r.flatten().map(|e| e.path()).filter(|p| !tmp_before.contains(p)).map(|p| path_size(&p)).sum())
                        .unwrap_or(0);
                    if written > 0 {
                        if phase != "Compression" {
                            phase = "Compression";
                            phase_t0 = None;
                        }
                        let pct = ((written as u128 * 100 / input_size as u128) as u32).min(99);
                        let now_i = std::time::Instant::now();
                        let (t0, p0) = *phase_t0.get_or_insert((now_i, pct));
                        let el = now_i.duration_since(t0).as_secs_f64();
                        let eta = (pct > p0 && el >= 3.0).then(|| (el * (100 - pct) as f64 / (pct - p0) as f64) as u64);
                        on_progress(&Progress { phase: phase.to_string(), pct, eta, ..Default::default() });
                        if last_log_t.elapsed() >= Duration::from_secs(60) {
                            write_log(&format!("(compression : {pct} %, {} écrits{})", prep::human_size(written), eta.map(|e| format!(", reste {}", fmt_secs_short(e))).unwrap_or_default()));
                            last_log_t = std::time::Instant::now();
                        }
                    }
                }
                return;
            }
            let named = phase_of(&line).filter(|p| phase_rank(p) >= phase_rank(phase));
            if let Some(p) = named {
                if p != phase {
                    phase_t0 = None;
                    window.clear();
                }
                phase = p;
            }
            // par2 : taille des données de réparation, puis écriture (fin de l'étape)
            let low = line.to_lowercase();
            if phase == "Par2" {
                let num = |s: &str| s.split(|c: char| !c.is_ascii_digit()).find(|w| !w.is_empty()).and_then(|w| w.parse::<u64>().ok()).unwrap_or(0);
                if let Some(v) = low.strip_prefix("block size:") {
                    par2_block = num(v);
                } else if let Some(v) = low.strip_prefix("recovery block count:") {
                    par2_total = num(v) * par2_block;
                } else if low.starts_with("wrote ") && low.contains("bytes") && par2_total > 0 {
                    par2_written += num(&low);
                    let pct = (90 + par2_written * 10 / par2_total).min(99) as u32;
                    let now_i = std::time::Instant::now();
                    let (t0, p0) = *phase_t0.get_or_insert((now_i, pct));
                    let el = now_i.duration_since(t0).as_secs_f64();
                    let eta = (pct > p0 && el >= 3.0).then(|| (el * (100 - pct) as f64 / (pct - p0) as f64) as u64);
                    prev_pct = pct;
                    on_progress(&Progress { phase: phase.to_string(), pct, eta, ..Default::default() });
                    write_log(&line);
                    return;
                }
            }
            // par2 : le calcul (« 10.0% ») compte pour 90 % de l'étape, l'écriture pour le reste
            let parsed = parse_percent(&line).map(|p| if phase == "Par2" && parse_counts(&line).is_none() { p * 9 / 10 } else { p });
            if let Some(pct) = parsed {
                // pourcentage qui repart de zéro sans nouvelle étape annoncée : après la
                // compression vient le calcul des par2 (l'outil par2 ne se nomme pas toujours)
                if named.is_none() && prev_pct >= 90 && pct <= 10 {
                    if phase == "Compression" && cfg.ng_par2 {
                        phase = "Par2";
                    }
                    phase_t0 = None;
                }
                prev_pct = pct;
                let now_i = std::time::Instant::now();
                let (t0, p0) = *phase_t0.get_or_insert((now_i, pct));
                let counts = parse_counts(&line);
                if let Some((done, t)) = counts {
                    total_articles = total_articles.max(t);
                    // débit mesuré toutes les 10 s (articles envoyés × taille d'un article)
                    let now = std::time::Instant::now();
                    match last_sample {
                        None => last_sample = Some((now, done)),
                        Some((t0, d0)) => {
                            let dt = now.duration_since(t0).as_secs_f64();
                            if dt >= 10.0 {
                                samples.push((prep::now_secs(), done.saturating_sub(d0) as f64 * art_size / dt));
                                last_sample = Some((now, done));
                            }
                        }
                    }
                }
                // envoi : débit sur les 10 dernières secondes ; autres étapes : au rythme du %
                let mut cur_bps = None;
                let mut eta = None;
                if let Some((done, t)) = counts {
                    window.push_back((now_i, done));
                    while window.len() > 2 && now_i.duration_since(window.front().map(|w| w.0).unwrap_or(now_i)).as_secs_f64() > 10.0 {
                        window.pop_front();
                    }
                    if let (Some(a), Some(b)) = (window.front(), window.back()) {
                        let dt = b.0.duration_since(a.0).as_secs_f64();
                        if dt >= 1.0 {
                            let bps = b.1.saturating_sub(a.1) as f64 * art_size / dt;
                            cur_bps = Some(bps);
                            if bps > 0.0 {
                                eta = Some((t.saturating_sub(done) as f64 * art_size / bps) as u64);
                            }
                        }
                    }
                }
                if eta.is_none() && pct > p0 && pct < 100 {
                    let el = now_i.duration_since(t0).as_secs_f64();
                    if el >= 3.0 {
                        eta = Some((el * (100 - pct) as f64 / (pct - p0) as f64) as u64);
                    }
                }
                let prog = Progress {
                    phase: phase.to_string(),
                    pct,
                    done: counts.map(|c| c.0).unwrap_or(0),
                    total: counts.map(|c| c.1).unwrap_or(0),
                    avg_bps: parse_speed(&line),
                    cur_bps,
                    eta,
                };
                on_progress(&prog);
                // barre de progression : une ligne tous les 10 %, et au moins une par minute,
                // avec le débit et le temps restant mesurés ici
                let log_it = match &last_logged_pct {
                    Some((ph, last)) => ph != phase || pct >= last + 10 || (pct == 100 && *last != 100) || last_log_t.elapsed() >= Duration::from_secs(60),
                    None => true,
                };
                if log_it {
                    let extra = match (prog.cur_bps, prog.eta) {
                        (Some(b), Some(e)) => format!("   [débit actuel {}/s, reste {}]", prep::human_size(b as u64), fmt_secs_short(e)),
                        (None, Some(e)) => format!("   [{} : reste {}]", phase, fmt_secs_short(e)),
                        _ => String::new(),
                    };
                    write_log(&format!("{line}{extra}"));
                    last_logged_pct = Some((phase.to_string(), pct));
                    last_log_t = std::time::Instant::now();
                }
                return;
            }
            write_log(&line);
            if is_noise(&line) && !show_noise {
                noise += 1;
                return;
            }
            if line.to_lowercase().contains("débit moyen") || line.to_lowercase().contains("average speed") {
                final_bps = parse_speed(&line).or(final_bps);
                connections = crate::stats::parse_connections(&line).or(connections);
            }
            on_line(&line);
            if is_summary_line(&line) {
                summary.push(line.clone());
                if summary.len() > 16 {
                    summary.remove(0);
                }
            }
            tail.push(line);
            if tail.len() > 8 {
                tail.remove(0);
            }
        };
        let drain_err = |handle: &mut dyn FnMut(String, &mut dyn FnMut(&str)), write_log: &mut dyn FnMut(&str)| {
            let lines: Vec<String> = err_lines.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
            for l in lines {
                handle(l, write_log);
            }
        };
        if let Some(mut out) = child.stdout.take() {
            // lignes lues dans un fil ; ici, une ligne ou un battement par seconde (7-Zip
            // n'écrit rien pendant qu'il compresse : on mesure alors l'archive qui grossit)
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            let reader = std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let mut cur: Vec<u8> = Vec::new();
                while let Ok(n) = out.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    for &c in &buf[..n] {
                        if c == b'\n' || c == b'\r' || c == 8 {
                            let l = decode_text(&cur).trim().to_string();
                            cur.clear();
                            if !l.is_empty() && tx.send(l).is_err() {
                                return;
                            }
                        } else {
                            cur.push(c);
                        }
                    }
                }
                let l = decode_text(&cur).trim().to_string();
                if !l.is_empty() {
                    let _ = tx.send(l);
                }
            });
            loop {
                match rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(l) => handle(l, &mut write_log),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => handle(TICK.to_string(), &mut write_log),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
                drain_err(&mut handle, &mut write_log);
            }
            let _ = reader.join();
        }
        let _ = err_handle.join();
        drain_err(&mut handle, &mut write_log);
    }
    if noise > 0 {
        on_line(&format!("({noise} lignes techniques sur les connexions, masquées ici : voir le fichier journal)"));
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    finished.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = watcher.join();
    let secs = started.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    if killed.load(std::sync::atomic::Ordering::SeqCst) {
        let freed = clean_tmp(&tmp_root, &tmp_before);
        write_log(&format!("# arrêté à la demande ; archives temporaires retirées : {freed}"));
        let _ = fs::remove_dir_all(&stage);
        return Err(format!("{POST_CANCELLED} (au bout de {}) ; rien n'a été rangé, la vidéo reste à poster", fmt_secs(secs)));
    }
    write_log(&format!("# fin : code {}, {} s", status.code().map(|c| c.to_string()).unwrap_or_else(|| "inconnu".into()), secs));

    // le NZB attendu (ou, à défaut, le plus récent du dossier temporaire)
    let mut produced = tmp_nzb.clone();
    if !produced.is_file() {
        if let Some(p) = fs::read_dir(&stage).ok().and_then(|rd| {
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("nzb")).unwrap_or(false)).max_by_key(|p| mtime_secs(p))
        }) {
            produced = p;
        }
    }
    // NgPost peut finir d'écrire juste après sa sortie : on laisse quelques secondes
    let mut check = nzb_info(&produced);
    for _ in 0..15 {
        if check.is_ok() || !produced.is_file() {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
        check = nzb_info(&produced);
    }
    let log_note = if has_log { format!(", journal : {}", log_path.display()) } else { String::new() };
    let why = tail.iter().rev().take(3).rev().cloned().collect::<Vec<_>>().join(" | ");
    let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "inconnu".into());
    // un NZB produit est toujours gardé : jamais perdu, même refusé
    let keep_bad = |produced: &Path, write_log: &mut dyn FnMut(&str)| {
        let kept = logs_dir().join(format!("{release}.nzb"));
        let _ = fs::remove_file(&kept);
        if move_path(produced, &kept).is_ok() {
            write_log(&format!("# NZB conservé : {}", kept.display()));
            format!(", NZB conservé : {}", kept.display())
        } else {
            String::new()
        }
    };
    let mut warnings: Vec<String> = Vec::new();
    let info = match check {
        Ok(i) => i,
        Err(e) if !produced.is_file() => {
            write_log(&format!("# pas de NZB : {e}"));
            let _ = fs::remove_dir_all(&stage);
            let cause = fail.explain().map(|c| format!("{c}. ")).unwrap_or_default();
            let kept = kept_note(video);
            write_log(&format!("# {kept}"));
            return Err(format!("Rien n'a été posté (NgPost, code {code}). {cause}{kept}. Dernières lignes : {why}{log_note}"));
        }
        Err(e) if cfg.post_verify => {
            write_log(&format!("# NZB refusé : {e}"));
            let kept = keep_bad(&produced, &mut write_log);
            let _ = fs::remove_dir_all(&stage);
            return Err(format!("{e} (code {code}) : {why}{log_note}{kept}. {}", kept_note(video)));
        }
        Err(e) => {
            // vérification désactivée : on garde le NZB tel quel, avec un avertissement
            warnings.push(format!("NZB non vérifié ({e})"));
            NzbInfo::default()
        }
    };
    if !status.success() {
        // NgPost peut quitter avec un code ≠ 0 alors que tout est posté : le NZB fait foi
        warnings.push(format!("NgPost a quitté avec le code {code}, mais le NZB est complet : post considéré comme réussi"));
    }
    if cfg.post_verify && total_articles > 0 && (info.segments as u64) < total_articles {
        warnings.push(format!("{} article(s) absents du NZB ({} sur {})", total_articles - info.segments as u64, info.segments, total_articles));
    }
    for w in &warnings {
        write_log(&format!("# attention : {w}"));
    }
    write_log(&if info.files > 0 { format!("# NZB complet : {}", info.summary()) } else { "# NZB non vérifié".to_string() });
    // groupes où la release a réellement été postée (lus dans le NZB)
    if !info.groups.is_empty() {
        on_line(&format!("Posté dans {} groupe(s) Usenet : {}", info.groups.len(), info.groups.join(", ")));
    }
    on_progress(&Progress { phase: "Terminé".into(), pct: 100, done: total_articles, total: total_articles, avg_bps: final_bps, cur_bps: None, eta: None });
    let nzb = nzb_target(cfg, release, video);
    if let Some(d) = nzb.parent() {
        fs::create_dir_all(d).map_err(|e| format!("dossier du NZB : {e}"))?;
    }
    let _ = fs::remove_file(&nzb);
    move_path(&produced, &nzb).map_err(|e| format!("rangement du NZB : {e}"))?;
    let _ = fs::remove_dir_all(&stage);
    write_log(&format!("# NZB rangé : {}", nzb.display()));
    // la vidéo postée : seulement si elle est dans le dossier de sortie (jamais un original ailleurs)
    let verified = info.files > 0 && !warnings.iter().any(|w| w.contains("absents"));
    let video_after = after_post(cfg, video, verified, &mut warnings);
    write_log(&format!("# vidéo : {video_after}"));
    let articles = if info.segments > 0 { info.segments as u64 } else { total_articles };
    let bytes = if info.bytes > 0 { info.bytes } else { (total_articles as f64 * art_size) as u64 };
    Ok(Posted { nzb, info, log: has_log.then_some(log_path), secs, summary, warnings, avg_bps: final_bps, started: stamp, bytes, articles, connections, samples, video_after })
}

/// Indices d'échec relevés dans la sortie de NgPost.
#[derive(Default)]
struct FailSigns {
    par2_missing: usize,
    par2_err: bool,
    compress_err: bool,
    no_server: bool,
    auth: bool,
}

impl FailSigns {
    /// Note une ligne ; renvoie le nombre de « fichier source absent » vus (pour les replier).
    fn note(&mut self, line: &str) -> usize {
        let l = line.to_lowercase();
        if l.contains("non-existent source file") {
            self.par2_missing += 1;
            return self.par2_missing;
        }
        if l.contains("error during par2") || (l.contains("erreur") && l.contains("par2")) {
            self.par2_err = true;
        }
        if l.contains("error during compression") || (l.contains("erreur") && l.contains("compression")) {
            self.compress_err = true;
        }
        if l.contains("no server") || l.contains("aucun serveur") || l.contains("host not found") || l.contains("connection refused") {
            self.no_server = true;
        }
        if l.contains("authentication") || l.contains("authentification") {
            self.auth = true;
        }
        0
    }

    /// Cause probable, en clair.
    fn explain(&self) -> Option<String> {
        if self.par2_missing > 0 {
            return Some(format!(
                "Échec de la création des par2 : l'outil par2 n'a pas retrouvé les {} archive(s) créées par NgPost (chemin du dossier temporaire mal compris). Corrigé à partir de la 3.23 (dossier temporaire passé à NgPost en barres « / ») ; si ça revient, essaie ParPar ou un autre dossier temporaire (Réglages → Options du post)",
                self.par2_missing
            ));
        }
        if self.par2_err {
            return Some("Échec de la création des par2 (voir le journal pour le message de l'outil par2)".into());
        }
        if self.compress_err {
            return Some("Échec de la compression (7-Zip / RAR) : place libre, chemin de 7-Zip ou fichier verrouillé ?".into());
        }
        if self.auth {
            return Some("Le serveur Usenet a refusé l'identification (identifiant ou mot de passe du fournisseur)".into());
        }
        if self.no_server {
            return Some("Aucun serveur Usenet joignable".into());
        }
        None
    }
}

/// Ce qu'est devenue la vidéo après un échec : jamais touchée par l'application.
fn kept_note(video: &Path) -> String {
    if video.exists() {
        format!("Rien n'a été supprimé ni déplacé : la vidéo est toujours dans {}", video.display())
    } else {
        format!(
            "⚠ La vidéo n'est plus à {} : l'application n'y a pas touché ; un autre programme l'a déplacée ou supprimée pendant le post (NgPost ouvert en surveillance de ce dossier avec « supprimer après post » ?)",
            video.display()
        )
    }
}

/// Aide de NgPost (`--help`), pour vérifier les options disponibles.
pub fn ngpost_help(cfg: &Config) -> Result<String, String> {
    let exe = exe_path(cfg);
    if !exe.is_file() {
        return Err(format!("NgPost introuvable : {}", exe.display()));
    }
    let mut cmd = Command::new(&exe);
    cmd.arg("--help").stdin(std::process::Stdio::null());
    if let Some(d) = exe.parent() {
        cmd.current_dir(d);
    }
    hidden(&mut cmd);
    let out = cmd.output().map_err(|e| e.to_string())?;
    let mut s = decode_text(&out.stdout);
    s.push_str(&decode_text(&out.stderr));
    if s.trim().is_empty() {
        Err("NgPost n'a rien affiché pour --help".into())
    } else {
        Ok(s)
    }
}

// ------------------------------------------------------------ plan par release

/// Fichier caché du dossier d'attente : ce qu'il faut faire de la release (jamais copié chez les indexers).
pub const PLAN_FILE: &str = ".nzbff-plan";

/// Action et indexers visés pour une release.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub action: String,
    /// Vide = tous les indexers actifs.
    pub targets: Vec<String>,
    /// Déposer le NZB par API sur les indexers visés qui en ont une.
    pub api: bool,
    /// Compression imposée par la catégorie de contenu (None : options du post).
    pub compress: Option<bool>,
    /// Envoi forcé (« Envoyer quand même », anti-doublon coupé) : remplace chez l'indexer une
    /// release déjà uploadée sous le même nom.
    pub redeposit: bool,
    /// Catégorie choisie à la main par indexer : (indexer, id).
    pub cats: Vec<(String, String)>,
    /// Groupes Usenet choisis à la main pour ce post (vide : automatiques).
    pub groups: String,
}

impl Plan {
    pub fn from_cfg(cfg: &Config) -> Plan {
        Plan { action: cfg.action.clone(), targets: cfg.targets.clone(), api: cfg.api_upload, compress: None, redeposit: !cfg.skip_duplicates, cats: cfg.cat_force.clone(), groups: cfg.groups_force.clone() }
    }
    /// Indexers actifs visés (tous si aucun choix valable).
    pub fn resolve(&self, cfg: &Config) -> Vec<String> {
        let act = active_indexers(cfg);
        let t: Vec<String> = act.iter().filter(|a| self.targets.iter().any(|x| x.trim().eq_ignore_ascii_case(a))).cloned().collect();
        if t.is_empty() {
            act
        } else {
            t
        }
    }
    pub fn auto(&self) -> bool {
        self.action == "auto"
    }
}

/// Enregistre le plan d'une release dans son dossier d'attente (sans effet sans dossier racine).
pub fn write_plan(cfg: &Config, rel: &str, plan: &Plan) -> Result<(), String> {
    let Some(w) = waiting_dir(cfg) else { return Ok(()) };
    let d = w.join(rel);
    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let comp = match plan.compress {
        Some(true) => "compress=on\n",
        Some(false) => "compress=off\n",
        None => "",
    };
    if plan.cats.is_empty() {
        let _ = fs::remove_file(d.join(CATID_FILE));
    } else {
        let t: String = plan.cats.iter().map(|(i, c)| format!("{}\t{}\n", i.trim(), c.trim())).collect();
        let _ = fs::write(d.join(CATID_FILE), t);
    }
    let groups = if plan.groups.trim().is_empty() { String::new() } else { format!("groups={}\n", plan.groups.trim().replace(['\r', '\n', ' '], "")) };
    fs::write(d.join(PLAN_FILE), format!("action={}\ntargets={}\napi={}\nredeposit={}\n{comp}{groups}", plan.action, plan.targets.join("|"), plan.api, plan.redeposit)).map_err(|e| e.to_string())
}

/// Catégories choisies à la main par indexer (suit la release jusque dans les dossiers des
/// indexers).
pub const CATID_FILE: &str = ".nzbff-catid";

pub fn read_cat_force(dir: &Path) -> Vec<(String, String)> {
    fs::read_to_string(dir.join(CATID_FILE))
        .map(|t| t.lines().filter_map(|l| l.split_once('\t')).map(|(i, c)| (i.trim().to_string(), c.trim().to_string())).filter(|(i, c)| !i.is_empty() && !c.is_empty()).collect())
        .unwrap_or_default()
}

fn parse_plan(text: &str) -> Plan {
    let mut p = Plan { action: "post".into(), targets: Vec::new(), api: false, compress: None, redeposit: false, cats: Vec::new(), groups: String::new() };
    for l in text.lines() {
        match l.split_once('=') {
            Some(("action", v)) if !v.trim().is_empty() => p.action = v.trim().to_string(),
            Some(("targets", v)) => p.targets = v.split('|').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect(),
            Some(("api", v)) => p.api = v.trim() == "true",
            Some(("compress", v)) => p.compress = Some(v.trim() == "on"),
            Some(("redeposit", v)) => p.redeposit = v.trim() == "true",
            Some(("groups", v)) => p.groups = v.trim().to_string(),
            _ => {}
        }
    }
    p
}

pub fn read_plan(cfg: &Config, rel: &str) -> Option<Plan> {
    let f = waiting_dir(cfg)?.join(rel).join(PLAN_FILE);
    fs::read_to_string(f).ok().map(|t| parse_plan(&t))
}

// ------------------------------------------------------------ arborescence
//
//  <racine>\_En attente du NZB\<release>\      rapport MediaInfo, en attente du NZB de NgPost
//  <racine>\<Indexer>\<release>\               NZB + MediaInfo à déposer sur cet indexer
//  <racine>\<Indexer>\_Déjà uploadés\<release>\
//  <racine>\<Indexer>\_Ignorés\<release>\      pas pour cet indexer

pub const WAITING_DIR: &str = "_En attente du NZB";
pub const UPLOADED_DIR: &str = "_Déjà uploadés";
/// Releases à déposer sur l'indexer (NZB + rapport).
pub const PENDING_DIR: &str = "_À uploader";
pub const SKIPPED_DIR: &str = "_Ignorés";
fn notes() -> &'static std::sync::Mutex<Vec<String>> {
    static N: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    N.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

fn note(m: String) {
    if let Ok(mut n) = notes().lock() {
        n.push(m);
    }
}

/// Messages de la distribution à afficher (ancienne copie mise de côté…), vidés à la lecture.
pub fn take_notes() -> Vec<String> {
    notes().lock().map(|mut n| std::mem::take(&mut *n)).unwrap_or_default()
}

/// Fichier caché d'une release préparée comme série (page Série, dossier de série de
/// l'inbox) : sa catégorie et ses groupes sont ceux des séries, quel que soit son nom.
pub const KIND_FILE: &str = ".nzbff-type";
/// Origine du NFO d'une release : « origine:film.nfo », « bdinfo », « mediainfo ».
pub const NFO_FILE: &str = ".nzbff-nfo";
/// Contenu posté de la release (chemin relatif, taille), pour le journal des dépôts.
pub const CONTENT_FILE: &str = ".nzbff-contenu";

/// Note l'origine du NFO d'une release en attente.
pub fn mark_nfo(cfg: &Config, release: &str, origin: &str) {
    if let Some(w) = waiting_dir(cfg) {
        let _ = fs::write(w.join(release).join(NFO_FILE), origin);
    }
}

/// Origine du NFO d'un dossier de release (vide : inconnue).
pub fn nfo_origin(dir: &Path) -> String {
    fs::read_to_string(dir.join(NFO_FILE)).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Liste ce que contient la release postée (fichier vidéo, ou dossier) : gardée à côté du NZB
/// pour dire dans le journal ce qui a été déposé.
pub fn write_content_list(cfg: &Config, release: &str, posted: &Path) {
    let Some(w) = waiting_dir(cfg) else { return };
    let d = w.join(release);
    if fs::create_dir_all(&d).is_err() {
        return;
    }
    let files = crate::content::files_of(posted);
    let s: String = files.iter().map(|(p, rel)| format!("{}\t{}\n", rel.display().to_string().replace('\\', "/"), fs::metadata(p).map(|m| m.len()).unwrap_or(0))).collect();
    let _ = fs::write(d.join(CONTENT_FILE), s);
}

/// Contenu posté d'une release : (chemin relatif, taille).
pub fn read_content_list(dir: &Path) -> Vec<(String, u64)> {
    fs::read_to_string(dir.join(CONTENT_FILE))
        .map(|t| t.lines().filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_string(), b.trim().parse().unwrap_or(0)))).collect())
        .unwrap_or_default()
}

/// Ce que le NZB décrit : nombre de fichiers postés et poids total.
/// Groupes Usenet listés dans un NZB (sans doublon, dans l'ordre).
pub fn nzb_groups(nzb: &Path) -> Option<Vec<String>> {
    let t = decode_text(&fs::read(nzb).ok()?);
    let mut v: Vec<String> = Vec::new();
    let mut rest = t.as_str();
    while let Some(i) = rest.find("<group>") {
        let after = &rest[i + 7..];
        let Some(e) = after.find("</group>") else { break };
        let g = after[..e].trim();
        if !g.is_empty() && !v.iter().any(|x| x == g) {
            v.push(g.to_string());
        }
        rest = &after[e..];
    }
    Some(v)
}

pub fn nzb_summary(nzb: &Path) -> Option<(usize, u64)> {
    let t = fs::read_to_string(nzb).ok()?;
    let files = t.matches("<file ").count();
    let mut bytes = 0u64;
    let mut rest = t.as_str();
    while let Some(i) = rest.find("bytes=\"") {
        let after = &rest[i + 7..];
        let end = after.find('"').unwrap_or(0);
        bytes += after[..end].parse::<u64>().unwrap_or(0);
        rest = &after[end..];
    }
    Some((files, bytes))
}

/// Marque une release comme série (dossier d'attente).
pub fn mark_series(cfg: &Config, rel: &str) {
    if let Some(w) = waiting_dir(cfg) {
        let d = w.join(rel);
        if fs::create_dir_all(&d).is_ok() {
            let _ = fs::write(d.join(KIND_FILE), "serie\n");
        }
    }
}

/// Release marquée comme série ?
pub fn is_marked_series(dir: &Path) -> bool {
    fs::read_to_string(dir.join(KIND_FILE)).map(|t| t.trim() == "serie").unwrap_or(false)
}

/// Anciennes copies d'une release reposté sous le même nom (jamais déposées à sa place).
pub const REPLACED_DIR: &str = "_Remplacés";

/// Nom de dossier valide pour Windows à partir d'un nom d'indexer.
pub fn folder_name(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    s.trim_end_matches(['.', ' ']).to_string()
}

pub fn root(cfg: &Config) -> Option<PathBuf> {
    let r = cfg.indexer_dir.trim();
    (!r.is_empty()).then(|| PathBuf::from(r))
}

pub fn waiting_dir(cfg: &Config) -> Option<PathBuf> {
    root(cfg).map(|r| r.join(WAITING_DIR))
}

pub fn indexer_dir(cfg: &Config, name: &str) -> Option<PathBuf> {
    let f = folder_name(name);
    if f.is_empty() {
        return None;
    }
    root(cfg).map(|r| r.join(f))
}

pub fn active_indexers(cfg: &Config) -> Vec<String> {
    cfg.indexers.iter().filter(|i| i.active && !folder_name(&i.name).is_empty()).map(|i| i.name.trim().to_string()).collect()
}

/// Crée la racine, le dossier d'attente et un dossier par indexer actif.
pub fn ensure_dirs(cfg: &Config) -> Result<(), String> {
    let Some(r) = root(cfg) else { return Ok(()) };
    fs::create_dir_all(r.join(WAITING_DIR)).map_err(|e| format!("{} : {e}", r.display()))?;
    for i in active_indexers(cfg) {
        if let Some(d) = indexer_dir(cfg, &i) {
            fs::create_dir_all(&d).map_err(|e| format!("{} : {e}", d.display()))?;
        }
    }
    Ok(())
}

/// Crée le dossier d'un indexer (à l'ajout).
pub fn create_indexer_dir(cfg: &Config, name: &str) -> Result<PathBuf, String> {
    let d = indexer_dir(cfg, name).ok_or("dossier racine des indexers non réglé (Réglages → NgPost et indexer)")?;
    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

/// Renomme le dossier d'un indexer (s'il existe), sinon le crée.
pub fn rename_indexer_dir(cfg: &Config, old: &str, new: &str) -> Result<PathBuf, String> {
    let to = indexer_dir(cfg, new).ok_or("nom invalide ou dossier racine non réglé")?;
    if let Some(from) = indexer_dir(cfg, old) {
        if from.is_dir() && from != to {
            if to.exists() {
                return Err(format!("le dossier {} existe déjà", to.display()));
            }
            fs::rename(&from, &to).map_err(|e| e.to_string())?;
            return Ok(to);
        }
    }
    fs::create_dir_all(&to).map_err(|e| e.to_string())?;
    Ok(to)
}

fn has_release_files(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|rd| {
            rd.flatten().any(|e| {
                let ext = e.path().extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                matches!(ext.as_str(), "nzb" | "xml" | "json" | "txt" | "html")
            })
        })
        .unwrap_or(false)
}

/// Ancienne structure (2.2–2.3 : <racine>\<release>\) → <racine>\_En attente du NZB\<release>\.
pub fn migrate_layout(cfg: &Config) {
    let (Some(r), Some(w)) = (root(cfg), waiting_dir(cfg)) else { return };
    let indexers: Vec<String> = cfg.indexers.iter().map(|i| folder_name(&i.name)).collect();
    let Ok(rd) = fs::read_dir(&r) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let n = prep::file_name(&p);
        if !p.is_dir() || n.starts_with('_') || n.starts_with('.') || indexers.contains(&n) || !has_release_files(&p) {
            continue;
        }
        let _ = move_path(&p, &w.join(&n));
    }
    // 2.2–2.8 : releases à déposer directement dans <Indexer>\<release>\ → <Indexer>\_À uploader\<release>\
    for i in &cfg.indexers {
        let Some(d) = indexer_dir(cfg, &i.name) else { continue };
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let n = prep::file_name(&p);
            if !p.is_dir() || n.starts_with('_') || n.starts_with('.') || !has_release_files(&p) {
                continue;
            }
            let to = d.join(PENDING_DIR).join(&n);
            if !to.exists() {
                let _ = move_path(&p, &to);
            }
        }
    }
}

/// Rapatrie les NZB terminés de NgPost dans le dossier d'attente de leur release.
/// `known` : true si ce nom de release a été préparé par l'application (historique).
pub fn collect_nzbs(cfg: &Config, known: &dyn Fn(&str) -> bool) -> Vec<(String, PathBuf)> {
    let nzb_dir = cfg.nzb_dir.trim();
    let Some(w) = waiting_dir(cfg) else { return Vec::new() };
    if nzb_dir.is_empty() {
        return Vec::new();
    }
    let Ok(rd) = fs::read_dir(nzb_dir) else { return Vec::new() };
    let indexers = active_indexers(cfg);
    let mut done = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let is_nzb = p.extension().map(|x| x.eq_ignore_ascii_case("nzb")).unwrap_or(false);
        // NZB terminé : non modifié depuis 5 s
        if !p.is_file() || !is_nzb || age(&p) < Duration::from_secs(5) || nzb_ready(cfg, &p).is_err() {
            continue; // NZB encore en cours d'écriture par NgPost (en-tête seul)
        }
        let rel = release_of_nzb(&p);
        let dir = w.join(&rel);
        let elsewhere = indexers.iter().any(|i| {
            indexer_dir(cfg, i)
                .map(|d| [d.join(&rel), d.join(PENDING_DIR).join(&rel), d.join(UPLOADED_DIR).join(&rel), d.join(SKIPPED_DIR).join(&rel)].iter().any(|x| x.is_dir()))
                .unwrap_or(false)
        });
        if elsewhere || (!dir.is_dir() && !known(&rel)) {
            continue; // déjà distribuée, ou pas une release préparée ici : on n'y touche pas
        }
        if fs::create_dir_all(&dir).is_err() {
            continue;
        }
        let target = dir.join(format!("{rel}.nzb"));
        if target.exists() {
            continue;
        }
        if move_path(&p, &target).is_ok() {
            done.push((rel, target));
        }
    }
    done
}

/// Copie le dossier d'une release (sans les fichiers cachés comme le plan).
fn copy_release(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)?.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        // fichiers cachés (plan…) gardés pour soi, sauf la catégorie de contenu
        if n.starts_with('.') && n != crate::content::CAT_FILE && n != KIND_FILE && n != NFO_FILE && n != CONTENT_FILE && n != CATID_FILE && n != IMPORT_FILE {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(&n))?;
        } else {
            fs::copy(&p, to.join(&n))?;
        }
    }
    Ok(())
}

/// NZB d'un dossier de release (le premier .nzb trouvé).
pub fn nzb_in(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|f| f.is_file() && f.extension().map(|x| x.eq_ignore_ascii_case("nzb")).unwrap_or(false))
}

/// Distribue chaque release prête (NZB complet + rapport) : dans le dossier des indexers visés
/// par son plan, et dans « _Ignorés » des autres. Renvoie les releases distribuées et leur plan.
pub fn distribute(cfg: &Config) -> Vec<(String, Plan)> {
    let Some(w) = waiting_dir(cfg) else { return Vec::new() };
    let indexers = active_indexers(cfg);
    if indexers.is_empty() {
        return Vec::new();
    }
    // une seule distribution à la fois (deux fenêtres, ou fenêtre + mode --watch) : sinon la
    // même release pourrait partir deux fois
    let lock = w.join(".nzbff-distribution");
    let stale = fs::metadata(&lock).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|e| e > Duration::from_secs(600)).unwrap_or(false);
    if stale {
        let _ = fs::remove_file(&lock);
    }
    if fs::OpenOptions::new().write(true).create_new(true).open(&lock).is_err() {
        return Vec::new();
    }
    struct Unlock(PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _unlock = Unlock(lock);
    let Ok(rd) = fs::read_dir(&w) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let src = e.path();
        if !src.is_dir() {
            continue;
        }
        let rel = prep::file_name(&src);
        match nzb_in(&src) {
            Some(n) if nzb_ready(cfg, &n).is_ok() => {}
            _ => continue, // pas encore de NZB, ou NZB incomplet : on attend
        }
        // rapport absent (emplacement « avec la vidéo », découpage…) : refait avant de distribuer
        if report_required(&src) && !has_report(&src) {
            let _ = repair_report(cfg, &rel, &[]);
        }
        let plan = read_plan(cfg, &rel).unwrap_or(Plan { action: "post".into(), targets: Vec::new(), api: cfg.api_upload, compress: None, redeposit: false, cats: Vec::new(), groups: String::new() });
        let chosen = plan.resolve(cfg);
        let new_nzb = nzb_in(&src).and_then(|n| fs::read(n).ok());
        let l = crate::history::local_time(prep::now_secs());
        let stamp = format!("{:04}-{:02}-{:02} {:02}h{:02}", l.year, l.month, l.day, l.hour, l.minute);
        let mut ok = true;
        // indexers où ce post est réellement à déposer (jamais une ancienne copie)
        let mut placed: Vec<String> = Vec::new();
        for i in &indexers {
            let Some(idir) = indexer_dir(cfg, i) else { continue };
            let existing: Vec<PathBuf> = [idir.join(&rel), idir.join(PENDING_DIR).join(&rel), idir.join(UPLOADED_DIR).join(&rel), idir.join(SKIPPED_DIR).join(&rel)].into_iter().filter(|d| d.is_dir()).collect();
            // même NZB : déjà distribuée (nouvel essai après un échec partiel)
            if existing.iter().any(|e| new_nzb.is_some() && nzb_in(e).and_then(|n| fs::read(n).ok()) == new_nzb) {
                continue;
            }
            // ancienne release du même nom (test, post précédent) : mise de côté, jamais réutilisée
            let mut uploaded = false;
            for e in &existing {
                if *e == idir.join(UPLOADED_DIR).join(&rel) && !plan.redeposit {
                    uploaded = true;
                } else if move_path(e, &idir.join(REPLACED_DIR).join(format!("{rel} (ancien, {stamp})"))).is_err() {
                    ok = false;
                } else {
                    let what = if *e == idir.join(UPLOADED_DIR).join(&rel) { "déjà uploadée, envoi forcé" } else { "post précédent du même nom" };
                    note(format!("{rel} : ancienne copie chez {i} ({what}) mise de côté dans « {REPLACED_DIR} » ; c'est le nouveau post qui sera déposé"));
                }
            }
            if uploaded {
                // déjà déposée sur cet indexer : pas de second dépôt automatique, le nouveau NZB
                // et son rapport sont gardés à part
                if copy_release(&src, &idir.join(REPLACED_DIR).join(format!("{rel} (nouveau post, {stamp})"))).is_err() {
                    ok = false;
                }
                note(format!("{rel} : déjà uploadée sur {i}, pas de second dépôt ; le nouveau post est gardé à part (page Indexer : « Déposer le nouveau post » pour remplacer l'ancien)"));
                continue;
            }
            let to = if chosen.contains(i) { idir.join(PENDING_DIR).join(&rel) } else { idir.join(SKIPPED_DIR).join(&rel) };
            if copy_release(&src, &to).is_err() {
                ok = false;
            } else if chosen.contains(i) {
                placed.push(i.clone());
            }
        }
        if ok && fs::remove_dir_all(&src).is_ok() && !placed.is_empty() {
            out.push((rel, Plan { targets: placed, ..plan }));
        }
    }
    out
}

/// Emplacement actuel d'un fichier préparé, même s'il a bougé depuis :
/// vidéo rangée dans « _Postés », rapport ou NZB passés de l'attente aux dossiers des indexers.
pub fn locate(cfg: &Config, p: &Path) -> PathBuf {
    if p.exists() {
        return p.to_path_buf();
    }
    let name = prep::file_name(p);
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(parent) = p.parent() {
        cands.push(parent.join(POSTED_DIR).join(&name));
    }
    if let Some(out) = cfg.output_dir() {
        cands.push(out.join(POSTED_DIR).join(&name));
    }
    // release = nom du dossier parent (attente) ou nom du fichier sans extension
    let mut rels: Vec<String> = Vec::new();
    if let Some(d) = p.parent().map(prep::file_name) {
        rels.push(d);
    }
    if let Some(stem) = p.file_stem().map(|s| s.to_string_lossy().into_owned()) {
        rels.push(stem);
    }
    let mut idx: Vec<String> = active_indexers(cfg);
    for i in &cfg.indexers {
        if !idx.contains(&i.name) {
            idx.push(i.name.clone());
        }
    }
    for rel in &rels {
        if let Some(w) = waiting_dir(cfg) {
            cands.push(w.join(rel).join(&name));
        }
        for i in &idx {
            if let Some(d) = indexer_dir(cfg, i) {
                for base in [d.join(PENDING_DIR), d.clone(), d.join(UPLOADED_DIR), d.join(SKIPPED_DIR)] {
                    cands.push(base.join(rel).join(&name));
                }
            }
        }
    }
    if let Some(found) = cands.into_iter().find(|c| c.exists()) {
        return found;
    }
    // sinon : le dossier de la release chez un indexer, puis le premier parent existant
    for rel in &rels {
        for i in &idx {
            if let Some(d) = indexer_dir(cfg, i) {
                for base in [d.join(PENDING_DIR), d.clone(), d.join(UPLOADED_DIR), d.join(SKIPPED_DIR)] {
                    if base.join(rel).is_dir() {
                        return base.join(rel);
                    }
                }
            }
        }
    }
    p.to_path_buf()
}

/// Comment le NZB d'une release arrivera sur un indexer : une seule règle pour toute
/// l'application (pages de traitement, bandeau, récapitulatif, Envoyer des NZB, dépôt réel).
#[derive(Clone, Debug, PartialEq)]
pub struct Deposit {
    /// Déposé automatiquement par l'API (Newznab, REST, commande) dès que le NZB est prêt.
    pub api: bool,
    /// Phrase complète pour l'utilisateur.
    pub text: String,
    /// Ce qui empêche le dépôt automatique alors que l'indexer est réglé pour (à corriger).
    pub problem: Option<String>,
}

/// Ce qui manque à la méthode de dépôt automatique réglée pour cet indexer.
pub fn api_problem(i: &prep::IndexerCfg) -> Option<String> {
    let miss = |what: &[(&str, bool)]| {
        let m: Vec<&str> = what.iter().filter(|(_, ok)| !ok).map(|(w, _)| *w).collect();
        (!m.is_empty()).then(|| m.join(" et "))
    };
    match i.mode.as_str() {
        "newznab" => miss(&[("l'adresse de l'API Newznab", !i.api_url.trim().is_empty()), ("la clé API", !i.api_key.trim().is_empty())]).map(|m| format!("API Newznab réglée mais {m} manque{}", if m.contains(" et ") { "nt" } else { "" })),
        "rest" => miss(&[("l'adresse d'envoi", !i.rest_url.trim().is_empty() || i.rest_path.trim().starts_with("http")), ("la clé", !i.rest_key_eff().is_empty())]).map(|m| format!("envoi à part réglé mais {m} manque{}", if m.contains(" et ") { "nt" } else { "" })),
        "cmd" => i.cmd.trim().is_empty().then(|| "commande de dépôt réglée mais vide".to_string()),
        _ => None,
    }
}

/// Dépôt sur cet indexer, pour un traitement : `api_on` (choix « Déposer directement par
/// API » du traitement), `action` (rename, post, auto).
pub fn deposit_for(cfg: &Config, i: &prep::IndexerCfg, api_on: bool, action: &str) -> Deposit {
    let how = match i.mode.as_str() {
        "newznab" => "API Newznab",
        "rest" => "adresse d'envoi à part",
        "cmd" => "commande",
        _ => "",
    };
    let site = |why: &str| {
        let open = if action == "auto" {
            if i.url.trim().is_empty() { "le dossier du NZB s'ouvre tout seul (pas de page d'upload réglée)" } else { "la page d'upload et le dossier du NZB s'ouvrent tout seuls" }
        } else {
            "depuis la page Indexer"
        };
        format!("dépôt sur le site, {open}{why}")
    };
    if how.is_empty() {
        return Deposit { api: false, text: site(""), problem: None };
    }
    if let Some(p) = api_problem(i) {
        return Deposit { api: false, text: site(&format!(", {p}")), problem: Some(p) };
    }
    if !api_on {
        return Deposit { api: false, text: site(", dépôt par API désactivé pour ce traitement"), problem: None };
    }
    let _ = cfg;
    Deposit { api: true, text: format!("dépôt automatique par {how}, dès que le NZB est prêt"), problem: None }
}

/// Indexers visés par ce plan qui seront servis automatiquement par API.
pub fn api_targets(cfg: &Config, plan: &Plan) -> Vec<String> {
    if !plan.api {
        return Vec::new();
    }
    plan.resolve(cfg).into_iter().filter(|i| cfg.indexers.iter().any(|x| x.name.trim() == i && x.has_api())).collect()
}

/// Fichier caché d'une release déposée par API (date, indexer, résultat).
pub const API_NOTE: &str = ".nzbff-api";

/// Rapport MediaInfo présent dans le dossier d'une release ?
pub fn has_report(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|r| {
            r.flatten().map(|e| e.path()).any(|p| {
                p.is_file()
                    && !prep::file_name(&p).starts_with('.')
                    && matches!(p.extension().map(|x| x.to_string_lossy().to_lowercase()).as_deref(), Some("xml" | "json" | "txt" | "html"))
            })
        })
        .unwrap_or(false)
}

/// Un film ou une série doit avoir son rapport MediaInfo ; un autre contenu (jeu, logiciel…)
/// n'en a pas forcément.
pub fn report_required(dir: &Path) -> bool {
    // NZB importé, accepté sans rapport MediaInfo
    if fs::read_to_string(dir.join(IMPORT_FILE)).map(|t| t.contains("sans-mediainfo")).unwrap_or(false) {
        return false;
    }
    // films, séries, et contenus dont la catégorie demande MediaInfo (disques complets…)
    crate::content::read_cat(dir).is_none() || crate::content::cat_wants_mi(dir)
}

/// Vidéo d'une release, pour refaire son rapport : dossier de sortie, « _Postés », dossiers
/// de releases déplacées sur un autre disque, puis les chemins donnés (historique).
pub fn find_media(cfg: &Config, rel: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    let first_video = |p: &Path| -> Option<PathBuf> {
        if p.is_dir() {
            prep::list_videos(p, true).into_iter().next()
        } else if p.is_file() && prep::is_video(p) {
            Some(p.to_path_buf())
        } else {
            None
        }
    };
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Some(o) = cfg.output_dir() {
        bases.push(o.join(POSTED_DIR));
        bases.push(o);
    }
    for (d, _) in prep::drives() {
        bases.push(d.join(RELEASES_DIR).join(POSTED_DIR));
        bases.push(d.join(RELEASES_DIR));
    }
    for b in &bases {
        if let Some(v) = first_video(&b.join(rel)) {
            return Some(v);
        }
        for ext in prep::VIDEO_EXT {
            let f = b.join(format!("{rel}.{ext}"));
            if f.is_file() {
                return Some(f);
            }
        }
    }
    extra.iter().find_map(|p| first_video(&locate(cfg, p)))
}

/// Refait le rapport MediaInfo d'une release là où il manque (attente et dossiers des
/// indexers). Renvoie le nombre de dossiers complétés.
pub fn repair_report(cfg: &Config, rel: &str, extra: &[PathBuf]) -> Result<usize, String> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(w) = waiting_dir(cfg) {
        dirs.push(w.join(rel));
    }
    for i in &cfg.indexers {
        for st in [UpState::Pending, UpState::Done, UpState::Skip] {
            if let Some(d) = state_dir(cfg, &i.name, rel, st) {
                dirs.push(d);
            }
        }
    }
    let todo: Vec<PathBuf> = dirs.into_iter().filter(|d| d.is_dir() && !has_report(d)).collect();
    if todo.is_empty() {
        return Ok(0);
    }
    let media = find_media(cfg, rel, extra).ok_or("vidéo de la release introuvable (supprimée après le post ?) : rapport impossible à refaire")?;
    let text = prep::mediainfo_export(cfg, &media)?;
    for d in &todo {
        fs::write(d.join(format!("{rel}.{}", cfg.mi_ext())), &text).map_err(|e| format!("écriture du rapport : {e}"))?;
    }
    Ok(todo.len())
}

/// Dépose une release « à uploader » sur un indexer par API ; en cas de succès,
/// elle passe dans « _Déjà uploadés ». Renvoie le message de l'indexer.
pub fn api_upload(cfg: &Config, rel: &str, idx: &str) -> Result<String, crate::upload::UpError> {
    use crate::upload::UpError;
    let icfg = cfg.indexers.iter().find(|i| i.name.trim() == idx.trim()).ok_or_else(|| UpError::fatal("indexer inconnu"))?;
    let dir = state_dir(cfg, idx, rel, UpState::Pending).ok_or_else(|| UpError::fatal("dossier racine des indexers non réglé"))?;
    if !dir.is_dir() {
        return Err(UpError::fatal(format!("{rel} n'est pas dans « {PENDING_DIR} » de {idx}")));
    }
    // jamais de film ou de série déposé sans son rapport MediaInfo : on le refait, sinon on bloque
    if report_required(&dir) && !has_report(&dir) {
        let fixed = repair_report(cfg, rel, &[]);
        if !has_report(&dir) {
            let why = fixed.err().unwrap_or_default();
            return Err(UpError::fatal(format!("MediaInfo manquant : dépôt bloqué ({why}). Page Indexer → « Générer le MediaInfo »")));
        }
    }
    // un seul envoi à la fois pour cette release et cet indexer, même avec deux fenêtres
    // ouvertes ou le mode --watch en parallèle
    let lock = dir.join(API_LOCK);
    let fresh = fs::metadata(&lock).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|e| e < Duration::from_secs(2 * 3600)).unwrap_or(false);
    if fresh {
        return Err(UpError::fatal(format!("{rel} est déjà en cours d'envoi sur {idx} (autre fenêtre de l'application ?) : pas de second envoi")));
    }
    let _ = fs::remove_file(&lock);
    if fs::OpenOptions::new().write(true).create_new(true).open(&lock).is_err() {
        return Err(UpError::fatal(format!("{rel} est déjà en cours d'envoi sur {idx} : pas de second envoi")));
    }
    let sent = crate::upload::release_files(&dir).map(|f| crate::upload::category_for(icfg, rel, &f)).unwrap_or_default();
    let cat = if sent.is_empty() || sent == "-1" { crate::upload::category_label(&sent) } else { format!("catégorie {}", icfg.cat_label(&sent)) };
    let mode = match icfg.mode.as_str() {
        "newznab" => "API Newznab",
        "rest" => "adresse d'envoi à part",
        _ => "commande",
    };
    let _ = crate::upload::take_trace();
    let started = std::time::Instant::now();
    let res = crate::upload::upload(icfg, rel, &dir);
    let mut detail = vec![format!("Dépôt de {rel} sur {idx} ({mode}), {cat}")];
    detail.extend(deposit_summary(icfg, rel, &dir));
    detail.extend(crate::upload::take_trace());
    detail.push(format!("  durée totale : {:.1} s", started.elapsed().as_secs_f64()));
    // réponse perdue ou erreur de passerelle : l'indexer a peut-être tout reçu. On vérifie
    // sur l'indexer au lieu de renvoyer (sinon : doublon)
    let res = match res {
        Err(e) if e.uncertain => match confirm_upload(icfg, rel, CONFIRM_WAITS) {
            Some(true) => Ok(crate::upload::Reply { message: format!("reçue par l'indexer malgré l'erreur « {} » (vérifié par une recherche)", e.message), duplicate: false, guid: None, category: None }),
            Some(false) => Err(UpError::fatal(format!(
                "réponse incertaine de l'indexer ({}) et release introuvable après vérification : pas de nouvel envoi automatique (risque de doublon). Vérifie sur {idx}, puis « ⇪ Déposer (API) » si elle n'y est pas",
                e.message
            ))),
            None => Err(UpError::fatal(format!(
                "réponse incertaine de l'indexer ({}) : la release a peut-être été reçue. Pas de nouvel envoi automatique (risque de doublon) : vérifie sur {idx}, puis « ✔ Uploadé » ou « ⇪ Déposer (API) »",
                e.message
            ))),
        },
        other => other,
    };
    let _ = fs::remove_file(&lock);
    // catégorie choisie par l'indexer, si sa réponse la donne : comparée à celle envoyée
    let mut warn = String::new();
    match &res {
        Ok(r) if r.duplicate => {
            warn = " ⚠ l'indexer avait déjà cette release : ce dépôt n'a rien changé chez lui (catégorie comprise)".into();
            detail.push(format!("  ⚠ refusé comme doublon : {idx} garde la release qu'il avait déjà, avec sa catégorie. Pour la changer : supprime-la sur le site de {idx}, puis « ↩ » et « ⇪ Déposer (API) » sur la page Indexer (ou coupe « dupecheck » dans ses réglages)"));
        }
        Ok(r) => match r.category.as_deref() {
            // même famille (2040 → 2340 « Movies - FR / HD ») : l'indexer précise, c'est normal
            Some(got) if !sent.is_empty() && sent != "-1" && got != sent && got.chars().next() == sent.chars().next() => {
                warn = format!(", envoyée en {}, classée par l'indexer en {}", icfg.cat_label(&sent), icfg.cat_label(got));
                detail.push(format!("  catégorie attribuée par l'indexer : {} (envoyée : {}, précisée par l'indexer, langue, résolution…)", icfg.cat_label(got), icfg.cat_label(&sent)));
            }
            Some(got) if !sent.is_empty() && sent != "-1" && got != sent => {
                warn = format!(" ⚠ l'indexer l'a classée en {} alors que {} a été envoyée", icfg.cat_label(got), icfg.cat_label(&sent));
                detail.push(format!("  ⚠ catégorie attribuée par l'indexer : {} (envoyée : {}, autre famille), vérifie les catégories de {idx} (Réglages → Indexers)", icfg.cat_label(got), icfg.cat_label(&sent)));
            }
            Some(got) => {
                warn = format!(", classée en {}", icfg.cat_label(got));
                detail.push(format!("  ✔ catégorie attribuée par l'indexer : {}", icfg.cat_label(got)));
            }
            None => detail.push("  (la réponse de l'indexer ne précise pas la catégorie attribuée)".into()),
        },
        Err(e) => detail.push(format!("  ✖ {}", e.message)),
    }
    write_api_log(&detail);
    API_DETAIL.with(|d| *d.borrow_mut() = detail);
    let mut reply = res?;
    reply.message.push_str(&warn);
    let note = format!(
        "{}\t{}\t{}\t{}\n",
        crate::history::format_ts(prep::now_secs()),
        idx,
        reply.guid.clone().unwrap_or_default(),
        reply.message
    );
    let _ = fs::write(dir.join(API_NOTE), &note);
    set_state(cfg, rel, idx, UpState::Pending, UpState::Done)?;
    detail_done(rel, idx);
    Ok(if reply.duplicate { format!("doublon : {} ({cat})", reply.message) } else { format!("{} ({cat})", reply.message) })
}

/// Ce qui est déposé sur l'indexer : NZB (et ce qu'il contient), rapport MediaInfo, NFO et son
/// origine, BDInfo, puis le contenu posté de la release.
pub fn deposit_summary(icfg: &prep::IndexerCfg, rel: &str, dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Some(f) = crate::upload::release_files(dir) else { return out };
    let sz = |p: &Path| fs::metadata(p).map(|m| prep::human_size(m.len())).unwrap_or_else(|_| "?".into());
    let kind = if let Some((_, _, name)) = crate::content::read_cat(dir) { name } else if f.series { "série".into() } else { "film".into() };
    let api = icfg.mode == "newznab" || icfg.mode == "rest";
    out.push(format!("  Déposé sur {} ({kind}) :", icfg.name.trim()));
    let nzb = match nzb_summary(&f.nzb) {
        Some((n, b)) => format!("{} ({}), {n} fichier(s) postés, {}", prep::file_name(&f.nzb), sz(&f.nzb), prep::human_size(b)),
        None => format!("{} ({})", prep::file_name(&f.nzb), sz(&f.nzb)),
    };
    out.push(format!("    • NZB : {nzb}"));
    if let Some(g) = nzb_groups(&f.nzb).filter(|g| !g.is_empty()) {
        out.push(format!("    • Groupes Usenet (d'après le NZB) : {}", g.join(", ")));
    }
    out.push(match (&f.mediainfo, api, icfg.send_mi) {
        (Some(m), true, true) => format!("    • MediaInfo : {} ({}), champ « mediainfo »", prep::file_name(m), sz(m)),
        (Some(m), true, false) => format!("    • MediaInfo : {} présent mais NON joint (champ « mediainfo » décoché dans les réglages de l'indexer)", prep::file_name(m)),
        (Some(m), false, _) => format!("    • MediaInfo : {} ({}), {{mediainfo}} de la commande", prep::file_name(m), sz(m)),
        (None, _, _) => match &f.other_report {
            Some(o) => format!("    • MediaInfo : ⚠ seulement {} (pas en XML) : non joint", prep::file_name(o)),
            None => "    • MediaInfo : ✖ aucun rapport".to_string(),
        },
    });
    let origin = nfo_origin(dir);
    let from = match origin.as_str() {
        o if o.starts_with("origine+bdinfo") => format!("NFO d'origine ({}) suivi du rapport BDInfo", o.trim_start_matches("origine+bdinfo:").trim()),
        o if o.starts_with("origine") => format!("NFO d'origine ({})", o.trim_start_matches("origine:").trim()),
        "bdinfo" => "créé depuis le rapport BDInfo".into(),
        "modifie" => "modifié à la main avant le traitement".into(),
        "mediainfo" => "créé depuis MediaInfo".into(),
        _ => "origine inconnue".into(),
    };
    out.push(match &f.nfo {
        Some(n) => format!("    • NFO : {} ({}), {from}, champ « nfo »", prep::file_name(n), sz(n)),
        None if origin == "non-envoye" => "    • NFO : NFO d'origine présent mais NON envoyé (« Envoyer le NFO d'origine » décoché)".to_string(),
        None => "    • NFO : aucun (pas de NFO d'origine, et « Créer un NFO » décoché)".to_string(),
    });
    if let Some(b) = &f.bdinfo {
        out.push(format!(
            "    • BDInfo : {} ({}), {}",
            prep::file_name(b),
            sz(b),
            if !icfg.bdinfo_field.trim().is_empty() && api {
                format!("champ « {} »", icfg.bdinfo_field.trim())
            } else if origin == "bdinfo" {
                "envoyé comme NFO".to_string()
            } else if origin.starts_with("origine+bdinfo") {
                "ajouté sous le NFO d'origine".to_string()
            } else if f.nfo_original {
                "gardé à côté du NZB (le NFO d'origine est envoyé ; champ BDInfo de l'indexer non réglé)".to_string()
            } else {
                "gardé à côté du NZB".to_string()
            }
        ));
    }
    let list = read_content_list(dir);
    if !list.is_empty() {
        let total: u64 = list.iter().map(|(_, b)| *b).sum();
        out.push(format!("  Contenu posté de {rel} : {} fichier(s), {}", list.len(), prep::human_size(total)));
        let mut sorted = list.clone();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        for (p, b) in sorted.iter().take(15) {
            out.push(format!("      {p}  ({})", prep::human_size(*b)));
        }
        if sorted.len() > 15 {
            out.push(format!("      … et {} autre(s) fichier(s)", sorted.len() - 15));
        }
    }
    out
}

/// Nouveau post d'une release déjà uploadée, gardé à part dans « _Remplacés » (le plus récent).
pub fn newer_post(cfg: &Config, rel: &str, idx: &str) -> Option<PathBuf> {
    let d = indexer_dir(cfg, idx)?.join(REPLACED_DIR);
    let prefix = format!("{rel} (nouveau post");
    let mut v: Vec<PathBuf> = fs::read_dir(&d).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir() && prep::file_name(p).starts_with(&prefix)).collect();
    v.sort();
    v.pop()
}

/// Le nouveau post remplace l'ancien chez cet indexer : l'ancienne copie (déjà uploadée) part
/// dans « _Remplacés », le nouveau post revient « à uploader ».
pub fn promote_newer(cfg: &Config, rel: &str, idx: &str) -> Result<PathBuf, String> {
    let newer = newer_post(cfg, rel, idx).ok_or("aucun nouveau post gardé pour cette release")?;
    let idir = indexer_dir(cfg, idx).ok_or("indexer invalide")?;
    let l = crate::history::local_time(prep::now_secs());
    let stamp = format!("{:04}-{:02}-{:02} {:02}h{:02}", l.year, l.month, l.day, l.hour, l.minute);
    for st in [UpState::Done, UpState::Pending, UpState::Skip] {
        if let Some(old) = state_dir(cfg, idx, rel, st).filter(|d| d.is_dir()) {
            move_path(&old, &idir.join(REPLACED_DIR).join(format!("{rel} (ancien, {stamp})"))).map_err(|e| e.to_string())?;
        }
    }
    let to = state_dir(cfg, idx, rel, UpState::Pending).ok_or("indexer invalide")?;
    move_path(&newer, &to).map_err(|e| e.to_string())?;
    Ok(to)
}

thread_local! {
    static API_DETAIL: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Ligne finale du détail : déposée et rangée dans « _Déjà uploadés ».
fn detail_done(rel: &str, idx: &str) {
    let l = format!("  ✔ {rel} déposée sur {idx}, rangée dans « {UPLOADED_DIR} »");
    write_api_log(std::slice::from_ref(&l));
    API_DETAIL.with(|d| d.borrow_mut().push(l));
}

/// Réponse brute de l'indexer dans le détail du dernier dépôt (pour l'e-mail).
pub fn last_reply(detail: &[String]) -> Option<String> {
    detail.iter().find(|l| l.trim_start().starts_with('←')).map(|l| l.trim().trim_start_matches('←').trim().to_string())
}

/// Détail du dernier dépôt fait par ce fil (requête, réponse, catégorie), pour le journal.
pub fn take_api_detail() -> Vec<String> {
    API_DETAIL.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

/// Journal des dépôts sur les indexers : `logs\api-AAAA-MM-JJ.log`, une ligne horodatée par étape.
fn write_api_log(lines: &[String]) {
    use std::io::Write;
    let l = crate::history::local_time(prep::now_secs());
    let dir = logs_dir();
    let _ = fs::create_dir_all(&dir);
    let p = dir.join(format!("api-{:04}-{:02}-{:02}.log", l.year, l.month, l.day));
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(p) {
        for x in lines {
            let _ = writeln!(f, "[{:02}:{:02}:{:02}] {}", l.hour, l.minute, l.second, prep::redact(x));
        }
        let _ = writeln!(f);
    }
}

// --------------------------------------------------------- déjà sur les indexers ?

/// Noms d'origine d'une release (fichier, dossier, NZB), pour la chercher aussi sous ces noms.
pub const ORIGIN_FILE: &str = ".nzbff-origine";
/// Début du message d'un post évité : la release est déjà sur tous les indexers visés.
pub const POST_DUPE: &str = "Pas posté : déjà présente sur";

pub fn write_origins(cfg: &Config, rel: &str, names: &[String]) {
    let Some(w) = waiting_dir(cfg) else { return };
    let d = w.join(rel);
    if fs::create_dir_all(&d).is_err() {
        return;
    }
    let mut v = read_origins(cfg, rel);
    for n in names {
        let n = n.trim().to_string();
        if !n.is_empty() && n != rel && !v.contains(&n) {
            v.push(n);
        }
    }
    let _ = fs::write(d.join(ORIGIN_FILE), v.join("\n"));
}

pub fn read_origins(cfg: &Config, rel: &str) -> Vec<String> {
    waiting_dir(cfg).and_then(|w| fs::read_to_string(w.join(rel).join(ORIGIN_FILE)).ok()).map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()).unwrap_or_default()
}

/// Avant de poster : la release (sous son nom et ses noms d'origine) est-elle déjà sur les
/// indexers visés ? Réglage « skip » : ils sont retirés du plan, et rien n'est posté si elle
/// est déjà partout (erreur commençant par `POST_DUPE`) ; « warn » : signalé seulement.
pub fn dupe_gate(cfg: &Config, rel: &str, video: &Path, on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
    if cfg.dupe_check == "off" {
        on_line("Présence sur les indexers : non vérifiée (réglage « Ne pas vérifier »)");
        return Ok(());
    }
    let mut plan = read_plan(cfg, rel).unwrap_or_else(|| Plan::from_cfg(cfg));
    if plan.redeposit {
        on_line("Présence sur les indexers : pas vérifiée (envoi forcé)");
        return Ok(());
    }
    let targets = plan.resolve(cfg);
    let searchable: Vec<&prep::IndexerCfg> = cfg.indexers.iter().filter(|i| i.active && targets.iter().any(|t| t.eq_ignore_ascii_case(i.name.trim())) && crate::search::method(i).is_some()).collect();
    if targets.is_empty() {
        return Ok(());
    }
    let mut origins = read_origins(cfg, rel);
    if let Some(stem) = video.file_stem().map(|s| s.to_string_lossy().into_owned()) {
        if video.is_file() {
            origins.push(stem);
        }
    }
    let names = crate::search::names_for(rel, &origins);
    // uploadée à la main et notée sur un indexer visé : comptée présente, même sans recherche
    let marked = targets.iter().any(|t| crate::search::marked_any(&names, t).is_some());
    if searchable.is_empty() && !marked {
        on_line(&format!("Présence sur les indexers : non vérifiée (aucune recherche réglée pour {}, Réglages → Indexers → Recherche)", targets.join(", ")));
        return Ok(());
    }
    on_line(&format!("Présence sur les indexers : recherche de {} sur {}…", names.join(" / "), searchable.iter().map(|i| i.name.trim()).collect::<Vec<_>>().join(", ")));
    let res = crate::search::check(cfg, &names, &targets);
    let mut present: Vec<String> = Vec::new();
    for l in &res {
        let how = cfg.indexers.iter().find(|i| i.name.trim() == l.indexer).and_then(crate::search::method).unwrap_or_else(|| "—".into());
        let mark = match &l.found {
            crate::search::Found::Exact(_) => "⚠",
            crate::search::Found::Similar(_) => "ℹ",
            crate::search::Found::Absent => "✔",
            crate::search::Found::Unknown(_) => "?",
        };
        on_line(&format!("  {mark} {} : {} ({how})", l.indexer, l.found.label()));
        if prep::detail() {
            for q in &l.requests {
                on_line(&format!("      → GET {q}"));
            }
        }
        if matches!(l.found, crate::search::Found::Exact(_)) {
            present.push(l.indexer.clone());
        }
    }
    let named = if names.len() > 1 { format!(" (cherchée aussi sous son nom d'origine : {})", names[1..].join(" / ")) } else { String::new() };
    if present.is_empty() {
        on_line(&format!("✔ Absente des indexers vérifiés : postée sous « {rel} »{named}"));
        return Ok(());
    }
    if cfg.dupe_check == "warn" {
        on_line(&format!("⚠ Déjà présente sur {} : postée quand même (réglage « signaler seulement »)", present.join(", ")));
        crate::notify::record(format!("⚠ {rel} déjà présente sur {} (postée quand même)", present.join(", ")));
        return Ok(());
    }
    let left: Vec<String> = targets.iter().filter(|t| !present.iter().any(|p| p.eq_ignore_ascii_case(t))).cloned().collect();
    if left.is_empty() {
        crate::notify::record(format!("⏭ {rel} pas postée : déjà présente sur {}", present.join(", ")));
        return Err(format!("{POST_DUPE} {} (vérification avant le post). « Poster quand même » pour forcer.", present.join(", ")));
    }
    on_line(&format!("⏭ Déjà présente sur {} : ignorée pour eux ; postée sous « {rel} » pour {}{named}", present.join(", "), left.join(", ")));
    plan.targets = left;
    let _ = write_plan(cfg, rel, &plan);
    Ok(())
}

/// Force le post d'une release refusée comme déjà présente (plan « envoi forcé »).
pub fn force_post(cfg: &Config, rel: &str) {
    let mut plan = read_plan(cfg, rel).unwrap_or_else(|| Plan::from_cfg(cfg));
    plan.redeposit = true;
    let _ = write_plan(cfg, rel, &plan);
}

/// Attentes avant chaque vérification d'un envoi incertain (secondes).
const CONFIRM_WAITS: &[u64] = if cfg!(test) { &[0] } else { &[20, 60, 120] };

/// Groupes Usenet d'une release selon son type : catégorie de contenu, série ou film.
/// Vide : ceux des options du post (ou de ngPost.conf). Renvoie (groupes, type).
pub fn groups_for(cfg: &Config, release: &str) -> (String, String) {
    let content = waiting_dir(cfg).and_then(|w| crate::content::read_cat(&w.join(release)));
    let marked = waiting_dir(cfg).map(|w| is_marked_series(&w.join(release))).unwrap_or(false);
    let (g, kind, _) = groups_kind(cfg, content.as_ref().map(|(k, _, n)| (k.as_str(), n.as_str())), marked, release);
    (g, kind)
}

/// Groupes d'un type de release : (groupes, type, d'où ils viennent). Groupes vides : ceux des
/// options du post (ou de ngPost.conf). Une catégorie de contenu sans groupes reçoit ceux
/// conseillés pour elle, jamais ceux des films.
pub fn groups_kind(cfg: &Config, content: Option<(&str, &str)>, series: bool, release: &str) -> (String, String, String) {
    if let Some((key, name)) = content {
        let own = cfg.content_cats.iter().find(|c| c.key == key).map(|c| c.groups.trim().to_string()).unwrap_or_default();
        if !own.is_empty() {
            return (own, name.to_string(), format!("ceux de la catégorie « {name} »"));
        }
        let d = crate::content::default_groups(key, release).join(", ");
        return (d, name.to_string(), format!("conseillés pour « {name} » (la catégorie n'en a pas : Réglages → Catégories de contenu)"));
    }
    if series || crate::series::is_series_name(release) {
        (cfg.groups_tv.trim().to_string(), "série".into(), "ceux des séries".into())
    } else {
        (cfg.groups_film.trim().to_string(), "film".into(), "ceux des films".into())
    }
}

/// Mots qui désignent, dans un nom de groupe, les groupes adaptés à un type de contenu.
fn group_words(family: &str) -> &'static [&'static str] {
    match family {
        "tv" => &["teevee", "tv", "hdtv", "series", "multimedia", "x264"],
        "music" => &["sounds", "mp3", "flac", "lossless", "music"],
        "audiobook" => &["audiobooks", "audiobook", "sounds"],
        "books" => &["ebook", "e-book", "books", "comics", "magazines", "mags"],
        "anime" => &["anime"],
        "games" => &["games", "console", "nintendo", "playstation", "xbox", "warez", "0day", "iso"],
        "film" => &["movies", "moovee", "hdtv", "bluray", "x264", "hd", "uhd", "multimedia", "dvd"],
        _ => &["multimedia", "misc", "boneless"],
    }
}

/// Famille de contenu d'un type de post (« film », « tv », « music »…), d'après la catégorie
/// de contenu (clé et nom) ou le nom de la release.
pub fn group_family(content: Option<(&str, &str)>, series: bool, release: &str) -> &'static str {
    if let Some((key, name)) = content {
        let t = format!("{key} {name}").to_lowercase();
        return if t.contains("audiobook") || t.contains("livre audio") || t.contains("livres audio") {
            "audiobook"
        } else if t.contains("musi") || t.contains("audio") || t.contains("flac") || t.contains("mp3") {
            "music"
        } else if t.contains("livre") || t.contains("ebook") || t.contains("book") || t.contains("comic") || t.contains("bd") || t.contains("magazine") {
            "books"
        } else if t.contains("anim") {
            "anime"
        } else if t.contains("jeu") || t.contains("game") || t.contains("console") || t.contains("logiciel") || t.contains("appli") || t.contains("pc") {
            "games"
        } else {
            "other"
        };
    }
    if series || crate::series::is_series_name(release) {
        "tv"
    } else {
        "film"
    }
}

/// Le post passe-t-il par un groupe indexé par cet indexer ?
#[derive(Clone, Debug, PartialEq)]
pub struct GroupFit {
    pub idx: String,
    /// Au moins un groupe du post est indexé (ou l'indexer n'a pas publié sa liste).
    pub ok: bool,
    /// Groupe ajouté au post pour cet indexer (règle « ajouter »).
    pub added: Option<String>,
    /// Groupes indexés adaptés au contenu, à proposer.
    pub suggest: Vec<String>,
    /// Groupes ajoutés par la règle de l'indexer pour cette catégorie : (catégorie, groupes).
    pub rule: Option<(String, String)>,
}

/// Groupes choisis à la main pour cette catégorie de l'indexer (numéro exact, sinon sa
/// famille : « 2000 » vaut pour 2040, 2045…).
pub fn group_rule_for<'a>(i: &'a prep::IndexerCfg, cat: &str) -> Option<&'a (String, String)> {
    let cat = cat.trim();
    if cat.is_empty() || cat == "-1" {
        return None;
    }
    i.group_rules.iter().find(|(c, g)| c.trim() == cat && !g.trim().is_empty()).or_else(|| {
        i.group_rules.iter().find(|(c, g)| {
            let c = c.trim();
            !g.trim().is_empty() && c.len() == cat.len() && c.len() >= 4 && c.ends_with("000") && c[..1] == cat[..1]
        })
    })
}

/// Groupes indexés de `idx` adaptés à la famille de contenu (les plus parlants d'abord).
pub fn indexed_for(idx: &prep::IndexerCfg, family: &str) -> Vec<String> {
    indexed_in(&idx.groups, family)
}

/// Groupes de cette liste adaptés au contenu, du plus au moins adapté.
pub fn indexed_in(groups: &[String], family: &str) -> Vec<String> {
    let words = group_words(family);
    let mut v: Vec<(usize, String)> = groups
        .iter()
        .filter_map(|g| {
            let parts: Vec<&str> = g.trim_start_matches("alt.binaries.").split(['.', '-', '_']).collect();
            words.iter().position(|w| parts.iter().any(|p| p == w)).map(|rank| (rank, g.clone()))
        })
        .collect();
    v.sort();
    v.into_iter().map(|x| x.1).collect()
}

/// Ajuste les groupes d'un post aux groupes indexés des indexers visés : règle « ajouter »,
/// un de leurs groupes adaptés est ajouté ; sinon le manque est signalé.
pub fn fit_groups(cfg: &Config, groups: &str, family: &str, targets: &[String], release: &str, content: Option<(String, String)>) -> (String, Vec<GroupFit>) {
    let mut list: Vec<String> = groups.split(',').map(|g| g.trim().to_lowercase()).filter(|g| !g.is_empty()).collect();
    let mut fits = Vec::new();
    let files = crate::upload::Files { content: content.clone(), series: family == "tv", ..Default::default() };
    for t in targets {
        let Some(i) = cfg.indexers.iter().find(|i| i.name.trim().eq_ignore_ascii_case(t.trim())) else { continue };
        // groupes choisis à la main pour la catégorie où la release sera envoyée
        if !release.is_empty() {
            let cat = crate::upload::category_for(i, release, &files);
            if let Some((c, g)) = group_rule_for(i, &cat) {
                let mut added = Vec::new();
                for x in g.split(',').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()) {
                    if !list.contains(&x) {
                        list.push(x.clone());
                        added.push(x);
                    }
                }
                fits.push(GroupFit { idx: i.name.trim().to_string(), ok: true, added: (!added.is_empty()).then(|| added.join(",")), suggest: Vec::new(), rule: Some((if c.trim() == cat.trim() { cat.clone() } else { format!("{cat} (famille {c})") }, g.clone())) });
                continue;
            }
        }
        if i.groups.is_empty() || i.groups_rule == "off" {
            continue;
        }
        let ok = list.iter().any(|g| i.groups.iter().any(|x| x == g));
        let suggest = indexed_for(i, family);
        let mut added = None;
        if !ok && i.groups_rule == "add" {
            if let Some(g) = suggest.first() {
                if !list.contains(g) {
                    list.push(g.clone());
                }
                added = Some(g.clone());
            }
        }
        fits.push(GroupFit { idx: i.name.trim().to_string(), ok: ok || added.is_some(), added, suggest, rule: None });
    }
    (list.join(","), fits)
}

/// Groupes réellement utilisés au post, et leur origine (pour le récapitulatif et le journal).
pub fn groups_used(cfg: &Config, conf: Option<&NgConf>, content: Option<(&str, &str)>, series: bool, release: &str) -> (String, String) {
    let (g, _, from) = groups_kind(cfg, content, series, release);
    if !g.is_empty() {
        return (g, from);
    }
    if !cfg.ng_groups.trim().is_empty() {
        return (cfg.ng_groups.trim().to_string(), "groupes par défaut (options du post)".into());
    }
    match conf.and_then(|c| c.get("GROUPS")).map(|g| g.trim().to_string()).filter(|g| !g.is_empty()) {
        Some(g) => (g, "ceux de ngPost.conf".into()),
        None => ("groupe par défaut de NgPost".into(), "rien de réglé".into()),
    }
}

/// Fichier caché posé pendant l'envoi d'une release à un indexer.
pub const API_LOCK: &str = ".nzbff-envoi-en-cours";

/// Après une réponse incertaine : la release apparaît-elle sur l'indexer ? Recherche après
/// chaque attente (secondes). None : impossible à vérifier (API sans recherche Newznab).
pub fn confirm_upload(idx: &prep::IndexerCfg, rel: &str, waits: &[u64]) -> Option<bool> {
    let mut checked = false;
    for w in waits {
        std::thread::sleep(Duration::from_secs(*w));
        match crate::upload::find_on_indexer(idx, rel) {
            Some(true) => return Some(true),
            Some(false) => checked = true,
            None => {}
        }
    }
    checked.then_some(false)
}

/// Page d'upload d'un indexer (si renseignée).
pub fn upload_url(cfg: &Config, idx: &str) -> Option<String> {
    cfg.indexers.iter().find(|i| i.name.trim() == idx.trim()).map(|i| i.url.trim().to_string()).filter(|u| !u.is_empty())
}

/// État d'une release pour un indexer (déduit du dossier où elle se trouve).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum UpState {
    Pending,
    Done,
    Skip,
}

impl UpState {
    fn sub(self) -> Option<&'static str> {
        match self {
            UpState::Pending => Some(PENDING_DIR),
            UpState::Done => Some(UPLOADED_DIR),
            UpState::Skip => Some(SKIPPED_DIR),
        }
    }
}

pub fn state_dir(cfg: &Config, idx: &str, rel: &str, st: UpState) -> Option<PathBuf> {
    let d = indexer_dir(cfg, idx)?;
    Some(match st.sub() {
        Some(s) => d.join(s).join(rel),
        None => d.join(rel),
    })
}

/// Une release et son état sur chaque indexer.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub name: String,
    /// Dans « _En attente du NZB » (NZB pas encore arrivé, ou aucun indexer actif).
    pub waiting: Option<PathBuf>,
    pub nzb: bool,
    /// Extension du rapport MediaInfo trouvé.
    pub report: Option<String>,
    pub states: Vec<(String, UpState, PathBuf)>,
    pub modified: u64,
    /// NZB présent mais refusé (incomplet…), dans le dossier d'attente.
    pub nzb_error: Option<String>,
    /// Plan enregistré (dossier d'attente).
    pub plan: Option<Plan>,
    /// Catégorie de contenu (jeu, logiciel…) : (clé, catégorie Newznab par défaut, nom).
    pub content: Option<(String, String, String)>,
    /// Indexers où un nouveau post de cette release (déjà uploadée) attend à part.
    pub newer: Vec<String>,
}

impl Release {
    pub fn pending_on(&self, idx: &str) -> bool {
        self.states.iter().any(|(i, s, _)| i == idx && *s == UpState::Pending)
    }
    pub fn any_pending(&self) -> bool {
        self.states.iter().any(|(_, s, _)| *s == UpState::Pending)
    }
    pub fn finished(&self) -> bool {
        self.waiting.is_none() && !self.states.is_empty() && !self.any_pending()
    }
    /// Dossier à ouvrir : celui de la release sur le premier indexer où elle reste à déposer.
    pub fn open_dir(&self) -> Option<&Path> {
        self.states
            .iter()
            .find(|(_, s, _)| *s == UpState::Pending)
            .map(|(_, _, d)| d.as_path())
            .or(self.waiting.as_deref())
            .or(self.states.first().map(|(_, _, d)| d.as_path()))
    }
}

fn scan_files(dir: &Path, r: &mut Release) {
    if let Ok(files) = fs::read_dir(dir) {
        for f in files.flatten().map(|f| f.path()) {
            let ext = f.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            if ext == "nzb" {
                r.nzb = true;
            } else if matches!(ext.as_str(), "xml" | "json" | "txt" | "html") {
                r.report = Some(ext);
            }
        }
    }
    if r.content.is_none() {
        r.content = crate::content::read_cat(dir);
    }
    r.modified = r.modified.max(mtime_secs(dir));
}

/// Toutes les releases connues (attente + dossiers des indexers actifs), les plus récentes d'abord.
pub fn list_releases(cfg: &Config) -> Vec<Release> {
    let mut map: std::collections::BTreeMap<String, Release> = std::collections::BTreeMap::new();
    let new = |n: &str| Release { name: n.to_string(), waiting: None, nzb: false, report: None, states: Vec::new(), modified: 0, nzb_error: None, plan: None, content: None, newer: Vec::new() };
    if let Some(w) = waiting_dir(cfg) {
        if let Ok(rd) = fs::read_dir(&w) {
            for e in rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
                let n = prep::file_name(&e);
                let r = map.entry(n.clone()).or_insert_with(|| new(&n));
                r.waiting = Some(e.clone());
                scan_files(&e, r);
                if let Some(n) = nzb_in(&e) {
                    if let Err(why) = nzb_ready(cfg, &n) {
                        r.nzb = false;
                        r.nzb_error = Some(why);
                    }
                }
                r.plan = fs::read_to_string(e.join(PLAN_FILE)).ok().map(|t| parse_plan(&t));
            }
        }
    }
    for i in active_indexers(cfg) {
        for st in [UpState::Pending, UpState::Done, UpState::Skip] {
            let Some(base) = indexer_dir(cfg, &i).map(|d| match st.sub() {
                Some(s) => d.join(s),
                None => d,
            }) else {
                continue;
            };
            let Ok(rd) = fs::read_dir(&base) else { continue };
            for e in rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
                let n = prep::file_name(&e);
                if n.starts_with('.') {
                    continue;
                }
                let r = map.entry(n.clone()).or_insert_with(|| new(&n));
                r.states.push((i.clone(), st, e.clone()));
                scan_files(&e, r);
            }
        }
    }
    for r in map.values_mut() {
        let done: Vec<String> = r.states.iter().filter(|(_, s, _)| *s == UpState::Done).map(|(i, _, _)| i.clone()).collect();
        r.newer = done.into_iter().filter(|i| newer_post(cfg, &r.name, i).is_some()).collect();
    }
    let mut v: Vec<Release> = map.into_values().collect();
    v.sort_by(|a, b| b.modified.cmp(&a.modified));
    v
}

/// NZB importés (déjà en ta possession) : gardés ici, d'où ils partent vers les indexers.
pub const IMPORT_DIR: &str = "_NZB importés";
/// Marqueur d'un NZB importé (« sans-mediainfo » : dépôt accepté sans rapport MediaInfo).
pub const IMPORT_FILE: &str = ".nzbff-import";

/// Rapport MediaInfo et NFO posés à côté d'un NZB (même nom, sinon le seul du dossier).
pub fn nzb_companions(nzb: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    let dir = nzb.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = nzb.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let files: Vec<PathBuf> = fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default();
    let pick = |exts: &[&str]| -> Option<PathBuf> {
        let of: Vec<&PathBuf> = files.iter().filter(|p| p.extension().map(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x))).unwrap_or(false)).collect();
        of.iter()
            .find(|p| {
                let n = prep::file_name(p).to_lowercase();
                n.starts_with(&stem)
            })
            .map(|p| (*p).clone())
            .or_else(|| (of.len() == 1).then(|| of[0].clone()))
    };
    (pick(&["xml"]), pick(&["nfo"]))
}

/// Importe un NZB déjà en ta possession comme release prête : NZB vérifié (complet), rapport
/// MediaInfo et NFO joints s'ils sont donnés. Renvoie son dossier (dans « _NZB importés »).
pub fn import_nzb(cfg: &Config, nzb: &Path, rel: &str, mi: Option<&Path>, nfo: Option<&Path>, series: bool, no_mi_ok: bool) -> Result<PathBuf, String> {
    let rel = prep::clean_name(rel);
    if rel.is_empty() {
        return Err("nom de release vide".into());
    }
    nzb_info(nzb)?;
    if mi.is_none() && !no_mi_ok {
        return Err("pas de rapport MediaInfo : joins-le, ou coche « Déposer sans MediaInfo »".into());
    }
    let root = root(cfg).ok_or("dossier racine des indexers non réglé (Réglages → Dossiers)")?;
    let dir = root.join(IMPORT_DIR).join(&rel);
    fs::create_dir_all(&dir).map_err(|e| format!("{} : {e}", dir.display()))?;
    fs::copy(nzb, dir.join(format!("{rel}.nzb"))).map_err(|e| format!("copie du NZB : {e}"))?;
    if let Some(m) = mi {
        fs::copy(m, dir.join(format!("{rel}.xml"))).map_err(|e| format!("copie du rapport MediaInfo : {e}"))?;
    }
    if let Some(n) = nfo {
        fs::copy(n, dir.join(format!("{rel}.nfo"))).map_err(|e| format!("copie du NFO : {e}"))?;
        let _ = fs::write(dir.join(NFO_FILE), format!("origine:{}", prep::file_name(n)));
    }
    if series {
        let _ = fs::write(dir.join(KIND_FILE), "serie\n");
    }
    let _ = fs::write(dir.join(IMPORT_FILE), if mi.is_none() { "importé, sans-mediainfo\n" } else { "importé\n" });
    Ok(dir)
}

/// Une copie d'une release déjà prête (NZB gardé sur le disque) : indexer (ou nom du dossier
/// s'il ne correspond plus à un indexer réglé), réglé ou non, état, dossier.
#[derive(Clone, Debug, PartialEq)]
pub struct Copy {
    pub place: String,
    pub known: bool,
    pub state: UpState,
    pub dir: PathBuf,
}

/// Release déjà prête, retrouvée partout où son NZB est gardé.
#[derive(Clone, Debug, PartialEq)]
pub struct Stored {
    pub name: String,
    pub copies: Vec<Copy>,
    /// NZB importé (« _NZB importés »).
    pub imported: Option<PathBuf>,
    /// Date du NZB le plus ancien (≈ date du post).
    pub when: u64,
}

impl Stored {
    /// Déposée (ou à déposer) sur cet indexer.
    pub fn on(&self, idx: &str) -> bool {
        self.copies.iter().any(|c| c.known && c.place.eq_ignore_ascii_case(idx.trim()) && c.state != UpState::Skip)
    }
    pub fn done_on(&self, idx: &str) -> bool {
        self.copies.iter().any(|c| c.known && c.place.eq_ignore_ascii_case(idx.trim()) && c.state == UpState::Done)
    }
    /// Dossier d'où la renvoyer : une copie déjà déposée de préférence.
    pub fn source(&self) -> Option<PathBuf> {
        let mut v: Vec<&Copy> = self.copies.iter().collect();
        v.sort_by_key(|c| match c.state {
            UpState::Done => 0,
            UpState::Pending => 1,
            UpState::Skip => 2,
        });
        v.first().map(|c| c.dir.clone()).or_else(|| self.imported.clone())
    }
}

/// Toutes les releases dont le NZB est gardé sous la racine des indexers : dans le dossier de
/// chaque indexer (actif ou non, et les dossiers d'indexers renommés ou retirés des réglages),
/// « à uploader », « déjà uploadés » et « ignorés », plus les NZB importés.
pub fn catalog(cfg: &Config) -> Vec<Stored> {
    let Some(root) = root(cfg) else { return Vec::new() };
    let mut map: std::collections::BTreeMap<String, Stored> = std::collections::BTreeMap::new();
    let mtime = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let mut add = |name: String, nzb: &Path, f: &mut dyn FnMut(&mut Stored)| {
        let when = mtime(nzb);
        let p = map.entry(name.clone()).or_insert_with(|| Stored { name, copies: Vec::new(), imported: None, when });
        if when > 0 && (p.when == 0 || when < p.when) {
            p.when = when;
        }
        f(p);
    };
    let Ok(rd) = fs::read_dir(&root) else { return Vec::new() };
    for top in rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        let folder = prep::file_name(&top);
        if folder.starts_with('.') || folder == WAITING_DIR {
            continue;
        }
        if folder == IMPORT_DIR {
            for d in fs::read_dir(&top).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
                if let Some(n) = nzb_in(&d) {
                    let dd = d.clone();
                    add(prep::file_name(&d), &n, &mut |p| p.imported = Some(dd.clone()));
                }
            }
            continue;
        }
        let known = cfg.indexers.iter().find(|i| folder_name(&i.name) == folder).map(|i| i.name.trim().to_string());
        for st in [UpState::Done, UpState::Pending, UpState::Skip] {
            let Some(sub) = st.sub() else { continue };
            for d in fs::read_dir(top.join(sub)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
                let Some(n) = nzb_in(&d) else { continue };
                let c = Copy { place: known.clone().unwrap_or_else(|| folder.clone()), known: known.is_some(), state: st, dir: d.clone() };
                add(prep::file_name(&d), &n, &mut |p| p.copies.push(c.clone()));
            }
        }
    }
    let mut v: Vec<Stored> = map.into_values().collect();
    v.sort_by(|a, b| b.when.cmp(&a.when).then(a.name.cmp(&b.name)));
    v
}

/// Dossier d'une release prête à repartir : celui d'un indexer où elle est (déjà déposée de
/// préférence), sinon « _NZB importés ».
#[cfg(test)]
pub fn release_source(cfg: &Config, rel: &str) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for st in [UpState::Done, UpState::Pending, UpState::Skip] {
        for i in &cfg.indexers {
            if let Some(d) = state_dir(cfg, &i.name, rel, st).filter(|d| nzb_in(d).is_some()) {
                found.push(d);
            }
        }
    }
    found
        .into_iter()
        .next()
        .or_else(|| root(cfg).map(|r| r.join(IMPORT_DIR).join(rel)).filter(|d| nzb_in(d).is_some()))
        // indexer désactivé, renommé ou retiré : son ancien dossier sert encore
        .or_else(|| catalog(cfg).into_iter().find(|p| p.name == rel).and_then(|p| p.source()))
}

/// Envoie une release prête (NZB + rapports) vers « à uploader » d'un indexer : depuis
/// « _Ignorés » de cet indexer si elle y est, sinon copiée depuis `src`. Refusé si elle y est
/// déjà déposée ou à déposer (sauf `again` : nouvel envoi d'une release déjà déposée).
pub fn send_to_indexer(cfg: &Config, src: &Path, rel: &str, idx: &str, again: bool) -> Result<PathBuf, String> {
    let dst = state_dir(cfg, idx, rel, UpState::Pending).ok_or("indexer invalide ou dossier racine non réglé")?;
    if dst.exists() {
        return Err(format!("{rel} est déjà à déposer sur {idx}"));
    }
    let done = state_dir(cfg, idx, rel, UpState::Done).filter(|d| d.exists());
    if done.is_some() && !again {
        return Err(format!("{rel} est déjà déposée sur {idx}"));
    }
    if let Some(skip) = state_dir(cfg, idx, rel, UpState::Skip).filter(|d| d.exists() && nzb_in(d).is_some()) {
        move_path(&skip, &dst).map_err(|e| e.to_string())?;
        return Ok(dst);
    }
    // nouvel envoi d'une release déjà déposée : elle repart de son dossier « déjà uploadés »
    if let (Some(d), true) = (done, again) {
        move_path(&d, &dst).map_err(|e| e.to_string())?;
        return Ok(dst);
    }
    if !src.is_dir() || nzb_in(src).is_none() {
        return Err(format!("{rel} : NZB introuvable ({})", src.display()));
    }
    copy_release(src, &dst).map_err(|e| format!("copie vers {idx} : {e}"))?;
    let _ = fs::remove_file(dst.join(API_LOCK));
    Ok(dst)
}

/// Change l'état d'une release sur un indexer en déplaçant son dossier.
pub fn set_state(cfg: &Config, rel: &str, idx: &str, from: UpState, to: UpState) -> Result<PathBuf, String> {
    let src = state_dir(cfg, idx, rel, from).ok_or("indexer invalide")?;
    let dst = state_dir(cfg, idx, rel, to).ok_or("indexer invalide")?;
    if dst.exists() {
        return Err(format!("{} existe déjà", dst.display()));
    }
    move_path(&src, &dst).map_err(|e| e.to_string())?;
    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Avant le post : release déjà sur un indexer visé → retiré du plan ; partout → pas de
    /// post ; envoi forcé → pas de vérification.
    #[test]
    fn deja_sur_les_indexers() {
        let _g = prep::test_lock();
        let (addr, seen) = crate::dl::tests::fake_server(|head, _| {
            if head.contains("/a/api") {
                r#"<?xml version="1.0"?><rss><channel><item><title>Le.Film.2020.FRENCH.1080p.WEB-GRP</title></item></channel></rss>"#.to_string()
            } else if head.contains("/b/api") && head.contains("original") {
                // B ne la connaît que sous son nom d'origine
                r#"<?xml version="1.0"?><rss><channel><item><title>Le Film 2020 Original</title></item></channel></rss>"#.to_string()
            } else {
                r#"<?xml version="1.0"?><rss><channel></channel></rss>"#.to_string()
            }
        });
        let root = std::env::temp_dir().join(format!("nzbff-dupe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mut cfg = Config { indexer_dir: root.display().to_string(), dupe_check: "skip".into(), ..Default::default() };
        let mk = |n: &str, api: &str| {
            let mut i = prep::IndexerCfg::new(n);
            if !api.is_empty() {
                i.mode = "newznab".into();
                i.api_url = format!("{addr}/{api}/api");
                i.api_key = "K3Y".into();
            }
            i
        };
        cfg.indexers = vec![mk("A", "a"), mk("B", "b"), mk("C", "")];
        let rel = "Le.Film.2020.FRENCH.1080p.WEB-GRP";
        let plan = Plan { action: "post".into(), targets: vec!["A".into(), "B".into(), "C".into()], api: true, compress: None, redeposit: false, cats: Vec::new(), groups: String::new() };
        write_plan(&cfg, rel, &plan).unwrap();
        write_origins(&cfg, rel, &["le film 2020 original".into()]);
        assert_eq!(read_origins(&cfg, rel), vec!["le film 2020 original".to_string()]);
        let mut log = Vec::new();
        dupe_gate(&cfg, rel, Path::new("/nulle/part.mkv"), &mut |l| log.push(l.to_string())).unwrap();
        assert_eq!(read_plan(&cfg, rel).unwrap().targets, vec!["C".to_string()], "{log:?}");
        assert!(log.iter().any(|l| l.contains("⚠ A : déjà présente")));
        assert!(log.iter().any(|l| l.contains("⚠ B : déjà présente")), "trouvée sous son nom d'origine : {log:?}");
        assert!(log.iter().any(|l| l.contains("postée sous « Le.Film.2020.FRENCH.1080p.WEB-GRP » pour C") && l.contains("le.film.2020.original")), "{log:?}");
        assert!(log.iter().any(|l| l.contains("? C : non vérifiée")));
        // la clé ne passe pas en clair dans les journaux
        assert!(!log.iter().any(|l| l.contains("K3Y")));
        assert!(seen.lock().unwrap().iter().any(|r| r.contains("t=search") && r.contains("q=Le.Film.2020")));
        // seulement A visé : pas de post
        write_plan(&cfg, rel, &Plan { targets: vec!["A".into()], ..plan.clone() }).unwrap();
        let e = dupe_gate(&cfg, rel, Path::new("/x.mkv"), &mut |_| {}).unwrap_err();
        assert!(e.starts_with(POST_DUPE) && e.contains('A'));
        // signaler seulement / forcer : posté
        cfg.dupe_check = "warn".into();
        dupe_gate(&cfg, rel, Path::new("/x.mkv"), &mut |_| {}).unwrap();
        cfg.dupe_check = "skip".into();
        force_post(&cfg, rel);
        dupe_gate(&cfg, rel, Path::new("/x.mkv"), &mut |_| {}).unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    /// par2cmdline-turbo et ParPar : options de NgPost et copie de ngPost.conf seulement
    /// quand il le faut ; outil introuvable ou mal nommé : par2 de NgPost.
    #[test]
    fn outil_par2_choisi() {
        let d = std::env::temp_dir().join(format!("nzbff-par2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("ParPar")).unwrap();
        fs::create_dir_all(d.join("turbo")).unwrap();
        let parpar = d.join("ParPar").join("parpar.exe");
        let turbo = d.join("turbo").join("par2.exe");
        let bad = d.join("autre.exe");
        for f in [&parpar, &turbo, &bad] {
            fs::write(f, "x").unwrap();
        }
        let conf = d.join("ngPost.conf");
        fs::write(&conf, "PAR2_PCT = 8\nPAR2_ARGS = -s5M -r1n*0.6 -m2048M -p1l --progress stdout -q\n[server]\nhost = news.example\npass = secret\n").unwrap();
        let mut log = Vec::new();
        let mut on = |l: &str| log.push(l.to_string());
        let mut cfg = Config { ng_par2: true, ..Default::default() };
        assert_eq!(par2_setup(&cfg, Some(&conf), &d, &mut on), (None, vec![]));
        // ParPar : PAR2_ARGS pour lui, redondance de ngPost.conf
        cfg.ng_par2_tool = "parpar".into();
        cfg.ng_parpar = parpar.display().to_string();
        let (c, a) = par2_setup(&cfg, Some(&conf), &d, &mut on);
        let t = fs::read_to_string(c.unwrap()).unwrap();
        assert!(t.contains("PAR2_ARGS = -s5M -r8% -m2048M -p1l --progress stdout -q"), "{t}");
        assert!(t.contains("host = news.example") && !t.contains("-r1n"));
        assert_eq!(a, vec!["--par2_path".to_string(), path_arg(&parpar)]);
        // nom sans « parpar » : NgPost ne le reconnaîtrait pas
        cfg.ng_parpar = bad.display().to_string();
        assert_eq!(par2_setup(&cfg, Some(&conf), &d, &mut on), (None, vec![]));
        // sans ngPost.conf : impossible de lui donner ses arguments
        cfg.ng_parpar = parpar.display().to_string();
        assert_eq!(par2_setup(&cfg, None, &d, &mut on), (None, vec![]));
        // ParPar mis par erreur dans le réglage de turbo : utilisé comme ParPar
        cfg.ng_par2_tool = "turbo".into();
        cfg.ng_turbo = parpar.display().to_string();
        let (c, a) = par2_setup(&cfg, Some(&conf), &d, &mut on);
        assert!(fs::read_to_string(c.unwrap()).unwrap().contains("PAR2_ARGS = -s5M -r8%"));
        assert_eq!(a, vec!["--par2_path".to_string(), path_arg(&parpar)]);
        assert_eq!(par2_kind(&parpar).0, Par2Kind::ParPar);
        // turbo : PAR2_ARGS de ParPar retirés, redondance donnée à NgPost
        cfg.ng_par2_tool = "turbo".into();
        cfg.ng_turbo = turbo.display().to_string();
        let (c, a) = par2_setup(&cfg, Some(&conf), &d, &mut on);
        let t = fs::read_to_string(c.unwrap()).unwrap();
        assert!(t.lines().any(|l| l.trim() == "PAR2_ARGS =") && !t.contains("--progress"), "{t}");
        assert_eq!(a, vec!["--par2_path".to_string(), path_arg(&turbo), "--par2_pct".into(), "8".into()]);
        // turbo avec une conf ordinaire : pas de copie de ngPost.conf (mots de passe)
        fs::write(&conf, "PAR2_PCT = 8\n[server]\npass = secret\n").unwrap();
        cfg.ng_par2_pct = 12;
        let (c, a) = par2_setup(&cfg, Some(&conf), &d, &mut on);
        assert!(c.is_none());
        assert_eq!(a, vec!["--par2_path".to_string(), path_arg(&turbo)]);
        // introuvable
        cfg.ng_turbo = d.join("absent.exe").display().to_string();
        assert_eq!(par2_setup(&cfg, Some(&conf), &d, &mut on), (None, vec![]));
        assert!(log.iter().filter(|l| l.starts_with('⚠')).count() >= 3);
        assert_eq!(parpar_args(10), "-s5M -r10% -m2048M -p1l --progress stdout -q");
        assert!(parpar_style("-s5M -r1n*0.6 -m2048M -p1l --progress stdout -q") && !parpar_style("c -l -m1024 -r8 -s768000"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn arguments() {
        assert_eq!(split_args(r#"--monitor "D:\Mes films\Sortie" -c x.conf"#), vec!["--monitor", r"D:\Mes films\Sortie", "-c", "x.conf"]);
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn lecture_ngpost_conf() {
        // contenu réel, enregistré en Windows-1252 (« défaut » sur un octet)
        let text = "## dossier de destination par d\u{e9}faut pour les fichiers nzb\r\n## si vous n'indiquez rien,le nzb sera \u{e9}crit dans le dossier de ngPost\r\nnzbPath  = C:/NZB upload\r\n";
        let latin1: Vec<u8> = text.chars().map(|c| c as u8).collect();
        assert!(std::str::from_utf8(&latin1).is_err());
        assert_eq!(parse_nzb_path(&decode_text(&latin1)).as_deref(), Some("C:/NZB upload"));
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice(text.as_bytes());
        assert_eq!(parse_nzb_path(&decode_text(&bom)).as_deref(), Some("C:/NZB upload"));
        let mut u16le = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            u16le.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(parse_nzb_path(&decode_text(&u16le)).as_deref(), Some("C:/NZB upload"));
        assert_eq!(parse_nzb_path("#nzbPath = X\nautre = 1"), None);
    }

    #[test]
    fn lecture_complete_ngpost_conf() {
        // extrait de la vraie configuration (commentaires accentués, espaces variables)
        let text = "## r\u{e9}pertoire par d\u{e9}faut pour l'ajout de nouveaux fichiers dans les posts rapide de l'interface graphique\r\ninputDir = C:/ngPost/AutoNZB\r\n\r\n## dossier de destination par d\u{e9}faut pour les fichiers nzb\r\nnzbPath  = C:/NZB upload\r\n#obfuscate = true\r\n";
        let latin1: Vec<u8> = text.chars().map(|c| c as u8).collect();
        let c = NgConf::parse(Path::new("ngPost.conf"), &decode_text(&latin1));
        assert_eq!(c.get("inputdir"), Some("C:/ngPost/AutoNZB"));
        assert_eq!(c.get("obfuscate"), None); // commentée
        let sep = if cfg!(windows) { "\\" } else { "/" };
        assert_eq!(c.watch_dir(), Some((format!("C:{sep}ngPost{sep}AutoNZB"), "inputDir".to_string())));
        assert_eq!(c.nzb_dir(), Some(format!("C:{sep}NZB upload")));
        // un dossier de surveillance explicite passe avant inputDir
        let c2 = NgConf::parse(Path::new("x"), "inputDir = C:/a\nMONITOR_FOLDERS = D:/watch1, D:/watch2\n");
        assert_eq!(c2.watch_dir().map(|w| w.1), Some("MONITOR_FOLDERS".to_string()));
        // lecture depuis le dossier de ngPost.exe
        let d = std::env::temp_dir().join(format!("nzbff_ngconf_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("ngPost.conf"), &latin1).unwrap();
        fs::write(d.join("ngPost.exe"), b"").unwrap();
        let cfg = Config { ngpost_exe: format!("\"{}\"", d.join("ngPost.exe").display()), ..Default::default() };
        let rc = read_conf(&cfg).unwrap();
        assert_eq!(rc.path, d.join("ngPost.conf"));
        assert!(read_nzb_dir(&cfg).unwrap().ends_with("NZB upload"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn pourcentages() {
        assert_eq!(parse_percent("[avg. speed: 55.2 MB/s] >>>>>>>>>>>>  45% (12 / 26 files)"), Some(45));
        assert_eq!(parse_percent("upload 99.6 %"), Some(99));
        assert_eq!(parse_percent("rien"), None);
        assert_eq!(parse_percent("par2 10% puis 250%"), Some(10));
    }

    /// NZB complet minimal, comme en écrit NgPost.
    pub(crate) const NZB_OK: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE nzb>\n<nzb xmlns=\"http://www.newzbin.com/DTD/2003/nzb\">\n<head>\n<meta type=\"password\">s3cr3t</meta>\n</head>\n<file poster=\"x@y\" date=\"1\" subject=\"a\">\n<groups>\n<group>alt.binaries.misc</group>\n</groups>\n<segments>\n<segment bytes=\"716800\" number=\"1\">a@ngPost</segment>\n<segment bytes=\"1000\" number=\"2\">b@ngPost</segment>\n</segments>\n</file>\n</nzb>\n";
    /// Ce que NgPost a écrit au début du post : l'en-tête seulement (cas du bug « line 4, column 0 »).
    const NZB_PARTIEL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE nzb>\n<nzb xmlns=\"http://www.newzbin.com/DTD/2003/nzb\">\n";

    #[test]
    fn validation_nzb() {
        let d = std::env::temp_dir().join(format!("nzbff_nzbinfo_{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("ok.nzb"), NZB_OK).unwrap();
        let i = nzb_info(&d.join("ok.nzb")).unwrap();
        assert_eq!((i.files, i.segments, i.bytes), (1, 2, 717_800));
        assert_eq!(i.groups, vec!["alt.binaries.misc"]);
        assert_eq!(i.password.as_deref(), Some("s3cr3t"));
        fs::write(d.join("part.nzb"), NZB_PARTIEL).unwrap();
        assert!(nzb_info(&d.join("part.nzb")).unwrap_err().contains("incomplet"));
        fs::write(d.join("vide.nzb"), "<nzb></nzb>").unwrap();
        assert!(nzb_info(&d.join("vide.nzb")).is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn plus_de_500_episodes_tiennent_dans_la_ligne_de_commande() {
        let d = std::env::temp_dir().join(format!("nzbff-500ep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let rel = d.join("Ma.Grande.Serie.S01.INTEGRALE.FRENCH.480p.WEB.H264-GRP");
        fs::create_dir_all(rel.join("Saison 1")).unwrap();
        fs::create_dir_all(rel.join("Saison 2")).unwrap();
        for i in 1..=320 {
            for s in [1, 2] {
                fs::write(rel.join(format!("Saison {s}")).join(format!("Ma.Grande.Serie.S{s:02}E{i:03}.FRENCH.480p.WEB.H264-GRP.mkv")), b"x").unwrap();
            }
        }
        fs::write(rel.join("Ma.Grande.Serie.nfo"), b"nfo").unwrap();
        let cfg = Config::default();
        let inputs = post_inputs(&cfg, &rel);
        // les dossiers de saison à la place de 640 fichiers, le NFO à part
        assert_eq!(inputs.len(), 3, "{inputs:?}");
        assert!(inputs.contains(&rel.join("Saison 1")) && inputs.contains(&rel.join("Saison 2")) && inputs.contains(&rel.join("Ma.Grande.Serie.nfo")));
        let args = post_args_multi(&cfg, None, &inputs, Path::new("x.nzb"));
        assert!(args.iter().map(|a| a.len() + 3).sum::<usize>() < 32_000);
        // compressé : le dossier lui-même s'il y a trop d'entrées
        let flat = d.join("Flat");
        fs::create_dir_all(&flat).unwrap();
        for i in 1..=600 {
            fs::write(flat.join(format!("Une.Serie.Avec.Un.Titre.Assez.Long.S01E{i:03}.FRENCH.480p.WEB.H264-GRP.mkv")), b"x").unwrap();
        }
        let c2 = Config { ng_compress: true, ..Default::default() };
        assert_eq!(post_inputs(&c2, &flat), vec![flat.clone()]);
        assert_eq!(post_inputs(&cfg, &flat), vec![flat.clone()]);
        // peu de fichiers : rien ne change
        assert_eq!(post_inputs(&cfg, &rel.join("Saison 1")).len(), 320.min(post_inputs(&cfg, &rel.join("Saison 1")).len()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn arguments_de_post() {
        // options reprises de la vraie configuration : PACK = COMPRESS, GEN_NAME, GEN_PAR2 ; obfuscate = article
        let conf = NgConf::parse(Path::new("ngPost.conf"), "PACK = COMPRESS, GEN_NAME, GEN_PAR2\nobfuscate = article\nRAR_SIZE = 42\nPAR2_PCT = 8\nGROUPS   = alt.binaries.misc\n");
        let mut cfg = Config { post_obfuscate: false, ..Default::default() };
        let got = import_post_opts(&mut cfg, &conf);
        assert!(cfg.ng_imported && cfg.ng_compress && cfg.ng_gen_name && cfg.ng_par2 && !cfg.ng_gen_pass && cfg.post_obfuscate, "{got:?}");
        assert_eq!(conf.num("RAR_SIZE"), Some(42));
        let a = post_args(&cfg, Some(Path::new("C:/ngPost/ngPost.conf")), Path::new("C:/Sortie/Mon Film.mkv"), Path::new("D:/tmp/R.nzb"));
        let sep = if cfg!(windows) { "\\" } else { "/" };
        assert_eq!(
            a,
            vec![
                "-c".to_string(),
                format!("C:{sep}ngPost{sep}ngPost.conf"),
                "-i".into(),
                format!("C:{sep}Sortie{sep}Mon Film.mkv"),
                "-o".into(),
                format!("D:{sep}tmp{sep}R.nzb"),
                "--disp_progress".into(),
                "BAR".into(),
                "-x".into(),
                "--compress".into(),
                "--gen_name".into(),
                "--gen_par2".into(),
            ]
        );
        // valeurs forcées + arguments supplémentaires
        let cfg2 = Config { ng_compress: true, ng_rar_size: 50, ng_par2: true, ng_par2_pct: 10, ng_groups: "a.b.c, d.e".into(), ng_threads: 4, ng_debug: true, post_args: "--meta x=y".into(), ..cfg.clone() };
        let a = post_args(&cfg2, None, Path::new("v.mkv"), Path::new("n.nzb"));
        let j = a.join(" ");
        assert!(j.contains("-g a.b.c,d.e") && j.contains("--rar_size 50") && j.contains("--par2_pct 10") && j.contains("-t 4") && j.contains(" -d") && j.ends_with("--meta x=y"), "{j}");
        // ancien réglage : ligne complète avec {input}
        let cfg3 = Config { post_args: "-c perso.conf -i {input} -o {nzb} --obfuscate".into(), post_obfuscate: true, ..Default::default() };
        assert_eq!(post_args(&cfg3, None, Path::new("v.mkv"), Path::new("n.nzb")), vec!["-c", "perso.conf", "-i", "v.mkv", "-o", "n.nzb", "--obfuscate"]);
        assert!(command_preview(&cfg, None).contains("--compress"));
        assert!(post_warnings(&Config { ng_compress: true, ..Default::default() }, None).iter().any(|w| w.contains("7-Zip")));
    }

    #[test]
    fn gravite() {
        assert_eq!(line_severity("nb article: 3108, failed: 0"), 0);
        assert_eq!(line_severity("nb article: 3108, failed: 12"), 2);
        assert_eq!(line_severity("Error: authentication failed"), 2);
        assert_eq!(line_severity("warning: connection 7 timeout"), 1);
    }

    #[test]
    fn etapes() {
        assert_eq!(phase_of("[avg. speed: 52 MB/s] >>>>>> 5%"), Some("Envoi"));
        assert_eq!(phase_of("Compressing files with 7z..."), Some("Compression"));
        assert_eq!(phase_of("Generating par2: 40%"), Some("Par2"));
        assert_eq!(phase_of("bonjour"), None);
    }

    #[cfg(unix)]
    #[test]
    fn post_d_une_saison_de_540_episodes() {
        let _verrou = crate::prep::test_lock();
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("nzbff_post540_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        let exe = d.join("ngPost");
        let argsf = d.join("args");
        fs::write(d.join("ok.nzb"), NZB_OK).unwrap();
        // faux NgPost : note ses arguments ; « --help » cite --rar_no_root_folder
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = --help ]; then echo '  --rar_no_root_folder  Remove root (parent) folder'; exit 0; fi\necho \"$@\" > '{}'\nwhile [ $# -gt 0 ]; do case $1 in -o) out=$2; shift;; esac; shift; done\ncp '{}' \"$out\"\n",
            argsf.display(),
            d.join("ok.nzb").display()
        );
        fs::write(&exe, script).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        let rel = "Ma.Grande.Serie.S01.COMPLETE.FRENCH.480p.WEB.H264-GRP";
        let dir = d.join("sortie").join(rel);
        fs::create_dir_all(&dir).unwrap();
        for i in 1..=540 {
            fs::write(dir.join(format!("Ma.Grande.Serie.S01E{i:03}.FRENCH.480p.WEB.H264-GRP.mkv")), b"x").unwrap();
        }
        for compress in [false, true] {
            let cfg = Config {
                ngpost_exe: exe.display().to_string(),
                ngpost_dir: d.join("sortie").display().to_string(),
                indexer_dir: d.join("idx").display().to_string(),
                ng_compress: compress,
                post_after: "keep".into(),
                ..Default::default()
            };
            let mut lines = Vec::new();
            post_file(&cfg, &dir, rel, &mut |l| lines.push(l.to_string()), &mut |_| {}).unwrap();
            let args = fs::read_to_string(&argsf).unwrap();
            // un seul « -i » : le dossier de la release, pas 540 fichiers
            assert_eq!(args.matches("-i ").count(), 1, "{args}");
            assert!(args.contains(&format!("-i {}", dir.display())), "{args}");
            assert_eq!(args.contains("--rar_no_root_folder"), compress, "{args}");
            assert!(lines.iter().any(|l| l.contains("540 fichiers")), "{lines:?}");
            let _ = fs::remove_dir_all(d.join("idx"));
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn post_avec_faux_ngpost() {
        let _verrou = crate::prep::test_lock();
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("nzbff_post_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        let exe = d.join("ngPost");
        fs::write(d.join("ok.nzb"), NZB_OK).unwrap();
        // faux NgPost : écrit l'en-tête du NZB tout de suite (comme le vrai), compresse, poste, puis termine le NZB
        let script = format!(
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do case $1 in -o) out=$2; shift;; esac; shift; done\nprintf '<?xml version=\"1.0\"?>\\n<nzb>\\n' > \"$out\"\necho 'Compressing files with 7z'\nprintf ' 50%%\\r100%%\\n'\necho 'erreur mineure' >&2\nprintf '[avg. speed: 50 MB/s] >>> 30%%\\r[avg. speed: 50 MB/s] >>> 100%%\\n'\necho 'Upload size: 1 GB, nb article: 2, failed: 0'\ncp '{}' \"$out\"\n",
            d.join("ok.nzb").display()
        );
        fs::write(&exe, script).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        let video = d.join("sortie/R.2001.FRENCH.1080p.x264.mkv");
        fs::write(&video, b"x").unwrap();
        let cfg = Config {
            ngpost_exe: exe.display().to_string(),
            ngpost_dir: d.join("sortie").display().to_string(),
            indexer_dir: d.join("idx").display().to_string(),
            post_obfuscate: true,
            post_after: "posted".into(),
            ..Default::default()
        };
        let (mut lines, mut pcts) = (Vec::new(), Vec::new());
        let p = post_file(&cfg, &video, "R.2001.FRENCH.1080p.x264", &mut |l| lines.push(l.to_string()), &mut |p| pcts.push((p.phase.clone(), p.pct))).unwrap();
        assert_eq!(p.nzb, d.join("idx").join(WAITING_DIR).join("R.2001.FRENCH.1080p.x264/R.2001.FRENCH.1080p.x264.nzb"));
        assert_eq!(p.info.files, 1);
        assert!(pcts.contains(&("Compression".into(), 50)) && pcts.contains(&("Envoi".into(), 30)), "{pcts:?}");
        assert!(lines.iter().find(|l| l.starts_with("> ")).map(|l| l.contains("-x")).unwrap_or(false), "{lines:?}");
        assert!(lines.iter().any(|l| l == "erreur mineure"), "{lines:?}");
        assert!(p.summary.iter().any(|l| l.contains("Upload size")));
        let log = fs::read_to_string(p.log.unwrap()).unwrap();
        assert!(log.contains("Compressing") && log.contains("NZB complet"), "{log}");
        // vidéo rangée dans _Postés, et retrouvée même après déplacement
        assert!(!video.exists() && d.join("sortie").join(POSTED_DIR).join("R.2001.FRENCH.1080p.x264.mkv").exists());
        assert_eq!(locate(&cfg, &video), d.join("sortie").join(POSTED_DIR).join("R.2001.FRENCH.1080p.x264.mkv"));
        assert!(list_unposted(&cfg).is_empty());
        // NgPost s'arrête au milieu : NZB réduit à l'en-tête → refusé, rien n'est rangé
        fs::write(&exe, "#!/bin/sh\nwhile [ $# -gt 0 ]; do case $1 in -o) out=$2; shift;; esac; shift; done\nprintf '<?xml version=\"1.0\"?>\\n<!DOCTYPE nzb>\\n<nzb>\\n' > \"$out\"\n").unwrap();
        fs::write(d.join("sortie/C.mkv"), b"x").unwrap();
        let e = post_file(&cfg, &d.join("sortie/C.mkv"), "C", &mut |_| {}, &mut |_| {}).unwrap_err();
        assert!(e.contains("incomplet") && e.contains("NZB conservé"), "{e}");
        assert!(!d.join("idx").join(WAITING_DIR).join("C").join("C.nzb").exists());
        assert!(logs_dir().join("C.nzb").is_file()); // jamais perdu
        // échec : pas de NZB produit
        fs::write(&exe, "#!/bin/sh\necho 'erreur serveur' >&2\nexit 3\n").unwrap();
        fs::write(d.join("sortie/B.mkv"), b"x").unwrap();
        let e = post_file(&cfg, &d.join("sortie/B.mkv"), "B", &mut |_| {}, &mut |_| {}).unwrap_err();
        assert!(e.contains("erreur serveur"), "{e}");
        assert!(d.join("sortie/B.mkv").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn sortie_reelle_de_ngpost() {
        let bar = "[=====>                                            ] 10 % (607 / 6022) avg. speed:  12.22 MB/s";
        assert_eq!(parse_percent(bar), Some(10));
        assert_eq!(parse_counts(bar), Some((607, 6022)));
        assert!((parse_speed(bar).unwrap() - 12.22 * 1048576.0).abs() < 1.0);
        assert_eq!(phase_of(bar), Some("Envoi"));
        let fin = "[20:51:26.748] Taille du Post: 3.98 GB en 00:02:04.509 (124 sec) => débit moyen:  32.76 MB/s (100 connexions sur 8 threads)";
        assert!((parse_speed(fin).unwrap() - 32.76 * 1048576.0).abs() < 1.0);
        assert!(is_summary_line(fin) && !is_summary_line(bar));
        assert_eq!(phase_of("Finishing posting..."), Some("Finalisation"));
        for n in ["[Poster #1 {NntpCon #42}] Closing connection...", "[Poster #2 {NntpCon #56}] No more articles", "[Poster #3] threads stopped", "All posters stopped...", "[Poster #0] connection thread done"] {
            assert!(is_noise(n), "{n}");
        }
        assert!(!is_noise(fin));
    }

    /// Cas réel : NgPost poste tout, écrit le NZB sous un autre nom (sans l'année), puis quitte avec le code 1.
    #[cfg(unix)]
    #[test]
    fn code_1_mais_nzb_complet() {
        let _verrou = crate::prep::test_lock();
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("nzbff_code1_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        fs::write(d.join("ok.nzb"), NZB_OK).unwrap();
        let exe = d.join("ngPost");
        let script = format!(
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do case $1 in -o) out=$2; shift;; esac; shift; done\ndir=$(dirname \"$out\")\nprintf '[=====>   ] 10 %% (1 / 2) avg. speed:  12.22 MB/s\\r'\nfor i in 1 2 3; do echo '[Poster #1 {{NntpCon #42}}] Closing connection...'; done\nprintf '[=========] 100 %% (2 / 2) avg. speed:  32.76 MB/s\\n'\necho 'Finishing posting...'\necho '[20:51:26.748] Taille du Post: 3.98 GB en 00:02:04.509 (124 sec) => débit moyen:  32.76 MB/s (100 connexions sur 8 threads)'\ncp '{}' \"$dir/monfilm.FRENCH.576p.DVDRip.nzb\"\necho '[Poster #3] threads stopped'\necho 'All posters stopped...'\nexit 1\n",
            d.join("ok.nzb").display()
        );
        fs::write(&exe, script).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        let video = d.join("sortie/monfilm.1983.FRENCH.576p.DVDRip.mkv");
        fs::write(&video, b"x").unwrap();
        let cfg = Config { ngpost_exe: exe.display().to_string(), ngpost_dir: d.join("sortie").display().to_string(), indexer_dir: d.join("idx").display().to_string(), ..Default::default() };
        let (mut lines, mut prog) = (Vec::new(), Vec::new());
        let p = post_file(&cfg, &video, "monfilm.1983.FRENCH.576p.DVDRip", &mut |l| lines.push(l.to_string()), &mut |p| prog.push(p.clone())).unwrap();
        // rangé sous le nom de la release, à côté du rapport
        assert_eq!(p.nzb, d.join("idx").join(WAITING_DIR).join("monfilm.1983.FRENCH.576p.DVDRip/monfilm.1983.FRENCH.576p.DVDRip.nzb"));
        assert!(p.warnings.iter().any(|w| w.contains("code 1")), "{:?}", p.warnings);
        // post vérifié : la vidéo est supprimée (corbeille par défaut)
        assert!(!video.exists() && p.video_after.starts_with("supprimée"), "{}", p.video_after);
        assert!((p.avg_bps.unwrap() - 32.76 * 1048576.0).abs() < 1.0);
        assert!(prog.iter().any(|x| x.done == 1 && x.total == 2 && x.avg_bps.is_some()));
        // lignes de connexion masquées à l'écran, résumées en une ligne
        assert!(!lines.iter().any(|l| l.contains("NntpCon")) && lines.iter().any(|l| l.contains("lignes techniques")), "{lines:?}");
        // vérification désactivée : un NZB imparfait est rangé quand même, avec un avertissement
        fs::write(d.join("ok.nzb"), NZB_PARTIEL).unwrap();
        fs::write(&video, b"x").unwrap();
        let cfg2 = Config { post_verify: false, post_after: "trash".into(), ..cfg.clone() };
        let p2 = post_file(&cfg2, &video, "R2", &mut |_| {}, &mut |_| {}).unwrap();
        assert!(p2.nzb.is_file() && p2.warnings.iter().any(|w| w.contains("non vérifié")));
        // NZB non vérifié : la vidéo n'est pas supprimée, elle est mise de côté
        assert!(d.join("sortie").join(POSTED_DIR).join("monfilm.1983.FRENCH.576p.DVDRip.mkv").exists());
        assert!(p2.video_after.contains(POSTED_DIR));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn detection_dans_un_sous_dossier() {
        let d = std::env::temp_dir().join(format!("nzbff_detect_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for v in ["v4.14_x64", "v4.16_x64"] {
            fs::create_dir_all(d.join("ngPost").join(v)).unwrap();
            fs::write(d.join("ngPost").join(v).join("ngPost.exe"), b"").unwrap();
        }
        fs::create_dir_all(d.join("Autre")).unwrap();
        assert_eq!(find_exe_under(&[d.clone()]), Some(d.join("ngPost/v4.16_x64/ngPost.exe")));
        assert_eq!(find_exe_under(&[d.join("ngPost")]), Some(d.join("ngPost/v4.16_x64/ngPost.exe")));
        assert_eq!(find_exe_under(&[d.join("Autre")]), None);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn controle_obfuscation() {
        let cfg = Config { post_obfuscate: true, ..Default::default() };
        let mut info = NzbInfo { subjects: vec!["[1/25] - \"eAKCCmI7llbpaN0nMcjxO2.7z.001\" yEnc (1/60)".into()], ..Default::default() };
        let r = obfuscation_report(&cfg, &info, "monfilm.1983.FRENCH.576p.DVDRip");
        assert!(r.iter().all(|x| x.0), "{r:?}");
        info.subjects = vec!["[1/3] - \"monfilm.1983.FRENCH.576p.DVDRip.mkv\" yEnc (1/60)".into()];
        assert!(obfuscation_report(&cfg, &info, "monfilm.1983.FRENCH.576p.DVDRip").iter().any(|x| !x.0));
        let d = std::env::temp_dir().join(format!("nzbff_subj_{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("a.nzb"), NZB_OK).unwrap();
        assert_eq!(nzb_info(&d.join("a.nzb")).unwrap().subjects, vec!["a"]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn nom_du_nzb() {
        assert_eq!(release_of_nzb(Path::new("/n/Film.2001.FRENCH.1080p.x264.nzb")), "Film.2001.FRENCH.1080p.x264");
        assert_eq!(release_of_nzb(Path::new("/n/Film.2001.FRENCH.1080p.x264.mkv.nzb")), "Film.2001.FRENCH.1080p.x264");
        assert_eq!(folder_name(" Mon:Indexer? "), "Mon_Indexer_");
    }

    fn old_file(p: &Path, content: &str) {
        fs::write(p, content).unwrap();
        fs::File::options().write(true).open(p).unwrap().set_modified(SystemTime::now() - Duration::from_secs(60)).unwrap();
    }

    /// Release arrivée chez les indexers sans son rapport : refait depuis la vidéo postée,
    /// sinon dépôt bloqué (jamais d'envoi sans MediaInfo).
    #[cfg(unix)]
    #[test]
    fn jamais_de_depot_sans_mediainfo() {
        use std::os::unix::fs::PermissionsExt;
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_sans_mi_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let mi = d.join("mediainfo");
        fs::create_dir_all(&d).unwrap();
        fs::write(&mi, "#!/bin/sh\nfor a; do f=\"$a\"; done\nprintf '<MediaInfo><File>%s</File></MediaInfo>\\n' \"$(basename \"$f\")\"\n").unwrap();
        fs::set_permissions(&mi, fs::Permissions::from_mode(0o755)).unwrap();
        let mut a = prep::IndexerCfg::new("A");
        a.mode = "newznab".into();
        a.api_url = "http://127.0.0.1:9/api".into();
        a.api_key = "cle".into();
        let cfg = Config {
            ngpost_dir: d.join("sortie").display().to_string(),
            indexer_dir: d.join("idx").display().to_string(),
            mediainfo: mi.display().to_string(),
            mi_dest: "indexer".into(),
            indexers: vec![a],
            ..Default::default()
        };
        ensure_dirs(&cfg).unwrap();
        let rel = "Film.2024.FRENCH.1080p.WEB.x264";
        // NZB arrivé, rapport absent (rangé « avec la vidéo », découpage…)
        let w = waiting_dir(&cfg).unwrap().join(rel);
        fs::create_dir_all(&w).unwrap();
        fs::write(w.join(format!("{rel}.nzb")), NZB_OK).unwrap();
        // vidéo introuvable : distribuée quand même, mais le dépôt par API est refusé
        assert_eq!(distribute(&cfg).len(), 1);
        let pending = state_dir(&cfg, "A", rel, UpState::Pending).unwrap();
        assert!(!has_report(&pending) && report_required(&pending));
        let e = api_upload(&cfg, rel, "A").unwrap_err();
        assert!(e.message.contains("MediaInfo manquant") && e.retry_after.is_none(), "{}", e.message);
        assert!(list_releases(&cfg).iter().any(|r| r.name == rel && r.report.is_none()));
        // la vidéo postée est dans « _Postés » : le rapport est refait, à son nom de release
        let posted = d.join("sortie").join(POSTED_DIR);
        fs::create_dir_all(&posted).unwrap();
        fs::write(posted.join(format!("{rel}.mkv")), b"video").unwrap();
        assert_eq!(repair_report(&cfg, rel, &[]).unwrap(), 1);
        assert!(has_report(&pending));
        assert!(fs::read_to_string(pending.join(format!("{rel}.xml"))).unwrap().contains(&format!("{rel}.mkv")));
        // une autre release, réparée toute seule au moment de la distribution
        let rel2 = "Autre.2024.FRENCH.1080p.WEB.x264";
        let w2 = waiting_dir(&cfg).unwrap().join(rel2);
        fs::create_dir_all(&w2).unwrap();
        fs::write(w2.join(format!("{rel2}.nzb")), NZB_OK).unwrap();
        fs::write(posted.join(format!("{rel2}.mkv")), b"video").unwrap();
        distribute(&cfg);
        assert!(has_report(&state_dir(&cfg, "A", rel2, UpState::Pending).unwrap()));
        // un autre contenu (logiciel) n'a pas besoin de rapport
        let soft = d.join("soft");
        fs::create_dir_all(&soft).unwrap();
        fs::write(soft.join(crate::content::CAT_FILE), "logiciels-pc\t4010\tLogiciels PC\n").unwrap();
        assert!(!report_required(&soft));
        let _ = fs::remove_dir_all(&d);
    }

    /// Release repostée sous le même nom qu'un ancien test : le nouveau NZB et son rapport
    /// sont déposés, l'ancienne copie (sans rapport) est mise de côté et jamais envoyée.
    #[test]
    fn repost_du_meme_nom_jamais_l_ancienne_copie() {
        let d = std::env::temp_dir().join(format!("nzbff_repost_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let cfg = Config { indexer_dir: d.join("idx").display().to_string(), indexers: vec![prep::IndexerCfg::new("A"), prep::IndexerCfg::new("B")], ..Default::default() };
        ensure_dirs(&cfg).unwrap();
        let rel = "Serie.S01.FRENCH.1080p.H264";
        let old_nzb = NZB_OK.replace("s3cr3t", "ancien");
        // ancien test : à uploader sur A (NZB seul, sans rapport), déjà uploadé sur B
        let old_a = state_dir(&cfg, "A", rel, UpState::Pending).unwrap();
        let old_b = state_dir(&cfg, "B", rel, UpState::Done).unwrap();
        for o in [&old_a, &old_b] {
            fs::create_dir_all(o).unwrap();
            fs::write(o.join(format!("{rel}.nzb")), &old_nzb).unwrap();
        }
        // nouveau post complet
        let w = waiting_dir(&cfg).unwrap().join(rel);
        fs::create_dir_all(&w).unwrap();
        fs::write(w.join(format!("{rel}.nzb")), NZB_OK).unwrap();
        fs::write(w.join(format!("{rel}.xml")), "<MediaInfo/>").unwrap();
        let out = distribute(&cfg);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1.targets, vec!["A".to_string()], "B a déjà la release : pas de second dépôt");
        let a = state_dir(&cfg, "A", rel, UpState::Pending).unwrap();
        assert_eq!(fs::read_to_string(a.join(format!("{rel}.nzb"))).unwrap(), NZB_OK, "le NZB déposé est celui du nouveau post");
        assert!(has_report(&a));
        let replaced = |idx: &str| fs::read_dir(indexer_dir(&cfg, idx).unwrap().join(REPLACED_DIR)).unwrap().flatten().map(|e| prep::file_name(&e.path())).collect::<Vec<_>>();
        assert!(replaced("A").iter().any(|n| n.starts_with(&format!("{rel} (ancien"))), "{:?}", replaced("A"));
        assert!(replaced("B").iter().any(|n| n.starts_with(&format!("{rel} (nouveau post"))));
        assert!(old_b.join(format!("{rel}.nzb")).is_file());
        assert!(!w.exists());
        // nouvel essai après un échec partiel : même NZB, rien n'est refait
        fs::create_dir_all(&w).unwrap();
        fs::write(w.join(format!("{rel}.nzb")), NZB_OK).unwrap();
        assert!(distribute(&cfg).is_empty());
        assert_eq!(replaced("A").len(), 1);
        // le nouveau post gardé à part sur B remplace l'ancien, d'un clic
        assert!(list_releases(&cfg).iter().any(|r| r.name == rel && r.newer == vec!["B".to_string()]));
        let p = promote_newer(&cfg, rel, "B").unwrap();
        assert_eq!(fs::read_to_string(p.join(format!("{rel}.nzb"))).unwrap(), NZB_OK);
        assert!(!old_b.exists() && newer_post(&cfg, rel, "B").is_none());
        // envoi forcé (« Envoyer quand même ») : remplace directement l'ancienne déjà uploadée
        set_state(&cfg, rel, "B", UpState::Pending, UpState::Done).unwrap();
        let nzb3 = NZB_OK.replace("s3cr3t", "troisieme");
        fs::create_dir_all(&w).unwrap();
        fs::write(w.join(format!("{rel}.nzb")), &nzb3).unwrap();
        fs::write(w.join(format!("{rel}.xml")), "<MediaInfo/>").unwrap();
        write_plan(&cfg, rel, &Plan { action: "post".into(), targets: vec!["B".into()], api: true, compress: None, redeposit: true, cats: Vec::new(), groups: String::new() }).unwrap();
        let out = distribute(&cfg);
        assert_eq!(out.first().map(|o| o.1.targets.clone()), Some(vec!["B".to_string()]));
        let b = state_dir(&cfg, "B", rel, UpState::Pending).unwrap();
        assert_eq!(fs::read_to_string(b.join(format!("{rel}.nzb"))).unwrap(), nzb3);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn circuit_complet_par_indexer() {
        let d = std::env::temp_dir().join(format!("nzbff_idx_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let (nzb, root) = (d.join("nzb"), d.join("Indexers NZB"));
        fs::create_dir_all(&nzb).unwrap();
        let cfg = Config {
            nzb_dir: nzb.display().to_string(),
            indexer_dir: root.display().to_string(),
            mi_dest: "indexer".into(),
            indexers: vec![
                prep::IndexerCfg::new("Idx A"),
                prep::IndexerCfg::new("Idx B"),
            ],
            ..Default::default()
        };
        // création automatique des dossiers
        ensure_dirs(&cfg).unwrap();
        assert!(root.join(WAITING_DIR).is_dir() && root.join("Idx A").is_dir() && root.join("Idx B").is_dir());
        // ancienne structure migrée vers l'attente
        fs::create_dir_all(root.join("Vieux.2000.FRENCH.720p.x264")).unwrap();
        fs::write(root.join("Vieux.2000.FRENCH.720p.x264/Vieux.2000.FRENCH.720p.x264.xml"), "<x/>").unwrap();
        migrate_layout(&cfg);
        assert!(root.join(WAITING_DIR).join("Vieux.2000.FRENCH.720p.x264").is_dir());
        // le rapport attend dans « _En attente du NZB »
        let rel = "R.2001.FRENCH.1080p.x264";
        let rep = prep::report_path(&cfg, Path::new("/sortie"), rel);
        assert_eq!(rep, root.join(WAITING_DIR).join(rel).join(format!("{rel}.xml")));
        fs::create_dir_all(rep.parent().unwrap()).unwrap();
        fs::write(&rep, "<x/>").unwrap();
        let rels = list_releases(&cfg);
        let r = rels.iter().find(|r| r.name == rel).unwrap();
        assert!(r.waiting.is_some() && !r.nzb && r.states.is_empty());
        // NgPost a fini : NZB rapatrié puis distribué à chaque indexer
        // NZB encore en cours d'écriture : ni rapatrié ni distribué
        old_file(&nzb.join(format!("{rel}.mkv.nzb")), NZB_PARTIEL);
        assert!(collect_nzbs(&cfg, &|_| true).is_empty());
        old_file(&nzb.join(format!("{rel}.mkv.nzb")), NZB_OK);
        old_file(&nzb.join("Inconnu.nzb"), NZB_OK);
        assert_eq!(collect_nzbs(&cfg, &|_| false).len(), 1);
        assert!(nzb.join("Inconnu.nzb").exists());
        assert_eq!(distribute(&cfg).into_iter().map(|d| d.0).collect::<Vec<_>>(), vec![rel.to_string()]);
        for i in ["Idx A", "Idx B"] {
            assert!(root.join(i).join(PENDING_DIR).join(rel).join(format!("{rel}.nzb")).is_file());
            assert!(root.join(i).join(PENDING_DIR).join(rel).join(format!("{rel}.xml")).is_file());
        }
        assert!(!root.join(WAITING_DIR).join(rel).exists());
        let r = list_releases(&cfg).into_iter().find(|r| r.name == rel).unwrap();
        assert!(r.pending_on("Idx A") && r.pending_on("Idx B") && r.nzb && r.report.as_deref() == Some("xml"));
        // déposée sur A, pas pour B
        set_state(&cfg, rel, "Idx A", UpState::Pending, UpState::Done).unwrap();
        set_state(&cfg, rel, "Idx B", UpState::Pending, UpState::Done).unwrap();
        set_state(&cfg, rel, "Idx B", UpState::Done, UpState::Skip).unwrap();
        assert!(root.join("Idx A").join(UPLOADED_DIR).join(rel).is_dir());
        assert!(root.join("Idx B").join(SKIPPED_DIR).join(rel).is_dir());
        let r = list_releases(&cfg).into_iter().find(|r| r.name == rel).unwrap();
        assert!(r.finished() && !r.any_pending());
        // retour à « à déposer »
        set_state(&cfg, rel, "Idx B", UpState::Skip, UpState::Pending).unwrap();
        assert!(list_releases(&cfg).into_iter().find(|r| r.name == rel).unwrap().pending_on("Idx B"));
        // une release déjà distribuée n'est pas reprise si NgPost réécrit le NZB
        old_file(&nzb.join(format!("{rel}.nzb")), NZB_OK);
        assert!(collect_nzbs(&cfg, &|_| true).iter().all(|(r, _)| r != rel));
        assert!(nzb.join(format!("{rel}.nzb")).exists());
        // un rapport déplacé est retrouvé chez l'indexer (bouton « Afficher »)
        let moved = root.join(WAITING_DIR).join(rel).join(format!("{rel}.xml"));
        assert_eq!(locate(&cfg, &moved), root.join("Idx A").join(UPLOADED_DIR).join(rel).join(format!("{rel}.xml")));
        // renommage d'un indexer : son dossier suit
        rename_indexer_dir(&cfg, "Idx B", "Idx C").unwrap();
        assert!(root.join("Idx C").join(PENDING_DIR).join(rel).is_dir() && !root.join("Idx B").exists());
        let _ = fs::remove_dir_all(&d);
    }

    /// NZB déjà en sa possession importé et déposé ; release déjà postée envoyée sur un nouvel
    /// indexer (copiée depuis celui où elle est déjà déposée), jamais deux fois.
    #[test]
    fn nzb_importe_et_release_vers_nouvel_indexer() {
        let d = std::env::temp_dir().join(format!("nzbff_import_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let ix = |n: &str| prep::IndexerCfg::new(n);
        let cfg = Config { indexer_dir: d.join("idx").display().to_string(), indexers: vec![ix("A"), ix("Nouveau")], ..Default::default() };
        ensure_dirs(&cfg).unwrap();
        // NZB à côté de son rapport et de son NFO
        let src = d.join("mes nzb");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("Film.2020.FRENCH.1080p.WEB.x264-GRP.nzb"), NZB_OK).unwrap();
        fs::write(src.join("Film.2020.FRENCH.1080p.WEB.x264-GRP.xml"), "<MediaInfo/>").unwrap();
        fs::write(src.join("film.nfo"), "NFO").unwrap();
        let (mi, nfo) = nzb_companions(&src.join("Film.2020.FRENCH.1080p.WEB.x264-GRP.nzb"));
        assert!(mi.is_some() && nfo.is_some());
        let rel = "Film.2020.FRENCH.1080p.WEB.x264-GRP";
        let dir = import_nzb(&cfg, &src.join(format!("{rel}.nzb")), rel, mi.as_deref(), nfo.as_deref(), false, false).unwrap();
        let p = send_to_indexer(&cfg, &dir, rel, "A", false).unwrap();
        let f = crate::upload::release_files(&p).unwrap();
        assert!(f.mediainfo.is_some() && f.nfo.is_some() && f.nfo_original);
        // sans rapport : refusé, sauf accord explicite (et alors le dépôt ne le réclame pas)
        fs::write(src.join("Autre.nzb"), NZB_OK).unwrap();
        assert!(import_nzb(&cfg, &src.join("Autre.nzb"), "Autre", None, None, false, false).is_err());
        let a = import_nzb(&cfg, &src.join("Autre.nzb"), "Autre", None, None, false, true).unwrap();
        assert!(!report_required(&a));
        // NZB incomplet : refusé
        fs::write(src.join("Partiel.nzb"), NZB_PARTIEL).unwrap();
        assert!(import_nzb(&cfg, &src.join("Partiel.nzb"), "Partiel", None, None, false, true).is_err());
        // déjà déposée sur A → envoyée sur le nouvel indexer depuis A
        set_state(&cfg, rel, "A", UpState::Pending, UpState::Done).unwrap();
        let from = release_source(&cfg, rel).unwrap();
        assert!(from.ends_with(Path::new(UPLOADED_DIR).join(rel)), "{}", from.display());
        let n = send_to_indexer(&cfg, &from, rel, "Nouveau", false).unwrap();
        assert!(nzb_in(&n).is_some() && n.join(format!("{rel}.xml")).is_file());
        assert!(send_to_indexer(&cfg, &from, rel, "Nouveau", false).unwrap_err().contains("déjà à déposer"));
        assert!(send_to_indexer(&cfg, &from, rel, "A", false).unwrap_err().contains("déjà déposée"));
        // nouvel envoi voulu sur A : repart de « déjà uploadés »
        assert!(send_to_indexer(&cfg, &from, rel, "A", true).is_ok());
        let _ = fs::remove_dir_all(&d);
    }

    /// Releases déjà postées retrouvées partout : indexer actif, désactivé, ou ancien dossier
    /// d'un indexer renommé ; renvoyées depuis là.
    #[test]
    fn catalogue_des_releases_deja_postees() {
        let d = std::env::temp_dir().join(format!("nzbff_catalogue_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let ix = |n: &str, on: bool| prep::IndexerCfg { active: on, ..prep::IndexerCfg::new(n) };
        let cfg = Config { indexer_dir: d.display().to_string(), indexers: vec![ix("altHUB", true), ix("Pause", false), ix("Nouveau", true)], ..Default::default() };
        let put = |dir: PathBuf| {
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("x.nzb"), NZB_OK).unwrap();
            fs::write(dir.join("x.xml"), "<MediaInfo/>").unwrap();
        };
        put(d.join("altHUB").join(UPLOADED_DIR).join("Film.A"));
        put(d.join("Pause").join(UPLOADED_DIR).join("Film.B"));
        put(d.join("Ancien Nom").join(UPLOADED_DIR).join("Film.C"));
        fs::create_dir_all(d.join("altHUB").join(UPLOADED_DIR).join("Sans.NZB")).unwrap();
        let c = catalog(&cfg);
        let names: Vec<&str> = c.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names.len(), 3, "{names:?}");
        let a = c.iter().find(|p| p.name == "Film.A").unwrap();
        assert!(a.done_on("altHUB") && !a.on("Nouveau"));
        let b = c.iter().find(|p| p.name == "Film.B").unwrap();
        assert!(b.done_on("Pause"), "indexer désactivé compris");
        let cc = c.iter().find(|p| p.name == "Film.C").unwrap();
        assert!(!cc.copies[0].known && cc.copies[0].place == "Ancien Nom");
        // renvoi sur le nouvel indexer depuis l'ancien dossier
        let src = release_source(&cfg, "Film.C").unwrap();
        let n = send_to_indexer(&cfg, &src, "Film.C", "Nouveau", false).unwrap();
        assert!(nzb_in(&n).is_some());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn indexers_choisis_et_nzb_partiel() {
        let d = std::env::temp_dir().join(format!("nzbff_plan_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let ix = |n: &str| prep::IndexerCfg { url: format!("https://{n}.example/upload"), ..prep::IndexerCfg::new(n) };
        let cfg = Config { indexer_dir: d.display().to_string(), indexers: vec![ix("A"), ix("B"), ix("C")], ..Default::default() };
        ensure_dirs(&cfg).unwrap();
        let w = d.join(WAITING_DIR);
        // R1 : uniquement A et C, tout automatique
        write_plan(&cfg, "R1", &Plan { action: "auto".into(), targets: vec!["A".into(), "C".into()], api: true, compress: None, redeposit: false, cats: vec![("C".into(), "5342".into())], groups: String::new() }).unwrap();
        assert!(read_plan(&cfg, "R1").unwrap().api);
        fs::write(w.join("R1/R1.xml"), "<x/>").unwrap();
        // NZB partiel (NgPost en train de poster) : rien ne bouge
        fs::write(w.join("R1/R1.nzb"), NZB_PARTIEL).unwrap();
        assert!(distribute(&cfg).is_empty());
        let r = list_releases(&cfg).into_iter().find(|r| r.name == "R1").unwrap();
        assert!(!r.nzb && r.nzb_error.is_some() && r.plan.as_ref().unwrap().auto());
        fs::write(w.join("R1/R1.nzb"), NZB_OK).unwrap();
        let out = distribute(&cfg);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1.resolve(&cfg), vec!["A", "C"]);
        assert!(d.join("A").join(PENDING_DIR).join("R1/R1.nzb").is_file() && d.join("C").join(PENDING_DIR).join("R1/R1.xml").is_file());
        assert!(d.join("B").join(SKIPPED_DIR).join("R1/R1.nzb").is_file());
        assert!(!d.join("A").join(PENDING_DIR).join("R1").join(PLAN_FILE).exists());
        // catégorie choisie à la main : suit la release chez l'indexer
        assert_eq!(read_cat_force(&d.join("C").join(PENDING_DIR).join("R1")), vec![("C".to_string(), "5342".to_string())]);
        assert_eq!(upload_url(&cfg, "C").as_deref(), Some("https://C.example/upload"));
        // sans plan : tous les indexers actifs
        fs::create_dir_all(w.join("R2")).unwrap();
        fs::write(w.join("R2/R2.nzb"), NZB_OK).unwrap();
        distribute(&cfg);
        assert!(["A", "B", "C"].iter().all(|i| d.join(i).join(PENDING_DIR).join("R2/R2.nzb").is_file()));
        // ancienne structure (release directement dans le dossier de l'indexer) migrée
        fs::create_dir_all(d.join("B/Vieille.Release")).unwrap();
        fs::write(d.join("B/Vieille.Release/Vieille.Release.nzb"), NZB_OK).unwrap();
        migrate_layout(&cfg);
        assert!(d.join("B").join(PENDING_DIR).join("Vieille.Release/Vieille.Release.nzb").is_file());
        assert!(list_releases(&cfg).iter().find(|r| r.name == "Vieille.Release").unwrap().pending_on("B"));
        let _ = fs::remove_dir_all(&d);
    }

    /// Faux serveur Newznab : renvoie `reply` et garde la requête reçue.
    fn fake_newznab(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        fake_server(vec![(200, reply)])
    }

    /// Faux indexer : une réponse (code HTTP, corps) par connexion, dans l'ordre.
    /// Renvoie l'adresse et, à la fin, toutes les requêtes reçues.
    fn fake_server(replies: Vec<(u16, &'static str)>) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let mut all = String::new();
            for (code, reply) in replies {
                let (mut c, _) = l.accept().unwrap();
                c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut req = Vec::new();
                let mut buf = [0u8; 8192];
                loop {
                    let n = c.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                    let t = String::from_utf8_lossy(&req).to_string();
                    if let Some(he) = t.find("\r\n\r\n") {
                        let len = t.to_lowercase().split("content-length:").nth(1).and_then(|x| x.lines().next()).and_then(|x| x.trim().parse::<usize>().ok()).unwrap_or(0);
                        if t.to_lowercase().contains("expect: 100-continue") && req.len() == he + 4 {
                            let _ = c.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
                            continue;
                        }
                        if req.len() >= he + 4 + len {
                            break;
                        }
                    }
                }
                let resp = format!("HTTP/1.1 {code} X\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", reply.len(), reply);
                let _ = c.write_all(resp.as_bytes());
                all.push_str(&String::from_utf8_lossy(&req));
                all.push_str("\n=====\n");
            }
            all
        });
        (addr, h)
    }

    /// Erreur 502 pendant un gros envoi : l'indexer a quand même enregistré la release.
    /// Pas de second envoi : une recherche le confirme. Introuvable : pas de renvoi non plus.
    #[test]
    fn reponse_incertaine_pas_de_doublon() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_incertain_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let rel = "Serie.S03.FRENCH.1080p.H264";
        let rss: &'static str = "<?xml version=\"1.0\"?><rss><channel><title>Indexer</title><item><title>Serie S03 FRENCH 1080p H264</title></item></channel></rss>";
        let (addr, h) = fake_server(vec![(502, "<html>Bad Gateway</html>"), (200, rss)]);
        let api = prep::IndexerCfg { mode: "newznab".into(), api_url: addr.clone(), api_key: "CLE-SECRETE".into(), ..prep::IndexerCfg::new("API") };
        let cfg = Config { indexer_dir: d.display().to_string(), indexers: vec![api], ..Default::default() };
        let p = state_dir(&cfg, "API", rel, UpState::Pending).unwrap();
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join(format!("{rel}.nzb")), NZB_OK).unwrap();
        fs::write(p.join(format!("{rel}.xml")), "<MediaInfo/>").unwrap();
        let m = api_upload(&cfg, rel, "API").unwrap();
        let reqs = h.join().unwrap();
        assert!(m.contains("reçue par l'indexer"), "{m}");
        assert_eq!(reqs.matches("t=nzbadd").count(), 1, "un seul envoi");
        assert!(reqs.contains("t=search"));
        assert!(state_dir(&cfg, "API", rel, UpState::Done).unwrap().is_dir() && !p.exists());
        // introuvable après vérification : erreur, aucun nouvel essai programmé
        let rel2 = "Serie.S04.FRENCH.1080p.H264";
        let (addr2, h2) = fake_server(vec![(504, "Gateway Timeout"), (200, "<rss><channel></channel></rss>")]);
        let mut cfg2 = cfg.clone();
        cfg2.indexers[0].api_url = addr2;
        let p2 = state_dir(&cfg2, "API", rel2, UpState::Pending).unwrap();
        fs::create_dir_all(&p2).unwrap();
        fs::write(p2.join(format!("{rel2}.nzb")), NZB_OK).unwrap();
        fs::write(p2.join(format!("{rel2}.xml")), "<MediaInfo/>").unwrap();
        let e = api_upload(&cfg2, rel2, "API").unwrap_err();
        let _ = h2.join();
        assert!(e.retry_after.is_none() && e.message.contains("pas de nouvel envoi automatique"), "{}", e.message);
        assert!(p2.is_dir() && !p2.join(API_LOCK).exists(), "reste à uploader, verrou retiré");
        // un envoi déjà en cours (autre fenêtre) : refusé
        fs::write(p2.join(API_LOCK), "").unwrap();
        assert!(api_upload(&cfg2, rel2, "API").unwrap_err().message.contains("déjà en cours d'envoi"));
        let _ = fs::remove_dir_all(&d);
    }

    /// Groupes choisis selon le type de post : contenu, série, film.
    #[test]
    fn groupes_selon_le_type() {
        let d = std::env::temp_dir().join(format!("nzbff_groupes_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let mut cfg = Config { indexer_dir: d.display().to_string(), ng_groups: "alt.binaries.misc".into(), groups_film: "alt.binaries.movies".into(), groups_tv: "alt.binaries.teevee".into(), ..Default::default() };
        for c in cfg.content_cats.iter_mut().filter(|c| c.key == "musique") {
            c.groups = "alt.binaries.sounds.lossless".into();
        }
        assert_eq!(groups_for(&cfg, "Film.2020.FRENCH.1080p.WEB.x264").0, "alt.binaries.movies");
        assert_eq!(groups_for(&cfg, "Ma.Serie.S01.FRENCH.1080p.H264").0, "alt.binaries.teevee");
        let musique = cfg.content_cats.iter().find(|c| c.key == "musique").unwrap().clone();
        crate::content::write_cat(&cfg, "Artiste-Album-2024-FLAC", &musique);
        assert_eq!(groups_for(&cfg, "Artiste-Album-2024-FLAC"), ("alt.binaries.sounds.lossless".to_string(), "Musique".to_string()));
        // vide : les groupes par défaut des options du post
        cfg.groups_film.clear();
        assert_eq!(groups_for(&cfg, "Film.2020.FRENCH.1080p.WEB.x264").0, "");
        let g = crate::content::group_suggestions("tv");
        assert!(g.contains(&"alt.binaries.teevee"));
        let _ = fs::remove_dir_all(&d);
    }

    /// La clé API ne passe jamais sur la ligne de commande de curl.
    #[test]
    fn cle_api_hors_ligne_de_commande() {
        let api = prep::IndexerCfg { mode: "newznab".into(), api_url: "https://idx.example/api".into(), api_key: "CLE-SECRETE".into(), auth: "header".into(), ..prep::IndexerCfg::new("API") };
        let f = crate::upload::Files { nzb: PathBuf::from("r.nzb"), ..Default::default() };
        for args in [crate::upload::nzbadd_args(&api, &f, "R"), crate::upload::rest_args(&api.rest_view(), &f, "R")] {
            let (plain, conf) = crate::upload::split_secret_args(&args);
            assert!(!plain.join(" ").contains("CLE-SECRETE"), "{plain:?}");
            assert!(conf.starts_with("url = \"https://") || conf.contains("header = "), "{conf}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn depot_par_api_newznab() {
        if Command::new("curl").arg("--version").output().is_err() {
            return;
        }
        let d = std::env::temp_dir().join(format!("nzbff_api_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let (addr, h) = fake_newznab(r#"<?xml version="1.0"?><success ID="42" guid="abc123" categoryID="2040" name="Film.2020.FRENCH.1080p.x264" />"#);
        let api = prep::IndexerCfg { mode: "newznab".into(), api_url: addr.clone(), api_key: "CLE-SECRETE".into(), send_nfo: true, ..prep::IndexerCfg::new("API") };
        let cfg = Config { indexer_dir: d.display().to_string(), indexers: vec![api, prep::IndexerCfg::new("Web")], api_upload: true, ..Default::default() };
        ensure_dirs(&cfg).unwrap();
        let rel = "Film.2020.FRENCH.1080p.x264";
        let w = d.join(WAITING_DIR).join(rel);
        fs::create_dir_all(&w).unwrap();
        fs::write(w.join(format!("{rel}.nzb")), NZB_OK).unwrap();
        fs::write(w.join(format!("{rel}.xml")), "<MediaInfo/>").unwrap();
        fs::write(w.join(format!("{rel}.nfo")), "NFO").unwrap();
        write_plan(&cfg, rel, &Plan::from_cfg(&cfg)).unwrap();
        let (r, plan) = distribute(&cfg).pop().unwrap();
        assert_eq!(api_targets(&cfg, &plan), vec!["API"]); // l'indexer « Web » reste manuel
        let msg = api_upload(&cfg, &r, "API").unwrap();
        assert!(msg.contains("accepté"), "{msg}");
        let req = h.join().unwrap();
        assert!(req.starts_with("POST /api?t=nzbadd&apikey=CLE-SECRETE&cat=2040&includemeta=true&dupecheck=true "), "{}", &req[..200.min(req.len())]);
        for f in ["name=\"file\"; filename=", "name=\"nzbname\"", "Film.2020.FRENCH.1080p.x264", "name=\"mediainfo\"", "name=\"nfo\""] {
            assert!(req.contains(f), "{f} absent");
        }
        // déposée : rangée dans « _Déjà uploadés » avec la note
        let done = d.join("API").join(UPLOADED_DIR).join(rel);
        assert!(done.join(format!("{rel}.nzb")).is_file() && fs::read_to_string(done.join(API_NOTE)).unwrap().contains("abc123"));
        assert!(d.join("Web").join(PENDING_DIR).join(rel).is_dir());
        // le détail du dépôt (requête, réponse, catégorie) part au Journal
        let detail = take_api_detail();
        assert!(detail.iter().any(|l| l.contains("POST ") && l.contains("t=nzbadd") && !l.contains("CLE-SECRETE")), "{detail:?}");
        assert!(detail.iter().any(|l| l.contains("← HTTP 200")) && detail.iter().any(|l| l.contains("catégorie attribuée par l'indexer : 2040")), "{detail:?}");
        // refus de l'indexer : la release reste à uploader, la clé n'apparaît pas dans l'erreur
        let rel2 = "Autre.2021.FRENCH.720p.x264";
        let p2 = d.join("API").join(PENDING_DIR).join(rel2);
        fs::create_dir_all(&p2).unwrap();
        fs::write(p2.join(format!("{rel2}.nzb")), NZB_OK).unwrap();
        fs::write(p2.join(format!("{rel2}.xml")), "<MediaInfo/>").unwrap();
        let (addr2, h2) = fake_newznab(r#"<error code="100" description="Incorrect user credentials"/>"#);
        let mut cfg2 = cfg.clone();
        cfg2.indexers[0].api_url = format!("{addr2}/api");
        let e = api_upload(&cfg2, rel2, "API").unwrap_err().message;
        let _ = h2.join();
        assert!(e.contains("Incorrect user credentials") && !e.contains("CLE-SECRETE"), "{e}");
        assert!(p2.is_dir());
        // doublon : l'indexer garde ce qu'il avait (catégorie comprise), dit clairement
        let (addr3, h3) = fake_newznab(r#"<error code="601" description="Duplicate NZB"/>"#);
        let mut cfg3 = cfg.clone();
        cfg3.indexers[0].api_url = format!("{addr3}/api");
        fs::write(p2.join(format!("{rel2}.xml")), "<MediaInfo/>").unwrap();
        let m = api_upload(&cfg3, rel2, "API").unwrap();
        let _ = h3.join();
        assert!(m.contains("doublon") && m.contains("rien changé"), "{m}");
        assert!(take_api_detail().iter().any(|l| l.contains("supprime-la sur le site")));
        let _ = fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn depot_par_api_rest() {
        if Command::new("curl").arg("--version").output().is_err() {
            return;
        }
        let d = std::env::temp_dir().join(format!("nzbff_rest_e2e_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let (addr, h) = fake_newznab(r#"{"id": 9876, "guid": "a1b2c3", "message": "NZB uploaded"}"#);
        let rest = prep::IndexerCfg { mode: "rest".into(), rest_url: format!("{addr}/api/v1"), api_key: "CLE-REST".into(), cat: "-1".into(), ..prep::IndexerCfg::new("REST") };
        let cfg = Config { indexer_dir: d.display().to_string(), indexers: vec![rest], ..Default::default() };
        let rel = "Film.2020.FRENCH.1080p.x264";
        let p = d.join("REST").join(PENDING_DIR).join(rel);
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join(format!("{rel}.nzb")), NZB_OK).unwrap();
        fs::write(p.join(format!("{rel}.xml")), "<MediaInfo/>").unwrap();
        let msg = api_upload(&cfg, rel, "REST").unwrap();
        assert!(msg.contains("NZB uploaded"), "{msg}");
        let req = h.join().unwrap();
        let low = req.to_lowercase();
        assert!(req.starts_with("POST /api/v1/nzb "), "{}", &req[..120.min(req.len())]);
        assert!(low.contains("x-api-key: cle-rest"), "en-tête de clé absent");
        for f in ["name=\"file\"; filename=", "name=\"cat\"", "-1", "name=\"nzbname\"", "name=\"mediainfo\""] {
            assert!(req.contains(f), "{f} absent");
        }
        assert!(d.join("REST").join(UPLOADED_DIR).join(rel).join(API_NOTE).is_file());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn sans_indexer_reste_en_attente() {
        let d = std::env::temp_dir().join(format!("nzbff_noidx_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let cfg = Config { indexer_dir: d.display().to_string(), ..Default::default() };
        ensure_dirs(&cfg).unwrap();
        fs::create_dir_all(d.join(WAITING_DIR).join("R")).unwrap();
        fs::write(d.join(WAITING_DIR).join("R/R.nzb"), "").unwrap();
        assert!(distribute(&cfg).is_empty());
        let _ = fs::remove_dir_all(&d);
    }
}

#[cfg(all(test, unix))]
mod tests_arret {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// « Arrêter » pendant la compression : NgPost et son compresseur arrêtés, archives
    /// temporaires retirées, la vidéo reste à poster. Et pas de place : NgPost n'est pas lancé.
    #[test]
    fn arret_du_post_et_manque_de_place() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_arret_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        fs::create_dir_all(d.join("tmp")).unwrap();
        fs::create_dir_all(d.join("tmp").join("autre-chose")).unwrap();
        let exe = d.join("ngPost");
        // faux NgPost : crée son archive dans le dossier temporaire puis « compresse » longtemps
        fs::write(&exe, format!("#!/bin/sh\nmkdir -p '{0}/xYz12'\necho data > '{0}/xYz12/xYz12.part01.rar'\necho 'Compressing files with rar'\nsleep 30 &\nwait\n", d.join("tmp").display())).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        let video = d.join("sortie/R.2001.FRENCH.1080p.x264.mkv");
        fs::write(&video, vec![0u8; 1000]).unwrap();
        let cfg = Config {
            ngpost_exe: exe.display().to_string(),
            ngpost_dir: d.join("sortie").display().to_string(),
            ng_compress: true,
            ng_par2: true,
            ng_tmp_dir: d.join("tmp").display().to_string(),
            ..Default::default()
        };
        let t0 = std::time::Instant::now();
        let stopper = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(1200));
            cancel_post();
        });
        let e = post_file(&cfg, &video, "R", &mut |_| {}, &mut |_| {}).unwrap_err();
        stopper.join().unwrap();
        assert!(e.contains(POST_CANCELLED), "{e}");
        assert!(t0.elapsed() < Duration::from_secs(10), "l'arrêt doit être rapide");
        assert!(!d.join("tmp").join("xYz12").exists(), "archive temporaire non retirée");
        assert!(d.join("tmp").join("autre-chose").exists(), "un dossier qui n'est pas à nous a été touché");
        assert!(video.exists());
        // pas assez de place : refus immédiat, sans lancer NgPost
        crate::prep::FAKE_FREE.with(|c| c.set(Some(500)));
        assert_eq!(tmp_needed(&cfg, None, 1000), 1133); // archives 1000 + par2 10 %, + 3 %
        let e = post_file(&cfg, &video, "R", &mut |_| {}, &mut |_| {}).unwrap_err();
        crate::prep::FAKE_FREE.with(|c| c.set(None));
        assert!(e.starts_with(SPACE_ERR) && e.contains("NgPost n'a pas été lancé"), "{e}");
        assert!(is_space_error(&e) && is_space_error("System ERROR:\nEspace insuffisant sur le disque.\n=> rar exit code: 2"));
        let _ = fs::remove_dir_all(&d);
    }

    /// Échec réel (journal d'un utilisateur, 3.22) : 7-Zip OK, par2cmdline-turbo ne retrouve pas
    /// les volumes (chemin « D:\tmp/x/x.7z* »), code 3, pas de NZB. La vidéo ne doit JAMAIS être
    /// touchée, même réglée sur « supprimer » ou « corbeille » après le post, et l'erreur doit dire
    /// pourquoi et où est la vidéo ; aucun mot de passe dans ce qui est affiché ou écrit.
    #[test]
    fn echec_du_par2_rien_n_est_supprime() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_par2ko_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        fs::create_dir_all(d.join("ng")).unwrap();
        let exe = d.join("ng").join("ngPost");
        let mut sh = String::from("#!/bin/sh\necho 'Extra logs ON'\necho 'srv: news.example.com:563 user: moi pass: S3cr3tPw!'\n");
        sh.push_str("echo '[14:28:24.889] Génération des fichiers par2: par2.exe c -l -m1024 -r8 D:\\nzbff-tmp/jC/jC.par2 D:\\nzbff-tmp/jC/jC.7z* -q'\n");
        for k in 1..=9 {
            sh.push_str(&format!("echo 'Ignoring non-existent source file: D:\\jC.7z.00{k}'\n"));
        }
        sh.push_str("echo 'You must specify a list of files when creating.'\necho '=> par2 exit code: 3'\necho 'Erreur durant la génération des par2: 3'\necho 'Archives et par2 supprimés.'\nexit 1\n");
        fs::write(&exe, sh).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(d.join("ng").join("ngPost.conf"), "TMP_DIR = D:\\nzbff-tmp\n[server]\nhost = news.example.com\nuser = moi\npass = S3cr3tPw!\n").unwrap();
        for after in ["delete", "trash", "posted"] {
            let video = d.join("sortie/Mon.Film.2020.FRENCH.1080p.x264.mkv");
            fs::write(&video, vec![7u8; 4096]).unwrap();
            let cfg = Config { ngpost_exe: exe.display().to_string(), ngpost_dir: d.join("sortie").display().to_string(), ng_compress: true, ng_par2: true, post_after: after.into(), ..Default::default() };
            let mut shown: Vec<String> = Vec::new();
            let e = post_file(&cfg, &video, "Mon.Film.2020.FRENCH.1080p.x264", &mut |l| shown.push(l.to_string()), &mut |_| {}).unwrap_err();
            assert!(video.is_file(), "{after} : la vidéo a été touchée après un échec");
            assert_eq!(fs::metadata(&video).unwrap().len(), 4096);
            assert!(!d.join("sortie").join(POSTED_DIR).exists(), "{after} : rangée dans _Postés après un échec");
            assert!(e.contains("Rien n'a été posté") && e.contains("par2") && e.contains("Rien n'a été supprimé ni déplacé") && e.contains("Mon.Film.2020"), "{e}");
            // 9 lignes « non-existent » : 3 à l'écran
            assert_eq!(shown.iter().filter(|l| l.contains("non-existent")).count(), 3, "{shown:?}");
            // mot de passe du serveur jamais affiché ni écrit
            assert!(!shown.iter().any(|l| l.contains("S3cr3tPw")), "{shown:?}");
            assert!(!e.contains("S3cr3tPw"));
        }
        for f in fs::read_dir(logs_dir()).unwrap().flatten() {
            let t = fs::read_to_string(f.path()).unwrap_or_default();
            assert!(!t.contains("S3cr3tPw"), "mot de passe dans {}", f.path().display());
        }
        // la commande passe le dossier temporaire de ngPost.conf en « / »
        let conf = d.join("ng").join("ngPost.conf");
        let cfg = Config { ng_compress: true, ng_par2: true, ..Default::default() };
        let a = post_args(&cfg, Some(&conf), Path::new("v.mkv"), Path::new("v.nzb"));
        let k = a.iter().position(|x| x == "--tmp_dir").expect("--tmp_dir absent");
        assert_eq!(a[k + 1], "D:/nzbff-tmp");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn dossier_temporaire_pour_ngpost() {
        assert_eq!(ngpost_dir_arg("D:\\nzbff-tmp"), "D:/nzbff-tmp");
        assert_eq!(ngpost_dir_arg("D:\\nzbff-tmp\\"), "D:/nzbff-tmp");
        assert_eq!(ngpost_dir_arg("\"D:\\Mes temp\\post\""), "D:/Mes temp/post");
        assert_eq!(ngpost_dir_arg("D:\\"), "D:/");
        assert_eq!(ngpost_dir_arg("C:/ngPost/tmp"), "C:/ngPost/tmp");
        assert_eq!(ngpost_dir_arg("\\\\nas\\tmp"), "//nas/tmp");
        let cfg = Config { ng_par2: true, ng_tmp_dir: "\\\\nas\\tmp".into(), ..Default::default() };
        assert!(tmp_dir_problem(&cfg, None).is_some());
        assert!(tmp_dir_problem(&Config { ng_par2_tool: "parpar".into(), ..cfg.clone() }, None).is_none());
        assert!(tmp_dir_problem(&Config { ng_tmp_dir: "D:\\tmp".into(), ..cfg }, None).is_none());
        let c = Config { ng_compress: true, ng_tmp_dir: "D:\\tmp\\".into(), ..Default::default() };
        let a = post_args(&c, None, Path::new("v.mkv"), Path::new("v.nzb"));
        assert!(a.windows(2).any(|w| w[0] == "--tmp_dir" && w[1] == "D:/tmp"), "{a:?}");
    }

    /// Compression puis par2 sans que l'outil se nomme : le % repart de zéro, l'étape devient
    /// « Par2 » ; temps restant estimé pendant la compression.
    #[test]
    fn etapes_et_temps_restant() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_eta_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        let exe = d.join("ngPost");
        fs::write(
            &exe,
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do case $1 in -o) out=$2; shift;; esac; shift; done\necho 'Compressing files with rar'\nfor p in 5 20 35 50 65 80 95; do printf '  %s%%\\r' $p; sleep 0.6; done\nprintf '100%%\\n'\nfor p in 3 50 100; do printf 'Processing: %s%%\\r' $p; sleep 0.2; done\necho\nprintf '[==>  ] 50 %% (5 / 10) avg. speed:  10.00 MB/s\\n'\nprintf '<?xml version=\"1.0\"?>\\n<nzb>\\n<file poster=\"a\" date=\"1\" subject=\"s\"><groups><group>a</group></groups><segments><segment bytes=\"10\" number=\"1\">a@b</segment></segments></file>\\n</nzb>\\n' > \"$out\"\n",
        )
        .unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        let video = d.join("sortie/E.mkv");
        fs::write(&video, b"x").unwrap();
        let cfg = Config { ngpost_exe: exe.display().to_string(), ngpost_dir: d.join("sortie").display().to_string(), ng_compress: true, ng_par2: true, post_after: "keep".into(), ..Default::default() };
        let mut prog: Vec<Progress> = Vec::new();
        post_file(&cfg, &video, "E", &mut |_| {}, &mut |p| prog.push(p.clone())).unwrap();
        let phases: Vec<&str> = prog.iter().map(|p| p.phase.as_str()).collect();
        assert!(phases.contains(&"Compression") && phases.contains(&"Par2") && phases.contains(&"Envoi"), "{phases:?}");
        // jamais « Compression » après le passage aux par2
        let first_par2 = phases.iter().position(|p| *p == "Par2").unwrap();
        assert!(!phases[first_par2..].contains(&"Compression"), "{phases:?}");
        // temps restant pendant la compression (au rythme du pourcentage)
        assert!(prog.iter().any(|p| p.phase == "Compression" && p.eta.is_some()), "{prog:?}");
        assert!(progress_text(&Progress { phase: "Envoi".into(), pct: 45, cur_bps: Some(3.0 * 1048576.0), eta: Some(1500), ..Default::default() }).contains("45 % · 3 Mo/s · reste 25 min"));
        let _ = fs::remove_dir_all(&d);
    }

    /// Sortie réelle de NgPost 4.16 (7-Zip silencieux, par2cmdline) : étapes dans l'ordre, sans
    /// retour à « Compression » sur les lignes « Opening: xxx.7z.001 », avancement mesuré pendant
    /// la compression, écriture des par2 suivie.
    #[test]
    fn sortie_reelle_7z_par2() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_reel_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        fs::create_dir_all(d.join("tmp")).unwrap();
        let video = d.join("sortie/S.mkv");
        fs::write(&video, vec![0u8; 4_000_000]).unwrap();
        let t = d.join("tmp").display().to_string();
        let script = format!(
            "#!/bin/sh
while [ $# -gt 0 ]; do case $1 in -o) out=$2; shift;; esac; shift; done
echo 'Obfuscation des Articles (leur sujet sera remplacé par un UUID)'
echo '+ Ajout du dossier à compresser: S'
echo '[22:19:24.791] Compression des fichiers: C:\\Program Files\\7-Zip\\7z.exe a -mx0 -v1498m {t}/ab/ab.7z S -r'
mkdir -p {t}/ab
for i in 1 2 3; do head -c 1000000 /dev/zero > {t}/ab/ab.7z.00$i; sleep 1.2; done
echo 'Everything is Ok'
echo '=> rar exit code: 0'
echo '[22:27:14.924] Génération des fichiers par2: par2.exe c -l -m1024 -r8 {t}/ab/ab.par2 {t}/ab/ab.7z* -q'
echo 'Block size: 1000'
echo 'Recovery block count: 300'
echo 'Opening: ab.7z.001'
echo '0.1%'
echo 'Opening: ab.7z.002'
echo '50.0%'
echo 'Opening: ab.7z.003'
echo '100.0%'
echo 'Computing Reed Solomon matrix.'
echo 'Writing recovery packets'
echo 'Wrote 150000 bytes to disk'
echo 'Wrote 150000 bytes to disk'
echo '=> par2 exit code: 0'
echo '[23:25:14.258] Nombre de connexions disponibles: 100'
printf '[=====>    ] 10 %% (23435 / 234326) avg. speed:  16.61 MB/s\n'
echo 'Finishing posting...'
printf '<?xml version=\"1.0\"?>\n<nzb>\n<file poster=\"a\" date=\"1\" subject=\"s\"><groups><group>a</group></groups><segments><segment bytes=\"10\" number=\"1\">a@b</segment></segments></file>\n</nzb>\n' > \"$out\"
"
        );
        let exe = d.join("ngPost");
        fs::write(&exe, script).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        let cfg = Config { ngpost_exe: exe.display().to_string(), ngpost_dir: d.join("sortie").display().to_string(), ng_compress: true, ng_par2: true, ng_tmp_dir: t.clone(), post_after: "keep".into(), ..Default::default() };
        let mut prog: Vec<Progress> = Vec::new();
        post_file(&cfg, &video, "S", &mut |_| {}, &mut |p| prog.push(p.clone())).unwrap();
        let phases: Vec<String> = prog.iter().map(|p| p.phase.clone()).collect();
        // l'ordre des étapes ne recule jamais
        let ranks: Vec<u8> = phases.iter().map(|p| phase_rank(p)).collect();
        assert!(ranks.windows(2).all(|w| w[1] >= w[0]), "{phases:?}");
        // compression silencieuse : avancement mesuré sur l'archive écrite
        let comp: Vec<u32> = prog.iter().filter(|p| p.phase == "Compression").map(|p| p.pct).collect();
        assert!(comp.iter().any(|p| *p >= 20 && *p < 100), "{comp:?}");
        // par2 : calcul jusqu'à 90 %, puis écriture des données de réparation
        let par2: Vec<u32> = prog.iter().filter(|p| p.phase == "Par2").map(|p| p.pct).collect();
        assert!(par2.contains(&45) && par2.iter().any(|p| *p >= 95), "{par2:?}");
        assert!(phases.iter().any(|p| p == "Envoi"));
        let _ = fs::remove_dir_all(&d);
    }

    /// Série plus grosse que la place libre : compression en lots, un seul NZB, un seul mot de
    /// passe, fichiers remis à leur place.
    #[test]
    fn compression_en_lots_un_seul_nzb() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_lots_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let rel = d.join("sortie").join("Show.S01-S02.FRENCH.1080p-GRP");
        for (s, e) in [(1, 1), (1, 2), (2, 1)] {
            let p = rel.join(format!("Saison {s}")).join(format!("Show.S{s:02}E{e:02}.mkv"));
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, vec![0u8; 1000]).unwrap();
        }
        fs::create_dir_all(d.join("tmp")).unwrap();
        let exe = d.join("ngPost");
        // faux NgPost : un <file> par fichier de l'entrée (chemin relatif dans le sujet)
        fs::write(
            &exe,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{0}'\nl=$(mktemp)\nwhile [ $# -gt 0 ]; do case $1 in -o) out=$2; shift;; -i) echo \"$2\" >> $l; shift;; esac; shift; done\nprintf '<?xml version=\"1.0\"?>\\n<nzb>\\n<head><meta type=\"password\">x</meta></head>\\n' > \"$out\"\nwhile read i; do base=$(dirname \"$i\"); find \"$i\" -type f | sort | while read f; do r=${{f#$base/}}; printf '<file poster=\"a\" date=\"1\" subject=\"%s\"><groups><group>a.b</group></groups><segments><segment bytes=\"10\" number=\"1\">%s@b</segment></segments></file>\\n' \"$r\" \"$r\" >> \"$out\"; done; done < $l\necho '</nzb>' >> \"$out\"\n",
                d.join("args").display()
            ),
        )
        .unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        let cfg = Config {
            ngpost_exe: exe.display().to_string(),
            ngpost_dir: d.join("sortie").display().to_string(),
            ng_compress: true,
            ng_par2: true,
            ng_gen_pass: true,
            ng_lots: true,
            ng_tmp_dir: d.join("tmp").display().to_string(),
            post_after: "keep".into(),
            ..Default::default()
        };
        crate::prep::FAKE_FREE.with(|c| c.set(Some(2600))); // place pour 2 épisodes à la fois
        let mut lines = Vec::new();
        let p = post_file(&cfg, &rel, "Show.S01-S02.FRENCH.1080p-GRP", &mut |l| lines.push(l.to_string()), &mut |_| {});
        crate::prep::FAKE_FREE.with(|c| c.set(None));
        let p = p.unwrap();
        assert!(lines.iter().any(|l| l.contains("en 2 lots")), "{lines:?}");
        // un seul NZB, avec les fichiers de tous les lots et leur chemin Série/Saison
        let nzb = fs::read_to_string(&p.nzb).unwrap();
        assert_eq!(nzb.matches("<file ").count(), 3, "{nzb}");
        assert!(nzb.contains("Saison 2/Show.S02E01.mkv") && !nzb.contains("Show/Saison") && nzb.matches("</nzb>").count() == 1);
        assert_eq!(p.info.files, 3);
        // un seul mot de passe pour les deux lots, pas de mot de passe aléatoire par lot
        let args = fs::read_to_string(d.join("args")).unwrap();
        let pw: Vec<&str> = args.lines().map(|l| l.split("--rar_pass ").nth(1).unwrap().split(' ').next().unwrap()).collect();
        assert_eq!(pw.len(), 2);
        assert_eq!(pw[0], pw[1]);
        assert!(!args.contains("--gen_pass") && args.lines().all(|l| l.contains(".nzbff-lots/lot") && l.contains("/Saison ")), "{args}");
        // tout est revenu à sa place, rien ne traîne
        assert!(rel.join("Saison 1/Show.S01E02.mkv").is_file() && rel.join("Saison 2/Show.S02E01.mkv").is_file());
        assert!(!rel.join(".nzbff-lots").exists());
        assert!(p.warnings[0].contains("2 lots"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn depot_et_groupes_coherents() {
        let cfg = Config { groups_film: "alt.binaries.movies".into(), ..Default::default() };
        let mut i = prep::IndexerCfg::new("A");
        i.url = "https://a/upload".into();
        let d = deposit_for(&cfg, &i, true, "auto");
        assert!(!d.api && d.text.contains("page d'upload") && d.text.contains("tout seuls"), "{d:?}");
        i.mode = "newznab".into();
        i.api_url = "https://a/api".into();
        let d = deposit_for(&cfg, &i, true, "post");
        assert!(!d.api && d.problem.as_deref() == Some("API Newznab réglée mais la clé API manque"), "{d:?}");
        i.api_key = "k".into();
        assert!(deposit_for(&cfg, &i, true, "post").api);
        let off = deposit_for(&cfg, &i, false, "post");
        assert!(!off.api && off.text.contains("désactivé pour ce traitement"));
        // contenu sans groupes : ceux de sa catégorie, jamais ceux des films
        let (g, _, from) = groups_kind(&cfg, Some(("consoles", "Jeux consoles")), false, "GT4.PS2-GRP");
        assert!(g.contains("alt.binaries.games") && !g.contains("movies") && from.contains("conseillés"), "{g}");
        let (g, _, _) = groups_kind(&cfg, Some(("consoles", "Jeux consoles")), false, "Halo.3.X360-GRP");
        assert!(g.contains("alt.binaries.games.xbox360"), "{g}");
        let (g, _, _) = groups_kind(&cfg, None, false, "Film.2020.1080p-G");
        assert_eq!(g, "alt.binaries.movies");
        let (g, from) = groups_used(&Config::default(), None, None, false, "Film.2020.1080p-G");
        assert_eq!((g.as_str(), from.as_str()), ("groupe par défaut de NgPost", "rien de réglé"));
    }


    /// Groupes exigés par l'indexer (capacités `<groups>`) : signalés, ou ajoutés au post.
    #[test]
    fn groupes_indexes_ajoutes_ou_signales() {
        let mut a = prep::IndexerCfg::new("A");
        a.groups = vec!["alt.binaries.moovee".into(), "alt.binaries.teevee".into(), "alt.binaries.sounds.flac".into()];
        a.groups_rule = "add".into();
        let mut b = prep::IndexerCfg::new("B");
        b.groups = vec!["alt.binaries.hdtv".into()];
        b.groups_rule = "warn".into();
        let c = prep::IndexerCfg::new("C");
        let cfg = Config { indexers: vec![a, b, c], ..Default::default() };
        let (g, fits) = fit_groups(&cfg, "alt.binaries.misc", "film", &["A".into(), "B".into(), "C".into()], "", None);
        assert_eq!(g, "alt.binaries.misc,alt.binaries.moovee", "un groupe film de A ajouté");
        assert_eq!(fits.len(), 2, "C n'a pas publié de liste : rien à dire");
        assert!(fits[0].ok && fits[0].added.as_deref() == Some("alt.binaries.moovee"));
        assert!(!fits[1].ok && fits[1].added.is_none() && fits[1].suggest == vec!["alt.binaries.hdtv".to_string()]);
        let (g, fits) = fit_groups(&cfg, "Alt.Binaries.Teevee", "tv", &["A".into()], "", None);
        assert_eq!(g, "alt.binaries.teevee");
        assert!(fits[0].ok && fits[0].added.is_none(), "déjà indexé");
        assert_eq!(indexed_for(&cfg.indexers[0], "music"), vec!["alt.binaries.sounds.flac".to_string()]);
        assert_eq!(group_family(None, false, "Show.S01E01.1080p-X"), "tv");
        assert_eq!(group_family(Some(("musique", "Musique")), false, "x"), "music");
    }

    /// Groupes choisis à la main par catégorie de l'indexer : la catégorie où la release part
    /// décide (numéro exact, sinon sa famille 2000).
    #[test]
    fn groupes_par_categorie_de_l_indexer() {
        let mut a = prep::IndexerCfg::new("A");
        a.cat = "auto".into();
        a.cat_map = vec![("film.1080".into(), "2040".into()), ("film.uhd".into(), "2045".into()), ("tv.1080".into(), "5040".into())];
        a.group_rules = vec![("2045".into(), "alt.binaries.uhd,alt.binaries.hdtv.x264".into()), ("5000".into(), "alt.binaries.teevee".into())];
        a.groups = vec!["alt.binaries.moovee".into()];
        a.groups_rule = "add".into();
        let cfg = Config { indexers: vec![a.clone()], ..Default::default() };
        assert_eq!(group_rule_for(&a, "2045").map(|r| r.1.as_str()), Some("alt.binaries.uhd,alt.binaries.hdtv.x264"));
        assert_eq!(group_rule_for(&a, "5040").map(|r| r.0.as_str()), Some("5000"), "famille");
        assert!(group_rule_for(&a, "2040").is_none());
        assert!(group_rule_for(&a, "").is_none() && group_rule_for(&a, "-1").is_none());
        // UHD : la règle 2045 ajoute ses deux groupes
        let (g, fits) = fit_groups(&cfg, "alt.binaries.misc", "film", &["A".into()], "Film.2020.2160p.WEB.x265-GRP", None);
        assert_eq!(g, "alt.binaries.misc,alt.binaries.uhd,alt.binaries.hdtv.x264");
        assert_eq!(fits[0].rule.as_ref().map(|r| r.0.as_str()), Some("2045"));
        // 1080p : pas de règle → règle « ajouter » des groupes indexés
        let (g, fits) = fit_groups(&cfg, "alt.binaries.misc", "film", &["A".into()], "Film.2020.1080p.WEB.x264-GRP", None);
        assert_eq!(g, "alt.binaries.misc,alt.binaries.moovee");
        assert!(fits[0].rule.is_none());
        // série 1080p (5040) : famille 5000
        let (g, _) = fit_groups(&cfg, "alt.binaries.teevee", "tv", &["A".into()], "Show.S01E01.1080p.WEB-GRP", None);
        assert_eq!(g, "alt.binaries.teevee", "déjà dans le post");
        // plan : groupes choisis pour ce traitement, relus
        let p = parse_plan("action=post\ntargets=A\napi=true\nredeposit=false\ngroups=alt.binaries.x,alt.binaries.y\n");
        assert_eq!(p.groups, "alt.binaries.x,alt.binaries.y");
    }

}
