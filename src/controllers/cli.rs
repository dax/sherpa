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
}

#[debug_handler]
async fn cli_setup(ViewEngine(v): ViewEngine<TeraView>) -> Result<Response> {
    let detection = cli_detection::detect_cli_tools();
    let config = load_config();

    let pre_selected = config
        .ai
        .selected_cli
        .or_else(|| detection.single_available());

    format::render().view(
        &v,
        "cli/setup.html",
        data!({
            "tools": detection.tools,
            "none_available": detection.none_available(),
            "pre_selected": pre_selected,
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

    if let Err(e) = config.save(&config_path) {
        return format::render().view(
            &v,
            "cli/_error.html",
            data!({"message": format!("Failed to save config: {e}")}),
        );
    }

    format::render().view(
        &v,
        "cli/_success.html",
        data!({
            "selected_name": selected.display_name(),
            "selected_value": selected.binary_name(),
        }),
    )
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
}

pub fn api_routes() -> Routes {
    Routes::new()
        .prefix("/api/cli")
        .add("/status", get(cli_status))
}
