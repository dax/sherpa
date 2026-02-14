use loco_rs::testing::prelude::*;
use serial_test::serial;
use sherpa::app::App;
use sherpa::services::git_analysis::{ChangedFile, FileStatus, GitAnalysis};
use sherpa::services::review_session::ReviewSession;

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
