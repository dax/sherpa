use std::time::Duration;

use tokio::process::Command;

use super::cli_detection::AiCli;

pub const DEFAULT_TIMEOUT_SECS: u64 = 240;

/// Which model tier to use for an AI call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTier {
    /// For complex analyses requiring deep reasoning (approach, review plan)
    Deep,
    /// For simpler, faster tasks (step explanations, chat, relations)
    Fast,
}

#[derive(Debug)]
pub struct AiPrompt {
    pub context: String,
    pub instruction: String,
}

impl AiPrompt {
    pub fn full_prompt(&self) -> String {
        format!("{}\n\n{}", self.context, self.instruction)
    }
}

#[derive(Debug)]
pub enum AiCliError {
    SpawnFailed(std::io::Error),
    CliFailure { stderr: String },
    Timeout,
}

impl std::fmt::Display for AiCliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpawnFailed(e) => write!(f, "Failed to start AI CLI: {e}"),
            Self::CliFailure { stderr } => write!(f, "AI CLI error: {stderr}"),
            Self::Timeout => write!(f, "AI request timed out"),
        }
    }
}

fn cli_failure_from_output(output: &std::process::Output) -> AiCliError {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        AiCliError::CliFailure { stderr: stdout }
    } else {
        AiCliError::CliFailure { stderr }
    }
}

impl std::error::Error for AiCliError {}

pub async fn generate(cli: AiCli, prompt: &AiPrompt) -> Result<String, AiCliError> {
    generate_with_timeout(cli, prompt, Duration::from_secs(DEFAULT_TIMEOUT_SECS), None).await
}

pub async fn generate_with_timeout(
    cli: AiCli,
    prompt: &AiPrompt,
    timeout: Duration,
    model: Option<&str>,
) -> Result<String, AiCliError> {
    let full_prompt = prompt.full_prompt();
    let model = model.map(|m| cli.normalize_model(m));

    let fut = async move {
        let output = match cli {
            AiCli::Claude => {
                let mut cmd = Command::new("claude");
                cmd.arg("--print");
                if let Some(m) = model {
                    cmd.arg("--model").arg(m);
                }
                cmd.arg(&full_prompt);
                cmd.output().await.map_err(AiCliError::SpawnFailed)?
            }
            AiCli::Opencode => {
                let mut cmd = Command::new("opencode");
                cmd.arg("run");
                if let Some(m) = model {
                    cmd.arg("--model").arg(m);
                }
                cmd.arg(&full_prompt);
                cmd.output().await.map_err(AiCliError::SpawnFailed)?
            }
        };

        if !output.status.success() {
            return Err(cli_failure_from_output(&output));
        }

        Ok::<String, AiCliError>(String::from_utf8_lossy(&output.stdout).trim().to_string())
    };

    tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| AiCliError::Timeout)?
}

pub fn build_context(
    branch: &str,
    default_branch: &str,
    changed_files_summary: &str,
    diff: &str,
) -> String {
    // Truncate diff if it's very large to avoid CLI argument limits
    let max_diff_len = 80_000;
    let diff_section = if diff.len() > max_diff_len {
        let truncated = &diff[..max_diff_len];
        format!(
            "{truncated}\n\n... (diff truncated, {total} total characters)",
            total = diff.len()
        )
    } else {
        diff.to_string()
    };

    format!(
        "You are reviewing code changes on branch `{branch}` against `{default_branch}`.\n\
         \n\
         Changed files:\n\
         {changed_files_summary}\n\
         \n\
         Diff:\n\
         ```\n\
         {diff_section}\n\
         ```"
    )
}

pub fn approach_instruction() -> &'static str {
    "Describe the implementation approach as a structured summary.\n\n\
     Use this format:\n\
     1. **What**: 1-2 sentences on what this branch implements overall\n\
     2. **How**: 1-2 sentences on the architecture/design approach — patterns, layers, key decisions\n\
     3. **Key details**: 2-4 bullet points on notable trade-offs, patterns used, or choices made\n\
     4. **Concerns**: 1-2 bullet points on potential issues (or \"None identified\" if clean)\n\n\
     Rules:\n\
     - Be specific: reference actual file names, patterns, and types\n\
     - Keep each point to 1-2 sentences maximum\n\
     - Output ONLY the structured summary. No introduction, no conclusion, no thinking, \
     no reasoning, no reflection, no preamble, no meta-commentary."
}

