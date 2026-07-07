//! The diary's memory. Every finished turn is kept — the writer's actual pen
//! strokes, a transcription of their words, and Tom's reply — so a later
//! incantation ("show me what I wrote about the garden") can conjure the page
//! back in the writer's own hand.
//!
//! Everything lives on the tablet, in plain files under
//! `/home/root/riddle-data/memories` (override: `RIDDLE_MEMORY_DIR`):
//!
//!   index.tsv        one line per memory: id \t transcript \t reply
//!                    (tabs/newlines/backslashes escaped)
//!   <id>.strokes     the pen strokes: one line per stroke, "x,y,r;x,y,r;…"
//!
//! Delete the directory and the diary forgets. `RIDDLE_MEMORY=off` disables
//! remembering entirely (no storage, no context sent with requests).

use std::io::Write as _;
use std::path::PathBuf;

/// Newest memories the diary keeps. Older pages are forgotten (pruned).
const MAX_MEMORIES: usize = 400;
/// Decimation: drop replay points closer than this (px) to the last kept one.
/// Handwriting stays faithful; files shrink several-fold.
const MIN_POINT_DIST2: i64 = 9;

pub type Strokes = Vec<Vec<(i32, i32, i32)>>;

#[derive(Clone)]
pub struct Entry {
    /// Unix seconds when the page was committed. Also the strokes filename.
    pub id: u64,
    pub transcript: String,
    pub reply: String,
}

pub struct MemoryStore {
    dir: PathBuf,
    pub entries: Vec<Entry>,
}

