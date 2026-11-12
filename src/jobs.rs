//! Traitement d'un fichier de bout en bout (analyse → doublon → XML + transfert → historique),
//! et passage de surveillance sur l'inbox.

use crate::history::{History, Record, Status};
use crate::prep::{self, Analysis, Config, Stage, Transfer};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, OnceLock};

/// Fichiers et noms en cours de traitement (évite deux envois simultanés du même film).
fn in_flight() -> &'static Mutex<HashSet<String>> {
    static SET: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

struct Claim(Vec<String>);

impl Claim {
    fn take(keys: Vec<String>) -> Result<Claim, String> {
        let mut set = in_flight().lock().map_err(|_| "verrou".to_string())?;
        if keys.iter().any(|k| set.contains(k)) {
            return Err("déjà en cours de traitement".into());
        }
        set.extend(keys.iter().cloned());
        Ok(Claim(keys))
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if let Ok(mut set) = in_flight().lock() {
            for k in &self.0 {
                set.remove(k);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Auto,
    Manual,
    Batch,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            // origine du traitement (pas le mode du post, suivi à part dans l'historique)
            Mode::Auto => "Surveillance",
            Mode::Manual => "Traitement",
            Mode::Batch => "Lot",
        }
    }
}

fn record_from(a: &Analysis, mode: Mode) -> Record {
    let (langs, duration) = a
        .facts
        .as_ref()
        .map(|f| (f.langs.join(","), f.duration.clone()))
        .unwrap_or_default();
    Record {
        ts: prep::now_secs(),
        status: Status::Failed,
        mode: mode.label().into(),
        original: a.file_name(),
        release: a.name.clone().unwrap_or_default(),
        size: a.size,
        resolution: a.resolution.clone(),
        codec: a.codec.clone(),
        langs,
        duration,
        source_path: a.path.display().to_string(),
        dest: String::new(),
        xml: String::new(),
        reason: String::new(),
        post: String::new(),
    }
}

/// Raison de refuser un envoi en double (option anti-doublon), ou None. Doublon : la release a
/// déjà été postée (NZB produit), ou elle a été préparée et attend encore d'être postée. Une
/// release préparée puis abandonnée (jamais postée, plus là) n'est pas un doublon.
pub fn duplicate_reason(name: &str, hist: &History, posts: &[crate::stats::PostRec]) -> Option<String> {
    duplicate_reason_with(name, hist, posts, &|p: &Path| p.exists())
}

/// Même chose, avec le test d'existence fourni (l'interface ne lit pas le disque elle-même).
pub fn duplicate_reason_with(name: &str, hist: &History, posts: &[crate::stats::PostRec], exists: &dyn Fn(&Path) -> bool) -> Option<String> {
    if let Some(p) = posts.iter().rev().find(|p| p.release.eq_ignore_ascii_case(name)) {
        return Some(format!("« {} » déjà posté le {}", p.release, crate::history::format_ts(p.end)));
    }
    let r = hist.find_sent(name)?;
    let waiting = !r.dest.is_empty() && exists(Path::new(&r.dest));
    waiting.then(|| format!("« {} » déjà préparé le {} et pas encore posté ({})", r.release, crate::history::format_ts(r.ts), r.dest))
}

fn duplicate_check(name: &str, hist: &Mutex<History>) -> Result<(), String> {
    let posts = crate::stats::PostStats::load(&crate::app_dir()).posts;
    match hist.lock().ok().and_then(|h| duplicate_reason(name, &h, &posts)) {
        Some(why) => Err(format!("{why} (option anti-doublon)")),
        None => Ok(()),
    }
}

/// Le rapport MediaInfo suit toujours le NZB chez les indexers : rangé « avec la vidéo » ou
/// dans un dossier dédié, une copie va aussi dans le dossier d'attente de la release.
pub fn report_to_waiting(cfg: &Config, release: &str, report: &Path) {
    let Some(w) = crate::ngpost::waiting_dir(cfg) else { return };
    if report.as_os_str().is_empty() || !report.is_file() {
        return;
    }
    let d = w.join(release);
    if report.parent() == Some(d.as_path()) {
        return;
    }
    if std::fs::create_dir_all(&d).is_ok() {
        let _ = std::fs::copy(report, d.join(prep::file_name(report)));
    }
}

/// Rapports à créer seuls (sans renommer, déplacer ni poster).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReportsOnly {
    pub nfo: bool,
    pub mediainfo: bool,
    pub bdinfo: bool,
}

/// Crée seulement les rapports d'un fichier, d'un dossier ou d'un disque (Blu-ray, DVD, ISO),
/// à côté de lui (ou dans `out`) : NFO (MediaInfo en texte), MediaInfo (format des réglages),
/// BDInfo (Blu-ray). Un NFO déjà présent n'est jamais écrasé. Renvoie les fichiers créés et
/// les remarques.
pub fn reports_only(cfg: &Config, src: &Path, want: ReportsOnly, out: Option<&Path>, on_line: &mut dyn FnMut(&str)) -> Result<(Vec<PathBuf>, Vec<String>), String> {
    if !src.exists() {
        return Err(format!("{} introuvable", src.display()));
    }
    let disc = crate::disc::detect(src);
    let media = match &disc {
        Some(d) => d.image.clone(),
        None if src.is_dir() => crate::content::first_media(src).ok_or_else(|| format!("aucune vidéo ni piste audio lisible dans {}", src.display()))?,
        None => src.to_path_buf(),
    };
    let stem = if src.is_dir() { prep::file_name(src) } else { src.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default() };
    let name = prep::clean_name(&stem);
    let name = if name.is_empty() { stem } else { name };
    let dir = match out {
        Some(o) => o.to_path_buf(),
        None => src.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(".")),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("{} : {e}", dir.display()))?;
    let mut made = Vec::new();
    let mut notes = Vec::new();
    if want.mediainfo {
        on_line(&format!("MediaInfo de {}…", prep::file_name(&media)));
        let text = match &disc {
            Some(d) => crate::disc::mediainfo_export(cfg, d),
            None => prep::mediainfo_export(cfg, &media),
        }?;
        let p = dir.join(format!("{name}.{}", cfg.mi_ext()));
        std::fs::write(&p, text).map_err(|e| format!("{} : {e}", p.display()))?;
        made.push(p);
    }
    if want.nfo {
        on_line("NFO (MediaInfo en texte)…");
        let text = prep::mediainfo_text(cfg, &media, false)?;
        let mut p = dir.join(format!("{name}.nfo"));
        if p.exists() {
            notes.push(format!("« {} » existe déjà : gardé, le nouveau est « {name}.nzbff.nfo »", prep::file_name(&p)));
            p = dir.join(format!("{name}.nzbff.nfo"));
        }
        std::fs::write(&p, text).map_err(|e| format!("{} : {e}", p.display()))?;
        made.push(p);
    }
    if want.bdinfo {
        match &disc {
            Some(d) if d.kind == crate::disc::Kind::Bluray => {
                on_line("BDInfo (lecture du disque, plusieurs minutes possibles)…");
                let raw = crate::disc::bdinfo_report(cfg, d, on_line, &|| false)?;
                let p = dir.join(format!("{name}.bdinfo.txt"));
                std::fs::write(&p, raw).map_err(|e| format!("{} : {e}", p.display()))?;
                made.push(p);
            }
            Some(_) => notes.push("BDInfo : seulement pour un Blu-ray (ce disque est un DVD)".into()),
            None => notes.push("BDInfo : seulement pour un Blu-ray (dossier BDMV ou image ISO)".into()),
        }
    }
    Ok((made, notes))
}

/// Ligne du résumé envoyé par e-mail à la fin des traitements.
fn note_record(rec: &Record) {
    // fin du traitement (réussi, échoué ou arrêté) : plus de copie dans l'infobulle
    crate::tray::set_copy("");
    match rec.status {
        Status::Sent => crate::notify::record(format!("✔ Préparée : {} ({})", rec.release, prep::human_size(rec.size))),
        _ if prep::is_cancel(&rec.reason) => crate::notify::record(format!("⏹ Arrêtée : {}", rec.original)),
        _ => crate::notify::record(format!("✖ Échec du traitement : {}, {}", rec.original, rec.reason)),
    }
}

/// Plan de la release (action + indexers visés), lu à l'arrivée du NZB, et NFO (MediaInfo
/// en texte de `media`) pour les indexers qui le demandent à l'envoi par API.
fn write_plan_and_nfo(cfg: &Config, release: &str, media: &Path, report: &Path, source: &Path) {
    if cfg.action == "rename" {
        return;
    }
    report_to_waiting(cfg, release, report);
    let plan = crate::ngpost::Plan::from_cfg(cfg);
    let _ = crate::ngpost::write_plan(cfg, release, &plan);
    // nom d'origine (fichier, et son dossier) : cherché aussi sur les indexers avant le post
    let mut origins: Vec<String> = source.file_stem().map(|x| x.to_string_lossy().into_owned()).into_iter().collect();
    if let Some(p) = source.parent().map(prep::file_name).filter(|p| p.chars().filter(|c| *c == '.').count() >= 3) {
        origins.push(p);
    }
    crate::ngpost::write_origins(cfg, release, &origins);
    let Some(w) = crate::ngpost::waiting_dir(cfg) else { return };
    let nfo = w.join(release).join(format!("{release}.nfo"));
    if nfo_edited(cfg, release) {
        return;
    }
    // NFO d'origine (à côté de la vidéo ou dans le dossier de la release) : envoyé tel quel,
    // sauf si « Envoyer le NFO d'origine » est décoché (et alors rien n'est créé à la place)
    if let Some(orig) = nfo_near(source) {
        if !cfg.nfo_send {
            crate::notify::record(format!("NFO d'origine « {} » non envoyé aux indexers (réglage)", prep::file_name(&orig)));
            crate::ngpost::mark_nfo(cfg, release, "non-envoye");
            return;
        }
        let _ = std::fs::create_dir_all(w.join(release));
        if std::fs::copy(&orig, &nfo).is_ok() {
            crate::ngpost::mark_nfo(cfg, release, &format!("origine:{}", prep::file_name(&orig)));
            return;
        }
    }
    // sinon créé depuis MediaInfo si un indexer le demande (« nfo » coché)
    if wants_nfo(cfg, &plan) {
        if let Ok(t) = prep::mediainfo_text(cfg, media, false) {
            let _ = std::fs::write(&nfo, t);
            crate::ngpost::mark_nfo(cfg, release, "mediainfo");
        }
    }
}

/// NFO d'un contenu : celui d'origine (déjà copié), sinon, si un indexer le demande, le rapport
/// BDInfo (Blu-ray) ou le rapport MediaInfo en texte (titre principal d'un disque, premier
/// fichier vidéo ou audio).
/// NFO modifié à la main avant le traitement : écrit tel quel (il remplace celui d'origine ou
/// celui qui aurait été créé). Vrai s'il a été écrit.
fn nfo_edited(cfg: &Config, release: &str) -> bool {
    let (Some(t), Some(w)) = (cfg.nfo_override.as_ref().filter(|t| !t.trim().is_empty()), crate::ngpost::waiting_dir(cfg)) else { return false };
    let dir = w.join(release);
    let _ = std::fs::create_dir_all(&dir);
    if std::fs::write(dir.join(format!("{release}.nfo")), t).is_ok() {
        crate::ngpost::mark_nfo(cfg, release, "modifie");
        return true;
    }
    false
}

