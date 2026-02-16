# PRD: Sherpa — AI-Guided Code Review Tool

## Overview
Sherpa is a single-user local tool that pairs a developer reviewing code changes (branch diffs) with an AI guide. The developer points Sherpa at a local Git repository, and the tool analyzes the current branch's changes against its merge base. An AI (via `opencode` or `claude` CLI, user's choice) then provides a project summary, change analysis, and a step-by-step guided review — grouping changes in whatever way makes the review easiest to understand. The developer walks through each step, chatting with the AI, and validates changes as they go. Review progress is persisted to disk so sessions can be resumed.

The stack is Loco.rs (Rust backend), HTMX + FlyonUI (UI), diff2html (client-side diff rendering), and devbox for dev environment management.

## Goals
- Provide AI-guided, step-by-step code review that makes complex changesets easy to understand
- Auto-detect the branch merge base and analyze all changes on the current branch
- Let the AI fully drive how changes are grouped and ordered for review
- Persist review state (validated steps, chat history) so reviews can be resumed
- Support both `opencode` and `claude` CLI as AI backends, user-selectable from the UI
- Deliver a smooth HTMX-driven UI with FlyonUI components and diff2html for diffs

## Quality Gates

These commands must pass for every user story:
- `cargo check` — Compilation check
- `cargo nextest run` — Test suite (parallel via nextest)
- `cargo clippy` — Lint check

For UI stories, also include:
- Verify in browser using dev-browser skill

## User Stories

### US-001: Initialize project scaffolding with Loco.rs and devbox
As a developer, I want the project bootstrapped with Loco.rs, devbox configuration, and basic dependencies so that I have a working foundation to build on.

**Acceptance Criteria:**
- [ ] Loco.rs project initialized with `loco` CLI (SaaS or lightweight template — whichever fits single-user local tool best)
- [ ] `devbox.json` (or `devbox.json`) configures Rust toolchain, `cargo`, and any needed tools
- [ ] `cargo check` passes on the scaffolded project
- [ ] A `README.md` documents how to `devbox shell` then `just dev` to start the app
- [ ] The app starts and responds on `localhost` with a basic health check endpoint

### US-002: Serve HTMX + FlyonUI frontend from Loco
As a developer, I want Loco to serve HTML pages using HTMX and FlyonUI so that the UI is rendered server-side with dynamic updates.

**Acceptance Criteria:**
- [ ] Loco serves static assets (JS, CSS) including HTMX, FlyonUI CSS/JS, and diff2html
- [ ] A Tera (or similar) template engine is configured for server-side HTML rendering
- [ ] A base layout template includes HTMX script, FlyonUI styles, and diff2html assets
- [ ] A test page renders at `/` using the base layout with a FlyonUI component visible
- [ ] HTMX `hx-get` works on the test page (e.g., a button that loads a fragment)

### US-003: AI CLI detection and user selection
As a user, I want to choose between `opencode` and `claude` CLI as my AI backend so that I can use whichever tool I have installed.

**Acceptance Criteria:**
- [ ] On startup, the backend detects which CLI tools are available (`which opencode`, `which claude`)
- [ ] The UI presents a selection screen showing available CLIs (grayed out if not found)
- [ ] The user's choice is stored in a config file (e.g., `~/.sherpa/config.toml` or project-local `.sherpa/config.toml`)
- [ ] If only one CLI is available, it is pre-selected but the user can still confirm
- [ ] The selected CLI is used for all subsequent AI interactions in the session
- [ ] If neither CLI is found, an error message explains what to install

### US-004: Repository path input and Git analysis
As a user, I want to provide a directory path so that Sherpa can analyze the Git branch history of that repository.

**Acceptance Criteria:**
- [ ] The UI shows a text input for the directory path (with a "Browse" affordance or paste support)
- [ ] The backend validates: directory exists, is a Git repo, has a checked-out branch
- [ ] The backend auto-detects the merge base of the current branch against the default branch (`main`, `master`, or configured default)
- [ ] If the current branch IS the default branch, show an error: "You're on the default branch — switch to a feature branch"
- [ ] On success, the backend extracts the full diff (`git diff <merge-base>..HEAD`) and the list of changed files
- [ ] The diff data and file list are stored in memory (or temp storage) for subsequent analysis
- [ ] Loading indicator shown while Git analysis runs

### US-005: AI-generated project and change summary screen
As a user, I want to see an AI-generated overview of the project, the branch changes, implementation approach, and basic metrics so that I understand the context before reviewing.

