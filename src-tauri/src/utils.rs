use std::path::Path;

pub fn ensure_trailing_newline(mut value: String) -> String {
    if !value.ends_with('\n') {
        value.push('\n');
    }
    value
}

pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
