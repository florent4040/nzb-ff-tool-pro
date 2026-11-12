#![cfg_attr(all(windows, feature = "gui", not(debug_assertions)), windows_subsystem = "windows")]
#![cfg_attr(not(feature = "gui"), allow(dead_code))]

mod apidef;
mod apidoc;
mod backup;
mod content;
mod disc;
mod dl;
mod finder;
mod history;
mod jobs;
mod logs;
mod notify;
mod ngpost;
mod prep;
mod search;
mod series;
mod shell;
mod stats;
mod tray;
mod upload;
mod usenet;

#[cfg(feature = "gui")]
mod gui;

use history::{History, Status};
use prep::{Config, Stage, Transfer};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::{env, fs, thread, time::Duration};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_NAME: &str = "NZB FF Tool Pro";
const CONF_FILE: &str = "nzbff.conf";
const HIST_FILE: &str = "nzbff-history.tsv";

const USAGE: &str = "\
NZB FF Tool Pro - prépare tes vidéos pour Usenet
(nom au format des indexers + rapport MediaInfo + envoi vers ton logiciel de post)

  \"NZB FF Tool Pro.exe\"                  ouvre l'interface graphique
  ... --tray                             ouvre l'interface réduite dans la zone de notification
  ... --envoyer <mode> <chemin>…         transmet à l'application (ouverte ou non) :
            auto | ouvrir                  fichier ou dossier traité tout seul / à régler
            nzb | reposter | telecharger | deposer
                                         .nzb : réglage du double clic / téléchargé puis
                                         reposté / téléchargé seulement / déposé
                                         (utilisé par le menu du clic droit de Windows)
  ... --watch                            surveille l'inbox sans interface
  ... --once                             un seul passage sur l'inbox puis quitte
  ... --file <chemin> [--name <nom>] [--copy|--move] [--dest <dossier>] [--dry-run]
                                         (un dossier de série : livré d'un bloc, voir Réglages → Séries)
                                         traite un seul fichier (par défaut : copie
                                         depuis un lecteur réseau, déplacement en local)
  options : --action rename|post|auto    renommer seulement / + poster / + pages d'upload
            --indexers \"A,B\"           indexers visés (défaut : ceux des réglages)
            --no-api                    ne pas déposer par API sur les indexers

Réglages (nzbff.conf) et historique (nzbff-history.tsv) sont rangés
à côté de l'exécutable : le dossier est portable.
";

/// Dossier de l'exécutable : réglages + historique y vivent (mode portable).
pub fn app_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Reprend les fichiers de l'ancienne version (NgPrep) au premier lancement.
fn migrate(dir: &Path) {
    for (old, new) in [("ngprep.conf", CONF_FILE), ("ngprep-history.tsv", HIST_FILE)] {
        let (o, n) = (dir.join(old), dir.join(new));
        if o.exists() && !n.exists() {
            let _ = fs::copy(&o, &n);
        }
    }
}

#[cfg(windows)]
fn attach_console() {
    // En mode fenêtre, l'exe n'a pas de console : on se rattache à celle du terminal
    // pour que les commandes --watch / --file affichent leurs messages.
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(pid: u32) -> i32;
    }
    unsafe {
        AttachConsole(0xFFFF_FFFF);
    }
}

