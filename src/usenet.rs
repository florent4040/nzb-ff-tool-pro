//! Serveurs Usenet gérés dans l'application (comme les blocs [server] de ngPost.conf) :
//! plusieurs fournisseurs, plusieurs serveurs par fournisseur, test de latence et choix
//! automatique du plus rapide.
//!
//! Au moment du post, une copie de ngPost.conf est écrite dans le dossier temporaire du post :
//! mêmes options, mais ses blocs [server] sont remplacés par les serveurs d'ici. Le fichier
//! d'origine n'est jamais modifié. Mots de passe : jamais dans les journaux ni sur la ligne
//! de commande (seulement dans cette copie, effacée à la fin du post).

use crate::prep::Config;
use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    /// Fournisseur (regroupe plusieurs serveurs : régions, ports…).
    pub provider: String,
    pub host: String,
    pub port: u16,
    pub ssl: bool,
    pub user: String,
    pub pass: String,
    /// Connexions quand le serveur est utilisé avec les autres (simultané).
    pub connections: u32,
    /// Connexions quand il est utilisé seul (choisi comme le plus rapide) ; 0 : les mêmes.
    pub connections_solo: u32,
    pub enabled: bool,
    /// Serveur utilisé par NgPost pour vérifier les articles (nzbCheck).
    pub nzb_check: bool,
}

impl Default for Server {
    fn default() -> Self {
        Server { provider: String::new(), host: String::new(), port: 563, ssl: true, user: String::new(), pass: String::new(), connections: 20, connections_solo: 0, enabled: true, nzb_check: false }
    }
}

impl Server {
    pub fn key(&self) -> String {
        format!("{}:{}", self.host.trim().to_ascii_lowercase(), self.port)
    }

    /// Ligne de nzbff.conf (séparée par des tabulations).
    pub fn line(&self) -> String {
        let c = |s: &str| s.replace(['\t', '\r', '\n'], " ");
        format!(
            "usenet_server={}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            c(if self.provider.trim().is_empty() { provider_of(&self.host) } else { self.provider.trim().to_string() }.as_str()),
            c(self.host.trim()),
            self.port,
            self.ssl,
            c(&self.user),
            c(&self.pass),
            self.connections,
            self.enabled,
            self.nzb_check,
            self.connections_solo
        )
    }

    /// Connexions quand le serveur est utilisé seul.
    pub fn solo(&self) -> u32 {
        if self.connections_solo > 0 {
            self.connections_solo
        } else {
            self.connections
        }
    }

    /// Description complète pour les journaux (jamais le mot de passe).
    pub fn describe_with(&self, connections: u32) -> String {
        let prov = if self.provider.trim().is_empty() { provider_of(&self.host) } else { self.provider.trim().to_string() };
        let mut d = format!("{prov} · {}:{} {} · {connections} connexion(s)", self.host.trim(), self.port, if self.ssl { "SSL" } else { "sans SSL" });
        if !self.user.trim().is_empty() {
            d.push_str(&format!(" · identifiant {}", mask_login(self.user.trim())));
        }
        if !self.pass.trim().is_empty() {
            d.push_str(" · mot de passe enregistré (masqué)");
        }
        if self.nzb_check {
            d.push_str(" · vérification des articles (nzbCheck)");
        }
        if !self.enabled {
            d.push_str(" · désactivé");
        }
        d
    }

    pub fn parse_line(v: &str) -> Option<Server> {
        let f: Vec<&str> = v.split('\t').collect();
        if f.len() < 9 || f[1].trim().is_empty() {
            return None;
        }
        Some(Server {
            provider: f[0].trim().to_string(),
            host: f[1].trim().to_string(),
            port: f[2].trim().parse().unwrap_or(563),
            ssl: f[3].trim() == "true",
            user: f[4].to_string(),
            pass: f[5].to_string(),
            connections: f[6].trim().parse().unwrap_or(20),
            enabled: f[7].trim() != "false",
            nzb_check: f[8].trim() == "true",
            connections_solo: f.get(9).and_then(|v| v.trim().parse().ok()).unwrap_or(0),
        })
    }
}

/// Identifiant affiché à moitié : « monlogin » → « mon•••• » (assez pour reconnaître le
/// compte, jamais en entier dans un journal ou un récapitulatif).
pub fn mask_login(u: &str) -> String {
    let n = u.chars().count();
    let keep = match n {
        0..=3 => 1,
        4..=8 => 3,
        _ => 4,
    };
    let head: String = u.chars().take(keep.min(n)).collect();
    format!("{head}••••")
}

/// Fournisseur déduit du nom du serveur : « eu.news.newshosting.com » → « Newshosting ».
pub fn provider_of(host: &str) -> String {
    let parts: Vec<&str> = host.trim().split('.').filter(|p| !p.is_empty()).collect();
    let name = match parts.len() {
        0 => return "Fournisseur".into(),
        1 => parts[0],
        n => {
            // « co.uk », « com.au »… : on remonte d'un cran
            let sld = parts[n - 2];
            if n >= 3 && (sld.len() <= 3 && matches!(sld, "co" | "com" | "net" | "org")) {
                parts[n - 3]
            } else {
                sld
            }
        }
    };
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "Fournisseur".into(),
    }
}

const SERVER_KEYS: &[&str] = &["host", "port", "ssl", "user", "pass", "connection", "connections", "enabled", "nzbcheck", "sslport", "usessl"];

fn is_server_key(k: &str) -> bool {
    SERVER_KEYS.contains(&k.trim().to_ascii_lowercase().as_str())
}

/// Serveurs des blocs [server] de ngPost.conf.
pub fn parse_conf_servers(text: &str) -> Vec<Server> {
    let mut out: Vec<Server> = Vec::new();
    let mut cur: Option<Server> = None;
    let flush = |cur: &mut Option<Server>, out: &mut Vec<Server>| {
        if let Some(mut s) = cur.take() {
            if !s.host.trim().is_empty() {
                s.provider = provider_of(&s.host);
                out.push(s);
            }
        }
    };
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            flush(&mut cur, &mut out);
            if l.eq_ignore_ascii_case("[server]") {
                cur = Some(Server { port: 0, connections: 0, ..Default::default() });
            }
            continue;
        }
        if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let Some(s) = cur.as_mut() else { continue };
        if !is_server_key(k) {
            // une option générale après les serveurs : fin du bloc
            flush(&mut cur, &mut out);
            continue;
        }
        let v = v.trim().trim_matches('"').trim();
        let yes = matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on");
        match k.trim().to_ascii_lowercase().as_str() {
            "host" => s.host = v.to_string(),
            "port" => s.port = v.parse().unwrap_or(0),
            "ssl" | "usessl" => s.ssl = yes,
            "user" => s.user = v.to_string(),
            "pass" => s.pass = v.to_string(),
            "connection" | "connections" => s.connections = v.parse().unwrap_or(0),
            "enabled" => s.enabled = yes,
            "nzbcheck" => s.nzb_check = yes,
            _ => {}
        }
    }
    flush(&mut cur, &mut out);
    for s in &mut out {
        if s.port == 0 {
            s.port = if s.ssl { 563 } else { 119 };
        }
        if s.connections == 0 {
            s.connections = 20;
        }
    }
    out
}

/// Repère de l'endroit où se trouvaient les serveurs (remplacé à l'écriture).
const HERE: &str = "\u{0}serveurs";

