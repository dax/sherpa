use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Form;
use loco_rs::prelude::*;
use sea_orm::DatabaseConnection;
use serde::Deserialize;

use crate::models::{ai_analyses, chat_messages, review_sessions};
use crate::services::{
    ai_cli::{self, AiCliError, AiPrompt, AiSettings, ModelTier, PrimedSession},
    background_analysis,
    config::SherpaConfig,
    git_analysis, markdown,
    review_session::{
        fallback_review_plan, ChatMessage, ReviewPlan, ReviewSession, ReviewStep, StepStatus,
    },
};

fn load_ai_settings() -> Option<AiSettings> {
    ai_cli::load_ai_settings()
}

fn log_ai_error(error: &AiCliError) {
    tracing::error!("AI CLI call failed: {error}");

    if let Ok(log_dir) = SherpaConfig::config_dir().map(|d| d.join("logs")) {
        let _ = std::fs::create_dir_all(&log_dir);
        let log_path = log_dir.join("ai_errors.log");
        let timestamp = chrono_now();
        let line = format!("[{timestamp}] {error}\n");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let _ = file.write_all(line.as_bytes());
        }
    }
}

fn build_changed_files_summary(session: &ReviewSession) -> String {
    session
        .changed_files
        .iter()
        .map(|f| format!("{}\t{}", f.status, f.path))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_ai_context(session: &ReviewSession) -> String {
    let files_summary = build_changed_files_summary(session);
    ai_cli::build_context(
        &session.branch,
        &session.default_branch,
        &files_summary,
        &session.diff,
    )
}

#[debug_handler]
async fn summary_page(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let (model, mut session) = load_session_from_db(&ctx.db, &session_id).await?;
    let session_db_id = model.id;

    session.ensure_validated_steps_size();

    let all_validated = session.all_steps_validated();

    let db_messages = chat_messages::find_by_session(&ctx.db, session_db_id)
        .await
        .map_err(|e| {
            tracing::error!("DB error loading chat messages: {e}");
            Error::InternalServerError
        })?;

    let reviewed_steps: Vec<serde_json::Value> = if all_validated {
        if let Some(ref plan) = session.review_plan {
            let mut steps_json = Vec::new();
            for (i, step) in plan.steps.iter().enumerate() {
                let step_number = i + 1;
                let files: Vec<String> = step.file_refs.iter().map(|f| f.path.clone()).collect();

                let explanation = ai_analyses::find_cached(
                    &ctx.db,
                    session_db_id,
                    "step_explanation",
                    Some(step_number as i32),
                )
                .await
                .map_err(|e| {
                    tracing::error!("DB error loading step explanation: {e}");
                    Error::InternalServerError
                })?
                .map(|c| markdown::to_html(&c))
                .unwrap_or_default();

                let step_chat: Vec<serde_json::Value> = db_messages
                    .iter()
                    .filter(|msg| msg.step_number == Some(step_number as i32))
                    .map(|msg| chat_msg_to_json(msg, None))
                    .collect();

                steps_json.push(serde_json::json!({
                    "number": step_number,
                    "title": step.title,
                    "rationale": step.rationale,
                    "files": files,
                    "explanation": explanation,
                    "chat_messages": step_chat,
                    "has_chat": !step_chat.is_empty(),
                }));
            }
            steps_json
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let chat_messages_json: Vec<serde_json::Value> = db_messages
        .iter()
        .map(|msg| chat_msg_to_json(msg, None))
        .collect();

    let is_live = session.is_live();
    let has_plan = session.review_plan.is_some();

    let metrics = if is_live && !session.diff.is_empty() {
        let (lines_added, lines_removed) = git_analysis::compute_diff_line_stats(&session.diff);
        let files_changed = session
            .diff
            .lines()
            .filter(|l| l.starts_with("diff --git"))
            .count();
        serde_json::json!({
            "files_changed": files_changed,
            "lines_added": lines_added,
            "lines_removed": lines_removed,
            "commits_on_branch": 0,
        })
    } else {
        serde_json::json!(session.metrics)
    };

    format::render().view(
        &v,
        "review/summary.html",
        data!({
            "session_id": session.id,
            "branch": session.branch,
            "default_branch": session.default_branch,
            "metrics": metrics,
            "chat_messages": chat_messages_json,
            "all_validated": all_validated,
            "reviewed_steps": reviewed_steps,
            "is_live": is_live,
            "has_plan": has_plan,
        }),
    )
}

async fn load_session_from_db(
    db: &DatabaseConnection,
    session_id: &str,
) -> Result<(review_sessions::Model, ReviewSession)> {
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

fn synthesize_approach_from_plan(session: &ReviewSession) -> String {
    let mut md = String::from("This is a **live review session** created by an AI coding agent. ");
    md.push_str("The implementation is organized into the following steps:\n\n");

    if let Some(ref plan) = session.review_plan {
        for (i, step) in plan.steps.iter().enumerate() {
            md.push_str(&format!(
                "{}. **{}** — {}\n",
                i + 1,
                step.title,
                step.rationale
            ));
        }
    }

    markdown::to_html(&md)
}

fn chat_msg_to_json(msg: &chat_messages::Model, current_step: Option<usize>) -> serde_json::Value {
    let content = if msg.role == "assistant" {
        markdown::to_html(&msg.content)
    } else {
        msg.content.clone()
    };
    let step_number = msg.step_number;
    let timestamp = msg.created_at.format("%H:%M:%S").to_string();
    serde_json::json!({
        "role": msg.role,
        "content": content,
        "timestamp": timestamp,
        "step_number": step_number,
        "is_current_step": current_step.is_some() && step_number == current_step.map(|n| n as i32),
    })
}

async fn generate_with_fork_or_fallback(
    settings: &AiSettings,
    primed: Option<&PrimedSession>,
    instruction: &str,
    context: &str,
    tier: ModelTier,
) -> std::result::Result<String, ai_cli::AiCliError> {
    let model = settings.model_for_tier(tier);

    if let Some(primed) = primed {
        match ai_cli::generate_forked(primed, instruction, settings.timeout, model).await {
            Ok(content) => return Ok(content),
            Err(e) => {
                tracing::warn!("Forked generation failed ({e}), falling back to legacy");
            }
        }
    }

    let prompt = AiPrompt {
        context: context.to_string(),
        instruction: instruction.to_string(),
    };
    ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout, model).await
}

fn build_primed_session(session: &ReviewSession) -> Option<PrimedSession> {
    let sid = session.primed_session_id.as_ref()?;
    let settings = load_ai_settings()?;
    Some(PrimedSession {
        session_id: sid.clone(),
        cli: settings.cli,
    })
}

async fn generate_section(
    db: &DatabaseConnection,
    session_db_id: i32,
    analysis_type: &str,
    instruction: &str,
    context: &str,
    tier: ModelTier,
    primed: Option<&PrimedSession>,
) -> std::result::Result<(String, bool), String> {
    if let Some(cached) = ai_analyses::find_cached(db, session_db_id, analysis_type, None)
        .await
        .map_err(|e| format!("DB error checking cache: {e}"))?
    {
        return Ok((markdown::to_html(&cached), true));
    }

    let settings = load_ai_settings().ok_or("No AI backend configured")?;
    let content = generate_with_fork_or_fallback(&settings, primed, instruction, context, tier)
        .await
        .map_err(|e| {
            log_ai_error(&e);
            e.to_string()
        })?;

    ai_analyses::upsert(db, session_db_id, analysis_type, None, &content)
        .await
        .map_err(|e| format!("DB error saving analysis: {e}"))?;

    Ok((markdown::to_html(&content), false))
}

#[debug_handler]
async fn summary_approach(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let title = "Implementation Approach";
    let section_id = "approach";

    let (model, session) = load_session_from_db(&ctx.db, &session_id).await?;

    if session.is_live() {
        let content = synthesize_approach_from_plan(&session);
        return format::render().view(
            &v,
            "review/_section_content.html",
            data!({"title": title, "content": content, "section_id": section_id}),
        );
    }

    let context = build_ai_context(&session);
    let primed = build_primed_session(&session);

    match generate_section(
        &ctx.db,
        model.id,
        "approach",
        ai_cli::approach_instruction(),
        &context,
        ModelTier::Deep,
        primed.as_ref(),
    )
    .await
    {
        Ok((content, _)) => format::render().view(
            &v,
            "review/_section_content.html",
            data!({"title": title, "content": content, "section_id": section_id}),
        ),
        Err(error) => format::render().view(
            &v,
            "review/_section_error.html",
            data!({
                "title": title,
                "error": error,
                "section_id": section_id,
                "session_id": session_id,
                "retry_url": format!("/review/{session_id}/summary/approach"),
            }),
        ),
    }
}

#[debug_handler]
async fn section_skip(
    ViewEngine(v): ViewEngine<TeraView>,
    Path((session_id, section)): Path<(String, String)>,
) -> Result<Response> {
    let title = match section.as_str() {
        "approach" => "Implementation Approach",
        _ => "Unknown Section",
    };

    format::render().view(
        &v,
        "review/_section_skipped.html",
        data!({
            "title": title,
            "section_id": section,
            "session_id": session_id,
            "retry_url": format!("/review/{session_id}/summary/{section}"),
        }),
    )
}

#[derive(Deserialize)]
pub struct ChatForm {
    message: String,
}

#[debug_handler]
async fn summary_chat(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
    Form(form): Form<ChatForm>,
) -> Result<Response> {
    let (model, mut session) = load_session_from_db(&ctx.db, &session_id).await?;
    let session_db_id = model.id;

    let user_msg_model = chat_messages::create(&ctx.db, session_db_id, None, "user", &form.message)
        .await
        .map_err(|e| {
            tracing::error!("DB error saving user chat message: {e}");
            Error::InternalServerError
        })?;

    let user_msg = ChatMessage {
        role: "user".to_string(),
        content: form.message.clone(),
        timestamp: chrono_now(),
        step_number: None,
    };
    session.chat_messages.push(user_msg);

    let settings = load_ai_settings();
    let ai_response = if let Some(settings) = settings {
        let context = build_ai_context(&session);
        let instruction = format!(
            "The user asks: {}\n\nRespond helpfully based on the code changes above.",
            form.message
        );
        let primed = build_primed_session(&session);
        match generate_with_fork_or_fallback(
            &settings,
            primed.as_ref(),
            &instruction,
            &context,
            ModelTier::Fast,
        )
        .await
        {
            Ok(response) => response,
            Err(e) => {
                log_ai_error(&e);
                format!("AI error: {e}")
            }
        }
    } else {
        "No AI backend configured. Please set up an AI backend first.".to_string()
    };

    let ai_msg_model =
        chat_messages::create(&ctx.db, session_db_id, None, "assistant", &ai_response)
            .await
            .map_err(|e| {
                tracing::error!("DB error saving AI chat message: {e}");
                Error::InternalServerError
            })?;

    let ai_msg = ChatMessage {
        role: "assistant".to_string(),
        content: ai_response,
        timestamp: chrono_now(),
        step_number: None,
    };
    session.chat_messages.push(ai_msg);
    let _ = session.save();

    let last_two: Vec<serde_json::Value> = vec![
        chat_msg_to_json(&user_msg_model, None),
        chat_msg_to_json(&ai_msg_model, None),
    ];

    format::render().view(
        &v,
        "review/_chat_messages.html",
        data!({"messages": last_two}),
    )
}

#[debug_handler]
async fn guide_start(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let (_model, session) = load_session_from_db(&ctx.db, &session_id).await?;

    if session.review_plan.is_some() {
        return format::render().redirect(&format!("/review/{session_id}/guide/step/1"));
    }

    let settings = match load_ai_settings() {
        Some(s) => s,
        None => {
            return format::render().view(
                &v,
                "review/_plan_error.html",
                data!({
                    "error": "No AI backend configured",
                    "session_id": session_id,
                }),
            );
        }
    };

    let context = build_ai_context(&session);
    let prompt = AiPrompt {
        context,
        instruction: ai_cli::review_plan_instruction().to_string(),
    };

    let model = settings.model_for_tier(ModelTier::Deep);
    match ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout, model).await {
        Ok(raw_response) => {
            match ai_cli::extract_json_from_response(&raw_response).and_then(|json| {
                serde_json::from_str::<ReviewPlanResponse>(&json)
                    .map_err(|e| format!("Failed to parse review plan JSON: {e}"))
            }) {
                Ok(plan_response) => {
                    let plan = ReviewPlan {
                        steps: plan_response.steps,
                        generated_at: chrono_now(),
                    };
                    let plan_json = serde_json::to_string(&plan).ok();

                    review_sessions::update_review_plan(&ctx.db, &session_id, plan_json)
                        .await
                        .map_err(|e| {
                            tracing::error!("DB error saving review plan: {e}");
                            Error::InternalServerError
                        })?;

                    if let Ok(mut file_session) = ReviewSession::load(&session_id) {
                        file_session.review_plan = Some(plan);
                        let _ = file_session.save();
                    }

                    format::render().redirect(&format!("/review/{session_id}/guide/step/1"))
                }
                Err(error) => format::render().view(
                    &v,
                    "review/_plan_error.html",
                    data!({
                        "error": error,
                        "session_id": session_id,
                    }),
                ),
            }
        }
        Err(e) => {
            log_ai_error(&e);
            format::render().view(
                &v,
                "review/_plan_error.html",
                data!({
                    "error": e.to_string(),
                    "session_id": session_id,
                }),
            )
        }
    }
}

#[derive(serde::Deserialize)]
struct ReviewPlanResponse {
    steps: Vec<ReviewStep>,
}

#[debug_handler]
async fn guide_plan_skip(
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let (_model, session) = load_session_from_db(&ctx.db, &session_id).await?;

    let plan = fallback_review_plan(&session);
    let plan_json = serde_json::to_string(&plan).ok();

    review_sessions::update_review_plan(&ctx.db, &session_id, plan_json)
        .await
        .map_err(|e| {
            tracing::error!("DB error saving fallback review plan: {e}");
            Error::InternalServerError
        })?;

    if let Ok(mut file_session) = ReviewSession::load(&session_id) {
        file_session.review_plan = Some(plan);
        let _ = file_session.save();
    }

    format::render().redirect(&format!("/review/{session_id}/guide/step/1"))
}

fn build_steps_data_for_guide(
    plan: &ReviewPlan,
    session: &ReviewSession,
) -> Vec<serde_json::Value> {
    plan.steps
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let validated = session.is_step_validated(i);
            let status = if validated {
                "Reviewed"
            } else {
                match step.status {
                    StepStatus::Planned => "Planned",
                    StepStatus::ReadyForReview => "ReadyForReview",
                    StepStatus::Reviewed => "Reviewed",
                    StepStatus::NeedsRevision => "NeedsRevision",
                }
            };
            serde_json::json!({
                "number": i + 1,
                "title": step.title,
                "file_count": step.file_refs.len(),
                "status": status,
                "is_new": step.status == StepStatus::ReadyForReview && !validated,
            })
        })
        .collect()
}

