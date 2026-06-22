mod json;
mod truncate;

pub use json::to_ascii_json_string;
pub use truncate::approx_bytes_for_tokens;
pub use truncate::approx_token_count;
pub use truncate::approx_tokens_from_byte_count;
pub use truncate::truncate_middle_chars;
pub use truncate::truncate_middle_with_token_budget;

// Truncate a &str to a byte budget at a char boundary (prefix)
#[inline]
pub fn take_bytes_at_char_boundary(s: &str, maxb: usize) -> &str {
    if s.len() <= maxb {
        return s;
    }
    let mut last_ok = 0;
    for (i, ch) in s.char_indices() {
        let nb = i + ch.len_utf8();
        if nb > maxb {
            break;
        }
        last_ok = nb;
    }
    &s[..last_ok]
}

/// Sanitize a tag value to comply with metric tag validation rules:
/// only ASCII alphanumeric, '.', '_', '-', and '/' are allowed.
pub fn sanitize_metric_tag_value(value: &str) -> String {
    const MAX_LEN: usize = 256;
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() || trimmed.chars().all(|ch| !ch.is_ascii_alphanumeric()) {
        return "unspecified".to_string();
    }
    if trimmed.len() <= MAX_LEN {
        trimmed.to_string()
    } else {
        trimmed[..MAX_LEN].to_string()
    }
}

/// Find all UUIDs in a string.
pub fn find_uuids(s: &str) -> Vec<String> {
    const UUID_LEN: usize = 36;

    let bytes = s.as_bytes();
    let mut matches = Vec::new();
    let mut index = 0;
    while index + UUID_LEN <= bytes.len() {
        let candidate = &bytes[index..index + UUID_LEN];
        if is_uuid_bytes(candidate) {
            matches.push(s[index..index + UUID_LEN].to_string());
            index += UUID_LEN;
        } else {
            index += 1;
        }
    }
    matches
}

fn is_uuid_bytes(bytes: &[u8]) -> bool {
    bytes.len() == 36
        && is_hex_range(bytes, 0..8)
        && bytes[8] == b'-'
        && is_hex_range(bytes, 9..13)
        && bytes[13] == b'-'
        && is_hex_range(bytes, 14..18)
        && bytes[18] == b'-'
        && is_hex_range(bytes, 19..23)
        && bytes[23] == b'-'
        && is_hex_range(bytes, 24..36)
}

fn is_hex_range(bytes: &[u8], range: std::ops::Range<usize>) -> bool {
    range
        .map(|index| bytes[index])
        .all(|byte| byte.is_ascii_hexdigit())
}

/// Convert a markdown-style `#L..` location suffix into a terminal-friendly
/// `:line[:column][-line[:column]]` suffix.
pub fn normalize_markdown_hash_location_suffix(suffix: &str) -> Option<String> {
    let fragment = suffix.strip_prefix('#')?;
    let (start, end) = match fragment.split_once('-') {
        Some((start, end)) => (start, Some(end)),
        None => (fragment, None),
    };
    let (start_line, start_column) = parse_markdown_hash_location_point(start)?;
    let mut normalized = String::from(":");
    normalized.push_str(start_line);
    if let Some(column) = start_column {
        normalized.push(':');
        normalized.push_str(column);
    }
    if let Some(end) = end {
        let (end_line, end_column) = parse_markdown_hash_location_point(end)?;
        normalized.push('-');
        normalized.push_str(end_line);
        if let Some(column) = end_column {
            normalized.push(':');
            normalized.push_str(column);
        }
    }
    Some(normalized)
}

fn parse_markdown_hash_location_point(point: &str) -> Option<(&str, Option<&str>)> {
    let point = point.strip_prefix('L')?;
    match point.split_once('C') {
        Some((line, column)) => Some((line, Some(column))),
        None => Some((point, None)),
    }
}

#[cfg(test)]
#[allow(warnings, clippy::all)]
mod tests {
    use super::find_uuids;
    use super::normalize_markdown_hash_location_suffix;
    use super::sanitize_metric_tag_value;
    use pretty_assertions::assert_eq;

    #[test]
    fn find_uuids_finds_multiple() {
        let input =
            "x 00112233-4455-6677-8899-aabbccddeeff-k y 12345678-90ab-cdef-0123-456789abcdef";
        assert_eq!(
            find_uuids(input),
            vec![
                "00112233-4455-6677-8899-aabbccddeeff".to_string(),
                "12345678-90ab-cdef-0123-456789abcdef".to_string(),
            ]
        );
    }

    #[test]
    fn find_uuids_ignores_invalid() {
        let input = "not-a-uuid-1234-5678-9abc-def0-123456789abc";
        assert_eq!(find_uuids(input), Vec::<String>::new());
    }

    #[test]
    fn find_uuids_handles_non_ascii_without_overlap() {
        let input = "🙂 55e5d6f7-8a7f-4d2a-8d88-123456789012abc";
        assert_eq!(
            find_uuids(input),
            vec!["55e5d6f7-8a7f-4d2a-8d88-123456789012".to_string()]
        );
    }

    #[test]
    fn sanitize_metric_tag_value_trims_and_fills_unspecified() {
        let msg = "///";
        assert_eq!(sanitize_metric_tag_value(msg), "unspecified");
    }

    #[test]
    fn sanitize_metric_tag_value_replaces_invalid_chars() {
        let msg = "bad value!";
        assert_eq!(sanitize_metric_tag_value(msg), "bad_value");
    }

    #[test]
    fn normalize_markdown_hash_location_suffix_converts_single_location() {
        assert_eq!(
            normalize_markdown_hash_location_suffix("#L74C3"),
            Some(":74:3".to_string())
        );
    }

    #[test]
    fn normalize_markdown_hash_location_suffix_converts_ranges() {
        assert_eq!(
            normalize_markdown_hash_location_suffix("#L74C3-L76C9"),
            Some(":74:3-76:9".to_string())
        );
    }
}
