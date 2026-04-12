//! Automatic bunsetsu segmentation backed by Lindera.

use anyhow::{Result, anyhow};
use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera::token::Token;
use lindera::tokenizer::Tokenizer;

use super::*;

#[derive(Debug, Clone)]
struct MorphToken {
    surface: String,
    reading: String,
    learnable: bool,
    attach_to_previous: bool,
    attach_to_next: bool,
}

#[derive(Debug, Clone)]
pub(super) struct LearningSpan {
    pub surface: String,
    pub reading: String,
    pub start: usize,
    pub end: usize,
    pub learnable: bool,
}

impl MorphToken {
    fn from_token(token: &mut Token<'_>) -> Option<Self> {
        let surface = token.surface.to_string();
        let details = token.details();
        let pos1 = details.first().copied().unwrap_or("*");
        let pos2 = details.get(1).copied().unwrap_or("*");
        let reading = details
            .get(7)
            .copied()
            .filter(|reading| *reading != "*")
            .map(karukan_engine::kana::katakana_to_hiragana)
            .or_else(|| Self::reading_from_surface(&surface))?;

        Some(Self {
            surface,
            reading,
            learnable: !matches!(pos1, "助詞" | "助動詞" | "記号" | "接頭詞") && pos2 != "接尾",
            attach_to_previous: matches!(pos1, "助詞" | "助動詞" | "記号") || pos2 == "接尾",
            attach_to_next: pos1 == "接頭詞",
        })
    }

    fn reading_from_surface(surface: &str) -> Option<String> {
        let normalized = karukan_engine::kana::katakana_to_hiragana(surface);
        normalized
            .chars()
            .all(|ch| matches!(ch, 'ぁ'..='ゖ' | 'ゝ' | 'ゞ' | 'ー'))
            .then_some(normalized)
    }
}

impl InputMethodEngine {
    pub(super) fn init_bunsetsu_tokenizer(&mut self) -> Result<()> {
        if self.converters.bunsetsu_tokenizer.is_some() {
            return Ok(());
        }

        let dictionary = load_dictionary("embedded://ipadic")?;
        let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
        self.converters.bunsetsu_tokenizer = Some(Tokenizer::new(segmenter));
        Ok(())
    }

    fn lindera_tokens(&mut self, surface: &str) -> Result<Vec<MorphToken>> {
        self.init_bunsetsu_tokenizer()?;
        let tokenizer = self
            .converters
            .bunsetsu_tokenizer
            .as_ref()
            .ok_or_else(|| anyhow!("bunsetsu tokenizer is not initialized"))?;
        let mut tokens = tokenizer.tokenize(surface)?;
        tokens
            .iter_mut()
            .map(|token| {
                MorphToken::from_token(token).ok_or_else(|| {
                    anyhow!(
                        "missing reading for token '{}': cannot build bunsetsu",
                        token.surface
                    )
                })
            })
            .collect()
    }

    pub(super) fn segment_surface_to_ranges(
        &mut self,
        surface: &str,
        reading: &str,
    ) -> Result<Vec<(usize, usize)>> {
        let tokens = self.lindera_tokens(surface)?;
        let mut ranges = Vec::new();
        let mut start = 0;
        let mut current_len = 0;
        let mut current_attaches_next = false;

        for token in tokens {
            let should_join =
                current_len > 0 && (token.attach_to_previous || current_attaches_next);
            if !should_join && current_len > 0 {
                let end = start + current_len;
                ranges.push((start, end));
                start = end;
                current_len = 0;
            }
            current_len += token.reading.chars().count();
            current_attaches_next = token.attach_to_next;
        }

        if current_len > 0 {
            let end = start + current_len;
            if end > reading.chars().count() {
                return Err(anyhow!("bunsetsu reading exceeded full input"));
            }
            ranges.push((start, end));
            start = end;
        }

        if start != reading.chars().count() {
            return Err(anyhow!("bunsetsu reading did not cover full input"));
        }

        Ok(ranges)
    }

    pub(super) fn segment_surface_to_learning_spans(
        &mut self,
        surface: &str,
        reading: &str,
    ) -> Result<Vec<LearningSpan>> {
        let tokens = self.lindera_tokens(surface)?;
        let mut spans = Vec::with_capacity(tokens.len());
        let mut start = 0;
        let reading_len = reading.chars().count();

        for token in tokens {
            let len = token.reading.chars().count();
            let end = start + len;
            if end > reading_len {
                return Err(anyhow!(
                    "learning spans exceeded input reading length at {}..{}",
                    start,
                    end
                ));
            }
            let span_reading = Self::slice_chars(reading, start, end);
            if span_reading.is_empty() {
                return Err(anyhow!(
                    "learning spans did not align with input reading at {}..{}",
                    start,
                    end
                ));
            }
            spans.push(LearningSpan {
                surface: token.surface,
                reading: span_reading,
                start,
                end,
                learnable: token.learnable,
            });
            start = end;
        }

        if start != reading_len {
            return Err(anyhow!("learning spans did not cover full input"));
        }

        Ok(spans)
    }
}
