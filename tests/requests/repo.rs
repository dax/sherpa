use loco_rs::testing::prelude::*;
use serial_test::serial;
use sherpa::app::App;

#[tokio::test]
#[serial]
async fn can_get_repo_analyze_page() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.get("/repo/analyze").await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("Analyze Repository"),
            "page should contain heading"
        );
        assert!(
            body.contains(r#"name="path""#),
            "page should contain path input"
        );
        assert!(
            body.contains("hx-post"),
            "page should use HTMX for form submission"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn repo_analyze_rejects_nonexistent_path() {
    request::<App, _, _>(|request, _ctx| async move {
        let payload = serde_json::json!({"path": "/nonexistent/path/to/repo"});
        let res = request.post("/repo/analyze").form(&payload).await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("alert-error"),
            "should show error alert for nonexistent path"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn repo_analyze_rejects_non_git_dir() {
    request::<App, _, _>(|request, _ctx| async move {
        let payload = serde_json::json!({"path": "/tmp"});
        let res = request.post("/repo/analyze").form(&payload).await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("alert-error"),
            "should show error alert for non-git directory"
        );
    })
    .await;
}
