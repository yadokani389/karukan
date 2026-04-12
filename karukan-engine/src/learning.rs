//! Learning cache for remembering user conversion preferences.
//!
//! Learning has two strengths:
//! - strong learning: an explicit candidate change by the user
//! - weak learning: the user keeps accepting the default candidate
//!
//! Weak learning only becomes visible after the same default candidate has been
//! accepted repeatedly. Persisted as TSV with v1 compatibility.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// A single learned conversion entry.
#[derive(Debug, Clone)]
pub struct LearningEntry {
    /// Surface form (e.g. "今日")
    pub surface: String,
    /// Number of explicit non-default selections.
    pub strong_selections: u32,
    /// Number of repeated default acceptances promoted into weak learning.
    pub weak_accepts: u32,
    /// Current streak of repeated default acceptances for this surface.
    pub pending_accept_streak: u8,
    /// Last time this entry was touched as Unix timestamp (seconds).
    pub last_access: u64,
}

impl LearningEntry {
    fn is_learned(&self) -> bool {
        self.strong_selections > 0 || self.weak_accepts > 0
    }

    fn is_strong_learned(&self) -> bool {
        self.strong_selections > 0
    }
}

/// A scored learning match for a reading.
#[derive(Debug, Clone)]
pub struct LearningMatch {
    pub surface: String,
    pub score: f64,
    pub strong_score: f64,
    pub weak_score: f64,
    pub strong_selections: u32,
    pub weak_accepts: u32,
}

/// In-memory cache of user learning data.
///
/// Keyed by reading (hiragana). Each reading maps to a list of surface
/// entries with explicit and implicit learning metadata.
#[derive(Debug)]
pub struct LearningCache {
    entries: HashMap<String, Vec<LearningEntry>>,
    max_entries: usize,
    dirty: bool,
}

impl LearningCache {
    /// Default maximum number of total entries across all readings.
    pub const DEFAULT_MAX_ENTRIES: usize = 10_000;
    /// Number of repeated default acceptances required before weak learning starts.
    pub const WEAK_ACCEPT_THRESHOLD: u8 = 3;

