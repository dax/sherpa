use std::env;
use std::sync::RwLock;

use reqwest::Client;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
struct FileRefParam {
    #[schemars(description = "Relative file path within the repository")]
    path: String,
    #[schemars(description = "Optional (start, end) 1-indexed line range in the diff")]
    diff_lines: Option<(usize, usize)>,
}

impl Serialize for FileRefParam {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let field_count = if self.diff_lines.is_some() { 2 } else { 1 };
        let mut state = serializer.serialize_struct("FileRefParam", field_count)?;
        state.serialize_field("path", &self.path)?;
        if let Some(dl) = &self.diff_lines {
            state.serialize_field("diff_lines", dl)?;
        }
        state.end()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StepParam {
    #[schemars(description = "Short title describing what this review step covers")]
    title: String,
    #[schemars(description = "Why this step is included in the review plan")]
    rationale: String,
    #[schemars(description = "Files relevant to this step")]
    #[serde(default)]
    file_refs: Vec<FileRefParam>,
}

impl Serialize for StepParam {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("StepParam", 3)?;
        state.serialize_field("title", &self.title)?;
        state.serialize_field("rationale", &self.rationale)?;
        state.serialize_field("file_refs", &self.file_refs)?;
        state.end()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateReviewSessionParams {
    #[schemars(description = "Absolute path to the local git repository")]
    repo_path: String,
    #[schemars(description = "Branch name to review")]
    branch: String,
    #[schemars(description = "Review plan steps")]
    steps: Vec<StepParam>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CheckFeedbackParams {
    #[schemars(description = "Session ID returned by create_review_session")]
    session_id: String,
    #[schemars(
        description = "Only return feedback since this ISO-8601 timestamp (e.g. 2025-01-15T10:30:00.000Z)"
    )]
    since: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PushStepParams {
    #[schemars(description = "Session ID returned by create_review_session")]
    session_id: String,
    #[schemars(description = "Step number (1-indexed)")]
    step_number: usize,
    #[schemars(description = "Unified diff content for this step")]
    diff: String,
    #[schemars(description = "Files relevant to this step")]
    #[serde(default)]
    file_refs: Vec<FileRefParam>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CompleteStepParams {
    #[schemars(description = "Session ID returned by create_review_session")]
    session_id: String,
    #[schemars(description = "Step number (1-indexed)")]
    step_number: usize,
    #[schemars(description = "Unified diff content for this step")]
    diff: String,
    #[schemars(description = "List of file paths changed in this step")]
    files_changed: Vec<String>,
    #[schemars(description = "Optional AI-generated explanation of the changes")]
    explanation: Option<String>,
    #[schemars(description = "Optional git commit SHA for this step")]
    commit_sha: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FreshSessionParams {
    #[schemars(description = "Session ID of the existing session to replace")]
    session_id: String,
    #[schemars(description = "Absolute path to the local git repository")]
    repo_path: String,
    #[schemars(description = "Branch name to review")]
    branch: String,
    #[schemars(description = "New review plan steps")]
    steps: Vec<StepParam>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpdatePlanParams {
    #[schemars(description = "Session ID returned by create_review_session")]
    session_id: String,
    #[schemars(description = "New steps to replace the remaining planned (unlocked) steps")]
    steps: Vec<StepParam>,
}

#[derive(Serialize)]
struct CreateSessionBody {
    repo_path: String,
    branch: String,
    plan: PlanBody,
}

#[derive(Serialize)]
struct PlanBody {
    steps: Vec<StepParam>,
}

#[derive(Serialize)]
struct PushStepBody {
    diff: String,
    file_refs: Vec<FileRefParam>,
}

#[derive(Serialize)]
struct CompleteStepBody {
    diff: String,
    files_changed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    explanation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit_sha: Option<String>,
}

#[derive(Serialize)]
struct UpdatePlanBody {
    steps: Vec<StepParam>,
}

#[derive(Debug)]
struct SherpaMcp {
    client: Client,
    base_url: String,
    /// Per-session agent token, updated after create/fresh session calls.
    agent_token: RwLock<String>,
    tool_router: ToolRouter<Self>,
}

impl Clone for SherpaMcp {
    fn clone(&self) -> Self {
        let token = self.agent_token.read().unwrap().clone();
        Self {
            client: self.client.clone(),
            base_url: self.base_url.clone(),
            agent_token: RwLock::new(token),
            tool_router: Self::tool_router(),
        }
    }
}

impl SherpaMcp {
    fn new(base_url: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            agent_token: RwLock::new(String::new()),
            tool_router: Self::tool_router(),
        }
    }

    fn current_token(&self) -> String {
        self.agent_token.read().unwrap().clone()
    }

    fn set_token(&self, token: &str) {
        *self.agent_token.write().unwrap() = token.to_string();
    }

    /// Extract and store the agent_token from a JSON response body.
    fn extract_and_store_token(&self, body: &str) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
            if let Some(token) = val.get("agent_token").and_then(|t| t.as_str()) {
                self.set_token(token);
                tracing::info!("Agent token updated from session response");
            }
        }
    }

    /// POST without auth (used for create_session which returns the token).
    async fn api_post_open(
        &self,
        path: &str,
        json_body: &impl Serialize,
    ) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .client
            .post(&url)
            .json(json_body)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("Read body failed: {e}"))?;