**Acceptance Criteria:**
- [ ] The summary screen displays four sections: Project Overview, Change Summary, Implementation Approach, and Metrics
- [ ] "Project Overview" is generated by the AI analyzing the repo (e.g., README, directory structure, key files)
- [ ] "Change Summary" is generated by the AI analyzing the diff — explains WHAT the changes introduce
- [ ] "Implementation Approach" is generated by the AI — explains HOW the changes are implemented
- [ ] "Metrics" section shows: number of files changed, lines added, lines removed, number of commits on the branch
- [ ] Each AI section shows a loading state while the AI CLI processes
- [ ] If an AI call fails, the section shows an error with "Retry" and "Skip" buttons
- [ ] A chat input is available at the bottom of the summary screen for the user to ask questions about the changes
- [ ] A "Start Review" button is visible to proceed to the guided review

### US-006: AI-driven change grouping for guided review
As a user, I want the AI to group the branch changes into logical review steps so that I can review them in the most understandable order.

**Acceptance Criteria:**
- [ ] After the user clicks "Start Review", the AI is prompted to analyze the full diff and produce a review plan
- [ ] The review plan is a list of steps, each with: a title, a list of file/hunk references, and a brief rationale for the grouping
- [ ] The AI may group by feature, concept, layer, or any strategy it deems best — it is NOT constrained to file-by-file or commit-by-commit
- [ ] A single file's changes may be split across multiple steps if the AI determines it aids understanding
- [ ] The review plan is displayed as a sidebar/progress indicator showing all steps
- [ ] Loading indicator shown while the AI generates the plan
- [ ] If the AI call fails, show error with "Retry" and "Skip" (skip falls back to file-by-file grouping)
- [ ] The review plan is persisted to disk as part of the review state

### US-007: Step-by-step review UI with diff and AI explanation
As a user, I want each review step to show the diff, an AI explanation, and contextual information so that I can understand each change thoroughly.

**Acceptance Criteria:**
- [ ] Each step shows: the diff for the grouped changes rendered with diff2html (unified view by default)
- [ ] An AI-generated explanation of the changes in this step is displayed above or beside the diff
- [ ] If there is a previous step, an AI-generated explanation of how this step relates to the previous one is shown
- [ ] For each changed symbol (module, class, function), its name is displayed with an AI-generated description of its responsibilities
- [ ] diff2html renders with syntax highlighting appropriate to the file type
- [ ] The step title and step number (e.g., "Step 3 of 8") are visible
- [ ] The sidebar/progress indicator highlights the current step
- [ ] If any AI call for the step fails, the section shows an error with "Retry" and "Skip" options

### US-008: Chat interface with step-scoped highlighting
As a user, I want a chat input on each review step to discuss changes with the AI, with full history visible but current-step messages highlighted.

**Acceptance Criteria:**
- [ ] A chat panel is visible on each review step
- [ ] The user can type a message and receive an AI response (via the selected CLI)
- [ ] The AI receives context: the current step's diff, explanation, and the overall review plan
- [ ] Full chat history from all steps is available and scrollable
- [ ] Messages from the current step are visually highlighted (e.g., different background color or a divider)
- [ ] Messages from previous steps are dimmed or visually differentiated
- [ ] Chat messages are persisted as part of the review state
- [ ] The chat auto-scrolls to the latest message

### US-009: Step validation and navigation
As a user, I want to validate each review step and navigate between steps so that I can mark changes as reviewed and move through the review.

**Acceptance Criteria:**
- [ ] A "Validate & Next" button marks the current step as reviewed and advances to the next step
- [ ] A "Previous" button navigates to the prior step (without un-validating it)
- [ ] Clicking a step in the sidebar navigates directly to that step
- [ ] Validated steps show a checkmark in the sidebar
- [ ] The user can re-visit validated steps and see the diff/explanation/chat again
- [ ] Validation state is persisted to disk

### US-010: Review completion summary screen
As a user, I want to see the summary screen again when all steps are validated, with a list of reviewed changes, so that I have a final overview.

**Acceptance Criteria:**
- [ ] When the last step is validated, the user is redirected to the summary screen
- [ ] The summary screen now includes a "Reviewed Changes" section listing all review steps
- [ ] Each reviewed step is shown as a collapsible/foldable card — folded by default
- [ ] Unfolding a card shows: step title, files involved, the AI explanation, and the chat messages from that step
- [ ] The original summary sections (Project Overview, Change Summary, Implementation, Metrics) are still visible
- [ ] The chat input is still available for final questions
- [ ] A visual indicator (e.g., banner) confirms "Review Complete — all changes reviewed"

### US-011: Review state persistence and resume
As a user, I want my review progress saved to disk so that I can close the tool and resume later where I left off.

