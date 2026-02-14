use axum::extract::Path;
use axum::Form;
use loco_rs::prelude::*;
use serde::Deserialize;

use crate::services::{
    ai_cli::{self, AiPrompt},
    cli_detection::AiCli,
    config::SherpaConfig,
    review_session::{ChatMessage, ReviewSession},
};

fn load_selected_cli() -> Option<AiCli> {
    SherpaConfig::default_path()
        .ok()
        .and_then(|p| SherpaConfig::load(&p).ok())
        .and_then(|c| c.ai.selected_cli)
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

    let cli = load_selected_cli().ok_or("No AI backend configured")?;
    let context = build_ai_context(&session);
    let prompt = AiPrompt {
        context,
        instruction: instruction.to_string(),
    };

    let content = ai_cli::generate(cli, &prompt)
        .await
        .map_err(|e| e.to_string())?;

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
    };
    session.chat_messages.push(user_msg);

    let cli = load_selected_cli();
    let ai_response = if let Some(cli) = cli {
        let context = build_ai_context(&session);
        let prompt = AiPrompt {
            context,
            instruction: format!(
                "The user asks: {}\n\nRespond helpfully based on the code changes above.",
                form.message
            ),
        };
        match ai_cli::generate(cli, &prompt).await {
            Ok(response) => response,
            Err(e) => format!("AI error: {e}"),
        }
    } else {
        "No AI backend configured. Please set up an AI backend first.".to_string()
    };

    let ai_msg = ChatMessage {
        role: "assistant".to_string(),
        content: ai_response,
        timestamp: chrono_now(),
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

pub fn page_routes() -> Routes {
    Routes::new()
        .prefix("/review")
        .add("/{session_id}/summary", get(summary_page))
        .add("/{session_id}/summary/overview", get(summary_overview))
        .add("/{session_id}/summary/changes", get(summary_changes))
        .add("/{session_id}/summary/approach", get(summary_approach))
        .add("/{session_id}/summary/skip/{section}", get(section_skip))
        .add("/{session_id}/summary/chat", post(summary_chat))
}

pub fn api_routes() -> Routes {
    Routes::new().prefix("/api/review")
}
