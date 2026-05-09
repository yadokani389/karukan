//! IME Engine - the core state machine and input processing
//!
//! This module contains the main `InputMethodEngine` struct that coordinates between
//! the romaji converter, kanji converter, and manages the IME state.

mod conversion;
mod cursor;
mod display;
mod init;
mod input;
mod input_buffer;
mod mode;
mod segmentation;
mod strategy;
mod types;

pub use types::*;

use input_buffer::InputBuffer;

#[cfg(test)]
mod tests;

use karukan_engine::{
    Dictionary, KanaKanjiConverter, LearningCache, LearningMatch, RomajiConverter,
};
use tracing::{debug, trace};

use super::candidate::{Candidate, CandidateCommitKind, CandidateList};
use super::keycode::{KeyEvent, Keysym};
use super::preedit::{AttributeType, Preedit, PreeditAttribute, PreeditSegment};
use super::state::{ConversionSegment, ConversionSession, InputState};
use crate::config::settings::Settings;

/// Source of a conversion candidate
#[derive(Debug, Clone, PartialEq, Eq)]
enum CandidateSource {
    /// User dictionary lookup
    UserDictionary,
    /// Learning cache (user history)
    Learning,
    /// Model inference result
    Model,
    /// System dictionary lookup
    Dictionary,
    /// Hiragana/katakana fallback
    Fallback,
}

impl CandidateSource {
    fn label(&self) -> &'static str {
        match self {
            CandidateSource::UserDictionary => "\u{1F464} \u{30E6}\u{30FC}\u{30B6}\u{30FC}", // 👤 ユーザー
            CandidateSource::Learning => "\u{1F4DD} \u{5B66}\u{7FD2}", // 📝 学習
            CandidateSource::Model => "\u{1F916} AI",                  // 🤖 AI
            CandidateSource::Dictionary => "\u{1F4DA} \u{8F9E}\u{66F8}", // 📚 辞書
            CandidateSource::Fallback => "",
        }
    }
}

/// A conversion candidate with source annotation
#[derive(Debug, Clone)]
struct AnnotatedCandidate {
    text: String,
    source: CandidateSource,
    /// Override reading (e.g. from prefix_lookup where the full reading differs from input)
    reading: Option<String>,
    commit_kind: CandidateCommitKind,
}

/// Resolve a model variant id from settings.
///
/// - `model` is None or empty → default variant from registry
/// - `model` matches a known variant id → that variant
/// - otherwise → error (unknown variant)
pub fn resolve_variant_id(model: Option<&str>) -> anyhow::Result<String> {
    let reg = karukan_engine::kanji::registry();
    match model {
        Some(id) if !id.is_empty() => {
            if reg.find_variant(id).is_some() {
                Ok(id.to_string())
            } else {
                anyhow::bail!("unknown model variant: {}", id)
            }
        }
        _ => Ok(reg.default_model.clone()),
    }
}

/// The main IME engine
pub struct InputMethodEngine {
    /// Current input state
    state: InputState,
    /// Converters (romaji, kanji, light kanji)
    converters: Converters,
    /// Surrounding text context from the editor (text around cursor)
    surrounding_context: Option<SurroundingContext>,
    /// Engine configuration
    config: EngineConfig,
    /// Conversion timing and adaptive model metrics
    metrics: ConversionMetrics,
    /// Current input mode (Hiragana, Katakana, or Alphabet)
    input_mode: InputMode,
    /// Temporary direct conversion mode triggered by F6-F10.
    direct_mode: Option<DirectConversionMode>,
    /// Composed input buffer (hiragana text, cursor position)
    input_buf: InputBuffer,
    /// Raw input chunks aligned with `input_buf.text` characters.
    raw_units: Vec<String>,
    /// Live conversion state
    live: LiveConversion,
    /// Dictionaries (system, user)
    dicts: Dictionaries,
    /// Learning cache (user conversion history)
    learning: Option<LearningCache>,
}

impl InputMethodEngine {
    /// Create a new IME engine
    pub fn new() -> Self {
        let config = EngineConfig::default();
        Self {
            state: InputState::Empty,
            converters: Converters {
                romaji: RomajiConverter::new(),
                kanji: None,
                light_kanji: None,
                bunsetsu_tokenizer: None,
            },
            surrounding_context: None,
            live: LiveConversion {
                enabled: config.live_conversion,
                text: String::new(),
            },
            config,
            metrics: ConversionMetrics::default(),
            input_mode: InputMode::Hiragana,
            direct_mode: None,
            input_buf: InputBuffer::new(),
            raw_units: Vec::new(),
            dicts: Dictionaries::default(),
            learning: None,
        }
    }

