use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use loco_rs::prelude::*;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::models::{ai_analyses, chat_messages, review_sessions};
use crate::services::background_analysis;
use crate::services::review_session::{
    FileRef, ReviewPlan, ReviewSession, ReviewStep, StepAiData, StepStatus, StepValidation,
};

#[derive(Deserialize)]
struct CreateSessionRequest {
    repo_path: String,
    branch: String,
    plan: CreateSessionPlan,
}

#[derive(Deserialize)]
struct CreateSessionPlan {
    steps: Vec<CreateSessionStep>,
}

#[derive(Deserialize)]
struct CreateSessionStep {
    title: String,
    rationale: String,
    #[serde(default)]
    file_refs: Vec<FileRef>,
}

#[derive(Serialize)]
struct CreateSessionResponse {
    session_id: String,
    agent_token: String,
    review_url: String,
}

#[derive(Serialize)]
struct ExistingSessionResponse {
    existing_session_id: String,
    message: String,
}

#[derive(Deserialize)]
pub struct FeedbackQuery {
    since: Option<String>,
}

#[derive(Serialize)]
struct FeedbackResponse {
    steps: Vec<StepFeedback>,
}

#[derive(Serialize)]
struct StepFeedback {
    step_number: usize,
    status: String,
    blocked: bool,
    comments: Vec<Comment>,
}

#[derive(Serialize)]
struct Comment {
    role: String,
    content: String,
    timestamp: String,
}

#[derive(Deserialize)]
struct PushStepRequest {
    diff: String,
    #[serde(default)]
    file_refs: Vec<FileRef>,
}

#[derive(Serialize)]
struct PushStepResponse {
    step_number: usize,
    status: String,
    review_url: String,
}

#[derive(Serialize)]
struct FreshSessionResponse {
    session_id: String,
    agent_token: String,
    review_url: String,
}

#[derive(Deserialize)]
struct UpdatePlanRequest {
    steps: Vec<UpdatePlanStep>,
}

#[derive(Deserialize)]
struct UpdatePlanStep {
    title: String,
    rationale: String,
    #[serde(default)]
    file_refs: Vec<FileRef>,
}

#[derive(Serialize)]
struct UpdatePlanResponse {
    total_steps: usize,
    locked_steps: usize,
    updated_steps: usize,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct CompleteStepRequest {
    diff: String,
    #[serde(default)]
    explanation: Option<String>,
    files_changed: Vec<String>,
    #[serde(default)]
    commit_sha: Option<String>,
}

#[derive(Serialize)]
struct CompleteStepResponse {
    step_number: usize,
    status: String,
    review_url: String,
    explanation_status: String,
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

/// Validate the Bearer token in the request headers against the session's stored agent_token.
fn validate_session_token(headers: &HeaderMap, session: &ReviewSession) -> Result<()> {
    let expected_token = session
        .agent_token
        .as_ref()
        .ok_or_else(|| Error::Unauthorized("Session has no agent token".to_string()))?;

    let provided_token = extract_bearer_token(headers)
        .ok_or_else(|| Error::Unauthorized("Missing Bearer token".to_string()))?;

    if provided_token != *expected_token {
        return Err(Error::Unauthorized("Invalid agent token".to_string()));
    }

    Ok(())
}

fn generate_agent_token() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_nanos();
    let pid = std::process::id();

    let mut hasher = DefaultHasher::new();
    nanos.hash(&mut hasher);
    pid.hash(&mut hasher);
    let h1 = hasher.finish();

    (nanos + 1).hash(&mut hasher);
    let h2 = hasher.finish();

