use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "chat_messages")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub session_id: i32,
    pub step_number: Option<i32>,
    pub role: String,
    #[sea_orm(column_type = "Text")]
    pub content: String,
    pub created_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::review_sessions::Entity",
        from = "Column::SessionId",
        to = "super::review_sessions::Column::Id"
    )]
    ReviewSession,
}

impl Related<super::review_sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ReviewSession.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
