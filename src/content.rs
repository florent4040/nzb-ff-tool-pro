//! Autres contenus (jeux, logiciels, musique, ebooks, XXX…) : catégories entièrement
//! configurables (nom, icône, catégorie Newznab par défaut et par indexer, modèle de nom,
//! rapport MediaInfo, compression, dossier de l'inbox).
//!
//! Une release de contenu = un dossier `<sortie>\<release>\…` (fichier seul ou arborescence
//! d'origine), postée comme une série, avec un fichier caché `.nzbff-cat` qui suit la release
//! jusqu'aux dossiers des indexers : la catégorie envoyée par API vient de là.

use crate::prep::{self, Config, Done, Stage, Transfer};
use std::fs;
use std::path::{Path, PathBuf};

/// Fichier caché d'une release de contenu : « clé\tcatégorie Newznab par défaut ».
pub const CAT_FILE: &str = ".nzbff-cat";

#[derive(Debug, Clone, PartialEq)]
pub struct Category {
    /// Identifiant stable (ne change pas quand on renomme la catégorie).
    pub key: String,
    pub name: String,
    pub icon: String,
    /// Catégorie Newznab par défaut (ex. 4050) ; par indexer : `IndexerCfg::cat_map`.
    pub newznab: String,
    /// Modèle du nom de release, avec des {champs}.
    pub template: String,
    /// Rapport MediaInfo du premier fichier vidéo ou audio.
    pub mediainfo: bool,
    /// Compression au post : "conf" (comme les options du post), "on", "off".
    pub compress: String,
    /// Sous-dossier de l'inbox traité automatiquement avec cette catégorie.
    pub inbox: bool,
    /// Groupes Usenet de cette catégorie (vide : ceux des options du post).
    pub groups: String,
}

impl Category {
    pub fn line(&self) -> String {
        let c = |s: &str| s.replace(['\t', '\r', '\n'], " ");
        format!(
            "content_cat={}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            c(&self.key),
            c(self.name.trim()),
            c(self.icon.trim()),
            c(self.newznab.trim()),
            c(self.template.trim()),
            self.mediainfo,
            self.compress,
            self.inbox,
            c(self.groups.trim())
        )
    }

    pub fn parse_line(v: &str) -> Option<Category> {
        let f: Vec<&str> = v.split('\t').collect();
        if f.len() < 8 || f[0].trim().is_empty() || f[1].trim().is_empty() {
            return None;
        }
        Some(Category {
            key: f[0].trim().to_string(),
            name: f[1].trim().to_string(),
            icon: f[2].trim().to_string(),
            newznab: f[3].trim().to_string(),
            template: f[4].trim().to_string(),
            mediainfo: f[5].trim() == "true",
            compress: match f[6].trim() {
                "on" | "off" => f[6].trim().to_string(),
                _ => "conf".into(),
            },
            inbox: f[7].trim() == "true",
            groups: f.get(8).map(|g| g.trim().to_string()).unwrap_or_default(),
        })
    }

    pub fn label(&self) -> String {
        if self.icon.trim().is_empty() {
            self.name.clone()
        } else {
            format!("{} {}", self.icon.trim(), self.name)
        }
    }

    /// Dossier de l'inbox de cette catégorie.
    pub fn inbox_dir(&self, cfg: &Config) -> Option<PathBuf> {
        let i = cfg.inbox.trim();
        (self.inbox && !i.is_empty()).then(|| Path::new(i).join(inbox_folder(&self.name)))
    }

    /// Compression imposée au post (None : celle des options du post).
    pub fn compress_override(&self) -> Option<bool> {
        match self.compress.as_str() {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        }
    }
}

/// Nom du sous-dossier de l'inbox d'une catégorie : « @Jeux PC ».
pub fn inbox_folder(name: &str) -> String {
    format!("@{}", crate::ngpost::folder_name(name))
}

/// Catégories proposées au départ (toutes modifiables, supprimables).
pub fn defaults() -> Vec<Category> {
    let c = |key: &str, name: &str, icon: &str, nz: &str, tpl: &str, mi: bool, comp: &str| Category {
        key: key.into(),
        name: name.into(),
        icon: icon.into(),
        newznab: nz.into(),
        template: tpl.into(),
        mediainfo: mi,
        compress: comp.into(),
        inbox: false,
        groups: String::new(),
    };
    vec![
        c("xxx", "XXX / Adulte", "🔞", "6000", "{titre}.{annee}.{langue}.{resolution}.{source}.{codec}-{tag}", true, "conf"),
        c("jeux-pc", "Jeux PC", "🎮", "4050", "{titre}.{version}.{langue}.{plateforme}-{tag}", false, "on"),
        c("consoles", "Jeux consoles", "🕹", "1000", "{titre}.{plateforme}.{langue}.{version}-{tag}", false, "on"),
        c("logiciels-pc", "Logiciels PC", "💾", "4010", "{titre}.{version}.{langue}.{plateforme}-{tag}", false, "on"),
        c("logiciels-mac", "Logiciels Mac", "🍎", "4030", "{titre}.{version}.MacOS.{langue}-{tag}", false, "on"),
        c("musique", "Musique", "🎵", "3000", "{artiste}-{titre}-{annee}-{format}-{tag}", true, "conf"),
        c("ebooks", "Ebooks", "📚", "7020", "{auteur}-{titre}.{annee}.{langue}.{format}-{tag}", false, "conf"),
        c("mangas", "Mangas / BD", "📖", "7030", "{titre}.{tome}.{langue}.{format}-{tag}", false, "conf"),
        c("animation", "Animation", "🎞", "5070", "{titre}.{annee}.{langue}.{resolution}.{source}.{codec}-{tag}", true, "conf"),
        c("education", "Éducation / Formation", "🎓", "8010", "{titre}.{annee}.{langue}.{format}-{tag}", true, "conf"),
        c("autre", "Autre", "📦", "8010", "{titre}.{version}.{langue}-{tag}", false, "conf"),
        // disques complets : dossier BDMV / VIDEO_TS ou image ISO, postés compressés (l'arborescence
        // du disque doit arriver intacte), MediaInfo du titre principal, NFO BDInfo
        c("bluray", "Film Blu-ray complet", "💿", "2050", "{titre}.{annee}.{langue}.{resolution}.{source}-{tag}", true, "on"),
        c("dvd", "Film DVD complet", "📀", "2070", "{titre}.{annee}.{langue}.{format}.{source}-{tag}", true, "on"),
    ]
}

