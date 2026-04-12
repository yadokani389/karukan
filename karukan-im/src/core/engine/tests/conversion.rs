use super::*;
use crate::core::keycode::KeyBinding;
use karukan_engine::LearningCache;

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
                explicit_candidate_selection: false,
            }],
            active_segment: 0,
            segmentation_applied: false,
            enter_segments: true,
        },
    };
    engine
}

fn make_single_segment_conversion_with_candidates(
    reading: &str,
    candidates: Vec<Candidate>,
) -> InputMethodEngine {
    let mut engine = InputMethodEngine::new();
    let candidates = CandidateList::new(candidates);
    let preedit = Preedit::with_text(candidates.selected_text().unwrap_or(""));
    engine.input_buf.text = reading.to_string();
    engine.input_buf.cursor_pos = reading.chars().count();
    engine.state = InputState::Conversion {
        preedit,
        candidates: candidates.clone(),
        session: ConversionSession {
            reading: reading.to_string(),
            segments: vec![ConversionSegment {
                reading_start: 0,
                reading_end: reading.chars().count(),
                candidates,
                explicit_candidate_selection: false,
            }],
            active_segment: 0,
            segmentation_applied: false,
            enter_segments: true,
        },
    };
    engine
}

fn set_single_segment_conversion(engine: &mut InputMethodEngine, reading: &str, surface: &str) {
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
                explicit_candidate_selection: false,
            }],
            active_segment: 0,
            segmentation_applied: false,
            enter_segments: true,
        },
    };
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
fn test_shift_tab_moves_to_previous_candidate() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('a'));
    engine.process_key(&press_key(Keysym::SPACE));
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    let result = engine.process_key(&press_shift_key(Keysym::TAB));
    assert!(result.consumed);

    let candidates = engine.state().candidates().unwrap();
    assert_eq!(candidates.cursor(), candidates.len() - 1);
}

#[test]
fn test_iso_left_tab_moves_to_previous_candidate() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('a'));
    engine.process_key(&press_key(Keysym::SPACE));
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    let result = engine.process_key(&press_key(Keysym::ISO_LEFT_TAB));
    assert!(result.consumed);

    let candidates = engine.state().candidates().unwrap();
    assert_eq!(candidates.cursor(), candidates.len() - 1);
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

#[test]
fn test_prefix_commit_candidate_keeps_remaining_reading_in_preedit() {
    let mut engine = make_single_segment_conversion_with_candidates(
        "きょうはいいてんきですね",
        vec![
            Candidate::with_reading("今日はいい天気ですね", "きょうはいいてんきですね"),
            Candidate::with_reading("今日は", "きょうは")
                .with_prefix_commit("きょうは".chars().count())
                .with_index(1),
        ],
    );

    let result = engine.process_key(&press_key(Keysym::DOWN));
    assert!(result.consumed);
    assert_eq!(engine.preedit().unwrap().text(), "今日はいいてんきですね");
}

#[test]
fn test_enter_on_prefix_commit_candidate_commits_prefix_and_continues_conversion() {
    let mut engine = make_single_segment_conversion_with_candidates(
        "きょうはいいてんきですね",
        vec![
            Candidate::with_reading("今日はいい天気ですね", "きょうはいいてんきですね"),
            Candidate::with_reading("今日は", "きょうは")
                .with_prefix_commit("きょうは".chars().count())
                .with_index(1),
        ],
    );

    engine.process_key(&press_key(Keysym::DOWN));
    let result = engine.process_key(&press_key(Keysym::RETURN));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));
    assert!(
        result
            .actions
            .iter()
            .any(|action| matches!(action, EngineAction::Commit(text) if text == "今日は"))
    );

    let session = engine.state().conversion_session().unwrap();
    assert_eq!(session.reading, "いいてんきですね");
    assert_eq!(session.segments.len(), 1);
    assert_eq!(
        engine.preedit().unwrap().text(),
        session.segments[0].selected_text()
    );
}

#[test]
fn test_prefix_commit_candidates_reconvert_first_reading() {
    let mut engine = InputMethodEngine::new();
    let mut learning = LearningCache::new(16);
    learning.record("きょうは", "今日は");
    learning.record("きょうは", "教派");
    engine.learning = Some(learning);

    let session = engine.build_single_segment_session(
        "きょうはいいてんきですね",
        Some("今日はいい天気ですね".to_string()),
    );
    let texts: Vec<_> = session.segments[0]
        .candidates
        .candidates()
        .iter()
        .map(|candidate| candidate.text.clone())
        .collect();

    assert!(texts.iter().any(|text| text == "今日は"));
    assert!(texts.iter().any(|text| text == "教派"));
}

#[test]
fn test_enter_without_explicit_selection_does_not_learn() {
    let mut engine = make_single_segment_conversion_with_candidates(
        "きょうは",
        vec![
            Candidate::with_reading("今日は", "きょうは"),
            Candidate::with_reading("教派", "きょうは").with_index(1),
        ],
    );
    engine.learning = Some(LearningCache::new(16));

    let result = engine.process_key(&press_key(Keysym::RETURN));

    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Empty));
    assert!(
        engine
            .learning
            .as_ref()
            .unwrap()
            .lookup("きょうは")
            .is_empty()
    );
}

