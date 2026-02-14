use sea_orm::{entity::prelude::*, ActiveValue::Set};

use super::_entities::ai_analyses::Column;
pub use super::_entities::ai_analyses::{ActiveModel, Entity, Model};

pub async fn find_cached(
    db: &DatabaseConnection,
    session_db_id: i32,
    analysis_type: &str,
    step_number: Option<i32>,
) -> Result<Option<String>, DbErr> {
    let mut query = Entity::find()
        .filter(Column::SessionId.eq(session_db_id))
        .filter(Column::AnalysisType.eq(analysis_type))
        .filter(Column::Status.eq("success"));

    match step_number {
        Some(n) => query = query.filter(Column::StepNumber.eq(n)),
        None => query = query.filter(Column::StepNumber.is_null()),
    }

    let result = query.one(db).await?;
    Ok(result.map(|m| m.content))
}

pub async fn find_any(
    db: &DatabaseConnection,
    session_db_id: i32,
    analysis_type: &str,
    step_number: Option<i32>,
) -> Result<Option<Model>, DbErr> {
    let mut query = Entity::find()
        .filter(Column::SessionId.eq(session_db_id))
        .filter(Column::AnalysisType.eq(analysis_type));

    match step_number {
        Some(n) => query = query.filter(Column::StepNumber.eq(n)),
        None => query = query.filter(Column::StepNumber.is_null()),
    }

    query.one(db).await
}

pub async fn has_failures(db: &DatabaseConnection, session_db_id: i32) -> Result<bool, DbErr> {
    let count = Entity::find()
        .filter(Column::SessionId.eq(session_db_id))
        .filter(Column::Status.eq("failed"))
        .count(db)
        .await?;
    Ok(count > 0)
}

pub async fn first_failure_message(
    db: &DatabaseConnection,
    session_db_id: i32,
) -> Result<Option<String>, DbErr> {
    let record = Entity::find()
        .filter(Column::SessionId.eq(session_db_id))
        .filter(Column::Status.eq("failed"))
        .one(db)
        .await?;
    Ok(record.map(|m| m.content))
}

pub async fn delete_failures(db: &DatabaseConnection, session_db_id: i32) -> Result<(), DbErr> {
    Entity::delete_many()
        .filter(Column::SessionId.eq(session_db_id))
        .filter(Column::Status.eq("failed"))
        .exec(db)
        .await?;
    Ok(())
}

pub async fn upsert(
    db: &DatabaseConnection,
    session_db_id: i32,
    analysis_type: &str,
    step_number: Option<i32>,
    content: &str,
) -> Result<Model, DbErr> {
    let mut query = Entity::find()
        .filter(Column::SessionId.eq(session_db_id))
        .filter(Column::AnalysisType.eq(analysis_type));

    match step_number {
        Some(n) => query = query.filter(Column::StepNumber.eq(n)),
        None => query = query.filter(Column::StepNumber.is_null()),
    }

    let existing = query.one(db).await?;
    let now = chrono::Utc::now().naive_utc();

    if let Some(existing) = existing {
        let mut active: ActiveModel = existing.into();
        active.content = Set(content.to_string());
        active.status = Set("success".to_string());
        active.created_at = Set(now);
        active.update(db).await
    } else {
        let active = ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            session_id: Set(session_db_id),
            analysis_type: Set(analysis_type.to_string()),
            step_number: Set(step_number),
            content: Set(content.to_string()),
            created_at: Set(now),
            status: Set("success".to_string()),
        };
        active.insert(db).await
    }
}

pub async fn record_failure(
    db: &DatabaseConnection,
    session_db_id: i32,
    analysis_type: &str,
    step_number: Option<i32>,
    error_message: &str,
) -> Result<Model, DbErr> {
    let mut query = Entity::find()
        .filter(Column::SessionId.eq(session_db_id))
        .filter(Column::AnalysisType.eq(analysis_type));

    match step_number {
        Some(n) => query = query.filter(Column::StepNumber.eq(n)),
        None => query = query.filter(Column::StepNumber.is_null()),
    }

    let existing = query.one(db).await?;
    let now = chrono::Utc::now().naive_utc();

    if let Some(existing) = existing {
        let mut active: ActiveModel = existing.into();
        active.content = Set(error_message.to_string());
        active.status = Set("failed".to_string());
        active.created_at = Set(now);
        active.update(db).await
    } else {
        let active = ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            session_id: Set(session_db_id),
            analysis_type: Set(analysis_type.to_string()),
            step_number: Set(step_number),
            content: Set(error_message.to_string()),
            created_at: Set(now),
            status: Set("failed".to_string()),
        };
        active.insert(db).await
    }
}
