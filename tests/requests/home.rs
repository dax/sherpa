use loco_rs::testing::prelude::*;
use serial_test::serial;
use sherpa::app::App;

#[tokio::test]
#[serial]
async fn can_get_home() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.get("/api").await;

        assert_eq!(res.status_code(), 200);
        res.assert_json(&serde_json::json!({"app_name":"loco"}));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_index_page() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.get("/").await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(body.contains("Sherpa"), "page should contain app name");
        assert!(
            body.contains("htmx.min.js"),
            "page should include HTMX script"
        );
        assert!(
            body.contains("flyonui"),
            "page should include FlyonUI assets"
        );
        assert!(
            body.contains("diff2html"),
            "page should include diff2html assets"
        );
        assert!(
            body.contains("/cli/setup") || body.contains("AI Backend"),
            "page should show CLI setup link or backend status"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_get_greeting_fragment() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.get("/greeting").await;

        assert_eq!(res.status_code(), 200);

        let body = res.text();
        assert!(
            body.contains("Hello from HTMX"),
            "fragment should contain greeting text"
        );
        assert!(
            !body.contains("<!DOCTYPE"),
            "fragment should NOT be a full HTML document"
        );
    })
    .await;
}
