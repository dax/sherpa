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
    /// Which review step this message belongs to.
    /// `None` for summary-level chat, `Some(n)` for step n.
    #[serde(default)]
    pub step_number: Option<usize>,
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
    #[serde(default)]
    pub ai_data: StepAiData,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StepAiData {
    pub explanation: Option<String>,
    pub relation_to_previous: Option<String>,
    pub symbols: Option<Vec<SymbolInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: String,
    pub description: String,
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
    #[serde(default)]
    pub validated_steps: Vec<bool>,
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
            validated_steps: Vec::new(),
        }
    }

    pub fn ensure_validated_steps_size(&mut self) {
        if let Some(plan) = &self.review_plan {
            let needed = plan.steps.len();
            if self.validated_steps.len() < needed {
                self.validated_steps.resize(needed, false);
            }
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

        // Best-effort write to repo-local .sherpa/ directory
        let _ = self.save_to_repo();

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

    pub fn save_to_repo(&self) -> Result<(), SessionError> {
        let file_path = repo_session_path(&self.repo_path, &self.branch);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).map_err(SessionError::Io)?;
        }

        let content = serde_json::to_string_pretty(self).map_err(SessionError::Serialize)?;
        let tmp_path = file_path.with_extension("json.tmp");
        let mut file = fs::File::create(&tmp_path).map_err(SessionError::Io)?;
        file.write_all(content.as_bytes())
            .map_err(SessionError::Io)?;
        file.sync_all().map_err(SessionError::Io)?;
        fs::rename(&tmp_path, &file_path).map_err(SessionError::Io)?;

        Ok(())
    }

    pub fn find_existing(repo_path: &str, branch: &str) -> Option<Self> {
        let path = repo_session_path(repo_path, branch);
        Self::load_from(&path).ok()
    }

    pub fn delete_repo_session(repo_path: &str, branch: &str) -> Result<(), SessionError> {
        let path = repo_session_path(repo_path, branch);
        if path.exists() {
            fs::remove_file(&path).map_err(SessionError::Io)?;
        }
        Ok(())
    }

    pub fn first_unvalidated_step(&self) -> Option<usize> {
        self.review_plan.as_ref()?;
        self.validated_steps.iter().position(|&v| !v).map(|i| i + 1)
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
            ai_data: StepAiData::default(),
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

/// Sanitize a git branch name for use as a filename.
///
/// Replaces `/` with `--` and strips characters that are invalid in filenames.
pub fn sanitize_branch_name(branch: &str) -> String {
    branch
        .replace('/', "--")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .collect()
}

/// Return the path to a repo-local session file for the given repo+branch.
///
/// The session is stored at `{repo_path}/.sherpa/review-{sanitized_branch}.json`.
pub fn repo_session_path(repo_path: &str, branch: &str) -> PathBuf {
    let sanitized = sanitize_branch_name(branch);
    Path::new(repo_path)
        .join(".sherpa")
        .join(format!("review-{sanitized}.json"))
}

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
                ai_data: StepAiData::default(),
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
                ai_data: StepAiData::default(),
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

    #[test]
    fn test_sanitize_branch_name_simple() {
        assert_eq!(sanitize_branch_name("main"), "main");
        assert_eq!(sanitize_branch_name("feature-branch"), "feature-branch");
    }

    #[test]
    fn test_sanitize_branch_name_with_slashes() {
        assert_eq!(sanitize_branch_name("feature/auth"), "feature--auth");
        assert_eq!(
            sanitize_branch_name("feature/auth/oauth"),
            "feature--auth--oauth"
        );
    }

    #[test]
    fn test_sanitize_branch_name_strips_invalid_chars() {
        assert_eq!(sanitize_branch_name("feat:test"), "feattest");
        assert_eq!(sanitize_branch_name("my branch"), "mybranch");
    }

    #[test]
    fn test_repo_session_path_structure() {
        let path = repo_session_path("/tmp/my-repo", "feature/auth");
        assert_eq!(
            path,
            PathBuf::from("/tmp/my-repo/.sherpa/review-feature--auth.json")
        );
    }

    #[test]
    fn test_save_to_repo_and_find_existing_roundtrip() {
        let dir = std::env::temp_dir().join("sherpa_test_repo_local");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut analysis = make_test_analysis();
        analysis.repo_path = dir.to_string_lossy().to_string();
        let session = ReviewSession::new(analysis);
        session.save_to_repo().unwrap();

        let found = ReviewSession::find_existing(&dir.to_string_lossy(), "feature-branch");
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.id, session.id);
        assert_eq!(found.branch, "feature-branch");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_existing_returns_none_for_missing() {
        let found = ReviewSession::find_existing("/tmp/sherpa_nonexistent_repo", "no-branch");
        assert!(found.is_none());
    }

    #[test]
    fn test_delete_repo_session() {
        let dir = std::env::temp_dir().join("sherpa_test_delete_repo");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut analysis = make_test_analysis();
        analysis.repo_path = dir.to_string_lossy().to_string();
        let session = ReviewSession::new(analysis);
        session.save_to_repo().unwrap();

        let path = repo_session_path(&dir.to_string_lossy(), "feature-branch");
        assert!(path.exists());

        ReviewSession::delete_repo_session(&dir.to_string_lossy(), "feature-branch").unwrap();
        assert!(!path.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_delete_repo_session_noop_for_missing() {
        let result = ReviewSession::delete_repo_session("/tmp/sherpa_nonexistent", "no-branch");
        assert!(result.is_ok());
    }

    #[test]
    fn test_first_unvalidated_step_no_plan() {
        let session = ReviewSession::new(make_test_analysis());
        assert_eq!(session.first_unvalidated_step(), None);
    }

    #[test]
    fn test_first_unvalidated_step_all_false() {
        let mut session = ReviewSession::new(make_test_analysis());
        session.review_plan = Some(ReviewPlan {
            steps: vec![
                ReviewStep {
                    title: "Step 1".to_string(),
                    rationale: "r".to_string(),
                    file_refs: vec![],
                    ai_data: StepAiData::default(),
                },
                ReviewStep {
                    title: "Step 2".to_string(),
                    rationale: "r".to_string(),
                    file_refs: vec![],
                    ai_data: StepAiData::default(),
                },
            ],
            generated_at: "now".to_string(),
        });
        session.validated_steps = vec![false, false];
        assert_eq!(session.first_unvalidated_step(), Some(1));
    }

    #[test]
    fn test_first_unvalidated_step_partial() {
        let mut session = ReviewSession::new(make_test_analysis());
        session.review_plan = Some(ReviewPlan {
            steps: vec![
                ReviewStep {
                    title: "Step 1".to_string(),
                    rationale: "r".to_string(),
                    file_refs: vec![],
                    ai_data: StepAiData::default(),
                },
                ReviewStep {
                    title: "Step 2".to_string(),
                    rationale: "r".to_string(),
                    file_refs: vec![],
                    ai_data: StepAiData::default(),
                },
            ],
            generated_at: "now".to_string(),
        });
        session.validated_steps = vec![true, false];
        assert_eq!(session.first_unvalidated_step(), Some(2));
    }

    #[test]
    fn test_first_unvalidated_step_all_validated() {
        let mut session = ReviewSession::new(make_test_analysis());
        session.review_plan = Some(ReviewPlan {
            steps: vec![ReviewStep {
                title: "Step 1".to_string(),
                rationale: "r".to_string(),
                file_refs: vec![],
                ai_data: StepAiData::default(),
            }],
            generated_at: "now".to_string(),
        });
        session.validated_steps = vec![true];
        assert_eq!(session.first_unvalidated_step(), None);
    }
}
