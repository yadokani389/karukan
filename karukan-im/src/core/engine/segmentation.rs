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
    reading: String,
    attach_to_previous: bool,
    attach_to_next: bool,
}

#[derive(Debug, Clone)]
pub(super) struct BunsetsuSpan {
    pub reading: String,
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
            reading,
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

    fn group_tokens_into_bunsetsu(tokens: Vec<MorphToken>) -> Vec<BunsetsuSpan> {
        let mut bunsetsu = Vec::new();
        let mut current_reading = String::new();
        let mut current_attaches_next = false;

        for token in tokens {
            let should_join =
                !current_reading.is_empty() && (token.attach_to_previous || current_attaches_next);
            if !should_join && !current_reading.is_empty() {
                bunsetsu.push(BunsetsuSpan {
                    reading: std::mem::take(&mut current_reading),
                });
            }
            current_reading.push_str(&token.reading);
            current_attaches_next = token.attach_to_next;
        }

        if !current_reading.is_empty() {
            bunsetsu.push(BunsetsuSpan {
                reading: current_reading,
            });
        }

        bunsetsu
    }

    pub(super) fn segment_surface_to_bunsetsu(
        &mut self,
        surface: &str,
    ) -> Result<Vec<BunsetsuSpan>> {
        let tokens = self.lindera_tokens(surface)?;
        Ok(Self::group_tokens_into_bunsetsu(tokens))
    }

    pub(super) fn segment_surface_to_ranges(
        &mut self,
        surface: &str,
        reading: &str,
    ) -> Result<Vec<(usize, usize)>> {
        let bunsetsu = self.segment_surface_to_bunsetsu(surface)?;
        let mut ranges = Vec::with_capacity(bunsetsu.len());
        let mut start = 0;

        for span in bunsetsu {
            let segment_reading = span.reading;
            let len = segment_reading.chars().count();
            let end = start + len;
            if Self::slice_chars(reading, start, end) != segment_reading {
                return Err(anyhow!(
                    "bunsetsu reading mismatch: expected '{}', got '{}'",
                    Self::slice_chars(reading, start, end),
                    segment_reading
                ));
            }
            ranges.push((start, end));
            start = end;
        }

        if start != reading.chars().count() {
            return Err(anyhow!("bunsetsu reading did not cover full input"));
        }

        Ok(ranges)
    }
}
