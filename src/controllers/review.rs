use axum::extract::Path;
use axum::Form;
use loco_rs::prelude::*;
use serde::Deserialize;

use crate::services::{
    ai_cli::{self, AiCliError, AiPrompt, DEFAULT_TIMEOUT_SECS},
    cli_detection::AiCli,
    config::SherpaConfig,
    git_analysis,
    review_session::{
        fallback_review_plan, ChatMessage, ReviewPlan, ReviewSession, ReviewStep, SymbolInfo,
    },
};

struct AiSettings {
    cli: AiCli,
    timeout: std::time::Duration,
}

fn load_ai_settings() -> Option<AiSettings> {
    let config = SherpaConfig::default_path()
        .ok()
        .and_then(|p| SherpaConfig::load(&p).ok())
        .unwrap_or_default();

    let cli = config.ai.selected_cli?;
    let timeout_secs = config.ai.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    Some(AiSettings {
        cli,
        timeout: std::time::Duration::from_secs(timeout_secs),
    })
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
    Path(session_id): Path<String>,
) -> Result<Response> {
    let session = ReviewSession::load(&session_id).map_err(|e| {
        tracing::error!("Failed to load session {session_id}: {e}");
        Error::NotFound
    })?;

    format::render().view(
        &v,
        "review/summary.html",
        data!({
            "session_id": session.id,
            "branch": session.branch,
            "default_branch": session.default_branch,
            "metrics": session.metrics,
            "chat_messages": session.chat_messages,
        }),
    )
}

async fn generate_section(
    session_id: &str,
    _section: &str,
    instruction: &str,
    get_cached: fn(&ReviewSession) -> &Option<String>,
    set_cached: fn(&mut ReviewSession, String),
) -> std::result::Result<(String, bool), String> {
    let session = ReviewSession::load(session_id)
        .map_err(|e| format!("Failed to load session: {e}"))?;

    if let Some(cached) = get_cached(&session) {
        return Ok((cached.clone(), true));
    }

    let settings = load_ai_settings().ok_or("No AI backend configured")?;
    let context = build_ai_context(&session);
    let prompt = AiPrompt {
        context,
        instruction: instruction.to_string(),
    };

    let content = ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout)
        .await
        .map_err(|e| {
            log_ai_error(&e);
            e.to_string()
        })?;

    let mut session = ReviewSession::load(session_id)
        .map_err(|e| format!("Failed to reload session: {e}"))?;
    set_cached(&mut session, content.clone());
    let _ = session.save();

    Ok((content, false))
}

#[debug_handler]
async fn summary_overview(
    ViewEngine(v): ViewEngine<TeraView>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let title = "Project Overview";
    let section_id = "overview";

    match generate_section(
        &session_id,
        section_id,
        ai_cli::overview_instruction(),
        |s| &s.summary.overview,
        |s, c| s.summary.overview = Some(c),
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
                "retry_url": format!("/review/{session_id}/summary/overview"),
            }),
        ),
    }
}

#[debug_handler]
async fn summary_changes(
    ViewEngine(v): ViewEngine<TeraView>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let title = "Change Summary";
    let section_id = "changes";

    match generate_section(
        &session_id,
        section_id,
        ai_cli::changes_instruction(),
        |s| &s.summary.changes,
        |s, c| s.summary.changes = Some(c),
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
                "retry_url": format!("/review/{session_id}/summary/changes"),
            }),
        ),
    }
}

