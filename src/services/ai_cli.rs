use std::time::Duration;

use tokio::process::Command;

use super::cli_detection::AiCli;

pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

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

impl std::error::Error for AiCliError {}

pub async fn generate(cli: AiCli, prompt: &AiPrompt) -> Result<String, AiCliError> {
    generate_with_timeout(cli, prompt, Duration::from_secs(DEFAULT_TIMEOUT_SECS)).await
}

pub async fn generate_with_timeout(
    cli: AiCli,
    prompt: &AiPrompt,
    timeout: Duration,
) -> Result<String, AiCliError> {
    let full_prompt = prompt.full_prompt();

    let fut = async move {
        let output = match cli {
            AiCli::Claude => Command::new("claude")
                .args(["--print", &full_prompt])
                .output()
                .await
                .map_err(AiCliError::SpawnFailed)?,
            AiCli::Opencode => Command::new("opencode")
                .args(["run", &full_prompt])
                .output()
                .await
                .map_err(AiCliError::SpawnFailed)?,
        };

        if !output.status.success() {
            return Err(AiCliError::CliFailure {
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
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
        format!("{truncated}\n\n... (diff truncated, {total} total characters)", total = diff.len())
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

pub fn overview_instruction() -> &'static str {
    "Based on the repository structure and changed files, write a 2-3 paragraph summary of \
     what this project is about and which area these changes affect. Be concise and specific."
}

pub fn changes_instruction() -> &'static str {
    "Summarize what changed in this branch in 3-5 bullet points. Focus on what was added, \
     modified, or removed. Be specific about file names and the purpose of each change."
}

pub fn approach_instruction() -> &'static str {
    "Describe the implementation approach taken in these changes in 2-3 paragraphs. Note any \
     design patterns, architectural decisions, or trade-offs. Mention potential concerns if any."
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
     Use diff_lines to reference specific line ranges within each file's diff section, \
     or null for the entire file. Order steps in recommended review sequence."
}

pub fn step_explanation_instruction(step_title: &str, step_diff: &str) -> String {
    format!(
        "You are explaining review step \"{step_title}\".\n\n\
         Here is the diff for this step:\n```\n{step_diff}\n```\n\n\
         Write a clear 2-3 paragraph explanation of what these changes do, why they exist, \
         and any important details a reviewer should notice. Be specific about the code."
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
         Be concise and specific."
    )
}

pub fn step_symbols_instruction(step_diff: &str) -> String {
    format!(
        "Analyze this diff and identify the key symbols (modules, classes, functions, types) \
         that were changed or introduced:\n```\n{step_diff}\n```\n\n\
         Respond with ONLY a JSON array inside a ```json fence:\n\
         ```json\n\
         [\n\
           {{\"name\": \"function_name\", \"kind\": \"function\", \"description\": \"Brief responsibility description\"}}\n\
         ]\n\
         ```\n\
         kind should be one of: module, class, struct, enum, function, method, trait, type, const. \
         Keep descriptions to one sentence each."
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
         Respond helpfully and concisely, referencing specific code from the diff when relevant."
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
    fn test_default_timeout_secs_is_120() {
        assert_eq!(DEFAULT_TIMEOUT_SECS, 120);
    }

    #[tokio::test]
    async fn test_generate_with_timeout_returns_timeout_error() {
        let prompt = AiPrompt {
            context: "test".to_string(),
            instruction: "test".to_string(),
        };
        let result = generate_with_timeout(
            AiCli::Claude,
            &prompt,
            Duration::from_millis(1),
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let is_timeout_or_spawn = matches!(err, AiCliError::Timeout | AiCliError::SpawnFailed(_));
        assert!(is_timeout_or_spawn);
    }

    #[test]
    fn test_instruction_strings_not_empty() {
        assert!(!overview_instruction().is_empty());
        assert!(!changes_instruction().is_empty());
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
}
