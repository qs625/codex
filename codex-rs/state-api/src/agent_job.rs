use anyhow::Result;
use chrono::DateTime;
use chrono::Utc;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentJobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl AgentJobStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            AgentJobStatus::Pending => "pending",
            AgentJobStatus::Running => "running",
            AgentJobStatus::Completed => "completed",
            AgentJobStatus::Failed => "failed",
            AgentJobStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(anyhow::anyhow!("invalid agent job status: {value}")),
        }
    }

    pub fn is_final(self) -> bool {
        matches!(
            self,
            AgentJobStatus::Completed | AgentJobStatus::Failed | AgentJobStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentJobItemStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl AgentJobItemStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            AgentJobItemStatus::Pending => "pending",
            AgentJobItemStatus::Running => "running",
            AgentJobItemStatus::Completed => "completed",
            AgentJobItemStatus::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(anyhow::anyhow!("invalid agent job item status: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentJob {
    pub id: String,
    pub name: String,
    pub status: AgentJobStatus,
    pub instruction: String,
    pub auto_export: bool,
    pub max_runtime_seconds: Option<u64>,
    pub output_schema_json: Option<Value>,
    pub input_headers: Vec<String>,
    pub input_csv_path: String,
    pub output_csv_path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentJobItem {
    pub job_id: String,
    pub item_id: String,
    pub row_index: i64,
    pub source_id: Option<String>,
    pub row_json: Value,
    pub status: AgentJobItemStatus,
    pub assigned_thread_id: Option<String>,
    pub attempt_count: i64,
    pub result_json: Option<Value>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub reported_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentJobProgress {
    pub total_items: usize,
    pub pending_items: usize,
    pub running_items: usize,
    pub completed_items: usize,
    pub failed_items: usize,
}

#[derive(Debug, Clone)]
pub struct AgentJobCreateParams {
    pub id: String,
    pub name: String,
    pub instruction: String,
    pub auto_export: bool,
    pub max_runtime_seconds: Option<u64>,
    pub output_schema_json: Option<Value>,
    pub input_headers: Vec<String>,
    pub input_csv_path: String,
    pub output_csv_path: String,
}

#[derive(Debug, Clone)]
pub struct AgentJobItemCreateParams {
    pub item_id: String,
    pub row_index: i64,
    pub source_id: Option<String>,
    pub row_json: Value,
}

pub fn build_agent_job_worker_prompt(job: &AgentJob, item: &AgentJobItem) -> Result<String> {
    let job_id = job.id.as_str();
    let item_id = item.item_id.as_str();
    let instruction =
        render_agent_job_instruction_template(job.instruction.as_str(), &item.row_json);
    let output_schema = job
        .output_schema_json
        .as_ref()
        .map(serde_json::to_string_pretty)
        .transpose()?
        .unwrap_or_else(|| "{}".to_string());
    let row_json = serde_json::to_string_pretty(&item.row_json)?;
    Ok(format!(
        "You are processing one item for a generic agent job.\n\
Job ID: {job_id}\n\
Item ID: {item_id}\n\n\
Task instruction:\n\
{instruction}\n\n\
Input row (JSON):\n\
{row_json}\n\n\
Expected result schema (JSON Schema or {{}}):\n\
{output_schema}\n\n\
You MUST call the `report_agent_job_result` tool exactly once with:\n\
1. `job_id` = \"{job_id}\"\n\
2. `item_id` = \"{item_id}\"\n\
3. `result` = a JSON object that contains your analysis result for this row.\n\n\
If you need to stop the job early, include `stop` = true in the tool call.\n\n\
After the tool call succeeds, stop.",
    ))
}

pub fn render_agent_job_instruction_template(instruction: &str, row_json: &Value) -> String {
    const OPEN_BRACE_SENTINEL: &str = "__CODEX_OPEN_BRACE__";
    const CLOSE_BRACE_SENTINEL: &str = "__CODEX_CLOSE_BRACE__";

    let mut rendered = instruction
        .replace("{{", OPEN_BRACE_SENTINEL)
        .replace("}}", CLOSE_BRACE_SENTINEL);
    let Some(row) = row_json.as_object() else {
        return rendered
            .replace(OPEN_BRACE_SENTINEL, "{")
            .replace(CLOSE_BRACE_SENTINEL, "}");
    };
    for (key, value) in row {
        let placeholder = format!("{{{key}}}");
        let replacement = value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string());
        rendered = rendered.replace(placeholder.as_str(), replacement.as_str());
    }
    rendered
        .replace(OPEN_BRACE_SENTINEL, "{")
        .replace(CLOSE_BRACE_SENTINEL, "}")
}

pub fn ensure_unique_agent_job_headers(headers: &[String]) -> Result<()> {
    let mut seen = HashSet::new();
    for header in headers {
        if !seen.insert(header) {
            anyhow::bail!("csv header {header} is duplicated");
        }
    }
    Ok(())
}

pub fn default_agent_job_output_csv_path(input_csv_path: &Path, job_id: &str) -> PathBuf {
    let stem = input_csv_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("agent_job_output");
    let job_suffix = &job_id[..8];
    let output_dir = input_csv_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| input_csv_path.to_path_buf());
    output_dir.join(format!("{stem}.agent-job-{job_suffix}.csv"))
}

pub fn parse_agent_job_csv(content: &str) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    let mut records = parse_csv_records(content)?;
    if records.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut headers = records.remove(0);
    if let Some(first) = headers.first_mut() {
        *first = first.trim_start_matches('\u{feff}').to_string();
    }
    let rows = records
        .into_iter()
        .filter(|row| !row.iter().all(std::string::String::is_empty))
        .collect();
    Ok((headers, rows))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CsvFieldState {
    Start,
    Unquoted,
    Quoted,
    AfterQuote,
}

fn parse_csv_records(content: &str) -> Result<Vec<Vec<String>>> {
    let mut records = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut state = CsvFieldState::Start;
    let mut chars = content.chars().peekable();
    let mut last_char_ended_record = false;

    while let Some(ch) = chars.next() {
        last_char_ended_record = false;
        match state {
            CsvFieldState::Start => match ch {
                '"' => state = CsvFieldState::Quoted,
                ',' => row.push(String::new()),
                '\n' => {
                    row.push(String::new());
                    records.push(std::mem::take(&mut row));
                    last_char_ended_record = true;
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    row.push(String::new());
                    records.push(std::mem::take(&mut row));
                    last_char_ended_record = true;
                }
                _ => {
                    field.push(ch);
                    state = CsvFieldState::Unquoted;
                }
            },
            CsvFieldState::Unquoted => match ch {
                '"' => anyhow::bail!("unexpected quote in unquoted csv field"),
                ',' => {
                    row.push(std::mem::take(&mut field));
                    state = CsvFieldState::Start;
                }
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut row));
                    state = CsvFieldState::Start;
                    last_char_ended_record = true;
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    row.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut row));
                    state = CsvFieldState::Start;
                    last_char_ended_record = true;
                }
                _ => field.push(ch),
            },
            CsvFieldState::Quoted => match ch {
                '"' => {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        field.push('"');
                    } else {
                        state = CsvFieldState::AfterQuote;
                    }
                }
                _ => field.push(ch),
            },
            CsvFieldState::AfterQuote => match ch {
                ',' => {
                    row.push(std::mem::take(&mut field));
                    state = CsvFieldState::Start;
                }
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut row));
                    state = CsvFieldState::Start;
                    last_char_ended_record = true;
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    row.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut row));
                    state = CsvFieldState::Start;
                    last_char_ended_record = true;
                }
                _ => anyhow::bail!("unexpected character after closing csv quote"),
            },
        }
    }

    match state {
        CsvFieldState::Quoted => anyhow::bail!("unterminated quoted csv field"),
        CsvFieldState::Start | CsvFieldState::Unquoted | CsvFieldState::AfterQuote => {}
    }

    if !content.is_empty() && !last_char_ended_record {
        row.push(field);
        records.push(row);
    }

    Ok(records)
}