fn content_nfo(cfg: &Config, release: &str, dest: &Path, plan: &crate::ngpost::Plan) {
    let Some(w) = crate::ngpost::waiting_dir(cfg) else { return };
    let dir = w.join(release);
    let nfo = dir.join(format!("{release}.nfo"));
    if nfo_edited(cfg, release) {
        return;
    }
    // Blu-ray qui a déjà son NFO : le remplacer par le BDInfo, ou ajouter le BDInfo dessous
    let bd = dir.join(format!("{release}.bdinfo.txt"));
    if let (true, Ok(b)) = (nfo.exists() && cfg.nfo_bd != "orig", std::fs::read_to_string(&bd)) {
        let orig = crate::ngpost::nfo_origin(&dir);
        let name = orig.trim_start_matches("origine:").trim().to_string();
        let (text, mark) = if cfg.nfo_bd == "bdinfo" {
            (b, "bdinfo".to_string())
        } else {
            let o = std::fs::read(&nfo).map(|x| crate::ngpost::decode_text(&x)).unwrap_or_default();
            (format!("{}\n\n{}", o.trim_end(), b.trim_end()), format!("origine+bdinfo:{name}"))
        };
        if std::fs::write(&nfo, text).is_ok() {
            crate::ngpost::mark_nfo(cfg, release, &mark);
        }
        return;
    }
    // NFO d'origine déjà là, ou volontairement pas envoyé : rien à créer
    if nfo.exists() || !wants_nfo(cfg, plan) || crate::ngpost::nfo_origin(&dir) == "non-envoye" {
        return;
    }
    let mut origin = "mediainfo";
    let text = if let Ok(t) = std::fs::read_to_string(&bd) {
        origin = "bdinfo";
        Some(t)
    } else {
        // disque : le dossier livré, ou l'image ISO seule dedans
        let iso = crate::content::files_of(dest).into_iter().map(|(p, _)| p).filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("iso")).unwrap_or(false)).collect::<Vec<_>>();
        let disc = crate::disc::detect(dest).or_else(|| (iso.len() == 1).then(|| crate::disc::detect(&iso[0])).flatten());
        match disc {
            Some(d) => Some(crate::disc::analyze(cfg, &d).text).filter(|t| !t.trim().is_empty()),
            None => crate::content::first_media(dest).and_then(|m| prep::mediainfo_text(cfg, &m, false).ok()),
        }
    };
    if let Some(t) = text {
        let _ = std::fs::create_dir_all(&dir);
        if std::fs::write(&nfo, t).is_ok() {
            crate::ngpost::mark_nfo(cfg, release, origin);
        }
    }
}

/// Déplacement d'un disque à l'autre dont l'original n'a pas pu être supprimé : signalé dans
/// l'historique, le Journal et l'e-mail (au lieu de laisser croire qu'il a été déplacé).
fn note_left(rec: &mut Record, done: &prep::Done) {
    if done.left.is_empty() {
        return;
    }
    let (p, e) = &done.left[0];
    let more = if done.left.len() > 1 { format!(" (et {} autre(s))", done.left.len() - 1) } else { String::new() };
    let msg = format!("copié, mais l'original n'a pas pu être supprimé : {}{more}, {e}", p.display());
    crate::notify::record(format!("⚠ {} : {msg}", rec.release));
    rec.reason = msg;
}

/// Créer un NFO quand la release n'en a pas ? Seulement si c'est demandé (Réglages → NFO, ou
/// le choix du traitement) : une case d'indexer ne force jamais la création.
fn wants_nfo(cfg: &Config, plan: &crate::ngpost::Plan) -> bool {
    let _ = plan;
    cfg.nfo_create
}

/// NFO d'origine d'une vidéo : même nom (`Film.nfo`), sinon le seul NFO de son dossier si ce
/// dossier ne contient qu'une vidéo (dossier de release).
pub fn nfo_near(source: &Path) -> Option<PathBuf> {
    let is_nfo = |p: &Path| p.extension().map(|e| e.eq_ignore_ascii_case("nfo")).unwrap_or(false);
    if source.is_dir() {
        // dossier (série) : NFO à sa racine, sinon le seul NFO de tout le dossier
        let top = std::fs::read_dir(source).ok().and_then(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_nfo(p)).min());
        if top.is_some() {
            return top;
        }
        let all: Vec<PathBuf> = crate::content::files_of(source).into_iter().map(|(p, _)| p).filter(|p| is_nfo(p)).collect();
        return (all.len() == 1).then(|| all[0].clone());
    }
    let same = source.with_extension("nfo");
    if same.is_file() {
        return Some(same);
    }
    let dir = source.parent()?;
    let entries: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    let nfos: Vec<&PathBuf> = entries.iter().filter(|p| p.is_file() && is_nfo(p)).collect();
    let videos = entries.iter().filter(|p| prep::is_video(p)).count();
    (nfos.len() == 1 && videos <= 1).then(|| nfos[0].clone())
}

/// Ce qu'on livre d'une série (un appel = une release).
pub struct SeriesJob<'a> {
    pub scan: &'a crate::series::Scan,
    pub fields: &'a prep::Fields,
    /// Saisons de cette release.
    pub unit: Vec<u32>,
    pub release: String,
    /// Renommer les épisodes (sinon noms d'origine).
    pub rename: bool,
}

/// Traite une release de série (une saison ou plusieurs) : comme `run_one`, pour un dossier.
#[allow(clippy::too_many_arguments)]
pub fn run_series(
    job: &SeriesJob,
    cfg: &Config,
    how: Transfer,
    dest: Option<&Path>,
    mode: Mode,
    quarantine_on_fail: bool,
    hist: &Mutex<History>,
    progress: &mut dyn FnMut(Stage, u64, u64),
) -> Record {
    prep::begin_job();
    let _busy = crate::notify::busy();
    let scan = job.scan;
    let seasons: Vec<&crate::series::Season> = scan.seasons.iter().filter(|s| job.unit.contains(&s.number)).collect();
    let mut rec = Record {
        ts: prep::now_secs(),
        status: Status::Failed,
        mode: mode.label().into(),
        original: format!("{} ({})", prep::file_name(&scan.root), crate::series::season_code(&job.unit)),
        release: job.release.clone(),
        size: seasons.iter().map(|s| s.size()).sum(),
        resolution: job.fields.resolution.clone(),
        codec: job.fields.codec.clone(),
        langs: job.fields.lang.clone(),
        duration: format!("{} épisodes", seasons.iter().map(|s| s.count()).sum::<usize>()),
        source_path: scan.root.display().to_string(),
        dest: String::new(),
        xml: String::new(),
        reason: String::new(),
        post: String::new(),
    };
    let outcome: Result<prep::Done, String> = (|| {
        let dest_dir: PathBuf = match dest {
            Some(d) => d.to_path_buf(),
            None => {
                cfg.ready_send()?;
                cfg.output_dir().unwrap_or_default()
            }
        };
        let name = prep::clean_name(&job.release);
        if name.is_empty() {
            return Err("nom de release vide".into());
        }
        let _claim = Claim::take(vec![format!("src:{}:{}", scan.root.display(), crate::series::season_code(&job.unit)), format!("rel:{}", name.to_lowercase())])?;
        if cfg.skip_duplicates {
            duplicate_check(&name, hist)?;
        }
        crate::series::execute(cfg, scan, job.fields, &job.unit, job.rename, &name, how, &dest_dir, progress)
    })();
    match outcome {
        Ok(done) => {
            note_left(&mut rec, &done);
            // NFO : MediaInfo du premier épisode livré
            let first = crate::series::first_file(&done.dest);
            // NFO d'origine : à la racine du dossier de la série, sinon emporté dans la release
            let nfo_src = if scan.root.is_dir() && nfo_near(&scan.root).is_some() { scan.root.clone() } else { done.dest.clone() };
            write_plan_and_nfo(cfg, &rec.release, first.as_deref().unwrap_or(&done.dest), &done.xml, &nfo_src);
            if cfg.action != "rename" {
                crate::ngpost::write_content_list(cfg, &rec.release, &done.dest);
            }
            // série, quel que soit son nom : catégories et groupes des séries
            if cfg.action != "rename" {
                crate::ngpost::mark_series(cfg, &rec.release);
            }
            if dest.is_none() && cfg.post_mode == "watch" && cfg.action != "rename" && cfg.ngpost_autostart {
                let _ = crate::ngpost::ensure_running(cfg);
            }
            rec.status = Status::Sent;
            rec.dest = done.dest.display().to_string();
            rec.xml = done.xml.display().to_string();
        }
        Err(why) => {
            rec.reason = why.clone();
            if quarantine_on_fail && !cfg.quarantine.trim().is_empty() && scan.root.exists() && !prep::is_cancel(&why) {
                match prep::quarantine(&scan.root, cfg) {
                    Ok(q) => {
                        rec.status = Status::Quarantine;
                        rec.dest = q.display().to_string();
                    }
                    Err(e) => rec.reason = format!("{why} (quarantaine impossible : {e})"),
                }
            }
        }
    }
    note_record(&rec);
    if let Ok(mut h) = hist.lock() {
        h.push(rec.clone());
    }
    rec
}

/// Tours de post d'une série : toute la sélection (intégrale / complète), puis une release par
/// saison, puis une par épisode, chacun au choix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rounds {
    pub whole: bool,
    pub seasons: bool,
    pub episodes: bool,
}

impl Rounds {
    pub fn from_cfg(cfg: &Config) -> Self {
        let r = Rounds { whole: cfg.series_round_whole, seasons: cfg.series_round_seasons, episodes: cfg.series_round_episodes };
        if r.any() {
            r
        } else {
            Rounds { whole: true, seasons: false, episodes: false }
        }
    }
    pub fn any(&self) -> bool {
        self.whole || self.seasons || self.episodes
    }
    pub fn label(&self) -> String {
        let mut v = Vec::new();
        if self.whole {
            v.push("toute la série");
        }
        if self.seasons {
            v.push("une par saison");
        }
        if self.episodes {
            v.push("une par épisode");
        }
        v.join(" + ")
    }
}

/// Une série à livrer en un ou plusieurs tours.
pub struct SeriesRun<'a> {
    pub scan: &'a crate::series::Scan,
    /// Éléments du nom de la release de toute la sélection.
    pub fields: &'a prep::Fields,
    pub selected: Vec<u32>,
    pub rounds: Rounds,
    /// Mention « complète » (release de toute la série seulement) et son mot.
    pub complete: bool,
    pub word: String,
    pub rename: bool,
    /// Nom écrit à la main pour la release de toute la sélection.
    pub whole_name: Option<String>,
    /// Résolution de chaque saison (MediaInfo) : les releases de saison portent la leur.
    pub season_res: BTreeMap<u32, String>,
}

/// Noms des releases de chaque tour : (tour, saisons, nom). Les épisodes : un par fichier.
pub fn series_names(cfg: &Config, run: &SeriesRun) -> Vec<(&'static str, Vec<u32>, String)> {
    let scan = run.scan;
    let sel: Vec<u32> = scan.numbers().into_iter().filter(|n| run.selected.contains(n)).collect();
    let mut c = cfg.clone();
    if !run.word.trim().is_empty() {
        c.complete_word = run.word.trim().to_string();
    }
    let season_fields = |n: u32| {
        let mut f = run.fields.clone();
        if let Some(r) = run.season_res.get(&n) {
            f.resolution = r.clone();
        }
        f
    };
    let mut out = Vec::new();
    if run.rounds.whole && !sel.is_empty() {
        let name = run.whole_name.clone().filter(|n| !n.trim().is_empty()).map(|n| prep::clean_name(&n)).unwrap_or_else(|| crate::series::release_name(&c, run.fields, &sel, run.complete));
        out.push(("whole", sel.clone(), name));
    }
    if run.rounds.seasons {
        for n in &sel {
            out.push(("season", vec![*n], crate::series::release_name(&c, &season_fields(*n), &[*n], false)));
        }
    }
    if run.rounds.episodes {
        for n in &sel {
            if let Some(s) = scan.season(*n) {
                for e in &s.episodes {
                    out.push(("episode", vec![*n], crate::series::episode_release(&c, &season_fields(*n), e)));
                }
            }
        }
    }
    out
}

