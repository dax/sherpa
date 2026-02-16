use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "review_sessions")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique)]
    pub session_key: String,
    pub repo_path: String,
    pub branch: String,
    pub default_branch: String,
    pub merge_base: String,
    #[sea_orm(column_type = "Text")]
    pub diff: String,
    #[sea_orm(column_type = "Text")]
    pub changed_files: String,
    #[sea_orm(column_type = "Text")]
    pub metrics: String,
    #[sea_orm(column_type = "Text", nullable)]
    pub review_plan: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub validated_steps: String,
    pub primed_session_id: Option<String>,
    pub review_mode: String,
    pub agent_token: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::ai_analyses::Entity")]
    AiAnalyses,
    #[sea_orm(has_many = "super::chat_messages::Entity")]
    ChatMessages,
}

impl Related<super::ai_analyses::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::AiAnalyses.def()
    }
}

impl Related<super::chat_messages::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ChatMessages.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