/// Groupes Usenet courants pour un type de post (« film », « tv » ou clé de catégorie) : ceux où
/// ce genre de contenu est posté et que les fournisseurs proposent.
pub fn group_suggestions(kind: &str) -> &'static [&'static str] {
    match kind {
        "film" => &["alt.binaries.movies", "alt.binaries.moovee", "alt.binaries.hdtv.x264", "alt.binaries.multimedia", "alt.binaries.boneless", "alt.binaries.misc"],
        "tv" => &["alt.binaries.teevee", "alt.binaries.tv", "alt.binaries.hdtv", "alt.binaries.multimedia", "alt.binaries.boneless", "alt.binaries.misc"],
        "jeux-pc" => &["alt.binaries.games", "alt.binaries.cd.image", "alt.binaries.iso", "alt.binaries.boneless"],
        "consoles" => &["alt.binaries.games", "alt.binaries.console.ps3", "alt.binaries.games.xbox360", "alt.binaries.nintendo.ds", "alt.binaries.boneless"],
        "logiciels-pc" => &["alt.binaries.cd.image", "alt.binaries.iso", "alt.binaries.warez", "alt.binaries.boneless"],
        "logiciels-mac" => &["alt.binaries.mac", "alt.binaries.mac.applications", "alt.binaries.boneless"],
        "musique" => &["alt.binaries.sounds.mp3", "alt.binaries.sounds.lossless", "alt.binaries.music", "alt.binaries.sounds.mp3.complete_cd"],
        "ebooks" => &["alt.binaries.e-book", "alt.binaries.e-book.flood", "alt.binaries.ebook", "alt.binaries.e-book.technical"],
        "mangas" => &["alt.binaries.pictures.comics.dcp", "alt.binaries.comics.dcp", "alt.binaries.e-book"],
        "animation" => &["alt.binaries.anime", "alt.binaries.multimedia.anime", "alt.binaries.multimedia.cartoons", "alt.binaries.multimedia"],
        "xxx" => &["alt.binaries.erotica", "alt.binaries.multimedia.erotica", "alt.binaries.boneless"],
        "education" => &["alt.binaries.e-book.technical", "alt.binaries.multimedia", "alt.binaries.misc"],
        _ => &["alt.binaries.misc", "alt.binaries.boneless"],
    }
}

/// Groupes d'une catégorie de contenu qui n'a pas les siens : des groupes où ce contenu est
/// posté (jamais ceux des films), selon la plateforme ou le format lus dans le nom.
pub fn default_groups(key: &str, release: &str) -> Vec<&'static str> {
    let up = release.to_uppercase();
    let has = |w: &str| up.split(|c: char| !c.is_alphanumeric()).any(|t| t == w);
    match key {
        "consoles" => {
            let mut v = vec!["alt.binaries.games", "alt.binaries.cd.image", "alt.binaries.boneless"];
            for p in crate::upload::platforms_in(release) {
                match p {
                    "ps3" => v.insert(1, "alt.binaries.console.ps3"),
                    "xbox360" => v.insert(1, "alt.binaries.games.xbox360"),
                    "nds" | "3ds" => v.insert(1, "alt.binaries.nintendo.ds"),
                    _ => {}
                }
            }
            v
        }
        "musique" if has("FLAC") || has("ALAC") || has("WAV") => vec!["alt.binaries.sounds.lossless", "alt.binaries.music", "alt.binaries.boneless"],
        "musique" => vec!["alt.binaries.sounds.mp3", "alt.binaries.music", "alt.binaries.boneless"],
        k => {
            let g = group_suggestions(k);
            g[..g.len().min(3)].to_vec()
        }
    }
}

/// Nouvelle clé unique tirée d'un nom.
pub fn new_key(name: &str, existing: &[Category]) -> String {
    let base: String = fold(name).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().split('-').filter(|x| !x.is_empty()).collect::<Vec<_>>().join("-");
    let base = if base.is_empty() { "categorie".to_string() } else { base };
    let mut k = base.clone();
    let mut n = 2;
    while existing.iter().any(|c| c.key == k) {
        k = format!("{base}-{n}");
        n += 1;
    }
    k
}

fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