/// Livre une série en un ou plusieurs tours, dans l'ordre où ils seront postés :
/// 1. toute la sélection (déplacée ou copiée selon `how`) ;
/// 2. une release par saison, puis une par épisode, faites à partir de la release précédente par
///    liens physiques (même disque : aucune copie, aucune place en plus ; sinon copie) ;
/// sans tour « toute la série », les saisons (ou les épisodes) partent directement de la
/// source. Chaque release a son rapport MediaInfo, son plan, son NFO et sa liste de contenu ;
/// une suppression (« après le post : supprimer ») ne retire que ses liens.
#[allow(clippy::too_many_arguments)]
pub fn run_series_rounds(run: &SeriesRun, cfg: &Config, how: Transfer, dest: Option<&Path>, mode: Mode, quarantine: bool, hist: &Mutex<History>, progress: &mut dyn FnMut(Stage, u64, u64)) -> Vec<Record> {
    let scan = run.scan;
    let sel: Vec<u32> = scan.numbers().into_iter().filter(|n| run.selected.contains(n)).collect();
    let rounds = if run.rounds.any() { run.rounds } else { Rounds { whole: true, seasons: false, episodes: false } };
    let mut out: Vec<Record> = Vec::new();
    if sel.is_empty() {
        return out;
    }
    let mut c = cfg.clone();
    if !run.word.trim().is_empty() {
        c.complete_word = run.word.trim().to_string();
    }
    let season_fields = |n: u32| {
        let mut f = run.fields.clone();
        if let Some(r) = run.season_res.get(&n) {
            f.resolution = r.clone();
        }
        f
    };
    // résolutions différentes : dit avant (page Série) et ici (journal, e-mail)
    let res_note = crate::series::resolution_warning(scan, &run.season_res);
    let names = series_names(cfg, run);
    let stop = |out: &Vec<Record>| out.last().map(|r| prep::is_cancel(&r.reason)).unwrap_or(false);
    // source des tours suivants : (scan relocalisé, saisons) ; None : la source d'origine
    let mut derived: Vec<(crate::series::Scan, Vec<u32>)> = Vec::new();

    // 1. toute la sélection
    if rounds.whole {
        let name = names.iter().find(|n| n.0 == "whole").map(|n| n.2.clone()).unwrap_or_default();
        let job = SeriesJob { scan, fields: run.fields, unit: sel.clone(), release: name, rename: run.rename };
        let mut r = run_series(&job, &c, how, dest, mode, false, hist, progress);
        if let (Status::Sent, Some(n)) = (&r.status, &res_note) {
            let msg = format!("résolutions différentes : {n} (release nommée en {})", run.fields.resolution);
            crate::notify::record(format!("⚠ {} : {msg}", r.release));
            if r.reason.is_empty() {
                r.reason = msg;
            }
        }
        let ok = r.status == Status::Sent;
        let dir = PathBuf::from(&r.dest);
        out.push(r);
        if !ok || stop(&out) {
            return finish(out, quarantine, cfg, &scan.root);
        }
        let placed = crate::series::layout(&c, scan, run.fields, &sel, run.rename);
        derived.push((crate::series::relocate(scan, &placed, &dir), sel.clone()));
    }

    // 2. une release par saison
    let link = |base_is_source: bool| if base_is_source { how } else { Transfer::Link };
    if rounds.seasons {
        let base_is_source = derived.is_empty();
        let base = derived.first().map(|d| d.0.clone()).unwrap_or_else(|| scan.clone());
        let mut season_dirs = Vec::new();
        for n in &sel {
            let name = names.iter().find(|x| x.0 == "season" && x.1 == vec![*n]).map(|x| x.2.clone()).unwrap_or_default();
            let f = season_fields(*n);
            // à partir de la release de toute la série : épisodes déjà renommés s'il le fallait
            let job = SeriesJob { scan: &base, fields: &f, unit: vec![*n], release: name, rename: run.rename && base_is_source };
            let mut r = run_series(&job, &c, link(base_is_source), dest, mode, false, hist, progress);
            if r.status == Status::Sent {
                if let (Some(sr), Some(main)) = (run.season_res.get(n), crate::series::main_resolution(scan, &run.season_res)) {
                    if *sr != main && r.reason.is_empty() {
                        r.reason = format!("saison en {sr} (le reste en {main}) : release nommée en {sr}");
                        crate::notify::record(format!("⚠ {} : {}", r.release, r.reason));
                    }
                }
                let placed = crate::series::layout(&c, &base, &f, &[*n], job.rename);
                season_dirs.push((crate::series::relocate(&base, &placed, Path::new(&r.dest)), vec![*n]));
            }
            out.push(r);
            if stop(&out) {
                return out;
            }
        }
        if derived.is_empty() {
            derived = season_dirs;
        }
    }

    // 3. une release par épisode (un fichier, comme un film)
    if rounds.episodes {
        let base_is_source = derived.is_empty();
        let bases: Vec<(crate::series::Scan, Vec<u32>)> = if base_is_source { vec![(scan.clone(), sel.clone())] } else { derived.clone() };
        for (b, only) in &bases {
            for s in b.seasons.iter().filter(|s| only.contains(&s.number)) {
                for e in &s.episodes {
                    if prep::job_cancelled() {
                        return out;
                    }
                    let mut f = season_fields(s.number);
                    // résolution de chaque épisode (MediaInfo), au cas où un épisode diffère
                    if let Ok(fa) = prep::query_facts(cfg, &e.path) {
                        let r = prep::resolution(&fa);
                        if !r.is_empty() {
                            f.resolution = r;
                        }
                    }
                    let name = crate::series::episode_release(&c, &f, e);
                    let mut r = run_one(&e.path, &c, Some(&name), link(base_is_source), dest, mode, false, hist, progress);
                    if r.status == Status::Sent {
                        if c.action != "rename" {
                            crate::ngpost::mark_series(&c, &r.release);
                        }
                        if let Some(sr) = run.season_res.get(&s.number) {
                            if *sr != f.resolution && r.reason.is_empty() {
                                r.reason = format!("épisode en {} (sa saison en {sr})", f.resolution);
                            }
                        }
                    }
                    out.push(r);
                    if stop(&out) {
                        return out;
                    }
                }
            }
        }
    }
    finish(out, quarantine, cfg, &scan.root)
}

/// Quarantaine de la source si tout a échoué (surveillance).
fn finish(mut out: Vec<Record>, quarantine: bool, cfg: &Config, root: &Path) -> Vec<Record> {
    if quarantine && !out.is_empty() && out.iter().all(|r| r.status == Status::Failed) && !cfg.quarantine.trim().is_empty() && root.exists() {
        if let Ok(q) = prep::quarantine(root, cfg) {
            if let Some(last) = out.last_mut() {
                last.status = Status::Quarantine;
                last.dest = q.display().to_string();
            }
        }
    }
    out
}

/// Série déposée dans l'inbox : traitée d'un bloc avec les réglages (renommage, une release
/// par saison ou non, mention COMPLETE).
pub fn run_series_auto(dir: &Path, cfg: &Config, hist: &Mutex<History>, progress: &mut dyn FnMut(Stage, u64, u64)) -> Vec<Record> {
    run_series_with(dir, cfg, Transfer::Move, None, Mode::Auto, true, hist, progress)
}

/// Releases qu'un dossier de série donnera avec les réglages (tours, mention complète,
/// résolution de chaque saison).
pub fn series_plan(dir: &Path, cfg: &Config) -> Option<(crate::series::Scan, Vec<String>)> {
    let scan = crate::series::scan(dir)?;
    let (fields, season_res) = auto_fields(cfg, &scan);
    let sel = scan.numbers();
    let run = SeriesRun {
        scan: &scan,
        fields: &fields,
        selected: sel.clone(),
        rounds: Rounds::from_cfg(cfg),
        complete: cfg.series_auto_complete && scan.complete_series(&sel),
        word: cfg.complete_word.clone(),
        rename: cfg.series_rename,
        whole_name: None,
        season_res,
    };
    let names = series_names(cfg, &run).into_iter().map(|(_, _, n)| n).collect();
    Some((scan, names))
}

/// Éléments du nom en automatique : ceux du premier épisode, la résolution étant celle du plus
/// grand nombre d'épisodes quand les saisons diffèrent.
fn auto_fields(cfg: &Config, scan: &crate::series::Scan) -> (prep::Fields, BTreeMap<u32, String>) {
    let (mut fields, _) = crate::series::detect_fields(cfg, scan);
    let res = crate::series::season_resolutions(cfg, scan, &scan.numbers());
    if let Some(m) = crate::series::main_resolution(scan, &res) {
        fields.resolution = m;
    }
    (fields, res)
}

/// Dossier de série traité d'un bloc (inbox, lot) : réglages des séries (tours, mention,
/// renommage), destination et transfert au choix. `quarantine` : quarantaine si tout échoue.
#[allow(clippy::too_many_arguments)]
pub fn run_series_with(dir: &Path, cfg: &Config, how: Transfer, dest: Option<&Path>, mode: Mode, quarantine: bool, hist: &Mutex<History>, progress: &mut dyn FnMut(Stage, u64, u64)) -> Vec<Record> {
    let Some(scan) = crate::series::scan(dir) else { return Vec::new() };
    let (fields, season_res) = auto_fields(cfg, &scan);
    let sel = scan.numbers();
    let run = SeriesRun {
        scan: &scan,
        fields: &fields,
        selected: sel.clone(),
        rounds: Rounds::from_cfg(cfg),
        complete: cfg.series_auto_complete && scan.complete_series(&sel),
        word: cfg.complete_word.clone(),
        rename: cfg.series_rename,
        whole_name: None,
        season_res,
    };
    run_series_rounds(&run, cfg, how, dest, mode, quarantine, hist, progress)
}

/// Traite un contenu (jeu, logiciel, musique, ebook…) : fichier ou dossier livré dans
/// `<destination>\<release>\`, avec sa catégorie (pour l'API des indexers et la compression).
#[allow(clippy::too_many_arguments)]
pub fn run_content(
    src: &Path,
    cat: &crate::content::Category,
    release: &str,
    cfg: &Config,
    how: Transfer,
    dest: Option<&Path>,
    mode: Mode,
    quarantine_on_fail: bool,
    hist: &Mutex<History>,
    progress: &mut dyn FnMut(Stage, u64, u64),
) -> Record {
    prep::begin_job();
    let _busy = crate::notify::busy();
    let files = crate::content::files_of(src);
    let mut rec = Record {
        ts: prep::now_secs(),
        status: Status::Failed,
        mode: mode.label().into(),
        original: prep::file_name(src),
        release: release.to_string(),
        size: crate::content::size_of(src),
        resolution: String::new(),
        codec: cat.label(),
        langs: String::new(),
        duration: format!("{} fichier(s)", files.len()),
        source_path: src.display().to_string(),
        dest: String::new(),
        xml: String::new(),
        reason: String::new(),
        post: String::new(),
    };
    let outcome: Result<prep::Done, String> = (|| {
        let dest_dir: PathBuf = match dest {
            Some(d) => d.to_path_buf(),
            None => {
                cfg.ready_send()?;
                cfg.output_dir().unwrap_or_default()
            }
        };
        let name = prep::clean_name(release);
        if name.is_empty() {
            return Err("nom de release vide".into());
        }
        let _claim = Claim::take(vec![format!("src:{}", src.display()), format!("rel:{}", name.to_lowercase())])?;
        if cfg.skip_duplicates {
            duplicate_check(&name, hist)?;
        }
        crate::content::execute(cfg, cat, src, &name, how, &dest_dir, progress)
    })();
    match outcome {
        Ok(done) => {
            note_left(&mut rec, &done);
            if cfg.action != "rename" {
                let plan = crate::ngpost::Plan { compress: cat.compress_override(), ..crate::ngpost::Plan::from_cfg(cfg) };
                let _ = crate::ngpost::write_plan(cfg, &rec.release, &plan);
                crate::content::write_cat(cfg, &rec.release, cat);
                crate::ngpost::write_origins(cfg, &rec.release, &[prep::file_name(src)]);
                report_to_waiting(cfg, &rec.release, &done.xml);
                content_nfo(cfg, &rec.release, &done.dest, &plan);
                crate::ngpost::write_content_list(cfg, &rec.release, &done.dest);
            }
            if dest.is_none() && cfg.post_mode == "watch" && cfg.action != "rename" && cfg.ngpost_autostart {
                let _ = crate::ngpost::ensure_running(cfg);
            }
            rec.status = Status::Sent;
            rec.dest = done.dest.display().to_string();
            rec.xml = done.xml.display().to_string();
        }
        Err(why) => {
            rec.reason = why.clone();
            if quarantine_on_fail && !cfg.quarantine.trim().is_empty() && src.exists() && !prep::is_cancel(&why) {
                match prep::quarantine(src, cfg) {
                    Ok(q) => {
                        rec.status = Status::Quarantine;
                        rec.dest = q.display().to_string();
                    }
                    Err(e) => rec.reason = format!("{why} (quarantaine impossible : {e})"),
                }
            }
        }
    }
    note_record(&rec);
    if let Ok(mut h) = hist.lock() {
        h.push(rec.clone());
    }
    rec
}

