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
    pub has_uncommitted_changes: bool,
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

/// Parsed hunk from a unified diff section.
struct ParsedHunk {
    /// 1-indexed start line in the new file (+ side)
    new_start: usize,
    /// Number of lines in the new file for this hunk
    new_count: usize,
    /// The raw lines of this hunk (starting with the @@ header, ending before the next @@ or EOF)
    lines: Vec<String>,
}

/// Parse a single file's diff section into its header and individual hunks.
///
/// Returns `(header, hunks)` where `header` is everything before the first `@@`
/// line (the `diff --git`, `index`, `---`, `+++` lines) and `hunks` contains
/// each parsed hunk with its new-file range metadata.
fn parse_hunks_from_section(section: &str) -> (String, Vec<ParsedHunk>) {
    let mut header_lines: Vec<&str> = Vec::new();
    let mut hunks: Vec<ParsedHunk> = Vec::new();
    let mut current_hunk_lines: Vec<String> = Vec::new();
    let mut current_new_start: usize = 0;
    let mut current_new_count: usize = 0;
    let mut in_hunk = false;

    for line in section.lines() {
        if line.starts_with("@@ ") {
            // Flush previous hunk if any
            if in_hunk {
                hunks.push(ParsedHunk {
                    new_start: current_new_start,
                    new_count: current_new_count,
                    lines: current_hunk_lines.clone(),
                });
                current_hunk_lines.clear();
            }

            // Parse @@ -old_start[,old_count] +new_start[,new_count] @@
            let (new_start, new_count) = parse_hunk_header(line);
            current_new_start = new_start;
            current_new_count = new_count;
            current_hunk_lines.push(line.to_string());
            in_hunk = true;
        } else if in_hunk {
            current_hunk_lines.push(line.to_string());
        } else {
            header_lines.push(line);
        }
    }

    // Flush last hunk
    if in_hunk {
        hunks.push(ParsedHunk {
            new_start: current_new_start,
            new_count: current_new_count,
            lines: current_hunk_lines,
        });
    }

    let header = if header_lines.is_empty() {
        String::new()
    } else {
        header_lines.join("\n") + "\n"
    };

    (header, hunks)
}

/// Parse a `@@ ... @@` hunk header and return `(new_start, new_count)`.
///
/// Handles formats like:
/// - `@@ -10,5 +10,8 @@` → (10, 8)
/// - `@@ -1 +1 @@` → (1, 1)  (count omitted means 1)
/// - `@@ -0,0 +1,5 @@` → (1, 5)
fn parse_hunk_header(line: &str) -> (usize, usize) {
    let default = (0, 0);

    let rest = match line.strip_prefix("@@ -") {
        Some(r) => r,
        None => return default,
    };

    let rest2 = match rest.split_once(" +") {
        Some((_, r)) => r,
        None => return default,
    };

    let new_part = match rest2.split_once(" @@") {
        Some((np, _)) => np,
        None => return default,
    };

    if let Some((start_str, count_str)) = new_part.split_once(',') {
        let start = start_str.parse::<usize>().unwrap_or(0);
        let count = count_str.parse::<usize>().unwrap_or(0);
        (start, count)
    } else {
        let start = new_part.parse::<usize>().unwrap_or(0);
        (start, 1)
    }
}