        if status.is_success() || status.as_u16() == 409 {
            Ok(body)
        } else {
            Err(format!("API error (HTTP {status}): {body}"))
        }
    }

    async fn api_get(&self, path: &str) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        let token = self.current_token();
        let mut req = self.client.get(&url);
        if !token.is_empty() {
            req = req.bearer_auth(&token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("Read body failed: {e}"))?;

        if status.is_success() || status.as_u16() == 409 {
            Ok(body)
        } else {
            Err(format!("API error (HTTP {status}): {body}"))
        }
    }

    async fn api_post(&self, path: &str, json_body: &impl Serialize) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        let token = self.current_token();
        let mut req = self.client.post(&url).json(json_body);
        if !token.is_empty() {
            req = req.bearer_auth(&token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("Read body failed: {e}"))?;

        if status.is_success() || status.as_u16() == 409 {
            Ok(body)
        } else {
            Err(format!("API error (HTTP {status}): {body}"))
        }
    }

    async fn api_put(&self, path: &str, json_body: &impl Serialize) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        let token = self.current_token();
        let mut req = self.client.put(&url).json(json_body);
        if !token.is_empty() {
            req = req.bearer_auth(&token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("Read body failed: {e}"))?;

        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("API error (HTTP {status}): {body}"))
        }
    }

    async fn api_patch(&self, path: &str, json_body: &impl Serialize) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        let token = self.current_token();
        let mut req = self.client.patch(&url).json(json_body);
        if !token.is_empty() {
            req = req.bearer_auth(&token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("Read body failed: {e}"))?;

        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("API error (HTTP {status}): {body}"))
        }
    }
}

