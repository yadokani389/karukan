use super::*;

// --- Mode toggle key tests (one-way: alphabet → hiragana) ---

#[test]
fn test_mode_toggle_key_switches_alphabet_to_hiragana() {
    let mut engine = InputMethodEngine::new();

    // Enter alphabet mode via Shift+A
    engine.process_key(&press_shift('A'));
    assert!(engine.input_mode == InputMode::Alphabet);

    // Alt_R press → switch to hiragana mode
    let result = engine.process_key(&press_key(Keysym::ALT_R));
    assert!(result.consumed);
    assert!(engine.input_mode != InputMode::Alphabet);

    // Type 'a' → should be 'あ' (hiragana mode)
    engine.process_key(&press('a'));
    assert_eq!(engine.preedit().unwrap().text(), "Aあ");
}

#[test]
fn test_mode_toggle_key_noop_in_hiragana() {
    let mut engine = InputMethodEngine::new();
    assert!(engine.input_mode != InputMode::Alphabet);

    // Alt_R press in hiragana mode → not consumed, no mode change
    let result = engine.process_key(&press_key(Keysym::ALT_R));
    assert!(!result.consumed);
    assert!(engine.input_mode != InputMode::Alphabet);

    // Type 'a' → still hiragana
    engine.process_key(&press('a'));
    assert_eq!(engine.preedit().unwrap().text(), "あ");
}

#[test]
fn test_ctrl_j_switches_alphabet_to_hiragana() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press_shift('A'));
    assert!(engine.input_mode == InputMode::Alphabet);

    let result = engine.process_key(&press_ctrl(Keysym::KEY_J));
    assert!(result.consumed);
    assert!(engine.input_mode == InputMode::Hiragana);

    engine.process_key(&press('k'));
    engine.process_key(&press('a'));
    assert_eq!(engine.preedit().unwrap().text(), "Aか");
}

#[test]
fn test_ctrl_j_switches_katakana_to_hiragana() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('a'));
    engine.process_key(&press_ctrl(Keysym::KEY_K));
    assert!(engine.input_mode == InputMode::Katakana);

    let result = engine.process_key(&press_ctrl(Keysym::KEY_J));
    assert!(result.consumed);
    assert!(engine.input_mode == InputMode::Hiragana);

    engine.process_key(&press('i'));
    assert_eq!(engine.preedit().unwrap().text(), "あい");
}

#[test]
fn test_mode_toggle_key_during_alphabet_input() {
    let mut engine = InputMethodEngine::new();

    // Enter alphabet mode via Shift+A and type "b"
    engine.process_key(&press_shift('A'));
    engine.process_key(&press('b'));
    assert_eq!(engine.preedit().unwrap().text(), "Ab");
    assert!(engine.input_mode == InputMode::Alphabet);

    // Alt_R → switch to hiragana
    let result = engine.process_key(&press_key(Keysym::ALT_R));
    assert!(result.consumed);
    assert!(engine.input_mode != InputMode::Alphabet);

    // Continue typing → hiragana
    engine.process_key(&press('k'));
    engine.process_key(&press('a'));
    assert_eq!(engine.preedit().unwrap().text(), "Abか");
}

#[test]
fn test_super_r_also_switches_alphabet_to_hiragana() {
    let mut engine = InputMethodEngine::new();

    // Enter alphabet mode via Shift+A
    engine.process_key(&press_shift('A'));
    assert!(engine.input_mode == InputMode::Alphabet);

    // Super_R press → switch to hiragana (one-way)
    let result = engine.process_key(&press_key(Keysym::SUPER_R));
    assert!(result.consumed);
    assert!(engine.input_mode != InputMode::Alphabet);
}

#[test]
fn test_meta_r_also_switches_alphabet_to_hiragana() {
    let mut engine = InputMethodEngine::new();

    // Enter alphabet mode via Shift+A
    engine.process_key(&press_shift('A'));
    assert!(engine.input_mode == InputMode::Alphabet);

    // Meta_R press → switch to hiragana (one-way)
    let result = engine.process_key(&press_key(Keysym::META_R));
    assert!(result.consumed);
    assert!(engine.input_mode != InputMode::Alphabet);
}
