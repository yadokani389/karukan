use super::*;
use karukan_engine::LearningCache;

// --- Candidate preservation tests ---

#[test]
fn test_live_text_preserved_in_conversion_via_down() {
    // When DOWN is pressed during live conversion, the AI inference result
    // (live_conversion_text) should appear in the candidate list.
    let mut engine = make_live_conversion_engine();

    // Simulate typing "あい" with live conversion active
    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.live.text = "愛".to_string();

    // Press DOWN → start_conversion()
    let result = engine.process_key(&press_key(Keysym::DOWN));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    // The candidate list should contain "愛"
    let candidates = engine.state().candidates().unwrap();
    assert!(
        candidates.candidates().iter().any(|c| c.text == "愛"),
        "AI inference result '愛' should be in the candidate list"
    );
}

#[test]
fn test_live_text_not_duplicated_in_conversion() {
    // If the live_text matches the reading, it should not be duplicated
    let mut engine = make_live_conversion_engine();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    // live_conversion_text same as hiragana reading → should not be added
    engine.live.text = "あい".to_string();

    let result = engine.process_key(&press_key(Keysym::DOWN));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    // "あい" should not appear twice (it's same as reading, so live_text is skipped)
    let candidates = engine.state().candidates().unwrap();
    let count = candidates
        .candidates()
        .iter()
        .filter(|c| c.text == "あい")
        .count();
    assert_eq!(count, 1, "Reading should appear exactly once");
}

#[test]
fn test_suggest_result_preserved_in_start_conversion() {
    // When Space is pressed, the previous auto-suggest/live conversion result
    // should be preserved in the candidate list even if re-inference doesn't produce it.
    // (Without a kanji converter, build_conversion_candidates returns fallback only,
    // so the live_conversion_text would be lost without the preservation logic.)
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.live.text = "愛".to_string();

    // Press Space → start_conversion()
    let result = engine.process_key(&press_key(Keysym::SPACE));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    // "愛" should be preserved in the candidate list
    let candidates = engine.state().candidates().unwrap();
    assert!(
        candidates.candidates().iter().any(|c| c.text == "愛"),
        "Previous suggest result '愛' should be preserved in candidates"
    );
}

#[test]
fn test_empty_live_text_not_added_to_candidates() {
    // When live_conversion_text is empty, no extra candidate should be added
    let mut engine = make_live_conversion_engine();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    // Force empty to test the "no live text" scenario
    engine.live.text.clear();

    // DOWN → start_conversion()
    let result = engine.process_key(&press_key(Keysym::DOWN));
    assert!(result.consumed);

    // Should have candidates but no empty-string candidate
    if let Some(candidates) = engine.state().candidates() {
        assert!(
            !candidates.candidates().iter().any(|c| c.text.is_empty()),
            "Empty candidate should not be in the list"
        );
    }
}

#[test]
fn test_sentence_alternatives_include_strong_learning_and_user_dictionary() {
    let mut engine = make_live_conversion_engine();
    engine.learning = Some(LearningCache::new(16));
    engine.learning.as_mut().unwrap().record_strong("め", "眼");
    engine.learning.as_mut().unwrap().record_strong("め", "芽");
    engine.dicts.user = Some(make_test_dictionary(
        r#"[
            {"reading":"め","candidates":[{"surface":"瞳","score":0.0}]}
        ]"#,
    ));

    let candidates = engine.sentence_alternatives_for_test("めがある", "目がある");
    let texts: Vec<_> = candidates
        .into_iter()
        .map(|candidate| candidate.text)
        .collect();

    assert!(texts.iter().any(|text| text == "目がある"));
    assert!(texts.iter().any(|text| text == "眼がある"));
    assert!(texts.iter().any(|text| text == "芽がある"));
    assert!(texts.iter().any(|text| text == "瞳がある"));
}

#[test]
fn test_sentence_alternatives_appear_in_conversion_candidates() {
    let mut engine = make_live_conversion_engine();
    engine.learning = Some(LearningCache::new(16));
    engine.learning.as_mut().unwrap().record_strong("め", "眼");
    engine.learning.as_mut().unwrap().record_strong("め", "芽");
    engine.dicts.user = Some(make_test_dictionary(
        r#"[
            {"reading":"め","candidates":[{"surface":"瞳","score":0.0}]}
        ]"#,
    ));

    engine.process_key(&press('m'));
    engine.process_key(&press('e'));
    engine.process_key(&press('g'));
    engine.process_key(&press('a'));
    engine.process_key(&press('a'));
    engine.process_key(&press('r'));
    engine.process_key(&press('u'));
    engine.live.text = "目がある".to_string();

    let result = engine.process_key(&press_key(Keysym::DOWN));
    assert!(result.consumed);

    let candidates = engine.state().candidates().unwrap();
    let texts: Vec<_> = candidates
        .candidates()
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect();
    assert!(texts.contains(&"目がある"));
    assert!(texts.contains(&"眼がある"));
    assert!(texts.contains(&"芽がある"));
    assert!(texts.contains(&"瞳がある"));
}