/// Nos anciens titres (jamais gardés : ils s'empilaient à chaque écriture).
fn is_our_marker(l: &str) -> bool {
    let t = l.trim_start_matches('#').trim().to_lowercase();
    t.starts_with("serveurs usenet (écrits par nzb ff") || t.starts_with("serveurs usenet gérés par nzb ff") || t.starts_with("serveurs usenet ecrits par nzb ff")
}

/// Lignes de ngPost.conf sans ses blocs [server] ni nos anciens titres, avec un repère à la
/// place du premier bloc. Les commentaires qui suivent un bloc et annoncent la section
/// suivante sont gardés ; un bloc de commentaires identique à un précédent (titre répété),
/// et les lignes vides en série, sont retirés : un fichier abîmé par les anciennes versions
/// est ainsi réparé.
fn clean_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_server = false;
    let mut placed = false;
    // commentaires lus dans un bloc [server] : à garder s'ils annoncent autre chose
    let mut pending: Vec<String> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if is_our_marker(l) {
            continue;
        }
        if l.starts_with('[') {
            let server = l.eq_ignore_ascii_case("[server]");
            if server {
                // commentaires entre deux serveurs : notes de serveur, retirées
                pending.clear();
                if !placed {
                    out.push(HERE.to_string());
                    placed = true;
                }
                in_server = true;
                continue;
            }
            out.append(&mut pending);
            in_server = false;
        } else if in_server {
            if l.is_empty() {
                continue;
            }
            if l.starts_with('#') || l.starts_with(';') {
                pending.push(line.to_string());
                continue;
            }
            match l.split_once('=') {
                Some((k, _)) if is_server_key(k) => continue,
                _ => {
                    in_server = false;
                    out.push(String::new());
                    out.append(&mut pending);
                }
            }
        }
        out.push(line.trim_end().to_string());
    }
    out.append(&mut pending);
    // blocs de commentaires répétés, lignes vides en série
    let mut seen: Vec<Vec<String>> = Vec::new();
    let mut titles: Vec<String> = Vec::new();
    let mut res: Vec<String> = Vec::new();
    let mut i = 0;
    while i < out.len() {
        let is_c = |s: &str| {
            let t = s.trim_start();
            t.starts_with('#') || t.starts_with(';')
        };
        if is_c(&out[i]) {
            let mut j = i;
            while j < out.len() && is_c(&out[j]) {
                j += 1;
            }
            let block: Vec<String> = out[i..j].iter().map(|x| x.trim().to_string()).collect();
            // un bloc décoratif (que des #) peut se répéter : il sépare les sections
            let decorative = block.iter().all(|x| x.chars().all(|c| c == '#' || c == ';' || c == '-' || c == '=' || c.is_whitespace()));
            if decorative || !seen.contains(&block) {
                if !decorative {
                    seen.push(block);
                }
                // un titre de section déjà présent plus haut (« # servers section ### ») : retiré
                for l in &out[i..j] {
                    let t = l.trim().to_lowercase();
                    let title = !t.chars().all(|c| c == '#' || c.is_whitespace()) && (t.contains("section") || t.contains("server") || t.contains("serveur") || t.ends_with("##"));
                    if title && titles.contains(&t) {
                        continue;
                    }
                    if title {
                        titles.push(t);
                    }
                    res.push(l.clone());
                }
            }
            i = j;
            continue;
        }
        if out[i].trim().is_empty() && res.last().map(|x| x.trim().is_empty()).unwrap_or(true) {
            i += 1;
            continue;
        }
        res.push(out[i].clone());
        i += 1;
    }
    // un titre décoratif resté orphelin en double (#### / ####) : réduit à un
    res.dedup_by(|a, b| a.trim() == b.trim() && a.trim().starts_with('#') && a.trim().chars().all(|c| c == '#'));
    while res.last().map(|x| x.trim().is_empty()).unwrap_or(false) {
        res.pop();
    }
    res
}

/// Texte de ngPost.conf sans ses blocs [server] (et nettoyé, voir `clean_lines`).
#[allow(dead_code)]
pub fn strip_servers(text: &str) -> String {
    let mut s: String = clean_lines(text).into_iter().filter(|l| l != HERE).map(|l| l + "\n").collect();
    // pas de ligne vide double là où étaient les serveurs
    while s.contains("\n\n\n") {
        s = s.replace("\n\n\n", "\n\n");
    }
    s
}

/// Remplace (ou ajoute) une option générale de ngPost.conf : « KEY = valeur ». Une ligne
/// commentée « #KEY = … » est gardée, la valeur est écrite juste après.
pub fn set_option(text: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let k = key.to_ascii_lowercase();
    let name = |l: &str| l.split_once('=').map(|(a, _)| a.trim().to_ascii_lowercase());
    let new = format!("{key} = {value}");
    if let Some(i) = lines.iter().position(|l| !l.trim_start().starts_with(['#', ';']) && name(l).as_deref() == Some(k.as_str())) {
        lines[i] = new;
    } else if let Some(i) = lines.iter().position(|l| l.trim_start().starts_with('#') && name(l.trim_start().trim_start_matches('#')).as_deref() == Some(k.as_str())) {
        lines.insert(i + 1, new);
    } else {
        // avant le premier bloc [server], sinon à la fin
        match lines.iter().position(|l| l.trim().eq_ignore_ascii_case("[server]") || l == HERE || l.trim() == MARK) {
            Some(i) => {
                lines.insert(i, String::new());
                lines.insert(i, new);
            }
            None => lines.push(new),
        }
    }
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

/// ngPost.conf nettoyé avec `blocks` à la place des anciens serveurs : là où ils étaient, sinon
/// après le titre « servers section » de ngPost, sinon à la fin.
fn place_servers(base: &str, title: &str, blocks: &str) -> String {
    let mut lines = clean_lines(base);
    let ins = format!("{title}\n{}\n", blocks.trim_end());
    match lines.iter().position(|l| l == HERE) {
        Some(i) => lines[i] = ins,
        None => {
            let after = lines.iter().position(|l| l.trim_start().starts_with('#') && l.to_lowercase().contains("server") && l.to_lowercase().contains("section")).map(|mut i| {
                while i + 1 < lines.len() && lines[i + 1].trim_start().starts_with('#') {
                    i += 1;
                }
                i + 1
            });
            match after {
                Some(i) => lines.insert(i, ins),
                None => {
                    lines.push(String::new());
                    lines.push(ins);
                }
            }
        }
    }
    let mut s = lines.join("\n");
    while s.contains("\n\n\n") {
        s = s.replace("\n\n\n", "\n\n");
    }
    // une ligne vide autour du bloc
    s = s.replace(&format!("\n{title}"), &format!("\n\n{title}")).replace("\n\n\n", "\n\n");
    s.trim_start_matches('\n').trim_end().to_string() + "\n"
}

/// Bloc [server] au format de ngPost.conf.
pub fn server_block(s: &Server) -> String {
    format!(
        "[server]\nhost = {}\nport = {}\nssl = {}\nuser = {}\npass = {}\nconnection = {}\nenabled = true\nnzbCheck = {}\n\n",
        s.host.trim(),
        s.port,
        s.ssl,
        s.user,
        s.pass,
        s.connections.max(1),
        s.nzb_check
    )
}

/// Bloc [server] avec l'état activé / désactivé réel (pour ngPost.conf lui-même).
fn server_block_full(s: &Server) -> String {
    server_block(s).replace("enabled = true\n", &format!("enabled = {}\n", s.enabled))
}

/// Écrit les serveurs d'ici dans ngPost.conf (à la place des siens, autres options gardées).
/// Une copie de l'ancien fichier est gardée une fois par jour (`ngPost.conf.nzbff-AAAA-MM-JJ.bak`).
/// Renvoie le nombre de serveurs écrits.
pub fn write_to_conf(conf: &Path, servers: &[Server], opts: &[(&str, String)]) -> Result<usize, String> {
    let list: Vec<&Server> = servers.iter().filter(|s| !s.host.trim().is_empty()).collect();
    if list.is_empty() {
        return Err("aucun serveur à écrire".into());
    }
    let old = std::fs::read(conf).map(|b| crate::ngpost::decode_text(&b)).map_err(|e| format!("lecture de {} : {e}", conf.display()))?;
    let l = crate::history::local_time(crate::prep::now_secs());
    let bak = conf.with_file_name(format!("{}.nzbff-{:04}-{:02}-{:02}.bak", conf.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), l.year, l.month, l.day));
    if !bak.exists() {
        std::fs::write(&bak, &old).map_err(|e| format!("copie de sauvegarde : {e}"))?;
    }
    let blocks: String = list.iter().map(|s| server_block_full(s)).collect();
    let mut out = place_servers(&old, MARK, &blocks);
    for (k, v) in opts {
        if !v.trim().is_empty() {
            out = set_option(&out, k, v.trim());
        }
    }
    // fins de ligne Windows, comme ngPost
    let out = if cfg!(windows) { out.replace("\r\n", "\n").replace('\n', "\r\n") } else { out };
    let tmp = conf.with_extension("conf.tmp");
    std::fs::write(&tmp, out).and_then(|_| std::fs::rename(&tmp, conf)).map_err(|e| format!("écriture de {} : {e}", conf.display()))?;
    Ok(list.len())
}

