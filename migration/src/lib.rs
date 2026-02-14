#![allow(elided_lifetimes_in_paths)]
#![allow(clippy::wildcard_imports)]
pub use sea_orm_migration::prelude::*;

mod m20260214_000001_create_review_sessions;
mod m20260214_000002_create_ai_analyses;
mod m20260214_000003_create_chat_messages;
mod m20260215_000001_add_status_to_ai_analyses;
mod m20260215_000002_add_primed_session_to_review_sessions;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260214_000001_create_review_sessions::Migration),
            Box::new(m20260214_000002_create_ai_analyses::Migration),
            Box::new(m20260214_000003_create_chat_messages::Migration),
            Box::new(m20260215_000001_add_status_to_ai_analyses::Migration),
            Box::new(m20260215_000002_add_primed_session_to_review_sessions::Migration),
        ]
    }
}