    /// Create with configuration
    pub fn with_config(config: EngineConfig) -> Self {
        let mut engine = Self::new();
        engine.live.enabled = config.live_conversion;
        engine.config = config;
        engine
    }

    /// Get last conversion time in milliseconds (inference only)
    pub fn last_conversion_ms(&self) -> u64 {
        self.metrics.conversion_ms
    }

    /// Get last process_key time in milliseconds (input to result, end-to-end)
    pub fn last_process_key_ms(&self) -> u64 {
        self.metrics.process_key_ms
    }

    /// Get the model name being used
    pub fn model_name(&self) -> String {
        let main = self
            .converters
            .kanji
            .as_ref()
            .map(|c| c.model_display_name());
        let sub = self
            .converters
            .light_kanji
            .as_ref()
            .map(|c| c.model_display_name());
        match (main, sub) {
            (Some(m), Some(s)) => format!("{}+{}", m, s),
            (Some(m), None) => m.to_string(),
            _ => "unknown".to_string(),
        }
    }

    /// Get the current state
    pub fn state(&self) -> &InputState {
        &self.state
    }

    /// Get the current preedit
    pub fn preedit(&self) -> Option<&Preedit> {
        self.state.preedit()
    }

    /// Get the current candidates
    pub fn candidates(&self) -> Option<&CandidateList> {
        self.state.candidates()
    }

    /// Reset the engine state
    /// Note: surrounding_context is intentionally NOT cleared here.
    /// It is set once at activate() time and should persist through
    /// the session. fcitx5 may send reset events between activate
    /// and the first keyEvent, which would wipe the context.
    pub fn reset(&mut self) {
        self.state = InputState::Empty;
        self.converters.romaji.reset();
        self.input_mode = InputMode::Hiragana;
        self.direct_mode = None;
        self.input_buf.clear();
        self.raw_units.clear();
        self.live.text.clear();
        self.metrics = ConversionMetrics::default();
    }

    /// If the display is empty, reset to Empty state and return the result.
    /// Returns None if display is not empty (caller should continue normally).
    fn try_reset_if_empty(&mut self) -> Option<EngineResult> {
        if self.build_input_display().is_empty() {
            self.state = InputState::Empty;
            self.input_buf.clear();
            self.raw_units.clear();
            self.direct_mode = None;
            Some(
                EngineResult::consumed()
                    .with_action(EngineAction::UpdatePreedit(Preedit::new()))
                    .with_action(EngineAction::HideCandidates)
                    .with_action(EngineAction::HideAuxText),
            )
        } else {
            None
        }
    }

    /// Update state to Composing with current preedit and romaji buffer, returning the preedit.
    /// Automatically uses live conversion display when `live.text` is non-empty.
    fn set_composing_state(&mut self) -> Preedit {
        let romaji_buffer = self.converters.romaji.buffer().to_string();
        let preedit = self.build_composing_preedit();
        self.state = InputState::Composing {
            preedit: preedit.clone(),
            romaji_buffer,
        };
        preedit
    }

    fn raw_text_before_cursor(&self) -> String {
        let split = self.input_buf.cursor_pos.min(self.raw_units.len());
        self.raw_units[..split].concat()
    }

    fn raw_text_after_cursor(&self) -> String {
        let split = self.input_buf.cursor_pos.min(self.raw_units.len());
        self.raw_units[split..].concat()
    }

    fn raw_text_for_range(&self, start: usize, end: usize) -> String {
        let start = start.min(self.raw_units.len());
        let end = end.min(self.raw_units.len()).max(start);
        self.raw_units[start..end].concat()
    }

    fn split_raw_chunks(raw: &str, output: &str) -> Vec<String> {
        let output_len = output.chars().count();
        if output_len == 0 {
            return Vec::new();
        }

        let raw_chars: Vec<char> = raw.chars().collect();
        let mut start = 0;
        let mut remaining_raw = raw_chars.len();
        let mut remaining_output = output_len;
        let mut chunks = Vec::with_capacity(output_len);

        for _ in 0..output_len {
            let take = if remaining_output == 1 {
                remaining_raw
            } else {
                remaining_raw.div_ceil(remaining_output)
            };
            let end = start + take.min(remaining_raw);
            chunks.push(raw_chars[start..end].iter().collect());
            let consumed = end - start;
            start = end;
            remaining_raw -= consumed;
            remaining_output -= 1;
        }

        chunks
    }

