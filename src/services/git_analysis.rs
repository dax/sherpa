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

    Ok(GitAnalysis {
        repo_path: canonical.to_string_lossy().to_string(),
        current_branch,
        default_branch,
        merge_base,
        diff,
        changed_files,
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
}
