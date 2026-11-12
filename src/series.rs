//! Séries : détection d'une série (Série > Saisons > épisodes, ou saison seule, ou épisodes
//! à plat), dans tous les formats de nommage courants ; complétude des saisons ; noms de
//! release (pack d'une ou plusieurs saisons, mention COMPLETE) ; arborescence livrée
//! (`<release>/<Série>/Saison N/épisodes`) et traitement de bout en bout.

use crate::prep::{self, Config, Done, Fields, Stage, Transfer};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

/// Modèles par défaut (modifiables dans Réglages → Séries).
pub const SERIES_TEMPLATE: &str = "{titre}.{annee}.{saison}.{complete}.{langue}.{resolution}.{source}.{codec}-{tag}";
pub const EPISODE_TEMPLATE: &str = "{titre}.{episode}.{langue}.{resolution}.{source}.{codec}-{tag}";

/// Fichiers qui accompagnent les épisodes (sous-titres, NFO) et partent avec eux.
const EXTRA_EXT: &[&str] = &["srt", "ass", "ssa", "sub", "idx", "sup", "vtt", "nfo"];

#[derive(Clone, Debug, PartialEq)]
pub struct Episode {
    pub path: PathBuf,
    pub season: u32,
    /// Un épisode, ou plusieurs pour un fichier double (S01E01E02).
    pub episodes: Vec<u32>,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Season {
    pub number: u32,
    /// Nom du dossier de saison d'origine, s'il y en avait un.
    pub dir: Option<String>,
    pub episodes: Vec<Episode>,
    /// Épisodes absents entre E01 et le dernier présent.
    pub missing: Vec<u32>,
    /// Épisodes présents en plusieurs exemplaires.
    pub duplicates: Vec<u32>,
    /// Sous-titres et NFO rangés avec la saison.
    pub extras: Vec<PathBuf>,
}

impl Season {
    pub fn last(&self) -> u32 {
        self.episodes.iter().flat_map(|e| e.episodes.iter().copied()).max().unwrap_or(0)
    }
    pub fn count(&self) -> usize {
        self.episodes.iter().map(|e| e.episodes.len()).sum()
    }
    /// Complète : de E01 au dernier épisode, sans trou (les spéciaux, saison 0, comptent à part).
    pub fn complete(&self) -> bool {
        !self.episodes.is_empty() && self.missing.is_empty()
    }
    pub fn size(&self) -> u64 {
        self.episodes.iter().map(|e| e.size).sum::<u64>() + self.extras.iter().filter_map(|p| fs::metadata(p).ok()).map(|m| m.len()).sum::<u64>()
    }
    pub fn label(&self) -> String {
        if self.number == 0 {
            "Spéciaux".into()
        } else {
            format!("Saison {}", self.number)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scan {
    pub root: PathBuf,
    /// Titre (format release : points) et année, si trouvée.
    pub title: String,
    pub year: String,
    /// Tag de groupe repris des noms d'épisodes.
    pub tag: String,
    pub seasons: Vec<Season>,
    /// Vidéos sans numéro d'épisode reconnaissable (ignorées).
    pub unknown: Vec<PathBuf>,
    /// Le dossier choisi est lui-même une saison (« Saison 1 », « Serie.S01.1080p… »).
    pub root_is_season: bool,
    /// Le dossier se dit complet : « Intégrale », « Integrale », « Complete », « L'intégrale »…
    pub named_whole: bool,
    /// NFO posés à la racine du dossier (celui de la série) : à la racine de chaque release.
    pub root_extras: Vec<PathBuf>,
}

impl Scan {
    pub fn episode_count(&self) -> usize {
        self.seasons.iter().map(|s| s.count()).sum()
    }
    pub fn size(&self) -> u64 {
        self.seasons.iter().map(|s| s.size()).sum()
    }
    pub fn numbers(&self) -> Vec<u32> {
        self.seasons.iter().map(|s| s.number).collect()
    }
    pub fn season(&self, n: u32) -> Option<&Season> {
        self.seasons.iter().find(|s| s.number == n)
    }
    /// Complète : chaque saison complète, et les saisons se suivent depuis la saison 1.
    pub fn complete_for(&self, unit: &[u32]) -> bool {
        let real: Vec<u32> = unit.iter().copied().filter(|n| *n > 0).collect();
        if real.is_empty() || !real.iter().all(|n| self.season(*n).map(|s| s.complete()).unwrap_or(false)) {
            return false;
        }
        // plusieurs saisons : elles doivent se suivre (S01-S03 sans S02 n'est pas complet)
        real.windows(2).all(|w| w[1] == w[0] + 1) && (real.len() == 1 || real[0] == 1)
    }
    /// Mention COMPLETE automatique : seulement pour une série entière livrée d'un bloc,
    /// dossier de la série (pas une saison seule), au moins deux saisons depuis S01, toutes
    /// sans épisode manquant, et toutes dans cette release. Une saison seule n'est jamais
    /// « complète » d'office : rien ne dit qu'il ne manque pas d'autres saisons ou épisodes.
    pub fn complete_series(&self, unit: &[u32]) -> bool {
        let all: Vec<u32> = self.numbers().into_iter().filter(|n| *n > 0).collect();
        let sel: Vec<u32> = unit.iter().copied().filter(|n| *n > 0).collect();
        // dossier « Intégrale » : même d'une seule saison (ex. 500 épisodes en S01)
        let enough = (!self.root_is_season && all.len() >= 2) || self.named_whole;
        enough && sel.len() == all.len() && self.whole_series()
    }
    /// Toute la série est-elle là (S01 → dernière saison, sans trou) ?
    pub fn whole_series(&self) -> bool {
        let real: Vec<u32> = self.numbers().into_iter().filter(|n| *n > 0).collect();
        !real.is_empty() && real[0] == 1 && self.complete_for(&real)
    }
    /// Premier épisode (pour le rapport MediaInfo et l'analyse) d'un ensemble de saisons.
    pub fn first_episode(&self, unit: &[u32]) -> Option<&Episode> {
        let mut nums: Vec<u32> = unit.to_vec();
        nums.sort_by_key(|n| if *n == 0 { u32::MAX } else { *n });
        nums.iter().filter_map(|n| self.season(*n)).flat_map(|s| s.episodes.first()).next()
    }
}

// ------------------------------------------------------------------ détection

fn lower_chars(s: &str) -> Vec<char> {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}

/// Nombre de 1 à `max` chiffres à la position `i` : (valeur, longueur).
fn digits_at(c: &[char], i: usize, max: usize) -> Option<(u32, usize)> {
    let mut len = 0;
    while i + len < c.len() && len < max && c[i + len].is_ascii_digit() {
        len += 1;
    }
    if len == 0 {
        return None;
    }
    let v: String = c[i..i + len].iter().collect();
    v.parse().ok().map(|v| (v, len))
}

fn boundary(c: &[char], k: usize) -> bool {
    k >= c.len() || !c[k].is_alphanumeric()
}

/// Numéros dans un nom d'épisode : (saison si présente, épisodes). Formats reconnus :
/// S01E02, s1e2, S01.E02, S01E01E02, S01E01-E03, S01E01-02, 1x02, « Saison 1 Épisode 2 »,
/// « Season 1 Episode 2 », Ep02 / E02 / « Épisode 2 » (saison donnée par le dossier),
/// « Titre - 12 » (numérotation continue).
pub fn parse_episode(stem: &str) -> Option<(Option<u32>, Vec<u32>)> {
    let c = lower_chars(stem);
    let n = c.len();
    // 1. S01E02 et variantes
    for i in 0..n {
        if c[i] != 's' || !(i == 0 || !c[i - 1].is_alphanumeric()) {
            continue;
        }
        let Some((s, ls)) = digits_at(&c, i + 1, 2) else { continue };
        let j = i + 1 + ls;
        if j < n && c[j].is_ascii_digit() {
            continue;
        }
        let mut k = j;
        if k < n && matches!(c[k], '.' | '_' | '-' | ' ') {
            k += 1;
        }
        if k >= n || c[k] != 'e' {
            continue;
        }
        let Some((e, le)) = digits_at(&c, k + 1, 4) else { continue };
        let mut p = k + 1 + le;
        if p < n && c[p].is_ascii_digit() {
            continue;
        }
        let mut eps = vec![e];
        loop {
            let last = *eps.last().unwrap_or(&e);
            // E02 collé, -E02 / _E02, ou -02
            let (q, ranged) = if p < n && c[p] == 'e' {
                (p + 1, false)
            } else if p + 1 < n && matches!(c[p], '-' | '_') && c[p + 1] == 'e' {
                (p + 2, true)
            } else if p < n && c[p] == '-' {
                (p + 1, true)
            } else {
                break;
            };
            let Some((v, lv)) = digits_at(&c, q, 4) else { break };
            let after = q + lv;
            let bare = !(c[q - 1] == 'e');
            if (after < n && c[after].is_ascii_digit()) || (bare && !boundary(&c, after)) || v <= last || v - last > 40 {
                break;
            }
            if ranged {
                eps.extend(last + 1..=v);
            } else {
                eps.push(v);
            }
            p = after;
        }
        return Some((Some(s), eps));
    }
    // 2. 1x02
    for i in 1..n {
        if c[i] != 'x' || !c[i - 1].is_ascii_digit() {
            continue;
        }
        let mut st = i;
        while st > 0 && c[st - 1].is_ascii_digit() {
            st -= 1;
        }
        if i - st > 2 || !(st == 0 || !c[st - 1].is_alphanumeric()) {
            continue;
        }
        let Some((e, le)) = digits_at(&c, i + 1, 4) else { continue };
        if le < 2 || !boundary(&c, i + 1 + le) {
            continue;
        }
        let s: String = c[st..i].iter().collect();
        if let Ok(s) = s.parse() {
            return Some((Some(s), vec![e]));
        }
    }
    // 3. mots : saison / season … épisode / episode / ep / e
    let words: Vec<String> = c.iter().collect::<String>().split(|ch: char| !ch.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect();
    let num = |w: &str| -> Option<u32> { (w.len() <= 4 && w.chars().all(|x| x.is_ascii_digit())).then(|| w.parse().ok()).flatten().filter(|v: &u32| !(1900..=2100).contains(v)) };
    let word_num = |i: usize, keys: &[&str]| -> Option<u32> {
        let w = &words[i];
        for k in keys {
            if w == k {
                return words.get(i + 1).and_then(|x| num(x));
            }
            if let Some(rest) = w.strip_prefix(k) {
                if let Some(v) = num(rest) {
                    return Some(v);
                }
            }
        }
        None
    };
    let mut season = None;
    let mut episode = None;
    for i in 0..words.len() {
        if season.is_none() {
            season = word_num(i, &["saison", "season", "temporada", "staffel", "stagione", "seizoen"]);
        }
        if episode.is_none() {
            episode = word_num(i, &["episode", "épisode", "episodio", "folge", "ep", "e"]);
        }
    }
    if let Some(e) = episode {
        return Some((season, vec![e]));
    }
    // 4. « Titre - 12 » (animés, numérotation continue)
    let s: String = c.iter().collect();
    if let Some(k) = s.rfind(" - ") {
        let rest: Vec<char> = s[k + 3..].chars().collect();
        if let Some((e, le)) = digits_at(&rest, 0, 4) {
            if le >= 2 && boundary(&rest, le) && !(1900..=2100).contains(&e) {
                return Some((None, vec![e]));
            }
        }
    }
    None
}

/// Numéro seul dans un nom de fichier (1 à 3 chiffres, ni année ni résolution).
pub fn bare_number(stem: &str) -> Option<u32> {
    stem.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && w.len() <= 4 && w.chars().all(|c| c.is_ascii_digit()))
        .filter_map(|w| w.parse::<u32>().ok())
        .find(|n| *n > 0 && !(1900..=2100).contains(n))
}

/// Numéro de saison dans un nom de dossier : « Saison 1 », « Season 01 », « S01 »,
/// « Serie.S02.1080p… », « Staffel 3 », « Spéciaux » (0).
pub fn season_from_dir(name: &str) -> Option<u32> {
    let l = name.to_lowercase();
    let words: Vec<&str> = l.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if words.len() <= 2 && words.iter().any(|w| matches!(*w, "specials" | "special" | "speciaux" | "spéciaux" | "bonus")) {
        return Some(0);
    }
    let num = |w: &str| -> Option<u32> { (w.len() <= 2 && !w.is_empty() && w.chars().all(|x| x.is_ascii_digit())).then(|| w.parse().ok()).flatten() };
    for (i, w) in words.iter().enumerate() {
        for k in ["saison", "season", "series", "serie", "temporada", "staffel", "stagione", "seizoen", "sezon"] {
            if *w == k {
                if let Some(v) = words.get(i + 1).and_then(|x| num(x)) {
                    return Some(v);
                }
            } else if let Some(v) = w.strip_prefix(k).and_then(num) {
                return Some(v);
            }
        }
        if let Some(v) = w.strip_prefix('s').and_then(num) {
            return Some(v);
        }
    }
    None
}

/// Le nom d'une release ressemble-t-il à une série (S01, S01E02, S01-S03, 1x02) ?
/// Code de saison collé à la fin d'un mot : « titres01 », « titres01e02 ».
fn glued_season(t: &str) -> bool {
    let b = t.as_bytes();
    // fin en sNN ou sNNeNN, précédée d'une lettre
    let tail = |i: usize| -> bool { i >= 1 && b[i - 1].is_ascii_alphabetic() && b[i] == b's' };
    let n = b.len();
    if n >= 4 && b[n - 2].is_ascii_digit() && b[n - 1].is_ascii_digit() && tail(n - 3) {
        return true;
    }
    if n >= 7 && b[n - 2].is_ascii_digit() && b[n - 1].is_ascii_digit() && b[n - 3] == b'e' && b[n - 4].is_ascii_digit() && b[n - 5].is_ascii_digit() && tail(n - 6) {
        return true;
    }
    false
}

pub fn is_series_name(name: &str) -> bool {
    let tokens: Vec<String> = name.split(|c: char| !c.is_alphanumeric() && c != '-').filter(|t| !t.is_empty()).map(|t| t.to_lowercase()).collect();
    let s_num = |x: &str| x.strip_prefix('s').map(|r| !r.is_empty() && r.len() <= 2 && r.chars().all(|c| c.is_ascii_digit())).unwrap_or(false);
    let number = |x: &str| !x.is_empty() && x.len() <= 2 && x.chars().all(|c| c.is_ascii_digit()) || x.split('-').all(|p| !p.is_empty() && p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit()));
    // « Saison.1 », « Season 2 », « Saisons.1-5 », « Saison1 », « Temporada.3 », « Staffel.1 »
    const WORDS: &[&str] = &["saison", "saisons", "season", "seasons", "temporada", "staffel", "stagione", "seizoen"];
    tokens.iter().enumerate().any(|(i, t)| {
        s_num(t)
            || t.split('-').all(s_num) && t.contains('-')
            || parse_episode(t).map(|(s, _)| s.is_some()).unwrap_or(false) && t.starts_with('s')
            || WORDS.iter().any(|w| t.ends_with(w)) && tokens.get(i + 1).map(|n| number(n)).unwrap_or(false)
            || WORDS.iter().any(|w| t.strip_prefix(w).map(number).unwrap_or(false))
            // « TitreS01 », « TitreS01E02 » : code collé au titre (lettre puis S + 2 chiffres)
            || glued_season(t)
            // « 1x02 » (saison x épisode)
            || t.split_once('x').map(|(a, b)| number(a) && b.len() == 2 && b.chars().all(|c| c.is_ascii_digit())).unwrap_or(false)
    })
}

const STOP_WORDS: &[&str] = &[
    "saison", "season", "temporada", "staffel", "integrale", "intégrale", "complete", "complète", "collection", "french", "truefrench", "multi", "vostfr", "vff", "vfq", "vfi", "vf", "subfrench", "480p", "576p", "720p", "1080p", "1080i", "2160p", "4k", "uhd", "x264", "x265", "h264", "h265", "hevc", "avc", "av1", "web", "webrip", "webdl", "bluray", "bdrip", "brrip", "hdtv", "dvdrip", "remux", "hdlight",
];

/// Le nom dit-il « série entière » ? Intégrale, Integrale, INTEGRALE, Complete, Complète,
/// « L'intégrale », Complete Series…
pub fn says_whole(name: &str) -> bool {
    let l = name.to_lowercase().replace(['’', '\''], " ");
    l.split(|c: char| !c.is_alphanumeric()).any(|w| matches!(w, "integrale" | "intégrale" | "integral" | "intégral" | "complete" | "complète" | "completes" | "complètes"))
}

/// Titre (format release) et année tirés d'un nom de dossier ou du début d'un nom d'épisode.
pub fn clean_title(name: &str) -> (String, String) {
    let n = name.replace(['\'', '’', '`'], "");
    let words: Vec<&str> = n.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let mut out: Vec<&str> = Vec::new();
    let mut year = String::new();
    for (i, w) in words.iter().enumerate() {
        let l = w.to_lowercase();
        let is_year = w.len() == 4 && w.chars().all(|c| c.is_ascii_digit()) && (w.starts_with("19") || w.starts_with("20"));
        if is_year && i > 0 {
            year = w.to_string();
            break;
        }
        let s_tok = l.strip_prefix('s').map(|r| !r.is_empty() && r.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)).unwrap_or(false);
        let xx = l.contains('x') && l.split('x').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        if STOP_WORDS.contains(&l.as_str()) || s_tok || xx || (matches!(l.as_str(), "serie" | "series" | "episode" | "ep") && words.get(i + 1).map(|x| x.chars().all(|c| c.is_ascii_digit())).unwrap_or(false)) {
            break;
        }
        out.push(w);
    }
    (out.join("."), year)
}

fn is_extra(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| EXTRA_EXT.iter().any(|x| x.eq_ignore_ascii_case(e))).unwrap_or(false)
}

fn is_sample(p: &Path) -> bool {
    let l = prep::file_name(p).to_lowercase();
    l == "sample" || l.contains(".sample.") || l.contains("-sample.") || l.starts_with("sample")
}

/// Fichiers sous `dir` (3 niveaux), hors dossiers cachés, « _… » et échantillons.
fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = prep::file_name(&p);
        if name.starts_with('.') || name.starts_with('_') || is_sample(&p) {
            continue;
        }
        if p.is_dir() {
            if depth > 0 {
                walk(&p, depth - 1, out);
            }
        } else {
            out.push(p);
        }
    }
}