#[cfg(not(windows))]
fn attach_console() {}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).cloned()
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let dir = app_dir();
    migrate(&dir);
    // nouvelle version installée dans un autre dossier : réglages, historique et statistiques
    // repris de la sauvegarde automatique (profil Windows)
    let restored = backup::auto_dir().map(|a| backup::auto_restore_missing(&dir, &a)).unwrap_or_default();
    if !restored.is_empty() {
        eprintln!("Repris de la sauvegarde automatique : {}", restored.join(", "));
        let _ = fs::write(dir.join(".nzbff-repris"), restored.join(", "));
    }
    let cfg_path = dir.join(CONF_FILE);
    let hist_path = dir.join(HIST_FILE);
    let mut cfg = Config::load(&cfg_path);

    // demande de l'Explorateur (clic droit, double clic sur un .nzb) : transmise à l'instance
    // ouverte, sinon l'application démarre (en fond si rien n'est à choisir)
    #[cfg(feature = "gui")]
    if let Some(reqs) = shell::parse_args(&args) {
        if let Err(e) = shell::enqueue(&shell::queue_dir(), &reqs) {
            eprintln!("Demande non transmise : {e}");
            return;
        }
        let show = reqs.iter().any(|r| shell::needs_window(&cfg, r));
        if tray::another_instance_show(show) {
            return;
        }
        gui::run(cfg, cfg_path, hist_path, !show && tray::available());
        backup::on_exit();
        return;
    }

    // sans argument : la fenêtre ; « --tray » (lancement avec Windows) : réduit dans la zone de notification
    #[cfg(feature = "gui")]
    if args.is_empty() || (args.len() == 1 && args[0] == "--tray") {
        // une seule instance : relancée, elle montre celle qui tourne déjà et s'arrête
        if tray::another_instance() {
            return;
        }
        let hidden = !args.is_empty();
        gui::run(cfg, cfg_path, hist_path, hidden);
        // fenêtre fermée : sauvegarde à la fermeture si demandée
        backup::on_exit();
        return;
    }

    attach_console();
    if let Some(a) = arg_value(&args, "--action") {
        if prep::ACTIONS.iter().any(|x| x.0 == a) {
            cfg.action = a;
        } else {
            eprintln!("--action : rename, post ou auto");
            std::process::exit(2);
        }
    }
    if args.iter().any(|a| a == "--no-api") {
        cfg.api_upload = false;
    }
    if let Some(t) = arg_value(&args, "--indexers") {
        cfg.targets = t.split([',', '|']).map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect();
    }
    if !cfg.ng_imported {
        if let Some(c) = ngpost::read_conf(&cfg) {
            ngpost::import_post_opts(&mut cfg, &c);
        }
    }
    if args.iter().any(|a| a == "-h" || a == "--help") || args.is_empty() {
        println!("{USAGE}");
        return;
    }
    let hist = Mutex::new(History::load(&hist_path));

    if let Some(file) = arg_value(&args, "--file") {
        let how = if args.iter().any(|a| a == "--copy") {
            Transfer::Copy
        } else if args.iter().any(|a| a == "--move") {
            Transfer::Move
        } else {
            prep::transfer_for(Path::new(&file), &cfg)
        };
        let dest = arg_value(&args, "--dest").map(PathBuf::from);
        let cfg = cfg;
        if Path::new(&file).is_dir() {
            series_cli(&cfg, &hist, Path::new(&file), how, dest, args.iter().any(|a| a == "--dry-run"));
            return;
        }
        single(&cfg, &hist, &file, arg_value(&args, "--name"), how, dest, args.iter().any(|a| a == "--dry-run"));
        return;
    }
    let once = args.iter().any(|a| a == "--once");
    if once || args.iter().any(|a| a == "--watch") {
        headless(&cfg, &hist, once);
        return;
    }
    println!("{USAGE}");
}

fn print_progress(st: Stage, done: u64, total: u64) {
    if st == Stage::Copy && total > 0 {
        print!("\r    copie : {:>3} %  ({} / {})   ", done * 100 / total, prep::human_size(done), prep::human_size(total));
        let _ = std::io::stdout().flush();
        if done == total {
            println!();
        }
    }
}

