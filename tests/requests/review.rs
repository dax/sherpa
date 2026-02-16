use loco_rs::testing::prelude::*;
use sea_orm::DatabaseConnection;
use serial_test::serial;
use sherpa::app::App;
use sherpa::models::ai_analyses;
use sherpa::models::chat_messages as cm_model;
use sherpa::models::review_sessions as rs_model;
use sherpa::services::git_analysis::{ChangedFile, FileStatus, GitAnalysis};
use sherpa::services::review_session::{
    FileRef, ReviewPlan, ReviewSession, ReviewStep, StepAiData, StepStatus, StepValidation,
};

fn make_test_session() -> ReviewSession {
    let analysis = GitAnalysis {
        repo_path: "/tmp/test-repo".to_string(),
        current_branch: "feature-test".to_string(),
        default_branch: "main".to_string(),
        merge_base: "abc123".to_string(),
        diff: "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n+new line\n+another\n-removed"
            .to_string(),
        changed_files: vec![
            ChangedFile {
                path: "src/lib.rs".to_string(),
                status: FileStatus::Modified,
            },
            ChangedFile {
                path: "src/new.rs".to_string(),
                status: FileStatus::Added,
            },
        ],
        commit_count: 5,
    };
    ReviewSession::new(analysis)
}

async fn ensure_db_session(db: &DatabaseConnection, session: &ReviewSession) {
    rs_model::find_or_create(db, session)
        .await
        .expect("insert test session into DB");
}

async fn sync_session_to_db(db: &DatabaseConnection, session: &ReviewSession) {
    rs_model::find_or_create(db, session)
        .await
        .expect("insert test session into DB");
    if let Some(ref plan) = session.review_plan {
        let plan_json = serde_json::to_string(plan).ok();
        rs_model::update_review_plan(db, &session.id, plan_json)
            .await
            .expect("update review plan in DB");
    }
    if !session.validated_steps.is_empty() {
        rs_model::update_validated_steps(db, &session.id, &session.validated_steps)
            .await
            .expect("update validated steps in DB");
    }
}

fn cleanup_session(_id: &str) {
    // File-based storage removed; DB cleanup handled by test framework
}

#[tokio::test]
#[serial]
async fn can_get_summary_page() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_test_session();
        let session_id = session.id.clone();
        ensure_db_session(&ctx.db, &session).await;

        let res = request.get(&format!("/review/{session_id}/summary")).await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("Review Summary"),
            "page should contain heading"
        );
        assert!(
            body.contains("feature-test"),
            "page should show branch name"
        );
        assert!(
            body.contains("Metrics"),
            "page should contain metrics section"
        );
        assert!(
            body.contains("hx-get"),
            "page should have HTMX triggers for AI sections"
        );
        assert!(body.contains("Chat"), "page should contain chat section");

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn summary_page_shows_correct_metrics() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_test_session();
        let session_id = session.id.clone();
        ensure_db_session(&ctx.db, &session).await;

        let res = request.get(&format!("/review/{session_id}/summary")).await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(body.contains("2"), "should show 2 files changed");
        assert!(body.contains("5"), "should show 5 commits");

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn summary_page_returns_404_for_invalid_session() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.get("/review/nonexistent-session/summary").await;

        assert_eq!(res.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_skip_section() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_test_session();
        let session_id = session.id.clone();
        ensure_db_session(&ctx.db, &session).await;

        let res = request
            .get(&format!("/review/{session_id}/summary/skip/approach"))
            .await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("AI analysis skipped"),
            "should show skipped message"
        );
        assert!(
            body.contains("Implementation Approach"),
            "should show section title"
        );
        assert!(body.contains("retry"), "should have retry option");

        cleanup_session(&session_id);
    })
    .await;
}