    format!("agent-{h1:016x}{h2:016x}")
}

#[debug_handler]
async fn create_session(
    State(ctx): State<AppContext>,
    Json(body): Json<CreateSessionRequest>,
) -> Result<Response> {
    if body.plan.steps.is_empty() {
        return Err(Error::BadRequest("Plan must have at least one step".into()));
    }

    if let Ok(Some(existing)) =
        review_sessions::Model::find_by_repo_and_branch(&ctx.db, &body.repo_path, &body.branch)
            .await
    {
        let resp = ExistingSessionResponse {
            existing_session_id: existing.session_key.clone(),
            message: format!(
                "A review session already exists for {}@{}. \
                 POST to /api/agent/sessions/{}/fresh to start fresh, \
                 or use the existing session.",
                body.repo_path, body.branch, existing.session_key
            ),
        };
        return Ok(axum::response::Response::builder()
            .status(409)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::to_string(&resp).unwrap_or_default(),
            ))
            .unwrap());
    }

    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let steps: Vec<ReviewStep> = body
        .plan
        .steps
        .into_iter()
        .map(|s| ReviewStep {
            title: s.title,
            rationale: s.rationale,
            file_refs: s.file_refs,
            ai_data: StepAiData::default(),
            status: StepStatus::Planned,
            step_diff: None,
        })
        .collect();

    let plan = ReviewPlan {
        steps,
        generated_at: now,
    };

    let agent_token = generate_agent_token();
    let session = ReviewSession::new_live(body.repo_path, body.branch, plan, agent_token.clone());
    let session_id = session.id.clone();

    review_sessions::find_or_create(&ctx.db, &session)
        .await
        .map_err(|e| {
            tracing::error!("DB error creating agent session: {e}");
            Error::InternalServerError
        })?;

    let review_url = format!("/review/{session_id}/guide");
    let resp = CreateSessionResponse {
        session_id,
        agent_token,
        review_url,
    };

    format::json(resp)
}

#[debug_handler]
async fn feedback(
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
    Query(query): Query<FeedbackQuery>,
) -> Result<Response> {
    let (model, session) = load_session_from_db(&ctx.db, &session_id).await?;
    validate_session_token(&headers, &session)?;
    let session_db_id = model.id;

    let messages = if let Some(since_str) = &query.since {
        let since_dt = parse_timestamp(since_str).ok_or_else(|| {
            tracing::error!("Invalid since timestamp: {since_str}");
            Error::BadRequest("Invalid 'since' timestamp format".into())
        })?;
        chat_messages::find_by_session_since(&ctx.db, session_db_id, since_dt)
            .await
            .map_err(|e| {
                tracing::error!("DB error loading chat messages: {e}");
                Error::InternalServerError
            })?
    } else {
        chat_messages::find_by_session(&ctx.db, session_db_id)
            .await
            .map_err(|e| {
                tracing::error!("DB error loading chat messages: {e}");
                Error::InternalServerError
            })?
    };

    let plan = session.review_plan.as_ref();
    let total_steps = plan.map(|p| p.steps.len()).unwrap_or(0);

    let mut step_feedbacks: Vec<StepFeedback> = Vec::new();

    for step_idx in 0..total_steps {
        let step_number = step_idx + 1;

        let step_messages: Vec<&chat_messages::Model> = messages
            .iter()
            .filter(|m| m.step_number == Some(step_number as i32))
            .collect();

        let validated = session.is_step_validated(step_idx);

        let step_status = plan.and_then(|p| p.steps.get(step_idx).map(|s| &s.status));
        let is_needs_revision = step_status == Some(&StepStatus::NeedsRevision);

        let has_changes = !step_messages.is_empty() || validated || is_needs_revision;

        if query.since.is_some() && !has_changes {
            continue;
        }

        let status = if is_needs_revision {
            "needs_revision".to_string()
        } else if validated {
            "validated".to_string()
        } else {
            "pending".to_string()
        };

        let blocked = is_needs_revision && session.block_agent.unwrap_or(false);

        let comments: Vec<Comment> = step_messages
            .iter()
            .map(|msg| Comment {
                role: msg.role.clone(),
                content: msg.content.clone(),
                timestamp: msg.created_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            })
            .collect();

        step_feedbacks.push(StepFeedback {
            step_number,
            status,
            blocked,
            comments,
        });
    }

    let summary_messages: Vec<&chat_messages::Model> = messages
        .iter()
        .filter(|m| m.step_number.is_none())
        .collect();

    if !summary_messages.is_empty() {
        let comments: Vec<Comment> = summary_messages
            .iter()
            .map(|msg| Comment {
                role: msg.role.clone(),
                content: msg.content.clone(),
                timestamp: msg.created_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            })
            .collect();

        step_feedbacks.insert(
            0,
            StepFeedback {
                step_number: 0,
                status: "summary".to_string(),
                blocked: false,
                comments,
            },
        );
    }

    let response = FeedbackResponse {
        steps: step_feedbacks,
    };

    format::json(response)
}

