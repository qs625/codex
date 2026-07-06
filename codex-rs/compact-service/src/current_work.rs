use compact_service_api::CompactFileNote;
use compact_service_api::CompactModelOutput;

const MAX_LIST_ITEMS: usize = 12;
const MAX_TEXT_CHARS: usize = 300;
const EMPTY_PLACEHOLDER: &str = "- 暂无";

pub(super) fn render_current_work(output: &CompactModelOutput) -> String {
    let current_work = &output.current_work;
    let mut lines = vec![
        "# Current Work".to_string(),
        String::new(),
        "## Current Goal".to_string(),
        format!("- {}", sanitize_line(&current_work.goal)),
        String::new(),
        "## Current Status".to_string(),
        format!("- {}", sanitize_line(&current_work.status)),
        String::new(),
        "## Recent Progress".to_string(),
    ];
    push_string_list(&mut lines, &current_work.recent_progress);
    lines.push(String::new());
    lines.push("## Files Already Read".to_string());
    push_files_read(&mut lines, &current_work.files_read);
    lines.push(String::new());
    lines.push("## Key Findings".to_string());
    push_string_list(&mut lines, &current_work.key_findings);
    lines.push(String::new());
    lines.push("## Likely Skippable Files".to_string());
    push_string_list(&mut lines, &current_work.skip_files);
    lines.push(String::new());
    lines.push("## Blockers".to_string());
    push_string_list(&mut lines, &current_work.blockers);
    lines.push(String::new());
    lines.push("## Next Steps".to_string());
    push_string_list(&mut lines, &current_work.next_steps);
    if !output.shared_fact_candidates.is_empty() {
        lines.push(String::new());
        lines.push("## Shared Fact Candidates".to_string());
        push_string_list(&mut lines, &output.shared_fact_candidates);
    }
    lines.join("\n")
}

pub(super) fn current_work_completeness(current_work: Option<&str>) -> f64 {
    let Some(current_work) = current_work else {
        return 0.0;
    };
    let required_sections = [
        "## Current Goal",
        "## Current Status",
        "## Files Already Read",
        "## Key Findings",
        "## Next Steps",
    ];
    let completed = required_sections
        .iter()
        .filter(|section| section_has_content(current_work, section))
        .count();
    completed as f64 / required_sections.len() as f64
}

fn section_has_content(current_work: &str, heading: &str) -> bool {
    let Some(start) = current_work.find(heading) else {
        return false;
    };
    let after_heading = &current_work[start + heading.len()..];
    let next_heading = after_heading.find("\n## ").unwrap_or(after_heading.len());
    after_heading[..next_heading]
        .lines()
        .map(str::trim)
        .any(|line| !line.is_empty() && line != "-" && line != EMPTY_PLACEHOLDER)
}

fn push_files_read(lines: &mut Vec<String>, files: &[CompactFileNote]) {
    if files.is_empty() {
        lines.push(EMPTY_PLACEHOLDER.to_string());
        return;
    }
    for file in files.iter().take(MAX_LIST_ITEMS) {
        let revisit = file
            .revisit
            .as_deref()
            .map(sanitize_line)
            .unwrap_or_else(|| "不需要".to_string());
        lines.push(format!(
            "- {} | 原因：{} | 结论：{} | 是否还需再看：{}",
            sanitize_line(&file.path),
            sanitize_line(&file.reason),
            sanitize_line(&file.conclusion),
            revisit,
        ));
    }
}

fn push_string_list(lines: &mut Vec<String>, values: &[String]) {
    if values.is_empty() {
        lines.push(EMPTY_PLACEHOLDER.to_string());
        return;
    }
    for value in values.iter().take(MAX_LIST_ITEMS) {
        lines.push(format!("- {}", sanitize_line(value)));
    }
}

fn sanitize_line(value: &str) -> String {
    value.replace('\n', " ")
        .chars()
        .take(MAX_TEXT_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}