pub fn review_plan_instruction() -> &'static str {
    "Analyze this diff and create a step-by-step review plan. Group changes by \
     feature, concept, or architectural layer — NOT necessarily one step per file. \
     A single file's changes may appear in multiple steps if they serve different purposes.\n\n\
     Respond with ONLY a JSON object inside a ```json fence, no other text:\n\
     ```json\n\
     {\n\
       \"steps\": [\n\
         {\n\
           \"title\": \"Short descriptive title\",\n\
           \"rationale\": \"Why review these changes together\",\n\
           \"file_refs\": [\n\
             {\"path\": \"src/foo.rs\", \"diff_lines\": [10, 45]},\n\
             {\"path\": \"src/bar.rs\", \"diff_lines\": null}\n\
           ]\n\
         }\n\
       ]\n\
     }\n\
     ```\n\
     diff_lines references 1-indexed line numbers within the file's diff output \
     (NOT source file line numbers). Line 1 is the 'diff --git ...' header. \
     For example, if a file's diff section is 30 lines long, valid values are \
     [1, 30]. Use null for the entire file's diff — prefer null unless you need \
     to split a single file across multiple steps. Order steps in recommended \
     review sequence."
}

pub fn step_explanation_instruction(step_title: &str, step_diff: &str) -> String {
    format!(
        "You are explaining review step \"{step_title}\".\n\n\
         Here is the diff for this step:\n```\n{step_diff}\n```\n\n\
         Explain these changes using this structure:\n\
         1. **What**: 1-2 sentences on what the changes do\n\
         2. **Why**: 1 sentence on the purpose or motivation\n\
         3. **Key details**: 2-4 bullet points a reviewer should notice \
         (e.g. edge cases, patterns used, potential issues)\n\n\
         Rules:\n\
         - Be specific: reference actual function names, types, and file names from the diff\n\
         - Keep it concise — no filler text\n\
         - Output ONLY the structured explanation. Do NOT include any thinking, reasoning, \
         reflection, preamble, meta-commentary, or phrases like \"Let me\", \"I'll\", \
         \"Looking at\", \"I need to\" etc."
    )
}

pub fn step_relation_instruction(
    prev_title: &str,
    current_title: &str,
    current_diff: &str,
) -> String {
    format!(
        "The reviewer just finished step \"{prev_title}\" and is now on step \"{current_title}\".\n\n\
         Here is the diff for the current step:\n```\n{current_diff}\n```\n\n\
         In 1-2 sentences, explain how this step relates to or builds on the previous step. \
         Be concise and specific.\n\n\
         Output ONLY the explanation. No thinking, reasoning, reflection, or preamble."
    )
}

pub fn step_chat_instruction(
    step_title: &str,
    step_diff: &str,
    explanation: &str,
    user_message: &str,
) -> String {
    let explanation_section = if explanation.is_empty() {
        String::new()
    } else {
        format!("\nExplanation of this step:\n{explanation}\n")
    };
    format!(
        "You are helping a reviewer who is on step \"{step_title}\".\n\n\
         Diff for this step:\n```\n{step_diff}\n```\n\
         {explanation_section}\n\
         The reviewer asks: {user_message}\n\n\
         Respond helpfully and concisely, referencing specific code from the diff when relevant.\n\
         Output ONLY the answer. No thinking, reasoning, reflection, or preamble."
    )
}

pub fn extract_json_from_response(raw: &str) -> Result<String, String> {
    if serde_json::from_str::<serde_json::Value>(raw).is_ok() {
        return Ok(raw.to_string());
    }

    if let Some(start) = raw.find("```json") {
        let after = &raw[start + 7..];
        if let Some(end) = after.find("```") {
            let candidate = after[..end].trim();
            if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                return Ok(candidate.to_string());
            }
        }
    }

    if let (Some(s), Some(e)) = (raw.find('{'), raw.rfind('}')) {
        if s < e {
            let candidate = &raw[s..=e];
            if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                return Ok(candidate.to_string());
            }
        }
    }

    Err("Could not extract valid JSON from AI response".to_string())
}

/// A primed CLI session that has already ingested the full diff context.
/// Fork from this to run analyses without re-sending the full context each time.
#[derive(Debug, Clone)]
pub struct PrimedSession {
    pub session_id: String,
    pub cli: AiCli,
}

pub fn primer_instruction() -> &'static str {
    "Read and internalize the code changes above. \
     Respond with a 2-3 bullet summary of the key themes you see. \
     After this, I will ask you to produce several different analyses \
     of these changes in follow-up messages."
}