#[debug_handler]
async fn summary_approach(
    ViewEngine(v): ViewEngine<TeraView>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let title = "Implementation Approach";
    let section_id = "approach";

    match generate_section(
        &session_id,
        section_id,
        ai_cli::approach_instruction(),
        |s| &s.summary.approach,
        |s, c| s.summary.approach = Some(c),
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
        "overview" => "Project Overview",
        "changes" => "Change Summary",
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
    Path(session_id): Path<String>,
    Form(form): Form<ChatForm>,
) -> Result<Response> {
    let mut session = ReviewSession::load(&session_id).map_err(|e| {
        tracing::error!("Failed to load session {session_id}: {e}");
        Error::NotFound
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
        let prompt = AiPrompt {
            context,
            instruction: format!(
                "The user asks: {}\n\nRespond helpfully based on the code changes above.",
                form.message
            ),
        };
        match ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout).await {
            Ok(response) => response,
            Err(e) => {
                log_ai_error(&e);
                format!("AI error: {e}")
            }
        }
    } else {
        "No AI backend configured. Please set up an AI backend first.".to_string()
    };

    let ai_msg = ChatMessage {
        role: "assistant".to_string(),
        content: ai_response,
        timestamp: chrono_now(),
        step_number: None,
    };
    session.chat_messages.push(ai_msg);
    let _ = session.save();

    let last_two: Vec<_> = session.chat_messages[session.chat_messages.len() - 2..].to_vec();

    format::render().view(
        &v,
        "review/_chat_messages.html",
        data!({"messages": last_two}),
    )
}

#[debug_handler]
async fn guide_start(
    ViewEngine(v): ViewEngine<TeraView>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let session = ReviewSession::load(&session_id).map_err(|e| {
        tracing::error!("Failed to load session {session_id}: {e}");
        Error::NotFound
    })?;

    if session.review_plan.is_some() {
        return format::render().redirect(&format!("/review/{session_id}/guide"));
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

    match ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout).await {
        Ok(raw_response) => {
            match ai_cli::extract_json_from_response(&raw_response)
                .and_then(|json| {
                    serde_json::from_str::<ReviewPlanResponse>(&json)
                        .map_err(|e| format!("Failed to parse review plan JSON: {e}"))
                })
            {
                Ok(plan_response) => {
                    let mut session = ReviewSession::load(&session_id).map_err(|e| {
                        tracing::error!("Failed to reload session: {e}");
                        Error::NotFound
                    })?;
                    session.review_plan = Some(ReviewPlan {
                        steps: plan_response.steps,
                        generated_at: chrono_now(),
                    });
                    let _ = session.save();

                    format::render().redirect(&format!("/review/{session_id}/guide"))
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
async fn guide_plan_skip(Path(session_id): Path<String>) -> Result<Response> {
    let mut session = ReviewSession::load(&session_id).map_err(|e| {
        tracing::error!("Failed to load session {session_id}: {e}");
        Error::NotFound
    })?;

    session.review_plan = Some(fallback_review_plan(&session));
    let _ = session.save();

    format::render().redirect(&format!("/review/{session_id}/guide"))
}

#[debug_handler]
async fn guide_page(
    ViewEngine(v): ViewEngine<TeraView>,
    Path(session_id): Path<String>,
) -> Result<Response> {
    let session = ReviewSession::load(&session_id).map_err(|e| {
        tracing::error!("Failed to load session {session_id}: {e}");
        Error::NotFound
    })?;

    let plan = match &session.review_plan {
        Some(plan) => plan.clone(),
        None => {
            return format::render().redirect(&format!("/review/{session_id}/summary"));
        }
    };

    let mut session = session;
    session.ensure_validated_steps_size();

    let steps_data: Vec<serde_json::Value> = plan
        .steps
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let files: Vec<String> = step.file_refs.iter().map(|f| f.path.clone()).collect();
            let validated = session.validated_steps.get(i).copied().unwrap_or(false);
            serde_json::json!({
                "number": i + 1,
                "title": step.title,
                "rationale": step.rationale,
                "file_count": step.file_refs.len(),
                "files": files,
                "validated": validated,
            })
        })
        .collect();

    let validated_count = session.validated_steps.iter().filter(|&&v| v).count();

    format::render().view(
        &v,
        "review/guide.html",
        data!({
            "session_id": session.id,
            "branch": session.branch,
            "default_branch": session.default_branch,
            "steps": steps_data,
            "total_steps": plan.steps.len(),
            "validated_count": validated_count,
        }),
    )
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
    Path((session_id, step_number)): Path<(String, usize)>,
) -> Result<Response> {
    let session = ReviewSession::load(&session_id).map_err(|e| {
        tracing::error!("Failed to load session {session_id}: {e}");
        Error::NotFound
    })?;

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

    let mut session = session;
    session.ensure_validated_steps_size();

    let step = &plan.steps[step_number - 1];
    let file_refs_tuples: Vec<(String, Option<(usize, usize)>)> = step
        .file_refs
        .iter()
        .map(|f| (f.path.clone(), f.diff_lines))
        .collect();
    let step_diff = git_analysis::extract_diff_for_files(&session.diff, &file_refs_tuples);

    let step_files: Vec<String> = step.file_refs.iter().map(|f| f.path.clone()).collect();

    let steps_data: Vec<serde_json::Value> = plan
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let validated = session.validated_steps.get(i).copied().unwrap_or(false);
            serde_json::json!({
                "number": i + 1,
                "title": s.title,
                "file_count": s.file_refs.len(),
                "active": i + 1 == step_number,
                "validated": validated,
            })
        })
        .collect();

    let current_step_validated = session
        .validated_steps
        .get(step_number - 1)
        .copied()
        .unwrap_or(false);

    let has_previous = step_number > 1;
    let prev_step_title = if has_previous {
        Some(plan.steps[step_number - 2].title.clone())
    } else {
        None
    };

    let chat_messages: Vec<serde_json::Value> = session
        .chat_messages
        .iter()
        .map(|msg| {
            serde_json::json!({
                "role": msg.role,
                "content": msg.content,
                "timestamp": msg.timestamp,
                "step_number": msg.step_number,
                "is_current_step": msg.step_number == Some(step_number),
            })
        })
        .collect();

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
            "step_diff": step_diff,
            "steps": steps_data,
            "has_previous": has_previous,
            "prev_step_title": prev_step_title,
            "chat_messages": chat_messages,
            "current_step_validated": current_step_validated,
        }),
    )
}