/// Tous les fichiers d'une série (pour la stabilité de la copie dans l'inbox).
pub fn all_files(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    walk(dir, 3, &mut v);
    v
}

/// Analyse un dossier. None : aucune vidéo reconnue comme épisode.
pub fn scan(root: &Path) -> Option<Scan> {
    let files = all_files(root);
    let videos: Vec<&PathBuf> = files.iter().filter(|p| prep::is_video(p)).collect();
    if videos.is_empty() {
        return None;
    }
    let root_name = prep::file_name(root);
    let has_season_dirs = fs::read_dir(root).map(|r| r.flatten().any(|e| e.path().is_dir() && season_from_dir(&prep::file_name(&e.path())).is_some())).unwrap_or(false);
    let root_season = if has_season_dirs { None } else { season_from_dir(&root_name) };
    let root_is_season = root_season.is_some();

    // saison de chaque vidéo : nom du fichier, sinon son dossier, sinon le dossier choisi
    let mut known: Vec<(PathBuf, Option<u32>, Vec<u32>, Option<String>)> = Vec::new();
    let mut unknown = Vec::new();
    for v in &videos {
        let stem = v.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let parent = v.parent().filter(|p| *p != root).map(prep::file_name);
        let dir_season = parent.as_deref().and_then(season_from_dir).or(root_season).or_else(|| says_whole(&root_name).then_some(1));
        // dans un dossier de saison, un simple numéro suffit (« 01 - Pilote », « Dark 03 »)
        let parsed = parse_episode(&stem).or_else(|| dir_season.and(bare_number(&stem)).map(|e| (None, vec![e])));
        match parsed {
            Some((s, eps)) => {
                let s = s.or_else(|| parent.as_deref().and_then(season_from_dir)).or(root_season);
                known.push((v.to_path_buf(), s, eps, parent.filter(|p| season_from_dir(p).is_some())));
            }
            None => unknown.push(v.to_path_buf()),
        }
    }
    if known.is_empty() {
        return None;
    }
    // épisodes sans saison : celle de tous les autres s'il n'y en a qu'une, sinon la 1
    let seasons_seen: Vec<u32> = {
        let mut v: Vec<u32> = known.iter().filter_map(|k| k.1).collect();
        v.sort();
        v.dedup();
        v
    };
    let mut map: BTreeMap<u32, Season> = BTreeMap::new();
    for (p, s, eps, dir) in known {
        let s = match s {
            Some(s) => s,
            None if seasons_seen.len() <= 1 => seasons_seen.first().copied().unwrap_or(1),
            None => {
                unknown.push(p);
                continue;
            }
        };
        let size = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let season = map.entry(s).or_insert_with(|| Season { number: s, dir: None, episodes: Vec::new(), missing: Vec::new(), duplicates: Vec::new(), extras: Vec::new() });
        if season.dir.is_none() {
            season.dir = dir;
        }
        season.episodes.push(Episode { path: p, season: s, episodes: eps, size });
    }
    for s in map.values_mut() {
        s.episodes.sort_by(|a, b| a.episodes.first().cmp(&b.episodes.first()).then(a.path.cmp(&b.path)));
        let mut seen: BTreeMap<u32, usize> = BTreeMap::new();
        for e in &s.episodes {
            for n in &e.episodes {
                *seen.entry(*n).or_default() += 1;
            }
        }
        let last = s.last();
        s.missing = (1..=last).filter(|n| !seen.contains_key(n)).collect();
        s.duplicates = seen.iter().filter(|(_, c)| **c > 1).map(|(n, _)| *n).collect();
    }
    // sous-titres / NFO : avec l'épisode dont ils portent le nom, sinon avec la saison de leur dossier
    for x in files.iter().filter(|p| is_extra(p)) {
        let stem = prep::file_name(x).to_lowercase();
        let by_ep = map.values().find(|s| s.episodes.iter().any(|e| stem.starts_with(&e.path.file_stem().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default()))).map(|s| s.number);
        let by_dir = x.parent().filter(|p| *p != root).map(prep::file_name).and_then(|d| map.values().find(|s| s.dir.as_deref() == Some(d.as_str())).map(|s| s.number));
        if let Some(n) = by_ep.or(by_dir) {
            if let Some(s) = map.get_mut(&n) {
                s.extras.push(x.clone());
            }
        }
    }
    let seasons: Vec<Season> = map.into_values().collect();

    // titre : dossier de la série, sinon début des noms d'épisodes
    let first = seasons.iter().flat_map(|s| s.episodes.first()).next()?;
    let ep_stem = first.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let (mut title, mut year) = if root_is_season {
        let t = clean_title(&root_name);
        if t.0.is_empty() {
            root.parent().map(|p| clean_title(&prep::file_name(p))).unwrap_or_default()
        } else {
            t
        }
    } else {
        clean_title(&root_name)
    };
    let (ep_title, ep_year) = clean_title(&ep_stem);
    if title.is_empty() {
        title = ep_title;
    }
    if year.is_empty() {
        year = ep_year;
    }
    // tag de groupe : celui des épisodes déjà nommés (« …-TAG »)
    let tag = ep_stem
        .rsplit_once('-')
        .map(|(_, t)| t.to_string())
        .filter(|t| (2..=15).contains(&t.len()) && t.chars().all(|c| c.is_ascii_alphanumeric()) && !t.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or_default();
    let named_whole = says_whole(&root_name) || (root_is_season && root.parent().map(|p| says_whole(&prep::file_name(p))).unwrap_or(false));
    // NFO de la série, à la racine, rattaché à aucune saison ni aucun épisode
    let attached: Vec<&PathBuf> = seasons.iter().flat_map(|s| s.extras.iter()).collect();
    let root_extras: Vec<PathBuf> = files
        .iter()
        .filter(|p| p.parent() == Some(root) && p.extension().map(|e| e.eq_ignore_ascii_case("nfo")).unwrap_or(false) && !attached.contains(p))
        .cloned()
        .collect();
    Some(Scan { root: root.to_path_buf(), title, year, tag, seasons, unknown, root_is_season, named_whole, root_extras })
}

/// Le dossier contient-il une série (au moins deux épisodes, ou une saison) ?
pub fn looks_like_series(dir: &Path) -> bool {
    scan(dir).map(|s| s.episode_count() >= 2 || s.root_is_season || s.seasons.iter().any(|x| x.dir.is_some())).unwrap_or(false)
}

// ------------------------------------------------------------------- noms

/// « S01 », « S01-S05 », ou « S01.S03 » si les saisons ne se suivent pas.
pub fn season_code(unit: &[u32]) -> String {
    let mut real: Vec<u32> = unit.iter().copied().filter(|n| *n > 0).collect();
    real.sort();
    real.dedup();
    if real.is_empty() {
        return if unit.contains(&0) { "S00".into() } else { String::new() };
    }
    if real.len() == 1 {
        return format!("S{:02}", real[0]);
    }
    if real.windows(2).all(|w| w[1] == w[0] + 1) {
        format!("S{:02}-S{:02}", real[0], real[real.len() - 1])
    } else {
        real.iter().map(|n| format!("S{n:02}")).collect::<Vec<_>>().join(".")
    }
}

/// « S01E02 », « S01E01E02 », « S01E01-E03 ».
pub fn episode_code(season: u32, eps: &[u32]) -> String {
    match eps {
        [] => format!("S{season:02}"),
        [e] => format!("S{season:02}E{e:02}"),
        [a, b] => format!("S{season:02}E{a:02}E{b:02}"),
        _ => format!("S{season:02}E{:02}-E{:02}", eps[0], eps[eps.len() - 1]),
    }
}

fn render(tpl: &str, f: &Fields, saison: &str, episode: &str, complete: &str) -> String {
    let t = tpl.replace("{saison}", saison).replace("{episode}", episode).replace("{épisode}", episode).replace("{complete}", complete);
    // le modèle de film sert de base : éléments vides retirés proprement
    prep::render_template(&t, &prep::NameParts { title: f.title.trim(), year: f.year.trim(), lang: f.lang.trim(), resolution: f.resolution.trim(), source: f.source.trim(), codec: f.codec.trim(), tag: f.tag.trim(), audio: f.audio.trim(), extra: f.extra.trim() })
}

fn template(cfg_value: &str, default: &str) -> String {
    if cfg_value.trim().is_empty() {
        default.to_string()
    } else {
        cfg_value.trim().to_string()
    }
}

/// Nom de release d'un ensemble de saisons.
pub fn release_name(cfg: &Config, f: &Fields, unit: &[u32], complete: bool) -> String {
    let word = if complete { cfg.complete_word.trim() } else { "" };
    render(&template(&cfg.series_template, SERIES_TEMPLATE), f, &season_code(unit), "", word)
}

/// Nom d'un épisode renommé (sans extension).
pub fn episode_name(cfg: &Config, f: &Fields, e: &Episode) -> String {
    render(&template(&cfg.episode_template, EPISODE_TEMPLATE), f, &season_code(&[e.season]), &episode_code(e.season, &e.episodes), "")
}

/// Ensembles postés : toute la sélection en une fois, ou une saison par release.
#[cfg(test)]
pub fn units(scan: &Scan, selected: &[u32], split: bool) -> Vec<Vec<u32>> {
    let sel: Vec<u32> = scan.numbers().into_iter().filter(|n| selected.contains(n)).collect();
    if sel.is_empty() {
        return Vec::new();
    }
    if split {
        sel.into_iter().map(|n| vec![n]).collect()
    } else {
        vec![sel]
    }
}

/// Fichiers à livrer : (origine, chemin relatif dans le dossier de la release) :
/// `<fichier>` pour une saison, `<Saison N>/<fichier>` pour plusieurs.
pub fn layout(cfg: &Config, scan: &Scan, f: &Fields, unit: &[u32], rename: bool) -> Vec<(PathBuf, PathBuf)> {
    // une saison : épisodes directement dans la release ; plusieurs : un dossier par saison
    let several = scan.seasons.iter().filter(|s| unit.contains(&s.number)).count() > 1;
    let mut out = Vec::new();
    for s in scan.seasons.iter().filter(|s| unit.contains(&s.number)) {
        let folder = match (&s.dir, rename) {
            (Some(d), false) => d.clone(),
            _ => s.label(),
        };
        let dir = if several { PathBuf::from(&folder) } else { PathBuf::new() };
        // nouveaux noms des épisodes (et des sous-titres qui portent leur nom)
        let mut renames: Vec<(String, String)> = Vec::new();
        for e in &s.episodes {
            let ext = e.path.extension().and_then(|x| x.to_str()).unwrap_or("mkv").to_string();
            let old_stem = e.path.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            let name = if rename { format!("{}.{ext}", episode_name(cfg, f, e)) } else { prep::file_name(&e.path) };
            if rename {
                renames.push((old_stem, episode_name(cfg, f, e)));
            }
            out.push((e.path.clone(), dir.join(name)));
        }
        for x in &s.extras {
            // NFO d'origine pas gardé dans la release postée (réglage) : laissé à sa place
            if !cfg.nfo_keep && x.extension().map(|e| e.eq_ignore_ascii_case("nfo")).unwrap_or(false) {
                continue;
            }
            let name = prep::file_name(x);
            let new = renames
                .iter()
                .filter(|(old, _)| name.to_lowercase().starts_with(&old.to_lowercase()))
                .max_by_key(|(old, _)| old.len())
                .map(|(old, new)| format!("{new}{}", name.get(old.len()..).unwrap_or("")))
                .unwrap_or(name);
            out.push((x.clone(), dir.join(new)));
        }
    }
    // NFO de la série (racine du dossier) : à la racine de la release, s'il est gardé
    if cfg.nfo_keep && !out.is_empty() {
        for x in &scan.root_extras {
            out.push((x.clone(), PathBuf::from(prep::file_name(x))));
        }
    }
    // deux fichiers ne doivent jamais viser le même nom
    let mut seen = std::collections::HashSet::new();
    for (_, rel) in out.iter_mut() {
        let mut k = 2;
        let base = rel.clone();
        while !seen.insert(rel.to_string_lossy().to_lowercase()) {
            let stem = base.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = base.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
            *rel = base.with_file_name(format!("{stem} ({k}){ext}"));
            k += 1;
        }
    }
    out
}

/// Éléments du nom détectés : titre et tag du dossier / des épisodes, langue, résolution,
/// codec et source par MediaInfo sur le premier épisode.
pub fn detect_fields(cfg: &Config, scan: &Scan) -> (Fields, Option<prep::Analysis>) {
    let mut f = Fields { title: scan.title.clone(), year: scan.year.clone(), tag: if scan.tag.is_empty() { cfg.group.trim().to_string() } else { scan.tag.clone() }, ..Default::default() };
    let Some(first) = scan.first_episode(&scan.numbers()) else { return (f, None) };
    let a = prep::analyze(&first.path, cfg, Some("x"), None);
    f.lang = a.lang.clone();
    f.resolution = a.resolution.clone();
    f.codec = a.codec.clone();
    f.source = a.source.clone();
    if cfg.series_template.contains("{audio}") {
        f.audio = a.fields.audio.clone();
        if f.audio.is_empty() {
            f.audio = a.facts.as_ref().map(prep::audio_tag).unwrap_or_default();
        }
    }
    // une langue explicite dans le nom de l'épisode (TRUEFRENCH, MULTi, VOSTFR…) l'emporte
    let stem = first.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(l) = stem.split(|c: char| !c.is_alphanumeric()).find(|w| prep::LANGS.iter().any(|x| x.eq_ignore_ascii_case(w))) {
        f.lang = prep::LANGS.iter().find(|x| x.eq_ignore_ascii_case(l)).map(|x| x.to_string()).unwrap_or(f.lang);
    }
    (f, Some(a))
}

// ------------------------------------------------- plusieurs tours, résolutions

/// Le même scan, épisodes et à-côtés pris dans le dossier d'une release déjà livrée
/// (`origine → chemin relatif` de `layout`) : pour en faire d'autres releases (une par saison,
/// une par épisode) sans relire ni recopier la source.
pub fn relocate(scan: &Scan, placed: &[(PathBuf, PathBuf)], dest: &Path) -> Scan {
    let map: HashMap<&PathBuf, PathBuf> = placed.iter().map(|(src, rel)| (src, dest.join(rel))).collect();
    let mut out = scan.clone();
    out.root = dest.to_path_buf();
    for s in out.seasons.iter_mut() {
        s.dir = None;
        for e in s.episodes.iter_mut() {
            if let Some(p) = map.get(&e.path) {
                e.path = p.clone();
            }
        }
        s.extras = s.extras.iter().filter_map(|x| map.get(x).cloned()).collect();
    }
    out.root_extras = out.root_extras.iter().filter_map(|x| map.get(x).cloned()).collect();
    out
}

/// Résolution de chaque saison (MediaInfo du premier épisode de la saison).
pub fn season_resolutions(cfg: &Config, scan: &Scan, unit: &[u32]) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    for s in scan.seasons.iter().filter(|s| unit.contains(&s.number)) {
        if let Some(e) = s.episodes.first() {
            if let Ok(f) = prep::query_facts(cfg, &e.path) {
                let r = prep::resolution(&f);
                if !r.is_empty() {
                    out.insert(s.number, r);
                }
            }
        }
    }
    out
}

