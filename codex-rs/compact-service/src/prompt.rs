use compact_service_api::CompactMemoryBundle;
use compact_service_api::CompactPromptSpec;
use serde_json::json;

pub(super) fn build_prompt_spec(
    compact_prompt: &str,
    bundle: &CompactMemoryBundle,
) -> CompactPromptSpec {
    let prompt_text = format!(
        "{compact_prompt}\n\n## Runtime Memory Bundle\n### Shared user preferences\n{user_preferences}\n\n### Shared project understanding\n{project_understanding}\n\n### Local current work\n{current_work}\n\n## Output contract\nReturn a JSON object matching the provided schema. Keep `shared_fact_candidates` empty when no new canonical fact should be proposed. Keep every list concise and remove stale facts instead of duplicating them.",
        user_preferences = render_optional_memory(
            bundle.user_preferences.as_deref(),
            "Shared memory unavailable or empty."
        ),
        project_understanding = render_optional_memory(
            bundle.project_understanding.as_deref(),
            "Shared memory unavailable or empty."
        ),
        current_work = render_optional_memory(
            bundle.current_work.as_deref(),
            "Local current-work is missing; rebuild it from the current thread."
        ),
    );

    let output_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["current_work", "shared_fact_candidates", "handoff_summary"],
        "properties": {
            "current_work": {
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "goal",
                    "status",
                    "recent_progress",
                    "files_read",
                    "key_findings",
                    "skip_files",
                    "blockers",
                    "next_steps"
                ],
                "properties": {
                    "goal": { "type": "string" },
                    "status": { "type": "string" },
                    "recent_progress": string_array_schema(),
                    "files_read": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["path", "reason", "conclusion"],
                            "properties": {
                                "path": { "type": "string" },
                                "reason": { "type": "string" },
                                "conclusion": { "type": "string" },
                                "revisit": { "type": ["string", "null"] }
                            }
                        }
                    },
                    "key_findings": string_array_schema(),
                    "skip_files": string_array_schema(),
                    "blockers": string_array_schema(),
                    "next_steps": string_array_schema()
                }
            },
            "shared_fact_candidates": string_array_schema(),
            "handoff_summary": { "type": "string" }
        }
    });

    CompactPromptSpec {
        prompt_text,
        output_schema,
    }
}

fn render_optional_memory(value: Option<&str>, fallback: &str) -> String {
    value.unwrap_or(fallback).to_string()
}

fn string_array_schema() -> serde_json::Value {
    json!({
        "type": "array",
        "items": { "type": "string" }
    })
}