#[debug_handler]
async fn guide_page(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let (_model, mut session) = load_session_from_db(&ctx.db, &session_id).await?;

    let plan = match &session.review_plan {
        Some(plan) => plan.clone(),
        None => {
            return format::render().redirect(&format!("/review/{session_id}/summary"));
        }
    };

    session.ensure_validated_steps_size();
    let is_live = session.is_live();
    let total_steps = plan.steps.len();
    let steps_ready = session.steps_ready_count();
    let steps = build_steps_data_for_guide(&plan, &session);

    format::render().view(
        &v,
        "review/guide.html",
        data!({
            "session_id": session.id,
            "branch": session.branch,
            "default_branch": session.default_branch,
            "is_live": is_live,
            "total_steps": total_steps,
            "steps_ready": steps_ready,
            "steps": steps,
        }),
    )
}

#[debug_handler]
async fn guide_steps(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let (_model, mut session) = load_session_from_db(&ctx.db, &session_id).await?;

    let plan = match &session.review_plan {
        Some(plan) => plan.clone(),
        None => {
            return Err(Error::NotFound);
        }
    };

    session.ensure_validated_steps_size();
    let is_live = session.is_live();
    let total_steps = plan.steps.len();
    let steps_ready = session.steps_ready_count();
    let steps = build_steps_data_for_guide(&plan, &session);

    format::render().view(
        &v,
        "review/_guide_steps.html",
        data!({
            "session_id": session.id,
            "is_live": is_live,
            "total_steps": total_steps,
            "steps_ready": steps_ready,
            "steps": steps,
        }),
    )
}

