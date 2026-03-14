use super::*;
use crate::core::keycode::KeyBinding;

fn make_single_segment_conversion(reading: &str, surface: &str) -> InputMethodEngine {
    let mut engine = InputMethodEngine::new();
    let candidates = CandidateList::from_strings_with_reading([surface], reading);
    engine.input_buf.text = reading.to_string();
    engine.input_buf.cursor_pos = reading.chars().count();
    engine.state = InputState::Conversion {
        preedit: Preedit::with_text(surface),
        candidates: candidates.clone(),
        session: ConversionSession {
            reading: reading.to_string(),
            segments: vec![ConversionSegment {
                reading_start: 0,
                reading_end: reading.chars().count(),
                candidates,
            }],
            active_segment: 0,
            segmentation_applied: false,
            enter_segments: true,
        },
    };
    engine
}

#[test]
fn test_conversion_char_commits_and_continues() {
    let mut engine = InputMethodEngine::new();

    // Type "あい" and enter conversion
    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.process_key(&press_key(Keysym::SPACE));
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    // Type 'k' during conversion → should commit candidate and start new input
    let result = engine.process_key(&press('k'));
    assert!(result.consumed);

    // Should have committed the conversion
    let has_commit = result
        .actions
        .iter()
        .any(|a| matches!(a, EngineAction::Commit(_)));
    assert!(has_commit, "Should have a commit action");

    // Should now be in Composing with 'k' in preedit
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "k");
}

#[test]
fn test_conversion_char_commits_and_continues_romaji() {
    let mut engine = InputMethodEngine::new();

    // Type "あ" and enter conversion
    engine.process_key(&press('a'));
    engine.process_key(&press_key(Keysym::SPACE));
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    // Type 'k', 'a' → commits conversion, then starts "か"
    engine.process_key(&press('k'));
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "k");

    engine.process_key(&press('a'));
    assert_eq!(engine.preedit().unwrap().text(), "か");
}

#[test]
fn test_alphabet_mode_space_inserts_literal_space() {
    let mut engine = InputMethodEngine::new();

    // Enter alphabet mode via Shift+N
    engine.process_key(&press_shift('N'));
    assert!(engine.input_mode == InputMode::Alphabet);

    // Type "ew"
    engine.process_key(&press('e'));
    engine.process_key(&press('w'));
    assert_eq!(engine.preedit().unwrap().text(), "New");

    // Space → should insert literal space, NOT start conversion
    engine.process_key(&press_key(Keysym::SPACE));
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "New ");

    // Type "york"
    engine.process_key(&press('y'));
    engine.process_key(&press('o'));
    engine.process_key(&press('r'));
    engine.process_key(&press('k'));
    assert_eq!(engine.preedit().unwrap().text(), "New york");
}

#[test]
fn test_conversion_starts_with_single_segment() {
    let engine = make_single_segment_conversion("きょうは", "今日は");

    let session = engine.state().conversion_session().unwrap();
    assert_eq!(session.segments.len(), 1);
    assert!(!session.segmentation_applied);
}

#[test]
fn test_right_triggers_delayed_segmentation() {
    let mut engine =
        make_single_segment_conversion("きょうはいいてんきですね", "今日はいい天気ですね");
    let result = engine.process_key(&press_key(Keysym::RIGHT));
    assert!(result.consumed);

    let session = engine.state().conversion_session().unwrap();
    assert_eq!(session.segments.len(), 3);
    assert!(session.segmentation_applied);
    assert_eq!(session.segments[0].reading_start, 0);
    assert_eq!(session.segments[0].reading_end, 4);
    assert_eq!(session.segments[1].reading_start, 4);
    assert_eq!(session.segments[1].reading_end, 6);
    assert_eq!(session.segments[2].reading_start, 6);
    assert_eq!(session.segments[2].reading_end, 12);
}

#[test]
fn test_right_moves_across_delayed_segments() {
    let mut engine =
        make_single_segment_conversion("きょうはいいてんきですね", "今日はいい天気ですね");
    engine.process_key(&press_key(Keysym::RIGHT));
    engine.process_key(&press_key(Keysym::RIGHT));
    let result = engine.process_key(&press_key(Keysym::RIGHT));
    assert!(result.consumed);

    let session = engine.state().conversion_session().unwrap();
    assert_eq!(session.active_segment, 2);
}

#[test]
fn test_left_triggers_delayed_segmentation_without_moving_past_first_segment() {
    let mut engine =
        make_single_segment_conversion("きょうはいいてんきですね", "今日はいい天気ですね");
    let result = engine.process_key(&press_key(Keysym::LEFT));
    assert!(result.consumed);

    let session = engine.state().conversion_session().unwrap();
    assert_eq!(session.segments.len(), 3);
    assert_eq!(session.active_segment, 0);
    assert!(session.segmentation_applied);
}

#[test]
fn test_digit_selection_in_conversion_does_not_commit() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('a'));
    engine.process_key(&press_key(Keysym::SPACE));
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    let result = engine.process_key(&press_key(Keysym::KEY_1));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));
    assert!(
        !result
            .actions
            .iter()
            .any(|action| matches!(action, EngineAction::Commit(_)))
    );
}