/// Create a primed CLI session by sending the full context once.
/// Returns a session ID that can be forked for each analysis.
pub async fn prime_session(
    cli: AiCli,
    context: &str,
    timeout: Duration,
    model: Option<&str>,
) -> Result<PrimedSession, AiCliError> {
    let prompt = AiPrompt {
        context: context.to_string(),
        instruction: primer_instruction().to_string(),
    };
    let full_prompt = prompt.full_prompt();
    let model = model.map(|m| cli.normalize_model(m));

    let fut = async move {
        let output = match cli {
            AiCli::Claude => {
                let mut cmd = Command::new("claude");
                cmd.args(["--print", "--output-format", "json"]);
                if let Some(m) = model {
                    cmd.args(["--model", m]);
                }
                cmd.arg(&full_prompt);
                cmd.output().await.map_err(AiCliError::SpawnFailed)?
            }
            AiCli::Opencode => {
                let mut cmd = Command::new("opencode");
                cmd.args(["run", "--format", "json"]);
                if let Some(m) = model {
                    cmd.args(["-m", m]);
                }
                cmd.arg(&full_prompt);
                cmd.output().await.map_err(AiCliError::SpawnFailed)?
            }
        };

        if !output.status.success() {
            return Err(cli_failure_from_output(&output));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let session_id = extract_session_id(cli, &stdout)?;
        Ok::<PrimedSession, AiCliError>(PrimedSession { session_id, cli })
    };

    tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| AiCliError::Timeout)?
}

/// Fork from a primed session to run a specific analysis.
/// The fork inherits the full conversation history but creates an independent branch.
pub async fn generate_forked(
    primed: &PrimedSession,
    instruction: &str,
    timeout: Duration,
    model: Option<&str>,
) -> Result<String, AiCliError> {
    let instruction = instruction.to_string();
    let primed = primed.clone();
    let model = model.map(|m| primed.cli.normalize_model(m));

    let fut = async move {
        let output = match primed.cli {
            AiCli::Claude => {
                let mut cmd = Command::new("claude");
                cmd.args(["--print", "--output-format", "text"]);
                cmd.args(["--resume", &primed.session_id, "--fork-session"]);
                if let Some(m) = model {
                    cmd.args(["--model", m]);
                }
                cmd.arg(&instruction);
                cmd.output().await.map_err(AiCliError::SpawnFailed)?
            }
            AiCli::Opencode => {
                let mut cmd = Command::new("opencode");
                cmd.args(["run", "--session", &primed.session_id, "--fork"]);
                if let Some(m) = model {
                    cmd.args(["-m", m]);
                }
                cmd.arg(&instruction);
                cmd.output().await.map_err(AiCliError::SpawnFailed)?
            }
        };

        if !output.status.success() {
            return Err(cli_failure_from_output(&output));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok::<String, AiCliError>(extract_text_content(primed.cli, &stdout))
    };

    tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| AiCliError::Timeout)?
}

/// Extract session ID from JSON output of a CLI call.
fn extract_session_id(cli: AiCli, stdout: &str) -> Result<String, AiCliError> {
    match cli {
        AiCli::Claude => {
            // Claude --output-format json returns a single JSON object with "session_id"
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(stdout.trim()) {
                if let Some(sid) = v.get("session_id").and_then(|s| s.as_str()) {
                    return Ok(sid.to_string());
                }
            }
            Err(AiCliError::CliFailure {
                stderr: "Could not extract session_id from Claude JSON output".to_string(),
            })
        }
        AiCli::Opencode => {
            // Opencode --format json emits newline-delimited JSON events.
            // The sessionID appears in every event; grab from the first one.
            for line in stdout.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                    if let Some(sid) = v.get("sessionID").and_then(|s| s.as_str()) {
                        return Ok(sid.to_string());
                    }
                }
            }
            Err(AiCliError::CliFailure {
                stderr: "Could not extract sessionID from OpenCode JSON output".to_string(),
            })
        }
    }
}

/// Extract the text content from CLI output.
/// For plain text mode this is the raw output; for JSON mode we parse out the text parts.
fn extract_text_content(cli: AiCli, stdout: &str) -> String {
    match cli {
        AiCli::Claude => {
            // With --output-format text, stdout is plain text
            stdout.trim().to_string()
        }
        AiCli::Opencode => {
            // Without --format json, opencode run returns plain text.
            // With --format json, we need to extract text parts from events.
            // Try JSON parse first; if it fails, treat as plain text.
            let mut texts = Vec::new();
            let mut found_json = false;
            for line in stdout.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                    found_json = true;
                    if v.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(part) = v.get("part") {
                            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                texts.push(text.to_string());
                            }
                        }
                    }
                }
            }
            if found_json {
                texts.join("")
            } else {
                stdout.trim().to_string()
            }
        }
    }
}

