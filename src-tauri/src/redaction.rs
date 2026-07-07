use crate::utils::ensure_trailing_newline;
use serde_json::Value;

pub fn redact_json_text(content: String) -> String {
    if content.trim().is_empty() {
        return content;
    }

    match serde_json::from_str::<Value>(&content) {
        Ok(mut value) => {
            redact_json_value(&mut value);
            serde_json::to_string_pretty(&value)
                .map(ensure_trailing_newline)
                .unwrap_or_else(|_| redact_plain_text(content))
        }
        Err(_) => redact_plain_text(content),
    }
}

fn redact_json_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                let lowered = key.to_ascii_lowercase();
                if lowered.contains("token")
                    || lowered.contains("api_key")
                    || lowered.contains("apikey")
                    || lowered.contains("secret")
                {
                    if let Some(raw) = item.as_str() {
                        *item = Value::String(mask_secret(raw));
                    }
                } else {
                    redact_json_value(item);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_json_value(item);
            }
        }
        _ => {}
    }
}

pub fn redact_plain_text(content: String) -> String {
    content
        .lines()
        .map(|line| {
            let lowered = line.to_ascii_lowercase();
            if lowered.contains("token")
                || lowered.contains("api_key")
                || lowered.contains("apikey")
                || lowered.contains("secret")
            {
                mask_assignment_line(line)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn mask_assignment_line(line: &str) -> String {
    if let Some((left, right)) = line.split_once('=') {
        format!(
            "{}= {}",
            left.trim_end(),
            mask_secret(right.trim().trim_matches('"'))
        )
    } else {
        line.to_string()
    }
}

pub fn mask_secret(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "".to_string();
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }
    let start: String = chars.iter().take(4).collect();
    let end: String = chars
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{start}****{end}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_masking_keeps_edges_only() {
        assert_eq!(mask_secret("sk-1234567890"), "sk-1****7890");
        assert_eq!(mask_secret("short"), "****");
    }
}