pub fn render_agent_job_csv(headers: &[String], items: &[AgentJobItem]) -> Result<String> {
    let mut csv = String::new();
    let mut output_headers = headers.to_vec();
    output_headers.extend([
        "job_id".to_string(),
        "item_id".to_string(),
        "row_index".to_string(),
        "source_id".to_string(),
        "status".to_string(),
        "attempt_count".to_string(),
        "last_error".to_string(),
        "result_json".to_string(),
        "reported_at".to_string(),
        "completed_at".to_string(),
    ]);
    csv.push_str(
        output_headers
            .iter()
            .map(|header| csv_escape(header.as_str()))
            .collect::<Vec<_>>()
            .join(",")
            .as_str(),
    );
    csv.push('\n');
    for item in items {
        let row_object = item.row_json.as_object().ok_or_else(|| {
            let item_id = item.item_id.as_str();
            anyhow::anyhow!("row_json for item {item_id} is not a JSON object")
        })?;
        let mut row_values = Vec::new();
        for header in headers {
            let value = row_object
                .get(header)
                .map_or_else(String::new, value_to_csv_string);
            row_values.push(csv_escape(value.as_str()));
        }
        row_values.push(csv_escape(item.job_id.as_str()));
        row_values.push(csv_escape(item.item_id.as_str()));
        row_values.push(csv_escape(item.row_index.to_string().as_str()));
        row_values.push(csv_escape(
            item.source_id.clone().unwrap_or_default().as_str(),
        ));
        row_values.push(csv_escape(item.status.as_str()));
        row_values.push(csv_escape(item.attempt_count.to_string().as_str()));
        row_values.push(csv_escape(
            item.last_error.clone().unwrap_or_default().as_str(),
        ));
        row_values.push(csv_escape(
            item.result_json
                .as_ref()
                .map_or_else(String::new, std::string::ToString::to_string)
                .as_str(),
        ));
        row_values.push(csv_escape(
            item.reported_at
                .map(|value| value.to_rfc3339())
                .unwrap_or_default()
                .as_str(),
        ));
        row_values.push(csv_escape(
            item.completed_at
                .map(|value| value.to_rfc3339())
                .unwrap_or_default()
                .as_str(),
        ));
        csv.push_str(row_values.join(",").as_str());
        csv.push('\n');
    }
    Ok(csv)
}

fn value_to_csv_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('\n') || value.contains('\r') || value.contains('"') {
        let escaped = value.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
#[path = "agent_job_tests.rs"]
mod tests;