const LOADING_HINTS: &[&str] = &[
    "AI is reading through your changes...",
    "Analyzing code patterns and structure...",
    "Building a mental model of your changes...",
    "Grouping related changes into review steps...",
    "Almost there — generating explanations...",
    "Understanding the impact of each change...",
    "Preparing a guided review experience...",
];

#[debug_handler]
async fn loading_page(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let (model, session) = load_session_from_db(&ctx.db, &session_id).await?;

    // Live sessions skip the loading page — plan is already available from session creation
    if session.is_live() {
        return format::render().redirect(&format!("/review/{session_id}/summary"));
    }

    let status = background_analysis::check_analysis_status(&ctx.db, model.id, &session_id).await;

    if status.summary_ready {
        return format::render().redirect(&format!("/review/{session_id}/summary"));
    }

    format::render().view(
        &v,
        "review/loading.html",
        data!({
            "session_id": session.id,
            "branch": session.branch,
            "default_branch": session.default_branch,
            "changed_files_count": session.changed_files.len(),
            "lines_added": session.metrics.lines_added,
            "lines_removed": session.metrics.lines_removed,
            "commits": session.metrics.commits_on_branch,
            "approach_ready": status.approach_ready,
            "plan_ready": status.plan_ready,
            "step_count": status.step_count,
            "steps_explained": status.steps_explained,
            "has_failures": status.has_failures,
            "failure_message": status.failure_message,
            "hint": pick_hint(),
        }),
    )
}