// ---------------------------------------------------------------- nom

/// Champs utilisables dans un modèle : (clé, libellé).
pub const FIELDS: &[(&str, &str)] = &[
    ("titre", "Titre"),
    ("artiste", "Artiste"),
    ("auteur", "Auteur"),
    ("annee", "Année"),
    ("version", "Version"),
    ("tome", "Tome / volume"),
    ("plateforme", "Plateforme"),
    ("langue", "Langue"),
    ("format", "Format"),
    ("resolution", "Résolution"),
    ("source", "Source"),
    ("codec", "Codec"),
    ("audio", "Audio"),
    ("tag", "Tag"),
];

/// Champs présents dans un modèle (dans l'ordre de FIELDS).
pub fn fields_in(template: &str) -> Vec<&'static str> {
    FIELDS.iter().map(|f| f.0).filter(|k| template.contains(&format!("{{{k}}}"))).collect()
}

/// Valeurs des champs d'un nom.
pub type Values = Vec<(String, String)>;

pub fn get<'a>(v: &'a Values, k: &str) -> &'a str {
    v.iter().find(|(x, _)| x == k).map(|(_, y)| y.as_str()).unwrap_or("")
}

pub fn set(v: &mut Values, k: &str, val: &str) {
    match v.iter_mut().find(|(x, _)| x == k) {
        Some(e) => e.1 = val.to_string(),
        None => v.push((k.to_string(), val.to_string())),
    }
}

/// Applique le modèle ; les champs vides disparaissent avec leur séparateur.
pub fn render(template: &str, v: &Values) -> String {
    let mut s = template.to_string();
    for (k, _) in FIELDS {
        s = s.replace(&format!("{{{k}}}"), &prep::clean_name(get(v, k).trim()));
    }
    s = s.replace("{année}", &prep::clean_name(get(v, "annee"))).replace("{résolution}", &prep::clean_name(get(v, "resolution")));
    let s = prep::clean_name(&s);
    let mut s = s.split('.').filter(|x| !x.is_empty()).collect::<Vec<_>>().join(".");
    loop {
        let t = s.replace("-.", ".").replace(".-", "-").replace("--", "-").replace("..", ".");
        if t == s {
            break;
        }
        s = t;
    }
    s.trim_matches(|c| c == '.' || c == '-').to_string()
}

const LANGS: &[&str] = &["MULTi", "MULTI", "FRENCH", "TRUEFRENCH", "VFF", "VFQ", "VFI", "VOF", "VF2", "VOSTFR", "SUBFRENCH", "ENGLISH", "VO", "GERMAN", "SPANISH", "ITALIAN", "JAPANESE"];
const PLATFORMS: &[&str] = &["PC", "WIN", "WIN64", "X64", "X86", "MACOS", "MAC", "LINUX", "PS5", "PS4", "PS3", "PSP", "PSVITA", "SWITCH", "NSW", "XBOX", "XBOXONE", "X360", "WII", "WIIU", "3DS", "NDS", "ANDROID", "IOS"];
const FORMATS: &[&str] = &["PAL", "NTSC", "EPUB", "PDF", "MOBI", "AZW3", "CBZ", "CBR", "FLAC", "MP3", "AAC", "ALAC", "OGG", "WAV", "ISO", "NSP", "XCI", "PKG", "MP4", "MKV", "WEB", "CD", "VINYL"];
const SOURCES: &[&str] = &["WEB", "WEBRIP", "WEB-DL", "BLURAY", "BDRIP", "HDTV", "DVDRIP", "DVD", "DVDR", "DVD9", "DVD5", "DVDREMUX", "REMUX"];
const CODECS: &[&str] = &["X264", "X265", "H264", "H265", "HEVC", "AVC", "AV1", "XVID", "DIVX", "MPEG2", "VC-1", "VC1"];
const RESOLUTIONS: &[&str] = &["2160P", "1080P", "720P", "576P", "480P", "4K"];

/// Suggestions proposées dans l'interface.
pub const PLATFORM_CHOICES: &[&str] = &[
    "PC", "MacOS", "Linux", "PS1", "PS2", "PS3", "PS4", "PS5", "PSP", "PSVita", "XBOX", "X360", "XboxOne", "XboxSeries", "Wii", "WiiU", "Switch", "GameCube", "N64", "NDS", "3DS", "GBA", "Dreamcast", "Android", "iOS", "Retro",
];
pub const FORMAT_CHOICES: &[&str] = &["FLAC", "MP3", "EPUB", "PDF", "CBZ", "CBR", "ISO", "MP4", "MKV", "PAL", "NTSC"];