pub struct AiSettings {
    pub cli: AiCli,
    pub timeout: std::time::Duration,
    pub deep_model: Option<String>,
    pub fast_model: Option<String>,
}

impl AiSettings {
    pub fn model_for_tier(&self, tier: ModelTier) -> Option<&str> {
        match tier {
            ModelTier::Deep => self.deep_model.as_deref(),
            ModelTier::Fast => self.fast_model.as_deref(),
        }
    }
}

pub fn load_ai_settings() -> Option<AiSettings> {
    use super::config::SherpaConfig;
    let config = SherpaConfig::default_path()
        .ok()
        .and_then(|p| SherpaConfig::load(&p).ok())
        .unwrap_or_default();

    let cli = config.ai.selected_cli?;
    let timeout_secs = config.ai.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    Some(AiSettings {
        cli,
        timeout: std::time::Duration::from_secs(timeout_secs),
        deep_model: config.ai.deep_model,
        fast_model: config.ai.fast_model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_prompt_full_prompt() {
        let prompt = AiPrompt {
            context: "You are reviewing code.".to_string(),
            instruction: "Summarize the changes.".to_string(),
        };
        let full = prompt.full_prompt();
        assert!(full.contains("You are reviewing code."));
        assert!(full.contains("Summarize the changes."));
        assert!(full.contains("\n\n"));
    }

    #[test]
    fn test_build_context_normal() {
        let ctx = build_context("feature-x", "main", "M\tsrc/lib.rs", "diff content");
        assert!(ctx.contains("feature-x"));
        assert!(ctx.contains("main"));
        assert!(ctx.contains("src/lib.rs"));
        assert!(ctx.contains("diff content"));
    }

    #[test]
    fn test_build_context_truncates_large_diff() {
        let large_diff = "x".repeat(100_000);
        let ctx = build_context("feature-x", "main", "M\tsrc/lib.rs", &large_diff);
        assert!(ctx.contains("truncated"));
        assert!(ctx.contains("100000 total characters"));
    }

    #[test]
    fn test_error_display() {
        let err = AiCliError::Timeout;
        assert_eq!(err.to_string(), "AI request timed out");

        let err = AiCliError::CliFailure {
            stderr: "some error".to_string(),
        };
        assert!(err.to_string().contains("some error"));

        let err = AiCliError::SpawnFailed(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not found",
        ));
        assert!(err.to_string().contains("Failed to start AI CLI"));
    }

    #[test]
    fn test_default_timeout_secs_is_240() {
        assert_eq!(DEFAULT_TIMEOUT_SECS, 240);
    }

    #[tokio::test]
    async fn test_generate_with_timeout_returns_timeout_error() {
        let prompt = AiPrompt {
            context: "test".to_string(),
            instruction: "test".to_string(),
        };
        let result =
            generate_with_timeout(AiCli::Claude, &prompt, Duration::from_millis(1), None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let is_timeout_or_spawn = matches!(err, AiCliError::Timeout | AiCliError::SpawnFailed(_));
        assert!(is_timeout_or_spawn);
    }

    #[test]
    fn test_instruction_strings_not_empty() {
        assert!(!approach_instruction().is_empty());
        assert!(!review_plan_instruction().is_empty());
    }

    #[test]
    fn test_extract_json_raw_json() {
        let raw = r#"{"steps": [{"title": "t", "rationale": "r", "file_refs": []}]}"#;
        let result = extract_json_from_response(raw).unwrap();
        assert_eq!(result, raw);
    }

    #[test]
    fn test_extract_json_fenced_block() {
        let raw = "Here is the plan:\n```json\n{\"steps\": []}\n```\nDone.";
        let result = extract_json_from_response(raw).unwrap();
        assert_eq!(result, "{\"steps\": []}");
    }

    #[test]
    fn test_extract_json_first_brace_to_last() {
        let raw = "Some preamble text {\"steps\": []} and trailing text";
        let result = extract_json_from_response(raw).unwrap();
        assert_eq!(result, "{\"steps\": []}");
    }

    #[test]
    fn test_extract_json_no_json() {
        let raw = "This has no JSON at all";
        let result = extract_json_from_response(raw);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_json_invalid_fenced_falls_back() {
        let raw = "```json\nnot valid json\n```\n{\"valid\": true}";
        let result = extract_json_from_response(raw).unwrap();
        assert_eq!(result, "{\"valid\": true}");
    }

    #[test]
    fn test_model_tier_equality() {
        assert_eq!(ModelTier::Deep, ModelTier::Deep);
        assert_eq!(ModelTier::Fast, ModelTier::Fast);
        assert_ne!(ModelTier::Deep, ModelTier::Fast);
    }

    #[test]
    fn test_ai_settings_model_for_tier_with_models() {
        let settings = AiSettings {
            cli: AiCli::Claude,
            timeout: std::time::Duration::from_secs(60),
            deep_model: Some("o3".to_string()),
            fast_model: Some("gpt-4o-mini".to_string()),
        };
        assert_eq!(settings.model_for_tier(ModelTier::Deep), Some("o3"));
        assert_eq!(
            settings.model_for_tier(ModelTier::Fast),
            Some("gpt-4o-mini")
        );
    }

    #[test]
    fn test_ai_settings_model_for_tier_defaults_to_none() {
        let settings = AiSettings {
            cli: AiCli::Claude,
            timeout: std::time::Duration::from_secs(60),
            deep_model: None,
            fast_model: None,
        };
        assert_eq!(settings.model_for_tier(ModelTier::Deep), None);
        assert_eq!(settings.model_for_tier(ModelTier::Fast), None);
    }

    #[test]
    fn test_extract_session_id_claude() {
        let json = r#"{"type":"result","session_id":"2754be3c-2637-4d07-b07d-af25b900624b","result":"PONG"}"#;
        let sid = extract_session_id(AiCli::Claude, json).unwrap();
        assert_eq!(sid, "2754be3c-2637-4d07-b07d-af25b900624b");
    }

    #[test]
    fn test_extract_session_id_claude_missing() {
        let json = r#"{"type":"result","result":"PONG"}"#;
        assert!(extract_session_id(AiCli::Claude, json).is_err());
    }

    #[test]
    fn test_extract_session_id_opencode() {
        let ndjson = r#"{"type":"step_start","sessionID":"ses_abc123"}
{"type":"text","sessionID":"ses_abc123","part":{"text":"hello"}}
{"type":"step_finish","sessionID":"ses_abc123"}"#;
        let sid = extract_session_id(AiCli::Opencode, ndjson).unwrap();
        assert_eq!(sid, "ses_abc123");
    }

    #[test]
    fn test_extract_session_id_opencode_missing() {
        let ndjson = r#"{"type":"step_start"}
{"type":"text"}"#;
        assert!(extract_session_id(AiCli::Opencode, ndjson).is_err());
    }

    #[test]
    fn test_extract_text_content_claude_plain() {
        let output = "  Hello world  \n";
        let text = extract_text_content(AiCli::Claude, output);
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn test_extract_text_content_opencode_plain() {
        let output = "  Hello world  \n";
        let text = extract_text_content(AiCli::Opencode, output);
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn test_extract_text_content_opencode_json_events() {
        let ndjson = r#"{"type":"step_start","sessionID":"ses_abc"}
{"type":"text","part":{"text":"Hello "}}
{"type":"text","part":{"text":"world"}}
{"type":"step_finish","sessionID":"ses_abc"}"#;
        let text = extract_text_content(AiCli::Opencode, ndjson);
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn test_primer_instruction_not_empty() {
        assert!(!primer_instruction().is_empty());
    }

    #[test]
    fn test_primed_session_clone() {
        let primed = PrimedSession {
            session_id: "test-123".to_string(),
            cli: AiCli::Claude,
        };
        let cloned = primed.clone();
        assert_eq!(cloned.session_id, "test-123");
        assert_eq!(cloned.cli, AiCli::Claude);
    }

    #[test]
    fn test_cli_failure_from_output_uses_stderr_when_present() {
        let output = std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: b"stdout content".to_vec(),
            stderr: b"stderr error".to_vec(),
        };
        let err = cli_failure_from_output(&output);
        match err {
            AiCliError::CliFailure { stderr } => assert_eq!(stderr, "stderr error"),
            _ => panic!("expected CliFailure"),
        }
    }

    #[test]
    fn test_cli_failure_from_output_falls_back_to_stdout() {
        let output = std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: b"model not found".to_vec(),
            stderr: Vec::new(),
        };
        let err = cli_failure_from_output(&output);
        match err {
            AiCliError::CliFailure { stderr } => assert_eq!(stderr, "model not found"),
            _ => panic!("expected CliFailure"),
        }
    }
}
