use super::*;

use crate::core::candidate::CandidateList;
use crate::core::state::{ConversionSegment, ConversionSession};

#[test]
fn test_f7_converts_composing_to_fullwidth_katakana() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));

    let result = engine.process_key(&press_key(Keysym::F7));
    assert!(result.consumed);
    assert_eq!(engine.preedit().unwrap().text(), "アイ");

    let commit = engine.process_key(&press_key(Keysym::RETURN));
    assert!(
        commit
            .actions
            .iter()
            .any(|a| matches!(a, EngineAction::Commit(text) if text == "アイ"))
    );
}

#[test]
fn test_f8_converts_composing_to_halfwidth_katakana() {
    let mut engine = InputMethodEngine::new();

    for ch in "gakkou".chars() {
        engine.process_key(&press(ch));
    }

    let result = engine.process_key(&press_key(Keysym::F8));
    assert!(result.consumed);
    assert_eq!(engine.preedit().unwrap().text(), "ｶﾞｯｺｳ");

    let commit = engine.process_key(&press_key(Keysym::RETURN));
    assert!(
        commit
            .actions
            .iter()
            .any(|a| matches!(a, EngineAction::Commit(text) if text == "ｶﾞｯｺｳ"))
    );
}

#[test]
fn test_f10_uses_typed_raw_sequence() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('s'));
    engine.process_key(&press('i'));
    assert_eq!(engine.preedit().unwrap().text(), "し");

    let result = engine.process_key(&press_key(Keysym::F10));
    assert!(result.consumed);
    assert_eq!(engine.preedit().unwrap().text(), "si");

    let commit = engine.process_key(&press_key(Keysym::RETURN));
    assert!(
        commit
            .actions
            .iter()
            .any(|a| matches!(a, EngineAction::Commit(text) if text == "si"))
    );
}

#[test]
fn test_f9_and_f10_cycle_alphabet_case() {
    let mut engine = InputMethodEngine::new();

    for ch in "shi".chars() {
        engine.process_key(&press(ch));
    }

    engine.process_key(&press_key(Keysym::F10));
    assert_eq!(engine.preedit().unwrap().text(), "shi");

    engine.process_key(&press_key(Keysym::F10));
    assert_eq!(engine.preedit().unwrap().text(), "SHI");

    engine.process_key(&press_key(Keysym::F10));
    assert_eq!(engine.preedit().unwrap().text(), "Shi");

    engine.process_key(&press_key(Keysym::F9));
    assert_eq!(engine.preedit().unwrap().text(), "ｓｈｉ");

    engine.process_key(&press_key(Keysym::F9));
    assert_eq!(engine.preedit().unwrap().text(), "ＳＨＩ");
}

#[test]
fn test_f10_from_conversion_uses_raw_sequence() {
    let mut engine = InputMethodEngine::new();

    engine.input_buf.text = "し".to_string();
    engine.input_buf.cursor_pos = 0;
    engine.raw_units = vec!["si".to_string()];
    let candidates = CandidateList::from_strings_with_reading(["し", "シ"], "し");
    engine.state = InputState::Conversion {
        preedit: Preedit::with_text("し"),
        candidates: candidates.clone(),
        session: ConversionSession {
            reading: "し".to_string(),
            segments: vec![ConversionSegment {
                reading_start: 0,
                reading_end: 1,
                candidates,
                explicit_candidate_selection: false,
            }],
            active_segment: 0,
            segmentation_applied: true,
            enter_segments: false,
        },
    };

    let result = engine.process_key(&press_key(Keysym::F10));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "si");
}