/// Résolution de l'ensemble : celle du plus grand nombre d'épisodes.
pub fn main_resolution(scan: &Scan, res: &BTreeMap<u32, String>) -> Option<String> {
    let mut count: BTreeMap<&String, usize> = BTreeMap::new();
    for (n, r) in res {
        *count.entry(r).or_default() += scan.season(*n).map(|s| s.count()).unwrap_or(0);
    }
    count.into_iter().max_by_key(|(_, c)| *c).map(|(r, _)| r.clone())
}

/// Saisons dont la résolution diffère de celle de l'ensemble : texte d'avertissement.
pub fn resolution_warning(scan: &Scan, res: &BTreeMap<u32, String>) -> Option<String> {
    let main = main_resolution(scan, res)?;
    let odd: Vec<String> = res.iter().filter(|(_, r)| **r != main).map(|(n, r)| format!("S{n:02} en {r}")).collect();
    (!odd.is_empty()).then(|| format!("{} ; le reste en {main}", odd.join(", ")))
}

/// Nom de release d'un épisode posté seul : son nom d'origine s'il en a déjà la forme
/// (S01E01 + tag), sinon le modèle des épisodes.
pub fn episode_release(cfg: &Config, f: &Fields, e: &Episode) -> String {
    let stem = e.path.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
    let p = prep::parse_release(&stem);
    let named = parse_episode(&stem).map(|(s, _)| s.is_some()).unwrap_or(false) && !p.tag.is_empty() && !stem.contains(' ');
    if named {
        prep::clean_name(&stem)
    } else {
        prep::clean_name(&episode_name(cfg, f, e))
    }
}