/// Catégorie devinée d'après les fichiers (clé d'une catégorie proposée, si elle existe encore).
pub fn guess(src: &Path, cats: &[Category]) -> Option<String> {
    if let Some(d) = crate::disc::detect(src) {
        if cats.iter().any(|c| c.key == d.kind.cat_key()) {
            return Some(d.kind.cat_key().to_string());
        }
    }
    let files = files_of(src);
    let n = files.len().max(1);
    let count = |exts: &[&str]| files.iter().filter(|(p, _)| p.extension().and_then(|e| e.to_str()).map(|e| exts.contains(&e.to_lowercase().as_str())).unwrap_or(false)).count();
    let name = prep::file_name(src).to_uppercase();
    let has = |k: &str| cats.iter().any(|c| c.key == k);
    let words: Vec<&str> = name.split(['.', ' ', '_', '-']).collect();
    let k = if count(AUDIO_EXT) * 2 >= n {
        "musique"
    } else if count(&["epub", "mobi", "azw3", "pdf"]) * 2 >= n {
        "ebooks"
    } else if count(&["cbz", "cbr", "cb7"]) * 2 >= n {
        "mangas"
    } else if count(&["dmg", "pkg", "app"]) > 0 || words.iter().any(|w| *w == "MACOS" || *w == "MAC") {
        "logiciels-mac"
    } else if count(&["nsp", "xci", "nsz", "wbfs", "cso", "pbp", "cia", "nds", "gba", "gcm", "rvz"]) > 0 || !crate::upload::platforms_in(&name).is_empty() {
        "consoles"
    } else if words.iter().any(|w| ["XXX", "ADULT", "PORN"].contains(w)) {
        "xxx"
    } else if count(&["iso", "exe", "bin"]) > 0 && words.iter().any(|w| ["GOG", "CODEX", "FITGIRL", "DODI", "ELAMIGOS", "RUNE", "TENOKE", "PLAZA", "SKIDROW", "RAZOR1911"].contains(w)) {
        "jeux-pc"
    } else if count(&["exe", "msi", "iso"]) > 0 {
        "logiciels-pc"
    } else {
        return None;
    };
    has(k).then(|| k.to_string())
}

/// Valeurs devinées d'après le nom du fichier ou du dossier (tout reste modifiable).
pub fn detect(cfg: &Config, src: &Path) -> Values {
    let raw = if src.is_dir() { prep::file_name(src) } else { src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() };
    let mut v: Values = Vec::new();
    // groupe : « …-GROUPE » en fin de nom
    let (body, group) = match raw.rsplit_once('-') {
        Some((b, g)) if !g.is_empty() && !g.contains(['.', ' ', '_']) && g.len() <= 15 && !b.is_empty() => (b.to_string(), g.to_string()),
        _ => (raw.clone(), String::new()),
    };
    let parts: Vec<&str> = body.split(['.', ' ', '_', '[', ']', '(', ')']).filter(|t| !t.is_empty()).collect();
    // « v1.2.3 » coupé par les points : on recolle les numéros qui suivent un « v1 »
    let mut tokens: Vec<String> = Vec::new();
    for t in parts {
        let numeric = t.chars().all(|c| c.is_ascii_digit());
        let after_version = tokens.last().map(|l: &String| {
            let mut c = l.chars();
            matches!(c.next(), Some('v') | Some('V')) && l.len() > 1 && l[1..].chars().all(|c| c.is_ascii_digit() || c == '.')
        });
        if numeric && after_version == Some(true) && t.len() <= 4 {
            let l = tokens.pop().unwrap_or_default();
            tokens.push(format!("{l}.{t}"));
        } else {
            tokens.push(t.to_string());
        }
    }
    let mut title: Vec<String> = Vec::new();
    let mut in_title = true;
    for t in &tokens {
        let u = t.to_uppercase();
        let year = t.len() == 4 && t.chars().all(|c| c.is_ascii_digit()) && (t.starts_with("19") || t.starts_with("20"));
        let version = (u.starts_with('V') && t.len() > 1 && t[1..].chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
            || (t.contains('.') && t.chars().all(|c| c.is_ascii_digit() || c == '.'));
        let tome = (u.starts_with("T") || u.starts_with("VOL")) && t.chars().skip_while(|c| c.is_alphabetic()).collect::<String>().chars().all(|c| c.is_ascii_digit()) && t.chars().any(|c| c.is_ascii_digit()) && t.len() <= 6;
        let known = if year {
            set(&mut v, "annee", t);
            true
        } else if LANGS.contains(&u.as_str()) {
            set(&mut v, "langue", if u == "MULTI" { "MULTi" } else { t });
            true
        } else if RESOLUTIONS.contains(&u.as_str()) {
            set(&mut v, "resolution", &t.to_lowercase());
            true
        } else if CODECS.contains(&u.as_str()) {
            set(&mut v, "codec", t);
            true
        } else if SOURCES.contains(&u.as_str()) {
            set(&mut v, "source", t);
            true
        } else if PLATFORMS.contains(&u.as_str()) || (crate::upload::platform_key(t).is_some() && !["DS", "GB", "DC", "VITA", "RETRO"].contains(&u.as_str())) {
            set(&mut v, "plateforme", t);
            true
        } else if FORMATS.contains(&u.as_str()) {
            set(&mut v, "format", t);
            true
        } else if version && !in_title {
            set(&mut v, "version", t);
            true
        } else if version && !title.is_empty() && u.starts_with('V') {
            set(&mut v, "version", t);
            true
        } else if tome && !title.is_empty() {
            set(&mut v, "tome", t);
            true
        } else {
            false
        };
        if known {
            in_title = false;
        } else if in_title {
            title.push(t.clone());
        }
    }
    // « Artiste - Album » (musique), « Auteur - Titre » (livres)
    let titre = title.join(".");
    set(&mut v, "titre", &titre);
    if let Some((a, b)) = raw.split_once(" - ") {
        set(&mut v, "artiste", &prep::clean_name(a.trim()));
        set(&mut v, "auteur", &prep::clean_name(a.trim()));
        let b = b.split(['(', '[']).next().unwrap_or(b).trim();
        if !b.is_empty() {
            set(&mut v, "titre", &prep::clean_name(b));
        }
    }
    // format : extension du fichier principal
    if get(&v, "format").is_empty() {
        if let Some(ext) = main_file(src).and_then(|p| p.extension().map(|e| e.to_string_lossy().to_uppercase())) {
            if FORMATS.contains(&ext.as_str()) {
                set(&mut v, "format", &ext);
            }
        }
    }
    let tag = if group.is_empty() { cfg.group.trim().to_string() } else { group };
    set(&mut v, "tag", &tag);
    v
}

/// Valeurs devinées, complétées par MediaInfo pour un disque complet (langue, résolution,
/// codec, audio, PAL / NTSC, source COMPLETE.BLURAY…). Lit le disque : à lancer hors de
/// l'interface.
pub fn detect_full(cfg: &Config, src: &Path) -> (Values, Option<crate::disc::Info>) {
    let mut v = detect(cfg, src);
    let Some(d) = crate::disc::detect(src) else { return (v, None) };
    let info = crate::disc::analyze(cfg, &d);
    if let Some(f) = &info.facts {
        fill_disc(&mut v, cfg, &d, f);
    }
    (v, Some(info))
}

/// Valeurs MediaInfo d'un disque : ajoutées là où le nom ne disait rien (la source vient
/// toujours du disque).
pub fn fill_disc(v: &mut Values, cfg: &Config, d: &crate::disc::Disc, f: &prep::Facts) {
    for (k, val) in crate::disc::name_values(cfg, d, f) {
        if get(v, k).is_empty() || k == "source" {
            set(v, k, &val);
        }
    }
}

/// Nom proposé : celui d'origine s'il a déjà la forme d'une release, sinon le modèle.
pub fn initial_name(cat: &Category, src: &Path, v: &Values) -> String {
    let raw = if src.is_dir() { prep::file_name(src) } else { src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() };
    if raw.contains('.') && !raw.contains(' ') && raw.rsplit_once('-').map(|(_, g)| !g.is_empty() && !g.contains('.')).unwrap_or(false) {
        return prep::clean_name(&raw);
    }
    render(&cat.template, v)
}

// ------------------------------------------------------------- fichiers

/// Fichiers d'un contenu : (chemin, chemin relatif dans la release). Fichiers cachés ignorés.
pub fn files_of(src: &Path) -> Vec<(PathBuf, PathBuf)> {
    if src.is_file() {
        return vec![(src.to_path_buf(), PathBuf::from(prep::file_name(src)))];
    }
    let mut out = Vec::new();
    walk(src, src, &mut out);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, PathBuf)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let n = prep::file_name(&p);
        if n.starts_with('.') || n.eq_ignore_ascii_case("thumbs.db") || n.eq_ignore_ascii_case("desktop.ini") {
            continue;
        }
        if p.is_dir() {
            walk(root, &p, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push((p.clone(), rel.to_path_buf()));
        }
    }
}