/// Contenus déposés dans les dossiers « @Catégorie » de l'inbox : (élément, catégorie, fichiers).
pub fn content_items(cfg: &Config) -> Vec<(PathBuf, crate::content::Category, Vec<PathBuf>)> {
    let mut out = Vec::new();
    for c in cfg.content_cats.iter().filter(|c| c.inbox) {
        let Some(dir) = c.inbox_dir(cfg) else { continue };
        let _ = std::fs::create_dir_all(&dir);
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let n = prep::file_name(&p);
            if n.starts_with('.') || n.starts_with('~') || n.ends_with(".part") || n.ends_with(".tmp") || n.ends_with(".crdownload") {
                continue;
            }
            let files: Vec<PathBuf> = crate::content::files_of(&p).into_iter().map(|(f, _)| f).collect();
            if !files.is_empty() {
                out.push((p, c.clone(), files));
            }
        }
    }
    // déposés directement dans l'inbox : disques complets (dossier BDMV / VIDEO_TS, image ISO
    // de film, dossier de release avec son ISO) et autres contenus reconnus
    let inbox = Path::new(cfg.inbox.trim());
    if !cfg.inbox.trim().is_empty() {
        if let Ok(rd) = std::fs::read_dir(inbox) {
            for e in rd.flatten() {
                let p = e.path();
                let n = prep::file_name(&p);
                if n.starts_with(['.', '_', '@', '~']) || n.ends_with(".part") || n.ends_with(".tmp") {
                    continue;
                }
                // disque complet, sinon autre contenu reconnu (jeu, logiciel, album, ebook…) ;
                // les vidéos, séries et dossiers de film sont traités à part
                let key = match crate::disc::detect(&p) {
                    Some(d) => d.kind.cat_key().to_string(),
                    None => {
                        let other = if p.is_dir() { prep::list_videos(&p, true).is_empty() && !crate::series::looks_like_series(&p) } else { !prep::is_video(&p) };
                        match other.then(|| crate::content::guess(&p, &cfg.content_cats)).flatten() {
                            Some(k) => k,
                            None => continue,
                        }
                    }
                };
                let Some(c) = cfg.content_cats.iter().find(|c| c.key == key) else { continue };
                let files: Vec<PathBuf> = crate::content::files_of(&p).into_iter().map(|(f, _)| f).collect();
                if !files.is_empty() && !out.iter().any(|(x, _, _)| x == &p) {
                    out.push((p, c.clone(), files));
                }
            }
        }
    }
    out
}

/// Traite un fichier. `dest` : dossier de destination (None = dossier de sortie des réglages).
/// `quarantine_on_fail` : en mode Auto, un échec déplace le fichier en quarantaine.
/// L'enregistrement est ajouté à l'historique et renvoyé.
#[allow(clippy::too_many_arguments)]
pub fn run_one(
    path: &Path,
    cfg: &Config,
    forced: Option<&str>,
    how: Transfer,
    dest: Option<&Path>,
    mode: Mode,
    quarantine_on_fail: bool,
    hist: &Mutex<History>,
    progress: &mut dyn FnMut(Stage, u64, u64),
) -> Record {
    prep::begin_job();
    let _busy = crate::notify::busy();
    let a = prep::analyze(path, cfg, forced, None);
    let mut rec = record_from(&a, mode);

    let outcome: Result<prep::Done, String> = (|| {
        let dest_dir: PathBuf = match dest {
            Some(d) => d.to_path_buf(),
            None => {
                cfg.ready_send()?;
                cfg.output_dir().unwrap_or_default()
            }
        };
        let name = a.name.clone()?;
        let _claim = Claim::take(vec![format!("src:{}", path.display()), format!("rel:{}", name.to_lowercase())])?;
        if cfg.skip_duplicates {
            duplicate_check(&name, hist)?;
        }
        prep::execute(path, &name, cfg, how, &dest_dir, progress)
    })();

    match outcome {
        Ok(done) => {
            note_left(&mut rec, &done);
            write_plan_and_nfo(cfg, &rec.release, &done.dest, &done.xml, path);
            if cfg.action != "rename" {
                crate::ngpost::write_content_list(cfg, &rec.release, &done.dest);
            }
            // vidéo déposée dans le dossier surveillé : NgPost doit être ouvert pour la poster
            if dest.is_none() && cfg.post_mode == "watch" && cfg.action != "rename" && cfg.ngpost_autostart {
                let _ = crate::ngpost::ensure_running(cfg);
            }
            rec.status = Status::Sent;
            rec.dest = done.dest.display().to_string();
            rec.xml = done.xml.display().to_string();
        }
        Err(why) => {
            rec.reason = why.clone();
            // arrêt demandé : rien en quarantaine, le fichier reste où il était
            if quarantine_on_fail && !cfg.quarantine.trim().is_empty() && !prep::is_cancel(&why) {
                match prep::quarantine(path, cfg) {
                    Ok(q) => {
                        rec.status = Status::Quarantine;
                        rec.dest = q.display().to_string();
                    }
                    Err(e) => rec.reason = format!("{why} (quarantaine impossible : {e})"),
                }
            }
        }
    }
    note_record(&rec);
    if let Ok(mut h) = hist.lock() {
        h.push(rec.clone());
    }
    rec
}

/// Traite un élément quelconque comme le ferait l'inbox (téléchargement terminé à reposter) :
/// disque ou autre contenu, série, dossier d'un film, vidéos. `name` : nom de release connu
/// (celui du téléchargement), repris s'il en a la forme. Jamais de quarantaine : l'original
/// reste à sa place en cas d'échec.
pub fn process_path(path: &Path, cfg: &Config, how: Transfer, name: Option<&str>, hist: &Mutex<History>, progress: &mut dyn FnMut(Stage, u64, u64)) -> Vec<Record> {
    let named = name.map(prep::clean_name).filter(|n| n.contains('.') && !n.contains(' ') && n.len() >= 6);
    let content = |key: &str| cfg.content_cats.iter().find(|c| c.key == key).cloned();
    let as_content = |p: &Path, key: Option<String>, progress: &mut dyn FnMut(Stage, u64, u64)| -> Vec<Record> {
        let key = key.or_else(|| crate::content::guess(p, &cfg.content_cats)).unwrap_or_else(|| "autre".into());
        match content(&key).or_else(|| cfg.content_cats.first().cloned()) {
            Some(cat) => {
                let v = crate::content::detect_full(cfg, p).0;
                let release = named.clone().unwrap_or_else(|| crate::content::initial_name(&cat, p, &v));
                vec![run_content(p, &cat, &release, cfg, how, None, Mode::Auto, false, hist, progress)]
            }
            None => Vec::new(),
        }
    };
    if let Some(d) = crate::disc::detect(path) {
        return as_content(path, Some(d.kind.cat_key().to_string()), progress);
    }
    if path.is_file() {
        return if prep::is_video(path) { vec![run_one(path, cfg, named.as_deref(), how, None, Mode::Auto, false, hist, progress)] } else { as_content(path, None, progress) };
    }
    if crate::series::looks_like_series(path) {
        return run_series_with(path, cfg, how, None, Mode::Auto, false, hist, progress);
    }
    let videos = prep::list_videos(path, true);
    match videos.len() {
        0 => as_content(path, None, progress),
        1 => {
            let r = run_one(&videos[0], cfg, named.as_deref(), how, None, Mode::Auto, false, hist, progress);
            // dossier du film déplacé : ce qui reste (NFO repris, échantillon, par2…) à la corbeille
            if r.status == Status::Sent && how == Transfer::Move && prep::list_videos(path, true).is_empty() {
                let _ = prep::trash(path);
            }
            vec![r]
        }
        _ => videos.iter().map(|v| run_one(v, cfg, None, how, None, Mode::Auto, false, hist, progress)).collect(),
    }
}

