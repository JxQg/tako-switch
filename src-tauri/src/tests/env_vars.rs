use super::*;

#[test]
fn marked_block_removal_only_removes_tako_block() {
    let start = "# >>> Tako Switch: TAKO_CODEX_API_KEY";
    let end = "# <<< Tako Switch: TAKO_CODEX_API_KEY";
    let existing = format!(
        "export TAKO_CODEX_API_KEY='manual'\n\n{start}\nexport TAKO_CODEX_API_KEY='one'\n{end}\n\nexport PATH=/bin\n"
    );
    let first = remove_marked_block(&existing, start, end);
    let second = remove_marked_block(&first, start, end);

    assert_eq!(first, second);
    assert!(first.contains("export TAKO_CODEX_API_KEY='manual'"));
    assert!(first.contains("export PATH=/bin"));
    assert!(!first.contains(start));
    assert!(!first.contains(end));
}

#[test]
fn marked_block_reader_only_reads_tako_block() {
    let content = "\
export TAKO_CODEX_API_KEY='manual'

# >>> Tako Switch: TAKO_CODEX_API_KEY
export TAKO_CODEX_API_KEY='one'\\''two'
# <<< Tako Switch: TAKO_CODEX_API_KEY
";

    assert_eq!(
        read_marked_env_value(content, LEGACY_CODEX_API_KEY_ENV).as_deref(),
        Some("one'two")
    );
}