    /// Create an empty cache with the given entry limit.
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries,
            dirty: false,
        }
    }

    /// Record an explicit candidate selection.
    pub fn record(&mut self, reading: &str, surface: &str) {
        self.record_strong(reading, surface);
    }

    /// Record an explicit candidate change by the user.
    pub fn record_strong(&mut self, reading: &str, surface: &str) {
        let now = now_unix();
        let entries = self.entries.entry(reading.to_string()).or_default();
        Self::reset_other_streaks(entries, surface);
        let entry = Self::entry_mut(entries, surface, now);
        entry.strong_selections += 1;
        entry.pending_accept_streak = 0;
        entry.last_access = now;
        self.dirty = true;
    }

    /// Record accepting the default candidate without changing it.
    pub fn record_weak(&mut self, reading: &str, surface: &str) {
        let now = now_unix();
        let entries = self.entries.entry(reading.to_string()).or_default();
        Self::reset_other_streaks(entries, surface);
        let entry = Self::entry_mut(entries, surface, now);
        entry.pending_accept_streak = entry.pending_accept_streak.saturating_add(1);
        entry.last_access = now;
        if entry.pending_accept_streak >= Self::WEAK_ACCEPT_THRESHOLD {
            entry.weak_accepts += 1;
        }
        self.dirty = true;
    }

    /// Record accepting a candidate as supporting evidence immediately.
    pub fn record_weak_immediate(&mut self, reading: &str, surface: &str) {
        let now = now_unix();
        let entries = self.entries.entry(reading.to_string()).or_default();
        Self::reset_other_streaks(entries, surface);
        let entry = Self::entry_mut(entries, surface, now);
        entry.weak_accepts += 1;
        entry.pending_accept_streak = 0;
        entry.last_access = now;
        self.dirty = true;
    }

    /// Exact-match lookup: returns `(surface, score)` pairs sorted by score descending.
    pub fn lookup(&self, reading: &str) -> Vec<(String, f64)> {
        self.lookup_matches(reading)
            .into_iter()
            .map(|entry| (entry.surface, entry.score))
            .collect()
    }

    /// Exact-match lookup with learning metadata.
    pub fn lookup_matches(&self, reading: &str) -> Vec<LearningMatch> {
        let now = now_unix();
        let Some(entries) = self.entries.get(reading) else {
            return Vec::new();
        };
        let mut scored: Vec<LearningMatch> = entries
            .iter()
            .filter(|entry| entry.is_learned())
            .map(|entry| learning_match(entry, now))
            .collect();
        scored.sort_by(|a, b| b.score.total_cmp(&a.score));
        scored
    }

    /// Exact-match lookup limited to explicit selections only.
    pub fn lookup_strong(&self, reading: &str) -> Vec<(String, f64)> {
        self.lookup_strong_matches(reading)
            .into_iter()
            .map(|entry| (entry.surface, entry.score))
            .collect()
    }

    /// Exact-match lookup limited to explicit selections only, with metadata.
    pub fn lookup_strong_matches(&self, reading: &str) -> Vec<LearningMatch> {
        let now = now_unix();
        let Some(entries) = self.entries.get(reading) else {
            return Vec::new();
        };
        let mut scored: Vec<LearningMatch> = entries
            .iter()
            .filter(|entry| entry.is_strong_learned())
            .map(|entry| learning_match(entry, now))
            .collect();
        scored.sort_by(|a, b| b.score.total_cmp(&a.score));
        scored
    }

    /// Prefix-match lookup: returns `(reading, surface, score)` triples
    /// for all readings that start with `prefix`, sorted by score descending.
    pub fn prefix_lookup(&self, prefix: &str) -> Vec<(String, String, f64)> {
        let now = now_unix();
        let mut results: Vec<(String, String, f64)> = Vec::new();
        for (reading, entries) in &self.entries {
            if reading.starts_with(prefix) {
                for entry in entries {
                    if entry.is_learned() {
                        results.push((reading.clone(), entry.surface.clone(), score(entry, now)));
                    }
                }
            }
        }
        results.sort_by(|a, b| b.2.total_cmp(&a.2));
        results
    }

    /// Load a learning cache from a TSV file.
    ///
    /// Supported formats:
    /// - v2: `reading\tsurface\tstrong\tweak\tpending_streak\tlast_access`
    /// - v1: `reading\tsurface\tfrequency\tlast_access`
    pub fn load(path: &Path, max_entries: usize) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let mut cache = Self::new(max_entries);

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            let (strong_selections, weak_accepts, pending_accept_streak, last_access) =
                match parts.len() {
                    len if len >= 6 => {
                        let strong_selections: u32 = match parts[2].parse() {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        let weak_accepts: u32 = match parts[3].parse() {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        let pending_accept_streak: u8 = match parts[4].parse() {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        let last_access: u64 = match parts[5].parse() {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        (
                            strong_selections,
                            weak_accepts,
                            pending_accept_streak,
                            last_access,
                        )
                    }
                    4 => {
                        let frequency: u32 = match parts[2].parse() {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        let last_access: u64 = match parts[3].parse() {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        (frequency, 0, 0, last_access)
                    }
                    _ => continue,
                };

            cache
                .entries
                .entry(parts[0].to_string())
                .or_default()
                .push(LearningEntry {
                    surface: parts[1].to_string(),
                    strong_selections,
                    weak_accepts,
                    pending_accept_streak,
                    last_access,
                });
        }

        cache.dirty = false;
        Ok(cache)
    }

    /// Save the cache to a TSV file, evicting low-score entries if over capacity.
    pub fn save(&mut self, path: &Path) -> anyhow::Result<()> {
        self.evict();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::File::create(path)?;
        let mut writer = std::io::BufWriter::new(file);
        writeln!(writer, "# karukan learning cache v2")?;

        let mut readings: Vec<&String> = self.entries.keys().collect();
        readings.sort();

        for reading in readings {
            if let Some(entries) = self.entries.get(reading) {
                for entry in entries {
                    writeln!(
                        writer,
                        "{}\t{}\t{}\t{}\t{}\t{}",
                        reading,
                        entry.surface,
                        entry.strong_selections,
                        entry.weak_accepts,
                        entry.pending_accept_streak,
                        entry.last_access
                    )?;
                }
            }
        }

        writer.flush()?;
        self.dirty = false;
        Ok(())
    }

    /// Whether there are unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Total number of (reading, surface) pairs across all readings.
    pub fn entry_count(&self) -> usize {
        self.entries.values().map(|entries| entries.len()).sum()
    }

    /// Evict lowest-score entries until total count is within `max_entries`.
    fn evict(&mut self) {
        let total = self.entry_count();
        if total <= self.max_entries {
            return;
        }

        let now = now_unix();
        let mut all: Vec<(String, usize, f64)> = Vec::with_capacity(total);
        for (reading, entries) in &self.entries {
            for (index, entry) in entries.iter().enumerate() {
                all.push((reading.clone(), index, score(entry, now)));
            }
        }
        all.sort_by(|a, b| a.2.total_cmp(&b.2));

        let to_remove = total - self.max_entries;
        let mut remove_set: HashMap<String, Vec<usize>> = HashMap::new();
        for &(ref reading, index, _) in all.iter().take(to_remove) {
            remove_set.entry(reading.clone()).or_default().push(index);
        }

        for (reading, indices) in &mut remove_set {
            indices.sort_unstable();
            indices.reverse();
            if let Some(entries) = self.entries.get_mut(reading) {
                for &index in indices.iter() {
                    if index < entries.len() {
                        entries.remove(index);
                    }
                }
                if entries.is_empty() {
                    self.entries.remove(reading);
                }
            }
        }
    }

    fn entry_mut<'a>(
        entries: &'a mut Vec<LearningEntry>,
        surface: &str,
        now: u64,
    ) -> &'a mut LearningEntry {
        if let Some(index) = entries.iter().position(|entry| entry.surface == surface) {
            return &mut entries[index];
        }

        entries.push(LearningEntry {
            surface: surface.to_string(),
            strong_selections: 0,
            weak_accepts: 0,
            pending_accept_streak: 0,
            last_access: now,
        });
        entries
            .last_mut()
            .expect("entry list must contain the inserted surface")
    }

    fn reset_other_streaks(entries: &mut [LearningEntry], surface: &str) {
        for entry in entries {
            if entry.surface != surface {
                entry.pending_accept_streak = 0;
            }
        }
    }
}

/// Compute a candidate score: recency-weighted with strong and weak boosts.
fn score(entry: &LearningEntry, now: u64) -> f64 {
    if !entry.is_learned() {
        return 0.0;
    }

    let age_days = if now > entry.last_access {
        (now - entry.last_access) / 86400
    } else {
        0
    };
    let recency = 1.0 / (1.0 + age_days as f64);
    let strong = strong_score(entry);
    let weak = weak_score(entry);
    recency * 10.0 + strong + weak
}

fn strong_score(entry: &LearningEntry) -> f64 {
    (entry.strong_selections as f64).ln_1p() * 3.0
}

fn weak_score(entry: &LearningEntry) -> f64 {
    (entry.weak_accepts as f64).ln_1p()
}

fn learning_match(entry: &LearningEntry, now: u64) -> LearningMatch {
    LearningMatch {
        surface: entry.surface.clone(),
        score: score(entry, now),
        strong_score: strong_score(entry),
        weak_score: weak_score(entry),
        strong_selections: entry.strong_selections,
        weak_accepts: entry.weak_accepts,
    }
}

/// Current time as Unix timestamp in seconds.
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_record_and_lookup() {
        let mut cache = LearningCache::new(100);

        cache.record("きょう", "今日");
        cache.record("きょう", "京");
        cache.record("きょう", "今日");

        let results = cache.lookup("きょう");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "今日");
        assert_eq!(results[1].0, "京");
    }

    #[test]
    fn test_lookup_empty() {
        let cache = LearningCache::new(100);
        let results = cache.lookup("きょう");
        assert!(results.is_empty());
    }

    #[test]
    fn test_lookup_strong_excludes_weak_learning() {
        let mut cache = LearningCache::new(100);
        cache.record_weak("きょう", "今日");
        cache.record_weak("きょう", "今日");
        cache.record_weak("きょう", "今日");
        cache.record_strong("きょう", "京");

        let results = cache.lookup_strong("きょう");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "京");
    }

    #[test]
    fn test_weak_accept_requires_threshold() {
        let mut cache = LearningCache::new(100);

        cache.record_weak("きょう", "今日");
        cache.record_weak("きょう", "今日");
        assert!(cache.lookup("きょう").is_empty());

        cache.record_weak("きょう", "今日");
        let results = cache.lookup("きょう");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "今日");
    }

    #[test]
    fn test_weak_accept_streak_resets_on_other_surface() {
        let mut cache = LearningCache::new(100);

        cache.record_weak("きょう", "今日");
        cache.record_weak("きょう", "今日");
        cache.record_weak("きょう", "京");
        cache.record_weak("きょう", "今日");

        assert!(cache.lookup("きょう").is_empty());
    }

    #[test]
    fn test_prefix_lookup() {
        let mut cache = LearningCache::new(100);
        cache.record("きょう", "今日");
        cache.record("きょうと", "京都");
        cache.record("あした", "明日");

        let results = cache.prefix_lookup("きょう");
        assert_eq!(results.len(), 2);
        let readings: Vec<&str> = results
            .iter()
            .map(|(reading, _, _)| reading.as_str())
            .collect();
        assert!(readings.contains(&"きょう"));
        assert!(readings.contains(&"きょうと"));
    }

    #[test]
    fn test_prefix_lookup_no_match() {
        let mut cache = LearningCache::new(100);
        cache.record("きょう", "今日");
        let results = cache.prefix_lookup("あ");
        assert!(results.is_empty());
    }

    #[test]
    fn test_save_and_load() {
        let mut cache = LearningCache::new(100);
        cache.record("きょう", "今日");
        cache.record("きょう", "今日");
        cache.record_weak("きょう", "京");
        cache.record_weak("きょう", "京");
        cache.record_weak("きょう", "京");

        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_path_buf();

        cache.save(&path).unwrap();
        assert!(!cache.is_dirty());

        let loaded = LearningCache::load(&path, 100).unwrap();
        assert!(!loaded.is_dirty());
        assert_eq!(loaded.entry_count(), 2);

        let results = loaded.lookup("きょう");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "今日");
    }

    #[test]
    fn test_load_v1_format() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            "# karukan learning cache v1\nきょう\t今日\t5\t1700000000\n",
        )
        .unwrap();

        let cache = LearningCache::load(file.path(), 100).unwrap();
        let results = cache.lookup("きょう");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "今日");
    }

    #[test]
    fn test_dirty_flag() {
        let mut cache = LearningCache::new(100);
        assert!(!cache.is_dirty());

        cache.record("きょう", "今日");
        assert!(cache.is_dirty());

        let file = NamedTempFile::new().unwrap();
        cache.save(file.path()).unwrap();
        assert!(!cache.is_dirty());
    }

    #[test]
    fn test_eviction() {
        let mut cache = LearningCache::new(3);

        cache.record("a", "A");
        cache.record("b", "B");
        cache.record("c", "C");
        cache.record("d", "D");
        cache.record("e", "E");

        cache.record("a", "A");
        cache.record("a", "A");
        cache.record("c", "C");

        let file = NamedTempFile::new().unwrap();
        cache.save(file.path()).unwrap();

        assert!(cache.entry_count() <= 3);
    }

    #[test]
    fn test_score_recency() {
        let now = now_unix();
        let recent = LearningEntry {
            surface: "A".to_string(),
            strong_selections: 1,
            weak_accepts: 0,
            pending_accept_streak: 0,
            last_access: now,
        };
        let old = LearningEntry {
            surface: "B".to_string(),
            strong_selections: 1,
            weak_accepts: 0,
            pending_accept_streak: 0,
            last_access: now.saturating_sub(30 * 86400),
        };
        assert!(score(&recent, now) > score(&old, now));
    }

    #[test]
    fn test_score_frequency() {
        let now = now_unix();
        let high_freq = LearningEntry {
            surface: "A".to_string(),
            strong_selections: 100,
            weak_accepts: 0,
            pending_accept_streak: 0,
            last_access: now,
        };
        let low_freq = LearningEntry {
            surface: "B".to_string(),
            strong_selections: 1,
            weak_accepts: 0,
            pending_accept_streak: 0,
            last_access: now,
        };
        assert!(score(&high_freq, now) > score(&low_freq, now));
    }

    #[test]
    fn test_load_nonexistent_file() {
        let result = LearningCache::load(Path::new("/nonexistent/path"), 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_tsv_format() {
        let mut cache = LearningCache::new(100);
        cache.record("きょう", "今日");

        let file = NamedTempFile::new().unwrap();
        cache.save(file.path()).unwrap();

        let content = std::fs::read_to_string(file.path()).unwrap();
        assert!(content.starts_with("# karukan learning cache v2"));
        assert!(content.contains("きょう\t今日\t1\t0\t0\t"));
    }

    #[test]
    fn test_tsv_comments_and_blanks_ignored() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            "# comment\n\nきょう\t今日\t5\t1\t0\t1700000000\n# another comment\n",
        )
        .unwrap();

        let cache = LearningCache::load(file.path(), 100).unwrap();
        assert_eq!(cache.entry_count(), 1);
        let results = cache.lookup("きょう");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "今日");
    }

    #[test]
    fn test_tsv_malformed_lines_skipped() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            "きょう\t今日\t5\t1\t0\t1700000000\nmalformed_line\nきょう\t京\tbad\t0\t0\t1700000000\n",
        )
        .unwrap();

        let cache = LearningCache::load(file.path(), 100).unwrap();
        assert_eq!(cache.entry_count(), 1);
    }
}
