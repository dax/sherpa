use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AiAnalyses::Table)
                    .if_not_exists()
                    .col(pk_auto(AiAnalyses::Id))
                    .col(integer(AiAnalyses::SessionId))
                    .col(string(AiAnalyses::AnalysisType))
                    .col(integer_null(AiAnalyses::StepNumber))
                    .col(text(AiAnalyses::Content))
                    .col(timestamp(AiAnalyses::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(AiAnalyses::Table, AiAnalyses::SessionId)
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
                    .name("idx_analyses_unique")
                    .table(AiAnalyses::Table)
                    .col(AiAnalyses::SessionId)
                    .col(AiAnalyses::AnalysisType)
                    .col(AiAnalyses::StepNumber)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AiAnalyses::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum AiAnalyses {
    Table,
    Id,
    SessionId,
    AnalysisType,
    StepNumber,
    Content,
    CreatedAt,
}