#[test]
fn test_shift_left_splits_conversion_segment() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('k'));
    engine.process_key(&press('a'));
    engine.process_key(&press('n'));
    engine.process_key(&press('a'));
    engine.process_key(&press_key(Keysym::SPACE));

    let result = engine.process_key(&press_shift_key(Keysym::LEFT));
    assert!(result.consumed);

    let session = engine.state().conversion_session().unwrap();
    assert_eq!(session.segments.len(), 2);
    assert_eq!(session.active_segment, 0);
    assert_eq!(engine.preedit().unwrap().text(), "かな");
}

#[test]
fn test_shift_right_merges_back_split_segment() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('k'));
    engine.process_key(&press('a'));
    engine.process_key(&press('n'));
    engine.process_key(&press('a'));
    engine.process_key(&press_key(Keysym::SPACE));
    engine.process_key(&press_shift_key(Keysym::LEFT));

    let result = engine.process_key(&press_shift_key(Keysym::RIGHT));
    assert!(result.consumed);

    let session = engine.state().conversion_session().unwrap();
    assert_eq!(session.segments.len(), 1);
    assert_eq!(engine.preedit().unwrap().text(), "かな");
}

#[test]
fn test_right_moves_active_segment() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('k'));
    engine.process_key(&press('a'));
    engine.process_key(&press('n'));
    engine.process_key(&press('a'));
    engine.process_key(&press_key(Keysym::SPACE));
    engine.process_key(&press_shift_key(Keysym::LEFT));

    let result = engine.process_key(&press_key(Keysym::RIGHT));
    assert!(result.consumed);

    let session = engine.state().conversion_session().unwrap();
    assert_eq!(session.active_segment, 1);
}

#[test]
fn test_enter_moves_to_next_segment() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('k'));
    engine.process_key(&press('a'));
    engine.process_key(&press('n'));
    engine.process_key(&press('a'));
    engine.process_key(&press_key(Keysym::SPACE));
    engine.process_key(&press_shift_key(Keysym::LEFT));

    let result = engine.process_key(&press_key(Keysym::RETURN));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));
    assert!(
        !result
            .actions
            .iter()
            .any(|action| matches!(action, EngineAction::Commit(_)))
    );

    let session = engine.state().conversion_session().unwrap();
    assert_eq!(session.active_segment, 1);
}

#[test]
fn test_enter_triggers_delayed_segmentation() {
    let mut engine =
        make_single_segment_conversion("きょうはいいてんきですね", "今日はいい天気ですね");

    let result = engine.process_key(&press_key(Keysym::RETURN));
    assert!(result.consumed);

    let session = engine.state().conversion_session().unwrap();
    assert_eq!(session.segments.len(), 3);
    assert!(session.segmentation_applied);
    assert_eq!(session.active_segment, 1);
}

#[test]
fn test_tab_then_enter_commits_without_delayed_segmentation() {
    let mut engine =
        make_single_segment_conversion("きょうはいいてんきですね", "今日はいい天気ですね");

    let tab = engine.process_key(&press_key(Keysym::TAB));
    assert!(tab.consumed);

    let result = engine.process_key(&press_key(Keysym::RETURN));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Empty));
    assert!(
        result
            .actions
            .iter()
            .any(|action| matches!(action, EngineAction::Commit(_)))
    );
}

#[test]
fn test_enter_commits_on_last_segment() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('k'));
    engine.process_key(&press('a'));
    engine.process_key(&press('n'));
    engine.process_key(&press('a'));
    engine.process_key(&press_key(Keysym::SPACE));
    engine.process_key(&press_shift_key(Keysym::LEFT));
    engine.process_key(&press_key(Keysym::RETURN));

    let result = engine.process_key(&press_key(Keysym::RETURN));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Empty));
    assert!(
        result
            .actions
            .iter()
            .any(|action| matches!(action, EngineAction::Commit(text) if text == "かな"))
    );
}

#[test]
fn test_f7_applies_to_active_segment_only_in_conversion() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('k'));
    engine.process_key(&press('a'));
    engine.process_key(&press('n'));
    engine.process_key(&press('a'));
    engine.process_key(&press_key(Keysym::SPACE));
    engine.process_key(&press_shift_key(Keysym::LEFT));

    let result = engine.process_key(&press_key(Keysym::F7));
    assert!(result.consumed);
    assert_eq!(engine.preedit().unwrap().text(), "カな");

    engine.process_key(&press_key(Keysym::RETURN));
    let commit = engine.process_key(&press_key(Keysym::RETURN));
    assert!(
        commit
            .actions
            .iter()
            .any(|action| matches!(action, EngineAction::Commit(text) if text == "カな"))
    );
}

#[test]
fn test_custom_segment_resize_key_binding() {
    let mut engine = InputMethodEngine::with_config(EngineConfig {
        segment_shrink_key: KeyBinding {
            keysym: Keysym::LEFT,
            shift: false,
            control: true,
            alt: false,
            super_key: false,
        },
        segment_expand_key: KeyBinding {
            keysym: Keysym::RIGHT,
            shift: false,
            control: true,
            alt: false,
            super_key: false,
        },
        ..EngineConfig::default()
    });

    engine.process_key(&press('k'));
    engine.process_key(&press('a'));
    engine.process_key(&press('n'));
    engine.process_key(&press('a'));
    engine.process_key(&press_key(Keysym::SPACE));

    let result = engine.process_key(&press_ctrl_key(Keysym::LEFT));
    assert!(result.consumed);

    let session = engine.state().conversion_session().unwrap();
    assert_eq!(session.segments.len(), 2);
    assert!(session.segmentation_applied);
}
