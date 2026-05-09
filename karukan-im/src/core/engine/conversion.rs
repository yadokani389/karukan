//! Conversion state handling (candidates, segments, commit)

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use tracing::debug;

use super::*;

/// Maximum number of learning candidates to show in tests.
#[cfg(test)]
const MAX_LEARNING_CANDIDATES: usize = 3;
const USER_DICT_BASE_SCORE: f64 = 18.0;
const USER_DICT_SCORE_SCALE: f64 = 4.0;
const STRONG_LEARNING_SCORE_SCALE: f64 = 1.3;
const WEAK_LEARNING_SCORE_SCALE: f64 = 0.35;
const MODEL_TOP_SCORE: f64 = 12.0;
const MODEL_RANK_DECAY: f64 = 2.0;
const SYSTEM_DICT_SCORE_SCALE: f64 = 6.0;
const HIRAGANA_FALLBACK_SCORE: f64 = -6.0;
const KATAKANA_FALLBACK_SCORE: f64 = -7.0;
const PRESERVED_AUTO_SUGGEST_BONUS: f64 = 2.0;
const AUTO_SUGGEST_STRONG_OVERRIDE_BONUS: f64 = 4.0;
const AUTO_SUGGEST_SWITCH_MARGIN: f64 = 3.0;
const MAX_SENTENCE_ALTERNATIVES_PER_SPAN: usize = 3;
const MAX_SENTENCE_ALTERNATIVES_TOTAL: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FallbackKind {
    Hiragana,
    Katakana,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LearningMode {
    Strong,
    Weak,
    WeakImmediate,
}

#[derive(Debug, Clone)]
pub(super) struct AggregatedCandidate {
    pub(super) text: String,
    reading: Option<String>,
    commit_kind: CandidateCommitKind,
    best_model_rank: Option<usize>,
    user_dict_score: Option<f32>,
    system_dict_score: Option<f32>,
    learning: Option<LearningMatch>,
    fallback: Option<FallbackKind>,
    preserved: bool,
}

impl AggregatedCandidate {
    fn new(text: String, reading: Option<String>, commit_kind: CandidateCommitKind) -> Self {
        Self {
            text,
            reading,
            commit_kind,
            best_model_rank: None,
            user_dict_score: None,
            system_dict_score: None,
            learning: None,
            fallback: None,
            preserved: false,
        }
    }

    pub(super) fn source(&self) -> CandidateSource {
        if self.user_dict_score.is_some() {
            CandidateSource::UserDictionary
        } else if self
            .learning
            .as_ref()
            .is_some_and(|entry| entry.strong_selections > 0 || entry.weak_accepts > 0)
        {
            CandidateSource::Learning
        } else if self.best_model_rank.is_some() {
            CandidateSource::Model
        } else if self.system_dict_score.is_some() {
            CandidateSource::Dictionary
        } else {
            CandidateSource::Fallback
        }
    }

    fn final_score(&self) -> f64 {
        let learning_score = self.learning.as_ref().map_or(0.0, |entry| {
            entry.strong_score * STRONG_LEARNING_SCORE_SCALE
                + entry.weak_score * WEAK_LEARNING_SCORE_SCALE
        });
        let user_dict_score = self.user_dict_score.map_or(0.0, |score| {
            USER_DICT_BASE_SCORE + normalized_dictionary_score(score, USER_DICT_SCORE_SCALE)
        });
        let system_dict_score = self.system_dict_score.map_or(0.0, |score| {
            normalized_dictionary_score(score, SYSTEM_DICT_SCORE_SCALE)
        });
        let model_score = self.best_model_rank.map_or(0.0, |rank| {
            (MODEL_TOP_SCORE - MODEL_RANK_DECAY * rank as f64).max(0.0)
        });
        let fallback_score = match self.fallback {
            Some(FallbackKind::Hiragana) => HIRAGANA_FALLBACK_SCORE,
            Some(FallbackKind::Katakana) => KATAKANA_FALLBACK_SCORE,
            None => 0.0,
        };

        learning_score + user_dict_score + system_dict_score + model_score + fallback_score
    }

    fn partial_override_score(&self) -> f64 {
        let mut score = self.final_score();
        if self.preserved {
            score += PRESERVED_AUTO_SUGGEST_BONUS;
        }
        if self.user_dict_score.is_some() || self.has_strong_learning_signal() {
            score += AUTO_SUGGEST_STRONG_OVERRIDE_BONUS;
        }
        score
    }

    fn has_strong_learning_signal(&self) -> bool {
        self.learning
            .as_ref()
            .is_some_and(|entry| entry.strong_selections > 0)
    }

    fn has_explicit_override_signal(&self) -> bool {
        self.user_dict_score.is_some() || self.has_strong_learning_signal()
    }

    fn partial_override_source(&self) -> CandidateSource {
        if self.user_dict_score.is_some() {
            CandidateSource::UserDictionary
        } else {
            CandidateSource::Learning
        }
    }

    fn into_annotated(self) -> AnnotatedCandidate {
        let source = self.source();
        AnnotatedCandidate {
            text: self.text,
            source,
            reading: self.reading,
            commit_kind: self.commit_kind,
        }
    }

    #[cfg(test)]
    fn from_source(text: impl Into<String>, source: CandidateSource) -> Self {
        let text = text.into();
        let mut candidate = Self::new(text.clone(), None, CandidateCommitKind::Whole);
        match source {
            CandidateSource::UserDictionary => candidate.user_dict_score = Some(0.0),
            CandidateSource::Learning => {
                candidate.learning = Some(LearningMatch {
                    surface: text,
                    score: 0.0,
                    strong_score: 1.0_f64.ln_1p() * 3.0,
                    weak_score: 0.0,
                    strong_selections: 1,
                    weak_accepts: 0,
                });
            }
            CandidateSource::Model => candidate.best_model_rank = Some(0),
            CandidateSource::Dictionary => candidate.system_dict_score = Some(0.0),
            CandidateSource::Fallback => candidate.fallback = Some(FallbackKind::Hiragana),
        }
        candidate
    }

    fn from_model_text(text: impl Into<String>) -> Self {
        let mut candidate = Self::new(text.into(), None, CandidateCommitKind::Whole);
        candidate.best_model_rank = Some(0);
        candidate
    }
}

fn normalized_dictionary_score(score: f32, scale: f64) -> f64 {
    scale / (1.0 + score.max(0.0) as f64)
}

impl InputMethodEngine {
    pub(super) fn slice_chars(text: &str, start: usize, end: usize) -> String {
        text.chars()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect()
    }

    pub(super) fn build_candidate_list_from_annotated(
        &self,
        reading: &str,
        candidates: Vec<AnnotatedCandidate>,
    ) -> CandidateList {
        CandidateList::new(
            candidates
                .into_iter()
                .enumerate()
                .map(|(i, ac)| {
                    let label = ac.source.label();
                    let cand_reading = ac.reading.unwrap_or_else(|| reading.to_string());
                    let commit_kind = ac.commit_kind;
                    let mut c = if label.is_empty() {
                        Candidate::with_reading(&ac.text, &cand_reading)
                    } else {
                        Candidate {
                            text: ac.text,
                            reading: Some(cand_reading),
                            annotation: Some(label.to_string()),
                            index: 0,
                            commit_kind: commit_kind.clone(),
                        }
                    };
                    if label.is_empty() {
                        c.commit_kind = commit_kind;
                    }
                    c.index = i;
                    c
                })
                .collect(),
        )
    }

    fn sentence_override_options(&self, reading: &str) -> Vec<(String, CandidateSource)> {
        let mut options = Vec::new();
        let mut seen = HashSet::new();

        if let Some(dict) = &self.dicts.user
            && let Some(result) = dict.exact_match_search(reading)
        {
            for candidate in result.candidates {
                if seen.insert(candidate.surface.clone()) {
                    options.push((candidate.surface.clone(), CandidateSource::UserDictionary));
                }
                if options.len() >= MAX_SENTENCE_ALTERNATIVES_PER_SPAN {
                    return options;
                }
            }
        }

        if let Some(cache) = &self.learning {
            for entry in cache.lookup_strong_matches(reading) {
                if seen.insert(entry.surface.clone()) {
                    options.push((entry.surface, CandidateSource::Learning));
                }
                if options.len() >= MAX_SENTENCE_ALTERNATIVES_PER_SPAN {
                    break;
                }
            }
        }

        options
    }

    fn append_sentence_alternatives(
        &mut self,
        reading: &str,
        base_surface: &str,
        candidates: &mut Vec<AnnotatedCandidate>,
    ) {
        let Ok(spans) = self.segment_surface_to_learning_spans(base_surface, reading) else {
            return;
        };

        let mut seen: HashSet<String> = candidates
            .iter()
            .map(|candidate| candidate.text.clone())
            .collect();
        let mut added = 0;

        for span in spans {
            if !span.learnable {
                continue;
            }

            let prefix = Self::slice_chars(base_surface, 0, span.start);
            let suffix = Self::slice_chars(base_surface, span.end, base_surface.chars().count());
            for (replacement, source) in self.sentence_override_options(&span.reading) {
                if replacement == span.surface {
                    continue;
                }

                let text = format!("{}{}{}", prefix, replacement, suffix);
                if !seen.insert(text.clone()) {
                    continue;
                }

                candidates.push(AnnotatedCandidate {
                    text,
                    source,
                    reading: None,
                    commit_kind: CandidateCommitKind::Whole,
                });
                added += 1;
                if added >= MAX_SENTENCE_ALTERNATIVES_TOTAL {
                    return;
                }
            }
        }
    }

    #[cfg(test)]
    pub(super) fn make_preserved_candidate(
        &self,
        text: &str,
        source: CandidateSource,
    ) -> AggregatedCandidate {
        AggregatedCandidate::from_source(text, source)
    }

    fn build_prefix_commit_candidates(
        &mut self,
        reading: &str,
        existing: &[AnnotatedCandidate],
        num_candidates: usize,
    ) -> Vec<AnnotatedCandidate> {
        let mut prefix_candidates = Vec::new();
        let mut seen: HashSet<String> = existing
            .iter()
            .map(|candidate| candidate.text.clone())
            .collect();
        let mut seen_prefix_readings = HashSet::new();

        for candidate in existing {
            if candidate.source == CandidateSource::Fallback {
                continue;
            }

            let Ok(ranges) = self.segment_surface_to_ranges(&candidate.text, reading) else {
                continue;
            };
            let Some((first_start, first_end)) = ranges.first().copied() else {
                continue;
            };
            let first_reading = Self::slice_chars(reading, first_start, first_end);
            if ranges.len() <= 1
                || first_start != 0
                || first_reading == reading
                || !seen_prefix_readings.insert(first_reading.clone())
            {
                continue;
            }

            let committed_reading_len = first_end;
            for prefix_candidate in
                self.build_exact_conversion_candidates(&first_reading, num_candidates)
            {
                if !seen.insert(prefix_candidate.text.clone()) {
                    continue;
                }

                prefix_candidates.push(AnnotatedCandidate {
                    text: prefix_candidate.text,
                    source: prefix_candidate.source,
                    reading: Some(first_reading.clone()),
                    commit_kind: CandidateCommitKind::Prefix {
                        committed_reading_len,
                    },
                });
            }
        }

        prefix_candidates
    }

    fn build_segment_candidates(
        &mut self,
        reading: &str,
        num_candidates: usize,
        preserved_text: Option<String>,
        allow_prefix_commit: bool,
    ) -> CandidateList {
        let ranked_candidates = self.ranked_candidates_with_preserved(
            reading,
            num_candidates,
            preserved_text
                .as_deref()
                .filter(|text| !text.is_empty() && *text != reading),
        );
        let mut candidates = self.annotate_candidates(ranked_candidates);
        if let Some(base_surface) = candidates.first().map(|candidate| candidate.text.clone()) {
            self.append_sentence_alternatives(reading, &base_surface, &mut candidates);
        }
        if allow_prefix_commit {
            candidates.extend(self.build_prefix_commit_candidates(
                reading,
                &candidates,
                num_candidates,
            ));
        }
        self.build_candidate_list_from_annotated(reading, candidates)
    }

    fn build_segment(&mut self, reading: &str, start: usize, end: usize) -> ConversionSegment {
        let segment_reading = Self::slice_chars(reading, start, end);
        ConversionSegment {
            reading_start: start,
            reading_end: end,
            candidates: self.build_segment_candidates(
                &segment_reading,
                self.config.num_candidates,
                None,
                false,
            ),
            explicit_candidate_selection: false,
        }
    }

    fn build_conversion_session_from_ranges(
        &mut self,
        reading: &str,
        ranges: Vec<(usize, usize)>,
    ) -> ConversionSession {
        let mut segments = Vec::with_capacity(ranges.len());
        for (start, end) in ranges {
            segments.push(self.build_segment(reading, start, end));
        }

        ConversionSession {
            reading: reading.to_string(),
            segments,
            active_segment: 0,
            segmentation_applied: true,
            enter_segments: false,
        }
    }

    pub(super) fn build_single_segment_session(
        &mut self,
        reading: &str,
        preserved_text: Option<String>,
    ) -> ConversionSession {
        let total = reading.chars().count();
        let segment = ConversionSegment {
            reading_start: 0,
            reading_end: total,
            candidates: self.build_segment_candidates(
                reading,
                self.config.num_candidates,
                preserved_text,
                true,
            ),
            explicit_candidate_selection: false,
        };

        ConversionSession {
            reading: reading.to_string(),
            segments: vec![segment],
            active_segment: 0,
            segmentation_applied: false,
            enter_segments: true,
        }
    }

    fn sync_conversion_state(
        &mut self,
        preedit: Preedit,
        candidates: CandidateList,
        session: ConversionSession,
    ) {
        if let InputState::Conversion {
            preedit: state_preedit,
            candidates: state_candidates,
            session: state_session,
        } = &mut self.state
        {
            *state_preedit = preedit;
            *state_candidates = candidates;
            *state_session = session;
        }
    }

    fn active_segment_reading(session: &ConversionSession) -> String {
        let Some(segment) = session.segments.get(session.active_segment) else {
            return String::new();
        };
        Self::slice_chars(&session.reading, segment.reading_start, segment.reading_end)
    }

    fn build_conversion_preedit(session: &ConversionSession) -> Preedit {
        let mut caret = 0;
        let segments: Vec<_> = session
            .segments
            .iter()
            .enumerate()
            .flat_map(|(index, segment)| {
                let segment_reading =
                    Self::slice_chars(&session.reading, segment.reading_start, segment.reading_end);
                let preedit_segments = Self::build_segment_preedit_segments(
                    segment,
                    &segment_reading,
                    index == session.active_segment,
                );
                let text_len: usize = preedit_segments
                    .iter()
                    .map(|seg| seg.text.chars().count())
                    .sum();
                if index <= session.active_segment {
                    caret += text_len;
                }
                preedit_segments
            })
            .collect();
        Preedit::from_segments(segments, caret)
    }

    fn build_segment_preedit_segments(
        segment: &ConversionSegment,
        segment_reading: &str,
        is_active: bool,
    ) -> Vec<PreeditSegment> {
        let attr = if is_active {
            AttributeType::Highlight
        } else {
            AttributeType::Underline
        };
        let Some(candidate) = segment.candidates.selected() else {
            return vec![PreeditSegment::new(String::new(), attr)];
        };

        match candidate.commit_kind {
            CandidateCommitKind::Whole => vec![PreeditSegment::new(candidate.text.clone(), attr)],
            CandidateCommitKind::Prefix {
                committed_reading_len,
            } => {
                let committed_len = committed_reading_len.min(segment_reading.chars().count());
                let remaining = Self::slice_chars(
                    segment_reading,
                    committed_len,
                    segment_reading.chars().count(),
                );
                let mut segments = vec![PreeditSegment::new(candidate.text.clone(), attr)];
                if !remaining.is_empty() {
                    segments.push(PreeditSegment::new(remaining, AttributeType::Underline));
                }
                segments
            }
        }
    }

    fn update_conversion_state(&mut self) -> EngineResult {
        let Some(session) = self.state.conversion_session().cloned() else {
            return EngineResult::not_consumed();
        };
        let candidates = session
            .segments
            .get(session.active_segment)
            .map(|segment| segment.candidates.clone())
            .unwrap_or_default();
        let preedit = Self::build_conversion_preedit(&session);
        let active_reading = Self::active_segment_reading(&session);
        let total_segments = session.segments.len();
        let active_segment = session.active_segment;
        self.sync_conversion_state(preedit.clone(), candidates.clone(), session);

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::ShowCandidates(candidates.clone()))
            .with_action(EngineAction::UpdateAuxText(
                self.format_aux_conversion_with_page(
                    &active_reading,
                    Some(&candidates),
                    active_segment,
                    total_segments,
                ),
            ))
    }

    fn replace_conversion_session(&mut self, session: ConversionSession) -> EngineResult {
        if !matches!(self.state, InputState::Conversion { .. }) {
            return EngineResult::not_consumed();
        }
        let preedit = Self::build_conversion_preedit(&session);
        let candidates = session
            .segments
            .get(session.active_segment)
            .map(|segment| segment.candidates.clone())
            .unwrap_or_default();
        self.sync_conversion_state(preedit, candidates, session);
        self.update_conversion_state()
    }

    /// Run kana-kanji conversion for a reading via llama.cpp model.
    ///
    /// Determines the conversion strategy (main model, light model, or parallel beam),
    /// dispatches to the appropriate model(s), measures latency, and records which model was used.
    fn run_kana_kanji_conversion(&mut self, reading: &str, num_candidates: usize) -> Vec<String> {
        let Some(converter) = self.converters.kanji.as_ref() else {
            return vec![];
        };
        let katakana = karukan_engine::kana::hiragana_to_katakana(reading);
        let api_context = self.truncate_context_for_api();
        let main_model_name = converter.model_display_name().to_string();

        let strategy = self.determine_strategy(reading, num_candidates);
        debug!(
            "convert: reading=\"{}\" api_context=\"{}\" candidates={} strategy={:?}",
            reading, api_context, num_candidates, strategy
        );

        let start = Instant::now();

        let candidates = match &strategy {
            ConversionStrategy::ParallelBeam { beam_width } => {
                let Some(light_converter) = self.converters.light_kanji.as_ref() else {
                    return vec![];
                };
                let bw = *beam_width;
                let (default_top1, light_candidates) = std::thread::scope(|s| {
                    let h_default = s.spawn(|| {
                        converter
                            .convert(&katakana, &api_context, 1)
                            .unwrap_or_default()
                    });
                    let h_beam = s.spawn(|| {
                        light_converter
                            .convert(&katakana, &api_context, bw)
                            .unwrap_or_default()
                    });
                    (
                        h_default.join().unwrap_or_default(),
                        h_beam.join().unwrap_or_default(),
                    )
                });
                Self::merge_candidates_dedup(default_top1, light_candidates, bw)
            }
            ConversionStrategy::LightModelOnly => {
                let Some(light_converter) = self.converters.light_kanji.as_ref() else {
                    return vec![];
                };
                light_converter
                    .convert(&katakana, &api_context, 1)
                    .unwrap_or_default()
            }
            ConversionStrategy::MainModelOnly => converter
                .convert(&katakana, &api_context, 1)
                .unwrap_or_default(),
            ConversionStrategy::MainModelBeam { beam_width } => converter
                .convert(&katakana, &api_context, *beam_width)
                .unwrap_or_default(),
        };

        self.metrics.conversion_ms = start.elapsed().as_millis() as u64;
        self.update_adaptive_model_flag(&strategy);

        self.metrics.model_name = match &strategy {
            ConversionStrategy::ParallelBeam { .. } => {
                let light_name = self
                    .converters
                    .light_kanji
                    .as_ref()
                    .map(|c| c.model_display_name().to_string())
                    .unwrap_or_default();
                format!("{}+{}", main_model_name, light_name)
            }
            ConversionStrategy::LightModelOnly => self
                .converters
                .light_kanji
                .as_ref()
                .map(|c| c.model_display_name().to_string())
                .unwrap_or(main_model_name),
            ConversionStrategy::MainModelOnly | ConversionStrategy::MainModelBeam { .. } => {
                main_model_name
            }
        };

        candidates
    }

    /// Start conversion using the current live-conversion result + dictionary candidates.
    ///
    /// Called when DOWN/TAB is pressed during live conversion.  Instead of
    /// Start kanji conversion
    pub(super) fn start_conversion(&mut self) -> EngineResult {
        self.direct_mode = None;

        // Flush any remaining romaji into composed_hiragana
        self.flush_romaji_to_composed();

        let reading = self.input_buf.text.clone();

        // Save auto-suggest/live conversion result before clearing state.
        // This ensures the candidate that was displayed during input is preserved
        // in the conversion candidate list even if the re-inference uses a different strategy.
        let prev_suggest_text = std::mem::take(&mut self.live.text);

        self.converters.romaji.reset();
        self.input_buf.cursor_pos = 0;

        if reading.is_empty() {
            return EngineResult::consumed();
        }

        let session = self.build_single_segment_session(
            &reading,
            (!prev_suggest_text.is_empty()).then_some(prev_suggest_text),
        );
        self.enter_conversion_state(session)
    }

    /// Transition to Conversion state with the given reading and candidate list.
    ///
    /// Sets up the preedit (highlighted selected text), updates the state, and
    /// returns an EngineResult with preedit, candidates, and aux text actions.
    fn enter_conversion_state(&mut self, session: ConversionSession) -> EngineResult {
        let preedit = Self::build_conversion_preedit(&session);
        let candidates = session
            .segments
            .get(session.active_segment)
            .map(|segment| segment.candidates.clone())
            .unwrap_or_default();
        let reading = Self::active_segment_reading(&session);
        let active_segment = session.active_segment;
        let total_segments = session.segments.len();

        self.state = InputState::Conversion {
            preedit: preedit.clone(),
            candidates: candidates.clone(),
            session,
        };

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::ShowCandidates(candidates.clone()))
            .with_action(EngineAction::UpdateAuxText(
                self.format_aux_conversion_with_page(
                    &reading,
                    Some(&candidates),
                    active_segment,
                    total_segments,
                ),
            ))
    }

    fn ranked_candidate_map(
        &mut self,
        reading: &str,
        num_candidates: usize,
    ) -> HashMap<String, AggregatedCandidate> {
        let mut map = HashMap::new();

        if let Some(cache) = &self.learning {
            for entry in cache.lookup_matches(reading) {
                let key = entry.surface.clone();
                map.entry(key.clone())
                    .or_insert_with(|| {
                        AggregatedCandidate::new(key, None, CandidateCommitKind::Whole)
                    })
                    .learning = Some(entry);
            }
        }

        if let Some(dict) = &self.dicts.user
            && let Some(result) = dict.exact_match_search(reading)
        {
            for candidate in result.candidates {
                let key = candidate.surface.clone();
                let entry = map.entry(key.clone()).or_insert_with(|| {
                    AggregatedCandidate::new(key, None, CandidateCommitKind::Whole)
                });
                entry.user_dict_score = Some(
                    entry
                        .user_dict_score
                        .map_or(candidate.score, |score| score.min(candidate.score)),
                );
            }
        }

        let model_candidates = self.run_kana_kanji_conversion(reading, num_candidates);
        for (rank, text) in model_candidates.into_iter().enumerate() {
            let entry = map.entry(text.clone()).or_insert_with(|| {
                AggregatedCandidate::new(text, None, CandidateCommitKind::Whole)
            });
            entry.best_model_rank = Some(entry.best_model_rank.map_or(rank, |best| best.min(rank)));
        }

        if let Some(dict) = &self.dicts.system
            && let Some(result) = dict.exact_match_search(reading)
        {
            for candidate in result.candidates {
                let key = candidate.surface.clone();
                let entry = map.entry(key.clone()).or_insert_with(|| {
                    AggregatedCandidate::new(key, None, CandidateCommitKind::Whole)
                });
                entry.system_dict_score = Some(
                    entry
                        .system_dict_score
                        .map_or(candidate.score, |score| score.min(candidate.score)),
                );
            }
        }

        let hiragana = reading.to_string();
        map.entry(hiragana.clone())
            .or_insert_with(|| AggregatedCandidate::new(hiragana, None, CandidateCommitKind::Whole))
            .fallback = Some(FallbackKind::Hiragana);

        let katakana = Self::hiragana_to_katakana(reading);
        map.entry(katakana.clone())
            .or_insert_with(|| AggregatedCandidate::new(katakana, None, CandidateCommitKind::Whole))
            .fallback = Some(FallbackKind::Katakana);

        map
    }

    fn ranked_candidates(
        &mut self,
        reading: &str,
        num_candidates: usize,
    ) -> Vec<AggregatedCandidate> {
        self.ranked_candidates_with_preserved(reading, num_candidates, None)
    }

    fn ranked_candidates_with_preserved(
        &mut self,
        reading: &str,
        num_candidates: usize,
        preserved_text: Option<&str>,
    ) -> Vec<AggregatedCandidate> {
        let mut candidate_map = self.ranked_candidate_map(reading, num_candidates);
        if let Some(preserved_text) = preserved_text {
            let entry = candidate_map
                .entry(preserved_text.to_string())
                .or_insert_with(|| {
                    AggregatedCandidate::new(
                        preserved_text.to_string(),
                        None,
                        CandidateCommitKind::Whole,
                    )
                });
            entry.best_model_rank = Some(0);
        }

        let mut ranked: Vec<_> = candidate_map.into_values().collect();
        ranked.sort_by(|a, b| {
            b.final_score()
                .total_cmp(&a.final_score())
                .then_with(|| a.text.cmp(&b.text))
        });
        ranked
    }

    pub(super) fn annotate_candidates(
        &self,
        candidates: Vec<AggregatedCandidate>,
    ) -> Vec<AnnotatedCandidate> {
        candidates
            .into_iter()
            .map(AggregatedCandidate::into_annotated)
            .collect()
    }

    fn partial_override_candidates(&self, reading: &str) -> Vec<AggregatedCandidate> {
        let mut map = HashMap::new();

        if let Some(cache) = &self.learning {
            for entry in cache.lookup_strong_matches(reading) {
                let key = entry.surface.clone();
                map.entry(key.clone())
                    .or_insert_with(|| {
                        AggregatedCandidate::new(key, None, CandidateCommitKind::Whole)
                    })
                    .learning = Some(entry);
            }
        }

        if let Some(dict) = &self.dicts.user
            && let Some(result) = dict.exact_match_search(reading)
        {
            for candidate in result.candidates {
                let key = candidate.surface.clone();
                let entry = map.entry(key.clone()).or_insert_with(|| {
                    AggregatedCandidate::new(key, None, CandidateCommitKind::Whole)
                });
                entry.user_dict_score = Some(
                    entry
                        .user_dict_score
                        .map_or(candidate.score, |score| score.min(candidate.score)),
                );
            }
        }

        let mut ranked: Vec<_> = map.into_values().collect();
        ranked.sort_by(|a, b| {
            b.partial_override_score()
                .total_cmp(&a.partial_override_score())
                .then_with(|| a.text.cmp(&b.text))
        });
        ranked
    }

    /// Build conversion candidates for a reading from multiple sources.
    ///
    /// Combines learning cache, dictionaries, model inference, and fallback
    /// into a single reranked candidate list.
    pub(super) fn build_exact_conversion_ranked_candidates(
        &mut self,
        reading: &str,
        num_candidates: usize,
    ) -> Vec<AggregatedCandidate> {
        if self.converters.kanji.is_none()
            && let Err(e) = self.init_kanji_converter()
        {
            debug!("Failed to initialize kanji converter: {}", e);
            return vec![AggregatedCandidate {
                text: reading.to_string(),
                reading: None,
                commit_kind: CandidateCommitKind::Whole,
                best_model_rank: None,
                user_dict_score: None,
                system_dict_score: None,
                learning: None,
                fallback: Some(FallbackKind::Hiragana),
                preserved: false,
            }];
        }

        self.ranked_candidates(reading, num_candidates)
    }

    pub(super) fn build_exact_conversion_candidates(
        &mut self,
        reading: &str,
        num_candidates: usize,
    ) -> Vec<AnnotatedCandidate> {
        let candidates = self.build_exact_conversion_ranked_candidates(reading, num_candidates);
        self.annotate_candidates(candidates)
    }

    fn select_partial_override_candidate(
        &self,
        reading: &str,
        preserved: &AggregatedCandidate,
    ) -> Option<AnnotatedCandidate> {
        let mut preserved = preserved.clone();
        preserved.preserved = true;
        let preserved_score = preserved.partial_override_score();

        let best = self
            .partial_override_candidates(reading)
            .into_iter()
            .max_by(|a, b| {
                a.partial_override_score()
                    .total_cmp(&b.partial_override_score())
                    .then_with(|| b.preserved.cmp(&a.preserved))
                    .then_with(|| a.text.cmp(&b.text).reverse())
            })?;

        if best.text == preserved.text {
            return None;
        }

        let best_score = best.partial_override_score();
        let should_override = if best.has_explicit_override_signal() {
            best_score > preserved_score
        } else {
            best_score >= preserved_score + AUTO_SUGGEST_SWITCH_MARGIN
        };

        let source = best.partial_override_source();
        should_override.then_some(AnnotatedCandidate {
            text: best.text,
            source,
            reading: None,
            commit_kind: CandidateCommitKind::Whole,
        })
    }

    pub(super) fn rerank_auto_suggest_text(
        &mut self,
        reading: &str,
        preserved: &AggregatedCandidate,
    ) -> String {
        let Ok(spans) = self.segment_surface_to_learning_spans(&preserved.text, reading) else {
            return preserved.text.clone();
        };

        let mut rebuilt = String::new();
        for span in spans {
            if !span.learnable {
                rebuilt.push_str(&span.surface);
                continue;
            }

            let text = self
                .select_partial_override_candidate(
                    &span.reading,
                    &AggregatedCandidate::new(
                        span.surface.clone(),
                        None,
                        CandidateCommitKind::Whole,
                    ),
                )
                .map(|candidate| candidate.text)
                .unwrap_or(span.surface);
            rebuilt.push_str(&text);
        }

        if rebuilt.is_empty() {
            preserved.text.clone()
        } else {
            rebuilt
        }
    }

    pub(super) fn build_live_conversion_text(&mut self, reading: &str) -> Option<String> {
        if self.converters.kanji.is_none()
            && let Err(e) = self.init_kanji_converter()
        {
            debug!("Failed to initialize kanji converter: {}", e);
            return None;
        }

        let model_top_text = self
            .run_kana_kanji_conversion(reading, 1)
            .into_iter()
            .next()?;
        if model_top_text == reading {
            return None;
        }

        let preserved = AggregatedCandidate::from_model_text(model_top_text.clone());
        Some(self.rerank_auto_suggest_text(reading, &preserved))
    }

    #[cfg(test)]
    pub(super) fn sentence_alternatives_for_test(
        &mut self,
        reading: &str,
        base_surface: &str,
    ) -> Vec<AnnotatedCandidate> {
        let mut candidates = vec![AnnotatedCandidate {
            text: base_surface.to_string(),
            source: CandidateSource::Model,
            reading: None,
            commit_kind: CandidateCommitKind::Whole,
        }];
        self.append_sentence_alternatives(reading, base_surface, &mut candidates);
        candidates
    }

    #[cfg(test)]
    /// Look up learned exact-match candidates for a reading (max 3).
    ///
    /// Returns candidates from the learning cache suitable for auto-suggest display.
    pub(super) fn lookup_learning_candidates(&self, reading: &str) -> Vec<Candidate> {
        let Some(cache) = &self.learning else {
            return vec![];
        };
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut seen = HashSet::new();
        let label = CandidateSource::Learning.label().to_string();

        for (surface, _score) in cache.lookup(reading) {
            if candidates.len() >= MAX_LEARNING_CANDIDATES {
                break;
            }
            if seen.insert(surface.clone()) {
                candidates.push(Candidate {
                    text: surface,
                    reading: Some(reading.to_string()),
                    annotation: Some(label.clone()),
                    index: candidates.len(),
                    commit_kind: CandidateCommitKind::Whole,
                });
            }
        }

        candidates
    }

    /// Merge two candidate lists with deduplication
    /// Primary candidates come first, then secondary candidates that aren't duplicates
    pub(super) fn merge_candidates_dedup(
        primary: Vec<String>,
        secondary: Vec<String>,
        max_candidates: usize,
    ) -> Vec<String> {
        let mut seen = HashSet::new();
        primary
            .into_iter()
            .chain(secondary)
            .filter(|c| seen.insert(c.clone()))
            .take(max_candidates)
            .collect()
    }

    /// Process key in conversion state
    pub(super) fn process_key_conversion(&mut self, key: &KeyEvent) -> EngineResult {
        if let Some(result) = self.handle_conversion_function_key(key.keysym) {
            return result;
        }

        if self.config.segment_shrink_key.matches(key) {
            return self.shrink_active_segment();
        }
        if self.config.segment_expand_key.matches(key) {
            return self.expand_active_segment();
        }

        if key.modifiers.shift_key && key.keysym == Keysym::TAB {
            return self.prev_candidate();
        }

        match key.keysym {
            Keysym::RETURN => self.confirm_or_next_segment(),
            Keysym::ESCAPE => self.cancel_conversion(),
            Keysym::SPACE | Keysym::DOWN | Keysym::TAB => self.next_candidate(),
            Keysym::ISO_LEFT_TAB | Keysym::UP => self.prev_candidate(),
            Keysym::PAGE_DOWN => self.next_candidate_page(),
            Keysym::PAGE_UP => self.prev_candidate_page(),
            Keysym::LEFT => self.prev_segment(),
            Keysym::RIGHT => self.next_segment(),
            Keysym::BACKSPACE => self.backspace_conversion(),
            _ => {
                // Ctrl+N / Ctrl+P: emacs-style candidate navigation
                if key.modifiers.control_key && !key.modifiers.alt_key {
                    match key.keysym {
                        Keysym::KEY_N | Keysym::KEY_N_UPPER => return self.next_candidate(),
                        Keysym::KEY_P | Keysym::KEY_P_UPPER => return self.prev_candidate(),
                        _ => {}
                    }
                }

                // Check for digit selection (1-9)
                if let Some(digit) = key.keysym.digit_value() {
                    return self.select_candidate_by_digit(digit);
                }

                // Any printable character: commit current conversion and start new input
                if let Some(ch) = key.to_char()
                    && !key.modifiers.control_key
                    && !key.modifiers.alt_key
                {
                    return self.commit_conversion_and_continue(ch);
                }

                EngineResult::not_consumed()
            }
        }
    }

    /// Get selected text and reading from conversion state, or None if not in conversion
    fn selected_conversion_info(&self) -> Option<(String, Option<String>)> {
        match &self.state {
            InputState::Conversion { session, .. } => {
                let text = Self::build_conversion_preedit(session).text().to_string();
                Some((text, Some(session.reading.clone())))
            }
            _ => None,
        }
    }

    fn selected_commit_kind(&self) -> Option<CandidateCommitKind> {
        self.state
            .conversion_session()
            .and_then(|session| session.segments.get(session.active_segment))
            .and_then(|segment| segment.candidates.selected())
            .map(|candidate| candidate.commit_kind.clone())
    }

    fn commit_prefix_candidate(&mut self, committed_reading_len: usize) -> EngineResult {
        let Some(session) = self.state.conversion_session().cloned() else {
            return EngineResult::not_consumed();
        };
        if session.segments.len() != 1 || session.active_segment != 0 {
            let Some((text, _reading)) = self.selected_conversion_info() else {
                return EngineResult::not_consumed();
            };
            return self.finish_full_conversion(text);
        }

        let Some(candidate) = session
            .segments
            .first()
            .and_then(|segment| segment.candidates.selected())
        else {
            return EngineResult::not_consumed();
        };

        let reading_len = session.reading.chars().count();
        let committed_len = committed_reading_len.min(reading_len);
        let remaining_reading = Self::slice_chars(&session.reading, committed_len, reading_len);
        if remaining_reading.is_empty() {
            return self.finish_full_conversion(candidate.text.clone());
        }

        let committed_text = candidate.text.clone();
        let committed_reading = Self::slice_chars(&session.reading, 0, committed_len);
        let explicit = session
            .segments
            .first()
            .is_some_and(|segment| segment.explicit_candidate_selection);
        let default_text = session
            .segments
            .first()
            .and_then(|segment| segment.candidates.candidates().first())
            .map(|candidate| candidate.text.clone())
            .unwrap_or_else(|| committed_text.clone());
        self.record_segment_learning(&committed_reading, &committed_text, &default_text, explicit);

        self.input_buf.text = remaining_reading.clone();
        self.input_buf.cursor_pos = 0;
        self.raw_units = self
            .raw_units
            .split_off(committed_len.min(self.raw_units.len()));
        self.direct_mode = None;

        let next_session = self.build_single_segment_session(&remaining_reading, None);
        let mut result = EngineResult::consumed().with_action(EngineAction::Commit(committed_text));
        result
            .actions
            .extend(self.enter_conversion_state(next_session).actions);
        result
    }

    /// Record a conversion selection in the learning cache.
    fn record_learning_mode(&mut self, reading: &str, surface: &str, mode: LearningMode) {
        if let Some(cache) = &mut self.learning {
            match mode {
                LearningMode::Strong => cache.record_strong(reading, surface),
                LearningMode::Weak => cache.record_weak(reading, surface),
                LearningMode::WeakImmediate => cache.record_weak_immediate(reading, surface),
            }
        }
    }

    fn learning_records_for_segment(
        &mut self,
        reading: &str,
        selected_surface: &str,
        default_surface: &str,
        explicit: bool,
    ) -> Vec<(String, String, LearningMode)> {
        let Ok(selected_spans) = self.segment_surface_to_learning_spans(selected_surface, reading)
        else {
            return Vec::new();
        };
        let Ok(default_spans) = self.segment_surface_to_learning_spans(default_surface, reading)
        else {
            return Vec::new();
        };

        let default_by_range: HashMap<(usize, usize), String> = default_spans
            .into_iter()
            .map(|span| ((span.start, span.end), span.surface))
            .collect();

        selected_spans
            .into_iter()
            .filter(|span| span.learnable && !span.surface.is_empty() && !span.reading.is_empty())
            .map(|span| {
                let changed = explicit
                    && default_by_range
                        .get(&(span.start, span.end))
                        .is_none_or(|surface| surface != &span.surface);
                let mode = if changed {
                    LearningMode::Strong
                } else if explicit {
                    LearningMode::WeakImmediate
                } else {
                    LearningMode::Weak
                };
                (span.reading, span.surface, mode)
            })
            .collect()
    }

    fn record_segment_learning(
        &mut self,
        reading: &str,
        selected_surface: &str,
        default_surface: &str,
        explicit: bool,
    ) {
        let records =
            self.learning_records_for_segment(reading, selected_surface, default_surface, explicit);
        for (span_reading, span_surface, mode) in records {
            self.record_learning_mode(&span_reading, &span_surface, mode);
        }
    }

    fn record_learning_from_session(&mut self, session: &ConversionSession) {
        let segment_records: Vec<_> = session
            .segments
            .iter()
            .filter_map(|segment| {
                let reading =
                    Self::slice_chars(&session.reading, segment.reading_start, segment.reading_end);
                let selected = segment.selected_text().to_string();
                let default = segment
                    .candidates
                    .candidates()
                    .first()
                    .map(|candidate| candidate.text.clone())
                    .unwrap_or_else(|| selected.clone());
                (!reading.is_empty() && !selected.is_empty()).then_some((
                    reading,
                    selected,
                    default,
                    segment.explicit_candidate_selection,
                ))
            })
            .collect();

        for (reading, selected, default, explicit) in segment_records {
            self.record_segment_learning(&reading, &selected, &default, explicit);
        }
    }

    fn finish_full_conversion(&mut self, text: String) -> EngineResult {
        if text.is_empty() {
            return EngineResult::consumed();
        }

        if let Some(session) = self.state.conversion_session().cloned() {
            self.record_learning_from_session(&session);
        }

        self.state = InputState::Empty;
        self.input_buf.text.clear();
        self.raw_units.clear();
        self.direct_mode = None;

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(Preedit::new()))
            .with_action(EngineAction::HideCandidates)
            .with_action(EngineAction::HideAuxText)
            .with_action(EngineAction::Commit(text))
    }

    /// Commit the current conversion
    fn commit_conversion(&mut self) -> EngineResult {
        if let Some(CandidateCommitKind::Prefix {
            committed_reading_len,
        }) = self.selected_commit_kind()
        {
            return self.commit_prefix_candidate(committed_reading_len);
        }

        let Some((text, _reading)) = self.selected_conversion_info() else {
            return EngineResult::not_consumed();
        };
        self.finish_full_conversion(text)
    }

    fn confirm_or_next_segment(&mut self) -> EngineResult {
        let Some(session) = self.state.conversion_session().cloned() else {
            return EngineResult::not_consumed();
        };

        if !session.segmentation_applied && session.segments.len() == 1 {
            if !session.enter_segments {
                return self.commit_conversion();
            }

            let surface = session
                .segments
                .first()
                .map(ConversionSegment::selected_text)
                .unwrap_or("")
                .to_string();

            let can_segment = self
                .segment_surface_to_ranges(&surface, &session.reading)
                .map(|ranges| ranges.len() > 1)
                .unwrap_or(false);

            return if can_segment {
                self.auto_segment_for_navigation(true)
            } else {
                self.commit_conversion()
            };
        }

        if session.active_segment + 1 < session.segments.len() {
            self.next_segment()
        } else {
            self.commit_conversion()
        }
    }

    /// Commit current conversion and then process a new character as fresh input
    fn commit_conversion_and_continue(&mut self, ch: char) -> EngineResult {
        let Some((text, _reading)) = self.selected_conversion_info() else {
            return EngineResult::not_consumed();
        };

        self.state = InputState::Empty;
        self.input_buf.text.clear();
        self.raw_units.clear();
        self.direct_mode = None;

        // Start new input with the character
        let new_input_result = self.start_input(ch);

        // Combine: commit first, then new input actions
        let mut result = EngineResult::consumed()
            .with_action(EngineAction::Commit(text))
            .with_action(EngineAction::HideCandidates);
        result.actions.extend(new_input_result.actions);
        result
    }

    /// Cancel conversion and return to hiragana
    pub(super) fn cancel_conversion(&mut self) -> EngineResult {
        if !matches!(self.state, InputState::Conversion { .. }) {
            return EngineResult::not_consumed();
        }
        let reading = self.input_buf.text.clone();

        if reading.is_empty() {
            self.state = InputState::Empty;
            self.input_buf.clear();
            self.raw_units.clear();
            self.direct_mode = None;
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(Preedit::new()))
                .with_action(EngineAction::HideCandidates)
                .with_action(EngineAction::HideAuxText);
        }

        // Set up composed_hiragana with the reading
        self.input_buf.text = reading.clone();
        self.input_buf.cursor_pos = self.input_buf.text.chars().count();
        self.direct_mode = None;

        // Reset romaji converter and set output to reading
        self.converters.romaji.reset();
        // We need to push each character to rebuild the state
        for ch in reading.chars() {
            self.converters.romaji.push(ch);
        }

        let preedit = self.set_composing_state();

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::HideCandidates)
            .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()))
    }

    /// Navigate candidates with the given operation, then update preedit.
    fn navigate_candidate(&mut self, op: impl FnOnce(&mut CandidateList) -> bool) -> EngineResult {
        {
            let Some(session) = self.state.conversion_session_mut() else {
                return EngineResult::not_consumed();
            };
            let Some(segment) = session.segments.get_mut(session.active_segment) else {
                return EngineResult::not_consumed();
            };
            if op(&mut segment.candidates) {
                segment.explicit_candidate_selection = true;
            }
            session.enter_segments = false;
        }
        self.direct_mode = None;
        self.update_conversion_state()
    }

    fn auto_segment_for_navigation(&mut self, move_right: bool) -> EngineResult {
        let Some(session) = self.state.conversion_session().cloned() else {
            return EngineResult::not_consumed();
        };
        if session.segmentation_applied || session.segments.len() != 1 {
            return EngineResult::not_consumed();
        }

        let surface = session
            .segments
            .first()
            .and_then(|segment| segment.candidates.selected_text())
            .unwrap_or("")
            .to_string();
        let ranges = match self.segment_surface_to_ranges(&surface, &session.reading) {
            Ok(ranges) if ranges.len() > 1 => ranges,
            _ => return EngineResult::consumed(),
        };

        let mut segmented = self.build_conversion_session_from_ranges(&session.reading, ranges);
        if move_right && segmented.segments.len() > 1 {
            segmented.active_segment = 1;
        }
        self.direct_mode = None;
        self.replace_conversion_session(segmented)
    }

    fn prev_segment(&mut self) -> EngineResult {
        if let Some(session) = self.state.conversion_session()
            && !session.segmentation_applied
            && session.segments.len() == 1
        {
            return self.auto_segment_for_navigation(false);
        }

        let Some(session) = self.state.conversion_session_mut() else {
            return EngineResult::not_consumed();
        };
        if session.active_segment > 0 {
            session.active_segment -= 1;
        }
        self.direct_mode = None;
        self.update_conversion_state()
    }

    fn next_segment(&mut self) -> EngineResult {
        if let Some(session) = self.state.conversion_session()
            && !session.segmentation_applied
            && session.segments.len() == 1
        {
            return self.auto_segment_for_navigation(true);
        }

        let Some(session) = self.state.conversion_session_mut() else {
            return EngineResult::not_consumed();
        };
        if session.active_segment + 1 < session.segments.len() {
            session.active_segment += 1;
        }
        self.direct_mode = None;
        self.update_conversion_state()
    }

    fn shrink_active_segment(&mut self) -> EngineResult {
        let Some(mut session) = self.state.conversion_session().cloned() else {
            return EngineResult::not_consumed();
        };
        let active = session.active_segment;
        let Some(segment) = session.segments.get(active).cloned() else {
            return EngineResult::not_consumed();
        };
        if segment.reading_end.saturating_sub(segment.reading_start) <= 1 {
            return EngineResult::consumed();
        }

        let split_at = segment.reading_end - 1;
        session.segments[active].reading_end = split_at;

        if active + 1 < session.segments.len() {
            session.segments[active + 1].reading_start = split_at;
        } else {
            session.segments.insert(
                active + 1,
                ConversionSegment {
                    reading_start: split_at,
                    reading_end: segment.reading_end,
                    candidates: CandidateList::default(),
                    explicit_candidate_selection: false,
                },
            );
        }

        let reading = session.reading.clone();
        session.segments[active] = self.build_segment(
            &reading,
            session.segments[active].reading_start,
            session.segments[active].reading_end,
        );
        session.segments[active + 1] = self.build_segment(
            &reading,
            session.segments[active + 1].reading_start,
            session.segments[active + 1].reading_end,
        );
        session.segmentation_applied = true;
        self.direct_mode = None;
        self.replace_conversion_session(session)
    }

    fn expand_active_segment(&mut self) -> EngineResult {
        let Some(mut session) = self.state.conversion_session().cloned() else {
            return EngineResult::not_consumed();
        };
        let active = session.active_segment;
        if active + 1 >= session.segments.len() {
            return EngineResult::consumed();
        }
        let next_len = session.segments[active + 1]
            .reading_end
            .saturating_sub(session.segments[active + 1].reading_start);
        if next_len == 0 {
            return EngineResult::consumed();
        }

        session.segments[active].reading_end += 1;
        session.segments[active + 1].reading_start += 1;

        let remove_next =
            session.segments[active + 1].reading_start >= session.segments[active + 1].reading_end;
        let reading = session.reading.clone();
        session.segments[active] = self.build_segment(
            &reading,
            session.segments[active].reading_start,
            session.segments[active].reading_end,
        );

        if remove_next {
            session.segments.remove(active + 1);
            if session.active_segment >= session.segments.len() {
                session.active_segment = session.segments.len().saturating_sub(1);
            }
        } else {
            session.segments[active + 1] = self.build_segment(
                &reading,
                session.segments[active + 1].reading_start,
                session.segments[active + 1].reading_end,
            );
        }

        session.segmentation_applied = true;
        self.direct_mode = None;
        self.replace_conversion_session(session)
    }

    /// Select next candidate
    fn next_candidate(&mut self) -> EngineResult {
        self.navigate_candidate(CandidateList::move_next)
    }

    /// Select previous candidate
    fn prev_candidate(&mut self) -> EngineResult {
        self.navigate_candidate(CandidateList::move_prev)
    }

    /// Go to next candidate page
    fn next_candidate_page(&mut self) -> EngineResult {
        self.navigate_candidate(CandidateList::next_page)
    }

    /// Go to previous candidate page
    fn prev_candidate_page(&mut self) -> EngineResult {
        self.navigate_candidate(CandidateList::prev_page)
    }

    /// Select candidate by digit (1-9)
    fn select_candidate_by_digit(&mut self, digit: usize) -> EngineResult {
        let Some(session) = self.state.conversion_session_mut() else {
            return EngineResult::not_consumed();
        };
        let Some(segment) = session.segments.get_mut(session.active_segment) else {
            return EngineResult::not_consumed();
        };
        if segment.candidates.select_on_page(digit).is_none() {
            return EngineResult::consumed();
        }
        session.enter_segments = false;
        segment.explicit_candidate_selection = true;
        self.direct_mode = None;
        self.update_conversion_state()
    }

    /// Handle backspace in conversion mode
    fn backspace_conversion(&mut self) -> EngineResult {
        // Return to hiragana mode with the reading
        self.cancel_conversion()
    }

    fn handle_conversion_function_key(&mut self, keysym: Keysym) -> Option<EngineResult> {
        let mode = self.direct_mode_for_function_key(keysym)?;
        Some(self.apply_function_key_to_active_segment(mode))
    }

    fn apply_function_key_to_active_segment(&mut self, mode: DirectConversionMode) -> EngineResult {
        let Some(mut session) = self.state.conversion_session().cloned() else {
            return EngineResult::not_consumed();
        };
        let active = session.active_segment;
        let Some(segment) = session.segments.get(active) else {
            return EngineResult::not_consumed();
        };
        let reading =
            Self::slice_chars(&session.reading, segment.reading_start, segment.reading_end);
        let source_text = match mode {
            DirectConversionMode::AlphabetFullwidth(_)
            | DirectConversionMode::AlphabetHalfwidth(_) => {
                self.raw_text_for_range(segment.reading_start, segment.reading_end)
            }
            DirectConversionMode::Hiragana
            | DirectConversionMode::KatakanaFullwidth
            | DirectConversionMode::KatakanaHalfwidth => reading.clone(),
        };
        let converted = self.convert_direct_raw_text(&source_text, mode);
        let mut candidates = CandidateList::new(vec![Candidate {
            text: converted,
            reading: Some(reading),
            annotation: None,
            index: 0,
            commit_kind: CandidateCommitKind::Whole,
        }]);
        candidates.reset();
        session.segments[active].candidates = candidates;
        session.segments[active].explicit_candidate_selection = true;
        session.enter_segments = false;
        self.direct_mode = Some(mode);
        self.replace_conversion_session(session)
    }
}
