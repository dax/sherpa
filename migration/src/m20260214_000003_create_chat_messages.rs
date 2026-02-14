use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ChatMessages::Table)
                    .if_not_exists()
                    .col(pk_auto(ChatMessages::Id))
                    .col(integer(ChatMessages::SessionId))
                    .col(integer_null(ChatMessages::StepNumber))
                    .col(string(ChatMessages::Role))
                    .col(text(ChatMessages::Content))
                    .col(timestamp(ChatMessages::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(ChatMessages::Table, ChatMessages::SessionId)
                            .to(
                                super::m20260214_000001_create_review_sessions::ReviewSessions::Table,
                                super::m20260214_000001_create_review_sessions::ReviewSessions::Id,
                            )
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_chat_session")
                    .table(ChatMessages::Table)
                    .col(ChatMessages::SessionId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_chat_session_step")
                    .table(ChatMessages::Table)
                    .col(ChatMessages::SessionId)
                    .col(ChatMessages::StepNumber)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ChatMessages::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum ChatMessages {
    Table,
    Id,
    SessionId,
    StepNumber,
    Role,
    Content,
    CreatedAt,
}