    fn insert_raw_chunks(&mut self, chunks: Vec<String>) {
        if chunks.is_empty() {
            return;
        }

        let insert_at = self.input_buf.cursor_pos.min(self.raw_units.len());
        self.raw_units.splice(insert_at..insert_at, chunks);
    }

    fn insert_raw_text(&mut self, text: &str) {
        self.insert_raw_chunks(text.chars().map(|ch| ch.to_string()).collect());
    }

    fn remove_raw_before_cursor(&mut self) {
        if self.input_buf.cursor_pos > 0 {
            self.raw_units.remove(self.input_buf.cursor_pos - 1);
        }
    }

    fn remove_raw_at_cursor(&mut self) {
        if self.input_buf.cursor_pos < self.raw_units.len() {
            self.raw_units.remove(self.input_buf.cursor_pos);
        }
    }

    /// Convert hiragana in input_buf to katakana permanently.
    /// Called when leaving Katakana mode so the preedit doesn't revert.
    fn bake_katakana(&mut self) {
        if !self.input_buf.text.is_empty() {
            self.input_buf.text = karukan_engine::kana::hiragana_to_katakana(&self.input_buf.text);
        }
    }

    /// Flush the romaji buffer and insert result at cursor position
    fn flush_romaji_to_composed(&mut self) {
        if self.converters.romaji.buffer().is_empty() {
            return;
        }
        let raw = self.converters.romaji.buffer().to_string();
        let prev_output_len = self.converters.romaji.output().chars().count();
        let _flushed = self.converters.romaji.flush();
        // flush() appends converted buffer to output internally
        let new_from_flush: String = self
            .converters
            .romaji
            .output()
            .chars()
            .skip(prev_output_len)
            .collect();
        if !new_from_flush.is_empty() {
            let raw_chunks = Self::split_raw_chunks(&raw, &new_from_flush);
            self.insert_raw_chunks(raw_chunks);
            self.input_buf.insert(&new_from_flush);
        }
    }

    /// Set both left and right context from surrounding text (from editor)
    /// left_context: text before cursor
    /// right_context: text after cursor
    pub fn set_surrounding_context(&mut self, left_context: &str, right_context: &str) {
        debug!(
            "set_surrounding_context: left=\"{}\" right=\"{}\"",
            left_context, right_context
        );

        // Strip to current line: left = text after last newline.
        // If cursor is right after a newline, left context is empty.
        let left_context = match left_context.rsplit_once('\n') {
            Some((_, after)) => after,
            None => left_context,
        };
        let right_context = right_context
            .split_once('\n')
            .map_or(right_context, |(before, _)| before);

        if left_context.is_empty() && right_context.is_empty() {
            self.surrounding_context = None;
            return;
        }

        // Truncate left context to max length (keep end)
        let left = if left_context.is_empty() {
            None
        } else {
            let left_count = left_context.chars().count();
            Some(if left_count > self.config.max_api_context_len {
                let start = left_count - self.config.max_api_context_len;
                left_context.chars().skip(start).collect()
            } else {
                left_context.to_string()
            })
        };

        // Truncate right context to max length (keep beginning)
        let right = if right_context.is_empty() {
            None
        } else {
            let right_count = right_context.chars().count();
            Some(if right_count > self.config.max_api_context_len {
                right_context
                    .chars()
                    .take(self.config.max_api_context_len)
                    .collect()
            } else {
                right_context.to_string()
            })
        };

        self.surrounding_context = Some(SurroundingContext { left, right });
    }