// -------------------------------------------------------------- traitement

/// Livre un ensemble de saisons dans `dest_dir/<release>/<Série>/<Saison N>/…` :
/// 1. fichiers placés (déplacés ou copiés) dans un dossier temporaire, renommés au besoin ;
/// 2. rapport MediaInfo du premier épisode, au nom de la release ;
/// 3. publication d'un coup du dossier de la release.
#[allow(clippy::too_many_arguments)]
pub fn execute(
    cfg: &Config,
    scan: &Scan,
    f: &Fields,
    unit: &[u32],
    rename: bool,
    release: &str,
    how: Transfer,
    dest_dir: &Path,
    progress: &mut dyn FnMut(Stage, u64, u64),
) -> Result<Done, String> {
    let files = layout(cfg, scan, f, unit, rename);
    if files.is_empty() {
        return Err("aucun épisode à livrer".into());
    }
    fs::create_dir_all(dest_dir).map_err(|e| format!("dossier de destination : {e}"))?;
    let dest = dest_dir.join(release);
    if dest.exists() {
        return Err(format!("{release} existe déjà dans la destination"));
    }
    let report = prep::report_path(cfg, dest_dir, release);
    if let Some(d) = report.parent() {
        fs::create_dir_all(d).map_err(|e| format!("dossier MediaInfo : {e}"))?;
    }
    let st = prep::staging_for(dest_dir, cfg);
    let staged_root = st.join(release);
    let _ = fs::remove_dir_all(&staged_root);
    fs::create_dir_all(&staged_root).map_err(|e| format!("dossier temporaire : {e}"))?;

    let total: u64 = files.iter().filter_map(|(p, _)| fs::metadata(p).ok()).map(|m| m.len()).sum();
    let mut done_bytes = 0u64;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let rollback = |moved: &[(PathBuf, PathBuf)], staged_root: &Path| {
        for (src, staged) in moved {
            let _ = fs::rename(staged, src);
        }
        let _ = fs::remove_dir_all(staged_root);
    };
    // 1. placement
    for (src, rel) in &files {
        if prep::job_cancelled() {
            rollback(&moved, &staged_root);
            return Err(prep::CANCELLED.to_string());
        }
        let to = staged_root.join(rel);
        if let Some(d) = to.parent() {
            if let Err(e) = fs::create_dir_all(d) {
                rollback(&moved, &staged_root);
                return Err(format!("dossier temporaire : {e}"));
            }
        }
        let size = fs::metadata(src).map(|m| m.len()).unwrap_or(0);
        if how == Transfer::Move && fs::rename(src, &to).is_ok() {
            moved.push((src.clone(), to));
        } else {
            let base = done_bytes;
            if let Err(e) = prep::place_copy(src, &to, how, &mut |d, _| progress(Stage::Copy, base + d, total)) {
                rollback(&moved, &staged_root);
                return Err(format!("copie de {} : {e}", prep::file_name(src)));
            }
        }
        done_bytes += size;
        progress(Stage::Copy, done_bytes, total);
    }
    // 2. MediaInfo du premier épisode (déjà à sa place et sous son nom final)
    progress(Stage::MediaInfo, 0, 1);
    let first = scan.first_episode(unit).and_then(|e| files.iter().find(|(src, _)| *src == e.path)).map(|(_, rel)| staged_root.join(rel));
    let Some(first) = first else {
        rollback(&moved, &staged_root);
        return Err("premier épisode introuvable".into());
    };
    let text = match prep::mediainfo_export(cfg, &first) {
        Ok(t) => t,
        Err(e) => {
            rollback(&moved, &staged_root);
            return Err(e);
        }
    };
    let report_tmp = report.with_file_name(format!("{}.tmp", prep::file_name(&report)));
    if let Err(e) = fs::write(&report_tmp, text).and_then(|_| fs::rename(&report_tmp, &report)) {
        let _ = fs::remove_file(&report_tmp);
        rollback(&moved, &staged_root);
        return Err(format!("écriture du rapport MediaInfo : {e}"));
    }
    // 3. publication
    progress(Stage::Publish, 0, 1);
    if fs::rename(&staged_root, &dest).is_err() {
        let mut cp_done = 0u64;
        let r = copy_tree(&staged_root, &dest, &mut |d| {
            cp_done += d;
            progress(Stage::Copy, cp_done, total)
        });
        if let Err(e) = r {
            let _ = fs::remove_dir_all(&dest);
            rollback(&moved, &staged_root);
            let _ = fs::remove_file(&report);
            return Err(format!("transfert vers la destination : {e}"));
        }
        let _ = fs::remove_dir_all(&staged_root);
    }
    // déplacement : originaux copiés supprimés, dossiers d'origine vidés retirés
    let mut left = Vec::new();
    if how == Transfer::Move {
        for (src, _) in &files {
            if !moved.iter().any(|(m, _)| m == src) {
                if let Err(e) = crate::prep::remove_original(src) {
                    left.push((src.clone(), e));
                }
            }
        }
        remove_empty_dirs(&scan.root);
    }
    let _ = fs::remove_dir(&st);
    progress(Stage::Publish, 1, 1);
    Ok(Done { dest, xml: report, left })
}

