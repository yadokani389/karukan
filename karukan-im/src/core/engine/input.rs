//! Composing input handling (Empty and Composing states)

use karukan_engine::ConversionEvent;

use super::*;

fn ascii_symbol_to_fullwidth(c: char) -> Option<char> {
    match c {
        '!'..='~' if c.is_ascii_punctuation() => std::char::from_u32(c as u32 + 0xfee0),
        _ => None,
    }
}

impl InputMethodEngine {
    fn normalize_input_symbol(&self, ch: char) -> char {
        if !ch.is_ascii() {
            return ch;
        }

        match ch {
            ',' => {
                if self.config.japanese_punctuation {
                    '、'
                } else if self.config.fullwidth_comma {
                    '，'
                } else {
                    ','
                }
            }
            '.' => {
                if self.config.japanese_punctuation {
                    '。'
                } else if self.config.fullwidth_period {
                    '．'
                } else {
                    '.'
                }
            }
            '/' => {
                if self.config.japanese_punctuation {
                    '・'
                } else if self.config.fullwidth_symbols {
                    '／'
                } else {
                    '/'
                }
            }
            '[' => {
                if self.config.japanese_punctuation {
                    '「'
                } else if self.config.fullwidth_symbols {
                    '［'
                } else {
                    '['
                }
            }
            ']' => {
                if self.config.japanese_punctuation {
                    '」'
                } else if self.config.fullwidth_symbols {
                    '］'
                } else {
                    ']'
                }
            }
            '-' => {
                if self.config.japanese_punctuation {
                    'ー'
                } else if self.config.fullwidth_symbols {
                    '－'
                } else {
                    '-'
                }
            }
            '~' => {
                if self.config.fullwidth_symbols {
                    '～'
                } else {
                    '~'
                }
            }
            _ => {
                if ch.is_ascii_punctuation() {
                    if self.config.fullwidth_symbols {
                        ascii_symbol_to_fullwidth(ch).unwrap_or(ch)
                    } else {
                        ch
                    }
                } else {
                    ch
                }
            }
        }
    }

    pub(super) fn normalize_input_text(&self, text: &str) -> String {
        text.chars()
            .map(|ch| self.normalize_input_symbol(ch))
            .collect()
    }

    fn normalize_converted_text(&self, raw: &str, output: &str) -> String {
        if raw.chars().count() != output.chars().count() {
            return output.to_string();
        }

        raw.chars()
            .zip(output.chars())
            .map(|(raw_ch, output_ch)| {
                if (output_ch == raw_ch && raw_ch.is_ascii_punctuation())
                    || matches!(raw_ch, ',' | '.' | '/' | '[' | ']' | '-')
                {
                    self.normalize_input_symbol(raw_ch)
                } else {
                    output_ch
                }
            })
            .collect()
    }

