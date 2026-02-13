use serde::Serialize;
use std::path::PathBuf;

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
}
