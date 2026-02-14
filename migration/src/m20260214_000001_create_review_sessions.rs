use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ReviewSessions::Table)
                    .if_not_exists()
                    .col(pk_auto(ReviewSessions::Id))
                    .col(string_uniq(ReviewSessions::SessionKey))
                    .col(string(ReviewSessions::RepoPath))
                    .col(string(ReviewSessions::Branch))
                    .col(string(ReviewSessions::DefaultBranch))
                    .col(string(ReviewSessions::MergeBase))
                    .col(text(ReviewSessions::Diff))
                    .col(text(ReviewSessions::ChangedFiles))
                    .col(text(ReviewSessions::Metrics))
                    .col(text_null(ReviewSessions::ReviewPlan))
                    .col(
                        ColumnDef::new(ReviewSessions::ValidatedSteps)
                            .text()
                            .not_null()
                            .default("[]"),
                    )
                    .col(timestamp(ReviewSessions::CreatedAt))
                    .col(timestamp(ReviewSessions::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_repo_branch")
                    .table(ReviewSessions::Table)
                    .col(ReviewSessions::RepoPath)
                    .col(ReviewSessions::Branch)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ReviewSessions::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub(crate) enum ReviewSessions {
    Table,
    Id,
    SessionKey,
    RepoPath,
    Branch,
    DefaultBranch,
    MergeBase,
    Diff,
    ChangedFiles,
    Metrics,
    ReviewPlan,
    ValidatedSteps,
    CreatedAt,
    UpdatedAt,
}