pub fn size_of(src: &Path) -> u64 {
    files_of(src).iter().filter_map(|(p, _)| fs::metadata(p).ok()).map(|m| m.len()).sum()
}

const AUDIO_EXT: &[&str] = &["flac", "mp3", "m4a", "aac", "ogg", "opus", "wav", "alac", "wma"];

pub fn is_media(p: &Path) -> bool {
    prep::is_video(p) || p.extension().and_then(|e| e.to_str()).map(|e| AUDIO_EXT.contains(&e.to_lowercase().as_str())).unwrap_or(false)
}

/// Fichiers système laissés par l'Explorateur ou macOS (sans valeur, gênent la suppression du dossier).
fn remove_junk(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let n = prep::file_name(&p).to_lowercase();
        if p.is_dir() {
            remove_junk(&p);
        } else if n == "thumbs.db" || n == "desktop.ini" || n == ".ds_store" {
            let _ = fs::remove_file(&p);
        }
    }
}

/// Premier fichier vidéo ou audio (pour le rapport MediaInfo).
pub fn first_media(src: &Path) -> Option<PathBuf> {
    files_of(src).into_iter().map(|(p, _)| p).find(|p| is_media(p))
}

/// Fichier principal : le plus gros.
pub fn main_file(src: &Path) -> Option<PathBuf> {
    files_of(src).into_iter().map(|(p, _)| p).max_by_key(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
}

/// NFO d'origine à côté d'un fichier seul : même nom (« Jeu.iso » / « Jeu.nfo »), sinon le
/// seul NFO du dossier quand ce fichier y est le seul gros fichier (dossier de release).
pub fn sibling_nfo(file: &Path) -> Option<PathBuf> {
    let is_nfo = |p: &Path| p.extension().map(|e| e.eq_ignore_ascii_case("nfo")).unwrap_or(false);
    let same = file.with_extension("nfo");
    if same.is_file() {
        return Some(same);
    }
    let dir = file.parent()?;
    let entries: Vec<PathBuf> = fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_file() && !prep::file_name(p).starts_with('.')).collect();
    let nfos: Vec<&PathBuf> = entries.iter().filter(|p| is_nfo(p)).collect();
    let big = entries.iter().filter(|p| !is_nfo(p) && fs::metadata(p).map(|m| m.len() > 4 * 1024 * 1024).unwrap_or(false)).count();
    (nfos.len() == 1 && big <= 1).then(|| nfos[0].clone())
}