#[debug_handler]
async fn push_step(
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path((session_id, step_number)): Path<(String, usize)>,
    Json(body): Json<PushStepRequest>,
) -> Result<Response> {
    let (_model, mut session) = load_session_from_db(&ctx.db, &session_id).await?;
    validate_session_token(&headers, &session)?;

    let plan = session
        .review_plan
        .as_mut()
        .ok_or_else(|| Error::BadRequest("Session has no review plan".into()))?;

    if step_number < 1 || step_number > plan.steps.len() {
        return Err(Error::BadRequest(format!(
            "Step {step_number} out of range (1-{})",
            plan.steps.len()
        )));
    }

    let idx = step_number - 1;
    let step = &mut plan.steps[idx];
    step.step_diff = Some(body.diff);
    step.status = StepStatus::ReadyForReview;
    if !body.file_refs.is_empty() {
        step.file_refs = body.file_refs;
    }

    let plan_json = serde_json::to_string(&session.review_plan).ok();
    review_sessions::update_review_plan(&ctx.db, &session_id, plan_json)
        .await
        .map_err(|e| {
            tracing::error!("DB error updating step {step_number}: {e}");
            Error::InternalServerError
        })?;

    let port = std::env::var("PORT").unwrap_or_else(|_| "5150".to_string());
    let resp = PushStepResponse {
        step_number,
        status: "ready_for_review".to_string(),
        review_url: format!("http://localhost:{port}/review/{session_id}/guide/step/{step_number}"),
    };

    format::json(resp)
}

#[debug_handler]
async fn complete_step(
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path((session_id, step_number)): Path<(String, usize)>,
    Json(body): Json<CompleteStepRequest>,
) -> Result<Response> {
    let (model, mut session) = load_session_from_db(&ctx.db, &session_id).await?;
    validate_session_token(&headers, &session)?;
    let session_db_id = model.id;

    let plan = session
        .review_plan
        .as_mut()
        .ok_or_else(|| Error::BadRequest("Session has no review plan".into()))?;

    if step_number < 1 || step_number > plan.steps.len() {
        return Err(Error::BadRequest(format!(
            "Step {step_number} out of range (1-{})",
            plan.steps.len()
        )));
    }

    let idx = step_number - 1;
    let step = &plan.steps[idx];

    if step.status != StepStatus::Planned && step.status != StepStatus::NeedsRevision {
        return Err(Error::BadRequest(format!(
            "Step {step_number} is not in Planned or NeedsRevision status (current: {:?})",
            step.status
        )));
    }

    let file_refs: Vec<FileRef> = body
        .files_changed
        .iter()
        .map(|p| FileRef {
            path: p.clone(),
            diff_lines: None,
        })
        .collect();

    let step = &mut plan.steps[idx];
    step.step_diff = Some(body.diff.clone());
    step.status = StepStatus::ReadyForReview;
    if !file_refs.is_empty() {
        step.file_refs = file_refs;
    }

    let explanation_status = if let Some(ref explanation) = body.explanation {
        step.ai_data.explanation = Some(explanation.clone());
        ai_analyses::upsert(
            &ctx.db,
            session_db_id,
            "step_explanation",
            Some(step_number as i32),
            explanation,
        )
        .await
        .map_err(|e| {
            tracing::error!("DB error caching agent explanation for step {step_number}: {e}");
            Error::InternalServerError
        })?;
        "provided".to_string()
    } else {
        "pending".to_string()
    };

    if !body.diff.is_empty() {
        if session.diff.is_empty() {
            session.diff = body.diff.clone();
        } else {
            session.diff.push('\n');
            session.diff.push_str(&body.diff);
        }
    }

    let plan_json = serde_json::to_string(&session.review_plan).ok();
    review_sessions::update_review_plan(&ctx.db, &session_id, plan_json)
        .await
        .map_err(|e| {
            tracing::error!("DB error updating step {step_number}: {e}");
            Error::InternalServerError
        })?;

    review_sessions::update_diff(&ctx.db, &session_id, &session.diff)
        .await
        .map_err(|e| {
            tracing::error!("DB error updating cumulative diff: {e}");
            Error::InternalServerError
        })?;

    background_analysis::spawn_live_step_analyses(
        ctx.db.clone(),
        session_db_id,
        session.clone(),
        step_number,
        body.diff.clone(),
    );

    let port = std::env::var("PORT").unwrap_or_else(|_| "5150".to_string());
    let resp = CompleteStepResponse {
        step_number,
        status: "ready_for_review".to_string(),
        review_url: format!("http://localhost:{port}/review/{session_id}/guide/step/{step_number}"),
        explanation_status,
    };

    format::json(resp)
}

