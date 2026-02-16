use axum::extract::Path as AxumPath;
use axum::response::IntoResponse;
use axum::Form;
use loco_rs::prelude::*;
use serde::Deserialize;

use crate::models::review_sessions as rs_model;
use crate::services::{background_analysis, git_analysis, review_session::ReviewSession};

#[derive(Deserialize)]
pub struct RepoAnalyzeForm {
    path: String,
}

#[derive(Deserialize)]
pub struct FreshForm {
    repo_path: String,
    branch: String,
}

#[debug_handler]
async fn analyze_page(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let models = rs_model::Model::find_all_ordered(&ctx.db)
        .await
        .unwrap_or_default();

    let sessions: Vec<serde_json::Value> = models
        .iter()
        .map(|m| {
            let session = m.to_review_session();
            let total_steps = session
                .review_plan
                .as_ref()
                .map(|p| p.steps.len())
                .unwrap_or(0);
            let validated_count = (0..total_steps)
                .filter(|&i| session.is_step_validated(i))
                .count();
            let is_completed = total_steps > 0 && validated_count == total_steps;
            let is_live = matches!(
                session.review_mode,
                crate::services::review_session::ReviewMode::Live
            );

            let repo_name = std::path::Path::new(&session.repo_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| session.repo_path.clone());

            serde_json::json!({
                "session_id": session.id,
                "repo_path": session.repo_path,
                "repo_name": repo_name,
                "branch": session.branch,
                "is_live": is_live,
                "total_steps": total_steps,
                "validated_count": validated_count,
                "is_completed": is_completed,
                "updated_at": m.updated_at.format("%Y-%m-%d %H:%M").to_string(),
            })
        })
        .collect();

    let has_active = sessions
        .iter()
        .any(|s| !s["is_completed"].as_bool().unwrap_or(true));
    let has_completed = sessions
        .iter()
        .any(|s| s["is_completed"].as_bool().unwrap_or(false));

    format::render().view(
        &v,
        "repo/analyze.html",
        data!({
            "sessions": sessions,
            "has_active": has_active,
            "has_completed": has_completed,
        }),
    )
}

#[debug_handler]
async fn analyze_submit(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Form(form): Form<RepoAnalyzeForm>,
) -> Result<Response> {
    let repo_path = form.path.clone();

    let analysis = match tokio::task::spawn_blocking(move || {
        git_analysis::analyze_repo(std::path::Path::new(&repo_path))
    })
    .await
    {
        Ok(Ok(analysis)) => analysis,
        Ok(Err(e)) => {
            return format::render().view(
                &v,
                "repo/_error.html",
                data!({"message": e.to_string()}),
            );
        }
        Err(e) => {
            return format::render().view(
                &v,
                "repo/_error.html",
                data!({"message": format!("Analysis task failed: {e}")}),
            );
        }
    };

    if let Ok(Some(existing_model)) = rs_model::Model::find_by_repo_and_branch(
        &ctx.db,
        &analysis.repo_path,
        &analysis.current_branch,
    )
    .await
    {
        let existing = existing_model.to_review_session();
        let merge_base_changed = existing.merge_base != analysis.merge_base;

        let total_steps = existing
            .review_plan
            .as_ref()
            .map(|p| p.steps.len())
            .unwrap_or(0);
        let validated_count = (0..total_steps)
            .filter(|&i| existing.is_step_validated(i))
            .count();

        return format::render().view(
            &v,
            "repo/_resume_prompt.html",
            data!({
                "session_id": existing.id,
                "branch": existing.branch,
                "default_branch": existing.default_branch,
                "repo_path": analysis.repo_path,
                "merge_base_changed": merge_base_changed,
                "validated_count": validated_count,
                "total_steps": total_steps,
                "has_plan": existing.review_plan.is_some(),
                "created_at": existing.created_at,
            }),
        );
    }

    let session = ReviewSession::new(analysis);

    let model = match rs_model::find_or_create(&ctx.db, &session).await {
        Ok(m) => m,
        Err(e) => {
            return format::render().view(
                &v,
                "repo/_error.html",
                data!({"message": format!("Failed to save session: {e}")}),
            );
        }
    };

    background_analysis::spawn_all_analyses(ctx.db.clone(), model.id, session.clone());

    Ok(axum::response::Response::builder()
        .header("HX-Redirect", format!("/review/{}/loading", session.id))
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response())
}

#[debug_handler]
async fn resume_submit(
    State(ctx): State<AppContext>,
    AxumPath(session_id): AxumPath<String>,
) -> Result<Response> {
    let model = rs_model::Model::find_by_session_key(&ctx.db, &session_id)
        .await
        .map_err(|e| {
            tracing::error!("DB error loading session for resume {session_id}: {e}");
            Error::InternalServerError
        })?
        .ok_or_else(|| {
            tracing::error!("Session not found for resume: {session_id}");
            Error::NotFound
        })?;

    let session = model.to_review_session();

    if let Some(step) = session.first_unvalidated_step() {
        format::render().redirect(&format!("/review/{session_id}/guide/step/{step}"))
    } else if session.review_plan.is_some() {
        format::render().redirect(&format!("/review/{session_id}/guide/step/1"))
    } else {
        format::render().redirect(&format!("/review/{session_id}/summary"))
    }
}

#[debug_handler]
async fn fresh_submit(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Form(form): Form<FreshForm>,
) -> Result<Response> {
    let _ =
        rs_model::Model::delete_by_repo_and_branch(&ctx.db, &form.repo_path, &form.branch).await;

    let repo_path = form.repo_path.clone();
    let analysis = match tokio::task::spawn_blocking(move || {
        git_analysis::analyze_repo(std::path::Path::new(&repo_path))
    })
    .await
    {
        Ok(Ok(analysis)) => analysis,
        Ok(Err(e)) => {
            return format::render().view(
                &v,
                "repo/_error.html",
                data!({"message": e.to_string()}),
            );
        }
        Err(e) => {
            return format::render().view(
                &v,
                "repo/_error.html",
                data!({"message": format!("Analysis task failed: {e}")}),
            );
        }
    };

    let session = ReviewSession::new(analysis);

    let model = match rs_model::find_or_create(&ctx.db, &session).await {
        Ok(m) => m,
        Err(e) => {
            return format::render().view(
                &v,
                "repo/_error.html",
                data!({"message": format!("Failed to save session: {e}")}),
            );
        }
    };

    background_analysis::spawn_all_analyses(ctx.db.clone(), model.id, session.clone());

    Ok(axum::response::Response::builder()
        .header("HX-Redirect", format!("/review/{}/loading", session.id))
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response())
}

pub fn page_routes() -> Routes {
    Routes::new()
        .prefix("/repo")
        .add("/analyze", get(analyze_page))
        .add("/analyze", post(analyze_submit))
        .add("/resume/{session_id}", post(resume_submit))
        .add("/fresh", post(fresh_submit))
}

pub fn api_routes() -> Routes {
    Routes::new().prefix("/api/repo")
}