impl MemoryStore {
    /// Open (or start) the diary's memory. Returns None when memory is off.
    pub fn open() -> Option<Self> {
        match std::env::var("RIDDLE_MEMORY").as_deref() {
            Ok("off") | Ok("0") | Ok("no") | Ok("false") => return None,
            _ => {}
        }
        let dir = std::env::var("RIDDLE_MEMORY_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/home/root/riddle-data/memories"));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("riddle: memory disabled ({}: {e})", dir.display());
            return None;
        }
        let mut store = Self { dir, entries: Vec::new() };
        store.load();
        Some(store)
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.tsv")
    }

    fn strokes_path(&self, id: u64) -> PathBuf {
        self.dir.join(format!("{id}.strokes"))
    }

    fn load(&mut self) {
        let Ok(text) = std::fs::read_to_string(self.index_path()) else { return };
        for line in text.lines() {
            let mut cols = line.splitn(3, '\t');
            let (Some(id), Some(t), Some(r)) = (cols.next(), cols.next(), cols.next()) else {
                continue;
            };
            let Ok(id) = id.parse() else { continue };
            self.entries.push(Entry { id, transcript: unescape(t), reply: unescape(r) });
        }
    }

    /// Remember a finished turn. Strokes are decimated before writing.
    pub fn append(&mut self, id: u64, transcript: &str, reply: &str, strokes: &Strokes) {
        let thin = decimate(strokes);
        let mut lines = String::new();
        for s in &thin {
            let mut first = true;
            for &(x, y, r) in s {
                if !first {
                    lines.push(';');
                }
                lines.push_str(&format!("{x},{y},{r}"));
                first = false;
            }
            lines.push('\n');
        }
        if let Err(e) = std::fs::write(self.strokes_path(id), lines) {
            eprintln!("riddle: memory strokes not kept: {e}");
        }
        let entry = Entry { id, transcript: transcript.to_string(), reply: reply.to_string() };
        let line = format!("{id}\t{}\t{}\n", escape(&entry.transcript), escape(&entry.reply));
        let appended = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.index_path())
            .and_then(|mut f| f.write_all(line.as_bytes()));
        if let Err(e) = appended {
            eprintln!("riddle: memory not kept: {e}");
            return;
        }
        self.entries.push(entry);
        self.prune();
    }

    /// The basilisk fang lands: forget every remembered page, on disk too.
    /// Every `*.strokes` file in the memory dir goes — including orphans not
    /// in the loaded index (a lost/corrupt index.tsv, or an append that wrote
    /// strokes and then failed) — because erase must mean erase.
    pub fn forget_all(&mut self) {
        if let Ok(dir) = std::fs::read_dir(&self.dir) {
            for entry in dir.flatten() {
                let p = entry.path();
                if p.extension().is_some_and(|x| x == "strokes") {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
        let _ = std::fs::remove_file(self.index_path());
        self.entries.clear();
    }

    /// Forget the oldest pages beyond MAX_MEMORIES.
    fn prune(&mut self) {
        if self.entries.len() <= MAX_MEMORIES {
            return;
        }
        let drop_n = self.entries.len() - MAX_MEMORIES;
        for e in &self.entries[..drop_n] {
            let _ = std::fs::remove_file(self.strokes_path(e.id));
        }
        self.entries.drain(..drop_n);
        let mut out = String::new();
        for e in &self.entries {
            out.push_str(&format!("{}\t{}\t{}\n", e.id, escape(&e.transcript), escape(&e.reply)));
        }
        if let Err(e) = std::fs::write(self.index_path(), out) {
            eprintln!("riddle: memory prune failed: {e}");
        }
    }

    /// Load the pen strokes of one remembered page.
    pub fn strokes(&self, id: u64) -> Option<Strokes> {
        let text = std::fs::read_to_string(self.strokes_path(id)).ok()?;
        let mut strokes = Vec::new();
        for line in text.lines() {
            let mut stroke = Vec::new();
            for pt in line.split(';') {
                let mut n = pt.split(',');
                let (Some(x), Some(y), Some(r)) = (n.next(), n.next(), n.next()) else {
                    continue;
                };
                if let (Ok(x), Ok(y), Ok(r)) = (x.parse(), y.parse(), r.parse()) {
                    stroke.push((x, y, r));
                }
            }
            if !stroke.is_empty() {
                strokes.push(stroke);
            }
        }
        Some(strokes)
    }

    pub fn get(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// The last `n` turns as (transcript, reply) pairs, oldest first — the
    /// conversational memory that rides along with each request.
    pub fn recent_dialogue(&self, n: usize) -> Vec<(String, String)> {
        self.entries
            .iter()
            .rev()
            .take(n)
            .filter(|e| !e.transcript.is_empty())
            .map(|e| (e.transcript.clone(), e.reply.clone()))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// The catalog shown to the oracle so it can pick a page to conjure:
    /// numbered newest-first. Returns (lines, ids) where ids[i] belongs to
    /// catalog number i+1.
    pub fn catalog(&self, max: usize) -> (Vec<String>, Vec<u64>) {
        let mut lines = Vec::new();
        let mut ids = Vec::new();
        for (i, e) in self.entries.iter().rev().take(max).enumerate() {
            let gist = if e.transcript.trim().is_empty() {
                format!("(reply: {})", one_line(&e.reply, 70))
            } else {
                one_line(&e.transcript, 70)
            };
            // One entry per line: the catalog is a numbered list the model
            // reads back, so a gist must never carry its own newline.
            lines.push(format!("{}. {} — {}", i + 1, spoken_date(e.id), gist));
            ids.push(e.id);
        }
        (lines, ids)
    }
}

/// Collapse whitespace (incl. newlines) to single spaces and cap at `max`
/// chars, so a multi-line transcript stays one catalog line.
fn one_line(s: &str, max: usize) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

fn decimate(strokes: &Strokes) -> Strokes {
    strokes
        .iter()
        .map(|s| {
            let mut out: Vec<(i32, i32, i32)> = Vec::new();
            for (i, &(x, y, r)) in s.iter().enumerate() {
                let keep = match out.last() {
                    None => true,
                    Some(&(lx, ly, _)) => {
                        let (dx, dy) = ((x - lx) as i64, (y - ly) as i64);
                        dx * dx + dy * dy >= MIN_POINT_DIST2 || i == s.len() - 1
                    }
                };
                if keep {
                    out.push((x, y, r));
                }
            }
            out
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// "the 6th of July, in the evening" — how the diary speaks of a moment.
/// Local time via libc so the device's timezone is respected; the writer can
/// nudge it with RIDDLE_TZ_OFFSET (hours) if the tablet clock runs on UTC.
pub fn spoken_date(id: u64) -> String {
    let offset: i64 = std::env::var("RIDDLE_TZ_OFFSET")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|h| (h * 3600.0) as i64)
        .unwrap_or(0);
    let t = id as i64 + offset;
    let (y, mo, d, h) = civil(t);
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September",
        "October", "November", "December",
    ];
    let suffix = match d {
        11..=13 => "th",
        _ => match d % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    };
    let tod = match h {
        0..=4 => "in the small hours",
        5..=11 => "in the morning",
        12..=17 => "in the afternoon",
        18..=21 => "in the evening",
        _ => "late at night",
    };
    let _ = y;
    format!("the {d}{suffix} of {}, {tod}", MONTHS[(mo - 1) as usize])
}

/// Days-since-epoch to civil date (Howard Hinnant's algorithm) + hour of day.
fn civil(secs: i64) -> (i64, i64, i64, i64) {
    let days = secs.div_euclid(86400);
    let hour = secs.rem_euclid(86400) / 3600;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d, hour)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store(name: &str) -> MemoryStore {
        let dir =
            std::env::temp_dir().join(format!("riddle-mem-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        MemoryStore { dir, entries: Vec::new() }
    }

    #[test]
    fn round_trip_and_reload() {
        let mut s = tmp_store("rt");
        let strokes: Strokes = vec![vec![(10, 20, 3), (14, 24, 3), (100, 120, 2)]];
        s.append(1751856000, "hello\ttom\nnewline", "Hello. Who writes?", &strokes);
        let dir = s.dir.clone();

        let mut s2 = MemoryStore { dir, entries: Vec::new() };
        s2.load();
        assert_eq!(s2.entries.len(), 1);
        assert_eq!(s2.entries[0].transcript, "hello\ttom\nnewline");
        assert_eq!(s2.entries[0].reply, "Hello. Who writes?");
        let back = s2.strokes(1751856000).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].first(), Some(&(10, 20, 3)));
        assert_eq!(back[0].last(), Some(&(100, 120, 2)));
        let _ = std::fs::remove_dir_all(&s2.dir);
    }

    #[test]
    fn decimation_keeps_endpoints_drops_dense() {
        let dense: Strokes = vec![(0..100).map(|i| (i, 0, 3)).collect()];
        let thin = decimate(&dense);
        assert!(thin[0].len() < 40, "kept too many: {}", thin[0].len());
        assert_eq!(thin[0].first(), Some(&(0, 0, 3)));
        assert_eq!(thin[0].last(), Some(&(99, 0, 3)));
    }

    #[test]
    fn prune_forgets_oldest() {
        let mut s = tmp_store("prune");
        for i in 0..(MAX_MEMORIES + 5) as u64 {
            s.append(i + 1, "t", "r", &vec![vec![(1, 1, 1)]]);
        }
        assert_eq!(s.entries.len(), MAX_MEMORIES);
        assert_eq!(s.entries[0].id, 6);
        assert!(!s.strokes_path(1).exists());
        assert!(s.strokes_path(6).exists());
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn catalog_is_numbered_newest_first() {
        let mut s = tmp_store("catalog");
        s.append(1751856000, "about the garden", "…", &vec![vec![(1, 1, 1)]]);
        s.append(1751942400, "about the rain", "…", &vec![vec![(1, 1, 1)]]);
        let (lines, ids) = s.catalog(10);
        assert_eq!(ids, vec![1751942400, 1751856000]);
        assert!(lines[0].starts_with("1. "));
        assert!(lines[0].contains("about the rain"));
        assert!(lines[1].contains("about the garden"));
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn spoken_dates_read_like_a_diary() {
        // 2026-07-06 23:30 UTC.
        let s = spoken_date(1783467000);
        assert!(s.contains("of July"), "{s}");
        assert!(s.contains("6th") || s.contains("7th"), "{s}");
    }
}