#[tool_router]
impl SherpaMcp {
    #[tool(
        name = "create_review_session",
        description = "Create a new Sherpa review session for a branch. \
        Returns session_id, agent_token, and review_url. \
        If a session already exists for the repo+branch, returns 409 with the existing session ID \
        and a hint to use fresh_session instead."
    )]
    async fn create_review_session(
        &self,
        Parameters(params): Parameters<CreateReviewSessionParams>,
    ) -> Result<String, String> {
        let body = CreateSessionBody {
            repo_path: params.repo_path,
            branch: params.branch,
            plan: PlanBody {
                steps: params.steps,
            },
        };
        let response = self.api_post_open("/api/agent/sessions", &body).await?;
        self.extract_and_store_token(&response);
        Ok(response)
    }

    #[tool(
        name = "check_feedback",
        description = "Check reviewer feedback and step validation status for a session. \
        Returns per-step status (validated/pending) and any chat comments from the reviewer. \
        Use the optional 'since' parameter to only get feedback newer than a timestamp."
    )]
    async fn check_feedback(
        &self,
        Parameters(params): Parameters<CheckFeedbackParams>,
    ) -> Result<String, String> {
        let path = if let Some(since) = &params.since {
            format!(
                "/api/agent/sessions/{}/feedback?since={}",
                params.session_id, since
            )
        } else {
            format!("/api/agent/sessions/{}/feedback", params.session_id)
        };
        self.api_get(&path).await
    }

    #[tool(
        name = "push_step",
        description = "Push a diff to a review step without completing it. \
        Sets the step status to ready_for_review so the human reviewer can see it. \
        Use this for incremental updates; use complete_step for the final submission."
    )]
    async fn push_step(
        &self,
        Parameters(params): Parameters<PushStepParams>,
    ) -> Result<String, String> {
        let path = format!(
            "/api/agent/sessions/{}/steps/{}",
            params.session_id, params.step_number
        );
        let body = PushStepBody {
            diff: params.diff,
            file_refs: params.file_refs,
        };
        self.api_put(&path, &body).await
    }

    #[tool(
        name = "complete_step",
        description = "Complete a review step with its diff, changed files, and optional explanation. \
        Marks the step as ready_for_review and spawns background AI analysis. \
        The step must be in 'Planned' status (not already pushed/completed)."
    )]
    async fn complete_step(
        &self,
        Parameters(params): Parameters<CompleteStepParams>,
    ) -> Result<String, String> {
        let path = format!(
            "/api/agent/sessions/{}/steps/{}/complete",
            params.session_id, params.step_number
        );
        let body = CompleteStepBody {
            diff: params.diff,
            files_changed: params.files_changed,
            explanation: params.explanation,
            commit_sha: params.commit_sha,
        };
        self.api_post(&path, &body).await
    }

    #[tool(
        name = "fresh_session",
        description = "Delete an existing review session and create a fresh one for the same repo+branch. \
        Use this when create_review_session returns 409 (session already exists) and you want to start over."
    )]
    async fn fresh_session(
        &self,
        Parameters(params): Parameters<FreshSessionParams>,
    ) -> Result<String, String> {
        let path = format!("/api/agent/sessions/{}/fresh", params.session_id);
        let body = CreateSessionBody {
            repo_path: params.repo_path,
            branch: params.branch,
            plan: PlanBody {
                steps: params.steps,
            },
        };
        let response = self.api_post(&path, &body).await?;
        self.extract_and_store_token(&response);
        Ok(response)
    }

    #[tool(
        name = "update_plan",
        description = "Update the remaining planned (unlocked) steps of a review session. \
        Steps that have already been pushed or validated are locked and preserved. \
        Only the planned steps at the tail of the plan are replaced with the new steps."
    )]
    async fn update_plan(
        &self,
        Parameters(params): Parameters<UpdatePlanParams>,
    ) -> Result<String, String> {
        let path = format!("/api/agent/sessions/{}/plan", params.session_id);
        let body = UpdatePlanBody {
            steps: params.steps,
        };
        self.api_patch(&path, &body).await
    }
}

#[tool_handler]
impl ServerHandler for SherpaMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Sherpa MCP server — enables AI agents to create and manage \
                 code review sessions so a human reviewer can follow your progress \
                 in real-time.\n\n\
                 ## Workflow\n\
                 1. Plan your implementation as conceptual review steps (not per-file — \
                    per concept, 50-200 lines of diff each).\n\
                 2. create_review_session — pass repo_path, branch, and your step plan.\n\
                 3. For each step, implement the code changes then call complete_step \
                    with the diff, changed files, and an explanation.\n\
                 4. Commit between steps so each step's diff is isolated.\n\
                 5. check_feedback every 2-3 steps — look for reviewer comments, \
                    validated/needs_revision status, and the blocked flag.\n\
                 6. If a step has status needs_revision, fix it and re-complete.\n\
                 7. If blocked is true, stop and wait for reviewer feedback.\n\
                 8. update_plan if your remaining steps change mid-implementation.\n\
                 9. If create_review_session returns 409, use fresh_session to start over.\n\n\
                 The human reviewer sees diffs and AI explanations in the Sherpa web UI."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let base_url = env::var("SHERPA_URL").unwrap_or_else(|_| "http://localhost:5150".to_string());

    tracing::info!("Starting Sherpa MCP server (base_url={base_url})");

    let server = SherpaMcp::new(base_url);
    let service = server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("MCP serve error: {e:?}");
    })?;

    service.waiting().await?;
    Ok(())
}
