//! Statistiques des posts Usenet : un enregistrement par post réussi (nzbff-posts.tsv)
//! et des échantillons de débit pris pendant les posts (nzbff-speed.tsv, 31 jours gardés).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const POSTS_FILE: &str = "nzbff-posts.tsv";
pub const SPEED_FILE: &str = "nzbff-speed.tsv";
/// Échantillons de débit gardés (secondes).
const KEEP_SPEED: u64 = 31 * 86_400;

/// Un post réussi.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostRec {
    pub start: u64,
    pub end: u64,
    pub release: String,
    /// Octets postés (somme des articles du NZB).
    pub bytes: u64,
    pub articles: u64,
    /// Débit moyen (octets/s).
    pub avg_bps: f64,
    /// Connexions utilisées (0 = inconnu).
    pub connections: u32,
    /// Indexers visés.
    pub indexers: String,
}

impl PostRec {
    fn to_line(&self) -> String {
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        format!(
            "{}\t{}\t{}\t{}\t{}\t{:.0}\t{}\t{}\n",
            self.start,
            self.end,
            clean(&self.release),
            self.bytes,
            self.articles,
            self.avg_bps,
            self.connections,
            clean(&self.indexers)
        )
    }
    fn parse(l: &str) -> Option<PostRec> {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 7 {
            return None;
        }
        Some(PostRec {
            start: f[0].parse().ok()?,
            end: f[1].parse().ok()?,
            release: f[2].to_string(),
            bytes: f[3].parse().ok()?,
            articles: f[4].parse().unwrap_or(0),
            avg_bps: f[5].parse().unwrap_or(0.0),
            connections: f[6].parse().unwrap_or(0),
            indexers: f.get(7).map(|s| s.to_string()).unwrap_or_default(),
        })
    }
    pub fn duration(&self) -> u64 {
        self.end.saturating_sub(self.start).max(1)
    }
    /// Temps d'envoi seul (secondes) : d'après le débit moyen annoncé par NgPost, qui ne
    /// compte que l'envoi — pas la compression, le par2 ni la vérification du NZB.
    pub fn upload_secs(&self) -> f64 {
        if self.avg_bps > 0.0 && self.bytes > 0 {
            (self.bytes as f64 / self.avg_bps).min(self.duration() as f64).max(0.001)
        } else {
            self.duration() as f64
        }
    }
}

/// Une case d'un graphique (heure ou jour).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bucket {
    pub label: String,
    pub start: u64,
    pub bytes: u64,
    pub posts: usize,
    /// Débit moyen pendant les posts de cette case (octets/s, pondéré par la durée).
    pub avg_bps: f64,
}

/// Totaux d'une période.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Totals {
    pub bytes: u64,
    pub posts: usize,
    pub articles: u64,
    pub avg_bps: f64,
    pub max_bps: f64,
    pub avg_connections: f64,
    /// Temps passé à poster (secondes).
    pub busy: u64,
}

#[derive(Clone, Debug, Default)]
pub struct PostStats {
    dir: PathBuf,
    pub posts: Vec<PostRec>,
    /// (instant, débit en octets/s)
    pub speed: Vec<(u64, f64)>,
    /// Change à chaque ajout (pour les caches de l'interface).
    pub revision: u64,
}

impl PostStats {
    pub fn load(dir: &Path) -> PostStats {
        let posts = fs::read_to_string(dir.join(POSTS_FILE)).map(|t| t.lines().filter_map(PostRec::parse).collect()).unwrap_or_default();
        let now = crate::prep::now_secs();
        let raw = fs::read_to_string(dir.join(SPEED_FILE)).unwrap_or_default();
        let speed: Vec<(u64, f64)> = raw
            .lines()
            .filter_map(|l| {
                let (a, b) = l.split_once('\t')?;
                Some((a.parse().ok()?, b.trim().parse().ok()?))
            })
            .filter(|(t, _): &(u64, f64)| now.saturating_sub(*t) <= KEEP_SPEED)
            .collect();
        // fichier compacté : les échantillons trop vieux disparaissent
        if raw.lines().count() > speed.len() + 500 {
            let s: String = speed.iter().map(|(t, b)| format!("{t}\t{b:.0}\n")).collect();
            let _ = fs::write(dir.join(SPEED_FILE), s);
        }
        PostStats { dir: dir.to_path_buf(), posts, speed, revision: 1 }
    }

