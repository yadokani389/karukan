use super::*;
use karukan_engine::LearningCache;

// --- Live conversion tests ---

#[test]
fn test_live_conversion_disabled_by_default() {
    let engine = InputMethodEngine::new();
    assert!(!engine.live.enabled);
}

#[test]
fn test_live_conversion_respects_config_default() {
    let engine = InputMethodEngine::with_config(EngineConfig {
        live_conversion: true,
        ..EngineConfig::default()
    });
    assert!(engine.live.enabled);
}

#[test]
fn test_live_conversion_enabled() {
    let engine = make_live_conversion_engine();
    assert!(engine.live.enabled);
}

#[test]
fn test_live_conversion_off_unchanged() {
    // With live_conversion=false, composing keeps the candidate window hidden.
    let mut engine = InputMethodEngine::new();
    assert!(!engine.live.enabled);

    // Type "ai" -> "あい" (standard hiragana preedit)
    engine.process_key(&press('a'));
    let result = engine.process_key(&press('i'));
    assert_eq!(engine.preedit().unwrap().text(), "あい");
    assert!(engine.live.text.is_empty());
    assert!(
        result
            .actions
            .iter()
            .any(|action| matches!(action, EngineAction::HideCandidates))
    );
}

#[test]
fn test_live_conversion_keeps_candidates_hidden_while_composing() {
    let mut engine = make_live_conversion_engine();

    engine.process_key(&press('a'));
    let result = engine.process_key(&press('i'));

    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert!(
        result
            .actions
            .iter()
            .any(|action| matches!(action, EngineAction::HideCandidates))
    );
}

#[test]
fn test_live_conversion_escape_shows_hiragana() {
    // Test that Escape clears live conversion text and shows hiragana
    let mut engine = make_live_conversion_engine();

    // Type "ai" -> "あい"
    engine.process_key(&press('a'));
    engine.process_key(&press('i'));

    // Simulate live conversion being active
    engine.live.text = "愛".to_string();

    // Press Escape -> should clear live_conversion_text and show hiragana
    let result = engine.process_key(&press_key(Keysym::ESCAPE));
    assert!(result.consumed);
    assert!(engine.live.text.is_empty());
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "あい");
}

#[test]
fn test_live_conversion_escape_twice_cancels() {
    // Test that double Escape cancels input
    let mut engine = make_live_conversion_engine();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));

    // Set live conversion text
    engine.live.text = "愛".to_string();

    // First Escape: clears live conversion, shows hiragana
    engine.process_key(&press_key(Keysym::ESCAPE));
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert!(engine.live.text.is_empty());

    // Second Escape: cancels input entirely
    engine.process_key(&press_key(Keysym::ESCAPE));
    assert!(matches!(engine.state(), InputState::Empty));
}