/// Remplace le code de saison d'un nom de release (S01-S03, S01.S03, S01) par celui de `n`.
pub fn release_for_season(release: &str, n: u32) -> String {
    let parts: Vec<&str> = release.split('.').collect();
    let is_code = |t: &str| {
        let t = t.to_lowercase();
        !t.is_empty() && t.split('-').all(|x| x.strip_prefix('s').map(|r| !r.is_empty() && r.len() <= 2 && r.chars().all(|c| c.is_ascii_digit())).unwrap_or(false))
    };
    let mut out: Vec<String> = Vec::new();
    let mut done = false;
    for p in parts {
        if is_code(p) {
            if !done {
                out.push(format!("S{n:02}"));
                done = true;
            }
        } else {
            out.push(p.to_string());
        }
    }
    if !done {
        // pas de code de saison : ajouté avant la langue, sinon à la fin du titre
        let pos = out.iter().position(|p| prep::LANGS.iter().any(|l| l.eq_ignore_ascii_case(p))).unwrap_or(out.len().min(1));
        out.insert(pos, format!("S{n:02}"));
    }
    out.join(".")
}

/// Découpe une release de série déjà livrée (`<sortie>/<release>/<Série>/<Saison N>/…`) en une
/// release par saison, pour un post sans compression qui garde un dossier par saison chez celui
/// qui télécharge. Rapport MediaInfo et plan (indexers visés) refaits pour chaque saison ;
/// les anciens retirés. Renvoie (dossier, nom de release) de chaque nouvelle release.
pub fn split_release(cfg: &Config, dir: &Path, release: &str) -> Result<Vec<(PathBuf, String)>, String> {
    let parent = dir.parent().ok_or("dossier de release invalide")?.to_path_buf();
    let season_dirs = |d: &Path| -> Vec<(u32, PathBuf)> {
        let mut v: Vec<(u32, PathBuf)> = fs::read_dir(d)
            .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_dir()).filter_map(|p| season_from_dir(&prep::file_name(&p)).map(|n| (n, p))).collect())
            .unwrap_or_default();
        v.sort();
        v
    };
    // release/Saison N (actuel) ou release/Série/Saison N (versions 3.4 et 3.5)
    let mut seasons = season_dirs(dir);
    if seasons.is_empty() {
        let subs: Vec<PathBuf> = fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !prep::file_name(p).starts_with('.')).collect();
        if let [one] = subs.as_slice() {
            seasons = season_dirs(one);
        }
    }
    if seasons.len() < 2 {
        return Err("une seule saison : rien à découper".into());
    }
    let plan = crate::ngpost::read_plan(cfg, release);
    let mut out = Vec::new();
    for (n, sdir) in seasons {
        let rel = release_for_season(release, n);
        let target = parent.join(&rel);
        if target.exists() {
            return Err(format!("{rel} existe déjà"));
        }
        // le dossier de la saison devient la release : épisodes directement dedans
        fs::rename(&sdir, &target).map_err(|e| format!("déplacement de {} : {e}", prep::file_name(&sdir)))?;
        // rapport MediaInfo du premier épisode de la saison, au nom de la nouvelle release
        if let Some(first) = prep::list_videos(&target, true).into_iter().next() {
            let report = prep::report_path(cfg, &parent, &rel);
            if let Some(d) = report.parent() {
                let _ = fs::create_dir_all(d);
            }
            if let Ok(t) = prep::mediainfo_export(cfg, &first) {
                let _ = fs::write(&report, t);
            }
        }
        if let Some(p) = &plan {
            let _ = crate::ngpost::write_plan(cfg, &rel, p);
            crate::ngpost::mark_series(cfg, &rel);
        }
        out.push((target, rel));
    }
    // l'ancienne release (vide) et ses fichiers d'accompagnement
    remove_empty_dirs(dir);
    let _ = fs::remove_file(prep::report_path(cfg, &parent, release));
    if let Some(w) = crate::ngpost::waiting_dir(cfg) {
        let old = w.join(release);
        let has_nzb = fs::read_dir(&old).map(|r| r.flatten().any(|e| e.path().extension().map(|x| x == "nzb").unwrap_or(false))).unwrap_or(false);
        if !has_nzb {
            let _ = fs::remove_dir_all(&old);
        }
    }
    Ok(out)
}