    /// Vide les statistiques des posts et du débit (copies .bak conservées).
    pub fn clear(&mut self) -> std::io::Result<()> {
        for f in [POSTS_FILE, SPEED_FILE] {
            let p = self.dir.join(f);
            if p.exists() {
                fs::copy(&p, self.dir.join(format!("{f}.bak")))?;
                fs::write(&p, "")?;
            }
        }
        self.posts.clear();
        self.speed.clear();
        self.revision += 1;
        Ok(())
    }

    fn append(&self, file: &str, line: &str) {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(self.dir.join(file)) {
            let _ = f.write_all(line.as_bytes());
        }
    }

    pub fn push_post(&mut self, r: PostRec) {
        self.append(POSTS_FILE, &r.to_line());
        self.posts.push(r);
        self.revision += 1;
    }

    pub fn push_speed(&mut self, samples: &[(u64, f64)]) {
        if samples.is_empty() {
            return;
        }
        let s: String = samples.iter().map(|(t, b)| format!("{t}\t{b:.0}\n")).collect();
        self.append(SPEED_FILE, &s);
        self.speed.extend_from_slice(samples);
        self.revision += 1;
    }

    pub fn totals(&self, since: u64) -> Totals {
        let mut t = Totals::default();
        let (mut conn_sum, mut conn_n) = (0.0, 0.0);
        let mut upload = 0.0f64;
        for p in self.posts.iter().filter(|p| p.end >= since) {
            t.bytes += p.bytes;
            t.posts += 1;
            t.articles += p.articles;
            t.busy += p.duration();
            upload += p.upload_secs();
            if p.connections > 0 {
                conn_sum += p.connections as f64;
                conn_n += 1.0;
            }
            t.max_bps = t.max_bps.max(p.avg_bps);
        }
        // débit d'envoi : octets / temps d'envoi (comme NgPost), pas / durée totale du post
        if upload > 0.0 {
            t.avg_bps = t.bytes as f64 / upload;
        }
        for (_, b) in self.speed.iter().filter(|(ts, _)| *ts >= since) {
            t.max_bps = t.max_bps.max(*b);
        }
        if conn_n > 0.0 {
            t.avg_connections = conn_sum / conn_n;
        }
        t
    }

    /// `n` cases de `step` secondes finissant maintenant (24 × 1 h, 7 × 1 jour…).
    /// Un post est compté dans la case de sa fin.
    pub fn buckets(&self, now: u64, n: usize, step: u64) -> Vec<Bucket> {
        // cases alignées sur l'heure / le jour local
        let local_offset = local_offset(now);
        let aligned_end = ((now as i64 + local_offset) / step as i64 + 1) * step as i64 - local_offset;
        let first = aligned_end - (n as i64) * step as i64;
        let mut out: Vec<Bucket> = (0..n)
            .map(|k| {
                let start = (first + k as i64 * step as i64).max(0) as u64;
                let l = crate::history::local_time(start);
                let label = if step >= 86_400 { format!("{:02}/{:02}", l.day, l.month) } else { format!("{:02}h", l.hour) };
                Bucket { label, start, ..Default::default() }
            })
            .collect();
        let mut busy = vec![0f64; n];
        for p in &self.posts {
            let e = p.end as i64;
            if e < first || e >= aligned_end {
                continue;
            }
            let k = ((e - first) / step as i64) as usize;
            out[k].bytes += p.bytes;
            out[k].posts += 1;
            busy[k] += p.upload_secs();
        }
        for (b, s) in out.iter_mut().zip(busy) {
            if s > 0.0 {
                b.avg_bps = b.bytes as f64 / s;
            }
        }
        out
    }

    /// Débit lissé en `n` tranches régulières de `since` à `now` : moyenne des mesures de chaque
    /// tranche (0 quand rien n'était posté). Renvoie (milieu de la tranche, octets/s).
    /// `extra` : mesures pas encore enregistrées (post en cours).
    pub fn speed_bins(&self, since: u64, now: u64, n: usize, extra: &[(u64, f64)]) -> Vec<(f64, f64)> {
        let n = n.max(1);
        let span = now.saturating_sub(since).max(1) as f64;
        let w = span / n as f64;
        let mut sum = vec![0.0f64; n];
        let mut cnt = vec![0u32; n];
        for (t, b) in self.speed.iter().chain(extra.iter()).filter(|(t, _)| *t >= since && *t <= now) {
            let k = (((*t - since) as f64 / w) as usize).min(n - 1);
            sum[k] += b;
            cnt[k] += 1;
        }
        (0..n).map(|k| (since as f64 + w * (k as f64 + 0.5), if cnt[k] > 0 { sum[k] / cnt[k] as f64 } else { 0.0 })).collect()
    }

}