#[debug_handler]
async fn fresh_session(
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
    Json(body): Json<CreateSessionRequest>,
) -> Result<Response> {
    let (_model, session) = load_session_from_db(&ctx.db, &session_id).await?;
    validate_session_token(&headers, &session)?;

    review_sessions::Model::delete_by_repo_and_branch(&ctx.db, &session.repo_path, &session.branch)
        .await
        .map_err(|e| {
            tracing::error!("DB error deleting session for fresh start: {e}");
            Error::InternalServerError
        })?;

    if body.plan.steps.is_empty() {
        return Err(Error::BadRequest("Plan must have at least one step".into()));
    }

    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let steps: Vec<ReviewStep> = body
        .plan
        .steps
        .into_iter()
        .map(|s| ReviewStep {
            title: s.title,
            rationale: s.rationale,
            file_refs: s.file_refs,
            ai_data: StepAiData::default(),
            status: StepStatus::Planned,
            step_diff: None,
        })
        .collect();

    let plan = ReviewPlan {
        steps,
        generated_at: now,
    };

    let agent_token = generate_agent_token();
    let new_session =
        ReviewSession::new_live(body.repo_path, body.branch, plan, agent_token.clone());
    let new_session_id = new_session.id.clone();

    review_sessions::find_or_create(&ctx.db, &new_session)
        .await
        .map_err(|e| {
            tracing::error!("DB error creating fresh agent session: {e}");
            Error::InternalServerError
        })?;

    let review_url = format!("/review/{new_session_id}/guide");
    let resp = FreshSessionResponse {
        session_id: new_session_id,
        agent_token,
        review_url,
    };

    format::json(resp)
}