/// Déplace une release (fichier ou dossier) dans `to_dir` : renommage instantané sur le même
/// disque, sinon copie suivie (et arrêtable) puis suppression de l'original.
pub fn move_to(src: &Path, to_dir: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<PathBuf, String> {
    let dest = to_dir.join(prep::file_name(src));
    if dest.exists() {
        return Err(format!("{} existe déjà", dest.display()));
    }
    fs::create_dir_all(to_dir).map_err(|e| format!("dossier de destination : {e}"))?;
    if fs::rename(src, &dest).is_ok() {
        return Ok(dest);
    }
    let total = crate::ngpost::path_size(src);
    if let Some(free) = prep::free_space(to_dir) {
        if free < total + total / 50 {
            return Err(format!("{} sur {} : {} libres, {} nécessaires", crate::ngpost::SPACE_ERR, to_dir.display(), prep::human_size(free), prep::human_size(total)));
        }
    }
    let res = if src.is_dir() {
        let mut done = 0u64;
        copy_tree(src, &dest, &mut |d| {
            done += d;
            progress(done, total)
        })
    } else {
        prep::copy_with_progress(src, &dest, progress)
    };
    match res {
        Ok(_) => {
            let _ = if src.is_dir() { fs::remove_dir_all(src) } else { fs::remove_file(src) };
            Ok(dest)
        }
        Err(e) => {
            let _ = if dest.is_dir() { fs::remove_dir_all(&dest) } else { fs::remove_file(&dest) };
            Err(e.to_string())
        }
    }
}

/// Premier fichier vidéo d'une release livrée (pour le NFO).
pub fn first_file(dir: &Path) -> Option<PathBuf> {
    prep::list_videos(dir, true).into_iter().next()
}

pub(crate) fn copy_tree(from: &Path, to: &Path, step: &mut dyn FnMut(u64)) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)?.flatten() {
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_tree(&p, &t, step)?;
        } else {
            let mut last = 0;
            prep::copy_with_progress(&p, &t, &mut |d, _| {
                step(d - last);
                last = d;
            })?;
        }
    }
    Ok(())
}