/// Jours depuis le 1970-01-01 pour une date civile.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Décalage de l'heure locale par rapport à UTC (secondes).
pub fn local_offset(ts: u64) -> i64 {
    let l = crate::history::local_time(ts);
    let secs = days_from_civil(l.year, l.month, l.day) * 86_400 + (l.hour * 3600 + l.minute * 60 + l.second) as i64;
    secs - ts as i64
}

/// « (100 connexions sur 8 threads) » / « (20 connections on 8 threads) » → 100 / 20.
pub fn parse_connections(line: &str) -> Option<u32> {
    let l = line.to_lowercase();
    let k = l.find("connexion").or_else(|| l.find("connection"))?;
    let before = l[..k].trim_end();
    let digits: String = before.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jours() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 11, 1), 20_758);
        assert!(local_offset(1_793_563_000).abs() <= 14 * 3600);
    }

    #[test]
    fn connexions() {
        assert_eq!(parse_connections("=> débit moyen:  32.76 MB/s (100 connexions sur 8 threads)"), Some(100));
        assert_eq!(parse_connections("average speed: 51.9 MB/s (20 connections on 8 threads)"), Some(20));
        assert_eq!(parse_connections("Closing connection..."), None);
    }

    #[test]
    fn historique_et_cases() {
        let d = std::env::temp_dir().join(format!("nzbff_stats_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let now = crate::prep::now_secs();
        let mut s = PostStats::load(&d);
        s.push_post(PostRec { start: now - 200, end: now - 100, release: "A\tB".into(), bytes: 1_000_000, articles: 2, avg_bps: 10_000.0, connections: 100, indexers: "X".into() });
        s.push_post(PostRec { start: now - 3 * 86_400, end: now - 3 * 86_400 + 50, release: "C".into(), bytes: 500_000, articles: 1, avg_bps: 10_000.0, connections: 20, indexers: String::new() });
        s.push_speed(&[(now - 150, 12_000.0), (now - 9 * 86_400, 1.0), (now - 40 * 86_400, 1.0)]);
        let s = PostStats::load(&d);
        assert_eq!(s.posts.len(), 2);
        assert_eq!(s.posts[0].release, "A B");
        assert_eq!(s.speed.len(), 2); // gardés 31 jours
        let bins = s.speed_bins(now - 86_400, now, 96, &[]);
        assert_eq!(bins.len(), 96);
        assert_eq!(bins.iter().filter(|b| b.1 > 0.0).count(), 1);
        assert!(bins.iter().any(|b| (b.1 - 12_000.0).abs() < 1e-6));
        let t24 = s.totals(now - 86_400);
        assert_eq!((t24.posts, t24.bytes), (1, 1_000_000));
        assert_eq!(t24.avg_bps, 10_000.0);
        assert_eq!(t24.max_bps, 12_000.0);
        let t7 = s.totals(now - 7 * 86_400);
        assert_eq!(t7.posts, 2);
        assert_eq!(t7.avg_connections, 60.0);
        let h = s.buckets(now, 24, 3600);
        assert_eq!(h.len(), 24);
        assert_eq!(h.iter().map(|b| b.bytes).sum::<u64>(), 1_000_000);
        let j = s.buckets(now, 7, 86_400);
        assert_eq!(j.iter().map(|b| b.posts).sum::<usize>(), 2);
        assert!(j.last().unwrap().label.contains('/'));
        let _ = fs::remove_dir_all(&d);
    }

    /// Débit moyen = celui de l'envoi (annoncé par NgPost), pas volume / durée totale du post
    /// (compression et par2 compris), qui le divisait par deux ou plus.
    #[test]
    fn debit_moyen_de_l_envoi_seul() {
        let mo = 1024.0 * 1024.0;
        let st = PostStats {
            posts: vec![
                // 30 Go envoyés à 55 Mo/s, mais 20 min de compression + par2 avant
                PostRec { start: 0, end: 559 + 1200, bytes: (30.0 * 1024.0 * mo) as u64, avg_bps: 55.0 * mo, ..Default::default() },
                PostRec { start: 5000, end: 5100, bytes: (4500.0 * mo) as u64, avg_bps: 45.0 * mo, ..Default::default() },
            ],
            ..Default::default()
        };
        let t = st.totals(0);
        let v = t.avg_bps / mo;
        assert!((53.0..55.0).contains(&v), "{v}");
        // ancien enregistrement sans débit : durée totale
        let r = PostRec { start: 0, end: 100, bytes: 1000, ..Default::default() };
        assert_eq!(r.upload_secs(), 100.0);
    }
}