#[debug_handler]
async fn analysis_status(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let (model, session) = load_session_from_db(&ctx.db, &session_id).await?;

    if session.is_live() {
        return Ok(axum::response::Response::builder()
            .header("HX-Redirect", format!("/review/{session_id}/summary"))
            .body(axum::body::Body::empty())
            .unwrap()
            .into_response());
    }

    let status = background_analysis::check_analysis_status(&ctx.db, model.id, &session_id).await;

    if status.summary_ready {
        return Ok(axum::response::Response::builder()
            .header("HX-Redirect", format!("/review/{session_id}/summary"))
            .body(axum::body::Body::empty())
            .unwrap()
            .into_response());
    }

    format::render().view(
        &v,
        "review/_loading_status.html",
        data!({
            "session_id": session_id,
            "approach_ready": status.approach_ready,
            "plan_ready": status.plan_ready,
            "step_count": status.step_count,
            "steps_explained": status.steps_explained,
            "has_failures": status.has_failures,
            "failure_message": status.failure_message,
            "hint": pick_hint(),
        }),
    )
}

#[debug_handler]
async fn plan_status(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let (model, _session) = load_session_from_db(&ctx.db, &session_id).await?;
    let status = background_analysis::check_analysis_status(&ctx.db, model.id, &session_id).await;

    format::render().view(
        &v,
        "review/_plan_status.html",
        data!({
            "session_id": session_id,
            "plan_ready": status.plan_ready,
            "step_count": status.step_count,
            "has_failures": status.has_failures,
            "failure_message": status.failure_message,
        }),
    )
}

#[debug_handler]
async fn bg_hint(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let (model, _session) = load_session_from_db(&ctx.db, &session_id).await?;
    let status = background_analysis::check_analysis_status(&ctx.db, model.id, &session_id).await;

    let all_done = status.plan_ready
        && status
            .step_count
            .map(|c| status.steps_explained >= c)
            .unwrap_or(true);

    format::render().view(
        &v,
        "review/_bg_hint.html",
        data!({
            "session_id": session_id,
            "all_done": all_done,
            "step_count": status.step_count,
            "steps_explained": status.steps_explained,
            "has_failures": status.has_failures,
            "failure_message": status.failure_message,
        }),
    )
}

fn pick_hint() -> &'static str {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis();
    let idx = (millis / 2000) as usize % LOADING_HINTS.len();
    LOADING_HINTS[idx]
}

fn chrono_now() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis();
    let secs = millis / 1000;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[debug_handler]