/// ngPost.conf d'origine (ou vide) avec les serveurs d'ici à la place des siens.
pub fn build_conf(base: &str, servers: &[&Server]) -> String {
    let blocks: String = servers.iter().map(|s| server_block(s)).collect();
    place_servers(base, MARK, &blocks)
}

/// Titre unique posé au-dessus de nos serveurs (retiré et reposé à chaque écriture).
const MARK: &str = "## Serveurs Usenet gérés par NZB FF Tool Pro";

/// Politique de groupes de ngPost (GROUP_POLICY) à écrire, selon le réglage et les groupes.
/// « each_post » : l'application tire elle-même un groupe au hasard (voir `pick_group`).
pub fn group_policy(policy: &str, groups: &str) -> Option<&'static str> {
    let n = groups.split(',').filter(|g| !g.trim().is_empty()).count();
    match policy {
        "all" => Some("ALL"),
        "each_file" => Some("EACH_FILE"),
        "each_post" => Some("ALL"),
        // automatique : un ou deux groupes → tous ; davantage → chaque fichier dans un groupe
        // différent (articles répartis, moins de risque de tout perdre sur un groupe)
        "auto" if n >= 3 => Some("EACH_FILE"),
        "auto" => Some("ALL"),
        _ => None,
    }
}

/// Valeur de GROUP_POLICY à écrire dans ngPost.conf lui-même (sans l'application).
pub fn conf_group_policy(policy: &str, groups: &str) -> Option<&'static str> {
    if policy == "each_post" {
        Some("EACH_POST")
    } else {
        group_policy(policy, groups)
    }
}

/// Un groupe au hasard parmi la liste (politique « un groupe au hasard par post »).
pub fn pick_group(groups: &str) -> String {
    let list: Vec<&str> = groups.split(',').map(|g| g.trim()).filter(|g| !g.is_empty()).collect();
    if list.is_empty() {
        return String::new();
    }
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as usize ^ (d.as_secs() as usize)).unwrap_or(0);
    list[n % list.len()].to_string()
}

/// Libellé d'une politique de groupes.
pub fn policy_label(p: &str) -> &'static str {
    match p {
        "all" => "tous les groupes à chaque post",
        "each_post" => "un groupe au hasard par post",
        "each_file" => "un groupe au hasard par fichier",
        "auto" => "automatique",
        _ => "celle de ngPost.conf",
    }
}

/// Serveurs à utiliser pour ce post : tous ceux activés, ou seulement le plus rapide.
pub fn servers_for_post(cfg: &Config, on_line: &mut dyn FnMut(&str)) -> Vec<Server> {
    let on: Vec<Server> = cfg.usenet_servers.iter().filter(|s| s.enabled && !s.host.trim().is_empty()).cloned().collect();
    if cfg.log_detail {
        for s in cfg.usenet_servers.iter().filter(|s| !s.enabled && !s.host.trim().is_empty()) {
            on_line(&format!("Serveur ignoré (désactivé dans les réglages) : {}", s.describe_with(s.connections)));
        }
    }
    if !cfg.usenet_auto_best || on.len() < 2 {
        on_line(&match (on.len(), cfg.usenet_auto_best) {
            (0, _) => "Choix des serveurs : aucun serveur activé".to_string(),
            (1, true) => "Choix des serveurs : un seul serveur activé, pas de test (« le plus rapide seulement » sans objet)".to_string(),
            (1, false) => "Choix des serveurs : un seul serveur activé".to_string(),
            (n, _) => format!("Choix des serveurs : les {n} serveurs activés en même temps (« le plus rapide seulement » coupé dans Réglages → Serveurs Usenet)"),
        });
        return on;
    }
    on_line(&format!(
        "Choix des serveurs : test des {} serveurs activés (2 s chacun, en parallèle), le plus rapide à répondre aux commandes est gardé seul ; identifiants refusés = jamais retenu",
        on.len()
    ));
    let res = test_all(&on, 2);
    let best = best_of(&res).map(|i| (i, res.iter().find(|(j, _)| *j == i).and_then(|(_, r)| r.as_ref().ok()).map(|s| s.score_ms()).unwrap_or(0)));
    for (i, r) in &res {
        let s = &on[*i];
        match r {
            Ok(sp) if sp.auth == Some(false) => on_line(&format!("Test {} ({}:{}) : identifiants refusés ({})", s.provider, s.host, s.port, sp.auth_reply.clone().unwrap_or_default())),
            Ok(sp) => on_line(&format!("Test {} ({}:{}) : {}", s.provider, s.host, s.port, if cfg.log_detail { describe_full(sp) } else { describe(sp) })),
            Err(e) => on_line(&format!("Test {} ({}:{}) : injoignable ({e})", s.provider, s.host, s.port)),
        }
    }
    match best {
        Some((i, ms)) => {
            let mut best = on[i].clone();
            let solo = best.solo();
            on_line(&format!(
                "Serveur le plus rapide retenu ({ms} ms) : {}{}",
                best.describe_with(solo),
                if solo != best.connections { format!(" — utilisé seul : {solo} connexions au lieu de {} en simultané", best.connections) } else { String::new() }
            ));
            best.connections = solo;
            let mut v = vec![best];
            // le serveur de vérification des articles reste utilisé s'il est à part
            v.extend(on.iter().filter(|s| s.nzb_check && s.key() != on[i].key()).cloned());
            v
        }
        None => {
            on_line("Aucun serveur n'a répondu au test de latence : tous les serveurs activés sont gardés");
            on
        }
    }
}

