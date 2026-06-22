use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::SkillLoadOutcome;
use crate::SkillMetadata;
use codex_utils_absolute_path::AbsolutePathBuf;

pub fn build_implicit_skill_path_indexes(
    skills: Vec<SkillMetadata>,
) -> (
    HashMap<AbsolutePathBuf, SkillMetadata>,
    HashMap<AbsolutePathBuf, SkillMetadata>,
    HashMap<AbsolutePathBuf, SkillMetadata>,
) {
    let mut by_scripts_dir = HashMap::new();
    let mut by_skill_doc_path = HashMap::new();
    let mut by_root_dir = HashMap::new();
    for skill in skills {
        let skill_doc_path = canonicalize_if_exists(&skill.path_to_skills_md);
        by_skill_doc_path.insert(skill_doc_path, skill.clone());

        if let Some(skill_dir) = skill.path_to_skills_md.parent() {
            let skill_root_dir = canonicalize_if_exists(&skill_dir);
            by_root_dir.insert(skill_root_dir, skill.clone());
            let scripts_dir = canonicalize_if_exists(&skill_dir.join("scripts"));
            by_scripts_dir.insert(scripts_dir, skill.clone());
            let references_dir = skill_dir.join("references");
            if references_dir.exists() {
                for reference_path in collect_reference_files(references_dir.as_path()) {
                    by_skill_doc_path.insert(reference_path, skill.clone());
                }
            }
        }
    }

    (by_scripts_dir, by_skill_doc_path, by_root_dir)
}

pub fn detect_implicit_skill_invocation_for_command(
    outcome: &SkillLoadOutcome,
    command: &str,
    workdir: &AbsolutePathBuf,
) -> Option<SkillMetadata> {
    let workdir = canonicalize_if_exists(workdir);
    let tokens = tokenize_command(command);
    let tokens = unwrap_rtk_tokens(tokens.as_slice());

    if let Some(candidate) = detect_skill_script_run(outcome, tokens, &workdir) {
        return Some(candidate);
    }

    detect_skill_doc_read(outcome, tokens, &workdir)
}

fn tokenize_command(command: &str) -> Vec<String> {
    shlex::split(command)
        .unwrap_or_else(|| command.split_whitespace().map(str::to_string).collect())
}

fn script_run_token(tokens: &[String]) -> Option<&str> {
    const RUNNERS: [&str; 10] = [
        "python", "python3", "bash", "zsh", "sh", "node", "deno", "ruby", "perl", "pwsh",
    ];
    const SCRIPT_EXTENSIONS: [&str; 7] = [".py", ".sh", ".js", ".ts", ".rb", ".pl", ".ps1"];

    let tokens = unwrap_rtk_tokens(tokens);
    let runner_token = tokens.first()?;
    let runner = command_basename(runner_token).to_ascii_lowercase();
    let runner = runner.strip_suffix(".exe").unwrap_or(&runner);
    if !RUNNERS.contains(&runner) {
        return None;
    }

    let mut script_token = None;
    for token in tokens.iter().skip(1) {
        if token == "--" || token.starts_with('-') {
            continue;
        }
        script_token = Some(token.as_str());
        break;
    }
    let script_token = script_token?;
    if SCRIPT_EXTENSIONS
        .iter()
        .any(|extension| script_token.to_ascii_lowercase().ends_with(extension))
    {
        return Some(script_token);
    }

    None
}

fn detect_skill_script_run(
    outcome: &SkillLoadOutcome,
    tokens: &[String],
    workdir: &AbsolutePathBuf,
) -> Option<SkillMetadata> {
    let script_token = script_run_token(tokens)?;
    let script_path = Path::new(script_token);
    let script_path = canonicalize_if_exists(&workdir.join(script_path));

    for path in script_path.ancestors() {
        if let Some(candidate) = outcome.implicit_skills_by_scripts_dir.get(&path) {
            return Some(candidate.clone());
        }
    }

    None
}

fn detect_skill_doc_read(
    outcome: &SkillLoadOutcome,
    tokens: &[String],
    workdir: &AbsolutePathBuf,
) -> Option<SkillMetadata> {
    let tokens = unwrap_rtk_tokens(tokens);
    if !command_reads_file(tokens) {
        return None;
    }

    for token in tokens.iter().skip(1) {
        if token.starts_with('-') {
            continue;
        }
        let path = Path::new(token);
        let candidate_path = canonicalize_if_exists(&workdir.join(path));
        if let Some(candidate) = outcome.implicit_skills_by_doc_path.get(&candidate_path) {
            return Some(candidate.clone());
        }
        for ancestor in candidate_path.ancestors() {
            let Ok(ancestor) = AbsolutePathBuf::try_from(ancestor.to_path_buf()) else {
                continue;
            };
            if let Some(candidate) = outcome.implicit_skills_by_root_dir.get(&ancestor) {
                return Some(candidate.clone());
            }
        }
    }

    None
}

fn command_reads_file(tokens: &[String]) -> bool {
    const READERS: [&str; 8] = ["cat", "sed", "head", "tail", "less", "more", "bat", "awk"];
    let tokens = unwrap_rtk_tokens(tokens);
    let Some(program) = tokens.first() else {
        return false;
    };
    let program = command_basename(program).to_ascii_lowercase();
    READERS.contains(&program.as_str())
}

fn unwrap_rtk_tokens(tokens: &[String]) -> &[String] {
    let Some(first) = tokens.first() else {
        return tokens;
    };
    if command_basename(first) != "rtk" {
        return tokens;
    }

    match tokens.get(1).map(|token| token.as_str()) {
        Some("proxy") if tokens.len() > 2 => &tokens[2..],
        Some(_) if tokens.len() > 1 => &tokens[1..],
        _ => tokens,
    }
}

fn command_basename(command: &str) -> String {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
        .to_string()
}

fn canonicalize_if_exists(path: &AbsolutePathBuf) -> AbsolutePathBuf {
    path.canonicalize().unwrap_or_else(|_| path.clone())
}

fn collect_reference_files(path: &Path) -> Vec<AbsolutePathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(path) else {
        return files;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_reference_files(path.as_path()));
        } else if path.is_file()
            && let Ok(path) = AbsolutePathBuf::try_from(path)
        {
            files.push(canonicalize_if_exists(&path));
        }
    }

    files
}

#[cfg(test)]
#[path = "invocation_utils_tests.rs"]
mod tests;
