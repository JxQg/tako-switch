use super::*;

#[test]
fn infers_clients_from_provider() {
    assert_eq!(clients_for_provider("Anthropic"), vec!["claude"]);
    assert_eq!(clients_for_provider("OpenAI"), vec!["codex"]);
    assert_eq!(clients_for_provider("Google"), vec!["gemini"]);
    assert!(clients_for_provider("Unknown").is_empty());
}

#[test]
fn parses_openai_models_payload() {
    let data = json!([
        { "id": "claude-opus-4", "owned_by": "Anthropic", "object": "model" },
        { "id": "gpt-5", "owned_by": "OpenAI" },
        { "object": "model" }
    ]);
    let models = parse_models(data.as_array().unwrap());
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "claude-opus-4");
    assert_eq!(models[0].clients, vec!["claude"]);
    assert_eq!(models[1].clients, vec!["codex"]);
}

#[test]
fn number_field_accepts_numbers_and_strings() {
    let value = json!({ "a": 1.5, "b": "2.25" });
    assert_eq!(number_field(Some(&value), "a"), 1.5);
    assert_eq!(number_field(Some(&value), "b"), 2.25);
    assert_eq!(number_field(Some(&value), "missing"), 0.0);
}
