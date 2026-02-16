use async_trait::async_trait;
use loco_rs::{
    app::{AppContext, Hooks, Initializer},
    bgworker::Queue,
    boot::{create_app, BootResult, StartMode},
    config::Config,
    controller::AppRoutes,
    environment::Environment,
    task::Tasks,
    Result,
};
use migration::Migrator;
use sea_orm::EntityTrait;
use std::path::Path;

#[allow(unused_imports)]
use crate::{controllers, initializers, models::_entities, tasks};

pub struct App;
#[async_trait]
impl Hooks for App {
    fn app_name() -> &'static str {
        env!("CARGO_CRATE_NAME")
    }

    fn app_version() -> String {
        format!(
            "{} ({})",
            env!("CARGO_PKG_VERSION"),
            option_env!("BUILD_SHA")
                .or(option_env!("GITHUB_SHA"))
                .unwrap_or("dev")
        )
    }

    async fn boot(
        mode: StartMode,
        environment: &Environment,
        config: Config,
    ) -> Result<BootResult> {
        create_app::<Self, Migrator>(mode, environment, config).await
    }

    async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
        Ok(vec![Box::new(
            initializers::view_engine::ViewEngineInitializer,
        )])
    }

    fn routes(_ctx: &AppContext) -> AppRoutes {
        AppRoutes::with_default_routes()
            .add_route(controllers::home::page_routes())
            .add_route(controllers::home::api_routes())
            .add_route(controllers::cli::page_routes())
            .add_route(controllers::cli::api_routes())
            .add_route(controllers::repo::page_routes())
            .add_route(controllers::repo::api_routes())
            .add_route(controllers::review::page_routes())
            .add_route(controllers::review::api_routes())
            .add_route(controllers::agent::page_routes())
            .add_route(controllers::agent::api_routes())
    }
    async fn connect_workers(_ctx: &AppContext, _queue: &Queue) -> Result<()> {
        Ok(())
    }

    #[allow(unused_variables)]
    fn register_tasks(tasks: &mut Tasks) {
        // tasks-inject (do not remove)
    }

    async fn truncate(ctx: &AppContext) -> Result<()> {
        _entities::chat_messages::Entity::delete_many()
            .exec(&ctx.db)
            .await?;
        _entities::ai_analyses::Entity::delete_many()
            .exec(&ctx.db)
            .await?;
        _entities::review_sessions::Entity::delete_many()
            .exec(&ctx.db)
            .await?;
        Ok(())
    }

    async fn seed(_ctx: &AppContext, _base: &Path) -> Result<()> {
        Ok(())
    }
}