/// Écrit la configuration de NgPost à utiliser pour un post, si les serveurs sont gérés ici.
/// `base` : ngPost.conf d'origine. Renvoie le chemin de la copie (dans `dir`).
/// Valeur d'une option générale de ngPost.conf (hors commentaires).
fn conf_value(text: &str, key: &str) -> Option<String> {
    text.lines().filter(|l| !l.trim_start().starts_with(['#', ';'])).find_map(|l| l.split_once('=').filter(|(k, _)| k.trim().eq_ignore_ascii_case(key)).map(|(_, v)| v.trim().to_string()))
}

pub fn write_post_conf(cfg: &Config, base: Option<&Path>, dir: &Path, on_line: &mut dyn FnMut(&str)) -> Result<Option<PathBuf>, String> {
    let text = base.and_then(|p| std::fs::read(p).ok()).map(|b| crate::ngpost::decode_text(&b)).unwrap_or_default();
    let groups = if cfg.ng_groups.trim().is_empty() { conf_value(&text, "GROUPS").unwrap_or_default() } else { cfg.ng_groups.trim().to_string() };
    let policy = group_policy(&cfg.group_policy, &groups);
    if let Some(gp) = policy {
        on_line(&format!("Politique de groupes : {} (GROUP_POLICY = {gp})", policy_label(&cfg.group_policy)));
    }
    let p = dir.join("ngPost-nzbff.conf");
    if !cfg.usenet_managed {
        // serveurs de ngPost.conf : détaillés dans le journal
        let list = parse_conf_servers(&text);
        if !list.is_empty() {
            let on: Vec<&Server> = list.iter().filter(|s| s.enabled).collect();
            let total: u32 = on.iter().filter(|s| !s.nzb_check || on.len() == 1).map(|s| s.connections).sum();
            on_line(&format!("Serveurs de ngPost.conf : {} serveur(s) activé(s), {total} connexion(s) en tout", on.len()));
            for s in &list {
                on_line(&format!("  · {}", s.describe_with(s.connections)));
            }
        }
        let Some(gp) = policy else { return Ok(None) };
        std::fs::write(&p, set_option(&text, "GROUP_POLICY", gp)).map_err(|e| format!("configuration NgPost : {e}"))?;
        return Ok(Some(p));
    }
    let servers = servers_for_post(cfg, on_line);
    if servers.is_empty() {
        return Err("aucun serveur Usenet activé (Réglages → Serveurs Usenet)".into());
    }
    let refs: Vec<&Server> = servers.iter().collect();
    let mut out = build_conf(&text, &refs);
    if let Some(gp) = policy {
        out = set_option(&out, "GROUP_POLICY", gp);
    }
    std::fs::write(&p, out).map_err(|e| format!("configuration NgPost : {e}"))?;
    let total: u32 = servers.iter().filter(|s| !s.nzb_check || servers.len() == 1).map(|s| s.connections).sum();
    on_line(&format!("Serveurs gérés par l'application : {} serveur(s), {total} connexion(s) en tout pour le post", servers.len()));
    for s in &servers {
        let role = if s.nzb_check && servers.len() > 1 { " — vérification des articles seulement (connexions non comptées)" } else { "" };
        on_line(&format!("  · {}{role}", s.describe_with(s.connections)));
    }
    Ok(Some(p))
}

// ---------------------------------------------------------------- latence

#[derive(Debug, Clone, PartialEq)]
pub struct Latency {
    /// Meilleur temps de connexion et moyenne (ms).
    pub best_ms: u32,
    pub avg_ms: u32,
    /// Message d'accueil NNTP (serveurs sans SSL).
    pub greeting: Option<String>,
}

/// Temps d'ouverture d'une connexion TCP vers le serveur (3 essais), comme une mesure de ping.
/// Sans SSL, lit en plus l'accueil du serveur (« 200 … »).
pub fn latency(host: &str, port: u16, ssl: bool) -> Result<Latency, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("serveur vide".into());
    }
    let addrs: Vec<_> = (host, port).to_socket_addrs().map_err(|e| format!("nom introuvable : {e}"))?.collect();
    let addr = *addrs.first().ok_or("nom introuvable")?;
    let mut times: Vec<u32> = Vec::new();
    let mut greeting = None;
    let mut last_err = String::new();
    for i in 0..3 {
        let t = Instant::now();
        match TcpStream::connect_timeout(&addr, Duration::from_secs(4)) {
            Ok(mut s) => {
                times.push(t.elapsed().as_millis().max(1) as u32);
                if i == 0 && !ssl {
                    let _ = s.set_read_timeout(Some(Duration::from_secs(4)));
                    let mut buf = [0u8; 256];
                    if let Ok(n) = s.read(&mut buf) {
                        let g = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").trim().to_string();
                        if !g.is_empty() {
                            greeting = Some(g);
                        }
                    }
                }
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    if times.is_empty() {
        return Err(if last_err.is_empty() { "pas de réponse".into() } else { last_err });
    }
    let best = *times.iter().min().unwrap_or(&0);
    let avg = times.iter().sum::<u32>() / times.len() as u32;
    Ok(Latency { best_ms: best, avg_ms: avg, greeting })
}

/// Teste tous les serveurs en parallèle (test complet de `secs` secondes de lecture).
pub fn test_all(servers: &[Server], secs: u32) -> Vec<(usize, Result<Speed, String>)> {
    let handles: Vec<_> = servers
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let s = s.clone();
            std::thread::spawn(move || (i, speed_test(&s, secs)))
        })
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok()).collect()
}

/// Le meilleur serveur parmi des résultats : identifiants acceptés, temps de réponse aux
/// commandes le plus court (par tranches de 10 ms), puis meilleur débit de lecture.
pub fn best_of(results: &[(usize, Result<Speed, String>)]) -> Option<usize> {
    results
        .iter()
        .filter_map(|(i, r)| r.as_ref().ok().filter(|s| s.usable()).map(|s| (*i, s)))
        .min_by(|(_, a), (_, b)| {
            (a.score_ms() / 10).cmp(&(b.score_ms() / 10)).then(b.dl_bps.unwrap_or(0.0).partial_cmp(&a.dl_bps.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal))
        })
        .map(|(i, _)| i)
}

// ------------------------------------------------------------ test complet

/// Résultat du test d'un serveur.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Speed {
    /// Ouverture de la connexion, poignée de main TLS (ms).
    pub connect_ms: u32,
    pub tls_ms: Option<u32>,
    /// Identifiants : acceptés, refusés, ou non testés (aucun identifiant).
    pub auth: Option<bool>,
    pub auth_reply: Option<String>,
    /// Temps de réponse médian et minimal du serveur à une commande (ms).
    pub rtt_ms: Option<u32>,
    pub rtt_min_ms: Option<u32>,
    /// Débit de lecture d'articles (octets/s) pendant le test.
    pub dl_bps: Option<f64>,
    pub dl_articles: u32,
    pub greeting: Option<String>,
    /// Test complet (protocole NNTP), ou connexion seulement.
    pub full: bool,
    pub note: String,
}

