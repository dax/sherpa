use sea_orm::{entity::prelude::*, ActiveValue::Set};

use super::_entities::review_sessions::Column;
pub use super::_entities::review_sessions::{ActiveModel, Entity, Model};

impl Model {
    pub async fn find_by_session_key(
        db: &DatabaseConnection,
        key: &str,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(Column::SessionKey.eq(key))
            .one(db)
            .await
    }

    pub async fn find_by_repo_and_branch(
        db: &DatabaseConnection,
        repo_path: &str,
        branch: &str,
    ) -> Result<Option<Self>, DbErr> {
        Entity::find()
            .filter(Column::RepoPath.eq(repo_path))
            .filter(Column::Branch.eq(branch))
            .one(db)
            .await
    }

    pub async fn delete_by_repo_and_branch(
        db: &DatabaseConnection,
        repo_path: &str,
        branch: &str,
    ) -> Result<(), DbErr> {
        Entity::delete_many()
            .filter(Column::RepoPath.eq(repo_path))
            .filter(Column::Branch.eq(branch))
            .exec(db)
            .await?;
        Ok(())
    }
}

impl ActiveModel {
    pub fn from_review_session(session: &crate::services::review_session::ReviewSession) -> Self {
        let now = chrono::Utc::now().naive_utc();
        Self {
            id: sea_orm::ActiveValue::NotSet,
            session_key: Set(session.id.clone()),
            repo_path: Set(session.repo_path.clone()),
            branch: Set(session.branch.clone()),
            default_branch: Set(session.default_branch.clone()),
            merge_base: Set(session.merge_base.clone()),
            diff: Set(session.diff.clone()),
            changed_files: Set(serde_json::to_string(&session.changed_files).unwrap_or_default()),
            metrics: Set(serde_json::to_string(&session.metrics).unwrap_or_default()),
            review_plan: Set(session
                .review_plan
                .as_ref()
                .and_then(|p| serde_json::to_string(p).ok())),
            validated_steps: Set(
                serde_json::to_string(&session.validated_steps).unwrap_or_else(|_| "[]".into())
            ),
            primed_session_id: Set(session.primed_session_id.clone()),
            created_at: Set(now),
            updated_at: Set(now),
        }
    }
}

impl Model {
    pub fn to_review_session(&self) -> crate::services::review_session::ReviewSession {
        use crate::services::review_session::*;

        let changed_files: Vec<crate::services::git_analysis::ChangedFile> =
            serde_json::from_str(&self.changed_files).unwrap_or_default();
        let metrics: DiffMetrics = serde_json::from_str(&self.metrics).unwrap_or_default();
        let review_plan: Option<ReviewPlan> = self
            .review_plan
            .as_ref()
            .and_then(|p| serde_json::from_str(p).ok());
        let validated_steps: Vec<bool> =
            serde_json::from_str(&self.validated_steps).unwrap_or_default();

        ReviewSession {
            id: self.session_key.clone(),
            repo_path: self.repo_path.clone(),
            branch: self.branch.clone(),
            default_branch: self.default_branch.clone(),
            merge_base: self.merge_base.clone(),
            diff: self.diff.clone(),
            changed_files,
            created_at: self.created_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            summary: SummaryData::default(),
            chat_messages: Vec::new(),
            metrics,
            review_plan,
            validated_steps,
            primed_session_id: self.primed_session_id.clone(),
        }
    }
}

pub async fn find_or_create(
    db: &DatabaseConnection,
    session: &crate::services::review_session::ReviewSession,
) -> Result<Model, DbErr> {
    if let Some(existing) = Model::find_by_session_key(db, &session.id).await? {
        return Ok(existing);
    }
    let active = ActiveModel::from_review_session(session);
    active.insert(db).await
}

pub async fn update_review_plan(
    db: &DatabaseConnection,
    session_key: &str,
    plan_json: Option<String>,
) -> Result<(), DbErr> {
    let model = Model::find_by_session_key(db, session_key)
        .await?
        .ok_or(DbErr::RecordNotFound("session not found".into()))?;
    let now = chrono::Utc::now().naive_utc();
    let mut active: ActiveModel = model.into();
    active.review_plan = Set(plan_json);
    active.updated_at = Set(now);
    active.update(db).await?;
    Ok(())
}

pub async fn update_primed_session(
    db: &DatabaseConnection,
    session_key: &str,
    primed_session_id: &str,
) -> Result<(), DbErr> {
    let model = Model::find_by_session_key(db, session_key)
        .await?
        .ok_or(DbErr::RecordNotFound("session not found".into()))?;
    let now = chrono::Utc::now().naive_utc();
    let mut active: ActiveModel = model.into();
    active.primed_session_id = Set(Some(primed_session_id.to_string()));
    active.updated_at = Set(now);
    active.update(db).await?;
    Ok(())
}

pub async fn update_validated_steps(
    db: &DatabaseConnection,
    session_key: &str,
    steps: &[bool],
) -> Result<(), DbErr> {
    let model = Model::find_by_session_key(db, session_key)
        .await?
        .ok_or(DbErr::RecordNotFound("session not found".into()))?;
    let now = chrono::Utc::now().naive_utc();
    let mut active: ActiveModel = model.into();
    active.validated_steps = Set(serde_json::to_string(steps).unwrap_or_else(|_| "[]".into()));
    active.updated_at = Set(now);
    active.update(db).await?;
    Ok(())
}