#[test]
fn test_live_conversion_commit_with_converted_text() {
    // Test that Enter commits the live conversion text
    let mut engine = make_live_conversion_engine();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));

    // Simulate live conversion
    engine.live.text = "愛".to_string();

    // Press Enter -> should commit "愛", not "あい"
    let result = engine.process_key(&press_key(Keysym::RETURN));
    assert!(result.consumed);

    let commit_text = result
        .actions
        .iter()
        .find_map(|a| {
            if let EngineAction::Commit(text) = a {
                Some(text.clone())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(commit_text, "愛");
    assert!(matches!(engine.state(), InputState::Empty));
    assert!(engine.live.text.is_empty());
}

#[test]
fn test_live_conversion_commit_empty_falls_back_to_hiragana() {
    // When live_conversion_text is empty, commit should use hiragana
    let mut engine = make_live_conversion_engine();

    engine.process_key(&press('a'));
    assert!(engine.live.text.is_empty());

    let result = engine.process_key(&press_key(Keysym::RETURN));
    let commit_text = result
        .actions
        .iter()
        .find_map(|a| {
            if let EngineAction::Commit(text) = a {
                Some(text.clone())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(commit_text, "あ");
}

#[test]
fn test_live_conversion_cursor_move_clears() {
    // Moving cursor should clear live conversion text
    let mut engine = make_live_conversion_engine();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.live.text = "愛".to_string();

    // Left arrow clears live conversion
    engine.process_key(&press_key(Keysym::LEFT));
    assert!(engine.live.text.is_empty());
}

#[test]
fn test_live_conversion_build_preedit() {
    // Test build_composing_preedit constructs correct display for live conversion
    let mut engine = make_live_conversion_engine();

    engine.live.text = "漢字".to_string();

    let preedit = engine.build_composing_preedit();
    assert_eq!(preedit.text(), "漢字");
    assert_eq!(preedit.caret(), 2); // 漢字 = 2 chars
}

#[test]
fn test_live_conversion_normalizes_symbol_width() {
    let mut engine = InputMethodEngine::with_config(EngineConfig {
        live_conversion: true,
        fullwidth_symbols: true,
        japanese_punctuation: false,
        ..EngineConfig::default()
    });

    engine.live.text = engine.normalize_input_text("abc!?");

    let preedit = engine.build_composing_preedit();
    assert_eq!(preedit.text(), "abc！？");
}

#[test]
fn test_live_conversion_allows_alphabet_mode_with_japanese_reading() {
    assert!(InputMethodEngine::contains_japanese_reading_text(
        "きょうもLinuxをつかう"
    ));
}

#[test]
fn test_live_conversion_skips_plain_alphabet_text() {
    assert!(!InputMethodEngine::contains_japanese_reading_text("Linux"));
}

#[test]
fn test_live_conversion_rewrites_segment_from_user_dictionary() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(make_test_dictionary(
        r#"[
            {"reading":"あめ","candidates":[{"surface":"飴","score":0.0}]}
        ]"#,
    ));

    let preserved = engine.make_preserved_candidate("今日は雨", CandidateSource::Model);
    let rewritten = engine.rerank_auto_suggest_text("きょうはあめ", &preserved);

    assert_eq!(rewritten, "今日は飴");
}

#[test]
fn test_live_conversion_rewrites_segment_from_strong_learning() {
    let mut engine = make_live_conversion_engine();
    let mut learning = LearningCache::new(16);
    learning.record_strong("あめ", "飴");
    engine.learning = Some(learning);

    let preserved = engine.make_preserved_candidate("今日は雨", CandidateSource::Model);
    let rewritten = engine.rerank_auto_suggest_text("きょうはあめ", &preserved);

    assert_eq!(rewritten, "今日は飴");
}

#[test]
fn test_live_conversion_rewrites_preserved_sentence_segment_from_strong_learning() {
    let mut engine = make_live_conversion_engine();
    let mut learning = LearningCache::new(16);
    learning.record_strong("あめ", "飴");
    engine.learning = Some(learning);

    let preserved = engine.make_preserved_candidate("雨です", CandidateSource::Model);
    let rewritten = engine.rerank_auto_suggest_text("あめです", &preserved);

    assert_eq!(rewritten, "飴です");
}

#[test]
fn test_live_conversion_prefers_user_dictionary_over_strong_learning() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(make_test_dictionary(
        r#"[
            {"reading":"あめ","candidates":[{"surface":"雨","score":0.0}]}
        ]"#,
    ));
    let mut learning = LearningCache::new(16);
    learning.record_strong("あめ", "飴");
    engine.learning = Some(learning);

    let preserved = engine.make_preserved_candidate("今日は飴", CandidateSource::Model);
    let rewritten = engine.rerank_auto_suggest_text("きょうはあめ", &preserved);

    assert_eq!(rewritten, "今日は雨");
}

#[test]
fn test_live_conversion_ignores_weak_learning_for_rewrite() {
    let mut engine = make_live_conversion_engine();
    let mut learning = LearningCache::new(16);
    learning.record_weak("あめ", "飴");
    learning.record_weak("あめ", "飴");
    learning.record_weak("あめ", "飴");
    engine.learning = Some(learning);

    let preserved = engine.make_preserved_candidate("今日は雨", CandidateSource::Model);
    let rewritten = engine.rerank_auto_suggest_text("きょうはあめ", &preserved);

    assert_eq!(rewritten, "今日は雨");
}

#[test]
fn test_live_conversion_preserves_unlearned_default_surface() {
    let mut engine = make_live_conversion_engine();

    let preserved = engine.make_preserved_candidate("のは", CandidateSource::Model);
    let rewritten = engine.rerank_auto_suggest_text("のは", &preserved);

    assert_eq!(rewritten, "のは");
}

#[test]
fn test_live_conversion_keeps_preserved_model_top_candidate() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(make_test_dictionary(
        r#"[
            {"reading":"どうかな","candidates":[{"surface":"どうかな〜","score":0.0}]}
        ]"#,
    ));

    let preserved = engine.make_preserved_candidate("どうかな", CandidateSource::Model);
    let rewritten = engine.rerank_auto_suggest_text("どうかな", &preserved);

    assert_eq!(rewritten, "どうかな");
}