impl Speed {
    pub fn usable(&self) -> bool {
        self.auth != Some(false)
    }
    /// Valeur de classement (ms) : réponse aux commandes, sinon ouverture de connexion.
    pub fn score_ms(&self) -> u32 {
        self.rtt_ms.unwrap_or(self.connect_ms)
    }
}

/// Groupe lu pour le test de débit (très actif, présent chez tous les fournisseurs).
pub const TEST_GROUP: &str = "alt.binaries.boneless";

/// Test complet d'un serveur : connexion, TLS, accueil, identifiants, temps de réponse à
/// 8 commandes, puis lecture d'articles pendant `secs` secondes. Sous Windows, les serveurs SSL
/// passent par PowerShell (SslStream de Windows) ; sans SSL, test direct.
pub fn speed_test(s: &Server, secs: u32) -> Result<Speed, String> {
    if s.host.trim().is_empty() {
        return Err("serveur vide".into());
    }
    if !s.ssl {
        return native_probe(s, secs);
    }
    #[cfg(windows)]
    {
        ps_probe(s, secs)
    }
    #[cfg(not(windows))]
    {
        let l = latency(&s.host, s.port, true)?;
        Ok(Speed { connect_ms: l.best_ms, full: false, note: "connexion seulement (test complet SSL sous Windows)".into(), ..Default::default() })
    }
}

/// Lecture des lignes `clé=valeur` du test (PowerShell ou direct).
pub fn parse_probe(text: &str) -> Result<Speed, String> {
    let mut sp = Speed { full: true, ..Default::default() };
    let mut err = None;
    let mut ok = false;
    let mut dl_bytes = 0f64;
    let mut dl_ms = 0f64;
    for l in text.lines() {
        let Some((k, v)) = l.trim().split_once('=') else { continue };
        let v = v.trim();
        let n = || v.parse::<f64>().ok();
        match k.trim() {
            "connect_ms" => sp.connect_ms = n().unwrap_or(0.0) as u32,
            "tls_ms" => sp.tls_ms = n().map(|x| x as u32),
            "greeting" => sp.greeting = Some(v.to_string()),
            "auth" => sp.auth = Some(v == "ok"),
            "auth_reply" => sp.auth_reply = Some(v.to_string()),
            "rtt_ms" => sp.rtt_ms = n().map(|x| x as u32),
            "rtt_min_ms" => sp.rtt_min_ms = n().map(|x| x as u32),
            "dl_bytes" => dl_bytes = n().unwrap_or(0.0),
            "dl_ms" => dl_ms = n().unwrap_or(0.0),
            "dl_articles" => sp.dl_articles = n().unwrap_or(0.0) as u32,
            "group" => sp.note = format!("groupe de test indisponible ({v})"),
            "ok" => ok = true,
            "error" => err = Some(v.to_string()),
            _ => {}
        }
    }
    if dl_ms > 0.0 && dl_bytes > 0.0 {
        sp.dl_bps = Some(dl_bytes * 1000.0 / dl_ms);
    }
    if sp.auth == Some(false) {
        // identifiants refusés : résultat utile (serveur à corriger), pas une panne
        return Ok(sp);
    }
    match (ok, err) {
        (true, _) => Ok(sp),
        (false, Some(e)) => Err(e),
        (false, None) => Err("pas de réponse du test".into()),
    }
}

/// Test direct (sans SSL) : même protocole que le script PowerShell.
fn native_probe(s: &Server, secs: u32) -> Result<Speed, String> {
    use std::io::{BufRead, BufReader, Write};
    let mut out = String::new();
    let res: Result<(), String> = (|| {
        let addr = (s.host.trim(), s.port).to_socket_addrs().map_err(|e| format!("nom introuvable : {e}"))?.next().ok_or("nom introuvable")?;
        let t = Instant::now();
        let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(8)).map_err(|e| format!("connexion : {e}"))?;
        out.push_str(&format!("connect_ms={}\n", t.elapsed().as_millis()));
        let _ = tcp.set_read_timeout(Some(Duration::from_secs(10)));
        let mut w = tcp.try_clone().map_err(|e| e.to_string())?;
        let mut r = BufReader::new(tcp);
        let mut line = String::new();
        let read = |r: &mut BufReader<TcpStream>, line: &mut String| -> Result<String, String> {
            line.clear();
            let n = r.read_line(line).map_err(|e| format!("lecture : {e}"))?;
            if n == 0 {
                return Err("connexion fermée par le serveur".into());
            }
            Ok(line.trim_end_matches(['\r', '\n']).to_string())
        };
        let t = Instant::now();
        let g = read(&mut r, &mut line)?;
        out.push_str(&format!("greeting_ms={}\ngreeting={g}\n", t.elapsed().as_millis()));
        if !g.starts_with("20") {
            return Err(format!("accueil refusé : {g}"));
        }
        if !s.user.trim().is_empty() {
            write!(w, "AUTHINFO USER {}\r\n", s.user.trim()).map_err(|e| e.to_string())?;
            let mut a = read(&mut r, &mut line)?;
            if a.starts_with("381") {
                write!(w, "AUTHINFO PASS {}\r\n", s.pass).map_err(|e| e.to_string())?;
                a = read(&mut r, &mut line)?;
            }
            if !a.starts_with("281") {
                out.push_str(&format!("auth=ko\nauth_reply={a}\n"));
                return Ok(());
            }
            out.push_str("auth=ok\n");
        }
        let mut rtts: Vec<u128> = Vec::new();
        for _ in 0..8 {
            let t = Instant::now();
            w.write_all(b"DATE\r\n").map_err(|e| e.to_string())?;
            read(&mut r, &mut line)?;
            rtts.push(t.elapsed().as_millis());
        }
        rtts.sort();
        out.push_str(&format!("rtt_ms={}\nrtt_min_ms={}\n", rtts[rtts.len() / 2], rtts[0]));
        if secs > 0 {
            write!(w, "GROUP {TEST_GROUP}\r\n").map_err(|e| e.to_string())?;
            let gr = read(&mut r, &mut line)?;
            let last: i64 = gr.split_whitespace().nth(3).and_then(|x| x.parse().ok()).unwrap_or(0);
            if gr.starts_with("211") && last > 0 {
                let t = Instant::now();
                let (mut bytes, mut n, mut id) = (0u64, 0u32, last);
                while t.elapsed() < Duration::from_secs(secs as u64) && n < 400 && id > 0 {
                    write!(w, "BODY {id}\r\n").map_err(|e| e.to_string())?;
                    id -= 1;
                    let b = read(&mut r, &mut line)?;
                    if b.starts_with("222") {
                        loop {
                            let l = read(&mut r, &mut line)?;
                            if l == "." {
                                break;
                            }
                            bytes += l.len() as u64 + 2;
                        }
                        n += 1;
                    } else if !b.starts_with('4') {
                        break;
                    }
                }
                out.push_str(&format!("dl_bytes={bytes}\ndl_ms={}\ndl_articles={n}\n", t.elapsed().as_millis().max(1)));
            } else {
                out.push_str(&format!("group={gr}\n"));
            }
        }
        let _ = w.write_all(b"QUIT\r\n");
        out.push_str("ok=1\n");
        Ok(())
    })();
    if let Err(e) = res {
        out.push_str(&format!("error={e}\n"));
    }
    parse_probe(&out)
}

