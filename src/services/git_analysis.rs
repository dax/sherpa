use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct GitAnalysis {
    pub repo_path: String,
    pub current_branch: String,
    pub default_branch: String,
    pub merge_base: String,
    pub diff: String,
    pub changed_files: Vec<ChangedFile>,
    pub commit_count: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChangedFile {
    pub path: String,
    pub status: FileStatus,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

impl FileStatus {
    pub fn from_status_char(c: &str) -> Option<Self> {
        match c {
            "A" => Some(Self::Added),
            "M" => Some(Self::Modified),
            "D" => Some(Self::Deleted),
            s if s.starts_with('R') => Some(Self::Renamed),
            _ => None,
        }
    }
}

impl std::fmt::Display for FileStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Added => write!(f, "Added"),
            Self::Modified => write!(f, "Modified"),
            Self::Deleted => write!(f, "Deleted"),
            Self::Renamed => write!(f, "Renamed"),
        }
    }
}

#[derive(Debug)]
pub enum GitAnalysisError {
    NotADirectory(String),
    NotAGitRepo(String),
    OnDefaultBranch(String),
    NoDefaultBranch,
    NoBranch,
    GitCommandFailed { command: String, stderr: String },
}

impl std::fmt::Display for GitAnalysisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotADirectory(path) => write!(f, "Path is not a directory: {path}"),
            Self::NotAGitRepo(path) => write!(f, "Not a git repository: {path}"),
            Self::OnDefaultBranch(branch) => write!(
                f,
                "You are on the default branch ({branch}). Switch to a feature branch."
            ),
            Self::NoDefaultBranch => write!(f, "Could not detect the default branch (main/master)"),
            Self::NoBranch => write!(f, "HEAD is detached — not on any branch"),
            Self::GitCommandFailed { command, stderr } => {
                write!(f, "Git command failed: {command}\n{stderr}")
            }
        }
    }
}

impl std::error::Error for GitAnalysisError {}

