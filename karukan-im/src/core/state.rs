//! Input state machine
//!
//! Defines the states of the IME and transitions between them.

use super::candidate::CandidateList;
use super::preedit::Preedit;

/// Conversion state for one segment.
#[derive(Debug, Clone)]
pub struct ConversionSegment {
    /// Start position in the original reading (character index).
    pub reading_start: usize,
    /// End position in the original reading (character index, exclusive).
    pub reading_end: usize,
    /// Candidates for this segment.
    pub candidates: CandidateList,
    /// Whether the user explicitly changed the candidate for this segment.
    pub explicit_candidate_selection: bool,
}

impl ConversionSegment {
    /// Get the current surface text for this segment.
    pub fn selected_text(&self) -> &str {
        self.candidates.selected_text().unwrap_or("")
    }
}

/// Conversion session containing all bunsetsu segments.
#[derive(Debug, Clone)]
pub struct ConversionSession {
    /// Original reading for the whole conversion.
    pub reading: String,
    /// Segments in reading order.
    pub segments: Vec<ConversionSegment>,
    /// Active segment index.
    pub active_segment: usize,
    /// Whether bunsetsu segmentation has already been applied.
    pub segmentation_applied: bool,
    /// Whether Enter can still trigger delayed bunsetsu navigation.
    pub enter_segments: bool,
}

impl ConversionSession {
    /// Get the selected surface for the whole conversion.
    pub fn composed_text(&self) -> String {
        self.segments
            .iter()
            .map(ConversionSegment::selected_text)
            .collect()
    }
}

/// The current state of the IME
#[derive(Debug, Clone, Default)]
pub enum InputState {
    /// No input, waiting for user to type
    #[default]
    Empty,

    /// Composing mode - building preedit text (hiragana, katakana, or alphabet)
    Composing {
        /// The preedit string being composed
        preedit: Preedit,
        /// Unconverted romaji buffer (e.g., "k" waiting for next char)
        romaji_buffer: String,
    },

    /// Conversion mode - selecting from candidates
    Conversion {
        /// The preedit string showing conversion result
        preedit: Preedit,
        /// List of conversion candidates for the active segment
        candidates: CandidateList,
        /// Bunsetsu conversion session
        session: ConversionSession,
    },
}

impl InputState {
    /// Check if the engine is in the Empty (idle) state
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Get the current preedit if any
    pub fn preedit(&self) -> Option<&Preedit> {
        match self {
            Self::Empty => None,
            Self::Composing { preedit, .. } => Some(preedit),
            Self::Conversion { preedit, .. } => Some(preedit),
        }
    }

    /// Get mutable reference to preedit
    pub fn preedit_mut(&mut self) -> Option<&mut Preedit> {
        match self {
            Self::Empty => None,
            Self::Composing { preedit, .. } => Some(preedit),
            Self::Conversion { preedit, .. } => Some(preedit),
        }
    }

    /// Get candidates in conversion state
    pub fn candidates(&self) -> Option<&CandidateList> {
        match self {
            Self::Conversion { candidates, .. } => Some(candidates),
            _ => None,
        }
    }

    /// Get mutable reference to candidates
    pub fn candidates_mut(&mut self) -> Option<&mut CandidateList> {
        match self {
            Self::Conversion { candidates, .. } => Some(candidates),
            _ => None,
        }
    }

    /// Get conversion session in conversion state.
    pub fn conversion_session(&self) -> Option<&ConversionSession> {
        match self {
            Self::Conversion { session, .. } => Some(session),
            _ => None,
        }
    }

    /// Get mutable conversion session in conversion state.
    pub fn conversion_session_mut(&mut self) -> Option<&mut ConversionSession> {
        match self {
            Self::Conversion { session, .. } => Some(session),
            _ => None,
        }
    }
}