// --- Ctrl+Space full-width space tests ---

#[test]
fn test_ctrl_space_inserts_fullwidth_space_in_empty() {
    let mut engine = InputMethodEngine::new();

    // Ctrl+Space in Empty state -> start input with full-width space
    let result = engine.process_key(&press_ctrl(Keysym::SPACE));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "\u{3000}");
}

#[test]
fn test_ctrl_space_inserts_fullwidth_space_in_hiragana() {
    let mut engine = InputMethodEngine::new();

    // Type "あ"
    engine.process_key(&press('a'));
    assert_eq!(engine.preedit().unwrap().text(), "あ");

    // Ctrl+Space -> insert full-width space
    let result = engine.process_key(&press_ctrl(Keysym::SPACE));
    assert!(result.consumed);
    assert_eq!(engine.preedit().unwrap().text(), "あ\u{3000}");
}

#[test]
fn test_ctrl_space_fullwidth_space_commit() {
    let mut engine = InputMethodEngine::new();

    // Type "あ" + fullwidth space
    engine.process_key(&press('a'));
    engine.process_key(&press_ctrl(Keysym::SPACE));

    // Enter to commit
    let result = engine.process_key(&press_key(Keysym::RETURN));
    let commit_text = result
        .actions
        .iter()
        .find_map(|a| {
            if let EngineAction::Commit(text) = a {
                Some(text.clone())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(commit_text, "あ\u{3000}");
}

// --- Ctrl+Shift+L live conversion toggle tests ---

#[test]
fn test_ctrl_shift_l_toggles_live_conversion() {
    let mut engine = InputMethodEngine::new();
    assert!(!engine.live.enabled);

    // Ctrl+Shift+L → toggle ON
    let result = engine.process_key(&press_ctrl_shift(Keysym::KEY_L_UPPER));
    assert!(result.consumed);
    assert!(engine.live.enabled);

    // Ctrl+Shift+L again → toggle OFF
    let result = engine.process_key(&press_ctrl_shift(Keysym::KEY_L_UPPER));
    assert!(result.consumed);
    assert!(!engine.live.enabled);
}

#[test]
fn test_ctrl_shift_l_lowercase_toggles() {
    let mut engine = InputMethodEngine::new();
    assert!(!engine.live.enabled);

    // Ctrl+Shift+l (lowercase keysym) → toggle ON
    let result = engine.process_key(&press_ctrl_shift(Keysym::KEY_L));
    assert!(result.consumed);
    assert!(engine.live.enabled);
}

#[test]
fn test_ctrl_shift_l_shows_aux_text() {
    let mut engine = InputMethodEngine::new();

    // Ctrl+Shift+L → check aux text shows "ライブ変換: ON"
    let result = engine.process_key(&press_ctrl_shift(Keysym::KEY_L_UPPER));
    let has_aux = result
        .actions
        .iter()
        .any(|a| matches!(a, EngineAction::UpdateAuxText(text) if text.contains("ライブ変換: ON")));
    assert!(has_aux);

    // Ctrl+Shift+L again → "ライブ変換: OFF"
    let result = engine.process_key(&press_ctrl_shift(Keysym::KEY_L_UPPER));
    let has_aux = result.actions.iter().any(
        |a| matches!(a, EngineAction::UpdateAuxText(text) if text.contains("ライブ変換: OFF")),
    );
    assert!(has_aux);
}