/// Script PowerShell du test SSL (SslStream de Windows : rien à installer). Les identifiants
/// passent par l'environnement, jamais par la ligne de commande.
#[cfg_attr(not(windows), allow(dead_code))]
pub const PS_PROBE: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
function Out($k, $v) { [Console]::Out.WriteLine("$k=$v") }
$sw = [Diagnostics.Stopwatch]::StartNew()
function Ms($t0) { [int]($sw.Elapsed.TotalMilliseconds - $t0) }
try {
  $h = $env:NZBFF_HOST; $p = [int]$env:NZBFF_PORT; $u = $env:NZBFF_USER; $pw = $env:NZBFF_PASS
  $grp = $env:NZBFF_GROUP; $secs = [int]$env:NZBFF_SECS
  $tcp = New-Object System.Net.Sockets.TcpClient
  $t0 = $sw.Elapsed.TotalMilliseconds
  $ar = $tcp.BeginConnect($h, $p, $null, $null)
  if (-not $ar.AsyncWaitHandle.WaitOne(8000)) { throw "connexion : délai dépassé" }
  $tcp.EndConnect($ar)
  Out 'connect_ms' (Ms $t0)
  $tcp.ReceiveTimeout = 10000; $tcp.SendTimeout = 10000
  $t0 = $sw.Elapsed.TotalMilliseconds
  $ssl = New-Object System.Net.Security.SslStream($tcp.GetStream(), $false)
  $ssl.ReadTimeout = 10000; $ssl.WriteTimeout = 10000
  $ssl.AuthenticateAsClient($h)
  Out 'tls_ms' (Ms $t0)
  $enc = [System.Text.Encoding]::GetEncoding(28591)
  $rd = New-Object System.IO.StreamReader($ssl, $enc)
  $wr = New-Object System.IO.StreamWriter($ssl, $enc)
  $wr.NewLine = "`r`n"; $wr.AutoFlush = $true
  function ReadL { $l = $rd.ReadLine(); if ($null -eq $l) { throw "connexion fermée par le serveur" }; return $l }
  $t0 = $sw.Elapsed.TotalMilliseconds
  $g = ReadL
  Out 'greeting_ms' (Ms $t0); Out 'greeting' $g
  if (-not $g.StartsWith('20')) { throw "accueil refusé : $g" }
  if ($u) {
    $wr.WriteLine("AUTHINFO USER $u"); $a = ReadL
    if ($a.StartsWith('381')) { $wr.WriteLine("AUTHINFO PASS $pw"); $a = ReadL }
    if (-not $a.StartsWith('281')) { Out 'auth' 'ko'; Out 'auth_reply' $a; exit 0 }
    Out 'auth' 'ok'
  }
  $rtts = @()
  for ($i = 0; $i -lt 8; $i++) { $t0 = $sw.Elapsed.TotalMilliseconds; $wr.WriteLine('DATE'); $null = ReadL; $rtts += ($sw.Elapsed.TotalMilliseconds - $t0) }
  $s = @($rtts | Sort-Object)
  Out 'rtt_ms' ([int]$s[[int][Math]::Floor($s.Count / 2)]); Out 'rtt_min_ms' ([int]$s[0])
  if ($secs -gt 0 -and $grp) {
    $wr.WriteLine("GROUP $grp"); $gr = ReadL
    $parts = $gr.Split(' ')
    if ($gr.StartsWith('211') -and $parts.Count -ge 4) {
      $id = [int64]$parts[3]; $bytes = [int64]0; $n = 0
      $t0 = $sw.Elapsed.TotalMilliseconds
      while ((($sw.Elapsed.TotalMilliseconds - $t0) -lt ($secs * 1000)) -and $n -lt 400 -and $id -gt 0) {
        $wr.WriteLine("BODY $id"); $id--
        $b = ReadL
        if ($b.StartsWith('222')) {
          while ($true) { $l = ReadL; if ($l -eq '.') { break }; $bytes += $l.Length + 2 }
          $n++
        } elseif (-not $b.StartsWith('4')) { break }
      }
      Out 'dl_bytes' $bytes; Out 'dl_ms' ([Math]::Max(1, (Ms $t0))); Out 'dl_articles' $n
    } else { Out 'group' $gr }
  }
  try { $wr.WriteLine('QUIT') } catch {}
  Out 'ok' '1'
} catch {
  Out 'error' (($_.Exception.Message) -replace "[`r`n]", ' ')
}
"#;