#[test]
fn test_enter_after_explicit_candidate_selection_learns() {
    let mut engine = make_single_segment_conversion_with_candidates(
        "きょうは",
        vec![
            Candidate::with_reading("今日は", "きょうは"),
            Candidate::with_reading("教派", "きょうは").with_index(1),
        ],
    );
    engine.learning = Some(LearningCache::new(16));

    engine.process_key(&press_key(Keysym::DOWN));
    let result = engine.process_key(&press_key(Keysym::RETURN));

    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Empty));
    let learned = engine.learning.as_ref().unwrap().lookup("きょうは");
    assert_eq!(learned.len(), 1);
    assert_eq!(learned[0].0, "教派");
}

#[test]
fn test_conversion_commit_and_continue_does_not_learn() {
    let mut engine = make_single_segment_conversion_with_candidates(
        "きょうは",
        vec![
            Candidate::with_reading("今日は", "きょうは"),
            Candidate::with_reading("教派", "きょうは").with_index(1),
        ],
    );
    engine.learning = Some(LearningCache::new(16));

    engine.process_key(&press_key(Keysym::DOWN));
    let result = engine.process_key(&press('k'));

    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert!(
        engine
            .learning
            .as_ref()
            .unwrap()
            .lookup("きょうは")
            .is_empty()
    );
}

#[test]
fn test_learning_candidates_require_exact_match() {
    let mut engine = InputMethodEngine::new();
    let mut learning = LearningCache::new(16);
    learning.record("きょうはいいてんきですね", "今日はいい天気ですね");
    engine.learning = Some(learning);

    assert!(engine.lookup_learning_candidates("きょう").is_empty());

    let exact = engine.lookup_learning_candidates("きょうはいいてんきですね");
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].text, "今日はいい天気ですね");
}

#[test]
fn test_repeated_default_commit_eventually_learns() {
    let mut engine = InputMethodEngine::new();
    engine.learning = Some(LearningCache::new(16));

    for _ in 0..2 {
        set_single_segment_conversion(&mut engine, "てんき", "天気");
        let result = engine.process_key(&press_key(Keysym::RETURN));
        assert!(result.consumed);
    }

    assert!(
        engine
            .learning
            .as_ref()
            .unwrap()
            .lookup_matches("てんき")
            .is_empty()
    );

    set_single_segment_conversion(&mut engine, "てんき", "天気");
    let result = engine.process_key(&press_key(Keysym::RETURN));
    assert!(result.consumed);

    let learned = engine.learning.as_ref().unwrap().lookup_matches("てんき");
    assert_eq!(learned.len(), 1);
    assert_eq!(learned[0].surface, "天気");
}

#[test]
fn test_user_dictionary_ranks_above_single_strong_learning() {
    let mut engine = InputMethodEngine::new();
    engine.dicts.user = Some(make_test_dictionary(
        r#"[
            {"reading":"あめ","candidates":[{"surface":"雨","score":0.0}]}
        ]"#,
    ));
    let mut learning = LearningCache::new(16);
    learning.record_strong("あめ", "飴");
    engine.learning = Some(learning);

    let candidates = engine.build_exact_conversion_candidates("あめ", 3);

    assert_eq!(candidates[0].text, "雨");
}

#[test]
fn test_sentence_commit_learns_content_words_only() {
    let mut engine = make_single_segment_conversion_with_candidates(
        "きょうはいいてんきです",
        vec![
            Candidate::with_reading("今日はいい天気です", "きょうはいいてんきです"),
            Candidate::with_reading("今日は良い天気です", "きょうはいいてんきです").with_index(1),
        ],
    );
    engine.learning = Some(LearningCache::new(32));

    engine.process_key(&press_key(Keysym::DOWN));
    let result = engine.process_key(&press_key(Keysym::RETURN));

    assert!(result.consumed);

    let learning = engine.learning.as_ref().unwrap();
    let ii = learning.lookup_matches("いい");
    assert_eq!(ii.len(), 1);
    assert_eq!(ii[0].surface, "良い");
    assert!(ii[0].strong_selections > 0);

    let kyou = learning.lookup_matches("きょう");
    assert_eq!(kyou.len(), 1);
    assert_eq!(kyou[0].surface, "今日");
    assert_eq!(kyou[0].strong_selections, 0);
    assert!(kyou[0].weak_accepts > 0);

    let tenki = learning.lookup_matches("てんき");
    assert_eq!(tenki.len(), 1);
    assert_eq!(tenki[0].surface, "天気");
    assert_eq!(tenki[0].strong_selections, 0);
    assert!(tenki[0].weak_accepts > 0);

    assert!(learning.lookup_matches("は").is_empty());
    assert!(learning.lookup_matches("です").is_empty());
}

#[test]
fn test_commit_api_does_not_learn_conversion_without_explicit_selection() {
    let mut engine = make_single_segment_conversion("きょうは", "今日は");
    engine.learning = Some(LearningCache::new(16));

    let text = engine.commit();

    assert_eq!(text, "今日は");
    assert!(
        engine
            .learning
            .as_ref()
            .unwrap()
            .lookup("きょうは")
            .is_empty()
    );
}
