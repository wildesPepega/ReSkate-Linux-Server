// Parsing helpers for console lines, admin requests and chat commands (Server/server_text.h).

pub fn trim(text: &str) -> &str {
    text.trim_matches(|c| c == ' ' || c == '\t')
}

// The first word and the rest, both trimmed.
pub fn split(text: &str) -> (&str, &str) {
    let text = trim(text);
    match text.find(' ') {
        None => (text, ""),
        Some(space) => (&text[..space], trim(&text[space + 1..])),
    }
}

// Digits only, like std::from_chars: no sign, no blanks, no overflow.
pub fn number(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse::<u64>().ok()
}

// A signed whole number as std::from_chars reads it: an optional '-', then digits.
pub fn integer(text: &str) -> Option<i32> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse::<i32>().ok()
}

// A decimal number as std::from_chars reads it (no leading '+', no blanks).
pub fn decimal(text: &str) -> Option<f32> {
    if text.is_empty() || text.starts_with('+') || text.contains(char::is_whitespace) {
        return None;
    }
    text.parse::<f32>().ok()
}

pub fn on_off(text: &str) -> Option<bool> {
    match text {
        "on" | "true" | "yes" => Some(true),
        "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

pub fn lower(text: &str) -> String {
    text.to_ascii_lowercase()
}

// The longest prefix of `text` that is at most `limit` bytes and ends on a character boundary.
pub fn prefix(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