async fn step_page(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path((session_id, step_number)): Path<(String, usize)>,
) -> Result<Response> {
    let (model, mut session) = load_session_from_db(&ctx.db, &session_id).await?;
    let session_db_id = model.id;

    let plan = match &session.review_plan {
        Some(plan) => plan.clone(),
        None => {
            return format::render().redirect(&format!("/review/{session_id}/summary"));
        }
    };

    let total_steps = plan.steps.len();
    if step_number < 1 || step_number > total_steps {
        return Err(Error::NotFound);
    }

    session.ensure_validated_steps_size();

    let step = &plan.steps[step_number - 1];
    let is_live = session.is_live();
    let step_files: Vec<String> = step.file_refs.iter().map(|f| f.path.clone()).collect();

    let step_validation = session
        .validated_steps
        .get(step_number - 1)
        .cloned()
        .unwrap_or_default();

    let file_diffs: Vec<serde_json::Value> = step
        .file_refs
        .iter()
        .map(|f| {
            let refs = vec![(f.path.clone(), f.diff_lines)];
            let diff = git_analysis::extract_diff_for_files(&session.diff, &refs);
            let validated = step_validation.is_file_validated(&f.path);
            serde_json::json!({
                "path": f.path,
                "diff": diff,
                "validated": validated,
            })
        })
        .collect();

    let step_diff = if is_live {
        step.step_diff.clone().unwrap_or_default()
    } else {
        let file_refs_tuples: Vec<(String, Option<(usize, usize)>)> = step
            .file_refs
            .iter()
            .map(|f| (f.path.clone(), f.diff_lines))
            .collect();
        git_analysis::extract_diff_for_files(&session.diff, &file_refs_tuples)
    };

    let validated_file_count = step_validation.validated_count();
    let total_file_count = step.file_refs.len();

    let steps_data: Vec<serde_json::Value> = plan
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let validated = session.is_step_validated(i);
            let status = if validated {
                "Reviewed"
            } else {
                match s.status {
                    StepStatus::Planned => "Planned",
                    StepStatus::ReadyForReview => "ReadyForReview",
                    StepStatus::Reviewed => "Reviewed",
                    StepStatus::NeedsRevision => "NeedsRevision",
                }
            };
            serde_json::json!({
                "number": i + 1,
                "title": s.title,
                "file_count": s.file_refs.len(),
                "active": i + 1 == step_number,
                "validated": validated,
                "status": status,
            })
        })
        .collect();

    let current_step_validated = session.is_step_validated(step_number - 1);

    let has_previous = step_number > 1;
    let prev_step_title = if has_previous {
        Some(plan.steps[step_number - 2].title.clone())
    } else {
        None
    };

    let db_messages = chat_messages::find_by_session(&ctx.db, session_db_id)
        .await
        .map_err(|e| {
            tracing::error!("DB error loading chat messages: {e}");
            Error::InternalServerError
        })?;

    let chat_messages_json: Vec<serde_json::Value> = db_messages
        .iter()
        .map(|msg| chat_msg_to_json(msg, Some(step_number)))
        .collect();

    let step_status = match step.status {
        StepStatus::Planned => "Planned",
        StepStatus::ReadyForReview => "ReadyForReview",
        StepStatus::Reviewed => "Reviewed",
        StepStatus::NeedsRevision => "NeedsRevision",
    };

    format::render().view(
        &v,
        "review/step.html",
        data!({
            "session_id": session.id,
            "branch": session.branch,
            "default_branch": session.default_branch,
            "step_number": step_number,
            "total_steps": total_steps,
            "step_title": step.title,
            "step_rationale": step.rationale,
            "step_files": step_files,
            "file_diffs": file_diffs,
            "steps": steps_data,
            "has_previous": has_previous,
            "prev_step_title": prev_step_title,
            "chat_messages": chat_messages_json,
            "current_step_validated": current_step_validated,
            "validated_file_count": validated_file_count,
            "total_file_count": total_file_count,
            "is_live": is_live,
            "step_diff": step_diff,
            "step_status": step_status,
        }),
    )
}

#[debug_handler]
async fn step_explanation(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path((session_id, step_number)): Path<(String, usize)>,
) -> Result<Response> {
    let (model, session) = load_session_from_db(&ctx.db, &session_id).await?;
    let session_db_id = model.id;

    let plan = session.review_plan.as_ref().ok_or(Error::NotFound)?;
    if step_number < 1 || step_number > plan.steps.len() {
        return Err(Error::NotFound);
    }

    let idx = step_number - 1;
    let step = &plan.steps[idx];
    let is_live = session.is_live();

    let cached = ai_analyses::find_cached(
        &ctx.db,
        session_db_id,
        "step_explanation",
        Some(step_number as i32),
    )
    .await
    .map_err(|e| {
        tracing::error!("DB error checking step explanation cache: {e}");
        Error::InternalServerError
    })?;

    if let Some(cached_content) = cached {
        let html_content = markdown::to_html(&cached_content);
        return format::render().view(
            &v,
            "review/_section_content.html",
            data!({"title": "Step Explanation", "content": html_content, "section_id": "step-explanation"}),
        );
    }

    // In Live mode, if the agent hasn't provided an explanation and there's no
    // cached one yet, show a "pending" message instead of trying to generate one
    // from an empty diff. The background analysis spawned by complete_step will
    // populate it eventually.
    if is_live && step.step_diff.is_none() {
        return format::render().view(
            &v,
            "review/_section_content.html",
            data!({
                "title": "Step Explanation",
                "content": "<p class=\"text-base-content/50 italic\">Waiting for agent to push this step...</p>",
                "section_id": "step-explanation",
            }),
        );
    }

    let settings = match load_ai_settings() {
        Some(s) => s,
        None => {
            return format::render().view(
                &v,
                "review/_section_error.html",
                data!({
                    "title": "Step Explanation",
                    "error": "No AI backend configured",
                    "section_id": "step-explanation",
                    "session_id": session_id,
                    "retry_url": format!("/review/{session_id}/guide/step/{step_number}/explanation"),
                    "skip_url": format!("/review/{session_id}/guide/step/{step_number}/skip/explanation"),
                }),
            );
        }
    };

    let step_diff = if is_live {
        step.step_diff.clone().unwrap_or_default()
    } else {
        let file_refs_tuples: Vec<(String, Option<(usize, usize)>)> = step
            .file_refs
            .iter()
            .map(|f| (f.path.clone(), f.diff_lines))
            .collect();
        git_analysis::extract_diff_for_files(&session.diff, &file_refs_tuples)
    };

    let context = build_ai_context(&session);
    let instruction = ai_cli::step_explanation_instruction(&step.title, &step_diff);
    let primed = build_primed_session(&session);

    match generate_with_fork_or_fallback(
        &settings,
        primed.as_ref(),
        &instruction,
        &context,
        ModelTier::Fast,
    )
    .await
    {
        Ok(content) => {
            ai_analyses::upsert(
                &ctx.db,
                session_db_id,
                "step_explanation",
                Some(step_number as i32),
                &content,
            )
            .await
            .map_err(|e| {
                tracing::error!("DB error saving step explanation: {e}");
                Error::InternalServerError
            })?;

            if let Ok(mut file_session) = ReviewSession::load(&session_id) {
                if let Some(ref mut plan) = file_session.review_plan {
                    if let Some(step) = plan.steps.get_mut(idx) {
                        step.ai_data.explanation = Some(content.clone());
                    }
                }
                let _ = file_session.save();
            }

            let html_content = markdown::to_html(&content);
            format::render().view(
                &v,
                "review/_section_content.html",
                data!({"title": "Step Explanation", "content": html_content, "section_id": "step-explanation"}),
            )
        }
        Err(e) => {
            log_ai_error(&e);
            format::render().view(
                &v,
                "review/_section_error.html",
                data!({
                    "title": "Step Explanation",
                    "error": e.to_string(),
                    "section_id": "step-explanation",
                    "session_id": session_id,
                    "retry_url": format!("/review/{session_id}/guide/step/{step_number}/explanation"),
                    "skip_url": format!("/review/{session_id}/guide/step/{step_number}/skip/explanation"),
                }),
            )
        }
    }
}