fn run_git(path: &Path, args: &[&str]) -> Result<String, GitAnalysisError> {
    let path_str = path.to_string_lossy().to_string();
    let mut cmd_args = vec!["-C", &path_str];
    cmd_args.extend_from_slice(args);

    let output = Command::new("git").args(&cmd_args).output().map_err(|e| {
        GitAnalysisError::GitCommandFailed {
            command: format!("git {}", args.join(" ")),
            stderr: e.to_string(),
        }
    })?;

    if !output.status.success() {
        return Err(GitAnalysisError::GitCommandFailed {
            command: format!("git {}", args.join(" ")),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_git_quiet(path: &Path, args: &[&str]) -> bool {
    let path_str = path.to_string_lossy().to_string();
    let mut cmd_args = vec!["-C", &path_str];
    cmd_args.extend_from_slice(args);

    Command::new("git")
        .args(&cmd_args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn parse_name_status_line(line: &str) -> Option<ChangedFile> {
    let parts: Vec<&str> = line.split('\t').collect();
    if parts.len() < 2 {
        return None;
    }
    let status = FileStatus::from_status_char(parts[0])?;
    let path = parts.last()?.to_string();
    Some(ChangedFile { path, status })
}

fn detect_default_branch(path: &Path) -> Result<String, GitAnalysisError> {
    if let Ok(symbolic) = run_git(path, &["symbolic-ref", "refs/remotes/origin/HEAD"]) {
        if let Some(branch) = symbolic.strip_prefix("refs/remotes/origin/") {
            return Ok(branch.to_string());
        }
    }

    if run_git_quiet(
        path,
        &["show-ref", "--verify", "--quiet", "refs/heads/main"],
    ) {
        return Ok("main".to_string());
    }

    if run_git_quiet(
        path,
        &["show-ref", "--verify", "--quiet", "refs/heads/master"],
    ) {
        return Ok("master".to_string());
    }

    Err(GitAnalysisError::NoDefaultBranch)
}

fn count_commits(path: &Path, range: &str) -> Result<usize, GitAnalysisError> {
    let output = run_git(path, &["rev-list", "--count", range])?;
    output
        .parse::<usize>()
        .map_err(|_| GitAnalysisError::GitCommandFailed {
            command: "git rev-list --count".to_string(),
            stderr: format!("Could not parse commit count: {output}"),
        })
}

/// Extract the unified diff section(s) for the given file paths from a full
/// multi-file unified diff string.  Returns a new unified diff containing only
/// the matching file sections.
///
/// If `diff_lines` is `Some((start, end))`, the returned diff for that file is
/// further narrowed to lines `start..=end` (1-indexed within that file's diff
/// section, including headers).  If `None`, the entire file section is included.
pub fn extract_diff_for_files(
    full_diff: &str,
    file_refs: &[(String, Option<(usize, usize)>)],
) -> String {
    let sections = split_diff_by_file(full_diff);
    let mut result = String::new();

    for (path, diff_lines) in file_refs {
        if let Some(section) = sections.iter().find(|s| s.path == *path) {
            match diff_lines {
                Some((start, end)) => {
                    let lines: Vec<&str> = section.content.lines().collect();
                    let start_idx = start.saturating_sub(1);
                    let end_idx = (*end).min(lines.len());
                    if start_idx < end_idx {
                        if !result.is_empty() {
                            result.push('\n');
                        }
                        result.push_str(&lines[start_idx..end_idx].join("\n"));
                        result.push('\n');
                    }
                }
                None => {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(&section.content);
                    if !section.content.ends_with('\n') {
                        result.push('\n');
                    }
                }
            }
        }
    }

    result
}

#[derive(Debug)]
struct DiffSection {
    path: String,
    content: String,
}

fn split_diff_by_file(full_diff: &str) -> Vec<DiffSection> {
    let mut sections = Vec::new();
    let mut current_lines: Vec<&str> = Vec::new();
    let mut current_path: Option<String> = None;

    for line in full_diff.lines() {
        if line.starts_with("diff --git ") {
            if let Some(path) = current_path.take() {
                sections.push(DiffSection {
                    path,
                    content: current_lines.join("\n") + "\n",
                });
            }
            current_lines.clear();
            current_path = parse_diff_git_path(line);
            current_lines.push(line);
        } else {
            current_lines.push(line);
        }
    }

    if let Some(path) = current_path {
        sections.push(DiffSection {
            path,
            content: current_lines.join("\n") + "\n",
        });
    }

    sections
}

fn parse_diff_git_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    let parts: Vec<&str> = rest.splitn(2, " b/").collect();
    if parts.len() == 2 {
        Some(parts[1].to_string())
    } else {
        None
    }
}

pub fn compute_diff_line_stats(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            added += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            removed += 1;
        }
    }
    (added, removed)
}

pub fn analyze_repo(path: &Path) -> Result<GitAnalysis, GitAnalysisError> {
    let canonical = path
        .canonicalize()
        .map_err(|_| GitAnalysisError::NotADirectory(path.to_string_lossy().to_string()))?;
    if !canonical.is_dir() {
        return Err(GitAnalysisError::NotADirectory(
            path.to_string_lossy().to_string(),
        ));
    }

    run_git(&canonical, &["rev-parse", "--is-inside-work-tree"])
        .map_err(|_| GitAnalysisError::NotAGitRepo(canonical.to_string_lossy().to_string()))?;

    let current_branch = run_git(&canonical, &["branch", "--show-current"])?;
    if current_branch.is_empty() {
        return Err(GitAnalysisError::NoBranch);
    }

    let default_branch = detect_default_branch(&canonical)?;

    if current_branch == default_branch {
        return Err(GitAnalysisError::OnDefaultBranch(current_branch));
    }

    let merge_base = run_git(&canonical, &["merge-base", "HEAD", &default_branch])?;

    let diff_range = format!("{merge_base}..HEAD");
    let diff = run_git(&canonical, &["diff", &diff_range])?;

    let name_status = run_git(&canonical, &["diff", "--name-status", &diff_range])?;
    let changed_files: Vec<ChangedFile> = name_status
        .lines()
        .filter_map(parse_name_status_line)
        .collect();

    let commit_count = count_commits(&canonical, &diff_range)?;

    Ok(GitAnalysis {
        repo_path: canonical.to_string_lossy().to_string(),
        current_branch,
        default_branch,
        merge_base,
        diff,
        changed_files,
        commit_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_status_from_status_char() {
        assert_eq!(FileStatus::from_status_char("A"), Some(FileStatus::Added));
        assert_eq!(
            FileStatus::from_status_char("M"),
            Some(FileStatus::Modified)
        );
        assert_eq!(FileStatus::from_status_char("D"), Some(FileStatus::Deleted));
        assert_eq!(
            FileStatus::from_status_char("R100"),
            Some(FileStatus::Renamed)
        );
        assert_eq!(
            FileStatus::from_status_char("R050"),
            Some(FileStatus::Renamed)
        );
        assert_eq!(FileStatus::from_status_char("X"), None);
        assert_eq!(FileStatus::from_status_char(""), None);
    }

    #[test]
    fn test_parse_name_status_line_added() {
        let line = "A\tsrc/new_file.rs";
        let result = parse_name_status_line(line).expect("should parse");
        assert_eq!(result.path, "src/new_file.rs");
        assert_eq!(result.status, FileStatus::Added);
    }

    #[test]
    fn test_parse_name_status_line_modified() {
        let line = "M\tsrc/existing.rs";
        let result = parse_name_status_line(line).expect("should parse");
        assert_eq!(result.path, "src/existing.rs");
        assert_eq!(result.status, FileStatus::Modified);
    }

    #[test]
    fn test_parse_name_status_line_deleted() {
        let line = "D\tsrc/removed.rs";
        let result = parse_name_status_line(line).expect("should parse");
        assert_eq!(result.path, "src/removed.rs");
        assert_eq!(result.status, FileStatus::Deleted);
    }

    #[test]
    fn test_parse_name_status_line_renamed() {
        let line = "R100\tsrc/old_name.rs\tsrc/new_name.rs";
        let result = parse_name_status_line(line).expect("should parse");
        assert_eq!(result.path, "src/new_name.rs");
        assert_eq!(result.status, FileStatus::Renamed);
    }

    #[test]
    fn test_parse_name_status_line_invalid() {
        assert!(parse_name_status_line("").is_none());
        assert!(parse_name_status_line("no-tab-here").is_none());
        assert!(parse_name_status_line("X\tunknown_status.rs").is_none());
    }

    #[test]
    fn test_parse_multiple_name_status_lines() {
        let output = "A\tsrc/new.rs\nM\tsrc/changed.rs\nD\tsrc/gone.rs";
        let files: Vec<ChangedFile> = output.lines().filter_map(parse_name_status_line).collect();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].status, FileStatus::Added);
        assert_eq!(files[1].status, FileStatus::Modified);
        assert_eq!(files[2].status, FileStatus::Deleted);
    }

    #[test]
    fn test_compute_diff_line_stats_basic() {
        let diff = "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1,3 +1,4 @@\n line1\n+added line\n line2\n-removed line\n line3\n";
        let (added, removed) = compute_diff_line_stats(diff);
        assert_eq!(added, 1);
        assert_eq!(removed, 1);
    }

    #[test]
    fn test_compute_diff_line_stats_empty() {
        let (added, removed) = compute_diff_line_stats("");
        assert_eq!(added, 0);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_compute_diff_line_stats_ignores_headers() {
        let diff = "--- a/file.rs\n+++ b/file.rs\n+real addition\n";
        let (added, removed) = compute_diff_line_stats(diff);
        assert_eq!(added, 1);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_error_display() {
        let err = GitAnalysisError::OnDefaultBranch("main".to_string());
        assert_eq!(
            err.to_string(),
            "You are on the default branch (main). Switch to a feature branch."
        );

        let err = GitAnalysisError::NotADirectory("/bad/path".to_string());
        assert!(err.to_string().contains("/bad/path"));

        let err = GitAnalysisError::NoBranch;
        assert!(err.to_string().contains("detached"));
    }

    fn multi_file_diff() -> &'static str {
        "diff --git a/src/lib.rs b/src/lib.rs\n\
         --- a/src/lib.rs\n\
         +++ b/src/lib.rs\n\
         @@ -1,3 +1,4 @@\n\
         +use crate::new;\n\
         pub mod old;\n\
         \n\
         diff --git a/src/new.rs b/src/new.rs\n\
         --- /dev/null\n\
         +++ b/src/new.rs\n\
         @@ -0,0 +1,5 @@\n\
         +pub fn hello() {\n\
         +    println!(\"hello\");\n\
         +}\n"
    }

    #[test]
    fn test_extract_diff_for_single_file() {
        let refs = vec![("src/lib.rs".to_string(), None)];
        let result = extract_diff_for_files(multi_file_diff(), &refs);
        assert!(result.contains("diff --git a/src/lib.rs"));
        assert!(result.contains("+use crate::new;"));
        assert!(!result.contains("src/new.rs"));
    }

    #[test]
    fn test_extract_diff_for_multiple_files() {
        let refs = vec![
            ("src/lib.rs".to_string(), None),
            ("src/new.rs".to_string(), None),
        ];
        let result = extract_diff_for_files(multi_file_diff(), &refs);
        assert!(result.contains("src/lib.rs"));
        assert!(result.contains("src/new.rs"));
    }

    #[test]
    fn test_extract_diff_with_line_range() {
        let refs = vec![("src/lib.rs".to_string(), Some((1, 3)))];
        let result = extract_diff_for_files(multi_file_diff(), &refs);
        assert!(result.contains("diff --git a/src/lib.rs"));
        assert!(result.contains("--- a/src/lib.rs"));
        assert!(!result.contains("+use crate::new;"));
    }

    #[test]
    fn test_extract_diff_for_nonexistent_file() {
        let refs = vec![("src/missing.rs".to_string(), None)];
        let result = extract_diff_for_files(multi_file_diff(), &refs);
        assert!(result.is_empty());
    }

    #[test]
    fn test_extract_diff_empty_input() {
        let refs = vec![("src/lib.rs".to_string(), None)];
        let result = extract_diff_for_files("", &refs);
        assert!(result.is_empty());
    }

    #[test]
    fn test_split_diff_by_file() {
        let sections = split_diff_by_file(multi_file_diff());
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].path, "src/lib.rs");
        assert_eq!(sections[1].path, "src/new.rs");
    }

    #[test]
    fn test_parse_diff_git_path() {
        assert_eq!(
            parse_diff_git_path("diff --git a/src/lib.rs b/src/lib.rs"),
            Some("src/lib.rs".to_string())
        );
        assert_eq!(parse_diff_git_path("not a diff line"), None);
    }
}
