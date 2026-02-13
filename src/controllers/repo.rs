use std::path::Path;

use axum::Form;
use loco_rs::prelude::*;
use serde::Deserialize;

use crate::services::{git_analysis, review_session::ReviewSession};

#[derive(Deserialize)]
pub struct RepoAnalyzeForm {
    path: String,
}

#[debug_handler]
async fn analyze_page(ViewEngine(v): ViewEngine<TeraView>) -> Result<Response> {
    format::render().view(&v, "repo/analyze.html", data!({}))
}

#[debug_handler]
async fn analyze_submit(
    ViewEngine(v): ViewEngine<TeraView>,
    Form(form): Form<RepoAnalyzeForm>,
) -> Result<Response> {
    let repo_path = form.path.clone();

    let analysis = match tokio::task::spawn_blocking(move || {
        git_analysis::analyze_repo(Path::new(&repo_path))
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
    if let Err(e) = session.save() {
        return format::render().view(
            &v,
            "repo/_error.html",
            data!({"message": format!("Failed to save session: {e}")}),
        );
    }

    let merge_base_short = if session.merge_base.len() > 8 {
        &session.merge_base[..8]
    } else {
        &session.merge_base
    };

    format::render().view(
        &v,
        "repo/_success.html",
        data!({
            "session_id": session.id,
            "branch": session.branch,
            "default_branch": session.default_branch,
            "changed_files_count": session.changed_files.len(),
            "merge_base": merge_base_short,
        }),
    )
}

pub fn page_routes() -> Routes {
    Routes::new()
        .prefix("/repo")
        .add("/analyze", get(analyze_page))
        .add("/analyze", post(analyze_submit))
}

pub fn api_routes() -> Routes {
    Routes::new().prefix("/api/repo")
}
