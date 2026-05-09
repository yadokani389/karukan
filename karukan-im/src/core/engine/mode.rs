//! Mode switching (katakana, alphabet, live conversion)

use tracing::debug;

use super::*;

impl InputMethodEngine {
    fn next_direct_alphabet_case(mode: DirectConversionMode) -> DirectConversionMode {
        match mode {
            DirectConversionMode::AlphabetFullwidth(DirectAlphabetCase::Lower) => {
                DirectConversionMode::AlphabetFullwidth(DirectAlphabetCase::Upper)
            }
            DirectConversionMode::AlphabetFullwidth(DirectAlphabetCase::Upper) => {
                DirectConversionMode::AlphabetFullwidth(DirectAlphabetCase::Capitalized)
            }
            DirectConversionMode::AlphabetFullwidth(DirectAlphabetCase::Capitalized) => {
                DirectConversionMode::AlphabetFullwidth(DirectAlphabetCase::Lower)
            }
            DirectConversionMode::AlphabetHalfwidth(DirectAlphabetCase::Lower) => {
                DirectConversionMode::AlphabetHalfwidth(DirectAlphabetCase::Upper)
            }
            DirectConversionMode::AlphabetHalfwidth(DirectAlphabetCase::Upper) => {
                DirectConversionMode::AlphabetHalfwidth(DirectAlphabetCase::Capitalized)
            }
            DirectConversionMode::AlphabetHalfwidth(DirectAlphabetCase::Capitalized) => {
                DirectConversionMode::AlphabetHalfwidth(DirectAlphabetCase::Lower)
            }
            _ => mode,
        }
    }

    pub(super) fn direct_mode_for_function_key(
        &self,
        keysym: Keysym,
    ) -> Option<DirectConversionMode> {
        match keysym {
            Keysym::F6 => Some(DirectConversionMode::Hiragana),
            Keysym::F7 => Some(DirectConversionMode::KatakanaFullwidth),
            Keysym::F8 => Some(DirectConversionMode::KatakanaHalfwidth),
            Keysym::F9 => Some(match self.direct_mode {
                Some(mode @ DirectConversionMode::AlphabetFullwidth(_)) => {
                    Self::next_direct_alphabet_case(mode)
                }
                _ => DirectConversionMode::AlphabetFullwidth(DirectAlphabetCase::Lower),
            }),
            Keysym::F10 => Some(match self.direct_mode {
                Some(mode @ DirectConversionMode::AlphabetHalfwidth(_)) => {
                    Self::next_direct_alphabet_case(mode)
                }
                _ => DirectConversionMode::AlphabetHalfwidth(DirectAlphabetCase::Lower),
            }),
            _ => None,
        }
    }

    pub(super) fn activate_direct_mode(&mut self, mode: DirectConversionMode) -> EngineResult {
        self.flush_romaji_to_composed();
        self.converters.romaji.reset();
        self.live.text.clear();
        self.direct_mode = Some(mode);
        self.input_buf.cursor_pos = self.input_buf.text.chars().count();
        let preedit = self.set_composing_state();

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::HideCandidates)
            .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()))
    }

    pub(super) fn handle_function_key(&mut self, keysym: Keysym) -> Option<EngineResult> {
        let mode = self.direct_mode_for_function_key(keysym)?;
        Some(self.activate_direct_mode(mode))
    }

    pub(super) fn commit_direct_mode(&mut self) -> EngineResult {
        let Some(text) = self.direct_commit_text() else {
            return EngineResult::not_consumed();
        };

        self.converters.romaji.reset();
        self.input_buf.clear();
        self.raw_units.clear();
        self.live.text.clear();
        self.direct_mode = None;
        self.state = InputState::Empty;

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(Preedit::new()))
            .with_action(EngineAction::HideCandidates)
            .with_action(EngineAction::Commit(text))
            .with_action(EngineAction::HideAuxText)
    }

    pub(super) fn commit_direct_mode_and_continue(&mut self, ch: char) -> EngineResult {
        let Some(text) = self.direct_commit_text() else {
            return EngineResult::not_consumed();
        };

        self.converters.romaji.reset();
        self.input_buf.clear();
        self.raw_units.clear();
        self.live.text.clear();
        self.direct_mode = None;
        self.state = InputState::Empty;

        let new_input_result = self.start_input(ch);
        let mut result = EngineResult::consumed().with_action(EngineAction::Commit(text));
        result.actions.extend(new_input_result.actions);
        result
    }

    /// Toggle katakana mode (Ctrl+k).
    pub(super) fn enter_katakana_mode(&mut self) -> EngineResult {
        if self.input_mode == InputMode::Katakana {
            return self.enter_hiragana_mode();
        }

        self.input_mode = InputMode::Katakana;
        self.direct_mode = None;
        // Clear live conversion text so katakana mode takes priority on commit
        self.live.text.clear();

        let romaji_buffer = self.converters.romaji.buffer().to_string();

        if self.input_buf.text.is_empty() && romaji_buffer.is_empty() {
            return EngineResult::consumed();
        }

        let preedit = self.set_composing_state();

        // Update aux text to show mode
        let aux = format!("{} Karukan ({})", self.mode_indicator(), self.model_name());

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::UpdateAuxText(aux))
    }

    pub(super) fn enter_hiragana_mode(&mut self) -> EngineResult {
        if self.input_mode == InputMode::Hiragana {
            return EngineResult::not_consumed();
        }

        if self.input_mode == InputMode::Katakana {
            self.bake_katakana();
        }
        self.input_mode = InputMode::Hiragana;
        self.direct_mode = None;
        self.flush_romaji_to_composed();

        let aux = self.format_aux_composing();
        if matches!(self.state, InputState::Composing { .. }) {
            let preedit = self.set_composing_state();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::UpdateAuxText(aux));
        }

        EngineResult::consumed().with_action(EngineAction::UpdateAuxText(aux))
    }

    /// Toggle live conversion mode via Ctrl+Shift+L
    pub(super) fn toggle_live_conversion(&mut self) -> EngineResult {
        self.direct_mode = None;
        self.live.enabled = !self.live.enabled;
        let mode = if self.live.enabled { "ON" } else { "OFF" };
        debug!("Live conversion toggled: {}", mode);
        EngineResult::consumed()
            .with_action(EngineAction::UpdateAuxText(format!("ライブ変換: {}", mode)))
    }
}