/// Un passage sur l'inbox : traite chaque vidéo stable. Renvoie les enregistrements produits.
pub fn scan_once(
    cfg: &Config,
    hist: &Mutex<History>,
    stop: &AtomicBool,
    on_progress: &mut dyn FnMut(&Path, Stage, u64, u64),
    on: &mut dyn FnMut(&Record),
) -> usize {
    let inbox = Path::new(cfg.inbox.trim());
    let files = prep::list_videos(inbox, false);
    // séries : sous-dossiers de l'inbox reconnus comme série (Série > Saisons > épisodes)
    let series_dirs: Vec<(PathBuf, Vec<PathBuf>)> = std::fs::read_dir(inbox)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && !prep::file_name(p).starts_with('.') && !prep::file_name(p).starts_with('_') && !prep::file_name(p).starts_with('@'))
                .filter(|p| crate::disc::detect(p).is_none() && crate::series::looks_like_series(p))
                .map(|p| {
                    let all = crate::series::all_files(&p);
                    (p, all)
                })
                .collect()
        })
        .unwrap_or_default();
    let contents = content_items(cfg);
    // dossiers de film (« Film.2010…\film.mkv » avec NFO, Sample, Proof) : la vidéo est traitée,
    // le reste du dossier part ensuite à la corbeille
    let movie_dirs: Vec<(PathBuf, PathBuf)> = std::fs::read_dir(inbox)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && !prep::file_name(p).starts_with(['.', '_', '@']))
                .filter(|p| !series_dirs.iter().any(|(d, _)| d == p) && !contents.iter().any(|(c, _, _)| c == p))
                .filter_map(|p| {
                    let v = prep::list_videos(&p, true);
                    (v.len() == 1).then(|| (p, v[0].clone()))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut files = files;
    files.extend(movie_dirs.iter().map(|(_, v)| v.clone()));
    if files.is_empty() && series_dirs.is_empty() && contents.is_empty() {
        return 0;
    }
    let mut watched = files.clone();
    for (_, f) in &series_dirs {
        watched.extend(f.iter().cloned());
    }
    for (_, _, f) in &contents {
        watched.extend(f.iter().cloned());
    }
    let ready: HashSet<PathBuf> = prep::stable_files(&watched, cfg.stable_secs).into_iter().collect();
    let mut n = 0;
    for (dir, all) in &series_dirs {
        // copie terminée : tous ses fichiers stables
        if stop.load(Ordering::Relaxed) || all.is_empty() || !all.iter().all(|f| ready.contains(f)) {
            continue;
        }
        for r in run_series_auto(dir, cfg, hist, &mut |st, d, t| on_progress(dir, st, d, t)) {
            on(&r);
            n += 1;
        }
    }
    // autres contenus : nom tiré du modèle de la catégorie (ou gardé s'il est déjà une release)
    for (item, cat, all) in &contents {
        if stop.load(Ordering::Relaxed) || !all.iter().all(|f| ready.contains(f)) {
            continue;
        }
        let v = crate::content::detect_full(cfg, item).0;
        let release = crate::content::initial_name(cat, item, &v);
        let r = run_content(item, cat, &release, cfg, Transfer::Move, None, Mode::Auto, true, hist, &mut |st, d, t| on_progress(item, st, d, t));
        on(&r);
        n += 1;
    }
    for p in files.into_iter().filter(|f| ready.contains(f)) {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let r = run_one(&p, cfg, None, Transfer::Move, None, Mode::Auto, true, hist, &mut |st, d, t| on_progress(&p, st, d, t));
        // dossier de film traité : ce qui reste (NFO déjà repris, échantillon, preuves) à la corbeille
        if r.status == Status::Sent {
            if let Some((dir, _)) = movie_dirs.iter().find(|(_, v)| v == &p) {
                if prep::list_videos(dir, true).is_empty() {
                    let _ = prep::trash(dir);
                }
            }
        }
        on(&r);
        n += 1;
    }
    n
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    const NZB: &str = "<?xml version=\"1.0\"?>\n<nzb>\n<file poster=\"a\" date=\"1\" subject=\"x\"><groups><group>a.b</group></groups><segments><segment bytes=\"10\" number=\"1\">a@b</segment></segments></file>\n</nzb>\n";

    fn script(p: &Path, body: &str) {
        fs::write(p, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Blu-ray complet (dossier BDMV) déposé dans l'inbox : catégorie « Film Blu-ray complet »,
    /// MediaInfo lancé sur la playlist principale, nom complété par MediaInfo, NFO créé.
    #[test]
    fn bluray_complet_de_l_inbox() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_bluray_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        let mi = d.join("mediainfo");
        let calls = d.join("calls");
        script(
            &mi,
            &format!(
                "for a; do last=\"$a\"; done\necho \"$*\" >> '{}'\ncase \"$*\" in\n *Inform=Video*) echo '1920|~|1080|~||~|AVC|~||~|' ;;\n *'Inform=Audio;%Language'*) echo 'fr|~||#|en|~||#|' ;;\n *Inform=Audio*) echo 'DTS|~|XLL|~|6|#|' ;;\n *Inform=Text*) echo '' ;;\n *Inform=General*) echo '02:00:00.000|~||~||~|' ;;\n *) echo \"<MediaInfo><File>$(basename \"$last\")</File></MediaInfo>\" ;;\nesac\n",
                calls.display()
            ),
        );
        let mut a = crate::prep::IndexerCfg::new("A");
        a.mode = "newznab".into();
        a.api_url = "http://127.0.0.1:9/api".into();
        a.api_key = "k".into();
        a.send_nfo = true;
        let cfg = Config {
            inbox: d.join("inbox").display().to_string(),
            ngpost_dir: d.join("sortie").display().to_string(),
            indexer_dir: d.join("idx").display().to_string(),
            mediainfo: mi.display().to_string(),
            stable_secs: 0,
            api_upload: true,
            bdinfo_scan: false,
            indexers: vec![a],
            ..Default::default()
        };
        let _ = crate::ngpost::ensure_dirs(&cfg);
        let bd = d.join("inbox").join("Mon Film (2010)");
        fs::create_dir_all(bd.join("BDMV/PLAYLIST")).unwrap();
        fs::create_dir_all(bd.join("BDMV/STREAM")).unwrap();
        fs::write(bd.join("BDMV/index.bdmv"), b"INDX0200").unwrap();
        fs::write(bd.join("BDMV/STREAM/00001.m2ts"), vec![1u8; 4000]).unwrap();
        // playlist d'un clip, 2 h
        let mut m = b"MPLS0200".to_vec();
        m.extend_from_slice(&20u32.to_be_bytes());
        m.resize(20, 0);
        let item: Vec<u8> = [b"00001M2TS".as_slice(), &[0, 1, 0], &0u32.to_be_bytes(), &(45_000u32 * 7200).to_be_bytes()].concat();
        let pl: Vec<u8> = [&[0u8, 0, 0, 1, 0, 0][..], &(item.len() as u16).to_be_bytes(), &item].concat();
        m.extend_from_slice(&(pl.len() as u32).to_be_bytes());
        m.extend_from_slice(&pl);
        fs::write(bd.join("BDMV/PLAYLIST/00800.mpls"), m).unwrap();
        // ses fichiers ne sont pas des vidéos à traiter une par une
        assert!(crate::prep::list_videos(&d.join("inbox"), true).is_empty());
        let hist = Mutex::new(History::load(&d.join("h.tsv")));
        let mut recs = Vec::new();
        scan_once(&cfg, &hist, &AtomicBool::new(false), &mut |_, _, _, _| {}, &mut |r| recs.push(r.clone()));
        assert_eq!(recs.len(), 1, "{recs:?}");
        let r = &recs[0];
        assert_eq!(r.status, Status::Sent, "{}", r.reason);
        assert_eq!(r.release, "Mon.Film.2010.MULTi.1080p.COMPLETE.BLURAY");
        let rel = d.join("sortie").join(&r.release);
        assert!(rel.join("BDMV/STREAM/00001.m2ts").is_file());
        let calls = fs::read_to_string(&calls).unwrap();
        assert!(calls.contains("00800.mpls"), "MediaInfo sur la playlist principale : {calls}");
        let w = crate::ngpost::waiting_dir(&cfg).unwrap().join(&r.release);
        assert!(fs::read_to_string(w.join(format!("{}.nfo", r.release))).unwrap().contains("00800.mpls"), "NFO créé depuis MediaInfo");
        let report = fs::read_dir(&w).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().map(|e| e == "xml").unwrap_or(false));
        assert!(report.is_some(), "rapport MediaInfo à côté du NZB");
        let _ = fs::remove_dir_all(&d);
    }

    /// Release Blu-ray rangée comme les releases FBR : « Film\Sample, Proof, film.iso, film.nfo »
    /// déposée dans l'inbox → reconnue toute seule, MediaInfo de l'ISO, NFO d'origine gardé et
    /// envoyé (même « créer le NFO » décoché), contenu posté et fichiers joints dans le journal.
    #[test]
    fn bdinfo_modifie_joint_et_ajoute_au_nfo() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_fbrbd_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        let mi = d.join("mediainfo");
        script(
            &mi,
            "for a; do last=\"$a\"; done\ncase \"$*\" in\n *Inform=Video*) echo '1920|~|1080|~||~|AVC|~||~|' ;;\n *'Inform=Audio;%Language'*) echo 'fr|~||#|en|~||#|' ;;\n *Inform=Audio*) echo 'DTS|~|XLL|~|6|#|' ;;\n *Inform=Text*) echo '' ;;\n *Inform=General*) echo '02:00:00.000|~||~||~|' ;;\n *) echo \"<MediaInfo><File>$(basename \"$last\")</File><Height>1080</Height></MediaInfo>\" ;;\nesac\n",
        );
        let mut a = crate::prep::IndexerCfg::new("A");
        a.mode = "newznab".into();
        a.api_url = "http://127.0.0.1:9/api".into();
        a.api_key = "k".into();
        a.send_nfo = false;
        a.bdinfo_field = "bdinfo".into();
        let cfg = Config {
            inbox: d.join("inbox").display().to_string(),
            ngpost_dir: d.join("sortie").display().to_string(),
            indexer_dir: d.join("idx").display().to_string(),
            mediainfo: mi.display().to_string(),
            stable_secs: 0,
            api_upload: true,
            bdinfo_scan: false,
            bd_override: Some("BD PROPRE".into()),
            nfo_bd: "both".into(),
            mi_override: Some("<MediaInfo>MODIFIE</MediaInfo>".into()),
            indexers: vec![a.clone()],
            ..Default::default()
        };
        let _ = crate::ngpost::ensure_dirs(&cfg);
        let rel_dir = d.join("inbox").join("Film.2010.MULTi.COMPLETE.BLURAY-GRP");
        fs::create_dir_all(rel_dir.join("Sample")).unwrap();
        fs::create_dir_all(rel_dir.join("Proof")).unwrap();
        fs::write(rel_dir.join("Sample").join("film-sample.mkv"), vec![1u8; 500]).unwrap();
        fs::write(rel_dir.join("Proof").join("proof.jpg"), b"jpg").unwrap();
        fs::write(rel_dir.join("film.nfo"), "NFO DU GROUPE").unwrap();
        let mut iso = vec![0u8; 400_000];
        iso[200_000..200_005].copy_from_slice(b"\x08BDMV");
        fs::write(rel_dir.join("film.iso"), &iso).unwrap();
        let hist = Mutex::new(History::load(&d.join("h.tsv")));
        let mut recs = Vec::new();
        scan_once(&cfg, &hist, &AtomicBool::new(false), &mut |_, _, _, _| {}, &mut |r| recs.push(r.clone()));
        let r = &recs[0];
        assert_eq!(r.status, Status::Sent, "{}", r.reason);
        let w = crate::ngpost::waiting_dir(&cfg).unwrap().join(&r.release);
        // BDInfo modifié livré tel quel, ajouté sous le NFO d'origine
        assert_eq!(fs::read_to_string(w.join(format!("{}.bdinfo.txt", r.release))).unwrap(), "BD PROPRE");
        assert_eq!(fs::read_to_string(w.join(format!("{}.nfo", r.release))).unwrap(), "NFO DU GROUPE\n\nBD PROPRE");
        // rapport MediaInfo modifié livré tel quel
        let xml = fs::read_dir(&w).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().map(|x| x == "xml").unwrap_or(false)).unwrap();
        assert_eq!(fs::read_to_string(xml).unwrap(), "<MediaInfo>MODIFIE</MediaInfo>");
        // BDInfo dans son champ chez l'indexer
        fs::write(w.join(format!("{}.nzb", r.release)), NZB).unwrap();
        let f = crate::upload::release_files(&w).unwrap();
        let args = crate::upload::nzbadd_args(&a, &f, &r.release).join(" ");
        assert!(args.contains("bdinfo=@") && args.contains(".bdinfo.txt"), "{args}");
        let sum = crate::ngpost::deposit_summary(&a, &r.release, &w).join("\n");
        assert!(sum.contains("suivi du rapport BDInfo") && sum.contains("champ « bdinfo »"), "{sum}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn release_bluray_iso_avec_nfo_sample_proof() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_fbr_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        let mi = d.join("mediainfo");
        script(
            &mi,
            "for a; do last=\"$a\"; done\ncase \"$*\" in\n *Inform=Video*) echo '1920|~|1080|~||~|AVC|~||~|' ;;\n *'Inform=Audio;%Language'*) echo 'fr|~||#|en|~||#|' ;;\n *Inform=Audio*) echo 'DTS|~|XLL|~|6|#|' ;;\n *Inform=Text*) echo '' ;;\n *Inform=General*) echo '02:00:00.000|~||~||~|' ;;\n *) echo \"<MediaInfo><File>$(basename \"$last\")</File><Height>1080</Height></MediaInfo>\" ;;\nesac\n",
        );
        let mut a = crate::prep::IndexerCfg::new("A");
        a.mode = "newznab".into();
        a.api_url = "http://127.0.0.1:9/api".into();
        a.api_key = "k".into();
        a.send_nfo = false;
        let cfg = Config {
            inbox: d.join("inbox").display().to_string(),
            ngpost_dir: d.join("sortie").display().to_string(),
            indexer_dir: d.join("idx").display().to_string(),
            mediainfo: mi.display().to_string(),
            stable_secs: 0,
            api_upload: true,
            bdinfo_scan: false,
            indexers: vec![a.clone()],
            ..Default::default()
        };
        let _ = crate::ngpost::ensure_dirs(&cfg);
        let rel_dir = d.join("inbox").join("Film.2010.MULTi.COMPLETE.BLURAY-GRP");
        fs::create_dir_all(rel_dir.join("Sample")).unwrap();
        fs::create_dir_all(rel_dir.join("Proof")).unwrap();
        fs::write(rel_dir.join("Sample").join("film-sample.mkv"), vec![1u8; 500]).unwrap();
        fs::write(rel_dir.join("Proof").join("proof.jpg"), b"jpg").unwrap();
        fs::write(rel_dir.join("film.nfo"), "NFO DU GROUPE").unwrap();
        let mut iso = vec![0u8; 400_000];
        iso[200_000..200_005].copy_from_slice(b"\x08BDMV");
        fs::write(rel_dir.join("film.iso"), &iso).unwrap();
        let hist = Mutex::new(History::load(&d.join("h.tsv")));
        let mut recs = Vec::new();
        scan_once(&cfg, &hist, &AtomicBool::new(false), &mut |_, _, _, _| {}, &mut |r| recs.push(r.clone()));
        assert_eq!(recs.len(), 1, "un seul traitement (pas l'échantillon) : {recs:?}");
        let r = &recs[0];
        assert_eq!(r.status, Status::Sent, "{}", r.reason);
        assert_eq!(r.release, "Film.2010.MULTi.COMPLETE.BLURAY-GRP");
        assert!(r.codec.contains("Blu-ray"), "catégorie : {}", r.codec);
        let out = d.join("sortie").join(&r.release);
        assert!(out.join("film.iso").is_file() && out.join("Sample/film-sample.mkv").is_file() && out.join("film.nfo").is_file());
        let w = crate::ngpost::waiting_dir(&cfg).unwrap().join(&r.release);
        assert_eq!(fs::read_to_string(w.join(format!("{}.nfo", r.release))).unwrap(), "NFO DU GROUPE");
        assert_eq!(crate::ngpost::nfo_origin(&w), "origine:film.nfo");
        assert!(crate::ngpost::report_required(&w) && crate::ngpost::has_report(&w), "MediaInfo exigé et présent");
        let list = crate::ngpost::read_content_list(&w);
        assert!(list.iter().any(|(p, _)| p == "film.iso") && list.iter().any(|(p, _)| p == "Sample/film-sample.mkv"), "{list:?}");
        // au dépôt : NFO d'origine joint même sans « créer le NFO », et tout est détaillé
        fs::write(w.join(format!("{}.nzb", r.release)), NZB).unwrap();
        let f = crate::upload::release_files(&w).unwrap();
        assert!(f.nfo_original && crate::upload::send_nfo(&a, &f));
        let args = crate::upload::nzbadd_args(&a, &f, &r.release).join(" ");
        assert!(args.contains("nfo=@") && args.contains("mediainfo=@") && args.contains("cat=2050"), "{args}");
        let sum = crate::ngpost::deposit_summary(&a, &r.release, &w).join("\n");
        assert!(sum.contains("NFO d'origine (film.nfo)") && sum.contains("champ « nfo »") && sum.contains("champ « mediainfo »") && sum.contains("film.iso") && sum.contains("1 fichier(s) postés"), "{sum}");
        assert!(sum.contains("Groupes Usenet (d'après le NZB) : a.b"), "{sum}");
        let _ = fs::remove_dir_all(&d);
    }

    /// Téléchargement terminé reposté : nom du téléchargement repris (fichier obfusqué), NFO
    /// du dossier repris, release préparée seulement pour l'indexer choisi (pas celui d'origine).
    #[test]
    fn telechargement_reposte_ailleurs() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_repost_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        let mi = d.join("mediainfo");
        script(&mi, "case \"$1\" in\n --Inform=Video*) printf '1920|~|1080|~|x264|~|AVC|~|;' ;;\n --Inform=Audio*) printf 'fr;' ;;\n --Inform=Text*) printf '' ;;\n --Inform=General*) printf '01:30:00.000|~||~||~|' ;;\n *) printf '<MediaInfo/>' ;;\nesac\n");
        let ix = |n: &str| crate::prep::IndexerCfg::new(n);
        let cfg = Config {
            ngpost_dir: d.join("sortie").display().to_string(),
            indexer_dir: d.join("idx").display().to_string(),
            mediainfo: mi.display().to_string(),
            indexers: vec![ix("Origine"), ix("Autre")],
            targets: crate::dl::repost_targets("Origine", &["Origine".into(), "Autre".into()]),
            ..Default::default()
        };
        let _ = crate::ngpost::ensure_dirs(&cfg);
        let dl = d.join("complete").join("Film.2020.FRENCH.1080p.WEB.x264-GRP");
        fs::create_dir_all(&dl).unwrap();
        fs::write(dl.join("a8f3c2e1.mkv"), vec![1u8; 2000]).unwrap();
        fs::write(dl.join("film.nfo"), "NFO").unwrap();
        let hist = Mutex::new(History::load(&d.join("h.tsv")));
        let recs = process_path(&dl, &cfg, Transfer::Copy, Some("Film.2020.FRENCH.1080p.WEB.x264-GRP"), &hist, &mut |_, _, _| {});
        assert_eq!(recs.len(), 1, "{recs:?}");
        assert_eq!(recs[0].status, Status::Sent, "{}", recs[0].reason);
        assert_eq!(recs[0].release, "Film.2020.FRENCH.1080p.WEB.x264-GRP");
        let plan = crate::ngpost::read_plan(&cfg, &recs[0].release).unwrap();
        assert_eq!(plan.resolve(&cfg), vec!["Autre".to_string()]);
        let w = crate::ngpost::waiting_dir(&cfg).unwrap().join(&recs[0].release);
        assert_eq!(fs::read_to_string(w.join(format!("{}.nfo", recs[0].release))).unwrap(), "NFO");
        // copie : le téléchargement reste chez le client
        assert!(dl.join("a8f3c2e1.mkv").is_file());
        let _ = fs::remove_dir_all(&d);
    }

    /// NFO d'origine retiré de la release postée et/ou pas envoyé aux indexers.
    #[test]
    fn nfo_garde_ou_non_envoye_ou_non() {
        for (keep, send) in [(false, true), (true, false), (false, false)] {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_nfochoix_{}_{keep}_{send}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        let mi = d.join("mediainfo");
        script(
            &mi,
            "for a; do last=\"$a\"; done\ncase \"$*\" in\n *Inform=Video*) echo '1920|~|1080|~||~|AVC|~||~|' ;;\n *'Inform=Audio;%Language'*) echo 'fr|~||#|en|~||#|' ;;\n *Inform=Audio*) echo 'DTS|~|XLL|~|6|#|' ;;\n *Inform=Text*) echo '' ;;\n *Inform=General*) echo '02:00:00.000|~||~||~|' ;;\n *) echo \"<MediaInfo><File>$(basename \"$last\")</File><Height>1080</Height></MediaInfo>\" ;;\nesac\n",
        );
        let mut a = crate::prep::IndexerCfg::new("A");
        a.mode = "newznab".into();
        a.api_url = "http://127.0.0.1:9/api".into();
        a.api_key = "k".into();
        a.send_nfo = false;
        let cfg = Config {
            inbox: d.join("inbox").display().to_string(),
            ngpost_dir: d.join("sortie").display().to_string(),
            indexer_dir: d.join("idx").display().to_string(),
            mediainfo: mi.display().to_string(),
            stable_secs: 0,
            api_upload: true,
            bdinfo_scan: false,
            nfo_keep: keep,
            nfo_send: send,
            indexers: vec![a.clone()],
            ..Default::default()
        };
        let _ = crate::ngpost::ensure_dirs(&cfg);
        let rel_dir = d.join("inbox").join("Film.2010.MULTi.COMPLETE.BLURAY-GRP");
        fs::create_dir_all(rel_dir.join("Sample")).unwrap();
        fs::create_dir_all(rel_dir.join("Proof")).unwrap();
        fs::write(rel_dir.join("Sample").join("film-sample.mkv"), vec![1u8; 500]).unwrap();
        fs::write(rel_dir.join("Proof").join("proof.jpg"), b"jpg").unwrap();
        fs::write(rel_dir.join("film.nfo"), "NFO DU GROUPE").unwrap();
        let mut iso = vec![0u8; 400_000];
        iso[200_000..200_005].copy_from_slice(b"\x08BDMV");
        fs::write(rel_dir.join("film.iso"), &iso).unwrap();
        let hist = Mutex::new(History::load(&d.join("h.tsv")));
        let mut recs = Vec::new();
        scan_once(&cfg, &hist, &AtomicBool::new(false), &mut |_, _, _, _| {}, &mut |r| recs.push(r.clone()));
        assert_eq!(recs.len(), 1, "{recs:?}");
        let r = &recs[0];
        assert_eq!(r.status, Status::Sent, "{}", r.reason);
        let out = d.join("sortie").join(&r.release);
        assert!(out.join("film.iso").is_file());
        assert_eq!(out.join("film.nfo").is_file(), keep, "gardé dans la release postée : {keep}");
        let w = crate::ngpost::waiting_dir(&cfg).unwrap().join(&r.release);
        let sent = w.join(format!("{}.nfo", r.release));
        assert_eq!(sent.is_file(), send, "envoyé : {send}");
        if send {
            assert_eq!(fs::read_to_string(&sent).unwrap(), "NFO DU GROUPE");
        } else {
            // pas de NFO créé à la place (« créer le NFO » coché par défaut)
            assert_eq!(crate::ngpost::nfo_origin(&w), "non-envoye");
            fs::write(w.join(format!("{}.nzb", r.release)), NZB).unwrap();
            let sum = crate::ngpost::deposit_summary(&a, &r.release, &w).join("\n");
            assert!(sum.contains("NON envoyé"), "{sum}");
        }
        let list = crate::ngpost::read_content_list(&w);
        assert_eq!(list.iter().any(|(p, _)| p == "film.nfo"), keep, "{list:?}");
        let _ = fs::remove_dir_all(&d);
        }
    }

    /// Dossier de film déposé dans l'inbox (film.mkv, film.nfo, Sample) : la vidéo est traitée
    /// avec le nom du dossier, le NFO d'origine est repris, l'échantillon ignoré.
    #[test]
    fn dossier_de_film_dans_l_inbox() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_dossierfilm_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        let mi = d.join("mediainfo");
        script(&mi, "case \"$*\" in\n *Inform=Video*) echo '1920|~|1080|~|x264|~|AVC|~||~|' ;;\n *'Inform=Audio;%Language'*) echo 'fr|~||#|' ;;\n *Inform=Audio*) echo 'AC-3|~||~|6|#|' ;;\n *Inform=Text*) echo '' ;;\n *Inform=General*) echo '01:50:00.000|~||~||~|' ;;\n *) echo '<MediaInfo/>' ;;\nesac\n");
        let cfg = Config {
            inbox: d.join("inbox").display().to_string(),
            ngpost_dir: d.join("sortie").display().to_string(),
            indexer_dir: d.join("idx").display().to_string(),
            mediainfo: mi.display().to_string(),
            stable_secs: 0,
            indexers: vec![crate::prep::IndexerCfg::new("A")],
            ..Default::default()
        };
        let _ = crate::ngpost::ensure_dirs(&cfg);
        let rel_dir = d.join("inbox").join("Film.2010.FRENCH.1080p.WEB.x264-GRP");
        fs::create_dir_all(rel_dir.join("Sample")).unwrap();
        fs::write(rel_dir.join("grp-film.mkv"), vec![1u8; 2000]).unwrap();
        fs::write(rel_dir.join("Sample").join("grp-film-sample.mkv"), vec![1u8; 100]).unwrap();
        fs::write(rel_dir.join("grp-film.nfo"), "NFO").unwrap();
        // une vidéo seule, sans NFO : NFO créé depuis MediaInfo (« Créer un NFO », par défaut)
        fs::write(d.join("inbox").join("Autre.Film.2011.FRENCH.1080p.WEB.x264-GRP.mkv"), vec![1u8; 2000]).unwrap();
        let hist = Mutex::new(History::load(&d.join("h.tsv")));
        let mut recs = Vec::new();
        scan_once(&cfg, &hist, &AtomicBool::new(false), &mut |_, _, _, _| {}, &mut |r| recs.push(r.clone()));
        assert_eq!(recs.len(), 2, "{recs:?}");
        recs.sort_by(|a, b| b.release.cmp(&a.release));
        assert_eq!(recs[0].status, Status::Sent, "{}", recs[0].reason);
        assert_eq!(recs[0].release, "Film.2010.FRENCH.1080p.WEB.x264-GRP");
        let w = crate::ngpost::waiting_dir(&cfg).unwrap().join(&recs[0].release);
        assert_eq!(crate::ngpost::nfo_origin(&w), "origine:grp-film.nfo");
        let w2 = crate::ngpost::waiting_dir(&cfg).unwrap().join(&recs[1].release);
        assert_eq!(crate::ngpost::nfo_origin(&w2), "mediainfo");
        assert!(w2.join(format!("{}.nfo", recs[1].release)).is_file());
        // second passage : rien de plus (l'échantillon n'est pas un film)
        let mut more = Vec::new();
        scan_once(&cfg, &hist, &AtomicBool::new(false), &mut |_, _, _, _| {}, &mut |r| more.push(r.clone()));
        assert!(more.is_empty(), "{more:?}");
        let _ = fs::remove_dir_all(&d);
    }

    /// Jeu déposé dans « @Jeux PC » → release dans la sortie → post compressé (imposé par la
    /// catégorie) → distribution : la catégorie et le NFO suivent jusqu'aux indexers.
    #[test]
    fn contenu_de_l_inbox_jusqu_a_l_indexer() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_contenu_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        let ng = d.join("ngPost");
        fs::write(d.join("ok.nzb"), NZB).unwrap();
        script(&ng, &format!("echo \"$@\" > '{}'\nwhile [ $# -gt 0 ]; do case $1 in -o) out=$2; shift;; esac; shift; done\ncp '{}' \"$out\"\n", d.join("args").display(), d.join("ok.nzb").display()));
        let mut a = crate::prep::IndexerCfg::new("A");
        a.cat_map.push(("jeux-pc".into(), "4051".into()));
        let mut cfg = Config {
            inbox: d.join("inbox").display().to_string(),
            ngpost_dir: d.join("sortie").display().to_string(),
            indexer_dir: d.join("idx").display().to_string(),
            ngpost_exe: ng.display().to_string(),
            stable_secs: 0,
            ng_compress: false,
            post_after: "keep".into(),
            indexers: vec![a, crate::prep::IndexerCfg::new("B")],
            ..Default::default()
        };
        for c in cfg.content_cats.iter_mut().filter(|c| c.key == "jeux-pc") {
            c.inbox = true;
        }
        let _ = crate::ngpost::ensure_dirs(&cfg);
        let hist = Mutex::new(History::load(&d.join("h.tsv")));
        // premier passage : le dossier « @Jeux PC » est créé dans l'inbox
        assert_eq!(scan_once(&cfg, &hist, &AtomicBool::new(false), &mut |_, _, _, _| {}, &mut |_| {}), 0);
        let game = d.join("inbox").join("@Jeux PC").join("Super Jeu v1.04 MULTi PC");
        fs::create_dir_all(game.join("Crack")).unwrap();
        fs::write(game.join("setup.iso"), vec![7u8; 3000]).unwrap();
        fs::write(game.join("Crack").join("game.exe"), b"exe").unwrap();
        fs::write(game.join("jeu.nfo"), b"NFO du jeu").unwrap();
        let mut recs = Vec::new();
        let n = scan_once(&cfg, &hist, &AtomicBool::new(false), &mut |_, _, _, _| {}, &mut |r| recs.push(r.clone()));
        assert_eq!(n, 1, "{recs:?}");
        let r = &recs[0];
        assert_eq!(r.status, Status::Sent, "{}", r.reason);
        assert_eq!(r.release, "Super.Jeu.v1.04.MULTi.PC");
        let rel = d.join("sortie").join(&r.release);
        assert!(rel.join("setup.iso").is_file() && rel.join("Crack").join("game.exe").is_file());
        assert!(!game.exists() && d.join("inbox").join("@Jeux PC").is_dir());
        assert!(crate::ngpost::list_unposted(&cfg).contains(&rel));
        // la catégorie impose la compression, même coupée dans les options du post
        let p = crate::ngpost::post_file(&cfg, &rel, &r.release, &mut |_| {}, &mut |_| {}).unwrap();
        let args = fs::read_to_string(d.join("args")).unwrap();
        assert!(args.contains("--compress"), "{args}");
        assert!(p.nzb.is_file());
        // distribution : catégorie et NFO suivent la release chez chaque indexer
        let out = crate::ngpost::distribute(&cfg);
        assert_eq!(out.len(), 1);
        let cat_of = |idx: &str| {
            let dir = d.join("idx").join(idx).join(crate::ngpost::PENDING_DIR).join(&r.release);
            let f = crate::upload::release_files(&dir).unwrap();
            assert!(f.nfo.is_some(), "NFO absent chez {idx}");
            let icfg = cfg.indexers.iter().find(|i| i.name == idx).unwrap();
            crate::upload::category_for(icfg, &r.release, &f)
        };
        assert_eq!(cat_of("A"), "4051", "catégorie propre à l'indexer A");
        assert_eq!(cat_of("B"), "4050", "catégorie par défaut de « Jeux PC »");
        // un film garde ses catégories habituelles
        let film = crate::upload::Files::default();
        assert_eq!(crate::upload::category_for(&cfg.indexers[1], "Film.2020.FRENCH.1080p.WEB.x264", &film), "2040");
        cfg.ng_compress = true;
        let _ = fs::remove_dir_all(&d);
    }

    /// Jeu PS2 de bout en bout : liste de catégories importée en texte (avec familles sans
    /// numéro), jeu reconnu « Jeux consoles », plateforme PS2 → catégorie PS2 de l'indexer,
    /// groupes de jeux (jamais ceux des films réglés par défaut), NFO d'origine joint.
    #[test]
    fn jeu_ps2_categorie_groupes_nfo() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_ps2_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("sortie")).unwrap();
        let ng = d.join("ngPost");
        fs::write(d.join("ok.nzb"), NZB).unwrap();
        script(&ng, &format!("echo \"$@\" > '{}'\nwhile [ $# -gt 0 ]; do case $1 in -o) out=$2; shift;; esac; shift; done\ncp '{}' \"$out\"\n", d.join("args").display(), d.join("ok.nzb").display()));
        let mut a = crate::prep::IndexerCfg::new("A");
        let (list, skipped) = crate::upload::parse_category_report("Catégories de l'indexer\nConsole\n  1010 NDS\n  1080 PS2\n  1081 PS3\nPC : 4050 Jeux\n2040 Films > HD\n");
        assert!(skipped.is_empty(), "{skipped:?}");
        a.cat_list = list;
        assert!(a.cat_list.contains(&("1080".to_string(), "Console > PS2".to_string())), "{:?}", a.cat_list);
        let cfg = Config {
            inbox: d.join("inbox").display().to_string(),
            ngpost_dir: d.join("sortie").display().to_string(),
            indexer_dir: d.join("idx").display().to_string(),
            ngpost_exe: ng.display().to_string(),
            stable_secs: 0,
            ng_compress: false,
            ng_groups: "alt.binaries.movies".into(),
            post_after: "keep".into(),
            indexers: vec![a, crate::prep::IndexerCfg::new("B")],
            ..Default::default()
        };
        let cats = cfg.content_cats.clone();
        let game = d.join("Gran.Turismo.4.PAL.PS2-GRP");
        fs::create_dir_all(&game).unwrap();
        // image de jeu PS2 : identifiant « PLAYSTATION », SYSTEM.CNF (et un VIDEO_TS de cinématiques)
        let mut iso = vec![0u8; 40000];
        iso[32776..32787].copy_from_slice(b"PLAYSTATION");
        iso[34000..34010].copy_from_slice(b"SYSTEM.CNF");
        iso[35000..35008].copy_from_slice(b"VIDEO_TS");
        fs::write(game.join("gt4.iso"), iso).unwrap();
        assert!(crate::disc::detect(&game).is_none(), "un jeu n'est pas un DVD vidéo");
        // même sans plateforme dans le nom (« PAL » ferait croire à un DVD)
        let bare = d.join("Gran.Turismo.4.PAL-GRP");
        fs::create_dir_all(&bare).unwrap();
        fs::copy(game.join("gt4.iso"), bare.join("gt4.iso")).unwrap();
        assert!(crate::disc::detect(&bare).is_none());
        fs::write(game.join("gt4.nfo"), b"NFO du jeu PS2").unwrap();
        assert_eq!(crate::content::guess(&game, &cats).as_deref(), Some("consoles"));
        let v = crate::content::detect(&cfg, &game);
        assert_eq!(crate::content::get(&v, "plateforme"), "PS2");
        let cat = cats.iter().find(|c| c.key == "consoles").unwrap().clone();
        let _ = crate::ngpost::ensure_dirs(&cfg);
        let hist = Mutex::new(History::load(&d.join("h.tsv")));
        let rel = "Gran.Turismo.4.PAL.PS2-GRP";
        let r = run_content(&game, &cat, rel, &cfg, crate::prep::Transfer::Copy, None, Mode::Manual, false, &hist, &mut |_, _, _| {});
        assert_eq!(r.status, Status::Sent, "{}", r.reason);
        let out = d.join("sortie").join(rel);
        let p = crate::ngpost::post_file(&cfg, &out, rel, &mut |_| {}, &mut |_| {}).unwrap();
        assert!(p.nzb.is_file());
        let args = fs::read_to_string(d.join("args")).unwrap();
        assert!(args.contains("alt.binaries.games") && !args.contains("alt.binaries.movies"), "groupes : {args}");
        crate::ngpost::distribute(&cfg);
        let files_of = |idx: &str| crate::upload::release_files(&d.join("idx").join(idx).join(crate::ngpost::PENDING_DIR).join(rel)).unwrap();
        let fa = files_of("A");
        assert!(fa.nfo.is_some() && fa.nfo_original, "NFO d'origine joint");
        assert_eq!(crate::upload::category_for(&cfg.indexers[0], rel, &fa), "1080", "catégorie PS2 de l'indexer A");
        assert_eq!(crate::upload::category_for(&cfg.indexers[1], rel, &files_of("B")), "1000", "B : catégorie par défaut des consoles");
        let _ = fs::remove_dir_all(&d);
    }

    /// Série déposée dans l'inbox → release « Série/Saison N/épisodes » dans la sortie →
    /// post du dossier par NgPost (compression : dossier de la série ; sinon : chaque fichier).
    #[test]
    fn serie_de_l_inbox_au_post() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_serie_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let root = d.join("inbox").join("Breaking Bad (2008)");
        for (s, e) in [(1, 1), (1, 2), (2, 1), (2, 2)] {
            let p = root.join(format!("Saison {s}")).join(format!("Breaking.Bad.S{s:02}E{e:02}.mkv"));
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"video").unwrap();
        }
        fs::write(root.join("Saison 1").join("Breaking.Bad.S01E01.fr.srt"), b"sub").unwrap();
        fs::create_dir_all(d.join("sortie")).unwrap();
        let mi = d.join("mediainfo");
        script(
            &mi,
            "for a; do f=\"$a\"; done\ncase \"$1\" in\n  --Inform=Video*) printf '1920|~|1080|~|x264|~|AVC|~|;' ;;\n  --Inform=Audio*) printf 'fr;' ;;\n  --Inform=Text*) printf '' ;;\n  --Inform=General*) printf '00:45:00.000|~||~||~|' ;;\n  *) printf '<MediaInfo><File>%s</File></MediaInfo>\\n' \"$(basename \"$f\")\" ;;\nesac\n",
        );
        let ng = d.join("ngPost");
        fs::write(d.join("ok.nzb"), NZB).unwrap();
        script(&ng, &format!("echo \"$@\" > '{}'\nwhile [ $# -gt 0 ]; do case $1 in -o) out=$2; shift;; esac; shift; done\ncp '{}' \"$out\"\n", d.join("args").display(), d.join("ok.nzb").display()));
        let mut cfg = Config {
            inbox: d.join("inbox").display().to_string(),
            ngpost_dir: d.join("sortie").display().to_string(),
            indexer_dir: d.join("idx").display().to_string(),
            mediainfo: mi.display().to_string(),
            ngpost_exe: ng.display().to_string(),
            stable_secs: 0,
            ng_compress: true,
            post_after: "keep".into(),
            ..Default::default()
        };
        let _ = crate::ngpost::ensure_dirs(&cfg);
        let hist = Mutex::new(History::load(&d.join("h.tsv")));
        let mut recs = Vec::new();
        let n = scan_once(&cfg, &hist, &AtomicBool::new(false), &mut |_, _, _, _| {}, &mut |r| recs.push(r.clone()));
        assert_eq!(n, 1, "{recs:?}");
        let r = &recs[0];
        assert_eq!(r.status, Status::Sent, "{}", r.reason);
        assert_eq!(r.release, "Breaking.Bad.2008.S01-S02.COMPLETE.FRENCH.1080p.x264");
        let rel = d.join("sortie").join(&r.release);
        assert_eq!(PathBuf::from(&r.dest), rel);
        // ce que verra celui qui télécharge : release / Saison N / épisodes (et sous-titres)
        for f in ["Saison 1/Breaking.Bad.S01E01.mkv", "Saison 1/Breaking.Bad.S01E01.fr.srt", "Saison 2/Breaking.Bad.S02E02.mkv"] {
            assert!(rel.join(f).is_file(), "{f} absent");
        }
        assert!(!root.exists(), "l'inbox doit être vidée");
        // rapport MediaInfo du premier épisode, au nom de la release
        assert!(fs::read_to_string(&r.xml).unwrap().contains("Breaking.Bad.S01E01.mkv"));
        // le dossier de la release est proposé au post, et NgPost reçoit le dossier de la série
        assert!(crate::ngpost::list_unposted(&cfg).contains(&rel));
        // marquée série : catégories et groupes des séries, quel que soit le nom
        assert!(crate::ngpost::is_marked_series(&crate::ngpost::waiting_dir(&cfg).unwrap().join(&r.release)));
        let p = crate::ngpost::post_file(&cfg, &rel, &r.release, &mut |_| {}, &mut |_| {}).unwrap();
        let args = fs::read_to_string(d.join("args")).unwrap();
        // NgPost reçoit le contenu de la release : chaque dossier « Saison N »
        assert!(args.contains(&format!("-i {} -i {}", rel.join("Saison 1").display(), rel.join("Saison 2").display())) && args.contains("--compress"), "{args}");
        assert!(p.nzb.is_file());
        // sans compression : chaque fichier, à plat
        cfg.ng_compress = false;
        let _ = crate::ngpost::post_file(&cfg, &rel, "Autre", &mut |_| {}, &mut |_| {});
        let args = fs::read_to_string(d.join("args")).unwrap();
        assert_eq!(args.matches("-i ").count(), 5, "{args}");
        // une release par saison, épisodes renommés
        let root2 = d.join("inbox").join("Dark");
        for e in 1..=3 {
            let p = root2.join("S01").join(format!("dark {e:02}.mkv"));
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"v").unwrap();
        }
        let p = root2.join("S02").join("Dark.S02E01.mkv");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, b"v").unwrap();
        cfg.series_round_whole = false;
        cfg.series_round_seasons = true;
        cfg.series_rename = true;
        cfg.group = "GRP".into();
        recs.clear();
        // « dark 01 » : numéros seuls, la saison vient du dossier S01
        let n = scan_once(&cfg, &hist, &AtomicBool::new(false), &mut |_, _, _, _| {}, &mut |r| recs.push(r.clone()));
        assert_eq!(n, 2, "{recs:?}");
        let names: Vec<&str> = recs.iter().map(|r| r.release.as_str()).collect();
        // une release par saison : jamais « COMPLETE » d'office (une saison ne dit rien de la série)
        assert_eq!(names, vec!["Dark.S01.FRENCH.1080p.x264-GRP", "Dark.S02.FRENCH.1080p.x264-GRP"]);
        // une saison : épisodes directement dans la release
        assert!(d.join("sortie").join(names[0]).join("Dark.S01E03.FRENCH.1080p.x264-GRP.mkv").is_file());
        let _ = fs::remove_dir_all(&d);
    }

    /// Série en trois tours : toute la série (nommée à la résolution majoritaire, avertissement
    /// pour la saison en 720p), puis une release par saison et une par épisode faites par liens
    /// physiques depuis la première (aucune copie de plus), chacune à sa résolution.
    #[test]
    fn serie_en_trois_tours_par_liens_physiques() {
        use std::os::unix::fs::MetadataExt;
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_tours_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let root = d.join("src").join("Dark");
        for (s, e) in [(1, 1), (1, 2), (1, 3), (2, 1), (2, 2)] {
            let p = root.join(format!("Saison {s}")).join(format!("Dark.S{s:02}E{e:02}.mkv"));
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, format!("video {s} {e}")).unwrap();
        }
        fs::write(root.join("dark.nfo"), "NFO DE LA SERIE").unwrap();
        fs::create_dir_all(d.join("sortie")).unwrap();
        let mi = d.join("mediainfo");
        // saison 2 en 720p, le reste en 1080p
        script(
            &mi,
            "for a; do f=\"$a\"; done\ncase \"$1\" in\n  --Inform=Video*) case \"$(basename \"$f\")\" in *S02*) printf '1280|~|720|~|x264|~|AVC|~|;' ;; *) printf '1920|~|1080|~|x264|~|AVC|~|;' ;; esac ;;\n  --Inform=Audio*) printf 'fr;' ;;\n  --Inform=Text*) printf '' ;;\n  --Inform=General*) printf '00:45:00.000|~||~||~|' ;;\n  *) printf '<MediaInfo><File>%s</File></MediaInfo>\\n' \"$(basename \"$f\")\" ;;\nesac\n",
        );
        let cfg = Config {
            ngpost_dir: d.join("sortie").display().to_string(),
            indexer_dir: d.join("idx").display().to_string(),
            mediainfo: mi.display().to_string(),
            group: "GRP".into(),
            series_round_whole: true,
            series_round_seasons: true,
            series_round_episodes: true,
            series_auto_complete: true,
            ..Default::default()
        };
        let _ = crate::ngpost::ensure_dirs(&cfg);
        // ce qui sera créé, annoncé avant
        let (_, plan) = series_plan(&root, &cfg).unwrap();
        assert_eq!(plan.len(), 1 + 2 + 5, "{plan:?}");
        let hist = Mutex::new(History::load(&d.join("h.tsv")));
        let recs = run_series_with(&root, &cfg, Transfer::Copy, None, Mode::Manual, false, &hist, &mut |_, _, _| {});
        assert_eq!(recs.len(), 8, "{recs:?}");
        assert!(recs.iter().all(|r| r.status == Status::Sent), "{recs:?}");
        let names: Vec<&str> = recs.iter().map(|r| r.release.as_str()).collect();
        assert_eq!(&names[..3], &["Dark.S01-S02.COMPLETE.FRENCH.1080p.x264-GRP", "Dark.S01.FRENCH.1080p.x264-GRP", "Dark.S02.FRENCH.720p.x264-GRP"], "{names:?}");
        // épisodes : nom complet (le fichier n'a pas de tag), chacun à sa résolution
        assert_eq!(
            &names[3..],
            &["Dark.S01E01.FRENCH.1080p.x264-GRP", "Dark.S01E02.FRENCH.1080p.x264-GRP", "Dark.S01E03.FRENCH.1080p.x264-GRP", "Dark.S02E01.FRENCH.720p.x264-GRP", "Dark.S02E02.FRENCH.720p.x264-GRP"],
            "{names:?}"
        );
        // avertissement de résolution sur la release de toute la série et la saison 2
        assert!(recs[0].reason.contains("720p"), "{}", recs[0].reason);
        assert!(recs[2].reason.contains("720p") && recs[2].reason.contains("1080p"), "{}", recs[2].reason);
        // liens physiques : même fichier sur le disque que dans la release de toute la série
        let whole = PathBuf::from(&recs[0].dest);
        let a = whole.join("Saison 2").join("Dark.S02E01.mkv");
        let b = PathBuf::from(&recs[2].dest).join("Dark.S02E01.mkv");
        assert!(a.is_file() && b.is_file(), "{} / {}", a.display(), b.display());
        assert_eq!(fs::metadata(&a).unwrap().ino(), fs::metadata(&b).unwrap().ino(), "saison : lien physique");
        let ep = PathBuf::from(&recs[6].dest);
        assert_eq!(fs::metadata(&ep).unwrap().ino(), fs::metadata(&a).unwrap().ino(), "épisode : lien physique");
        // NFO de la série (racine) : gardé à la racine de chaque release, et envoyé
        assert!(whole.join("dark.nfo").is_file() && PathBuf::from(&recs[1].dest).join("dark.nfo").is_file());
        let w = crate::ngpost::waiting_dir(&cfg).unwrap();
        assert_eq!(fs::read_to_string(w.join(&recs[0].release).join(format!("{}.nfo", recs[0].release))).unwrap(), "NFO DE LA SERIE");
        // la source est gardée (copie)
        assert!(root.join("Saison 1").join("Dark.S01E01.mkv").is_file());
        let _ = fs::remove_dir_all(&d);
    }

    /// Rapports seuls : MediaInfo et NFO à côté du fichier, NFO existant jamais écrasé, rien
    /// d'autre (pas de renommage, pas de copie, pas de post).
    #[test]
    fn rapports_seuls_sans_rien_toucher() {
        let _verrou = crate::prep::test_lock();
        let d = std::env::temp_dir().join(format!("nzbff_rapports_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let mi = d.join("mediainfo");
        script(&mi, "for a; do last=\"$a\"; done\ncase \"$*\" in\n *Output=XML*|*--Output=XML*) echo \"<MediaInfo><File>$(basename \"$last\")</File></MediaInfo>\" ;;\n *) echo \"General\nComplete name : $(basename \"$last\")\" ;;\nesac\n");
        let video = d.join("Mon Film 2020 1080p.mkv");
        fs::write(&video, b"x").unwrap();
        fs::write(d.join("Mon.Film.2020.1080p.nfo"), "ancien").unwrap();
        let cfg = Config { mediainfo: mi.display().to_string(), ..Default::default() };
        let mut lines = Vec::new();
        let (made, notes) = reports_only(&cfg, &video, ReportsOnly { nfo: true, mediainfo: true, bdinfo: true }, None, &mut |l| lines.push(l.to_string())).unwrap();
        let names: Vec<String> = made.iter().map(|p| crate::prep::file_name(p)).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.iter().any(|n| n == "Mon.Film.2020.1080p.nzbff.nfo"), "{names:?}");
        assert_eq!(fs::read_to_string(d.join("Mon.Film.2020.1080p.nfo")).unwrap(), "ancien", "NFO existant gardé");
        assert!(notes.iter().any(|n| n.contains("existe déjà")) && notes.iter().any(|n| n.contains("Blu-ray")), "{notes:?}");
        assert!(video.exists(), "la vidéo n'a pas bougé");
        assert!(fs::read_to_string(&made[1]).unwrap().contains("Mon Film 2020 1080p.mkv"));
        assert!(!lines.is_empty());
        assert!(reports_only(&cfg, &d.join("absent.mkv"), ReportsOnly { nfo: true, mediainfo: false, bdinfo: false }, None, &mut |_| {}).is_err());
        let _ = fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod tests_doublons {
    use super::*;

    #[test]
    fn rapport_range_avec_la_video_copie_pour_les_indexers() {
        let d = std::env::temp_dir().join(format!("nzbff_rapport_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let cfg = Config { indexer_dir: d.join("idx").display().to_string(), ..Default::default() };
        let rep = d.join("sortie").join("R.2024.FRENCH.1080p.x264.xml");
        std::fs::create_dir_all(rep.parent().unwrap()).unwrap();
        std::fs::write(&rep, "<x/>").unwrap();
        report_to_waiting(&cfg, "R.2024.FRENCH.1080p.x264", &rep);
        let w = crate::ngpost::waiting_dir(&cfg).unwrap().join("R.2024.FRENCH.1080p.x264");
        assert!(w.join("R.2024.FRENCH.1080p.x264.xml").is_file());
        assert!(rep.is_file(), "l'original reste avec la vidéo");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn doublon_seulement_si_poste_ou_en_attente() {
        let d = std::env::temp_dir().join(format!("nzbff_dup_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut h = History::load(&d.join("h.tsv"));
        let rec = |rel: &str, dest: &Path| Record { ts: 1, status: Status::Sent, mode: "Manuel".into(), original: "x".into(), release: rel.into(), size: 1, resolution: String::new(), codec: String::new(), langs: String::new(), duration: String::new(), source_path: String::new(), dest: dest.display().to_string(), xml: String::new(), reason: String::new(), post: String::new() };
        // préparée lors d'un test puis supprimée, jamais postée : pas un doublon
        h.push(rec("Serie.S01.FRENCH.1080p-GRP", &d.join("disparu")));
        assert_eq!(duplicate_reason("serie.s01.french.1080p-grp", &h, &[]), None);
        // préparée et toujours en attente dans la sortie : doublon
        std::fs::create_dir_all(d.join("attente")).unwrap();
        h.push(rec("Serie.S02.FRENCH.1080p-GRP", &d.join("attente")));
        assert!(duplicate_reason("Serie.S02.FRENCH.1080p-GRP", &h, &[]).unwrap().contains("pas encore posté"));
        // déjà postée : doublon, même si plus rien n'est sur le disque
        let posted = crate::stats::PostRec { start: 1, end: 2, release: "Serie.S03.FRENCH.1080p-GRP".into(), bytes: 1, articles: 1, avg_bps: 1.0, connections: 1, indexers: String::new() };
        assert!(duplicate_reason("Serie.S03.FRENCH.1080p-GRP", &h, &[posted.clone()]).unwrap().contains("déjà posté"));
        // une saison seule n'est pas un doublon du pack complet déjà posté
        let pack = crate::stats::PostRec { release: "Serie.S01-S05.COMPLETE.FRENCH.1080p-GRP".into(), ..posted };
        assert_eq!(duplicate_reason("Serie.S01.FRENCH.1080p-GRP", &h, &[pack]), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