#[debug_handler]
async fn update_plan(
    headers: HeaderMap,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
    Json(body): Json<UpdatePlanRequest>,
) -> Result<Response> {
    let (_model, mut session) = load_session_from_db(&ctx.db, &session_id).await?;
    validate_session_token(&headers, &session)?;

    let plan = session
        .review_plan
        .as_mut()
        .ok_or_else(|| Error::BadRequest("Session has no review plan".into()))?;

    let locked_count = plan
        .steps
        .iter()
        .filter(|s| s.status != StepStatus::Planned)
        .count();

    let locked_prefix_len = plan
        .steps
        .iter()
        .take_while(|s| s.status != StepStatus::Planned)
        .count();

    if locked_prefix_len != locked_count {
        return Err(Error::BadRequest(
            "Cannot update plan: non-Planned steps are not contiguous at the start".into(),
        ));
    }

    let locked_steps: Vec<ReviewStep> = plan.steps.drain(..locked_prefix_len).collect();

    let new_planned: Vec<ReviewStep> = body
        .steps
        .into_iter()
        .map(|s| ReviewStep {
            title: s.title,
            rationale: s.rationale,
            file_refs: s.file_refs,
            ai_data: StepAiData::default(),
            status: StepStatus::Planned,
            step_diff: None,
        })
        .collect();

    let updated_count = new_planned.len();
    let mut all_steps = locked_steps;
    all_steps.extend(new_planned);

    if all_steps.is_empty() {
        return Err(Error::BadRequest("Plan must have at least one step".into()));
    }

    plan.steps = all_steps;
    let total = plan.steps.len();

    session.validated_steps.truncate(locked_prefix_len);
    session
        .validated_steps
        .resize(total, StepValidation::default());

    let plan_json = serde_json::to_string(&session.review_plan).ok();
    review_sessions::update_review_plan(&ctx.db, &session_id, plan_json)
        .await
        .map_err(|e| {
            tracing::error!("DB error updating plan for session {session_id}: {e}");
            Error::InternalServerError
        })?;

    review_sessions::update_validated_steps(&ctx.db, &session_id, &session.validated_steps)
        .await
        .map_err(|e| {
            tracing::error!("DB error updating validated_steps for session {session_id}: {e}");
            Error::InternalServerError
        })?;

    let resp = UpdatePlanResponse {
        total_steps: total,
        locked_steps: locked_prefix_len,
        updated_steps: updated_count,
    };

    format::json(resp)
}

async fn load_session_from_db(
    db: &DatabaseConnection,
    session_id: &str,
) -> Result<(
    review_sessions::Model,
    crate::services::review_session::ReviewSession,
)> {
    let model = review_sessions::Model::find_by_session_key(db, session_id)
        .await
        .map_err(|e| {
            tracing::error!("DB error loading session {session_id}: {e}");
            Error::InternalServerError
        })?
        .ok_or_else(|| {
            tracing::error!("Session not found in DB: {session_id}");
            Error::NotFound
        })?;
    let session = model.to_review_session();
    Ok((model, session))
}

fn parse_timestamp(s: &str) -> Option<chrono::NaiveDateTime> {
    // Try ISO 8601 format: 2025-01-15T10:30:00.000Z
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ") {
        return Some(dt);
    }
    // Try without fractional seconds: 2025-01-15T10:30:00Z
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ") {
        return Some(dt);
    }
    // Try Unix timestamp (seconds)
    if let Ok(ts) = s.parse::<i64>() {
        return chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.naive_utc());
    }
    None
}

pub fn page_routes() -> Routes {
    Routes::new().prefix("/api/agent")
}

