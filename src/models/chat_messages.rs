use sea_orm::{entity::prelude::*, ActiveValue::Set, QueryOrder};

use super::_entities::chat_messages::Column;
pub use super::_entities::chat_messages::{ActiveModel, Entity, Model};

pub async fn find_by_session(
    db: &DatabaseConnection,
    session_db_id: i32,
) -> Result<Vec<Model>, DbErr> {
    Entity::find()
        .filter(Column::SessionId.eq(session_db_id))
        .order_by_asc(Column::CreatedAt)
        .all(db)
        .await
}

pub async fn find_by_session_and_step(
    db: &DatabaseConnection,
    session_db_id: i32,
    step_number: Option<i32>,
) -> Result<Vec<Model>, DbErr> {
    let mut query = Entity::find().filter(Column::SessionId.eq(session_db_id));

    match step_number {
        Some(n) => query = query.filter(Column::StepNumber.eq(n)),
        None => query = query.filter(Column::StepNumber.is_null()),
    }

    query.order_by_asc(Column::CreatedAt).all(db).await
}

pub async fn create(
    db: &DatabaseConnection,
    session_db_id: i32,
    step_number: Option<i32>,
    role: &str,
    content: &str,
) -> Result<Model, DbErr> {
    let now = chrono::Utc::now().naive_utc();
    let active = ActiveModel {
        id: sea_orm::ActiveValue::NotSet,
        session_id: Set(session_db_id),
        step_number: Set(step_number),
        role: Set(role.to_string()),
        content: Set(content.to_string()),
        created_at: Set(now),
    };
    active.insert(db).await
}