#[cfg(windows)]
fn ps_probe(s: &Server, secs: u32) -> Result<Speed, String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let dir = crate::app_dir().join("tmp");
    let _ = std::fs::create_dir_all(&dir);
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let script = dir.join(format!("test-serveur-{}-{}.ps1", std::process::id(), SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    // BOM UTF-8 : accents des messages lus correctement par Windows PowerShell 5
    let mut body = vec![0xEF, 0xBB, 0xBF];
    body.extend_from_slice(PS_PROBE.as_bytes());
    std::fs::write(&script, body).map_err(|e| format!("script de test : {e}"))?;
    let child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .env("NZBFF_HOST", s.host.trim())
        .env("NZBFF_PORT", s.port.to_string())
        .env("NZBFF_USER", s.user.trim())
        .env("NZBFF_PASS", &s.pass)
        .env("NZBFF_GROUP", TEST_GROUP)
        .env("NZBFF_SECS", secs.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000)
        .spawn();
    let res = match child {
        Ok(c) => {
            // délai maximal : le test ne doit jamais bloquer un post
            let (tx, rx) = std::sync::mpsc::channel();
            let pid = c.id();
            std::thread::spawn(move || {
                let _ = tx.send(c.wait_with_output());
            });
            match rx.recv_timeout(Duration::from_secs(secs as u64 + 45)) {
                Ok(Ok(o)) => {
                    let out = String::from_utf8_lossy(&o.stdout).to_string();
                    if out.contains('=') {
                        parse_probe(&out)
                    } else {
                        // rien en sortie (stratégie de sécurité, PowerShell bloqué…) : test simple
                        let err = String::from_utf8_lossy(&o.stderr).lines().find(|l| !l.trim().is_empty()).unwrap_or("aucune sortie").trim().to_string();
                        Err(format!("PowerShell : {err}"))
                    }
                }
                Ok(Err(e)) => Err(format!("PowerShell : {e}")),
                Err(_) => {
                    let pid = pid.to_string();
                    let _ = Command::new("taskkill").args(["/PID", pid.as_str(), "/T", "/F"]).creation_flags(0x0800_0000).output();
                    Err("test trop long (pas de réponse du serveur)".into())
                }
            }
        }
        Err(e) => Err(format!("PowerShell introuvable ({e})")),
    };
    let _ = std::fs::remove_file(&script);
    // PowerShell indisponible : on se rabat sur la mesure de connexion
    match res {
        Err(e) if e.starts_with("PowerShell") => {
            let l = latency(&s.host, s.port, true)?;
            Ok(Speed { connect_ms: l.best_ms, full: false, note: format!("connexion seulement ({e})"), ..Default::default() })
        }
        r => r,
    }
}

/// Résumé lisible d'un test.
pub fn describe(sp: &Speed) -> String {
    let mut v = Vec::new();
    match sp.rtt_ms {
        Some(r) => v.push(format!("réponse {r} ms")),
        None => v.push(format!("connexion {} ms", sp.connect_ms)),
    }
    if let Some(b) = sp.dl_bps {
        v.push(format!("lecture {:.1} Mo/s", b / 1_048_576.0));
    }
    if sp.auth == Some(true) {
        v.push("identifiants OK".into());
    }
    if !sp.note.is_empty() {
        v.push(sp.note.clone());
    }
    v.join(", ")
}

/// Résultat complet d'un test, pour le journal détaillé.
pub fn describe_full(sp: &Speed) -> String {
    let mut v = vec![format!("connexion {} ms", sp.connect_ms)];
    if let Some(t) = sp.tls_ms {
        v.push(format!("TLS {t} ms"));
    }
    match (sp.rtt_ms, sp.rtt_min_ms) {
        (Some(r), Some(m)) => v.push(format!("réponse aux commandes {r} ms (médiane), {m} ms (meilleure)")),
        (Some(r), None) => v.push(format!("réponse aux commandes {r} ms")),
        _ => {}
    }
    if let Some(b) = sp.dl_bps {
        v.push(format!("lecture {:.1} Mo/s sur {} article(s)", b / 1_048_576.0, sp.dl_articles));
    }
    v.push(match sp.auth {
        Some(true) => "identifiants acceptés".into(),
        Some(false) => "identifiants refusés".into(),
        None => "identifiants non testés".into(),
    });
    if let Some(g) = &sp.greeting {
        v.push(format!("accueil « {} »", g.chars().take(80).collect::<String>()));
    }
    if !sp.full {
        v.push("test partiel".into());
    }
    if !sp.note.is_empty() {
        v.push(sp.note.clone());
    }
    format!("{} → classé {} ms", v.join(", "), sp.score_ms())
}

/// Ports courants (pour l'aide de saisie).
pub fn usual_ports(ssl: bool) -> &'static str {
    if ssl {
        "563 ou 443 (SSL)"
    } else {
        "119 ou 80 (sans SSL)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONF: &str = "## ngPost\nnzbPath = C:/nzb\nPACK = COMPRESS, GEN_PAR2\n\n[server]\nhost = news.newshosting.com\nport = 563\nssl = true\nuser = moi\npass = s3cret\nconnection = 30\nenabled = true\nnzbCheck = false\n\n[server]\nhost = eu.news.eweka.nl\nport = 119\nssl = false\nuser = moi2\npass = p\nconnection = 10\nenabled = false\n\nGROUPS = alt.binaries.test\n";

    #[test]
    fn import_et_conf_generee() {
        let s = parse_conf_servers(CONF);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].provider.as_str(), s[0].host.as_str(), s[0].port, s[0].ssl, s[0].connections, s[0].enabled), ("Newshosting", "news.newshosting.com", 563, true, 30, true));
        assert_eq!((s[1].provider.as_str(), s[1].port, s[1].ssl, s[1].enabled), ("Eweka", 119, false, false));
        assert_eq!(provider_of("news.usenet.co.uk"), "Usenet");
        // la copie garde les options, remplace les serveurs
        let stripped = strip_servers(CONF);
        assert!(stripped.contains("nzbPath = C:/nzb") && stripped.contains("GROUPS = alt.binaries.test") && !stripped.contains("host =") && !stripped.contains("s3cret"), "{stripped}");
        let mine = Server { provider: "X".into(), host: "news.x.com".into(), port: 443, connections: 50, ..Default::default() };
        let built = build_conf(CONF, &[&mine]);
        assert!(built.contains("GROUPS = alt.binaries.test") && built.contains("[server]\nhost = news.x.com\nport = 443\nssl = true") && built.contains("connection = 50"));
        assert_eq!(built.matches("[server]").count(), 1);
        assert_eq!(parse_conf_servers(&built), vec![mine.clone()]);
        // ligne de nzbff.conf
        let l = mine.line();
        assert_eq!(Server::parse_line(l.trim_end().strip_prefix("usenet_server=").unwrap()), Some(mine));
    }

    #[test]
    fn ngpost_conf_abime_repare_et_stable() {
        // fichier abîmé par les anciennes versions : titres empilés, serveurs à la fin
        let damaged = "## ngPost configuration\nnzbPath = C:/nzb\n\n#####################\n# servers section ####\n#####################\n\n\n\n## Serveurs Usenet (écrits par NZB FF Tool Pro)\n\n# servers section ####\n\n## Serveurs Usenet (écrits par NZB FF Tool Pro)\n## Serveurs Usenet gérés par NZB FF Tool Pro\n[server]\nhost = a.example.com\nport = 563\nssl = true\nuser = u\npass = p\nconnection = 10\nenabled = true\nnzbCheck = false\n\n## posting section\nGROUPS = alt.binaries.test\n#GROUP_POLICY = EACH_FILE\n\n## Serveurs Usenet (écrits par NZB FF Tool Pro)\n[server]\nhost = b.example.com\nport = 119\nssl = false\nuser = u\npass = p\nconnection = 5\nenabled = true\nnzbCheck = false\n";
        let dir = std::env::temp_dir().join(format!("nzbff-conf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("ngPost.conf");
        std::fs::write(&conf, damaged).unwrap();
        let sv = vec![
            Server { provider: "A".into(), host: "a.example.com".into(), connections: 33, connections_solo: 100, ..Default::default() },
            Server { provider: "A".into(), host: "a2.example.com".into(), connections: 34, ..Default::default() },
        ];
        let opts = [("GROUPS", "alt.binaries.movies,alt.binaries.misc".to_string()), ("GROUP_POLICY", "EACH_POST".to_string())];
        write_to_conf(&conf, &sv, &opts).unwrap();
        let once = std::fs::read_to_string(&conf).unwrap().replace("\r\n", "\n");
        assert_eq!(once.matches("Serveurs Usenet").count(), 1, "{once}");
        assert_eq!(once.matches("servers section").count(), 1, "{once}");
        assert!(!once.contains("\n\n\n"), "{once}");
        assert_eq!(once.matches("[server]").count(), 2, "{once}");
        assert!(once.contains("## posting section") && once.contains("GROUPS = alt.binaries.movies,alt.binaries.misc") && once.contains("#GROUP_POLICY = EACH_FILE\nGROUP_POLICY = EACH_POST"), "{once}");
        // nos serveurs à la place des anciens (après le titre de ngPost), pas à la fin
        let (srv, post) = (once.find("host = a.example.com").unwrap(), once.find("## posting section").unwrap());
        assert!(once.find("servers section").unwrap() < srv && srv < post, "{once}");
        assert_eq!(once.matches("#####################").count(), 2, "{once}");
        // réécrire ne change plus rien
        write_to_conf(&conf, &sv, &opts).unwrap();
        assert_eq!(std::fs::read_to_string(&conf).unwrap().replace("\r\n", "\n"), once);
        // connexions « seul » gardées dans nzbff.conf
        let l = sv[0].line();
        let back = Server::parse_line(l.trim_end().strip_prefix("usenet_server=").unwrap()).unwrap();
        assert_eq!((back.connections, back.solo()), (33, 100));
        assert_eq!(sv[1].solo(), 34);
        assert!(sv[0].describe_with(100).contains("a.example.com:563 SSL · 100 connexion(s)"));
        assert!(!sv[0].describe_with(1).contains("p\n"));
        // jamais le mot de passe, identifiant à moitié masqué
        let s = Server { host: "news.x.com".into(), user: "monlogin".into(), pass: "Tr3sSecret".into(), ..Default::default() };
        let d = s.describe_with(20);
        assert!(!d.contains("Tr3sSecret") && !d.contains("monlogin") && d.contains("mon••••") && d.contains("mot de passe enregistré (masqué)"), "{d}");
        // politique de groupes
        assert_eq!(group_policy("auto", "a,b,c"), Some("EACH_FILE"));
        assert_eq!(group_policy("auto", "a"), Some("ALL"));
        assert_eq!(group_policy("", "a,b"), None);
        assert_eq!(conf_group_policy("each_post", "a,b"), Some("EACH_POST"));
        assert!(["a", "b"].contains(&pick_group("a, b").as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn latence_locale_et_choix_du_plus_rapide() {
        use std::io::Write;
        // faux serveur NNTP local : accueil, identifiants, DATE, GROUP, BODY, QUIT
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            for c in l.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut w = c.try_clone().unwrap();
                    let _ = w.write_all(b"200 news.local ready\r\n");
                    for line in BufReader::new(c).lines() {
                        let Ok(line) = line else { return };
                        let rep: String = if line.starts_with("AUTHINFO USER") {
                            "381 PASS required\r\n".into()
                        } else if line.starts_with("AUTHINFO PASS") {
                            if line.ends_with("secret") { "281 Ok\r\n".into() } else { "481 Authentication failed\r\n".into() }
                        } else if line == "DATE" {
                            "111 20261103120000\r\n".into()
                        } else if line.starts_with("GROUP") {
                            "211 3 1 3 alt.binaries.boneless\r\n".into()
                        } else if line.starts_with("BODY 2") {
                            "423 No such article\r\n".into()
                        } else if line.starts_with("BODY") {
                            format!("222 0 <a@b>\r\n{}\r\n..ligne\r\n.\r\n", "y".repeat(1000))
                        } else if line == "QUIT" {
                            let _ = w.write_all(b"205 bye\r\n");
                            return;
                        } else {
                            "500 ?\r\n".into()
                        };
                        if w.write_all(rep.as_bytes()).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        // test complet : identifiants, temps de réponse, lecture d'articles
        let ok = Server { host: "127.0.0.1".into(), port, ssl: false, user: "moi".into(), pass: "secret".into(), ..Default::default() };
        let sp = speed_test(&ok, 1).unwrap();
        assert!(sp.full && sp.auth == Some(true) && sp.rtt_ms.is_some() && sp.dl_articles == 2 && sp.dl_bps.unwrap() > 0.0, "{sp:?}");
        assert!(describe(&sp).contains("identifiants OK"));
        // mauvais mot de passe : signalé, jamais retenu comme le plus rapide
        let bad = Server { pass: "faux".into(), ..ok.clone() };
        let sb = speed_test(&bad, 1).unwrap();
        assert_eq!(sb.auth, Some(false));
        assert!(sb.auth_reply.as_deref().unwrap_or("").contains("481"));
        assert_eq!(best_of(&[(0, Ok(sb)), (1, Ok(sp.clone()))]), Some(1));
        // sortie du script PowerShell
        let ps = parse_probe("connect_ms=21\ntls_ms=35\ngreeting=200 hi\nauth=ok\nrtt_ms=18\nrtt_min_ms=15\ndl_bytes=10485760\ndl_ms=2000\ndl_articles=14\nok=1\n").unwrap();
        assert_eq!((ps.tls_ms, ps.rtt_ms), (Some(35), Some(18)));
        assert!((ps.dl_bps.unwrap() - 5_242_880.0).abs() < 1.0);
        assert_eq!(parse_probe("connect_ms=3\nerror=accueil refusé : 502 non\n").unwrap_err(), "accueil refusé : 502 non");
        // le script PowerShell suit le même protocole ; le mot de passe n'est jamais affiché
        for k in ["AUTHINFO USER $u", "AUTHINFO PASS $pw", "'DATE'", "GROUP $grp", "BODY $id", "Out 'auth' 'ok'", "Out 'rtt_ms'", "Out 'dl_bytes'", "Out 'error'", "$env:NZBFF_PASS"] {
            assert!(PS_PROBE.contains(k), "{k}");
        }
        assert!(PS_PROBE.lines().filter(|l| l.contains("Out ")).all(|l| !l.contains("$pw")), "le mot de passe ne doit jamais être écrit en sortie");
        let r = latency("127.0.0.1", port, false).unwrap();
        assert_eq!(r.greeting.as_deref(), Some("200 news.local ready"));
        assert!(r.best_ms <= r.avg_ms);
        // un port fermé : injoignable
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(latency("127.0.0.1", closed, true).is_err());
        let cfg = Config {
            usenet_managed: true,
            usenet_auto_best: true,
            usenet_servers: vec![
                Server { provider: "Mort".into(), host: "127.0.0.1".into(), port: closed, ..Default::default() },
                Server { provider: "Local".into(), host: "127.0.0.1".into(), port, ssl: false, connections: 8, user: "moi".into(), pass: "secret".into(), ..Default::default() },
            ],
            ..Default::default()
        };
        let dir = std::env::temp_dir().join(format!("nzbff-usenet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("ngPost.conf");
        std::fs::write(&base, CONF).unwrap();
        let mut log = Vec::new();
        let p = write_post_conf(&cfg, Some(&base), &dir, &mut |l| log.push(l.to_string())).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(text.matches("[server]").count(), 1, "{text}");
        assert!(text.contains(&format!("port = {port}")) && text.contains("PACK = COMPRESS") && !text.contains("newshosting"));
        assert!(log.iter().any(|l| l.contains("plus rapide retenu (") && l.contains(": Local · 127.0.0.1")), "{log:?}");
        assert!(!log.iter().any(|l| l.contains("s3cret")));
        assert!(log.iter().any(|l| l.starts_with("Choix des serveurs : test des")), "{log:?}");
        // journal détaillé : mesures complètes de chaque test, mot de passe jamais écrit
        let detail = Config { log_detail: true, ..cfg.clone() };
        let mut log2 = Vec::new();
        write_post_conf(&detail, Some(&base), &dir, &mut |l| log2.push(l.to_string())).unwrap();
        assert!(log2.iter().any(|l| l.contains("→ classé") && l.contains("connexion")), "{log2:?}");
        assert!(!log2.iter().any(|l| l.contains("s3cret")));
        // écriture dans ngPost.conf lui-même : options gardées, serveurs remplacés, copie de l'ancien
        let n = write_to_conf(&base, &cfg.usenet_servers, &[]).unwrap();
        assert_eq!(n, 2);
        let conf = std::fs::read_to_string(&base).unwrap();
        assert!(conf.contains("PACK = COMPRESS") && conf.contains("GROUPS = alt.binaries.test") && !conf.contains("newshosting"), "{conf}");
        let back = parse_conf_servers(&conf);
        assert_eq!(back.len(), 2);
        assert!(back.iter().all(|s| s.enabled) && back.iter().any(|s| s.port == closed));
        assert!(std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains(".nzbff-") && std::fs::read_to_string(e.path()).unwrap().contains("newshosting")), "copie de l'ancien fichier");
        // non géré : rien n'est écrit
        let off = Config { usenet_managed: false, ..cfg.clone() };
        assert_eq!(write_post_conf(&off, Some(&base), &dir, &mut |_| {}).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
