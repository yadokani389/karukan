use std::io::Write;

use tempfile::NamedTempFile;

use super::*;

fn write_input_table(contents: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(contents.as_bytes()).unwrap();
    file.flush().unwrap();
    file
}

#[test]
fn test_custom_input_table_uses_tsv_rules() {
    let file = write_input_table("k\tき\nkk\tきん\nq\tん\n");
    let mut engine = InputMethodEngine::new();
    engine
        .init_input_table(file.path().to_str())
        .expect("custom input table should load");

    engine.process_key(&press('k'));
    assert_eq!(engine.preedit().unwrap().text(), "k");

    engine.process_key(&press('k'));
    assert_eq!(engine.preedit().unwrap().text(), "きん");

    engine.process_key(&press('q'));
    assert_eq!(engine.preedit().unwrap().text(), "きんん");
}

#[test]
fn test_custom_input_table_keeps_unknown_input_buffered_then_passthrough() {
    let file = write_input_table("nn\tん\n");
    let mut engine = InputMethodEngine::new();
    engine
        .init_input_table(file.path().to_str())
        .expect("custom input table should load");

    engine.process_key(&press('n'));
    assert_eq!(engine.preedit().unwrap().text(), "n");

    let result = engine.process_key(&press_key(Keysym::RETURN));
    let has_commit = result
        .actions
        .iter()
        .any(|action| matches!(action, EngineAction::Commit(text) if text == "n"));
    assert!(has_commit, "unknown buffered input should flush as-is");
}