/// Retire les dossiers vides (de bas en haut), le dossier lui-même compris.
pub(crate) fn remove_empty_dirs(dir: &Path) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                remove_empty_dirs(&e.path());
            }
        }
    }
    let _ = fs::remove_dir(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeros_d_episodes() {
        let p = |s: &str| parse_episode(s);
        assert_eq!(p("Breaking.Bad.S01E02.FRENCH.1080p.WEB.x264-TAG"), Some((Some(1), vec![2])));
        assert_eq!(p("breaking bad s1e2"), Some((Some(1), vec![2])));
        assert_eq!(p("Show.S01.E02.720p"), Some((Some(1), vec![2])));
        assert_eq!(p("Show.S02E01E02.MULTi.1080p"), Some((Some(2), vec![1, 2])));
        assert_eq!(p("Show.S02E01-E03.1080p"), Some((Some(2), vec![1, 2, 3])));
        assert_eq!(p("Show S02E01-02 FRENCH"), Some((Some(2), vec![1, 2])));
        assert_eq!(p("Show.S01E10.1080p-GRP"), Some((Some(1), vec![10])));
        assert_eq!(p("Show.S01E10-1080p"), Some((Some(1), vec![10]))); // pas un épisode 1080
        assert_eq!(p("Show.S2024E05"), None); // année, pas une saison
        assert_eq!(p("Show 1x02 Titre"), Some((Some(1), vec![2])));
        assert_eq!(p("Show.1920x1080.S01E03"), Some((Some(1), vec![3])));
        assert_eq!(p("Saison 2 Épisode 7"), Some((Some(2), vec![7])));
        assert_eq!(p("Season 3 Episode 12 - Titre"), Some((Some(3), vec![12])));
        assert_eq!(p("Episode 05"), Some((None, vec![5])));
        assert_eq!(p("Ep12"), Some((None, vec![12])));
        assert_eq!(p("E04 - Le retour"), Some((None, vec![4])));
        assert_eq!(p("[Grp] Mon Anime - 012 [1080p]"), Some((None, vec![12])));
        assert_eq!(p("Mon.Film.2010.FRENCH.1080p.BluRay.x264-TAG"), None);
        assert_eq!(p("Les Experts - 2010"), None);
    }

    #[test]
    fn saisons_et_titres() {
        assert_eq!(season_from_dir("Saison 1"), Some(1));
        assert_eq!(season_from_dir("Season 02"), Some(2));
        assert_eq!(season_from_dir("S03"), Some(3));
        assert_eq!(season_from_dir("Saison1"), Some(1));
        assert_eq!(season_from_dir("Breaking.Bad.S04.FRENCH.1080p.WEB.x264-TAG"), Some(4));
        assert_eq!(season_from_dir("Staffel 5"), Some(5));
        assert_eq!(season_from_dir("Spéciaux"), Some(0));
        assert_eq!(season_from_dir("Breaking Bad"), None);
        assert_eq!(season_from_dir("Les Simpson (1989)"), None);
        assert_eq!(clean_title("Breaking Bad (2008)"), ("Breaking.Bad".into(), "2008".into()));
        assert_eq!(clean_title("Breaking.Bad.S01.FRENCH.1080p"), ("Breaking.Bad".into(), String::new()));
        assert_eq!(clean_title("Grey's Anatomy - Saison 3"), ("Greys.Anatomy".into(), String::new()));
        assert_eq!(clean_title("The.Office.US.2005.S01E01"), ("The.Office.US".into(), "2005".into()));
        assert_eq!(clean_title("Kaamelott Integrale"), ("Kaamelott".into(), String::new()));
        assert_eq!(season_code(&[1]), "S01");
        assert_eq!(season_code(&[1, 2, 3]), "S01-S03");
        assert_eq!(season_code(&[1, 3]), "S01.S03");
        assert_eq!(season_code(&[0, 1, 2]), "S01-S02");
        assert_eq!(episode_code(1, &[2]), "S01E02");
        assert_eq!(episode_code(1, &[1, 2]), "S01E01E02");
        assert_eq!(episode_code(1, &[1, 2, 3]), "S01E01-E03");
        assert!(is_series_name("Breaking.Bad.S01.COMPLETE.FRENCH.1080p.WEB.x264-TAG"));
        assert!(is_series_name("Breaking.Bad.S01-S05.FRENCH.1080p-TAG"));
        assert!(is_series_name("Show.S01E02.FRENCH.720p-TAG"));
        assert!(!is_series_name("Mon.Film.2010.FRENCH.1080p.BluRay.x264-TAG"));
        assert!(!is_series_name("Seven.1995.FRENCH.1080p-TAG"));
        // « Saison.1 » et ses variantes : une série aussi (sinon : catégorie des films)
        for n in ["Ma.SerieS01.FRENCH.1080p.H264", "Ma serieS01 FRENCH 1080p H264", "Ma Serie S01 FRENCH 1080p H264", "TitreS01E02.FRENCH.720p", "Ma.Serie.Saison.1.FRENCH.1080p.H264", "Ma serieSaison.1.FRENCH.1080p.H264", "Ma.Serie.Saison1.FRENCH.1080p", "Show.Season.2.MULTi.2160p", "Show.Saisons.1-5.FRENCH.720p", "Show.Temporada.3.1080p", "Show.1x02.FRENCH.720p"] {
            assert!(is_series_name(n), "{n}");
        }
        for n in ["Season.of.the.Witch.2011.FRENCH.1080p", "Les.4.Saisons.1988.FRENCH.1080p", "Film.2x4.2020.1080p", "Mission.Impossible.2.1080p.x264"] {
            assert!(!is_series_name(n), "{n}");
        }
    }

    fn touch(p: &Path, len: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![0u8; len]).unwrap();
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nzbff-series-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn dossier_integrale_d_une_seule_saison() {
        let d = std::env::temp_dir().join(format!("nzbff-integrale-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let root = d.join("Ma Grande Serie - Intégrale");
        fs::create_dir_all(&root).unwrap();
        for i in 1..=12 {
            fs::write(root.join(format!("Ma.Grande.Serie.S01E{i:03}.FRENCH.480p.mkv")), b"x").unwrap();
        }
        let s = scan(&root).unwrap();
        assert!(s.named_whole && s.complete_series(&[1]), "intégrale d'une saison : complète");
        assert_eq!(s.title, "Ma.Grande.Serie");
        let cfg = Config { complete_word: "INTEGRALE".into(), ..Default::default() };
        let f = Fields { title: s.title.clone(), lang: "FRENCH".into(), resolution: "480p".into(), codec: "H264".into(), ..Default::default() };
        assert_eq!(release_name(&cfg, &f, &[1], true), "Ma.Grande.Serie.S01.INTEGRALE.FRENCH.480p.H264");
        assert!(says_whole("L'intégrale") && says_whole("Serie.COMPLETE.FRENCH") && says_whole("Serie Integrale") && !says_whole("Serie Saison 1"));
        // un épisode manquant : pas complète, même si le dossier le dit
        fs::remove_file(root.join("Ma.Grande.Serie.S01E005.FRENCH.480p.mkv")).unwrap();
        let s = scan(&root).unwrap();
        assert!(s.named_whole && !s.complete_series(&[1]));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn une_saison_de_plus_de_500_episodes() {
        let d = std::env::temp_dir().join(format!("nzbff-500-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        // 1. S01E001 → S01E540
        let a = d.join("Ma Grande Serie Integrale");
        fs::create_dir_all(&a).unwrap();
        for i in 1..=540 {
            fs::write(a.join(format!("Ma.Grande.Serie.S01E{i:03}.FRENCH.480p.H264.mkv")), b"x").unwrap();
        }
        let s = scan(&a).unwrap();
        assert_eq!((s.seasons.len(), s.episode_count(), s.season(1).unwrap().last()), (1, 540, 540));
        assert!(s.complete_series(&[1]) && looks_like_series(&a));
        // 2. numérotation continue sans saison (« Ma Grande Serie 001 », « - 0512 », « Episode 513 »)
        let b = d.join("Ma Grande Serie - L'intégrale");
        fs::create_dir_all(&b).unwrap();
        for i in 1..=510 {
            fs::write(b.join(format!("Ma Grande Serie {i:03} VF.mkv")), b"x").unwrap();
        }
        fs::write(b.join("Ma Grande Serie - 0511.mkv"), b"x").unwrap();
        fs::write(b.join("Ma Grande Serie Episode 512.mkv"), b"x").unwrap();
        let s = scan(&b).unwrap();
        assert_eq!((s.seasons.len(), s.episode_count(), s.unknown.len()), (1, 512, 0), "{:?}", s.unknown.iter().take(3).collect::<Vec<_>>());
        assert!(s.complete_series(&[1]));
        // 3. plus de 999 épisodes
        assert_eq!(parse_episode("Serie.S01E1043.FRENCH"), Some((Some(1), vec![1043])));
        assert_eq!(parse_episode("Serie.2010.S01E05"), Some((Some(1), vec![5])));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn analyse_serie_complete_et_trous() {
        let d = tmp("scan");
        let root = d.join("Breaking Bad (2008)");
        for e in 1..=7 {
            touch(&root.join("Saison 1").join(format!("Breaking.Bad.S01E{e:02}.FRENCH.1080p.WEB.x264-GRP.mkv")), 10);
        }
        touch(&root.join("Saison 1").join("Breaking.Bad.S01E01.FRENCH.1080p.WEB.x264-GRP.fr.srt"), 3);
        for e in [1, 2, 3, 5] {
            touch(&root.join("Saison 2").join(format!("Breaking.Bad.S02E{e:02}.FRENCH.1080p.WEB.x264-GRP.mkv")), 10);
        }
        touch(&root.join("Saison 2").join("Sample").join("sample.mkv"), 1);
        touch(&root.join("Saison 2").join("bonus making-of.mkv"), 1);
        let s = scan(&root).unwrap();
        assert_eq!((s.title.as_str(), s.year.as_str(), s.tag.as_str()), ("Breaking.Bad", "2008", "GRP"));
        assert_eq!(s.numbers(), vec![1, 2]);
        assert!(s.season(1).unwrap().complete());
        assert_eq!(s.season(1).unwrap().extras.len(), 1);
        assert_eq!(s.season(2).unwrap().missing, vec![4]);
        assert!(!s.whole_series());
        assert!(s.complete_for(&[1]));
        assert_eq!(s.unknown.len(), 1); // « bonus making-of » ; l'échantillon est ignoré
        assert_eq!(s.episode_count(), 11);
        assert!(looks_like_series(&root));
        // une saison choisie directement
        let s1 = scan(&root.join("Saison 1")).unwrap();
        assert!(s1.root_is_season);
        assert_eq!((s1.title.as_str(), s1.numbers()), ("Breaking.Bad", vec![1]));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn noms_et_arborescence() {
        let d = tmp("layout");
        let root = d.join("Kaamelott");
        for s in 1..=2 {
            for e in 1..=3 {
                touch(&root.join(format!("Season {s}")).join(format!("kaamelott {s}x{e:02}.avi")), 5);
            }
        }
        let s = scan(&root).unwrap();
        assert!(s.whole_series());
        // « complète » d'office : la série entière d'un bloc, jamais une saison seule
        assert!(s.complete_series(&[1, 2]) && !s.complete_series(&[1]) && !s.complete_series(&[2]));
        let season = scan(&root.join("Season 1")).unwrap();
        assert!(season.root_is_season && season.complete_for(&[1]) && !season.complete_series(&[1]));
        let cfg = Config { complete_word: "COMPLETE".into(), ..Default::default() };
        let f = Fields { title: "Kaamelott".into(), lang: "FRENCH".into(), resolution: "576p".into(), source: "DVDRip".into(), codec: "x264".into(), tag: "GRP".into(), ..Default::default() };
        assert_eq!(release_name(&cfg, &f, &[1, 2], true), "Kaamelott.S01-S02.COMPLETE.FRENCH.576p.DVDRip.x264-GRP");
        assert_eq!(release_name(&cfg, &f, &[2], false), "Kaamelott.S02.FRENCH.576p.DVDRip.x264-GRP");
        assert_eq!(units(&s, &[1, 2], true), vec![vec![1], vec![2]]);
        // sans renommage : noms et dossiers d'origine gardés
        // plusieurs saisons : un dossier par saison dans la release (noms d'origine gardés)
        let l = layout(&cfg, &s, &f, &[1, 2], false);
        assert_eq!(l[0].1, PathBuf::from("Season 1").join("kaamelott 1x01.avi"));
        // une saison : épisodes directement dans la release, renommés au format release
        let l = layout(&cfg, &s, &f, &[2], true);
        assert_eq!(l[0].1, PathBuf::from("Kaamelott.S02E01.FRENCH.576p.DVDRip.x264-GRP.avi"));
        let l = layout(&cfg, &s, &f, &[1, 2], true);
        assert_eq!(l[3].1, PathBuf::from("Saison 2").join("Kaamelott.S02E01.FRENCH.576p.DVDRip.x264-GRP.avi"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn deplacement_d_une_release() {
        let d = tmp("move");
        let rel = d.join("sortie").join("R.S01.FRENCH.1080p-GRP");
        touch(&rel.join("R").join("Saison 1").join("R.S01E01.mkv"), 10);
        let to = move_to(&rel, &d.join("autre"), &mut |_, _| {}).unwrap();
        assert_eq!(to, d.join("autre").join("R.S01.FRENCH.1080p-GRP"));
        assert!(to.join("R/Saison 1/R.S01E01.mkv").is_file() && !rel.exists());
        // déjà présent : refusé, rien n'est touché
        touch(&rel.join("x.mkv"), 1);
        assert!(move_to(&rel, &d.join("autre"), &mut |_, _| {}).is_err() && rel.exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn decoupe_par_saison() {
        assert_eq!(release_for_season("Breaking.Bad.2008.S01-S05.COMPLETE.FRENCH.1080p.WEB.x264-GRP", 3), "Breaking.Bad.2008.S03.COMPLETE.FRENCH.1080p.WEB.x264-GRP");
        assert_eq!(release_for_season("Dark.S01.S03.MULTi.1080p-ZT", 3), "Dark.S03.MULTi.1080p-ZT");
        assert_eq!(release_for_season("Kaamelott.FRENCH.576p", 2), "Kaamelott.S02.FRENCH.576p");
        let d = tmp("split");
        let out = d.join("sortie");
        let rel = "Show.S01-S02.COMPLETE.FRENCH.1080p-GRP";
        for (s, e) in [(1, 1), (1, 2), (2, 1)] {
            touch(&out.join(rel).join(format!("Saison {s}")).join(format!("Show.S{s:02}E{e:02}.mkv")), 3);
        }
        let cfg = Config { ngpost_dir: out.display().to_string(), mi_dest: "video".into(), mediainfo: "/bin/false".into(), ..Default::default() };
        let r = split_release(&cfg, &out.join(rel), rel).unwrap();
        let names: Vec<&str> = r.iter().map(|x| x.1.as_str()).collect();
        assert_eq!(names, vec!["Show.S01.COMPLETE.FRENCH.1080p-GRP", "Show.S02.COMPLETE.FRENCH.1080p-GRP"]);
        // une release par saison : épisodes directement dans la release
        assert!(out.join(names[1]).join("Show.S02E01.mkv").is_file());
        assert!(!out.join(rel).exists());
        // ancienne arborescence (release/Série/Saison N) toujours découpable
        let old = "Old.S01-S02.FRENCH-GRP";
        for s in 1..=2 {
            touch(&out.join(old).join("Old").join(format!("Saison {s}")).join(format!("Old.S{s:02}E01.mkv")), 3);
        }
        let r = split_release(&cfg, &out.join(old), old).unwrap();
        assert!(out.join(&r[0].1).join("Old.S01E01.mkv").is_file());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn episodes_a_plat_et_saisons_dans_les_noms() {
        let d = tmp("flat");
        let root = d.join("Dark");
        for (s, e) in [(1, 1), (1, 2), (2, 1)] {
            touch(&root.join(format!("Dark.S{s:02}E{e:02}.MULTi.1080p.NF.WEB-DL.x265-ZT.mkv")), 5);
        }
        let s = scan(&root).unwrap();
        assert_eq!(s.numbers(), vec![1, 2]);
        assert!(s.seasons.iter().all(|x| x.dir.is_none()));
        let cfg = Config::default();
        let f = Fields { title: "Dark".into(), lang: "MULTi".into(), resolution: "1080p".into(), tag: "ZT".into(), ..Default::default() };
        // pas de dossier de saison à l'origine : « Saison N » créé
        let l = layout(&cfg, &s, &f, &[1, 2], false);
        assert_eq!(l[2].1, PathBuf::from("Saison 2").join("Dark.S02E01.MULTi.1080p.NF.WEB-DL.x265-ZT.mkv"));
        let _ = fs::remove_dir_all(&d);
    }
}
