use loco_rs::prelude::*;

#[debug_handler]
async fn index(ViewEngine(v): ViewEngine<TeraView>) -> Result<Response> {
    format::render().view(&v, "home/index.html", data!({"app_name": "Sherpa"}))
}

#[debug_handler]
async fn greeting(ViewEngine(v): ViewEngine<TeraView>) -> Result<Response> {
    format::render().view(&v, "home/_greeting.html", data!({}))
}

#[debug_handler]
async fn api_home() -> Result<Response> {
    format::json(serde_json::json!({"app_name": "loco"}))
}

pub fn page_routes() -> Routes {
    Routes::new()
        .add("/", get(index))
        .add("/greeting", get(greeting))
}

pub fn api_routes() -> Routes {
    Routes::new()
        .prefix("/api")
        .add("/", get(api_home))
}