/// Écrit la catégorie d'une release dans son dossier d'attente.
pub fn write_cat(cfg: &Config, release: &str, cat: &Category) {
    if let Some(w) = crate::ngpost::waiting_dir(cfg) {
        let d = w.join(release);
        if fs::create_dir_all(&d).is_ok() {
            let _ = fs::write(d.join(CAT_FILE), format!("{}\t{}\t{}\t{}\n", cat.key, cat.newznab.trim(), cat.name, if cat.mediainfo { "mi" } else { "" }));
        }
    }
}

/// La catégorie d'une release exige-t-elle un rapport MediaInfo ?
pub fn cat_wants_mi(dir: &Path) -> bool {
    fs::read_to_string(dir.join(CAT_FILE)).map(|t| t.trim_end().split('\t').nth(3) == Some("mi")).unwrap_or(false)
}

/// Catégorie d'une release (dossier d'attente ou d'indexer) : (clé, Newznab par défaut, nom).
pub fn read_cat(dir: &Path) -> Option<(String, String, String)> {
    let t = fs::read_to_string(dir.join(CAT_FILE)).ok()?;
    let f: Vec<&str> = t.trim_end().split('\t').collect();
    (f.len() >= 2 && !f[0].is_empty()).then(|| (f[0].to_string(), f[1].to_string(), f.get(2).unwrap_or(&"").to_string()))
}

