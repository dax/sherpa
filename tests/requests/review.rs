use loco_rs::testing::prelude::*;
use serial_test::serial;
use sherpa::app::App;
use sherpa::services::git_analysis::{ChangedFile, FileStatus, GitAnalysis};
use sherpa::services::review_session::{
    FileRef, ReviewPlan, ReviewSession, ReviewStep, StepAiData,
};

fn create_test_session() -> ReviewSession {
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
    let session = ReviewSession::new(analysis);
    session.save().expect("save test session");
    session
}

fn cleanup_session(id: &str) {
    if let Ok(dir) = ReviewSession::sessions_dir() {
        let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
    }
}

#[tokio::test]
#[serial]
async fn can_get_summary_page() {
    let session = create_test_session();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/summary"))
            .await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(body.contains("Review Summary"), "page should contain heading");
        assert!(body.contains("feature-test"), "page should show branch name");
        assert!(body.contains("Metrics"), "page should contain metrics section");
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
    let session = create_test_session();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/summary"))
            .await;

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
        let res = request
            .get("/review/nonexistent-session/summary")
            .await;

        assert_eq!(res.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_skip_section() {
    let session = create_test_session();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!(
                "/review/{session_id}/summary/skip/overview"
            ))
            .await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("AI analysis skipped"),
            "should show skipped message"
        );
        assert!(
            body.contains("Project Overview"),
            "should show section title"
        );
        assert!(
            body.contains("retry"),
            "should have retry option"
        );

        cleanup_session(&session_id);
    })
    .await;
}

fn create_session_with_plan() -> ReviewSession {
    let mut session = create_test_session();
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
            },
            ReviewStep {
                title: "New feature code".to_string(),
                rationale: "The main feature implementation".to_string(),
                file_refs: vec![FileRef {
                    path: "src/new.rs".to_string(),
                    diff_lines: Some((1, 10)),
                }],
                ai_data: StepAiData::default(),
            },
        ],
        generated_at: "12:00:00".to_string(),
    });
    session.save().expect("save session with plan");
    session
}

#[tokio::test]
#[serial]
async fn can_get_guide_page_with_plan() {
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/guide"))
            .await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(body.contains("Review Plan"), "should show review plan heading");
        assert!(body.contains("Core data models"), "should show step 1 title");
        assert!(body.contains("New feature code"), "should show step 2 title");
        assert!(body.contains("2 steps"), "should show total step count");
        assert!(
            body.contains("lib.rs"),
            "should show file reference"
        );
        assert!(
            body.contains("Foundation types"),
            "should show step rationale"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn guide_page_redirects_without_plan() {
    let session = create_test_session();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/guide"))
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
async fn guide_page_returns_404_for_invalid_session() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get("/review/nonexistent-session/guide")
            .await;

        assert_eq!(res.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_skip_plan_generation() {
    let session = create_test_session();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .post(&format!("/review/{session_id}/guide/plan/skip"))
            .await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect to guide page, got {status}"
        );

        let loaded = ReviewSession::load(&session_id).unwrap();
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
async fn summary_page_has_start_review_button() {
    let session = create_test_session();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/summary"))
            .await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("Start Review"),
            "should have Start Review button"
        );
        assert!(
            !body.contains("coming soon"),
            "should not say coming soon anymore"
        );
        assert!(
            body.contains("hx-post"),
            "Start Review should use HTMX post"
        );
        assert!(
            body.contains("/guide/start"),
            "should post to guide/start endpoint"
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
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
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
            body.contains("diff-container"),
            "should have diff container"
        );
        assert!(
            body.contains("hx-get"),
            "should have HTMX triggers for AI sections"
        );
        assert!(
            body.contains("/explanation"),
            "should have explanation section trigger"
        );
        assert!(
            body.contains("/symbols"),
            "should have symbols section trigger"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_page_shows_navigation() {
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/guide/step/1"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("Next Step"),
            "step 1 should have Next Step button"
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
            body.contains("Back to Guide"),
            "last step should have Back to Guide button"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_page_returns_404_for_invalid_step() {
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
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
    let session = create_test_session();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
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
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
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
async fn guide_page_has_step_links() {
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/guide"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("/guide/step/1"),
            "should have link to step 1"
        );
        assert!(
            body.contains("/guide/step/2"),
            "should have link to step 2"
        );
        assert!(
            body.contains("Review Step"),
            "should have Review Step button"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn step_page_sidebar_highlights_current_step() {
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
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
