use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::git_analysis::{self, ChangedFile, GitAnalysis};

/// Whether the review is happening after all changes are done (PostHoc)
/// or while the developer is still building (Live).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ReviewMode {
    /// Traditional mode: all changes exist upfront, reviewer sees everything at once.
    #[default]
    PostHoc,
    /// Live mode: an agent pushes steps incrementally as the developer codes.
    Live,
}

/// Status of an individual review step in Live mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum StepStatus {
    /// Step is in the plan but the agent hasn't pushed its diff yet.
    #[default]
    Planned,
    /// The agent has pushed the diff — ready for the reviewer to look at.
    ReadyForReview,
    /// The reviewer has validated this step.
    Reviewed,
    /// The reviewer has requested revisions on this step.
    NeedsRevision,
}

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
    #[serde(default)]
    pub status: StepStatus,
    #[serde(default)]
    pub step_diff: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StepAiData {
    pub explanation: Option<String>,
    pub relation_to_previous: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRef {
    pub path: String,
    /// Optional line range within the per-file diff segment (start, end).
    /// Refers to 1-indexed line numbers in the unified diff output for this file.
    /// None means "entire file's diff."
    pub diff_lines: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StepValidation {
    #[serde(default)]
    pub files: HashMap<String, bool>,
}

impl StepValidation {
    /// Empty `expected_files` → treated as validated (no files to review).
    pub fn is_step_validated(&self, expected_files: &[String]) -> bool {
        if expected_files.is_empty() {
            return true;
        }
        expected_files
            .iter()
            .all(|f| self.files.get(f).copied().unwrap_or(false))
    }

    pub fn validate_file(&mut self, path: &str) {
        self.files.insert(path.to_string(), true);
    }

    pub fn is_file_validated(&self, path: &str) -> bool {
        self.files.get(path).copied().unwrap_or(false)
    }

    pub fn ensure_files(&mut self, paths: &[String]) {
        for path in paths {
            self.files.entry(path.clone()).or_insert(false);
        }
    }

    pub fn validated_count(&self) -> usize {
        self.files.values().filter(|&&v| v).count()
    }
}

/// Handles both old `Vec<bool>` and new `Vec<StepValidation>` JSON formats.
pub fn deserialize_validated_steps<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<StepValidation>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ValidatedStepsFormat {
        New(Vec<StepValidation>),
        Legacy(Vec<bool>),
    }

    match ValidatedStepsFormat::deserialize(deserializer) {
        Ok(ValidatedStepsFormat::New(steps)) => Ok(steps),
        Ok(ValidatedStepsFormat::Legacy(bools)) => {
            // Legacy Vec<bool>: file paths unknown here; caller must
            // call ensure_validated_steps_size() to populate them.
            Ok(bools
                .into_iter()
                .map(|validated| {
                    if validated {
                        let mut sv = StepValidation::default();
                        sv.files.insert("__legacy_validated__".to_string(), true);
                        sv
                    } else {
                        StepValidation::default()
                    }
                })
                .collect())
        }
        Err(e) => Err(de::Error::custom(format!(
            "failed to deserialize validated_steps: {e}"
        ))),
    }
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
    #[serde(default, deserialize_with = "deserialize_validated_steps")]
    pub validated_steps: Vec<StepValidation>,
    #[serde(default)]
    pub primed_session_id: Option<String>,
    #[serde(default)]
    pub review_mode: ReviewMode,
    #[serde(default)]
    pub agent_token: Option<String>,
    #[serde(default)]
    pub block_agent: Option<bool>,
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
            primed_session_id: None,
            review_mode: ReviewMode::PostHoc,
            agent_token: None,
            block_agent: None,
        }
    }

    pub fn new_live(
        repo_path: String,
        branch: String,
        plan: ReviewPlan,
        agent_token: String,
    ) -> Self {
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

        let validated_steps = vec![StepValidation::default(); plan.steps.len()];

        Self {
            id,
            repo_path,
            branch,
            default_branch: String::new(),
            merge_base: String::new(),
            diff: String::new(),
            changed_files: Vec::new(),
            created_at,
            summary: SummaryData::default(),
            chat_messages: Vec::new(),
            metrics: DiffMetrics::default(),
            review_plan: Some(plan),
            validated_steps,
            primed_session_id: None,
            review_mode: ReviewMode::Live,
            agent_token: Some(agent_token),
            block_agent: None,
        }
    }

    pub fn ensure_validated_steps_size(&mut self) {
        if let Some(plan) = &self.review_plan {
            let needed = plan.steps.len();
            if self.validated_steps.len() < needed {
                self.validated_steps
                    .resize(needed, StepValidation::default());
            }
            for (i, step) in plan.steps.iter().enumerate() {
                let file_paths: Vec<String> =
                    step.file_refs.iter().map(|f| f.path.clone()).collect();
                self.validated_steps[i].ensure_files(&file_paths);
            }
        }
    }

    pub fn is_step_validated(&self, step_index: usize) -> bool {
        let plan = match &self.review_plan {
            Some(p) => p,
            None => return false,
        };
        let step = match plan.steps.get(step_index) {
            Some(s) => s,
            None => return false,
        };
        let sv = match self.validated_steps.get(step_index) {
            Some(sv) => sv,
            None => return false,
        };
        let expected: Vec<String> = step.file_refs.iter().map(|f| f.path.clone()).collect();
        sv.is_step_validated(&expected)
    }

    pub fn all_steps_validated(&self) -> bool {
        let plan = match &self.review_plan {
            Some(p) => p,
            None => return false,
        };
        if plan.steps.is_empty() {
            return false;
        }
        (0..plan.steps.len()).all(|i| self.is_step_validated(i))
    }

    pub fn first_unvalidated_step(&self) -> Option<usize> {
        self.review_plan.as_ref()?;
        (0..self.validated_steps.len())
            .find(|&i| !self.is_step_validated(i))
            .map(|i| i + 1)
    }

    pub fn steps_ready_count(&self) -> usize {
        self.review_plan
            .as_ref()
            .map(|plan| {
                plan.steps
                    .iter()
                    .filter(|s| s.status != StepStatus::Planned)
                    .count()
            })
            .unwrap_or(0)
    }

    pub fn is_live(&self) -> bool {
        self.review_mode == ReviewMode::Live
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
            status: StepStatus::ReadyForReview,
            step_diff: None,
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
                status: StepStatus::default(),
                step_diff: None,
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
    fn test_new_live_creates_live_session() {
        let plan = ReviewPlan {
            steps: vec![
                ReviewStep {
                    title: "Step 1".to_string(),
                    rationale: "First change".to_string(),
                    file_refs: vec![FileRef {
                        path: "src/lib.rs".to_string(),
                        diff_lines: None,
                    }],
                    ai_data: StepAiData::default(),
                    status: StepStatus::Planned,
                    step_diff: None,
                },
                ReviewStep {
                    title: "Step 2".to_string(),
                    rationale: "Second change".to_string(),
                    file_refs: vec![],
                    ai_data: StepAiData::default(),
                    status: StepStatus::Planned,
                    step_diff: None,
                },
            ],
            generated_at: "now".to_string(),
        };

        let session = ReviewSession::new_live(
            "/tmp/test-repo".to_string(),
            "feature-branch".to_string(),
            plan,
            "test-token-123".to_string(),
        );

        assert!(session.id.starts_with("review-"));
        assert_eq!(session.repo_path, "/tmp/test-repo");
        assert_eq!(session.branch, "feature-branch");
        assert_eq!(session.review_mode, ReviewMode::Live);
        assert_eq!(session.agent_token, Some("test-token-123".to_string()));
        assert!(session.review_plan.is_some());
        assert_eq!(session.review_plan.as_ref().unwrap().steps.len(), 2);
        assert_eq!(session.validated_steps.len(), 2);
        assert!(!session.is_step_validated(0));
        // Step 2 has no file_refs, so it's trivially validated
        assert!(session.is_step_validated(1));
        assert!(session.diff.is_empty());
        assert!(session.is_live());
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
                    file_refs: vec![FileRef {
                        path: "a.rs".to_string(),
                        diff_lines: None,
                    }],
                    ai_data: StepAiData::default(),
                    status: StepStatus::default(),
                    step_diff: None,
                },
                ReviewStep {
                    title: "Step 2".to_string(),
                    rationale: "r".to_string(),
                    file_refs: vec![FileRef {
                        path: "b.rs".to_string(),
                        diff_lines: None,
                    }],
                    ai_data: StepAiData::default(),
                    status: StepStatus::default(),
                    step_diff: None,
                },
            ],
            generated_at: "now".to_string(),
        });
        session.validated_steps = vec![StepValidation::default(), StepValidation::default()];
        session.ensure_validated_steps_size();
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
                    file_refs: vec![FileRef {
                        path: "a.rs".to_string(),
                        diff_lines: None,
                    }],
                    ai_data: StepAiData::default(),
                    status: StepStatus::default(),
                    step_diff: None,
                },
                ReviewStep {
                    title: "Step 2".to_string(),
                    rationale: "r".to_string(),
                    file_refs: vec![FileRef {
                        path: "b.rs".to_string(),
                        diff_lines: None,
                    }],
                    ai_data: StepAiData::default(),
                    status: StepStatus::default(),
                    step_diff: None,
                },
            ],
            generated_at: "now".to_string(),
        });
        let mut sv1 = StepValidation::default();
        sv1.validate_file("a.rs");
        session.validated_steps = vec![sv1, StepValidation::default()];
        session.ensure_validated_steps_size();
        assert_eq!(session.first_unvalidated_step(), Some(2));
    }

    #[test]
    fn test_first_unvalidated_step_all_validated() {
        let mut session = ReviewSession::new(make_test_analysis());
        session.review_plan = Some(ReviewPlan {
            steps: vec![ReviewStep {
                title: "Step 1".to_string(),
                rationale: "r".to_string(),
                file_refs: vec![FileRef {
                    path: "a.rs".to_string(),
                    diff_lines: None,
                }],
                ai_data: StepAiData::default(),
                status: StepStatus::default(),
                step_diff: None,
            }],
            generated_at: "now".to_string(),
        });
        let mut sv = StepValidation::default();
        sv.validate_file("a.rs");
        session.validated_steps = vec![sv];
        session.ensure_validated_steps_size();
        assert_eq!(session.first_unvalidated_step(), None);
    }

    #[test]
    fn test_step_validation_per_file() {
        let mut sv = StepValidation::default();
        sv.ensure_files(&["a.rs".to_string(), "b.rs".to_string()]);
        assert!(!sv.is_file_validated("a.rs"));
        assert!(!sv.is_step_validated(&["a.rs".to_string(), "b.rs".to_string(),]));

        sv.validate_file("a.rs");
        assert!(sv.is_file_validated("a.rs"));
        assert!(!sv.is_file_validated("b.rs"));
        assert_eq!(sv.validated_count(), 1);

        sv.validate_file("b.rs");
        assert!(sv.is_step_validated(&["a.rs".to_string(), "b.rs".to_string(),]));
        assert_eq!(sv.validated_count(), 2);
    }

    #[test]
    fn test_backward_compat_legacy_bool_deserialize() {
        let json = r#"{
            "id": "review-123",
            "repo_path": "/tmp/test",
            "branch": "feature",
            "default_branch": "main",
            "merge_base": "abc",
            "diff": "",
            "changed_files": [],
            "created_at": "2025-01-01T00:00:00Z",
            "validated_steps": [true, false]
        }"#;
        let session: ReviewSession = serde_json::from_str(json).unwrap();
        assert_eq!(session.validated_steps.len(), 2);
        assert!(session.validated_steps[0]
            .files
            .contains_key("__legacy_validated__"));
        assert!(session.validated_steps[1].files.is_empty());
    }

    #[test]
    fn test_needs_revision_serialization_roundtrip() {
        let step = ReviewStep {
            title: "Revise this".to_string(),
            rationale: "Needs work".to_string(),
            file_refs: vec![FileRef {
                path: "src/lib.rs".to_string(),
                diff_lines: None,
            }],
            ai_data: StepAiData::default(),
            status: StepStatus::NeedsRevision,
            step_diff: Some("diff content".to_string()),
        };

        let json = serde_json::to_string(&step).unwrap();
        assert!(json.contains("NeedsRevision"));

        let deserialized: ReviewStep = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.status, StepStatus::NeedsRevision);
        assert_eq!(deserialized.title, "Revise this");
    }

    #[test]
    fn test_block_agent_serialization_roundtrip() {
        let mut session = ReviewSession::new(make_test_analysis());
        assert_eq!(session.block_agent, None);

        session.block_agent = Some(true);
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("\"block_agent\":true"));

        let loaded: ReviewSession = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.block_agent, Some(true));
    }

    #[test]
    fn test_block_agent_defaults_to_none() {
        let json = r#"{
            "id": "review-123",
            "repo_path": "/tmp/test",
            "branch": "main",
            "default_branch": "main",
            "merge_base": "abc",
            "diff": "",
            "changed_files": [],
            "created_at": "2025-01-01T00:00:00.000Z"
        }"#;
        let session: ReviewSession = serde_json::from_str(json).unwrap();
        assert_eq!(session.block_agent, None);
    }
}
