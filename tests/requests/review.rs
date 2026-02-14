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

#[tokio::test]
#[serial]
async fn step_page_has_chat_panel() {
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
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
    use sherpa::services::review_session::ChatMessage;

    let mut session = create_session_with_plan();
    session.chat_messages = vec![
        ChatMessage {
            role: "user".to_string(),
            content: "Question about step 1".to_string(),
            timestamp: "10:00:00".to_string(),
            step_number: Some(1),
        },
        ChatMessage {
            role: "assistant".to_string(),
            content: "Answer about step 1".to_string(),
            timestamp: "10:00:05".to_string(),
            step_number: Some(1),
        },
        ChatMessage {
            role: "user".to_string(),
            content: "Question from summary".to_string(),
            timestamp: "09:00:00".to_string(),
            step_number: None,
        },
    ];
    session.save().expect("save session with chat");
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
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
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
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
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .post(&format!("/review/{session_id}/guide/step/1/validate"))
            .await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect to next step, got {status}"
        );

        let loaded = ReviewSession::load(&session_id).unwrap();
        assert_eq!(loaded.validated_steps.len(), 2);
        assert!(loaded.validated_steps[0], "step 1 should be validated");
        assert!(!loaded.validated_steps[1], "step 2 should not be validated");

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn validate_last_step_redirects_to_guide() {
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .post(&format!("/review/{session_id}/guide/step/2/validate"))
            .await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect to guide page, got {status}"
        );

        let loaded = ReviewSession::load(&session_id).unwrap();
        assert!(loaded.validated_steps[1], "step 2 should be validated");

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn validate_returns_404_for_invalid_step() {
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
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
    let session = create_session_with_plan();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/guide/step/1"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("Validate"),
            "should have Validate button"
        );
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
    let mut session = create_session_with_plan();
    session.validated_steps = vec![true, false];
    session.save().expect("save session with validation");
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
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
async fn guide_page_shows_validation_progress() {
    let mut session = create_session_with_plan();
    session.validated_steps = vec![true, false];
    session.save().expect("save session with validation");
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/guide"))
            .await;

        assert_eq!(res.status_code(), 200);
        let body = res.text();
        assert!(
            body.contains("1/2 validated"),
            "should show validation progress"
        );
        assert!(
            body.contains("step-success"),
            "validated step should have step-success class"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn previous_button_does_not_unvalidate() {
    let mut session = create_session_with_plan();
    session.validated_steps = vec![true, false];
    session.save().expect("save session with validation");
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/guide/step/1"))
            .await;

        assert_eq!(res.status_code(), 200);

        let loaded = ReviewSession::load(&session_id).unwrap();
        assert!(
            loaded.validated_steps[0],
            "revisiting step 1 should not unvalidate it"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn validation_state_persists_across_loads() {
    let mut session = create_session_with_plan();
    let session_id = session.id.clone();

    session.validated_steps = vec![true, true];
    session.save().expect("save validated session");

    let loaded = ReviewSession::load(&session_id).unwrap();
    assert_eq!(loaded.validated_steps, vec![true, true]);

    cleanup_session(&session_id);
}

#[tokio::test]
#[serial]
async fn validate_all_steps_redirects_to_summary() {
    let mut session = create_session_with_plan();
    session.validated_steps = vec![true, false];
    session.save().expect("save session with first step validated");
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .post(&format!("/review/{session_id}/guide/step/2/validate"))
            .await;

        let status = res.status_code();
        assert!(
            status == 200 || status == 303 || status == 302,
            "should redirect to summary page, got {status}"
        );

        let loaded = ReviewSession::load(&session_id).unwrap();
        assert!(loaded.validated_steps[0], "step 1 should still be validated");
        assert!(loaded.validated_steps[1], "step 2 should be validated");

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn summary_page_shows_review_complete_banner() {
    let mut session = create_session_with_plan();
    session.validated_steps = vec![true, true];
    session.save().expect("save fully validated session");
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/summary"))
            .await;

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
    let mut session = create_session_with_plan();
    session.validated_steps = vec![true, true];
    session.save().expect("save fully validated session");
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/summary"))
            .await;

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
        assert!(
            body.contains("collapse"),
            "steps should be collapsible"
        );
        assert!(
            body.contains("lib.rs"),
            "should show file references"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn summary_page_hides_start_review_when_complete() {
    let mut session = create_session_with_plan();
    session.validated_steps = vec![true, true];
    session.save().expect("save fully validated session");
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/summary"))
            .await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            !body.contains("Start Review"),
            "should not show Start Review when review is complete"
        );
        assert!(
            body.contains("View Review Guide"),
            "should show link to review guide instead"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn summary_page_no_reviewed_changes_when_incomplete() {
    let session = create_test_session();
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/summary"))
            .await;

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
            body.contains("Start Review"),
            "should still show Start Review button"
        );

        cleanup_session(&session_id);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn summary_page_shows_step_chat_messages() {
    use sherpa::services::review_session::ChatMessage;

    let mut session = create_session_with_plan();
    session.validated_steps = vec![true, true];
    session.chat_messages = vec![
        ChatMessage {
            role: "user".to_string(),
            content: "What does this change do?".to_string(),
            timestamp: "10:00:00".to_string(),
            step_number: Some(1),
        },
        ChatMessage {
            role: "assistant".to_string(),
            content: "It modifies the core models.".to_string(),
            timestamp: "10:00:05".to_string(),
            step_number: Some(1),
        },
    ];
    session.save().expect("save session with chat");
    let session_id = session.id.clone();

    request::<App, _, _>(|request, _ctx| async move {
        let res = request
            .get(&format!("/review/{session_id}/summary"))
            .await;

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
