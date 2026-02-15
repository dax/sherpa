use serde::Serialize;
use std::path::PathBuf;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiCli {
    Opencode,
    Claude,
}

impl AiCli {
    pub fn binary_name(self) -> &'static str {
        match self {
            Self::Opencode => "opencode",
            Self::Claude => "claude",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Opencode => "OpenCode",
            Self::Claude => "Claude Code",
        }
    }

    pub fn all() -> &'static [AiCli] {
        &[AiCli::Opencode, AiCli::Claude]
    }

    /// Strip `provider/` prefix for Claude (e.g. `anthropic/claude-sonnet-4-5` → `claude-sonnet-4-5`).
    /// OpenCode model names pass through unchanged.
    pub fn normalize_model(self, model: &str) -> &str {
        match self {
            Self::Claude => model.rsplit('/').next().unwrap_or(model),
            Self::Opencode => model,
        }
    }
}

impl std::fmt::Display for AiCli {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CliStatus {
    pub cli: AiCli,
    pub available: bool,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DetectionResult {
    pub tools: Vec<CliStatus>,
}

impl DetectionResult {
    pub fn available(&self) -> Vec<&CliStatus> {
        self.tools.iter().filter(|t| t.available).collect()
    }

    pub fn none_available(&self) -> bool {
        self.available().is_empty()
    }

    pub fn single_available(&self) -> Option<AiCli> {
        let avail = self.available();
        if avail.len() == 1 {
            Some(avail[0].cli)
        } else {
            None
        }
    }
}

/// Claude Code has no `models` subcommand, so we maintain a static list.
const CLAUDE_MODELS: &[&str] = &[
    "sonnet",
    "opus",
    "haiku",
    "claude-sonnet-4-5-20250929",
    "claude-sonnet-4-20250514",
    "claude-opus-4-6",
    "claude-opus-4-5-20251101",
    "claude-opus-4-1-20250805",
    "claude-opus-4-20250514",
    "claude-haiku-4-5-20251001",
    "claude-3-7-sonnet-20250219",
    "claude-3-5-sonnet-20241022",
    "claude-3-5-haiku-20241022",
];

/// OpenCode: runs `opencode models`; Claude Code: returns a hardcoded list.
pub async fn list_models(cli: AiCli) -> Vec<String> {
    match cli {
        AiCli::Opencode => list_opencode_models().await,
        AiCli::Claude => CLAUDE_MODELS.iter().map(|s| (*s).to_string()).collect(),
    }
}

async fn list_opencode_models() -> Vec<String> {
    let output = Command::new("opencode").arg("models").output().await;

    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

pub fn detect_cli_tools() -> DetectionResult {
    let tools = AiCli::all()
        .iter()
        .map(|&cli| {
            let result = which::which(cli.binary_name());
            CliStatus {
                cli,
                available: result.is_ok(),
                path: result.ok(),
            }
        })
        .collect();

    DetectionResult { tools }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_cli_binary_names() {
        assert_eq!(AiCli::Opencode.binary_name(), "opencode");
        assert_eq!(AiCli::Claude.binary_name(), "claude");
    }

    #[test]
    fn test_ai_cli_display_names() {
        assert_eq!(AiCli::Opencode.display_name(), "OpenCode");
        assert_eq!(AiCli::Claude.display_name(), "Claude Code");
    }

    #[test]
    fn test_detection_result_helpers() {
        let result = DetectionResult {
            tools: vec![
                CliStatus {
                    cli: AiCli::Opencode,
                    available: true,
                    path: Some(PathBuf::from("/usr/bin/opencode")),
                },
                CliStatus {
                    cli: AiCli::Claude,
                    available: false,
                    path: None,
                },
            ],
        };

        assert_eq!(result.available().len(), 1);
        assert!(!result.none_available());
        assert_eq!(result.single_available(), Some(AiCli::Opencode));
    }

    #[test]
    fn test_detection_result_none_available() {
        let result = DetectionResult {
            tools: vec![
                CliStatus {
                    cli: AiCli::Opencode,
                    available: false,
                    path: None,
                },
                CliStatus {
                    cli: AiCli::Claude,
                    available: false,
                    path: None,
                },
            ],
        };

        assert!(result.none_available());
        assert_eq!(result.single_available(), None);
    }

    #[test]
    fn test_detection_result_both_available() {
        let result = DetectionResult {
            tools: vec![
                CliStatus {
                    cli: AiCli::Opencode,
                    available: true,
                    path: Some(PathBuf::from("/usr/bin/opencode")),
                },
                CliStatus {
                    cli: AiCli::Claude,
                    available: true,
                    path: Some(PathBuf::from("/usr/bin/claude")),
                },
            ],
        };

        assert_eq!(result.available().len(), 2);
        assert!(!result.none_available());
        assert_eq!(result.single_available(), None);
    }

    #[test]
    fn test_detect_cli_tools_runs_without_panic() {
        let result = detect_cli_tools();
        assert_eq!(result.tools.len(), 2);
    }

    #[tokio::test]
    async fn test_list_models_claude_returns_hardcoded() {
        let models = list_models(AiCli::Claude).await;
        assert!(!models.is_empty());
        assert!(models.contains(&"sonnet".to_string()));
        assert!(models.contains(&"opus".to_string()));
        assert!(models.contains(&"haiku".to_string()));
    }

    #[tokio::test]
    async fn test_list_models_opencode_returns_without_panic() {
        let models = list_models(AiCli::Opencode).await;
        if which::which("opencode").is_ok() {
            assert!(!models.is_empty());
        } else {
            assert!(models.is_empty());
        }
    }

    #[test]
    fn test_normalize_model_claude_strips_provider_prefix() {
        assert_eq!(
            AiCli::Claude.normalize_model("anthropic/claude-sonnet-4-5-20250929"),
            "claude-sonnet-4-5-20250929"
        );
    }

    #[test]
    fn test_normalize_model_claude_keeps_bare_name() {
        assert_eq!(AiCli::Claude.normalize_model("sonnet"), "sonnet");
        assert_eq!(AiCli::Claude.normalize_model("opus"), "opus");
        assert_eq!(
            AiCli::Claude.normalize_model("claude-opus-4-6"),
            "claude-opus-4-6"
        );
    }

    #[test]
    fn test_normalize_model_opencode_preserves_prefix() {
        assert_eq!(
            AiCli::Opencode.normalize_model("anthropic/claude-sonnet-4-5-20250929"),
            "anthropic/claude-sonnet-4-5-20250929"
        );
        assert_eq!(AiCli::Opencode.normalize_model("sonnet"), "sonnet");
    }
}