fn make_session_with_plan() -> ReviewSession {
    let mut session = make_test_session();
    session.review_plan = Some(ReviewPlan {
        steps: vec![
            ReviewStep {
                title: "Core data models".to_string(),
                rationale: "Foundation types used everywhere".to_string(),
                file_refs: vec![FileRef {
                    path: "src/lib.rs".to_string(),
                    diff_lines: None,
                }],
                ai_data: StepAiData::default(),
                status: StepStatus::default(),
                step_diff: None,
            },
            ReviewStep {
                title: "New feature code".to_string(),
                rationale: "The main feature implementation".to_string(),
                file_refs: vec![FileRef {
                    path: "src/new.rs".to_string(),
                    diff_lines: Some((1, 10)),
                }],
                ai_data: StepAiData::default(),
                status: StepStatus::default(),
                step_diff: None,
            },
        ],
        generated_at: "12:00:00".to_string(),
    });
    session
}

#[tokio::test]
#[serial]
async fn can_skip_plan_generation() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_test_session();
        let session_id = session.id.clone();
        ensure_db_session(&ctx.db, &session).await;

        let res = request
            .post(&format!("/review/{session_id}/guide/plan/skip"))
            .await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect to guide page, got {status}"
        );

        let model = rs_model::Model::find_by_session_key(&ctx.db, &session_id)
            .await
            .unwrap()
            .unwrap();
        let loaded = model.to_review_session();
        assert!(loaded.review_plan.is_some(), "should have a review plan");
        let plan = loaded.review_plan.unwrap();
        assert_eq!(
            plan.steps.len(),
            2,
            "fallback should create one step per file"
        );
        assert!(plan.steps[0].title.contains("src/lib.rs"));
        assert!(plan.steps[1].title.contains("src/new.rs"));

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn summary_page_has_plan_status_polling() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_test_session();
        let session_id = session.id.clone();
        ensure_db_session(&ctx.db, &session).await;

        let res = request.get(&format!("/review/{session_id}/summary")).await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("plan-status"),
            "should have plan-status polling endpoint"
        );
        assert!(
            body.contains("Preparing review plan"),
            "should show plan preparation message"
        );
        assert!(
            body.contains("hx-get"),
            "plan status should use HTMX polling"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn success_page_links_to_summary() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.get("/repo/analyze").await;
        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            !body.contains("coming soon"),
            "Continue button should no longer say coming soon"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_step_page() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let res = request
            .get(&format!("/review/{session_id}/guide/step/1"))
            .await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(body.contains("Step 1 of 2"), "should show step number");
        assert!(body.contains("Core data models"), "should show step title");
        assert!(
            body.contains("Foundation types"),
            "should show step rationale"
        );
        assert!(body.contains("lib.rs"), "should show file reference");
        assert!(
            body.contains("diff-file-container"),
            "should have per-file diff containers"
        );
        assert!(
            body.contains("hx-get"),
            "should have HTMX triggers for AI sections"
        );
        assert!(
            body.contains("/explanation"),
            "should have explanation section trigger"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_page_shows_navigation() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let res = request
            .get(&format!("/review/{session_id}/guide/step/1"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("Validate"),
            "step 1 should have Validate & Next button"
        );
        assert!(
            !body.contains("prev_step_title"),
            "step 1 should not show previous step"
        );

        let res = request
            .get(&format!("/review/{session_id}/guide/step/2"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("Core data models"),
            "step 2 should show previous step title"
        );
        assert!(
            body.contains("/relation"),
            "step 2 should have relation section trigger"
        );
        assert!(
            body.contains("Back to Summary"),
            "last step should have Back to Summary button"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_page_returns_404_for_invalid_step() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let res = request
            .get(&format!("/review/{session_id}/guide/step/99"))
            .await;

        assert_eq!(res.status_code(), 404);

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_page_returns_404_for_invalid_session() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get("/review/nonexistent-session/guide/step/1")
            .await;

        assert_eq!(res.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_page_redirects_without_plan() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_test_session();
        let session_id = session.id.clone();
        ensure_db_session(&ctx.db, &session).await;

        let res = request
            .get(&format!("/review/{session_id}/guide/step/1"))
            .await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect or show summary, got {status}"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_skip_step_section() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let res = request
            .get(&format!(
                "/review/{session_id}/guide/step/1/skip/explanation"
            ))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("AI analysis skipped"),
            "should show skipped message"
        );
        assert!(
            body.contains("Step Explanation"),
            "should show section title"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_page_sidebar_highlights_current_step() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let res = request
            .get(&format!("/review/{session_id}/guide/step/1"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("step-primary"),
            "should highlight current step in sidebar"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_page_has_chat_panel() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let res = request
            .get(&format!("/review/{session_id}/guide/step/1"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(body.contains("Chat"), "should have chat section heading");
        assert!(
            body.contains("step-chat-history"),
            "should have chat history container"
        );
        assert!(
            body.contains("/guide/step/1/chat"),
            "should have chat form posting to step chat endpoint"
        );
        assert!(
            body.contains("Ask about this step"),
            "should have step-specific placeholder text"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_page_shows_chat_history_with_highlighting() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let model = rs_model::Model::find_by_session_key(&ctx.db, &session_id)
            .await
            .unwrap()
            .unwrap();
        let session_db_id = model.id;

        cm_model::create(
            &ctx.db,
            session_db_id,
            Some(1),
            "user",
            "Question about step 1",
        )
        .await
        .unwrap();
        cm_model::create(
            &ctx.db,
            session_db_id,
            Some(1),
            "assistant",
            "Answer about step 1",
        )
        .await
        .unwrap();
        cm_model::create(
            &ctx.db,
            session_db_id,
            None,
            "user",
            "Question from summary",
        )
        .await
        .unwrap();

        let res = request
            .get(&format!("/review/{session_id}/guide/step/1"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("Question about step 1"),
            "should show step 1 chat messages"
        );
        assert!(
            body.contains("Answer about step 1"),
            "should show AI response for step 1"
        );
        assert!(
            body.contains("Question from summary"),
            "should show messages from other contexts"
        );
        assert!(
            body.contains("opacity-50"),
            "non-current-step messages should be dimmed"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_chat_returns_404_for_invalid_step() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let res = request
            .post(&format!("/review/{session_id}/guide/step/99/chat"))
            .form(&[("message", "hello")])
            .await;

        assert_eq!(res.status_code(), 404);

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_chat_returns_404_for_invalid_session() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .post("/review/nonexistent-session/guide/step/1/chat")
            .form(&[("message", "hello")])
            .await;

        assert_eq!(res.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_validate_step() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let res = request
            .post(&format!("/review/{session_id}/guide/step/1/validate"))
            .await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect to next step, got {status}"
        );

        let model = rs_model::Model::find_by_session_key(&ctx.db, &session_id)
            .await
            .unwrap()
            .unwrap();
        let loaded = model.to_review_session();
        assert_eq!(loaded.validated_steps.len(), 2);
        assert!(loaded.is_step_validated(0), "step 1 should be validated");
        assert!(
            !loaded.is_step_validated(1),
            "step 2 should not be validated"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn validate_last_step_redirects_to_guide() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let res = request
            .post(&format!("/review/{session_id}/guide/step/2/validate"))
            .await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect to guide page, got {status}"
        );

        let model = rs_model::Model::find_by_session_key(&ctx.db, &session_id)
            .await
            .unwrap()
            .unwrap();
        let loaded = model.to_review_session();
        assert!(loaded.is_step_validated(1), "step 2 should be validated");

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn validate_returns_404_for_invalid_step() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let res = request
            .post(&format!("/review/{session_id}/guide/step/99/validate"))
            .await;

        assert_eq!(res.status_code(), 404);

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn validate_returns_404_for_invalid_session() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .post("/review/nonexistent-session/guide/step/1/validate")
            .await;

        assert_eq!(res.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_page_shows_validate_button() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let res = request
            .get(&format!("/review/{session_id}/guide/step/1"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(body.contains("Validate"), "should have Validate button");
        assert!(
            body.contains("/validate"),
            "should post to validate endpoint"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn validated_step_shows_checkmark_in_sidebar() {
    request::<App, _, _>(|request, ctx| async move {
        let mut session = make_session_with_plan();
        let mut sv_validated = StepValidation::default();
        sv_validated.validate_file("src/lib.rs");
        session.validated_steps = vec![sv_validated, StepValidation::default()];
        sync_session_to_db(&ctx.db, &session).await;
        let session_id = session.id.clone();

        let res = request
            .get(&format!("/review/{session_id}/guide/step/2"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("step-success"),
            "validated step should have step-success class in sidebar"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn previous_button_does_not_unvalidate() {
    request::<App, _, _>(|request, ctx| async move {
        let mut session = make_session_with_plan();
        let mut sv_validated = StepValidation::default();
        sv_validated.validate_file("src/lib.rs");
        session.validated_steps = vec![sv_validated, StepValidation::default()];
        sync_session_to_db(&ctx.db, &session).await;
        let session_id = session.id.clone();

        let res = request
            .get(&format!("/review/{session_id}/guide/step/1"))
            .await;

        assert_eq!(res.status_code(), 200);

        let model = rs_model::Model::find_by_session_key(&ctx.db, &session_id)
            .await
            .unwrap()
            .unwrap();
        let loaded = model.to_review_session();
        assert!(
            loaded.is_step_validated(0),
            "revisiting step 1 should not unvalidate it"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn validation_state_persists_across_loads() {
    request::<App, _, _>(|_request, ctx| async move {
        let mut session = make_session_with_plan();
        let mut sv1 = StepValidation::default();
        sv1.validate_file("src/lib.rs");
        let mut sv2 = StepValidation::default();
        sv2.validate_file("src/new.rs");
        session.validated_steps = vec![sv1, sv2];
        sync_session_to_db(&ctx.db, &session).await;
        let session_id = session.id.clone();

        let model = rs_model::Model::find_by_session_key(&ctx.db, &session_id)
            .await
            .unwrap()
            .unwrap();
        let loaded = model.to_review_session();
        assert!(loaded.is_step_validated(0));
        assert!(loaded.is_step_validated(1));

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn validate_all_steps_redirects_to_summary() {
    request::<App, _, _>(|request, ctx| async move {
        let mut session = make_session_with_plan();
        let mut sv_validated = StepValidation::default();
        sv_validated.validate_file("src/lib.rs");
        session.validated_steps = vec![sv_validated, StepValidation::default()];
        sync_session_to_db(&ctx.db, &session).await;
        let session_id = session.id.clone();

        let res = request
            .post(&format!("/review/{session_id}/guide/step/2/validate"))
            .await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect to summary page, got {status}"
        );

        let model = rs_model::Model::find_by_session_key(&ctx.db, &session_id)
            .await
            .unwrap()
            .unwrap();
        let loaded = model.to_review_session();
        assert!(
            loaded.is_step_validated(0),
            "step 1 should still be validated"
        );
        assert!(loaded.is_step_validated(1), "step 2 should be validated");

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn summary_page_shows_review_complete_banner() {
    request::<App, _, _>(|request, ctx| async move {
        let mut session = make_session_with_plan();
        let mut sv1 = StepValidation::default();
        sv1.validate_file("src/lib.rs");
        let mut sv2 = StepValidation::default();
        sv2.validate_file("src/new.rs");
        session.validated_steps = vec![sv1, sv2];
        sync_session_to_db(&ctx.db, &session).await;
        let session_id = session.id.clone();

        let res = request.get(&format!("/review/{session_id}/summary")).await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("Review Complete"),
            "should show Review Complete banner"
        );
        assert!(
            body.contains("all changes reviewed"),
            "banner should confirm all changes reviewed"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn summary_page_shows_reviewed_changes_section() {
    request::<App, _, _>(|request, ctx| async move {
        let mut session = make_session_with_plan();
        let mut sv1 = StepValidation::default();
        sv1.validate_file("src/lib.rs");
        let mut sv2 = StepValidation::default();
        sv2.validate_file("src/new.rs");
        session.validated_steps = vec![sv1, sv2];
        sync_session_to_db(&ctx.db, &session).await;
        let session_id = session.id.clone();

        let res = request.get(&format!("/review/{session_id}/summary")).await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("Reviewed Changes"),
            "should show Reviewed Changes section"
        );
        assert!(
            body.contains("Core data models"),
            "should show step 1 title"
        );
        assert!(
            body.contains("New feature code"),
            "should show step 2 title"
        );
        assert!(body.contains("collapse"), "steps should be collapsible");
        assert!(body.contains("lib.rs"), "should show file references");

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn summary_page_hides_start_review_when_complete() {
    request::<App, _, _>(|request, ctx| async move {
        let mut session = make_session_with_plan();
        let mut sv1 = StepValidation::default();
        sv1.validate_file("src/lib.rs");
        let mut sv2 = StepValidation::default();
        sv2.validate_file("src/new.rs");
        session.validated_steps = vec![sv1, sv2];
        sync_session_to_db(&ctx.db, &session).await;
        let session_id = session.id.clone();

        let res = request.get(&format!("/review/{session_id}/summary")).await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            !body.contains("Start Review"),
            "should not show Start Review when review is complete"
        );
        assert!(
            body.contains("View Review Steps"),
            "should show link to review steps instead"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn summary_page_no_reviewed_changes_when_incomplete() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_test_session();
        let session_id = session.id.clone();
        ensure_db_session(&ctx.db, &session).await;

        let res = request.get(&format!("/review/{session_id}/summary")).await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            !body.contains("Review Complete"),
            "should not show Review Complete when review is not done"
        );
        assert!(
            !body.contains("Reviewed Changes"),
            "should not show Reviewed Changes when review is not done"
        );
        assert!(
            body.contains("plan-status"),
            "should show plan-status polling for review"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn summary_page_shows_step_chat_messages() {
    request::<App, _, _>(|request, ctx| async move {
        let mut session = make_session_with_plan();
        let mut sv1 = StepValidation::default();
        sv1.validate_file("src/lib.rs");
        let mut sv2 = StepValidation::default();
        sv2.validate_file("src/new.rs");
        session.validated_steps = vec![sv1, sv2];
        sync_session_to_db(&ctx.db, &session).await;
        let session_id = session.id.clone();

        let model = rs_model::Model::find_by_session_key(&ctx.db, &session_id)
            .await
            .unwrap()
            .unwrap();
        let session_db_id = model.id;

        cm_model::create(
            &ctx.db,
            session_db_id,
            Some(1),
            "user",
            "What does this change do?",
        )
        .await
        .unwrap();
        cm_model::create(
            &ctx.db,
            session_db_id,
            Some(1),
            "assistant",
            "It modifies the core models.",
        )
        .await
        .unwrap();

        let res = request.get(&format!("/review/{session_id}/summary")).await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("What does this change do?"),
            "should show step chat user message"
        );
        assert!(
            body.contains("It modifies the core models."),
            "should show step chat AI response"
        );
        assert!(
            body.contains("Chat Messages"),
            "should show Chat Messages heading in step card"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn resume_endpoint_redirects_to_first_unvalidated_step() {
    request::<App, _, _>(|request, ctx| async move {
        let mut session = make_session_with_plan();
        let mut sv_validated = StepValidation::default();
        sv_validated.validate_file("src/lib.rs");
        session.validated_steps = vec![sv_validated, StepValidation::default()];
        sync_session_to_db(&ctx.db, &session).await;
        let session_id = session.id.clone();

        let res = request.post(&format!("/repo/resume/{session_id}")).await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect, got {status}"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn resume_endpoint_redirects_to_guide_when_all_validated() {
    request::<App, _, _>(|request, ctx| async move {
        let mut session = make_session_with_plan();
        let mut sv1 = StepValidation::default();
        sv1.validate_file("src/lib.rs");
        let mut sv2 = StepValidation::default();
        sv2.validate_file("src/new.rs");
        session.validated_steps = vec![sv1, sv2];
        sync_session_to_db(&ctx.db, &session).await;
        let session_id = session.id.clone();

        let res = request.post(&format!("/repo/resume/{session_id}")).await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect, got {status}"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn resume_endpoint_redirects_to_summary_without_plan() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_test_session();
        let session_id = session.id.clone();
        ensure_db_session(&ctx.db, &session).await;

        let res = request.post(&format!("/repo/resume/{session_id}")).await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect, got {status}"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn resume_endpoint_returns_404_for_invalid_session() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.post("/repo/resume/nonexistent-session").await;

        assert_eq!(res.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_loading_page() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_test_session();
        let session_id = session.id.clone();
        ensure_db_session(&ctx.db, &session).await;

        let res = request.get(&format!("/review/{session_id}/loading")).await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("Analyzing Repository"),
            "should show loading page heading"
        );
        assert!(body.contains("feature-test"), "should show branch name");
        assert!(
            body.contains("analysis-status"),
            "should have status polling element"
        );
        assert!(
            body.contains("hx-trigger"),
            "should have HTMX polling trigger"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn loading_status_returns_progress() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_test_session();
        let session_id = session.id.clone();
        ensure_db_session(&ctx.db, &session).await;

        let res = request.get(&format!("/review/{session_id}/status")).await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("Implementation Approach"),
            "should show approach status"
        );
        assert!(
            body.contains("loading-spinner"),
            "should show spinners for pending items"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn loading_status_redirects_when_summary_ready() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_test_session();
        let session_id = session.id.clone();
        ensure_db_session(&ctx.db, &session).await;

        let model = rs_model::Model::find_by_session_key(&ctx.db, &session_id)
            .await
            .unwrap()
            .unwrap();

        ai_analyses::upsert(&ctx.db, model.id, "overview", None, "overview content")
            .await
            .unwrap();
        ai_analyses::upsert(&ctx.db, model.id, "changes", None, "changes content")
            .await
            .unwrap();
        ai_analyses::upsert(&ctx.db, model.id, "approach", None, "approach content")
            .await
            .unwrap();

        let res = request.get(&format!("/review/{session_id}/status")).await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect when summary is ready, got {status}"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn plan_status_shows_spinner_when_not_ready() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_test_session();
        let session_id = session.id.clone();
        ensure_db_session(&ctx.db, &session).await;

        let res = request
            .get(&format!("/review/{session_id}/plan-status"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("Preparing review plan"),
            "should show preparing message"
        );
        assert!(body.contains("disabled"), "button should be disabled");

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn plan_status_shows_button_when_ready() {
    request::<App, _, _>(|request, ctx| async move {
        let session = make_session_with_plan();
        let session_id = session.id.clone();
        sync_session_to_db(&ctx.db, &session).await;

        let res = request
            .get(&format!("/review/{session_id}/plan-status"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("Start Review"),
            "should show Start Review button when plan is ready"
        );
        assert!(body.contains("/guide"), "should link to guide page");

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn loading_page_returns_404_for_invalid_session() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.get("/review/nonexistent-session/loading").await;
        assert_eq!(res.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn first_unvalidated_step_returns_correct_step() {
    let mut session = make_session_with_plan();

    session.validated_steps = vec![StepValidation::default(), StepValidation::default()];
    assert_eq!(session.first_unvalidated_step(), Some(1));

    let mut sv_validated = StepValidation::default();
    sv_validated.validate_file("src/lib.rs");
    session.validated_steps = vec![sv_validated, StepValidation::default()];
    assert_eq!(session.first_unvalidated_step(), Some(2));

    let mut sv1 = StepValidation::default();
    sv1.validate_file("src/lib.rs");
    let mut sv2 = StepValidation::default();
    sv2.validate_file("src/new.rs");
    session.validated_steps = vec![sv1, sv2];
    assert_eq!(session.first_unvalidated_step(), None);

    cleanup_session(&session.id);
}