/// Prépare la release : place le contenu dans `<dest_dir>\<release>\` (dossier temporaire puis
/// publication d'un coup), avec le rapport MediaInfo si la catégorie le demande et le NFO trouvé.
pub fn execute(cfg: &Config, cat: &Category, src: &Path, release: &str, how: Transfer, dest_dir: &Path, progress: &mut dyn FnMut(Stage, u64, u64)) -> Result<Done, String> {
    let is_nfo = |p: &Path| p.extension().map(|e| e.eq_ignore_ascii_case("nfo")).unwrap_or(false);
    // NFO d'origine : dans le dossier, sinon à côté du fichier (« Jeu.iso » + « Jeu.nfo »)
    let src_nfo = files_of(src).into_iter().map(|(p, _)| p).find(|p| is_nfo(p)).or_else(|| if src.is_file() { sibling_nfo(src) } else { None });
    // NFO d'origine pas gardé dans la release postée (réglage) : laissé à sa place
    let files: Vec<(PathBuf, PathBuf)> = files_of(src).into_iter().filter(|(p, _)| cfg.nfo_keep || !is_nfo(p)).collect();
    if files.is_empty() {
        return Err("rien à livrer (dossier vide)".into());
    }
    fs::create_dir_all(dest_dir).map_err(|e| format!("dossier de destination : {e}"))?;
    let dest = dest_dir.join(release);
    if dest.exists() {
        return Err(format!("{release} existe déjà dans la destination"));
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
    // 1. placement (déplacement instantané sur le même disque, sinon copie suivie)
    for (from, rel) in &files {
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
        let size = fs::metadata(from).map(|m| m.len()).unwrap_or(0);
        if how == Transfer::Move && fs::rename(from, &to).is_ok() {
            moved.push((from.clone(), to));
        } else {
            let base = done_bytes;
            if let Err(e) = prep::place_copy(from, &to, how, &mut |d, _| progress(Stage::Copy, base + d, total)) {
                rollback(&moved, &staged_root);
                return Err(format!("copie de {} : {e}", prep::file_name(from)));
            }
        }
        done_bytes += size;
        progress(Stage::Copy, done_bytes, total);
    }
    // 2. rapport MediaInfo (premier fichier vidéo ou audio), facultatif ; disque complet : titre
    // principal (playlist .mpls / jeu de titres VTS), image ISO montée le temps de la lecture
    let mut report = PathBuf::new();
    let disc = if src.is_file() { crate::disc::detect(&staged_root.join(prep::file_name(src))) } else { crate::disc::detect(&staged_root) };
    if let (true, Some(d)) = (cat.mediainfo, &disc) {
        progress(Stage::MediaInfo, 0, 1);
        let r = prep::report_path(cfg, dest_dir, release);
        if let Some(dd) = r.parent() {
            let _ = fs::create_dir_all(dd);
        }
        match crate::disc::mediainfo_export(cfg, d).and_then(|t| fs::write(&r, t).map_err(|e| format!("écriture du rapport MediaInfo : {e}"))) {
            Ok(_) => report = r,
            Err(e) => {
                rollback(&moved, &staged_root);
                return Err(format!("MediaInfo du {} : {e}", d.kind.label()));
            }
        }
        // rapport BDInfo du titre principal (NFO des Blu-ray), long : plusieurs minutes
        // texte modifié à la main avant le traitement : pris tel quel, sans nouveau scan
        let edited = cfg.bd_override.clone().filter(|t| !t.trim().is_empty());
        if d.kind == crate::disc::Kind::Bluray && (edited.is_some() || (cfg.bdinfo_scan && crate::disc::bdinfo_exe(cfg).is_some())) {
            if let Some(w) = crate::ngpost::waiting_dir(cfg).filter(|_| cfg.action != "rename") {
                progress(Stage::MediaInfo, 0, 1);
                let got = match edited {
                    Some(t) => Ok(t),
                    // rapport propre, selon le modèle BDInfo des réglages
                    None => crate::disc::bdinfo_report(cfg, d, &mut |_| {}, &prep::job_cancelled).map(|raw| crate::disc::bdinfo_render(&raw, &cfg.bd_sections, &cfg.bd_template, release)),
                };
                match got {
                    Ok(t) => {
                        let wd = w.join(release);
                        let _ = fs::create_dir_all(&wd);
                        let _ = fs::write(wd.join(format!("{release}.bdinfo.txt")), t);
                    }
                    Err(e) if prep::is_cancel(&e) => {
                        rollback(&moved, &staged_root);
                        if !report.as_os_str().is_empty() {
                            let _ = fs::remove_file(&report);
                        }
                        return Err(e);
                    }
                    Err(e) => crate::notify::record(format!("⚠ {release} : rapport BDInfo impossible ({e}), NFO tiré de MediaInfo")),
                }
            }
        }
    } else if cat.mediainfo {
        if let Some(media) = first_media(&staged_root) {
            progress(Stage::MediaInfo, 0, 1);
            let r = prep::report_path(cfg, dest_dir, release);
            if let Some(d) = r.parent() {
                let _ = fs::create_dir_all(d);
            }
            match prep::mediainfo_export(cfg, &media).and_then(|t| fs::write(&r, t).map_err(|e| format!("écriture du rapport MediaInfo : {e}"))) {
                Ok(_) => report = r,
                Err(e) => {
                    rollback(&moved, &staged_root);
                    return Err(e);
                }
            }
        }
    }
    // 3. publication
    progress(Stage::Publish, 0, 1);
    if fs::rename(&staged_root, &dest).is_err() {
        let mut cp = 0u64;
        if let Err(e) = crate::series::copy_tree(&staged_root, &dest, &mut |d| {
            cp += d;
            progress(Stage::Copy, cp, total)
        }) {
            let _ = fs::remove_dir_all(&dest);
            rollback(&moved, &staged_root);
            if !report.as_os_str().is_empty() {
                let _ = fs::remove_file(&report);
            }
            return Err(format!("transfert vers la destination : {e}"));
        }
        let _ = fs::remove_dir_all(&staged_root);
    }
    let mut left = Vec::new();
    if how == Transfer::Move {
        for (from, _) in &files {
            if !moved.iter().any(|(m, _)| m == from) {
                if let Err(e) = prep::remove_original(from) {
                    left.push((from.clone(), e));
                }
            }
        }
        if src.is_dir() {
            remove_junk(src);
            crate::series::remove_empty_dirs(src);
        }
    }
    let _ = fs::remove_dir(&st);
    // NFO du contenu (jeux, logiciels…) : copié à côté du NZB pour les indexers
    if let Some(w) = crate::ngpost::waiting_dir(cfg) {
        let kept = files_of(&dest).into_iter().map(|(p, _)| p).find(|p| is_nfo(p));
        let d = w.join(release);
        let orig = kept.or(src_nfo.filter(|p| p.is_file()));
        if let (Some(nfo), false) = (&orig, cfg.nfo_send) {
            // pas envoyé (réglage) : noté, pour ne pas en créer un autre à la place
            crate::notify::record(format!("NFO d'origine « {} » non envoyé aux indexers (réglage)", prep::file_name(nfo)));
            if fs::create_dir_all(&d).is_ok() {
                let _ = fs::write(d.join(crate::ngpost::NFO_FILE), "non-envoye");
            }
        }
        if let Some(nfo) = orig.filter(|_| cfg.nfo_send) {
            if fs::create_dir_all(&d).is_ok() {
                if fs::copy(&nfo, d.join(format!("{release}.nfo"))).is_ok() {
                    let rel = nfo.strip_prefix(&dest).map(|r| r.display().to_string()).unwrap_or_else(|_| prep::file_name(&nfo));
                    let _ = fs::write(d.join(crate::ngpost::NFO_FILE), format!("origine:{rel}"));
                }
            }
        }
    }
    progress(Stage::Publish, 1, 1);
    Ok(Done { dest, xml: report, left })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modeles_et_detection() {
        let cats = defaults();
        let jeux = cats.iter().find(|c| c.key == "jeux-pc").unwrap();
        assert_eq!(fields_in(&jeux.template), vec!["titre", "version", "plateforme", "langue", "tag"]);
        let mut v: Values = Vec::new();
        set(&mut v, "titre", "Mon Super Jeu");
        set(&mut v, "langue", "MULTi");
        set(&mut v, "tag", "NZBFF");
        // champs vides retirés proprement
        assert_eq!(render(&jeux.template, &v), "Mon.Super.Jeu.MULTi-NZBFF");
        set(&mut v, "version", "v1.2.3");
        set(&mut v, "plateforme", "PC");
        assert_eq!(render(&jeux.template, &v), "Mon.Super.Jeu.v1.2.3.MULTi.PC-NZBFF");
        let musique = cats.iter().find(|c| c.key == "musique").unwrap();
        let m: Values = vec![("artiste".into(), "Daft Punk".into()), ("titre".into(), "Discovery".into()), ("annee".into(), "2001".into()), ("format".into(), "FLAC".into())];
        assert_eq!(render(&musique.template, &m), "Daft.Punk-Discovery-2001-FLAC");

        let cfg = Config { group: "NZBFF".into(), ..Default::default() };
        let d = std::env::temp_dir().join(format!("nzbff-content-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let g = d.join("Super_Jeu v1.04 MULTi PC");
        fs::create_dir_all(&g).unwrap();
        fs::write(g.join("setup.iso"), vec![0u8; 100]).unwrap();
        let v = detect(&cfg, &g);
        assert_eq!((get(&v, "titre"), get(&v, "version"), get(&v, "langue"), get(&v, "plateforme"), get(&v, "tag"), get(&v, "format")), ("Super.Jeu", "v1.04", "MULTi", "PC", "NZBFF", "ISO"));
        assert_eq!(initial_name(jeux, &g, &v), "Super.Jeu.v1.04.MULTi.PC-NZBFF");
        // nom déjà au format release : gardé
        let r = d.join("Super.Jeu.v1.04.MULTi-GRP");
        fs::create_dir_all(&r).unwrap();
        let v2 = detect(&cfg, &r);
        assert_eq!(get(&v2, "tag"), "GRP");
        assert_eq!(initial_name(jeux, &r, &v2), "Super.Jeu.v1.04.MULTi-GRP");
        let b = d.join("Victor Hugo - Les Misérables (1862).epub");
        fs::write(&b, b"x").unwrap();
        let v3 = detect(&cfg, &b);
        assert_eq!((get(&v3, "auteur"), get(&v3, "titre"), get(&v3, "format")), ("Victor.Hugo", "Les.Misérables", "EPUB"));
        // catégorie devinée
        assert_eq!(guess(&b, &cats).as_deref(), Some("ebooks"));
        assert_eq!(guess(&g, &cats).as_deref(), Some("logiciels-pc"));
        let a = d.join("Album");
        fs::create_dir_all(&a).unwrap();
        for t in ["01.flac", "02.flac", "cover.jpg"] {
            fs::write(a.join(t), b"x").unwrap();
        }
        assert_eq!(guess(&a, &cats).as_deref(), Some("musique"));
        assert_eq!(guess(&a, &[]), None);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn ligne_de_config_et_cle() {
        for c in defaults() {
            assert_eq!(Category::parse_line(c.line().trim_end().strip_prefix("content_cat=").unwrap()), Some(c));
        }
        let cats = defaults();
        assert_eq!(new_key("Jeux PC", &cats), "jeux-pc-2");
        assert_eq!(new_key("Séries Éducatives", &cats), "series-educatives");
        assert_eq!(inbox_folder("Jeux/PC"), "@Jeux_PC");
    }

    #[test]
    fn preparation_d_une_release() {
        let d = std::env::temp_dir().join(format!("nzbff-content-x-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let src = d.join("src").join("Mon Logiciel 2.0");
        fs::create_dir_all(src.join("crack")).unwrap();
        fs::write(src.join("setup.exe"), vec![1u8; 2000]).unwrap();
        fs::write(src.join("crack").join("readme.txt"), b"lisez-moi").unwrap();
        fs::write(src.join("info.nfo"), b"NFO").unwrap();
        fs::write(src.join("Thumbs.db"), b"cache").unwrap();
        let out = d.join("sortie");
        let idx = d.join("idx");
        let cfg = Config { ngpost_dir: out.display().to_string(), indexer_dir: idx.display().to_string(), ..Default::default() };
        let cat = defaults().into_iter().find(|c| c.key == "logiciels-pc").unwrap();
        let done = execute(&cfg, &cat, &src, "Mon.Logiciel.v2.0-NZBFF", Transfer::Move, &out, &mut |_, _, _| {}).unwrap();
        let rel = out.join("Mon.Logiciel.v2.0-NZBFF");
        assert_eq!(done.dest, rel);
        assert!(rel.join("setup.exe").is_file() && rel.join("crack").join("readme.txt").is_file() && !rel.join("Thumbs.db").exists());
        assert!(!src.exists(), "déplacé : dossier d'origine retiré");
        assert!(done.xml.as_os_str().is_empty(), "pas de MediaInfo pour un logiciel");
        assert!(idx.join(crate::ngpost::WAITING_DIR).join("Mon.Logiciel.v2.0-NZBFF").join("Mon.Logiciel.v2.0-NZBFF.nfo").is_file());
        write_cat(&cfg, "Mon.Logiciel.v2.0-NZBFF", &cat);
        let (k, nz, name) = read_cat(&idx.join(crate::ngpost::WAITING_DIR).join("Mon.Logiciel.v2.0-NZBFF")).unwrap();
        assert_eq!((k.as_str(), nz.as_str(), name.as_str()), ("logiciels-pc", "4010", "Logiciels PC"));
        // déjà présent : refus
        let f = d.join("autre.iso");
        fs::write(&f, b"iso").unwrap();
        assert!(execute(&cfg, &cat, &f, "Mon.Logiciel.v2.0-NZBFF", Transfer::Copy, &out, &mut |_, _, _| {}).is_err());
        // fichier seul : dans un dossier à son nom de release
        let done = execute(&cfg, &cat, &f, "Autre.ISO-NZBFF", Transfer::Copy, &out, &mut |_, _, _| {}).unwrap();
        assert!(done.dest.join("autre.iso").is_file() && f.exists());
        let _ = fs::remove_dir_all(&d);
    }
}
