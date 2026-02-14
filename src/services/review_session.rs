use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::git_analysis::{self, ChangedFile, GitAnalysis};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SummaryData {
    pub overview: Option<String>,
    pub changes: Option<String>,
    pub approach: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewPlan {
    pub steps: Vec<ReviewStep>,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewStep {
    pub title: String,
    pub rationale: String,
    pub file_refs: Vec<FileRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRef {
    pub path: String,
    /// Optional line range within the per-file diff segment (start, end).
    /// Refers to 1-indexed line numbers in the unified diff output for this file.
    /// None means "entire file's diff."
    pub diff_lines: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSession {
    pub id: String,
    pub repo_path: String,
    pub branch: String,
    pub default_branch: String,
    pub merge_base: String,
    pub diff: String,
    pub changed_files: Vec<ChangedFile>,
    pub created_at: String,
    #[serde(default)]
    pub summary: SummaryData,
    #[serde(default)]
    pub chat_messages: Vec<ChatMessage>,
    #[serde(default)]
    pub metrics: DiffMetrics,
    #[serde(default)]
    pub review_plan: Option<ReviewPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiffMetrics {
    pub files_changed: usize,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub commits_on_branch: usize,
}

impl ReviewSession {
    pub fn new(analysis: GitAnalysis) -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX epoch")
            .as_millis();
        let id = format!("review-{millis}");

        let created_at = {
            let secs = millis / 1000;
            let remaining_millis = millis % 1000;
            let days_since_epoch = secs / 86400;
            let time_of_day = secs % 86400;
            let hours = time_of_day / 3600;
            let minutes = (time_of_day % 3600) / 60;
            let seconds = time_of_day % 60;

            let (year, month, day) = days_to_ymd(days_since_epoch as i64);
            format!(
                "{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{remaining_millis:03}Z"
            )
        };

        let (lines_added, lines_removed) = git_analysis::compute_diff_line_stats(&analysis.diff);
        let metrics = DiffMetrics {
            files_changed: analysis.changed_files.len(),
            lines_added,
            lines_removed,
            commits_on_branch: analysis.commit_count,
        };

        Self {
            id,
            repo_path: analysis.repo_path,
            branch: analysis.current_branch,
            default_branch: analysis.default_branch,
            merge_base: analysis.merge_base,
            diff: analysis.diff,
            changed_files: analysis.changed_files,
            created_at,
            summary: SummaryData::default(),
            chat_messages: Vec::new(),
            metrics,
            review_plan: None,
        }
    }

    pub fn sessions_dir() -> Result<PathBuf, SessionError> {
        dirs::home_dir()
            .map(|h| h.join(".sherpa").join("sessions"))
            .ok_or(SessionError::NoHomeDir)
    }

    pub fn save(&self) -> Result<(), SessionError> {
        let dir = Self::sessions_dir()?;
        fs::create_dir_all(&dir).map_err(SessionError::Io)?;

        let file_path = dir.join(format!("{}.json", self.id));
        let content = serde_json::to_string_pretty(self).map_err(SessionError::Serialize)?;

        let tmp_path = file_path.with_extension("json.tmp");
        let mut file = fs::File::create(&tmp_path).map_err(SessionError::Io)?;
        file.write_all(content.as_bytes())
            .map_err(SessionError::Io)?;
        file.sync_all().map_err(SessionError::Io)?;
        fs::rename(&tmp_path, &file_path).map_err(SessionError::Io)?;

        Ok(())
    }

    pub fn load(id: &str) -> Result<Self, SessionError> {
        let dir = Self::sessions_dir()?;
        let file_path = dir.join(format!("{id}.json"));
        Self::load_from(&file_path)
    }

    fn load_from(path: &Path) -> Result<Self, SessionError> {
        if !path.exists() {
            return Err(SessionError::NotFound(path.to_string_lossy().to_string()));
        }
        let content = fs::read_to_string(path).map_err(SessionError::Io)?;
        serde_json::from_str(&content).map_err(SessionError::Parse)
    }
}

pub fn fallback_review_plan(session: &ReviewSession) -> ReviewPlan {
    let steps = session
        .changed_files
        .iter()
        .map(|f| ReviewStep {
            title: format!("Review {}", f.path),
            rationale: format!("{} file", f.status),
            file_refs: vec![FileRef {
                path: f.path.clone(),
                diff_lines: None,
            }],
        })
        .collect();

    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis();
    let secs = millis / 1000;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    ReviewPlan {
        steps,
        generated_at: format!("{hours:02}:{minutes:02}:{seconds:02}"),
    }
}

fn days_to_ymd(mut days: i64) -> (i64, u32, u32) {
    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = (days - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

#[derive(Debug)]
pub enum SessionError {
    Io(std::io::Error),
    Serialize(serde_json::Error),
    Parse(serde_json::Error),
    NotFound(String),
    NoHomeDir,
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "session I/O error: {e}"),
            Self::Serialize(e) => write!(f, "session serialize error: {e}"),
            Self::Parse(e) => write!(f, "session parse error: {e}"),
            Self::NotFound(path) => write!(f, "session not found: {path}"),
            Self::NoHomeDir => write!(f, "could not determine home directory"),
        }
    }
}

impl std::error::Error for SessionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::git_analysis::{ChangedFile, FileStatus};

    fn make_test_analysis() -> GitAnalysis {
        GitAnalysis {
            repo_path: "/tmp/test-repo".to_string(),
            current_branch: "feature-branch".to_string(),
            default_branch: "main".to_string(),
            merge_base: "abc123def456".to_string(),
            diff: "diff --git a/file.rs b/file.rs\n+new line".to_string(),
            changed_files: vec![ChangedFile {
                path: "file.rs".to_string(),
                status: FileStatus::Modified,
            }],
            commit_count: 3,
        }
    }

    #[test]
    fn test_new_session_generates_id() {
        let session = ReviewSession::new(make_test_analysis());
        assert!(session.id.starts_with("review-"));
        assert_eq!(session.branch, "feature-branch");
        assert_eq!(session.default_branch, "main");
        assert!(!session.created_at.is_empty());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("sherpa_test_sessions_roundtrip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let session = ReviewSession::new(make_test_analysis());
        let file_path = dir.join(format!("{}.json", session.id));

        let content = serde_json::to_string_pretty(&session).unwrap();
        fs::write(&file_path, content).unwrap();

        let loaded = ReviewSession::load_from(&file_path).unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.repo_path, session.repo_path);
        assert_eq!(loaded.branch, session.branch);
        assert_eq!(loaded.changed_files.len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_nonexistent_returns_error() {
        let path = PathBuf::from("/tmp/sherpa_nonexistent_session/does-not-exist.json");
        let result = ReviewSession::load_from(&path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, SessionError::NotFound(_)));
    }

    #[test]
    fn test_created_at_is_iso8601() {
        let session = ReviewSession::new(make_test_analysis());
        assert!(session.created_at.ends_with('Z'));
        assert!(session.created_at.contains('T'));
    }

    #[test]
    fn test_new_session_has_no_review_plan() {
        let session = ReviewSession::new(make_test_analysis());
        assert!(session.review_plan.is_none());
    }

    #[test]
    fn test_fallback_review_plan_creates_one_step_per_file() {
        let mut analysis = make_test_analysis();
        analysis.changed_files = vec![
            ChangedFile {
                path: "src/lib.rs".to_string(),
                status: FileStatus::Modified,
            },
            ChangedFile {
                path: "src/new.rs".to_string(),
                status: FileStatus::Added,
            },
            ChangedFile {
                path: "src/old.rs".to_string(),
                status: FileStatus::Deleted,
            },
        ];
        let session = ReviewSession::new(analysis);
        let plan = fallback_review_plan(&session);

        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].title, "Review src/lib.rs");
        assert_eq!(plan.steps[0].file_refs.len(), 1);
        assert_eq!(plan.steps[0].file_refs[0].path, "src/lib.rs");
        assert!(plan.steps[0].file_refs[0].diff_lines.is_none());

        assert_eq!(plan.steps[1].title, "Review src/new.rs");
        assert!(plan.steps[1].rationale.contains("Added"));

        assert_eq!(plan.steps[2].title, "Review src/old.rs");
        assert!(plan.steps[2].rationale.contains("Deleted"));

        assert!(!plan.generated_at.is_empty());
    }

    #[test]
    fn test_review_plan_serialization_roundtrip() {
        let plan = ReviewPlan {
            steps: vec![ReviewStep {
                title: "Core changes".to_string(),
                rationale: "Foundation code".to_string(),
                file_refs: vec![
                    FileRef {
                        path: "src/lib.rs".to_string(),
                        diff_lines: Some((1, 20)),
                    },
                    FileRef {
                        path: "src/main.rs".to_string(),
                        diff_lines: None,
                    },
                ],
            }],
            generated_at: "12:00:00".to_string(),
        };

        let json = serde_json::to_string(&plan).unwrap();
        let deserialized: ReviewPlan = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.steps.len(), 1);
        assert_eq!(deserialized.steps[0].title, "Core changes");
        assert_eq!(deserialized.steps[0].file_refs.len(), 2);
        assert_eq!(deserialized.steps[0].file_refs[0].diff_lines, Some((1, 20)));
        assert!(deserialized.steps[0].file_refs[1].diff_lines.is_none());
    }

    #[test]
    fn test_session_with_review_plan_roundtrip() {
        let dir = std::env::temp_dir().join("sherpa_test_sessions_plan");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut session = ReviewSession::new(make_test_analysis());
        session.review_plan = Some(ReviewPlan {
            steps: vec![ReviewStep {
                title: "Test step".to_string(),
                rationale: "Testing".to_string(),
                file_refs: vec![FileRef {
                    path: "file.rs".to_string(),
                    diff_lines: None,
                }],
            }],
            generated_at: "10:00:00".to_string(),
        });

        let file_path = dir.join(format!("{}.json", session.id));
        let content = serde_json::to_string_pretty(&session).unwrap();
        fs::write(&file_path, content).unwrap();

        let loaded = ReviewSession::load_from(&file_path).unwrap();
        assert!(loaded.review_plan.is_some());
        let plan = loaded.review_plan.unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].title, "Test step");

        let _ = fs::remove_dir_all(&dir);
    }
}