#[debug_handler]
async fn step_relation(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path((session_id, step_number)): Path<(String, usize)>,
) -> Result<Response> {
    let (model, session) = load_session_from_db(&ctx.db, &session_id).await?;
    let session_db_id = model.id;

    let plan = session.review_plan.as_ref().ok_or(Error::NotFound)?;
    if step_number < 2 || step_number > plan.steps.len() {
        return Err(Error::NotFound);
    }

    let idx = step_number - 1;
    let step = &plan.steps[idx];

    let cached = ai_analyses::find_cached(
        &ctx.db,
        session_db_id,
        "step_relation",
        Some(step_number as i32),
    )
    .await
    .map_err(|e| {
        tracing::error!("DB error checking step relation cache: {e}");
        Error::InternalServerError
    })?;

    if let Some(cached_content) = cached {
        let html_content = markdown::to_html(&cached_content);
        return format::render().view(
            &v,
            "review/_section_content.html",
            data!({"title": "Relation to Previous Step", "content": html_content, "section_id": "step-relation"}),
        );
    }

    let settings = match load_ai_settings() {
        Some(s) => s,
        None => {
            return format::render().view(
                &v,
                "review/_section_error.html",
                data!({
                    "title": "Relation to Previous Step",
                    "error": "No AI backend configured",
                    "section_id": "step-relation",
                    "session_id": session_id,
                    "retry_url": format!("/review/{session_id}/guide/step/{step_number}/relation"),
                    "skip_url": format!("/review/{session_id}/guide/step/{step_number}/skip/relation"),
                }),
            );
        }
    };

    let prev_title = &plan.steps[idx - 1].title;
    let is_live = session.is_live();
    let step_diff = if is_live {
        step.step_diff.clone().unwrap_or_default()
    } else {
        let file_refs_tuples: Vec<(String, Option<(usize, usize)>)> = step
            .file_refs
            .iter()
            .map(|f| (f.path.clone(), f.diff_lines))
            .collect();
        git_analysis::extract_diff_for_files(&session.diff, &file_refs_tuples)
    };

    let context = build_ai_context(&session);
    let instruction = ai_cli::step_relation_instruction(prev_title, &step.title, &step_diff);
    let primed = build_primed_session(&session);

    match generate_with_fork_or_fallback(
        &settings,
        primed.as_ref(),
        &instruction,
        &context,
        ModelTier::Fast,
    )
    .await
    {
        Ok(content) => {
            ai_analyses::upsert(
                &ctx.db,
                session_db_id,
                "step_relation",
                Some(step_number as i32),
                &content,
            )
            .await
            .map_err(|e| {
                tracing::error!("DB error saving step relation: {e}");
                Error::InternalServerError
            })?;

            if let Ok(mut file_session) = ReviewSession::load(&session_id) {
                if let Some(ref mut plan) = file_session.review_plan {
                    if let Some(step) = plan.steps.get_mut(idx) {
                        step.ai_data.relation_to_previous = Some(content.clone());
                    }
                }
                let _ = file_session.save();
            }

            let html_content = markdown::to_html(&content);
            format::render().view(
                &v,
                "review/_section_content.html",
                data!({"title": "Relation to Previous Step", "content": html_content, "section_id": "step-relation"}),
            )
        }
        Err(e) => {
            log_ai_error(&e);
            format::render().view(
                &v,
                "review/_section_error.html",
                data!({
                    "title": "Relation to Previous Step",
                    "error": e.to_string(),
                    "section_id": "step-relation",
                    "session_id": session_id,
                    "retry_url": format!("/review/{session_id}/guide/step/{step_number}/relation"),
                    "skip_url": format!("/review/{session_id}/guide/step/{step_number}/skip/relation"),
                }),
            )
        }
    }
}

