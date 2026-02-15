use axum::extract::Query;
use axum::Form;
use loco_rs::prelude::*;
use serde::Deserialize;

use crate::services::{
    cli_detection::{self, AiCli},
    config::SherpaConfig,
};

#[derive(Deserialize)]
pub struct CliSelectForm {
    cli_tool: String,
    deep_model: Option<String>,
    fast_model: Option<String>,
}

#[derive(Deserialize)]
pub struct ModelsQuery {
    cli: String,
}

#[debug_handler]
async fn cli_setup(ViewEngine(v): ViewEngine<TeraView>) -> Result<Response> {
    let detection = cli_detection::detect_cli_tools();
    let config = load_config();

    let pre_selected = config
        .ai
        .selected_cli
        .or_else(|| detection.single_available());

    let models = if let Some(cli) = pre_selected {
        cli_detection::list_models(cli).await
    } else {
        Vec::new()
    };

    format::render().view(
        &v,
        "cli/setup.html",
        data!({
            "tools": detection.tools,
            "none_available": detection.none_available(),
            "pre_selected": pre_selected,
            "deep_model": config.ai.deep_model,
            "fast_model": config.ai.fast_model,
            "models": models,
        }),
    )
}

#[debug_handler]
async fn cli_models(
    ViewEngine(v): ViewEngine<TeraView>,
    Query(query): Query<ModelsQuery>,
) -> Result<Response> {
    let cli = match query.cli.as_str() {
        "opencode" => AiCli::Opencode,
        "claude" => AiCli::Claude,
        _ => return format::render().view(&v, "cli/_model_selects.html", data!({"models": []})),
    };

    let config = load_config();
    let models = cli_detection::list_models(cli).await;

    let same_cli = config.ai.selected_cli == Some(cli);
    let deep_model = if same_cli { config.ai.deep_model } else { None };
    let fast_model = if same_cli { config.ai.fast_model } else { None };

    format::render().view(
        &v,
        "cli/_model_selects.html",
        data!({
            "models": models,
            "deep_model": deep_model,
            "fast_model": fast_model,
        }),
    )
}

#[debug_handler]
async fn cli_status() -> Result<Response> {
    let detection = cli_detection::detect_cli_tools();
    format::json(serde_json::json!({
        "tools": detection.tools,
        "none_available": detection.none_available(),
    }))
}

#[debug_handler]
async fn cli_select(
    ViewEngine(v): ViewEngine<TeraView>,
    Form(form): Form<CliSelectForm>,
) -> Result<Response> {
    let selected = match form.cli_tool.as_str() {
        "opencode" => AiCli::Opencode,
        "claude" => AiCli::Claude,
        _ => {
            return format::render().view(
                &v,
                "cli/_error.html",
                data!({"message": format!("Unknown CLI tool: {}", form.cli_tool)}),
            );
        }
    };

    let detection = cli_detection::detect_cli_tools();
    let tool_status = detection.tools.iter().find(|t| t.cli == selected);

    if !tool_status.is_some_and(|t| t.available) {
        return format::render().view(
            &v,
            "cli/_error.html",
            data!({"message": format!("{} is not installed on this system", selected)}),
        );
    }

    let config_path = match SherpaConfig::default_path() {
        Ok(p) => p,
        Err(e) => {
            return format::render().view(
                &v,
                "cli/_error.html",
                data!({"message": format!("Failed to determine config path: {e}")}),
            );
        }
    };

    let mut config = load_config();
    config.ai.selected_cli = Some(selected);
    config.ai.deep_model = form
        .deep_model
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());
    config.ai.fast_model = form
        .fast_model
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());

    if let Err(e) = config.save(&config_path) {
        return format::render().view(
            &v,
            "cli/_error.html",
            data!({"message": format!("Failed to save config: {e}")}),
        );
    }

    Ok(axum::response::Response::builder()
        .header("HX-Redirect", "/")
        .body(axum::body::Body::empty())
        .unwrap()
        .into_response())
}

fn load_config() -> SherpaConfig {
    SherpaConfig::default_path()
        .ok()
        .and_then(|p| SherpaConfig::load(&p).ok())
        .unwrap_or_default()
}

pub fn page_routes() -> Routes {
    Routes::new()
        .prefix("/cli")
        .add("/setup", get(cli_setup))
        .add("/select", post(cli_select))
        .add("/models", get(cli_models))
}

pub fn api_routes() -> Routes {
    Routes::new()
        .prefix("/api/cli")
        .add("/status", get(cli_status))
}
