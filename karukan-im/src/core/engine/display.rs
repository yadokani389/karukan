//! Display and preedit construction for the IME engine

use super::*;

impl InputMethodEngine {
    fn current_input_parts(&self) -> (String, String, String) {
        let before: String = self
            .input_buf
            .text
            .chars()
            .take(self.input_buf.cursor_pos)
            .collect();
        let after: String = self
            .input_buf
            .text
            .chars()
            .skip(self.input_buf.cursor_pos)
            .collect();
        let buffer = self.converters.romaji.buffer().to_string();

        (before, buffer, after)
    }

    fn direct_mode_indicator(&self, mode: DirectConversionMode) -> &'static str {
        match mode {
            DirectConversionMode::Hiragana => "[あ]",
            DirectConversionMode::KatakanaFullwidth => "[カ]",
            DirectConversionMode::KatakanaHalfwidth => "[ｶ]",
            DirectConversionMode::AlphabetFullwidth(_) => "[Ａ]",
            DirectConversionMode::AlphabetHalfwidth(_) => "[A]",
        }
    }

    fn apply_direct_alphabet_case(text: &str, case: DirectAlphabetCase) -> String {
        match case {
            DirectAlphabetCase::Lower => text.to_lowercase(),
            DirectAlphabetCase::Upper => text.to_uppercase(),
            DirectAlphabetCase::Capitalized => {
                let mut chars = text.chars();
                let Some(first) = chars.next() else {
                    return String::new();
                };
                format!("{}{}", first.to_uppercase(), chars.as_str().to_lowercase())
            }
        }
    }

    fn current_raw_parts(&self) -> (String, String, String) {
        (
            self.raw_text_before_cursor(),
            self.converters.romaji.buffer().to_string(),
            self.raw_text_after_cursor(),
        )
    }

    fn convert_direct_kana_text(&self, text: &str, mode: DirectConversionMode) -> String {
        match mode {
            DirectConversionMode::Hiragana => karukan_engine::kana::katakana_to_hiragana(
                &karukan_engine::kana::normalize_nfkc(text),
            ),
            DirectConversionMode::KatakanaFullwidth => karukan_engine::kana::hiragana_to_katakana(
                &karukan_engine::kana::katakana_to_hiragana(&karukan_engine::kana::normalize_nfkc(
                    text,
                )),
            ),
            DirectConversionMode::KatakanaHalfwidth => {
                karukan_engine::kana::kana_to_halfwidth_katakana(text)
            }
            DirectConversionMode::AlphabetFullwidth(_)
            | DirectConversionMode::AlphabetHalfwidth(_) => text.to_string(),
        }
    }

    pub(super) fn convert_direct_raw_text(&self, text: &str, mode: DirectConversionMode) -> String {
        match mode {
            DirectConversionMode::AlphabetFullwidth(case) => {
                karukan_engine::kana::ascii_to_fullwidth(&Self::apply_direct_alphabet_case(
                    text, case,
                ))
            }
            DirectConversionMode::AlphabetHalfwidth(case) => Self::apply_direct_alphabet_case(
                &karukan_engine::kana::ascii_to_halfwidth(text),
                case,
            ),
            DirectConversionMode::Hiragana
            | DirectConversionMode::KatakanaFullwidth
            | DirectConversionMode::KatakanaHalfwidth => self.convert_direct_kana_text(text, mode),
        }
    }

    fn direct_conversion_display(&self, mode: DirectConversionMode) -> (String, usize) {
        let (before, buffer, after) = match mode {
            DirectConversionMode::AlphabetFullwidth(_)
            | DirectConversionMode::AlphabetHalfwidth(_) => self.current_raw_parts(),
            DirectConversionMode::Hiragana
            | DirectConversionMode::KatakanaFullwidth
            | DirectConversionMode::KatakanaHalfwidth => self.current_input_parts(),
        };
        let display_before = self.convert_direct_raw_text(&format!("{}{}", before, buffer), mode);
        let display_after = self.convert_direct_raw_text(&after, mode);
        let caret = display_before.chars().count();
        (format!("{}{}", display_before, display_after), caret)
    }

    pub(super) fn direct_commit_text(&self) -> Option<String> {
        let mode = self.direct_mode?;
        let (before, buffer, after) = match mode {
            DirectConversionMode::AlphabetFullwidth(_)
            | DirectConversionMode::AlphabetHalfwidth(_) => self.current_raw_parts(),
            DirectConversionMode::Hiragana
            | DirectConversionMode::KatakanaFullwidth
            | DirectConversionMode::KatakanaHalfwidth => self.current_input_parts(),
        };
        Some(self.convert_direct_raw_text(&format!("{}{}{}", before, buffer, after), mode))
    }

    /// Build display text from the input buffer and romaji buffer
    /// Format: composed[:cursor] + romaji_buffer + composed[cursor:]
    /// In katakana mode, the composed parts are converted to katakana.
    pub(super) fn build_input_display(&self) -> String {
        let (before, buffer, after) = self.current_input_parts();

        let katakana = self.input_mode == InputMode::Katakana;
        let display_before = if katakana {
            Self::hiragana_to_katakana(&before)
        } else {
            before
        };
        let display_after = if katakana {
            Self::hiragana_to_katakana(&after)
        } else {
            after
        };

        format!("{}{}{}", display_before, buffer, display_after)
    }

    /// Get the caret position in the display text (in characters)
    pub(super) fn display_caret_position(&self) -> usize {
        self.input_buf.cursor_pos + self.converters.romaji.buffer().chars().count()
    }

    /// Build a preedit for composing state.
    /// If live conversion text is present, shows live_text + romaji_buffer with caret at end.
    /// Otherwise shows the input buffer display with cursor-based caret.
    pub(super) fn build_composing_preedit(&self) -> Preedit {
        let (display, caret) = if let Some(mode) = self.direct_mode {
            self.direct_conversion_display(mode)
        } else if !self.live.text.is_empty() {
            let buffer = self.converters.romaji.buffer();
            let display = format!("{}{}", self.live.text, buffer);
            let caret = display.chars().count();
            (display, caret)
        } else {
            (self.build_input_display(), self.display_caret_position())
        };
        let len = display.chars().count();
        let mut preedit = Preedit::with_text(&display);
        preedit.set_caret(caret);
        preedit.set_attributes(vec![PreeditAttribute::underline(0, len)]);
        preedit
    }

    /// Get combined context display string (lctx: ... rctx: ...)
    /// Only displays context when surrounding text has been set from the editor.
    pub(super) fn display_context(&self) -> String {
        let max_len = self.config.display_context_len;
        if max_len == 0 {
            return String::new();
        }
        let ctx = self.surrounding_context.as_ref();

        let lctx = ctx.and_then(|c| c.left.as_deref()).map(|left| {
            let char_count = left.chars().count();
            if char_count > max_len {
                let start = char_count - max_len;
                format!("...{}", left.chars().skip(start).collect::<String>())
            } else {
                left.to_string()
            }
        });

        let rctx = ctx.and_then(|c| c.right.as_deref()).map(|right| {
            let char_count = right.chars().count();
            if char_count > max_len {
                format!("{}...", right.chars().take(max_len).collect::<String>())
            } else {
                right.to_string()
            }
        });

        match (lctx, rctx) {
            (Some(l), Some(r)) => format!("lctx: {} rctx: {}", l, r),
            (Some(l), None) => format!("lctx: {}", l),
            (None, Some(r)) => format!("rctx: {}", r),
            (None, None) => String::new(),
        }
    }

    /// Get the current mode indicator string
    pub(super) fn mode_indicator(&self) -> String {
        if let Some(mode) = self.direct_mode {
            let base = self.direct_mode_indicator(mode);
            return if self.live.enabled {
                format!("⚡{}", base)
            } else {
                base.to_string()
            };
        }

        let base = match self.input_mode {
            InputMode::Alphabet => "[A]",
            InputMode::Katakana => "[カ]",
            InputMode::Hiragana => "[あ]",
        };
        if self.live.enabled {
            format!("⚡{}", base)
        } else {
            base.to_string()
        }
    }

    /// Format aux text for composing input mode
    pub(super) fn format_aux_composing(&self) -> String {
        let ctx = self.display_context();
        let model = self.model_name();
        let indicator = self.mode_indicator();
        // Show reading + unconverted romaji buffer (e.g. "わせだd")
        let romaji_buf = self.converters.romaji.buffer();
        let reading = if self.input_buf.text.is_empty() && romaji_buf.is_empty() {
            String::new()
        } else {
            format!(" {}{}", self.input_buf.text, romaji_buf)
        };
        if ctx.is_empty() {
            format!("{}{} Karukan ({})", indicator, reading, model)
        } else {
            format!("{}{} Karukan ({}) | {}", indicator, reading, model, ctx)
        }
    }

    /// Get token count for a reading (returns None if converter not initialized)
    pub(super) fn get_token_count(&self, reading: &str) -> Option<usize> {
        self.converters
            .kanji
            .as_ref()
            .and_then(|c| c.count_input_tokens(reading).ok())
    }

    /// Get the display name of the model used for the last conversion
    /// Falls back to the static model name if no conversion has happened yet
    fn last_used_model(&self) -> String {
        if self.metrics.model_name.is_empty() {
            self.model_name()
        } else {
            self.metrics.model_name.clone()
        }
    }

    /// Format aux text for conversion mode
    pub(super) fn format_aux_conversion_with_page(
        &self,
        reading: &str,
        candidates: Option<&CandidateList>,
        active_segment: usize,
        total_segments: usize,
    ) -> String {
        let ctx = self.display_context();
        let timing = format!(
            "{}ms/{}ms",
            self.metrics.conversion_ms, self.metrics.process_key_ms
        );
        let model = self.last_used_model();
        let tokens = self
            .get_token_count(reading)
            .map(|t| format!("{}tok", t))
            .unwrap_or_default();
        let page_info = candidates
            .filter(|c| c.total_pages() > 1)
            .map(|c| format!(" ({}/{})", c.current_page() + 1, c.total_pages()))
            .unwrap_or_default();
        let source_label = candidates
            .and_then(|c| c.selected())
            .and_then(|c| c.annotation.as_deref())
            .filter(|a| !a.is_empty())
            .map(|a| format!(" | {}", a))
            .unwrap_or_default();
        let segment_info = if total_segments > 1 {
            format!(" [{}/{}]", active_segment + 1, total_segments)
        } else {
            String::new()
        };
        if ctx.is_empty() {
            format!(
                "[変換]{}{} {} | {} {} | {}{}",
                segment_info, page_info, reading, timing, tokens, model, source_label
            )
        } else {
            format!(
                "[変換]{}{} {} | {} | {} {} | {}{}",
                segment_info, page_info, reading, ctx, timing, tokens, model, source_label
            )
        }
    }

    /// Format aux text for auto-suggest mode
    /// Note: token count is not shown here to avoid performance overhead on every keystroke
    /// Timing shows inference_ms/process_key_ms (process_key_ms is from previous keystroke)
    pub(super) fn format_aux_suggest(&self, reading: &str) -> String {
        let ctx = self.display_context();
        let timing = format!(
            "{}ms/{}ms",
            self.metrics.conversion_ms, self.metrics.process_key_ms
        );
        let model = self.last_used_model();
        let indicator = self.mode_indicator();
        // Append unconverted romaji buffer to reading (e.g. "わせだ" + "d" → "わせだd")
        let romaji_buf = self.converters.romaji.buffer();
        let display_reading = if romaji_buf.is_empty() {
            reading.to_string()
        } else {
            format!("{}{}", reading, romaji_buf)
        };
        if ctx.is_empty() {
            format!("{} {} | {} | {}", indicator, display_reading, timing, model)
        } else {
            format!(
                "{} {} | ctx: {} | {} | {}",
                indicator, display_reading, ctx, timing, model
            )
        }
    }

    /// Convert hiragana string to katakana
    pub(super) fn hiragana_to_katakana(hiragana: &str) -> String {
        karukan_engine::kana::hiragana_to_katakana(hiragana)
    }

    /// Truncate context to safe size for API calls
    pub(super) fn truncate_context_for_api(&self) -> String {
        match self
            .surrounding_context
            .as_ref()
            .and_then(|ctx| ctx.left.as_deref())
        {
            Some(left) => self.truncate_context(left),
            None => String::new(),
        }
    }

    /// Truncate a context string to safe size for API calls
    pub(super) fn truncate_context(&self, context: &str) -> String {
        let char_count = context.chars().count();
        if char_count > self.config.max_api_context_len {
            let start = char_count - self.config.max_api_context_len;
            context.chars().skip(start).collect()
        } else {
            context.to_string()
        }
    }
}
