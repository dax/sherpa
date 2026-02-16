use std::process::Command;

use regex::Regex;
use serde::Deserialize;

use super::git_analysis::{ChangedFile, FileStatus, GitAnalysis};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct PrMetadata {
    number: u32,
    title: String,
    state: String,
    base_ref_name: String,
    head_ref_name: String,
    #[serde(default)]
    base_ref_oid: String,
    changed_files: u32,
    additions: u32,
    deletions: u32,
    url: String,
    #[serde(default)]
    commits: PrCommits,
}

/// Wrapper to deserialize the `commits` array from `gh pr view --json commits`.
/// The CLI returns an array of commit objects; we only need the count.
#[derive(Debug, Default)]
struct PrCommits {
    count: usize,
}

impl<'de> Deserialize<'de> for PrCommits {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let items: Vec<serde_json::Value> = Vec::deserialize(deserializer)?;
        Ok(PrCommits { count: items.len() })
    }
}

#[derive(Debug, Deserialize)]
struct PrFile {
    filename: String,
    status: String,
}

#[derive(Debug)]
pub enum GithubPrError {
    InvalidUrl(String),
    GhNotInstalled,
    GhNotAuthenticated,
    PrNotFound(String),
    GhCommandFailed { command: String, stderr: String },
    ParseError(String),
}

impl std::fmt::Display for GithubPrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(url) => write!(
                f,
                "Invalid GitHub PR URL: {url}. Expected format: \
                 https://github.com/owner/repo/pull/123"
            ),
            Self::GhNotInstalled => write!(
                f,
                "GitHub CLI (gh) is not installed. \
                 Install it from https://cli.github.com/"
            ),
            Self::GhNotAuthenticated => write!(
                f,
                "GitHub CLI is not authenticated. Run `gh auth login` first."
            ),
            Self::PrNotFound(url) => {
                write!(f, "Pull request not found or inaccessible: {url}")
            }
            Self::GhCommandFailed { command, stderr } => {
                write!(f, "GitHub CLI command failed: {command}\n{stderr}")
            }
            Self::ParseError(msg) => write!(f, "Failed to parse PR data: {msg}"),
        }
    }
}

impl std::error::Error for GithubPrError {}

#[derive(Debug, Clone)]
pub struct PrUrlInfo {
    pub owner: String,
    pub repo: String,
    pub number: u32,
    pub full_url: String,
}

/// Parse a GitHub PR URL into its components.
///
/// Accepts formats like:
/// - `https://github.com/owner/repo/pull/123`
/// - `github.com/owner/repo/pull/123`
/// - `owner/repo#123`
pub fn parse_pr_url(input: &str) -> Result<PrUrlInfo, GithubPrError> {
    let input = input.trim();

    let url_re =
        Regex::new(r"(?:https?://)?github\.com/([^/]+)/([^/]+)/pull/(\d+)").expect("valid regex");
    if let Some(caps) = url_re.captures(input) {
        return Ok(PrUrlInfo {
            owner: caps[1].to_string(),
            repo: caps[2].to_string(),
            number: caps[3].parse().unwrap(),
            full_url: format!(
                "https://github.com/{}/{}/pull/{}",
                &caps[1], &caps[2], &caps[3]
            ),
        });
    }

    let short_re = Regex::new(r"^([^/]+)/([^#]+)#(\d+)$").expect("valid regex");
    if let Some(caps) = short_re.captures(input) {
        return Ok(PrUrlInfo {
            owner: caps[1].to_string(),
            repo: caps[2].to_string(),
            number: caps[3].parse().unwrap(),
            full_url: format!(
                "https://github.com/{}/{}/pull/{}",
                &caps[1], &caps[2], &caps[3]
            ),
        });
    }

    Err(GithubPrError::InvalidUrl(input.to_string()))
}

/// Fetch PR metadata and diff from GitHub using `gh` CLI and produce a
/// [`GitAnalysis`] that can be fed into `ReviewSession::new()`.
pub fn analyze_pr(input: &str) -> Result<GitAnalysis, GithubPrError> {
    let pr_info = parse_pr_url(input)?;

    if Command::new("gh").arg("--version").output().is_err() {
        return Err(GithubPrError::GhNotInstalled);
    }

    let metadata = fetch_pr_metadata(&pr_info)?;
    let diff = fetch_pr_diff(&pr_info)?;
    let changed_files = fetch_changed_files(&pr_info)?;

    let repo_path = format!(
        "github:{}/{}#{}",
        pr_info.owner, pr_info.repo, pr_info.number
    );

    Ok(GitAnalysis {
        repo_path,
        current_branch: format!(
            "PR #{} — {}",
            metadata.number,
            truncate_title(&metadata.title, 60)
        ),
        default_branch: metadata.base_ref_name,
        merge_base: metadata.base_ref_oid,
        diff,
        changed_files,
        commit_count: metadata.commits.count,
    })
}

fn truncate_title(title: &str, max: usize) -> &str {
    if title.len() <= max {
        title
    } else {
        let end = title
            .char_indices()
            .take_while(|(i, _)| *i < max)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(max);
        &title[..end]
    }
}