/// Extract the function context from a unified diff string.
///
/// Looks at hunk headers (the text after `@@`) and hunk content to find the
/// enclosing scope and function name.  Returns a breadcrumb like
/// `impl TaskService > fn create_task` when the `@@` header shows an `impl`
/// block and a `fn` definition is found in the hunk body.
///
/// When `source_info` is provided as `(repo_path, file_path)`, falls back to
/// reading the source file and walking backward from the hunk start line to
/// find the enclosing `fn` when the diff body has no fn definition.
///
/// Returns `None` when no meaningful context can be determined.
pub fn extract_function_context(diff: &str, source_info: Option<(&Path, &str)>) -> Option<String> {
    let mut contexts: Vec<String> = Vec::new();
    let mut first_hunk_start: Option<usize> = None;

    for line in diff.lines() {
        if !line.starts_with("@@ ") {
            continue;
        }
        if first_hunk_start.is_none() {
            let (new_start, _) = parse_hunk_header(line);
            if new_start > 0 {
                first_hunk_start = Some(new_start);
            }
        }
        let header_ctx = line
            .splitn(3, "@@")
            .nth(2)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if let Some(ctx) = header_ctx {
            if !contexts.contains(&ctx) {
                contexts.push(ctx);
            }
        }
    }

    let mut fn_sigs: Vec<String> = Vec::new();
    for line in diff.lines() {
        let content = if let Some(rest) = line.strip_prefix(' ') {
            rest
        } else if let Some(rest) = line.strip_prefix('+') {
            rest
        } else {
            continue;
        };
        if let Some(sig) = extract_fn_signature(content.trim()) {
            if !fn_sigs.contains(&sig) {
                fn_sigs.push(sig);
            }
        }
    }

    if fn_sigs.is_empty() {
        if let (Some((repo_path, file_path)), Some(hunk_start)) = (source_info, first_hunk_start) {
            if let Some(sig) = find_enclosing_fn_from_source(repo_path, file_path, hunk_start) {
                fn_sigs.push(sig);
            }
        }
    }

    if contexts.is_empty() && fn_sigs.is_empty() {
        return None;
    }

    let scope_ctx: Option<&str> = contexts.iter().find_map(|c| {
        let trimmed = c.trim_start_matches("pub ");
        let trimmed = trimmed.trim_start_matches("pub(crate) ");
        if trimmed.starts_with("impl ")
            || trimmed.starts_with("trait ")
            || trimmed.starts_with("mod ")
        {
            Some(c.as_str())
        } else {
            None
        }
    });

    let fn_in_header = contexts.iter().any(|c| {
        let trimmed = c.trim_start_matches("pub ");
        let trimmed = trimmed.trim_start_matches("pub(crate) ");
        let trimmed = trimmed.trim_start_matches("async ");
        let trimmed = trimmed.trim_start_matches("unsafe ");
        let trimmed = trimmed.trim_start_matches("const ");
        trimmed.starts_with("fn ")
    });
    if fn_in_header {
        return None;
    }

    match (scope_ctx, fn_sigs.is_empty()) {
        (Some(scope), false) => {
            let scope_clean = scope.trim_end_matches(" {").trim_end_matches('{');
            let fns = fn_sigs.join(", ");
            Some(format!("{} > {}", scope_clean.trim(), fns))
        }
        (None, false) => Some(fn_sigs.join(", ")),
        _ => None,
    }
}