fn single(cfg: &Config, hist: &Mutex<History>, file: &str, name: Option<String>, how: Transfer, dest: Option<PathBuf>, dry: bool) {
    let path = PathBuf::from(file);
    if !path.is_file() {
        eprintln!("Fichier introuvable : {file}");
        std::process::exit(1);
    }
    if dry {
        let a = prep::analyze(&path, cfg, name.as_deref(), None);
        match a.name {
            Ok(n) => println!("{n}"),
            Err(e) => {
                eprintln!("Impossible : {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let to_output = dest.is_none();
    let r = jobs::run_one(&path, cfg, name.as_deref(), how, dest.as_deref(), jobs::Mode::Manual, false, hist, &mut print_progress);
    match r.status {
        Status::Sent => {
            println!("OK  {}\n    -> {}\n    MediaInfo : {}", r.release, r.dest, r.xml);
            let _ = to_output;
            if cfg.action != "rename" && cfg.post_mode == "cli" && !cfg.ngpost_exe.trim().is_empty() {
                post_now(cfg, &r.dest, &r.release);
                for (rel, plan) in ngpost::distribute(cfg) {
                    for n in ngpost::take_notes() {
                        eprintln!("! {n}");
                    }
                    println!("    prêt pour : {} ({rel})", plan.resolve(cfg).join(", "));
                    for i in ngpost::api_targets(cfg, &plan) {
                        deposit(cfg, &rel, &i);
                    }
                }
            }
        }
        _ => {
            eprintln!("Échec : {}", r.reason);
            std::process::exit(1);
        }
    }
}

/// `--file <dossier de série>` : une release (ou une par saison), postée et déposée comme un film.
fn series_cli(cfg: &Config, hist: &Mutex<History>, dir: &Path, how: Transfer, dest: Option<PathBuf>, dry: bool) {
    let Some(scan) = series::scan(dir) else {
        eprintln!("Aucun épisode reconnu dans {}", dir.display());
        std::process::exit(1);
    };
    let (fields, _) = series::detect_fields(cfg, &scan);
    for s in &scan.seasons {
        let state = if s.complete() { "complète".to_string() } else { format!("manque {:?}", s.missing) };
        println!("{} : {} épisode(s), {state}", s.label(), s.count());
    }
    let mut failed = false;
    let _ = fields;
    if dry {
        if let Some((_, names)) = jobs::series_plan(dir, cfg) {
            for n in names {
                println!("{n}");
            }
        }
        return;
    }
    for r in jobs::run_series_with(dir, cfg, how, dest.as_deref(), jobs::Mode::Manual, false, hist, &mut print_progress) {
        if r.status != Status::Sent {
            eprintln!("Échec {} : {}", r.release, r.reason);
            failed = true;
            continue;
        }
        println!("OK  {}\n    -> {}\n    MediaInfo : {}", r.release, r.dest, r.xml);
        if dest.is_none() && cfg.action != "rename" && cfg.post_mode == "cli" && !cfg.ngpost_exe.trim().is_empty() {
            post_now(cfg, &r.dest, &r.release);
            for (rel, plan) in ngpost::distribute(cfg) {
                println!("    prêt pour : {} ({rel})", plan.resolve(cfg).join(", "));
                for i in ngpost::api_targets(cfg, &plan) {
                    deposit(cfg, &rel, &i);
                }
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}

/// Dépose une release sur un indexer par API, avec nouveaux essais si l'erreur est passagère.
fn deposit(cfg: &Config, rel: &str, idx: &str) {
    for attempt in 0..=5u32 {
        let r = ngpost::api_upload(cfg, rel, idx);
        for l in ngpost::take_api_detail() {
            println!("    {l}");
        }
        match r {
            Ok(m) => {
                println!("    déposée sur {idx} : {m}");
                return;
            }
            Err(e) => match e.retry_after {
                Some(secs) if attempt < 5 => {
                    eprintln!("    {idx} : {}, nouvel essai dans {secs} s", e.message);
                    thread::sleep(Duration::from_secs(secs));
                }
                _ => {
                    eprintln!("    dépôt sur {idx} impossible : {}", e.message);
                    return;
                }
            },
        }
    }
}

/// Poste une vidéo avec NgPost en ligne de commande (mode sans interface).
fn post_now(cfg: &Config, video: &str, release: &str) {
    println!("    post Usenet avec NgPost…");
    let on_bar = std::cell::Cell::new(false);
    let res = ngpost::post_file(
        cfg,
        Path::new(video),
        release,
        &mut |l| {
            if on_bar.replace(false) {
                println!();
            }
            println!("    {l}");
        },
        &mut |p| {
            on_bar.set(true);
            let arts = if p.total > 0 { format!("  {} / {} articles", p.done, p.total) } else { String::new() };
            let speed = p.avg_bps.map(|b| format!("  moyenne {}/s", prep::human_size(b as u64))).unwrap_or_default();
            print!("\r    {} : {:>3} %{arts}{speed}      ", p.phase, p.pct);
            let _ = std::io::stdout().flush();
        },
    );
    match res {
        Ok(p) => {
            println!("\n    NZB : {} ({})", p.nzb.display(), p.info.summary());
            for w in &p.warnings {
                println!("    attention : {w}");
            }
            let mut st = stats::PostStats::load(&app_dir());
            st.push_speed(&p.samples);
            st.push_post(stats::PostRec {
                start: p.started,
                end: prep::now_secs(),
                release: release.to_string(),
                bytes: p.bytes,
                articles: p.articles,
                avg_bps: p.avg_bps.unwrap_or(p.bytes as f64 / p.secs.max(1) as f64),
                connections: p.connections.unwrap_or(0),
                indexers: ngpost::read_plan(cfg, release).unwrap_or_else(|| ngpost::Plan::from_cfg(cfg)).resolve(cfg).join(", "),
            });
            if let Some(l) = p.log {
                println!("    journal : {}", l.display());
            }
        }
        Err(e) => eprintln!("\n    Post impossible : {e}"),
    }
}

fn headless(cfg: &Config, hist: &Mutex<History>, once: bool) {
    if let Err(e) = cfg.ready_watch() {
        eprintln!("Configuration incomplète : {e}\nVoir --help.");
        std::process::exit(1);
    }
    if let Err(e) = cfg.make_dirs() {
        eprintln!("Dossiers : {e}");
        std::process::exit(1);
    }
    let stop = AtomicBool::new(false);
    // e-mail de fin des traitements (si réglé)
    notify::spawn(
        std::sync::Arc::new(Mutex::new(cfg.clone())),
        Box::new(|r| match r {
            Ok(to) => println!("E-mail de fin des traitements envoyé à {to}"),
            Err(e) => eprintln!("E-mail de fin des traitements : {e}"),
        }),
    );
    println!("Surveillance de {} (Ctrl+C pour arrêter)", cfg.inbox);
    println!("Action : {} · indexers : {}", prep::action_label(&cfg.action), {
        let t = ngpost::Plan::from_cfg(cfg).resolve(cfg);
        if t.is_empty() { "aucun".to_string() } else { t.join(", ") }
    });
    if cfg.post_mode == "watch" && cfg.action != "rename" && cfg.ngpost_autostart {
        match ngpost::ensure_running(cfg) {
            Ok(true) => println!("NgPost lancé"),
            Ok(false) => {}
            Err(e) => eprintln!("NgPost : {e}"),
        }
    }
    loop {
        let known = |r: &str| hist.lock().map(|h| h.find_sent(r).is_some()).unwrap_or(false);
        let _ = ngpost::ensure_dirs(cfg);
        ngpost::migrate_layout(cfg);
        for (rel, _) in ngpost::collect_nzbs(cfg, &known) {
            println!("NZB rapatrié : {rel}");
        }
        for (rel, plan) in ngpost::distribute(cfg) {
            println!("Prêt pour {} : {rel}", plan.resolve(cfg).join(", "));
            let api = ngpost::api_targets(cfg, &plan);
            for i in &api {
                deposit(cfg, &rel, i);
            }
            if plan.auto() {
                for i in plan.resolve(cfg).iter().filter(|i| !api.contains(i)) {
                    if let Some(u) = ngpost::upload_url(cfg, i) {
                        let _ = prep::open_url(&u);
                    }
                }
            }
        }
        jobs::scan_once(cfg, hist, &stop, &mut |_, st, d, t| print_progress(st, d, t), &mut |r| match r.status {
            Status::Sent => {
                println!("OK           {}  ->  {}", r.original, r.release);
                if cfg.action != "rename" && cfg.post_mode == "cli" && !cfg.ngpost_exe.trim().is_empty() {
                    post_now(cfg, &r.dest, &r.release);
                }
            }
            Status::Quarantine => println!("QUARANTAINE  {}  ({})", r.original, r.reason),
            Status::Failed => println!("ÉCHEC        {}  ({})", r.original, r.reason),
        });
        if once {
            break;
        }
        thread::sleep(Duration::from_secs(cfg.interval));
    }
}
