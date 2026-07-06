const EMPTY_PLACEHOLDER: &str = "- 暂无";

pub(super) fn current_work_completeness(current_work: Option<&str>) -> f64 {
    let Some(current_work) = current_work else {
        return 0.0;
    };
    let meaningful_line_count = current_work
        .lines()
        .map(str::trim)
        .filter(|line| is_meaningful_line(line))
        .count();

    match meaningful_line_count {
        0 => 0.0,
        1 => 0.25,
        2 => 0.5,
        3 | 4 => 0.8,
        _ => 1.0,
    }
}

fn is_meaningful_line(line: &str) -> bool {
    if line.is_empty() || line == "-" || line == EMPTY_PLACEHOLDER {
        return false;
    }
    if line.starts_with('#') {
        return false;
    }

    let normalized = line
        .trim_start_matches([
            '-', '*', '+', '•', '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '.', ')',
        ])
        .trim();

    if normalized.is_empty()
        || normalized == "暂无"
        || normalized.eq_ignore_ascii_case("none")
        || normalized.eq_ignore_ascii_case("n/a")
    {
        return false;
    }

    normalized.chars().any(char::is_alphanumeric)
}