    /// Handle mode toggle keys (Right Alt/Super/Meta/Hyper): one-way non-Hiragana → Hiragana.
    /// Returns `Some(result)` if the key was handled, `None` if not a mode toggle key.
    fn handle_mode_toggle_key(&mut self, key: &KeyEvent) -> Option<EngineResult> {
        if !key.keysym.is_mode_toggle_key() {
            return None;
        }
        // Only consume the key when actually switching; otherwise pass through
        // so the system can properly track modifier state.
        if key.is_press && self.input_mode != InputMode::Hiragana {
            self.input_mode = InputMode::Hiragana;
            self.direct_mode = None;
            self.flush_romaji_to_composed();
            let aux = self.format_aux_composing();
            if matches!(self.state, InputState::Composing { .. }) {
                let preedit = self.set_composing_state();
                return Some(
                    EngineResult::consumed()
                        .with_action(EngineAction::UpdatePreedit(preedit))
                        .with_action(EngineAction::UpdateAuxText(aux)),
                );
            }
            return Some(EngineResult::consumed().with_action(EngineAction::UpdateAuxText(aux)));
        }
        Some(EngineResult::not_consumed())
    }

    /// Process a key event
    pub fn process_key(&mut self, key: &KeyEvent) -> EngineResult {
        // Log modifier key events for debugging key mapping issues
        if key.keysym.is_modifier() {
            debug!(
                "modifier key: keysym=0x{:04x} press={} modifiers={:?}",
                key.keysym.0, key.is_press, key.modifiers
            );
        }

        // Right Alt/Super/Meta/Hyper: one-way non-Hiragana → Hiragana switch
        if let Some(result) = self.handle_mode_toggle_key(key) {
            return result;
        }

        // Modifier-only keys (Shift, Ctrl, Alt_L, Super_L, etc.): pass through
        if key.keysym.is_modifier() {
            return EngineResult::not_consumed();
        }

        // Only process key presses
        if !key.is_press {
            return EngineResult::not_consumed();
        }

        // Ctrl+Shift+L: toggle live conversion (works in all states)
        if key.modifiers.control_key
            && key.modifiers.shift_key
            && (key.keysym == Keysym::KEY_L || key.keysym == Keysym::KEY_L_UPPER)
        {
            return self.toggle_live_conversion();
        }

        // Ctrl+J: explicit return to Hiragana from Alphabet/Katakana modes.
        if key.modifiers.control_key
            && !key.modifiers.alt_key
            && !key.modifiers.shift_key
            && (key.keysym == Keysym::KEY_J || key.keysym == Keysym::KEY_J_UPPER)
        {
            return self.enter_hiragana_mode();
        }

        // Reset adaptive model flag when starting a new word (first key in Empty state)
        if matches!(self.state, InputState::Empty) {
            self.metrics.adaptive_use_light_model = false;
        }

        trace!(
            "Processing key: {:?} in state: {:?}",
            key.keysym, self.state
        );

        let start = std::time::Instant::now();

        let shift_active = key.modifiers.shift_key;

        let result = match &self.state {
            InputState::Empty => self.process_key_empty(key, shift_active),
            InputState::Composing { .. } => self.process_key_composing(key, shift_active),
            InputState::Conversion { .. } => self.process_key_conversion(key),
        };

        self.metrics.process_key_ms = start.elapsed().as_millis() as u64;

        result
    }

    /// Commit any pending input and return the text
    pub fn commit(&mut self) -> String {
        match &self.state {
            InputState::Empty => String::new(),
            InputState::Composing { .. } => {
                // Flush romaji buffer into composed_hiragana
                self.flush_romaji_to_composed();
                let reading = self.input_buf.text.clone();
                let text = if !self.live.text.is_empty() {
                    self.live.text.clone()
                } else {
                    reading.clone()
                };
                self.converters.romaji.reset();
                self.input_buf.clear();
                self.live.text.clear();
                self.direct_mode = None;
                self.input_mode = InputMode::Hiragana;
                self.state = InputState::Empty;
                self.raw_units.clear();
                self.surrounding_context = None;
                text
            }
            InputState::Conversion { session, .. } => {
                let text = session.composed_text();
                self.input_buf.clear();
                self.direct_mode = None;
                self.raw_units.clear();
                self.input_mode = InputMode::Hiragana;
                self.state = InputState::Empty;
                self.surrounding_context = None;
                text
            }
        }
    }

    /// Save the learning cache to disk if it has unsaved changes.
    pub fn save_learning(&mut self) {
        if let Some(cache) = &mut self.learning
            && cache.is_dirty()
            && let Some(path) = Settings::learning_file()
        {
            if let Err(e) = cache.save(&path) {
                debug!("Failed to save learning cache: {}", e);
            } else {
                debug!("Learning cache saved to {:?}", path);
            }
        }
    }
}

impl Default for InputMethodEngine {
    fn default() -> Self {
        Self::new()
    }
}