#[debug_handler]
async fn step_chat(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path((session_id, step_number)): Path<(String, usize)>,
    Form(form): Form<ChatForm>,
) -> Result<Response> {
    let (model, mut session) = load_session_from_db(&ctx.db, &session_id).await?;
    let session_db_id = model.id;

    let plan = session.review_plan.as_ref().ok_or(Error::NotFound)?;
    if step_number < 1 || step_number > plan.steps.len() {
        return Err(Error::NotFound);
    }

    let idx = step_number - 1;
    let step = &plan.steps[idx];
    let step_title = step.title.clone();

    let explanation = ai_analyses::find_cached(
        &ctx.db,
        session_db_id,
        "step_explanation",
        Some(step_number as i32),
    )
    .await
    .map_err(|e| {
        tracing::error!("DB error loading step explanation for chat context: {e}");
        Error::InternalServerError
    })?
    .unwrap_or_default();

    let is_live = session.is_live();
    let step_diff = if is_live {
        step.step_diff.clone().unwrap_or_default()
    } else {
        let file_refs_tuples: Vec<(String, Option<(usize, usize)>)> = step
            .file_refs
            .iter()
            .map(|f| (f.path.clone(), f.diff_lines))
            .collect();
        git_analysis::extract_diff_for_files(&session.diff, &file_refs_tuples)
    };

    let user_msg_model = chat_messages::create(
        &ctx.db,
        session_db_id,
        Some(step_number as i32),
        "user",
        &form.message,
    )
    .await
    .map_err(|e| {
        tracing::error!("DB error saving user step chat message: {e}");
        Error::InternalServerError
    })?;

    let user_msg = ChatMessage {
        role: "user".to_string(),
        content: form.message.clone(),
        timestamp: chrono_now(),
        step_number: Some(step_number),
    };
    session.chat_messages.push(user_msg);

    let settings = load_ai_settings();
    let ai_response = if let Some(settings) = settings {
        let context = build_ai_context(&session);
        let instruction =
            ai_cli::step_chat_instruction(&step_title, &step_diff, &explanation, &form.message);
        let primed = build_primed_session(&session);
        match generate_with_fork_or_fallback(
            &settings,
            primed.as_ref(),
            &instruction,
            &context,
            ModelTier::Fast,
        )
        .await
        {
            Ok(response) => response,
            Err(e) => {
                log_ai_error(&e);
                format!("AI error: {e}")
            }
        }
    } else {
        "No AI backend configured. Please set up an AI backend first.".to_string()
    };

    let ai_msg_model = chat_messages::create(
        &ctx.db,
        session_db_id,
        Some(step_number as i32),
        "assistant",
        &ai_response,
    )
    .await
    .map_err(|e| {
        tracing::error!("DB error saving AI step chat message: {e}");
        Error::InternalServerError
    })?;

    let ai_msg = ChatMessage {
        role: "assistant".to_string(),
        content: ai_response,
        timestamp: chrono_now(),
        step_number: Some(step_number),
    };
    session.chat_messages.push(ai_msg);
    let _ = session.save();

    let last_two: Vec<serde_json::Value> = vec![
        chat_msg_to_json(&user_msg_model, Some(step_number)),
        chat_msg_to_json(&ai_msg_model, Some(step_number)),
    ];

    format::render().view(
        &v,
        "review/_step_chat_messages.html",
        data!({"messages": last_two, "current_step": step_number}),
    )
}

#[derive(Deserialize)]
pub struct ValidateFileForm {
    file_path: String,
}

#[derive(Deserialize)]
pub struct RevisionForm {
    block: Option<String>,
}

#[debug_handler]
async fn step_request_revision(
    State(ctx): State<AppContext>,
    Path((session_id, step_number)): Path<(String, usize)>,
    Form(form): Form<RevisionForm>,
) -> Result<Response> {
    let (_model, mut session) = load_session_from_db(&ctx.db, &session_id).await?;

    let plan = session.review_plan.as_mut().ok_or(Error::NotFound)?;
    let total_steps = plan.steps.len();
    if step_number < 1 || step_number > total_steps {
        return Err(Error::NotFound);
    }

    let idx = step_number - 1;
    let step = &plan.steps[idx];
    if step.status != StepStatus::ReadyForReview && step.status != StepStatus::Reviewed {
        return Err(Error::BadRequest(format!(
            "Step {step_number} cannot be revised (current status: {:?})",
            step.status
        )));
    }

    plan.steps[idx].status = StepStatus::NeedsRevision;
    if form.block.is_some() {
        session.block_agent = Some(true);
    }

    let plan_json = serde_json::to_string(&session.review_plan).ok();
    review_sessions::update_review_plan(&ctx.db, &session_id, plan_json)
        .await
        .map_err(|e| {
            tracing::error!("DB error saving revision request: {e}");
            Error::InternalServerError
        })?;

    let _ = session.save();

    format::render().redirect(&format!("/review/{session_id}/guide/step/{step_number}"))
}