/// Read the source file at `repo_path/file_path` and walk backward from
/// `line_number` (1-indexed) to find the nearest enclosing `fn` definition.
fn find_enclosing_fn_from_source(
    repo_path: &Path,
    file_path: &str,
    line_number: usize,
) -> Option<String> {
    let full_path = repo_path.join(file_path);
    let content = std::fs::read_to_string(&full_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();

    let start = line_number.saturating_sub(1).min(lines.len());
    for i in (0..start).rev() {
        if let Some(sig) = extract_fn_signature(lines[i].trim()) {
            return Some(sig);
        }
    }
    None
}

/// Find the start and end line numbers of the function enclosing `line_number`.
///
/// Reads the source file at `repo_path/file_path`, walks backward from
/// `line_number` (1-indexed) to find the `fn` definition, then walks forward
/// counting braces (skipping comments and string literals) to find the closing
/// `}`.  Returns `(fn_start, fn_end)` both 1-indexed inclusive, or `None` if
/// no enclosing function can be determined.
pub fn find_function_boundaries(
    repo_path: &Path,
    file_path: &str,
    line_number: usize,
) -> Option<(usize, usize)> {
    if line_number == 0 {
        return None;
    }

    let full_path = repo_path.join(file_path);
    let content = std::fs::read_to_string(&full_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();

    if line_number > lines.len() {
        return None;
    }

    let start_idx = line_number - 1;
    let mut fn_line_idx: Option<usize> = None;
    for i in (0..=start_idx).rev() {
        if extract_fn_signature(lines[i].trim()).is_some() {
            fn_line_idx = Some(i);
            break;
        }
    }
    let fn_line_idx = fn_line_idx?;

    let mut brace_count: i32 = 0;
    let mut found_open = false;
    let mut in_block_comment = false;
    let mut end_line_idx: Option<usize> = None;

    for (i, line) in lines.iter().enumerate().skip(fn_line_idx) {
        let mut chars = line.chars().peekable();
        let mut in_string = false;

        while let Some(ch) = chars.next() {
            if in_block_comment {
                if ch == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    in_block_comment = false;
                }
                continue;
            }

            if in_string {
                if ch == '\\' {
                    chars.next();
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }

            if ch == '/' {
                if chars.peek() == Some(&'/') {
                    break; // line comment — skip rest of line
                }
                if chars.peek() == Some(&'*') {
                    chars.next();
                    in_block_comment = true;
                    continue;
                }
            }

            if ch == '"' {
                in_string = true;
                continue;
            }

            if ch == '{' {
                brace_count += 1;
                found_open = true;
            } else if ch == '}' {
                brace_count -= 1;
            }

            if found_open && brace_count == 0 {
                end_line_idx = Some(i);
                break;
            }
        }

        if end_line_idx.is_some() {
            break;
        }
    }

    let end_line_idx = end_line_idx?;

    Some((fn_line_idx + 1, end_line_idx + 1))
}

/// Takes the original diff for a file and returns an expanded diff showing
/// the full enclosing function body around the changed hunks.
pub fn generate_expanded_diff(
    repo_path: &Path,
    file_path: &str,
    original_diff: &str,
    merge_base: &str,
) -> Option<String> {
    let (_header, hunks) = parse_hunks_from_section(original_diff);
    if hunks.is_empty() {
        return None;
    }

    let first_hunk = &hunks[0];
    let (fn_start, fn_end) = find_function_boundaries(repo_path, file_path, first_hunk.new_start)?;

    let last_hunk = &hunks[hunks.len() - 1];
    let last_hunk_end = last_hunk.new_start + last_hunk.new_count.saturating_sub(1);

    let context_before = first_hunk.new_start.saturating_sub(fn_start);
    let context_after = fn_end.saturating_sub(last_hunk_end);
    let context = context_before.max(context_after) + 3;

    let context_arg = format!("-U{context}");
    let expanded_full = run_git(
        repo_path,
        &["diff", &context_arg, merge_base, "--", file_path],
    )
    .ok()?;

    if expanded_full.is_empty() {
        return None;
    }

    let file_refs = vec![(file_path.to_string(), Some((fn_start, fn_end)))];
    let filtered = extract_diff_for_files(&expanded_full, &file_refs);

    if filtered.is_empty() {
        None
    } else {
        Some(filtered)
    }
}

/// Extract a cleaned-up function signature from a source line.
///
/// Matches lines like `fn foo(`, `pub fn bar(`, `pub async fn baz(`,
/// `pub(crate) const fn qux(`, etc.  Returns just `fn name` without
/// parameters or body.
fn extract_fn_signature(line: &str) -> Option<String> {
    let mut rest = line;
    loop {
        if let Some(r) = rest.strip_prefix("pub(crate) ") {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("pub ") {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("async ") {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("unsafe ") {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("const ") {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("extern \"C\" ") {
            rest = r;
        } else {
            break;
        }
    }
    if !rest.starts_with("fn ") {
        return None;
    }
    let after_fn = &rest[3..];
    let name_end = after_fn.find(['(', '<', ' ']).unwrap_or(after_fn.len());
    let name = &after_fn[..name_end];
    if name.is_empty() {
        return None;
    }
    Some(format!("fn {name}"))
}

/// Extract the unified diff section(s) for the given file paths from a full
/// multi-file unified diff string.  Returns a new unified diff containing only
/// the matching file sections.
///
/// If `diff_lines` is `Some((start, end))`, the returned diff includes only
/// hunks that overlap with new-file source lines `start..=end` (1-indexed).
/// Whole hunks are always included — never trimmed mid-hunk.
/// Falls back to the full section if no hunks overlap.
/// If `None`, the entire file section is included.
pub fn extract_diff_for_files(
    full_diff: &str,
    file_refs: &[(String, Option<(usize, usize)>)],
) -> String {
    let sections = split_diff_by_file(full_diff);
    let mut result = String::new();

    for (path, diff_lines) in file_refs {
        if let Some(section) = sections.iter().find(|s| s.path == *path) {
            let filtered = match diff_lines {
                Some((start, end)) => {
                    let (header, hunks) = parse_hunks_from_section(&section.content);

                    let matching: Vec<&ParsedHunk> = hunks
                        .iter()
                        .filter(|h| {
                            if h.new_count == 0 {
                                return false;
                            }
                            let hunk_end = h.new_start + h.new_count.max(1) - 1;
                            h.new_start <= *end && hunk_end >= *start
                        })
                        .collect();

                    if matching.is_empty() {
                        tracing::warn!(
                            "diff_lines [{start}, {end}] matched no \
                             hunks for {path} — using full section"
                        );
                        section.content.clone()
                    } else {
                        let mut out = header;
                        for hunk in matching {
                            for line in &hunk.lines {
                                out.push_str(line);
                                out.push('\n');
                            }
                        }
                        out
                    }
                }
                None => section.content.clone(),
            };

            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&filtered);
            if !filtered.ends_with('\n') {
                result.push('\n');
            }
        }
    }

    result
}

#[derive(Debug)]
pub struct DiffSection {
    pub path: String,
    pub content: String,
}

pub fn split_diff_by_file(full_diff: &str) -> Vec<DiffSection> {
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

fn get_untracked_files(path: &Path) -> Result<Vec<String>, GitAnalysisError> {
    let output = run_git(path, &["ls-files", "--others", "--exclude-standard"])?;
    Ok(output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect())
}

fn generate_untracked_diff(repo_path: &Path, file_path: &str) -> Option<String> {
    let full_path = repo_path.join(file_path);
    let content = match std::fs::read_to_string(&full_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Skipping untracked file {file_path}: {e}");
            return None;
        }
    };
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();
    let mut diff = String::new();
    diff.push_str(&format!("diff --git a/{file_path} b/{file_path}\n"));
    diff.push_str("new file mode 100644\n");
    diff.push_str("--- /dev/null\n");
    diff.push_str(&format!("+++ b/{file_path}\n"));
    if line_count > 0 {
        diff.push_str(&format!("@@ -0,0 +1,{line_count} @@\n"));
        for line in &lines {
            diff.push('+');
            diff.push_str(line);
            diff.push('\n');
        }
    }
    Some(diff)
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

    // Diff working tree against merge_base to include uncommitted changes
    let diff = run_git(&canonical, &["diff", &merge_base])?;

    let name_status = run_git(&canonical, &["diff", "--name-status", &merge_base])?;
    let changed_files: Vec<ChangedFile> = name_status
        .lines()
        .filter_map(parse_name_status_line)
        .collect();

    // commit_count uses committed-only range (meaningful metric)
    let commit_range = format!("{merge_base}..HEAD");
    let commit_count = count_commits(&canonical, &commit_range)?;

    let has_uncommitted = !run_git(&canonical, &["status", "--porcelain"])
        .unwrap_or_default()
        .is_empty();

    // Append synthetic diffs for untracked (non-ignored) files
    let untracked = get_untracked_files(&canonical).unwrap_or_default();
    let mut full_diff = diff;
    let mut all_changed_files = changed_files;

    for file_path in &untracked {
        if let Some(file_diff) = generate_untracked_diff(&canonical, file_path) {
            if !full_diff.is_empty() && !full_diff.ends_with('\n') {
                full_diff.push('\n');
            }
            full_diff.push_str(&file_diff);
            all_changed_files.push(ChangedFile {
                path: file_path.clone(),
                status: FileStatus::Added,
            });
        }
    }

    Ok(GitAnalysis {
        repo_path: canonical.to_string_lossy().to_string(),
        current_branch,
        default_branch,
        merge_base,
        diff: full_diff,
        changed_files: all_changed_files,
        commit_count,
        has_uncommitted_changes: has_uncommitted,
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
        // Source lines 1-3 overlap with hunk @@ -1,3 +1,4 @@ (new lines 1-4)
        // so the whole hunk is included
        let refs = vec![("src/lib.rs".to_string(), Some((1, 3)))];
        let result = extract_diff_for_files(multi_file_diff(), &refs);
        assert!(result.contains("diff --git a/src/lib.rs"));
        assert!(result.contains("--- a/src/lib.rs"));
        assert!(result.contains("+use crate::new;"));
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

    #[test]
    fn test_extract_diff_with_out_of_range_lines_falls_back_to_full_section() {
        let refs = vec![("src/lib.rs".to_string(), Some((100, 200)))];
        let result = extract_diff_for_files(multi_file_diff(), &refs);
        assert!(
            result.contains("diff --git a/src/lib.rs"),
            "should fall back to full section when diff_lines are out of range"
        );
        assert!(result.contains("+use crate::new;"));
    }

    #[test]
    fn test_extract_diff_with_overlapping_source_lines() {
        // Source lines 3-5 overlap with hunk @@ -1,3 +1,4 @@ (new lines 1-4)
        // so the whole hunk is included via hunk-based filtering
        let refs = vec![("src/lib.rs".to_string(), Some((3, 5)))];
        let result = extract_diff_for_files(multi_file_diff(), &refs);
        assert!(result.contains("diff --git a/src/lib.rs"));
        assert!(result.contains("+use crate::new;"));
    }

    #[test]
    fn test_extract_diff_with_partially_out_of_range_start() {
        let lines_count = multi_file_diff()
            .lines()
            .take_while(|l| !l.starts_with("diff --git a/src/new.rs"))
            .count();
        let refs = vec![(
            "src/lib.rs".to_string(),
            Some((lines_count + 1, lines_count + 50)),
        )];
        let result = extract_diff_for_files(multi_file_diff(), &refs);
        assert!(
            !result.is_empty(),
            "should fall back to full section when start exceeds section length"
        );
    }

    #[test]
    fn test_split_diff_by_file_is_public() {
        let sections = split_diff_by_file(multi_file_diff());
        assert_eq!(sections.len(), 2);
        assert!(!sections[0].content.is_empty());
        assert!(!sections[1].content.is_empty());
    }

    #[test]
    fn test_extract_function_context_impl_with_fn() {
        let diff = "\
diff --git a/src/services/task.rs b/src/services/task.rs
--- a/src/services/task.rs
+++ b/src/services/task.rs
@@ -826,6 +826,49 @@ impl TaskService {
+    pub async fn create_task(&self, input: CreateInput) -> Result<Task> {
+        let task = Task::new(input);
+        self.repo.save(&task).await
+    }
";
        let ctx = extract_function_context(diff, None);
        assert_eq!(ctx, Some("impl TaskService > fn create_task".to_string()));
    }

    #[test]
    fn test_extract_function_context_fn_in_header() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,3 +10,4 @@ fn existing_function() {
     let x = 1;
+    let y = 2;
     x + 1
";
        let ctx = extract_function_context(diff, None);
        assert_eq!(ctx, None);
    }

    #[test]
    fn test_extract_function_context_no_context() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 use std::io;
+use std::fs;
 use std::path;
";
        let ctx = extract_function_context(diff, None);
        assert_eq!(ctx, None);
    }

    #[test]
    fn test_extract_function_context_trait_with_fn() {
        let diff = "\
@@ -100,6 +100,10 @@ trait MyTrait {
+    fn required_method(&self) -> bool;
+
+    fn optional_method(&self) {}
";
        let ctx = extract_function_context(diff, None);
        assert_eq!(
            ctx,
            Some("trait MyTrait > fn required_method, fn optional_method".to_string())
        );
    }

    #[test]
    fn test_extract_function_context_multiple_hunks() {
        let diff = "\
@@ -10,3 +10,4 @@ impl Foo {
+    pub fn bar(&self) {}
@@ -50,3 +51,4 @@ impl Foo {
+    fn baz(&self) {}
";
        let ctx = extract_function_context(diff, None);
        assert_eq!(ctx, Some("impl Foo > fn bar, fn baz".to_string()));
    }

    #[test]
    fn test_extract_function_context_mod_scope() {
        let diff = "\
@@ -5,6 +5,10 @@ mod helpers {
+    pub fn cleanup() {}
";
        let ctx = extract_function_context(diff, None);
        assert_eq!(ctx, Some("mod helpers > fn cleanup".to_string()));
    }

    #[test]
    fn test_extract_fn_signature_variants() {
        assert_eq!(extract_fn_signature("fn foo("), Some("fn foo".to_string()));
        assert_eq!(
            extract_fn_signature("pub fn bar()"),
            Some("fn bar".to_string())
        );
        assert_eq!(
            extract_fn_signature("pub async fn baz(x: i32)"),
            Some("fn baz".to_string())
        );
        assert_eq!(
            extract_fn_signature("pub(crate) const fn qux()"),
            Some("fn qux".to_string())
        );
        assert_eq!(extract_fn_signature("let x = 1;"), None);
        assert_eq!(extract_fn_signature("struct Foo {"), None);
        assert_eq!(
            extract_fn_signature("unsafe fn danger()"),
            Some("fn danger".to_string())
        );
    }

    #[test]
    fn test_extract_function_context_source_file_fallback() {
        let dir = std::env::temp_dir().join("sherpa-test-fn-ctx");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("service.rs"),
            "\
use std::io;

impl TaskService {
    pub async fn sync_third_party_item(
        &self,
        executor: &mut Transaction<'_, Postgres>,
    ) -> Result<()> {
        let item = self.fetch_item().await?;
        // line 9: some code
        // line 10: more code
        // line 11: even more
    }
}
",
        )
        .unwrap();

        let diff = "\
diff --git a/service.rs b/service.rs
--- a/service.rs
+++ b/service.rs
@@ -9,3 +9,6 @@ impl TaskService {
         // line 9: some code
+        if item.is_valid() {
+            self.process(&item).await?;
+        }
         // line 10: more code
";
        let ctx = extract_function_context(diff, Some((&dir, "service.rs")));
        assert_eq!(
            ctx,
            Some("impl TaskService > fn sync_third_party_item".to_string())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn make_temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sherpa-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_generate_untracked_diff_basic() {
        let dir = make_temp_dir("basic");
        std::fs::write(dir.join("new.rs"), "fn main() {}\n").unwrap();
        let diff = generate_untracked_diff(&dir, "new.rs").unwrap();
        assert!(diff.starts_with("diff --git a/new.rs b/new.rs"));
        assert!(diff.contains("new file mode 100644"));
        assert!(diff.contains("--- /dev/null"));
        assert!(diff.contains("+++ b/new.rs"));
        assert!(diff.contains("@@ -0,0 +1,1 @@"));
        assert!(diff.contains("+fn main() {}"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_generate_untracked_diff_multiline() {
        let dir = make_temp_dir("multiline");
        std::fs::write(dir.join("multi.rs"), "line1\nline2\nline3\n").unwrap();
        let diff = generate_untracked_diff(&dir, "multi.rs").unwrap();
        assert!(diff.contains("@@ -0,0 +1,3 @@"));
        assert!(diff.contains("+line1\n+line2\n+line3\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_generate_untracked_diff_empty_file() {
        let dir = make_temp_dir("empty");
        std::fs::write(dir.join("empty.rs"), "").unwrap();
        let diff = generate_untracked_diff(&dir, "empty.rs").unwrap();
        assert!(diff.contains("new file mode 100644"));
        assert!(!diff.contains("@@"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_generate_untracked_diff_binary_returns_none() {
        let dir = make_temp_dir("binary");
        std::fs::write(dir.join("binary.bin"), &[0u8, 159, 146, 150]).unwrap();
        assert!(generate_untracked_diff(&dir, "binary.bin").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn multi_hunk_diff() -> &'static str {
        "diff --git a/src/app.rs b/src/app.rs\n\
         index abc1234..def5678 100644\n\
         --- a/src/app.rs\n\
         +++ b/src/app.rs\n\
         @@ -8,6 +8,8 @@\n\
         context line 8\n\
         context line 9\n\
         +added line 10\n\
         +added line 11\n\
         context line 12\n\
         context line 13\n\
         @@ -48,6 +50,8 @@\n\
         context line 50\n\
         context line 51\n\
         +added line 52\n\
         +added line 53\n\
         context line 54\n\
         context line 55\n"
    }

    #[test]
    fn test_extract_hunks_with_range_filter() {
        // Range 50-60 overlaps only hunk2 (@@ -48,6 +50,8 @@, new lines 50-57)
        let refs = vec![("src/app.rs".to_string(), Some((50, 60)))];
        let result = extract_diff_for_files(multi_hunk_diff(), &refs);
        assert!(result.contains("diff --git a/src/app.rs"));
        assert!(result.contains("@@ -48,6 +50,8 @@"));
        assert!(result.contains("+added line 52"));
        assert!(!result.contains("@@ -8,6 +8,8 @@"));
        assert!(!result.contains("+added line 10"));
    }

    #[test]
    fn test_extract_hunks_overlapping_range() {
        // Range 10-55 overlaps both hunks
        let refs = vec![("src/app.rs".to_string(), Some((10, 55)))];
        let result = extract_diff_for_files(multi_hunk_diff(), &refs);
        assert!(result.contains("@@ -8,6 +8,8 @@"));
        assert!(result.contains("@@ -48,6 +50,8 @@"));
        assert!(result.contains("+added line 10"));
        assert!(result.contains("+added line 52"));
    }

    #[test]
    fn test_extract_hunks_no_match_falls_back() {
        // Range 200-300 matches no hunks — should fall back to full section
        let refs = vec![("src/app.rs".to_string(), Some((200, 300)))];
        let result = extract_diff_for_files(multi_hunk_diff(), &refs);
        assert!(result.contains("@@ -8,6 +8,8 @@"));
        assert!(result.contains("@@ -48,6 +50,8 @@"));
    }

    #[test]
    fn test_extract_hunks_none_returns_full() {
        let refs = vec![("src/app.rs".to_string(), None)];
        let result = extract_diff_for_files(multi_hunk_diff(), &refs);
        assert!(result.contains("@@ -8,6 +8,8 @@"));
        assert!(result.contains("@@ -48,6 +50,8 @@"));
        assert!(result.contains("+added line 10"));
        assert!(result.contains("+added line 52"));
    }

    #[test]
    fn test_parse_hunk_header_edge_cases() {
        // No count (means 1)
        assert_eq!(parse_hunk_header("@@ -1 +1 @@"), (1, 1));

        // New file
        assert_eq!(parse_hunk_header("@@ -0,0 +1,5 @@"), (1, 5));

        // Normal
        assert_eq!(parse_hunk_header("@@ -10,5 +10,8 @@"), (10, 8));

        // With trailing context after @@
        assert_eq!(parse_hunk_header("@@ -10,5 +10,8 @@ fn main()"), (10, 8));

        // Invalid
        assert_eq!(parse_hunk_header("not a hunk header"), (0, 0));
    }

    #[test]
    fn test_find_function_boundaries_simple() {
        let dir = make_temp_dir("fn-bounds-simple");
        std::fs::write(
            dir.join("simple.rs"),
            "fn foo() {\n    let x = 1;\n    x + 1\n}\n",
        )
        .unwrap();
        let result = find_function_boundaries(&dir, "simple.rs", 2);
        assert_eq!(result, Some((1, 4)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_function_boundaries_nested_braces() {
        let dir = make_temp_dir("fn-bounds-nested");
        std::fs::write(
            dir.join("nested.rs"),
            "\
fn complex() {
    if true {
        match x {
            1 => {}
            _ => {
                let c = || { 42 };
            }
        }
    }
}
",
        )
        .unwrap();
        let result = find_function_boundaries(&dir, "nested.rs", 5);
        assert_eq!(result, Some((1, 10)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_function_boundaries_comments() {
        let dir = make_temp_dir("fn-bounds-comments");
        std::fs::write(
            dir.join("comments.rs"),
            "\
fn with_comments() {
    // { this brace in comment should be ignored
    let x = 1;
    /* { block comment with brace } */
    x + 1
}
",
        )
        .unwrap();
        let result = find_function_boundaries(&dir, "comments.rs", 3);
        assert_eq!(result, Some((1, 6)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_function_boundaries_string_literals() {
        let dir = make_temp_dir("fn-bounds-strings");
        std::fs::write(
            dir.join("strings.rs"),
            "\
fn with_strings() {
    let a = \"{ not a brace }\";
    let b = \"escaped \\\" quote { still string }\";
    println!(\"{{}}\");
}
",
        )
        .unwrap();
        let result = find_function_boundaries(&dir, "strings.rs", 2);
        assert_eq!(result, Some((1, 5)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_function_boundaries_line_before_any_fn() {
        let dir = make_temp_dir("fn-bounds-before");
        std::fs::write(
            dir.join("before.rs"),
            "use std::io;\n\nfn foo() {\n    1\n}\n",
        )
        .unwrap();
        let result = find_function_boundaries(&dir, "before.rs", 1);
        assert_eq!(result, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_function_boundaries_line_beyond_file() {
        let dir = make_temp_dir("fn-bounds-beyond");
        std::fs::write(dir.join("short.rs"), "fn foo() {}\n").unwrap();
        assert_eq!(find_function_boundaries(&dir, "short.rs", 0), None);
        assert_eq!(find_function_boundaries(&dir, "short.rs", 100), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_function_boundaries_pub_async_multiline_sig() {
        let dir = make_temp_dir("fn-bounds-async");
        std::fs::write(
            dir.join("async_fn.rs"),
            "\
use std::io;

pub async fn long_signature(
    param1: String,
    param2: i32,
) -> Result<()> {
    let x = param1;
    let y = param2;
    Ok(())
}
",
        )
        .unwrap();
        let result = find_function_boundaries(&dir, "async_fn.rs", 7);
        assert_eq!(result, Some((3, 10)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn make_git_repo(name: &str) -> std::path::PathBuf {
        let dir = make_temp_dir(name);
        run_git(&dir, &["init"]).unwrap();
        run_git(&dir, &["config", "user.email", "test@test.com"]).unwrap();
        run_git(&dir, &["config", "user.name", "Test"]).unwrap();
        dir
    }

    #[test]
    fn test_generate_expanded_diff_includes_function_context() {
        let dir = make_git_repo("expand-diff");

        let original = "\
use std::io;

fn helper() {
    println!(\"helper\");
}

fn target_function() {
    let line1 = 1;
    let line2 = 2;
    let line3 = 3;
    let line4 = 4;
    let line5 = 5;
    let line6 = 6;
    let line7 = 7;
    let line8 = 8;
    let line9 = 9;
    let line10 = 10;
}

fn another() {
    println!(\"another\");
}
";
        std::fs::write(dir.join("code.rs"), original).unwrap();
        run_git(&dir, &["add", "."]).unwrap();
        run_git(&dir, &["commit", "-m", "initial"]).unwrap();

        let merge_base = run_git(&dir, &["rev-parse", "HEAD"]).unwrap();

        let modified = original.replace(
            "    let line5 = 5;",
            "    let line5 = 5;\n    let inserted = true;",
        );
        std::fs::write(dir.join("code.rs"), modified).unwrap();

        let small_diff = run_git(&dir, &["diff", "-U1", &merge_base, "--", "code.rs"]).unwrap();

        let file_section = {
            let sections = split_diff_by_file(&small_diff);
            sections
                .into_iter()
                .find(|s| s.path == "code.rs")
                .map(|s| s.content)
                .unwrap_or_default()
        };

        let result = generate_expanded_diff(&dir, "code.rs", &file_section, &merge_base);
        assert!(result.is_some(), "should return expanded diff");
        let expanded = result.unwrap();

        assert!(
            expanded.contains("target_function"),
            "expanded diff should contain function name context"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_generate_expanded_diff_includes_lines_after_hunk() {
        let dir = make_git_repo("expand-diff-after");

        let original = "\
fn big_function() {
    let a = 1;
    let b = 2;
    let c = 3;
    let d = 4;
    let e = 5;
    let f = 6;
    let g = 7;
    let h = 8;
    let i = 9;
    let j = 10;
}
";
        std::fs::write(dir.join("code.rs"), original).unwrap();
        run_git(&dir, &["add", "."]).unwrap();
        run_git(&dir, &["commit", "-m", "initial"]).unwrap();

        let merge_base = run_git(&dir, &["rev-parse", "HEAD"]).unwrap();

        let modified = original.replace("    let b = 2;", "    let b = 999;");
        std::fs::write(dir.join("code.rs"), modified).unwrap();

        let small_diff = run_git(&dir, &["diff", "-U0", &merge_base, "--", "code.rs"]).unwrap();

        let file_section = {
            let sections = split_diff_by_file(&small_diff);
            sections
                .into_iter()
                .find(|s| s.path == "code.rs")
                .map(|s| s.content)
                .unwrap_or_default()
        };

        let result = generate_expanded_diff(&dir, "code.rs", &file_section, &merge_base);
        assert!(result.is_some());
        let expanded = result.unwrap();

        assert!(
            expanded.contains("let j = 10"),
            "expanded diff should include lines after the hunk within the function"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_generate_expanded_diff_returns_none_for_missing_file() {
        let dir = make_git_repo("expand-diff-none");

        let original = "fn foo() {}\n";
        std::fs::write(dir.join("code.rs"), original).unwrap();
        run_git(&dir, &["add", "."]).unwrap();
        run_git(&dir, &["commit", "-m", "initial"]).unwrap();

        let merge_base = run_git(&dir, &["rev-parse", "HEAD"]).unwrap();

        let fake_diff = "\
diff --git a/missing.rs b/missing.rs
--- a/missing.rs
+++ b/missing.rs
@@ -1,3 +1,4 @@
 fn foo() {
+    let x = 1;
 }
";
        let result = generate_expanded_diff(&dir, "missing.rs", fake_diff, &merge_base);
        assert!(result.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
