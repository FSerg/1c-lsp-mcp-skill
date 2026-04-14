use lsp_skill_core::toon::format_response;
use std::fs;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures");
    path.push(name);
    path
}

fn load_json(name: &str) -> serde_json::Value {
    let path = fixture_path(name);
    let content =
        fs::read_to_string(&path).unwrap_or_else(|_| panic!("failed to read {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|_| panic!("failed to parse {}", path.display()))
}

fn load_toon(name: &str) -> String {
    let path = fixture_path(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("failed to read {}", path.display()))
        .trim_end()
        .to_string()
}

#[test]
fn golden_diagnostics_simple() {
    let input = load_json("diagnostics_simple.json");
    let expected = load_toon("diagnostics_simple.toon");
    assert_eq!(format_response("diagnostics", &input, true), expected);
}

#[test]
fn golden_diagnostics_with_related() {
    let input = load_json("diagnostics_with_related.json");
    let expected = load_toon("diagnostics_with_related.toon");
    assert_eq!(format_response("diagnostics", &input, true), expected);
}

#[test]
fn golden_references() {
    let input = load_json("references.json");
    let expected = load_toon("references.toon");
    assert_eq!(format_response("references", &input, true), expected);
}

#[test]
fn golden_definition_locationlink() {
    let input = load_json("definition_locationlink.json");
    let expected = load_toon("definition_locationlink.toon");
    assert_eq!(format_response("definition", &input, true), expected);
}

#[test]
fn golden_document_symbol_nested() {
    let input = load_json("document_symbol_nested.json");
    let expected = load_toon("document_symbol_nested.toon");
    assert_eq!(format_response("symbols", &input, true), expected);
}

#[test]
fn golden_symbol_information_flat() {
    let input = load_json("symbol_information_flat.json");
    let expected = load_toon("symbol_information_flat.toon");
    assert_eq!(format_response("workspace_symbols", &input, true), expected);
}

#[test]
fn golden_incoming_calls() {
    let input = load_json("incoming_calls.json");
    let expected = load_toon("incoming_calls.toon");
    assert_eq!(format_response("incoming_calls", &input, true), expected);
}

#[test]
fn golden_null_response() {
    assert_eq!(
        format_response("definition", &serde_json::Value::Null, true),
        "null"
    );
}

#[test]
fn golden_empty_array() {
    assert_eq!(
        format_response("references", &serde_json::json!([]), true),
        "references[0]:"
    );
}