pub fn api_routes() -> Routes {
    Routes::new()
        .prefix("/api/agent")
        .add("/sessions", post(create_session))
        .add("/sessions/{session_id}/feedback", get(feedback))
        .add("/sessions/{session_id}/steps/{step_number}", put(push_step))
        .add(
            "/sessions/{session_id}/steps/{step_number}/complete",
            post(complete_step),
        )
        .add("/sessions/{session_id}/fresh", post(fresh_session))
        .add("/sessions/{session_id}/plan", patch(update_plan))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_bearer_token_valid() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer test-token-123".parse().unwrap());
        assert_eq!(
            extract_bearer_token(&headers),
            Some("test-token-123".to_string())
        );
    }

    #[test]
    fn test_extract_bearer_token_missing() {
        let headers = HeaderMap::new();
        assert_eq!(extract_bearer_token(&headers), None);
    }

    #[test]
    fn test_extract_bearer_token_wrong_scheme() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Basic dXNlcjpwYXNz".parse().unwrap());
        assert_eq!(extract_bearer_token(&headers), None);
    }

    #[test]
    fn test_parse_timestamp_iso8601_with_millis() {
        let result = parse_timestamp("2025-01-15T10:30:00.123Z");
        assert!(result.is_some());
        let dt = result.unwrap();
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2025-01-15 10:30:00"
        );
    }

    #[test]
    fn test_parse_timestamp_iso8601_no_millis() {
        let result = parse_timestamp("2025-01-15T10:30:00Z");
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_timestamp_unix() {
        let result = parse_timestamp("1705312200");
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_timestamp_invalid() {
        assert!(parse_timestamp("not-a-date").is_none());
        assert!(parse_timestamp("").is_none());
    }

    #[test]
    fn test_generate_agent_token_format() {
        let token = generate_agent_token();
        assert!(token.starts_with("agent-"));
        assert!(token.len() > 20);
    }

    #[test]
    fn test_generate_agent_token_uniqueness() {
        let t1 = generate_agent_token();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let t2 = generate_agent_token();
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_deserialize_create_session_request() {
        let json = r#"{
            "repo_path": "/tmp/my-repo",
            "branch": "feature/auth",
            "plan": {
                "steps": [
                    {
                        "title": "Add auth middleware",
                        "rationale": "Foundation for auth system",
                        "file_refs": [{"path": "src/auth.rs"}]
                    },
                    {
                        "title": "Add login endpoint",
                        "rationale": "User-facing login API"
                    }
                ]
            }
        }"#;
        let req: CreateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.repo_path, "/tmp/my-repo");
        assert_eq!(req.branch, "feature/auth");
        assert_eq!(req.plan.steps.len(), 2);
        assert_eq!(req.plan.steps[0].title, "Add auth middleware");
        assert_eq!(req.plan.steps[0].file_refs.len(), 1);
        assert_eq!(req.plan.steps[1].file_refs.len(), 0);
    }

    #[test]
    fn test_deserialize_push_step_request_with_file_refs() {
        let json = r#"{
            "diff": "diff --git a/src/auth.rs b/src/auth.rs\n+pub fn login() {}",
            "file_refs": [
                {"path": "src/auth.rs"},
                {"path": "src/middleware.rs", "diff_lines": [1, 30]}
            ]
        }"#;
        let req: PushStepRequest = serde_json::from_str(json).unwrap();
        assert!(req.diff.contains("auth.rs"));
        assert_eq!(req.file_refs.len(), 2);
        assert_eq!(req.file_refs[0].path, "src/auth.rs");
        assert_eq!(req.file_refs[1].diff_lines, Some((1, 30)));
    }

    #[test]
    fn test_deserialize_push_step_request_minimal() {
        let json = r#"{"diff": "some diff content"}"#;
        let req: PushStepRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.diff, "some diff content");
        assert!(req.file_refs.is_empty());
    }

    #[test]
    fn test_serialize_push_step_response() {
        let resp = PushStepResponse {
            step_number: 3,
            status: "ready_for_review".to_string(),
            review_url: "http://localhost:5150/review/r-123/guide/step/3".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"step_number\":3"));
        assert!(json.contains("ready_for_review"));
        assert!(json.contains("/guide/step/3"));
    }

    #[test]
    fn test_serialize_fresh_session_response() {
        let resp = FreshSessionResponse {
            session_id: "review-999".to_string(),
            agent_token: "agent-abc123".to_string(),
            review_url: "/review/review-999/guide".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("review-999"));
        assert!(json.contains("agent-abc123"));
    }

    #[test]
    fn test_deserialize_update_plan_request() {
        let json = r#"{
            "steps": [
                {
                    "title": "New step A",
                    "rationale": "Reordered first",
                    "file_refs": [{"path": "src/a.rs"}]
                },
                {
                    "title": "New step B",
                    "rationale": "Appended"
                }
            ]
        }"#;
        let req: UpdatePlanRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.steps.len(), 2);
        assert_eq!(req.steps[0].title, "New step A");
        assert_eq!(req.steps[0].file_refs.len(), 1);
        assert_eq!(req.steps[1].file_refs.len(), 0);
    }

    #[test]
    fn test_deserialize_update_plan_request_empty_steps() {
        let json = r#"{"steps": []}"#;
        let req: UpdatePlanRequest = serde_json::from_str(json).unwrap();
        assert!(req.steps.is_empty());
    }

    #[test]
    fn test_serialize_update_plan_response() {
        let resp = UpdatePlanResponse {
            total_steps: 5,
            locked_steps: 2,
            updated_steps: 3,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"total_steps\":5"));
        assert!(json.contains("\"locked_steps\":2"));
        assert!(json.contains("\"updated_steps\":3"));
    }

    #[test]
    fn test_deserialize_complete_step_request_full() {
        let json = r#"{
            "diff": "diff --git a/src/lib.rs b/src/lib.rs\n+pub fn new_fn() {}",
            "explanation": "Added new_fn for feature X",
            "files_changed": ["src/lib.rs", "src/main.rs"],
            "commit_sha": "abc123def456"
        }"#;
        let req: CompleteStepRequest = serde_json::from_str(json).unwrap();
        assert!(req.diff.contains("lib.rs"));
        assert_eq!(req.explanation, Some("Added new_fn for feature X".into()));
        assert_eq!(req.files_changed, vec!["src/lib.rs", "src/main.rs"]);
        assert_eq!(req.commit_sha, Some("abc123def456".into()));
    }

    #[test]
    fn test_deserialize_complete_step_request_minimal() {
        let json = r#"{
            "diff": "some diff",
            "files_changed": ["src/foo.rs"]
        }"#;
        let req: CompleteStepRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.diff, "some diff");
        assert!(req.explanation.is_none());
        assert_eq!(req.files_changed, vec!["src/foo.rs"]);
        assert!(req.commit_sha.is_none());
    }

    #[test]
    fn test_serialize_complete_step_response() {
        let resp = CompleteStepResponse {
            step_number: 2,
            status: "ready_for_review".to_string(),
            review_url: "http://localhost:5150/review/r-123/guide/step/2".to_string(),
            explanation_status: "provided".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"step_number\":2"));
        assert!(json.contains("ready_for_review"));
        assert!(json.contains("/guide/step/2"));
        assert!(json.contains("\"explanation_status\":\"provided\""));
    }

    #[test]
    fn test_serialize_complete_step_response_pending_explanation() {
        let resp = CompleteStepResponse {
            step_number: 1,
            status: "ready_for_review".to_string(),
            review_url: "http://localhost:5150/review/r-456/guide/step/1".to_string(),
            explanation_status: "pending".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"explanation_status\":\"pending\""));
    }

    #[test]
    fn test_step_feedback_serialization_with_blocked() {
        let feedback = StepFeedback {
            step_number: 3,
            status: "needs_revision".to_string(),
            blocked: true,
            comments: vec![Comment {
                role: "user".to_string(),
                content: "Please fix the error handling".to_string(),
                timestamp: "2025-01-15T10:30:00.000Z".to_string(),
            }],
        };
        let json = serde_json::to_string(&feedback).unwrap();
        assert!(json.contains("\"blocked\":true"));
        assert!(json.contains("\"status\":\"needs_revision\""));
        assert!(json.contains("\"step_number\":3"));
    }

    #[test]
    fn test_step_feedback_serialization_not_blocked() {
        let feedback = StepFeedback {
            step_number: 1,
            status: "validated".to_string(),
            blocked: false,
            comments: vec![],
        };
        let json = serde_json::to_string(&feedback).unwrap();
        assert!(json.contains("\"blocked\":false"));
        assert!(json.contains("\"status\":\"validated\""));
    }

    #[test]
    fn test_feedback_response_with_blocked_steps() {
        let response = FeedbackResponse {
            steps: vec![
                StepFeedback {
                    step_number: 1,
                    status: "validated".to_string(),
                    blocked: false,
                    comments: vec![],
                },
                StepFeedback {
                    step_number: 2,
                    status: "needs_revision".to_string(),
                    blocked: true,
                    comments: vec![],
                },
            ],
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let steps = parsed["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0]["blocked"], false);
        assert_eq!(steps[1]["blocked"], true);
        assert_eq!(steps[1]["status"], "needs_revision");
    }
}