fn run_gh(args: &[&str]) -> Result<String, GithubPrError> {
    let output =
        Command::new("gh")
            .args(args)
            .output()
            .map_err(|e| GithubPrError::GhCommandFailed {
                command: format!("gh {}", args.join(" ")),
                stderr: e.to_string(),
            })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if stderr.contains("gh auth login") || output.status.code() == Some(4) {
            return Err(GithubPrError::GhNotAuthenticated);
        }
        if stderr.contains("Could not resolve to a PullRequest")
            || stderr.contains("Could not resolve to a Repository")
        {
            return Err(GithubPrError::PrNotFound(
                args.iter()
                    .find(|a| a.contains("github.com") || a.contains('/'))
                    .unwrap_or(&"unknown")
                    .to_string(),
            ));
        }

        return Err(GithubPrError::GhCommandFailed {
            command: format!("gh {}", args.join(" ")),
            stderr,
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn fetch_pr_metadata(pr: &PrUrlInfo) -> Result<PrMetadata, GithubPrError> {
    let json_fields = "number,title,state,baseRefName,headRefName,baseRefOid,\
                       changedFiles,additions,deletions,url,commits";

    let output = run_gh(&["pr", "view", &pr.full_url, "--json", json_fields])?;

    serde_json::from_str::<PrMetadata>(&output)
        .map_err(|e| GithubPrError::ParseError(format!("PR metadata JSON: {e}")))
}

fn fetch_pr_diff(pr: &PrUrlInfo) -> Result<String, GithubPrError> {
    run_gh(&["pr", "diff", &pr.full_url, "--color", "never"])
}

fn fetch_changed_files(pr: &PrUrlInfo) -> Result<Vec<ChangedFile>, GithubPrError> {
    let endpoint = format!("repos/{}/{}/pulls/{}/files", pr.owner, pr.repo, pr.number);
    let output = run_gh(&["api", &endpoint, "--paginate"])?;

    // gh api --paginate can return multiple JSON arrays concatenated.
    // Parse them all and flatten.
    let files: Vec<PrFile> = if output.trim().starts_with('[') {
        // Might be multiple arrays concatenated
        let mut all_files = Vec::new();
        for chunk in output.split("][") {
            let normalized = if !chunk.starts_with('[') {
                format!("[{chunk}")
            } else {
                chunk.to_string()
            };
            let normalized = if !normalized.ends_with(']') {
                format!("{normalized}]")
            } else {
                normalized
            };
            if let Ok(files) = serde_json::from_str::<Vec<PrFile>>(&normalized) {
                all_files.extend(files);
            }
        }
        all_files
    } else {
        serde_json::from_str(&output)
            .map_err(|e| GithubPrError::ParseError(format!("files JSON: {e}")))?
    };

    Ok(files
        .into_iter()
        .filter_map(|f| {
            let status = match f.status.as_str() {
                "added" => FileStatus::Added,
                "modified" | "changed" => FileStatus::Modified,
                "removed" => FileStatus::Deleted,
                "renamed" | "copied" => FileStatus::Renamed,
                _ => return None,
            };
            Some(ChangedFile {
                path: f.filename,
                status,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pr_url_full_https() {
        let info = parse_pr_url("https://github.com/loco-rs/loco/pull/999").unwrap();
        assert_eq!(info.owner, "loco-rs");
        assert_eq!(info.repo, "loco");
        assert_eq!(info.number, 999);
        assert_eq!(info.full_url, "https://github.com/loco-rs/loco/pull/999");
    }

    #[test]
    fn test_parse_pr_url_without_https() {
        let info = parse_pr_url("github.com/owner/repo/pull/42").unwrap();
        assert_eq!(info.owner, "owner");
        assert_eq!(info.repo, "repo");
        assert_eq!(info.number, 42);
    }

    #[test]
    fn test_parse_pr_url_shorthand() {
        let info = parse_pr_url("owner/repo#123").unwrap();
        assert_eq!(info.owner, "owner");
        assert_eq!(info.repo, "repo");
        assert_eq!(info.number, 123);
        assert_eq!(info.full_url, "https://github.com/owner/repo/pull/123");
    }

    #[test]
    fn test_parse_pr_url_with_whitespace() {
        let info = parse_pr_url("  https://github.com/a/b/pull/1  ").unwrap();
        assert_eq!(info.owner, "a");
        assert_eq!(info.number, 1);
    }

    #[test]
    fn test_parse_pr_url_invalid() {
        assert!(parse_pr_url("not-a-url").is_err());
        assert!(parse_pr_url("https://gitlab.com/a/b/merge_requests/1").is_err());
        assert!(parse_pr_url("").is_err());
    }

    #[test]
    fn test_truncate_title_short() {
        assert_eq!(truncate_title("short", 60), "short");
    }

    #[test]
    fn test_truncate_title_exact() {
        let s = "a".repeat(60);
        assert_eq!(truncate_title(&s, 60).len(), 60);
    }

    #[test]
    fn test_truncate_title_long() {
        let s = "a".repeat(100);
        assert_eq!(truncate_title(&s, 60).len(), 60);
    }

    #[test]
    fn test_error_display() {
        let err = GithubPrError::GhNotInstalled;
        assert!(err.to_string().contains("not installed"));

        let err = GithubPrError::InvalidUrl("bad".into());
        assert!(err.to_string().contains("bad"));
    }
}
