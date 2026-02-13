use loco_rs::testing::prelude::*;
use serial_test::serial;
use sherpa::app::App;

#[tokio::test]
#[serial]
async fn can_get_cli_setup_page() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.get("/cli/setup").await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("Select AI Backend"),
            "page should contain selection heading"
        );
        assert!(
            body.contains("opencode") || body.contains("OpenCode"),
            "page should list opencode option"
        );
        assert!(
            body.contains("claude") || body.contains("Claude"),
            "page should list claude option"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_cli_status_api() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.get("/api/cli/status").await;

        assert_eq!(res.status_code(), 200);

        let body: serde_json::Value = serde_json::from_str(&res.text()).unwrap();
        assert!(body.get("tools").is_some(), "response should have tools array");
        assert!(
            body.get("none_available").is_some(),
            "response should have none_available field"
        );

        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2, "should report status for both CLI tools");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn cli_select_rejects_unknown_tool() {
    request::<App, _, _>(|request, _ctx| async move {
        let payload = serde_json::json!({"cli_tool": "unknown_tool"});
        let res = request
            .post("/cli/select")
            .form(&payload)
            .await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("Unknown CLI tool"),
            "should show error for unknown tool"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn home_page_shows_cli_status() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.get("/").await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        let has_setup_link = body.contains("/cli/setup");
        let has_backend_badge = body.contains("AI Backend");
        assert!(
            has_setup_link || has_backend_badge,
            "home page should show CLI setup link or current backend status"
        );
    })
    .await;
}