#[debug_handler]
async fn step_validate_file(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path((session_id, step_number)): Path<(String, usize)>,
    Form(form): Form<ValidateFileForm>,
) -> Result<Response> {
    let (_model, mut session) = load_session_from_db(&ctx.db, &session_id).await?;

    let plan = match &session.review_plan {
        Some(p) => p.clone(),
        None => return Err(Error::NotFound),
    };
    let total_steps = plan.steps.len();
    if step_number < 1 || step_number > total_steps {
        return Err(Error::NotFound);
    }

    session.ensure_validated_steps_size();
    session.validated_steps[step_number - 1].validate_file(&form.file_path);

    review_sessions::update_validated_steps(&ctx.db, &session_id, &session.validated_steps)
        .await
        .map_err(|e| {
            tracing::error!("DB error saving validated steps: {e}");
            Error::InternalServerError
        })?;

    let _ = session.save();

    let current_step_validated = session.is_step_validated(step_number - 1);
    let sv = &session.validated_steps[step_number - 1];
    let validated_file_count = sv.validated_count();
    let total_file_count = plan.steps[step_number - 1].file_refs.len();

    let step = &plan.steps[step_number - 1];
    let file_diff = step
        .file_refs
        .iter()
        .find(|f| f.path == form.file_path)
        .map(|f| {
            let refs = vec![(f.path.clone(), f.diff_lines)];
            git_analysis::extract_diff_for_files(&session.diff, &refs)
        })
        .unwrap_or_default();

    format::render().view(
        &v,
        "review/_file_validated.html",
        data!({
            "session_id": session_id,
            "file_path": form.file_path,
            "file_diff": file_diff,
            "step_number": step_number,
            "total_steps": total_steps,
            "current_step_validated": current_step_validated,
            "validated_file_count": validated_file_count,
            "total_file_count": total_file_count,
        }),
    )
}

#[debug_handler]
async fn step_validate(
    State(ctx): State<AppContext>,
    Path((session_id, step_number)): Path<(String, usize)>,
) -> Result<Response> {
    let (_model, mut session) = load_session_from_db(&ctx.db, &session_id).await?;

    let plan = match &session.review_plan {
        Some(p) => p.clone(),
        None => return Err(Error::NotFound),
    };
    let total_steps = plan.steps.len();
    if step_number < 1 || step_number > total_steps {
        return Err(Error::NotFound);
    }

    session.ensure_validated_steps_size();

    let step = &plan.steps[step_number - 1];
    for file_ref in &step.file_refs {
        session.validated_steps[step_number - 1].validate_file(&file_ref.path);
    }

    review_sessions::update_validated_steps(&ctx.db, &session_id, &session.validated_steps)
        .await
        .map_err(|e| {
            tracing::error!("DB error saving validated steps: {e}");
            Error::InternalServerError
        })?;

    let _ = session.save();

    let all_validated = session.all_steps_validated();

    if all_validated {
        format::render().redirect(&format!("/review/{session_id}/summary"))
    } else if step_number < total_steps {
        format::render().redirect(&format!(
            "/review/{session_id}/guide/step/{}",
            step_number + 1
        ))
    } else {
        format::render().redirect(&format!("/review/{session_id}/guide/step/1"))
    }
}

#[debug_handler]
async fn step_section_skip(
    ViewEngine(v): ViewEngine<TeraView>,
    Path((session_id, step_number, section)): Path<(String, usize, String)>,
) -> Result<Response> {
    let title = match section.as_str() {
        "explanation" => "Step Explanation",
        "relation" => "Relation to Previous Step",
        _ => "Unknown Section",
    };

    let section_id = match section.as_str() {
        "explanation" => "step-explanation",
        "relation" => "step-relation",
        _ => &section,
    };

    format::render().view(
        &v,
        "review/_section_skipped.html",
        data!({
            "title": title,
            "section_id": section_id,
            "session_id": session_id,
            "retry_url": format!("/review/{session_id}/guide/step/{step_number}/{section}"),
        }),
    )
}

#[debug_handler]
async fn retry_analysis(
    State(ctx): State<AppContext>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let (model, session) = load_session_from_db(&ctx.db, &session_id).await?;
    background_analysis::spawn_all_analyses(ctx.db.clone(), model.id, session);

    Ok(axum::response::Response::builder()
        .header("HX-Redirect", format!("/review/{session_id}/loading"))
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response())
}

pub fn page_routes() -> Routes {
    Routes::new()
        .prefix("/review")
        .add("/{session_id}/loading", get(loading_page))
        .add("/{session_id}/status", get(analysis_status))
        .add("/{session_id}/plan-status", get(plan_status))
        .add("/{session_id}/bg-hint", get(bg_hint))
        .add("/{session_id}/retry", post(retry_analysis))
        .add("/{session_id}/summary", get(summary_page))
        .add("/{session_id}/summary/approach", get(summary_approach))
        .add("/{session_id}/summary/skip/{section}", get(section_skip))
        .add("/{session_id}/summary/chat", post(summary_chat))
        .add("/{session_id}/guide/start", post(guide_start))
        .add("/{session_id}/guide", get(guide_page))
        .add("/{session_id}/guide/steps", get(guide_steps))
        .add("/{session_id}/guide/plan/skip", post(guide_plan_skip))
        .add("/{session_id}/guide/step/{step_number}", get(step_page))
        .add(
            "/{session_id}/guide/step/{step_number}/explanation",
            get(step_explanation),
        )
        .add(
            "/{session_id}/guide/step/{step_number}/relation",
            get(step_relation),
        )
        .add(
            "/{session_id}/guide/step/{step_number}/chat",
            post(step_chat),
        )
        .add(
            "/{session_id}/guide/step/{step_number}/validate",
            post(step_validate),
        )
        .add(
            "/{session_id}/guide/step/{step_number}/validate-file",
            post(step_validate_file),
        )
        .add(
            "/{session_id}/guide/step/{step_number}/request-revision",
            post(step_request_revision),
        )
        .add(
            "/{session_id}/guide/step/{step_number}/skip/{section}",
            get(step_section_skip),
        )
}

pub fn api_routes() -> Routes {
    Routes::new().prefix("/api/review")
}