#[debug_handler]
async fn step_explanation(
    ViewEngine(v): ViewEngine<TeraView>,
    Path((session_id, step_number)): Path<(String, usize)>,
) -> Result<Response> {
    let session = ReviewSession::load(&session_id).map_err(|e| {
        tracing::error!("Failed to load session {session_id}: {e}");
        Error::NotFound
    })?;

    let plan = session.review_plan.as_ref().ok_or(Error::NotFound)?;
    if step_number < 1 || step_number > plan.steps.len() {
        return Err(Error::NotFound);
    }

    let idx = step_number - 1;
    let step = &plan.steps[idx];

    if let Some(ref cached) = step.ai_data.explanation {
        return format::render().view(
            &v,
            "review/_section_content.html",
            data!({"title": "Step Explanation", "content": cached, "section_id": "step-explanation"}),
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

    let file_refs_tuples: Vec<(String, Option<(usize, usize)>)> = step
        .file_refs
        .iter()
        .map(|f| (f.path.clone(), f.diff_lines))
        .collect();
    let step_diff = git_analysis::extract_diff_for_files(&session.diff, &file_refs_tuples);

    let context = build_ai_context(&session);
    let prompt = AiPrompt {
        context,
        instruction: ai_cli::step_explanation_instruction(&step.title, &step_diff),
    };

    match ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout).await {
        Ok(content) => {
            let mut session = ReviewSession::load(&session_id).map_err(|e| {
                tracing::error!("Failed to reload session: {e}");
                Error::NotFound
            })?;
            session.review_plan.as_mut().unwrap().steps[idx]
                .ai_data
                .explanation = Some(content.clone());
            let _ = session.save();

            format::render().view(
                &v,
                "review/_section_content.html",
                data!({"title": "Step Explanation", "content": content, "section_id": "step-explanation"}),
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
    Path((session_id, step_number)): Path<(String, usize)>,
) -> Result<Response> {
    let session = ReviewSession::load(&session_id).map_err(|e| {
        tracing::error!("Failed to load session {session_id}: {e}");
        Error::NotFound
    })?;

    let plan = session.review_plan.as_ref().ok_or(Error::NotFound)?;
    if step_number < 2 || step_number > plan.steps.len() {
        return Err(Error::NotFound);
    }

    let idx = step_number - 1;
    let step = &plan.steps[idx];

    if let Some(ref cached) = step.ai_data.relation_to_previous {
        return format::render().view(
            &v,
            "review/_section_content.html",
            data!({"title": "Relation to Previous Step", "content": cached, "section_id": "step-relation"}),
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
    let file_refs_tuples: Vec<(String, Option<(usize, usize)>)> = step
        .file_refs
        .iter()
        .map(|f| (f.path.clone(), f.diff_lines))
        .collect();
    let step_diff = git_analysis::extract_diff_for_files(&session.diff, &file_refs_tuples);

    let context = build_ai_context(&session);
    let prompt = AiPrompt {
        context,
        instruction: ai_cli::step_relation_instruction(prev_title, &step.title, &step_diff),
    };

    match ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout).await {
        Ok(content) => {
            let mut session = ReviewSession::load(&session_id).map_err(|e| {
                tracing::error!("Failed to reload session: {e}");
                Error::NotFound
            })?;
            session.review_plan.as_mut().unwrap().steps[idx]
                .ai_data
                .relation_to_previous = Some(content.clone());
            let _ = session.save();

            format::render().view(
                &v,
                "review/_section_content.html",
                data!({"title": "Relation to Previous Step", "content": content, "section_id": "step-relation"}),
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
async fn step_symbols(
    ViewEngine(v): ViewEngine<TeraView>,
    Path((session_id, step_number)): Path<(String, usize)>,
) -> Result<Response> {
    let session = ReviewSession::load(&session_id).map_err(|e| {
        tracing::error!("Failed to load session {session_id}: {e}");
        Error::NotFound
    })?;

    let plan = session.review_plan.as_ref().ok_or(Error::NotFound)?;
    if step_number < 1 || step_number > plan.steps.len() {
        return Err(Error::NotFound);
    }

    let idx = step_number - 1;
    let step = &plan.steps[idx];

    if let Some(ref cached) = step.ai_data.symbols {
        return format::render().view(
            &v,
            "review/_step_symbols.html",
            data!({"symbols": cached}),
        );
    }

    let settings = match load_ai_settings() {
        Some(s) => s,
        None => {
            return format::render().view(
                &v,
                "review/_section_error.html",
                data!({
                    "title": "Changed Symbols",
                    "error": "No AI backend configured",
                    "section_id": "step-symbols",
                    "session_id": session_id,
                    "retry_url": format!("/review/{session_id}/guide/step/{step_number}/symbols"),
                    "skip_url": format!("/review/{session_id}/guide/step/{step_number}/skip/symbols"),
                }),
            );
        }
    };

    let file_refs_tuples: Vec<(String, Option<(usize, usize)>)> = step
        .file_refs
        .iter()
        .map(|f| (f.path.clone(), f.diff_lines))
        .collect();
    let step_diff = git_analysis::extract_diff_for_files(&session.diff, &file_refs_tuples);

    let context = build_ai_context(&session);
    let prompt = AiPrompt {
        context,
        instruction: ai_cli::step_symbols_instruction(&step_diff),
    };

    match ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout).await {
        Ok(raw) => {
            match ai_cli::extract_json_from_response(&raw)
                .and_then(|json| {
                    serde_json::from_str::<Vec<SymbolInfo>>(&json)
                        .map_err(|e| format!("Failed to parse symbols JSON: {e}"))
                })
            {
                Ok(symbols) => {
                    let mut session = ReviewSession::load(&session_id).map_err(|e| {
                        tracing::error!("Failed to reload session: {e}");
                        Error::NotFound
                    })?;
                    session.review_plan.as_mut().unwrap().steps[idx]
                        .ai_data
                        .symbols = Some(symbols.clone());
                    let _ = session.save();

                    format::render().view(
                        &v,
                        "review/_step_symbols.html",
                        data!({"symbols": symbols}),
                    )
                }
                Err(error) => format::render().view(
                    &v,
                    "review/_section_error.html",
                    data!({
                        "title": "Changed Symbols",
                        "error": error,
                        "section_id": "step-symbols",
                        "session_id": session_id,
                        "retry_url": format!("/review/{session_id}/guide/step/{step_number}/symbols"),
                        "skip_url": format!("/review/{session_id}/guide/step/{step_number}/skip/symbols"),
                    }),
                ),
            }
        }
        Err(e) => {
            log_ai_error(&e);
            format::render().view(
                &v,
                "review/_section_error.html",
                data!({
                    "title": "Changed Symbols",
                    "error": e.to_string(),
                    "section_id": "step-symbols",
                    "session_id": session_id,
                    "retry_url": format!("/review/{session_id}/guide/step/{step_number}/symbols"),
                    "skip_url": format!("/review/{session_id}/guide/step/{step_number}/skip/symbols"),
                }),
            )
        }
    }
}

#[debug_handler]
async fn step_chat(
    ViewEngine(v): ViewEngine<TeraView>,
    Path((session_id, step_number)): Path<(String, usize)>,
    Form(form): Form<ChatForm>,
) -> Result<Response> {
    let mut session = ReviewSession::load(&session_id).map_err(|e| {
        tracing::error!("Failed to load session {session_id}: {e}");
        Error::NotFound
    })?;

    let plan = session.review_plan.as_ref().ok_or(Error::NotFound)?;
    if step_number < 1 || step_number > plan.steps.len() {
        return Err(Error::NotFound);
    }

    let idx = step_number - 1;
    let step = &plan.steps[idx];
    let step_title = step.title.clone();
    let explanation = step
        .ai_data
        .explanation
        .clone()
        .unwrap_or_default();

    let file_refs_tuples: Vec<(String, Option<(usize, usize)>)> = step
        .file_refs
        .iter()
        .map(|f| (f.path.clone(), f.diff_lines))
        .collect();
    let step_diff = git_analysis::extract_diff_for_files(&session.diff, &file_refs_tuples);

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
        let prompt = AiPrompt {
            context,
            instruction,
        };
        match ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout).await {
            Ok(response) => response,
            Err(e) => {
                log_ai_error(&e);
                format!("AI error: {e}")
            }
        }
    } else {
        "No AI backend configured. Please set up an AI backend first.".to_string()
    };

    let ai_msg = ChatMessage {
        role: "assistant".to_string(),
        content: ai_response,
        timestamp: chrono_now(),
        step_number: Some(step_number),
    };
    session.chat_messages.push(ai_msg);
    let _ = session.save();

    let last_two: Vec<_> = session.chat_messages[session.chat_messages.len() - 2..].to_vec();

    format::render().view(
        &v,
        "review/_step_chat_messages.html",
        data!({"messages": last_two, "current_step": step_number}),
    )
}

#[debug_handler]
async fn step_validate(
    Path((session_id, step_number)): Path<(String, usize)>,
) -> Result<Response> {
    let mut session = ReviewSession::load(&session_id).map_err(|e| {
        tracing::error!("Failed to load session {session_id}: {e}");
        Error::NotFound
    })?;

    let plan = session.review_plan.as_ref().ok_or(Error::NotFound)?;
    let total_steps = plan.steps.len();
    if step_number < 1 || step_number > total_steps {
        return Err(Error::NotFound);
    }

    session.ensure_validated_steps_size();
    session.validated_steps[step_number - 1] = true;
    let _ = session.save();

    if step_number < total_steps {
        format::render().redirect(&format!("/review/{session_id}/guide/step/{}", step_number + 1))
    } else {
        format::render().redirect(&format!("/review/{session_id}/guide"))
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
        "symbols" => "Changed Symbols",
        _ => "Unknown Section",
    };

    let section_id = match section.as_str() {
        "explanation" => "step-explanation",
        "relation" => "step-relation",
        "symbols" => "step-symbols",
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

pub fn page_routes() -> Routes {
    Routes::new()
        .prefix("/review")
        .add("/{session_id}/summary", get(summary_page))
        .add("/{session_id}/summary/overview", get(summary_overview))
        .add("/{session_id}/summary/changes", get(summary_changes))
        .add("/{session_id}/summary/approach", get(summary_approach))
        .add("/{session_id}/summary/skip/{section}", get(section_skip))
        .add("/{session_id}/summary/chat", post(summary_chat))
        .add("/{session_id}/guide/start", post(guide_start))
        .add("/{session_id}/guide/plan/skip", post(guide_plan_skip))
        .add("/{session_id}/guide/step/{step_number}", get(step_page))
        .add("/{session_id}/guide/step/{step_number}/explanation", get(step_explanation))
        .add("/{session_id}/guide/step/{step_number}/relation", get(step_relation))
        .add("/{session_id}/guide/step/{step_number}/symbols", get(step_symbols))
        .add("/{session_id}/guide/step/{step_number}/chat", post(step_chat))
        .add("/{session_id}/guide/step/{step_number}/validate", post(step_validate))
        .add("/{session_id}/guide/step/{step_number}/skip/{section}", get(step_section_skip))
        .add("/{session_id}/guide", get(guide_page))
}

pub fn api_routes() -> Routes {
    Routes::new().prefix("/api/review")
}