    /// Refresh the input state: rebuild preedit and run auto-suggest for candidates.
    pub(super) fn refresh_input_state(&mut self) -> EngineResult {
        if self.direct_mode.is_some() {
            let preedit = self.set_composing_state();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::HideCandidates)
                .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()));
        }

        // Alphabet mode with active live conversion: preserve the conversion display
        if self.input_mode == InputMode::Alphabet && !self.live.text.is_empty() {
            let preedit = self.set_composing_state();
            return EngineResult::consumed().with_action(EngineAction::UpdatePreedit(preedit));
        }

        let suggestions =
            if self.input_mode != InputMode::Alphabet && !self.input_buf.text.is_empty() {
                let reading = self.input_buf.text.clone();
                let live_text = self.build_live_conversion_text(&reading);
                let ranked_candidates = self.build_exact_conversion_ranked_candidates(
                    &reading,
                    self.config
                        .num_candidates
                        .max(CandidateList::DEFAULT_PAGE_SIZE),
                );
                let has_non_fallback = ranked_candidates
                    .iter()
                    .any(|candidate| candidate.source() != CandidateSource::Fallback);
                Some((reading, live_text, ranked_candidates, has_non_fallback))
            } else {
                None
            };

        let Some((reading, live_text, ranked_candidates, has_non_fallback)) = suggestions else {
            self.live.text.clear();
            let preedit = self.set_composing_state();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::HideCandidates)
                .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()));
        };

        if !has_non_fallback {
            self.live.text.clear();
            let preedit = self.set_composing_state();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::HideCandidates)
                .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()));
        }

        let mut annotated_candidates = self.annotate_candidates(ranked_candidates.clone());
        self.extend_live_candidates(&reading, live_text.as_deref(), &mut annotated_candidates);
        let candidates = self.build_candidate_list_from_annotated(&reading, annotated_candidates);

        // Live conversion mode: show converted text in preedit
        if self.live.enabled && self.input_mode != InputMode::Katakana {
            self.live.text = live_text
                .map(|text| self.normalize_input_text(&text))
                .unwrap_or_default();
            let preedit = self.set_composing_state();
            let result = EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::ShowCandidates(candidates.clone()));
            let aux = self.format_aux_suggest(&self.input_buf.text.clone());
            return result.with_action(EngineAction::UpdateAuxText(aux));
        }

        // Normal auto-suggest: show hiragana preedit + reranked candidates
        self.live.text.clear();
        let preedit = self.set_composing_state();
        let aux = self.format_aux_suggest(&self.input_buf.text.clone());
        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::ShowCandidates(candidates))
            .with_action(EngineAction::UpdateAuxText(aux))
    }

    /// Process key in empty state
    pub(super) fn process_key_empty(&mut self, key: &KeyEvent, shift_active: bool) -> EngineResult {
        // Ctrl+Space: start input with full-width space
        if key.modifiers.control_key && key.keysym == Keysym::SPACE {
            self.converters.romaji.reset();
            self.input_buf.clear();
            self.raw_units.clear();
            self.insert_raw_text(" ");
            self.input_buf.insert("\u{3000}");
            let preedit = self.set_composing_state();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()));
        }

        // Only handle printable characters without modifiers (except shift)
        if let Some(ch) = key.to_char()
            && !key.modifiers.control_key
            && !key.modifiers.alt_key
        {
            // Detect Shift+letter: shift modifier with alphabetic, OR uppercase keysym.
            // fcitx5 may resolve Shift into the keysym (sending 'A' instead of 'a'+shift),
            // so we must also check for uppercase to handle both cases.
            let is_shift_alpha =
                ch.is_ascii_uppercase() || (shift_active && ch.is_ascii_alphabetic());

            if is_shift_alpha && self.input_mode != InputMode::Alphabet {
                self.input_mode = InputMode::Alphabet;
            }
            let ch = if self.input_mode == InputMode::Alphabet && is_shift_alpha {
                ch.to_ascii_uppercase()
            } else {
                ch
            };
            return self.start_input(ch);
        }
        EngineResult::not_consumed()
    }

    /// Start input with a character (first character of a new input session).
    /// In alphabet mode, inserts directly; otherwise goes through romaji conversion.
    pub(super) fn start_input(&mut self, ch: char) -> EngineResult {
        self.converters.romaji.reset();
        self.input_buf.clear();
        self.raw_units.clear();
        self.direct_mode = None;

        if self.input_mode == InputMode::Alphabet {
            let text = self.normalize_input_text(&ch.to_string());
            self.insert_raw_text(&ch.to_string());
            self.input_buf.insert(&text);
        } else {
            let prev_buffer = self.converters.romaji.buffer().to_string();
            let prev_output_len = 0;
            let event = self.converters.romaji.push(ch);
            let romaji_buffer = self.converters.romaji.buffer().to_string();
            let combined_raw = format!("{}{}", prev_buffer, ch.to_ascii_lowercase());

            // Check for PassThrough FIRST: the converter adds PassThrough chars
            // to its output, so we must check the event before checking output emptiness.
            // Exception: digits should enter Composing so users can type "20世紀" etc.
            if let ConversionEvent::PassThrough(c) = event
                && !c.is_ascii_digit()
            {
                self.converters.romaji.reset();
                let text = self.normalize_input_text(&c.to_string());
                return EngineResult::consumed().with_action(EngineAction::Commit(text));
            }
            // For digits, fall through to enter Composing normally

            if self.converters.romaji.output().is_empty() && romaji_buffer.is_empty() {
                return EngineResult::not_consumed();
            }

            // Consume new converter output into composed_hiragana
            let new_output_len = self.converters.romaji.output().chars().count();
            if new_output_len > prev_output_len {
                let new_chars: String = self
                    .converters
                    .romaji
                    .output()
                    .chars()
                    .skip(prev_output_len)
                    .collect();
                let consumed_len = combined_raw
                    .chars()
                    .count()
                    .saturating_sub(romaji_buffer.chars().count());
                let consumed_raw: String = combined_raw.chars().take(consumed_len).collect();
                let new_chars = self.normalize_converted_text(&consumed_raw, &new_chars);
                self.insert_raw_chunks(Self::split_raw_chunks(&consumed_raw, &new_chars));
                self.input_buf.insert(&new_chars);
            }
        }

        let preedit = self.set_composing_state();

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()))
    }

    /// Insert a full-width space (U+3000) at cursor position
    pub(super) fn input_fullwidth_space(&mut self) -> EngineResult {
        self.insert_raw_text(" ");
        self.input_buf.insert("\u{3000}");
        self.direct_mode = None;
        self.refresh_input_state()
    }

    /// Process key in hiragana input state
    pub(super) fn process_key_composing(
        &mut self,
        key: &KeyEvent,
        shift_active: bool,
    ) -> EngineResult {
        if let Some(result) = self.handle_function_key(key.keysym) {
            return result;
        }

        // Handle Ctrl+key shortcuts
        if key.modifiers.control_key {
            match key.keysym {
                // Ctrl+Space: insert full-width space (U+3000)
                Keysym::SPACE => return self.input_fullwidth_space(),
                // Ctrl+K: enter katakana mode
                Keysym::KEY_K | Keysym::KEY_K_UPPER => return self.enter_katakana_mode(),
                // Ctrl+A: move to beginning (Emacs-style Home)
                Keysym::KEY_A | Keysym::KEY_A_UPPER => return self.move_caret_home(),
                // Ctrl+B: move left (Emacs-style Left)
                Keysym::KEY_B | Keysym::KEY_B_UPPER => return self.move_caret_left(),
                // Ctrl+E: move to end (Emacs-style End)
                Keysym::KEY_E | Keysym::KEY_E_UPPER => return self.move_caret_end(),
                // Ctrl+F: move right (Emacs-style Right)
                Keysym::KEY_F | Keysym::KEY_F_UPPER => return self.move_caret_right(),
                _ => {}
            }
        }

        match key.keysym {
            Keysym::RETURN => self.commit_composing(),
            Keysym::ESCAPE => self.cancel_composing(),
            Keysym::BACKSPACE => self.backspace_composing(),
            Keysym::DELETE => self.delete_composing(),
            Keysym::SPACE if self.input_mode == InputMode::Alphabet => self.input_char(' '),
            Keysym::SPACE | Keysym::DOWN | Keysym::TAB => self.start_conversion(),
            Keysym::LEFT => self.move_caret_left(),
            Keysym::RIGHT => self.move_caret_right(),
            Keysym::HOME => self.move_caret_home(),
            Keysym::END => self.move_caret_end(),
            _ => {
                if let Some(ch) = key.to_char()
                    && !key.modifiers.control_key
                    && !key.modifiers.alt_key
                {
                    if self.direct_mode.is_some() {
                        let is_shift_alpha =
                            ch.is_ascii_uppercase() || (shift_active && ch.is_ascii_alphabetic());
                        if is_shift_alpha && self.input_mode != InputMode::Alphabet {
                            self.input_mode = InputMode::Alphabet;
                        }
                        let ch = if self.input_mode == InputMode::Alphabet && is_shift_alpha {
                            ch.to_ascii_uppercase()
                        } else {
                            ch
                        };
                        return self.commit_direct_mode_and_continue(ch);
                    }

                    // Detect Shift+letter: shift modifier with alphabetic, OR uppercase keysym.
                    // fcitx5 may resolve Shift into the keysym (sending 'A' instead of 'a'+shift).
                    let is_shift_alpha =
                        ch.is_ascii_uppercase() || (shift_active && ch.is_ascii_alphabetic());

                    if is_shift_alpha && self.input_mode != InputMode::Alphabet {
                        // Bake katakana before switching so preedit doesn't revert
                        if self.input_mode == InputMode::Katakana {
                            self.bake_katakana();
                        }
                        self.input_mode = InputMode::Alphabet;
                        self.flush_romaji_to_composed();
                        self.live.text.clear();
                    }
                    let ch = if self.input_mode == InputMode::Alphabet && is_shift_alpha {
                        ch.to_ascii_uppercase()
                    } else {
                        ch
                    };
                    return self.input_char(ch);
                }
                EngineResult::not_consumed()
            }
        }
    }

    /// Input a character during composing.
    /// In alphabet mode, inserts directly; otherwise goes through romaji conversion.
    pub(super) fn input_char(&mut self, ch: char) -> EngineResult {
        if self.input_mode == InputMode::Alphabet {
            let text = self.normalize_input_text(&ch.to_string());
            self.insert_raw_text(&ch.to_string());
            self.input_buf.insert(&text);
            self.direct_mode = None;
            return self.refresh_input_state();
        }

        let prev_buffer = self.converters.romaji.buffer().to_string();
        let prev_output_len = self.converters.romaji.output().chars().count();
        let event = self.converters.romaji.push(ch);
        let curr_output_len = self.converters.romaji.output().chars().count();
        let romaji_buffer = self.converters.romaji.buffer().to_string();
        let combined_raw = format!("{}{}", prev_buffer, ch.to_ascii_lowercase());
        self.direct_mode = None;

        // Track whether composed_hiragana was empty before processing.
        // Used to decide if PassThrough chars should auto-commit (standalone punctuation)
        // or stay in preedit (punctuation after hiragana).
        let composed_was_empty = self.input_buf.text.is_empty();

        // Consume ALL new converter output into composed_hiragana at cursor position.
        // This must happen BEFORE PassThrough handling because the converter may
        // recursively pass through multiple chars (e.g., "thx" → output="th", buffer="x",
        // event=PassThrough('h')), and we need to capture all of them via delta detection.
        // PassThrough chars are already included in the converter output, so we must NOT
        // also insert them separately.
        if curr_output_len > prev_output_len {
            let new_chars: String = self
                .converters
                .romaji
                .output()
                .chars()
                .skip(prev_output_len)
                .collect();
            let consumed_len = combined_raw
                .chars()
                .count()
                .saturating_sub(romaji_buffer.chars().count());
            let consumed_raw: String = combined_raw.chars().take(consumed_len).collect();
            let new_chars = self.normalize_converted_text(&consumed_raw, &new_chars);
            self.insert_raw_chunks(Self::split_raw_chunks(&consumed_raw, &new_chars));
            self.input_buf.insert(&new_chars);
        }

        // Handle pass-through: only auto-commit when there was no previous hiragana
        // (standalone punctuation). If hiragana was already in the preedit, keep the
        // passthrough chars in composed_hiragana (already inserted by delta detection).
        // Exception: digits always stay in preedit to allow "20世紀" style input.
        if let ConversionEvent::PassThrough(c) = event
            && !c.is_ascii_digit()
            && composed_was_empty
            && romaji_buffer.is_empty()
        {
            let text = self.input_buf.text.clone();
            self.input_buf.clear();
            self.raw_units.clear();
            self.state = InputState::Empty;
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(Preedit::new()))
                .with_action(EngineAction::HideAuxText)
                .with_action(EngineAction::Commit(text));
        }

        if let Some(result) = self.try_reset_if_empty() {
            return result;
        }

        self.refresh_input_state()
    }

    /// Commit the current hiragana input (or katakana if in katakana mode)
    /// In live conversion mode, commits the converted text instead of hiragana.
    pub(super) fn commit_composing(&mut self) -> EngineResult {
        if self.direct_mode.is_some() {
            return self.commit_direct_mode();
        }

        // Flush any pending romaji into composed_hiragana
        self.flush_romaji_to_composed();

        let reading = self.input_buf.text.clone();
        let text = if self.input_mode == InputMode::Katakana {
            // Katakana mode always commits katakana, ignoring live conversion
            Self::hiragana_to_katakana(&reading)
        } else if !self.live.text.is_empty() {
            // Live conversion active: commit converted text
            self.live.text.clone()
        } else {
            reading.clone()
        };

        if text.is_empty() {
            self.state = InputState::Empty;
            self.input_buf.clear();
            self.raw_units.clear();
            self.live.text.clear();
            return EngineResult::consumed().with_action(EngineAction::HideAuxText);
        }

        self.converters.romaji.reset();
        self.input_buf.clear();
        self.raw_units.clear();
        self.live.text.clear();
        self.state = InputState::Empty;
        self.direct_mode = None;

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(Preedit::new()))
            .with_action(EngineAction::Commit(text))
            .with_action(EngineAction::HideAuxText)
    }

    /// Cancel the current input
    /// In live conversion mode: first Escape clears live conversion and shows hiragana,
    /// second Escape cancels input entirely.
    pub(super) fn cancel_composing(&mut self) -> EngineResult {
        if self.direct_mode.is_some() {
            self.direct_mode = None;
            self.live.text.clear();
            return self.refresh_input_state();
        }

        // If live conversion is active, first Escape returns to hiragana display
        if !self.live.text.is_empty() {
            self.live.text.clear();
            let preedit = self.set_composing_state();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::HideCandidates)
                .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()));
        }

        self.converters.romaji.reset();
        self.input_buf.clear();
        self.raw_units.clear();
        self.live.text.clear();
        self.state = InputState::Empty;
        self.direct_mode = None;

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(Preedit::new()))
            .with_action(EngineAction::HideCandidates)
            .with_action(EngineAction::HideAuxText)
    }
}