**Acceptance Criteria:**
- [ ] Review state is saved to a `.sherpa/` directory inside the analyzed repository (or a configurable location)
- [ ] State includes: repository path, branch name, merge base commit, review plan, validation status per step, and full chat history
- [ ] On startup, if a saved review exists for the current repo+branch, the user is prompted: "Resume previous review?" or "Start fresh?"
- [ ] Resuming restores the user to the last un-validated step with all previous state intact
- [ ] If the branch has new commits since the saved review, warn the user: "Branch has changed since last review — resume may be outdated"
- [ ] State is saved automatically after each step validation and chat message (no manual save)

### US-012: Error handling for AI CLI calls
As a user, I want clear error messages and recovery options when AI CLI calls fail so that a single failure doesn't block my entire review.

**Acceptance Criteria:**
- [ ] All AI CLI calls have a configurable timeout (default: 120 seconds)
- [ ] On failure (timeout, non-zero exit, malformed output), the UI shows the error message inline
- [ ] Each failed section offers "Retry" (re-run the same call) and "Skip" (proceed without that AI output)
- [ ] Skipped sections show a placeholder: "AI analysis skipped — click to retry"
- [ ] Errors are logged to `.sherpa/logs/` for debugging
- [ ] Network/CLI errors do NOT crash the application or lose review state

## Functional Requirements
- FR-1: The backend must shell out to `opencode` or `claude` CLI to perform AI analysis, passing prompts via stdin or arguments and capturing stdout
- FR-2: The backend must use `git` CLI commands to extract diff, log, merge-base, and file information
- FR-3: The UI must be server-rendered with Tera templates, using HTMX for dynamic updates without full page reloads
- FR-4: FlyonUI components must be used for all UI elements (buttons, cards, inputs, navigation, modals)
- FR-5: diff2html must be loaded client-side to render diffs from unified diff format provided by the backend
- FR-6: All AI-generated content must be streamed or loaded asynchronously with visible loading indicators
- FR-7: The review state must be serializable to JSON (or TOML) and written to disk atomically (write-tmp-then-rename)
- FR-8: The chat interface must send user messages via HTMX POST and append AI responses via HTMX swap
- FR-9: The guided review sidebar must update step status via HTMX without full page reload
- FR-10: The application must work fully offline (no external services beyond the locally installed AI CLI)

## Non-Goals (Out of Scope)
- Multi-user support or authentication
- Remote repository analysis (only local repos)
- Direct code modification or PR creation from within the tool (review only for v1)
- Custom AI model configuration (model selection is delegated to the CLI tool)
- Support for non-Git version control systems
- Side-by-side diff view (unified only for v1 — side-by-side can come later)
- Branch selection UI (always uses current checked-out branch)
- Integration with GitHub/GitLab APIs for PR metadata

## Technical Considerations
- **Loco.rs** provides the HTTP server, routing, and middleware. Use its controller/view pattern for serving templates.
- **Tera templates** for server-side HTML rendering — Loco supports this natively or via middleware.
- **HTMX** handles all dynamic interactions — form submissions, partial page updates, polling for AI responses.
- **FlyonUI** is a Tailwind CSS + DaisyUI-based component library — ensure the Tailwind build pipeline is configured (possibly via `just` recipe).
- **diff2html** loaded as a client-side JS library. The backend provides raw unified diff strings; diff2html renders them in the browser.
- **AI CLI invocation** should use Rust's `tokio::process::Command` for async subprocess execution with timeout support.
- **State persistence** as JSON files in `.sherpa/` — consider using `serde_json` for serialization.
- **devbox** should configure: Rust toolchain, Node.js (for Tailwind/FlyonUI build), and any other tools.
- **Git operations** via `git2` crate (libgit2 bindings) for merge-base detection and diff extraction, with fallback to `git` CLI if needed.

## Success Metrics
- User can go from `cargo run` to viewing a review summary in under 60 seconds (excluding AI response time)
- All review steps can be validated without errors on a typical feature branch (10-50 changed files)
- Review state persists correctly across application restarts
- AI grouping produces meaningful steps (not just file-by-file) for branches with cross-cutting changes
- Chat responses include relevant context from the current review step

## Open Questions
- Should the tool support reviewing stashed changes or only committed changes on the branch?
- What is the maximum diff size the AI CLI can handle in a single prompt? May need chunking strategy for very large branches.
- Should the `.sherpa/` state directory be gitignored by default (add to the repo's `.gitignore`)?
- Should there be a "reject" action per step (in addition to "validate") for future use when the tool supports requesting changes?
- How should the tool handle binary files in the diff (images, compiled assets)?